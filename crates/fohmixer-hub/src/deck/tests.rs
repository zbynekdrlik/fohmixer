use serde_json::json;

use super::*;
use crate::companion::{Answer, KeyUpdate, Press};

fn update(key: u32, img: Option<&str>, color: Option<&str>, pressed: Option<bool>) -> KeyUpdate {
    KeyUpdate {
        key,
        img: img.map(str::to_string),
        color: color.map(str::to_string),
        pressed,
    }
}

fn online() -> Deck {
    let mut deck = Deck::default();
    assert_eq!(deck.link_up(), Vec::<u32>::new());
    deck
}

#[test]
fn holders_forward_the_first_down_and_the_last_up() {
    let mut h = Holders::default();
    assert_eq!(h.down(4, 1), Hold::Forward);
    assert_eq!(h.down(4, 2), Hold::Held);
    assert_eq!(h.holders(4), 2);
    // The first finger lifts: Companion still holds the key.
    assert_eq!(h.up(4, 1), Hold::Held);
    assert_eq!(h.holders(4), 1);
    assert_eq!(h.up(4, 2), Hold::Forward);
    assert_eq!(h.holders(4), 0);
    // An up from a client that does not hold the key, or of a key nobody holds.
    assert_eq!(h.up(4, 2), Hold::NotHeld);
    assert_eq!(h.down(5, 1), Hold::Forward);
    assert_eq!(h.up(5, 3), Hold::NotHeld);
    assert_eq!(h.holders(5), 1, "a stranger's up leaves the holder");
    // A second down of the same client is no second press.
    assert_eq!(h.down(5, 1), Hold::Held);
    assert_eq!(h.up(5, 1), Hold::Forward);
}

#[test]
fn a_gone_client_releases_only_the_keys_it_held_alone() {
    let mut h = Holders::default();
    h.down(1, 7);
    h.down(1, 8);
    h.down(2, 7);
    h.down(5, 7);
    h.down(6, 8);
    assert_eq!(h.clients(), BTreeSet::from([7, 8]));
    assert_eq!(h.drop_client(7), vec![2, 5]);
    assert_eq!(h.holders(1), 1);
    assert_eq!(h.drop_client(7), Vec::<u32>::new());
    assert_eq!(h.clear(), vec![1, 6]);
    assert_eq!(h.clients(), BTreeSet::new());
}

#[test]
fn the_deck_thresholds_at_their_boundaries() {
    assert!(!silent(1999.0));
    assert!(silent(2000.0));
    assert!(in_press_window(10_000.0));
    assert!(!in_press_window(10_000.0_f64.next_up()));
    assert!(key_log_due(None));
    assert!(!key_log_due(Some(999.999)));
    assert!(key_log_due(Some(1000.0)));
    assert!(!summary_due(59_999.0));
    assert!(summary_due(60_000.0));
    assert_eq!(TICK, std::time::Duration::from_millis(100));
}

#[test]
fn a_key_change_is_recorded_on_a_pressed_change_after_a_press_or_once_a_second_while_viewed() {
    // The pressed flag changed: always.
    assert!(key_record(true, None, false, Some(0.0)));
    // Within 10 s of a press on the key: every change.
    assert!(key_record(false, Some(10_000.0), false, Some(0.0)));
    assert!(!key_record(false, Some(10_001.0), false, Some(0.0)));
    // While a client views the tab: at most once a second.
    assert!(key_record(false, None, true, Some(1000.0)));
    assert!(!key_record(false, None, true, Some(999.0)));
    assert!(key_record(false, None, true, None));
    // Nobody views it and no press: only the summary counts it.
    assert!(!key_record(false, None, false, None));
}

#[test]
fn an_image_is_logged_as_its_fnv_1a_hash() {
    assert_eq!(img_hash(""), "cbf29ce484222325");
    assert_eq!(img_hash("a"), "af63dc4c8601ec8c");
    assert_eq!(img_hash("foobar"), "85944171f73967e8");
    assert_eq!(img_hash("data:image/webp;base64,AAAA"), "89e4799c631f507f");
}

#[test]
fn a_press_is_forwarded_held_not_held_or_refused_offline() {
    let mut deck = Deck::default();
    assert!(!deck.online());
    assert_eq!(deck.press(1, 3, true), PressOutcome::Offline);
    assert_eq!(deck.holders_of(3), 0, "an offline down is not held");
    let mut deck = online();
    assert_eq!(deck.press(1, 3, true), PressOutcome::Forwarded);
    assert_eq!(deck.press(2, 3, true), PressOutcome::Held);
    assert_eq!(deck.press(1, 3, false), PressOutcome::Held);
    assert_eq!(deck.press(9, 3, false), PressOutcome::NotHeld);
    assert_eq!(deck.press(2, 3, false), PressOutcome::Forwarded);
    for (outcome, reason, ack) in [
        (PressOutcome::Forwarded, None, None),
        (PressOutcome::Held, Some("held"), Some((true, None))),
        (PressOutcome::NotHeld, Some("not held"), Some((true, None))),
        (
            PressOutcome::Offline,
            Some("offline"),
            Some((false, Some("offline"))),
        ),
        (
            PressOutcome::NoDeck,
            Some("no Stream Deck"),
            Some((false, Some("no Stream Deck"))),
        ),
    ] {
        assert_eq!(
            (outcome.reason(), outcome.ack()),
            (reason, ack),
            "{outcome:?}"
        );
    }
}

#[test]
fn gaps_and_holds_are_measured_per_client_and_key() {
    let mut deck = online();
    assert_eq!(deck.gap(1, 3, 1000.0), None);
    assert_eq!(deck.gap(1, 3, 1250.5), Some(250.5));
    assert_eq!(deck.gap(2, 3, 1300.0), None, "another client's own gap");
    assert_eq!(deck.gap(1, 4, 1400.0), None, "another key's own gap");
    assert_eq!(deck.forwarded(3, true, 5000.0), None);
    assert_eq!(deck.forwarded(3, false, 5120.0), Some(120.0));
    assert_eq!(
        deck.forwarded(3, false, 5200.0),
        None,
        "no down to measure from"
    );
}

#[test]
fn a_detach_forgets_the_client_and_names_the_keys_to_release() {
    let mut deck = online();
    deck.heard(7, 0.0);
    deck.view(7, true);
    deck.press(7, 2, true);
    deck.press(7, 3, true);
    deck.press(8, 3, true);
    deck.gap(7, 2, 10.0);
    assert_eq!(deck.detach(7), vec![2]);
    assert_eq!(deck.viewers(), Vec::<ClientId>::new());
    assert_eq!(deck.holders_of(3), 1);
    assert_eq!(deck.gap(7, 2, 50.0), None, "its press history is gone");
}

#[test]
fn a_holding_client_is_released_after_two_silent_seconds() {
    let mut deck = online();
    deck.heard(7, 1000.0);
    deck.heard(8, 1000.0);
    deck.heard(9, 0.0);
    deck.press(7, 2, true);
    deck.press(8, 4, true);
    deck.heard(8, 2500.0);
    // 9 is silent but holds nothing.
    assert_eq!(deck.silent_clients(2999.0), vec![]);
    assert_eq!(deck.silent_clients(3000.0), vec![(7, vec![2])]);
    assert_eq!(deck.holders_of(2), 0);
    assert_eq!(deck.silent_clients(4500.0), vec![(8, vec![4])]);
    // A holder never heard (it cannot happen through ws.rs) counts as silent.
    deck.press(5, 6, true);
    assert_eq!(deck.silent_clients(4500.0), vec![(5, vec![6])]);
}

#[test]
fn a_lost_link_keeps_the_held_keys_for_a_release_once_it_is_back() {
    let mut deck = online();
    deck.press(1, 2, true);
    deck.press(2, 2, true);
    deck.press(1, 5, true);
    deck.forwarded(2, true, 10.0);
    deck.link_down();
    assert!(!deck.online());
    assert_eq!(deck.holders_of(2), 0);
    assert_eq!(deck.press(1, 2, false), PressOutcome::Offline);
    assert_eq!(deck.link_up(), vec![2, 5]);
    assert!(deck.online());
    assert_eq!(deck.link_up(), Vec::<u32>::new(), "released once");
    assert_eq!(
        deck.forwarded(2, false, 20.0),
        None,
        "the old hold is forgotten"
    );
}

#[test]
fn the_stop_names_the_keys_to_release_and_the_keys_lost() {
    let mut deck = online();
    deck.press(1, 9, true);
    deck.press(2, 9, true);
    deck.press(2, 1, true);
    assert_eq!(
        deck.stop(),
        StopKeys {
            release: vec![1, 9],
            lost: vec![]
        }
    );
    assert_eq!(
        deck.stop(),
        StopKeys {
            release: vec![],
            lost: vec![]
        }
    );
    // Held, then the link goes: the key cannot be released any more.
    let mut deck = online();
    deck.press(1, 4, true);
    deck.press(1, 2, true);
    deck.link_down();
    assert_eq!(
        deck.stop(),
        StopKeys {
            release: vec![],
            lost: vec![2, 4]
        }
    );
    assert_eq!(
        deck.stop(),
        StopKeys {
            release: vec![],
            lost: vec![]
        }
    );
}

#[test]
fn key_states_merge_and_viewers_get_the_whole_cache() {
    let mut deck = online();
    let (key, _) = deck.apply(
        &update(3, Some("data:a"), Some("#000000"), Some(false)),
        0.0,
    );
    assert_eq!(
        key,
        DeckKey {
            key: 3,
            img: Some("data:a".into()),
            color: Some("#000000".into()),
            pressed: false
        }
    );
    // A missing field keeps its old value.
    let (key, _) = deck.apply(&update(3, None, Some("#ff0000"), None), 1.0);
    assert_eq!(
        (key.img.as_deref(), key.color.as_deref(), key.pressed),
        (Some("data:a"), Some("#ff0000"), false)
    );
    deck.apply(&update(0, Some("data:b"), None, Some(true)), 2.0);
    assert_eq!(
        deck.view(4, true)
            .unwrap()
            .iter()
            .map(|k| k.key)
            .collect::<Vec<_>>(),
        vec![0, 3]
    );
    assert_eq!(deck.viewers(), vec![4]);
    assert_eq!(deck.view(4, false), None);
    assert_eq!(deck.viewers(), Vec::<ClientId>::new());
    // KEYS-CLEAR: every cached key black and released.
    let cleared = deck.clear();
    assert_eq!(
        cleared,
        vec![
            DeckKey {
                key: 0,
                img: None,
                color: Some("#000000".into()),
                pressed: false
            },
            DeckKey {
                key: 3,
                img: None,
                color: Some("#000000".into()),
                pressed: false
            },
        ]
    );
}

#[test]
fn a_key_record_is_due_by_its_rules_and_counts_the_changes_it_stands_for() {
    let mut deck = online();
    // Nobody views, no press: only counted.
    assert_eq!(
        deck.apply(&update(1, Some("data:x"), None, Some(false)), 0.0)
            .1,
        None
    );
    assert_eq!(
        deck.apply(&update(1, Some("data:y"), None, None), 10.0).1,
        None
    );
    // The pressed flag changes: recorded, with the changes since the last record.
    let record = deck
        .apply(&update(1, None, None, Some(true)), 20.0)
        .1
        .unwrap();
    assert_eq!(
        record,
        json!({"key": 1, "pressed": true, "color": null, "img_hash": img_hash("data:y"),
               "img_bytes": 6, "changes": 3})
    );
    // After a press on the key every change for 10 s.
    deck.forwarded(1, true, 100.0);
    assert!(
        deck.apply(&update(1, Some("data:z"), None, None), 10_100.0)
            .1
            .is_some()
    );
    assert!(
        deck.apply(&update(1, Some("data:w"), None, None), 10_101.0)
            .1
            .is_none()
    );
    // While a client views the tab: once a second at most.
    deck.view(2, true);
    let first = deck
        .apply(&update(1, Some("data:v"), None, None), 11_102.0)
        .1
        .unwrap();
    assert_eq!(first["changes"], 2);
    assert!(
        deck.apply(&update(1, Some("data:u"), None, None), 11_500.0)
            .1
            .is_none()
    );
    assert!(
        deck.apply(&update(1, Some("data:t"), None, None), 12_102.0)
            .1
            .is_some()
    );
}

#[test]
fn the_summary_comes_every_minute_while_the_link_is_up() {
    let mut deck = online();
    deck.apply(&update(1, Some("data:x"), None, None), 0.0);
    deck.apply(&update(1, Some("data:y"), None, None), 1.0);
    deck.apply(&update(4, Some("data:z"), None, None), 2.0);
    assert_eq!(deck.summary(59_999.0), None);
    assert_eq!(
        deck.summary(60_000.0),
        Some(json!({"changes": {"1": 2, "4": 1}}))
    );
    assert_eq!(deck.summary(119_999.0), None);
    assert_eq!(deck.summary(120_000.0), Some(json!({"changes": {}})));
    deck.link_down();
    assert_eq!(deck.summary(500_000.0), None, "none while the link is down");
}

#[test]
fn the_record_fields() {
    let record = PressRecord {
        client: 7,
        peer: Some("10.0.0.5"),
        key: 3,
        down: false,
        seq: 12,
        t: 1_000.5,
        hub_ms: 1_250.5,
        offset_ms: Some(200.0),
        gap_ms: Some(180.0),
        hold_ms: Some(175.0),
        why: Some("up"),
        hub_hold_ms: Some(172.5),
        outcome: PressOutcome::Forwarded,
        holders: 0,
    };
    assert_eq!(
        press_fields(&record),
        json!({"client": 7, "peer": "10.0.0.5", "key": 3, "down": false, "seq": 12,
               "t": 1_000.5, "hub_ms": 1_250.5, "offset_ms": 200.0, "delay_ms": 50.0,
               "gap_ms": 180.0, "hold_ms": 175.0, "why": "up", "hub_hold_ms": 172.5,
               "forwarded": true, "reason": null, "holders": 0})
    );
    let held = PressRecord {
        client: 8,
        peer: None,
        key: 3,
        down: true,
        seq: 1,
        t: 0.0,
        hub_ms: 10.0,
        offset_ms: None,
        gap_ms: None,
        hold_ms: None,
        why: None,
        hub_hold_ms: None,
        outcome: PressOutcome::Held,
        holders: 2,
    };
    let fields = press_fields(&held);
    assert_eq!(
        (fields["forwarded"].clone(), fields["reason"].clone()),
        (json!(false), json!("held"))
    );
    assert_eq!(fields["delay_ms"], json!(null));
    let answer = Answer {
        press: Press {
            key: 3,
            down: true,
            from: Some((7, 12)),
        },
        ok: false,
        error: Some("Invalid KEY".into()),
        rtt_ms: Some(3.5),
    };
    assert_eq!(
        ok_fields(&answer),
        json!({"client": 7, "seq": 12, "key": 3, "down": true, "ok": false,
               "error": "Invalid KEY", "rtt_ms": 3.5})
    );
    let own = Answer::offline(Press {
        key: 5,
        down: false,
        from: None,
    });
    assert_eq!(
        ok_fields(&own),
        json!({"client": null, "seq": null, "key": 5, "down": false, "ok": false,
               "error": "offline", "rtt_ms": null})
    );
    assert_eq!(
        release_fields(Some(7), 3, "silent", Some(2010.0)),
        json!({"client": 7, "key": 3, "reason": "silent", "hub_hold_ms": 2010.0})
    );
    assert_eq!(
        link_fields(
            "up",
            Some("5.0.7"),
            Some("1.12.0"),
            None,
            Some(812.5),
            Some(3)
        ),
        json!({"state": "up", "companion": "5.0.7", "api": "1.12.0", "error": null,
               "down_ms": 812.5, "attempts": 3})
    );
    assert_eq!(view_fields(4, true), json!({"client": 4, "on": true}));
    assert_eq!(
        key_fields(
            &DeckKey {
                key: 2,
                img: None,
                color: Some("#00aa00".into()),
                pressed: true
            },
            1
        ),
        json!({"key": 2, "pressed": true, "color": "#00aa00", "img_hash": null, "img_bytes": null,
               "changes": 1})
    );
}
