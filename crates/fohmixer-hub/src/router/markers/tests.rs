use serde_json::json;

use super::*;

fn key(watch: &Watch) -> String {
    let (instance, target, prop) = watch.target();
    format!("{instance}|{target}|{prop}")
}

/// The router's part: every `Sub` subscribed under its key; every `Unsub`'s
/// key, every read and the other actions returned.
#[derive(Debug, Default)]
struct Applied {
    subs: Vec<Watch>,
    unsubs: Vec<String>,
    reads: Vec<(String, u64, Step, Vec<Value>)>,
    rest: Vec<Action>,
}

fn apply(keeper: &mut Keeper, actions: Vec<Action>) -> Applied {
    let mut out = Applied::default();
    for action in actions {
        match action {
            Action::Sub(watch) => {
                keeper.subscribed(watch.clone(), key(&watch));
                out.subs.push(watch);
            }
            Action::Unsub { key } => out.unsubs.push(key),
            Action::Read {
                instance,
                seq,
                step,
                commands,
            } => out.reads.push((instance, seq, step, commands)),
            other => out.rest.push(other),
        }
    }
    out
}

fn device(class: &str, name: &str) -> Value {
    json!({"$ref": "x", "class": class, "name": name, "path": "p"})
}

/// A device of Live's own (class `Device`) with its own `$ref`.
fn device_ref(reference: &str, name: &str) -> Value {
    json!({"$ref": reference, "class": "Device", "name": name, "path": "p"})
}

fn ok(data: Value) -> Value {
    json!({"ok": true, "data": data})
}

fn tracks_key(instance: &str) -> String {
    key(&Watch::Tracks(instance.into()))
}

fn returns_key(instance: &str) -> String {
    key(&Watch::Returns(instance.into()))
}

/// A keeper following `band`, its lists subscribed.
fn keeper() -> Keeper {
    let mut k = Keeper::default();
    let actions = k.set_instances(["band".to_string()]);
    let applied = apply(&mut k, actions);
    assert_eq!(
        applied.subs,
        vec![Watch::Tracks("band".into()), Watch::Returns("band".into())]
    );
    k
}

fn list(n: usize) -> Value {
    Value::Array((0..n).map(|i| json!({"$ref": format!("t{i}")})).collect())
}

fn found(index: u32, kind: TrackKind, name: &str, tuners: u32) -> Found {
    Found {
        instance: "band".into(),
        kind,
        index,
        name: name.into(),
        tuners,
    }
}

#[test]
fn paths_and_watch_targets() {
    assert_eq!(track_path(TrackKind::Track, 3), "live_set tracks 3");
    assert_eq!(track_path(TrackKind::Return, 0), "live_set return_tracks 0");
    assert_eq!(
        device_path(TrackKind::Track, 3, 2),
        "live_set tracks 3 devices 2"
    );
    assert_eq!(
        Watch::Tracks("band".into()).target(),
        ("band", "live_set".to_string(), TRACKS)
    );
    assert_eq!(
        Watch::Returns("band".into()).target(),
        ("band", "live_set".to_string(), RETURNS)
    );
    assert_eq!(
        Watch::Devices {
            instance: "band".into(),
            kind: TrackKind::Return,
            index: 1
        }
        .target(),
        ("band", "live_set return_tracks 1".to_string(), DEVICES)
    );
    assert_eq!(
        Watch::Name {
            instance: "master".into(),
            kind: TrackKind::Track,
            index: 4,
            device: 0
        }
        .target(),
        ("master", "live_set tracks 4 devices 0".to_string(), NAME)
    );
    assert_eq!(len_of(Some(&list(3))), 3);
    assert_eq!(len_of(Some(&json!("not a list"))), 0);
    assert_eq!(len_of(None), 0);
}

#[test]
fn a_list_value_reads_every_tracks_devices_and_the_same_value_reads_nothing() {
    let mut k = keeper();
    let actions = k.value(&tracks_key("band"), Some(&list(2)));
    let applied = apply(&mut k, actions);
    assert_eq!(applied.reads.len(), 1);
    let (instance, _, step, commands) = &applied.reads[0];
    assert_eq!(instance, "band");
    assert_eq!(*step, Step::Devices);
    assert_eq!(
        commands,
        &vec![
            json!({"target": "live_set tracks 0", "name": "get_prop", "args": {"prop": "devices"}}),
            json!({"target": "live_set tracks 1", "name": "get_prop", "args": {"prop": "devices"}}),
        ]
    );
    assert_eq!(k.value(&tracks_key("band"), Some(&list(2))), vec![]);
    // The returns too, after the tracks.
    let actions = k.value(&returns_key("band"), Some(&list(1)));
    let applied = apply(&mut k, actions);
    assert_eq!(applied.reads[0].3.len(), 3);
    assert_eq!(
        applied.reads[0].3[2]["target"],
        json!("live_set return_tracks 0")
    );
    // An error value, an unknown key: nothing.
    assert_eq!(k.value(&tracks_key("band"), None), vec![]);
    assert_eq!(k.value("nobody", Some(&list(1))), vec![]);
}

/// Reads `band` with two tracks and one return through both batches: track
/// 0 holds a marker Tuner and a rack, track 1 two markers and a plain
/// Tuner, the return a Utility.
fn read_band(k: &mut Keeper) -> Applied {
    let actions = k.value(&tracks_key("band"), Some(&list(2)));
    apply(k, actions);
    let actions = k.value(&returns_key("band"), Some(&list(1)));
    let mut applied = apply(k, actions);
    let (_, seq, _, _) = applied.reads.pop().expect("a read");
    let devices = vec![
        ok(json!([
            device("Device", r#""Vox 1" +G:A"#),
            device("RackDevice", "Rack")
        ])),
        ok(json!([
            device("Device", r#""Vox 2" +G:A"#),
            device("Device", "Tuner"),
            device("Device", "+PIN")
        ])),
        ok(json!([device("Device", "Gain")])),
    ];
    let actions = k.read_done("band", seq, Step::Devices, &Ok(devices));
    let mut applied = apply(k, actions);
    let (_, second, step, commands) = applied.reads.pop().expect("the classes read");
    assert_eq!(second, seq);
    assert_eq!(step, Step::Classes);
    let targets: Vec<&str> = commands
        .iter()
        .map(|c| c["target"].as_str().unwrap())
        .collect();
    assert_eq!(
        targets,
        [
            "live_set tracks 0 devices 0",
            "live_set tracks 1 devices 0",
            "live_set tracks 1 devices 1",
            "live_set tracks 1 devices 2",
            "live_set return_tracks 0 devices 0",
        ]
    );
    assert_eq!(commands[0]["args"], json!({"prop": "class_name"}));
    let classes = vec![
        ok(json!("Tuner")),
        ok(json!("Tuner")),
        ok(json!("Tuner")),
        ok(json!("Tuner")),
        ok(json!("StereoGain")),
    ];
    let actions = k.read_done("band", seq, Step::Classes, &Ok(classes));
    apply(k, actions)
}

#[test]
fn a_read_finds_the_markers_and_listens_to_every_tracks_devices_and_tuners_name() {
    let mut k = keeper();
    let applied = read_band(&mut k);
    let expected = vec![
        found(0, TrackKind::Track, r#""Vox 1" +G:A"#, 1),
        found(1, TrackKind::Track, r#""Vox 2" +G:A"#, 2),
    ];
    assert_eq!(applied.rest, vec![Action::Found(expected.clone())]);
    assert_eq!(k.found(), expected.as_slice());
    assert_eq!(applied.unsubs, Vec::<String>::new());
    let devices = |kind, index| Watch::Devices {
        instance: "band".into(),
        kind,
        index,
    };
    let name = |index, device| Watch::Name {
        instance: "band".into(),
        kind: TrackKind::Track,
        index,
        device,
    };
    assert_eq!(
        applied.subs,
        vec![
            devices(TrackKind::Track, 0),
            devices(TrackKind::Track, 1),
            devices(TrackKind::Return, 0),
            name(0, 0),
            name(1, 0),
            name(1, 1),
            name(1, 2),
        ]
    );
}

#[test]
fn fresh_subscriptions_values_start_nothing_and_a_change_reads_again() {
    let mut k = keeper();
    read_band(&mut k);
    let devices = key(&Watch::Devices {
        instance: "band".into(),
        kind: TrackKind::Track,
        index: 0,
    });
    // The value the read saw: nothing.
    let seen = json!([
        device("Device", r#""Vox 1" +G:A"#),
        device("RackDevice", "Rack")
    ]);
    assert_eq!(k.value(&devices, Some(&seen)), vec![]);
    // A Tuner added: a read.
    let more = json!([
        device("Device", r#""Vox 1" +G:A"#),
        device("RackDevice", "Rack"),
        device("Device", "Tuner")
    ]);
    let actions = k.value(&devices, Some(&more));
    assert!(matches!(
        actions.as_slice(),
        [Action::Read {
            step: Step::Devices,
            ..
        }]
    ));
}

#[test]
fn a_tuners_rename_changes_the_markers_without_a_read() {
    let mut k = keeper();
    read_band(&mut k);
    let name = |index, device| {
        key(&Watch::Name {
            instance: "band".into(),
            kind: TrackKind::Track,
            index,
            device,
        })
    };
    // The same name: nothing.
    assert_eq!(
        k.value(&name(0, 0), Some(&json!(r#""Vox 1" +G:A"#))),
        vec![]
    );
    // Renamed: the markers change, no read.
    let actions = k.value(&name(0, 0), Some(&json!(r#""Vox 9" +G:B"#)));
    assert_eq!(
        actions,
        vec![Action::Found(vec![
            found(0, TrackKind::Track, r#""Vox 9" +G:B"#, 1),
            found(1, TrackKind::Track, r#""Vox 2" +G:A"#, 2),
        ])]
    );
    // The plain Tuner of track 1 becomes a third marker there.
    let actions = k.value(&name(1, 1), Some(&json!(r#""Vox 3""#)));
    assert_eq!(
        actions,
        vec![Action::Found(vec![
            found(0, TrackKind::Track, r#""Vox 9" +G:B"#, 1),
            found(1, TrackKind::Track, r#""Vox 2" +G:A"#, 3),
        ])]
    );
    // A marker renamed to no marker: track 0 has none any more.
    let actions = k.value(&name(0, 0), Some(&json!("Tuner")));
    assert_eq!(
        actions,
        vec![Action::Found(vec![found(
            1,
            TrackKind::Track,
            r#""Vox 2" +G:A"#,
            3
        )])]
    );
}

#[test]
fn a_second_read_keeps_the_devices_watches_and_subscribes_the_names_afresh() {
    let mut k = keeper();
    let first = read_band(&mut k);
    let actions = k.value(&tracks_key("band"), Some(&list(1)));
    let applied = apply(&mut k, actions);
    let (_, seq, _, commands) = applied.reads[0].clone();
    assert_eq!(commands.len(), 2, "one track and the return");
    let actions = k.read_done(
        "band",
        seq,
        Step::Devices,
        &Ok(vec![ok(json!([])), ok(json!([]))]),
    );
    let applied = apply(&mut k, actions);
    // No candidate: no second batch. The devices of the track and the
    // return still there stay; the second track's and every name go.
    assert_eq!(applied.reads.len(), 0);
    let kept = [
        Watch::Devices {
            instance: "band".into(),
            kind: TrackKind::Track,
            index: 0,
        },
        Watch::Devices {
            instance: "band".into(),
            kind: TrackKind::Return,
            index: 0,
        },
    ];
    let gone: Vec<String> = first
        .subs
        .iter()
        .filter(|w| !kept.contains(w))
        .map(key)
        .collect();
    assert_eq!(gone.len(), 5, "{gone:?}");
    assert_eq!(applied.unsubs, gone);
    assert_eq!(applied.subs, Vec::<Watch>::new());
    assert_eq!(applied.rest, vec![Action::Found(Vec::new())]);
    assert_eq!(k.found(), &[] as &[Found]);
    // A third read with a Tuner: its name is subscribed again each time,
    // the devices stay.
    let actions = k.value(&tracks_key("band"), Some(&list(2)));
    let applied = apply(&mut k, actions);
    let (_, seq, _, _) = applied.reads[0].clone();
    let devices = vec![
        ok(json!([device("Device", r#""Vox 1" +G:A"#)])),
        ok(json!([])),
        ok(json!([])),
    ];
    let actions = k.read_done("band", seq, Step::Devices, &Ok(devices));
    let applied = apply(&mut k, actions);
    let actions = k.read_done("band", seq, Step::Classes, &Ok(vec![ok(json!("Tuner"))]));
    let applied_classes = apply(&mut k, actions);
    assert_eq!(applied.subs, Vec::<Watch>::new());
    let name = Watch::Name {
        instance: "band".into(),
        kind: TrackKind::Track,
        index: 0,
        device: 0,
    };
    assert_eq!(
        applied_classes.subs,
        vec![
            Watch::Devices {
                instance: "band".into(),
                kind: TrackKind::Track,
                index: 1,
            },
            name.clone(),
        ]
    );
    assert_eq!(applied_classes.unsubs, Vec::<String>::new());
    let actions = k.value(&tracks_key("band"), Some(&list(3)));
    let applied = apply(&mut k, actions);
    let (_, seq, _, _) = applied.reads[0].clone();
    let devices = vec![
        ok(json!([device("Device", r#""Vox 1" +G:A"#)])),
        ok(json!([])),
        ok(json!([])),
        ok(json!([])),
    ];
    let actions = k.read_done("band", seq, Step::Devices, &Ok(devices));
    apply(&mut k, actions);
    let actions = k.read_done("band", seq, Step::Classes, &Ok(vec![ok(json!("Tuner"))]));
    let applied = apply(&mut k, actions);
    assert_eq!(applied.unsubs, vec![key(&name)]);
    assert_eq!(
        applied.subs,
        vec![
            Watch::Devices {
                instance: "band".into(),
                kind: TrackKind::Track,
                index: 2,
            },
            name,
        ]
    );
}

#[test]
fn a_newer_read_voids_an_older_answer() {
    let mut k = keeper();
    let actions = k.value(&tracks_key("band"), Some(&list(1)));
    let old = apply(&mut k, actions).reads[0].1;
    let actions = k.value(&tracks_key("band"), Some(&list(2)));
    let new = apply(&mut k, actions).reads[0].1;
    assert!(new > old);
    let answer = Ok(vec![ok(json!([device("Device", "x")]))]);
    assert_eq!(k.read_done("band", old, Step::Devices, &answer), vec![]);
    assert_eq!(k.read_done("nobody", new, Step::Devices, &answer), vec![]);
    assert_eq!(k.retry("nobody"), vec![]);
}

#[test]
fn a_failed_read_is_tried_again_three_times_in_a_row() {
    let mut k = keeper();
    let actions = k.value(&tracks_key("band"), Some(&list(1)));
    let mut seq = apply(&mut k, actions).reads[0].1;
    let failed: Result<Vec<Value>, String> = Err("timeout".into());
    for _ in 0..MAX_RETRIES {
        assert_eq!(
            k.read_done("band", seq, Step::Devices, &failed),
            vec![Action::Retry {
                instance: "band".into()
            }]
        );
        let actions = k.retry("band");
        seq = apply(&mut k, actions).reads[0].1;
    }
    assert_eq!(k.read_done("band", seq, Step::Devices, &failed), vec![]);
    // A success resets the count.
    let actions = k.retry("band");
    seq = apply(&mut k, actions).reads[0].1;
    let actions = k.read_done("band", seq, Step::Devices, &Ok(vec![ok(json!([]))]));
    apply(&mut k, actions);
    let actions = k.retry("band");
    seq = apply(&mut k, actions).reads[0].1;
    assert_eq!(
        k.read_done("band", seq, Step::Devices, &failed),
        vec![Action::Retry {
            instance: "band".into()
        }]
    );
}

#[test]
fn a_track_that_failed_to_answer_holds_no_tuner() {
    let mut k = keeper();
    let actions = k.value(&tracks_key("band"), Some(&list(2)));
    let seq = apply(&mut k, actions).reads[0].1;
    let devices = vec![
        json!({"ok": false, "error": "gone"}),
        ok(json!([device("Device", r#""B" +G:A"#)])),
    ];
    let actions = k.read_done("band", seq, Step::Devices, &Ok(devices));
    let applied = apply(&mut k, actions);
    let commands = &applied.reads[0].3;
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0]["target"], json!("live_set tracks 1 devices 0"));
    // A class that did not answer is no Tuner.
    let actions = k.read_done(
        "band",
        seq,
        Step::Classes,
        &Ok(vec![json!({"ok": false, "error": "gone"})]),
    );
    let applied = apply(&mut k, actions);
    assert_eq!(applied.rest, vec![]);
    assert_eq!(k.found(), &[] as &[Found]);
}

#[test]
fn the_markers_of_an_instance_are_counted_per_track_in_order() {
    let tuners: BTreeMap<(TrackAt, u32), String> = BTreeMap::from([
        (((TrackKind::Return, 0), 0), r#""R""#.to_string()),
        (((TrackKind::Track, 2), 3), "+PIN".to_string()),
        (((TrackKind::Track, 2), 1), r#""A""#.to_string()),
        (((TrackKind::Track, 0), 0), "Tuner".to_string()),
    ]);
    assert_eq!(
        found_of("band", &tuners),
        vec![
            found(2, TrackKind::Track, r#""A""#, 2),
            found(0, TrackKind::Return, r#""R""#, 1),
        ]
    );
    assert_eq!(found_of("band", &BTreeMap::new()), vec![]);
}

#[test]
fn a_second_batch_that_keeps_failing_is_tried_again_three_times_in_a_row() {
    let mut k = keeper();
    let actions = k.value(&tracks_key("band"), Some(&list(1)));
    let mut seq = apply(&mut k, actions).reads[0].1;
    let devices = Ok(vec![ok(json!([device("Device", "Tuner")]))]);
    let failed: Result<Vec<Value>, String> = Err("timeout".into());
    for _ in 0..MAX_RETRIES {
        let actions = k.read_done("band", seq, Step::Devices, &devices);
        assert!(matches!(
            actions.as_slice(),
            [Action::Read {
                step: Step::Classes,
                ..
            }]
        ));
        assert_eq!(
            k.read_done("band", seq, Step::Classes, &failed),
            vec![Action::Retry {
                instance: "band".into()
            }]
        );
        let actions = k.retry("band");
        seq = apply(&mut k, actions).reads[0].1;
    }
    k.read_done("band", seq, Step::Devices, &devices);
    assert_eq!(k.read_done("band", seq, Step::Classes, &failed), vec![]);
}

#[test]
fn only_the_last_read_of_each_instance_is_sent() {
    let read = |instance: &str, seq: u64| Action::Read {
        instance: instance.into(),
        seq,
        step: Step::Devices,
        commands: Vec::new(),
    };
    let sub = Action::Sub(Watch::Tracks("band".into()));
    let actions = vec![
        read("band", 1),
        sub.clone(),
        read("master", 2),
        read("band", 3),
        Action::Found(Vec::new()),
    ];
    assert_eq!(
        latest_reads(actions),
        vec![
            sub,
            read("master", 2),
            read("band", 3),
            Action::Found(Vec::new())
        ]
    );
}

#[test]
fn a_list_change_that_binds_every_tracks_devices_again_asks_for_one_read() {
    let mut k = keeper();
    read_band(&mut k);
    // A track inserted: the table binds each track's `devices` to the
    // object now at its index, and every value differs from the read's.
    let value = json!([device("Device", "new")]);
    let keys: Vec<String> = [(TrackKind::Track, 0), (TrackKind::Track, 1)]
        .into_iter()
        .map(|(kind, index)| {
            key(&Watch::Devices {
                instance: "band".into(),
                kind,
                index,
            })
        })
        .collect();
    let reads = k.values(keys.iter().map(|key| (key.as_str(), Some(&value))));
    assert_eq!(reads.len(), 1, "one read for the flush: {reads:?}");
    let Action::Read { seq, .. } = &reads[0] else {
        panic!("a read: {reads:?}");
    };
    // The one sent is the newest: its answer counts.
    let answer = Ok(vec![ok(json!([])), ok(json!([])), ok(json!([]))]);
    assert_ne!(k.read_done("band", *seq, Step::Devices, &answer), vec![]);
}

fn plan(kind: TrackKind, track: &str, marker: &str) -> Planned {
    Planned {
        instance: "band".into(),
        kind,
        track: track.into(),
        marker: marker.into(),
    }
}

#[test]
fn the_migration_finds_each_planned_track_and_renames_only_a_ready_one() {
    // #68 PR C: the planned tracks by name in the keeper's own lists.
    let mut k = keeper();
    let tracks = json!([
        {"$ref": "t0", "name": "Vox 1 #"},
        {"$ref": "t1", "name": "Vox 2 #"},
        {"$ref": "t2", "name": "Dup #"},
        {"$ref": "t3", "name": "Dup #"},
        {"$ref": "t4", "name": "Two #"},
    ]);
    let actions = k.value(&tracks_key("band"), Some(&tracks));
    apply(&mut k, actions);
    let actions = k.value(
        &returns_key("band"),
        Some(&json!([{"$ref": "r0", "name": "A-Hall #"}])),
    );
    let mut applied = apply(&mut k, actions);
    let (_, seq, _, _) = applied.reads.pop().expect("a read");
    let devices = vec![
        ok(json!([device_ref("d0", "Tuner")])),
        ok(json!([
            device("Device", r#""Vox 2" +G:A"#),
            device("Device", "Tuner")
        ])),
        ok(json!([device("Device", "Tuner")])),
        ok(json!([])),
        ok(json!([
            device("Device", "Tuner"),
            device("Device", "Tuner 2")
        ])),
        ok(json!([device_ref("r0d0", "Tuner")])),
    ];
    let actions = k.read_done("band", seq, Step::Devices, &Ok(devices));
    apply(&mut k, actions);
    let classes = vec![ok(json!("Tuner")); 7];
    let actions = k.read_done("band", seq, Step::Classes, &Ok(classes));
    apply(&mut k, actions);
    let vox1 = r#""Vox 1" +G:A:1"#;
    let hall = r#""Hall" +G:A:2"#;
    let planned = vec![
        plan(TrackKind::Track, "Vox 1 #", vox1),
        plan(TrackKind::Track, "Vox 2 #", r#""Vox 2" +G:A"#),
        plan(TrackKind::Track, "Dup #", r#""Dup" +G:A"#),
        plan(TrackKind::Track, "Missing #", r#""Missing" +G:A"#),
        plan(TrackKind::Track, "Two #", r#""Two" +G:A"#),
        plan(TrackKind::Return, "A-Hall #", hall),
        plan(TrackKind::Return, "Vox 1 #", vox1),
        Planned {
            instance: "nobody".into(),
            kind: TrackKind::Track,
            track: "Vox 1 #".into(),
            marker: vox1.into(),
        },
    ];
    let seen = |k: &Keeper| -> Vec<(u32, Option<u32>, u32, u32, bool)> {
        k.migration_rows(&planned)
            .iter()
            .map(|r| (r.matches, r.index, r.plain, r.markers, r.done))
            .collect()
    };
    assert_eq!(
        seen(&k),
        vec![
            (1, Some(0), 1, 0, false),
            (1, Some(1), 1, 1, true),
            (2, None, 0, 0, false),
            (0, None, 0, 0, false),
            (1, Some(4), 2, 0, false),
            (1, Some(0), 1, 0, false),
            (0, None, 0, 0, false),
            (0, None, 0, 0, false),
        ]
    );
    let rows = k.migration_rows(&planned);
    assert_eq!(rows[0].track, "Vox 1 #");
    assert_eq!(rows[0].marker, vox1);
    assert_eq!(rows[5].kind, TrackKind::Return);
    // Each names its Tuner by the `$ref` the read saw.
    let rename = |target: &str, name: &str| Action::Rename {
        instance: "band".into(),
        target: json!({"$ref": target}),
        name: name.into(),
    };
    assert_eq!(
        k.renames(&planned),
        vec![rename("d0", vox1), rename("r0d0", hall)]
    );
    // The rename comes back through the Tuner's name watch: done, and no
    // second rename.
    let name = Watch::Name {
        instance: "band".into(),
        kind: TrackKind::Track,
        index: 0,
        device: 0,
    };
    k.value(&key(&name), Some(&json!(vox1)));
    assert_eq!(seen(&k)[0], (1, Some(0), 0, 1, true));
    assert_eq!(k.renames(&planned), vec![rename("r0d0", hall)]);
}

#[test]
fn a_rename_names_the_one_plain_tuner_after_another_device() {
    // The plain Tuner is the track's second device (after an EQ Eight): the
    // rename names that device.
    let mut k = keeper();
    let actions = k.value(
        &tracks_key("band"),
        Some(&json!([{"$ref": "t0", "name": "Bass #"}])),
    );
    apply(&mut k, actions);
    let actions = k.value(&returns_key("band"), Some(&json!([])));
    let mut applied = apply(&mut k, actions);
    let (_, seq, _, _) = applied.reads.pop().expect("a read");
    let devices = vec![ok(json!([
        device_ref("e0", "EQ Eight"),
        device_ref("d1", "Tuner")
    ]))];
    let actions = k.read_done("band", seq, Step::Devices, &Ok(devices));
    apply(&mut k, actions);
    let classes = vec![ok(json!("Eq8")), ok(json!("Tuner"))];
    let actions = k.read_done("band", seq, Step::Classes, &Ok(classes));
    apply(&mut k, actions);
    let planned = vec![plan(TrackKind::Track, "Bass #", r#""Bass" +G:A:1"#)];
    assert_eq!(
        k.renames(&planned),
        vec![Action::Rename {
            instance: "band".into(),
            target: json!({"$ref": "d1"}),
            name: r#""Bass" +G:A:1"#.into(),
        }]
    );
}

#[test]
fn the_lists_read_afresh_replace_the_kept_ones_and_a_change_reads_again() {
    let mut k = keeper();
    let named = |name: &str| json!([{"$ref": "t0", "name": name}]);
    let actions = k.value(&tracks_key("band"), Some(&named("Bass #")));
    apply(&mut k, actions);
    let actions = k.value(&returns_key("band"), Some(&json!([])));
    let mut applied = apply(&mut k, actions);
    let (_, seq, _, _) = applied.reads.pop().expect("a read");
    let actions = k.read_done(
        "band",
        seq,
        Step::Devices,
        &Ok(vec![ok(json!([device("Device", "Tuner")]))]),
    );
    apply(&mut k, actions);
    let actions = k.read_done("band", seq, Step::Classes, &Ok(vec![ok(json!("Tuner"))]));
    apply(&mut k, actions);
    assert!(!k.reading("band"));
    let planned = vec![plan(TrackKind::Track, "Bass #", r#""Bass" +G:A:1"#)];
    assert_eq!(k.renames(&planned).len(), 1);
    let lists = |tracks: Value, returns: Value| vec![ok(tracks), ok(returns)];
    // The same lists: nothing to do.
    assert_eq!(
        k.refresh("band", Some(lists(named("Bass #"), json!([])).as_slice())),
        vec![]
    );
    assert!(!k.reading("band"));
    // A failed read, a list that did not answer: the kept lists stay, the
    // instance counts as reading, and nothing is renamed until a read
    // answers again.
    assert_eq!(k.refresh("band", None), vec![]);
    assert!(k.reading("band"));
    assert_eq!(k.renames(&planned), vec![]);
    assert_eq!(k.migration_rows(&planned)[0].matches, 1);
    let error = vec![json!({"ok": false, "error": "x"}), ok(json!([]))];
    assert_eq!(
        k.refresh("band", Some(lists(named("Bass #"), json!([])).as_slice())),
        vec![]
    );
    assert!(!k.reading("band"));
    assert_eq!(k.refresh("band", Some(error.as_slice())), vec![]);
    assert!(k.reading("band"));
    assert_eq!(
        k.refresh("band", Some([ok(named("Bass #"))].as_slice())),
        vec![]
    );
    assert!(k.reading("band"));
    assert_eq!(
        k.refresh("band", Some(lists(named("Bass #"), json!([])).as_slice())),
        vec![]
    );
    assert_eq!(k.renames(&planned).len(), 1);
    // An unknown instance: nothing.
    assert_eq!(
        k.refresh("nobody", Some(lists(named("Kick #"), json!([])).as_slice())),
        vec![]
    );
    assert!(!k.reading("nobody"));
    // A rename Live told no listener about: kept, and a read starts.
    let actions = k.refresh("band", Some(lists(named("Kick #"), json!([])).as_slice()));
    assert!(
        matches!(actions.as_slice(), [Action::Read { .. }]),
        "{actions:?}"
    );
    assert_eq!(k.migration_rows(&planned)[0].matches, 0);
    // While that read runs, no rename goes out.
    assert!(k.reading("band"));
    let planned = vec![plan(TrackKind::Track, "Kick #", r#""Kick" +G:A:1"#)];
    assert_eq!(k.migration_rows(&planned)[0].plain, 1);
    assert_eq!(k.renames(&planned), vec![]);
    // A changed list of returns reads again too.
    let actions = k.refresh(
        "band",
        Some(lists(named("Kick #"), json!([{"$ref": "r0", "name": "A-Hall #"}])).as_slice()),
    );
    assert!(
        matches!(actions.as_slice(), [Action::Read { .. }]),
        "{actions:?}"
    );
}
