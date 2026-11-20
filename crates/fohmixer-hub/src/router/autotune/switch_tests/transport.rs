//! The switch-on and the stop (AUTOTUNE AUT, Live's transport, the songs
//! and their keys), and the spreads of their selector writes.

use super::*;

#[tokio::test]
async fn aut_off_writes_dry_to_a_selector_that_shows_the_chain() {
    // Spec §2.2: AUT off, every singer on DRY, always; one write per
    // change.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = scripted_router(dir.path(), HubState::default());
    router.handle(layout(1, vec![pa(), stream()]));
    find(&mut router, PA, rack(6));
    find(&mut router, STREAM, rack(7));
    // The PA's selector on its chain (0), the stream's on DRY.
    deliver_macro(&mut router, "p6", 0.0);
    deliver_macro(&mut router, "p7", 0.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0)])]);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "write"),
        vec![
            json!({"step": "write", "instance": "band", "cause": "found", "on": [],
                    "off": ["Vox 2"]})
        ]
    );
    assert_eq!(of(&got, "select"), Vec::<Value>::new(), "no switch: DRY");
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Aut), off(SwitchWhy::Aut)]
    );
    // Its echo: nothing more; found again: nothing more.
    find(&mut router, PA, rack(6));
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(status(&mut router).autotune.writes, 1);
}

#[tokio::test]
async fn aut_on_and_playing_in_a_keyed_song_switches_on_after_the_keys() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = ready(dir.path(), HubState::default()).await;
    // Out of every song, AUT on, Live plays: DRY (span).
    deliver(&mut router, TIME, 105.0);
    aut_on(&mut router);
    transport(&mut router, true);
    // Live plays on: a song start is one batch (#190).
    router.autotune_played_on("band");
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Span), off(SwitchWhy::Span)]
    );
    // Song 2 starts: its key goes first; no selector moves before its
    // read-back confirms it.
    deliver(&mut router, TIME, 125.0);
    let write = live.next("the song start's key write", |c| {
        c.iter().all(|c| c["name"] == "set_prop") && !is_selector_write(c)
    });
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Mismatch), off(SwitchWhy::Mismatch)]
    );
    let n = write.commands.len();
    let commands = write.commands.clone();
    for c in &commands {
        live.params.insert(
            ref_of(c)
                .map(str::to_string)
                .unwrap_or_else(|| c["args"]["prop"].as_str().unwrap().to_string()),
            c["args"]["value"].clone(),
        );
    }
    write.answer(vec![ok(Value::Null); n]);
    let msg = next_of(&mut live.own, "the key write's answer", |m| {
        matches!(m, RouterMsg::AutotuneKeysWritten { .. })
    })
    .await;
    router.handle(msg);
    live.quiet();
    assert_eq!(
        live.written(),
        Vec::<Vec<(String, f64)>>::new(),
        "still checking"
    );
    assert!(router.autotune.state().checking);
    // The read-back confirms both: one batch, each its own on value.
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0), ("p7", 127.0)])]);
    assert_eq!(decisions(&mut router), [on(), on()]);
    assert!(!router.autotune.state().checking);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "select"),
        vec![
            select(6, true, "play", "key"),
            select(7, true, "play", "key")
        ]
    );
    assert_eq!(
        of(&got, "write"),
        vec![json!({"step": "write", "instance": "band", "cause": "key",
                    "on": ["Vox 2", "#8"], "off": []})]
    );
    assert_eq!(router.autotune.state().targets.len(), 2);
}

#[tokio::test]
async fn aut_on_with_the_keys_confirmed_switches_on_at_once() {
    // The keys follow the song position with AUT off too (spec §2.2), so
    // AUT on in a keyed song tunes at once.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = ready(dir.path(), HubState::default()).await;
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new(), "AUT off");
    aut_on(&mut router);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0), ("p7", 127.0)])]);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "select"),
        vec![
            select(6, true, "play", "flag"),
            select(7, true, "play", "flag")
        ]
    );
    assert_eq!(
        of(&got, "flag"),
        vec![json!({"step": "flag", "on": true, "client": 1, "peer": null})]
    );
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("hub-state.json")).unwrap())
            .unwrap();
    assert_eq!(saved["autotune_aut"], json!(true), "saved on");
    // AUT off: DRY at once.
    set_hub(&mut router, HUB_AUTOTUNE_AUT, json!(false));
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0), ("p7", 0.0)])]);
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Aut), off(SwitchWhy::Aut)]
    );
}

#[tokio::test]
async fn a_stop_mid_song_writes_dry_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    transport(&mut router, false);
    // At once, before any read-back: the call is on its way already (the
    // first selector; the rest follow one per 40 ms since #140, see
    // `a_stop_writes_its_dry_one_selector_per_40_ms_in_lives_order`).
    let dry = live.next("the stop's DRY", is_selector_write);
    assert_eq!(
        dry.commands,
        vec![
            json!({"target": {"$ref": "p6", "class": "DeviceParameter"}, "name": "set_prop",
                   "args": {"prop": "value", "value": 127.0}}),
        ]
    );
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Stop), off(SwitchWhy::Stop)]
    );
    let got = steps(&rx);
    assert_eq!(
        of(&got, "select"),
        vec![
            select(6, false, "stop", "transport"),
            select(7, false, "stop", "transport")
        ]
    );
}

#[tokio::test]
async fn out_of_a_song_and_a_keyless_song_are_dry() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    // Song 1 ends: DRY (span).
    deliver(&mut router, TIME, 105.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0), ("p7", 0.0)])]);
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Span), off(SwitchWhy::Span)]
    );
    // Song 3 has no key: DRY stays (no_key), nothing written again.
    deliver(&mut router, TIME, 215.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::NoKey), off(SwitchWhy::NoKey)]
    );
    let state = router.autotune.state();
    assert_eq!((state.song, state.key), (Some(3), None));
    let got = steps(&rx);
    assert_eq!(
        of(&got, "select"),
        vec![
            select(6, false, "span", "position"),
            select(7, false, "span", "position")
        ]
    );
}

#[tokio::test]
async fn a_span_ends_and_starts_at_the_cues_own_value() {
    // The PC's cue times are the beat plus 5.2e-5 (hub-rust.md, the song
    // map): a playhead between the rounded beat and Live's value is still
    // in the song before an end, and still out of every song before a
    // start.
    let cue = |name: &str, time: f64| Cue {
        name: name.to_string(),
        time: Some(time),
    };
    let listing = Listing {
        cues: vec![
            cue("001 Test One {D 120}", 0.0),
            cue("SONG END", 100.000052291458),
            cue("002 Test Two {D 90}", 120.000052291458),
            cue("SONG END", 200.000052291458),
        ],
        clips: Vec::new(),
        sections: SectionsState::One,
        track: None,
        sections_error: None,
    };
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = ready_with(dir.path(), HubState::default(), listing).await;
    aut_on(&mut router);
    transport(&mut router, true);
    live.settle(&mut router).await;
    next_turn(&mut router);
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        vec![batch(&[("p6", 0.0)]), batch(&[("p7", 127.0)])]
    );
    // Live plays: the playhead is judged one tick ahead.
    let ahead = crate::rules::autotune::keys::lookahead_beats(Some(120.0));
    assert!(ahead > 0.0);
    deliver(&mut router, TIME, 100.00002 - ahead);
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        Vec::<Vec<(String, f64)>>::new(),
        "past the rounded end"
    );
    assert_eq!(decisions(&mut router), [on(), on()]);
    deliver(&mut router, TIME, 100.00006 - ahead);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0), ("p7", 0.0)])]);
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Span), off(SwitchWhy::Span)]
    );
    deliver(&mut router, TIME, 120.00002 - ahead);
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        Vec::<Vec<(String, f64)>>::new(),
        "past the rounded start"
    );
    assert_eq!(router.autotune.state().song, None);
    deliver(&mut router, TIME, 120.00006 - ahead);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0), ("p7", 127.0)])]);
    assert_eq!(router.autotune.state().song, Some(2));
    assert_eq!(decisions(&mut router), [on(), on()]);
}

#[tokio::test]
async fn no_write_when_live_already_shows_the_decision() {
    // One write per change: the same decision again, a playhead that stays
    // in the song, the macro found again write nothing.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    aut_on(&mut router);
    transport(&mut router, true);
    deliver(&mut router, TIME, 11.0);
    deliver(&mut router, TIME, 12.0);
    find(&mut router, PA, rack(6));
    find(&mut router, STREAM, rack(7));
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    let got = steps(&rx);
    assert_eq!(of(&got, "write"), Vec::<Value>::new());
    assert_eq!(of(&got, "select"), Vec::<Value>::new());
    assert_eq!(
        status(&mut router).autotune.writes,
        2,
        "tuning's switch-on, one selector per batch"
    );
}

#[tokio::test(start_paused = true)]
async fn a_stop_before_the_switch_on_landed_writes_dry_at_once() {
    // PR #147 review I1 (a): Live stopped before it took the switch-on, so
    // it still shows DRY while the chain's write is on its way; DRY is
    // written at once, and Live takes both in one tick (both answers, then
    // one value per selector, the last). Since #140 the stop's DRY is
    // spread: the PA's at once, the stream's 40 ms later. Since #190 the
    // switch-on is spread too: both its writes are sent before the stop.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = ready(dir.path(), HubState::default()).await;
    aut_on(&mut router);
    transport(&mut router, true);
    let pa_on = live.next("the PA's switch-on", is_selector_write);
    next_turn(&mut router);
    let stream_on = live.next("the stream's switch-on", is_selector_write);
    transport(&mut router, false);
    let dry = live.next("the stop's DRY", is_selector_write);
    assert_eq!(dry.commands, selector_commands(&[("p6", 127.0)]));
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Stop), off(SwitchWhy::Stop)]
    );
    live.answer_quiet(pa_on);
    let stream = live.answer_quiet(stream_on);
    let last = live.answer_quiet(dry);
    written_handled(&mut router, &mut live).await;
    written_handled(&mut router, &mut live).await;
    written_handled(&mut router, &mut live).await;
    // The stream's chain lands (its DRY still to come), then the PA's DRY.
    for (id, value) in stream.iter().chain(&last) {
        deliver_macro(&mut router, id, *value);
    }
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        vec![
            batch(&[("p6", 0.0)]),
            batch(&[("p7", 127.0)]),
            batch(&[("p6", 127.0)])
        ]
    );
    pass(&mut router, &mut live, SPREAD).await;
    assert_eq!(live.written(), vec![batch(&[("p7", 0.0)])]);
    assert_eq!(of(&steps(&rx), "selector_writer"), Vec::<Value>::new());
    assert!(router.autotune.targets[PA].writing.is_empty());
    assert!(router.autotune.targets[STREAM].writing.is_empty());
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Stop), off(SwitchWhy::Stop)]
    );
}

#[tokio::test(start_paused = true)]
async fn a_start_before_the_stops_dry_landed_writes_the_chain_at_once() {
    // PR #147 review I1, the other direction: Live plays again before it
    // took the stop's DRY; each write in its own tick, one echo after its
    // answer and the other before it. No echo is another writer. Since
    // #140 the stop wrote only the PA's DRY before the start (the stream's
    // was still to come): the start drops it, and Live still shows the
    // stream's chain, so the start writes the PA's chain only.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    transport(&mut router, false);
    let dry = live.next("the stop's DRY", is_selector_write);
    transport(&mut router, true);
    let chain = live.next("the start's chain", is_selector_write);
    assert_eq!(chain.commands, selector_commands(&[("p6", 0.0)]));
    // The DRY's tick: its answer, then its values.
    let echoes = live.answer_quiet(dry);
    written_handled(&mut router, &mut live).await;
    for (id, value) in &echoes {
        deliver_macro(&mut router, id, *value);
    }
    // The chain's tick: its values first, then its answer.
    let echoes = live.answer_quiet(chain);
    for (id, value) in &echoes {
        deliver_macro(&mut router, id, *value);
    }
    written_handled(&mut router, &mut live).await;
    pass(&mut router, &mut live, SPREAD * 5).await;
    assert_eq!(
        live.written(),
        vec![batch(&[("p6", 127.0)]), batch(&[("p6", 0.0)])]
    );
    assert_eq!(of(&steps(&rx), "selector_writer"), Vec::<Value>::new());
    assert_eq!(decisions(&mut router), [on(), on()]);
    assert!(router.autotune.targets[PA].writing.is_empty());
    assert!(router.autotune.targets[STREAM].writing.is_empty());
}

#[tokio::test]
async fn a_dry_decision_writes_a_stream_selector_anywhere_in_its_chains_zone() {
    // PR #147 review N4, ruled (DRY wins): 64 lies in a stream rack's
    // autotune zone (58 to 127 on the PC); the pages' window reads it off.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = scripted_router(dir.path(), HubState::default());
    router.handle(layout(1, vec![stream()]));
    find(&mut router, STREAM, rack(7));
    deliver_macro(&mut router, "p7", 64.0);
    assert_eq!(router.autotune.state().targets[0].state, TargetState::Off);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p7", 0.0)])]);
}

#[tokio::test(start_paused = true)]
async fn a_stop_after_a_switch_on_live_did_not_answer_writes_dry_at_once() {
    // PR #147 re-review R3: Live may still take the switch-on. Since #140
    // the stop's DRY is spread: the PA's at once, the stream's 40 ms later;
    // since #190 the switch-on too, both its writes sent before the stop.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = ready(dir.path(), HubState::default()).await;
    aut_on(&mut router);
    transport(&mut router, true);
    let pa_on = live.next("the PA's switch-on", is_selector_write);
    let walk = router.autotune.targets[STREAM].walk;
    drop(pa_on);
    written_handled(&mut router, &mut live).await;
    next_turn(&mut router);
    let stream_on = live.next("the stream's switch-on", is_selector_write);
    drop(stream_on);
    written_handled(&mut router, &mut live).await;
    let target = &router.autotune.targets[STREAM];
    assert_eq!(target.writing_now(), Some(true), "still on its way");
    assert!(target.sure);
    assert_eq!(target.walk, walk, "not walked");
    transport(&mut router, false);
    let dry = live.next("the stop's DRY", is_selector_write);
    assert_eq!(dry.commands, selector_commands(&[("p6", 127.0)]));
    assert_eq!(of(&steps(&rx), "failed").len(), 2);
    pass(&mut router, &mut live, SPREAD).await;
    assert_eq!(live.written().last(), Some(&batch(&[("p7", 0.0)])));
}

#[tokio::test(start_paused = true)]
async fn a_stop_after_a_lost_write_and_an_answered_one_writes_dry_at_once() {
    // PR #147 re-review R9: the stop's DRY goes unanswered, the start's
    // chain is answered, and its value comes while the markers are
    // unsettled (no decision then); the next stop writes DRY. Since #140
    // the stop's DRY is spread: the PA's at once; the start drops the
    // stream's still to come (Live still shows its chain).
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = tuning(dir.path()).await;
    transport(&mut router, false);
    let lost = live.next("the stop's DRY", is_selector_write);
    drop(lost);
    written_handled(&mut router, &mut live).await;
    assert_eq!(router.autotune.targets[PA].writing_now(), Some(false));
    transport(&mut router, true);
    let chain = live.next("the start's chain", is_selector_write);
    assert_eq!(chain.commands, selector_commands(&[("p6", 0.0)]));
    let echoes = live.answer_quiet(chain);
    written_handled(&mut router, &mut live).await;
    router.markers_pending = Some(Vec::new());
    for (id, value) in &echoes {
        deliver_macro(&mut router, id, *value);
    }
    assert!(router.autotune.targets[PA].writing.is_empty());
    assert!(router.autotune.targets[STREAM].writing.is_empty());
    router.markers_pending = None;
    transport(&mut router, false);
    let dry = live.next("the second stop's DRY", is_selector_write);
    assert_eq!(dry.commands, selector_commands(&[("p6", 127.0)]));
    pass(&mut router, &mut live, SPREAD).await;
    assert_eq!(live.written().last(), Some(&batch(&[("p7", 0.0)])));
}

#[tokio::test(start_paused = true)]
async fn a_stop_writes_its_dry_one_selector_per_40_ms_in_lives_order() {
    // #140 (dasio's measurement of 2026-11-09): the stop's batch of every
    // selector made band Live's audio callback take 5.8-11.5 ms, a click.
    // One selector per batch, 40 ms apart, in Live's order.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    transport(&mut router, false);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0)])], "the first");
    // The next one is still on its way: it counts as written, so the
    // answer's decision writes nothing more.
    assert_eq!(router.autotune.targets[STREAM].writing_now(), Some(false));
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Stop), off(SwitchWhy::Stop)]
    );
    pass(
        &mut router,
        &mut live,
        SPREAD - std::time::Duration::from_millis(1),
    )
    .await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new(), "not yet");
    pass(&mut router, &mut live, std::time::Duration::from_millis(1)).await;
    assert_eq!(live.written(), vec![batch(&[("p7", 0.0)])], "40 ms later");
    pass(&mut router, &mut live, SPREAD * 5).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new(), "no more");
    assert!(router.autotune.targets[PA].writing.is_empty());
    assert!(router.autotune.targets[STREAM].writing.is_empty());
    let got = steps(&rx);
    assert_eq!(
        of(&got, "write"),
        vec![
            json!({"step": "write", "instance": "band", "cause": "transport", "on": [],
                   "off": ["Vox 2"]}),
            json!({"step": "write", "instance": "band", "cause": "transport", "on": [],
                   "off": ["#8"]}),
        ]
    );
    assert_eq!(
        of(&got, "spread"),
        vec![json!({"step": "spread", "instance": "band", "targets": ["#8"], "ms": 40})]
    );
    // A lost transport is a stop too.
    let lost = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = tuning(lost.path()).await;
    router.transport_failed(TRANSPORT, "not observable");
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0)])]);
    pass(&mut router, &mut live, SPREAD).await;
    assert_eq!(live.written(), vec![batch(&[("p7", 0.0)])]);
}

#[tokio::test(start_paused = true)]
async fn a_start_within_the_stops_spread_drops_its_rest_and_decides_afresh() {
    // The stream selector's DRY is still to come when Live plays again: it
    // is never written, and the start writes the chain only where the stop
    // took it (one batch).
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    transport(&mut router, false);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0)])]);
    pass(&mut router, &mut live, SPREAD / 2).await;
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0)])], "one batch");
    pass(&mut router, &mut live, SPREAD * 5).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(decisions(&mut router), [on(), on()]);
    assert!(router.autotune.targets[STREAM].writing.is_empty());
    assert_eq!(
        of(&steps(&rx), "unspread"),
        vec![
            json!({"step": "unspread", "instance": "band", "targets": ["#8"],
                    "why": "start"})
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_hold_within_the_stops_spread_writes_its_rest_dry_at_once() {
    // A hold's DRY stays immediate: a track-list change within the spread
    // writes the selectors still to come in its own batch, through their
    // last refs, and nothing more comes after it.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    transport(&mut router, false);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0)])]);
    router.autotune_tracks("band");
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p7", 0.0)])], "at once");
    pass(&mut router, &mut live, SPREAD * 5).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    let got = steps(&rx);
    assert_eq!(
        of(&got, "unspread"),
        vec![
            json!({"step": "unspread", "instance": "band", "targets": ["#8"],
                    "why": "hold"})
        ]
    );
    assert_eq!(
        of(&got, "write").last(),
        Some(
            &json!({"step": "write", "instance": "band", "cause": "hold", "on": [],
                     "off": ["#8"]})
        )
    );
}

#[tokio::test(start_paused = true)]
async fn a_layout_without_a_target_within_the_spread_writes_it_dry_at_once() {
    // A target let go while its spread DRY is still to come is written DRY
    // first, as any target let go (PR #147 review I3), never left on.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = tuning(dir.path()).await;
    transport(&mut router, false);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0)])]);
    router.handle(layout(2, vec![pa()]));
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p7", 0.0)])], "at once");
    pass(&mut router, &mut live, SPREAD * 5).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
}

#[tokio::test(start_paused = true)]
async fn a_spread_write_live_already_shows_is_dropped_at_its_turn() {
    // Another writer (or Live) puts the stream selector on DRY before its
    // turn: the write is owed no more, so its turn sends nothing.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    transport(&mut router, false);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0)])]);
    by_hand(&mut router, &mut live, "p7", 0.0);
    assert!(router.autotune.targets[STREAM].writing.is_empty());
    pass(&mut router, &mut live, SPREAD).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert!(router.autotune.spreads.is_empty());
    assert_eq!(
        of(&steps(&rx), "unspread"),
        vec![
            json!({"step": "unspread", "instance": "band", "targets": ["#8"],
                    "why": "owed"})
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_spread_turn_that_meets_the_markers_unsettled_holds_the_set_dry_at_once() {
    // A composition waits when the stream selector's turn comes: the set is
    // held, and the hold's own batch writes it DRY (never by the spread).
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    transport(&mut router, false);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0)])]);
    router.markers_pending = Some(Vec::new());
    pass(&mut router, &mut live, SPREAD).await;
    assert_eq!(live.written(), vec![batch(&[("p7", 0.0)])]);
    assert!(router.autotune.held.contains("band"));
    assert!(router.autotune.spreads.is_empty());
    let got = steps(&rx);
    assert_eq!(
        of(&got, "write").last(),
        Some(
            &json!({"step": "write", "instance": "band", "cause": "hold", "on": [],
                     "off": ["#8"]})
        )
    );
    assert_eq!(
        of(&got, "unspread"),
        vec![
            json!({"step": "unspread", "instance": "band", "targets": ["#8"],
                    "why": "hold"})
        ]
    );
}

#[tokio::test(start_paused = true)]
async fn a_stop_start_stop_spreads_again_and_the_first_spreads_turn_does_nothing() {
    // The first spread's timer still fires after a start dropped it: its
    // turn is stale (its generation), and the second stop's own turn comes
    // 40 ms after that stop, never sooner.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    let ms = std::time::Duration::from_millis;
    transport(&mut router, false);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0)])]);
    // The stream's DRY is on its way, unanswered: the status says so.
    let writing: Vec<bool> = status(&mut router)
        .autotune
        .targets
        .iter()
        .map(|t| t.writing)
        .collect();
    assert_eq!(writing, [false, true]);
    pass(&mut router, &mut live, ms(20)).await;
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0)])]);
    pass(&mut router, &mut live, ms(10)).await;
    transport(&mut router, false);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0)])]);
    // The first spread's 40 ms (10 ms after the second stop): nothing.
    pass(&mut router, &mut live, ms(10)).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    pass(&mut router, &mut live, ms(29)).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new(), "39 ms");
    pass(&mut router, &mut live, ms(1)).await;
    assert_eq!(live.written(), vec![batch(&[("p7", 0.0)])], "40 ms");
    pass(&mut router, &mut live, SPREAD * 5).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Stop), off(SwitchWhy::Stop)]
    );
    assert_eq!(of(&steps(&rx), "spread").len(), 2);
}

#[tokio::test(start_paused = true)]
async fn a_stop_with_one_selector_to_write_writes_it_and_spreads_nothing() {
    // Another writer took the stream to DRY while it tuned (left so): the
    // stop has only the PA's DRY to write, at once, and queues nothing.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    by_hand(&mut router, &mut live, "p7", 0.0);
    live.settle(&mut router).await;
    let _ = live.written();
    let _ = steps(&rx);
    transport(&mut router, false);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0)])]);
    assert!(router.autotune.spreads.is_empty());
    pass(&mut router, &mut live, SPREAD * 3).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(of(&steps(&rx), "spread"), Vec::<Value>::new());
}
