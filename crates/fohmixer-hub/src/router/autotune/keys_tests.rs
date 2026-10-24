use std::collections::BTreeMap;

use fohmixer_proto::client::{
    AutotuneState, HUB_AUTOTUNE, HUB_AUTOTUNE_AUT, KeyInForce, KeyTargetStatus, ServerMsg,
    TargetState,
};
use fohmixer_proto::layout::{Anchor, AutotuneTarget, Binding};
use fohmixer_proto::songmap::{ClipName, Cue, Key, SectionsState, build};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

use super::super::super::songmap::Listing;
use super::super::super::tests::{attach, offline_router, offline_router_with, records, status};
use super::super::super::{Router, RouterMsg};
use super::super::same_macro;
use super::{
    CURRENT_SONG_TIME, KeyPart, Position, PositionPart, Route, TEMPO, VALUE, heard_number, key_now,
    key_status,
};
use crate::live::client::LiveEvent;
use crate::outbox::Outbox;
use crate::rules::HubState;
use crate::rules::autotune::find::{Found, KeyFound, Param};
use crate::rules::autotune::keys::{KeyDevice, KeyFollow, KeyMissing, KeyParams};
use crate::rules::reverb::Saved;

/// The stream target of the band (on 127).
const STREAM: &str = "band|live_set tracks 7";
/// The band's playhead, tempo and transport (hub keys).
const TIME: &str = "band|live_set|current_song_time|false";
const TEMPO_KEY: &str = "band|live_set|tempo|false";
const TRANSPORT: &str = "band|live_set|is_playing|false";
/// The stream rack's AudioGridder (its Latencies rack on the track).
const PLUGIN: &str = "live_set tracks 7 devices 0 chains 0 devices 0";
const MACRO: &str = "live_set tracks 7 devices 0 parameters 1";
const REFUSED: &str = "keys only until #140 PR 3";

fn stream() -> AutotuneTarget {
    AutotuneTarget {
        binding: Binding {
            instance: "band".into(),
            anchor: Anchor::TrackAt { index: 7 },
            path: None,
        },
        label: "#8".into(),
        on: 127,
        off: 0,
    }
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

/// The stream rack's AudioGridder, its slots' `$ref`s `k<n>` and `s<n>`.
fn gridder(n: u32) -> KeyFound {
    KeyFound {
        params: KeyParams {
            device: KeyDevice::Gridder,
            key: 1,
            scale: 2,
        },
        device: PLUGIN.into(),
        key: Param {
            path: format!("{PLUGIN} parameters 1"),
            id: format!("k{n}"),
        },
        scale: Param {
            path: format!("{PLUGIN} parameters 2"),
            id: format!("s{n}"),
        },
    }
}

/// The stream rack's macro with the key device `key`.
fn found_with(key: Result<KeyFound, KeyMissing>) -> Found {
    Found {
        path: MACRO.into(),
        id: "p7".into(),
        key,
    }
}

/// The stream target's newest walk answers `found`.
fn find(router: &mut Router, found: Found) {
    let walk = router.autotune.targets[STREAM].walk;
    router.autotune_found(STREAM, walk, Ok(found));
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

/// The `key` records among `got`.
fn keys_of(got: &[Value]) -> Vec<Value> {
    got.iter().filter(|r| r["step"] == "key").cloned().collect()
}

/// A `key` record of the band.
fn key(song: Option<u16>, key: Option<&str>, why: &str) -> Value {
    json!({"step": "key", "instance": "band", "song": song, "key": key, "why": why})
}

/// The `autotune` messages waiting in `outbox`.
fn told(outbox: &Outbox) -> Vec<AutotuneState> {
    outbox
        .take()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| match m {
            ServerMsg::Autotune(s) => Some(s),
            _ => None,
        })
        .collect()
}

fn set_hub(router: &mut Router, key: &str, value: Value) {
    router.handle(RouterMsg::SetHub {
        client: 1,
        key: key.into(),
        value,
    });
}

/// The band's transport heard, as the flush hands it over.
fn transport(router: &mut Router, playing: bool) {
    router.on_transport(TRANSPORT, &json!(playing));
}

/// A delivery of one of AUTOTUNE's subscriptions, as the flush hands it.
fn deliver(router: &mut Router, sub: &str, value: f64) {
    router.autotune_delivery(sub, Some(json!(value)), None);
}

/// Song 1 (`001 Test One {<song_key> 120}`) from 0 to its SONG END at 100,
/// a key clip to G at 50, song 2 in F#m from 120 to 200, and song 3 with no
/// key from 210 to 300 (invented titles).
fn songs(song_key: &str) -> Listing {
    let cue = |name: String, time: f64| Cue {
        name,
        time: Some(time),
    };
    Listing {
        cues: vec![
            cue(format!("001 Test One {{{song_key} 120}}"), 0.0),
            cue("SONG END".into(), 100.0),
            cue("002 Test Two {F#m 90}".into(), 120.0),
            cue("SONG END".into(), 200.0),
            cue("003 Test Three".into(), 210.0),
            cue("SONG END".into(), 300.0),
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

/// Live answers every listener of the band waiting (the table online), each
/// `value_of(target, prop)`; the `(target, prop)` asked.
fn answer(router: &mut Router, value_of: impl Fn(&str, &str) -> Value) -> Vec<(String, String)> {
    router.subs.connected("band");
    let mut asked = Vec::new();
    for o in router.subs.drain_outgoing() {
        let slots: Vec<Value> = o
            .commands
            .iter()
            .map(|c| {
                let target = c["target"].as_str().unwrap_or("").to_string();
                let prop = c["args"]["prop"].as_str().unwrap_or("").to_string();
                let live = format!("live_{}.{prop}", asked.len());
                let value = value_of(&target, &prop);
                asked.push((target, prop));
                json!({"ok": true, "data": {"key": live, "value": value}})
            })
            .collect();
        router.subs.on_result(&o.instance, &o.uuid, &slots);
    }
    asked
}

/// Whether a write of AUTOTUNE's reaches Live within 300 ms (its answer
/// comes back as `AutotuneWritten`).
async fn wrote(own: &mut UnboundedReceiver<RouterMsg>) -> bool {
    tokio::time::timeout(std::time::Duration::from_millis(300), async {
        while let Some(msg) = own.recv().await {
            if matches!(msg, RouterMsg::AutotuneWritten { .. }) {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false)
}

fn major(root: u8) -> Key {
    Key { root, minor: false }
}

#[test]
fn the_glues_names_and_small_decisions() {
    assert_eq!(
        (CURRENT_SONG_TIME, TEMPO, VALUE),
        ("current_song_time", "tempo", "value")
    );
    assert_eq!(heard_number(Some(&Ok(json!(2.5))), Some(1.0)), Some(2.5));
    assert_eq!(heard_number(Some(&Ok(json!("x"))), Some(1.0)), None);
    assert_eq!(heard_number(Some(&Err("gone".into())), Some(1.0)), None);
    assert_eq!(heard_number(None, Some(1.0)), Some(1.0));
    assert_eq!(heard_number(None, None), None);
    // The same macro is the same path and `$ref`; its key device does not
    // count.
    let was = found_with(Ok(gridder(1)));
    assert!(same_macro(
        Some(&was),
        &found_with(Err(KeyMissing::NoChain))
    ));
    assert!(!same_macro(None, &was));
    let moved = Found {
        path: "live_set tracks 8 devices 0 parameters 1".into(),
        id: "p7".into(),
        key: Err(KeyMissing::NoChain),
    };
    assert!(!same_macro(Some(&was), &moved), "another path");
    let replaced = Found {
        path: MACRO.into(),
        id: "p7b".into(),
        key: Err(KeyMissing::NoChain),
    };
    assert!(!same_macro(Some(&was), &replaced), "another ref");
}

#[test]
fn the_key_in_force_at_a_position_needs_a_map_and_a_position() {
    let listing = songs("C");
    let map = build(&listing.cues, &listing.clips);
    let at = |time: Option<f64>, playing: Option<bool>, tempo: Option<f64>| Position {
        time,
        playing,
        tempo,
        time_sub: None,
        tempo_sub: None,
        follow: KeyFollow::new(),
    };
    assert_eq!(
        key_now(None, &at(Some(10.0), None, None)),
        (None, None),
        "no map"
    );
    assert_eq!(
        key_now(Some(&map), &at(None, None, None)),
        (None, None),
        "no position"
    );
    assert_eq!(
        key_now(Some(&map), &at(Some(10.0), None, None)),
        (Some(1), Some(major(0)))
    );
    // One tick ahead only while known to play.
    let near = Some(49.95);
    assert_eq!(
        key_now(Some(&map), &at(near, Some(true), Some(120.0))),
        (Some(1), Some(major(7)))
    );
    assert_eq!(
        key_now(Some(&map), &at(near, Some(false), Some(120.0))),
        (Some(1), Some(major(0)))
    );
    assert_eq!(
        key_now(Some(&map), &at(near, None, Some(120.0))),
        (Some(1), Some(major(0)))
    );
    assert_eq!(
        key_now(Some(&map), &at(Some(110.0), None, None)),
        (None, None)
    );
    // A keyless song (003) is a song with no key in force.
    assert_eq!(
        key_now(Some(&map), &at(Some(250.0), None, None)),
        (Some(3), None)
    );
}

#[test]
fn a_key_device_in_the_status_shows_its_values_once_both_came() {
    let found = Ok(gridder(1));
    assert_eq!(
        key_status(&found, (Some(9.0 / 11.0), Some(1.0 / 28.0))),
        KeyTargetStatus {
            device: Some("gridder".into()),
            path: Some(PLUGIN.into()),
            values: Some([9.0 / 11.0, 1.0 / 28.0]),
            shows: Some("Am".into()),
            problem: None,
        }
    );
    let one = key_status(&found, (Some(0.5), None));
    assert_eq!((one.values, one.shows), (None, None));
    let other = key_status(&found, (None, Some(0.0)));
    assert_eq!((other.values, other.shows), (None, None));
    // Chromatic shows no key, its values stay.
    let chromatic = key_status(&found, (Some(0.0), Some(2.0 / 28.0)));
    assert_eq!(
        (chromatic.values, chromatic.shows),
        (Some([0.0, 2.0 / 28.0]), None)
    );
    let auto_tune = KeyFound {
        params: KeyParams {
            device: KeyDevice::AutoTune,
            key: 2,
            scale: 3,
        },
        device: PLUGIN.into(),
        key: gridder(2).key,
        scale: gridder(2).scale,
    };
    assert_eq!(
        key_status(&Ok(auto_tune), (Some(0.5), Some(0.0))).device,
        Some("auto_tune".into())
    );
    assert_eq!(
        key_status(
            &Err("chain 0 holds 2 plug-ins, not one".into()),
            (None, None)
        ),
        KeyTargetStatus {
            device: None,
            path: None,
            values: None,
            shows: None,
            problem: Some("chain 0 holds 2 plug-ins, not one".into()),
        }
    );
}

#[tokio::test]
async fn each_subscription_goes_to_its_own() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, _rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    find(&mut router, found_with(Ok(gridder(1))));
    let t = router.autotune.targets[STREAM].clone();
    let io = &router.autotune;
    assert_eq!(
        io.route(t.sub.as_deref().unwrap()),
        Some(Route::Macro(STREAM.into()))
    );
    assert_eq!(
        io.route(t.key_sub.as_deref().unwrap()),
        Some(Route::Key(STREAM.into(), KeyPart::Key))
    );
    assert_eq!(
        io.route(t.scale_sub.as_deref().unwrap()),
        Some(Route::Key(STREAM.into(), KeyPart::Scale))
    );
    assert_eq!(
        io.route(TIME),
        Some(Route::Position("band".into(), PositionPart::Time))
    );
    assert_eq!(
        io.route(TEMPO_KEY),
        Some(Route::Position("band".into(), PositionPart::Tempo))
    );
    assert_eq!(io.route("band|live_set|cue_points|false"), None);
    assert_eq!(io.route(TRANSPORT), None, "the rules' own");
}

#[tokio::test]
async fn a_found_target_follows_its_key_and_scale() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    find(&mut router, found_with(Ok(gridder(1))));
    assert_eq!(
        steps(&rx),
        vec![
            json!({"step": "found", "instance": "band", "target": "live_set tracks 7",
                   "path": MACRO}),
            json!({"step": "key_device", "instance": "band", "target": "live_set tracks 7",
                   "device": "gridder", "path": PLUGIN}),
        ]
    );
    assert_eq!(
        status(&mut router).autotune.targets[0].key,
        Some(KeyTargetStatus {
            device: Some("gridder".into()),
            path: Some(PLUGIN.into()),
            values: None,
            shows: None,
            problem: None,
        }),
        "no value heard yet"
    );
    assert_eq!(
        router.subs.subscriptions("band"),
        6,
        "the transport, the playhead and tempo, the macro, the Key and the Scale"
    );
    // Live answers the listeners: one at each of the key device's paths.
    let asked = answer(&mut router, |target, prop| match (target, prop) {
        (_, "is_playing") => json!(false),
        (_, "tempo") => json!(120.0),
        (_, "current_song_time") => json!(0.0),
        (MACRO, _) => json!(127.0),
        (t, _) if t.ends_with("parameters 1") => json!(9.0 / 11.0),
        _ => json!(1.0 / 28.0),
    });
    for path in [
        format!("{PLUGIN} parameters 1"),
        format!("{PLUGIN} parameters 2"),
    ] {
        let listeners = asked
            .iter()
            .filter(|(t, p)| *t == path && p == "value")
            .count();
        assert_eq!(listeners, 1, "{path}");
    }
    router.flush();
    let now = status(&mut router).autotune;
    assert_eq!(
        now.targets[0].key.as_ref().unwrap().shows.as_deref(),
        Some("Am")
    );
    assert_eq!(
        now.targets[0].key.as_ref().unwrap().values,
        Some([9.0 / 11.0, 1.0 / 28.0])
    );
    assert_eq!(now.targets[0].state, TargetState::On, "the macro as before");
    // A later value: the key it shows.
    let key_sub = router.autotune.targets[STREAM].key_sub.clone().unwrap();
    deliver(&mut router, &key_sub, 2.0 / 11.0);
    assert_eq!(
        status(&mut router).autotune.targets[0]
            .key
            .as_ref()
            .unwrap()
            .shows
            .as_deref(),
        Some("Dm")
    );
    // Found again with the same refs (a track-list change's walk): kept as
    // it is, nothing recorded.
    let _ = steps(&rx);
    router.autotune_walk(STREAM);
    find(&mut router, found_with(Ok(gridder(1))));
    assert_eq!(steps(&rx), Vec::<Value>::new());
    assert_eq!(router.subs.subscriptions("band"), 6);
    assert_eq!(router.autotune.targets[STREAM].values.0, Some(2.0 / 11.0));
    // Another device at the path (new refs): followed afresh.
    router.autotune_walk(STREAM);
    find(&mut router, found_with(Ok(gridder(2))));
    assert_eq!(
        steps(&rx),
        vec![
            json!({"step": "key_device", "instance": "band", "target": "live_set tracks 7",
                   "device": "gridder", "path": PLUGIN})
        ]
    );
    assert_eq!(router.subs.subscriptions("band"), 6);
    assert_eq!(router.autotune.targets[STREAM].values, (None, None));
}

#[tokio::test]
async fn a_missing_key_device_is_recorded_once_and_keeps_the_macro() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    find(&mut router, found_with(Err(KeyMissing::Plugins(2))));
    let missing = |problem: &str| {
        json!({"step": "key_missing", "instance": "band", "target": "live_set tracks 7",
               "problem": problem})
    };
    let got = steps(&rx);
    assert_eq!(got[0]["step"], "found");
    assert_eq!(&got[1..], [missing("chain 0 holds 2 plug-ins, not one")]);
    // The second walk finds the same: no news.
    router.autotune_walk(STREAM);
    find(&mut router, found_with(Err(KeyMissing::Plugins(2))));
    assert_eq!(steps(&rx), Vec::<Value>::new());
    assert_eq!(
        router.subs.subscriptions("band"),
        4,
        "the transport, the playhead and tempo, the macro"
    );
    // The macro's value still makes the state.
    answer(&mut router, |_, prop| match prop {
        "is_playing" => json!(false),
        _ => json!(127.0),
    });
    router.flush();
    let now = status(&mut router).autotune;
    assert_eq!(now.targets[0].state, TargetState::On);
    assert_eq!(
        now.targets[0].key.as_ref().unwrap().problem.as_deref(),
        Some("chain 0 holds 2 plug-ins, not one")
    );
    // Another problem is news; so is the same one after a device.
    find(&mut router, found_with(Err(KeyMissing::NoKeyParams)));
    assert_eq!(
        steps(&rx),
        vec![missing("the plug-in has no Key/Scale parameters")]
    );
    find(&mut router, found_with(Ok(gridder(1))));
    assert_eq!(steps(&rx)[0]["step"], "key_device");
    find(&mut router, found_with(Err(KeyMissing::NoKeyParams)));
    assert_eq!(
        steps(&rx),
        vec![missing("the plug-in has no Key/Scale parameters")]
    );
    assert_eq!(router.subs.subscriptions("band"), 4, "its slots let go");
    // A rack gone takes its key device with it.
    let walk = router.autotune.targets[STREAM].walk;
    router.autotune_found(STREAM, walk, Err("no rack named Latencies".into()));
    assert_eq!(status(&mut router).autotune.targets[0].key, None);
}

#[tokio::test]
async fn the_key_in_force_follows_the_song_time() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    map_read(&mut router, 1, songs("C"));
    transport(&mut router, true);
    deliver(&mut router, TEMPO_KEY, 120.0);
    for time in [10.0, 49.9, 50.1] {
        deliver(&mut router, TIME, time);
    }
    assert_eq!(
        keys_of(&steps(&rx)),
        vec![
            key(Some(1), Some("C"), "song"),
            key(Some(1), Some("G"), "change")
        ]
    );
    assert_eq!(
        status(&mut router).autotune.keys,
        vec![KeyInForce {
            instance: "band".into(),
            song: Some(1),
            key: Some("G".into()),
            time: Some(50.1),
            playing: Some(true),
            tempo: Some(120.0),
        }]
    );
    // The next song: a start, in its own key.
    deliver(&mut router, TIME, 130.0);
    assert_eq!(
        keys_of(&steps(&rx)),
        vec![key(Some(2), Some("F#m"), "song")]
    );
}

#[tokio::test]
async fn the_lookahead_is_a_playing_sets_only() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    map_read(&mut router, 1, songs("C"));
    deliver(&mut router, TEMPO_KEY, 120.0);
    deliver(&mut router, TIME, 49.95);
    transport(&mut router, true);
    // Playing: a tick ahead is past the key clip.
    assert_eq!(
        keys_of(&steps(&rx)),
        vec![
            key(Some(1), Some("C"), "song"),
            key(Some(1), Some("G"), "change")
        ]
    );
    // The rules' transport listener failed: not known to play.
    router.transport_failed(TRANSPORT, "not observable");
    assert_eq!(
        keys_of(&steps(&rx)),
        vec![key(Some(1), Some("C"), "change")]
    );
    assert_eq!(status(&mut router).autotune.keys[0].playing, None);
}

#[tokio::test]
async fn a_stopped_jump_re_evaluates_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    map_read(&mut router, 1, songs("C"));
    transport(&mut router, false);
    for time in [10.0, 60.0, 10.0] {
        deliver(&mut router, TIME, time);
    }
    assert_eq!(
        keys_of(&steps(&rx)),
        vec![
            key(Some(1), Some("C"), "song"),
            key(Some(1), Some("G"), "change"),
            key(Some(1), Some("C"), "change"),
        ]
    );
}

#[tokio::test]
async fn leaving_the_songs_records_span() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    map_read(&mut router, 1, songs("C"));
    deliver(&mut router, TIME, 10.0);
    deliver(&mut router, TIME, 110.0);
    assert_eq!(
        keys_of(&steps(&rx)),
        vec![key(Some(1), Some("C"), "song"), key(None, None, "span")]
    );
    let keys = status(&mut router).autotune.keys;
    assert_eq!(
        (keys[0].song, keys[0].key.as_deref(), keys[0].time),
        (None, None, Some(110.0))
    );
    // Back into the same song: a start again.
    deliver(&mut router, TIME, 20.0);
    assert_eq!(keys_of(&steps(&rx)), vec![key(Some(1), Some("C"), "song")]);
    // A failed position listener: the position unknown, no song.
    router.autotune_delivery(TIME, None, Some("gone".into()));
    assert_eq!(keys_of(&steps(&rx)), vec![key(None, None, "span")]);
    assert_eq!(status(&mut router).autotune.keys[0].time, None);
}

#[tokio::test]
async fn a_keyless_song_is_a_song_with_no_key_in_force() {
    // PR 2a review M2: song 003 has no key; it is told apart from out of
    // every song.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    map_read(&mut router, 1, songs("C"));
    // From out of every song into song 3: no key, its song known.
    deliver(&mut router, TIME, 110.0);
    deliver(&mut router, TIME, 250.0);
    assert_eq!(keys_of(&steps(&rx)), vec![key(Some(3), None, "no_key")]);
    let keys = status(&mut router).autotune.keys;
    assert_eq!((keys[0].song, keys[0].key.as_deref()), (Some(3), None));
    // A jump from song 2 straight into song 3: a song start with no key.
    deliver(&mut router, TIME, 150.0);
    deliver(&mut router, TIME, 250.0);
    assert_eq!(
        keys_of(&steps(&rx)),
        vec![
            key(Some(2), Some("F#m"), "song"),
            key(Some(3), None, "song")
        ]
    );
    // Out of every song after it.
    deliver(&mut router, TIME, 350.0);
    assert_eq!(keys_of(&steps(&rx)), vec![key(None, None, "span")]);
}

#[tokio::test]
async fn a_map_read_keeps_the_key_in_force_until_the_new_map() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    router.states.get_mut("band").unwrap().online = true;
    router.handle(layout(1, vec![stream()]));
    map_read(&mut router, 1, songs("C"));
    transport(&mut router, false);
    deliver(&mut router, TIME, 10.0);
    // A transport change reads the map again; the read takes its time.
    transport(&mut router, true);
    let reading = *router
        .songmap
        .reading
        .get("band")
        .expect("the transport change's read runs");
    for time in [10.5, 20.0, 30.0] {
        deliver(&mut router, TIME, time);
    }
    assert_eq!(
        keys_of(&steps(&rx)),
        vec![key(Some(1), Some("C"), "song")],
        "the old map's key while the new one is read, never none"
    );
    // The new map: song 1 is in D now; its answer judges the key again.
    map_read(&mut router, reading, songs("D"));
    assert_eq!(
        keys_of(&steps(&rx)),
        vec![key(Some(1), Some("D"), "change")]
    );
}

#[tokio::test]
async fn a_set_whose_position_a_page_follows_has_its_key_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    map_read(&mut router, 1, songs("C"));
    // A page follows the playhead already (Live answered it).
    assert!(
        router
            .subs
            .subscribe(1, "band", "live_set", "current_song_time", false)
            .is_ok()
    );
    answer(&mut router, |_, prop| match prop {
        "current_song_time" => json!(130.0),
        _ => json!(false),
    });
    router.handle(layout(1, vec![stream()]));
    assert_eq!(
        keys_of(&steps(&rx)),
        vec![key(Some(2), Some("F#m"), "song")]
    );
}

#[tokio::test]
async fn a_set_without_targets_lets_its_position_go() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, _rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    assert_eq!(router.subs.subscriptions("band"), 3);
    assert_eq!(status(&mut router).autotune.keys.len(), 1);
    // The same set again: followed once.
    router.handle(layout(2, vec![stream()]));
    assert_eq!(router.subs.subscriptions("band"), 3);
    router.handle(layout(3, Vec::new()));
    assert_eq!(router.subs.subscriptions("band"), 1, "the rules' transport");
    assert!(router.autotune.positions.is_empty());
    assert!(status(&mut router).autotune.keys.is_empty());
}

#[tokio::test]
async fn aut_on_is_refused_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx) = offline_router(dir.path());
    let page = attach(&mut router, 1);
    router.handle(layout(1, vec![stream()]));
    find(&mut router, found_with(Err(KeyMissing::NoChain)));
    // Live plays: 1.8.0's AUT on would write the macro on at once.
    transport(&mut router, true);
    let _ = steps(&rx);
    let _ = told(&page);
    set_hub(&mut router, HUB_AUTOTUNE_AUT, json!(true));
    assert_eq!(
        steps(&rx),
        vec![json!({"step": "refused", "what": "aut", "why": REFUSED,
                    "client": 1, "peer": "10.0.0.1"})]
    );
    assert!(!router.autotune.state().aut);
    // The page is told the state again (unchanged: AUT off).
    let again = told(&page);
    assert_eq!(again, vec![router.autotune.state()]);
    assert!(!again[0].aut);
    assert!(!wrote(&mut own).await, "nothing goes to Live");
    assert!(!dir.path().join("hub-state.json").exists(), "nothing saved");
    assert_eq!(status(&mut router).autotune.writes, 0);
    // AUT off is taken as before: its record, nothing saved (no change).
    set_hub(&mut router, HUB_AUTOTUNE_AUT, json!(false));
    assert_eq!(
        steps(&rx),
        vec![json!({"step": "flag", "on": false, "client": 1, "peer": "10.0.0.1"})]
    );
    assert!(!dir.path().join("hub-state.json").exists());
    assert!(!wrote(&mut own).await);
}

#[tokio::test]
async fn a_tap_is_refused_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut own, rx) = offline_router(dir.path());
    let page = attach(&mut router, 1);
    router.handle(layout(1, vec![stream()]));
    find(&mut router, found_with(Err(KeyMissing::NoChain)));
    transport(&mut router, false);
    let _ = steps(&rx);
    let _ = told(&page);
    for value in [json!(true), json!(false), json!("on")] {
        set_hub(&mut router, HUB_AUTOTUNE, value.clone());
        assert_eq!(
            steps(&rx),
            vec![json!({"step": "refused", "what": "tap", "why": REFUSED,
                        "client": 1, "peer": "10.0.0.1"})],
            "{value}"
        );
        let replies = page.take().unwrap_or_default();
        assert_eq!(
            replies,
            vec![ServerMsg::Autotune(router.autotune.state())],
            "told again, no error: {value}"
        );
    }
    assert!(!wrote(&mut own).await, "nothing goes to Live");
    let now = status(&mut router).autotune;
    assert_eq!((now.taps, now.writes, now.aut), (0, 0, false));
    assert!(!dir.path().join("hub-state.json").exists());
}

#[tokio::test]
async fn a_persisted_aut_on_starts_off() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, _rx) = offline_router_with(
        dir.path(),
        HubState {
            stage_aut: false,
            reverb: Saved::default(),
            fader_links: Vec::new(),
            autotune_aut: true,
        },
    );
    assert!(!router.autotune.state().aut);
    assert!(!status(&mut router).autotune.aut);
    let saved: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("hub-state.json")).unwrap())
            .unwrap();
    assert_eq!(saved, json!({"stage_aut": false}), "saved off at once");
    let page = attach(&mut router, 1);
    assert!(!told(&page)[0].aut, "told off at attach");
}

#[tokio::test]
async fn a_disconnect_drops_the_position_and_key_values() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    find(&mut router, found_with(Ok(gridder(1))));
    map_read(&mut router, 1, songs("C"));
    transport(&mut router, true);
    deliver(&mut router, TEMPO_KEY, 120.0);
    deliver(&mut router, TIME, 10.0);
    let key_sub = router.autotune.targets[STREAM].key_sub.clone().unwrap();
    let scale_sub = router.autotune.targets[STREAM].scale_sub.clone().unwrap();
    deliver(&mut router, &key_sub, 0.0);
    deliver(&mut router, &scale_sub, 0.0);
    assert_eq!(
        status(&mut router).autotune.targets[0]
            .key
            .as_ref()
            .unwrap()
            .shows
            .as_deref(),
        Some("C")
    );
    let _ = steps(&rx);
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    let now = status(&mut router).autotune;
    assert_eq!(
        now.keys,
        vec![KeyInForce {
            instance: "band".into(),
            song: None,
            key: None,
            time: None,
            playing: None,
            tempo: None,
        }]
    );
    assert_eq!(now.targets[0].key, None);
    assert_eq!(router.autotune.targets[STREAM].values, (None, None));
    assert_eq!(
        router.subs.subscriptions("band"),
        3,
        "the transport and the playhead stay; the slots go with the refs"
    );
    assert_eq!(
        keys_of(&steps(&rx)),
        Vec::<Value>::new(),
        "forgotten, not recorded"
    );
    // The previous session's map gives no key in force (#140 PR 2a review
    // M1); the next connect's map read does, a song start again.
    deliver(&mut router, TIME, 10.0);
    assert_eq!(keys_of(&steps(&rx)), Vec::<Value>::new());
    map_read(&mut router, 2, songs("C"));
    assert_eq!(keys_of(&steps(&rx)), vec![key(Some(1), Some("C"), "song")]);
}

#[tokio::test]
async fn a_reconnect_has_no_key_in_force_until_its_maps_read() {
    // PR 2a review M1: a set loaded meanwhile has other songs; the map from
    // before the disconnect never gives the new session a key.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    map_read(&mut router, 1, songs("C"));
    deliver(&mut router, TIME, 10.0);
    assert_eq!(keys_of(&steps(&rx)), vec![key(Some(1), Some("C"), "song")]);
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Connected(crate::live::ConnectInfo {
            instance: "band".into(),
            set_name: "set".into(),
            script_version: "0".into(),
            live_version: "12".into(),
        }),
    });
    deliver(&mut router, TEMPO_KEY, 120.0);
    deliver(&mut router, TIME, 10.0);
    assert_eq!(
        keys_of(&steps(&rx)),
        Vec::<Value>::new(),
        "no key from the previous session's map"
    );
    let keys = status(&mut router).autotune.keys;
    assert_eq!(
        (keys[0].song, keys[0].key.as_deref(), keys[0].time),
        (None, None, Some(10.0))
    );
    // The connect's read answers: the key in force at once, a song start.
    map_read(&mut router, 2, songs("C"));
    assert_eq!(keys_of(&steps(&rx)), vec![key(Some(1), Some("C"), "song")]);
    assert_eq!(
        status(&mut router).autotune.keys[0].key.as_deref(),
        Some("C")
    );
}

#[tokio::test]
async fn a_lost_key_value_follows_the_device_afresh() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    find(&mut router, found_with(Ok(gridder(1))));
    let key_sub = router.autotune.targets[STREAM].key_sub.clone().unwrap();
    let scale_sub = router.autotune.targets[STREAM].scale_sub.clone().unwrap();
    deliver(&mut router, &key_sub, 0.5);
    deliver(&mut router, &scale_sub, 0.0);
    assert_eq!(
        router.autotune.targets[STREAM].values,
        (Some(0.5), Some(0.0))
    );
    let walk = router.autotune.targets[STREAM].walk;
    let _ = steps(&rx);
    // The Scale's listener ends (the plug-in replaced): its value dropped,
    // the device forgotten, the target walked again.
    router.autotune_delivery(&scale_sub, None, Some("gone".into()));
    let target = &router.autotune.targets[STREAM];
    assert_eq!(target.values, (Some(0.5), None));
    assert_eq!(target.key, None);
    assert!(target.walk > walk, "walked again");
    // The walk finds the same device: it is followed afresh.
    find(&mut router, found_with(Ok(gridder(1))));
    assert_eq!(steps(&rx)[0]["step"], "key_device");
    assert_eq!(router.subs.subscriptions("band"), 6);
    // A delivery that holds neither a value nor an error keeps the value.
    let key_sub = router.autotune.targets[STREAM].key_sub.clone().unwrap();
    deliver(&mut router, &key_sub, 0.25);
    router.autotune_delivery(&key_sub, None, None);
    assert_eq!(router.autotune.targets[STREAM].values.0, Some(0.25));
}

#[tokio::test]
async fn a_target_that_leaves_the_layout_lets_its_key_device_go() {
    // PR 2a review M5: no Live listener of a key device outlives its target.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, _rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    find(&mut router, found_with(Ok(gridder(1))));
    assert_eq!(
        router.subs.subscriptions("band"),
        6,
        "the transport, the playhead and tempo, the macro, the Key and the Scale"
    );
    router.handle(layout(2, Vec::new()));
    assert_eq!(router.subs.subscriptions("band"), 1, "the rules' transport");
    assert!(router.autotune.targets.is_empty());
}

#[tokio::test]
async fn a_lost_rack_lets_its_key_device_go() {
    // PR 2a review M5: a rack gone takes its macro and its key device's
    // listeners with it; the set's playhead stays followed.
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, _rx) = offline_router(dir.path());
    router.handle(layout(1, vec![stream()]));
    find(&mut router, found_with(Ok(gridder(1))));
    let key_sub = router.autotune.targets[STREAM].key_sub.clone().unwrap();
    deliver(&mut router, &key_sub, 0.5);
    assert_eq!(router.subs.subscriptions("band"), 6);
    let walk = router.autotune.targets[STREAM].walk;
    router.autotune_found(STREAM, walk, Err("no rack named Latencies".into()));
    assert_eq!(
        router.subs.subscriptions("band"),
        3,
        "the transport and the playhead and tempo"
    );
    let target = &router.autotune.targets[STREAM];
    assert_eq!(
        (
            &target.key,
            &target.key_sub,
            &target.scale_sub,
            target.values
        ),
        (&None, &None, &None, (None, None))
    );
}
