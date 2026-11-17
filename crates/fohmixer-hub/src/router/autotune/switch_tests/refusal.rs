//! The refusals: a page's tap, a key write and a selector write Live
//! refuses.

use super::*;

#[tokio::test]
async fn a_tap_is_refused_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    let page = attach(&mut router, 1);
    page.take();
    for value in [json!(true), json!(false), json!("on")] {
        set_hub(&mut router, HUB_AUTOTUNE, value.clone());
        assert_eq!(
            steps(&rx),
            vec![json!({"step": "refused", "what": "tap",
                        "why": "the AUTOTUNE tap is gone; a tap opens the singers' panel",
                        "client": 1, "peer": "10.0.0.1"})],
            "{value}"
        );
        assert_eq!(
            page.take().unwrap_or_default(),
            vec![ServerMsg::Autotune(router.autotune.state())],
            "told again, no error: {value}"
        );
    }
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    let now = status(&mut router).autotune;
    assert_eq!((now.taps, now.aut), (0, true));
}

#[tokio::test]
async fn a_key_refusal_on_a_singers_other_track_wins_over_the_hubs_wait() {
    // #140 (carried Minor): the PA (Live's first track of VOX1) waits for
    // its fall back to DRY, the stream's key write is refused; the singer
    // names the refusal, which the engineer can act on, never the wait.
    let dir = tempfile::tempdir().unwrap();
    let (router, _live) = unarmed_in_a_song(dir.path(), &["k7", "s7"]).await;
    let state = router.autotune.state();
    let vox = &state.singers[0];
    assert!(!vox.key_ok, "{vox:?}");
    assert_eq!(vox.why.as_deref(), Some("stream Key refused, wrote D"));
    assert!(!vox.waiting, "a key's why: {vox:?}");
}

#[tokio::test]
async fn a_selector_write_live_refuses_gets_one_retry_then_the_hub_stops() {
    // PR #147 review I5, ruled: a Max map holds the stream's selector.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = ready(dir.path(), HubState::default()).await;
    live.held.insert("p7".into());
    let walk = router.autotune.targets[STREAM].walk;
    aut_on(&mut router);
    transport(&mut router, true);
    live.settle(&mut router).await;
    next_turn(&mut router);
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        vec![
            batch(&[("p6", 0.0)]),
            batch(&[("p7", 127.0)]),
            batch(&[("p7", 127.0)])
        ]
    );
    let failed = of(&steps(&rx), "failed");
    assert_eq!(failed.len(), 2, "{failed:?}");
    assert!(failed.iter().all(|r| r["problem"] == DISABLED));
    assert_eq!(router.autotune.targets[STREAM].walk, walk, "not walked");
    assert_eq!(decisions(&mut router), [on(), off(SwitchWhy::Problem)]);
    assert_eq!(
        router.autotune.state().singers[0].why.as_deref(),
        Some(format!("stream: Live refuses its selector: {DISABLED}").as_str())
    );
    // A key change's brief `mismatch` while its key is written is no new
    // decision (PR #147 re-review R7): the refused chain is not asked again.
    deliver(&mut router, TIME, 55.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(decisions(&mut router), [on(), off(SwitchWhy::Problem)]);
    // A stop is a new decision: the refusal ends (Live shows DRY there).
    transport(&mut router, false);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0)])]);
    // The next start asks once more, and its retry, then stops again.
    transport(&mut router, true);
    live.settle(&mut router).await;
    next_turn(&mut router);
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        vec![
            batch(&[("p6", 0.0)]),
            batch(&[("p7", 127.0)]),
            batch(&[("p7", 127.0)])
        ]
    );
}
