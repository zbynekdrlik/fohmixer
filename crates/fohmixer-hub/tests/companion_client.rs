//! The Companion task (#52, `companion/client.rs`) against the scripted fake
//! Companion (`support/companion.rs`): the handshake to `ADD-DEVICE OK` and
//! Companion's key states, the pings, a press's round trip and its error, a
//! refused API and a refused `ADD-DEVICE`, 5 s of silence and the reconnect
//! with the next device id, a line over 256 KiB, and the stop's
//! `REMOVE-DEVICE`. Host-free: it also runs in the `windows` job.

mod support;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fohmixer_hub::companion::{Answer, CompanionEvent, CompanionHandle, Events, Press};
use fohmixer_hub::config::CompanionCfg;
use support::companion::{FakeCompanion, Script, image};
use support::{runtime, serial};

/// The task's events with when they came.
#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<(Instant, CompanionEvent)>>>);

impl Seen {
    fn events(&self) -> Events {
        let seen = self.clone();
        Arc::new(move |event: CompanionEvent| {
            seen.0.lock().unwrap().push((Instant::now(), event));
        })
    }

    fn all(&self) -> Vec<(Instant, CompanionEvent)> {
        self.0.lock().unwrap().clone()
    }

    /// The first event `pick` takes (seen earlier or within `limit`), with
    /// when it came; the events up to it are consumed.
    async fn wait<T>(
        &self,
        limit: Duration,
        pick: impl Fn(&CompanionEvent) -> Option<T>,
    ) -> (Instant, T) {
        let deadline = Instant::now() + limit;
        loop {
            {
                let mut seen = self.0.lock().unwrap();
                if let Some(i) = seen.iter().position(|(_, e)| pick(e).is_some()) {
                    let (at, event) = seen[i].clone();
                    seen.drain(..=i);
                    return (at, pick(&event).unwrap());
                }
            }
            assert!(
                Instant::now() < deadline,
                "no such event within {limit:?}: {:?}",
                self.all()
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}

fn deck(port: u16) -> CompanionCfg {
    CompanionCfg {
        host: "127.0.0.1".into(),
        port,
        columns: 8,
        rows: 4,
        bitmap_px: 72,
        title: "Stream Deck".into(),
    }
}

fn up(event: &CompanionEvent) -> Option<(String, String, u64, Option<f64>)> {
    match event {
        CompanionEvent::Up {
            companion,
            api,
            attempts,
            down_ms,
        } => Some((companion.clone(), api.clone(), *attempts, *down_ms)),
        _ => None,
    }
}

fn answered(event: &CompanionEvent) -> Option<Answer> {
    match event {
        CompanionEvent::Answered(answer) => Some(answer.clone()),
        _ => None,
    }
}

const ADD_DEVICE_1: &str = "ADD-DEVICE DEVICEID=\"fohmixer-1\" SERIAL=\"fohmixer\" PRODUCT_NAME=\"fohmixer\" \
    KEYS_TOTAL=32 KEYS_PER_ROW=8 BITMAPS=72 BITMAP_FORMAT=webp COLORS=hex TEXT=0 BRIGHTNESS=0";

#[test]
fn the_handshake_registers_one_surface_and_passes_companions_keys_on() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let seen = Seen::default();
        let (handle, _task) = CompanionHandle::spawn(&deck(fake.port), seen.events());
        let (_, registered) = seen.wait(Duration::from_secs(5), up).await;
        assert_eq!(
            registered,
            ("5.0.7+fake".to_string(), "1.12.0".to_string(), 1, None)
        );
        assert_eq!(fake.lines_of(1)[0], ADD_DEVICE_1);
        let (_, last) = seen
            .wait(Duration::from_secs(5), |e| match e {
                CompanionEvent::Key(k) if k.key == 31 => Some(k.clone()),
                _ => None,
            })
            .await;
        assert_eq!(last.img.as_deref(), Some(image(31, false).as_str()));
        assert_eq!(
            (last.color.as_deref(), last.pressed),
            (Some("#000000"), Some(false))
        );
        let status = handle.status();
        assert!(status.online, "{status:?}");
        assert_eq!(
            (
                status.companion_version.as_deref(),
                status.api_version.as_deref(),
                status.keys
            ),
            (Some("5.0.7+fake"), Some("1.12.0"), 32)
        );
        assert_eq!((status.connect_failures, status.last_error), (0, None));
        // KEYS-CLEAR goes on; BRIGHTNESS (sent after ADD-DEVICE OK) was
        // ignored: no event of it.
        fake.send("KEYS-CLEAR DEVICEID=\"fohmixer-1\"");
        seen.wait(Duration::from_secs(2), |e| {
            matches!(e, CompanionEvent::Clear).then_some(())
        })
        .await;
    });
}

#[test]
fn the_hub_pings_every_two_seconds_and_answers_companions_ping() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let seen = Seen::default();
        let (_handle, _task) = CompanionHandle::spawn(&deck(fake.port), seen.events());
        seen.wait(Duration::from_secs(5), up).await;
        let got = fake
            .until(Duration::from_secs(6), "two pings", |g| {
                g.iter().any(|l| l.line == "PING 2")
            })
            .await;
        let at = |text: &str| got.iter().find(|l| l.line == text).unwrap().at;
        let gap = at("PING 2") - at("PING 1");
        assert!(
            gap >= Duration::from_millis(1700) && gap <= Duration::from_millis(2300),
            "{gap:?}"
        );
        fake.send("PING abc");
        fake.until(Duration::from_secs(2), "the pong", |g| {
            g.iter().any(|l| l.line == "PONG abc")
        })
        .await;
    });
}

#[test]
fn a_forwarded_press_is_answered_with_companions_round_trip() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let seen = Seen::default();
        let (handle, _task) = CompanionHandle::spawn(&deck(fake.port), seen.events());
        seen.wait(Duration::from_secs(5), up).await;
        let down = Press {
            key: 3,
            down: true,
            from: Some((7, 1)),
        };
        handle.press(down);
        let (_, answer) = seen.wait(Duration::from_secs(2), answered).await;
        assert_eq!(
            (answer.press, answer.ok, answer.error.clone()),
            (down, true, None)
        );
        let rtt = answer.rtt_ms.unwrap();
        assert!((0.0..1000.0).contains(&rtt), "{rtt}");
        assert!(
            fake.lines_of(1)
                .contains(&"KEY-PRESS DEVICEID=\"fohmixer-1\" KEY=3 PRESSED=1".to_string())
        );
        // The fake's new state of the key follows its OK.
        seen.wait(Duration::from_secs(2), |e| match e {
            CompanionEvent::Key(k) if k.key == 3 && k.pressed == Some(true) => Some(()),
            _ => None,
        })
        .await;
        // A press Companion refuses: an answer the fake writes by hand,
        // after a bare ERROR that answers no press.
        fake.set_script(Script {
            api: "1.12.0".into(),
            refuse_add: None,
            answers: false,
        });
        fake.close();
        seen.wait(Duration::from_secs(5), |e| {
            matches!(e, CompanionEvent::Down { .. }).then_some(())
        })
        .await;
        seen.wait(Duration::from_secs(5), up).await;
        let refused = Press {
            key: 40,
            down: true,
            from: Some((7, 2)),
        };
        handle.press(refused);
        fake.until(Duration::from_secs(2), "the refused press", |g| {
            g.iter().any(|l| l.line.contains("KEY=40"))
        })
        .await;
        fake.send("ERROR MESSAGE=\"Unknown command: FOO\"");
        fake.send("KEY-PRESS ERROR DEVICEID=\"fohmixer-2\" MESSAGE=\"Invalid KEY\"");
        let (_, answer) = seen.wait(Duration::from_secs(2), answered).await;
        assert_eq!(
            (answer.press, answer.ok, answer.error.as_deref()),
            (refused, false, Some("Invalid KEY"))
        );
    });
}

#[test]
fn an_api_before_1_12_and_a_refused_add_device_are_failed_attempts() {
    let _serial = serial();
    runtime().block_on(async {
        let old = FakeCompanion::start(Script {
            api: "1.11.0".into(),
            refuse_add: None,
            answers: true,
        })
        .await;
        let seen = Seen::default();
        let (handle, task) = CompanionHandle::spawn(&deck(old.port), seen.events());
        let (_, failed) = seen
            .wait(Duration::from_secs(5), |e| match e {
                CompanionEvent::Failed {
                    error,
                    refused,
                    api,
                    attempts,
                    ..
                } => Some((error.clone(), *refused, api.clone(), *attempts)),
                _ => None,
            })
            .await;
        assert!(failed.0.contains("1.11.0"), "{failed:?}");
        assert_eq!(
            (failed.1, failed.2.as_deref(), failed.3),
            (true, Some("1.11.0"), 1)
        );
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(
            old.got().iter().all(|g| !g.line.starts_with("ADD-DEVICE")),
            "never registered"
        );
        let status = handle.status();
        assert!(!status.online);
        assert!(status.connect_failures >= 2, "{status:?}");
        assert!(status.last_error.unwrap().contains("1.11.0"));
        // Only the outage's first failure is an event.
        let failures = seen
            .all()
            .iter()
            .filter(|(_, e)| matches!(e, CompanionEvent::Failed { .. }))
            .count();
        assert_eq!(failures, 0, "the first was consumed; no other came");
        handle.stop();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();

        let refusing = FakeCompanion::start(Script {
            api: "1.12.0".into(),
            refuse_add: Some("test refusal".into()),
            answers: true,
        })
        .await;
        let seen = Seen::default();
        let (handle, _task) = CompanionHandle::spawn(&deck(refusing.port), seen.events());
        let (_, error) = seen
            .wait(Duration::from_secs(5), |e| match e {
                CompanionEvent::Failed {
                    error,
                    refused: true,
                    ..
                } => Some(error.clone()),
                _ => None,
            })
            .await;
        assert_eq!(error, "ADD-DEVICE refused: test refusal");
        assert!(!handle.status().online);
    });
}

#[test]
fn companion_silent_for_five_seconds_is_lost_and_the_next_device_id_registers() {
    let _serial = serial();
    runtime().block_on(async {
        // Companion goes silent after the handshake: no PONG, no answers.
        let fake = FakeCompanion::start(Script {
            api: "1.12.0".into(),
            refuse_add: None,
            answers: false,
        })
        .await;
        let seen = Seen::default();
        let (handle, _task) = CompanionHandle::spawn(&deck(fake.port), seen.events());
        seen.wait(Duration::from_secs(5), up).await;
        let (heard, _) = seen
            .wait(Duration::from_secs(2), |e| match e {
                CompanionEvent::Key(k) if k.key == 31 => Some(()),
                _ => None,
            })
            .await;
        let press = Press {
            key: 2,
            down: true,
            from: Some((7, 1)),
        };
        handle.press(press);
        // The press waiting for its answer goes offline with the link.
        let (lost_answer_at, offline) = seen.wait(Duration::from_secs(8), answered).await;
        assert_eq!(offline, Answer::offline(press));
        let (down_at, error) = seen
            .wait(Duration::from_secs(2), |e| match e {
                CompanionEvent::Down { error } => Some(error.clone()),
                _ => None,
            })
            .await;
        assert_eq!(error, "nothing from Companion for 5 s");
        assert!(lost_answer_at <= down_at);
        let silence = down_at - heard;
        assert!(
            silence >= Duration::from_secs(5) && silence < Duration::from_millis(6500),
            "{silence:?}"
        );
        // The pings went out meanwhile.
        assert!(fake.lines_of(1).iter().any(|l| l == "PING 2"));
        // The reconnect registers the next device id after the backoff.
        let (_, again) = seen.wait(Duration::from_secs(5), up).await;
        assert_eq!(again.2, 1);
        assert!(
            again.3.unwrap() >= 200.0,
            "down for the backoff at least: {again:?}"
        );
        assert!(
            fake.lines_of(2)[0]
                .starts_with("ADD-DEVICE DEVICEID=\"fohmixer-2\" SERIAL=\"fohmixer\"")
        );
    });
}

#[test]
fn a_line_over_256_kib_ends_the_link() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let seen = Seen::default();
        let (_handle, _task) = CompanionHandle::spawn(&deck(fake.port), seen.events());
        seen.wait(Duration::from_secs(5), up).await;
        fake.send(&format!(
            "KEY-STATE DEVICEID=\"fohmixer-1\" KEY=0 BITMAP=\"data:image/webp;base64,{}\"",
            "A".repeat(256 * 1024)
        ));
        let (_, error) = seen
            .wait(Duration::from_secs(5), |e| match e {
                CompanionEvent::Down { error } => Some(error.clone()),
                _ => None,
            })
            .await;
        assert_eq!(error, "a line over 256 KiB");
    });
}

#[test]
fn a_stop_writes_what_came_before_it_then_removes_the_device() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let seen = Seen::default();
        let (handle, task) = CompanionHandle::spawn(&deck(fake.port), seen.events());
        seen.wait(Duration::from_secs(5), up).await;
        handle.press(Press {
            key: 4,
            down: false,
            from: None,
        });
        handle.stop();
        tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("the task ends within the stop's bound")
            .unwrap();
        let lines = fake
            .until(Duration::from_secs(2), "REMOVE-DEVICE", |g| {
                g.iter().any(|l| l.line.starts_with("REMOVE-DEVICE"))
            })
            .await;
        // The last lines of the connection, a ping that may fall between
        // them left out: the release, then REMOVE-DEVICE.
        let lines: Vec<&str> = lines
            .iter()
            .map(|g| g.line.as_str())
            .filter(|l| !l.starts_with("PING "))
            .collect();
        assert_eq!(
            lines[lines.len() - 2..],
            [
                "KEY-PRESS DEVICEID=\"fohmixer-1\" KEY=4 PRESSED=0",
                "REMOVE-DEVICE DEVICEID=\"fohmixer-1\"",
            ]
        );
    });
}
