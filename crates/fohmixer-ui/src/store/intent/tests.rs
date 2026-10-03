use super::*;
use serde_json::json;

const TARGET: &str = "live_set tracks[name=Hand1 #] mixer_device volume";
const AT: (&str, &str, &str) = ("band", TARGET, "value");

fn key() -> String {
    set_key("band", TARGET, "value")
}

#[test]
fn a_write_is_a_set_with_the_next_sequence_number() {
    let mut intents = Intents::<&str>::default();
    assert!(intents.is_empty());
    let (key, msg) = intents.set(AT, json!(0.5), 1_000.5, false, None);
    assert!(!intents.is_empty());
    assert_eq!(
        key,
        "band|live_set tracks[name=Hand1 #] mixer_device volume|value"
    );
    assert_eq!(
        msg,
        ClientMsg::Set {
            instance: "band".into(),
            target: TARGET.into(),
            prop: "value".into(),
            value: json!(0.5),
            seq: 1,
            t: 1_000.5,
            is_final: false,
        }
    );
    // The sequence is the page's, over every key.
    let (_, mute) = intents.set(
        ("band", "live_set tracks 1", "mute"),
        json!(true),
        1_001.0,
        true,
        None,
    );
    assert!(matches!(
        mute,
        ClientMsg::Set {
            seq: 2,
            is_final: true,
            ..
        }
    ));
    let (_, again) = intents.set(AT, json!(0.6), 1_002.0, true, None);
    assert!(matches!(again, ClientMsg::Set { seq: 3, .. }));
    // One intent per key: the latest.
    assert_eq!(intents.len(), 2);
    assert_eq!(
        intents.open(&key),
        Some(&Intent {
            seq: 3,
            value: json!(0.6),
            t: 1_002.0,
            is_final: true,
        })
    );
}

#[test]
fn an_ack_at_least_as_new_closes_the_intent() {
    let mut intents = Intents::<&str>::default();
    intents.set(AT, json!(0.5), 1.0, false, Some("first"));
    intents.set(AT, json!(0.6), 2.0, false, Some("second"));
    // The ack of seq 1: a newer write is on its way.
    assert_eq!(
        intents.ack(&AckItem::applied(&key(), 1, None)),
        Acked::Stale
    );
    assert_eq!(intents.open(&key()).map(|i| i.seq), Some(2));
    assert_eq!(
        intents.ack(&AckItem::applied(&key(), 2, None)),
        Acked::Confirmed
    );
    assert!(intents.is_empty());
    assert_eq!(
        intents.handler(&key()),
        None,
        "a confirmed write's handler goes"
    );
    // Nothing open: stale.
    assert_eq!(
        intents.ack(&AckItem::applied(&key(), 2, None)),
        Acked::Stale
    );
    // A newer seq than the intent's closes it too.
    intents.set(AT, json!(0.7), 3.0, true, None);
    assert_eq!(
        intents.ack(&AckItem::applied(&key(), 9, Some(json!(0.7)))),
        Acked::Confirmed
    );
    assert!(intents.is_empty());
}

#[test]
fn an_error_fails_with_the_latest_writes_handler() {
    let mut intents = Intents::<&str>::default();
    intents.set(AT, json!(0.4), 1.0, false, Some("old"));
    intents.set(AT, json!(0.5), 2.0, true, Some("flash"));
    assert_eq!(
        intents.ack(&AckItem::failed(&key(), 2, "instance offline")),
        Acked::Failed("instance offline".into(), Some("flash"))
    );
    assert!(intents.is_empty());
    // A write without a handler drops the one before.
    intents.set(AT, json!(0.5), 3.0, false, Some("flash"));
    intents.set(AT, json!(0.6), 4.0, false, None);
    assert_eq!(
        intents.ack(&AckItem::failed(&key(), 4, "refused")),
        Acked::Failed("refused".into(), None)
    );
}

#[test]
fn a_superseded_ack_closes_only_at_its_sequence() {
    let mut intents = Intents::<&str>::default();
    intents.set(AT, json!(0.5), 2.0, false, Some("flash"));
    intents.set(AT, json!(0.6), 3.0, true, Some("flash"));
    // Seq 1 lost to another client, but this page's seq 2 is on its way.
    assert_eq!(intents.ack(&AckItem::superseded(&key(), 1)), Acked::Stale);
    assert_eq!(
        intents.ack(&AckItem::failed(&key(), 1, "late")),
        Acked::Stale
    );
    assert_eq!(intents.len(), 1);
    assert_eq!(
        intents.ack(&AckItem::superseded(&key(), 2)),
        Acked::Superseded
    );
    assert!(intents.is_empty());
    assert_eq!(
        intents.handler(&key()),
        None,
        "a superseded write's handler goes"
    );
    // Another key's ack leaves an intent alone.
    intents.set(AT, json!(0.1), 4.0, true, None);
    assert_eq!(
        intents.ack(&AckItem::applied("band|live_set|tempo", 3, None)),
        Acked::Stale
    );
    assert_eq!(intents.len(), 1);
}

#[test]
fn a_write_the_socket_could_not_take_stays_open_with_its_handler() {
    let mut intents = Intents::<&str>::default();
    let (key, _) = intents.set(AT, json!(0.5), 1.0, true, Some("flash"));
    assert_eq!(intents.open(&key).map(|i| i.seq), Some(1), "still open");
    // A stale ack keeps the handler of the newer write.
    intents.set(AT, json!(0.6), 2.0, true, Some("newer"));
    assert_eq!(intents.ack(&AckItem::failed(&key, 1, "late")), Acked::Stale);
    assert_eq!(
        intents.ack(&AckItem::failed(&key, 2, "refused")),
        Acked::Failed("refused".into(), Some("newer"))
    );
}

#[test]
fn each_state_has_its_name_its_openness_and_its_ghost() {
    let all = [
        State::Confirmed,
        State::Sending,
        State::Unconfirmed,
        State::NotSent,
    ];
    assert_eq!(
        all.map(State::name),
        ["confirmed", "sending", "unconfirmed", "not_sent"]
    );
    assert_eq!(all.map(State::is_open), [false, true, true, true]);
    assert_eq!(all.map(State::shows_ghost), [false, false, true, true]);
}

#[test]
fn the_bounds_are_one_and_two_seconds() {
    assert_eq!(UNCONFIRMED_MS, 1000.0);
    assert_eq!(RESEND_MAX_AGE_MS, 2000.0);
}

#[test]
fn a_release_is_unconfirmed_after_1000_ms_without_its_ack() {
    let mut intents = Intents::<&str>::default();
    assert_eq!(intents.state(&key(), 0.0), State::Confirmed, "no write");
    // A held control's write is on its way however long it waits.
    intents.set(AT, json!(0.5), 100.0, false, None);
    assert_eq!(intents.state(&key(), 100.0), State::Sending);
    assert_eq!(intents.state(&key(), 60_000.0), State::Sending, "held");
    // Let go at 200: 999 ms later it is still sending, 1000 ms later not.
    intents.release(&key(), 200.0);
    assert_eq!(intents.state(&key(), 1_199.0), State::Sending);
    assert_eq!(
        intents.state(&key(), 1_200.0_f64.next_down()),
        State::Sending
    );
    assert_eq!(intents.state(&key(), 1_200.0), State::Unconfirmed);
    assert_eq!(intents.state(&key(), 50_000.0), State::Unconfirmed);
    // A final write (a tap, a release with an unsent move) is let go at
    // its own time.
    let (mute, _) = intents.set(
        ("band", "live_set tracks 1", "mute"),
        json!(true),
        500.0,
        true,
        None,
    );
    assert_eq!(intents.state(&mute, 1_499.0), State::Sending);
    assert_eq!(intents.state(&mute, 1_500.0), State::Unconfirmed);
    // The ack closes it.
    intents.ack(&AckItem::applied(&mute, 2, None));
    assert_eq!(intents.state(&mute, 1_600.0), State::Confirmed);
    assert_eq!(intents.state(&key(), 1_600.0), State::Unconfirmed);
}

#[test]
fn a_release_marks_only_an_open_write_and_only_once() {
    let mut intents = Intents::<&str>::default();
    intents.release(&key(), 50.0);
    assert_eq!(
        intents.state(&key(), 5_000.0),
        State::Confirmed,
        "nothing open"
    );
    assert!(intents.is_empty());
    intents.set(AT, json!(0.5), 100.0, false, None);
    assert_eq!(intents.open(&key()).map(|i| i.is_final), Some(false));
    intents.release(&key(), 200.0);
    intents.release(&key(), 900.0);
    assert_eq!(
        intents.state(&key(), 1_200.0),
        State::Unconfirmed,
        "the first release counts"
    );
    assert_eq!(
        intents.open(&key()).map(|i| i.is_final),
        Some(true),
        "a released write is final"
    );
    // A new touch's move is held again until its own release.
    intents.set(AT, json!(0.6), 1_300.0, false, None);
    assert_eq!(intents.state(&key(), 9_000.0), State::Sending);
}

/// The `set` of `(instance, target, prop)` in `sets`.
fn set_of<'a>(sets: &'a [ClientMsg], prop_of: &str) -> &'a ClientMsg {
    sets.iter()
        .find(|m| matches!(m, ClientMsg::Set { prop, .. } if prop == prop_of))
        .expect("a set of that prop")
}

#[test]
fn an_instance_back_gets_its_held_writes_and_releases_younger_than_2000_ms() {
    let mut intents = Intents::<&str>::default();
    let held_at = ("band", "live_set tracks 1", "panning");
    let (held, _) = intents.set(held_at, json!(-0.2), 10.0, false, None);
    let (young, _) = intents.set(AT, json!(0.6), 20.0, false, Some("flash"));
    intents.release(&young, 1_000.0);
    let (other, _) = intents.set(("master", TARGET, "value"), json!(0.3), 1_000.0, true, None);
    // The band instance is back 1999.x ms after the release.
    let now = 3_000.0_f64.next_down();
    let resend = intents.resend("band", now);
    assert!(resend.not_sent.is_empty());
    assert_eq!(resend.sets.len(), 2, "the master's write waits for its own");
    assert_eq!(
        set_of(&resend.sets, "panning"),
        &ClientMsg::Set {
            instance: "band".into(),
            target: "live_set tracks 1".into(),
            prop: "panning".into(),
            value: json!(-0.2),
            seq: 4,
            t: now,
            is_final: false,
        },
        "a held control's value, as a new set at the time it goes"
    );
    assert_eq!(
        set_of(&resend.sets, "value"),
        &ClientMsg::Set {
            instance: "band".into(),
            target: TARGET.into(),
            prop: "value".into(),
            value: json!(0.6),
            seq: 5,
            t: now,
            is_final: true,
        },
        "a release younger than 2000 ms, final"
    );
    // Only an ack of the new sequence numbers closes them.
    assert_eq!(
        intents.ack(&AckItem::applied(&young, 2, None)),
        Acked::Stale
    );
    assert_eq!(
        intents.ack(&AckItem::applied(&held, 4, None)),
        Acked::Confirmed
    );
    assert_eq!(
        intents.ack(&AckItem::failed(&young, 5, "refused")),
        Acked::Failed("refused".into(), Some("flash"))
    );
    // The master is back: its write goes, numbered after the others.
    let master = intents.resend("master", 1_500.0);
    assert!(matches!(
        master.sets.as_slice(),
        [ClientMsg::Set { seq: 6, .. }]
    ));
    assert_eq!(
        intents.open(&other).map(|i| (i.seq, i.t)),
        Some((6, 1_500.0))
    );
}

#[test]
fn a_release_2000_ms_old_is_not_sent_again_but_kept_until_touched() {
    let mut intents = Intents::<&str>::default();
    let (key, _) = intents.set(AT, json!(0.6), 0.0, true, None);
    let resend = intents.resend("band", 2_000.0);
    assert!(resend.sets.is_empty(), "too old to send blindly (L4)");
    assert_eq!(resend.not_sent, vec![key.clone()]);
    assert_eq!(intents.state(&key, 2_000.0), State::NotSent);
    assert_eq!(intents.state(&key, 1e9), State::NotSent, "until touched");
    assert_eq!(
        intents.open(&key).map(|i| i.seq),
        Some(1),
        "unsent, not renumbered"
    );
    // A later reconnect neither sends it nor counts it again.
    assert_eq!(intents.resend("band", 2_100.0), Resend::default());
    // A touch on a held write (another instance's, under a finger)
    // changes nothing.
    let (held, _) = intents.set(
        ("master", TARGET, "value"),
        json!(0.1),
        2_150.0,
        false,
        None,
    );
    intents.touch(&held);
    assert_eq!(intents.state(&held, 9_000.0), State::Sending);
    let (mute, _) = intents.set(
        ("band", "live_set tracks 1", "mute"),
        json!(true),
        2_200.0,
        true,
        None,
    );
    assert_eq!(intents.len(), 3);
    // A touch on the not-sent control drops it: confirmed (nothing open).
    intents.touch(&key);
    assert_eq!(intents.state(&key, 2_300.0), State::Confirmed);
    assert_eq!(intents.len(), 2);
    // A release just younger than 2000 ms still goes.
    let resend = intents.resend("band", 4_200.0_f64.next_down());
    assert!(resend.not_sent.is_empty());
    assert!(matches!(
        resend.sets.as_slice(),
        [ClientMsg::Set {
            seq: 4,
            is_final: true,
            ..
        }]
    ));
    let resend = intents.resend("band", 4_200.0);
    assert_eq!(
        resend.not_sent,
        vec![mute.clone()],
        "and then it is too old"
    );
    // A new write on a not-sent key replaces it.
    intents.set(
        ("band", "live_set tracks 1", "mute"),
        json!(false),
        5_000.0,
        true,
        None,
    );
    assert_eq!(intents.state(&mute, 5_000.0), State::Sending);
}

#[test]
fn a_touch_holds_an_open_write_again_and_drops_a_not_sent_one_with_its_handler() {
    let mut intents = Intents::<&str>::default();
    // Released at 0, unconfirmed, touched again at 1500 and held still.
    let (key, _) = intents.set(AT, json!(0.6), 0.0, true, Some("flash"));
    assert_eq!(intents.state(&key, 1_500.0), State::Unconfirmed);
    intents.touch(&key);
    assert_eq!(intents.state(&key, 1_500.0), State::Sending, "held again");
    assert_eq!(intents.open(&key).map(|i| i.is_final), Some(false));
    // Back after 5 s with the finger still on it: a held control's
    // value goes (L4), whatever the age of the first release.
    let resend = intents.resend("band", 5_000.0);
    assert!(resend.not_sent.is_empty());
    assert!(matches!(
        resend.sets.as_slice(),
        [ClientMsg::Set {
            seq: 2,
            is_final: false,
            ..
        }]
    ));
    // Its release counts from the new one.
    intents.release(&key, 6_000.0);
    assert_eq!(intents.state(&key, 6_999.0), State::Sending);
    assert_eq!(intents.state(&key, 7_000.0), State::Unconfirmed);
    // A not-sent write is dropped by a touch, with its handler.
    let resend = intents.resend("band", 8_000.0);
    assert_eq!(resend.not_sent, vec![key.clone()]);
    assert_eq!(intents.handler(&key), Some(&"flash"));
    intents.touch(&key);
    assert_eq!(intents.state(&key, 8_000.0), State::Confirmed);
    assert_eq!(intents.handler(&key), None, "its handler goes with it");
    intents.touch(&key);
    assert!(intents.is_empty(), "nothing open: nothing to touch");
}

#[test]
fn a_number_is_the_same_value_within_one_millionth() {
    assert!(same_number(0.0, 0.0));
    assert!(same_number(0.0, 1e-6) && same_number(1e-6, 0.0));
    assert!(!same_number(0.0, 1e-6_f64.next_up()));
    // Live's float32 of a volume.
    assert!(same_number(0.6, 0.6000000238418579));
    assert!(!same_number(0.6, 0.6001));
    // Relative above 1: a frequency of 2000 Hz within 2 mHz.
    assert!(same_number(2000.0, 2000.0015));
    assert!(same_number(-2000.0, -2000.0015));
    assert!(!same_number(2000.0, 2000.0025));
    assert!(!same_number(-2000.0, -2000.0025));
    assert!(!same_number(1.0, -1.0));
}

#[test]
fn a_not_sent_write_live_already_holds_is_closed_by_its_value() {
    let mut intents = Intents::<&str>::default();
    let (key, _) = intents.set(AT, json!(0.6), 0.0, true, Some("flash"));
    // While it is on its way only its ack closes it.
    intents.live_value(&key, &json!(0.6));
    assert_eq!(intents.state(&key, 10.0), State::Sending);
    intents.resend("band", 2_000.0);
    assert_eq!(intents.state(&key, 2_000.0), State::NotSent);
    // Another value: the release never reached Live.
    intents.live_value(&key, &json!(0.5));
    intents.live_value("band|live_set tracks 1|mute", &json!(0.6));
    assert_eq!(intents.state(&key, 2_000.0), State::NotSent);
    // Live's float32 of it: the hub applied it before the link went.
    intents.live_value(&key, &json!(0.6000000238418579));
    assert_eq!(intents.state(&key, 2_000.0), State::Confirmed);
    assert_eq!(intents.handler(&key), None);
    // A flag is the same value only exactly.
    let (mute, _) = intents.set(
        ("band", "live_set tracks 1", "mute"),
        json!(true),
        0.0,
        true,
        None,
    );
    intents.resend("band", 2_000.0);
    intents.live_value(&mute, &json!(false));
    intents.live_value(&mute, &json!(1));
    assert_eq!(intents.state(&mute, 2_000.0), State::NotSent);
    intents.live_value(&mute, &json!(true));
    assert_eq!(intents.state(&mute, 2_000.0), State::Confirmed);
}
