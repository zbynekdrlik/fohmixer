//! The Live instance client against the real FohMixer script on SimLive
//! (`sim/host.py`), and against a scripted fake for the frames the real
//! script only sends when something is wrong.
#![cfg(unix)]

mod support;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fohmixer_hub::config::InstanceCfg;
use fohmixer_hub::live::client::{Events, LiveError, LiveEvent, LiveHandle};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use support::{Host, runtime, serial};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

/// The events of one instance, as they arrive.
#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<LiveEvent>>>);

impl Seen {
    fn events(&self) -> Events {
        let seen = self.clone();
        Arc::new(move |_: &str, event: LiveEvent| seen.0.lock().unwrap().push(event))
    }

    /// Waits up to `limit` for an event `pick` accepts; takes the events up
    /// to it.
    async fn wait<T>(&self, limit: Duration, pick: impl Fn(&LiveEvent) -> Option<T>) -> T {
        let deadline = Instant::now() + limit;
        loop {
            {
                let mut seen = self.0.lock().unwrap();
                if let Some(i) = seen.iter().position(|e| pick(e).is_some()) {
                    let found = pick(&seen[i]).unwrap();
                    seen.drain(..=i);
                    return found;
                }
            }
            assert!(
                Instant::now() < deadline,
                "no such event within {limit:?}: {:?}",
                self.0.lock().unwrap()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

fn get_name() -> Vec<Value> {
    vec![json!({"target": "live_set tracks 0", "name": "get_prop", "args": {"prop": "name"}})]
}

#[test]
fn a_request_reaches_the_script_and_its_result_comes_back() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let seen = Seen::default();
        let (live, _task) = LiveHandle::spawn(&host.cfg(), seen.events());
        let info = seen
            .wait(Duration::from_secs(3), |e| match e {
                LiveEvent::Connected(info) => Some(info.clone()),
                _ => None,
            })
            .await;
        assert_eq!(info.set_name, "Test Site");
        assert_eq!(info.instance, "band");
        assert_eq!(info.live_version, "12.2.5");
        let slots = live.call(get_name()).await.unwrap();
        assert_eq!(slots, vec![json!({"ok": true, "data": "Hand1 #"})]);
        let snap = live.snapshot();
        assert!(snap.online);
        assert!(!snap.busy);
        assert_eq!(snap.info, info);
        let deadline = Instant::now() + Duration::from_secs(1);
        while live.snapshot().main_tick_age_ms.is_none() {
            assert!(Instant::now() < deadline, "no heartbeat within 1 s");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        host.stop();
    });
}

#[test]
fn a_host_restart_is_a_disconnect_then_a_connect() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let port = host.port;
        let seen = Seen::default();
        let (live, _task) = LiveHandle::spawn(&host.cfg(), seen.events());
        seen.wait(Duration::from_secs(3), |e| {
            matches!(e, LiveEvent::Connected(_)).then_some(())
        })
        .await;
        host.stop();
        seen.wait(Duration::from_secs(1), |e| {
            matches!(e, LiveEvent::Disconnected).then_some(())
        })
        .await;
        assert!(!live.snapshot().online);
        assert_eq!(live.call(get_name()).await, Err(LiveError::Offline));
        // A few retries find nothing; the client keeps trying.
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert!(!live.snapshot().online);
        let host = Host::start_with("band", port, 0.0);
        seen.wait(Duration::from_secs(3), |e| {
            matches!(e, LiveEvent::Connected(_)).then_some(())
        })
        .await;
        assert_eq!(live.call(get_name()).await.unwrap()[0]["data"], "Hand1 #");
        host.stop();
    });
}

#[test]
fn a_main_thread_stall_is_busy_while_it_lasts() {
    let _serial = serial();
    runtime().block_on(async {
        let mut host = Host::start("band");
        let seen = Seen::default();
        let (live, _task) = LiveHandle::spawn(&host.cfg(), seen.events());
        seen.wait(Duration::from_secs(3), |e| {
            matches!(e, LiveEvent::Connected(_)).then_some(())
        })
        .await;
        host.stall(700);
        seen.wait(Duration::from_secs(1), |e| {
            matches!(e, LiveEvent::Busy { busy: true }).then_some(())
        })
        .await;
        assert!(live.snapshot().busy);
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut oldest = 0.0_f64;
        while Instant::now() < deadline && oldest < 300.0 {
            oldest = oldest.max(live.snapshot().main_tick_age_ms.unwrap_or(0.0));
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            oldest >= 300.0,
            "the heartbeat showed the stall: {oldest} ms"
        );
        seen.wait(Duration::from_secs(3), |e| {
            matches!(e, LiveEvent::Busy { busy: false }).then_some(())
        })
        .await;
        assert!(!live.snapshot().busy);
        host.stop();
    });
}

#[test]
fn a_sent_request_answers_as_an_event_in_order_with_the_pushes() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let seen = Seen::default();
        let (live, _task) = LiveHandle::spawn(&host.cfg(), seen.events());
        seen.wait(Duration::from_secs(3), |e| matches!(e, LiveEvent::Connected(_)).then_some(()))
            .await;
        live.send(
            "s1".into(),
            vec![json!({"target": "live_set tracks 0", "name": "add_listener", "args": {"prop": "mute"}})],
        );
        let data = seen
            .wait(Duration::from_secs(3), |e| match e {
                LiveEvent::Result { uuid, data } if uuid == "s1" => Some(data.clone()),
                _ => None,
            })
            .await;
        let key = data[0]["data"]["key"].as_str().unwrap().to_string();
        assert!(key.ends_with(".mute"), "{key}");
        let set = vec![json!({"target": "live_set tracks 0", "name": "set_prop", "args": {"prop": "mute", "value": true}})];
        assert_eq!(live.call(set).await.unwrap()[0]["ok"], true);
        let items = seen
            .wait(Duration::from_secs(3), |e| match e {
                LiveEvent::Values(items) => Some(items.clone()),
                _ => None,
            })
            .await;
        assert_eq!(items[0].key, key);
        assert_eq!(items[0].value, json!(true));
        host.stop();
    });
}

/// A fake script of `band` on a port of its own: it sends `connect`, then
/// answers every request with `answer(request)` (frames to send back).
async fn fake_script<F>(answer: F) -> u16
where
    F: Fn(&Value) -> Vec<String> + Send + 'static,
{
    fake_script_as("band", answer).await
}

/// [`fake_script`] of another instance (its one connection only).
async fn fake_script_as<F>(instance: &str, answer: F) -> u16
where
    F: Fn(&Value) -> Vec<String> + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let connect =
        json!({"event": "connect", "data": {"instance": instance, "set_name": "Fake"}}).to_string();
    tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(tcp).await.unwrap();
        ws.send(Message::Text(connect.into())).await.unwrap();
        while let Some(Ok(Message::Text(text))) = ws.next().await {
            let request: Value = serde_json::from_str(text.as_str()).unwrap();
            for frame in answer(&request) {
                if ws.send(Message::Text(frame.into())).await.is_err() {
                    return;
                }
            }
        }
    });
    port
}

fn cfg(port: u16) -> InstanceCfg {
    InstanceCfg {
        name: "band".into(),
        port,
    }
}

#[test]
fn a_refused_request_and_odd_frames_are_handled() {
    let _serial = serial();
    runtime().block_on(async {
        let port = fake_script(|request| {
            let uuid = request["uuid"].as_str().unwrap().to_string();
            match request["commands"][0]["name"].as_str().unwrap() {
                "refuse" => vec![
                    "not json".to_string(),
                    json!({"event": "surprise"}).to_string(),
                    json!({"event": "error", "uuid": uuid, "data": "missing or invalid commands array"}).to_string(),
                ],
                "refuse_quietly" => vec![
                    json!({"event": "error", "data": "no uuid"}).to_string(),
                    json!({"event": "error", "uuid": uuid, "data": "no"}).to_string(),
                ],
                _ => vec![json!({"event": "result", "uuid": "someone-else", "data": []}).to_string()],
            }
        })
        .await;
        let seen = Seen::default();
        let (live, _task) = LiveHandle::spawn(&cfg(port), seen.events());
        seen.wait(Duration::from_secs(3), |e| matches!(e, LiveEvent::Connected(_)).then_some(()))
            .await;
        assert_eq!(live.snapshot().info.set_name, "Fake");
        let refused = live
            .call(vec![json!({"target": "live_set", "name": "refuse"})])
            .await;
        assert_eq!(
            refused,
            Err(LiveError::Refused("missing or invalid commands array".into()))
        );
        // A refused request of the router comes back as a result without
        // slots (each slot then reads as "no result").
        live.send(
            "s7".into(),
            vec![json!({"target": "live_set", "name": "refuse_quietly"})],
        );
        let data = seen
            .wait(Duration::from_secs(3), |e| match e {
                LiveEvent::Result { uuid, data } if uuid == "s7" => Some(data.clone()),
                _ => None,
            })
            .await;
        assert!(data.is_empty());
        // A result nobody waits for goes to the events; the caller times out.
        let started = Instant::now();
        let lost = live
            .call(vec![json!({"target": "live_set", "name": "mislabel"})])
            .await;
        assert_eq!(lost, Err(LiveError::Timeout));
        assert!(started.elapsed() >= Duration::from_millis(2900));
        seen.wait(Duration::from_secs(1), |e| match e {
            LiveEvent::Result { uuid, .. } if uuid == "someone-else" => Some(()),
            _ => None,
        })
        .await;
    });
}

#[test]
fn a_silent_script_is_busy_after_300_ms_without_a_heartbeat() {
    let _serial = serial();
    runtime().block_on(async {
        // The fake never sends a heartbeat.
        let port = fake_script(|_| Vec::new()).await;
        let seen = Seen::default();
        let (live, _task) = LiveHandle::spawn(&cfg(port), seen.events());
        seen.wait(Duration::from_secs(3), |e| {
            matches!(e, LiveEvent::Connected(_)).then_some(())
        })
        .await;
        let connected = Instant::now();
        assert!(!live.snapshot().busy);
        seen.wait(Duration::from_secs(2), |e| {
            matches!(e, LiveEvent::Busy { busy: true }).then_some(())
        })
        .await;
        assert!(
            connected.elapsed() >= Duration::from_millis(280),
            "{:?}",
            connected.elapsed()
        );
        assert!(live.snapshot().busy);
    });
}

#[test]
fn a_port_that_answers_as_another_instance_is_never_used() {
    let _serial = serial();
    runtime().block_on(async {
        // The script on this port says it is `master`: its Config.py and
        // the hub's config disagree. No command may reach it.
        let reached = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&reached);
        let port = fake_script_as("master", move |_| {
            count.fetch_add(1, Ordering::SeqCst);
            Vec::new()
        })
        .await;
        let seen = Seen::default();
        let (live, _task) = LiveHandle::spawn(&cfg(port), seen.events());
        let deadline = Instant::now() + Duration::from_secs(3);
        while live.snapshot().connect_failures == 0 {
            assert!(Instant::now() < deadline, "{:?}", live.snapshot());
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let snap = live.snapshot();
        assert!(!snap.online);
        assert_eq!(
            snap.last_error.as_deref(),
            Some(format!("port {port} answers as instance \"master\", not \"band\"").as_str())
        );
        assert_eq!(live.call(get_name()).await, Err(LiveError::Offline));
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            seen.0.lock().unwrap().is_empty(),
            "no event: it never connected"
        );
        assert_eq!(reached.load(Ordering::SeqCst), 0, "no request reached it");
    });
}

#[test]
fn a_session_resets_the_failure_count() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let port = host.port;
        let seen = Seen::default();
        let (live, _task) = LiveHandle::spawn(&host.cfg(), seen.events());
        seen.wait(Duration::from_secs(3), |e| {
            matches!(e, LiveEvent::Connected(_)).then_some(())
        })
        .await;
        host.stop();
        let deadline = Instant::now() + Duration::from_secs(3);
        while live.snapshot().connect_failures < 2 {
            assert!(Instant::now() < deadline, "{:?}", live.snapshot());
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(live.snapshot().last_error.is_some());
        let host = Host::start_with("band", port, 0.0);
        seen.wait(Duration::from_secs(3), |e| {
            matches!(e, LiveEvent::Connected(_)).then_some(())
        })
        .await;
        let snap = live.snapshot();
        assert_eq!(snap.connect_failures, 0);
        assert_eq!(snap.last_error, None);
        host.stop();
    });
}
