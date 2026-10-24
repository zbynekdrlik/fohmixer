use std::collections::BTreeMap;

use fohmixer_proto::client::KeyState;
use fohmixer_proto::layout::{Anchor, AutotuneTarget, Binding};
use fohmixer_proto::songmap::{ClipName, Cue, Key, SectionsState, build};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

use super::super::super::songmap::Listing;
use super::super::super::tests::{next_of, records, status};
use super::super::super::{Router, RouterIo, RouterMsg};
use super::{
    BAR_ROOT, BAR_SCALE, bar_read, elapsed_ms, found_key_write, judged_for_another,
    key_read_commands, key_write_commands, late_note, read_values, write_problem,
};
use crate::live::client::{Asked, LiveEvent, Script, scripted};
use crate::rules::HubState;
use crate::rules::autotune::find::{Found, KeyFound, Param};
use crate::rules::autotune::keys::{KeyDevice, KeyParams};

/// The PA target (an Auto-Tune Artist, on 0) and the stream target (an
/// AudioGridder, on 127) of the band.
const PA: &str = "band|live_set tracks 6";
const STREAM: &str = "band|live_set tracks 7";
/// The band's playhead, tempo and transport (hub keys).
const TIME: &str = "band|live_set|current_song_time|false";
const TEMPO_KEY: &str = "band|live_set|tempo|false";
const TRANSPORT: &str = "band|live_set|is_playing|false";
/// Live's words for a parameter a Max map holds (the PC, 24.10.2026).
const DISABLED: &str = "live error: Value cannot be set, the parameter is disabled";

fn target(index: u32, label: &str, on: u8, off: u8) -> AutotuneTarget {
    AutotuneTarget {
        binding: Binding {
            instance: "band".into(),
            anchor: Anchor::TrackAt { index },
            path: None,
        },
        label: label.into(),
        on,
        off,
    }
}

fn pa() -> AutotuneTarget {
    target(6, "Vox 2", 0, 127)
}

fn stream() -> AutotuneTarget {
    target(7, "#8", 127, 0)
}

fn layout(rev: u64, autotune: Vec<AutotuneTarget>) -> RouterMsg {
    RouterMsg::Layout {
        rev,
        stage: None,
        targets: BTreeMap::new(),
        strips: Vec::new(),
        reverb: Vec::new(),
        fader_leaders: Vec::new(),
        autotune,
    }
}

/// Band track `index`'s key device in its rack's chain 0: the Auto-Tune
/// Artist on track 6, AudioGridder on the others; its Key and Scale
/// `$ref`s `k<index><n>` and `s<index><n>` (`n`: another device's mark).
fn device(index: u32, n: &str) -> KeyFound {
    let plugin = format!("live_set tracks {index} devices 0 chains 0 devices 0");
    let kind = if index == 6 {
        KeyDevice::AutoTune
    } else {
        KeyDevice::Gridder
    };
    KeyFound {
        params: KeyParams {
            device: kind,
            key: 1,
            scale: 2,
        },
        device: plugin.clone(),
        key: Param {
            path: format!("{plugin} parameters 1"),
            id: format!("k{index}{n}"),
        },
        scale: Param {
            path: format!("{plugin} parameters 2"),
            id: format!("s{index}{n}"),
        },
    }
}

/// Band track `index`'s `Latencies` rack: its macro (`$ref` `p<index>`, a
/// selector) and its key device.
fn rack(index: u32, n: &str) -> Found {
    Found {
        path: format!("live_set tracks {index} devices 0 parameters 1"),
        id: format!("p{index}"),
        key: Ok(device(index, n)),
    }
}

/// A router whose band is a scripted Live: every call comes to the test.
fn scripted_router(
    dir: &std::path::Path,
) -> (
    Router,
    UnboundedReceiver<RouterMsg>,
    std::sync::mpsc::Receiver<Value>,
    Script,
) {
    let (live, script) = scripted("band");
    let (tx, own) = tokio::sync::mpsc::unbounded_channel();
    let (events, records) = crate::events::EventLog::channel(1024);
    let router = Router::new(
        BTreeMap::from([("band".to_string(), live)]),
        HubState::default(),
        dir.to_path_buf(),
        RouterIo { tx, events },
    );
    (router, own, records, script)
}

/// A target's newest walk answers `found`.
fn find(router: &mut Router, key: &str, found: Found) {
    let walk = router.autotune.targets[key].walk;
    router.autotune_found(key, walk, Ok(found));
}

/// A delivery of one of AUTOTUNE's subscriptions, as the flush hands it.
fn deliver(router: &mut Router, sub: &str, value: f64) {
    router.autotune_delivery(sub, Some(json!(value)), None);
}

/// A target's key device's Key and Scale values heard.
fn heard(router: &mut Router, key: &str, values: (f64, f64)) {
    let target = &router.autotune.targets[key];
    let (key_sub, scale_sub) = (
        target.key_sub.clone().unwrap(),
        target.scale_sub.clone().unwrap(),
    );
    deliver(router, &key_sub, values.0);
    deliver(router, &scale_sub, values.1);
}

/// The band's transport heard, as the flush hands it over.
fn transport(router: &mut Router, playing: bool) {
    router.on_transport(TRANSPORT, &json!(playing));
}

/// Song 1 (`001 Test One {D 120}`) from 0 to 100 with a key clip to G at
/// 50, song 2 in D too from 120 to 200, song 3 in F#m from 210 to 300 and
/// song 4 with no key from 310 to 400 (invented titles).
fn songs() -> Listing {
    let cue = |name: &str, time: f64| Cue {
        name: name.to_string(),
        time: Some(time),
    };
    Listing {
        cues: vec![
            cue("001 Test One {D 120}", 0.0),
            cue("SONG END", 100.0),
            cue("002 Test Two {D 90}", 120.0),
            cue("SONG END", 200.0),
            cue("003 Test Three {F#m 100}", 210.0),
            cue("SONG END", 300.0),
            cue("004 Test Four", 310.0),
            cue("SONG END", 400.0),
        ],
        clips: vec![ClipName {
            name: "Key {G}".into(),
            start: Some(50.0),
        }],
        sections: SectionsState::One,
        track: None,
        sections_error: None,
    }
}

/// The band's song map read `read_no` answers `listing`.
fn map_read(router: &mut Router, read_no: u64, listing: Listing) {
    router.songmap.reading.insert("band".into(), read_no);
    router.handle(RouterMsg::SongMapRead {
        instance: "band".into(),
        read_no,
        result: Ok(listing),
        ms: 5,
    });
}

/// The `autotune` records waiting, without `ev` and `ts`.
fn steps(rx: &std::sync::mpsc::Receiver<Value>) -> Vec<Value> {
    records(rx)
        .into_iter()
        .filter(|r| r["ev"] == "autotune")
        .map(|mut r| {
            let fields = r.as_object_mut().unwrap();
            fields.remove("ev");
            fields.remove("ts");
            r
        })
        .collect()
}

/// The records of `step` among `got`.
fn of(got: &[Value], step: &str) -> Vec<Value> {
    got.iter().filter(|r| r["step"] == step).cloned().collect()
}

fn key_write(key: &str, why: &str, targets: &[&str], bar: bool) -> Value {
    json!({"step": "key_write", "instance": "band", "key": key, "why": why,
           "targets": targets, "bar": bar})
}

fn readback(
    index: u32,
    key: &str,
    state: &str,
    shows: Option<&str>,
    problem: Option<&str>,
) -> Value {
    json!({"step": "readback", "instance": "band", "target": format!("live_set tracks {index}"),
           "key": key, "state": state, "shows": shows, "problem": problem})
}

fn bar(key: &str, state: &str, shows: Option<&str>) -> Value {
    json!({"step": "bar", "instance": "band", "key": key, "state": state, "shows": shows})
}

fn ok(data: Value) -> Value {
    json!({"ok": true, "data": data})
}

fn disabled() -> Value {
    json!({"ok": false, "error": DISABLED, "errorType": "RuntimeError"})
}

fn set_ref(id: &str, value: f64) -> Value {
    json!({"target": {"$ref": id, "class": "DeviceParameter"}, "name": "set_prop",
           "args": {"prop": "value", "value": value}})
}

fn get_ref(id: &str) -> Value {
    json!({"target": {"$ref": id, "class": "DeviceParameter"}, "name": "get_prop",
           "args": {"prop": "value"}})
}

fn set_bar(root: u8, scale: &str) -> [Value; 2] {
    [
        json!({"target": "live_set", "name": "set_prop", "args": {"prop": "root_note", "value": root}}),
        json!({"target": "live_set", "name": "set_prop", "args": {"prop": "scale_name", "value": scale}}),
    ]
}

fn get_bar() -> [Value; 2] {
    [
        json!({"target": "live_set", "name": "get_prop", "args": {"prop": "root_note"}}),
        json!({"target": "live_set", "name": "get_prop", "args": {"prop": "scale_name"}}),
    ]
}

/// A command's key: its `$ref`, or Live's bar's property.
fn id_of(command: &Value) -> String {
    command["target"]["$ref"].as_str().map_or_else(
        || command["args"]["prop"].as_str().unwrap_or("").to_string(),
        str::to_string,
    )
}

/// Whether a call is one of the key writes or read-backs: every command
/// goes through a key parameter's `$ref` (`k…`, `s…`) or to Live's bar.
fn is_keys(commands: &[Value]) -> bool {
    !commands.is_empty()
        && commands.iter().all(|c| match c["target"]["$ref"].as_str() {
            Some(id) => id.starts_with('k') || id.starts_with('s'),
            None => {
                c["target"] == "live_set"
                    && (c["args"]["prop"] == BAR_ROOT || c["args"]["prop"] == BAR_SCALE)
            }
        })
}

/// No command of a call writes a macro or a chain selector: every
/// `set_prop` is a key parameter's `value` through its `$ref` and class,
/// or Live's bar.
fn no_selector(commands: &[Value]) {
    for c in commands.iter().filter(|c| c["name"] == "set_prop") {
        match c["target"]["$ref"].as_str() {
            Some(id) => {
                assert!(id.starts_with('k') || id.starts_with('s'), "{c}");
                assert_eq!(c["target"]["class"], "DeviceParameter", "{c}");
                assert_eq!(c["args"]["prop"], "value", "{c}");
            }
            None => {
                assert_eq!(c["target"], "live_set", "{c}");
                assert!(
                    c["args"]["prop"] == BAR_ROOT || c["args"]["prop"] == BAR_SCALE,
                    "{c}"
                );
            }
        }
    }
}

/// The next key write or read-back to Live (a walk or another read is
/// passed over, unanswered), none writing a selector.
fn next_call(script: &mut Script) -> Option<Asked> {
    while let Some(asked) = script.try_call() {
        no_selector(&asked.commands);
        if is_keys(&asked.commands) {
            return Some(asked);
        }
    }
    None
}

fn call(script: &mut Script) -> Asked {
    next_call(script).expect("a key write or read-back to Live")
}

fn no_call(script: &mut Script) {
    if let Some(asked) = next_call(script) {
        panic!("no call expected: {:?}", asked.commands);
    }
}

/// Live answers a key write with `slots`; its answer handled.
async fn written(
    router: &mut Router,
    own: &mut UnboundedReceiver<RouterMsg>,
    asked: Asked,
    slots: Vec<Value>,
) {
    asked.answer(slots);
    let msg = next_of(own, "the key write's answer", |m| {
        matches!(m, RouterMsg::AutotuneKeysWritten { .. })
    })
    .await;
    router.handle(msg);
}

/// Live answers a read-back with `slots`; its answer handled.
async fn read_back(
    router: &mut Router,
    own: &mut UnboundedReceiver<RouterMsg>,
    asked: Asked,
    slots: Vec<Value>,
) {
    asked.answer(slots);
    let msg = next_of(own, "the read-back", |m| {
        matches!(m, RouterMsg::AutotuneKeysRead { .. })
    })
    .await;
    router.handle(msg);
}

/// The values a write's commands set: by `$ref`, Live's bar by property.
fn set_by(commands: &[Value]) -> BTreeMap<String, Value> {
    commands
        .iter()
        .filter(|c| c["name"] == "set_prop")
        .map(|c| (id_of(c), c["args"]["value"].clone()))
        .collect()
}

/// Live's answer to a read-back: each value from `values`.
fn read_from(asked: &Asked, values: &BTreeMap<String, Value>) -> Vec<Value> {
    asked
        .commands
        .iter()
        .map(|c| ok(values[&id_of(c)].clone()))
        .collect()
}

/// The next key write goes through and reads back as written: its
/// commands.
async fn confirm(
    router: &mut Router,
    own: &mut UnboundedReceiver<RouterMsg>,
    script: &mut Script,
) -> Vec<Value> {
    let write = call(script);
    let commands = write.commands.clone();
    let n = commands.len();
    written(router, own, write, vec![ok(Value::Null); n]).await;
    let read = call(script);
    let answer = read_from(&read, &set_by(&commands));
    read_back(router, own, read, answer).await;
    commands
}

/// Each target's key state in `/api/status`, in Live's order.
fn states(router: &mut Router) -> Vec<Option<KeyState>> {
    status(router)
        .autotune
        .targets
        .iter()
        .map(|t| t.key.as_ref().and_then(|k| k.state))
        .collect()
}

/// The stream target found with its key device's values heard (`values`)
/// and the song map read: Live stands out of every song.
fn ready_stream(router: &mut Router, values: (f64, f64)) {
    router.handle(layout(1, vec![stream()]));
    find(router, STREAM, rack(7, ""));
    heard(router, STREAM, values);
    map_read(router, 1, songs());
}

#[test]
fn the_write_and_read_back_calls_go_by_ref() {
    assert_eq!((BAR_ROOT, BAR_SCALE), ("root_note", "scale_name"));
    let (pa, stream) = (device(6, ""), device(7, ""));
    let d = Key {
        root: 2,
        minor: false,
    };
    let f_sharp_m = Key {
        root: 6,
        minor: true,
    };
    let [root, scale] = set_bar(2, "Major");
    assert_eq!(
        key_write_commands(&[(&pa, (0.5, 0.25)), (&stream, (0.75, 1.0))], Some(d)),
        vec![
            set_ref("k6", 0.5),
            set_ref("s6", 0.25),
            set_ref("k7", 0.75),
            set_ref("s7", 1.0),
            root,
            scale
        ]
    );
    assert_eq!(
        key_write_commands(&[(&stream, (0.75, 1.0))], None),
        vec![set_ref("k7", 0.75), set_ref("s7", 1.0)]
    );
    assert_eq!(
        key_write_commands(&[], Some(f_sharp_m)),
        set_bar(6, "Minor").to_vec()
    );
    let [root, scale] = get_bar();
    assert_eq!(
        key_read_commands(&[&pa, &stream], true),
        vec![
            get_ref("k6"),
            get_ref("s6"),
            get_ref("k7"),
            get_ref("s7"),
            root,
            scale
        ]
    );
    assert_eq!(
        key_read_commands(&[&stream], false),
        vec![get_ref("k7"), get_ref("s7")]
    );
    assert_eq!(key_read_commands(&[], true), get_bar().to_vec());
    assert!(key_read_commands(&[], false).is_empty());
}

#[test]
fn a_writes_slots_say_which_device_was_refused() {
    let null = ok(Value::Null);
    let stale = json!({"ok": false, "error": "stale ref", "errorType": "StaleRef"});
    let slots = Ok(vec![
        null.clone(),
        null.clone(),
        disabled(),
        null.clone(),
        null.clone(),
        disabled(),
        stale,
        null,
    ]);
    assert_eq!(write_problem(&slots, 0), None);
    assert_eq!(
        write_problem(&slots, 2),
        Some((DISABLED.to_string(), false))
    );
    assert_eq!(
        write_problem(&slots, 4),
        Some((DISABLED.to_string(), false)),
        "the Scale's slot alone"
    );
    assert_eq!(
        write_problem(&slots, 6),
        Some(("stale ref".to_string(), true))
    );
    assert_eq!(
        write_problem(&slots, 8),
        Some(("no result".to_string(), false))
    );
    assert_eq!(
        write_problem(&Err("instance offline".into()), 0),
        Some(("instance offline".to_string(), false))
    );
}

#[test]
fn a_read_backs_slots_give_each_devices_values() {
    let slots = Ok(vec![
        ok(json!(0.5)),
        ok(json!(0.25)),
        ok(json!(0.75)),
        disabled(),
        ok(json!("x")),
        ok(json!(1.0)),
        ok(json!(6)),
        ok(json!("Minor")),
    ]);
    assert_eq!(read_values(&slots, 0), Ok((0.5, 0.25)));
    assert_eq!(read_values(&slots, 2), Err(DISABLED.to_string()));
    assert_eq!(read_values(&slots, 4), Err("not a number".to_string()));
    assert_eq!(read_values(&slots, 8), Err("no result".to_string()));
    assert_eq!(
        read_values(&Err("instance offline".into()), 0),
        Err("instance offline".to_string())
    );
    assert_eq!(
        bar_read(&slots, 6),
        Some(Key {
            root: 6,
            minor: true
        })
    );
    assert_eq!(bar_read(&slots, 5), None, "no index for the root");
    assert_eq!(bar_read(&slots, 8), None);
    assert_eq!(bar_read(&Err("instance offline".into()), 6), None);
}

#[test]
fn the_small_decisions_of_the_writes() {
    assert_eq!(elapsed_ms(std::time::Duration::from_millis(12)), 12.0);
    assert_eq!(elapsed_ms(std::time::Duration::from_micros(2500)), 2.5);
    let d = Some(Key {
        root: 2,
        minor: false,
    });
    let refs = ("k7".to_string(), "s7".to_string());
    let other = ("k7b".to_string(), "s7b".to_string());
    assert_eq!(found_key_write(None, &refs, true, d), d, "owed");
    assert_eq!(
        found_key_write(Some(&other), &refs, true, d),
        d,
        "another device"
    );
    assert_eq!(
        found_key_write(Some(&refs), &refs, true, d),
        None,
        "written"
    );
    assert_eq!(
        found_key_write(None, &refs, false, d),
        None,
        "values unheard"
    );
    assert_eq!(
        found_key_write(None, &refs, true, None),
        None,
        "no key in force"
    );
    assert!(judged_for_another(true, false, false));
    assert!(!judged_for_another(false, false, false), "not confirmed");
    assert!(!judged_for_another(true, true, false), "held");
    assert!(!judged_for_another(true, false, true), "once per key");
}

#[test]
fn a_key_changes_lateness_is_measured_while_playing_only() {
    let listing = songs();
    let map = build(&listing.cues, &listing.clips);
    // 120 bpm: a tick ahead (0.07 beats) passes the clip at 50.
    let note = late_note(&map, 1, 49.95, true, Some(120.0)).expect("a crossing");
    assert_eq!(
        (note.song, note.at, note.change, note.tempo),
        (1, 49.95, 50.0, 120.0)
    );
    let past = late_note(&map, 1, 50.5, true, Some(90.0)).expect("past it");
    assert_eq!((past.at, past.change, past.tempo), (50.5, 50.0, 90.0));
    assert_eq!(
        late_note(&map, 1, 60.0, false, Some(120.0)),
        None,
        "stopped"
    );
    assert_eq!(late_note(&map, 1, 60.0, true, None), None, "no tempo");
    assert_eq!(
        late_note(&map, 1, 10.0, true, Some(120.0)),
        None,
        "before it"
    );
    assert_eq!(
        late_note(&map, 3, 250.0, true, Some(120.0)),
        None,
        "no change"
    );
}

#[tokio::test]
async fn a_song_start_writes_every_key_device_and_the_bar() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    router.handle(layout(1, vec![pa(), stream()]));
    find(&mut router, PA, rack(6, ""));
    find(&mut router, STREAM, rack(7, ""));
    // The PA's Auto-Tune on G# Chromatic, the stream's slots on C.
    heard(&mut router, PA, (0.75, 2.0 / 28.0));
    heard(&mut router, STREAM, (0.0, 0.0));
    map_read(&mut router, 1, songs());
    let _ = steps(&rx);
    no_call(&mut script);
    // Song 1 starts: D, every key device and the bar in one call through
    // their `$ref`s; the stream's Scale already holds Major's centre, so it
    // is written a nudge (0.05 of a step) beside it.
    deliver(&mut router, TIME, 10.0);
    let write = call(&mut script);
    let [root, scale] = set_bar(2, "Major");
    assert_eq!(
        write.commands,
        vec![
            set_ref("k6", 2.0 / 11.0),
            set_ref("s6", 0.0),
            set_ref("k7", 2.0 / 11.0),
            set_ref("s7", 0.05 / 28.0),
            root,
            scale
        ]
    );
    assert_eq!(
        of(&steps(&rx), "key_write"),
        vec![key_write("D", "song", &["Vox 2", "#8"], true)]
    );
    assert_eq!(
        states(&mut router),
        vec![Some(KeyState::Writing), Some(KeyState::Writing)]
    );
    let set = set_by(&write.commands);
    written(&mut router, &mut own, write, vec![ok(Value::Null); 6]).await;
    // The read-back: every value through its `$ref`, and Live's bar.
    let read = call(&mut script);
    let [root, scale] = get_bar();
    assert_eq!(
        read.commands,
        vec![
            get_ref("k6"),
            get_ref("s6"),
            get_ref("k7"),
            get_ref("s7"),
            root,
            scale
        ]
    );
    let answer = read_from(&read, &set);
    read_back(&mut router, &mut own, read, answer).await;
    let got = steps(&rx);
    assert_eq!(
        of(&got, "readback"),
        vec![
            readback(6, "D", "confirmed", Some("D"), None),
            readback(7, "D", "confirmed", Some("D"), None)
        ]
    );
    assert_eq!(of(&got, "bar"), vec![bar("D", "confirmed", Some("D"))]);
    assert_eq!(of(&got, "late"), Vec::<Value>::new(), "a song start");
    let now = status(&mut router).autotune;
    let keys: Vec<_> = now
        .targets
        .iter()
        .map(|t| {
            let k = t.key.as_ref().unwrap();
            (k.want.as_deref(), k.state, k.values)
        })
        .collect();
    assert_eq!(
        keys,
        vec![
            (
                Some("D"),
                Some(KeyState::Confirmed),
                Some([2.0 / 11.0, 0.0])
            ),
            (
                Some("D"),
                Some(KeyState::Confirmed),
                Some([2.0 / 11.0, 0.05 / 28.0])
            )
        ]
    );
    assert_eq!(now.keys[0].bar.as_deref(), Some("D"));
    no_call(&mut script);
    // The same song's position again: nothing more.
    deliver(&mut router, TIME, 11.0);
    no_call(&mut script);
}

#[tokio::test]
async fn an_equal_key_at_the_next_song_start_is_written_nudged() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    ready_stream(&mut router, (0.5, 1.0 / 28.0));
    deliver(&mut router, TIME, 10.0);
    let [root, scale] = set_bar(2, "Major");
    assert_eq!(
        confirm(&mut router, &mut own, &mut script).await,
        vec![
            set_ref("k7", 2.0 / 11.0),
            set_ref("s7", 0.0),
            root.clone(),
            scale.clone()
        ]
    );
    // Song 2 is in D too: Live holds both centres, so each goes a nudge
    // beside its centre.
    deliver(&mut router, TIME, 130.0);
    assert_eq!(
        confirm(&mut router, &mut own, &mut script).await,
        vec![
            set_ref("k7", 2.05 / 11.0),
            set_ref("s7", 0.05 / 28.0),
            root.clone(),
            scale.clone()
        ]
    );
    // Back into song 1: the centre again.
    deliver(&mut router, TIME, 20.0);
    assert_eq!(
        confirm(&mut router, &mut own, &mut script).await,
        vec![set_ref("k7", 2.0 / 11.0), set_ref("s7", 0.0), root, scale]
    );
    let got = steps(&rx);
    assert_eq!(
        of(&got, "key_write"),
        vec![
            key_write("D", "song", &["#8"], true),
            key_write("D", "song", &["#8"], true),
            key_write("D", "song", &["#8"], true)
        ]
    );
    assert_eq!(of(&got, "readback").len(), 3);
    assert_eq!(states(&mut router), vec![Some(KeyState::Confirmed)]);
}

#[tokio::test]
async fn a_refused_write_is_a_mismatch_retried_once() {
    // The PA's Auto-Tune held by a Max map as on the PC: Live refuses both
    // its parameters with its exact words.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    router.handle(layout(1, vec![pa(), stream()]));
    find(&mut router, PA, rack(6, ""));
    find(&mut router, STREAM, rack(7, ""));
    heard(&mut router, PA, (0.75, 2.0 / 28.0));
    heard(&mut router, STREAM, (0.5, 0.0));
    map_read(&mut router, 1, songs());
    deliver(&mut router, TIME, 10.0);
    let _ = steps(&rx);
    let write = call(&mut script);
    let null = ok(Value::Null);
    written(
        &mut router,
        &mut own,
        write,
        vec![
            disabled(),
            disabled(),
            null.clone(),
            null.clone(),
            null.clone(),
            null.clone(),
        ],
    )
    .await;
    let got = steps(&rx);
    assert_eq!(
        of(&got, "readback"),
        vec![readback(6, "D", "refused", None, Some(DISABLED))]
    );
    assert_eq!(
        of(&got, "key_write"),
        vec![key_write("D", "retry", &["Vox 2"], false)]
    );
    // The stream and the bar are read back; the PA is written once more.
    let read = call(&mut script);
    let [root, scale] = get_bar();
    assert_eq!(
        read.commands,
        vec![get_ref("k7"), get_ref("s7"), root, scale]
    );
    let answer = vec![
        ok(json!(2.0 / 11.0)),
        ok(json!(0.0)),
        ok(json!(2)),
        ok(json!("Major")),
    ];
    read_back(&mut router, &mut own, read, answer).await;
    assert_eq!(
        of(&steps(&rx), "readback"),
        vec![readback(7, "D", "confirmed", Some("D"), None)]
    );
    let retry = call(&mut script);
    assert_eq!(
        retry.commands,
        vec![set_ref("k6", 2.0 / 11.0), set_ref("s6", 0.0)]
    );
    written(&mut router, &mut own, retry, vec![disabled(), disabled()]).await;
    assert_eq!(
        of(&steps(&rx), "readback"),
        vec![readback(6, "D", "refused", None, Some(DISABLED))]
    );
    no_call(&mut script);
    assert_eq!(
        states(&mut router),
        vec![Some(KeyState::Refused), Some(KeyState::Confirmed)]
    );
    // The key clip (a jump past it): another key, its own retry; never a
    // write loop.
    deliver(&mut router, TIME, 60.0);
    let write = call(&mut script);
    assert_eq!(write.commands.len(), 6);
    written(
        &mut router,
        &mut own,
        write,
        vec![
            disabled(),
            disabled(),
            null.clone(),
            null.clone(),
            null.clone(),
            null.clone(),
        ],
    )
    .await;
    let read = call(&mut script);
    let answer = vec![
        ok(json!(7.0 / 11.0)),
        ok(json!(0.0)),
        ok(json!(7)),
        ok(json!("Major")),
    ];
    read_back(&mut router, &mut own, read, answer).await;
    let retry = call(&mut script);
    written(&mut router, &mut own, retry, vec![disabled(), disabled()]).await;
    no_call(&mut script);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "key_write"),
        vec![
            key_write("G", "change", &["Vox 2", "#8"], true),
            key_write("G", "retry", &["Vox 2"], false)
        ]
    );
    assert_eq!(
        of(&got, "readback")
            .iter()
            .filter(|r| r["state"] == "refused")
            .count(),
        2
    );
    assert_eq!(states(&mut router)[0], Some(KeyState::Refused));
}

#[tokio::test]
async fn a_wrong_read_back_is_a_mismatch_retried_once() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    ready_stream(&mut router, (0.5, 1.0 / 28.0));
    deliver(&mut router, TIME, 10.0);
    let _ = steps(&rx);
    let write = call(&mut script);
    written(&mut router, &mut own, write, vec![ok(Value::Null); 4]).await;
    // Live's slot and bar show C, not D.
    let read = call(&mut script);
    let c = vec![
        ok(json!(0.0)),
        ok(json!(0.0)),
        ok(json!(0)),
        ok(json!("Major")),
    ];
    read_back(&mut router, &mut own, read, c).await;
    let got = steps(&rx);
    assert_eq!(
        of(&got, "readback"),
        vec![readback(7, "D", "mismatch", Some("C"), None)]
    );
    assert_eq!(of(&got, "bar"), vec![bar("D", "shows", Some("C"))]);
    assert_eq!(
        of(&got, "key_write"),
        vec![key_write("D", "retry", &["#8"], false)]
    );
    assert_eq!(
        status(&mut router).autotune.keys[0].bar.as_deref(),
        Some("C")
    );
    // The retry, against what was read back: the Scale's centre is there.
    let retry = call(&mut script);
    assert_eq!(
        retry.commands,
        vec![set_ref("k7", 2.0 / 11.0), set_ref("s7", 0.05 / 28.0)]
    );
    written(&mut router, &mut own, retry, vec![ok(Value::Null); 2]).await;
    let read = call(&mut script);
    assert_eq!(read.commands, vec![get_ref("k7"), get_ref("s7")]);
    // A Chromatic scale: no key at all.
    let chromatic = vec![ok(json!(2.0 / 11.0)), ok(json!(2.0 / 28.0))];
    read_back(&mut router, &mut own, read, chromatic).await;
    assert_eq!(
        of(&steps(&rx), "readback"),
        vec![readback(7, "D", "mismatch", None, None)]
    );
    no_call(&mut script);
    assert_eq!(states(&mut router), vec![Some(KeyState::Mismatch)]);
}

#[tokio::test]
async fn another_writer_is_recorded_and_not_fought() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    ready_stream(&mut router, (0.5, 1.0 / 28.0));
    deliver(&mut router, TIME, 10.0);
    let write = call(&mut script);
    // While the write is on its way, Live's values are no other writer's.
    heard(&mut router, STREAM, (2.0 / 11.0, 0.0));
    heard(&mut router, STREAM, (0.0, 0.0));
    let n = write.commands.len();
    let commands = write.commands.clone();
    written(&mut router, &mut own, write, vec![ok(Value::Null); n]).await;
    let read = call(&mut script);
    let answer = read_from(&read, &set_by(&commands));
    read_back(&mut router, &mut own, read, answer).await;
    let _ = steps(&rx);
    // Its own key heard again (a listener's echo): nothing.
    heard(&mut router, STREAM, (2.0 / 11.0, 0.0));
    assert_eq!(of(&steps(&rx), "another_writer"), Vec::<Value>::new());
    // Someone turns the slot to A: shown, once, never written against.
    let key_sub = router.autotune.targets[STREAM].key_sub.clone().unwrap();
    deliver(&mut router, &key_sub, 9.0 / 11.0);
    deliver(&mut router, &key_sub, 10.0 / 11.0);
    assert_eq!(
        of(&steps(&rx), "another_writer"),
        vec![json!({"step": "another_writer", "instance": "band",
                    "target": "live_set tracks 7", "shows": "A"})]
    );
    no_call(&mut script);
    assert_eq!(states(&mut router), vec![Some(KeyState::AnotherWriter)]);
    // The next song start writes as usual.
    deliver(&mut router, TIME, 130.0);
    assert_eq!(states(&mut router), vec![Some(KeyState::Writing)]);
    confirm(&mut router, &mut own, &mut script).await;
    assert_eq!(states(&mut router), vec![Some(KeyState::Confirmed)]);
}

#[tokio::test]
async fn a_key_change_records_its_lateness() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    ready_stream(&mut router, (0.5, 1.0 / 28.0));
    transport(&mut router, true);
    deliver(&mut router, TEMPO_KEY, 120.0);
    deliver(&mut router, TIME, 10.0);
    confirm(&mut router, &mut own, &mut script).await;
    // Live plays on to just before the clip (one script tick a delivery).
    deliver(&mut router, TIME, 49.9);
    no_call(&mut script);
    let _ = steps(&rx);
    // A tick before the clip at 50, at 120 bpm: G is written 25 ms before
    // it, plus the write's round trip (within what this test measured).
    let started = std::time::Instant::now();
    deliver(&mut router, TIME, 49.95);
    let write = call(&mut script);
    let n = write.commands.len();
    written(&mut router, &mut own, write, vec![ok(Value::Null); n]).await;
    let rtt = elapsed_ms(started.elapsed());
    let got = steps(&rx);
    assert_eq!(
        of(&got, "key_write"),
        vec![key_write("G", "change", &["#8"], true)]
    );
    let late = of(&got, "late");
    assert_eq!(late.len(), 1, "{got:?}");
    assert_eq!(
        (&late[0]["instance"], &late[0]["song"]),
        (&json!("band"), &json!(1))
    );
    let ms = late[0]["ms"].as_f64().unwrap();
    assert!(
        (-25.0 - 1e-6..=-25.0 + rtt + 1e-6).contains(&ms),
        "{ms} (rtt {rtt})"
    );
    let read = call(&mut script);
    let answer = vec![
        ok(json!(7.0 / 11.0)),
        ok(json!(0.0)),
        ok(json!(7)),
        ok(json!("Major")),
    ];
    read_back(&mut router, &mut own, read, answer).await;
    // A jump back before the clip while playing: a change with no
    // lateness (no change in force there).
    deliver(&mut router, TIME, 20.0);
    confirm(&mut router, &mut own, &mut script).await;
    // Stopped, a jump past the clip: no crossing, no lateness.
    transport(&mut router, false);
    deliver(&mut router, TIME, 60.0);
    confirm(&mut router, &mut own, &mut script).await;
    let got = steps(&rx);
    assert_eq!(
        of(&got, "key_write"),
        vec![
            key_write("D", "change", &["#8"], true),
            key_write("G", "change", &["#8"], true)
        ]
    );
    assert_eq!(of(&got, "late"), Vec::<Value>::new());
}

#[tokio::test]
async fn a_jump_past_a_key_clip_while_playing_records_no_lateness() {
    // PR #144 review M1, as ruled: lateness is a clip crossed in continuous
    // play; a jump or a locate past it, playing or not, records none (a
    // jump 10 beats past the clip at 120 bpm would read "5000 ms late").
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    ready_stream(&mut router, (0.5, 1.0 / 28.0));
    transport(&mut router, true);
    deliver(&mut router, TEMPO_KEY, 120.0);
    deliver(&mut router, TIME, 10.0);
    confirm(&mut router, &mut own, &mut script).await;
    let _ = steps(&rx);
    deliver(&mut router, TIME, 60.0);
    confirm(&mut router, &mut own, &mut script).await;
    let got = steps(&rx);
    assert_eq!(
        of(&got, "key_write"),
        vec![key_write("G", "change", &["#8"], true)]
    );
    assert_eq!(of(&got, "late"), Vec::<Value>::new(), "a jump");
    // A locate back before the clip, then Live plays across it: a crossing.
    deliver(&mut router, TIME, 49.9);
    confirm(&mut router, &mut own, &mut script).await;
    deliver(&mut router, TIME, 49.95);
    confirm(&mut router, &mut own, &mut script).await;
    let got = steps(&rx);
    assert_eq!(
        of(&got, "key_write"),
        vec![
            key_write("D", "change", &["#8"], true),
            key_write("G", "change", &["#8"], true)
        ]
    );
    assert_eq!(of(&got, "late").len(), 1, "a crossing: {got:?}");
}

#[tokio::test]
async fn no_tempo_heard_means_no_lateness() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    ready_stream(&mut router, (0.5, 1.0 / 28.0));
    transport(&mut router, true);
    deliver(&mut router, TIME, 10.0);
    confirm(&mut router, &mut own, &mut script).await;
    deliver(&mut router, TIME, 50.5);
    confirm(&mut router, &mut own, &mut script).await;
    let got = steps(&rx);
    assert_eq!(of(&got, "key_write").len(), 2);
    assert_eq!(of(&got, "late"), Vec::<Value>::new());
}

#[tokio::test]
async fn writes_are_held_while_the_markers_are_unsettled() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    ready_stream(&mut router, (0.5, 1.0 / 28.0));
    deliver(&mut router, TIME, 10.0);
    confirm(&mut router, &mut own, &mut script).await;
    let _ = steps(&rx);
    // A track-list change: the set is held; song 3 starts meanwhile.
    router.autotune_tracks("band");
    deliver(&mut router, TIME, 250.0);
    no_call(&mut script);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "held"),
        vec![json!({"step": "held", "instance": "band", "on": true, "why": "tracks"})]
    );
    assert_eq!(of(&got, "key_write"), Vec::<Value>::new());
    assert_eq!(of(&got, "key").len(), 1, "the key in force still follows");
    // The release walks the target again; its answer writes F#m once, the
    // bar with it (owed since the held song start).
    let walk = router.autotune.targets[STREAM].walk;
    router.autotune_release("band", "settled");
    assert!(router.autotune.targets[STREAM].walk > walk, "walked again");
    no_call(&mut script);
    find(&mut router, STREAM, rack(7, ""));
    let [root, scale] = set_bar(6, "Minor");
    assert_eq!(
        confirm(&mut router, &mut own, &mut script).await,
        vec![
            set_ref("k7", 6.0 / 11.0),
            set_ref("s7", 1.0 / 28.0),
            root,
            scale
        ]
    );
    // Found again: nothing more.
    find(&mut router, STREAM, rack(7, ""));
    no_call(&mut script);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "key_write"),
        vec![key_write("F#m", "found", &["#8"], true)]
    );
    assert_eq!(states(&mut router), vec![Some(KeyState::Confirmed)]);
    assert_eq!(
        status(&mut router).autotune.keys[0].bar.as_deref(),
        Some("F#m")
    );
}

/// The band connected (a set loaded, a Live started).
fn connect(router: &mut Router) {
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Connected(crate::live::ConnectInfo {
            instance: "band".into(),
            set_name: "set".into(),
            script_version: "0".into(),
            live_version: "12".into(),
        }),
    });
}

/// The marker keeper's next read of the band (its one track's `devices`),
/// unanswered; every other call before it is passed over (and fails).
fn markers_call(script: &mut Script) -> Option<Asked> {
    let devices = json!({"target": "live_set tracks 0", "name": "get_prop",
                         "args": {"prop": "devices"}});
    while let Some(asked) = script.try_call() {
        if asked.commands == vec![devices.clone()] {
            return Some(asked);
        }
    }
    None
}

/// Live answers the marker keeper's read: no device on its one track.
async fn markers_answered(
    router: &mut Router,
    own: &mut UnboundedReceiver<RouterMsg>,
    script: &mut Script,
) {
    markers_call(script)
        .expect("a read of the markers")
        .answer(vec![ok(json!([]))]);
    let msg = next_of(own, "the markers' read", |m| {
        matches!(m, RouterMsg::MarkersRead { .. })
    })
    .await;
    router.handle(msg);
}

#[tokio::test(start_paused = true)]
async fn a_slow_set_load_writes_the_key_once_the_markers_read_answers() {
    // PR #145 review I1: a set load (a disconnect, then a connect while Live
    // still loads the set) holds the band's writes until a read of the
    // marker keeper answers, and that read fails for as long as Live loads:
    // here five times, more than the keeper's three retries of 1.12.0. The
    // song map and the walks recover meanwhile; the key must too.
    let dir = tempfile::tempdir().unwrap();
    let (router, mut own, rx, mut script) = scripted_router(dir.path());
    let store = std::sync::Arc::new(crate::layout::store_in(
        dir.path(),
        std::path::Path::new("layout.json"),
        vec!["band".into()],
    ));
    let mut router = router.with_layout(store);
    connect(&mut router);
    ready_stream(&mut router, (0.5, 1.0 / 28.0));
    // The keeper knows the band's one track and reads it whole.
    let actions = router.markers.value(
        "band|live_set|tracks|false",
        Some(&json!([{"$ref": "t0", "name": "Bass #"}])),
    );
    router.markers_apply(actions);
    markers_answered(&mut router, &mut own, &mut script).await;
    deliver(&mut router, TIME, 10.0);
    confirm(&mut router, &mut own, &mut script).await;
    let _ = steps(&rx);
    // The set load: offline, then connected while Live still loads.
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    connect(&mut router);
    map_read(&mut router, 2, songs());
    find(&mut router, STREAM, rack(7, ""));
    heard(&mut router, STREAM, (2.0 / 11.0, 0.0));
    // Song 3 starts: held, nothing written.
    deliver(&mut router, TIME, 250.0);
    // The keeper's read and its retries fail five times.
    for n in 1..=5 {
        let read = markers_call(&mut script).unwrap_or_else(|| panic!("read {n}"));
        drop(read);
        let failed = next_of(&mut own, "the read's failure", |m| {
            matches!(m, RouterMsg::MarkersRead { .. })
        })
        .await;
        router.handle(failed);
        let retry = next_of(&mut own, "the read's retry", |m| {
            matches!(m, RouterMsg::MarkersRetry { .. })
        })
        .await;
        router.handle(retry);
    }
    let got = steps(&rx);
    assert_eq!(of(&got, "key_write"), Vec::<Value>::new(), "{got:?}");
    assert_eq!(
        of(&got, "held"),
        vec![json!({"step": "held", "instance": "band", "on": true, "why": "offline"})]
    );
    // Live answers the sixth: the hold ends, the target is walked again,
    // and its answer writes F#m once, the bar with it.
    markers_answered(&mut router, &mut own, &mut script).await;
    find(&mut router, STREAM, rack(7, ""));
    let [root, scale] = set_bar(6, "Minor");
    assert_eq!(
        confirm(&mut router, &mut own, &mut script).await,
        vec![
            set_ref("k7", 6.0 / 11.0),
            set_ref("s7", 1.0 / 28.0),
            root,
            scale
        ]
    );
    find(&mut router, STREAM, rack(7, ""));
    no_call(&mut script);
    let got = steps(&rx);
    assert_eq!(
        of(&got, "held"),
        vec![json!({"step": "held", "instance": "band", "on": false, "why": "settled"})]
    );
    assert_eq!(
        of(&got, "key_write"),
        vec![key_write("F#m", "found", &["#8"], true)]
    );
    assert_eq!(states(&mut router), vec![Some(KeyState::Confirmed)]);
}

#[tokio::test]
async fn a_found_target_waits_for_both_its_values_with_the_bar_it_owes() {
    // A held song start owes the key and the bar; after the release the
    // walk finds another device, whose values come one at a time: nothing
    // goes out (not even the bar alone) until both are heard.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    ready_stream(&mut router, (0.5, 1.0 / 28.0));
    deliver(&mut router, TIME, 10.0);
    confirm(&mut router, &mut own, &mut script).await;
    router.autotune_tracks("band");
    deliver(&mut router, TIME, 250.0);
    router.autotune_release("band", "settled");
    find(&mut router, STREAM, rack(7, "b"));
    no_call(&mut script);
    let key_sub = router.autotune.targets[STREAM].key_sub.clone().unwrap();
    deliver(&mut router, &key_sub, 0.5);
    no_call(&mut script);
    let _ = steps(&rx);
    let scale_sub = router.autotune.targets[STREAM].scale_sub.clone().unwrap();
    deliver(&mut router, &scale_sub, 0.0);
    let [root, scale] = set_bar(6, "Minor");
    assert_eq!(
        confirm(&mut router, &mut own, &mut script).await,
        vec![
            set_ref("k7b", 6.0 / 11.0),
            set_ref("s7b", 1.0 / 28.0),
            root,
            scale
        ]
    );
    assert_eq!(
        of(&steps(&rx), "key_write"),
        vec![key_write("F#m", "found", &["#8"], true)]
    );
}

#[tokio::test]
async fn a_track_inserted_above_a_target_is_no_other_writer_and_writes_nothing_by_the_old_index() {
    // PR 2a review M3 (the main session's ruling): the key device is
    // followed by its index path, so after a track-list change the values
    // heard may be another track's until the walk confirmed the device.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    ready_stream(&mut router, (0.5, 1.0 / 28.0));
    deliver(&mut router, TIME, 10.0);
    confirm(&mut router, &mut own, &mut script).await;
    let _ = steps(&rx);
    router.autotune_tracks("band");
    // Held: another track's values at the index path are no other writer.
    heard(&mut router, STREAM, (9.0 / 11.0, 0.0));
    // Released, its walk not answered yet: still none.
    router.autotune_release("band", "settled");
    heard(&mut router, STREAM, (10.0 / 11.0, 0.0));
    let got = steps(&rx);
    assert_eq!(of(&got, "another_writer"), Vec::<Value>::new(), "{got:?}");
    no_call(&mut script);
    // The walk finds the same device: confirmed again. Its values were
    // only another track's, so it is written nothing (no key change, its
    // `$ref`s written already).
    find(&mut router, STREAM, rack(7, ""));
    no_call(&mut script);
    // From now on a value off its key is another writer.
    heard(&mut router, STREAM, (9.0 / 11.0, 0.0));
    assert_eq!(of(&steps(&rx), "another_writer").len(), 1);
    // A track inserted again, and this time the walk finds another device
    // at the index (the rack of the track now there): it is followed
    // afresh and gets the key in force through its own `$ref`s, only once
    // the hold is over and its values are heard.
    let _ = steps(&rx);
    router.autotune_tracks("band");
    find(&mut router, STREAM, rack(7, "x"));
    heard(&mut router, STREAM, (5.0 / 11.0, 0.0));
    no_call(&mut script);
    let got = steps(&rx);
    assert_eq!(of(&got, "key_write"), Vec::<Value>::new(), "{got:?}");
    assert_eq!(of(&got, "another_writer"), Vec::<Value>::new());
    router.autotune_release("band", "settled");
    find(&mut router, STREAM, rack(7, "x"));
    let commands = confirm(&mut router, &mut own, &mut script).await;
    assert_eq!(
        commands,
        vec![set_ref("k7x", 2.0 / 11.0), set_ref("s7x", 0.05 / 28.0)]
    );
    assert_eq!(
        of(&steps(&rx), "key_write"),
        vec![key_write("D", "found", &["#8"], false)]
    );
}

#[tokio::test]
async fn a_stale_ref_walks_the_target_again_and_writes_it_found() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    ready_stream(&mut router, (0.5, 1.0 / 28.0));
    deliver(&mut router, TIME, 10.0);
    let walk = router.autotune.targets[STREAM].walk;
    let write = call(&mut script);
    let stale = json!({"ok": false, "error": "stale ref", "errorType": "StaleRef"});
    let null = ok(Value::Null);
    written(
        &mut router,
        &mut own,
        write,
        vec![stale.clone(), null.clone(), null.clone(), null.clone()],
    )
    .await;
    assert!(router.autotune.targets[STREAM].walk > walk, "walked again");
    let got = steps(&rx);
    assert_eq!(
        of(&got, "readback"),
        vec![readback(7, "D", "refused", None, Some("stale ref"))]
    );
    // No retry through the stale ref: only the bar is read back.
    let read = call(&mut script);
    assert_eq!(read.commands, get_bar().to_vec());
    read_back(
        &mut router,
        &mut own,
        read,
        vec![ok(json!(2)), ok(json!("Major"))],
    )
    .await;
    no_call(&mut script);
    // The walk finds the device anew (another plug-in in its place): it is
    // written once its values are heard.
    find(&mut router, STREAM, rack(7, "b"));
    no_call(&mut script);
    heard(&mut router, STREAM, (0.5, 0.0));
    assert_eq!(
        confirm(&mut router, &mut own, &mut script).await,
        vec![set_ref("k7b", 2.0 / 11.0), set_ref("s7b", 0.05 / 28.0)]
    );
    let got = steps(&rx);
    assert_eq!(
        of(&got, "key_write"),
        vec![key_write("D", "found", &["#8"], false)]
    );
    assert_eq!(states(&mut router), vec![Some(KeyState::Confirmed)]);
}

#[tokio::test]
async fn a_write_to_a_replaced_device_is_void_there() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    ready_stream(&mut router, (0.5, 1.0 / 28.0));
    deliver(&mut router, TIME, 10.0);
    let write = call(&mut script);
    // Before Live answers, the walk finds another plug-in in its place.
    find(&mut router, STREAM, rack(7, "b"));
    let _ = steps(&rx);
    written(&mut router, &mut own, write, vec![ok(Value::Null); 4]).await;
    // Only the bar is read back: the old device's write is void.
    let read = call(&mut script);
    assert_eq!(read.commands, get_bar().to_vec());
    read_back(
        &mut router,
        &mut own,
        read,
        vec![ok(json!(2)), ok(json!("Major"))],
    )
    .await;
    let got = steps(&rx);
    assert_eq!(of(&got, "readback"), Vec::<Value>::new());
    assert_eq!(of(&got, "bar"), vec![bar("D", "confirmed", Some("D"))]);
    assert_eq!(states(&mut router), vec![Some(KeyState::NoKey)]);
    // Its values heard: the new device gets D through its own refs.
    heard(&mut router, STREAM, (0.5, 0.0));
    assert_eq!(
        confirm(&mut router, &mut own, &mut script).await,
        vec![set_ref("k7b", 2.0 / 11.0), set_ref("s7b", 0.05 / 28.0)]
    );
}

#[tokio::test]
async fn an_older_writes_answer_is_void() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    ready_stream(&mut router, (0.5, 1.0 / 28.0));
    deliver(&mut router, TIME, 10.0);
    let first = call(&mut script);
    // Song 3 starts before Live answered the first write.
    deliver(&mut router, TIME, 250.0);
    let second = call(&mut script);
    let n = second.commands.len();
    let commands = second.commands.clone();
    written(&mut router, &mut own, second, vec![ok(Value::Null); n]).await;
    let read = call(&mut script);
    let answer = read_from(&read, &set_by(&commands));
    read_back(&mut router, &mut own, read, answer).await;
    let _ = steps(&rx);
    // The first write's late answer: void (no record, no retry, no read).
    written(&mut router, &mut own, first, vec![disabled(); 4]).await;
    assert_eq!(steps(&rx), Vec::<Value>::new());
    no_call(&mut script);
    let now = status(&mut router).autotune;
    let key = now.targets[0].key.as_ref().unwrap();
    assert_eq!(
        (key.want.as_deref(), key.state),
        (Some("F#m"), Some(KeyState::Confirmed))
    );
    assert_eq!(now.keys[0].bar.as_deref(), Some("F#m"));
}

#[tokio::test]
async fn no_key_in_force_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx, mut script) = scripted_router(dir.path());
    ready_stream(&mut router, (0.5, 1.0 / 28.0));
    // Out of every song, then a keyless song: no key in force.
    deliver(&mut router, TIME, 110.0);
    deliver(&mut router, TIME, 350.0);
    no_call(&mut script);
    assert_eq!(of(&steps(&rx), "key_write"), Vec::<Value>::new());
    assert_eq!(states(&mut router), vec![Some(KeyState::NoKey)]);
}

#[tokio::test]
async fn a_target_found_after_the_song_start_gets_the_key_once_its_values_are_heard() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    router.handle(layout(1, vec![pa(), stream()]));
    find(&mut router, STREAM, rack(7, ""));
    heard(&mut router, STREAM, (0.5, 1.0 / 28.0));
    map_read(&mut router, 1, songs());
    deliver(&mut router, TIME, 10.0);
    // Only the stream is found: it and the bar.
    let [root, scale] = set_bar(2, "Major");
    assert_eq!(
        confirm(&mut router, &mut own, &mut script).await,
        vec![set_ref("k7", 2.0 / 11.0), set_ref("s7", 0.0), root, scale]
    );
    // The PA is found later; it waits for its values, then gets D alone.
    find(&mut router, PA, rack(6, ""));
    no_call(&mut script);
    let key_sub = router.autotune.targets[PA].key_sub.clone().unwrap();
    deliver(&mut router, &key_sub, 0.75);
    no_call(&mut script);
    let scale_sub = router.autotune.targets[PA].scale_sub.clone().unwrap();
    deliver(&mut router, &scale_sub, 0.0);
    assert_eq!(
        confirm(&mut router, &mut own, &mut script).await,
        vec![set_ref("k6", 2.0 / 11.0), set_ref("s6", 0.05 / 28.0)]
    );
    // Its values again, and found again: nothing more.
    heard(&mut router, PA, (2.0 / 11.0, 0.05 / 28.0));
    find(&mut router, PA, rack(6, ""));
    no_call(&mut script);
    assert_eq!(
        of(&steps(&rx), "key_write"),
        vec![
            key_write("D", "song", &["#8"], true),
            key_write("D", "found", &["Vox 2"], false)
        ]
    );
    assert_eq!(
        states(&mut router),
        vec![Some(KeyState::Confirmed), Some(KeyState::Confirmed)]
    );
}

#[tokio::test]
async fn no_key_is_written_from_the_map_before_the_connect() {
    // PR 2a review M1 stays true with the writes: a set loaded meanwhile
    // has other songs, so a reconnect writes nothing until its connect's
    // map answered.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx, mut script) = scripted_router(dir.path());
    ready_stream(&mut router, (0.5, 1.0 / 28.0));
    deliver(&mut router, TIME, 10.0);
    confirm(&mut router, &mut own, &mut script).await;
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    assert_eq!(states(&mut router), vec![None], "its device forgotten");
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Connected(crate::live::ConnectInfo {
            instance: "band".into(),
            set_name: "set".into(),
            script_version: "0".into(),
            live_version: "12".into(),
        }),
    });
    router.autotune_release("band", "settled");
    let _ = steps(&rx);
    deliver(&mut router, TIME, 10.0);
    find(&mut router, STREAM, rack(7, "c"));
    heard(&mut router, STREAM, (0.5, 1.0 / 28.0));
    no_call(&mut script);
    assert_eq!(of(&steps(&rx), "key_write"), Vec::<Value>::new());
    // The connect's map: a song start, written through the new refs.
    map_read(&mut router, 2, songs());
    let [root, scale] = set_bar(2, "Major");
    assert_eq!(
        confirm(&mut router, &mut own, &mut script).await,
        vec![set_ref("k7c", 2.0 / 11.0), set_ref("s7c", 0.0), root, scale]
    );
    assert_eq!(
        of(&steps(&rx), "key_write"),
        vec![key_write("D", "song", &["#8"], true)]
    );
}

#[tokio::test]
async fn no_selector_is_ever_written() {
    // Every write kind (a song start, a key change, a target found, a
    // retry) sends only key parameters' `$ref`s and Live's bar: `call`
    // checks each call, and no macro's write (`AutotuneWritten`) is made.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, _rx, mut script) = scripted_router(dir.path());
    router.handle(layout(1, vec![pa(), stream()]));
    find(&mut router, STREAM, rack(7, ""));
    heard(&mut router, STREAM, (0.5, 1.0 / 28.0));
    map_read(&mut router, 1, songs());
    transport(&mut router, true);
    deliver(&mut router, TEMPO_KEY, 120.0);
    deliver(&mut router, TIME, 10.0);
    confirm(&mut router, &mut own, &mut script).await;
    find(&mut router, PA, rack(6, ""));
    heard(&mut router, PA, (0.75, 0.0));
    confirm(&mut router, &mut own, &mut script).await;
    deliver(&mut router, TIME, 55.0);
    let write = call(&mut script);
    let mut all = vec![write.commands.clone()];
    let null = ok(Value::Null);
    written(
        &mut router,
        &mut own,
        write,
        vec![
            null.clone(),
            null.clone(),
            disabled(),
            null.clone(),
            null.clone(),
            null.clone(),
        ],
    )
    .await;
    while let Some(asked) = next_call(&mut script) {
        all.push(asked.commands.clone());
        let n = asked.commands.len();
        asked.answer(vec![ok(json!(0.0)); n]);
    }
    transport(&mut router, false);
    for commands in &all {
        no_selector(commands);
        for c in commands {
            assert_ne!(c["target"]["$ref"], json!("p6"), "{c}");
            assert_ne!(c["target"]["$ref"], json!("p7"), "{c}");
        }
    }
    // No macro write went out: none of its answers comes back.
    let quiet = tokio::time::timeout(std::time::Duration::from_millis(300), async {
        while let Some(msg) = own.recv().await {
            if matches!(msg, RouterMsg::AutotuneWritten { .. }) {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(!quiet, "a macro was written");
    assert_eq!(status(&mut router).autotune.writes, 0);
}
