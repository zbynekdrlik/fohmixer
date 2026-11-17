//! The walks paced (#235): band Live lost 10 audio blocks at a hub restart
//! that walked every AUTOTUNE target in one tick. One target's walk at a
//! time per Live set; the next starts 100 ms after the previous walk's
//! last answer; a disconnect drops the walks still to come.

use super::*;

/// Every call a task made to the scripted Live by now, in order.
async fn calls_now(script: &mut Script) -> Vec<Asked> {
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
    std::iter::from_fn(|| script.try_call()).collect()
}

/// A walk's first read: band track `index`'s devices.
fn devices_of(index: u32) -> Vec<Value> {
    vec![
        json!({"target": format!("live_set tracks {index}"), "name": "get_prop",
               "args": {"prop": "devices"}}),
    ]
}

fn commands(asked: &[Asked]) -> Vec<Vec<Value>> {
    asked.iter().map(|a| a.commands.clone()).collect()
}

/// The one walk's read answered "no rack" (a definite answer in one read),
/// and that answer handled by the router.
async fn no_rack(router: &mut Router, own: &mut UnboundedReceiver<RouterMsg>, asked: Asked) {
    asked.answer(vec![ok(json!([]))]);
    let msg = next_of(own, "the walk's answer", |m| {
        matches!(m, RouterMsg::AutotuneFound { .. })
    })
    .await;
    router.handle(msg);
}

#[tokio::test(start_paused = true)]
async fn a_connect_walks_one_target_at_a_time_the_next_100_ms_after_the_last_answer() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, _rx, mut script) = scripted_router(dir.path());
    router.handle(layout(1, vec![repro(), stream()]));
    assert!(calls_now(&mut script).await.is_empty(), "offline: no walk");
    online(&mut router);
    router.autotune_connected("band");
    let first = calls_now(&mut script).await;
    assert_eq!(
        commands(&first),
        vec![devices_of(6)],
        "one target's walk at a time, in Live's order"
    );
    let first = first.into_iter().next().unwrap();
    no_rack(&mut router, &mut own, first).await;
    // Nothing for 99 ms after the answer, then the next walk.
    pump(&mut router, &mut own, 99).await;
    assert!(calls_now(&mut script).await.is_empty(), "the gap first");
    pump(&mut router, &mut own, 2).await;
    let second = calls_now(&mut script).await;
    assert_eq!(commands(&second), vec![devices_of(7)]);
    // Both still answer as before: the walk's reads are unchanged.
    let second = second.into_iter().next().unwrap();
    no_rack(&mut router, &mut own, second).await;
    for key in [REPRO, STREAM] {
        assert_eq!(
            router.autotune.targets[key].problem.as_deref(),
            Some("no rack named Latencies"),
            "{key}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_disconnect_drops_the_walks_still_to_come() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, _rx, mut script) = scripted_router(dir.path());
    online(&mut router);
    router.handle(layout(1, vec![repro(), stream()]));
    let first = calls_now(&mut script).await;
    assert_eq!(commands(&first), vec![devices_of(6)], "one at a time");
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    // The walk in flight answers after the disconnect: void, and no walk
    // still queued goes.
    let first = first.into_iter().next().unwrap();
    no_rack(&mut router, &mut own, first).await;
    pump(&mut router, &mut own, 1000).await;
    assert!(calls_now(&mut script).await.is_empty(), "the queue dropped");
    assert!(router.autotune.targets[STREAM].problem.is_none());
    // The connect walks them again from the first.
    online(&mut router);
    router.autotune_connected("band");
    assert_eq!(commands(&calls_now(&mut script).await), vec![devices_of(6)]);
}

/// The band's disconnect, then its connect: online, its targets walked.
fn reconnect(router: &mut Router) {
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    online(router);
    router.autotune_connected("band");
}

#[tokio::test(start_paused = true)]
async fn a_reconnect_walks_at_once_whatever_the_old_connections_walk_does() {
    // Review of #235: the connect never waits for a walk of the connection
    // before (up to its 3 s answer timeout).
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, _rx, mut script) = scripted_router(dir.path());
    online(&mut router);
    router.handle(layout(1, vec![repro(), stream()]));
    let old = calls_now(&mut script).await;
    assert_eq!(commands(&old), vec![devices_of(6)]);
    reconnect(&mut router);
    let new = calls_now(&mut script).await;
    assert_eq!(commands(&new), vec![devices_of(6)], "at once");
    // The old walk's answer is void and frees nothing: no second walk.
    let old = old.into_iter().next().unwrap();
    no_rack(&mut router, &mut own, old).await;
    pump(&mut router, &mut own, 1000).await;
    assert!(calls_now(&mut script).await.is_empty(), "one at a time");
    assert!(router.autotune.targets[REPRO].problem.is_none(), "void");
    let new = new.into_iter().next().unwrap();
    no_rack(&mut router, &mut own, new).await;
    pump(&mut router, &mut own, 101).await;
    assert_eq!(commands(&calls_now(&mut script).await), vec![devices_of(7)]);
}

#[tokio::test(start_paused = true)]
async fn a_gap_of_the_connection_before_starts_no_walk() {
    // Review of #235: each gap has its own generation, so the gap of a
    // walk before a reconnect never ends a newer gap early.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, _rx, mut script) = scripted_router(dir.path());
    online(&mut router);
    router.handle(layout(1, vec![repro(), stream()]));
    let first = calls_now(&mut script).await.into_iter().next().unwrap();
    no_rack(&mut router, &mut own, first).await;
    // Its gap runs until t0 + 100; the reconnect at t0 + 10.
    pump(&mut router, &mut own, 10).await;
    reconnect(&mut router);
    let again = calls_now(&mut script).await;
    assert_eq!(commands(&again), vec![devices_of(6)]);
    pump(&mut router, &mut own, 10).await;
    let again = again.into_iter().next().unwrap();
    no_rack(&mut router, &mut own, again).await;
    // The old gap ends at t0 + 100, the new one at t0 + 120.
    pump(&mut router, &mut own, 90).await;
    assert!(calls_now(&mut script).await.is_empty(), "the old gap");
    pump(&mut router, &mut own, 11).await;
    assert_eq!(commands(&calls_now(&mut script).await), vec![devices_of(7)]);
}
