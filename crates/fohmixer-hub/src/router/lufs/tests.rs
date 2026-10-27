use super::*;
use serde_json::json;

fn lists() -> BTreeMap<String, Value> {
    BTreeMap::from([
        (
            hub_key("band", "live_set", "tracks", false),
            json!([
                {"$ref": "1", "path": "live_set tracks 0", "class": "Track", "name": "Hand1 #"},
                {"$ref": "2", "path": "live_set tracks 1", "class": "Track", "name": "Vox 1"},
            ]),
        ),
        (
            hub_key("band", "live_set", "return_tracks", false),
            json!([{"$ref": "3", "path": "live_set return_tracks 0", "class": "Track", "name": "A-Reverb #"}]),
        ),
    ])
}

#[test]
fn the_constants() {
    assert_eq!(LEVELS_CLIENT, ClientId::MAX - 5);
    assert_eq!(LUFS_TICK, Duration::from_millis(100));
}

#[test]
fn a_points_track_is_named_from_its_anchor_or_the_hubs_lists() {
    let masters = BTreeMap::from([("band".to_string(), "Main".to_string())]);
    let name = |anchor: Anchor| track_name(&lists(), &masters, "band", &anchor);
    assert_eq!(name(Anchor::TrackAt { index: 1 }).as_deref(), Some("Vox 1"));
    assert_eq!(name(Anchor::TrackAt { index: 2 }), None);
    assert_eq!(
        name(Anchor::ReturnAt { index: 0 }).as_deref(),
        Some("A-Reverb #")
    );
    assert_eq!(name(Anchor::ReturnAt { index: 1 }), None);
    assert_eq!(
        name(Anchor::Track {
            name: "Vocals".into()
        })
        .as_deref(),
        Some("Vocals")
    );
    assert_eq!(
        name(Anchor::Return {
            name: "PA Ret".into()
        })
        .as_deref(),
        Some("PA Ret")
    );
    assert_eq!(name(Anchor::Master).as_deref(), Some("Main"));
    assert_eq!(name(Anchor::Song), None);
    // Another instance's lists name nothing here.
    assert_eq!(
        track_name(&lists(), &masters, "master", &Anchor::TrackAt { index: 1 }),
        None
    );
    assert_eq!(
        track_name(&lists(), &masters, "master", &Anchor::Master),
        None
    );
}

#[test]
fn an_item_without_a_name_names_nothing() {
    assert_eq!(item_name(&json!([{"$ref": "1"}]), 0), None);
    assert_eq!(item_name(&json!("not a list"), 0), None);
    assert_eq!(item_name(&json!([{"name": "A"}]), 0).as_deref(), Some("A"));
}

#[test]
fn mute_reads_a_bool_or_a_number() {
    assert_eq!(mute_of(&json!(true)), Some(true));
    assert_eq!(mute_of(&json!(false)), Some(false));
    assert_eq!(mute_of(&json!(1)), Some(true));
    assert_eq!(mute_of(&json!(0)), Some(false));
    assert_eq!(mute_of(&json!("on")), None);
}

#[test]
fn the_names_read_answers_both_names_or_why_not() {
    let ok = |name: &str| json!({"ok": true, "data": name});
    assert_eq!(
        names_of(&[ok("FOH Band"), ok("Main")]),
        Ok(("FOH Band".to_string(), "Main".to_string()))
    );
    assert!(names_of(&[ok("FOH Band")]).is_err());
    assert!(names_of(&[json!({"ok": false, "error": "x"}), ok("Main")]).is_err());
    assert!(names_of(&[json!({"ok": true, "data": 3}), ok("Main")]).is_err());
}

#[test]
fn a_names_read_is_tried_again_only_when_live_did_not_answer() {
    assert!(read_again("instance offline", 1));
    assert!(read_again("no result within 3 s", CONNECT_RETRIES));
    assert!(!read_again("no result within 3 s", CONNECT_RETRIES + 1));
    assert!(!read_again("not found: live_set", 1));
}

#[test]
fn a_point_follows_volume_pan_mute_and_an_input_its_arm() {
    let t = "live_set tracks 1";
    let out = follows(t, false);
    assert_eq!(
        out,
        vec![
            (
                "live_set tracks 1 mixer_device volume".to_string(),
                "value",
                Part::Volume
            ),
            (
                "live_set tracks 1 mixer_device panning".to_string(),
                "value",
                Part::Pan
            ),
            ("live_set tracks 1".to_string(), "mute", Part::Mute),
        ]
    );
    let input = follows(t, true);
    assert_eq!(input.len(), 4);
    assert_eq!(input[..3], out[..]);
    assert_eq!(
        input[3],
        ("live_set tracks 1".to_string(), "arm", Part::Arm)
    );
}

#[test]
fn each_part_lands_in_its_field() {
    let mut fader = Fader::default();
    heard(&mut fader, Part::Volume, Some(&json!(0.6)));
    heard(&mut fader, Part::Pan, Some(&json!(-0.5)));
    heard(&mut fader, Part::Mute, Some(&json!(true)));
    heard(&mut fader, Part::Arm, Some(&json!(1)));
    assert_eq!(
        fader,
        Fader {
            volume: Some(0.6),
            mute: Some(true),
            pan: Some(-0.5),
            arm: Some(true),
        }
    );
    heard(&mut fader, Part::Arm, Some(&json!(false)));
    assert_eq!(fader.arm, Some(false));
    heard(&mut fader, Part::Pan, None);
    heard(&mut fader, Part::Volume, Some(&json!("x")));
    assert_eq!(
        fader,
        Fader {
            volume: None,
            mute: Some(true),
            pan: None,
            arm: Some(false),
        }
    );
}

#[test]
fn a_points_meter_keys_carry_its_levels() {
    let keys = meter_keys("band", "live_set tracks 1", (0.25, 0.5));
    assert_eq!(
        keys,
        vec![
            (
                "band|live_set tracks 1|output_meter_level|false".to_string(),
                0.5
            ),
            (
                "band|live_set tracks 1|output_meter_level|true".to_string(),
                0.5
            ),
            (
                "band|live_set tracks 1|output_meter_left|false".to_string(),
                0.25
            ),
            (
                "band|live_set tracks 1|output_meter_left|true".to_string(),
                0.25
            ),
            (
                "band|live_set tracks 1|output_meter_right|false".to_string(),
                0.5
            ),
            (
                "band|live_set tracks 1|output_meter_right|true".to_string(),
                0.5
            ),
        ]
    );
    assert!(
        keys.iter()
            .all(|(k, _)| fohmixer_proto::client::is_meter_key(k))
    );
}

#[test]
fn a_meter_keys_source_change_replaces_what_a_paced_page_holds() {
    assert!(replaces(true, false), "the levels start");
    assert!(!replaces(true, true), "the levels move on");
    assert!(replaces(false, true), "the levels end");
    assert!(replaces(false, false));
}

#[tokio::test(start_paused = true)]
async fn the_first_tick_comes_one_period_after_the_start_then_every_period() {
    // PR #155's mutation gate: the interval starts one period from now, never
    // at once (a `-` there ticks at the start and then on the period).
    let (tx, mut rx) = mpsc::unbounded_channel();
    let ticks = tokio::spawn(lufs_ticks(tx, Duration::from_millis(100)));
    tokio::time::sleep(Duration::from_millis(99)).await;
    assert!(rx.try_recv().is_err(), "no tick before the first period");
    tokio::time::sleep(Duration::from_millis(2)).await;
    assert!(
        matches!(rx.try_recv(), Ok(RouterMsg::LufsTick)),
        "the first tick at 100 ms"
    );
    assert!(rx.try_recv().is_err(), "one tick only");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        matches!(rx.try_recv(), Ok(RouterMsg::LufsTick)),
        "the next tick a period later"
    );
    drop(rx);
    let ended = tokio::time::timeout(Duration::from_millis(250), ticks).await;
    assert!(ended.is_ok(), "the ticks end with the router's channel");
}

// --- PR #155's mutation gate: the glue on a whole router ---

use std::collections::BTreeSet;

use fohmixer_proto::client::{LufsItem, ServerMsg};
use fohmixer_proto::layout::LufsSpec;

use super::super::MARKERS_CLIENT;
use super::super::tests::{attach, next_of, offline_router};
use crate::live::client::LiveError;

/// A band loudness point on `target` (an input point when `input`).
fn point(target: &str, anchor: Anchor, input: bool) -> LufsPoint {
    LufsPoint {
        instance: "band".into(),
        anchor,
        target: target.into(),
        spec: LufsSpec {
            target: input.then_some(-25),
        },
        label: "Vox 1".into(),
    }
}

/// Live's answer to each pending listener: `answer(prop)` the slot.
fn answer(router: &mut Router, answer: impl Fn(&str) -> Value) {
    for o in router.subs.drain_outgoing() {
        let slots: Vec<Value> = o
            .commands
            .iter()
            .map(|c| answer(c["args"]["prop"].as_str().unwrap_or("")))
            .collect();
        router.subs.on_result(&o.instance, &o.uuid, &slots);
    }
}

/// A listener's answer: Live's key for it and its value.
fn slot(key: &str, value: Value) -> Value {
    json!({"ok": true, "data": {"key": key, "value": value}})
}

#[tokio::test]
async fn a_kept_read_stays_until_it_ends_and_an_abort_ends_every_one() {
    let mut io = LufsIo::default();
    let done = tokio::spawn(async {});
    let done_handle = done.abort_handle();
    done.await.unwrap();
    io.keep("band", done_handle);
    let first = tokio::spawn(std::future::pending::<()>());
    io.keep("band", first.abort_handle());
    assert_eq!(io.tasks["band"].len(), 1, "the finished read let go");
    let second = tokio::spawn(std::future::pending::<()>());
    io.keep("band", second.abort_handle());
    assert_eq!(io.tasks["band"].len(), 2);
    io.abort("band");
    assert!(io.tasks.is_empty());
    for task in [first, second] {
        let ended = tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("an aborted read ends");
        assert!(ended.unwrap_err().is_cancelled());
    }
}

#[tokio::test]
async fn a_layout_without_a_point_lets_its_fader_go() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, _records) = offline_router(dir.path());
    router.lufs_layout(vec![
        point("live_set tracks 1", Anchor::TrackAt { index: 1 }, false),
        point("live_set tracks 2", Anchor::TrackAt { index: 2 }, false),
    ]);
    assert_eq!(router.lufs.watched.len(), 6);
    router.lufs_layout(vec![point(
        "live_set tracks 1",
        Anchor::TrackAt { index: 1 },
        false,
    )]);
    let points: BTreeSet<&str> = router
        .lufs
        .watched
        .values()
        .map(|(_, point, _)| point.as_str())
        .collect();
    assert_eq!(points, BTreeSet::from(["live_set tracks 1"]));
    assert_eq!(router.lufs.watched.len(), 3);
    let gone = hub_key(
        "band",
        "live_set tracks 2 mixer_device volume",
        "value",
        false,
    );
    assert!(router.subs.clients_of(&gone).is_empty());
    let kept = hub_key(
        "band",
        "live_set tracks 1 mixer_device volume",
        "value",
        false,
    );
    assert_eq!(router.subs.clients_of(&kept), vec![LEVELS_CLIENT]);
}

#[tokio::test]
async fn a_new_layout_takes_a_points_fader_the_table_already_holds() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, _records) = offline_router(dir.path());
    let _page = attach(&mut router, 1);
    // A page follows the track's volume before any layout names the point.
    router.handle(RouterMsg::Sub {
        client: 1,
        instance: "band".into(),
        target: "live_set tracks 1 mixer_device volume".into(),
        prop: "value".into(),
        display: false,
    });
    router.subs.connected("band");
    answer(&mut router, |prop| match prop {
        "value" => slot("live_1.value", json!(0.85)),
        _ => json!({"ok": true, "data": null}),
    });
    router.lufs_layout(vec![point(
        "live_set tracks 1",
        Anchor::TrackAt { index: 1 },
        false,
    )]);
    let fader = router
        .lufs
        .faders
        .get(&("band".to_string(), "live_set tracks 1".to_string()))
        .copied();
    assert_eq!(fader.and_then(|f| f.volume), Some(0.85));
}

#[tokio::test]
async fn a_flush_that_hears_a_track_list_names_the_points_tracks_again() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, _records) = offline_router(dir.path());
    router.lufs_layout(vec![point(
        "live_set tracks 1",
        Anchor::TrackAt { index: 1 },
        true,
    )]);
    assert_eq!(router.lufs.rule.strips()[0].track, None);
    router
        .subs
        .subscribe(MARKERS_CLIENT, "band", "live_set", TRACKS, false)
        .unwrap();
    router.subs.connected("band");
    answer(&mut router, |prop| match prop {
        "tracks" => slot(
            "live_1.tracks",
            json!([
                {"$ref": "1", "path": "live_set tracks 0", "class": "Track", "name": "Hand1 #"},
                {"$ref": "2", "path": "live_set tracks 1", "class": "Track", "name": "Vox 1"},
            ]),
        ),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
    assert_eq!(router.lufs.rule.strips()[0].track.as_deref(), Some("Vox 1"));
}

#[tokio::test]
async fn the_names_read_names_the_master_point_and_tells_the_rule() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, _records) = offline_router(dir.path());
    router.lufs_layout(vec![point("live_set master_track", Anchor::Master, false)]);
    assert_eq!(router.lufs.rule.strips()[0].track, None);
    router.lufs.reading.insert("band".into(), 4);
    // An older read's answer is void.
    router.lufs_named("band", 3, Ok(("FOH Set".into(), "Old".into())));
    assert!(router.lufs.masters.is_empty());
    router.lufs_named("band", 4, Ok(("FOH Set".into(), "Main".into())));
    assert_eq!(
        router.lufs.masters.get("band").map(String::as_str),
        Some("Main")
    );
    assert_eq!(router.lufs.rule.strips()[0].track.as_deref(), Some("Main"));
}

#[tokio::test]
async fn a_connect_reads_the_names_under_a_new_number_each_time() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut rx, _records) = offline_router(dir.path());
    router.lufs_connected("band");
    assert_eq!(router.lufs.reads, 1);
    assert_eq!(router.lufs.reading.get("band"), Some(&1));
    assert_eq!(router.lufs.failures.get("band"), Some(&0));
    let named = next_of(&mut rx, "the names' answer", |m| {
        matches!(m, RouterMsg::LufsNamed { .. })
    })
    .await;
    let RouterMsg::LufsNamed {
        instance,
        read,
        outcome,
    } = named
    else {
        unreachable!()
    };
    assert_eq!((instance.as_str(), read), ("band", 1));
    assert!(outcome.is_err(), "Live is offline");
    router.lufs_connected("band");
    assert_eq!(router.lufs.reads, 2);
    assert_eq!(router.lufs.reading.get("band"), Some(&2));
}

#[tokio::test]
async fn a_names_read_live_did_not_answer_is_counted_and_tried_again() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut rx, _records) = offline_router(dir.path());
    router.lufs.reading.insert("band".into(), 7);
    let offline = LiveError::Offline.to_string();
    router.lufs_named("band", 7, Err(offline.clone()));
    assert_eq!(router.lufs.failures.get("band"), Some(&1));
    let retry = next_of(&mut rx, "the names' retry", |m| {
        matches!(m, RouterMsg::LufsNamesRetry { .. })
    })
    .await;
    let RouterMsg::LufsNamesRetry { instance, read } = retry else {
        unreachable!()
    };
    assert_eq!((instance.as_str(), read), ("band", 7));
    router.lufs_named("band", 7, Err(offline));
    assert_eq!(router.lufs.failures.get("band"), Some(&2));
}

#[tokio::test]
async fn a_names_retry_reads_again_only_while_online_and_still_the_newest() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, _records) = offline_router(dir.path());
    router.lufs.reading.insert("band".into(), 3);
    // Offline: nothing is read.
    router.lufs_names_retry("band", 3);
    assert_eq!(router.lufs.reads, 0);
    router.states.get_mut("band").unwrap().online = true;
    // Another read started since: void.
    router.lufs_names_retry("band", 2);
    assert_eq!(router.lufs.reads, 0);
    router.lufs_names_retry("band", 3);
    assert_eq!(router.lufs.reads, 1);
    assert_eq!(router.lufs.reading.get("band"), Some(&1));
}

#[tokio::test]
async fn an_offline_live_drops_its_own_points_faders_and_names_only() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, _records) = offline_router(dir.path());
    let t = "live_set tracks 1".to_string();
    router
        .lufs
        .faders
        .insert(("band".to_string(), t.clone()), Fader::default());
    router
        .lufs
        .faders
        .insert(("master".to_string(), t.clone()), Fader::default());
    router.lufs.reading.insert("band".into(), 1);
    router.lufs.masters.insert("band".into(), "Main".into());
    router.lufs_offline("band");
    assert_eq!(
        router.lufs.faders.keys().cloned().collect::<Vec<_>>(),
        vec![("master".to_string(), t)]
    );
    assert!(router.lufs.reading.is_empty());
    assert!(router.lufs.masters.is_empty());
}

#[tokio::test]
async fn a_page_attached_gets_every_points_item() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, _records) = offline_router(dir.path());
    router.lufs_layout(vec![point(
        "live_set tracks 1",
        Anchor::TrackAt { index: 1 },
        true,
    )]);
    let page = attach(&mut router, 1);
    let items: Vec<LufsItem> = page
        .take()
        .unwrap()
        .into_iter()
        .flat_map(|m| match m {
            ServerMsg::Lufs { items } => items,
            _ => Vec::new(),
        })
        .collect();
    assert_eq!(
        items,
        vec![LufsItem {
            instance: "band".into(),
            target: "live_set tracks 1".into(),
            tenths: None,
            problem: None,
        }]
    );
}
