//! The hub between clients and real FohMixer scripts on SimLive
//! (`sim/host.py`): pass-through, one Live listener per key, the fan-out to
//! every subscriber, resync after a host restart (in order: offline, online,
//! the fresh value), renamed bindings, busy, and a client that stops
//! reading holds up nobody and is closed (S3 plan, Task 3).
#![cfg(unix)]

mod support;

use std::time::{Duration, Instant};

use fohmixer_proto::client::{ClientMsg, ServerMsg, UI_PROTO};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use support::{Client, Host, TestHub, runtime, serial};
use tokio_tungstenite::tungstenite::Message;

const VOLUME: &str = "live_set tracks[name=Hand1 #] mixer_device volume";
const SECS_3: Duration = Duration::from_secs(3);

#[test]
fn two_clients_share_one_live_listener_and_both_get_every_change() {
    let _serial = serial();
    runtime().block_on(async {
        let mut host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut a = hub.client().await;
        let mut b = hub.client().await;
        a.instance_state("band", true, Some(false), SECS_3).await;
        b.instance_state("band", true, Some(false), SECS_3).await;
        let key = a.sub_key("band", VOLUME, "value", true).await;
        let first = a.value_of(&key, SECS_3).await;
        assert_eq!(first.value, Some(json!(0.85)));
        assert_eq!(first.display.as_deref(), Some("0.00 dB"));
        // The second subscriber gets the cached value at once, in `subbed`.
        match b.sub("band", VOLUME, "value", true).await {
            ServerMsg::Subbed {
                sub,
                value,
                display,
                error,
            } => {
                assert_eq!(sub, key);
                assert_eq!(value, Some(json!(0.85)));
                assert_eq!(display.as_deref(), Some("0.00 dB"));
                assert_eq!(error, None);
            }
            other => panic!("{other:?}"),
        }
        let status = hub.status().await;
        assert_eq!(status.instances[0].subscriptions, 1, "{status:?}");
        assert_eq!(status.clients, 2);
        // One Live listener. (The hub sends one add_listener per key — its
        // table's unit tests prove that; the script would also dedupe a
        // second one of the same connection, so this probe shows the end
        // result, not the hub's dedupe.)
        assert_eq!(host.listeners("value", VOLUME), 1, "one Live listener");
        // A sets the volume; B sees it with Live's display (one sample,
        // bounded at 500 ms; the typical time is the next test's median).
        let sent = Instant::now();
        a.set("band", VOLUME, "value", json!(0.5)).await;
        let seen = b
            .value_until(&key, SECS_3, |i| i.value == Some(json!(0.5)))
            .await;
        assert!(
            sent.elapsed() < Duration::from_millis(500),
            "B saw the change after {:?}",
            sent.elapsed()
        );
        assert_eq!(seen.display.as_deref(), Some("-14.0 dB"));
        a.value_until(&key, SECS_3, |i| i.value == Some(json!(0.5)))
            .await;
        // Both leave: the listener goes.
        a.send(&ClientMsg::Unsub { sub: key.clone() }).await;
        b.send(&ClientMsg::Unsub { sub: key.clone() }).await;
        hub.status_until(SECS_3, |s| {
            s.instances[0].subscriptions == 0 && s.instances[0].listeners == 0
        })
        .await;
        let deadline = Instant::now() + SECS_3;
        while host.listeners("value", VOLUME) != 0 {
            assert!(Instant::now() < deadline, "the Live listener stayed");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn b_sees_a_change_from_a_within_a_hundred_milliseconds_typically() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut a = hub.client().await;
        let mut b = hub.client().await;
        let key = b.sub_key("band", VOLUME, "value", true).await;
        b.value_of(&key, SECS_3).await;
        let mut delays = Vec::new();
        for i in 0..10 {
            // Sixteenths: exact in binary and in their shortest decimal text.
            let value = f64::from(i + 3) / 16.0;
            let sent = Instant::now();
            a.set("band", VOLUME, "value", json!(value)).await;
            b.value_until(&key, SECS_3, |item| item.value == Some(json!(value)))
                .await;
            delays.push(sent.elapsed());
        }
        delays.sort();
        assert!(
            delays[5] < Duration::from_millis(100),
            "median {:?} (all {delays:?})",
            delays[5]
        );
        hub.stop().await;
        host.stop();
    });
}

/// The median delay (10 samples) from A's set to B's value; the values are
/// `(i + first) / 64`, exact in binary and in their shortest text.
async fn median_delay(a: &mut Client, b: &mut Client, key: &str, first: u32) -> Duration {
    let mut delays = Vec::new();
    for i in 0..10 {
        let value = f64::from(i + first) / 64.0;
        let sent = Instant::now();
        a.set("band", VOLUME, "value", json!(value)).await;
        b.value_until(key, SECS_3, |item| item.value == Some(json!(value)))
            .await;
        delays.push(sent.elapsed());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    delays.sort();
    delays[5]
}

#[test]
fn a_client_that_never_reads_delays_nobody_and_is_closed() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut a = hub.client().await;
        let mut b = hub.client().await;
        let key = b.sub_key("band", VOLUME, "value", true).await;
        b.value_of(&key, SECS_3).await;
        let baseline = median_delay(&mut a, &mut b, &key, 3).await;
        // A client with a tiny receive window that never reads. Its replies
        // are a few hundred kB each (the key and the error both name its bad
        // prop): megabytes more than the kernel buffers hold, so the hub's
        // writer for it blocks at once.
        let socket = tokio::net::TcpSocket::new_v4().unwrap();
        socket.set_recv_buffer_size(4096).unwrap();
        let tcp = socket.connect(hub.addr).await.unwrap();
        let (mut stuck, _) = tokio_tungstenite::client_async(hub.ws_url(), tcp)
            .await
            .unwrap();
        let hello = tokio::time::timeout(SECS_3, stuck.next()).await.unwrap();
        assert!(matches!(hello, Some(Ok(Message::Text(_)))), "{hello:?}");
        hub.status_until(SECS_3, |s| s.clients == 3).await;
        let bad = serde_json::to_string(&ClientMsg::Sub {
            instance: "band".into(),
            target: "live_set".into(),
            prop: format!("_{}", "x".repeat(200_000)),
            display: false,
        })
        .unwrap();
        let burst = Instant::now();
        for _ in 0..40 {
            stuck.send(Message::Text(bad.clone().into())).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        // The others: no more than 50 ms added (S3 design note §8).
        let with_stuck = median_delay(&mut a, &mut b, &key, 20).await;
        assert!(
            with_stuck < baseline + Duration::from_millis(50),
            "median {with_stuck:?} with a stuck client, {baseline:?} before"
        );
        // The stuck client is closed once one message waited SEND_TIMEOUT
        // (5 s) for it — not earlier: its writer really blocked.
        hub.status_until(Duration::from_secs(15), |s| s.clients == 2)
            .await;
        let closed_after = burst.elapsed();
        assert!(
            closed_after >= Duration::from_millis(4900),
            "closed after {closed_after:?}"
        );
        // The others are still served.
        a.set("band", VOLUME, "value", json!(0.75)).await;
        b.value_until(&key, SECS_3, |i| i.value == Some(json!(0.75)))
            .await;
        drop(stuck);
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_host_restart_goes_offline_online_and_resubscribes_once() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let port = host.port;
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut a = hub.client().await;
        let key = a.sub_key("band", VOLUME, "value", true).await;
        a.value_of(&key, SECS_3).await;
        a.set("band", VOLUME, "value", json!(0.5)).await;
        a.value_until(&key, SECS_3, |i| i.value == Some(json!(0.5)))
            .await;
        host.stop();
        a.instance_state("band", false, None, SECS_3).await;
        // Only what comes after the offline state counts from here.
        a.clear();
        let offline = hub.status().await;
        assert!(!offline.instances[0].online);
        assert_eq!(offline.instances[0].listeners, 0);
        assert_eq!(
            a.cmd(
                "band",
                vec![json!({"target": "live_set", "name": "get_prop", "args": {"prop": "tempo"}})]
            )
            .await,
            Err("instance offline".to_string())
        );
        let mut host = Host::start_with("band", port, 0.0);
        // In arrival order: online, then the fresh session's own value —
        // never a value of the old session (0.5).
        let mut online = false;
        let deadline = Instant::now() + SECS_3;
        let fresh = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match a.next_message(left).await.expect("online, then the value") {
                ServerMsg::Instance {
                    name, online: true, ..
                } if name == "band" => online = true,
                ServerMsg::Values { items } => {
                    if let Some(item) = items.into_iter().find(|i| i.sub == key) {
                        assert!(online, "a value before the online state: {item:?}");
                        break item;
                    }
                }
                _ => {}
            }
        };
        assert_eq!(fresh.value, Some(json!(0.85)));
        assert_eq!(fresh.display.as_deref(), Some("0.00 dB"));
        let status = hub
            .status_until(SECS_3, |s| {
                s.instances[0].online && s.instances[0].listeners == 3
            })
            .await;
        assert_eq!(status.instances[0].subscriptions, 1);
        assert_eq!(status.instances[0].set_name, "Test Site");
        assert_eq!(
            host.listeners("value", VOLUME),
            1,
            "one listener after the restart"
        );
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_renamed_track_gives_its_subscriber_an_error_and_a_rename_back_heals_it() {
    let _serial = serial();
    runtime().block_on(async {
        let mut host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut a = hub.client().await;
        let key = a.sub_key("band", VOLUME, "value", true).await;
        a.value_of(&key, SECS_3).await;
        assert_eq!(host.rename("Hand1 #", "Hand9 #"), 1);
        let error = a.value_until(&key, SECS_3, |i| i.error.is_some()).await;
        assert_eq!(
            error.error.as_deref(),
            Some("not found: tracks[name=Hand1 #]")
        );
        assert_eq!(error.value, None);
        // The stale listener is removed; a change of the renamed track is
        // not delivered as this binding's value.
        let deadline = Instant::now() + SECS_3;
        while host.listeners("value", "live_set tracks 0 mixer_device volume") != 0 {
            assert!(Instant::now() < deadline, "the stale listener stayed");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        a.set(
            "band",
            "live_set tracks 0 mixer_device volume",
            "value",
            json!(0.4),
        )
        .await;
        assert!(
            a.gets(Duration::from_millis(400), |m| match m {
                ServerMsg::Values { items } => items
                    .iter()
                    .find(|i| i.sub == key && i.value.is_some())
                    .cloned(),
                _ => None,
            })
            .await
            .is_none(),
            "no stale value"
        );
        assert_eq!(host.rename("Hand9 #", "Hand1 #"), 1);
        let healed = a
            .value_until(&key, SECS_3, |i| i.value == Some(json!(0.4)))
            .await;
        assert_eq!(healed.error, None);
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_subscription_made_while_its_track_is_missing_heals_when_the_track_is_renamed_back() {
    let _serial = serial();
    runtime().block_on(async {
        let mut host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        assert_eq!(host.rename("Hand1 #", "Hand9 #"), 1);
        let mut a = hub.client().await;
        let key = a.sub_key("band", VOLUME, "value", true).await;
        let error = a.value_until(&key, SECS_3, |i| i.error.is_some()).await;
        assert_eq!(
            error.error.as_deref(),
            Some("not found: tracks[name=Hand1 #]")
        );
        // No list changes, only a rename: the hub watches every track's
        // name while the binding is in error (#58), so it heals by itself.
        assert_eq!(host.rename("Hand9 #", "Hand1 #"), 1);
        let healed = a.value_until(&key, SECS_3, |i| i.value.is_some()).await;
        assert_eq!(healed.error, None);
        assert_eq!(healed.display.as_deref(), Some("0.00 dB"));
        // Healed: the watches go, the binding's three listeners stay.
        let status = hub
            .status_until(SECS_3, |s| s.instances[0].listeners == 3)
            .await;
        assert_eq!(status.instances[0].subscriptions, 1);
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_stall_shows_the_instance_busy_then_free() {
    let _serial = serial();
    runtime().block_on(async {
        let mut host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut a = hub.client().await;
        a.instance_state("band", true, Some(false), SECS_3).await;
        host.stall(700);
        a.instance_state("band", true, Some(true), SECS_3).await;
        // Live's health goes out as `link` while busy (#43), the tick's age
        // growing.
        let age = a
            .wait(SECS_3, |m| match m {
                ServerMsg::Link {
                    instance,
                    tick_age_ms,
                    busy: true,
                } if instance == "band" && *tick_age_ms >= 300.0 => Some(*tick_age_ms),
                _ => None,
            })
            .await;
        assert!(age < 5_000.0, "{age}");
        a.instance_state("band", true, Some(false), SECS_3).await;
        // Both changes are in the event log with their reason.
        let records = hub
            .events_until(SECS_3, |r| {
                r.iter()
                    .filter(|x| x["ev"] == "link" && x["instance"] == "band")
                    .count()
                    >= 2
            })
            .await;
        let links: Vec<&serde_json::Value> = records.iter().filter(|x| x["ev"] == "link").collect();
        assert_eq!(links[0]["busy"], json!(true));
        assert!(links[0]["tick_age_ms"].as_f64().unwrap() > 0.0);
        assert!(links[0]["reason"].is_string());
        assert_eq!(links[1]["busy"], json!(false));
        hub.stop().await;
        host.stop();
    });
}

/// The volume of the load test's first strip.
const VOLUME_1: &str = "live_set tracks[name=Strip 1 #] mixer_device volume";

/// The subscriptions of one strip of the load test: volume with its
/// display, pan, mute and both meters.
fn load_strip_subs(n: usize) -> Vec<(String, &'static str, bool)> {
    let track = format!("live_set tracks[name=Strip {n} #]");
    vec![
        (format!("{track} mixer_device volume"), "value", true),
        (format!("{track} mixer_device panning"), "value", false),
        (track.clone(), "mute", false),
        (track.clone(), "output_meter_left", false),
        (track, "output_meter_right", false),
    ]
}

/// Reads both clients for `span`; the `busy: true` states they saw.
async fn busy_states(clients: &mut [Client], span: Duration) -> Vec<String> {
    let mut busy = Vec::new();
    let deadline = Instant::now() + span;
    while Instant::now() < deadline {
        // Up to 200 messages from each in turn: both keep reading (the hub
        // closes a client that takes no message for 5 s).
        for client in clients.iter_mut() {
            for _ in 0..200 {
                let Some(msg) = client.next_message(Duration::from_millis(2)).await else {
                    break;
                };
                if let ServerMsg::Instance {
                    name, busy: true, ..
                } = msg
                {
                    busy.push(format!(
                        "{name}, {:?} before the end of a read",
                        deadline.saturating_duration_since(Instant::now())
                    ));
                }
            }
        }
    }
    busy
}

#[test]
fn many_metered_strips_and_two_resubscribing_clients_never_show_busy() {
    // #9: on the PC the busy badge flapped after every client connect while
    // Live's main thread ticked every 31–47 ms. Here 60 strips move their
    // meters 30 times a second, two clients subscribe to all of them and
    // resubscribe three times (unsubscribe everything, subscribe again, as
    // a page switch or a reconnect does). SimLive's main thread is never
    // late, so no instance state may say busy.
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let site = dir.path().join("load-site.json");
        let tracks: Vec<serde_json::Value> = (1..=60)
            .map(|n| json!({"name": format!("Strip {n} #")}))
            .collect();
        std::fs::write(&site, json!({"name": "Load", "tracks": tracks}).to_string()).unwrap();
        let host = Host::start_site("band", &site, 0, 30.0);
        let hub = TestHub::start(vec![host.cfg()], &dir.path().join("hub")).await;
        let mut clients = vec![hub.client().await, hub.client().await];
        for client in &mut clients {
            client
                .instance_state("band", true, Some(false), SECS_3)
                .await;
        }
        let subs: Vec<(String, &str, bool)> = (1..=60).flat_map(load_strip_subs).collect();
        let mut busy = Vec::new();
        for _round in 0..3 {
            for client in &mut clients {
                for (target, prop, display) in &subs {
                    client
                        .send(&ClientMsg::Sub {
                            instance: "band".into(),
                            target: target.clone(),
                            prop: (*prop).to_string(),
                            display: *display,
                        })
                        .await;
                }
            }
            busy.extend(busy_states(&mut clients, Duration::from_millis(1500)).await);
            for client in &mut clients {
                for (target, prop, display) in &subs {
                    let sub = fohmixer_proto::client::hub_key("band", target, prop, *display);
                    client.send(&ClientMsg::Unsub { sub }).await;
                }
            }
        }
        busy.extend(busy_states(&mut clients, Duration::from_millis(500)).await);
        assert!(busy.is_empty(), "busy while Live was fine: {busy:?}");
        // Both clients were served the whole time (neither was closed).
        for client in &mut clients {
            client.sub("band", VOLUME_1, "value", true).await;
        }
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn commands_pass_through_and_listener_commands_are_refused() {
    let _serial = serial();
    runtime().block_on(async {
        let band = Host::start("band");
        let master = Host::start("master");
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![band.cfg(), master.cfg()], dir.path()).await;
        let mut a = hub.client().await;
        a.instance_state("master", true, None, SECS_3).await;
        a.instance_state("band", true, None, SECS_3).await;
        let slots = a
            .cmd(
                "master",
                vec![
                    json!({"target": "live_set tracks 0", "name": "get_prop", "args": {"prop": "name"}}),
                    json!({"target": "live_set tracks[name=Nobody]", "name": "get_prop", "args": {"prop": "name"}}),
                ],
            )
            .await
            .unwrap();
        assert_eq!(slots[0], json!({"ok": true, "data": "Hand1 #"}));
        assert_eq!(slots[1]["ok"], json!(false));
        assert_eq!(
            a.cmd("drums", vec![json!({"target": "live_set", "name": "stop_playing"})])
                .await,
            Err("unknown instance \"drums\"".to_string())
        );
        assert_eq!(
            a.cmd(
                "band",
                vec![json!({"target": "live_set", "name": "add_listener", "args": {"prop": "tempo"}})]
            )
            .await,
            Err("add_listener goes through sub/unsub".to_string())
        );
        // Two instances, one hub: the same path on each is its own.
        a.set("band", "live_set", "tempo", json!(100.0)).await;
        assert_eq!(a.get("master", "live_set", "tempo").await, json!(120.0));
        assert_eq!(a.get("band", "live_set", "tempo").await, json!(100.0));
        hub.stop().await;
        band.stop();
        master.stop();
    });
}

#[test]
fn bad_requests_get_errors_and_the_connection_stays() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut a = hub.client().await;
        match a
            .sub("band", "live_set tracks[name=Hand1 #", "mute", false)
            .await
        {
            ServerMsg::Subbed { error, value, .. } => {
                assert_eq!(error.as_deref(), Some("syntax: unterminated [name="));
                assert_eq!(value, None);
            }
            other => panic!("{other:?}"),
        }
        match a.sub("drums", "live_set", "is_playing", false).await {
            ServerMsg::Subbed { error, .. } => {
                assert_eq!(error.as_deref(), Some("unknown instance \"drums\""));
            }
            other => panic!("{other:?}"),
        }
        a.send_text("{not json").await;
        let message = a
            .wait(SECS_3, |m| match m {
                ServerMsg::Error { id: None, message } => Some(message.clone()),
                _ => None,
            })
            .await;
        assert!(message.starts_with("unreadable message: "), "{message}");
        a.send(&ClientMsg::SetHub {
            key: "tempo".into(),
            value: json!(1),
        })
        .await;
        let message = a
            .wait(SECS_3, |m| match m {
                ServerMsg::Error { id: None, message } => Some(message.clone()),
                _ => None,
            })
            .await;
        assert_eq!(message, "no hub value \"tempo\" taking 1");
        a.send(&ClientMsg::SetHub {
            key: "stage_aut".into(),
            value: json!("yes"),
        })
        .await;
        a.wait(SECS_3, |m| {
            matches!(m, ServerMsg::Error { id: None, message } if message.contains("stage_aut"))
                .then_some(())
        })
        .await;
        // An unknown subscription key is a no-op; the connection still serves.
        a.send(&ClientMsg::Unsub {
            sub: "nothing".into(),
        })
        .await;
        let is_playing = a.sub_key("band", "live_set", "is_playing", false).await;
        let item = a.value_of(&is_playing, SECS_3).await;
        assert_eq!(item.value, Some(json!(false)));
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn the_handshake_is_hello_and_a_protocol_mismatch_closes_with_4001() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![], dir.path()).await;
        let a = hub.client().await;
        match &a.hello {
            ServerMsg::Hello {
                proto,
                build,
                min_client_proto,
            } => {
                assert_eq!(*proto, UI_PROTO);
                assert_eq!(build, fohmixer_proto::VERSION);
                assert_eq!(*min_client_proto, 2);
            }
            other => panic!("{other:?}"),
        }
        // The first messages: every instance's state (none here), STAGE AUT.
        let mut a = a;
        a.wait(SECS_3, |m| {
            matches!(m, ServerMsg::Hub { key, value } if key == "stage_aut" && *value == json!(false))
                .then_some(())
        })
        .await;
        // Protocol 1 (before #43) and a future 3 are not served: a hello,
        // then the reload code.
        for (query, hello) in [("proto=1", true), ("proto=3", true), ("", false)] {
            let url = format!("ws://{}/ws?token={}&{query}", hub.addr, hub.token);
            let (mut ws, _) = tokio_tungstenite::connect_async(url.as_str()).await.unwrap();
            let mut got_hello = false;
            let code = loop {
                match tokio::time::timeout(SECS_3, ws.next()).await.unwrap() {
                    Some(Ok(Message::Text(text))) => {
                        assert!(text.as_str().contains("\"hello\""), "{text}");
                        got_hello = true;
                    }
                    Some(Ok(Message::Close(Some(frame)))) => break u16::from(frame.code),
                    other => panic!("{other:?}"),
                }
            };
            assert_eq!(code, 4001, "{query}");
            assert_eq!(got_hello, hello, "{query}");
        }
        // Without a valid token there is no WebSocket.
        for token in ["", "not.a.token"] {
            let url = format!("ws://{}/ws?token={token}&proto=2", hub.addr);
            match tokio_tungstenite::connect_async(url.as_str()).await {
                Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                    assert_eq!(response.status(), 401);
                }
                other => panic!("{token:?}: {:?}", other.map(|_| ())),
            }
        }
        hub.stop().await;
    });
}

#[test]
fn a_stopping_hub_closes_its_clients() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![], dir.path()).await;
        let a = hub.client().await;
        let started = Instant::now();
        hub.stop().await;
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
        let mut ws = a.into_socket();
        let deadline = Instant::now() + SECS_3;
        loop {
            match tokio::time::timeout(SECS_3, ws.next()).await.unwrap() {
                Some(Ok(Message::Text(_))) => {}
                _ => break,
            }
            assert!(Instant::now() < deadline);
        }
    });
}
