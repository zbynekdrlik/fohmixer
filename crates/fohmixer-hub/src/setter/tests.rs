use super::*;
use serde_json::json;

const KEY: &str = "band|live_set tracks 0 mixer_device volume|value";
const TARGET: &str = "live_set tracks 0 mixer_device volume";

/// A want of `client`'s set `seq` of the volume, arriving at `t_hub`.
fn want(client: ClientId, seq: u64, value: f64, t_hub: f64) -> Want {
    Want {
        client,
        target: TARGET.into(),
        prop: "value".into(),
        value: json!(value),
        seq,
        t_page: 1_000.0 + t_hub,
        t_hub,
        is_final: false,
    }
}

fn ok_slots(n: usize) -> Result<Vec<Value>, String> {
    Ok(vec![json!({"ok": true, "data": null}); n])
}

#[test]
fn a_stalled_instance_gets_two_batches_for_sixty_sets_and_ends_at_the_last() {
    // Spec §6.2 test 5: the first set is written at once; while Live stalls
    // on it, 59 more sets of the key pile up as ONE pending want.
    let mut setter = Setter::default();
    let mut batches = Vec::new();
    for seq in 1..=60_u64 {
        let outcome = setter.on_set(KEY, want(1, seq, seq as f64 / 100.0, seq as f64));
        assert!(!outcome.dropped_old);
        if let Some(batch) = setter.next_batch(seq as f64) {
            batches.push(batch);
        }
    }
    assert_eq!(batches.len(), 1, "one batch in flight while Live stalls");
    assert_eq!(setter.pending().len(), 1, "latest-wins: one want per key");
    assert_eq!(setter.pending()[KEY].seq, 60);
    // The stall ends: batch 1's result, then the last value as batch 2.
    let first = setter.on_result(1, &ok_slots(1), 1_000.0).unwrap();
    assert_eq!(first.acks, vec![(1, AckItem::applied(KEY, 1, None))]);
    batches.push(setter.next_batch(1_000.0).unwrap());
    assert!(setter.next_batch(1_001.0).is_none(), "batch 2 in flight");
    let second = setter.on_result(2, &ok_slots(1), 1_034.0).unwrap();
    assert_eq!(second.acks, vec![(1, AckItem::applied(KEY, 60, None))]);
    assert!(setter.next_batch(1_035.0).is_none(), "nothing left");
    assert_eq!(batches.len(), 2);
    let last = &batches[1];
    assert_eq!(last.id, 2);
    assert_eq!(last.items.len(), 1);
    assert_eq!(last.items[0].1.value, json!(0.6));
    assert_eq!(last.items[0].1.seq, 60);
}

#[test]
fn a_batch_carries_every_pending_key_once_in_key_order() {
    let mut setter = Setter::default();
    let mute = "band|live_set tracks 1 mute|value";
    let mut m = want(2, 1, 0.0, 5.0);
    m.target = "live_set tracks 1".into();
    m.prop = "mute".into();
    m.value = json!(true);
    m.is_final = true;
    setter.on_set(KEY, want(1, 1, 0.5, 4.0));
    setter.on_set(mute, m);
    let batch = setter.next_batch(10.0).unwrap();
    assert_eq!(batch.id, 1);
    assert_eq!(batch.sent_at, 10.0);
    assert_eq!(
        batch
            .items
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>(),
        vec![KEY, mute],
        "key order"
    );
    assert_eq!(
        batch.commands(),
        vec![
            json!({"target": TARGET, "name": "set_prop", "args": {"prop": "value", "value": 0.5}}),
            json!({"target": "live_set tracks 1", "name": "set_prop", "args": {"prop": "mute", "value": true}}),
        ]
    );
    assert_eq!(setter.in_flight(), Some(&batch));
    assert!(setter.pending().is_empty());
    // Pending wants wait while a batch is in flight; nothing at all is no
    // batch.
    setter.on_set(KEY, want(1, 2, 0.6, 11.0));
    assert!(setter.next_batch(12.0).is_none());
    let applied = setter
        .on_result(
            1,
            &Ok(vec![
                json!({"ok": true, "data": null}),
                json!({"ok": false, "error": "not found: tracks 1", "errorType": "LookupError"}),
            ]),
            60.5,
        )
        .unwrap();
    assert_eq!(
        applied,
        Applied {
            batch: 1,
            n: 2,
            rtt_ms: 50.5,
            acks: vec![
                (1, AckItem::applied(KEY, 1, None)),
                (2, AckItem::failed(mute, 1, "not found: tracks 1")),
            ],
            errors: 1,
        }
    );
    assert_eq!(setter.in_flight(), None);
    let next = setter.next_batch(61.0).unwrap();
    assert_eq!(next.id, 2);
    assert_eq!(next.items.len(), 1);
    setter.on_result(2, &ok_slots(1), 70.0).unwrap();
    assert!(setter.next_batch(71.0).is_none(), "nothing pending");
    setter.on_set(KEY, want(1, 3, 0.7, 72.0));
    assert_eq!(setter.next_batch(73.0).unwrap().id, 3, "one counter");
}

#[test]
fn an_old_sequence_number_is_dropped() {
    let mut setter = Setter::default();
    let first = setter.on_set(KEY, want(1, 5, 0.5, 100.0));
    assert_eq!(
        first,
        SetOutcome {
            dropped_old: false,
            gap_ms: None,
            superseded: None
        }
    );
    for old in [5, 4, 0] {
        assert_eq!(
            setter.on_set(KEY, want(1, old, 0.1, 120.0)),
            SetOutcome {
                dropped_old: true,
                gap_ms: None,
                superseded: None
            },
            "seq {old}"
        );
    }
    assert_eq!(
        setter.pending()[KEY].value,
        json!(0.5),
        "the old set changed nothing"
    );
    // The next newer set counts its gap from the last TAKEN one.
    let newer = setter.on_set(KEY, want(1, 6, 0.6, 133.5));
    assert!(!newer.dropped_old);
    assert_eq!(newer.gap_ms, Some(33.5));
    assert_eq!(setter.pending()[KEY].seq, 6);
    // Sequence numbers are per client and per key.
    assert!(!setter.on_set(KEY, want(2, 1, 0.2, 140.0)).dropped_old);
    let mut other = want(1, 1, 0.3, 141.0);
    other.prop = "panning".into();
    assert!(
        !setter
            .on_set("band|live_set tracks 0 mixer_device panning|value", other)
            .dropped_old
    );
    assert_eq!(setter.tracked(), 3);
}

#[test]
fn the_newer_arrival_of_two_clients_wins_and_the_other_is_superseded() {
    let mut setter = Setter::default();
    setter.on_set(KEY, want(1, 7, 0.4, 200.0));
    // Client 2's set arrives later: it replaces client 1's want, which was
    // never written.
    let outcome = setter.on_set(KEY, want(2, 3, 0.9, 210.0));
    assert_eq!(outcome.superseded, Some((1, AckItem::superseded(KEY, 7))));
    assert_eq!(setter.pending()[KEY].client, 2);
    assert_eq!(setter.pending()[KEY].value, json!(0.9));
    // At the same hub time the arriving set wins too.
    let tie = setter.on_set(KEY, want(1, 8, 0.1, 210.0));
    assert_eq!(tie.superseded, Some((2, AckItem::superseded(KEY, 3))));
    assert_eq!(setter.pending()[KEY].client, 1);
    // A set stamped before the slot's want loses at once: its own client
    // hears it was superseded, the slot keeps the newer want.
    let late = setter.on_set(KEY, want(2, 4, 0.2, 205.0));
    assert!(!late.dropped_old);
    assert_eq!(late.gap_ms, Some(-5.0));
    assert_eq!(late.superseded, Some((2, AckItem::superseded(KEY, 4))));
    assert_eq!(setter.pending()[KEY].client, 1);
    assert_eq!(setter.pending()[KEY].seq, 8);
    // A client replacing its own want is not superseded.
    let own = setter.on_set(KEY, want(1, 9, 0.15, 220.0));
    assert_eq!(own.superseded, None);
    assert_eq!(setter.pending()[KEY].seq, 9);
    // A want already written is not superseded: the next one waits.
    let batch = setter.next_batch(230.0).unwrap();
    assert_eq!(batch.items[0].1.client, 1);
    assert_eq!(setter.on_set(KEY, want(2, 5, 0.3, 231.0)).superseded, None);
}

#[test]
fn a_failed_batch_acks_its_error_and_newer_wants_stay() {
    let mut setter = Setter::default();
    setter.on_set(KEY, want(1, 1, 0.5, 0.0));
    let batch = setter.next_batch(0.0).unwrap();
    setter.on_set(KEY, want(1, 2, 0.6, 1.0));
    let applied = setter
        .on_result(batch.id, &Err("no result within 3 s".into()), 3_000.0)
        .unwrap();
    assert_eq!(
        applied.acks,
        vec![(1, AckItem::failed(KEY, 1, "no result within 3 s"))]
    );
    assert_eq!((applied.n, applied.errors, applied.rtt_ms), (1, 1, 3_000.0));
    assert_eq!(
        batch_problem(&applied).as_deref(),
        Some("1 of 1 writes of batch 1 failed: no result within 3 s")
    );
    // The newer want is written next.
    let next = setter.next_batch(3_000.0).unwrap();
    assert_eq!(next.items[0].1.seq, 2);
    // Offline answers the same way.
    let offline = setter
        .on_result(next.id, &Err("instance offline".into()), 3_001.0)
        .unwrap();
    assert_eq!(
        offline.acks,
        vec![(1, AckItem::failed(KEY, 2, "instance offline"))]
    );
}

#[test]
fn a_disconnect_forgets_the_batch_in_flight_and_the_pending_wants() {
    let mut setter = Setter::default();
    setter.on_set(KEY, want(1, 1, 0.5, 0.0));
    let batch = setter.next_batch(0.0).unwrap();
    setter.on_set(KEY, want(1, 2, 0.6, 1.0));
    setter.on_disconnect();
    assert_eq!(setter.in_flight(), None);
    assert!(setter.pending().is_empty());
    // The old batch's late answer acks nothing.
    assert_eq!(setter.on_result(batch.id, &ok_slots(1), 5.0), None);
    assert!(setter.next_batch(6.0).is_none());
    // The client's sequence numbers stay: an old set is still old.
    assert!(setter.on_set(KEY, want(1, 2, 0.6, 7.0)).dropped_old);
    assert!(!setter.on_set(KEY, want(1, 3, 0.7, 8.0)).dropped_old);
    assert_eq!(setter.next_batch(9.0).unwrap().id, 2);
}

#[test]
fn a_result_for_another_batch_leaves_the_one_in_flight() {
    let mut setter = Setter::default();
    assert_eq!(
        setter.on_result(1, &ok_slots(1), 1.0),
        None,
        "nothing in flight"
    );
    setter.on_set(KEY, want(1, 1, 0.5, 0.0));
    let batch = setter.next_batch(0.0).unwrap();
    assert_eq!(setter.on_result(batch.id + 1, &ok_slots(1), 1.0), None);
    assert_eq!(setter.in_flight(), Some(&batch));
    assert!(setter.on_result(batch.id, &ok_slots(1), 2.0).is_some());
}

#[test]
fn a_left_client_is_forgotten_but_its_wants_stay() {
    let mut setter = Setter::default();
    setter.on_set(KEY, want(1, 9, 0.5, 0.0));
    setter.on_set("band|live_set|tempo", want(1, 10, 120.0, 1.0));
    setter.on_set("band|live_set|metronome", want(2, 1, 1.0, 2.0));
    assert_eq!(setter.tracked(), 3);
    setter.drop_client(1);
    assert_eq!(setter.tracked(), 1, "client 2's number stays");
    assert_eq!(setter.pending().len(), 3, "the wants stay");
    // A new socket of that number starts afresh.
    assert!(!setter.on_set(KEY, want(1, 1, 0.4, 3.0)).dropped_old);
    assert!(
        setter
            .on_set("band|live_set|metronome", want(2, 1, 0.0, 4.0))
            .dropped_old
    );
}

#[test]
fn an_ack_says_what_live_answered() {
    assert_eq!(
        ack_for("k", 1, Some(&json!({"ok": true, "data": 0.75}))),
        AckItem::applied("k", 1, Some(json!(0.75)))
    );
    assert_eq!(
        ack_for("k", 2, Some(&json!({"ok": true, "data": null}))),
        AckItem::applied("k", 2, None)
    );
    assert_eq!(
        ack_for("k", 3, Some(&json!({"ok": true}))),
        AckItem::applied("k", 3, None)
    );
    assert_eq!(
        ack_for("k", 4, Some(&json!({"ok": false, "error": "live error"}))),
        AckItem::failed("k", 4, "live error")
    );
    assert_eq!(
        ack_for("k", 5, Some(&json!({"ok": false}))),
        AckItem::failed("k", 5, "failed")
    );
    assert_eq!(
        ack_for("k", 6, Some(&json!({"ok": "true", "data": 1}))),
        AckItem::failed("k", 6, "failed"),
        "only a real true is ok"
    );
    assert_eq!(ack_for("k", 7, None), AckItem::failed("k", 7, "no result"));
}

#[test]
fn a_short_result_fails_the_wants_it_has_no_slot_for() {
    let mut setter = Setter::default();
    setter.on_set("a", want(1, 1, 0.1, 0.0));
    setter.on_set("b", want(2, 1, 0.2, 0.0));
    let batch = setter.next_batch(0.0).unwrap();
    let applied = setter.on_result(batch.id, &ok_slots(1), 1.0).unwrap();
    assert_eq!(
        applied.acks,
        vec![
            (1, AckItem::applied("a", 1, None)),
            (2, AckItem::failed("b", 1, "no result")),
        ]
    );
    assert_eq!(applied.errors, 1);
    assert_eq!(
        batch_problem(&applied).as_deref(),
        Some("1 of 2 writes of batch 1 failed: no result")
    );
}

#[test]
fn a_fully_applied_batch_is_no_problem() {
    let mut setter = Setter::default();
    setter.on_set(KEY, want(1, 1, 0.5, 0.0));
    setter.next_batch(0.0).unwrap();
    let applied = setter.on_result(1, &ok_slots(1), 1.0).unwrap();
    assert_eq!(applied.errors, 0);
    assert_eq!(batch_problem(&applied), None);
}
