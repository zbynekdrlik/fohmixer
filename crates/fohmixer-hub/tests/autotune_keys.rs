//! The autotune keys end to end (#140 PR 2a) on SimLive, read only: a set
//! of our own (invented names) with two stream tracks, each a `Latencies`
//! rack inside `Stream FX` whose chain 0 holds an AudioGridder (the second
//! one's slots held as a Max map holds them), a PA track whose rack's chain
//! 0 holds an Auto-Tune Artist, a `Sections` track with a key clip, and two
//! songs. The hub finds every key device and follows its values, follows
//! the key in force along the playhead (a jump, Live playing across the key
//! clip, a jump into the next song), refuses AUTOTUNE AUT on and the tap,
//! and writes nothing to Live.
#![cfg(unix)]

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use fohmixer_proto::client::{ClientMsg, HubStatus, ServerMsg};
use serde_json::{Value, json};
use support::{Client, Host, TestHub, runtime, serial};

const SECS_5: Duration = Duration::from_secs(5);
const SECS_10: Duration = Duration::from_secs(10);

/// The two stream tracks' AudioGridders and the PA track's Auto-Tune Artist.
const PLUGIN_A: &str = "live_set tracks 0 devices 0 chains 0 devices 0 chains 0 devices 0";
const PLUGIN_B: &str = "live_set tracks 1 devices 0 chains 0 devices 0 chains 0 devices 0";
const PLUGIN_V: &str = "live_set tracks 2 devices 0 chains 0 devices 0";
/// Their racks' first macros.
const MACROS: [&str; 3] = [
    "live_set tracks 0 devices 0 chains 0 devices 0 parameters 1",
    "live_set tracks 1 devices 0 chains 0 devices 0 parameters 1",
    "live_set tracks 2 devices 0 parameters 1",
];
/// Stream B's slots as the fixture holds them: F#m.
const B_KEY: f64 = 0.5;
const B_SCALE: f64 = 1.0 / 28.0;
/// The Auto-Tune Artist's Key and Scale: G#, Chromatic (no key shown).
const V_KEY: f64 = 0.75;
const V_SCALE: f64 = 2.0 / 28.0;

/// The test's own set: Live stands between the songs (110), Live's bar
/// on Eb minor.
fn site(dir: &Path) -> PathBuf {
    let tuner = |name: &str| json!({"template": "plain", "name": name, "class_name": "Tuner"});
    let slot = |n: u32, value: f64, enabled: bool| json!({"name": format!("{n}:(unused):(unassigned)"), "value": value, "enabled": enabled});
    let gridder = |key: f64, scale: f64, enabled: bool| {
        json!({"template": "plugin", "name": "AudioGridder", "product": "AudioGridder",
               "parameters": [slot(0, key, enabled), slot(1, scale, enabled), slot(2, 0.0, true)]})
    };
    let latencies = |plugin: Value| {
        json!({"template": "rack", "name": "Latencies",
               "chains": [{"name": "Auto", "devices": [plugin]}, {"name": "Dry"}]})
    };
    let stream = |name: &str, plugin: Value| {
        json!({"name": name, "devices": [
            {"template": "rack", "name": "Stream FX",
             "chains": [{"name": "Chain 1", "devices": [latencies(plugin)]}]},
            tuner("+AUTOTUNE")]})
    };
    let auto_tune = json!({"template": "plugin", "name": "Auto-Tune Artist",
        "product": "Auto-Tune Artist",
        "parameters": [{"name": "Key", "value": V_KEY}, {"name": "Scale", "value": V_SCALE}]});
    let set = json!({"name": "Keys Site", "current_song_time": 110.0,
        "root_note": 3, "scale_name": "Minor",
        "tracks": [
            stream("Stream A #", gridder(0.0, 0.0, true)),
            stream("Stream B #", gridder(B_KEY, B_SCALE, false)),
            {"name": "Vocal A #", "devices": [latencies(auto_tune), tuner("+AUTOTUNE:0")]},
            {"name": "Sections", "arrangement_clips": [
                {"name": "Key {G}", "start_time": 50.0, "end_time": 51.0}]}],
        "cue_points": [
            {"name": "001 Test One {C 120 4/4}", "time": 0.0},
            {"name": "SONG END", "time": 100.0},
            {"name": "002 Test Two {F#m 90}", "time": 120.0},
            {"name": "SONG END", "time": 200.0}]});
    let path = dir.join("keys-site.json");
    std::fs::write(&path, set.to_string()).unwrap();
    path
}

/// One page whose rail holds the AUTOTUNE button.
fn frame(dir: &Path) {
    let frame = json!({"schema": 2, "default_page": "p", "pages": [{"id": "p", "title": "P",
        "rail": [{"kind": "autotune", "label": "AUTOTUNE"}]}]});
    std::fs::write(dir.join("layout.json"), frame.to_string()).unwrap();
}

/// Every target's key device found and both its values heard.
fn keyed(s: &HubStatus) -> bool {
    let t = &s.autotune.targets;
    t.len() == 3
        && t.iter()
            .all(|t| t.key.as_ref().is_some_and(|k| k.values.is_some()))
}

/// Keyed, the song map read and the playhead heard.
fn following(s: &HubStatus) -> bool {
    keyed(s)
        && s.songmap.len() == 1
        && s.autotune
            .keys
            .first()
            .is_some_and(|k| k.time.is_some() && k.tempo.is_some())
}

/// The `key` records' song, key and why, in order.
fn keys_in(records: &[Value]) -> Vec<(Value, Value, Value)> {
    records
        .iter()
        .filter(|r| r["ev"] == "autotune" && r["step"] == "key")
        .map(|r| (r["song"].clone(), r["key"].clone(), r["why"].clone()))
        .collect()
}

fn key(song: u16, key: &str, why: &str) -> (Value, Value, Value) {
    (json!(song), json!(key), json!(why))
}

/// How many `autotune` records of `records` are the step `step`.
fn count(records: &[Value], step: &str) -> usize {
    records
        .iter()
        .filter(|r| r["ev"] == "autotune" && r["step"] == step)
        .count()
}

/// Live's value of `prop` at `path`, read with a command.
async fn number(c: &mut Client, path: &str, prop: &str) -> f64 {
    c.get("band", path, prop).await.as_f64().unwrap()
}

/// Starts or stops SimLive's transport.
async fn transport(c: &mut Client, verb: &str) {
    let slots = c
        .cmd(
            "band",
            vec![json!({"target": "live_set", "name": verb, "args": []})],
        )
        .await
        .unwrap();
    assert_eq!(slots[0]["ok"], json!(true), "{slots:?}");
}

#[test]
fn every_target_finds_its_key_device() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut host = Host::start_site("band", &site(dir.path()), 0, 0.0);
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let status = hub.status_until(SECS_5, keyed).await;
        let targets = &status.autotune.targets;
        let found: Vec<(Option<&str>, Option<&str>, Option<&str>)> = targets
            .iter()
            .map(|t| {
                let k = t.key.as_ref().unwrap();
                (k.device.as_deref(), k.path.as_deref(), k.shows.as_deref())
            })
            .collect();
        assert_eq!(
            found,
            vec![
                (Some("gridder"), Some(PLUGIN_A), Some("C")),
                (Some("gridder"), Some(PLUGIN_B), Some("F#m")),
                (Some("auto_tune"), Some(PLUGIN_V), None),
            ]
        );
        let values: Vec<Option<[f64; 2]>> = targets
            .iter()
            .map(|t| t.key.as_ref().unwrap().values)
            .collect();
        assert_eq!(
            values,
            vec![
                Some([0.0, 0.0]),
                Some([B_KEY, B_SCALE]),
                Some([V_KEY, V_SCALE])
            ]
        );
        // The racks' macros are found as before.
        let paths: Vec<Option<&str>> = targets.iter().map(|t| t.path.as_deref()).collect();
        assert_eq!(paths, MACROS.map(Some).to_vec());
        let records = hub
            .events_until(SECS_5, |r| count(r, "key_device") == 3)
            .await;
        assert_eq!(count(&records, "key_missing"), 0);
        // Another writer sets Stream A's slot 0 (a click in Live): heard.
        let slot = format!("{PLUGIN_A} parameters 1");
        assert_eq!(host.param(&slot, "value", 9.0 / 11.0), 9.0 / 11.0);
        let status = hub
            .status_until(SECS_5, |s| {
                s.autotune.targets[0]
                    .key
                    .as_ref()
                    .is_some_and(|k| k.shows.as_deref() == Some("A"))
            })
            .await;
        assert_eq!(
            status.autotune.targets[0].key.as_ref().unwrap().values,
            Some([9.0 / 11.0, 0.0])
        );
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn the_key_in_force_follows_play_and_jumps_and_nothing_is_written_to_live() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut host = Host::start_site("band", &site(dir.path()), 0, 0.0);
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let status = hub.status_until(SECS_5, following).await;
        let now = &status.autotune.keys[0];
        assert_eq!(
            (
                now.instance.as_str(),
                now.song,
                now.key.as_deref(),
                now.time
            ),
            ("band", None, None, Some(110.0)),
            "between the songs: no key in force"
        );
        let mut a = hub.client().await;
        // A jump into song 1: its start, in C.
        assert_eq!(host.song_time(10.0), 10.0);
        hub.events_until(SECS_5, |r| keys_in(r) == [key(1, "C", "song")])
            .await;
        // Close to the key clip, then Live plays across it at 120 bpm.
        assert_eq!(host.song_time(46.0), 46.0);
        transport(&mut a, "start_playing").await;
        hub.events_until(SECS_10, |r| keys_in(r).len() == 2).await;
        // A jump into song 2 while Live plays.
        assert_eq!(host.song_time(130.0), 130.0);
        let records = hub.events_until(SECS_5, |r| keys_in(r).len() == 3).await;
        transport(&mut a, "stop_playing").await;
        assert_eq!(
            keys_in(&records),
            vec![
                key(1, "C", "song"),
                key(1, "G", "change"),
                key(2, "F#m", "song")
            ]
        );
        let status = hub
            .status_until(SECS_5, |s| s.autotune.keys[0].playing == Some(false))
            .await;
        let now = &status.autotune.keys[0];
        assert_eq!((now.song, now.key.as_deref()), (Some(2), Some("F#m")));
        assert_eq!(now.tempo, Some(120.0));
        // Nothing was written to Live: every macro and key parameter holds
        // its fixture value, and so does Live's bar.
        for path in MACROS {
            assert_eq!(number(&mut a, path, "value").await, 0.0, "{path}");
        }
        let held = [
            (format!("{PLUGIN_A} parameters 1"), 0.0),
            (format!("{PLUGIN_A} parameters 2"), 0.0),
            (format!("{PLUGIN_B} parameters 1"), B_KEY),
            (format!("{PLUGIN_B} parameters 2"), B_SCALE),
            (format!("{PLUGIN_V} parameters 1"), V_KEY),
            (format!("{PLUGIN_V} parameters 2"), V_SCALE),
        ];
        for (path, value) in &held {
            assert_eq!(number(&mut a, path, "value").await, *value, "{path}");
        }
        assert_eq!(number(&mut a, "live_set", "root_note").await, 3.0);
        assert_eq!(
            a.get("band", "live_set", "scale_name").await,
            json!("Minor")
        );
        let status = hub.status().await;
        assert_eq!((status.autotune.aut, status.autotune.writes), (false, 0));
        // One Live listener per key the hub follows.
        let mut followed: Vec<(String, String)> = MACROS
            .iter()
            .map(|m| ("value".to_string(), (*m).to_string()))
            .collect();
        followed.extend(held.iter().map(|(p, _)| ("value".to_string(), p.clone())));
        followed.push(("current_song_time".into(), "live_set".into()));
        followed.push(("tempo".into(), "live_set".into()));
        for (prop, path) in followed {
            assert_eq!(host.listeners(&prop, &path), 1, "{prop} of {path}");
        }
        a.close().await;
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn aut_on_and_a_tap_are_refused_end_to_end() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let host = Host::start_site("band", &site(dir.path()), 0, 0.0);
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_5, |s| keyed(s) && s.instances[0].playing.is_some())
            .await;
        let mut a = hub.client().await;
        // Live plays: 1.8.0's AUT on would switch the stream racks on.
        transport(&mut a, "start_playing").await;
        hub.status_until(SECS_5, |s| s.instances[0].playing == Some(true))
            .await;
        a.clear();
        for (key, value) in [("autotune_aut", true), ("autotune", true)] {
            a.send(&ClientMsg::SetHub {
                key: key.into(),
                value: json!(value),
            })
            .await;
        }
        let records = hub.events_until(SECS_5, |r| count(r, "refused") == 2).await;
        let refused: Vec<Value> = records
            .iter()
            .filter(|r| r["ev"] == "autotune" && r["step"] == "refused")
            .map(|r| json!([r["what"], r["why"]]))
            .collect();
        assert_eq!(
            refused,
            vec![
                json!(["aut", "keys only until #140 PR 3"]),
                json!(["tap", "keys only until #140 PR 3"])
            ]
        );
        // The page is told the state again: AUT off.
        let told = a
            .wait(SECS_5, |m| match m {
                ServerMsg::Autotune(s) => Some(s.clone()),
                _ => None,
            })
            .await;
        assert!(!told.aut);
        tokio::time::sleep(Duration::from_millis(300)).await;
        for path in MACROS {
            assert_eq!(number(&mut a, path, "value").await, 0.0, "{path}");
        }
        let status = hub.status().await;
        assert_eq!(
            (
                status.autotune.aut,
                status.autotune.writes,
                status.autotune.taps
            ),
            (false, 0, 0)
        );
        transport(&mut a, "stop_playing").await;
        a.close().await;
        hub.stop().await;
        host.stop();
    });
}
