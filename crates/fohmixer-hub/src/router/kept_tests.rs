//! Live pushes kept for a subscription batch in flight (#243): those dropped
//! past the bound are one warn-class record.

use serde_json::{Value, json};

use super::tests::{offline_router, records};
use super::{KEPT_PUSHES, LiveEvent};
use crate::live::LiveValue;

fn push(key: String) -> LiveValue {
    LiveValue {
        key,
        value: json!(1),
        display: None,
        error: None,
    }
}

#[tokio::test]
async fn pushes_dropped_past_the_bound_are_one_warn_record() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    // The rules' transport is resolved at the connect: a batch in flight.
    router.subs.connected("band");
    assert!(!router.subs.drain_outgoing().is_empty());
    records(&rx);
    // Within the bound nothing is recorded.
    let within: Vec<LiveValue> = (0..1024).map(|i| push(format!("live_{i}.value"))).collect();
    router.live_event("band", LiveEvent::Values(within));
    assert_eq!(records(&rx), Vec::<Value>::new());
    router.live_event(
        "band",
        LiveEvent::Values(vec![
            push("live_a.value".into()),
            push("live_b.value".into()),
        ]),
    );
    let got = records(&rx);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0]["ev"], json!(KEPT_PUSHES));
    assert_eq!(
        (
            &got[0]["instance"],
            &got[0]["dropped"],
            &got[0]["total"],
            &got[0]["max"]
        ),
        (&json!("band"), &json!(2), &json!(2), &json!(1024))
    );
    assert!(crate::events::is_warn(&got[0]));
}
