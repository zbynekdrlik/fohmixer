//! Tests of `trace.rs`: the flight recorder's events, its ring, when a batch
//! goes, what it holds, and when its events leave the ring.

use super::*;

/// An event whose JSON is exactly `len` bytes.
fn sized(len: usize) -> Value {
    let base = json!({"ev": "x", "t": 0, "pad": ""}).to_string().len();
    json!({"ev": "x", "t": 0, "pad": "a".repeat(len - base)})
}

/// Event number `i` (a small one).
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

#[test]
fn the_bounds_are_these() {
    assert_eq!(MAX_EVENTS, 20_000);
    assert_eq!(MAX_BYTES, 2_097_152);
    assert_eq!(UPLOAD_MS, 2000.0);
    assert_eq!(BACKLOG_MS, 200.0);
    assert_eq!(BATCH_BYTES, 8_192);
    assert_eq!(LONG_FRAME_MS, 50.0);
    assert!(!is_long_frame(50.0));
    assert!(is_long_frame(50.0_f64.next_up()));
    assert!(!is_long_frame(16.7));
    assert!(is_long_frame(139.0));
}

#[test]
fn each_event_carries_its_kind_its_page_time_and_its_facts() {
    let keys = vec!["band|live_set tracks[name=Vox 1] mixer_device volume|value".to_string()];
    assert_eq!(
        touch(1_000.5, "down", &keys, 7),
        json!({"ev": "touch", "t": 1_000.5, "what": "down", "keys": keys, "pointer": 7})
    );
    assert_eq!(
        pong(2_003.0, 41, 2.75),
        json!({"ev": "pong", "t": 2_003.0, "n": 41, "rtt": 2.75})
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
fn a_send_is_its_sets_key_sequence_value_and_whether_the_socket_took_it() {
    let set = ClientMsg::Set {
        instance: "band".into(),
        target: "live_set tracks[name=Vox 1] mixer_device volume".into(),
        prop: "value".into(),
        value: json!(0.75),
        seq: 12,
        t: 5_000.125,
        is_final: true,
    };
    assert_eq!(
        send(&set, false),
        Some(json!({
            "ev": "send",
            "t": 5_000.125,
            "key": "band|live_set tracks[name=Vox 1] mixer_device volume|value",
            "seq": 12,
            "value": 0.75,
            "final": true,
            "sent": false,
        }))
    );
    assert_eq!(send(&set, true).expect("a set")["sent"], json!(true));
    assert_eq!(send(&ClientMsg::Unsub { sub: "x".into() }, true), None);
}

#[test]
fn an_ack_says_why_it_failed_or_that_it_was_superseded_only_when_so() {
    let key = "band|live_set tracks[name=Vox 1] mixer_device volume|value";
    assert_eq!(
        ack(6_000.0, &AckItem::applied(key, 3, Some(json!(0.5)))),
        json!({"ev": "ack", "t": 6_000.0, "key": key, "seq": 3})
    );
    assert_eq!(
        ack(6_001.0, &AckItem::failed(key, 4, "no result within 3 s")),
        json!({"ev": "ack", "t": 6_001.0, "key": key, "seq": 4,
               "error": "no result within 3 s"})
    );
    let mut superseded = AckItem::applied(key, 5, None);
    superseded.superseded = true;
    assert_eq!(
        ack(6_002.0, &superseded),
        json!({"ev": "ack", "t": 6_002.0, "key": key, "seq": 5, "superseded": true})
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
    assert_eq!(r.upload(10.0, true, 1, 1), None, "the socket still sends");
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
    assert_eq!(r.upload(2_300.0, true, 9, 4), None);
    assert_eq!(ts(&r.upload(2_400.0, true, 0, 4).expect("now")), [4]);
}

#[test]
fn a_backlog_drains_a_batch_every_200_ms_then_every_2_s_again() {
    let mut r = Recorder::default();
    r.push(&sized(9_000));
    r.push(&sized(9_000));
    r.push(&sized(9_000));
    assert_eq!(
        events_of(&r.upload(0.0, true, 0, 1).expect("a batch")).len(),
        1
    );
    assert!(r.backlog, "two events wait");
    assert_eq!(r.upload(200.0_f64.next_down(), true, 0, 2), None);
    assert_eq!(
        events_of(&r.upload(200.0, true, 0, 2).expect("200 ms on")).len(),
        1
    );
    assert_eq!(r.upload(399.0, true, 7, 3), None, "the socket still sends");
    assert_eq!(
        events_of(&r.upload(400.0, true, 0, 3).expect("the last")).len(),
        1
    );
    assert!(!r.backlog, "nothing left behind");
    r.push(&event(4));
    assert_eq!(r.upload(600.0, true, 0, 4), None, "no backlog: 2 s again");
    assert_eq!(r.upload(2_400.0_f64.next_down(), true, 0, 4), None);
    assert_eq!(ts(&r.upload(2_400.0, true, 0, 4).expect("2 s on")), [4]);
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
    r.push(&event(3));
    r.requeue();
    // At once after the next hello, the backlog first.
    let again = r.upload(100.0, true, 0, 7).expect("again");
    let all: Vec<String> = (1..=3).map(|i| event(i).to_string()).collect();
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
fn a_batch_holds_8_kb_of_events_joined_by_commas_at_least_one() {
    // 4 000 + 1 + 4 191 = 8 192: one batch.
    let mut r = Recorder::default();
    r.push(&sized(4_000));
    r.push(&sized(4_191));
    let both = r.upload(0.0, true, 0, 1).expect("a batch");
    assert_eq!(events_of(&both).len(), 2);
    // One byte more: two batches.
    let mut r = Recorder::default();
    r.push(&sized(4_000));
    r.push(&sized(4_192));
    let one = r.upload(0.0, true, 0, 1).expect("a batch");
    assert_eq!(events_of(&one), vec![sized(4_000)]);
    r.soon();
    let two = r.upload(1.0, true, 0, 2).expect("a batch");
    assert_eq!(events_of(&two), vec![sized(4_192)]);
    // An event over the bound goes alone.
    let mut r = Recorder::default();
    r.push(&sized(10_000));
    r.push(&event(1));
    let big = r.upload(0.0, true, 0, 1).expect("a batch");
    assert_eq!(events_of(&big), vec![sized(10_000)]);
    // Many small events: as many as fit.
    let mut r = Recorder::default();
    let len = event(0).to_string().len();
    for _ in 0..2_000 {
        r.push(&event(0));
    }
    let fit = r.upload(0.0, true, 0, 1).expect("a batch");
    assert_eq!(events_of(&fit).len(), (BATCH_BYTES + 1) / (len + 1));
}

#[test]
fn the_ring_keeps_20_000_events_and_drops_the_oldest_unsent() {
    let mut r = Recorder::default();
    for i in 0..MAX_EVENTS {
        r.push(&event(i));
    }
    assert_eq!((r.len(), r.overflow), (20_000, 0));
    r.push(&event(MAX_EVENTS));
    assert_eq!((r.len(), r.overflow), (20_000, 1));
    r.push(&event(MAX_EVENTS + 1));
    assert_eq!((r.len(), r.overflow), (20_000, 2));
    let batch = r.upload(0.0, true, 0, 1).expect("a batch");
    let events = events_of(&batch);
    assert_eq!(
        events[0],
        json!({"ev": "overflow", "t": 0.0, "n": 2}),
        "the drops are said first"
    );
    assert_eq!(events[1]["t"], json!(2), "events 0 and 1 went");
    assert_eq!(r.overflow, 0);
}

#[test]
fn the_ring_keeps_2_mb_and_never_drops_a_batch_on_its_way() {
    let mut r = Recorder::default();
    for _ in 0..2_048 {
        r.push(&sized(1_024));
    }
    assert_eq!(
        (r.len(), r.bytes, r.overflow),
        (2_048, MAX_BYTES, 0),
        "exactly full"
    );
    // A batch goes (7 events of 1 024 bytes and their commas fit 8 KB).
    let batch = r.upload(0.0, true, 0, 5).expect("a batch");
    let on_its_way = events_of(&batch).len();
    assert_eq!(on_its_way, 7);
    r.push(&sized(1_024));
    assert_eq!((r.len(), r.overflow), (2_048, 1), "one unsent went");
    assert_eq!(r.bytes, MAX_BYTES);
    // The batch on its way stays until it is proved; the next batch says
    // what went.
    r.proved(5);
    assert_eq!(r.len(), 2_048 - 7);
    assert_eq!(r.bytes, (2_048 - 7) * 1_024);
    r.soon();
    let next = r.upload(1.0, true, 0, 6).expect("a batch");
    assert_eq!(
        events_of(&next)[0],
        json!({"ev": "overflow", "t": 1.0, "n": 1})
    );
    assert_eq!(events_of(&next).len(), 8, "the note and 7 events");
    r.proved(6);
    assert_eq!(r.len(), 2_048 - 7 - 7);
    assert_eq!(
        r.bytes,
        (2_048 - 7 - 7) * 1_024,
        "the note left with its batch"
    );
    // Only the drops are waiting: they still go.
    let mut r = Recorder {
        events: VecDeque::new(),
        bytes: 0,
        sent: 0,
        flights: VecDeque::new(),
        overflow: 3,
        uploaded: None,
        soon: false,
        backlog: false,
        after_visibility: false,
    };
    let note = r.upload(2.0, true, 0, 1).expect("the drops");
    assert_eq!(
        events_of(&note),
        vec![json!({"ev": "overflow", "t": 2.0, "n": 3})]
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
