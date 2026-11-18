//! The holds and their releases (the markers, a reconnect, a track-list
//! change), and the targets let go or walked afresh.

use super::*;

#[tokio::test]
async fn a_hold_writes_dry_first_through_the_last_ref() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    // A track-list change while both tune: DRY by `$ref` first, then held.
    router.autotune_tracks("band");
    let dry = live.next("the hold's DRY", is_selector_write);
    assert_eq!(
        dry.commands,
        vec![
            json!({"target": {"$ref": "p6", "class": "DeviceParameter"}, "name": "set_prop",
                   "args": {"prop": "value", "value": 127.0}}),
            json!({"target": {"$ref": "p7", "class": "DeviceParameter"}, "name": "set_prop",
                   "args": {"prop": "value", "value": 0.0}}),
        ]
    );
    let got = steps(&rx);
    let order: Vec<&str> = got.iter().map(|r| r["step"].as_str().unwrap()).collect();
    let write_at = order.iter().position(|s| *s == "write").unwrap();
    let held_at = order.iter().position(|s| *s == "held").unwrap();
    assert!(write_at < held_at, "{order:?}");
    assert_eq!(
        of(&got, "write"),
        vec![
            json!({"step": "write", "instance": "band", "cause": "hold", "on": [],
                    "off": ["Vox 2", "#8"]})
        ]
    );
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Held), off(SwitchWhy::Held)]
    );
    // A stale ref answers StaleRef: nothing more is written (its target is
    // walked again).
    let stale = json!({"ok": false, "error": "stale ref", "errorType": "StaleRef"});
    dry.answer(vec![ok(Value::Null), stale]);
    let msg = next_of(&mut live.own, "the hold's answer", |m| {
        matches!(m, RouterMsg::AutotuneWritten { .. })
    })
    .await;
    router.handle(msg);
    // While held, a stop, a start and another writer write nothing, also
    // on a target its walk confirmed meanwhile (the set is still held).
    transport(&mut router, false);
    transport(&mut router, true);
    by_hand(&mut router, &mut live, "p6", 0.0);
    find(&mut router, PA, rack(6));
    assert!(router.autotune.targets[PA].sure);
    by_hand(&mut router, &mut live, "p6", 127.0);
    by_hand(&mut router, &mut live, "p6", 0.0);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    let got = steps(&rx);
    assert_eq!(of(&got, "selector_writer"), Vec::<Value>::new());
    assert_eq!(
        of(&got, "failed"),
        vec![
            json!({"step": "failed", "instance": "band", "target": "live_set tracks 7",
                    "problem": "stale ref"})
        ]
    );
}

#[tokio::test]
async fn a_write_that_meets_the_markers_unsettled_holds_the_set_dry_first() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    // A composition waits (the markers not settled): the stop's DRY goes
    // through the refs, and the set is held.
    router.markers_pending = Some(Vec::new());
    transport(&mut router, false);
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0), ("p7", 0.0)])]);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "held"),
        vec![json!({"step": "held", "instance": "band", "on": true, "why": "markers"})]
    );
    assert_eq!(of(&got, "write")[0]["cause"], "hold");
    // Settled again: the release walks every target again; each is DRY and
    // written nothing until its walk confirms it (PR #147 review I4).
    router.markers_pending = None;
    router.autotune_release("band", "settled");
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    // Its own reason, the set no longer held (PR #147 re-review R6).
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Unconfirmed), off(SwitchWhy::Unconfirmed)]
    );
    // The walks answer: each tunes again once its own walk confirmed it.
    find(&mut router, PA, rack(6));
    find(&mut router, STREAM, rack(7));
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        vec![batch(&[("p6", 0.0)]), batch(&[("p7", 127.0)])]
    );
    assert_eq!(decisions(&mut router), [on(), on()]);
}

#[tokio::test]
async fn a_reconnect_decides_again_and_switches_on_only_after_the_keys() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    // The script made its DRY writes; the hub forgot its refs and
    // decisions.
    assert_eq!(decisions(&mut router), [None, None]);
    live.params.insert("p6".into(), json!(127.0));
    live.params.insert("p7".into(), json!(0.0));
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Connected(crate::live::ConnectInfo {
            instance: "band".into(),
            set_name: "set".into(),
            script_version: "1.14.0".into(),
            live_version: "12".into(),
        }),
    });
    router.autotune_release("band", "settled");
    transport(&mut router, true);
    deliver(&mut router, TEMPO_KEY, 120.0);
    deliver(&mut router, TIME, 12.0);
    find(&mut router, PA, rack(6));
    find(&mut router, STREAM, rack(7));
    deliver_macro(&mut router, "p6", 127.0);
    deliver_macro(&mut router, "p7", 0.0);
    heard(&mut router, PA, (2.0 / 11.0, 0.05 / 28.0));
    heard(&mut router, STREAM, (2.0 / 11.0, 1.0 / 28.0));
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        Vec::<Vec<(String, f64)>>::new(),
        "no map yet"
    );
    assert_eq!(router.autotune.safe["band"].state, SafeState::Kept);
    let _ = steps(&rx);
    // The connect's map: the keys first, then the selectors.
    map_read(&mut router, 2, songs());
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Mismatch), off(SwitchWhy::Mismatch)]
    );
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0), ("p7", 127.0)])]);
    let got = steps(&rx);
    let order: Vec<&str> = got
        .iter()
        .map(|r| r["step"].as_str().unwrap())
        .filter(|s| ["key_write", "readback", "write"].contains(s))
        .collect();
    assert_eq!(order, ["key_write", "readback", "readback", "write"]);
}

#[tokio::test]
async fn a_layout_without_a_tuning_target_writes_it_dry_first_then_registers_without_it() {
    // PR #147 review I3: its last ref is the hub's only way back to DRY;
    // the script keeps its DRY write until that write answered.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    let seq = router.autotune.safe["band"].seq;
    router.handle(layout(2, vec![pa()]));
    let dry = live.next("the dropped target's DRY", is_selector_write);
    assert_eq!(dry.commands, selector_commands(&[("p7", 0.0)]));
    assert!(!router.autotune.targets.contains_key(STREAM));
    assert!(live.script.try_call().is_none(), "no registration yet");
    assert_eq!(router.autotune.safe["band"].seq, seq);
    assert_eq!(
        of(&steps(&rx), "write"),
        vec![
            json!({"step": "write", "instance": "band", "cause": "layout", "on": [],
                    "off": ["#8"]})
        ]
    );
    live.answer(&mut router, dry);
    written_handled(&mut router, &mut live).await;
    let registration = live.next("the registration", |c| {
        c.len() == 1 && c[0]["name"] == SAFE_WRITES
    });
    assert_eq!(
        registration.commands[0]["args"]["writes"],
        json!([{"target": {"$ref": "p6", "class": "DeviceParameter"}, "value": 127.0}])
    );
    assert_eq!(decisions(&mut router), [on()], "the PA still tunes");
}

#[tokio::test]
async fn a_tuning_target_found_missing_is_written_dry_first() {
    // PR #147 review I3: a definite answer that its rack is gone (renamed,
    // or a second one: `Twice`).
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = tuning(dir.path()).await;
    let walk = router.autotune.targets[STREAM].walk;
    router.autotune_found(STREAM, walk, Err("2 racks named Latencies".into()));
    let dry = live.next("its DRY", is_selector_write);
    assert_eq!(dry.commands, selector_commands(&[("p7", 0.0)]));
    assert!(router.autotune.targets[STREAM].found.is_none());
    assert!(live.script.try_call().is_none(), "no registration yet");
    live.answer(&mut router, dry);
    written_handled(&mut router, &mut live).await;
    let registration = live.next("the registration", |c| {
        c.len() == 1 && c[0]["name"] == SAFE_WRITES
    });
    assert_eq!(
        registration.commands[0]["args"]["writes"],
        json!([{"target": {"$ref": "p6", "class": "DeviceParameter"}, "value": 127.0}])
    );
}

#[tokio::test]
async fn a_layout_that_swaps_two_tracks_pairs_no_new_spec_with_an_old_selector() {
    // PR #147 review I4: a track-list change moved the stream track to
    // index 6 and the PA track to 7, so each kept key takes the other's
    // spec; the walks find each rack at its new index.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    router.autotune_tracks("band");
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0), ("p7", 0.0)])]);
    let seq = router.autotune.safe["band"].seq;
    let _ = steps(&rx);
    let stream_at_6 = target(6, "#7", (127, 0), Some("VOX1"));
    let pa_at_7 = target(7, "Vox 2", (0, 127), Some("VOX1"));
    router.handle(layout(2, vec![stream_at_6, pa_at_7]));
    live.settle(&mut router).await;
    assert_eq!(
        live.written(),
        Vec::<Vec<(String, f64)>>::new(),
        "nothing by the old refs"
    );
    assert_eq!(router.autotune.safe["band"].seq, seq, "nothing registered");
    assert_eq!(decisions(&mut router), [None, None]);
    // The walks: the stream rack (p7) at 6, the PA rack (p6) at 7.
    let at = |index: u32, rack_of: u32| Found {
        path: format!("live_set tracks {index} devices 0 parameters 1"),
        id: format!("p{rack_of}"),
        key: Ok(device(rack_of)),
    };
    find(&mut router, PA, at(6, 7));
    find(&mut router, STREAM, at(7, 6));
    deliver_macro(&mut router, "p7", 0.0);
    deliver_macro(&mut router, "p6", 127.0);
    live.settle(&mut router).await;
    // Each ref with its own spec's DRY value; both DRY until their keys.
    assert_eq!(
        router.autotune.safe["band"].list,
        vec![("p7".to_string(), 0), ("p6".to_string(), 127)]
    );
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert!(
        of(&steps(&rx), "select")
            .iter()
            .all(|r| r["on"] == json!(false))
    );
}

#[tokio::test]
async fn a_polarity_change_walks_before_any_write_or_registration() {
    // PR #147 review I4: the stream target's marker now says PA (on 0,
    // off 127) at the same track: its old ref's DRY value is the other
    // polarity's.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = tuning(dir.path()).await;
    let seq = router.autotune.safe["band"].seq;
    router.handle(layout(
        2,
        vec![pa(), target(7, "#8", (0, 127), Some("VOX1"))],
    ));
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new(), "no write");
    assert_eq!(router.autotune.safe["band"].seq, seq, "no registration");
    assert!(router.autotune.targets[STREAM].found.is_none());
    assert_eq!(decisions(&mut router)[0], on(), "the PA still tunes");
    // Its walk: the same rack; Live at 127, its DRY now.
    find(&mut router, STREAM, rack(7));
    deliver_macro(&mut router, "p7", 127.0);
    live.settle(&mut router).await;
    assert_eq!(
        router.autotune.safe["band"].list,
        vec![("p6".to_string(), 127), ("p7".to_string(), 127)]
    );
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(decisions(&mut router)[0], on());
}

#[tokio::test]
async fn the_markers_settling_decides_again_without_a_hold() {
    // PR #147 review N2: judged held while a composition waits, with
    // nothing to write, no hold opens; their settling decides again.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = ready(dir.path(), HubState::default()).await;
    aut_on(&mut router);
    router.markers_pending = Some(Vec::new());
    transport(&mut router, true);
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Held), off(SwitchWhy::Held)]
    );
    assert_eq!(of(&steps(&rx), "held"), Vec::<Value>::new(), "no hold");
    router.markers_pending = None;
    router.autotune_release("band", "settled");
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 0.0), ("p7", 127.0)])]);
    assert_eq!(decisions(&mut router), [on(), on()]);
}

#[tokio::test]
async fn a_second_track_list_change_within_a_hold_unconfirms_a_walks_answer_again() {
    // PR #147 re-review R1: a walk answered between two track-list changes
    // in one hold found the rack the first change put at its index; the
    // markers' layout then keeps that index's polarity.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, _rx) = tuning(dir.path()).await;
    let seq = router.autotune.safe["band"].seq;
    router.autotune_tracks("band");
    live.settle(&mut router).await;
    assert_eq!(live.written(), vec![batch(&[("p6", 127.0), ("p7", 0.0)])]);
    find(
        &mut router,
        PA,
        Found {
            id: "p8".into(),
            ..rack(6)
        },
    );
    find(&mut router, STREAM, rack(7));
    assert!(router.autotune.targets[PA].sure);
    // The second change, within the same hold.
    router.autotune_tracks("band");
    assert!(!router.autotune.targets[PA].sure);
    assert!(!router.autotune.targets[STREAM].sure);
    // The layout ends the hold: nothing by the walk's ref, nothing
    // registered with it, before the newest walks answer.
    router.handle(layout(
        2,
        vec![target(6, "Vox 5", (0, 127), Some("VOX1")), stream()],
    ));
    live.settle(&mut router).await;
    assert_eq!(live.written(), Vec::<Vec<(String, f64)>>::new());
    assert_eq!(router.autotune.safe["band"].seq, seq, "nothing registered");
    assert_eq!(
        decisions(&mut router),
        [off(SwitchWhy::Unconfirmed), off(SwitchWhy::Unconfirmed)]
    );
    // The newest walks answer: registered with their refs.
    find(
        &mut router,
        PA,
        Found {
            id: "p9".into(),
            ..rack(6)
        },
    );
    find(&mut router, STREAM, rack(7));
    live.settle(&mut router).await;
    assert_eq!(
        router.autotune.safe["band"].list,
        vec![("p9".to_string(), 127), ("p7".to_string(), 0)]
    );
    assert!(live.written().iter().flatten().all(|(id, _)| id != "p8"));
}

#[tokio::test]
async fn a_target_let_go_while_the_markers_are_unsettled_is_held_dry_first() {
    // PR #147 re-review R2: no hold open yet, a composition waiting.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut live, rx) = tuning(dir.path()).await;
    let seq = router.autotune.safe["band"].seq;
    router.markers_pending = Some(Vec::new());
    router.handle(layout(2, vec![pa()]));
    let dry = live.next("the hold's DRY", is_selector_write);
    assert_eq!(
        dry.commands,
        selector_commands(&[("p6", 127.0), ("p7", 0.0)])
    );
    let got = steps(&rx);
    assert_eq!(
        of(&got, "held"),
        vec![json!({"step": "held", "instance": "band", "on": true, "why": "markers"})]
    );
    assert_eq!(of(&got, "write")[0]["cause"], "hold");
    assert!(!router.autotune.targets.contains_key(STREAM));
    live.answer(&mut router, dry);
    live.settle(&mut router).await;
    assert_eq!(router.autotune.safe["band"].seq, seq, "held: nothing yet");
    // Settled: once the PA's walk confirmed it, registered without the
    // stream's selector.
    router.markers_pending = None;
    router.autotune_release("band", "settled");
    find(&mut router, PA, rack(6));
    live.settle(&mut router).await;
    assert_eq!(
        router.autotune.safe["band"].list,
        vec![("p6".to_string(), 127)]
    );
}
