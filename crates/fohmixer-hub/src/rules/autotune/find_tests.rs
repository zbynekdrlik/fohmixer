use super::*;
use crate::rules::autotune::keys::KeyDevice;
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

/// An answer to a matched rack's parts read: its parameters, and its
/// chains (`chains`, the list's data).
fn parts(rack: &str, chains: Value) -> [Value; 2] {
    [parameters(rack), ok(chains)]
}

/// A rack's chains list: `n` chains, paths `<rack> chains <i>`.
fn chains(rack: &str, n: usize) -> Value {
    Value::from(
        (0..n)
            .map(|i| json!({"$ref": format!("c{i}"), "path": format!("{rack} chains {i}"), "class": "Chain", "name": format!("Chain {i}")}))
            .collect::<Vec<Value>>(),
    )
}

/// The parameters of a plug-in at `plugin` named `names` in order, each
/// with the `$ref` `<plugin>/<i>`.
fn plugin_parameters(plugin: &str, names: &[&str]) -> Value {
    ok(Value::from(
        names
            .iter()
            .enumerate()
            .map(|(i, name)| json!({"$ref": format!("{plugin}/{i}"), "path": format!("{plugin} parameters {i}"), "class": "DeviceParameter", "name": name}))
            .collect::<Vec<Value>>(),
    ))
}

/// AudioGridder's parameters as Live lists them (spike, 2026-10-24).
const GRIDDER: [&str; 4] = [
    "Device On",
    "0:(unused):(unassigned)",
    "1:(unused):(unassigned)",
    "2:(unused):(unassigned)",
];

/// The macro the parts read found at `rack`, with the key device `key`.
fn macro_found(rack: &str, key: Result<KeyFound, KeyMissing>) -> Found {
    Found {
        path: format!("{rack} parameters 1"),
        id: "p2".into(),
        key,
    }
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

/// A matched rack's parts read: its parameters and its chains.
fn parts_read(target: &str) -> Vec<(String, String)> {
    vec![
        (target.to_string(), "parameters".to_string()),
        (target.to_string(), "chains".to_string()),
    ]
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
        Step::Read(vec![
            get("live_set tracks 6 devices 0", "parameters"),
            get("live_set tracks 6 devices 0", "chains"),
        ])
    );
    // A rack without chains: its macro, and no key device.
    assert_eq!(
        find.answer(&parts("live_set tracks 6 devices 0", json!([]))),
        Step::Done(Ok(macro_found(
            "live_set tracks 6 devices 0",
            Err(KeyMissing::NoChain)
        )))
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
    // 3: the Latencies rack inside: its parameters and chains.
    let inner = "live_set tracks 7 devices 0 chains 0 devices 0";
    let step = find.answer(&[ok(json!([rack(inner, "Latencies")]))]);
    assert_eq!(targets(&step), parts_read(inner));
    // 4: its first macro.
    assert_eq!(
        find.answer(&parts(inner, json!([]))),
        Step::Done(Ok(macro_found(inner, Err(KeyMissing::NoChain))))
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
    assert_eq!(MAX_READS, 10);
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
    assert_eq!(targets(&step), parts_read(&latencies));
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

/// A find on the stream track 7 up to its matched rack (inside `Stream
/// FX`'s chain), its parts read waiting.
fn to_the_rack() -> (Find, String) {
    let (mut find, _) = Find::start("live_set tracks 7");
    find.answer(&[ok(json!([rack(
        "live_set tracks 7 devices 0",
        "Stream FX"
    )]))]);
    find.answer(&[chain("live_set tracks 7 devices 0 chains 0")]);
    let inner = "live_set tracks 7 devices 0 chains 0 devices 0".to_string();
    let step = find.answer(&[ok(json!([rack(&inner, "Latencies")]))]);
    assert_eq!(targets(&step), parts_read(&inner));
    (find, inner)
}

#[test]
fn the_find_reads_chain_zero_and_its_plug_in_parameters() {
    let (mut find, rack_path) = to_the_rack();
    let chain0 = format!("{rack_path} chains 0");
    let step = find.answer(&parts(&rack_path, chains(&rack_path, 2)));
    assert_eq!(targets(&step), read(&chain0, "devices"));
    let plugin = format!("{chain0} devices 0");
    let step = find.answer(&[ok(json!([device("PluginDevice", &plugin, "AudioGridder")]))]);
    assert_eq!(targets(&step), read(&plugin, "parameters"));
    let Step::Done(Ok(found)) = find.answer(&[plugin_parameters(&plugin, &GRIDDER)]) else {
        panic!("the find ends with the macro")
    };
    assert_eq!(
        (found.path.as_str(), found.id.as_str()),
        (format!("{rack_path} parameters 1").as_str(), "p2")
    );
    assert_eq!(
        found.key,
        Ok(KeyFound {
            params: KeyParams {
                device: KeyDevice::Gridder,
                key: 1,
                scale: 2
            },
            device: plugin.clone(),
            key: Param {
                path: format!("{plugin} parameters 1"),
                id: format!("{plugin}/1"),
            },
            scale: Param {
                path: format!("{plugin} parameters 2"),
                id: format!("{plugin}/2"),
            },
        })
    );
}

#[test]
fn an_auto_tune_artist_is_found_by_its_names() {
    let (mut find, rack_path) = to_the_rack();
    find.answer(&parts(&rack_path, chains(&rack_path, 2)));
    let plugin = format!("{rack_path} chains 0 devices 1");
    // A native device beside the plug-in is no plug-in.
    find.answer(&[ok(json!([
        device(
            "Device",
            &format!("{rack_path} chains 0 devices 0"),
            "Utility"
        ),
        device("PluginDevice", &plugin, "Auto-Tune Artist"),
    ]))]);
    let Step::Done(Ok(found)) = find.answer(&[plugin_parameters(
        &plugin,
        &["Device On", "Input Type", "Key", "Scale"],
    )]) else {
        panic!("the find ends with the macro")
    };
    let key = found.key.expect("a key device");
    assert_eq!(key.params.device, KeyDevice::AutoTune);
    assert_eq!(
        (key.key.path.as_str(), key.scale.path.as_str()),
        (
            format!("{plugin} parameters 2").as_str(),
            format!("{plugin} parameters 3").as_str()
        )
    );
    assert_eq!(
        (key.key.id, key.scale.id),
        (format!("{plugin}/2"), format!("{plugin}/3"))
    );
}

#[test]
fn only_chain_zero_is_searched_for_the_key_device() {
    // The master's rack: a native Auto-Tune in chain 2 that no value
    // selects, the AudioGridder in chain 0.
    let (mut find, rack_path) = to_the_rack();
    let step = find.answer(&parts(&rack_path, chains(&rack_path, 3)));
    assert_eq!(
        targets(&step),
        read(&format!("{rack_path} chains 0"), "devices"),
        "chain 0's devices only"
    );
    let plugin = format!("{rack_path} chains 0 devices 0");
    find.answer(&[ok(json!([device("PluginDevice", &plugin, "AudioGridder")]))]);
    let Step::Done(Ok(found)) = find.answer(&[plugin_parameters(&plugin, &GRIDDER)]) else {
        panic!("the find ends with the macro")
    };
    let key = found.key.expect("the AudioGridder");
    assert_eq!(key.device, plugin);
    assert_eq!(key.params.device, KeyDevice::Gridder);
}

#[test]
fn a_rack_without_chains_keeps_its_macro_and_misses_the_key_device() {
    let (mut find, rack_path) = to_the_rack();
    assert_eq!(
        find.answer(&parts(&rack_path, json!([]))),
        Step::Done(Ok(macro_found(&rack_path, Err(KeyMissing::NoChain))))
    );
    // Chains that are no list: none either.
    let (mut find, rack_path) = to_the_rack();
    assert_eq!(
        find.answer(&parts(&rack_path, json!("no list"))),
        Step::Done(Ok(macro_found(&rack_path, Err(KeyMissing::NoChain))))
    );
}

#[test]
fn two_plug_ins_in_chain_zero_miss_the_key_device() {
    let (mut find, rack_path) = to_the_rack();
    find.answer(&parts(&rack_path, chains(&rack_path, 1)));
    let chain0 = format!("{rack_path} chains 0");
    assert_eq!(
        find.answer(&[ok(json!([
            device("PluginDevice", &format!("{chain0} devices 0"), "smartDeess"),
            device(
                "PluginDevice",
                &format!("{chain0} devices 1"),
                "AudioGridder"
            ),
        ]))]),
        Step::Done(Ok(macro_found(&rack_path, Err(KeyMissing::Plugins(2)))))
    );
    // None at all: an empty chain, or native devices only.
    let (mut find, rack_path) = to_the_rack();
    find.answer(&parts(&rack_path, chains(&rack_path, 1)));
    assert_eq!(
        find.answer(&[ok(json!([device(
            "Device",
            &format!("{rack_path} chains 0 devices 0"),
            "Utility"
        )]))]),
        Step::Done(Ok(macro_found(&rack_path, Err(KeyMissing::Plugins(0)))))
    );
}

#[test]
fn a_plug_in_without_key_parameters_misses_the_key_device() {
    let (mut find, rack_path) = to_the_rack();
    find.answer(&parts(&rack_path, chains(&rack_path, 1)));
    let plugin = format!("{rack_path} chains 0 devices 0");
    find.answer(&[ok(json!([device("PluginDevice", &plugin, "Pro-Q 4")]))]);
    assert_eq!(
        find.answer(&[plugin_parameters(&plugin, &["Device On"])]),
        Step::Done(Ok(macro_found(&rack_path, Err(KeyMissing::NoKeyParams))))
    );
}

#[test]
fn a_refused_read_of_the_rack_its_chain_or_its_plug_in_fails_the_find() {
    let refusal = json!({"ok": false, "error": "stale"});
    // The rack's chains refused.
    let (mut find, rack_path) = to_the_rack();
    assert_eq!(
        find.answer(&[parameters(&rack_path), refusal.clone()]),
        Step::Failed("stale".into())
    );
    // The rack's chains missing from the answer.
    let (mut find, rack_path) = to_the_rack();
    assert_eq!(
        find.answer(&[parameters(&rack_path)]),
        Step::Failed("no result".into())
    );
    // Chain 0's devices refused.
    let (mut find, rack_path) = to_the_rack();
    find.answer(&parts(&rack_path, chains(&rack_path, 1)));
    assert_eq!(
        find.answer(&[refusal.clone()]),
        Step::Failed("stale".into())
    );
    // The plug-in's parameters refused.
    let (mut find, rack_path) = to_the_rack();
    find.answer(&parts(&rack_path, chains(&rack_path, 1)));
    let plugin = format!("{rack_path} chains 0 devices 0");
    find.answer(&[ok(json!([device("PluginDevice", &plugin, "AudioGridder")]))]);
    assert_eq!(find.answer(&[refusal]), Step::Failed("stale".into()));
    let (mut find, rack_path) = to_the_rack();
    find.answer(&parts(&rack_path, chains(&rack_path, 1)));
    find.answer(&[ok(json!([device("PluginDevice", &plugin, "AudioGridder")]))]);
    assert_eq!(find.answer(&[]), Step::Failed("no result".into()));
}

#[test]
fn the_find_ends_within_max_reads() {
    // The deepest nesting a find walks (the Latencies rack inside the third
    // rack's chain), then the rack's parts, chain 0 and the plug-in.
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
                let depth = target.matches("chains").count();
                match (c["args"]["prop"].as_str().unwrap(), depth) {
                    ("parameters", 4) => plugin_parameters(target, &GRIDDER),
                    ("parameters", _) => parameters(target),
                    ("chains", _) => chain(&format!("{target} chains 0")),
                    ("devices", 4) => ok(json!([device(
                        "PluginDevice",
                        &format!("{target} devices 0"),
                        "AudioGridder"
                    )])),
                    ("devices", 3) => {
                        ok(json!([rack(&format!("{target} devices 0"), "Latencies")]))
                    }
                    (_, _) => ok(json!([rack(&format!("{target} devices 0"), "Deep FX")])),
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
    let Some(Step::Done(Ok(found))) = end else {
        panic!("the find ended with the macro: {end:?}")
    };
    assert_eq!(reads, MAX_READS);
    assert_eq!(found.key.map(|k| k.params.device), Ok(KeyDevice::Gridder));
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
