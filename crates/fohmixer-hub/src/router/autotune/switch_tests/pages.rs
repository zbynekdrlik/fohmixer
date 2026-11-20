//! The pages' state: the singers, the targets' problems and the
//! decisions' words.

use super::*;

#[tokio::test]
async fn a_targets_problems_are_its_walks_its_tuner_its_key_device_and_another_writer() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _live, _rx) = scripted_router(dir.path(), HubState::default());
    router.handle(layout(1, vec![stream()]));
    find(&mut router, STREAM, rack(7));
    let fine = router.autotune.targets[STREAM].clone();
    assert!(!has_problem(&fine));
    assert_eq!(trouble(&fine), None);
    let mut walks = fine.clone();
    walks.problem = Some("gave up".into());
    assert!(has_problem(&walks));
    assert_eq!(trouble(&walks).as_deref(), Some("gave up"));
    let mut two = fine.clone();
    two.spec.two_singers = true;
    assert!(has_problem(&two));
    assert_eq!(
        trouble(&two).as_deref(),
        Some("its Tuner names two singers")
    );
    let mut keyless = fine.clone();
    keyless.key = Some(Err("chain 0 holds 2 plug-ins, not one".into()));
    assert!(has_problem(&keyless));
    assert_eq!(
        trouble(&keyless).as_deref(),
        Some("chain 0 holds 2 plug-ins, not one")
    );
    let mut lost = fine.clone();
    lost.key = None;
    assert!(!has_problem(&lost), "a key device looked for again");
    // Only a found target with a key device and no problem counts for the
    // pages' `checking` (PR #147 review N3).
    assert!(checked(&fine));
    assert!(!checked(&walks) && !checked(&keyless) && !checked(&two));
    assert!(!checked(&lost), "no key device followed now");
    let mut unfound = fine.clone();
    unfound.found = None;
    assert!(!checked(&unfound));
    // No id and a placeholder label: no singer, a problem (PR #147 review
    // N6); a label of its own is a singer.
    let mut lone = fine.clone();
    lone.spec.singer = None;
    assert!(has_problem(&lone));
    assert_eq!(trouble(&lone).as_deref(), Some(NO_SINGER));
    lone.spec.label = "Vox 9".into();
    assert!(!has_problem(&lone));
    assert_eq!(trouble(&lone), None);
    // Live refused its selector for good (PR #147 review I5): shown, its
    // decision reads it on its own (no problem input).
    let mut refused = fine.clone();
    refused.refusal = Some(Refusal::after(
        Some(&Refusal::after(None, "p7", true, "held")),
        "p7",
        true,
        "held",
    ));
    assert!(!has_problem(&refused));
    assert_eq!(
        trouble(&refused).as_deref(),
        Some("Live refuses its selector: held")
    );
    let mut once = fine.clone();
    once.refusal = Some(Refusal::after(None, "p7", true, "held"));
    assert_eq!(trouble(&once), None, "its retry goes");
    let mut other_ref = refused.clone();
    other_ref.found = Some(rack(8));
    other_ref.problem = None;
    assert_eq!(trouble(&other_ref), None, "a new ref");
    let mut writer = fine;
    writer.guard.left();
    assert!(!has_problem(&writer), "the guard is its own input");
    assert_eq!(
        trouble(&writer).as_deref(),
        Some("another writer moves its selector")
    );
}

#[tokio::test]
async fn a_singer_off_covers_both_tracks_and_is_saved() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    let page = attach(&mut router, 1);
    page.take();
    set_hub(
        &mut router,
        HUB_AUTOTUNE_SINGER,
        json!({"id": "VOX1", "off": true}),
    );
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0), ("p7", 0.0)])]);
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::SingerOff), off(SwitchWhy::SingerOff)]
    );
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("hub-state.json")).unwrap())
            .unwrap();
    assert_eq!(saved["autotune_off"], json!(["VOX1"]));
    assert_eq!(status(&mut router).autotune.singers_off, ["VOX1"]);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "singer"),
        vec![
            json!({"step": "singer", "id": "VOX1", "off": true, "client": 1,
                    "peer": "10.0.0.1"})
        ]
    );
    // The pages are told: the singer off.
    let told: Vec<_> = page
        .take()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| match m {
            ServerMsg::Autotune(s) => Some(s),
            _ => None,
        })
        .collect();
    assert!(told.last().is_some_and(|s| s.singers[0].off), "{told:?}");
    // Back on: both tune again (their keys are confirmed).
    set_hub(
        &mut router,
        HUB_AUTOTUNE_SINGER,
        json!({"id": "VOX1", "off": false}),
    );
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0), ("p7", 127.0)])]);
    assert!(status(&mut router).autotune.singers_off.is_empty());
    // An id no target carries and a value that is no `{id, off}` get the
    // error reply; nothing changes.
    for value in [
        json!({"id": "VOX9", "off": true}),
        json!({"id": "VOX1"}),
        json!(true),
    ] {
        set_hub(&mut router, HUB_AUTOTUNE_SINGER, value.clone());
        let replies = page.take().unwrap_or_default();
        assert!(
            replies
                .iter()
                .any(|m| matches!(m, ServerMsg::Error { message, .. }
                if message.starts_with("no hub value"))),
            "{value}: {replies:?}"
        );
    }
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
}

#[tokio::test]
async fn a_singer_off_is_kept_across_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let state = HubState {
        stage_aut: false,
        reverb: crate::rules::reverb::Saved::default(),
        fader_links: Vec::new(),
        autotune_aut: true,
        autotune_off: vec!["VOX1".into()],
    };
    let (mut router, mut live, _rx) = ready(dir.path(), state).await;
    // A saved AUT on is honoured at the start (#140 PR 3c).
    assert!(router.autotune.state().aut);
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::SingerOff), off(SwitchWhy::SingerOff)]
    );
    let state = router.autotune.state();
    assert_eq!(state.singers.len(), 1);
    assert!(state.singers[0].off);
}

#[test]
fn a_decision_of_the_hubs_own_has_its_words_and_every_other_none() {
    // PR #150 review I2.
    assert_eq!(
        waiting_words(SwitchWhy::Unarmed, Some("no such function: safe_writes")).as_deref(),
        Some("its fall back to DRY is not kept (no such function: safe_writes)")
    );
    assert_eq!(
        waiting_words(SwitchWhy::Unarmed, None).as_deref(),
        Some("its fall back to DRY is not kept yet")
    );
    assert_eq!(
        waiting_words(SwitchWhy::Held, None).as_deref(),
        Some("waits for Live's track list")
    );
    assert_eq!(
        waiting_words(SwitchWhy::Unconfirmed, None).as_deref(),
        Some("waits for its rack to be found again")
    );
    for why in [
        SwitchWhy::Aut,
        SwitchWhy::Stop,
        SwitchWhy::Span,
        SwitchWhy::NoKey,
        SwitchWhy::SingerOff,
        SwitchWhy::Problem,
        SwitchWhy::AnotherWriter,
        SwitchWhy::Mismatch,
        SwitchWhy::Play,
    ] {
        assert_eq!(waiting_words(why, Some("x")), None, "{why:?}");
    }
}

#[tokio::test]
async fn a_singer_kept_dry_by_its_sets_fall_back_not_kept_says_why() {
    // PR #150 review I2: the pages never read `tuning` with every track DRY
    // for a reason of the hub's own; the singer shows it.
    let dir = tempfile::tempdir().unwrap();
    let (router, _live) = unarmed_in_a_song(dir.path(), &[]).await;
    let state = router.autotune.state();
    let vox = &state.singers[0];
    assert!(!vox.key_ok, "{vox:?}");
    assert_eq!(
        vox.why.as_deref(),
        Some("PA: its fall back to DRY is not kept (no such function: safe_writes)")
    );
    // PR #150 re-review m1: the hub's own wait, never its key's.
    assert!(vox.waiting, "{vox:?}");
}

#[tokio::test]
async fn a_singers_key_is_confirmed_only_once_written_and_read_back() {
    // PR #150 review M4: the panel's ✓ is a read-back, never a key on its
    // way.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = scripted_router(dir.path(), HubState::default());
    router.handle(layout(1, vec![pa(), stream()]));
    find(&mut router, PA, rack(6));
    find(&mut router, STREAM, rack(7));
    heard(&mut router, PA, (0.75, 0.0));
    heard(&mut router, STREAM, (0.5, 1.0 / 28.0));
    transport(&mut router, false);
    map_read(&mut router, 1, songs());
    deliver(&mut router, TEMPO_KEY, 120.0);
    deliver(&mut router, TIME, 10.0);
    // The keys on their way (stopped, they wait for the playhead to stay,
    // #190), not yet read back: no singer is confirmed.
    let state = router.autotune.state();
    assert!(
        state.singers.iter().all(|s| s.key_ok && !s.confirmed),
        "{state:?}"
    );
    keys_now(&mut router, &mut live).await;
    let state = router.autotune.state();
    assert!(state.singers[0].confirmed, "{state:?}");
    // Out of every song: no key in force, nothing confirmed.
    deliver(&mut router, TIME, 110.0);
    assert!(!router.autotune.state().singers[0].confirmed);
}

#[tokio::test]
async fn the_pages_are_told_the_singers_and_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = ready(dir.path(), HubState::default()).await;
    router.handle(layout(
        2,
        vec![pa(), stream(), target(9, "Vox 7", (0, 127), None)],
    ));
    let state = router.autotune.state();
    assert_eq!((state.song, state.key.as_deref()), (Some(1), Some("D")));
    assert_eq!(
        (state.map_keyed, state.map_changes, state.map_problems),
        (2, 1, 0)
    );
    assert!(!state.checking, "AUT off");
    assert_eq!(
        state.singers,
        vec![
            SingerState {
                id: "VOX1".into(),
                label: "Vox 2".into(),
                stream: TrackState::Dry,
                pa: TrackState::Dry,
                off: false,
                key_ok: true,
                why: None,
                confirmed: true,
                waiting: false,
            },
            SingerState {
                id: "#Vox 7".into(),
                label: "Vox 7".into(),
                stream: TrackState::None,
                pa: TrackState::Dry,
                off: false,
                key_ok: true,
                why: None,
                confirmed: false,
                waiting: false,
            },
        ]
    );
    assert_eq!(state.bar.as_deref(), Some("D"));
    // Tuning, the stream's slot then read by another key.
    aut_on(&mut router);
    transport(&mut router, true);
    live.settle(&mut router).await;
    next_turn(&mut router);
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        vec![batch(&[("p6", 0.0)]), batch(&[("p7", 127.0)])]
    );
    let state = router.autotune.state();
    assert_eq!(
        (state.singers[0].stream, state.singers[0].pa),
        (TrackState::Tune, TrackState::Tune)
    );
    let key_sub = router.autotune.targets[STREAM].key_sub.clone().unwrap();
    deliver(&mut router, &key_sub, 7.0 / 11.0);
    let state = router.autotune.state();
    assert!(!state.singers[0].key_ok);
    assert_eq!(
        state.singers[0].why.as_deref(),
        Some("stream Key reads G, wrote D")
    );
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        vec![batch(&[("p7", 0.0)])],
        "another writer's key: DRY"
    );
}

#[tokio::test]
async fn a_missing_target_is_a_problem_never_a_key_being_checked() {
    // PR #147 review N3.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = tuning(dir.path()).await;
    let walk = router.autotune.targets[STREAM].walk;
    router.autotune_found(STREAM, walk, Err("no rack named Latencies".into()));
    live.settle(&mut router).await;
    let state = router.autotune.state();
    assert!(!state.checking, "a missing rack is no key being checked");
    let vox1 = &state.singers[0];
    assert!(!vox1.key_ok);
    assert_eq!(vox1.why.as_deref(), Some("stream: no rack named Latencies"));
    assert!(!vox1.waiting, "a target's trouble");
    assert_eq!(vox1.stream, TrackState::Dry);
    // A key device missing: a problem too, never checking.
    let mut keyless = rack(7);
    keyless.key = Err(KeyMissing::NoChain);
    let walk = router.autotune.targets[STREAM].walk;
    router.autotune_found(STREAM, walk, Ok(keyless));
    let state = router.autotune.state();
    assert!(!state.checking);
    assert!(!state.singers[0].key_ok);
}

#[tokio::test]
async fn a_singers_manual_off_is_cleared_for_an_id_no_target_carries() {
    // PR #147 review N6: an id a marker edit left behind.
    let dir = tempfile::tempdir().unwrap();
    let state = HubState {
        stage_aut: false,
        reverb: crate::rules::reverb::Saved::default(),
        fader_links: Vec::new(),
        autotune_aut: false,
        autotune_off: vec!["VOX1".into(), "VOX9".into()],
    };
    let (mut router, _live, rx) = ready(dir.path(), state).await;
    let page = attach(&mut router, 1);
    page.take();
    // Off again while off: no target carries it, the error reply.
    set_hub(
        &mut router,
        HUB_AUTOTUNE_SINGER,
        json!({"id": "VOX9", "off": true}),
    );
    let replies = page.take().unwrap_or_default();
    assert!(
        replies
            .iter()
            .any(|m| matches!(m, ServerMsg::Error { message, .. }
                if message.starts_with("no hub value"))),
        "{replies:?}"
    );
    set_hub(
        &mut router,
        HUB_AUTOTUNE_SINGER,
        json!({"id": "VOX9", "off": false}),
    );
    assert_eq!(
        of(&steps(&rx), "singer"),
        vec![
            json!({"step": "singer", "id": "VOX9", "off": false, "client": 1,
                    "peer": "10.0.0.1"})
        ]
    );
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("hub-state.json")).unwrap())
            .unwrap();
    assert_eq!(saved["autotune_off"], json!(["VOX1"]));
    // Off again: no target carries it, the error reply.
    set_hub(
        &mut router,
        HUB_AUTOTUNE_SINGER,
        json!({"id": "VOX9", "off": true}),
    );
    let replies = page.take().unwrap_or_default();
    assert!(
        replies
            .iter()
            .any(|m| matches!(m, ServerMsg::Error { message, .. }
                if message.starts_with("no hub value"))),
        "{replies:?}"
    );
}

#[tokio::test]
async fn a_target_naming_no_singer_has_no_manual_off_and_never_tunes() {
    // PR #147 review N6: a placeholder label (`#10`) names a track index,
    // never a singer.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = ready(dir.path(), HubState::default()).await;
    let lone = "band|live_set tracks 9";
    router.handle(layout(
        2,
        vec![pa(), stream(), target(9, "#10", (0, 127), None)],
    ));
    find(&mut router, lone, rack(9));
    deliver_macro(&mut router, "p9", 127.0);
    aut_on(&mut router);
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(decisions(&mut router)[2], off(SwitchWhy::Problem));
    let state = router.autotune.state();
    let lone_singer = &state.singers[1];
    assert_eq!(lone_singer.id, "");
    assert!(!lone_singer.off);
    assert_eq!(
        lone_singer.why.as_deref(),
        Some(format!("PA: {}", super::super::NO_SINGER).as_str())
    );
    // No id to switch off by hand: `##10` is no singer.
    let page = attach(&mut router, 1);
    page.take();
    set_hub(
        &mut router,
        HUB_AUTOTUNE_SINGER,
        json!({"id": "##10", "off": true}),
    );
    assert!(
        page.take()
            .unwrap_or_default()
            .iter()
            .any(|m| matches!(m, ServerMsg::Error { .. }))
    );
}
