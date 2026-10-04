//! `Subs::live_value` (#43 PR D): Live's value of a write key as the hub
//! knows it, for a touch's first set (`live_before`).

use super::*;

#[test]
fn a_write_keys_live_value_is_the_cached_value_with_or_without_display() {
    let key = "band|live_set tracks[name=Hand1 #] mixer_device volume|value";
    let mut subs = online();
    assert_eq!(subs.live_value(key), None, "nobody subscribed");
    sub_volume(&mut subs, 1);
    assert_eq!(subs.live_value(key), None, "not resolved yet");
    let out = subs.drain_outgoing();
    answer_all(&mut subs, &out);
    assert_eq!(
        subs.live_value(key),
        Some(&json!(0.85)),
        "the display subscription's"
    );
    // Without the display string (a parameter fader's other targets).
    let mut subs = online();
    subs.subscribe(2, "band", VOLUME, "value", false).unwrap();
    let out = subs.drain_outgoing();
    answer_all(&mut subs, &out);
    assert_eq!(subs.live_value(key), Some(&json!(0.85)));
    subs.on_values("band", &[push("live_10.value", json!(0.6))]);
    assert_eq!(
        subs.live_value(key),
        Some(&json!(0.6)),
        "Live's latest push"
    );
    assert_eq!(
        subs.live_value("band|live_set tracks[name=Hand1 #]|mute"),
        None
    );
    // A binding in error holds no value.
    let mut subs = online();
    sub_volume(&mut subs, 1);
    for o in subs.drain_outgoing() {
        let slots: Vec<Value> = o.commands.iter().map(|_| fail("gone")).collect();
        subs.on_result(&o.instance, &o.uuid, &slots);
    }
    assert!(matches!(subs.cached(VOLUME_KEY), Some(Cached::Error(_))));
    assert_eq!(subs.live_value(key), None);
}
