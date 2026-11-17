//! A push ahead of its batch's result (#243): kept while a batch is in
//! flight, applied after the result that made its key known.

use super::*;

const PLAYING_KEY: &str = "band|live_set|is_playing|false";

fn deliveries_of(subs: &mut Subs, key: &str) -> Vec<Option<Value>> {
    subs.take_deliveries()
        .into_iter()
        .filter(|(_, item)| item.sub == key)
        .map(|(_, item)| item.value)
        .collect()
}

#[test]
fn a_push_before_the_result_is_applied_over_the_value_read_at_registration() {
    let mut subs = online();
    sub_volume(&mut subs, 1);
    let out = subs.drain_outgoing();
    // The script registered the listener and pushed a change on a tick
    // before the batch's result.
    subs.on_values("band", &[push("live_10.value", json!(0.6))]);
    assert_eq!(subs.kept("band"), 1);
    assert_eq!(subs.cached(VOLUME_KEY), None, "not known yet");
    answer_all(&mut subs, &out);
    assert_eq!(
        subs.cached(VOLUME_KEY),
        Some(&Cached::Value {
            value: json!(0.6),
            display: None
        })
    );
    // The result's value, then the newer push.
    assert_eq!(
        deliveries_of(&mut subs, VOLUME_KEY),
        vec![Some(json!(0.85)), Some(json!(0.6))]
    );
    assert_eq!(subs.kept("band"), 0, "no batch in flight: nothing kept");
}

#[test]
fn kept_pushes_are_applied_in_arrival_order_frame_by_frame() {
    let mut subs = online();
    sub_volume(&mut subs, 1);
    let out = subs.drain_outgoing();
    subs.on_values("band", &[push("live_10.value", json!(0.6))]);
    subs.on_values("band", &[push("live_10.value", json!(0.4))]);
    answer_all(&mut subs, &out);
    assert_eq!(
        deliveries_of(&mut subs, VOLUME_KEY),
        vec![Some(json!(0.85)), Some(json!(0.6)), Some(json!(0.4))]
    );
}

#[test]
fn a_kept_listener_error_drops_the_listener_after_the_result() {
    let mut subs = online();
    sub_volume(&mut subs, 1);
    let out = subs.drain_outgoing();
    subs.on_values("band", &[gone("live_10.value")]);
    answer_all(&mut subs, &out);
    assert_eq!(
        subs.cached(VOLUME_KEY),
        Some(&Cached::Error("gone".to_string()))
    );
    assert_eq!(subs.listeners("band"), 2, "the guards' only");
}

#[test]
fn a_push_with_no_batch_in_flight_is_dropped_as_before() {
    let mut subs = online();
    subs.on_values("band", &[push("live_10.value", json!(0.6))]);
    assert_eq!(subs.kept("band"), 0);
    sub_volume(&mut subs, 1);
    let out = subs.drain_outgoing();
    answer_all(&mut subs, &out);
    assert_eq!(
        deliveries_of(&mut subs, VOLUME_KEY),
        vec![Some(json!(0.85))]
    );
}

#[test]
fn a_push_kept_before_its_batch_was_sent_is_older_than_its_result_and_goes() {
    let mut subs = online();
    // A batch of another key is in flight.
    subs.subscribe(1, "band", "live_set", "is_playing", false)
        .unwrap();
    let first = subs.drain_outgoing();
    // An older listener of the volume pushes (its entry went before).
    subs.on_values("band", &[push("live_10.value", json!(0.6))]);
    assert_eq!(subs.kept("band"), 1);
    // The volume's batch is sent after it: its read is the newer value.
    sub_volume(&mut subs, 2);
    let second = subs.drain_outgoing();
    answer_all(&mut subs, &second);
    assert_eq!(
        deliveries_of(&mut subs, VOLUME_KEY),
        vec![Some(json!(0.85))]
    );
    assert_eq!(subs.kept("band"), 0, "taken by the result that knew it");
    answer_all(&mut subs, &first);
    assert!(matches!(
        subs.cached(PLAYING_KEY),
        Some(Cached::Value { .. })
    ));
}

#[test]
fn a_push_stays_kept_until_no_batch_of_its_instance_is_in_flight() {
    let mut subs = online();
    subs.subscribe(1, "band", "live_set", "is_playing", false)
        .unwrap();
    let first = subs.drain_outgoing();
    sub_volume(&mut subs, 2);
    let second = subs.drain_outgoing();
    // A key neither batch makes known.
    subs.on_values("band", &[push("live_99.value", json!(1))]);
    // Another instance's push is never kept for this one's batches.
    subs.on_values("master", &[push("live_98.value", json!(1))]);
    assert_eq!(subs.kept("master"), 0);
    answer_all(&mut subs, &first);
    assert_eq!(
        subs.kept("band"),
        1,
        "the volume's batch is still in flight"
    );
    answer_all(&mut subs, &second);
    assert_eq!(subs.kept("band"), 0);
}

#[test]
fn a_disconnect_forgets_the_kept_pushes() {
    let mut subs = online();
    sub_volume(&mut subs, 1);
    subs.drain_outgoing();
    subs.on_values("band", &[push("live_10.value", json!(0.6))]);
    assert_eq!(subs.kept("band"), 1);
    subs.disconnected("band");
    assert_eq!(subs.kept("band"), 0);
}

#[test]
fn a_release_only_batch_keeps_nothing() {
    let mut subs = online();
    sub_volume(&mut subs, 1);
    let out = subs.drain_outgoing();
    answer_all(&mut subs, &out);
    // The volume's only subscriber leaves: its listener's removal goes out.
    assert!(subs.unsubscribe(1, VOLUME_KEY));
    let out = subs.drain_outgoing();
    assert!(!out.is_empty(), "the removals");
    assert!(
        commands(&out)
            .iter()
            .all(|(_, name, _)| name == "remove_listener")
    );
    subs.on_values("band", &[push("live_10.value", json!(0.6))]);
    assert_eq!(subs.kept("band"), 0);
}

#[test]
fn pushes_dropped_beyond_the_bound_are_logged() {
    let log = crate::ws::tests::logged(|| {
        warn_overflow(
            "band",
            &Overflow {
                dropped: 2,
                total: 7,
            },
        );
    });
    assert!(log.contains(" WARN "), "{log}");
    assert!(
        log.contains("more Live pushes came than the hub keeps"),
        "{log}"
    );
    assert!(log.contains("instance=\"band\""), "{log}");
    assert!(log.contains("dropped=2"), "{log}");
    assert!(log.contains("total=7"), "{log}");
    assert!(log.contains("max=1024"), "{log}");
}
