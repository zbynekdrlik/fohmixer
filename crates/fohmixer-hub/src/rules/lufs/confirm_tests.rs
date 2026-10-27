use super::*;
use serde_json::json;

const TRACK: &str = "live_set tracks 1";

fn on_track() -> [String; PARENTS] {
    ["Vox 1", "FOH Band", "-", "-"].map(str::to_string)
}

fn in_chain() -> [String; PARENTS] {
    ["LUFS", "LUFS rack", "Vox 1", "FOH Band"].map(str::to_string)
}

fn ok(data: Value) -> Value {
    json!({"ok": true, "data": data})
}

fn device(path: &str, class: &str, name: &str) -> Value {
    json!({"$ref": "1", "path": path, "class": class, "name": name})
}

fn read(step: Step) -> Vec<Value> {
    match step {
        Step::Read(commands) => commands,
        other => panic!("expected a read, got {other:?}"),
    }
}

#[test]
fn the_constants_and_names() {
    assert_eq!(MAX_CLASS, "MxDeviceAudioEffect");
    assert_eq!(device_name(DeviceKind::Lufs), "FohMixer LUFS");
    assert_eq!(device_name(DeviceKind::Meter), "FohMixer Meter");
    assert_eq!((CLASS_NAME, MAX_READS), ("class_name", 4));
    assert_eq!(answer_of(0), Answer::NotOurs);
    assert_eq!(answer_of(1), Answer::Ours);
    assert_eq!(answer_of(2), Answer::Two);
    assert_eq!(answer_of(3), Answer::Two);
    assert_eq!(Answer::Ours.word(), "ours");
    assert_eq!(Answer::NotOurs.word(), "not_ours");
    assert_eq!(Answer::Two.word(), "two");
    assert_eq!(Answer::Failed("x".into()).word(), "failed");
}

#[test]
fn the_place_comes_from_the_parents() {
    assert_eq!(place_of(&on_track()), Place::Track);
    assert_eq!(
        place_of(&in_chain()),
        Place::Chain {
            rack: "LUFS rack".into(),
            chain: "LUFS".into()
        }
    );
}

#[test]
fn a_device_on_the_track_is_ours() {
    let (mut walk, first) = Confirm::start(DeviceKind::Lufs, &on_track(), TRACK);
    assert_eq!(first, vec![get(TRACK, "devices")]);
    let devices = ok(json!([
        device("live_set tracks 1 devices 0", "Device", "Tuner"),
        device("live_set tracks 1 devices 1", "Device", "FohMixer LUFS"),
        device("live_set tracks 1 devices 2", "Device", "FohMixer Meter"),
    ]));
    let next = read(walk.answer(&[devices]));
    assert_eq!(next, vec![get("live_set tracks 1 devices 1", "class_name")]);
    assert_eq!(
        walk.answer(&[ok(json!("MxDeviceAudioEffect"))]),
        Step::Done(Answer::Ours)
    );
}

#[test]
fn a_device_of_another_class_is_not_ours() {
    let (mut walk, _) = Confirm::start(DeviceKind::Lufs, &on_track(), TRACK);
    let devices = ok(json!([device(
        "live_set tracks 1 devices 0",
        "Device",
        "FohMixer LUFS"
    )]));
    read(walk.answer(&[devices]));
    assert_eq!(
        walk.answer(&[ok(json!("AudioEffectGroupDevice"))]),
        Step::Done(Answer::NotOurs)
    );
}

#[test]
fn no_device_of_the_name_is_not_ours_at_once() {
    let (mut walk, _) = Confirm::start(DeviceKind::Meter, &on_track(), TRACK);
    let devices = ok(json!([device(
        "live_set tracks 1 devices 0",
        "Device",
        "FohMixer LUFS"
    )]));
    assert_eq!(walk.answer(&[devices]), Step::Done(Answer::NotOurs));
}

#[test]
fn two_on_the_track_are_two() {
    let (mut walk, _) = Confirm::start(DeviceKind::Lufs, &on_track(), TRACK);
    let devices = ok(json!([
        device("live_set tracks 1 devices 0", "Device", "FohMixer LUFS"),
        device("live_set tracks 1 devices 1", "Device", "FohMixer LUFS"),
    ]));
    assert_eq!(read(walk.answer(&[devices])).len(), 2);
    let class = ok(json!("MxDeviceAudioEffect"));
    assert_eq!(
        walk.answer(&[class.clone(), class]),
        Step::Done(Answer::Two)
    );
}

#[test]
fn a_device_in_its_racks_chain_is_ours_and_others_are_not_counted() {
    let (mut walk, _) = Confirm::start(DeviceKind::Lufs, &in_chain(), TRACK);
    let devices = ok(json!([
        // On the track itself: not the key's place.
        device("live_set tracks 1 devices 0", "Device", "FohMixer LUFS"),
        device("live_set tracks 1 devices 1", "RackDevice", "Other rack"),
        device("live_set tracks 1 devices 2", "RackDevice", "LUFS rack"),
    ]));
    let next = read(walk.answer(&[devices]));
    assert_eq!(next, vec![get("live_set tracks 1 devices 2", "chains")]);
    let chains = ok(json!([
        {"$ref": "5", "path": "live_set tracks 1 devices 2 chains 0", "class": "Chain", "name": "Dry"},
        {"$ref": "6", "path": "live_set tracks 1 devices 2 chains 1", "class": "Chain", "name": "LUFS"},
    ]));
    let next = read(walk.answer(&[chains]));
    assert_eq!(
        next,
        vec![get("live_set tracks 1 devices 2 chains 1", "devices")]
    );
    let in_it = ok(json!([device(
        "live_set tracks 1 devices 2 chains 1 devices 0",
        "Device",
        "FohMixer LUFS"
    )]));
    let next = read(walk.answer(&[in_it]));
    assert_eq!(
        next,
        vec![get(
            "live_set tracks 1 devices 2 chains 1 devices 0",
            "class_name"
        )]
    );
    assert_eq!(
        walk.answer(&[ok(json!("MxDeviceAudioEffect"))]),
        Step::Done(Answer::Ours)
    );
}

#[test]
fn a_chain_device_on_a_track_without_its_rack_is_not_ours() {
    let (mut walk, _) = Confirm::start(DeviceKind::Lufs, &in_chain(), TRACK);
    let devices = ok(json!([device(
        "live_set tracks 1 devices 0",
        "Device",
        "FohMixer LUFS"
    )]));
    assert_eq!(walk.answer(&[devices]), Step::Done(Answer::NotOurs));
}

#[test]
fn a_track_refused_fails_and_a_later_refusal_counts_nothing() {
    let (mut walk, _) = Confirm::start(DeviceKind::Lufs, &on_track(), TRACK);
    let refusal = json!({"ok": false, "error": "not found: tracks 1"});
    assert_eq!(
        walk.answer(&[refusal.clone()]),
        Step::Done(Answer::Failed("not found: tracks 1".into()))
    );
    let (mut walk, _) = Confirm::start(DeviceKind::Lufs, &on_track(), TRACK);
    assert_eq!(
        walk.answer(&[]),
        Step::Done(Answer::Failed("no result".into()))
    );
    let (mut walk, _) = Confirm::start(DeviceKind::Lufs, &on_track(), TRACK);
    let devices = ok(json!([device(
        "live_set tracks 1 devices 0",
        "Device",
        "FohMixer LUFS"
    )]));
    read(walk.answer(&[devices]));
    assert_eq!(walk.answer(&[refusal]), Step::Done(Answer::NotOurs));
}
