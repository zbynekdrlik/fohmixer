use std::collections::BTreeMap;
use std::sync::Arc;

use fohmixer_proto::client::{
    AutotuneState, AutotuneTargetStatus, Decision, HUB_AUTOTUNE_AUT, HUB_STAGE_AUT,
    KeyTargetStatus, ServerMsg, SwitchWhy, TargetState,
};
use fohmixer_proto::layout::{Anchor, AutotuneTarget, Binding, Layout};
use serde_json::{Value, json};

use tokio::sync::mpsc::UnboundedReceiver;

use super::super::retries::{CONNECT_RETRIES, RETRY_MS};
use super::super::tests::{attach, next_of, offline_router, records, status};
use super::super::{Router, RouterMsg, layout_msg, markers};
use super::{
    AUTOTUNE_CLIENT, AfterWalk, MOVED_TRACK, PARAMETER_CLASS, TOO_DEEP, Target, TrackCheck,
    after_walk, autotune_fields, gave_up_problem, item_ref, keeps_found, macro_value, may_write,
    missing_news, same_track, slot_failures, state_of, target_key, track_check, walk_again,
    walk_retryable, write_commands,
};
use crate::live::LiveValue;
use crate::live::client::LiveEvent;
use crate::live::client::{Asked, Script};
use crate::outbox::Outbox;
use crate::rules::autotune::Step;
use crate::rules::autotune::find::Found;
use crate::rules::autotune::keys::KeyMissing;
use crate::rules::autotune::switch::SelectorGuard;

/// The repro target (on 0) and the stream target (on 127) of the band.
const REPRO: &str = "band|live_set tracks 6";
const STREAM: &str = "band|live_set tracks 7";
const PLAIN: &str = "band|live_set tracks 8";
const TRANSPORT: &str = "band|live_set|is_playing|false";

fn target(index: u32, label: &str, on: u8, off: u8) -> AutotuneTarget {
    AutotuneTarget {
        binding: Binding {
            instance: "band".into(),
            anchor: Anchor::TrackAt { index },
            path: None,
        },
        label: label.into(),
        on,
        off,
        singer: None,
        two_singers: false,
    }
}

fn repro() -> AutotuneTarget {
    target(6, "Vox 2", 0, 127)
}

fn stream() -> AutotuneTarget {
    target(7, "#8", 127, 0)
}

fn layout(rev: u64, autotune: Vec<AutotuneTarget>) -> RouterMsg {
    RouterMsg::Layout {
        rev,
        stage: None,
        targets: BTreeMap::new(),
        strips: Vec::new(),
        reverb: Vec::new(),
        fader_leaders: Vec::new(),
        autotune,
    }
}

/// The rack's first macro of band track `index`, as a find gives it.
fn found(index: u32) -> Found {
    Found {
        path: format!("live_set tracks {index} devices 0 parameters 1"),
        id: format!("p{index}"),
        key: Err(KeyMissing::NoChain),
    }
}

/// The band treated as connected (a walk then goes out; the offline router
/// answers it `instance offline`).
fn online(router: &mut Router) {
    router.states.get_mut("band").unwrap().online = true;
}

fn walk_of(router: &Router, key: &str) -> u64 {
    router.autotune.targets[key].walk
}

/// A target's newest walk answers with its macro.
fn find(router: &mut Router, key: &str, index: u32) {
    let walk = walk_of(router, key);
    router.autotune_found(key, walk, Ok(found(index)));
}

/// The band's transport heard, as the flush hands it over.
fn transport(router: &mut Router, playing: bool) {
    router.on_transport(TRANSPORT, &json!(playing));
}

fn set_hub(router: &mut Router, key: &str, value: Value) {
    router.handle(RouterMsg::SetHub {
        client: 1,
        key: key.into(),
        value,
    });
}

/// The `autotune` records waiting, without `ev` and `ts`.
fn steps(rx: &std::sync::mpsc::Receiver<Value>) -> Vec<Value> {
    records(rx)
        .into_iter()
        .filter(|r| r["ev"] == "autotune")
        .map(|mut r| {
            let fields = r.as_object_mut().unwrap();
            fields.remove("ev");
            fields.remove("ts");
            r
        })
        .collect()
}

/// The `autotune` messages waiting in `outbox`.
fn told(outbox: &Outbox) -> Vec<AutotuneState> {
    outbox
        .take()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| match m {
            ServerMsg::Autotune(s) => Some(s),
            _ => None,
        })
        .collect()
}

fn states(state: &AutotuneState) -> Vec<(String, TargetState)> {
    state
        .targets
        .iter()
        .map(|t| (t.label.clone(), t.state))
        .collect()
}

/// Live answers every listener of the band waiting (the table online):
/// the transport `playing`, each macro `value`.
fn answer(router: &mut Router, playing: bool, value: f64) {
    router.subs.connected("band");
    for o in router.subs.drain_outgoing() {
        let slots: Vec<Value> = o
            .commands
            .iter()
            .map(|c| match c["args"]["prop"].as_str().unwrap_or("") {
                "is_playing" => {
                    json!({"ok": true, "data": {"key": "live_1.is_playing", "value": playing}})
                }
                "value" => json!({"ok": true, "data": {"key": "live_9.value", "value": value}}),
                _ => json!({"ok": true, "data": null}),
            })
            .collect();
        router.subs.on_result(&o.instance, &o.uuid, &slots);
    }
}

#[test]
fn write_commands_set_each_parameter_through_its_ref_in_one_batch() {
    assert_eq!(PARAMETER_CLASS, "DeviceParameter");
    assert_eq!(
        write_commands(&[("p1", 127), ("p2", 0)]),
        vec![
            json!({"target": {"$ref": "p1", "class": "DeviceParameter"}, "name": "set_prop",
                   "args": {"prop": "value", "value": 127.0}}),
            json!({"target": {"$ref": "p2", "class": "DeviceParameter"}, "name": "set_prop",
                   "args": {"prop": "value", "value": 0.0}}),
        ]
    );
    assert!(write_commands(&[]).is_empty());
    // Each target its own value: the repro racks are on at 0.
    assert_eq!(macro_value(&repro(), true), 0);
    assert_eq!(macro_value(&repro(), false), 127);
    assert_eq!(macro_value(&stream(), true), 127);
    assert_eq!(macro_value(&stream(), false), 0);
}

#[test]
fn slot_failures_name_each_target_that_did_not_answer_ok() {
    let keys = [REPRO.to_string(), STREAM.to_string()];
    let ok = json!({"ok": true, "data": null});
    let failure = |key: &str, why: &str| (key.to_string(), why.to_string());
    assert_eq!(
        slot_failures(
            &Ok(vec![
                ok.clone(),
                json!({"ok": false, "error": "stale ref", "errorType": "StaleRef"})
            ]),
            &keys
        ),
        vec![failure(STREAM, "stale ref")]
    );
    assert_eq!(
        slot_failures(&Ok(vec![ok.clone(), ok.clone()]), &keys),
        vec![]
    );
    assert_eq!(
        slot_failures(&Ok(vec![ok.clone()]), &keys),
        vec![failure(STREAM, "no result")]
    );
    assert_eq!(
        slot_failures(&Ok(vec![json!({"ok": false}), ok]), &keys),
        vec![failure(REPRO, "no answer")]
    );
    assert_eq!(
        slot_failures(&Err("instance offline".into()), &keys),
        vec![
            failure(REPRO, "instance offline"),
            failure(STREAM, "instance offline")
        ]
    );
}

#[test]
fn the_glues_small_decisions() {
    assert_eq!(AUTOTUNE_CLIENT, u64::MAX - 3);
    assert_eq!(TOO_DEEP, "the devices nest too deep");
    assert_eq!(target_key("band", "live_set tracks 6"), REPRO);
    assert!(missing_news(None, "no rack named Latencies"));
    assert!(missing_news(
        Some("2 racks named Latencies"),
        "no rack named Latencies"
    ));
    assert!(!missing_news(
        Some("no rack named Latencies"),
        "no rack named Latencies"
    ));
    // A set's writes go only with no hold and its markers settled.
    assert!(may_write(false, true));
    assert!(!may_write(true, true), "held");
    assert!(!may_write(false, false), "the markers not settled");
    assert!(!may_write(true, false));
    assert_eq!(
        autotune_fields(
            &Step::Singer {
                id: "VOX1".into(),
                off: true
            },
            Some(3),
            Some("10.0.0.3")
        ),
        json!({"step": "singer", "id": "VOX1", "off": true, "client": 3, "peer": "10.0.0.3"})
    );
    assert_eq!(
        autotune_fields(&Step::Flag { on: false }, None, None),
        json!({"step": "flag", "on": false})
    );
}

#[test]
fn a_targets_state_is_missing_waiting_or_its_macro_against_its_on_value() {
    let at = |found: Option<Found>, problem: Option<&str>, value: Option<f64>| Target {
        spec: stream(),
        instance: "band".into(),
        track: "live_set tracks 7".into(),
        walk: 1,
        found,
        problem: problem.map(str::to_string),
        sub: None,
        value,
        decision: None,
        guard: SelectorGuard::new(),
        writing: None,
        key: None,
        key_sub: None,
        scale_sub: None,
        values: (None, None),
        keyt: crate::rules::autotune::keys::KeyTarget::new(),
        key_write: 0,
        key_wrote: None,
        key_sure: false,
        walk_failures: 0,
    };
    assert_eq!(
        state_of(&at(None, Some("no rack named Latencies"), None)),
        TargetState::Missing
    );
    assert_eq!(state_of(&at(None, None, None)), TargetState::Waiting);
    assert_eq!(
        state_of(&at(Some(found(7)), None, None)),
        TargetState::Waiting,
        "no value heard yet"
    );
    assert_eq!(
        state_of(&at(Some(found(7)), None, Some(127.0))),
        TargetState::On
    );
    assert_eq!(
        state_of(&at(Some(found(7)), None, Some(0.0))),
        TargetState::Off
    );
}

#[test]
fn a_layout_msg_carries_the_autotune_targets() {
    let served: Layout = serde_json::from_value(json!({"schema": 2, "default_page": "p",
        "pages": [{"id": "p", "title": "P", "rail": [{"kind": "autotune", "targets": [
            {"binding": {"instance": "band", "anchor": {"kind": "track_at", "index": 7}},
             "label": "#8", "on": 127, "off": 0}]}]}]}))
    .unwrap();
    let RouterMsg::Layout { autotune, .. } = layout_msg(3, Some(&served)) else {
        panic!("a layout message")
    };
    assert_eq!(autotune, vec![stream()]);
    let RouterMsg::Layout { autotune, .. } = layout_msg(4, None) else {
        panic!("a layout message")
    };
    assert!(autotune.is_empty());
}

#[tokio::test]
async fn a_layout_walks_each_new_target_and_keeps_the_found_ones() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, _records) = offline_router(dir.path());
    online(&mut router);
    router.handle(layout(1, vec![repro(), stream()]));
    let (w6, w7) = (walk_of(&router, REPRO), walk_of(&router, STREAM));
    assert!(w6 > 0 && w7 > 0 && w6 != w7, "{w6} {w7}");
    find(&mut router, REPRO, 6);
    find(&mut router, STREAM, 7);
    assert_eq!(
        router.subs.subscriptions("band"),
        5,
        "the rules' transport, the band's playhead and tempo (#140) and both macros"
    );
    router.handle(layout(2, vec![repro(), target(8, "#9", 127, 0)]));
    assert_eq!(walk_of(&router, REPRO), w6, "kept: not walked again");
    assert_eq!(router.autotune.targets[REPRO].found, Some(found(6)));
    assert!(!router.autotune.targets.contains_key(STREAM));
    assert!(walk_of(&router, PLAIN) > w7);
    assert_eq!(
        router.subs.subscriptions("band"),
        4,
        "track 7's macro let go"
    );
    assert_eq!(router.autotune.order, vec![REPRO, PLAIN]);
    // A kept target takes the new layout's label and values.
    router.handle(layout(3, vec![target(6, "Vox 2 R", 0, 127)]));
    assert_eq!(router.autotune.targets[REPRO].spec.label, "Vox 2 R");
    assert_eq!(walk_of(&router, REPRO), w6);
}

#[tokio::test]
async fn a_target_of_an_offline_set_waits_for_its_connect() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut rx, _records) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    let early = tokio::time::timeout(std::time::Duration::from_millis(300), rx.recv()).await;
    assert!(
        !matches!(
            early,
            Ok(Some(
                RouterMsg::AutotuneFound { .. } | RouterMsg::AutotuneWalkFailed { .. }
            ))
        ),
        "no walk while the band is offline"
    );
    assert_eq!(
        status(&mut router).autotune.targets[0].state,
        TargetState::Waiting
    );
    online(&mut router);
    router.autotune_connected("band");
    // The offline instance's task answers no read: since #140 a failed
    // walk, never a definite answer.
    let walked = next_of(&mut rx, "the connect's walk", |m| {
        matches!(m, RouterMsg::AutotuneWalkFailed { .. })
    })
    .await;
    let RouterMsg::AutotuneWalkFailed { key, walk, why } = walked else {
        unreachable!()
    };
    assert_eq!(
        (key.as_str(), walk, why.as_str()),
        (STREAM, walk_of(&router, STREAM), "instance offline")
    );
}

#[tokio::test]
async fn a_found_target_is_followed_and_told_to_every_page() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, rx) = offline_router(dir.path());
    let page = attach(&mut router, 1);
    assert_eq!(
        told(&page),
        vec![AutotuneState {
            aut: false,
            playing: None,
            targets: Vec::new(),
            song: None,
            key: None,
            checking: false,
            singers: Vec::new(),
            map_problems: 0,
            map_changes: 0,
            map_keyed: 0,
            bar: None,
        }],
        "at attach"
    );
    router.handle(layout(1, vec![stream()]));
    let first = told(&page);
    assert_eq!(first.len(), 1);
    assert_eq!(
        states(&first[0]),
        vec![("#8".to_string(), TargetState::Waiting)]
    );
    find(&mut router, STREAM, 7);
    assert_eq!(
        steps(&rx),
        vec![
            json!({"step": "found", "instance": "band", "target": "live_set tracks 7",
                    "path": "live_set tracks 7 devices 0 parameters 1"}),
            json!({"step": "key_missing", "instance": "band", "target": "live_set tracks 7",
                    "problem": "the Latencies rack has no chain"}),
        ]
    );
    assert_eq!(told(&page), vec![], "still waiting for its value");
    // Live answers the listeners: the macro at 127, the transport stopped.
    // AUT off: the hub wants DRY, so its first value is written DRY (#140
    // PR 3b).
    answer(&mut router, false, 127.0);
    router.flush();
    assert_eq!(
        steps(&rx),
        vec![
            json!({"step": "write", "instance": "band", "cause": "found", "on": [], "off": ["#8"]})
        ]
    );
    let now = told(&page);
    assert_eq!(
        now.last().map(states),
        Some(vec![("#8".to_string(), TargetState::On)])
    );
    assert_eq!(now.last().unwrap().playing, Some(false));
    // A page attached later is told the state as it is.
    let second = attach(&mut router, 2);
    assert_eq!(told(&second), vec![router.autotune.state()]);
    assert_eq!(
        status(&mut router).autotune.targets,
        vec![AutotuneTargetStatus {
            instance: "band".into(),
            track: "live_set tracks 7".into(),
            label: "#8".into(),
            on: 127,
            state: TargetState::On,
            path: Some("live_set tracks 7 devices 0 parameters 1".into()),
            problem: None,
            key: Some(KeyTargetStatus {
                device: None,
                path: None,
                values: None,
                shows: None,
                problem: Some("the Latencies rack has no chain".into()),
                want: None,
                state: None,
            }),
            decision: Some(Decision {
                on: false,
                why: SwitchWhy::Aut,
            }),
        }]
    );
    // A later value off its on value: off.
    let sub = router.autotune.targets[STREAM].sub.clone().unwrap();
    router.autotune_delivery(&sub, Some(json!(16.0)), None);
    assert_eq!(
        told(&page).last().map(states),
        Some(vec![("#8".to_string(), TargetState::Off)])
    );
}

#[tokio::test]
async fn a_stale_walk_answer_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, rx) = offline_router(dir.path());
    online(&mut router);
    router.handle(layout(1, vec![stream()]));
    let older = walk_of(&router, STREAM);
    router.autotune_walk(STREAM);
    assert!(walk_of(&router, STREAM) > older);
    router.autotune_found(STREAM, older, Ok(found(7)));
    assert_eq!(router.autotune.targets[STREAM].found, None);
    assert_eq!(steps(&rx), Vec::<Value>::new());
    assert_eq!(
        router.subs.subscriptions("band"),
        3,
        "the transport and the playhead only"
    );
    find(&mut router, STREAM, 7);
    assert_eq!(router.autotune.targets[STREAM].found, Some(found(7)));
    assert_eq!(router.subs.subscriptions("band"), 4);
    // An answer for a key no layout names does nothing.
    router.autotune_found("band|live_set tracks 30", 1, Ok(found(30)));
    assert!(
        !router
            .autotune
            .targets
            .contains_key("band|live_set tracks 30")
    );
}

#[tokio::test]
async fn a_walk_answer_after_a_disconnect_is_void() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, rx) = offline_router(dir.path());
    online(&mut router);
    router.handle(layout(1, vec![repro(), stream()]));
    find(&mut router, REPRO, 6);
    let walk = walk_of(&router, STREAM);
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    // The refs died with the connection: the found one is dropped and
    // unsubscribed, the walk on its way voided.
    assert_eq!(router.autotune.targets[REPRO].found, None);
    assert_eq!(router.autotune.targets[REPRO].sub, None);
    assert_eq!(
        router.subs.subscriptions("band"),
        3,
        "the transport and the playhead only"
    );
    let _ = steps(&rx);
    router.autotune_found(STREAM, walk, Ok(found(7)));
    assert_eq!(router.autotune.targets[STREAM].found, None);
    assert_eq!(steps(&rx), Vec::<Value>::new());
    assert_eq!(router.autotune.state().targets.len(), 2);
    assert!(
        router
            .autotune
            .state()
            .targets
            .iter()
            .all(|t| t.state == TargetState::Waiting)
    );
}

#[tokio::test]
async fn a_failed_write_is_recorded_per_target_and_walks_it_again() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut rx, records_rx) = offline_router(dir.path());
    online(&mut router);
    router.handle(layout(1, vec![stream()]));
    find(&mut router, STREAM, 7);
    let before = walk_of(&router, STREAM);
    // AUT off and the stream's selector on its chain: DRY, to an offline
    // Live.
    answer(&mut router, false, 127.0);
    router.flush();
    let _ = steps(&records_rx);
    let written = next_of(&mut rx, "the write's answer", |m| {
        matches!(m, RouterMsg::AutotuneWritten { .. })
    })
    .await;
    router.handle(written);
    assert_eq!(
        steps(&records_rx),
        vec![
            json!({"step": "failed", "instance": "band", "target": "live_set tracks 7",
                    "problem": "instance offline"})
        ]
    );
    assert!(walk_of(&router, STREAM) > before, "walked again");
    assert_eq!(
        router.autotune.targets[STREAM].writing, None,
        "no write on its way"
    );
    // An answer for a target the layout dropped meanwhile is still recorded.
    router.autotune_written(
        "band",
        &["band|live_set tracks 30".to_string()],
        &Err("instance offline".into()),
    );
    assert_eq!(
        steps(&records_rx)[0]["target"],
        json!("band|live_set tracks 30")
    );
}

#[tokio::test]
async fn other_hub_keys_fall_through_to_stage_aut_and_its_error() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, _records) = offline_router(dir.path());
    let page = attach(&mut router, 1);
    page.take();
    // (A tap of any value is refused, no error: `keys_tests.rs`.)
    for (key, value) in [(HUB_AUTOTUNE_AUT, json!(1)), ("autotune_lol", json!(true))] {
        set_hub(&mut router, key, value);
        let replies = page.take().unwrap();
        assert!(
            replies
                .iter()
                .any(|m| matches!(m, ServerMsg::Error { message, .. }
                if message.starts_with("no hub value"))),
            "{key}: {replies:?}"
        );
    }
    set_hub(&mut router, HUB_STAGE_AUT, json!(true));
    let now = status(&mut router);
    assert!(now.stage_aut.on);
    assert!(!now.autotune.aut);
    assert_eq!(now.autotune.taps, 0);
}

#[tokio::test]
async fn status_leaves_the_rules_subscriptions_out_and_lists_each_target() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, rx) = offline_router(dir.path());
    router.handle(layout(1, vec![repro(), stream(), target(8, "#9", 127, 0)]));
    find(&mut router, REPRO, 6);
    let walk = walk_of(&router, PLAIN);
    router.autotune_found(PLAIN, walk, Err("no rack named Latencies".into()));
    let now = status(&mut router);
    assert_eq!(now.subscriptions["band"], 0, "the pages' only");
    assert_eq!(now.listeners["band"], 0, "the pages' only");
    let targets: Vec<_> = now
        .autotune
        .targets
        .iter()
        .map(|t| (t.track.clone(), t.state, t.path.clone(), t.problem.clone()))
        .collect();
    assert_eq!(
        targets,
        vec![
            (
                "live_set tracks 6".to_string(),
                TargetState::Waiting,
                Some("live_set tracks 6 devices 0 parameters 1".to_string()),
                None
            ),
            (
                "live_set tracks 7".to_string(),
                TargetState::Waiting,
                None,
                None
            ),
            (
                "live_set tracks 8".to_string(),
                TargetState::Missing,
                None,
                Some("no rack named Latencies".to_string())
            ),
        ]
    );
    assert_eq!(now.autotune.targets[0].on, 0);
    assert_eq!(now.autotune.targets[0].label, "Vox 2");
    assert_eq!(
        steps(&rx).last().unwrap(),
        &json!({"step": "missing", "instance": "band", "target": "live_set tracks 8",
                "problem": "no rack named Latencies"})
    );
    // The same problem again is no news; another one is.
    router.autotune_found(PLAIN, walk, Err("no rack named Latencies".into()));
    assert_eq!(steps(&rx), Vec::<Value>::new());
    router.autotune_found(PLAIN, walk, Err("2 racks named Latencies".into()));
    assert_eq!(steps(&rx).len(), 1);
    // Found after all: no problem any more.
    router.autotune_found(PLAIN, walk, Ok(found(8)));
    assert_eq!(router.autotune.targets[PLAIN].problem, None);
}

#[tokio::test]
async fn a_track_list_change_walks_the_sets_targets_again_and_keeps_what_they_found() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, _records) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    find(&mut router, STREAM, 7);
    let walk = walk_of(&router, STREAM);
    router.reverb_list("band|live_set|tracks|false", &json!([1, 2]));
    assert_eq!(walk_of(&router, STREAM), walk, "a first list is no change");
    router.reverb_list("band|live_set|tracks|false", &json!([1, 3, 2]));
    assert!(walk_of(&router, STREAM) > walk);
    assert_eq!(router.autotune.targets[STREAM].found, Some(found(7)));
}

#[tokio::test]
async fn a_lost_macro_drops_its_value_and_walks_it_again() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, _records) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    find(&mut router, STREAM, 7);
    let sub = router.autotune.targets[STREAM].sub.clone().unwrap();
    router.autotune_delivery(&sub, Some(json!(127.0)), None);
    assert_eq!(router.autotune.targets[STREAM].value, Some(127.0));
    router.autotune_delivery(&sub, Some(json!(127.0)), Some("gone".into()));
    assert_eq!(router.autotune.targets[STREAM].value, None);
    let walk = walk_of(&router, STREAM);
    router.autotune_delivery(&sub, None, Some("gone".into()));
    assert!(walk_of(&router, STREAM) > walk, "walked again");
    // Neither a value nor an error: nothing.
    router.autotune_delivery(&sub, None, None);
    assert_eq!(walk_of(&router, STREAM), walk + 1);
    // Another key: nothing.
    router.autotune_delivery("band|live_set|cue_points|false", Some(json!(120.0)), None);
    assert_eq!(router.autotune.targets[STREAM].value, None);
}

#[tokio::test]
async fn a_macro_a_page_already_follows_gives_a_found_target_its_value_at_once() {
    // A page follows the stream rack's macro already (Live answered it):
    // the target found at that path takes the table's value at once, with
    // no listener answer of its own to wait for.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, _records) = offline_router(dir.path());
    let page = attach(&mut router, 1);
    router.handle(layout(1, vec![stream()]));
    let path = found(7).path;
    assert!(
        router
            .subs
            .subscribe(1, "band", &path, "value", false)
            .is_ok()
    );
    answer(&mut router, false, 127.0);
    let _ = told(&page);
    find(&mut router, STREAM, 7);
    assert_eq!(router.autotune.targets[STREAM].value, Some(127.0));
    assert_eq!(
        told(&page).last().map(states),
        Some(vec![("#8".to_string(), TargetState::On)])
    );
}

#[tokio::test]
async fn every_walk_after_a_disconnect_has_a_new_number() {
    // A disconnect gives each target of the set a walk number none had
    // before, so no walk on its way from the old connection can match.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, _records) = offline_router(dir.path());
    online(&mut router);
    router.handle(layout(1, vec![repro(), stream()]));
    let before = [walk_of(&router, REPRO), walk_of(&router, STREAM)];
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    let after = [walk_of(&router, REPRO), walk_of(&router, STREAM)];
    let newest = before[0].max(before[1]);
    assert!(
        after.iter().all(|w| *w > newest),
        "{before:?} then {after:?}"
    );
    assert_ne!(after[0], after[1]);
    // The repro target's walk from before the disconnect is void.
    router.autotune_found(REPRO, before[0], Ok(found(6)));
    assert_eq!(router.autotune.targets[REPRO].found, None);
}

/// AUTOTUNE's two Tuner tags: the stream form (on 127) and the repro form
/// (on 0).
const STREAM_TAG: &str = "+AUTOTUNE";
const REPRO_TAG: &str = "+AUTOTUNE:0";
/// The targets of a marked band, by track index.
const AT0: &str = "band|live_set tracks 0";
const AT1: &str = "band|live_set tracks 1";
const AT2: &str = "band|live_set tracks 2";

/// Live's answer to a command: its data.
fn ok(data: Value) -> Value {
    json!({"ok": true, "data": data})
}

/// A listener's answer: Live's key for it and its value.
fn slot(key: &str, value: Value) -> Value {
    json!({"ok": true, "data": {"key": key, "value": value}})
}

/// Live's push of `value` under its listener key `key`.
fn pushed(key: &str, value: Value) -> LiveValue {
    LiveValue {
        key: key.into(),
        value,
        display: None,
        error: None,
    }
}

/// Live's answer to each pending listener of the band (the table online):
/// `answer(target, prop)` the slot.
fn answer_with(router: &mut Router, answer: impl Fn(&str, &str) -> Value) {
    for o in router.subs.drain_outgoing() {
        let slots: Vec<Value> = o
            .commands
            .iter()
            .map(|c| {
                answer(
                    c["target"].as_str().unwrap_or(""),
                    c["args"]["prop"].as_str().unwrap_or(""),
                )
            })
            .collect();
        router.subs.on_result(&o.instance, &o.uuid, &slots);
    }
}

/// The band's listeners answered (its track list `tracks`, no returns, Live
/// playing or not, nothing for any other) and flushed.
fn listened(router: &mut Router, tracks: &Value, playing: bool) {
    answer_with(router, |_, prop| match prop {
        "tracks" => slot("live_1.tracks", tracks.clone()),
        "return_tracks" => slot("live_1.returns", json!([])),
        "is_playing" => slot("live_1.is_playing", json!(playing)),
        _ => ok(Value::Null),
    });
    router.flush();
}

/// The marker keeper's next read: its number and batch, from the offline
/// answer its task sent the router (left unhandled: the read stays open).
async fn next_read(own: &mut UnboundedReceiver<RouterMsg>) -> (u64, markers::Step) {
    let read = next_of(own, "a read", |m| {
        matches!(m, RouterMsg::MarkersRead { .. })
    })
    .await;
    let RouterMsg::MarkersRead { seq, step, .. } = read else {
        unreachable!()
    };
    (seq, step)
}

/// Live's answer to the keeper's read `seq` of `n` band tracks, a Tuner
/// named `name` alone on each `(index, name)` track: both batches.
async fn read_answered(
    router: &mut Router,
    own: &mut UnboundedReceiver<RouterMsg>,
    seq: u64,
    n: u32,
    tuners: &[(u32, &str)],
) {
    let devices = (0..n)
        .map(|i| match tuners.iter().find(|(at, _)| *at == i) {
            Some((_, name)) => ok(json!([
                {"$ref": format!("d{i}"), "class": "Device", "name": name, "path": "p"}
            ])),
            None => ok(json!([])),
        })
        .collect();
    router.handle(RouterMsg::MarkersRead {
        instance: "band".into(),
        seq,
        step: markers::Step::Devices,
        outcome: Ok(devices),
    });
    let classes = next_read(own).await;
    assert_eq!(classes, (seq, markers::Step::Classes));
    router.handle(RouterMsg::MarkersRead {
        instance: "band".into(),
        seq,
        step: markers::Step::Classes,
        outcome: Ok(vec![ok(json!("Tuner")); tuners.len()]),
    });
}

/// The markers' quiet time ends: its message handled.
async fn settle(router: &mut Router, own: &mut UnboundedReceiver<RouterMsg>) {
    let msg = next_of(own, "the quiet time", |m| {
        matches!(m, RouterMsg::MarkersSettle { .. })
    })
    .await;
    router.handle(msg);
}

/// A walk of `key` answered: the macro on band track `index`, `$ref` `id`.
fn found_at(router: &mut Router, key: &str, index: u32, id: &str) {
    let walk = walk_of(router, key);
    router.autotune_found(
        key,
        walk,
        Ok(Found {
            path: format!("live_set tracks {index} devices 0 parameters 1"),
            id: id.into(),
            key: Err(KeyMissing::NoChain),
        }),
    );
}

/// The `write` steps among `got`.
fn writes(got: &[Value]) -> Vec<Value> {
    got.iter()
        .filter(|r| r["step"] == "write")
        .cloned()
        .collect()
}

/// The `held` steps among `got`.
fn holds(got: &[Value]) -> Vec<Value> {
    got.iter()
        .filter(|r| r["step"] == "held")
        .cloned()
        .collect()
}

/// A batch of the band's selectors written DRY: `labels`, `cause`.
fn dry_of(cause: &str, labels: &[&str]) -> Value {
    json!({"step": "write", "instance": "band", "cause": cause, "on": [], "off": labels})
}

/// Live's value of `key`'s selector reaches the hub, as the flush hands it.
fn value(router: &mut Router, key: &str, v: f64) {
    let sub = router.autotune.targets[key].sub.clone().unwrap();
    router.autotune_delivery(&sub, Some(json!(v)), None);
}

type Tagged = (
    Router,
    std::sync::mpsc::Receiver<Value>,
    tempfile::TempDir,
    UnboundedReceiver<RouterMsg>,
);

/// A router whose marker keeper follows the band (a frame with the
/// AUTOTUNE rail button): the stream rack's Tuner `+AUTOTUNE` on band
/// track 0 and the repro rack's `+AUTOTUNE:0` on track 1, composed from
/// the keeper's own read; both racks found (`p0`, `p1`), their values not
/// heard yet, AUTOTUNE AUT off (the hub wants DRY).
async fn tagged() -> Tagged {
    let dir = tempfile::tempdir().unwrap();
    let frame = json!({"schema": 2, "default_page": "p", "pages": [{"id": "p", "title": "P",
        "rail": [{"kind": "autotune", "label": "AUTOTUNE"}],
        "rows": [{"sections": [{"kind": "group", "id": "fx", "title": "FX", "tags": "FX"}]}]}]});
    std::fs::write(dir.path().join("layout.json"), frame.to_string()).unwrap();
    let store = Arc::new(crate::layout::store_in(
        dir.path(),
        std::path::Path::new("layout.json"),
        vec!["band".into()],
    ));
    assert_eq!(store.poll(), Some(1));
    let (router, mut own, rx) = offline_router(dir.path());
    let mut router = router.with_layout(store);
    router.subs.connected("band");
    listened(&mut router, &json!([1, 2]), false);
    let (seq, _) = next_read(&mut own).await;
    read_answered(
        &mut router,
        &mut own,
        seq,
        2,
        &[(0, STREAM_TAG), (1, REPRO_TAG)],
    )
    .await;
    settle(&mut router, &mut own).await;
    assert_eq!(
        router.autotune.order,
        vec![AT0.to_string(), AT1.to_string()]
    );
    found_at(&mut router, AT0, 0, "p0");
    found_at(&mut router, AT1, 1, "p1");
    assert_eq!(writes(&steps(&rx)), Vec::<Value>::new());
    (router, rx, dir, own)
}

#[tokio::test]
async fn a_set_load_writes_nothing_by_the_old_layout_until_the_markers_settle_then_once_per_target()
{
    // PR #139 review I1 (#140 PR 3b: the hub wants DRY, AUTOTUNE AUT off);
    // the band reconnects with a set loaded meanwhile, a track inserted
    // before the singers. The old layout (the stream rack on track 0, the
    // repro rack on 1) is still served when the walks answer: track 1 holds
    // the stream rack now, at 0, which the old layout's repro target reads
    // as its chain; its DRY (127) would turn that stream rack on.
    let (mut router, rx, _dir, mut own) = tagged().await;
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Connected(crate::live::ConnectInfo {
            instance: "band".into(),
            set_name: "set".into(),
            script_version: "0".into(),
            live_version: "12".into(),
        }),
    });
    // The connect's read (of the tracks it knew) goes out; the new list
    // starts a newer one.
    let _ = next_read(&mut own).await;
    listened(&mut router, &json!([9, 1, 2]), false);
    let (seq, _) = next_read(&mut own).await;
    // The walks by the old indices: track 0 is the new track (no rack),
    // track 1 holds the stream rack (the old layout's repro target).
    let walk = walk_of(&router, AT0);
    router.autotune_found(AT0, walk, Err("no rack named Latencies".into()));
    found_at(&mut router, AT1, 1, "q0");
    value(&mut router, AT1, 0.0);
    let got = steps(&rx);
    assert_eq!(
        writes(&got),
        Vec::<Value>::new(),
        "nothing by the old layout: {got:?}"
    );
    assert_eq!(
        holds(&got),
        vec![json!({"step": "held", "instance": "band", "on": true, "why": "offline"})]
    );
    // The read finds the Tuners on tracks 1 and 2; their layout ends the
    // hold, and nothing is written before its walks answer.
    read_answered(
        &mut router,
        &mut own,
        seq,
        3,
        &[(1, STREAM_TAG), (2, REPRO_TAG)],
    )
    .await;
    settle(&mut router, &mut own).await;
    assert_eq!(
        router.autotune.order,
        vec![AT1.to_string(), AT2.to_string()]
    );
    let got = steps(&rx);
    assert_eq!(writes(&got), Vec::<Value>::new(), "{got:?}");
    assert_eq!(
        holds(&got),
        vec![json!({"step": "held", "instance": "band", "on": false, "why": "layout"})]
    );
    // The walks find the stream rack on track 1 (at 0: DRY for it) and the
    // repro rack on 2 (at 0: its chain): only the repro rack is written
    // DRY, through its own ref, with its own value.
    found_at(&mut router, AT1, 1, "q0");
    found_at(&mut router, AT2, 2, "q1");
    value(&mut router, AT2, 0.0);
    assert_eq!(writes(&steps(&rx)), vec![dry_of("found", &["#3"])]);
    assert_eq!(router.autotune.targets[AT1].spec.on, 127);
    assert_eq!(router.autotune.targets[AT2].spec.on, 0);
    assert_eq!(router.autotune.targets[AT2].writing, Some(false));
    // Found again: nothing more.
    found_at(&mut router, AT1, 1, "q0");
    found_at(&mut router, AT2, 2, "q1");
    assert_eq!(writes(&steps(&rx)), Vec::<Value>::new());
}

#[tokio::test]
async fn a_track_list_change_writes_nothing_by_the_old_index_until_the_new_layout() {
    // PR #139 review I1 (#140 PR 3b: AUTOTUNE AUT off, the hub wants DRY);
    // a track is inserted before the singers. The walks by the old indices
    // find other racks (track 1's stream rack at 0, which the old repro
    // target reads as its chain), and Live starts before the markers' new
    // layout.
    let (mut router, rx, _dir, mut own) = tagged().await;
    router
        .subs
        .on_values("band", &[pushed("live_1.tracks", json!([9, 1, 2]))]);
    router.flush();
    let (seq, _) = next_read(&mut own).await;
    let walk = walk_of(&router, AT0);
    router.autotune_found(AT0, walk, Err("no rack named Latencies".into()));
    found_at(&mut router, AT1, 1, "p0");
    value(&mut router, AT1, 0.0);
    router
        .subs
        .on_values("band", &[pushed("live_1.is_playing", json!(true))]);
    router.flush();
    let got = steps(&rx);
    assert_eq!(
        writes(&got),
        Vec::<Value>::new(),
        "nothing by the old index: {got:?}"
    );
    assert_eq!(
        holds(&got),
        vec![json!({"step": "held", "instance": "band", "on": true, "why": "tracks"})]
    );
    // The markers' new layout ends the hold; its walks find the stream
    // rack on track 1 and the repro rack on 2: only the repro rack, on its
    // chain, is written DRY.
    read_answered(
        &mut router,
        &mut own,
        seq,
        3,
        &[(1, STREAM_TAG), (2, REPRO_TAG)],
    )
    .await;
    settle(&mut router, &mut own).await;
    let got = steps(&rx);
    assert_eq!(writes(&got), Vec::<Value>::new(), "{got:?}");
    assert_eq!(
        holds(&got),
        vec![json!({"step": "held", "instance": "band", "on": false, "why": "layout"})]
    );
    found_at(&mut router, AT1, 1, "p0");
    found_at(&mut router, AT2, 2, "p1");
    value(&mut router, AT2, 0.0);
    assert_eq!(writes(&steps(&rx)), vec![dry_of("found", &["#3"])]);
}

#[tokio::test]
async fn a_track_list_change_registers_no_dry_write_by_the_old_index() {
    // #140 PR 3a: the DRY writes go by the macros' refs, each with its
    // target's off value. While a track-list change holds the band, a walk
    // by an old index finds another track's rack, whose DRY value is not
    // that target's: nothing is registered until the new layout.
    let (mut router, _rx, _dir, mut own) = tagged().await;
    let registered = |router: &Router| {
        router
            .autotune
            .safe
            .get("band")
            .map(|safe| safe.list.clone())
    };
    let both = vec![("p0".to_string(), 0), ("p1".to_string(), 127)];
    // The stream rack (off 0) on track 0 and the repro rack (off 127) on 1.
    assert_eq!(registered(&router), Some(both.clone()));
    let seq = router.autotune.safe["band"].seq;
    router
        .subs
        .on_values("band", &[pushed("live_1.tracks", json!([9, 1, 2]))]);
    router.flush();
    let (read, _) = next_read(&mut own).await;
    let walk = walk_of(&router, AT0);
    router.autotune_found(AT0, walk, Err("no rack named Latencies".into()));
    // Track 1 holds the stream rack now, and AT1 is still the repro target.
    found_at(&mut router, AT1, 1, "p0");
    assert_eq!(router.autotune.safe["band"].seq, seq, "nothing while held");
    assert_eq!(registered(&router), Some(both.clone()));
    // The markers' new layout ends the hold: the found set as it stands
    // (AT1 is the stream target now, off 0).
    read_answered(
        &mut router,
        &mut own,
        read,
        3,
        &[(1, STREAM_TAG), (2, REPRO_TAG)],
    )
    .await;
    settle(&mut router, &mut own).await;
    assert_eq!(registered(&router), Some(vec![("p0".to_string(), 0)]));
    // Its walks find both racks again: each with its own off value.
    found_at(&mut router, AT1, 1, "p0");
    found_at(&mut router, AT2, 2, "p1");
    assert_eq!(registered(&router), Some(both));
}

/// Every message the router's own tasks send within `limit` (on tokio's
/// virtual clock), handled.
async fn pump(router: &mut Router, own: &mut UnboundedReceiver<RouterMsg>, limit_ms: u64) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(limit_ms);
    while let Ok(Some(msg)) = tokio::time::timeout_at(deadline, own.recv()).await {
        router.handle(msg);
    }
}

#[tokio::test(start_paused = true)]
async fn a_failed_walk_is_walked_again_after_retry_ms() {
    // #140 (the PC, 24.10.2026): the band Live loading its set answered no
    // read for a while; every target's first walk failed ("no result
    // within 3 s") and nothing walked it again until a page's flag. A
    // failed walk (here Live offline in its task) is walked again every
    // RETRY_MS while its set is online.
    let retry_ms = RETRY_MS;
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, _records) = offline_router(dir.path());
    online(&mut router);
    router.handle(layout(1, vec![stream()]));
    let first = walk_of(&router, STREAM);
    pump(&mut router, &mut own, 2 * retry_ms + retry_ms / 2).await;
    // Exactly twice more: each retry RETRY_MS after its failure (PR #145
    // review M4: no back-to-back burst).
    assert_eq!(
        walk_of(&router, STREAM),
        first + 2,
        "walked {} times",
        walk_of(&router, STREAM) - first + 1
    );
    assert_eq!(
        router.autotune.targets[STREAM].problem.as_deref(),
        Some("instance offline")
    );
}

#[tokio::test]
async fn a_disconnect_without_autotune_targets_holds_nothing() {
    // The hold is recorded only for a Live set with AUTOTUNE targets.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, rx) = offline_router(dir.path());
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    assert_eq!(steps(&rx), Vec::<Value>::new());
}

#[tokio::test]
async fn a_rack_replaced_at_the_same_path_is_followed_afresh_and_written_once() {
    // PR #139 review M1: a preset hot-swapped onto the stream rack (or the
    // rack deleted and undone): the script ends the macro's listener
    // (`gone`), and the walk finds a new `$ref` at the same path.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    find(&mut router, STREAM, 7);
    // Live stands, the macro at 127 (its chain): AUT off writes it DRY.
    answer(&mut router, false, 127.0);
    router.flush();
    assert_eq!(router.autotune.targets[STREAM].value, Some(127.0));
    assert_eq!(writes(&steps(&rx)), vec![dry_of("found", &["#8"])]);
    router.subs.on_values(
        "band",
        &[LiveValue {
            key: "live_9.value".into(),
            value: Value::Null,
            display: None,
            error: Some("gone".into()),
        }],
    );
    router.flush();
    assert_eq!(router.autotune.targets[STREAM].value, None);
    let walk = walk_of(&router, STREAM);
    router.autotune_found(
        STREAM,
        walk,
        Ok(Found {
            path: found(7).path,
            id: "p7b".into(),
            key: Err(KeyMissing::NoChain),
        }),
    );
    // The new rack is decided afresh: nothing is written before its value
    // is heard.
    assert_eq!(writes(&steps(&rx)), Vec::<Value>::new());
    assert_eq!(router.autotune.targets[STREAM].writing, None);
    assert_eq!(
        router.autotune.targets[STREAM]
            .found
            .as_ref()
            .map(|f| f.id.as_str()),
        Some("p7b")
    );
    // Its value is followed afresh: exactly one new listener of the macro
    // goes out, and Live's answer to it is the target's value. Only the
    // pending requests are answered (no connect, which would re-resolve
    // the errored entry by itself: PR #139 re-review N1).
    let mut added = 0;
    for o in router.subs.drain_outgoing() {
        let slots: Vec<Value> = o
            .commands
            .iter()
            .map(|c| {
                if c["name"] == "add_listener" && c["args"]["prop"] == "value" {
                    added += 1;
                    slot("live_10.value", json!(127.0))
                } else {
                    ok(Value::Null)
                }
            })
            .collect();
        router.subs.on_result(&o.instance, &o.uuid, &slots);
    }
    assert_eq!(added, 1, "one new listener of the macro");
    router.flush();
    assert_eq!(router.autotune.targets[STREAM].value, Some(127.0));
    assert_eq!(router.autotune.state().targets[0].state, TargetState::On);
    // Its first value on its chain: DRY once, through its own ref.
    assert_eq!(writes(&steps(&rx)), vec![dry_of("found", &["#8"])]);
    assert_eq!(router.autotune.targets[STREAM].writing, Some(false));
}

#[test]
fn a_failed_walk_is_walked_again_up_to_max_retries() {
    // #140: the song map's bound, about two minutes of a set still loading.
    assert_eq!((CONNECT_RETRIES, RETRY_MS), (24, 2000));
    assert!(walk_again(1));
    assert!(walk_again(CONNECT_RETRIES));
    assert!(!walk_again(CONNECT_RETRIES + 1));
}

/// The band's target's walk `walk` failed with Live's words.
fn walk_failed(router: &mut Router, walk: u64) {
    router.autotune_walk_failed(STREAM, walk, "no result within 3 s".into());
}

#[tokio::test(start_paused = true)]
async fn a_failed_walk_is_walked_again_and_a_definite_answer_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx) = offline_router(dir.path());
    // The band offline: a walk goes out to nobody, the test answers it.
    router.handle(layout(1, vec![stream()]));
    let _ = steps(&rx);
    let w0 = walk_of(&router, STREAM);
    walk_failed(&mut router, w0);
    assert_eq!(
        steps(&rx),
        vec![
            json!({"step": "missing", "instance": "band", "target": "live_set tracks 7",
                    "problem": "no result within 3 s"})
        ]
    );
    assert_eq!(router.autotune.targets[STREAM].walk_failures, 1);
    // Not before RETRY_MS (PR #145 review M4), then once.
    pump(&mut router, &mut own, RETRY_MS - 1).await;
    assert_eq!(walk_of(&router, STREAM), w0, "not walked before RETRY_MS");
    pump(&mut router, &mut own, RETRY_MS / 2).await;
    let w1 = walk_of(&router, STREAM);
    assert_eq!(w1, w0 + 1, "walked again");
    // An older walk's failure is void.
    router.autotune_walk_failed(STREAM, w0, "late".into());
    assert_eq!(router.autotune.targets[STREAM].walk_failures, 1);
    assert_eq!(steps(&rx), Vec::<Value>::new());
    // A definite answer is never walked again, and ends the count.
    router.autotune_found(STREAM, w1, Err("no rack named Latencies".into()));
    assert_eq!(router.autotune.targets[STREAM].walk_failures, 0);
    pump(&mut router, &mut own, 3 * RETRY_MS).await;
    assert_eq!(walk_of(&router, STREAM), w1, "never walked again");
    // A failure, its retry, then the rack found: the count over, no more.
    router.autotune_walk(STREAM);
    let w2 = walk_of(&router, STREAM);
    walk_failed(&mut router, w2);
    pump(&mut router, &mut own, RETRY_MS + RETRY_MS / 2).await;
    assert_eq!(walk_of(&router, STREAM), w2 + 1);
    find(&mut router, STREAM, 7);
    assert_eq!(router.autotune.targets[STREAM].walk_failures, 0);
    assert!(router.autotune.targets[STREAM].found.is_some());
    pump(&mut router, &mut own, 3 * RETRY_MS).await;
    assert_eq!(walk_of(&router, STREAM), w2 + 1);
}

#[tokio::test(start_paused = true)]
async fn a_failed_walk_gives_up_after_max_retries_until_the_next_trigger() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    let _ = steps(&rx);
    for n in 1..=CONNECT_RETRIES {
        let w = walk_of(&router, STREAM);
        walk_failed(&mut router, w);
        pump(&mut router, &mut own, RETRY_MS + RETRY_MS / 2).await;
        assert_eq!(walk_of(&router, STREAM), w + 1, "retry {n}");
    }
    let w = walk_of(&router, STREAM);
    walk_failed(&mut router, w);
    let gave_up: Vec<Value> = steps(&rx)
        .into_iter()
        .filter(|r| r["step"] == "gave_up")
        .collect();
    assert_eq!(
        gave_up,
        vec![
            json!({"step": "gave_up", "instance": "band", "target": "live_set tracks 7",
                    "problem": "no result within 3 s", "tries": CONNECT_RETRIES + 1})
        ]
    );
    // `/api/status` says it gave up, and after how many tries (PR #145
    // review M7).
    assert_eq!(
        status(&mut router).autotune.targets[0].problem.as_deref(),
        Some("no result within 3 s (gave up after 25 tries, until the next trigger)")
    );
    assert_eq!(
        gave_up_problem("instance offline", 3),
        "instance offline (gave up after 3 tries, until the next trigger)"
    );
    pump(&mut router, &mut own, 3 * RETRY_MS).await;
    assert_eq!(walk_of(&router, STREAM), w, "no retry after the last");
    // The next trigger (Live starts: an unfound target is walked) walks it,
    // its count over, so its failure is tried again.
    transport(&mut router, true);
    assert_eq!(walk_of(&router, STREAM), w + 1);
    assert_eq!(router.autotune.targets[STREAM].walk_failures, 0);
    walk_failed(&mut router, w + 1);
    pump(&mut router, &mut own, RETRY_MS + RETRY_MS / 2).await;
    assert_eq!(walk_of(&router, STREAM), w + 2);
}

#[test]
fn only_a_found_target_whose_walk_live_did_not_answer_keeps_its_rack() {
    // PR #146 review I3.
    assert!(keeps_found(true, true));
    assert!(!keeps_found(true, false), "nothing found to keep");
    assert!(!keeps_found(false, true), "a definite refusal drops it");
    assert!(!keeps_found(false, false));
}

#[test]
fn only_a_walk_live_did_not_answer_is_walked_again() {
    // PR #145 review M2: a timeout or an offline Live, while retries are
    // left; a definite refusal waits for the next trigger.
    assert_eq!(after_walk(true, 1), AfterWalk::Retry);
    assert_eq!(after_walk(true, CONNECT_RETRIES), AfterWalk::Retry);
    assert_eq!(after_walk(true, CONNECT_RETRIES + 1), AfterWalk::GiveUp);
    assert_eq!(after_walk(false, 1), AfterWalk::Wait);
    assert_eq!(after_walk(false, CONNECT_RETRIES + 1), AfterWalk::Wait);
}

#[tokio::test(start_paused = true)]
async fn a_refused_walk_is_shown_and_not_walked_again() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    let _ = steps(&rx);
    let w = walk_of(&router, STREAM);
    router.autotune_walk_failed(STREAM, w, "live error: busy".into());
    assert_eq!(
        steps(&rx),
        vec![
            json!({"step": "missing", "instance": "band", "target": "live_set tracks 7",
                    "problem": "live error: busy"})
        ]
    );
    pump(&mut router, &mut own, 3 * RETRY_MS).await;
    assert_eq!(walk_of(&router, STREAM), w, "never walked again");
    assert_eq!(steps(&rx), Vec::<Value>::new(), "no give-up");
    // The next trigger walks it.
    transport(&mut router, true);
    assert_eq!(walk_of(&router, STREAM), w + 1);
}

#[tokio::test(start_paused = true)]
async fn a_walk_retry_due_while_live_is_busy_waits_uncounted() {
    // PR #145 review M1: while the band's Live is busy (a set loading) a
    // due retry waits another RETRY_MS, walks nothing and counts nothing.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, _rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    let w = walk_of(&router, STREAM);
    walk_failed(&mut router, w);
    router.states.get_mut("band").unwrap().busy = true;
    pump(&mut router, &mut own, 3 * RETRY_MS + RETRY_MS / 2).await;
    assert_eq!(walk_of(&router, STREAM), w, "no walk while busy");
    assert_eq!(router.autotune.targets[STREAM].walk_failures, 1);
    router.states.get_mut("band").unwrap().busy = false;
    pump(&mut router, &mut own, RETRY_MS).await;
    assert_eq!(walk_of(&router, STREAM), w + 1, "walked once Live is ready");
}

#[tokio::test(start_paused = true)]
async fn a_disconnect_ends_a_failed_walks_retry() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, _rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    let w = walk_of(&router, STREAM);
    walk_failed(&mut router, w);
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    let after = walk_of(&router, STREAM);
    assert!(after > w, "the disconnect voids the walk");
    pump(&mut router, &mut own, 3 * RETRY_MS).await;
    assert_eq!(walk_of(&router, STREAM), after, "its retry is void");
}

/// A router whose band is a scripted Live (`live::client::scripted`).
fn scripted_router(
    dir: &std::path::Path,
) -> (
    Router,
    UnboundedReceiver<RouterMsg>,
    std::sync::mpsc::Receiver<Value>,
    Script,
) {
    let (live, script) = crate::live::client::scripted("band");
    let (tx, own) = tokio::sync::mpsc::unbounded_channel();
    let (events, records) = crate::events::EventLog::channel(1024);
    let router = Router::new(
        BTreeMap::from([("band".to_string(), live)]),
        crate::rules::HubState::default(),
        dir.to_path_buf(),
        super::super::RouterIo { tx, events },
    );
    (router, own, records, script)
}

/// The next call a task made to the scripted Live.
async fn next_asked(script: &mut Script) -> Asked {
    for _ in 0..100 {
        if let Some(asked) = script.try_call() {
            return asked;
        }
        tokio::task::yield_now().await;
    }
    panic!("no call to Live");
}

#[tokio::test]
async fn a_refused_read_fails_the_walk_and_a_definite_answer_ends_it() {
    // #140: a read Live refused fails the walk (a definite refusal, which
    // waits for the next trigger: PR #145 review M2); "no rack named
    // Latencies" is an answer.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, _rx, mut script) = scripted_router(dir.path());
    online(&mut router);
    router.handle(layout(1, vec![stream()]));
    let asked = next_asked(&mut script).await;
    assert_eq!(
        asked.commands,
        vec![
            json!({"target": "live_set tracks 7", "name": "get_prop", "args": {"prop": "devices"}})
        ]
    );
    asked.answer(vec![
        json!({"ok": false, "error": "live error: busy", "errorType": "RuntimeError"}),
    ]);
    let msg = next_of(&mut own, "the failed walk", |m| {
        matches!(m, RouterMsg::AutotuneWalkFailed { .. })
    })
    .await;
    assert!(
        matches!(&msg, RouterMsg::AutotuneWalkFailed { key, why, .. }
            if key == STREAM && why == "live error: busy")
    );
    router.handle(msg);
    assert_eq!(router.autotune.targets[STREAM].walk_failures, 1);
    router.autotune_walk(STREAM);
    let asked = next_asked(&mut script).await;
    asked.answer(vec![json!({"ok": true, "data": []})]);
    let msg = next_of(&mut own, "the walk's answer", |m| {
        matches!(m, RouterMsg::AutotuneFound { .. })
    })
    .await;
    assert!(
        matches!(&msg, RouterMsg::AutotuneFound { outcome: Err(why), .. }
            if why == "no rack named Latencies")
    );
}

#[test]
fn a_walk_checks_the_track_at_its_index_against_the_hubs_list() {
    // PR #146 review M2.
    assert_eq!(
        MOVED_TRACK,
        "the track at its index is another than the hub's track list"
    );
    let track = |id: &str, i: usize| {
        json!({"$ref": id, "path": format!("live_set tracks {i}"), "class": "Track",
               "name": format!("T{i}")})
    };
    let list = json!([track("t0", 0), track("t1", 1)]);
    assert_eq!(item_ref(&list, 1), Some("t1"));
    assert_eq!(item_ref(&list, 2), None);
    assert_eq!(item_ref(&json!([1, 2]), 0), None, "no object");
    assert_eq!(item_ref(&json!("text"), 0), None);
    let lists = BTreeMap::from([
        ("band|live_set|tracks|false".to_string(), list.clone()),
        (
            "band|live_set|return_tracks|false".to_string(),
            json!([{"$ref": "r0", "path": "live_set return_tracks 0"}]),
        ),
    ]);
    assert_eq!(
        track_check(&lists, "band", &Anchor::TrackAt { index: 1 }),
        Some(TrackCheck {
            list: "tracks",
            index: 1,
            id: "t1".into()
        })
    );
    assert_eq!(
        track_check(&lists, "band", &Anchor::ReturnAt { index: 0 }),
        Some(TrackCheck {
            list: "return_tracks",
            index: 0,
            id: "r0".into()
        })
    );
    assert_eq!(
        track_check(&lists, "band", &Anchor::TrackAt { index: 5 }),
        None,
        "no such item"
    );
    assert_eq!(
        track_check(&lists, "master", &Anchor::TrackAt { index: 1 }),
        None,
        "no list heard"
    );
    assert_eq!(track_check(&lists, "band", &Anchor::Master), None);
    let check = TrackCheck {
        list: "tracks",
        index: 1,
        id: "t1".into(),
    };
    assert!(same_track(&[ok(list)], &check));
    let moved = json!([{"$ref": "t0"}, {"$ref": "t9"}]);
    assert!(!same_track(&[ok(moved)], &check));
    assert!(!same_track(
        &[json!({"ok": false, "error": "busy", "errorType": "RuntimeError"})],
        &check
    ));
    assert!(!same_track(&[], &check));
    assert!(walk_retryable(MOVED_TRACK));
    assert!(walk_retryable("instance offline"));
    assert!(walk_retryable("no result within 3 s"));
    assert!(!walk_retryable("live error: busy"));
}

/// The band's track list as the script encodes it: each `$ref`.
fn tracks(ids: &[&str]) -> Value {
    Value::Array(
        ids.iter()
            .enumerate()
            .map(|(i, id)| {
                json!({"$ref": id, "path": format!("live_set tracks {i}"), "class": "Track",
                       "name": format!("T{i}")})
            })
            .collect(),
    )
}

/// The stream target's walk of track 7 answered up to its check: a
/// `Latencies` rack on the track, its macro `p7`, no chain.
async fn walked_to_the_check(script: &mut Script) -> Asked {
    let devices = next_asked(script).await;
    assert_eq!(
        devices.commands,
        vec![
            json!({"target": "live_set tracks 7", "name": "get_prop", "args": {"prop": "devices"}})
        ]
    );
    devices.answer(vec![ok(
        json!([{"class": "RackDevice", "name": "Latencies",
        "path": "live_set tracks 7 devices 0", "$ref": "r7"}]),
    )]);
    let parts = next_asked(script).await;
    assert_eq!(parts.commands.len(), 2, "its parameters and chains");
    parts.answer(vec![
        ok(json!([
            {"class": "DeviceParameter", "name": "Device On",
             "path": "live_set tracks 7 devices 0 parameters 0", "$ref": "x7"},
            {"class": "DeviceParameter", "name": "Macro 1",
             "path": "live_set tracks 7 devices 0 parameters 1", "$ref": "p7"}])),
        ok(json!([])),
    ]);
    let check = next_asked(script).await;
    assert_eq!(
        check.commands,
        vec![json!({"target": "live_set", "name": "get_prop", "args": {"prop": "tracks"}})]
    );
    check
}

#[tokio::test]
async fn a_walk_answer_read_by_an_index_the_hubs_list_no_longer_names_is_never_registered_or_written()
 {
    // PR #146 review M2: in the tick of a track-list change the script
    // answers a walk's reads by the old index before it pushes the list,
    // so the walk read the track there now. Its check finds another track
    // at the index than the hub's list: no answer, nothing registered or
    // written; the list's push then holds the set and walks it again.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    online(&mut router);
    let before = ["t0", "t1", "t2", "t3", "t4", "t5", "t6", "t7"];
    router.reverb_list("band|live_set|tracks|false", &tracks(&before));
    router.handle(layout(1, vec![stream()]));
    let check = walked_to_the_check(&mut script).await;
    // Live's list now: a track inserted before track 7.
    let after = ["t0", "t1", "t2", "t3", "t4", "t5", "tx", "t6", "t7"];
    check.answer(vec![ok(tracks(&after))]);
    let msg = next_of(&mut own, "the walk's end", |m| {
        matches!(
            m,
            RouterMsg::AutotuneWalkFailed { .. } | RouterMsg::AutotuneFound { .. }
        )
    })
    .await;
    assert!(
        matches!(&msg, RouterMsg::AutotuneWalkFailed { key, why, .. }
            if key == STREAM && why == MOVED_TRACK)
    );
    router.handle(msg);
    let target = &router.autotune.targets[STREAM];
    assert!(target.found.is_none(), "no answer");
    assert_eq!(target.walk_failures, 1, "walked again after RETRY_MS");
    assert!(
        !router.autotune.safe.contains_key("band"),
        "nothing registered"
    );
    assert_eq!(writes(&steps(&rx)), Vec::<Value>::new());
    // The list's push: held, walked again; its check is against the new
    // list, which names the same track at the index: found, and held.
    router.reverb_list("band|live_set|tracks|false", &tracks(&after));
    assert!(router.autotune.held.contains("band"));
    let check = walked_to_the_check(&mut script).await;
    check.answer(vec![ok(tracks(&after))]);
    let msg = next_of(&mut own, "the walk's answer", |m| {
        matches!(m, RouterMsg::AutotuneFound { .. })
    })
    .await;
    router.handle(msg);
    assert_eq!(
        router.autotune.targets[STREAM]
            .found
            .as_ref()
            .map(|f| f.id.as_str()),
        Some("p7")
    );
    assert!(
        !router.autotune.safe.contains_key("band"),
        "held: the release registers"
    );
}
