//! The autotune keys end to end (#140 PR 2a, 2b) on SimLive: a set of our
//! own (invented names) with two stream tracks, each a `Latencies` rack
//! inside `Stream FX` whose chain 0 holds an AudioGridder (the second one's
//! slots held as a Max map holds them), a PA track whose rack's chain 0
//! holds an Auto-Tune Artist (held too in one test, as on the PC), a
//! `Sections` track with a key clip, and two songs. The hub finds every
//! key device and follows its values and the key in force along the
//! playhead (a jump, Live playing across the key clip, a jump into the
//! next song); it writes the key in force to every key device and Live's
//! bar and reads each write back: a held parameter is refused with Live's
//! words and retried once, a released one written, another writer shown
//! and not fought, a set load written again. AUTOTUNE AUT on and the tap
//! are refused, and no selector moves (SimLive's per-object set count).
#![cfg(unix)]

mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fohmixer_proto::client::{ClientMsg, HubStatus, KeyState, KeyTargetStatus, ServerMsg};
use serde_json::{Value, json};
use support::{Client, Host, TestHub, runtime, serial};

const SECS_5: Duration = Duration::from_secs(5);
const SECS_10: Duration = Duration::from_secs(10);
const QUIET: Duration = Duration::from_millis(300);

/// The two stream tracks' AudioGridders and the PA track's Auto-Tune Artist.
const PLUGIN_A: &str = "live_set tracks 0 devices 0 chains 0 devices 0 chains 0 devices 0";
const PLUGIN_B: &str = "live_set tracks 1 devices 0 chains 0 devices 0 chains 0 devices 0";
const PLUGIN_V: &str = "live_set tracks 2 devices 0 chains 0 devices 0";
/// Their racks' first macros (the selectors' mappings).
const MACROS: [&str; 3] = [
    "live_set tracks 0 devices 0 chains 0 devices 0 parameters 1",
    "live_set tracks 1 devices 0 chains 0 devices 0 parameters 1",
    "live_set tracks 2 devices 0 parameters 1",
];
/// Stream B's slots as the fixture holds them: F# (Major).
const B_KEY: f64 = 0.5;
const B_SCALE: f64 = 0.0;
/// The Auto-Tune Artist's Key and Scale: G#, Chromatic (no key shown).
const V_KEY: f64 = 0.75;
const V_SCALE: f64 = 2.0 / 28.0;
/// Live's words, through the script, for a parameter a Max map holds.
const DISABLED: &str = "live error: Value cannot be set, the parameter is disabled";

/// The test's own set: Live stands at `at` (110: between the songs), Live's
/// bar on Eb minor; with `pa_held` the Auto-Tune Artist's Key and Scale are
/// held as Stream B's slots are (as on the PC before its maps go off).
fn site_with(dir: &Path, at: f64, pa_held: bool) -> PathBuf {
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
        "parameters": [{"name": "Key", "value": V_KEY, "enabled": !pa_held},
                       {"name": "Scale", "value": V_SCALE, "enabled": !pa_held}]});
    let set = json!({"name": "Keys Site", "current_song_time": at,
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

/// The set standing between the songs, the PA's Auto-Tune free.
fn site(dir: &Path) -> PathBuf {
    site_with(dir, 110.0, false)
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

/// Target `i`'s key device in the status (Stream A, Stream B, Vocal A).
fn key_of(s: &HubStatus, i: usize) -> &KeyTargetStatus {
    s.autotune.targets[i].key.as_ref().unwrap()
}

/// Target `i` wants `want` and stands at `state`.
fn stands(s: &HubStatus, i: usize, want: &str, state: KeyState) -> bool {
    s.autotune.targets.get(i).is_some_and(|t| {
        t.key
            .as_ref()
            .is_some_and(|k| k.want.as_deref() == Some(want) && k.state == Some(state))
    })
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

/// The `autotune` records of `records` that are the step `step`.
fn steps_of(records: &[Value], step: &str) -> Vec<Value> {
    records
        .iter()
        .filter(|r| r["ev"] == "autotune" && r["step"] == step)
        .cloned()
        .collect()
}

/// How many `autotune` records of `records` are the step `step`.
fn count(records: &[Value], step: &str) -> usize {
    steps_of(records, step).len()
}

/// How many key writes of `records` named the target labelled `label`,
/// their retries apart.
fn writes_to(records: &[Value], label: &str) -> usize {
    steps_of(records, "key_write")
        .iter()
        .filter(|r| r["why"] != "retry")
        .filter(|r| {
            r["targets"]
                .as_array()
                .is_some_and(|t| t.contains(&json!(label)))
        })
        .count()
}

/// The `readback` records of the target on band track `index`.
fn readbacks_of(records: &[Value], index: u32) -> Vec<Value> {
    let target = format!("live_set tracks {index}");
    steps_of(records, "readback")
        .into_iter()
        .filter(|r| r["target"] == json!(target))
        .collect()
}

/// A value as Live keeps a plug-in's parameter: a float32.
fn f32_of(value: f64) -> f64 {
    f64::from(value as f32)
}

/// Live's value of `prop` at `path`, read with a command.
async fn number(c: &mut Client, path: &str, prop: &str) -> f64 {
    c.get("band", path, prop).await.as_f64().unwrap()
}

/// Live's Key and Scale of a key device at `plugin`.
async fn values_at(c: &mut Client, plugin: &str) -> (f64, f64) {
    (
        number(c, &format!("{plugin} parameters 1"), "value").await,
        number(c, &format!("{plugin} parameters 2"), "value").await,
    )
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

/// No selector moved: every rack's macro still holds `value`, and Live's
/// setter saw `sets` sets of each (the test's own, if any).
async fn no_selector_moved(c: &mut Client, host: &mut Host, value: f64, sets: i64) {
    for path in MACROS {
        assert_eq!(number(c, path, "value").await, value, "{path}");
        assert_eq!(host.sets("value", path), sets, "{path}");
    }
}

/// The hub's clock in ms, as the records' `ts`.
fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// The records written at or after `since`.
fn since(records: &[Value], since: i64) -> Vec<Value> {
    records
        .iter()
        .filter(|r| r["ts"].as_i64().is_some_and(|ts| ts >= since))
        .cloned()
        .collect()
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
                (Some("gridder"), Some(PLUGIN_B), Some("F#")),
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
        // Between the songs: nothing written yet.
        let states: Vec<Option<KeyState>> = targets
            .iter()
            .map(|t| t.key.as_ref().unwrap().state)
            .collect();
        assert_eq!(states, vec![Some(KeyState::NoKey); 3]);
        // The racks' macros are found as before.
        let paths: Vec<Option<&str>> = targets.iter().map(|t| t.path.as_deref()).collect();
        assert_eq!(paths, MACROS.map(Some).to_vec());
        let records = hub
            .events_until(SECS_5, |r| count(r, "key_device") == 3)
            .await;
        assert_eq!(count(&records, "key_missing"), 0);
        // Another writer sets Stream A's slot 0 (a click in Live): heard,
        // as Live's float32 of it.
        let slot = format!("{PLUGIN_A} parameters 1");
        let a = f32_of(9.0 / 11.0);
        assert_eq!(host.param(&slot, "value", 9.0 / 11.0), a);
        let status = hub
            .status_until(SECS_5, |s| key_of(s, 0).shows.as_deref() == Some("A"))
            .await;
        assert_eq!(key_of(&status, 0).values, Some([a, 0.0]));
        assert_eq!(count(&hub.events(), "key_write"), 0);
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn the_key_in_force_follows_play_and_jumps_and_no_selector_moves() {
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
        // Every rack's macro between its values, so a write of either shows
        // (PR 2a review M6): the test's own one set of each.
        for path in MACROS {
            a.set("band", path, "value", json!(64.0)).await;
        }
        // A jump into song 1: its start, in C, written and read back.
        assert_eq!(host.song_time(10.0), 10.0);
        hub.events_until(SECS_5, |r| keys_in(r) == [key(1, "C", "song")])
            .await;
        hub.status_until(SECS_5, |s| stands(s, 0, "C", KeyState::Confirmed))
            .await;
        // Close to the key clip, then Live plays across it at 120 bpm: G
        // goes to every key device within the lateness bound.
        assert_eq!(host.song_time(46.0), 46.0);
        transport(&mut a, "start_playing").await;
        let records = hub
            .events_until(SECS_10, |r| keys_in(r).len() == 2 && count(r, "late") == 1)
            .await;
        let late = &steps_of(&records, "late")[0];
        assert_eq!(late["song"], json!(1));
        let ms = late["ms"].as_f64().unwrap();
        assert!((-40.0..=100.0).contains(&ms), "{late}");
        assert!(
            steps_of(&records, "key_write")
                .iter()
                .any(|w| w["why"] == "change" && w["key"] == "G" && w["bar"] == true),
            "{records:?}"
        );
        let status = hub
            .status_until(SECS_5, |s| stands(s, 0, "G", KeyState::Confirmed))
            .await;
        assert_eq!(key_of(&status, 0).shows.as_deref(), Some("G"));
        let (k, s) = values_at(&mut a, PLUGIN_A).await;
        assert_eq!((k, s), (f32_of(7.0 / 11.0), 0.0));
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
            .status_until(SECS_5, |s| {
                s.autotune.keys[0].playing == Some(false)
                    && stands(s, 0, "F#m", KeyState::Confirmed)
            })
            .await;
        let now = &status.autotune.keys[0];
        assert_eq!((now.song, now.key.as_deref()), (Some(2), Some("F#m")));
        assert_eq!(now.tempo, Some(120.0));
        assert_eq!(now.bar.as_deref(), Some("F#m"));
        // No selector moved: every macro holds the test's 64, Live's setter
        // saw the test's own set of each and nothing else, and no macro
        // write was recorded.
        no_selector_moved(&mut a, &mut host, 64.0, 1).await;
        let records = hub.events();
        assert_eq!(count(&records, "write"), 0);
        assert_eq!(count(&records, "failed"), 0);
        let status = hub.status().await;
        assert_eq!((status.autotune.aut, status.autotune.writes), (false, 0));
        // One Live listener per key the hub follows.
        let mut followed: Vec<(String, String)> = MACROS
            .iter()
            .map(|m| ("value".to_string(), (*m).to_string()))
            .collect();
        for plugin in [PLUGIN_A, PLUGIN_B, PLUGIN_V] {
            for n in [1, 2] {
                followed.push(("value".into(), format!("{plugin} parameters {n}")));
            }
        }
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
fn a_song_start_writes_and_confirms_the_key() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut host = Host::start_site("band", &site(dir.path()), 0, 0.0);
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_5, following).await;
        let mut a = hub.client().await;
        // Song 1 starts: C to every key device and to Live's bar.
        assert_eq!(host.song_time(10.0), 10.0);
        let status = hub
            .status_until(SECS_5, |s| {
                stands(s, 0, "C", KeyState::Confirmed)
                    && stands(s, 1, "C", KeyState::Refused)
                    && stands(s, 2, "C", KeyState::Confirmed)
                    && s.autotune.keys[0].bar.is_some()
            })
            .await;
        assert_eq!(status.autotune.keys[0].bar.as_deref(), Some("C"));
        // Stream A's slots held C already: a nudge beside it; the
        // Auto-Tune Artist gets the centres.
        let nudged = (f32_of(0.05 / 11.0), f32_of(0.05 / 28.0));
        assert_eq!(values_at(&mut a, PLUGIN_A).await, nudged);
        assert_eq!(values_at(&mut a, PLUGIN_V).await, (0.0, 0.0));
        assert_eq!(
            values_at(&mut a, PLUGIN_B).await,
            (B_KEY, B_SCALE),
            "refused"
        );
        assert_eq!(number(&mut a, "live_set", "root_note").await, 0.0);
        assert_eq!(
            a.get("band", "live_set", "scale_name").await,
            json!("Major")
        );
        // Stream B's slots are held: refused with Live's words, retried
        // once, and never again.
        let records = hub
            .events_until(SECS_5, |r| readbacks_of(r, 1).len() == 2)
            .await;
        for readback in readbacks_of(&records, 1) {
            assert_eq!(
                (&readback["state"], &readback["problem"]),
                (&json!("refused"), &json!(DISABLED))
            );
        }
        tokio::time::sleep(QUIET).await;
        let records = hub.events();
        let writes: Vec<(Value, Value, Value)> = steps_of(&records, "key_write")
            .iter()
            .map(|w| (w["why"].clone(), w["targets"].clone(), w["bar"].clone()))
            .collect();
        assert_eq!(
            writes,
            vec![
                (json!("song"), json!(["#1", "#2", "#3"]), json!(true)),
                (json!("retry"), json!(["#2"]), json!(false))
            ]
        );
        assert_eq!(readbacks_of(&records, 1).len(), 2);
        assert_eq!(host.sets("value", &format!("{PLUGIN_B} parameters 1")), 2);
        assert_eq!(count(&records, "bar"), 1);
        // Another writer turns Stream A's slot 0: shown, never fought.
        let slot = format!("{PLUGIN_A} parameters 1");
        let other = f32_of(0.9);
        assert_eq!(host.param(&slot, "value", 0.9), other);
        let records = hub
            .events_until(SECS_5, |r| count(r, "another_writer") == 1)
            .await;
        assert_eq!(
            (
                &steps_of(&records, "another_writer")[0]["target"],
                &steps_of(&records, "another_writer")[0]["shows"]
            ),
            (&json!("live_set tracks 0"), &json!("Bb"))
        );
        hub.status_until(SECS_5, |s| {
            key_of(s, 0).state == Some(KeyState::AnotherWriter)
        })
        .await;
        tokio::time::sleep(QUIET).await;
        assert_eq!(number(&mut a, &slot, "value").await, other);
        assert_eq!(
            host.sets("value", &slot),
            2,
            "the song start's and the other writer's"
        );
        assert_eq!(count(&hub.events(), "key_write"), 2);
        // The next song start writes it again: F#m.
        assert_eq!(host.song_time(130.0), 130.0);
        hub.status_until(SECS_5, |s| stands(s, 0, "F#m", KeyState::Confirmed))
            .await;
        assert_eq!(
            values_at(&mut a, PLUGIN_A).await,
            (f32_of(6.0 / 11.0), f32_of(1.0 / 28.0))
        );
        no_selector_moved(&mut a, &mut host, 0.0, 0).await;
        assert_eq!(count(&hub.events(), "write"), 0);
        a.close().await;
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_held_map_refuses_its_keys_until_released() {
    // As on the PC before its maps go off: the PA's Auto-Tune Artist and
    // Stream B's slots are held.
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut host = Host::start_site("band", &site_with(dir.path(), 110.0, true), 0, 0.0);
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_5, following).await;
        let mut a = hub.client().await;
        assert_eq!(host.song_time(10.0), 10.0);
        let records = hub
            .events_until(SECS_5, |r| {
                readbacks_of(r, 1).len() == 2 && readbacks_of(r, 2).len() == 2
            })
            .await;
        for index in [1, 2] {
            for readback in readbacks_of(&records, index) {
                assert_eq!(
                    (&readback["key"], &readback["state"], &readback["problem"]),
                    (&json!("C"), &json!("refused"), &json!(DISABLED)),
                    "track {index}"
                );
            }
        }
        // Retried once, together; never a write loop.
        tokio::time::sleep(QUIET).await;
        let records = hub.events();
        let writes: Vec<(Value, Value)> = steps_of(&records, "key_write")
            .iter()
            .map(|w| (w["why"].clone(), w["targets"].clone()))
            .collect();
        assert_eq!(
            writes,
            vec![
                (json!("song"), json!(["#1", "#2", "#3"])),
                (json!("retry"), json!(["#2", "#3"]))
            ]
        );
        for plugin in [PLUGIN_B, PLUGIN_V] {
            assert_eq!(
                host.sets("value", &format!("{plugin} parameters 1")),
                2,
                "{plugin}"
            );
        }
        let status = hub.status().await;
        let states: Vec<Option<KeyState>> = (0..3).map(|i| key_of(&status, i).state).collect();
        assert_eq!(
            states,
            vec![
                Some(KeyState::Confirmed),
                Some(KeyState::Refused),
                Some(KeyState::Refused)
            ]
        );
        assert_eq!(values_at(&mut a, PLUGIN_B).await, (B_KEY, B_SCALE));
        assert_eq!(values_at(&mut a, PLUGIN_V).await, (V_KEY, V_SCALE));
        // The maps go off: their parameters are free, and the next song
        // start goes through, F#m: 6/11 and 1/28 as Live's float32.
        for plugin in [PLUGIN_B, PLUGIN_V] {
            for n in [1, 2] {
                let path = format!("{plugin} parameters {n}");
                assert_eq!(host.param(&path, "enabled", 1.0), 1.0, "{path}");
            }
        }
        assert_eq!(host.song_time(130.0), 130.0);
        hub.status_until(SECS_5, |s| {
            (0..3).all(|i| stands(s, i, "F#m", KeyState::Confirmed))
        })
        .await;
        let f_sharp_m = (f32_of(6.0 / 11.0), f32_of(1.0 / 28.0));
        assert_eq!(values_at(&mut a, PLUGIN_B).await, f_sharp_m);
        assert_eq!(values_at(&mut a, PLUGIN_V).await, f_sharp_m);
        no_selector_moved(&mut a, &mut host, 0.0, 0).await;
        a.close().await;
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_reconnect_writes_the_key_again() {
    // A host restart with the same fixture is a set load: Live stands in
    // song 1, its key devices and bar as the set holds them.
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let set = site_with(dir.path(), 10.0, false);
        let host = Host::start_site("band", &set, 0, 0.0);
        let port = host.port;
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_5, |s| {
            stands(s, 0, "C", KeyState::Confirmed) && stands(s, 2, "C", KeyState::Confirmed)
        })
        .await;
        let mut a = hub.client().await;
        host.stop();
        a.instance_state("band", false, None, SECS_5).await;
        let reloaded = now_ts();
        let mut host = Host::start_site("band", &set, port, 0.0);
        let status = hub
            .status_until(SECS_10, |s| {
                s.instances[0].online
                    && stands(s, 0, "C", KeyState::Confirmed)
                    && stands(s, 1, "C", KeyState::Refused)
                    && stands(s, 2, "C", KeyState::Confirmed)
                    && s.autotune.keys[0].bar.as_deref() == Some("C")
            })
            .await;
        assert_eq!(status.autotune.keys[0].song, Some(1));
        // Each key device written once after the set load (its retry
        // apart), through the new session's refs.
        tokio::time::sleep(QUIET).await;
        let records = since(&hub.events(), reloaded);
        for label in ["#1", "#2", "#3"] {
            assert_eq!(writes_to(&records, label), 1, "{label}: {records:?}");
        }
        let nudged = (f32_of(0.05 / 11.0), f32_of(0.05 / 28.0));
        assert_eq!(values_at(&mut a, PLUGIN_A).await, nudged);
        assert_eq!(values_at(&mut a, PLUGIN_V).await, (0.0, 0.0));
        assert_eq!(number(&mut a, "live_set", "root_note").await, 0.0);
        no_selector_moved(&mut a, &mut host, 0.0, 0).await;
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
        // Live plays: 1.8.0's AUT on would switch the stream racks on. The
        // transport's own `autotune` message (playing) comes first, so every
        // later one is a re-tell (PR 2a review M7).
        transport(&mut a, "start_playing").await;
        a.wait(SECS_5, |m| match m {
            ServerMsg::Autotune(s) if s.playing == Some(true) => Some(()),
            _ => None,
        })
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
        // The page is told the state again, once per refusal: AUT off.
        let mut told = Vec::new();
        let deadline = Instant::now() + SECS_5;
        while told.len() < 2 {
            let left = deadline.saturating_duration_since(Instant::now());
            match a.next_message(left).await {
                Some(ServerMsg::Autotune(s)) => told.push(s),
                Some(_) => {}
                None => break,
            }
        }
        // Nothing more within 300 ms: no third.
        let quiet = Instant::now() + QUIET;
        loop {
            let left = quiet.saturating_duration_since(Instant::now());
            match a.next_message(left).await {
                Some(ServerMsg::Autotune(s)) => told.push(s),
                Some(_) => {}
                None => break,
            }
        }
        assert_eq!(told.len(), 2, "{told:?}");
        assert!(told.iter().all(|s| !s.aut && s.playing == Some(true)));
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
