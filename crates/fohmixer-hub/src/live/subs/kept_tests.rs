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
