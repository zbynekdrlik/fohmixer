use std::collections::BTreeMap;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use fohmixer_proto::layout::Anchor;
use tokio::sync::mpsc;

use super::*;
use crate::config::InstanceCfg;
use crate::events::EventLog;
use crate::live::client::LiveEvent;
use crate::outbox::Outbox;
use crate::plugwin::sim::{REFUSED, Sim, SimHandle};
use crate::router::RouterIo;

const PATH: &str = "live_set tracks 1 devices 0";

fn key() -> EditorKey {
    EditorKey::new("band", PATH)
}

/// `key()` as a list found it, with its device's name.
fn found_key() -> (EditorKey, String) {
    (key(), "Pro-Q 4".to_string())
}

/// The `band` instance on a port nothing listens on: every call answers
/// "instance offline".
fn offline_live() -> LiveHandle {
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let (live, _task) = LiveHandle::spawn(
        &InstanceCfg {
            name: "band".into(),
            port,
        },
        Arc::new(|_: &str, _: LiveEvent| {}),
    );
    live
}

/// The window worker on a sim.
fn sim_plugwin() -> (Plugwin, SimHandle) {
    let (sim, handle) = Sim::new(None);
    let plugwin = Plugwin::spawn(Box::new(sim), Arc::new(|_: PlugwinEvent| {})).unwrap();
    (plugwin, handle)
}

struct Rig {
    router: Router,
    rx: mpsc::UnboundedReceiver<RouterMsg>,
    records: Receiver<Value>,
    sim: SimHandle,
    plugwin: Plugwin,
}

/// A router over an offline `band` with the Pro-Q screen on a sim.
fn rig(dir: &std::path::Path) -> Rig {
    let (tx, rx) = mpsc::unbounded_channel();
    let (events, records) = EventLog::channel(1024);
    let (plugwin, sim) = sim_plugwin();
    let router = Router::new(
        BTreeMap::from([("band".to_string(), offline_live())]),
        false,
        dir.to_path_buf(),
        RouterIo { tx, events },
    )
    .with_eq(plugwin.clone(), Arc::new(Pictures::default()));
    Rig {
        router,
        rx,
        records,
        sim,
        plugwin,
    }
}

fn attach(router: &mut Router, client: ClientId) -> Arc<Outbox> {
    let outbox = Arc::new(Outbox::new());
    router.handle(RouterMsg::Attach {
        client,
        outbox: Arc::clone(&outbox),
        peer: format!("10.0.0.{client}"),
    });
    outbox
}

/// The `eq*` messages waiting in an outbox.
fn eq_msgs(outbox: &Outbox) -> Vec<ServerMsg> {
    outbox
        .take()
        .unwrap()
        .into_iter()
        .filter(|m| {
            matches!(
                m,
                ServerMsg::Eq { .. } | ServerMsg::EqList { .. } | ServerMsg::EqLocks { .. }
            )
        })
        .collect()
}

/// The `eq` records waiting: their `what`s.
fn whats(records: &Receiver<Value>) -> Vec<String> {
    records
        .try_iter()
        .filter(|r| r["ev"] == "eq")
        .map(|r| r["what"].as_str().unwrap().to_string())
        .collect()
}

/// The router's next own message `want` takes (bounded).
async fn next_of(
    rx: &mut mpsc::UnboundedReceiver<RouterMsg>,
    want: impl Fn(&RouterMsg) -> bool,
) -> RouterMsg {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("a message")
            .expect("the channel is open");
        if want(&msg) {
            return msg;
        }
    }
}

/// Waits (bounded) for the sim's records to satisfy `check`.
async fn sim_until(sim: &SimHandle, what: &str, check: impl Fn(&[Value]) -> bool) {
    for _ in 0..300 {
        if check(&sim.records()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the sim never recorded {what}: {:?}", sim.records());
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

#[test]
fn a_minutes_rate_is_one_record() {
    let rate = Rate {
        grabs: 1500,
        sent: 1200,
        failed: 2,
        grab_ms: 4.5,
        encode_ms: 9.25,
        bytes: 61_000.0,
        width: 1349,
        height: 809,
        gap_ms: 61.5,
    };
    assert_eq!(
        rate_fields(7, &rate),
        json!({"what": "rate", "session": 7, "grabs": 1500, "sent": 1200, "failed": 2,
               "grab_ms": 4.5, "encode_ms": 9.25, "bytes": 61_000.0,
               "width": 1349, "height": 809, "gap_ms": 61.5})
    );
    assert_eq!(EDITOR_OPEN, "is_editor_open");
}

#[tokio::test]
async fn a_new_client_hears_the_locks_and_an_unlisted_open_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let Rig {
        mut router,
        records,
        ..
    } = rig(dir.path());
    let outbox = attach(&mut router, 1);
    assert_eq!(
        eq_msgs(&outbox),
        vec![ServerMsg::EqLocks { items: Vec::new() }]
    );
    router.handle(RouterMsg::EqOpen {
        client: 1,
        instance: "band".into(),
        path: PATH.into(),
    });
    assert_eq!(
        eq_msgs(&outbox),
        vec![crate::eq::closed_msg(&key(), reason::UNKNOWN, None)]
    );
    assert_eq!(whats(&records), vec!["refused"]);
}

#[tokio::test]
async fn a_list_answers_what_it_could_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let Rig {
        mut router, mut rx, ..
    } = rig(dir.path());
    let outbox = attach(&mut router, 1);
    outbox.take();
    // The instance is offline: the walk's first read fails.
    router.handle(RouterMsg::EqList {
        client: 1,
        binding: hand2(),
    });
    let listed = next_of(&mut rx, |m| matches!(m, RouterMsg::EqListed { .. })).await;
    router.handle(listed);
    let error = |binding: Binding, why: &str| ServerMsg::EqList {
        binding,
        items: Vec::new(),
        error: Some(why.to_string()),
    };
    assert_eq!(eq_msgs(&outbox), vec![error(hand2(), "instance offline")]);
    // An instance the hub does not know, and a path that does not parse.
    let mut elsewhere = hand2();
    elsewhere.instance = "drums".into();
    router.handle(RouterMsg::EqList {
        client: 1,
        binding: elsewhere.clone(),
    });
    let mut bad = hand2();
    bad.path = Some("devices[".into());
    let why = bad.target().expect_err("a bad path").to_string();
    router.handle(RouterMsg::EqList {
        client: 1,
        binding: bad.clone(),
    });
    assert_eq!(
        eq_msgs(&outbox),
        vec![error(elsewhere, UNKNOWN_INSTANCE), error(bad, &why)]
    );
}

#[tokio::test]
async fn a_listed_editor_is_known_and_says_whether_a_picture_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let Rig {
        mut router,
        records,
        ..
    } = rig(dir.path());
    let outbox = attach(&mut router, 1);
    outbox.take();
    let found = |path: &str, place: &str| Found {
        path: path.into(),
        place: place.into(),
        name: "Pro-Q 4".into(),
    };
    let inner = "live_set tracks 1 devices 1 chains 0 devices 0";
    router
        .eq
        .as_ref()
        .unwrap()
        .pictures
        .put(&EditorKey::new("band", inner), Bytes::from_static(b"jpeg"));
    router.handle(RouterMsg::EqListed {
        client: 1,
        binding: hand2(),
        outcome: Ok(vec![
            found(PATH, "na tracku"),
            found(inner, "Vocal FX › Main"),
        ]),
    });
    let item = |path: &str, place: &str, picture: bool| EqItem {
        path: path.into(),
        place: place.into(),
        name: "Pro-Q 4".into(),
        picture,
    };
    assert_eq!(
        eq_msgs(&outbox),
        vec![ServerMsg::EqList {
            binding: hand2(),
            items: vec![
                item(PATH, "na tracku", false),
                item(inner, "Vocal FX › Main", true)
            ],
            error: None,
        }]
    );
    // Both may be opened now (here the open fails: the instance is offline).
    router.handle(RouterMsg::EqOpen {
        client: 1,
        instance: "band".into(),
        path: inner.into(),
    });
    let opening = eq_msgs(&outbox);
    assert_eq!(
        opening[0],
        crate::eq::opening_msg(&EditorKey::new("band", inner))
    );
    assert!(matches!(opening[1], ServerMsg::EqLocks { .. }));
    assert_eq!(whats(&records), vec!["take", "open"]);
}

/// The `band` instance connected (again).
fn connected() -> RouterMsg {
    RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Connected(crate::live::ConnectInfo {
            instance: "band".into(),
            set_name: "set".into(),
            script_version: "0".into(),
            live_version: "12".into(),
        }),
    }
}

#[tokio::test]
async fn an_open_after_its_instance_connected_again_is_refused_until_listed() {
    let dir = tempfile::tempdir().unwrap();
    let Rig {
        mut router,
        records,
        ..
    } = rig(dir.path());
    let outbox = attach(&mut router, 1);
    router.handle(RouterMsg::EqListed {
        client: 1,
        binding: hand2(),
        outcome: Ok(vec![Found {
            path: PATH.into(),
            place: "na tracku".into(),
            name: "Pro-Q 4".into(),
        }]),
    });
    // The instance connects again: another set may be loaded, so the path
    // listed before may name another device.
    router.handle(connected());
    outbox.take();
    let _ = records.try_iter().count();
    router.handle(RouterMsg::EqOpen {
        client: 1,
        instance: "band".into(),
        path: PATH.into(),
    });
    assert_eq!(
        eq_msgs(&outbox),
        vec![crate::eq::closed_msg(&key(), reason::UNKNOWN, None)]
    );
    assert_eq!(whats(&records), vec!["refused"]);
}

#[tokio::test]
async fn a_new_list_and_a_connect_drop_the_pictures_they_no_longer_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = rig(dir.path());
    let _outbox = attach(&mut rig.router, 1);
    let pictures = Arc::clone(&rig.router.eq.as_ref().unwrap().pictures);
    let inner = EditorKey::new("band", "live_set tracks 1 devices 1 chains 0 devices 0");
    let drums = EditorKey::new("drums", PATH);
    for k in [key(), inner.clone(), drums.clone()] {
        pictures.put(&k, Bytes::from_static(b"jpeg"));
    }
    let found = |key: &EditorKey| Found {
        path: key.path.clone(),
        place: "na tracku".into(),
        name: "Pro-Q 4".into(),
    };
    rig.router.handle(RouterMsg::EqListed {
        client: 1,
        binding: hand2(),
        outcome: Ok(vec![found(&key()), found(&inner)]),
    });
    // The track lists again without the chain's Pro-Q 4: its card's
    // picture goes; the other stays.
    rig.router.handle(RouterMsg::EqListed {
        client: 1,
        binding: hand2(),
        outcome: Ok(vec![found(&key())]),
    });
    assert_eq!(
        [&key(), &inner, &drums].map(|k| pictures.has(k)),
        [true, false, true]
    );
    // The band instance connects again: its pictures and refs go.
    let refs = &mut rig.router.eq.as_mut().unwrap().refs;
    refs.insert(1, ("band".to_string(), json!({"$ref": "a"})));
    refs.insert(2, ("drums".to_string(), json!({"$ref": "b"})));
    rig.router.handle(connected());
    assert_eq!([&key(), &drums].map(|k| pictures.has(k)), [false, true]);
    let kept: Vec<u32> = rig
        .router
        .eq
        .as_ref()
        .unwrap()
        .refs
        .keys()
        .copied()
        .collect();
    assert_eq!(kept, [2]);
}

#[tokio::test]
async fn an_open_of_an_offline_instance_fails_and_frees_the_editor() {
    let dir = tempfile::tempdir().unwrap();
    let Rig {
        mut router,
        mut rx,
        records,
        ..
    } = rig(dir.path());
    let outbox = attach(&mut router, 1);
    let other = attach(&mut router, 2);
    router
        .eq
        .as_mut()
        .unwrap()
        .state
        .listed("band", "live_set tracks 1", [found_key()]);
    router.handle(RouterMsg::EqOpen {
        client: 1,
        instance: "band".into(),
        path: PATH.into(),
    });
    outbox.take();
    other.take();
    let opened = next_of(&mut rx, |m| matches!(m, RouterMsg::EqOpened { .. })).await;
    let RouterMsg::EqOpened { ref outcome, .. } = opened else {
        unreachable!()
    };
    assert_eq!(outcome, &Err("instance offline".to_string()));
    router.handle(opened);
    assert_eq!(
        eq_msgs(&outbox),
        vec![
            crate::eq::closed_msg(&key(), "instance offline", None),
            ServerMsg::EqLocks { items: Vec::new() },
        ]
    );
    assert_eq!(
        eq_msgs(&other),
        vec![ServerMsg::EqLocks { items: Vec::new() }]
    );
    assert_eq!(whats(&records), vec!["take", "open", "failed"]);
}

/// Client 1 holds `key()` open as session 1, its window taken by the
/// worker (the router's state driven directly: the instance is offline).
async fn held_open(rig: &mut Rig) -> Arc<Outbox> {
    let outbox = attach(&mut rig.router, 1);
    assert_eq!(rig.plugwin.take(1, Vec::new()).await, Ok((1349, 809)));
    let io = rig.router.eq.as_mut().unwrap();
    io.state.listed("band", "live_set tracks 1", [found_key()]);
    io.state.open(1, &key(), 0.0);
    let acts = io.state.opened(&key(), 1, Ok((1349, 809)));
    rig.router.eq_acts(acts);
    let _ = rig.records.try_iter().count();
    outbox.take();
    outbox
}

#[tokio::test]
async fn an_open_editors_frames_reach_its_holder_and_its_picture_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = rig(dir.path());
    let outbox = held_open(&mut rig).await;
    let mut frame = None;
    for _ in 0..300 {
        frame = outbox.take_frame();
        if frame.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let frame = frame.expect("a frame");
    let (session, jpeg) = fohmixer_proto::eq::frame_parts(&frame).unwrap();
    assert_eq!(session, 1);
    assert_eq!(jpeg[..2], [0xFF_u8, 0xD8]);
    let pictures = &rig.router.eq.as_ref().unwrap().pictures;
    assert!(pictures.has(&key()));
}

#[tokio::test]
async fn a_finger_reaches_the_window_and_a_resting_one_goes_again() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = rig(dir.path());
    let _outbox = held_open(&mut rig).await;
    rig.router.handle(RouterMsg::EqInput {
        client: 1,
        touch: Touch::Down,
        x: 10.4,
        y: 20.6,
    });
    sim_until(&rig.sim, "the down", |r| {
        r.iter()
            .any(|r| r["phase"] == "down" && r["x"] == 10 && r["y"] == 21)
    })
    .await;
    assert_eq!(whats(&rig.records), vec!["touch"]);
    tokio::time::sleep(Duration::from_millis(120)).await;
    rig.router.handle(RouterMsg::Heard { client: 1 });
    rig.router.handle(RouterMsg::Tick);
    sim_until(&rig.sim, "the resend", |r| {
        r.iter()
            .any(|r| r["phase"] == "update" && r["x"] == 10 && r["y"] == 21)
    })
    .await;
    rig.router.handle(RouterMsg::EqInput {
        client: 1,
        touch: Touch::Up,
        x: 10.0,
        y: 21.0,
    });
    sim_until(&rig.sim, "the up", |r| r.iter().any(|r| r["phase"] == "up")).await;
}

#[tokio::test]
async fn a_closed_socket_closes_its_editor_through_the_guard() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = rig(dir.path());
    let _outbox = held_open(&mut rig).await;
    rig.router.handle(RouterMsg::EqInput {
        client: 1,
        touch: Touch::Down,
        x: 5.0,
        y: 5.0,
    });
    rig.router.handle(RouterMsg::Detach { client: 1 });
    let closed = next_of(&mut rig.rx, |m| matches!(m, RouterMsg::EqClosed { .. })).await;
    let RouterMsg::EqClosed { ref problem, .. } = closed else {
        unreachable!()
    };
    // The guard tapped; the offline instance could not be read again, so
    // Live is left alone.
    assert_eq!(
        problem.as_deref(),
        Some(close::unread("instance offline").as_str())
    );
    rig.router.handle(closed);
    let phases: Vec<(String, i64, i64)> = rig
        .sim
        .records()
        .iter()
        .filter(|r| r["op"] == "touch")
        // The window worker's keep-alive re-sends the resting point (50 ms).
        .filter(|r| r["phase"] != "update")
        .map(|r| {
            (
                r["phase"].as_str().unwrap().to_string(),
                r["x"].as_i64().unwrap(),
                r["y"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        phases,
        [
            ("down".to_string(), 5, 5),
            ("up".to_string(), 5, 5),
            ("down".to_string(), 546, 15),
            ("up".to_string(), 546, 15),
        ]
    );
    assert_eq!(
        rig.sim.records().last().unwrap()["op"],
        "release",
        "released after the guard"
    );
    assert_eq!(
        whats(&rig.records),
        vec!["touch", "close", "problem", "closed"]
    );
}

#[tokio::test]
async fn the_close_sequence_leaves_an_editor_open_when_its_guard_fails() {
    let (plugwin, sim) = sim_plugwin();
    let live = offline_live();
    let (events, _records) = EventLog::channel(64);
    // No instance: the guard taps, and nothing can close it.
    plugwin.take(1, Vec::new()).await.unwrap();
    assert_eq!(
        close_editor(None, plugwin.clone(), key(), 1, None, events.clone()).await,
        Some(UNKNOWN_INSTANCE.to_string())
    );
    // The guard cannot tap: the editor stays open (not even asked).
    plugwin.take(2, Vec::new()).await.unwrap();
    sim.refuse(true);
    assert_eq!(
        close_editor(
            Some(live.clone()),
            plugwin.clone(),
            key(),
            2,
            None,
            events.clone()
        )
        .await,
        Some(format!(
            "the guard failed, the editor stays open: {REFUSED}"
        ))
    );
    let released = sim
        .records()
        .iter()
        .filter(|r| r["op"] == "release")
        .count();
    assert_eq!(released, 2, "both windows handed back");
    assert_eq!(sim.windows().len(), 2, "both editors still open in Live");
    // Opens: no instance, no listed name, an offline instance (its read).
    let name = || Some("Pro-Q 4".to_string());
    assert_eq!(
        open_editor(None, plugwin.clone(), key(), name(), 3, events.clone()).await,
        Err(UNKNOWN_INSTANCE.to_string())
    );
    assert_eq!(
        open_editor(
            Some(live.clone()),
            plugwin.clone(),
            key(),
            None,
            3,
            events.clone()
        )
        .await,
        Err(reason::UNKNOWN.to_string())
    );
    assert_eq!(
        open_editor(
            Some(live.clone()),
            plugwin.clone(),
            key(),
            name(),
            3,
            events.clone()
        )
        .await,
        Err("instance offline".to_string())
    );
    assert_eq!(
        set_editor(&live, PATH, true).await,
        Err("instance offline".to_string())
    );
    assert_eq!(
        walk(live, PATH.into()).await,
        Err("instance offline".to_string())
    );
}

#[tokio::test]
async fn a_close_whose_ref_fails_checks_the_held_path() {
    let (plugwin, _sim) = sim_plugwin();
    let (events, _records) = EventLog::channel(64);
    plugwin.take(1, Vec::new()).await.unwrap();
    // The ref's turn-off fails (here the instance is offline): the close
    // check reads the held path, which fails too, so Live is left alone.
    let reference = Some(json!({"$ref": "live_1", "class": "PluginDevice"}));
    assert_eq!(
        close_editor(Some(offline_live()), plugwin, key(), 1, reference, events).await,
        Some(close::unread("instance offline"))
    );
}

#[tokio::test]
async fn an_opened_editors_ref_is_kept_for_its_close_only() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = rig(dir.path());
    let _outbox = attach(&mut rig.router, 1);
    let io = rig.router.eq.as_mut().unwrap();
    io.state.listed("band", "live_set tracks 1", [found_key()]);
    io.state.open(1, &key(), 0.0);
    let reference = json!({"$ref": "live_1", "class": "PluginDevice"});
    // An answer of another session keeps none.
    rig.router.handle(RouterMsg::EqOpened {
        key: key(),
        session: 9,
        outcome: Ok((1349, 809)),
        reference: Some(reference.clone()),
    });
    assert!(rig.router.eq.as_ref().unwrap().refs.is_empty());
    rig.router.handle(RouterMsg::EqOpened {
        key: key(),
        session: 1,
        outcome: Ok((1349, 809)),
        reference: Some(reference.clone()),
    });
    assert_eq!(
        rig.router.eq.as_ref().unwrap().refs.get(&1),
        Some(&("band".to_string(), reference))
    );
    // Its close's end forgets it.
    rig.router.handle(RouterMsg::EqClosed {
        key: key(),
        session: 1,
        problem: None,
    });
    assert!(rig.router.eq.as_ref().unwrap().refs.is_empty());
}

#[tokio::test]
async fn the_workers_events_free_a_lost_editor_and_are_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = rig(dir.path());
    let outbox = held_open(&mut rig).await;
    rig.router.eq_worker(PlugwinEvent::Lost { session: 9 });
    assert!(eq_msgs(&outbox).is_empty(), "not a session given out");
    rig.router.eq_worker(PlugwinEvent::ContactEnded {
        session: 1,
        contact: 1,
        why: REFUSED.into(),
    });
    rig.router.eq_worker(PlugwinEvent::Rate {
        session: 1,
        rate: Rate::default(),
    });
    rig.router.eq_worker(PlugwinEvent::Lost { session: 1 });
    assert_eq!(
        eq_msgs(&outbox),
        vec![
            crate::eq::closed_msg(&key(), reason::GONE, None),
            ServerMsg::EqLocks { items: Vec::new() },
        ]
    );
    assert_eq!(whats(&rig.records), vec!["contact_ended", "rate", "lost"]);
}

#[tokio::test]
async fn the_stop_hands_the_windows_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut rig = rig(dir.path());
    let _outbox = held_open(&mut rig).await;
    rig.router.eq_stop();
    sim_until(&rig.sim, "the release", |r| {
        r.iter().any(|r| r["op"] == "release")
    })
    .await;
    assert_eq!(
        rig.plugwin.list().await,
        Err(crate::plugwin::STOPPED.into())
    );
}

#[tokio::test]
async fn a_hub_without_the_screen_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let (tx, _rx) = mpsc::unbounded_channel();
    let mut router = Router::new(
        BTreeMap::new(),
        false,
        dir.path().to_path_buf(),
        RouterIo {
            tx,
            events: EventLog::off(),
        },
    );
    let outbox = attach(&mut router, 1);
    assert!(eq_msgs(&outbox).is_empty(), "no locks");
    router.handle(RouterMsg::EqList {
        client: 1,
        binding: hand2(),
    });
    router.handle(RouterMsg::EqOpen {
        client: 1,
        instance: "band".into(),
        path: PATH.into(),
    });
    router.handle(RouterMsg::EqInput {
        client: 1,
        touch: Touch::Down,
        x: 1.0,
        y: 1.0,
    });
    router.handle(RouterMsg::EqClose { client: 1 });
    router.handle(RouterMsg::Tick);
    router.handle(RouterMsg::EqWorker {
        event: PlugwinEvent::Lost { session: 1 },
    });
    router.handle(RouterMsg::EqClosed {
        key: key(),
        session: 1,
        problem: None,
    });
    router.handle(RouterMsg::EqOpened {
        key: key(),
        session: 1,
        outcome: Ok((1, 1)),
        reference: Some(json!({"$ref": "live_1"})),
    });
    router.handle(RouterMsg::EqListed {
        client: 1,
        binding: hand2(),
        outcome: Ok(Vec::new()),
    });
    assert_eq!(
        eq_msgs(&outbox),
        vec![
            ServerMsg::EqList {
                binding: hand2(),
                items: Vec::new(),
                error: Some(reason::OFF.to_string()),
            },
            crate::eq::closed_msg(&key(), reason::OFF, None),
        ]
    );
    router.eq_stop();
}
