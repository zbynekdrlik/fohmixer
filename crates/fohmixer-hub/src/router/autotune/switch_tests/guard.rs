//! The guards: another writer on a selector, the keys' read-back, and the
//! script's DRY registration (`armed`).

use super::*;

#[test]
fn the_glues_small_decisions() {
    // The fall back is armed for a selector only while both the list the
    // script last kept and the newest one sent have it with its off value
    // (PR #146 review I2, the PR #147 ruling on I2).
    let both = vec![("p6".to_string(), 127), ("p7".to_string(), 0)];
    let safe = |list: &[(String, u8)], kept: Option<&[(String, u8)]>, state| Safe {
        list: list.to_vec(),
        seq: 3,
        state,
        problem: None,
        failures: 0,
        kept: kept.map(<[(String, u8)]>::to_vec),
        probes: 0,
        lost: false,
    };
    let kept = safe(both.as_slice(), Some(both.as_slice()), SafeState::Kept);
    assert!(armed(Some(&kept), "p6", 127));
    assert!(armed(Some(&kept), "p7", 0));
    assert!(!armed(Some(&kept), "p7", 127), "another off value");
    assert!(!armed(Some(&kept), "p8", 0), "not in the list");
    assert!(!armed(None, "p6", 127), "nothing registered");
    // A list that adds a target, on its way or refused: the two it shares
    // with the kept one stay armed, the new one is not.
    let more = vec![both[0].clone(), both[1].clone(), ("p8".to_string(), 0)];
    for state in [SafeState::Sent, SafeState::Failed] {
        let adding = safe(more.as_slice(), Some(both.as_slice()), state);
        assert!(armed(Some(&adding), "p6", 127), "{state:?}");
        assert!(armed(Some(&adding), "p7", 0), "{state:?}");
        assert!(!armed(Some(&adding), "p8", 0), "{state:?}");
    }
    // A list that drops a selector's write: no longer armed, whatever the
    // script holds; nothing kept yet: none armed.
    let fewer = safe(&both[..1], Some(both.as_slice()), SafeState::Sent);
    assert!(armed(Some(&fewer), "p6", 127));
    assert!(!armed(Some(&fewer), "p7", 0));
    let first = safe(both.as_slice(), None, SafeState::Sent);
    assert!(!armed(Some(&first), "p6", 127));
    // A singer's off by hand: `{id, off}`, nothing more.
    assert_eq!(
        singer_value(&json!({"id": "VOX1", "off": true})),
        Some(("VOX1".to_string(), true))
    );
    assert_eq!(
        singer_value(&json!({"off": false, "id": "#Vox 7"})),
        Some(("#Vox 7".to_string(), false))
    );
    for bad in [
        json!({"id": "VOX1"}),
        json!({"id": "VOX1", "off": 1}),
        json!({"id": 1, "off": true}),
        json!({"id": "VOX1", "off": true, "more": 1}),
        json!({"name": "VOX1", "off": true}),
        json!({"id": "VOX1", "on": true}),
        json!(true),
        json!(["VOX1", true]),
    ] {
        assert_eq!(singer_value(&bad), None, "{bad}");
    }
    // A key being checked: AUT on, Live playing in a keyed song, a key on
    // its way.
    let d = Some(Key {
        root: 2,
        minor: false,
    });
    let pending = || [KeyCheck::Confirmed, KeyCheck::Pending].into_iter();
    assert!(checking(true, Some(true), d, pending()));
    assert!(!checking(false, Some(true), d, pending()), "AUT off");
    assert!(!checking(true, Some(false), d, pending()), "stopped");
    assert!(!checking(true, None, d, pending()), "not heard");
    assert!(!checking(true, Some(true), None, pending()), "no key");
    assert!(!checking(
        true,
        Some(true),
        d,
        [KeyCheck::Confirmed, KeyCheck::Bad].into_iter()
    ));
    // A singer's tracks, part and label.
    assert_eq!(track_state(None), TrackState::None);
    assert_eq!(track_state(Some(TargetState::On)), TrackState::Tune);
    for dry in [TargetState::Off, TargetState::Waiting, TargetState::Missing] {
        assert_eq!(track_state(Some(dry)), TrackState::Dry, "{dry:?}");
    }
    assert_eq!((part(0), part(127), part(64)), ("PA", "stream", "stream"));
    assert_eq!(singer_label(&["#8", "Vox 2"]), "Vox 2");
    assert_eq!(singer_label(&["Vox 1", "Vox 2"]), "Vox 1");
    assert_eq!(singer_label(&["#8", "#9"]), "#8");
    assert_eq!(singer_label(&[]), "");
    // Why a key is wrong.
    let f = Key {
        root: 5,
        minor: false,
    };
    let g = Key {
        root: 7,
        minor: false,
    };
    assert_eq!(
        key_why("stream", KeyState::Mismatch, Some(f), Some(g)),
        "stream Key reads G, wrote F"
    );
    assert_eq!(
        key_why("PA", KeyState::AnotherWriter, Some(f), None),
        "PA Key reads no key, wrote F"
    );
    assert_eq!(
        key_why("PA", KeyState::Refused, Some(f), Some(g)),
        "PA Key refused, wrote F"
    );
    assert_eq!(
        key_why("PA", KeyState::Mismatch, None, Some(g)),
        "PA Key reads G, wrote no key"
    );
}

#[tokio::test]
async fn a_key_change_keeps_tuning_and_a_bad_read_back_drops_only_that_track() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    // A Max map holds the stream's slots now: its key write is refused.
    live.held.insert("k7".into());
    live.held.insert("s7".into());
    // The key clip to G passed: both stay on through the write.
    deliver(&mut router, TIME, 55.0);
    assert_eq!(decisions(&mut router), [on(), on()]);
    live.quiet_until_written(&mut router).await;
    // The stream's write refused: that track alone goes DRY; the PA's
    // read-back confirms G and it stays on.
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p7", 0.0)])]);
    assert_eq!(decisions(&mut router), [on(), off(SwitchWhy::Mismatch)]);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "select"),
        vec![select(7, false, "mismatch", "key")]
    );
    let singer = &router.autotune.state().singers[0];
    assert_eq!(
        (singer.stream, singer.pa, singer.key_ok),
        (TrackState::Dry, TrackState::Tune, false)
    );
    assert_eq!(singer.why.as_deref(), Some("stream Key refused, wrote G"));
    assert!(!singer.waiting, "a key's why");
}

#[tokio::test]
async fn a_read_back_that_disagrees_drops_its_track_before_the_retry() {
    // The stream's slot reads another key back: DRY at once, the one retry
    // after it; a confirmed retry turns it on again.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = tuning(dir.path()).await;
    deliver(&mut router, TIME, 55.0);
    live.quiet_until_written(&mut router).await;
    let read = live.next("the read-back", |c| {
        c.iter().all(|c| c["name"] == "get_prop")
    });
    let slots: Vec<Value> = read
        .commands
        .iter()
        .map(|c| match ref_of(c) {
            // The stream's Key reads C, not G.
            Some("k7") => ok(json!(0.0)),
            Some(id) => ok(live.params[id].clone()),
            None => ok(live.params[c["args"]["prop"].as_str().unwrap()].clone()),
        })
        .collect();
    read.answer(slots);
    let msg = next_of(&mut live.own, "the read-back", |m| {
        matches!(m, RouterMsg::AutotuneKeysRead { .. })
    })
    .await;
    router.handle(msg);
    // DRY before the retry goes: the batch first, then the retry's write.
    let dry = live.next("the stream's DRY", is_selector_write);
    assert_eq!(dry.commands.len(), 1);
    assert_eq!(ref_of(&dry.commands[0]), Some("p7"));
    live.answer(&mut router, dry);
    assert_eq!(live.written(), vec![batch(&[("p7", 0.0)])]);
    assert_eq!(decisions(&mut router)[1], off(SwitchWhy::Mismatch));
    live.settle(&mut router).await;
    // The retry confirmed G: on again.
    assert_eq!(live.written(), vec![batch(&[("p7", 127.0)])]);
    assert_eq!(decisions(&mut router), [on(), on()]);
}

#[tokio::test]
async fn another_writer_is_shown_and_dry_wins() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = ready(dir.path(), HubState::default()).await;
    // AUT off: the stream's selector moved to its chain by hand: DRY again.
    by_hand(&mut router, &mut live, "p7", 127.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p7", 0.0)])]);
    let writer = |act: &str| {
        json!({"step": "selector_writer", "instance": "band", "target": "live_set tracks 7",
               "act": act})
    };
    assert_eq!(
        of(&steps(&rx), "selector_writer"),
        vec![writer("dry_again")]
    );
    // Again within 10 s: the hub stops writing it, and says so.
    by_hand(&mut router, &mut live, "p7", 127.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(of(&steps(&rx), "selector_writer"), vec![writer("gave_up")]);
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Aut), off(SwitchWhy::Aut)],
        "AUT off comes first"
    );
    aut_on(&mut router);
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(decisions(&mut router)[1], off(SwitchWhy::AnotherWriter));
    let singer = &router.autotune.state().singers[0];
    assert_eq!(
        singer.why.as_deref(),
        Some("stream: another writer moves its selector")
    );
    assert!(!singer.waiting, "a target's why");
    // The PA tunes meanwhile; the stream's selector left as it is.
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0)])]);
    // The next song start resets the guard: DRY again, then on once its
    // key is confirmed.
    deliver(&mut router, TIME, 105.0);
    live.settle(&mut router).await;
    let _ = live.written();
    deliver(&mut router, TIME, 125.0);
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        vec![batch(&[("p7", 0.0)]), batch(&[("p6", 0.0), ("p7", 127.0)])]
    );
    assert_eq!(decisions(&mut router), [on(), on()]);
}

#[tokio::test]
async fn a_selector_another_writer_takes_to_dry_is_left_so() {
    // The hub never fights to turn autotune on (spec §7).
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    by_hand(&mut router, &mut live, "p6", 127.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    let got = steps(&rx);
    assert_eq!(
        of(&got, "selector_writer"),
        vec![json!({"step": "selector_writer", "instance": "band",
                    "target": "live_set tracks 6", "act": "left_dry"})]
    );
    assert_eq!(
        of(&got, "select"),
        vec![select(6, false, "another_writer", "guard")]
    );
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::AnotherWriter), on()]
    );
    // A key change after it writes it nothing.
    deliver(&mut router, TIME, 55.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
}

#[tokio::test]
async fn the_hubs_own_echo_is_no_other_writer() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    // The echoes of the ON batch came back during the settle: no
    // `selector_writer` and nothing written again.
    assert_eq!(of(&steps(&rx), "selector_writer"), Vec::<Value>::new());
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
}

#[tokio::test]
async fn a_registration_mid_song_takes_no_selector_dry_and_one_dropping_its_write_does() {
    // The PR #147 ruling on I2: a selector stays on while both the list the
    // script kept and the newest one sent have its DRY write.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    // A third target found mid-song: the list goes again (sent) with the
    // two tuning selectors in it: nothing flicks.
    let third = "band|live_set tracks 8";
    router.handle(layout(
        2,
        vec![pa(), stream(), target(8, "Vox 9", (127, 0), Some("VOX9"))],
    ));
    find(&mut router, third, rack(8));
    assert_eq!(router.autotune.safe["band"].state, SafeState::Sent);
    assert_eq!(decisions(&mut router)[..2], [on(), on()]);
    live.settle(&mut router).await;
    assert_eq!(router.autotune.safe["band"].state, SafeState::Kept);
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new(), "no flick");
    assert_eq!(decisions(&mut router)[..2], [on(), on()]);
    assert_eq!(of(&steps(&rx), "select"), Vec::<Value>::new());
    // A registration Live did not answer: both lists still have the two
    // selectors (the script holds one of them): they stay on.
    router.handle(layout(3, vec![pa(), stream()]));
    let registration = live.next("the registration", |c| {
        c.len() == 1 && c[0]["name"] == SAFE_WRITES
    });
    drop(registration);
    live.settle(&mut router).await;
    assert_eq!(router.autotune.safe["band"].state, SafeState::Failed);
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(decisions(&mut router), [on(), on()]);
    // The stream rack replaced in place: its new ref is in no list the
    // script kept, so it is DRY until the new list is kept, then on again.
    let replaced = Found {
        path: rack(7).path,
        id: "p7b".into(),
        key: Ok(device(7)),
    };
    find(&mut router, STREAM, replaced);
    deliver_macro(&mut router, "p7b", 127.0);
    assert_eq!(decisions(&mut router)[1], off(SwitchWhy::Unarmed));
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        vec![batch(&[("p7b", 0.0)]), batch(&[("p7b", 127.0)])]
    );
    assert_eq!(decisions(&mut router), [on(), on()]);
}

#[tokio::test]
async fn a_keyless_song_start_ends_the_guards_give_up() {
    // PR #147 review N5: a keyless song entered from out of every song is
    // a song start too.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = ready(dir.path(), HubState::default()).await;
    // Another writer twice within 10 s (AUT off): the hub stops.
    by_hand(&mut router, &mut live, "p7", 127.0);
    live.settle(&mut router).await;
    by_hand(&mut router, &mut live, "p7", 127.0);
    live.settle(&mut router).await;
    let _ = live.written();
    assert_eq!(
        of(&steps(&rx), "selector_writer").last().unwrap()["act"],
        "gave_up"
    );
    // Out of every song, then into song 3 (no key): DRY again.
    deliver(&mut router, TIME, 105.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new(), "a span");
    deliver(&mut router, TIME, 215.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p7", 0.0)])]);
}

#[tokio::test]
async fn a_list_sent_over_one_still_on_its_way_narrows_what_the_script_may_hold() {
    // PR #147 re-review R4: the script may take either list.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = ready(dir.path(), HubState::default()).await;
    // The stream's rack gone for good (DRY: nothing to write first): a list
    // without it goes, unanswered yet.
    let walk = router.autotune.targets[STREAM].walk;
    router.autotune_found(STREAM, walk, Err("no rack named Latencies".into()));
    let without = live.next("the list without it", |c| {
        c.len() == 1 && c[0]["name"] == SAFE_WRITES
    });
    // Found again with the same ref: the list with it goes after.
    find(&mut router, STREAM, rack(7));
    assert_eq!(
        router.autotune.safe["band"].kept.as_deref(),
        Some(&[("p6".to_string(), 127)][..])
    );
    aut_on(&mut router);
    transport(&mut router, true);
    assert_eq!(decisions(&mut router)[1], off(SwitchWhy::Unarmed));
    drop(without);
}

#[tokio::test]
async fn a_list_live_did_not_answer_narrows_what_the_script_may_hold() {
    // PR #147 re-review R15: a registration Live did not answer
    // (`Safe.lost`) may still become the script's list, so the list after
    // it keeps only the pairs both share; one the script refused keeps the
    // kept list whole.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = ready(dir.path(), HubState::default()).await;
    let both = [("p6".to_string(), 127), ("p7".to_string(), 0)];
    let without_stream = |router: &mut Router| {
        // The stream's rack gone for good (DRY: nothing to write first).
        let walk = router.autotune.targets[STREAM].walk;
        router.autotune_found(STREAM, walk, Err("no rack named Latencies".into()));
    };
    let is_list = |c: &[Value]| c.len() == 1 && c[0]["name"] == SAFE_WRITES;
    // Refused: the list without the stream goes and the script refuses it.
    without_stream(&mut router);
    let refused = live.next("the list without it", is_list);
    refused.answer(vec![json!({"ok": false,
        "error": "no such function: safe_writes", "errorType": "OpError"})]);
    live.settle(&mut router).await;
    let safe = &router.autotune.safe["band"];
    assert_eq!(safe.state, SafeState::Failed);
    assert!(!safe.lost, "refused: the script did not take it");
    // Found again: the list with it goes after, the kept list whole.
    find(&mut router, STREAM, rack(7));
    assert_eq!(
        router.autotune.safe["band"].kept.as_deref(),
        Some(&both[..]),
        "kept whole after a refusal"
    );
    live.settle(&mut router).await;
    assert_eq!(router.autotune.safe["band"].state, SafeState::Kept);
    // Not answered: the list without the stream goes, Live never answers.
    without_stream(&mut router);
    let lost = live.next("the list without it, again", is_list);
    drop(lost);
    live.settle(&mut router).await;
    let safe = &router.autotune.safe["band"];
    assert_eq!(safe.state, SafeState::Failed);
    assert!(safe.lost, "Live did not answer it");
    assert_eq!(safe.kept.as_deref(), Some(&both[..]));
    // Found again: the script may hold the list without it, so only the
    // pair both share stays kept and the stream is not armed.
    find(&mut router, STREAM, rack(7));
    assert_eq!(
        router.autotune.safe["band"].kept.as_deref(),
        Some(&both[..1])
    );
    aut_on(&mut router);
    transport(&mut router, true);
    assert_eq!(decisions(&mut router)[1], off(SwitchWhy::Unarmed));
}

#[tokio::test]
async fn a_walk_that_gives_up_unconfirmed_leaves_its_target_out_of_the_registered_list() {
    // PR #147 re-review R8: two track-list changes in one hold; the PA's
    // walk after the second gives up, keeping the ref the first change's
    // walk found, which an older layout named at its index.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = tuning(dir.path()).await;
    let seq = router.autotune.safe["band"].seq;
    router.autotune_tracks("band");
    live.settle(&mut router).await;
    find(
        &mut router,
        PA,
        Found {
            id: "p8".into(),
            ..rack(6)
        },
    );
    find(&mut router, STREAM, rack(7));
    router.autotune_tracks("band");
    find(&mut router, STREAM, rack(7));
    let walk = router.autotune.targets[PA].walk;
    for _ in 0..=super::super::super::super::retries::CONNECT_RETRIES {
        router.autotune_walk_failed(PA, walk, "no result within 3 s".into());
    }
    let pa = &router.autotune.targets[PA];
    assert!(pa.problem.as_deref().is_some_and(|p| p.contains("gave up")));
    assert_eq!(pa.found.as_ref().map(|f| f.id.as_str()), Some("p8"));
    assert!(!pa.sure);
    // The layout ends the hold: the stream registers, never the PA by
    // `p8`.
    router.handle(layout(
        2,
        vec![target(6, "Vox 5", (0, 127), Some("VOX1")), stream()],
    ));
    let safe = &router.autotune.safe["band"];
    assert!(safe.seq > seq, "registered");
    assert_eq!(safe.list, vec![("p7".to_string(), 0)]);
    live.settle(&mut router).await;
    assert!(live.written().iter().flatten().all(|(id, _)| id != "p8"));
}
