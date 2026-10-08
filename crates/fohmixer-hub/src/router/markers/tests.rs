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
fn a_second_read_subscribes_the_per_track_watches_afresh() {
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
    // No candidate: no second batch; every old watch let go.
    assert_eq!(applied.reads.len(), 0);
    let old: Vec<String> = first.subs.iter().map(key).collect();
    assert_eq!(applied.unsubs, old);
    assert_eq!(
        applied.subs,
        vec![
            Watch::Devices {
                instance: "band".into(),
                kind: TrackKind::Track,
                index: 0
            },
            Watch::Devices {
                instance: "band".into(),
                kind: TrackKind::Return,
                index: 0
            },
        ]
    );
    assert_eq!(applied.rest, vec![Action::Found(Vec::new())]);
    assert_eq!(k.found(), &[] as &[Found]);
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
