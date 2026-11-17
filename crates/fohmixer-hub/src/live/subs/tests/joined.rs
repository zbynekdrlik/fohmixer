//! A new hub key joining a Live key another entry holds (#244): at its
//! batch's result it takes that key's push from after the batch was sent.

use super::*;

/// The same volume by `VOLUME` without the display.
const BARE_KEY: &str = "band|live_set tracks[name=Hand1 #] mixer_device volume|value|false";
/// The same volume (Live answers the same object, one Live key) by index.
const BY_INDEX: &str = "live_set tracks 0 mixer_device volume";
const BY_INDEX_KEY: &str = "band|live_set tracks 0 mixer_device volume|value|false";

fn shown(key: &str, value: Value, display: &str) -> LiveValue {
    LiveValue {
        key: key.to_string(),
        value,
        display: Some(display.to_string()),
        error: None,
    }
}

/// A key's state now.
fn cached(subs: &Subs, key: &str) -> Option<Cached> {
    subs.cached(key).cloned()
}

fn value(value: Value, display: Option<&str>) -> Option<Cached> {
    Some(Cached::Value {
        value,
        display: display.map(str::to_string),
    })
}

/// Answers every `value` listener (by name or by index) as the volume's
/// Live key, read at registration (0.85, "0.0 dB"), and the guards.
fn answer(subs: &mut Subs, out: &[Outgoing]) {
    for o in out {
        let slots: Vec<Value> = o
            .commands
            .iter()
            .map(|c| match c["args"]["prop"].as_str().unwrap() {
                "value" => ok_display("live_10.value", json!(0.85), "0.0 dB"),
                "name" => ok("live_11.name", json!("Hand1 #")),
                "tracks" => ok("live_1.tracks", json!([])),
                other => panic!("unexpected listener {other}"),
            })
            .collect();
        subs.on_result(&o.instance, &o.uuid, &slots);
    }
}

/// Client 1 follows the volume by index; resolved.
fn followed() -> Subs {
    let mut subs = online();
    subs.subscribe(1, "band", BY_INDEX, "value", false).unwrap();
    let out = subs.drain_outgoing();
    answer(&mut subs, &out);
    assert_eq!(cached(&subs, BY_INDEX_KEY), value(json!(0.85), None));
    subs.take_deliveries();
    subs
}

fn deliveries_of(subs: &mut Subs, key: &str) -> Vec<(Option<Value>, Option<String>)> {
    subs.take_deliveries()
        .into_iter()
        .filter(|(_, item)| item.sub == key)
        .map(|(_, item)| (item.value, item.display))
        .collect()
}

#[test]
fn a_new_key_with_the_display_takes_its_live_keys_push_after_the_send() {
    let mut subs = followed();
    sub_volume(&mut subs, 2);
    let out = subs.drain_outgoing();
    // The script registered B's listener (reused, now with the display)
    // and pushed a change before the batch's result.
    subs.on_values("band", &[shown("live_10.value", json!(0.6), "-3.00 dB")]);
    assert_eq!(cached(&subs, BY_INDEX_KEY), value(json!(0.6), None));
    answer(&mut subs, &out);
    assert_eq!(
        cached(&subs, VOLUME_KEY),
        value(json!(0.6), Some("-3.00 dB"))
    );
    // Never the older read first.
    assert_eq!(
        deliveries_of(&mut subs, VOLUME_KEY),
        vec![(Some(json!(0.6)), Some("-3.00 dB".to_string()))]
    );
    assert_eq!(cached(&subs, BY_INDEX_KEY), value(json!(0.6), None));
}

#[test]
fn a_new_key_without_the_display_takes_the_value_alone() {
    let mut subs = online();
    sub_volume(&mut subs, 1);
    let out = subs.drain_outgoing();
    answer(&mut subs, &out);
    subs.subscribe(2, "band", VOLUME, "value", false).unwrap();
    let out = subs.drain_outgoing();
    subs.on_values("band", &[shown("live_10.value", json!(0.6), "-3.00 dB")]);
    answer(&mut subs, &out);
    assert_eq!(cached(&subs, BARE_KEY), value(json!(0.6), None));
    assert_eq!(
        cached(&subs, VOLUME_KEY),
        value(json!(0.6), Some("-3.00 dB"))
    );
}

#[test]
fn a_push_without_the_display_the_new_key_wants_leaves_the_results_value() {
    let mut subs = followed();
    sub_volume(&mut subs, 2);
    let out = subs.drain_outgoing();
    // Read before the script's add_listener asked for the display.
    subs.on_values("band", &[push("live_10.value", json!(0.6))]);
    answer(&mut subs, &out);
    assert_eq!(
        cached(&subs, VOLUME_KEY),
        value(json!(0.85), Some("0.0 dB"))
    );
}

#[test]
fn a_push_heard_before_the_batch_was_sent_leaves_the_results_value() {
    let mut subs = followed();
    // Another batch in flight: the push is heard, before B's batch.
    subs.subscribe(3, "band", "live_set", "tracks", false)
        .unwrap();
    let earlier = subs.drain_outgoing();
    subs.on_values("band", &[shown("live_10.value", json!(0.6), "-3.00 dB")]);
    sub_volume(&mut subs, 2);
    let out = subs.drain_outgoing();
    answer(&mut subs, &out);
    assert_eq!(
        cached(&subs, VOLUME_KEY),
        value(json!(0.85), Some("0.0 dB"))
    );
    answer(&mut subs, &earlier);
}

#[test]
fn a_live_key_every_entry_left_keeps_its_push_for_its_new_keys() {
    let mut subs = followed();
    // Two new keys of the Live key in one batch.
    sub_volume(&mut subs, 2);
    subs.subscribe(2, "band", VOLUME, "value", false).unwrap();
    let out = subs.drain_outgoing();
    subs.on_values("band", &[shown("live_10.value", json!(0.6), "-3.00 dB")]);
    // The only entry that held the Live key goes before the result.
    assert!(subs.unsubscribe(1, BY_INDEX_KEY));
    answer(&mut subs, &out);
    // The push is the Live key's newest value whether or not a sibling is
    // left at the result (#245): both new keys take it, and agree.
    assert_eq!(cached(&subs, BARE_KEY), value(json!(0.6), None));
    assert_eq!(
        cached(&subs, VOLUME_KEY),
        value(json!(0.6), Some("-3.00 dB"))
    );
}

#[test]
fn a_listener_error_forgets_its_push_so_its_new_keys_agree() {
    let mut subs = followed();
    sub_volume(&mut subs, 2);
    subs.subscribe(2, "band", VOLUME, "value", false).unwrap();
    let out = subs.drain_outgoing();
    subs.on_values("band", &[shown("live_10.value", json!(0.6), "-3.00 dB")]);
    subs.on_values("band", &[gone("live_10.value")]);
    assert_eq!(
        cached(&subs, BY_INDEX_KEY),
        Some(Cached::Error("gone".to_string()))
    );
    answer(&mut subs, &out);
    assert_eq!(cached(&subs, BARE_KEY), value(json!(0.85), None));
    assert_eq!(
        cached(&subs, VOLUME_KEY),
        value(json!(0.85), Some("0.0 dB"))
    );
}
