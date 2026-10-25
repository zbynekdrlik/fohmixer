//! The autotune rule end to end (#140 PR 3b, spec §2.2, §7, §9) on
//! SimLive: a set of our own (invented names) with two singers, VOX1 and
//! VOX2, each a stream track (an AudioGridder in a `Latencies` rack inside
//! `Stream FX`, its Tuner `+MUTE:<id> +AUTOTUNE`: on 127, off 0) and a PA
//! track (an Auto-Tune Artist in a `Latencies` rack on the track, its Tuner
//! `"Vocal …" +ID:<id> +AUTOTUNE:0`: on 0, off 127, standing on DRY at 127),
//! VOX2's stream slots held as a Max map holds them, and two songs. A
//! page's AUTOTUNE AUT on is refused until #140 PR 3c (the PR #147
//! rulings; the switching itself is the router tests', and the rehearsal's
//! after PR 3c): the keys follow the songs and no selector moves, a track
//! whose key Live refuses shows its singer's key problem, a singer off by
//! hand stays so across a hub restart, and the pages are told the song,
//! the key and the singers.
#![cfg(unix)]

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use fohmixer_proto::client::{
    ClientMsg, HubStatus, KeyState, SafeState, ServerMsg, SwitchWhy, TrackState,
};
use serde_json::{Value, json};
use support::{Client, Host, TestHub, runtime, serial};

const SECS_10: Duration = Duration::from_secs(10);

/// The racks' first macros: Stream A's, Stream B's, Vocal A's, Vocal B's.
const MACROS: [&str; 4] = [
    "live_set tracks 0 devices 0 chains 0 devices 0 parameters 1",
    "live_set tracks 1 devices 0 chains 0 devices 0 parameters 1",
    "live_set tracks 2 devices 0 parameters 1",
    "live_set tracks 3 devices 0 parameters 1",
];
/// Every selector on DRY.
const DRY: [f64; 4] = [0.0, 0.0, 127.0, 127.0];

/// The test's own set, Live standing at `at`: song 1 in C from 0 to 100,
/// song 2 in F#m from 120 to 200.
fn site(dir: &Path, at: f64) -> PathBuf {
    let tuner = |name: &str| json!({"template": "plain", "name": name, "class_name": "Tuner"});
    let slot = |n: u32, enabled: bool| json!({"name": format!("{n}:(unused):(unassigned)"), "value": 0.0, "enabled": enabled});
    let gridder = |enabled: bool| {
        json!({"template": "plugin", "name": "AudioGridder", "product": "AudioGridder",
               "parameters": [slot(0, enabled), slot(1, enabled), slot(2, true)]})
    };
    let latencies = |plugin: Value, dry: f64| {
        json!({"template": "rack", "name": "Latencies", "macros": [dry],
               "chains": [{"name": "Auto", "devices": [plugin]}, {"name": "Dry"}]})
    };
    let stream = |name: &str, enabled: bool, id: &str| {
        json!({"name": name, "devices": [
            {"template": "rack", "name": "Stream FX",
             "chains": [{"name": "Chain 1", "devices": [latencies(gridder(enabled), 0.0)]}]},
            tuner(&format!("+MUTE:{id} +AUTOTUNE"))]})
    };
    let pa = |name: &str, label: &str, id: &str| {
        let auto_tune = json!({"template": "plugin", "name": "Auto-Tune Artist",
            "product": "Auto-Tune Artist",
            "parameters": [{"name": "Key", "value": 0.0}, {"name": "Scale", "value": 0.0}]});
        json!({"name": name, "devices": [
            latencies(auto_tune, 127.0),
            tuner(&format!("\"{label}\" +G:VOCALS +ID:{id} +AUTOTUNE:0"))]})
    };
    let set = json!({"name": "Switch Site", "current_song_time": at, "tracks": [
            stream("Stream A #", true, "VOX1"),
            stream("Stream B #", false, "VOX2"),
            pa("Vocal A #", "Vocal A", "VOX1"),
            pa("Vocal B #", "Vocal B", "VOX2")],
        "cue_points": [
            {"name": "001 Test One {C 120 4/4}", "time": 0.0},
            {"name": "SONG END", "time": 100.0},
            {"name": "002 Test Two {F#m 90}", "time": 120.0},
            {"name": "SONG END", "time": 200.0}]});
    let path = dir.join("switch-site.json");
    std::fs::write(&path, set.to_string()).unwrap();
    path
}

/// One page whose rail holds the AUTOTUNE button.
fn frame(dir: &Path) {
    let frame = json!({"schema": 2, "default_page": "p", "pages": [{"id": "p", "title": "P",
        "rail": [{"kind": "autotune", "label": "AUTOTUNE"}]}]});
    std::fs::write(dir.join("layout.json"), frame.to_string()).unwrap();
}

/// Every target's key device found with both values heard, the song map
/// read, the playhead heard, and the DRY writes of all four kept.
fn ready(s: &HubStatus) -> bool {
    let t = &s.autotune.targets;
    t.len() == 4
        && t.iter()
            .all(|t| t.key.as_ref().is_some_and(|k| k.values.is_some()))
        && s.songmap.len() == 1
        && s.autotune.keys.first().is_some_and(|k| k.time.is_some())
        && s.autotune
            .safe
            .iter()
            .any(|r| r.state == SafeState::Kept && r.targets == 4)
}

/// Target `i`'s key stands at `state` for `want`.
fn stands(s: &HubStatus, i: usize, want: &str, state: KeyState) -> bool {
    s.autotune.targets.get(i).is_some_and(|t| {
        t.key
            .as_ref()
            .is_some_and(|k| k.want.as_deref() == Some(want) && k.state == Some(state))
    })
}

/// Every target's key for `want`: confirmed, Stream B's refused.
fn keyed(s: &HubStatus, want: &str) -> bool {
    stands(s, 0, want, KeyState::Confirmed)
        && stands(s, 1, want, KeyState::Refused)
        && stands(s, 2, want, KeyState::Confirmed)
        && stands(s, 3, want, KeyState::Confirmed)
}

/// Each target's decision's why, in Live's order.
fn whys(s: &HubStatus) -> Vec<Option<SwitchWhy>> {
    s.autotune
        .targets
        .iter()
        .map(|t| t.decision.map(|d| d.why))
        .collect()
}

/// The `autotune` records of `records` that are the step `step`.
fn steps_of(records: &[Value], step: &str) -> Vec<Value> {
    records
        .iter()
        .filter(|r| r["ev"] == "autotune" && r["step"] == step)
        .cloned()
        .collect()
}

/// Every macro as Live holds it, in [`MACROS`]' order.
async fn macros(c: &mut Client) -> [f64; 4] {
    let mut values = [0.0; 4];
    for (value, path) in values.iter_mut().zip(MACROS) {
        *value = c.get("band", path, "value").await.as_f64().unwrap();
    }
    values
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

async fn set_hub(c: &mut Client, key: &str, value: Value) {
    c.send(&ClientMsg::SetHub {
        key: key.into(),
        value,
    })
    .await;
}

#[test]
fn a_song_played_writes_the_keys_and_with_aut_refused_no_selector() {
    // Until #140 PR 3c a page's AUT on is refused (the PR #147 rulings):
    // the keys follow the songs, every selector stays DRY.
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut host = Host::start_site("band", &site(dir.path(), 110.0), 0, 0.0);
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_10, ready).await;
        let mut a = hub.client().await;
        set_hub(&mut a, "autotune_aut", json!(true)).await;
        let records = hub
            .events_until(SECS_10, |r| !steps_of(r, "refused").is_empty())
            .await;
        assert_eq!(steps_of(&records, "refused")[0]["what"], "aut");
        transport(&mut a, "start_playing").await;
        // Into song 2: its key written and read back (Stream B's refused).
        assert_eq!(host.song_time(125.0), 125.0);
        let status = hub.status_until(SECS_10, |s| keyed(s, "F#m")).await;
        assert!(!status.autotune.aut);
        assert_eq!(whys(&status), vec![Some(SwitchWhy::Aut); 4]);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(macros(&mut a).await, DRY);
        let sets: Vec<i64> = MACROS.iter().map(|p| host.sets("value", p)).collect();
        assert_eq!(sets, [0, 0, 0, 0], "no selector written");
        let records = hub.events();
        assert!(steps_of(&records, "select").is_empty());
        assert!(steps_of(&records, "write").is_empty());
        assert_eq!(hub.status().await.autotune.writes, 0);
        transport(&mut a, "stop_playing").await;
        a.close().await;
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_held_stream_slot_shows_its_singers_key_problem() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let host = Host::start_site("band", &site(dir.path(), 10.0), 0, 0.0);
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_10, |s| ready(s) && keyed(s, "C"))
            .await;
        let mut a = hub.client().await;
        let state = a
            .wait(SECS_10, |m| match m {
                ServerMsg::Autotune(s) if s.singers.len() == 2 && !s.singers[1].key_ok => {
                    Some(s.clone())
                }
                _ => None,
            })
            .await;
        let vox2 = &state.singers[1];
        assert_eq!(
            (vox2.id.as_str(), vox2.stream, vox2.pa, vox2.key_ok),
            ("VOX2", TrackState::Dry, TrackState::Dry, false)
        );
        assert_eq!(vox2.why.as_deref(), Some("stream Key refused, wrote C"));
        let vox1 = &state.singers[0];
        assert_eq!(
            (vox1.stream, vox1.pa, vox1.key_ok, vox1.why.as_deref()),
            (TrackState::Dry, TrackState::Dry, true, None)
        );
        a.close().await;
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn the_manual_off_survives_a_hub_restart() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let host = Host::start_site("band", &site(dir.path(), 10.0), 0, 0.0);
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_10, |s| ready(s) && keyed(s, "C"))
            .await;
        let mut a = hub.client().await;
        set_hub(
            &mut a,
            "autotune_singer",
            json!({"id": "VOX1", "off": true}),
        )
        .await;
        hub.status_until(SECS_10, |s| s.autotune.singers_off == ["VOX1"])
            .await;
        let saved: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("hub-state.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved["autotune_off"], json!(["VOX1"]));
        a.close().await;
        hub.stop().await;
        // A new hub on the same data: the singer's off kept, told to the
        // pages.
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let status = hub
            .status_until(SECS_10, |s| ready(s) && keyed(s, "C"))
            .await;
        assert_eq!(status.autotune.singers_off, ["VOX1"]);
        let mut a = hub.client().await;
        let state = a
            .wait(SECS_10, |m| match m {
                ServerMsg::Autotune(s) if s.singers.len() == 2 => Some(s.clone()),
                _ => None,
            })
            .await;
        assert_eq!(
            state
                .singers
                .iter()
                .map(|s| (s.id.as_str(), s.off))
                .collect::<Vec<_>>(),
            [("VOX1", true), ("VOX2", false)]
        );
        a.close().await;
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn the_pages_are_told_the_singers_and_the_key() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let host = Host::start_site("band", &site(dir.path(), 10.0), 0, 0.0);
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut a = hub.client().await;
        let state = a
            .wait(SECS_10, |m| match m {
                ServerMsg::Autotune(s)
                    if s.key.as_deref() == Some("C")
                        && s.singers.len() == 2
                        && s.map_keyed == 2 =>
                {
                    Some(s.clone())
                }
                _ => None,
            })
            .await;
        assert_eq!(state.song, Some(1));
        assert_eq!((state.map_keyed, state.map_changes), (2, 0));
        let singers: Vec<(&str, &str, TrackState, TrackState, bool)> = state
            .singers
            .iter()
            .map(|s| (s.id.as_str(), s.label.as_str(), s.stream, s.pa, s.off))
            .collect();
        assert_eq!(
            singers,
            vec![
                ("VOX1", "Vocal A", TrackState::Dry, TrackState::Dry, false),
                ("VOX2", "Vocal B", TrackState::Dry, TrackState::Dry, false),
            ]
        );
        assert!(!state.aut);
        a.close().await;
        hub.stop().await;
        host.stop();
    });
}
