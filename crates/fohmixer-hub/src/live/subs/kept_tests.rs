use serde_json::{Value, json};

use super::*;

fn push(key: &str, value: Value) -> LiveValue {
    LiveValue {
        key: key.to_string(),
        value,
        display: None,
        error: None,
    }
}

fn values(frames: &[Vec<LiveValue>]) -> Vec<Vec<(String, Value)>> {
    frames
        .iter()
        .map(|f| f.iter().map(|i| (i.key.clone(), i.value.clone())).collect())
        .collect()
}

fn pairs(items: &[(&str, Value)]) -> Vec<(String, Value)> {
    items
        .iter()
        .map(|(k, v)| (k.to_string(), v.clone()))
        .collect()
}

#[test]
fn the_pushes_for_the_keys_made_known_come_back_by_frame_in_arrival_order() {
    let mut kept = Kept::default();
    let mark = kept.mark();
    assert_eq!(mark, 0);
    assert_eq!(
        kept.keep(vec![
            push("live_1.value", json!(1)),
            push("live_2.value", json!(2))
        ]),
        None
    );
    assert_eq!(kept.keep(vec![push("live_1.value", json!(3))]), None);
    assert_eq!(kept.keep(vec![push("live_3.value", json!(4))]), None);
    assert_eq!(kept.mark(), 3);
    let taken = kept.take(mark, |key| key != "live_3.value");
    assert_eq!(
        values(&taken),
        vec![
            pairs(&[("live_1.value", json!(1)), ("live_2.value", json!(2))]),
            pairs(&[("live_1.value", json!(3))]),
        ]
    );
    // The rest stays for a later result.
    assert_eq!(kept.len(), 1);
    let taken = kept.take(mark, |key| key == "live_3.value");
    assert_eq!(values(&taken), vec![pairs(&[("live_3.value", json!(4))])]);
    assert_eq!(kept.len(), 0);
}

#[test]
fn a_frame_split_by_the_keys_made_known_keeps_its_other_items() {
    let mut kept = Kept::default();
    kept.keep(vec![
        push("live_1.value", json!(1)),
        push("live_2.value", json!(2)),
    ]);
    let taken = kept.take(0, |key| key == "live_2.value");
    assert_eq!(values(&taken), vec![pairs(&[("live_2.value", json!(2))])]);
    let taken = kept.take(0, |key| key == "live_1.value");
    assert_eq!(values(&taken), vec![pairs(&[("live_1.value", json!(1))])]);
}

#[test]
fn a_push_from_before_the_batch_was_sent_is_dropped_never_applied() {
    let mut kept = Kept::default();
    kept.keep(vec![push("live_1.value", json!("older listener"))]);
    // The batch is sent now: its mark is the frame kept so far.
    let mark = kept.mark();
    assert_eq!(mark, 1);
    kept.keep(vec![push("live_1.value", json!("its listener"))]);
    kept.keep(vec![push("live_2.value", json!("not yet"))]);
    let taken = kept.take(mark, |key| key == "live_1.value");
    assert_eq!(
        values(&taken),
        vec![pairs(&[("live_1.value", json!("its listener"))])]
    );
    // The older one is gone too, not kept for later.
    assert_eq!(kept.len(), 1);
    assert!(kept.take(0, |key| key == "live_1.value").is_empty());
}

#[test]
fn at_most_kept_max_pushes_stay_and_the_oldest_go_first() {
    let mut kept = Kept::default();
    let full: Vec<LiveValue> = (0..KEPT_MAX)
        .map(|i| push("live_1.value", json!(i)))
        .collect();
    assert_eq!(kept.keep(full), None, "exactly the bound: nothing dropped");
    assert_eq!(kept.len(), KEPT_MAX);
    assert_eq!(
        kept.keep(vec![
            push("live_2.value", json!("a")),
            push("live_2.value", json!("b"))
        ]),
        Some(Overflow {
            dropped: 2,
            total: 2
        })
    );
    assert_eq!(kept.len(), KEPT_MAX);
    assert_eq!(
        kept.keep(vec![push("live_2.value", json!("c"))]),
        Some(Overflow {
            dropped: 1,
            total: 3
        })
    );
    let taken = kept.take(0, |key| key == "live_1.value");
    let first = taken
        .first()
        .and_then(|f| f.first())
        .map(|i| i.value.clone());
    assert_eq!(first, Some(json!(3)), "values 0, 1 and 2 went first");
    assert_eq!(taken.iter().map(Vec::len).sum::<usize>(), KEPT_MAX - 3);
    let newest = kept.take(0, |key| key == "live_2.value");
    assert_eq!(
        values(&newest),
        vec![
            pairs(&[("live_2.value", json!("a")), ("live_2.value", json!("b"))]),
            pairs(&[("live_2.value", json!("c"))]),
        ]
    );
}

#[test]
fn a_clear_forgets_every_push_and_keeps_the_marks_going() {
    let mut kept = Kept::default();
    kept.keep(vec![push("live_1.value", json!(1))]);
    kept.keep(vec![push("live_1.value", json!(2))]);
    kept.clear();
    assert_eq!(kept.len(), 0);
    assert!(kept.take(0, |_| true).is_empty());
    assert_eq!(
        kept.mark(),
        2,
        "a later batch's mark stays above the old frames"
    );
    kept.keep(vec![push("live_1.value", json!(3))]);
    assert_eq!(kept.mark(), 3);
}

#[test]
fn an_overflows_record_names_its_instance_counts_and_bound() {
    let overflow = Overflow {
        dropped: 2,
        total: 7,
    };
    assert_eq!(
        overflow.fields("band"),
        json!({"instance": "band", "dropped": 2, "total": 7, "max": 1024})
    );
}

fn shown(key: &str, value: Value, display: &str) -> LiveValue {
    LiveValue {
        key: key.to_string(),
        value,
        display: Some(display.to_string()),
        error: None,
    }
}

fn gone(key: &str) -> LiveValue {
    LiveValue {
        key: key.to_string(),
        value: Value::Null,
        display: None,
        error: Some("gone".to_string()),
    }
}

/// The value `newer_push` gives, if any.
fn chosen(heard: Option<&(u64, LiveValue)>, mark: u64, wants_display: bool) -> Option<Value> {
    newer_push(heard, mark, wants_display).map(|i| i.value.clone())
}

#[test]
fn a_joining_key_takes_a_push_only_from_after_its_batch_was_sent() {
    let heard = (3, push("live_1.value", json!(0.6)));
    assert_eq!(chosen(None, 0, false), None, "nothing heard: the result's");
    assert_eq!(chosen(Some(&heard), 2, false), Some(json!(0.6)));
    // A push of the frame the batch was sent after is older than its read.
    assert_eq!(chosen(Some(&heard), 3, false), None);
    assert_eq!(chosen(Some(&heard), 4, false), None);
}

#[test]
fn a_push_without_the_display_a_joining_key_wants_is_older_than_its_read() {
    let bare = (3, push("live_1.value", json!(0.6)));
    let with = (3, shown("live_1.value", json!(0.6), "-3.00 dB"));
    assert_eq!(chosen(Some(&bare), 2, true), None);
    assert_eq!(chosen(Some(&bare), 2, false), Some(json!(0.6)));
    assert_eq!(
        newer_push(Some(&with), 2, true),
        Some(&shown("live_1.value", json!(0.6), "-3.00 dB"))
    );
    assert_eq!(chosen(Some(&with), 2, false), Some(json!(0.6)));
}

#[test]
fn a_frame_keeps_the_unknown_keys_and_hears_the_last_push_of_each_known_one() {
    let mut kept = Kept::default();
    let known = |key: &str| key == "live_1.value";
    let mark = kept.mark();
    let first = [
        push("live_1.value", json!(1)),
        push("live_2.value", json!(2)),
        push("live_1.value", json!(3)),
    ];
    assert_eq!(kept.frame(&first, known), None);
    assert_eq!(kept.mark(), 1, "one frame numbered");
    assert_eq!(kept.len(), 1, "the unknown key's push is kept");
    assert_eq!(
        kept.heard_after("live_1.value", mark, false),
        Some(&push("live_1.value", json!(3))),
        "the frame's last item of the known key"
    );
    assert_eq!(kept.heard_after("live_2.value", mark, false), None);
    // A later frame with an error is not heard: the earlier push stays,
    // with the number of the frame it came in.
    kept.frame(&[gone("live_1.value")], known);
    assert_eq!(kept.mark(), 2);
    assert_eq!(
        kept.heard_after("live_1.value", mark, false),
        Some(&push("live_1.value", json!(3)))
    );
    assert_eq!(kept.heard_after("live_1.value", 1, false), None);
}

#[test]
fn a_heard_push_goes_when_forgotten_or_cleared() {
    let mut kept = Kept::default();
    let both = [
        push("live_1.value", json!(1)),
        push("live_2.value", json!(2)),
    ];
    kept.frame(&both, |key| key.starts_with("live_"));
    kept.forget("live_1.value");
    assert_eq!(kept.heard_after("live_1.value", 0, false), None);
    assert_eq!(
        kept.heard_after("live_2.value", 0, false),
        Some(&push("live_2.value", json!(2)))
    );
    kept.clear();
    assert_eq!(kept.heard_after("live_2.value", 0, false), None);
}

#[test]
fn a_frames_overflow_is_that_of_its_kept_pushes() {
    let mut kept = Kept::default();
    let full: Vec<LiveValue> = (0..=KEPT_MAX)
        .map(|i| push("live_2.value", json!(i)))
        .collect();
    assert_eq!(
        kept.frame(&full, |key| key == "live_1.value"),
        Some(Overflow {
            dropped: 1,
            total: 1
        })
    );
}
