//! A play start's selector-on writes wait the hub's `[autotune]
//! engage_delay_ms` (#190, the DECIDED section of the ticket): a stop, a
//! lost transport or a disconnect within the wait writes nothing that
//! waited, DRY stays immediate, and 0 writes at once as before.

use std::time::Duration;

use super::*;

/// The wait the PC's test sets.
const DELAY: Duration = Duration::from_millis(1000);

/// [`ready`] with a wait of [`DELAY`] and AUTOTUNE AUT on, Live stopped in
/// a keyed song with the keys confirmed: a play start tunes both tracks.
async fn waiting(dir: &std::path::Path) -> (Router, Live, std::sync::mpsc::Receiver<Value>) {
    let (router, mut live, rx) = ready(dir, HubState::default()).await;
    let mut router = router.with_engage_delay(1000);
    aut_on(&mut router);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new(), "stopped");
    let _ = steps(&rx);
    (router, live, rx)
}

/// An `engage` record of the band's wait.
fn engage(act: &str) -> Value {
    json!({"step": "engage", "instance": "band", "ms": 1000, "act": act})
}

#[tokio::test(start_paused = true)]
async fn a_play_starts_switch_on_waits_the_delay_then_goes_with_its_cause() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = waiting(dir.path()).await;
    assert_eq!(status(&mut router).autotune.engage_delay_ms, 1000);
    transport(&mut router, true);
    live.settle(&mut router).await;
    // Decided on at once (the pages and the status see it), written later.
    assert_eq!(decisions(&mut router), [on(), on()]);
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new(), "waits");
    let got = steps(&rx);
    assert_eq!(of(&got, "engage"), vec![engage("waits")]);
    assert_eq!(
        of(&got, "select"),
        vec![
            select(6, true, "play", "transport"),
            select(7, true, "play", "transport")
        ]
    );
    assert_eq!(of(&got, "write"), Vec::<Value>::new());
    pass(&mut router, &mut live, DELAY - Duration::from_millis(1)).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new(), "not yet");
    pass(&mut router, &mut live, Duration::from_millis(1)).await;
    assert_eq!(
        live.written(),
        vec![batch(&[("p6", 0.0), ("p7", 127.0)])],
        "one batch at the wait's end"
    );
    let got = steps(&rx);
    assert_eq!(of(&got, "engage"), vec![engage("ends")]);
    assert_eq!(
        of(&got, "write"),
        vec![
            json!({"step": "write", "instance": "band", "cause": "transport",
                    "on": ["Vox 2", "#8"], "off": []})
        ]
    );
    assert_eq!(of(&got, "select"), Vec::<Value>::new(), "no new switch");
    assert!(router.autotune.engage.waiting.is_empty());
    // The next stop's DRY is not delayed (spread as before), and a start
    // waits again.
    transport(&mut router, false);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0)])], "DRY at once");
    pass(&mut router, &mut live, SPREAD).await;
    assert_eq!(live.written(), vec![batch(&[("p7", 0.0)])]);
    let _ = steps(&rx);
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    pass(&mut router, &mut live, DELAY).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0), ("p7", 127.0)])]);
}

#[tokio::test(start_paused = true)]
async fn a_stop_within_the_wait_writes_nothing_and_a_new_start_waits_afresh() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = waiting(dir.path()).await;
    transport(&mut router, true);
    live.settle(&mut router).await;
    pass(&mut router, &mut live, Duration::from_millis(300)).await;
    transport(&mut router, false);
    live.settle(&mut router).await;
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Stop), off(SwitchWhy::Stop)]
    );
    pass(&mut router, &mut live, Duration::from_millis(300)).await;
    // Live plays again 600 ms after the first start: the first wait's end
    // (at 1000) is stale, the new one ends at 1600.
    transport(&mut router, true);
    live.settle(&mut router).await;
    pass(&mut router, &mut live, Duration::from_millis(400)).await;
    assert_eq!(
        live.written(),
        Vec::<Vec<(String, f64)>>::new(),
        "the first wait's end writes nothing"
    );
    pass(&mut router, &mut live, Duration::from_millis(599)).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    pass(&mut router, &mut live, Duration::from_millis(1)).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0), ("p7", 127.0)])]);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "engage"),
        vec![
            engage("waits"),
            engage("stopped"),
            engage("waits"),
            engage("ends")
        ]
    );
    assert_eq!(of(&got, "write").len(), 1, "only the second start's");
    // A stop and a stale end with no wait running record nothing more.
    router.autotune_engage_end("band", 1);
    transport(&mut router, false);
    live.settle(&mut router).await;
    let got = steps(&rx);
    assert_eq!(of(&got, "engage"), Vec::<Value>::new());
}

#[tokio::test(start_paused = true)]
async fn a_lost_transport_or_a_disconnect_within_the_wait_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = waiting(dir.path()).await;
    transport(&mut router, true);
    live.settle(&mut router).await;
    router.transport_failed(TRANSPORT, "not observable");
    live.settle(&mut router).await;
    pass(&mut router, &mut live, DELAY * 2).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(
        of(&steps(&rx), "engage"),
        vec![engage("waits"), engage("stopped")]
    );
    let other = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = waiting(other.path()).await;
    transport(&mut router, true);
    live.settle(&mut router).await;
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    pass(&mut router, &mut live, DELAY * 2).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(
        of(&steps(&rx), "engage"),
        vec![engage("waits"), engage("offline")]
    );
    assert!(router.autotune.engage.waiting.is_empty());
}

#[tokio::test(start_paused = true)]
async fn dry_within_the_wait_stays_immediate() {
    // A selector another writer took to its chain within the wait: the hub
    // wants it on, so it is left; AUT off then writes it DRY at once, not
    // at the wait's end.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = waiting(dir.path()).await;
    transport(&mut router, true);
    live.settle(&mut router).await;
    by_hand(&mut router, &mut live, "p6", 0.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    set_hub(&mut router, HUB_AUTOTUNE_AUT, json!(false));
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0)])], "at once");
    pass(&mut router, &mut live, DELAY).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new(), "AUT off");
}

#[tokio::test(start_paused = true)]
async fn no_delay_writes_a_play_starts_switch_on_at_once_with_no_wait() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = ready(dir.path(), HubState::default()).await;
    assert_eq!(status(&mut router).autotune.engage_delay_ms, 0);
    aut_on(&mut router);
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0), ("p7", 127.0)])]);
    assert_eq!(of(&steps(&rx), "engage"), Vec::<Value>::new());
    assert!(router.autotune.engage.waiting.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_sessions_first_value_of_playing_opens_no_wait() {
    // A reconnect while Live plays is no play start (the connect's hold
    // keeps the writes back): no `engage` record, no wait.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = waiting(dir.path()).await;
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    live.settle(&mut router).await;
    let _ = steps(&rx);
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(of(&steps(&rx), "engage"), Vec::<Value>::new());
    assert!(router.autotune.engage.waiting.is_empty());
}
