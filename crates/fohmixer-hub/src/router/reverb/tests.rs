use std::collections::BTreeMap;

use fohmixer_proto::client::{
    AckItem, HUB_REVERB_AUT, HUB_REVERB_LOW, ReverbState, ServerMsg, SetItem,
};
use fohmixer_proto::layout::RuleFader;
use serde_json::{Value, json};

use super::super::tests::{attach, offline_router, records, set_msg, status};
use super::super::{MARKERS_CLIENT, Router, RouterMsg, UNFOLD_CLIENT};
use super::{REVERB_CLIENT, REVERB_PEER, list_changed, list_instance, rearm, wake_in};
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
