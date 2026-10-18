use std::collections::BTreeMap;
use std::sync::Arc;

use fohmixer_proto::client::{
    AckItem, HUB_REVERB_AUT, HUB_REVERB_LOW, Low, ReverbState, ServerMsg, SetItem,
};
use fohmixer_proto::layout::RuleFader;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

use super::super::tests::{attach, next_of, offline_router, records, set_msg, status};
use super::super::{MARKERS_CLIENT, Router, RouterMsg, UNFOLD_CLIENT, layout_msg, markers};
use super::{
    REVERB_CLIENT, REVERB_PEER, list_changed, list_instance, rearm, tracks_settled, wake_in,
};
use crate::live::LiveValue;
use crate::live::client::LiveEvent;
use crate::outbox::Outbox;
use crate::rules::reverb::lowered;

const ST: &str = "band|live_set tracks 0 mixer_device volume|value";
const PA: &str = "band|live_set tracks 1 mixer_device volume|value";
/// Rev ST's volume subscription (the rule's hub key).
const ST_SUB: &str = "band|live_set tracks 0 mixer_device volume|value|false";

fn fader(label: &str, index: u32) -> RuleFader {
    RuleFader {
        instance: "band".into(),
        label: label.into(),
        target: format!("live_set tracks {index} mixer_device volume"),
    }
}

fn faders() -> Vec<RuleFader> {
    vec![fader("Rev ST", 0)]
}

/// The `reverb` messages waiting in `outbox`.
fn reverb(outbox: &Outbox) -> Vec<ReverbState> {
    outbox
        .take()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| match m {
            ServerMsg::Reverb(s) => Some(s),
            _ => None,
        })
        .collect()
}

/// `hub-state.json` as the router last saved it.
fn saved(dir: &std::path::Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join("hub-state.json")).unwrap()).unwrap()
}

/// Live's value of a fader, handed to the rule as the flush would.
fn live(router: &mut Router, key: &str, value: f64) {
    let now = router.now_ms();
    let acts = router.reverb.rule.live(key, value, now);
    router.reverb_apply(acts, None);
}

fn set_hub(router: &mut Router, key: &str, value: Value) {
    router.handle(RouterMsg::SetHub {
        client: 1,
        key: key.into(),
        value,
    });
}

/// Live's answer to each pending listener of the band (the table online):
/// `answer(target, prop)` the slot.
fn answer(router: &mut Router, answer: impl Fn(&str, &str) -> Value) {
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

/// A listener's answer: Live's key for it and its value.
fn slot(key: &str, value: Value) -> Value {
    json!({"ok": true, "data": {"key": key, "value": value}})
}

/// A page's write of a band track's volume, as an item of a `sets`.
fn item(index: u32, seq: u64, value: f64) -> SetItem {
    SetItem {
        instance: "band".into(),
        target: format!("live_set tracks {index} mixer_device volume"),
        prop: "value".into(),
        value: json!(value),
        seq,
        t: 1_000.0 + seq as f64,
        is_final: false,
    }
}

#[test]
fn the_rules_client_is_the_hubs_own_after_the_keepers() {
    assert_eq!(
        (UNFOLD_CLIENT, MARKERS_CLIENT, REVERB_CLIENT),
        (u64::MAX, u64::MAX - 1, u64::MAX - 2)
    );
}

#[test]
fn a_wake_is_armed_only_when_sooner_than_the_one_on_its_way() {
    assert!(rearm(None, 100.0));
    assert!(rearm(Some(200.0), 100.0));
    assert!(!rearm(Some(100.0), 100.0));
    assert!(!rearm(Some(50.0), 100.0));
}

#[test]
fn a_wake_sleeps_until_its_need_and_never_less_than_nothing() {
    assert_eq!(
        wake_in(1500.0, 1000.0),
        std::time::Duration::from_millis(500)
    );
    assert_eq!(wake_in(900.0, 1000.0), std::time::Duration::ZERO);
}

#[test]
fn a_list_key_names_its_instance_and_only_a_new_value_is_a_change() {
    assert_eq!(
        list_instance("band|live_set|tracks|false").as_deref(),
        Some("band")
    );
    assert_eq!(
        list_instance("master|live_set|return_tracks|false").as_deref(),
        Some("master")
    );
    assert_eq!(list_instance("band|live_set tracks 3|devices|false"), None);
    assert_eq!(list_instance("band|live_set tracks 3|tracks|false"), None);
    assert_eq!(list_instance("band|live_set|is_playing|false"), None);
    assert_eq!(list_instance("band"), None);
    assert!(
        !list_changed(None, &json!([1, 2])),
        "a session's first value"
    );
    assert!(!list_changed(Some(&json!([1, 2])), &json!([1, 2])));
    assert!(list_changed(Some(&json!([1, 2])), &json!([1, 3, 2])));
}

#[tokio::test]
async fn a_page_is_told_the_reverb_faders_at_attach_and_at_a_new_layout() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, rx) = offline_router(dir.path());
    let first = attach(&mut router, 1);
    assert_eq!(reverb(&first).len(), 1, "at attach");
    router.handle(RouterMsg::Layout {
        rev: 1,
        stage: None,
        targets: BTreeMap::new(),
        strips: Vec::new(),
        reverb: vec![fader("Rev ST", 4)],
        fader_leaders: Vec::new(),
    });
    let told = reverb(&first);
    assert_eq!(told.last().map(|s| s.faders.len()), Some(1));
    let steps: Vec<Value> = rx.try_iter().filter(|r| r["ev"] == "reverb").collect();
    assert_eq!(steps.last().unwrap()["step"], json!("faders"));
    assert_eq!(steps.last().unwrap()["labels"], json!(["Rev ST"]));
    let second = attach(&mut router, 2);
    assert_eq!(
        reverb(&second).last().map(|s| s
            .faders
            .iter()
            .map(|f| f.label.clone())
            .collect::<Vec<_>>()),
        Some(vec!["Rev ST".to_string()]),
        "a page attached later is told the faders as they are"
    );
}

#[tokio::test]
async fn a_new_layout_follows_its_faders_volumes_and_lets_the_others_go() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, _records) = offline_router(dir.path());
    router.reverb_layout(faders());
    assert_eq!(router.reverb.watched.keys().collect::<Vec<_>>(), vec![ST]);
    assert_eq!(
        router.subs.subscriptions("band"),
        2,
        "the rules' transport and Rev ST's volume"
    );
    router.reverb_layout(vec![fader("Rev PA", 1)]);
    assert_eq!(router.reverb.watched.keys().collect::<Vec<_>>(), vec![PA]);
    assert_eq!(router.subs.subscriptions("band"), 2, "Rev ST's let go");
}

#[tokio::test]
async fn the_rules_writes_are_sets_of_its_own_client_and_a_pages_set_lets_the_fader_go() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, rx) = offline_router(dir.path());
    let page = attach(&mut router, 1);
    router.reverb_layout(faders());
    live(&mut router, ST, 0.85);
    set_hub(&mut router, HUB_REVERB_LOW, json!(true));
    assert_eq!(
        saved(dir.path()),
        json!({"stage_aut": false, "reverb_low": {"by": "tap", "faders": [
            {"instance": "band", "label": "Rev ST", "back": 0.85, "left": lowered(0.85)}]}}),
        "the hub state saved whole"
    );
    let now = router.now_ms() + 30.0;
    let acts = router.reverb.rule.tick(now);
    router.reverb_apply(acts, None);
    let seen = records(&rx);
    let set = seen
        .iter()
        .find(|r| r["ev"] == "set" && r["key"] == ST)
        .expect("the rule's set");
    assert_eq!(set["client"], json!(REVERB_CLIENT));
    assert_eq!(set["peer"], json!(REVERB_PEER));
    assert!(seen.iter().any(|r| r["ev"] == "reverb"
        && r["step"] == "tap"
        && r["client"] == json!(1)
        && r["peer"] == json!("10.0.0.1")));
    router.handle(set_msg(1, "band", 1, 0.5));
    assert!(router.reverb.rule.state().faders[0].hand);
    assert!(records(&rx).iter().any(|r| r["ev"] == "reverb"
        && r["step"] == "hand"
        && r["by"] == "page"
        && r["client"] == json!(1)));
    assert!(
        page.take()
            .unwrap()
            .iter()
            .any(|m| matches!(m, ServerMsg::Reverb(s) if s.faders.first().is_some_and(|f| f.hand)))
    );
    assert_eq!(
        saved(dir.path()).get("reverb_low"),
        None,
        "nothing held: no lowering kept"
    );
    // A REVERB key with a value that is no bool: the usual error.
    set_hub(&mut router, HUB_REVERB_AUT, json!("yes"));
    assert!(
        page.take()
            .unwrap()
            .iter()
            .any(|m| matches!(m, ServerMsg::Error { .. }))
    );
}

#[tokio::test]
async fn only_the_latest_armed_wake_ticks() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, rx) = offline_router(dir.path());
    let _page = attach(&mut router, 1);
    router.reverb_layout(faders());
    live(&mut router, ST, 0.85);
    router.reverb_transport("band", true);
    set_hub(&mut router, HUB_REVERB_AUT, json!(true));
    let aut = records(&rx)
        .into_iter()
        .find(|r| r["ev"] == "reverb" && r["step"] == "aut")
        .expect("the aut record");
    assert_eq!(
        (
            aut["on"].clone(),
            aut["client"].clone(),
            aut["peer"].clone()
        ),
        (json!(true), json!(1), json!("10.0.0.1"))
    );
    assert_eq!(saved(dir.path())["reverb_aut"], json!(true));
    assert_eq!(router.reverb.generation, 0, "nothing to wait for");
    router.reverb_transport("band", false);
    assert_eq!(router.reverb.generation, 1, "the stop's wait");
    let wait_at = router.reverb.armed.expect("armed");
    set_hub(&mut router, HUB_REVERB_LOW, json!(true));
    assert_eq!(
        router.reverb.generation, 2,
        "the glide's first step, sooner"
    );
    assert!(router.reverb.armed.unwrap() < wait_at);
    let _ = records(&rx);
    router.reverb_wake(1);
    assert_eq!(
        records(&rx),
        Vec::<Value>::new(),
        "a stale wake does nothing"
    );
    router.reverb_wake(2);
    assert!(
        records(&rx)
            .iter()
            .any(|r| r["ev"] == "set" && r["client"] == json!(REVERB_CLIENT))
    );
    assert_eq!(router.reverb.generation, 3, "the glide's next step");
}

#[tokio::test]
async fn a_track_list_value_stops_the_glides_of_its_instance() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, rx) = offline_router(dir.path());
    router.reverb_layout(faders());
    live(&mut router, ST, 0.85);
    let now = router.now_ms();
    let acts = router.reverb.rule.tap(true, now);
    router.reverb_apply(acts, None);
    router.reverb_tracks("band");
    assert!(
        records(&rx)
            .iter()
            .any(|r| r["ev"] == "reverb" && r["step"] == "stopped" && r["why"] == "tracks")
    );
    let now = router.now_ms();
    assert_eq!(router.reverb.rule.next_tick(now), None);
}

#[tokio::test]
async fn the_ack_of_the_last_write_settles_the_fader() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, _records) = offline_router(dir.path());
    router.reverb_layout(faders());
    router.reverb.finals.insert(ST.to_string(), 9);
    router.reverb_acked(&AckItem::applied(ST, 8, None));
    assert!(
        router.reverb.finals.contains_key(ST),
        "an older write's ack"
    );
    router.reverb_acked(&AckItem::applied(ST, 9, None));
    assert!(!router.reverb.finals.contains_key(ST));
}

#[tokio::test]
async fn lives_ack_of_the_last_write_lets_lives_own_values_count_again() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, rx) = offline_router(dir.path());
    router.reverb_layout(faders());
    live(&mut router, ST, 0.85);
    let start = router.now_ms();
    let acts = router.reverb.rule.tap(true, start);
    router.reverb_apply(acts, None);
    let acts = router.reverb.rule.tick(start + 600.0);
    router.reverb_apply(acts, None);
    assert_eq!(
        router.reverb.finals.get(ST),
        Some(&1),
        "the glide's one and last write"
    );
    router.reverb_value(ST_SUB, 0.7);
    assert!(
        !router.reverb.rule.state().faders[0].hand,
        "before its ack: an echo"
    );
    // Live's answer to the setter's batch, as the router gets it.
    router.on_applied("band", 1, &Ok(vec![json!({"ok": true, "data": null})]));
    assert!(router.reverb.finals.is_empty());
    let _ = records(&rx);
    router.reverb_value(ST_SUB, 0.7);
    assert!(router.reverb.rule.state().faders[0].hand);
    let hand = records(&rx)
        .into_iter()
        .find(|r| r["ev"] == "reverb" && r["step"] == "hand")
        .expect("the hand");
    assert_eq!(
        (hand["by"].clone(), hand["value"].clone()),
        (json!("live"), json!(0.7))
    );
    assert!(
        hand.get("client").is_none(),
        "Live's own value names no page"
    );
}

#[tokio::test]
async fn the_flush_hands_the_rule_its_volumes_the_transport_and_a_changed_track_list() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, rx) = offline_router(dir.path());
    let _page = attach(&mut router, 1);
    // A page follows Rev ST's volume before any layout names it; the marker
    // keeper follows the band's tracks.
    router.handle(RouterMsg::Sub {
        client: 1,
        instance: "band".into(),
        target: "live_set tracks 0 mixer_device volume".into(),
        prop: "value".into(),
        display: false,
    });
    router
        .subs
        .subscribe(MARKERS_CLIENT, "band", "live_set", "tracks", false)
        .unwrap();
    router.subs.connected("band");
    answer(&mut router, |target, prop| match (target, prop) {
        (_, "value") => slot("live_1.value", json!(0.85)),
        (_, "tracks") => slot("live_1.tracks", json!([1, 2])),
        (_, "is_playing") => slot("live_1.is_playing", json!(false)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
    // Rev ST's value is in the table already: the rule takes it from there;
    // Rev PA's comes through the flush.
    router.reverb_layout(vec![fader("Rev ST", 0), fader("Rev PA", 1)]);
    assert_eq!(router.reverb.rule.state().playing, Some(false));
    answer(&mut router, |target, prop| match (target, prop) {
        ("live_set tracks 1 mixer_device volume", "value") => slot("live_2.value", json!(0.6)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
    assert!(
        records(&rx).iter().all(|r| r["step"] != "stopped"),
        "a list's first value is no change"
    );
    // The rule's subscriptions are the hub's own: only the page's counts.
    assert_eq!(status(&mut router).subscriptions["band"], 1);
    // Live's values reached the rule: a tap lowers both from them.
    set_hub(&mut router, HUB_REVERB_LOW, json!(true));
    let lower = records(&rx)
        .into_iter()
        .find(|r| r["step"] == "lower")
        .expect("lowered");
    assert_eq!(
        (
            lower["faders"][0]["from"].clone(),
            lower["faders"][1]["from"].clone()
        ),
        (json!(0.85), json!(0.6))
    );
    // A track inserted: both glides stop.
    router.subs.on_values(
        "band",
        &[LiveValue {
            key: "live_1.tracks".into(),
            value: json!([1, 3, 2]),
            display: None,
            error: None,
        }],
    );
    router.flush();
    assert_eq!(
        records(&rx)
            .iter()
            .filter(|r| r["ev"] == "reverb" && r["step"] == "stopped" && r["why"] == "tracks")
            .count(),
        2
    );
    // A disconnect forgets the instance's lists: its next session's first
    // values are no change.
    router
        .reverb
        .lists
        .insert("master|live_set|tracks|false".into(), json!([7]));
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    assert_eq!(
        router.reverb.lists.keys().collect::<Vec<_>>(),
        vec!["master|live_set|tracks|false"]
    );
    assert_eq!(router.reverb.rule.state().playing, None);
}

#[tokio::test]
async fn an_offline_instance_forgets_the_rules_writes_in_flight() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, rx) = offline_router(dir.path());
    router.reverb_layout(faders());
    live(&mut router, ST, 0.85);
    let now = router.now_ms();
    let acts = router.reverb.rule.tap(true, now);
    router.reverb_apply(acts, None);
    router.reverb.finals.insert(ST.to_string(), 4);
    router.reverb.finals.insert(
        "master|live_set tracks 0 mixer_device volume|value".into(),
        5,
    );
    router.reverb_offline("band");
    assert_eq!(
        router.reverb.finals.keys().collect::<Vec<_>>(),
        vec!["master|live_set tracks 0 mixer_device volume|value"]
    );
    assert!(
        records(&rx)
            .iter()
            .any(|r| r["ev"] == "reverb" && r["step"] == "stopped" && r["why"] == "offline")
    );
}

#[tokio::test]
async fn a_linked_drag_lets_go_of_every_fader_it_writes_and_a_partner_alone_of_itself() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, _records) = offline_router(dir.path());
    let _page = attach(&mut router, 1);
    router.reverb_layout(vec![fader("Rev ST", 0), fader("Rev PA", 1)]);
    live(&mut router, ST, 0.85);
    live(&mut router, PA, 0.6);
    let hands = |r: &Router| {
        r.reverb
            .rule
            .state()
            .faders
            .iter()
            .map(|f| f.hand)
            .collect::<Vec<_>>()
    };
    set_hub(&mut router, HUB_REVERB_LOW, json!(true));
    router.handle(RouterMsg::Sets {
        client: 1,
        sets: vec![item(1, 1, 0.5)],
        hub_ms: 1_250.0,
        offset_ms: Some(200.0),
    });
    assert_eq!(hands(&router), vec![false, true], "the partner alone");
    set_hub(&mut router, HUB_REVERB_LOW, json!(false));
    set_hub(&mut router, HUB_REVERB_LOW, json!(true));
    assert_eq!(hands(&router), vec![false, false], "a tap takes both again");
    assert_eq!(
        router
            .reverb
            .rule
            .state()
            .faders
            .iter()
            .map(|f| f.back)
            .collect::<Vec<_>>(),
        vec![Some(0.85), Some(0.6)],
        "a lowering during Rev ST's return keeps its song level"
    );
    router.handle(RouterMsg::Sets {
        client: 1,
        sets: vec![item(0, 2, 0.7), item(1, 3, 0.4)],
        hub_ms: 1_300.0,
        offset_ms: Some(200.0),
    });
    assert_eq!(
        hands(&router),
        vec![true, true],
        "the leader's drag writes both"
    );
    assert_eq!(router.reverb.rule.status().hands, 3);
}

#[tokio::test]
async fn a_layout_never_writes_over_a_hub_state_it_could_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hub-state.json");
    std::fs::write(&file, "{not json").unwrap();
    let (mut router, _rx, _records) = offline_router(dir.path());
    router.reverb_layout(faders());
    router.reverb_layout(vec![fader("Rev ST", 0), fader("Rev PA", 1)]);
    router.reverb_layout(Vec::new());
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "{not json",
        "nothing the rule holds changed: nothing saved"
    );
}

#[tokio::test]
async fn a_transport_listener_failure_ends_reverb_auts_wait() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, rx) = offline_router(dir.path());
    let _page = attach(&mut router, 1);
    router.reverb_layout(faders());
    live(&mut router, ST, 0.85);
    router.reverb_transport("band", true);
    set_hub(&mut router, HUB_REVERB_AUT, json!(true));
    router.reverb_transport("band", false);
    assert!(router.reverb.rule.state().waiting);
    let _ = records(&rx);
    let key = router.transport["band"].key.clone();
    router.transport_failed(&key, "gone");
    let state = router.reverb.rule.state();
    assert!(
        !state.waiting && state.playing.is_none(),
        "an unknown transport never lowers"
    );
    assert!(
        records(&rx)
            .iter()
            .any(|r| r["ev"] == "reverb" && r["step"] == "cancelled")
    );
}

#[tokio::test]
async fn a_reconnects_values_are_judged_once_its_window_ends() {
    // One connect answer carrying a held fader's volume off where the rule
    // left it and Live playing. The disconnect opened the track-list window
    // (review W2): the start's return waits, and the value is judged once
    // the connect's read settles: the hand wins, nothing is written.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, rx) = offline_router(dir.path());
    let _page = attach(&mut router, 1);
    router.reverb_layout(faders());
    router.subs.connected("band");
    answer(&mut router, |_, prop| match prop {
        "value" => slot("live_1.value", json!(0.85)),
        "is_playing" => slot("live_1.is_playing", json!(false)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
    // AUT on while Live stands, and a tap takes the wait over: lowered for
    // the stop; the glide runs to its end.
    set_hub(&mut router, HUB_REVERB_AUT, json!(true));
    set_hub(&mut router, HUB_REVERB_LOW, json!(true));
    let now = router.now_ms() + 600.0;
    let acts = router.reverb.rule.tick(now);
    router.reverb_apply(acts, None);
    let state = router.reverb.rule.state();
    assert_eq!(
        (state.low, state.faders[0].back),
        (Some(Low::Stop), Some(0.85))
    );
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    let _ = records(&rx);
    router.subs.connected("band");
    answer(&mut router, |_, prop| match prop {
        "value" => slot("live_1.value", json!(0.7)),
        "is_playing" => slot("live_1.is_playing", json!(true)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
    let got = records(&rx);
    let steps: Vec<&Value> = got
        .iter()
        .filter(|r| r["ev"] == "reverb")
        .map(|r| &r["step"])
        .collect();
    assert_eq!(steps, vec!["pending"], "{got:?}");
    assert_eq!(rule_sets(&got), vec![], "nothing written");
    router.reverb_tracks_known("band");
    let got = records(&rx);
    let steps: Vec<&Value> = got
        .iter()
        .filter(|r| r["ev"] == "reverb")
        .map(|r| &r["step"])
        .collect();
    assert_eq!(steps, vec!["hand"], "{got:?}");
    assert_eq!(rule_sets(&got), vec![], "nothing written");
    assert_eq!(router.reverb.rule.state().low, None);
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

/// Rev ST on band track 0 and Rev PA on track 1, Live stopped, with the
/// marker keeper's list followed; then a track inserted before both, its
/// index bindings resolved again (track 0 is the new track, track 1 Rev
/// ST), and the markers' new layout: Rev ST on track 1 (Rev PA's old,
/// already followed key), Rev PA on track 2. `lowered_first` first lowers
/// both for the stop and lets Live report them where the rule left them.
/// The router, its records, and what must outlive the test's checks (the
/// data folder, the router's own channel).
type Shifted = (
    Router,
    std::sync::mpsc::Receiver<Value>,
    tempfile::TempDir,
    tokio::sync::mpsc::UnboundedReceiver<RouterMsg>,
);

fn shifted(lowered_first: bool) -> Shifted {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, own, rx) = offline_router(dir.path());
    let _page = attach(&mut router, 1);
    router.reverb_layout(vec![fader("Rev ST", 0), fader("Rev PA", 1)]);
    router
        .subs
        .subscribe(MARKERS_CLIENT, "band", "live_set", "tracks", false)
        .unwrap();
    router.subs.connected("band");
    answer(&mut router, |target, prop| match (target, prop) {
        ("live_set tracks 0 mixer_device volume", "value") => slot("live_st.value", json!(0.85)),
        ("live_set tracks 1 mixer_device volume", "value") => slot("live_pa.value", json!(0.6)),
        (_, "tracks") => slot("live_1.tracks", json!([1, 2])),
        (_, "is_playing") => slot("live_1.is_playing", json!(false)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
    let st = if lowered_first {
        set_hub(&mut router, HUB_REVERB_AUT, json!(true));
        set_hub(&mut router, HUB_REVERB_LOW, json!(true));
        let end = router.now_ms() + 600.0;
        let acts = router.reverb.rule.tick(end);
        router.reverb_apply(acts, None);
        router.subs.on_values(
            "band",
            &[
                pushed("live_st.value", json!(lowered(0.85))),
                pushed("live_pa.value", json!(lowered(0.6))),
            ],
        );
        router.flush();
        lowered(0.85)
    } else {
        0.85
    };
    // A track inserted before both: the list changes, and the index
    // bindings are resolved again.
    router
        .subs
        .on_values("band", &[pushed("live_1.tracks", json!([9, 1, 2]))]);
    router.flush();
    router.subs.connected("band");
    answer(&mut router, |target, prop| match (target, prop) {
        ("live_set tracks 0 mixer_device volume", "value") => slot("live_new.value", json!(0.5)),
        ("live_set tracks 1 mixer_device volume", "value") => slot("live_st.value", json!(st)),
        (_, "tracks") => slot("live_1.tracks", json!([9, 1, 2])),
        (_, "is_playing") => slot("live_1.is_playing", json!(false)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
    // The markers' new layout; Rev PA's new key is followed afresh.
    router.reverb_layout(vec![fader("Rev ST", 1), fader("Rev PA", 2)]);
    let pa = if lowered_first { lowered(0.6) } else { 0.6 };
    answer(&mut router, |target, prop| match (target, prop) {
        ("live_set tracks 2 mixer_device volume", "value") => slot("live_pa.value", json!(pa)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
    let _ = records(&rx);
    (router, rx, dir, own)
}

#[tokio::test]
async fn a_held_fader_recomposed_onto_a_followed_key_returns_exactly() {
    // Review N1: Rev ST, held low at rest, lands on Rev PA's old key, which
    // REVERB already follows; the start returns it from Live's value.
    let (mut router, rx, _dir, _own) = shifted(true);
    router
        .subs
        .on_values("band", &[pushed("live_1.is_playing", json!(true))]);
    router.flush();
    let got = records(&rx);
    let steps: Vec<&Value> = got
        .iter()
        .filter(|r| r["ev"] == "reverb")
        .map(|r| &r["step"])
        .collect();
    assert!(!steps.contains(&&json!("pending")), "{got:?}");
    let back = got
        .iter()
        .find(|r| r["ev"] == "reverb" && r["step"] == "return")
        .expect("a return");
    assert_eq!(
        back["faders"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| (m["label"].clone(), m["from"].clone(), m["to"].clone()))
            .collect::<Vec<_>>(),
        vec![
            (json!("Rev ST"), json!(lowered(0.85)), json!(0.85)),
            (json!("Rev PA"), json!(lowered(0.6)), json!(0.6)),
        ]
    );
}

#[tokio::test]
async fn a_fader_recomposed_onto_a_followed_key_is_lowered_from_its_value() {
    // The same shift with nothing held: the next tap lowers Rev ST from
    // the value the table holds, never skipping it for "no value".
    let (mut router, rx, _dir, _own) = shifted(false);
    set_hub(&mut router, HUB_REVERB_LOW, json!(true));
    let got = records(&rx);
    assert!(
        !got.iter()
            .any(|r| r["ev"] == "reverb" && r["step"] == "skipped"),
        "{got:?}"
    );
    let lower = got
        .iter()
        .find(|r| r["ev"] == "reverb" && r["step"] == "lower")
        .expect("a lowering");
    assert_eq!(
        lower["faders"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| (m["label"].clone(), m["from"].clone()))
            .collect::<Vec<_>>(),
        vec![
            (json!("Rev ST"), json!(0.85)),
            (json!("Rev PA"), json!(0.6)),
        ]
    );
}

/// Rev ST on band track 0 at 0.85 and Rev PA on track 1 at 0.6, the marker
/// keeper's list followed, Live playing or not.
fn listed(playing: bool) -> Shifted {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, own, rx) = offline_router(dir.path());
    let _page = attach(&mut router, 1);
    router.reverb_layout(vec![fader("Rev ST", 0), fader("Rev PA", 1)]);
    router
        .subs
        .subscribe(MARKERS_CLIENT, "band", "live_set", "tracks", false)
        .unwrap();
    router.subs.connected("band");
    answer(&mut router, |target, prop| match (target, prop) {
        ("live_set tracks 0 mixer_device volume", "value") => slot("live_st.value", json!(0.85)),
        ("live_set tracks 1 mixer_device volume", "value") => slot("live_pa.value", json!(0.6)),
        (_, "tracks") => slot("live_1.tracks", json!([1, 2])),
        (_, "is_playing") => slot("live_1.is_playing", json!(playing)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
    (router, rx, dir, own)
}

/// A track inserted before both while Live stands: the list changes in its
/// own flush, then the index bindings are resolved again (track 0 the new
/// track at 0.5, track 1 Rev ST at `st`).
fn insert_track(router: &mut Router, st: f64) {
    router
        .subs
        .on_values("band", &[pushed("live_1.tracks", json!([9, 1, 2]))]);
    router.flush();
    router.subs.connected("band");
    answer(router, |target, prop| match (target, prop) {
        ("live_set tracks 0 mixer_device volume", "value") => slot("live_new.value", json!(0.5)),
        ("live_set tracks 1 mixer_device volume", "value") => slot("live_st.value", json!(st)),
        (_, "tracks") => slot("live_1.tracks", json!([9, 1, 2])),
        (_, "is_playing") => slot("live_1.is_playing", json!(false)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
}

/// The markers' layout after the insert: Rev ST on track 1 (followed
/// before), Rev PA on track 2, whose value `pa` Live then reports.
fn relayout(router: &mut Router, pa: f64) {
    router.reverb_layout(vec![fader("Rev ST", 1), fader("Rev PA", 2)]);
    answer(router, |target, prop| match (target, prop) {
        ("live_set tracks 2 mixer_device volume", "value") => slot("live_pa.value", json!(pa)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
}

/// The `set` records of REVERB's own writes in `got`: key, value, final.
fn rule_sets(got: &[Value]) -> Vec<(Value, Value, Value)> {
    got.iter()
        .filter(|r| r["ev"] == "set" && r["client"] == json!(REVERB_CLIENT))
        .map(|r| (r["key"].clone(), r["value"].clone(), r["final"].clone()))
        .collect()
}

#[tokio::test]
async fn a_lowering_writes_nothing_between_a_track_list_change_and_the_next_layout() {
    // Review W1: AUT's wait ends after a track was inserted before both
    // reverbs and before the markers composed the new layout.
    let (mut router, rx, _dir, _own) = listed(true);
    set_hub(&mut router, HUB_REVERB_AUT, json!(true));
    router
        .subs
        .on_values("band", &[pushed("live_1.is_playing", json!(false))]);
    router.flush();
    assert!(router.reverb.rule.state().waiting);
    insert_track(&mut router, 0.85);
    let _ = records(&rx);
    let end = router.now_ms() + 2100.0;
    let acts = router.reverb.rule.tick(end);
    router.reverb_apply(acts, None);
    let got = records(&rx);
    assert_eq!(rule_sets(&got), vec![], "nothing by the old index: {got:?}");
    // The new layout ends the window: a tap lowers both by their new keys.
    relayout(&mut router, 0.6);
    set_hub(&mut router, HUB_REVERB_LOW, json!(true));
    let _ = records(&rx);
    let step = router.now_ms() + 30.0;
    let acts = router.reverb.rule.tick(step);
    router.reverb_apply(acts, None);
    let keys: Vec<Value> = rule_sets(&records(&rx))
        .into_iter()
        .map(|(key, _, _)| key)
        .collect();
    assert_eq!(
        keys,
        vec![
            json!("band|live_set tracks 1 mixer_device volume|value"),
            json!("band|live_set tracks 2 mixer_device volume|value"),
        ]
    );
}

#[tokio::test]
async fn a_return_between_a_track_list_change_and_the_next_layout_waits_for_it() {
    // Review W1: both held low at rest, a track inserted before them, then
    // a start before the markers composed the new layout.
    let (mut router, rx, _dir, _own) = listed(false);
    set_hub(&mut router, HUB_REVERB_AUT, json!(true));
    set_hub(&mut router, HUB_REVERB_LOW, json!(true));
    let end = router.now_ms() + 600.0;
    let acts = router.reverb.rule.tick(end);
    router.reverb_apply(acts, None);
    router.subs.on_values(
        "band",
        &[
            pushed("live_st.value", json!(lowered(0.85))),
            pushed("live_pa.value", json!(lowered(0.6))),
        ],
    );
    router.flush();
    insert_track(&mut router, lowered(0.85));
    let _ = records(&rx);
    router
        .subs
        .on_values("band", &[pushed("live_1.is_playing", json!(true))]);
    router.flush();
    let end = router.now_ms() + 600.0;
    let acts = router.reverb.rule.tick(end);
    router.reverb_apply(acts, None);
    let got = records(&rx);
    assert_eq!(rule_sets(&got), vec![], "nothing by the old index: {got:?}");
    // The new layout: each returns exactly to its song level.
    relayout(&mut router, lowered(0.6));
    let end = router.now_ms() + 600.0;
    let acts = router.reverb.rule.tick(end);
    router.reverb_apply(acts, None);
    let ends: Vec<(Value, Value, Value)> = rule_sets(&records(&rx))
        .into_iter()
        .filter(|(_, _, last)| *last == json!(true))
        .collect();
    assert_eq!(
        ends,
        vec![
            (
                json!("band|live_set tracks 1 mixer_device volume|value"),
                json!(0.85),
                json!(true)
            ),
            (
                json!("band|live_set tracks 2 mixer_device volume|value"),
                json!(0.6),
                json!(true)
            ),
        ]
    );
}

#[test]
fn a_track_list_window_closes_only_when_the_markers_read_settles() {
    assert!(tracks_settled(true, false));
    assert!(!tracks_settled(false, false), "the markers not settled");
    assert!(!tracks_settled(true, true), "a layout to come");
    assert!(!tracks_settled(false, true));
}

#[tokio::test]
async fn the_markers_settling_with_no_new_layout_lets_a_waiting_return_go_on() {
    // A track added after both reverbs: the list changes, their indices
    // do not, and no new layout comes.
    let (mut router, rx, _dir, _own) = listed(false);
    set_hub(&mut router, HUB_REVERB_AUT, json!(true));
    set_hub(&mut router, HUB_REVERB_LOW, json!(true));
    let end = router.now_ms() + 600.0;
    let acts = router.reverb.rule.tick(end);
    router.reverb_apply(acts, None);
    router.subs.on_values(
        "band",
        &[
            pushed("live_st.value", json!(lowered(0.85))),
            pushed("live_pa.value", json!(lowered(0.6))),
        ],
    );
    router.flush();
    router
        .subs
        .on_values("band", &[pushed("live_1.tracks", json!([1, 2, 9]))]);
    router.flush();
    router
        .subs
        .on_values("band", &[pushed("live_1.is_playing", json!(true))]);
    router.flush();
    let _ = records(&rx);
    let end = router.now_ms() + 600.0;
    let acts = router.reverb.rule.tick(end);
    router.reverb_apply(acts, None);
    assert_eq!(
        rule_sets(&records(&rx)),
        vec![],
        "the window holds the return"
    );
    router.reverb_tracks_known("band");
    let end = router.now_ms() + 600.0;
    let acts = router.reverb.rule.tick(end);
    router.reverb_apply(acts, None);
    let ends: Vec<(Value, Value, Value)> = rule_sets(&records(&rx))
        .into_iter()
        .filter(|(_, _, last)| *last == json!(true))
        .collect();
    assert_eq!(
        ends,
        vec![
            (
                json!("band|live_set tracks 0 mixer_device volume|value"),
                json!(0.85),
                json!(true)
            ),
            (
                json!("band|live_set tracks 1 mixer_device volume|value"),
                json!(0.6),
                json!(true)
            ),
        ]
    );
}

/// Rev ST's and Rev PA's Tuner markers: strips of the tag group FX.
const ST_MARK: &str = r#""Rev ST" +G:FX +REVERB"#;
const PA_MARK: &str = r#""Rev PA" +G:FX +REVERB"#;

/// Live's answer to a command: its data.
fn ok(data: Value) -> Value {
    json!({"ok": true, "data": data})
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
/// named `name` alone on each `(index, name)` track: both batches, handled
/// as the read's tasks send them.
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
    if tuners.is_empty() {
        return;
    }
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

/// The rule's time `ms` from now: its wake handled.
fn tick_after(router: &mut Router, ms: f64) {
    let at = router.now_ms() + ms;
    let acts = router.reverb.rule.tick(at);
    router.reverb_apply(acts, None);
}

/// A router whose marker keeper follows the band (a frame of one tag group
/// FX): Rev ST on band track 0 and Rev PA on track 1, composed from the
/// keeper's own read, at 0.85 and 0.6, AUT on, Live playing or not.
async fn marked(playing: bool) -> Shifted {
    let dir = tempfile::tempdir().unwrap();
    let frame = json!({"schema": 2, "default_page": "p", "pages": [{"id": "p", "title": "P",
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
    let _page = attach(&mut router, 1);
    router.subs.connected("band");
    answer(&mut router, |target, prop| match (target, prop) {
        (_, "tracks") => slot("live_1.tracks", json!([1, 2])),
        (_, "return_tracks") => slot("live_1.returns", json!([])),
        (_, "is_playing") => slot("live_1.is_playing", json!(playing)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
    let (seq, _) = next_read(&mut own).await;
    read_answered(&mut router, &mut own, seq, 2, &[(0, ST_MARK), (1, PA_MARK)]).await;
    settle(&mut router, &mut own).await;
    assert_eq!(router.reverb.rule.state().faders.len(), 2);
    answer(&mut router, |target, prop| match (target, prop) {
        ("live_set tracks 0 mixer_device volume", "value") => slot("live_st.value", json!(0.85)),
        ("live_set tracks 1 mixer_device volume", "value") => slot("live_pa.value", json!(0.6)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
    set_hub(&mut router, HUB_REVERB_AUT, json!(true));
    let _ = records(&rx);
    (router, rx, dir, own)
}

/// Both lowered for the stop by a tap (Live stands, AUT on), the glide run
/// to its end, and Live reporting them where the rule left them.
fn lowered_at_rest(router: &mut Router) {
    set_hub(router, HUB_REVERB_LOW, json!(true));
    tick_after(router, 600.0);
    router.subs.on_values(
        "band",
        &[
            pushed("live_st.value", json!(lowered(0.85))),
            pushed("live_pa.value", json!(lowered(0.6))),
        ],
    );
    router.flush();
}

/// A track added after both reverbs (the list changes, their indices do
/// not), then Live starts: both returns wait for the window. The keeper's
/// read of the band runs.
async fn added_after(router: &mut Router, own: &mut UnboundedReceiver<RouterMsg>) -> u64 {
    router
        .subs
        .on_values("band", &[pushed("live_1.tracks", json!([1, 2, 9]))]);
    router.flush();
    let (seq, _) = next_read(own).await;
    router
        .subs
        .on_values("band", &[pushed("live_1.is_playing", json!(true))]);
    router.flush();
    seq
}

/// The ends of REVERB's writes in `got`: both back exactly on their own
/// keys.
fn returned_exactly(got: &[Value]) {
    let ends: Vec<(Value, Value, Value)> = rule_sets(got)
        .into_iter()
        .filter(|(_, _, last)| *last == json!(true))
        .collect();
    assert_eq!(
        ends,
        vec![
            (json!(ST), json!(0.85), json!(true)),
            (json!(PA), json!(0.6), json!(true)),
        ]
    );
}

#[tokio::test]
async fn a_second_track_list_change_within_the_settle_keeps_the_window_until_its_read() {
    // Review M1, the first trigger: the first change's read reports its
    // markers, and a second change comes within their 300 ms quiet time,
    // so the first composition is served while the second read runs. AUT's
    // wait then ends.
    let (mut router, rx, _dir, mut own) = marked(true).await;
    router
        .subs
        .on_values("band", &[pushed("live_1.tracks", json!([9, 1, 2]))]);
    router.flush();
    let (first, _) = next_read(&mut own).await;
    read_answered(
        &mut router,
        &mut own,
        first,
        3,
        &[(1, ST_MARK), (2, PA_MARK)],
    )
    .await;
    router
        .subs
        .on_values("band", &[pushed("live_1.tracks", json!([8, 9, 1, 2]))]);
    router.flush();
    let (second, _) = next_read(&mut own).await;
    // The index bindings resolved again: track 1 is the first new track,
    // track 2 Rev ST's.
    router.subs.connected("band");
    answer(&mut router, |target, prop| match (target, prop) {
        ("live_set tracks 0 mixer_device volume", "value") => slot("live_a.value", json!(0.5)),
        ("live_set tracks 1 mixer_device volume", "value") => slot("live_b.value", json!(0.4)),
        (_, "tracks") => slot("live_1.tracks", json!([8, 9, 1, 2])),
        (_, "return_tracks") => slot("live_1.returns", json!([])),
        (_, "is_playing") => slot("live_1.is_playing", json!(true)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
    // The first composition (Rev ST on track 1, Rev PA on 2) is served.
    settle(&mut router, &mut own).await;
    answer(&mut router, |target, prop| match (target, prop) {
        ("live_set tracks 2 mixer_device volume", "value") => slot("live_st.value", json!(0.85)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
    router
        .subs
        .on_values("band", &[pushed("live_1.is_playing", json!(false))]);
    router.flush();
    assert!(router.reverb.rule.state().waiting);
    let _ = records(&rx);
    tick_after(&mut router, 2100.0);
    let got = records(&rx);
    assert_eq!(
        rule_sets(&got),
        vec![],
        "nothing by the old indices: {got:?}"
    );
    // The second read's composition ends the window: a tap lowers both by
    // their keys now.
    read_answered(
        &mut router,
        &mut own,
        second,
        4,
        &[(2, ST_MARK), (3, PA_MARK)],
    )
    .await;
    settle(&mut router, &mut own).await;
    answer(&mut router, |target, prop| match (target, prop) {
        ("live_set tracks 3 mixer_device volume", "value") => slot("live_pa.value", json!(0.6)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
    let _ = records(&rx);
    set_hub(&mut router, HUB_REVERB_LOW, json!(true));
    tick_after(&mut router, 30.0);
    let got = records(&rx);
    let lower = got
        .iter()
        .find(|r| r["ev"] == "reverb" && r["step"] == "lower")
        .expect("a lowering");
    assert_eq!(
        lower["faders"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["from"].clone())
            .collect::<Vec<_>>(),
        vec![json!(0.85), json!(0.6)]
    );
    let keys: Vec<Value> = rule_sets(&got).into_iter().map(|(key, _, _)| key).collect();
    assert_eq!(
        keys,
        vec![
            json!("band|live_set tracks 2 mixer_device volume|value"),
            json!("band|live_set tracks 3 mixer_device volume|value"),
        ]
    );
}

#[tokio::test]
async fn a_settle_that_serves_nothing_ends_the_window_only_of_a_settled_instance() {
    // Review M1, the second trigger, and M4: a composition that serves
    // nothing (the same markers again, as a Tuner's own name value gives)
    // settles while the band's read runs; its window stays.
    let (mut router, rx, _dir, mut own) = marked(false).await;
    lowered_at_rest(&mut router);
    let seq = added_after(&mut router, &mut own).await;
    let found = router.markers.found().to_vec();
    router.markers_found(found.clone());
    settle(&mut router, &mut own).await;
    let _ = records(&rx);
    tick_after(&mut router, 600.0);
    assert_eq!(rule_sets(&records(&rx)), vec![], "the read still runs");
    // The read answers, unchanged, while another such composition waits:
    // the window ends once that one settles, serving nothing.
    router.markers_found(found);
    read_answered(&mut router, &mut own, seq, 3, &[(0, ST_MARK), (1, PA_MARK)]).await;
    tick_after(&mut router, 600.0);
    assert_eq!(rule_sets(&records(&rx)), vec![], "a composition waits");
    settle(&mut router, &mut own).await;
    tick_after(&mut router, 600.0);
    returned_exactly(&records(&rx));
}

#[tokio::test]
async fn a_layout_served_while_the_markers_read_runs_keeps_the_window() {
    // Review M1, the fourth trigger: the layout file's poll serves the
    // same strips while the band's read after a track-list change runs.
    let (mut router, rx, _dir, mut own) = marked(false).await;
    lowered_at_rest(&mut router);
    let seq = added_after(&mut router, &mut own).await;
    let (rev, served) = router.layout_store.clone().unwrap().current();
    router.handle(layout_msg(rev + 1, served.as_deref()));
    let _ = records(&rx);
    tick_after(&mut router, 600.0);
    assert_eq!(rule_sets(&records(&rx)), vec![], "the read still runs");
    read_answered(&mut router, &mut own, seq, 3, &[(0, ST_MARK), (1, PA_MARK)]).await;
    tick_after(&mut router, 600.0);
    returned_exactly(&records(&rx));
}

#[tokio::test]
async fn a_failed_markers_read_keeps_the_window_until_a_read_answers() {
    // The keeper's read after a track-list change fails: no read runs, yet
    // where each strip is stays unknown, so a composition that settles
    // meanwhile ends no window.
    let (mut router, rx, _dir, mut own) = marked(false).await;
    lowered_at_rest(&mut router);
    router
        .subs
        .on_values("band", &[pushed("live_1.tracks", json!([1, 2, 9]))]);
    router.flush();
    let failed = next_of(&mut own, "a read", |m| {
        matches!(m, RouterMsg::MarkersRead { .. })
    })
    .await;
    router.handle(failed);
    router
        .subs
        .on_values("band", &[pushed("live_1.is_playing", json!(true))]);
    router.flush();
    let found = router.markers.found().to_vec();
    router.markers_found(found);
    settle(&mut router, &mut own).await;
    let _ = records(&rx);
    tick_after(&mut router, 600.0);
    assert_eq!(rule_sets(&records(&rx)), vec![], "the read failed");
    // The read tried again answers, unchanged: the window ends.
    router.handle(RouterMsg::MarkersRetry {
        instance: "band".into(),
    });
    let (seq, _) = next_read(&mut own).await;
    read_answered(&mut router, &mut own, seq, 3, &[(0, ST_MARK), (1, PA_MARK)]).await;
    tick_after(&mut router, 600.0);
    returned_exactly(&records(&rx));
}

#[tokio::test]
async fn a_reconnect_after_a_track_list_change_writes_nothing_until_its_read_settles() {
    // Review W2: the band goes offline; a track is inserted before both
    // reverbs meanwhile; it reconnects.
    let (mut router, rx, _dir, mut own) = marked(true).await;
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    // While it is offline, a composition that serves nothing settles: the
    // window stays (no read can tell where the strips are).
    let found = router.markers.found().to_vec();
    router.markers_found(found);
    settle(&mut router, &mut own).await;
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
    answer(&mut router, |target, prop| match (target, prop) {
        ("live_set tracks 0 mixer_device volume", "value") => slot("live_new.value", json!(0.5)),
        ("live_set tracks 1 mixer_device volume", "value") => slot("live_st.value", json!(0.85)),
        (_, "tracks") => slot("live_1.tracks", json!([9, 1, 2])),
        (_, "return_tracks") => slot("live_1.returns", json!([])),
        (_, "is_playing") => slot("live_1.is_playing", json!(true)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
    let (seq, _) = next_read(&mut own).await;
    let _ = records(&rx);
    // A tap down before the read settles: nothing by the old indices.
    set_hub(&mut router, HUB_REVERB_LOW, json!(true));
    tick_after(&mut router, 600.0);
    let got = records(&rx);
    assert_eq!(
        rule_sets(&got),
        vec![],
        "nothing by the old indices: {got:?}"
    );
    // The read finds the strips moved; their layout ends the window, and
    // the faders behave as before: a tap down lowers both by their keys now.
    read_answered(&mut router, &mut own, seq, 3, &[(1, ST_MARK), (2, PA_MARK)]).await;
    settle(&mut router, &mut own).await;
    answer(&mut router, |target, prop| match (target, prop) {
        ("live_set tracks 2 mixer_device volume", "value") => slot("live_pa.value", json!(0.6)),
        _ => json!({"ok": true, "data": null}),
    });
    router.flush();
    let _ = records(&rx);
    set_hub(&mut router, HUB_REVERB_LOW, json!(true));
    tick_after(&mut router, 30.0);
    let got = records(&rx);
    let lower = got
        .iter()
        .find(|r| r["ev"] == "reverb" && r["step"] == "lower")
        .expect("a lowering");
    assert_eq!(
        lower["faders"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["from"].clone())
            .collect::<Vec<_>>(),
        vec![json!(0.85), json!(0.6)]
    );
    let keys: Vec<Value> = rule_sets(&got).into_iter().map(|(key, _, _)| key).collect();
    assert_eq!(
        keys,
        vec![
            json!("band|live_set tracks 1 mixer_device volume|value"),
            json!("band|live_set tracks 2 mixer_device volume|value"),
        ]
    );
}
