//! Tests of `trace.rs`: the flight recorder's events, its ring and backlog,
//! when a batch goes, what it holds, and when its events leave the ring.

use super::*;

/// An event of kind `kind` whose JSON is exactly `len` bytes.
fn sized_kind(kind: &str, len: usize) -> Value {
    let base = json!({"ev": kind, "t": 0, "pad": ""}).to_string().len();
    json!({"ev": kind, "t": 0, "pad": "a".repeat(len - base)})
}

/// A non-essential event whose JSON is exactly `len` bytes.
fn sized(len: usize) -> Value {
    sized_kind("x", len)
}

/// Non-essential event number `i` (a single digit) of exactly `len` bytes.
fn sized_at(i: usize, len: usize) -> Value {
    let mut event = sized(len);
    event["t"] = json!(i);
    event
}

/// Event number `i` (a small, non-essential one).
fn event(i: usize) -> Value {
    json!({"ev": "x", "t": i})
}

/// The events of a batch.
fn events_of(batch: &str) -> Vec<Value> {
    match serde_json::from_str::<ClientMsg>(batch).expect("a batch is a client message") {
        ClientMsg::Trace { events } => events,
        other => panic!("not a trace: {other:?}"),
    }
}

/// The `t` of each event of a batch.
fn ts(batch: &str) -> Vec<u64> {
    events_of(batch)
        .iter()
        .map(|e| e["t"].as_u64().expect("a t"))
        .collect()
}

fn set_msg() -> ClientMsg {
    ClientMsg::Set {
        instance: "band".into(),
        target: "live_set tracks[name=Vox 1] mixer_device volume".into(),
        prop: "value".into(),
        value: json!(0.75),
        seq: 12,
        t: 5_000.125,
        is_final: true,
    }
}

#[test]
fn the_bounds_are_these() {
    assert_eq!(MAX_EVENTS, 20_000);
    assert_eq!(MAX_BYTES, 2_097_152);
    assert_eq!(UPLOAD_MS, 2000.0);
    assert_eq!(BATCH_BYTES, 1_000);
    assert_eq!(ENVELOPE, 28);
    assert_eq!(batch_text(&[]).len(), ENVELOPE);
    assert_eq!(RATE_BYTES_PER_S, 10_240.0);
    assert_eq!(BACKLOG_BYTES, 786_432);
    assert_eq!(BUFFERED_MAX, 1_024);
    assert_eq!(LONG_FRAME_MS, 50.0);
    assert!(!is_long_frame(50.0));
    assert!(is_long_frame(50.0_f64.next_up()));
    assert!(!is_long_frame(16.7));
    assert!(is_long_frame(139.0));
    assert!(takes_batch(0));
    assert!(takes_batch(1_024));
    assert!(!takes_batch(1_025));
}

#[test]
fn what_a_touch_and_an_outage_need_is_essential_moves_round_trips_and_long_frames_are_not() {
    for ev in [
        "touch",
        "dropout",
        "reset",
        "sock",
        "visibility",
        "overflow",
        "intent",
    ] {
        assert!(is_essential(ev), "{ev}");
    }
    for ev in ["mv", "rtt", "frame", "x", ""] {
        assert!(!is_essential(ev), "{ev}");
    }
}

#[test]
fn each_event_carries_its_kind_its_page_time_and_its_facts() {
    let keys = vec!["band|live_set tracks[name=Vox 1] mixer_device volume|value".to_string()];
    assert_eq!(
        touch(1_000.5, "up", &keys, 7),
        json!({"ev": "touch", "t": 1_000.5, "what": "up", "keys": keys, "pointer": 7})
    );
    assert_eq!(
        sock(3_000.0, "open", Some(4), None, None),
        json!({"ev": "sock", "t": 3_000.0, "what": "open", "socket": 4})
    );
    assert_eq!(
        sock(3_100.0, "close", None, Some(1006), None),
        json!({"ev": "sock", "t": 3_100.0, "what": "close", "code": 1006})
    );
    assert_eq!(
        sock(
            3_200.0,
            "drop",
            None,
            None,
            Some("the hub has been silent for 3 s")
        ),
        json!({"ev": "sock", "t": 3_200.0, "what": "drop",
               "reason": "the hub has been silent for 3 s"})
    );
}

#[test]
fn a_sets_sequence_names_its_frames_move_record() {
    assert_eq!(seq_of(&set_msg()), Some(12));
    assert_eq!(seq_of(&ClientMsg::Unsub { sub: "x".into() }), None);
}

#[test]
fn an_intent_change_is_its_time_key_sequence_and_state() {
    let key = "band|live_set tracks[name=Vox 1] mixer_device volume|value";
    assert_eq!(
        intent(7_000.5, key, 31, "unconfirmed"),
        json!({"ev": "intent", "t": 7_000.5, "key": key, "seq": 31, "state": "unconfirmed"})
    );
    assert_eq!(
        intent(9_100.25, key, 31, "not_sent")["state"],
        json!("not_sent")
    );
}

#[test]
fn a_batch_is_a_trace_message_of_its_events() {
    let a = event(1).to_string();
    let b = json!({"ev": "dropout", "t": 2.5, "ms": 420.0}).to_string();
    let text = batch_text(&[a.as_str(), b.as_str()]);
    assert_eq!(
        events_of(&text),
        vec![event(1), json!({"ev": "dropout", "t": 2.5, "ms": 420.0})]
    );
    assert_eq!(events_of(&batch_text(&[])), Vec::<Value>::new());
}

#[test]
fn the_first_batch_goes_at_once_then_every_2_s_or_at_once_when_asked() {
    let mut r = Recorder::default();
    assert_eq!(r.upload(0.0, true, 0, 1), None, "nothing to send");
    r.push(&event(1));
    assert_eq!(r.upload(10.0, false, 0, 1), None, "no hello");
    assert_eq!(
        r.upload(10.0, true, 1_025, 1),
        None,
        "the socket is backed up"
    );
    let first = r.upload(10.0, true, 0, 1).expect("the first batch at once");
    assert_eq!(ts(&first), [1]);
    r.push(&event(2));
    assert_eq!(r.upload(2_010.0_f64.next_down(), true, 0, 2), None);
    let second = r.upload(2_010.0, true, 0, 2).expect("2 s later");
    assert_eq!(ts(&second), [2], "only what was not sent");
    r.push(&event(3));
    assert_eq!(r.upload(2_100.0, true, 0, 3), None);
    r.soon();
    let third = r.upload(2_100.0, true, 0, 3).expect("at once when asked");
    assert_eq!(ts(&third), [3]);
    r.push(&event(4));
    assert_eq!(r.upload(2_200.0, true, 0, 4), None, "at once only once");
    // `soon` waits for a socket that takes it.
    r.soon();
    assert_eq!(r.upload(2_300.0, false, 0, 4), None);
    assert_eq!(r.upload(2_300.0, true, 9_999, 4), None);
    assert_eq!(ts(&r.upload(2_400.0, true, 0, 4).expect("now")), [4]);
}

#[test]
fn no_batch_goes_in_a_tick_after_a_set_went() {
    let mut r = Recorder::default();
    r.push(&event(1));
    r.set_went();
    assert_eq!(r.upload(0.0, true, 0, 1), None, "a finger moves a fader");
    let batch = r.upload(100.0, true, 0, 1).expect("the next quiet tick");
    assert_eq!(ts(&batch), [1]);
    // A tick forgets its sets even when nothing would go.
    let mut r = Recorder::default();
    r.set_went();
    assert_eq!(r.upload(0.0, false, 0, 1), None);
    r.push(&event(2));
    assert_eq!(ts(&r.upload(100.0, true, 0, 1).expect("quiet")), [2]);
}

#[test]
fn each_batch_waits_for_the_previous_ones_bytes_at_10_kb_a_second() {
    // 972 bytes of event in 28 of envelope: 1 000 bytes, 97.65625 ms at
    // 10 240 bytes a second, inside the link's 100 ms tick.
    let mut r = Recorder::default();
    r.push(&sized(972));
    let first = r.upload(0.0, true, 0, 1).expect("a batch");
    assert_eq!(first.len(), 1_000);
    r.push(&sized(972));
    r.soon();
    assert_eq!(
        r.upload(97.656_25_f64.next_down(), true, 0, 2),
        None,
        "the cap"
    );
    assert_eq!(ts(&r.upload(97.656_25, true, 0, 2).expect("paid")), [0]);
}

#[test]
fn a_full_batch_goes_as_soon_as_the_cap_lets_it_smaller_amounts_every_2_s() {
    let mut r = Recorder::default();
    for _ in 0..3 {
        r.push(&sized(600));
    }
    assert_eq!(r.backlog(), 1_800);
    // 600 bytes of event, 628 of message: 61.3 ms at the cap.
    assert_eq!(
        events_of(&r.upload(0.0, true, 0, 1).expect("a batch")).len(),
        1
    );
    assert_eq!(r.backlog(), 1_200, "a full batch waits");
    assert_eq!(r.upload(61.0, true, 0, 2), None, "the cap");
    assert_eq!(
        events_of(
            &r.upload(62.0, true, 0, 2)
                .expect("as soon as the cap lets it")
        )
        .len(),
        1
    );
    assert_eq!(r.backlog(), 600);
    assert_eq!(
        r.upload(200.0, true, 0, 3),
        None,
        "less than a batch: every 2 s"
    );
    assert_eq!(r.upload(2_062.0_f64.next_down(), true, 0, 3), None);
    assert!(r.upload(2_062.0, true, 0, 3).is_some());
    assert_eq!(r.backlog(), 0);
    // Events that fill a 1 000-byte message with the envelope are a full
    // batch (972 bytes); one byte less waits its 2 s.
    let mut r = Recorder::default();
    r.push(&sized(100));
    r.push(&sized(972));
    let _ = r.upload(0.0, true, 0, 1).expect("a batch");
    assert_eq!(r.backlog(), 972);
    assert_eq!(
        events_of(&r.upload(98.0, true, 0, 2).expect("a full batch")).len(),
        1
    );
    let mut r = Recorder::default();
    r.push(&sized(100));
    r.push(&sized(971));
    let _ = r.upload(0.0, true, 0, 1).expect("a batch");
    assert_eq!(r.upload(98.0, true, 0, 2), None, "not full: 2 s");
}

#[test]
fn the_commas_between_events_count_toward_a_full_batch() {
    // After a first batch (its 128 bytes pace 12.5 ms), 28 + 500 + 1 + 471
    // = 1 000: the two events fill a message.
    let mut r = Recorder::default();
    r.push(&sized(100));
    let _ = r.upload(0.0, true, 0, 1).expect("a batch");
    r.push(&sized(500));
    r.push(&sized(471));
    let full = r.upload(98.0, true, 0, 2).expect("a full batch");
    assert_eq!(full.len(), 1_000);
    assert_eq!(events_of(&full).len(), 2);
    // One byte less: 999, not full, waits its 2 s.
    let mut r = Recorder::default();
    r.push(&sized(100));
    let _ = r.upload(0.0, true, 0, 1).expect("a batch");
    r.push(&sized(500));
    r.push(&sized(470));
    assert_eq!(r.upload(98.0, true, 0, 2), None, "not full: 2 s");
}

#[test]
fn a_batch_leaves_the_ring_on_the_pong_of_the_ping_after_it() {
    let mut r = Recorder::default();
    for i in 1..=3 {
        r.push(&event(i));
    }
    let first = r.upload(0.0, true, 0, 10).expect("a batch");
    assert_eq!(ts(&first), [1, 2, 3]);
    r.push(&event(4));
    r.soon();
    let second = r.upload(50.0, true, 0, 11).expect("a batch");
    assert_eq!(ts(&second), [4]);
    assert_eq!(r.len(), 4, "nothing proved yet");
    r.proved(9);
    assert_eq!(r.len(), 4, "a pong of an earlier ping proves nothing");
    r.proved(10);
    assert_eq!(r.len(), 1, "the first batch is logged");
    assert!(!r.is_empty());
    assert_eq!(r.bytes, event(4).to_string().len());
    r.proved(12);
    assert!(r.is_empty());
    assert_eq!(r.bytes, 0);
    assert_eq!(r.sent, 0);
    assert_eq!(r.backlog(), 0);
    // Nothing waits: no batch.
    r.soon();
    assert_eq!(r.upload(5_000.0, true, 0, 13), None);
}

#[test]
fn a_lost_socket_sends_the_unproved_batches_again_first() {
    let mut r = Recorder::default();
    r.push(&event(1));
    r.push(&event(2));
    let lost = r.upload(0.0, true, 0, 3).expect("a batch");
    assert_eq!(r.backlog(), 0);
    r.push(&event(3));
    r.requeue();
    let all: Vec<String> = (1..=3).map(|i| event(i).to_string()).collect();
    assert_eq!(r.backlog(), all.iter().map(String::len).sum::<usize>());
    // As soon as the cap lets it after the next hello, the backlog first.
    let again = r.upload(100.0, true, 0, 7).expect("again");
    let all: Vec<&str> = all.iter().map(String::as_str).collect();
    assert_eq!(again, batch_text(&all));
    assert_eq!(ts(&lost), [1, 2]);
    // The old proof is gone with the socket: only the new one counts.
    r.proved(5);
    assert_eq!(r.len(), 3);
    r.proved(7);
    assert!(r.is_empty());
}

#[test]
fn a_batch_is_at_most_1000_bytes_with_its_envelope_at_least_one_event() {
    // 28 + 500 + 1 + 471 = 1 000: one batch.
    let mut r = Recorder::default();
    r.push(&sized(500));
    r.push(&sized(471));
    let both = r.upload(0.0, true, 0, 1).expect("a batch");
    assert_eq!(events_of(&both).len(), 2);
    assert_eq!(both.len(), 1_000);
    // One byte more: two batches.
    let mut r = Recorder::default();
    r.push(&sized(500));
    r.push(&sized(472));
    let one = r.upload(0.0, true, 0, 1).expect("a batch");
    assert_eq!(events_of(&one), vec![sized(500)]);
    r.soon();
    let two = r.upload(1_000.0, true, 0, 2).expect("a batch");
    assert_eq!(events_of(&two), vec![sized(472)]);
    // An event over the bound goes alone.
    let mut r = Recorder::default();
    r.push(&sized(3_000));
    r.push(&event(1));
    let big = r.upload(0.0, true, 0, 1).expect("a batch");
    assert_eq!(events_of(&big), vec![sized(3_000)]);
    // Many small events: as many as fit.
    let mut r = Recorder::default();
    let len = event(0).to_string().len();
    for _ in 0..200 {
        r.push(&event(0));
    }
    let fit = r.upload(0.0, true, 0, 1).expect("a batch");
    let envelope = batch_text(&[]).len();
    assert_eq!(
        events_of(&fit).len(),
        (BATCH_BYTES - envelope + 1) / (len + 1)
    );
    assert!(fit.len() <= BATCH_BYTES);
}

#[test]
fn past_the_bound_the_oldest_droppable_events_go_and_are_counted() {
    let mut r = Recorder::default();
    // A batch on its way (unproved) never goes.
    r.push(&sized(1_000));
    let flying = r.upload(0.0, true, 0, 1).expect("a batch");
    assert_eq!(events_of(&flying).len(), 1);
    r.push(&sized_kind("touch", 1_000));
    r.push(&sized_kind("mv", 1_000));
    r.push_essential(&sized_kind("mv", 1_000));
    r.push(&sized_kind("frame", 1_000));
    let rtts = (BACKLOG_BYTES - 4_000) / 1_000;
    for _ in 0..rtts {
        r.push(&sized_kind("rtt", 1_000));
    }
    let within = 4_000 + rtts * 1_000;
    assert!(BACKLOG_BYTES - within < 1_000);
    assert_eq!(r.backlog(), within, "within the bound");
    assert_eq!(r.len(), rtts + 5);
    assert_eq!(
        r.optional,
        rtts + 2,
        "the unsent mv, long frame and round trips"
    );
    r.push(&sized_kind("rtt", 1_000));
    assert_eq!(
        r.backlog(),
        within,
        "the oldest event of the lowest rank went: the long frame"
    );
    r.push(&sized_kind("rtt", 2_000));
    assert_eq!(
        r.backlog(),
        within,
        "then the two oldest round trips; the move stays"
    );
    assert_eq!(r.len(), rtts + 4);
    assert_eq!(r.bytes, within + 1_000);
    assert_eq!(r.optional, rtts + 1);
    // The next batch says what went, per kind and when, first.
    let batch = r.upload(500.0, true, 0, 2).expect("a batch");
    assert_eq!(
        events_of(&batch),
        vec![
            json!({"ev": "overflow", "t": 500.0, "n": 1, "kinds": {"frame": 1}, "from": 0.0, "to": 0.0}),
            json!({"ev": "overflow", "t": 500.0, "n": 2, "kinds": {"rtt": 2}, "from": 0.0, "to": 0.0}),
        ],
        "the notes fill the batch"
    );
    let next = events_of(&r.upload(1_000.0, true, 0, 3).expect("a batch"));
    assert_eq!(next[0]["ev"], json!("touch"), "the essential ones stayed");
    let next = events_of(&r.upload(1_200.0, true, 0, 4).expect("a batch"));
    assert_eq!(
        next[0]["ev"],
        json!("mv"),
        "the move stayed: moves go after round trips and long frames"
    );
    assert_eq!(r.optional, rtts, "a batch takes its droppable events along");
    let next = events_of(&r.upload(1_400.0, true, 0, 5).expect("a batch"));
    assert_eq!(
        next[0]["ev"],
        json!("mv"),
        "the first move pushed as essential"
    );
    assert_eq!(r.optional, rtts, "an essential one changes no count");
    // The proved batch on its way leaves on its pong.
    r.proved(1);
    assert_eq!(r.len(), rtts + 5);
}

#[test]
fn a_requeued_batch_counts_toward_the_backlog_again() {
    let n = BACKLOG_BYTES / 1_000 - 1;
    let mut r = Recorder::default();
    for i in 0..n {
        r.push(&sized_at(i % 10, 1_000));
    }
    let _ = r.upload(0.0, true, 0, 1).expect("a batch");
    assert_eq!(r.optional, n - 1);
    r.push(&sized_at(8, 1_000));
    assert_eq!(r.backlog(), n * 1_000);
    r.requeue();
    assert_eq!(
        r.backlog(),
        (n + 1) * 1_000,
        "the batch on its way is unsent again"
    );
    assert_eq!(r.optional, n + 1);
    r.push(&sized_at(9, 1_000));
    assert_eq!(
        r.backlog(),
        (n + 1) * 1_000,
        "past the bound the oldest went"
    );
    assert_eq!(r.len(), n + 1);
    assert_eq!(r.optional, n + 1);
    let note = r.upload(200.0, true, 0, 2).expect("again");
    assert_eq!(
        events_of(&note),
        vec![
            json!({"ev": "overflow", "t": 200.0, "n": 1, "kinds": {"x": 1}, "from": 0.0, "to": 0.0})
        ]
    );
    let next = r.upload(300.0, true, 0, 3).expect("the backlog");
    assert_eq!(ts(&next), [1], "event 0, the requeued one, went first");
}

#[test]
fn the_ring_keeps_20_000_essential_events_and_drops_the_oldest_unsent() {
    let mut r = Recorder::default();
    let touch = |i: usize| json!({"ev": "touch", "t": i});
    for i in 0..MAX_EVENTS {
        r.push(&touch(i));
    }
    assert_eq!((r.len(), r.overflow.len()), (20_000, 0));
    r.push(&touch(MAX_EVENTS));
    assert_eq!(r.len(), 20_000);
    r.push(&touch(MAX_EVENTS + 1));
    assert_eq!(r.len(), 20_000);
    let batch = r.upload(0.0, true, 0, 1).expect("a batch");
    let events = events_of(&batch);
    assert_eq!(
        events[0],
        json!({"ev": "overflow", "t": 0.0, "n": 2, "kinds": {"touch": 2}, "from": 0.0, "to": 1.0}),
        "the drops are said first, with their span"
    );
    assert_eq!(events[1]["t"], json!(2), "events 0 and 1 went");
    assert!(r.overflow.is_empty());
}

#[test]
fn the_ring_keeps_2_mb_and_never_drops_a_batch_on_its_way() {
    let mut r = Recorder::default();
    for _ in 0..2_048 {
        r.push(&sized_kind("touch", 1_024));
    }
    assert_eq!((r.len(), r.bytes), (2_048, MAX_BYTES), "exactly full");
    // A batch goes (one event of 1 024 bytes: over the bound, alone).
    let batch = r.upload(0.0, true, 0, 5).expect("a batch");
    assert_eq!(events_of(&batch).len(), 1);
    r.push(&sized_kind("touch", 1_024));
    assert_eq!(r.len(), 2_048, "one unsent went");
    assert_eq!(r.bytes, MAX_BYTES);
    // The batch on its way stays until it is proved; the next batch says
    // what went.
    r.proved(5);
    assert_eq!(r.len(), 2_047);
    assert_eq!(r.bytes, 2_047 * 1_024);
    r.soon();
    let next = r.upload(1_000.0, true, 0, 6).expect("a batch");
    assert_eq!(
        events_of(&next),
        vec![
            json!({"ev": "overflow", "t": 1_000.0, "n": 1, "kinds": {"touch": 1}, "from": 0.0, "to": 0.0})
        ]
    );
    // Only the drops are waiting: they still go. An event over the bound by
    // itself goes at once.
    let mut r = Recorder::default();
    r.push(&sized(BACKLOG_BYTES + 1));
    assert_eq!((r.len(), r.backlog()), (0, 0), "it went at once");
    let note = r.upload(2.0, true, 0, 1).expect("the drops");
    assert_eq!(
        events_of(&note),
        vec![
            json!({"ev": "overflow", "t": 2.0, "n": 1, "kinds": {"x": 1}, "from": 0.0, "to": 0.0})
        ]
    );
    assert_eq!(
        r.bytes,
        note.len() - batch_text(&[]).len(),
        "the note's bytes count"
    );
    r.proved(1);
    assert!(r.is_empty());
    assert_eq!((r.bytes, r.backlog()), (0, 0));
}

#[test]
fn each_second_of_pongs_is_one_round_trip_summary() {
    let mut r = Recorder::default();
    for (i, rtt) in [30.0, 10.0, 20.0].into_iter().enumerate() {
        r.pong(1_000.0 + 100.0 * i as f64, rtt);
    }
    assert!(r.is_empty(), "the second lasts");
    r.pong(2_000.0, 5.0);
    assert_eq!(r.len(), 1, "the pong at 2 s closed the first second");
    // A tick closes a second that is over, even without a pong.
    let batch = r.upload(3_000.0, true, 0, 1).expect("a batch");
    assert_eq!(
        events_of(&batch),
        vec![
            json!({"ev": "rtt", "t": 1_000.0, "n": 3, "min": 10.0, "med": 20.0, "max": 30.0}),
            json!({"ev": "rtt", "t": 2_000.0, "n": 1, "min": 5.0, "med": 5.0, "max": 5.0}),
        ]
    );
}

#[test]
fn a_long_frame_is_recorded_unless_it_follows_a_visibility_change() {
    let mut r = Recorder::default();
    r.frame(1_000.0, 16.7);
    r.frame(1_050.0, 50.0);
    assert!(r.is_empty(), "not long");
    r.frame(1_200.0, 139.0);
    r.visibility(2_000.0, true);
    r.frame(60_000.0, 58_000.0);
    r.frame(60_100.0, 100.0);
    let batch = r.upload(60_200.0, true, 0, 1).expect("a batch");
    assert_eq!(
        events_of(&batch),
        vec![
            json!({"ev": "frame", "t": 1_200.0, "ms": 139.0}),
            json!({"ev": "visibility", "t": 2_000.0, "hidden": true}),
            json!({"ev": "frame", "t": 60_100.0, "ms": 100.0}),
        ],
        "the gap of the time hidden is no stall"
    );
}

#[test]
fn a_ping_still_leaving_the_socket_never_holds_a_batch() {
    // The service of 2026-10-04 (#43): the iPad's WebKit reported a ping's
    // bytes in `bufferedAmount` at every link tick for up to 9 minutes, and
    // no batch went while the page pinged on.
    let mut r = Recorder::default();
    r.push(&event(1));
    let batch = r
        .upload(10.0, true, 120, 1)
        .expect("a ping in the buffer holds nothing");
    assert_eq!(ts(&batch), [1]);
    // A socket that holds more than 1 KB unsent is backed up: the batch waits.
    let mut r = Recorder::default();
    r.push(&event(1));
    assert_eq!(r.upload(10.0, true, 1_025, 1), None);
    assert_eq!(
        ts(&r.upload(10.0, true, 1_024, 1).expect("1 KB passes")),
        [1]
    );
}

#[test]
fn a_batch_holds_at_most_1_kb_so_a_set_never_waits_behind_more() {
    // One WebSocket frame of the recorder is all a set can wait behind.
    let mut r = Recorder::default();
    r.push(&sized(600));
    r.push(&sized(600));
    let first = r.upload(0.0, true, 0, 1).expect("a batch");
    assert_eq!(events_of(&first), vec![sized(600)]);
}

#[test]
fn a_stream_deck_press_and_the_tab_are_page_events() {
    assert_eq!(
        deck(1_000.5, 3, true, None, None, true, Some(7)),
        json!({"ev": "deck", "t": 1_000.5, "k": 3, "d": 1, "sent": true, "q": 7})
    );
    assert_eq!(
        deck(
            1_150.5,
            3,
            false,
            Some(149.6),
            Some("cancel"),
            true,
            Some(8)
        ),
        json!({"ev": "deck", "t": 1_150.5, "k": 3, "d": 0, "h": 150.0, "why": "cancel",
               "sent": true, "q": 8})
    );
    // A down that could not go: no seq.
    assert_eq!(
        deck(2_000.0, 6, true, None, None, false, None),
        json!({"ev": "deck", "t": 2_000.0, "k": 6, "d": 1, "sent": false})
    );
    assert_eq!(
        deck_view(5.0, true),
        json!({"ev": "deck_view", "t": 5.0, "on": true})
    );
    assert_eq!(
        deck_view(6.0, false),
        json!({"ev": "deck_view", "t": 6.0, "on": false})
    );
}
