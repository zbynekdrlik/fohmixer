use super::*;
use serde_json::json;

fn ok(data: Value) -> Value {
    json!({"ok": true, "data": data})
}

fn device(class: &str, path: &str, name: &str) -> Value {
    json!({"$ref": format!("ref {path}"), "path": path, "class": class, "name": name})
}

fn rack(path: &str, name: &str) -> Value {
    device("RackDevice", path, name)
}

/// A list answer of one chain.
fn chain(path: &str) -> Value {
    ok(json!([{"$ref": "c", "path": path, "class": "Chain", "name": "Chain 1"}]))
}

/// A rack's parameters as the script lists them: `Device On`, then
/// `Macro 1` with its `$ref`.
fn parameters(rack: &str) -> Value {
    ok(json!([
        {"$ref": "p1", "path": format!("{rack} parameters 0"), "class": "DeviceParameter", "name": "Device On"},
        {"$ref": "p2", "path": format!("{rack} parameters 1"), "class": "DeviceParameter", "name": "Macro 1"},
        {"$ref": "p3", "path": format!("{rack} parameters 2"), "class": "DeviceParameter", "name": "Macro 2"},
    ]))
}

/// The `(target, prop)` of each command of a read.
fn targets(step: &Step) -> Vec<(String, String)> {
    let Step::Read(commands) = step else {
        panic!("a read: {step:?}")
    };
    commands
        .iter()
        .map(|c| {
            assert_eq!(c["name"], "get_prop");
            (
                c["target"].as_str().unwrap().to_string(),
                c["args"]["prop"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn read(target: &str, prop: &str) -> Vec<(String, String)> {
    vec![(target.to_string(), prop.to_string())]
}

const TRACK: &str = "live_set tracks 6";

#[test]
fn a_top_level_latencies_rack_is_found_with_its_first_macro() {
    let (mut find, first) = Find::start(TRACK);
    assert_eq!(
        first,
        vec![json!({"target": TRACK, "name": "get_prop", "args": {"prop": "devices"}})]
    );
    let step = find.answer(&[ok(json!([
        {"$ref": "r1", "path": "live_set tracks 6 devices 0", "class": "RackDevice", "name": "Latencies"},
        device("Device", "live_set tracks 6 devices 1", "Tuner"),
    ]))]);
    assert_eq!(
        step,
        Step::Read(vec![get("live_set tracks 6 devices 0", "parameters")])
    );
    assert_eq!(
        find.answer(&[parameters("live_set tracks 6 devices 0")]),
        Step::Done(Ok(Found {
            path: "live_set tracks 6 devices 0 parameters 1".into(),
            id: "p2".into(),
        }))
    );
    assert_eq!((RACK, MACRO, PARAMETERS), ("Latencies", 1, "parameters"));
}

#[test]
fn a_latencies_rack_one_rack_down_is_found() {
    let (mut find, _) = Find::start("live_set tracks 7");
    // 1: the track holds a rack of another name: its chains.
    let step = find.answer(&[ok(json!([rack(
        "live_set tracks 7 devices 0",
        "Stream FX"
    )]))]);
    assert_eq!(
        targets(&step),
        read("live_set tracks 7 devices 0", "chains")
    );
    // 2: its chain's devices.
    let step = find.answer(&[chain("live_set tracks 7 devices 0 chains 0")]);
    assert_eq!(
        targets(&step),
        read("live_set tracks 7 devices 0 chains 0", "devices")
    );
    // 3: the Latencies rack inside: its parameters.
    let inner = "live_set tracks 7 devices 0 chains 0 devices 0";
    let step = find.answer(&[ok(json!([rack(inner, "Latencies")]))]);
    assert_eq!(targets(&step), read(inner, "parameters"));
    // 4: its first macro.
    assert_eq!(
        find.answer(&[parameters(inner)]),
        Step::Done(Ok(Found {
            path: format!("{inner} parameters 1"),
            id: "p2".into(),
        }))
    );
}

#[test]
fn no_rack_named_latencies_is_missing() {
    let (mut find, _) = Find::start(TRACK);
    let step = find.answer(&[ok(json!([
        device("PluginDevice", "live_set tracks 6 devices 0", "Pro-Q 4"),
        rack("live_set tracks 6 devices 1", "Vocal FX"),
    ]))]);
    assert_eq!(
        targets(&step),
        read("live_set tracks 6 devices 1", "chains")
    );
    let step = find.answer(&[ok(json!([
        {"path": "live_set tracks 6 devices 1 chains 0", "class": "Chain", "name": "Main"},
        {"path": "live_set tracks 6 devices 1 chains 1", "class": "Chain", "name": "Air"},
    ]))]);
    assert_eq!(
        targets(&step),
        vec![
            (
                "live_set tracks 6 devices 1 chains 0".to_string(),
                "devices".to_string()
            ),
            (
                "live_set tracks 6 devices 1 chains 1".to_string(),
                "devices".to_string()
            ),
        ]
    );
    let step = find.answer(&[
        ok(json!([device(
            "PluginDevice",
            "live_set tracks 6 devices 1 chains 0 devices 0",
            "De-ess"
        )])),
        ok(json!([])),
    ]);
    assert_eq!(step, Step::Done(Err(Missing::NoRack)));
    // A track with no device at all.
    let (mut find, _) = Find::start(TRACK);
    assert_eq!(
        find.answer(&[ok(json!([]))]),
        Step::Done(Err(Missing::NoRack))
    );
}

#[test]
fn a_plugin_or_a_plain_device_named_latencies_is_no_rack() {
    let (mut find, _) = Find::start(TRACK);
    assert_eq!(
        find.answer(&[ok(json!([
            device("PluginDevice", "live_set tracks 6 devices 0", "Latencies"),
            device("Device", "live_set tracks 6 devices 1", "Latencies"),
            // A name is matched exactly.
            rack("live_set tracks 6 devices 2", "latencies"),
        ]))]),
        Step::Read(vec![get("live_set tracks 6 devices 2", "chains")])
    );
    assert_eq!(
        find.answer(&[ok(json!([]))]),
        Step::Done(Err(Missing::NoRack))
    );
}

#[test]
fn two_latencies_racks_are_ambiguous_at_any_depth() {
    let (mut find, _) = Find::start(TRACK);
    let step = find.answer(&[ok(json!([
        rack("live_set tracks 6 devices 0", "Latencies"),
        rack("live_set tracks 6 devices 1", "Stream FX"),
    ]))]);
    // The match is not walked into; the other rack is.
    assert_eq!(
        targets(&step),
        read("live_set tracks 6 devices 1", "chains")
    );
    let step = find.answer(&[chain("live_set tracks 6 devices 1 chains 0")]);
    assert_eq!(
        targets(&step),
        read("live_set tracks 6 devices 1 chains 0", "devices")
    );
    // A second one deeper: no parameters read, the two counted.
    assert_eq!(
        find.answer(&[ok(json!([rack(
            "live_set tracks 6 devices 1 chains 0 devices 0",
            "Latencies"
        )]))]),
        Step::Done(Err(Missing::Twice(2)))
    );
    // Three on one level.
    let (mut find, _) = Find::start(TRACK);
    assert_eq!(
        find.answer(&[ok(json!([
            rack("a", "Latencies"),
            rack("b", "Latencies"),
            rack("c", "Latencies"),
        ]))]),
        Step::Done(Err(Missing::Twice(3)))
    );
}

#[test]
fn racks_deeper_than_max_racks_are_not_walked_and_the_find_ends_within_max_reads() {
    assert_eq!(MAX_READS, 8);
    // Four racks deep, the Latencies rack inside the fourth: a devices
    // read `n` chains down holds a rack named Deep FX, 4 down Latencies.
    let site = |path: &str| -> Value {
        let depth = path.matches("chains").count();
        if depth < 4 {
            ok(json!([rack(&format!("{path} devices 0"), "Deep FX")]))
        } else {
            ok(json!([rack(&format!("{path} devices 0"), "Latencies")]))
        }
    };
    let (mut find, first) = Find::start(TRACK);
    let mut commands = first;
    let mut reads = 0;
    let mut end = None;
    for _ in 0..MAX_READS {
        reads += 1;
        let slots: Vec<Value> = commands
            .iter()
            .map(|c| {
                let target = c["target"].as_str().unwrap();
                match c["args"]["prop"].as_str().unwrap() {
                    "chains" => chain(&format!("{target} chains 0")),
                    _ => site(target),
                }
            })
            .collect();
        match find.answer(&slots) {
            Step::Read(next) => commands = next,
            other => {
                end = Some(other);
                break;
            }
        }
    }
    assert_eq!(end, Some(Step::Done(Err(Missing::NoRack))));
    // The track's devices, then chains and devices for three racks.
    assert_eq!(reads, 7);
}

#[test]
fn the_third_rack_down_is_walked_and_a_latencies_rack_inside_it_found() {
    // Three racks deep is within MAX_RACKS: the rack inside the third
    // rack's chain is a match.
    let (mut find, _) = Find::start(TRACK);
    let mut path = format!("{TRACK} devices 0");
    let mut step = find.answer(&[ok(json!([rack(&path, "R1")]))]);
    for level in 2..=3 {
        assert_eq!(targets(&step), read(&path, "chains"));
        step = find.answer(&[chain(&format!("{path} chains 0"))]);
        assert_eq!(targets(&step), read(&format!("{path} chains 0"), "devices"));
        path = format!("{path} chains 0 devices 0");
        step = find.answer(&[ok(json!([rack(&path, &format!("R{level}"))]))]);
    }
    assert_eq!(targets(&step), read(&path, "chains"));
    step = find.answer(&[chain(&format!("{path} chains 0"))]);
    let latencies = format!("{path} chains 0 devices 0");
    assert_eq!(targets(&step), read(&format!("{path} chains 0"), "devices"));
    step = find.answer(&[ok(json!([rack(&latencies, "Latencies")]))]);
    assert_eq!(targets(&step), read(&latencies, "parameters"));
}

#[test]
fn a_failed_first_read_fails_with_the_scripts_error() {
    let (mut find, _) = Find::start("live_set tracks 40");
    assert_eq!(
        find.answer(&[json!({"ok": false, "error": "not found: tracks 40"})]),
        Step::Failed("not found: tracks 40".into())
    );
    let (mut find, _) = Find::start(TRACK);
    assert_eq!(find.answer(&[]), Step::Failed("no result".into()));
    // Only the track's own read fails the find: a later failed slot (a
    // chain deleted meanwhile) is skipped.
    let (mut find, _) = Find::start(TRACK);
    let step = find.answer(&[ok(json!([rack("live_set tracks 6 devices 0", "FX")]))]);
    assert!(matches!(step, Step::Read(_)));
    assert_eq!(
        find.answer(&[json!({"ok": false, "error": "gone"})]),
        Step::Done(Err(Missing::NoRack))
    );
    // A failed read of the matched rack's parameters fails the find.
    let (mut find, _) = Find::start(TRACK);
    let step = find.answer(&[ok(json!([rack(
        "live_set tracks 6 devices 0",
        "Latencies"
    )]))]);
    assert!(matches!(step, Step::Read(_)));
    assert_eq!(
        find.answer(&[json!({"ok": false, "error": "stale"})]),
        Step::Failed("stale".into())
    );
}

#[test]
fn a_rack_without_a_second_parameter_has_no_macro() {
    let (mut find, _) = Find::start(TRACK);
    find.answer(&[ok(json!([rack(
        "live_set tracks 6 devices 0",
        "Latencies"
    )]))]);
    assert_eq!(
        find.answer(&[ok(json!([
            {"$ref": "p1", "path": "live_set tracks 6 devices 0 parameters 0", "class": "DeviceParameter", "name": "Device On"},
        ]))]),
        Step::Done(Err(Missing::NoMacro))
    );
    let (mut find, _) = Find::start(TRACK);
    find.answer(&[ok(json!([rack(
        "live_set tracks 6 devices 0",
        "Latencies"
    )]))]);
    assert_eq!(
        find.answer(&[ok(json!("no list"))]),
        Step::Done(Err(Missing::NoMacro))
    );
}

#[test]
fn missing_says_why_in_words() {
    assert_eq!(Missing::NoRack.to_string(), "no rack named Latencies");
    assert_eq!(Missing::Twice(2).to_string(), "2 racks named Latencies");
    assert_eq!(
        Missing::NoMacro.to_string(),
        "the Latencies rack has no parameter 1"
    );
}
