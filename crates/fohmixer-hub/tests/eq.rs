//! The Pro-Q 4 screen through the hub (#71 PR E, F28): a strip's Pro-Q 4
//! instances listed from SimLive (`Hand2 #`: one on the track, a renamed one
//! in a rack chain), an editor opened in Live (`is_editor_open`), its frames
//! as binary messages, the lock another client sees and is refused by, a
//! finger's contact on the simulated window backend (its record file
//! `eq-sim.jsonl`), the close through the guard (the inert spot tapped
//! before `is_editor_open = false`), a switch to another editor, a closed
//! socket's editor closed, a resting contact sent again and a silent page's
//! contact ended, and the stop handing the window back.
#![cfg(unix)]

mod support;

use std::path::Path;
use std::time::{Duration, Instant};

use fohmixer_hub::config::{Config, EqBackend, EqCfg};
use fohmixer_proto::client::{ClientMsg, ServerMsg};
use fohmixer_proto::eq::{EqItem, EqLock, EqState, Touch, frame_parts, reason};
use fohmixer_proto::layout::{Anchor, Binding};
use serde_json::{Value, json};
use support::{Client, Host, TestHub, runtime, serial};

const WAIT: Duration = Duration::from_secs(5);
const ON_TRACK: &str = "live_set tracks 1 devices 0";
const IN_CHAIN: &str = "live_set tracks 1 devices 1 chains 0 devices 0";
/// The same devices once the track above Hand2 # is deleted.
const MOVED_ON_TRACK: &str = "live_set tracks 0 devices 0";
const MOVED_IN_CHAIN: &str = "live_set tracks 0 devices 1 chains 0 devices 0";

fn config(dir: &Path, host: &Host) -> Config {
    let mut config = Config::defaults(dir);
    config.instances = vec![host.cfg()];
    config.layout_poll_ms = 100;
    config.eq = Some(EqCfg {
        backend: Some(EqBackend::Sim),
    });
    config
}

fn hand2() -> Binding {
    Binding {
        instance: "band".into(),
        anchor: Anchor::Track {
            name: "Hand2 #".into(),
        },
        path: None,
    }
}

/// A connected client once the band instance is online.
async fn client(hub: &TestHub) -> Client {
    let mut client = hub.client().await;
    client
        .instance_state("band", true, None, Duration::from_secs(10))
        .await;
    client
}

/// The client's next `eq_list` answer: its items and error.
async fn listed(client: &mut Client) -> (Vec<EqItem>, Option<String>) {
    client
        .wait(WAIT, |m| match m {
            ServerMsg::EqList { items, error, .. } => Some((items.clone(), error.clone())),
            _ => None,
        })
        .await
}

/// The client's next `eq` state: state, session, size, reason, since.
type State = (
    EqState,
    Option<u32>,
    Option<(u32, u32)>,
    Option<String>,
    Option<f64>,
);

async fn state(client: &mut Client, path: &str) -> State {
    let path = path.to_string();
    client
        .wait(WAIT, move |m| match m {
            ServerMsg::Eq {
                path: p,
                state,
                session,
                width,
                height,
                reason,
                since,
                ..
            } if *p == path => Some((*state, *session, width.zip(*height), reason.clone(), *since)),
            _ => None,
        })
        .await
}

/// The client's next `eq_locks`.
async fn locks(client: &mut Client) -> Vec<EqLock> {
    client
        .wait(WAIT, |m| match m {
            ServerMsg::EqLocks { items } => Some(items.clone()),
            _ => None,
        })
        .await
}

/// Opens `path` for `client`: its session.
async fn open(client: &mut Client, path: &str) -> u32 {
    client
        .send(&ClientMsg::EqOpen {
            instance: "band".into(),
            path: path.into(),
        })
        .await;
    let opening = state(client, path).await;
    assert_eq!(opening.0, EqState::Opening, "{opening:?}");
    let open = state(client, path).await;
    assert_eq!(open.0, EqState::Open, "{open:?}");
    assert_eq!(open.2, Some((1349, 809)));
    open.1.expect("a session")
}

/// Opens `path` for `client` and wants it refused after its opening (the
/// open sequence reads the device first): why.
async fn open_refused(client: &mut Client, path: &str) -> Option<String> {
    client
        .send(&ClientMsg::EqOpen {
            instance: "band".into(),
            path: path.into(),
        })
        .await;
    let opening = state(client, path).await;
    assert_eq!(opening.0, EqState::Opening, "{opening:?}");
    let refused = state(client, path).await;
    assert_eq!(
        (refused.0, refused.1, refused.2),
        (EqState::Closed, None, None),
        "{refused:?}"
    );
    refused.3
}

async fn input(client: &mut Client, touch: Touch, x: f64, y: f64) {
    client.send(&ClientMsg::EqInput { touch, x, y }).await;
}

/// The simulated backend's records so far.
fn sim_records(dir: &Path) -> Vec<Value> {
    std::fs::read_to_string(dir.join("eq-sim.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// A record as op, phase and point.
fn op(record: &Value) -> (String, String, i64, i64) {
    (
        record["op"].as_str().unwrap_or("").to_string(),
        record["phase"].as_str().unwrap_or("").to_string(),
        record["x"].as_i64().unwrap_or(-1),
        record["y"].as_i64().unwrap_or(-1),
    )
}

fn step(o: &str, phase: &str, x: i64, y: i64) -> (String, String, i64, i64) {
    (o.to_string(), phase.to_string(), x, y)
}

/// Whether `want` appears in `got` in its order (other records between).
fn in_order<T: PartialEq>(got: &[T], want: &[T]) -> bool {
    let mut rest = got.iter();
    want.iter().all(|w| rest.any(|g| g == w))
}

/// Waits (bounded) for the sim's records to hold `want` in order.
async fn sim_until(
    dir: &Path,
    want: &[(String, String, i64, i64)],
) -> Vec<(String, String, i64, i64)> {
    let deadline = Instant::now() + WAIT;
    loop {
        let got: Vec<_> = sim_records(dir).iter().map(op).collect();
        if in_order(&got, want) {
            return got;
        }
        assert!(Instant::now() < deadline, "never {want:?}: {got:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Waits (bounded) for Live's `is_editor_open` of `path` to be `open`.
async fn editor_open(client: &mut Client, path: &str, open: bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        if client.get("band", path, "is_editor_open").await == json!(open) {
            return;
        }
        assert!(Instant::now() < deadline, "{path} never open={open}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The `what`s of `eq` records.
fn eq_whats(records: &[Value]) -> Vec<String> {
    records
        .iter()
        .filter(|r| r["ev"] == "eq")
        .map(|r| r["what"].as_str().unwrap_or("").to_string())
        .collect()
}

#[test]
fn a_strips_pro_q_opens_sends_frames_takes_a_finger_and_closes_through_the_guard() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(config(dir.path(), &host)).await;
        let mut a = client(&hub).await;
        assert!(locks(&mut a).await.is_empty(), "none held at the attach");
        a.send(&ClientMsg::EqList { binding: hand2() }).await;
        let item = |path: &str, place: &str, name: &str, picture: bool| EqItem {
            path: path.into(),
            place: place.into(),
            name: name.into(),
            picture,
        };
        assert_eq!(
            listed(&mut a).await,
            (
                vec![
                    item(ON_TRACK, "na tracku", "Pro-Q 4", false),
                    item(IN_CHAIN, "Vocal FX › Main", "De-ess", false),
                ],
                None
            )
        );
        let session = open(&mut a, ON_TRACK).await;
        assert_eq!(session, 1);
        editor_open(&mut a, ON_TRACK, true).await;
        let frame = a.frame(WAIT).await.expect("a frame");
        let (of, jpeg) = frame_parts(&frame).unwrap();
        assert_eq!(of, session);
        assert_eq!(jpeg[..2], [0xFF_u8, 0xD8]);
        // Another client sees the lock and is refused.
        let mut b = client(&hub).await;
        let held = locks(&mut b).await;
        assert_eq!(held.len(), 1);
        assert_eq!(
            (
                held[0].instance.as_str(),
                held[0].path.as_str(),
                held[0].mine
            ),
            ("band", ON_TRACK, false)
        );
        b.send(&ClientMsg::EqOpen {
            instance: "band".into(),
            path: ON_TRACK.into(),
        })
        .await;
        let refused = state(&mut b, ON_TRACK).await;
        assert_eq!(
            refused,
            (
                EqState::Closed,
                None,
                None,
                Some("locked".into()),
                Some(held[0].since)
            )
        );
        // A finger on the picture, in its pixels.
        input(&mut a, Touch::Down, 10.4, 20.6).await;
        input(&mut a, Touch::Move, 30.0, 40.0).await;
        input(&mut a, Touch::Up, 30.0, 40.0).await;
        sim_until(
            dir.path(),
            &[
                step("take", "", -1, -1),
                step("touch", "down", 10, 21),
                step("touch", "update", 30, 40),
                step("touch", "up", 30, 40),
            ],
        )
        .await;
        // Leaving the screen closes it: the guard's tap first, then Live.
        b.clear();
        a.send(&ClientMsg::EqClose).await;
        let closed = state(&mut a, ON_TRACK).await;
        assert_eq!(closed.0, EqState::Closed);
        assert_eq!(closed.3.as_deref(), Some("exit"));
        assert!(locks(&mut b).await.is_empty());
        editor_open(&mut a, ON_TRACK, false).await;
        sim_until(
            dir.path(),
            &[
                step("touch", "up", 30, 40),
                step("touch", "down", 546, 15),
                step("touch", "up", 546, 15),
                step("release", "", -1, -1),
            ],
        )
        .await;
        // Its last picture is kept for the card.
        let (code, _) = hub
            .get("/api/eq/picture?instance=band&path=live_set%20tracks%201%20devices%200")
            .await;
        assert_eq!(code, 200);
        a.send(&ClientMsg::EqList { binding: hand2() }).await;
        let (items, _) = listed(&mut a).await;
        assert_eq!(
            items.iter().map(|i| i.picture).collect::<Vec<_>>(),
            [true, false]
        );
        let want: Vec<String> = [
            "take", "open", "opened", "refused", "touch", "touch", "close", "closed",
        ]
        .map(String::from)
        .to_vec();
        hub.events_until(WAIT, |records| in_order(&eq_whats(records), &want))
            .await;
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn only_a_listed_pro_q_opens_and_a_missing_track_says_why() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(config(dir.path(), &host)).await;
        let mut a = client(&hub).await;
        a.send(&ClientMsg::EqOpen {
            instance: "band".into(),
            path: ON_TRACK.into(),
        })
        .await;
        let refused = state(&mut a, ON_TRACK).await;
        assert_eq!(refused.0, EqState::Closed);
        assert_eq!(refused.3.as_deref(), Some("unknown"));
        let mut missing = hand2();
        missing.anchor = Anchor::Track {
            name: "Nobody #".into(),
        };
        a.send(&ClientMsg::EqList { binding: missing }).await;
        let (items, error) = listed(&mut a).await;
        assert!(items.is_empty());
        let error = error.expect("why");
        assert!(error.starts_with("not found"), "{error}");
        // A track without plug-ins lists none.
        let mut plain = hand2();
        plain.anchor = Anchor::Track {
            name: "Hand1 #".into(),
        };
        a.send(&ClientMsg::EqList { binding: plain }).await;
        assert_eq!(listed(&mut a).await, (Vec::new(), None));
        assert!(sim_records(dir.path()).is_empty(), "no window touched");
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn an_editor_already_open_in_live_is_refused_and_left_open() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(config(dir.path(), &host)).await;
        let mut a = client(&hub).await;
        a.send(&ClientMsg::EqList { binding: hand2() }).await;
        listed(&mut a).await;
        // Opened on the PC (a failed guard and a hub's restart leave one
        // open too): the hub has no window of it, so it never takes it and
        // never closes it (a close without its window skips the guard).
        a.set("band", ON_TRACK, "is_editor_open", json!(true)).await;
        assert_eq!(
            open_refused(&mut a, ON_TRACK).await.as_deref(),
            Some(reason::OPEN_ON_PC)
        );
        editor_open(&mut a, ON_TRACK, true).await;
        assert!(
            sim_records(dir.path()).is_empty(),
            "no window taken: {:?}",
            sim_records(dir.path())
        );
        // Closed on the PC, it opens again.
        a.set("band", ON_TRACK, "is_editor_open", json!(false))
            .await;
        open(&mut a, ON_TRACK).await;
        editor_open(&mut a, ON_TRACK, true).await;
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_device_renamed_or_moved_since_its_list_is_refused() {
    let _serial = serial();
    runtime().block_on(async {
        let mut host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(config(dir.path(), &host)).await;
        let mut a = client(&hub).await;
        a.send(&ClientMsg::EqList { binding: hand2() }).await;
        listed(&mut a).await;
        // Renamed in Live since the list: not the device the page listed.
        a.set("band", IN_CHAIN, "name", json!("Air EQ")).await;
        assert_eq!(
            open_refused(&mut a, IN_CHAIN).await.as_deref(),
            Some(reason::MOVED)
        );
        // The track above deleted: the listed path names no device now.
        assert!(host.delete_track(0) > 0, "Hand1 # deleted");
        assert_eq!(
            open_refused(&mut a, ON_TRACK).await.as_deref(),
            Some(reason::MOVED)
        );
        // Live is left alone: neither editor opened, no window taken.
        editor_open(&mut a, MOVED_ON_TRACK, false).await;
        editor_open(&mut a, MOVED_IN_CHAIN, false).await;
        assert!(
            sim_records(dir.path()).is_empty(),
            "no window taken: {:?}",
            sim_records(dir.path())
        );
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn another_pro_q_closes_the_first_and_a_closed_socket_closes_its_own() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(config(dir.path(), &host)).await;
        let mut a = client(&hub).await;
        let mut watcher = client(&hub).await;
        a.send(&ClientMsg::EqList { binding: hand2() }).await;
        listed(&mut a).await;
        assert_eq!(open(&mut a, ON_TRACK).await, 1);
        // The other one: the first closes (switch), then the second opens.
        a.send(&ClientMsg::EqOpen {
            instance: "band".into(),
            path: IN_CHAIN.into(),
        })
        .await;
        let first = state(&mut a, ON_TRACK).await;
        assert_eq!(
            (first.0, first.3.as_deref()),
            (EqState::Closed, Some("switch"))
        );
        let opening = state(&mut a, IN_CHAIN).await;
        assert_eq!(opening.0, EqState::Opening);
        let second = state(&mut a, IN_CHAIN).await;
        assert_eq!((second.0, second.1), (EqState::Open, Some(2)));
        editor_open(&mut watcher, ON_TRACK, false).await;
        editor_open(&mut watcher, IN_CHAIN, true).await;
        // Its page goes away: the hub closes it.
        a.close().await;
        editor_open(&mut watcher, IN_CHAIN, false).await;
        // The watcher's locks: none at its attach, then held, then none.
        let mut seen = false;
        for _ in 0..10 {
            let held = locks(&mut watcher).await;
            if held.is_empty() && seen {
                break;
            }
            seen |= !held.is_empty();
        }
        assert!(seen);
        let closed = |records: &[Value]| -> Vec<Value> {
            records
                .iter()
                .filter(|r| r["ev"] == "eq" && r["what"] == "closed")
                .cloned()
                .collect()
        };
        let records = hub
            .events_until(WAIT, |records| closed(records).len() == 2)
            .await;
        let closed = closed(&records);
        assert_eq!(closed[0]["why"], "switch");
        assert_eq!(closed[1]["why"], "detach");
        hub.stop().await;
        host.stop();
    });
}

/// Opens Hand2 #'s Pro-Q 4 on the track, deletes the track above it (the
/// held path now names Hand3 #'s first device: none), then leaves the
/// screen; with `several`, the rack's Pro-Q 4 has its editor open in Live
/// too (opened there by hand). Live's two editors afterwards (on the track,
/// in the chain, at their new paths) and the hub's `eq` records.
async fn close_after_a_move(several: bool) -> (Value, Value, Vec<Value>) {
    let mut host = Host::start("band");
    let dir = tempfile::tempdir().unwrap();
    let hub = TestHub::start_config(config(dir.path(), &host)).await;
    let mut a = client(&hub).await;
    a.send(&ClientMsg::EqList { binding: hand2() }).await;
    listed(&mut a).await;
    open(&mut a, ON_TRACK).await;
    if several {
        a.set("band", IN_CHAIN, "is_editor_open", json!(true)).await;
    }
    assert!(host.delete_track(0) > 0, "Hand1 # deleted");
    a.send(&ClientMsg::EqClose).await;
    // The closed state comes once the whole close sequence ran.
    let closed = state(&mut a, ON_TRACK).await;
    assert_eq!(
        (closed.0, closed.3.as_deref()),
        (EqState::Closed, Some("exit"))
    );
    let records = hub
        .events_until(WAIT, |records| {
            eq_whats(records).contains(&"closed".to_string())
        })
        .await;
    let on_track = a.get("band", MOVED_ON_TRACK, "is_editor_open").await;
    let in_chain = a.get("band", MOVED_IN_CHAIN, "is_editor_open").await;
    hub.stop().await;
    host.stop();
    let eq = records.into_iter().filter(|r| r["ev"] == "eq").collect();
    (on_track, in_chain, eq)
}

/// The `eq` record of `what`, if any.
fn eq_record<'a>(records: &'a [Value], what: &str) -> Option<&'a Value> {
    records.iter().find(|r| r["what"] == what)
}

#[test]
fn a_moved_editor_is_closed_where_it_moved() {
    let _serial = serial();
    runtime().block_on(async {
        let (on_track, in_chain, records) = close_after_a_move(false).await;
        assert_eq!((on_track, in_chain), (json!(false), json!(false)));
        assert!(eq_record(&records, "problem").is_none(), "{records:?}");
    });
}

#[test]
fn a_moved_editor_is_closed_and_another_open_one_left_alone() {
    let _serial = serial();
    runtime().block_on(async {
        // The held path names no device; the editor is the same object
        // where it moved, and the one opened by hand is not the hub's.
        let (on_track, in_chain, records) = close_after_a_move(true).await;
        assert_eq!((on_track, in_chain), (json!(false), json!(true)));
        assert!(eq_record(&records, "problem").is_none(), "{records:?}");
    });
}

/// The `eq` records once a close ended (its `closed` record logged).
async fn closed_records(hub: &TestHub) -> Vec<Value> {
    let records = hub
        .events_until(WAIT, |records| {
            eq_whats(records).contains(&"closed".to_string())
        })
        .await;
    records.into_iter().filter(|r| r["ev"] == "eq").collect()
}

#[test]
fn a_deleted_editors_close_leaves_live_alone_and_tells_its_holder_nothing_is_open() {
    let _serial = serial();
    runtime().block_on(async {
        let mut host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(config(dir.path(), &host)).await;
        let mut a = client(&hub).await;
        a.send(&ClientMsg::EqList { binding: hand2() }).await;
        listed(&mut a).await;
        open(&mut a, ON_TRACK).await;
        // Its own track deleted: Live closed its editor with it, and the
        // script answers the ref's turn-off `StaleRef`. Nothing of the
        // hub's is open in Live, so Live is left alone and the page hears
        // its own close (never "left open": nothing is there to close).
        assert!(host.delete_track(1) > 0, "Hand2 # deleted");
        a.send(&ClientMsg::EqClose).await;
        let closed = state(&mut a, ON_TRACK).await;
        assert_eq!(
            (closed.0, closed.3.as_deref()),
            (EqState::Closed, Some(reason::EXIT))
        );
        let eq = closed_records(&hub).await;
        assert!(eq_record(&eq, "problem").is_none(), "{eq:?}");
        let none_open = eq_record(&eq, "none_open").expect("a none_open record");
        let why = none_open["why"].as_str().unwrap_or("");
        assert!(why.starts_with("the editor's device was deleted"), "{why}");
        hub.stop().await;
        host.stop();
    });
}

/// A set whose second and third tracks hold a Pro-Q 4 each (two engineers'
/// editors on neighbouring tracks), its first track none.
fn neighbours(dir: &Path) -> std::path::PathBuf {
    let pro_q = json!({"template": "plugin", "name": "Pro-Q 4", "product": "Pro-Q 4"});
    let set = json!({"name": "Neighbours", "tracks": [
        {"name": "Lead #"},
        {"name": "Kick #", "devices": [pro_q.clone()]},
        {"name": "Snare #", "devices": [pro_q]},
    ]});
    let site = dir.join("neighbours.json");
    std::fs::write(&site, set.to_string()).unwrap();
    site
}

/// The band's track `name` as a strip binds it.
fn track(name: &str) -> Binding {
    Binding {
        instance: "band".into(),
        anchor: Anchor::Track { name: name.into() },
        path: None,
    }
}

#[test]
fn a_close_turns_off_its_own_moved_editor_never_the_one_now_at_its_path() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut host = Host::start_site("band", &neighbours(dir.path()), 0, 0.0);
        let hub = TestHub::start_config(config(dir.path(), &host)).await;
        let (kick, snare) = ("live_set tracks 1 devices 0", "live_set tracks 2 devices 0");
        // A holds the Snare's editor, B the Kick's.
        let mut a = client(&hub).await;
        a.send(&ClientMsg::EqList {
            binding: track("Snare #"),
        })
        .await;
        assert_eq!(listed(&mut a).await.0.len(), 1);
        open(&mut a, snare).await;
        let mut b = client(&hub).await;
        b.send(&ClientMsg::EqList {
            binding: track("Kick #"),
        })
        .await;
        assert_eq!(listed(&mut b).await.0.len(), 1);
        open(&mut b, kick).await;
        // The first track deleted: B's held path names A's editor now.
        assert!(host.delete_track(0) > 0, "Lead # deleted");
        b.send(&ClientMsg::EqClose).await;
        let closed = state(&mut b, kick).await;
        assert_eq!(
            (closed.0, closed.3.as_deref()),
            (EqState::Closed, Some(reason::EXIT))
        );
        // B's own editor (the Kick's, track 0 now) is off; A's stays open.
        assert_eq!(
            b.get("band", "live_set tracks 0 devices 0", "is_editor_open")
                .await,
            json!(false),
            "B's editor"
        );
        assert_eq!(
            b.get("band", "live_set tracks 1 devices 0", "is_editor_open")
                .await,
            json!(true),
            "A's editor"
        );
        let records = hub
            .events_until(WAIT, |records| {
                eq_whats(records).contains(&"closed".to_string())
            })
            .await;
        let eq: Vec<Value> = records.into_iter().filter(|r| r["ev"] == "eq").collect();
        assert!(eq_record(&eq, "problem").is_none(), "{eq:?}");
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_deleted_editors_close_never_turns_off_the_editor_now_at_its_path() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut host = Host::start_site("band", &neighbours(dir.path()), 0, 0.0);
        let hub = TestHub::start_config(config(dir.path(), &host)).await;
        let (kick, snare) = ("live_set tracks 1 devices 0", "live_set tracks 2 devices 0");
        let mut a = client(&hub).await;
        a.send(&ClientMsg::EqList {
            binding: track("Kick #"),
        })
        .await;
        assert_eq!(listed(&mut a).await.0.len(), 1);
        open(&mut a, kick).await;
        // The Snare's editor opened on the PC by hand.
        a.set("band", snare, "is_editor_open", json!(true)).await;
        // The Kick's track deleted with A's editor (Live closed it): A's
        // held path names the Snare's open editor now. Its ref answers
        // `StaleRef`, and the close never looks at the held path (its
        // device would close without its guard).
        assert!(host.delete_track(1) > 0, "Kick # deleted");
        a.send(&ClientMsg::EqClose).await;
        let closed = state(&mut a, kick).await;
        assert_eq!(
            (closed.0, closed.3.as_deref()),
            (EqState::Closed, Some(reason::EXIT))
        );
        assert_eq!(
            a.get("band", kick, "is_editor_open").await,
            json!(true),
            "the Snare's editor, at the held path now, stays open"
        );
        let eq = closed_records(&hub).await;
        assert!(eq_record(&eq, "problem").is_none(), "{eq:?}");
        assert!(eq_record(&eq, "none_open").is_some(), "{eq:?}");
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_resting_finger_goes_again_until_its_page_falls_silent() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(config(dir.path(), &host)).await;
        let mut a = client(&hub).await;
        a.send(&ClientMsg::EqList { binding: hand2() }).await;
        listed(&mut a).await;
        open(&mut a, ON_TRACK).await;
        input(&mut a, Touch::Down, 100.0, 100.0).await;
        // The page pings (heard) for 2.4 s: the contact rests, sent again.
        let started = Instant::now();
        let mut last_ping = Instant::now();
        for n in 1..=8 {
            a.send(&ClientMsg::Ping {
                n,
                t: 1_000.0,
                rtt: None,
                rtt_n: None,
            })
            .await;
            last_ping = Instant::now();
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        let records = sim_records(dir.path());
        let ops: Vec<_> = records.iter().map(op).collect();
        let resends = ops
            .iter()
            .filter(|o| **o == step("touch", "update", 100, 100))
            .count();
        // Windows ends an injected contact that gets no frame for 100 ms:
        // the window worker injects a resting one again every 50 ms.
        assert!(resends >= 30, "{ops:?}");
        assert!(!ops.iter().any(|o| o.1 == "cancel"), "{ops:?}");
        let times: Vec<f64> = records
            .iter()
            .filter(|r| r["op"] == "touch")
            .map(|r| r["t"].as_f64().expect("a touch's time"))
            .collect();
        // Wall-clock times of a whole hub: a loaded runner (two host tests
        // at once, the coverage build, the encoder's JPEGs) may hold the
        // worker up once. The strict 100 ms bound is the explicit-clock
        // unit tests' (`plugwin::tests`).
        let gaps: Vec<f64> = times.windows(2).map(|pair| pair[1] - pair[0]).collect();
        let late = gaps.iter().filter(|gap| **gap >= 100.0).count();
        let largest = gaps.iter().copied().fold(0.0, f64::max);
        assert!(
            late <= 1 && largest < 150.0,
            "{late} gaps of 100 ms or more, the largest {largest} ms: {gaps:?}"
        );
        // Silent for 2 s: the contact ends where it rested.
        let ops = sim_until(dir.path(), &[step("touch", "cancel", 100, 100)]).await;
        assert!(
            last_ping.elapsed() >= Duration::from_millis(2000),
            "{:?} after the start: {ops:?}",
            started.elapsed()
        );
        hub.events_until(WAIT, |records| {
            eq_whats(records).contains(&"silent".to_string())
        })
        .await;
        // The stop hands the window back (the editor stays open in Live).
        hub.stop().await;
        sim_until(dir.path(), &[step("release", "", -1, -1)]).await;
        host.stop();
    });
}
