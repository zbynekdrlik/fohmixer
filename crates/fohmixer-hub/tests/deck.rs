//! The Stream Deck through the hub (#52, spec §5): the `deck` message on
//! attach and on Companion's link changes, the keys only to viewers,
//! KEYS-CLEAR, a press's round trip and its records, two fingers on one
//! key, the hub's own releases (a closed page socket, a holding page silent
//! for 2 s, a key held when Companion was lost, the stop), a key held when
//! Companion was lost and the hub stopped before the link came back (logged
//! `lost`, never sent), a press waiting when the link goes, presses while
//! Companion is away, and a hub without `[companion]`. Against the scripted
//! fake Companion (`support/companion.rs`); host-free: it also runs in the
//! `windows` job.

mod support;

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use fohmixer_hub::config::{CompanionCfg, Config};
use fohmixer_hub::deck::img_hash;
use fohmixer_proto::client::{ClientMsg, DeckKey, ServerMsg};
use serde_json::{Value, json};
use support::companion::{FakeCompanion, Got, Script, image, key_state};
use support::{Client, TestHub, events_in, runtime, serial};

const WAIT: Duration = Duration::from_secs(5);

fn deck_config(dir: &std::path::Path, port: u16) -> Config {
    let mut config = Config::defaults(dir);
    config.instances.clear();
    config.companion = Some(CompanionCfg {
        host: "127.0.0.1".into(),
        port,
        columns: 8,
        rows: 4,
        bitmap_px: 72,
        title: "Stream Deck".into(),
    });
    config
}

/// Waits for the `deck` message saying `online`.
async fn deck_online(client: &mut Client, online: bool) {
    client
        .wait(Duration::from_secs(8), |m| match m {
            ServerMsg::Deck {
                online: now,
                columns: 8,
                rows: 4,
                title,
            } if *now == online && title == "Stream Deck" => Some(()),
            _ => None,
        })
        .await;
}

/// A press of `key` (an up with the page's 150 ms hold, why `up`).
async fn press(client: &mut Client, key: u32, down: bool, seq: u64) {
    client
        .send(&ClientMsg::DeckPress {
            key,
            down,
            seq,
            t: 1_000.0 + seq as f64,
            hold_ms: (!down).then_some(150.0),
            why: (!down).then(|| "up".to_string()),
        })
        .await;
}

/// The `deck_ack` of `seq`: ok, error, round trip.
async fn ack(client: &mut Client, seq: u64) -> (bool, Option<String>, Option<f64>) {
    client
        .wait(WAIT, move |m| match m {
            ServerMsg::DeckAck {
                seq: s,
                ok,
                error,
                rtt_ms,
            } if *s == seq => Some((*ok, error.clone(), *rtt_ms)),
            _ => None,
        })
        .await
}

/// The keys of the `deck_keys` messages until `enough` holds.
async fn keys_until(
    client: &mut Client,
    enough: impl Fn(&BTreeMap<u32, DeckKey>) -> bool,
) -> BTreeMap<u32, DeckKey> {
    let deadline = Instant::now() + WAIT;
    let mut keys = BTreeMap::new();
    while !enough(&keys) {
        let left = deadline.saturating_duration_since(Instant::now());
        let items = client
            .wait(left, |m| match m {
                ServerMsg::DeckKeys { items } => Some(items.clone()),
                _ => None,
            })
            .await;
        for item in items {
            keys.insert(item.key, item);
        }
    }
    keys
}

/// The press lines of `key` the fake got: (connection, down).
fn presses_in(got: &[Got], key: u32) -> Vec<(usize, bool)> {
    let field = format!(" KEY={key} PRESSED=");
    got.iter()
        .filter(|g| g.line.starts_with("KEY-PRESS "))
        .filter_map(|g| g.line.split_once(&field).map(|(_, v)| (g.conn, v == "1")))
        .collect()
}

fn of<'a>(records: &'a [Value], ev: &str) -> Vec<&'a Value> {
    records.iter().filter(|r| r["ev"] == ev).collect()
}

/// A line the fake writes as Companion: the key's state, its terminator off.
fn state_line(key: u32, pressed: bool) -> String {
    key_state("fohmixer-1", key, pressed).trim_end().to_string()
}

/// The event log of a stopped hub in `dir`, once it holds a `deck_release`
/// (the writer thread may still be flushing).
async fn stopped_log_with_a_release(dir: &std::path::Path) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let records = events_in(dir);
        if records.iter().any(|r| r["ev"] == "deck_release") {
            return records;
        }
        assert!(Instant::now() < deadline, "no deck_release: {records:?}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[test]
fn the_deck_comes_on_attach_and_only_viewers_get_the_keys() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut viewer = hub.client().await;
        let mut other = hub.client().await;
        deck_online(&mut viewer, true).await;
        deck_online(&mut other, true).await;
        viewer.send(&ClientMsg::DeckView { on: true }).await;
        let keys = keys_until(&mut viewer, |k| k.len() == 32).await;
        assert_eq!(
            keys[&31],
            DeckKey {
                key: 31,
                img: Some(image(31, false)),
                color: Some("#000000".into()),
                pressed: false
            }
        );
        fake.send(&state_line(5, true));
        let keys = keys_until(&mut viewer, |k| k.get(&5).is_some_and(|k| k.pressed)).await;
        assert_eq!(keys[&5].img.as_deref(), Some(image(5, true).as_str()));
        assert!(
            other
                .gets(Duration::from_millis(400), |m| matches!(
                    m,
                    ServerMsg::DeckKeys { .. }
                )
                .then_some(()))
                .await
                .is_none(),
            "a client not viewing the tab gets no key"
        );
        fake.send("KEYS-CLEAR DEVICEID=\"fohmixer-1\"");
        let keys = keys_until(&mut viewer, |k| k.get(&5).is_some_and(|k| k.img.is_none())).await;
        assert_eq!(
            keys[&5],
            DeckKey {
                key: 5,
                img: None,
                color: Some("#000000".into()),
                pressed: false
            }
        );
        viewer.send(&ClientMsg::DeckView { on: false }).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        viewer.clear();
        fake.send(&state_line(6, true));
        assert!(
            viewer
                .gets(Duration::from_millis(400), |m| matches!(
                    m,
                    ServerMsg::DeckKeys { .. }
                )
                .then_some(()))
                .await
                .is_none(),
            "a closed tab gets no key"
        );
        let status = hub
            .status()
            .await
            .companion
            .expect("a hub with [companion]");
        assert!(status.online);
        assert_eq!(
            (status.api_version.as_deref(), status.keys),
            (Some("1.12.0"), 32)
        );
        let records = hub
            .events_until(WAIT, |r| {
                r.iter().any(|e| e["ev"] == "deck_view" && e["on"] == false)
            })
            .await;
        let link = of(&records, "deck_link");
        assert_eq!(
            (
                link[0]["state"].clone(),
                link[0]["companion"].clone(),
                link[0]["api"].clone(),
                link[0]["attempts"].clone(),
                link[0]["down_ms"].clone()
            ),
            (
                json!("up"),
                json!("5.0.7+fake"),
                json!("1.12.0"),
                json!(1),
                Value::Null
            )
        );
        let pressed = of(&records, "deck_key")
            .into_iter()
            .find(|r| r["key"] == 5 && r["pressed"] == true)
            .expect("key 5's pressed change is recorded");
        assert_eq!(pressed["img_hash"], json!(img_hash(&image(5, true))));
        assert_eq!(pressed["img_bytes"], json!(image(5, true).len()));
        assert_eq!(of(&records, "deck_view").len(), 2);
        hub.stop().await;
    });
}

#[test]
fn a_press_goes_to_companion_and_its_ack_carries_the_round_trip() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut client = hub.client().await;
        deck_online(&mut client, true).await;
        press(&mut client, 3, true, 1).await;
        let (ok, error, rtt) = ack(&mut client, 1).await;
        assert!(ok && error.is_none(), "{error:?}");
        assert!(rtt.is_some_and(|ms| (0.0..1000.0).contains(&ms)), "{rtt:?}");
        press(&mut client, 3, false, 2).await;
        assert!(ack(&mut client, 2).await.0);
        assert_eq!(presses_in(&fake.got(), 3), vec![(1, true), (1, false)]);
        let records = hub
            .events_until(WAIT, |r| {
                r.iter().filter(|e| e["ev"] == "deck_ok").count() == 2
            })
            .await;
        let presses = of(&records, "deck_press");
        assert_eq!(presses.len(), 2);
        assert_eq!(
            (
                presses[0]["key"].clone(),
                presses[0]["down"].clone(),
                presses[0]["seq"].clone(),
                presses[0]["forwarded"].clone(),
                presses[0]["reason"].clone(),
                presses[0]["holders"].clone(),
                presses[0]["hold_ms"].clone()
            ),
            (
                json!(3),
                json!(true),
                json!(1),
                json!(true),
                Value::Null,
                json!(1),
                Value::Null
            )
        );
        assert_eq!(
            (
                presses[1]["down"].clone(),
                presses[1]["holders"].clone(),
                presses[1]["hold_ms"].clone(),
                presses[1]["why"].clone(),
                presses[1]["t"].clone()
            ),
            (
                json!(false),
                json!(0),
                json!(150.0),
                json!("up"),
                json!(1002.0)
            )
        );
        assert!(
            presses[1]["hub_hold_ms"]
                .as_f64()
                .is_some_and(|ms| ms >= 0.0)
        );
        let client_id = presses[0]["client"].clone();
        let oks = of(&records, "deck_ok");
        assert_eq!(
            oks.iter()
                .map(|r| (r["client"].clone(), r["seq"].clone(), r["ok"].clone()))
                .collect::<Vec<_>>(),
            vec![
                (client_id.clone(), json!(1), json!(true)),
                (client_id, json!(2), json!(true))
            ]
        );
        assert!(oks.iter().all(|r| r["rtt_ms"].as_f64().is_some()));
        hub.stop().await;
    });
}

#[test]
fn two_fingers_on_one_key_give_companion_one_down_and_one_up() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut a = hub.client().await;
        let mut b = hub.client().await;
        let mut c = hub.client().await;
        deck_online(&mut a, true).await;
        press(&mut a, 4, true, 1).await;
        assert!(
            ack(&mut a, 1).await.2.is_some(),
            "forwarded: Companion's round trip"
        );
        press(&mut b, 4, true, 1).await;
        assert_eq!(ack(&mut b, 1).await, (true, None, None));
        press(&mut a, 4, false, 2).await;
        assert_eq!(ack(&mut a, 2).await, (true, None, None));
        press(&mut c, 4, false, 1).await;
        assert_eq!(ack(&mut c, 1).await, (true, None, None));
        press(&mut b, 4, false, 2).await;
        assert!(ack(&mut b, 2).await.2.is_some());
        assert_eq!(presses_in(&fake.got(), 4), vec![(1, true), (1, false)]);
        let records = hub
            .events_until(WAIT, |r| {
                r.iter().filter(|e| e["ev"] == "deck_press").count() == 5
            })
            .await;
        let seen: Vec<(Value, Value)> = of(&records, "deck_press")
            .iter()
            .map(|r| (r["reason"].clone(), r["holders"].clone()))
            .collect();
        assert_eq!(
            seen,
            vec![
                (Value::Null, json!(1)),
                (json!("held"), json!(2)),
                (json!("held"), json!(1)),
                (json!("not held"), json!(1)),
                (Value::Null, json!(0)),
            ]
        );
        hub.stop().await;
    });
}

#[test]
fn a_closed_page_socket_releases_its_keys() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut a = hub.client().await;
        deck_online(&mut a, true).await;
        press(&mut a, 6, true, 1).await;
        assert!(ack(&mut a, 1).await.0);
        a.close().await;
        fake.until(Duration::from_secs(2), "key 6's release", |g| {
            presses_in(g, 6) == vec![(1, true), (1, false)]
        })
        .await;
        let records = hub
            .events_until(WAIT, |r| r.iter().any(|e| e["ev"] == "deck_release"))
            .await;
        let release = of(&records, "deck_release")[0];
        assert_eq!(
            (release["key"].clone(), release["reason"].clone()),
            (json!(6), json!("detach"))
        );
        assert_eq!(release["client"], of(&records, "deck_press")[0]["client"]);
        assert!(release["hub_hold_ms"].as_f64().is_some());
        hub.stop().await;
    });
}

#[test]
fn a_silent_holding_page_is_released_after_two_seconds_not_before() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut quiet = hub.client().await;
        let mut pinging = hub.client().await;
        deck_online(&mut quiet, true).await;
        press(&mut pinging, 8, true, 1).await;
        assert!(ack(&mut pinging, 1).await.0);
        let sent = Instant::now();
        press(&mut quiet, 7, true, 1).await;
        assert!(ack(&mut quiet, 1).await.0);
        // The pinging page keeps its key; the quiet one is released.
        for n in 0..15_u32 {
            pinging
                .send(&ClientMsg::Ping {
                    n,
                    t: 5_000.0 + f64::from(n) * 200.0,
                    rtt: None,
                    rtt_n: None,
                })
                .await;
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let got = fake.got();
        assert_eq!(presses_in(&got, 7), vec![(1, true), (1, false)]);
        let released = got
            .iter()
            .find(|g| g.line.ends_with(" KEY=7 PRESSED=0"))
            .unwrap()
            .at;
        let silence = released - sent;
        assert!(
            silence >= Duration::from_secs(2) && silence <= Duration::from_secs(3),
            "{silence:?}"
        );
        assert_eq!(
            presses_in(&got, 8),
            vec![(1, true)],
            "a pinging page keeps its key"
        );
        // The quiet page's late up: taken, not forwarded.
        press(&mut quiet, 7, false, 2).await;
        assert_eq!(ack(&mut quiet, 2).await, (true, None, None));
        press(&mut pinging, 8, false, 2).await;
        assert!(ack(&mut pinging, 2).await.2.is_some());
        let records = hub
            .events_until(WAIT, |r| {
                r.iter().filter(|e| e["ev"] == "deck_press").count() == 4
            })
            .await;
        let releases = of(&records, "deck_release");
        assert_eq!(releases.len(), 1);
        assert_eq!(
            (releases[0]["key"].clone(), releases[0]["reason"].clone()),
            (json!(7), json!("silent"))
        );
        assert_eq!(of(&records, "deck_press")[2]["reason"], "not held");
        hub.stop().await;
    });
}

#[test]
fn a_key_held_when_companion_was_lost_is_released_after_the_reconnect() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut a = hub.client().await;
        deck_online(&mut a, true).await;
        press(&mut a, 2, true, 1).await;
        assert!(ack(&mut a, 1).await.0);
        fake.close();
        deck_online(&mut a, false).await;
        deck_online(&mut a, true).await;
        // Companion 5.0.7 keeps a key held across its surface's removal: the
        // new surface releases it.
        fake.until(
            Duration::from_secs(2),
            "the release on the new surface",
            |g| presses_in(g, 2) == vec![(1, true), (2, false)],
        )
        .await;
        assert!(fake.lines_of(2)[0].starts_with("ADD-DEVICE DEVICEID=\"fohmixer-2\""));
        // The page's own up later: not held any more, not forwarded.
        press(&mut a, 2, false, 2).await;
        assert_eq!(ack(&mut a, 2).await, (true, None, None));
        assert_eq!(presses_in(&fake.got(), 2).len(), 2);
        let records = hub
            .events_until(WAIT, |r| r.iter().any(|e| e["ev"] == "deck_release"))
            .await;
        let links: Vec<Value> = of(&records, "deck_link")
            .iter()
            .map(|r| r["state"].clone())
            .collect();
        assert_eq!(links, vec![json!("up"), json!("down"), json!("up")]);
        let down = of(&records, "deck_link")[1];
        assert_eq!(down["error"], "Companion closed the connection");
        let up = of(&records, "deck_link")[2];
        assert!(up["down_ms"].as_f64().is_some_and(|ms| ms >= 200.0), "{up}");
        let release = of(&records, "deck_release")[0];
        assert_eq!(
            (
                release["client"].clone(),
                release["key"].clone(),
                release["reason"].clone()
            ),
            (Value::Null, json!(2), json!("reconnect"))
        );
        hub.stop().await;
    });
}

#[test]
fn a_key_held_when_companion_was_lost_is_logged_lost_when_the_hub_stops_first() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut a = hub.client().await;
        deck_online(&mut a, true).await;
        press(&mut a, 11, true, 1).await;
        assert!(ack(&mut a, 1).await.0);
        // A `deck` message the attach may have left waiting is not the loss.
        a.clear();
        // Companion goes away and stays away: the hub stops before the link
        // is back, so the key held at the loss cannot be released.
        fake.refuse(true);
        deck_online(&mut a, false).await;
        hub.stop().await;
        let records = stopped_log_with_a_release(dir.path()).await;
        let releases = of(&records, "deck_release");
        assert_eq!(releases.len(), 1, "{releases:?}");
        assert_eq!(
            (
                releases[0]["client"].clone(),
                releases[0]["key"].clone(),
                releases[0]["reason"].clone(),
                releases[0]["hub_hold_ms"].clone()
            ),
            (Value::Null, json!(11), json!("lost"), Value::Null)
        );
        assert_eq!(
            presses_in(&fake.got(), 11),
            vec![(1, true)],
            "nothing sent for the lost key after the loss"
        );
    });
}

#[test]
fn a_press_waiting_for_companion_is_answered_offline_when_the_link_goes() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script {
            api: "1.12.0".into(),
            refuse_add: None,
            answers: false,
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut a = hub.client().await;
        deck_online(&mut a, true).await;
        press(&mut a, 3, true, 1).await;
        assert!(
            a.gets(Duration::from_millis(300), |m| matches!(
                m,
                ServerMsg::DeckAck { .. }
            )
            .then_some(()))
                .await
                .is_none(),
            "no answer from Companion yet"
        );
        fake.close();
        assert_eq!(
            ack(&mut a, 1).await,
            (false, Some("offline".to_string()), None)
        );
        hub.stop().await;
    });
}

#[test]
fn presses_while_companion_is_away_are_refused_at_once_and_never_sent_later() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        fake.refuse(true);
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut a = hub.client().await;
        deck_online(&mut a, false).await;
        let started = Instant::now();
        press(&mut a, 1, true, 1).await;
        assert_eq!(
            ack(&mut a, 1).await,
            (false, Some("offline".to_string()), None)
        );
        assert!(started.elapsed() < Duration::from_millis(500), "at once");
        let status = hub
            .status_until(WAIT, |s| {
                s.companion
                    .as_ref()
                    .is_some_and(|c| c.connect_failures >= 1)
            })
            .await
            .companion
            .unwrap();
        assert!(!status.online && status.last_error.is_some(), "{status:?}");
        fake.refuse(false);
        deck_online(&mut a, true).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert!(
            presses_in(&fake.got(), 1).is_empty(),
            "a refused press is never sent later"
        );
        let records = hub
            .events_until(WAIT, |r| {
                r.iter().filter(|e| e["ev"] == "deck_link").count() >= 2
            })
            .await;
        let refused = of(&records, "deck_press")[0];
        assert_eq!(
            (refused["forwarded"].clone(), refused["reason"].clone()),
            (json!(false), json!("offline"))
        );
        let first = of(&records, "deck_link")[0];
        assert_eq!(
            (first["state"].clone(), first["attempts"].clone()),
            (json!("down"), json!(1))
        );
        hub.stop().await;
    });
}

#[test]
fn a_hub_without_companion_sends_no_deck_and_refuses_presses() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(vec![], dir.path()).await;
        let mut client = hub.client().await;
        assert!(
            client
                .gets(Duration::from_millis(500), |m| matches!(
                    m,
                    ServerMsg::Deck { .. }
                )
                .then_some(()))
                .await
                .is_none()
        );
        press(&mut client, 1, true, 1).await;
        assert_eq!(
            ack(&mut client, 1).await,
            (false, Some("no Stream Deck".to_string()), None)
        );
        assert_eq!(hub.status().await.companion, None);
        hub.stop().await;
    });
}

#[test]
fn the_stop_releases_held_keys_then_removes_the_device() {
    let _serial = serial();
    runtime().block_on(async {
        let fake = FakeCompanion::start(Script::companion()).await;
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(deck_config(dir.path(), fake.port)).await;
        let mut a = hub.client().await;
        deck_online(&mut a, true).await;
        press(&mut a, 9, true, 1).await;
        assert!(ack(&mut a, 1).await.0);
        hub.stop().await;
        let got = fake
            .until(Duration::from_secs(2), "REMOVE-DEVICE", |g| {
                g.iter().any(|l| l.line.starts_with("REMOVE-DEVICE"))
            })
            .await;
        // A ping that may fall between them left out: the release, then
        // REMOVE-DEVICE.
        let lines: Vec<&str> = got
            .iter()
            .map(|g| g.line.as_str())
            .filter(|l| !l.starts_with("PING "))
            .collect();
        assert_eq!(
            lines[lines.len() - 2..],
            [
                "KEY-PRESS DEVICEID=\"fohmixer-1\" KEY=9 PRESSED=0",
                "REMOVE-DEVICE DEVICEID=\"fohmixer-1\"",
            ]
        );
        // The stopped hub's log has the release.
        let records = stopped_log_with_a_release(dir.path()).await;
        let release = of(&records, "deck_release")[0];
        assert_eq!(
            (release["key"].clone(), release["reason"].clone()),
            (json!(9), json!("stop"))
        );
    });
}
