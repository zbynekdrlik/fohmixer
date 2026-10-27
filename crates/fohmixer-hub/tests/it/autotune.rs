//! AUTOTUNE end to end (#104, D19; #140 PR 2a, PR 3b) on SimLive: the
//! `+AUTOTUNE` Tuners of a set of our own make the targets; the hub finds
//! each track's `Latencies` rack (one rack deep on a stream track, on the
//! track itself on a repro track) and says why a track has none or two.
//! This set has no song, so nothing tunes: AUT off the hub writes DRY once
//! to the repro rack, which stands on its chain (its macro at 0), AUT on
//! is taken and kept across a restart (#140 PR 3c lifted 3b's refusal), a
//! tap is refused, no transport change writes, and a host restart (a set
//! load) walks the racks again and writes DRY once more. The keys are
//! `tests/it/autotune_keys.rs`', the switching `tests/it/autotune_switch.rs`'.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::support::{Client, Host, TestHub, runtime, serial};
use fohmixer_proto::client::{ClientMsg, HubStatus, ServerMsg, SwitchWhy};
use serde_json::{Value, json};

const SECS_5: Duration = Duration::from_secs(5);

/// The stream track's macro: its Latencies rack inside a rack's chain.
const STREAM_MACRO: &str = "live_set tracks 0 devices 0 chains 0 devices 0 parameters 1";
/// The repro track's macro: its Latencies rack on the track itself.
const REPRO_MACRO: &str = "live_set tracks 1 devices 0 parameters 1";

/// The test's own set (synthetic names): a stream track, a repro track, a
/// track without the rack and one with two, each with its Tuner marker
/// after its racks (a set load brings them back).
fn site(dir: &Path) -> PathBuf {
    let tuner = |name: &str| json!({"template": "plain", "name": name, "class_name": "Tuner"});
    let latencies =
        |chains: Value| json!({"template": "rack", "name": "Latencies", "chains": chains});
    let set = json!({"name": "Autotune Site", "tracks": [
        {"name": "Stream #", "devices": [
            {"template": "rack", "name": "Stream FX", "chains": [
                {"name": "Chain 1", "devices": [latencies(json!([{"name": "Dry"}, {"name": "Auto"}]))]}]},
            tuner("+AUTOTUNE")]},
        {"name": "Repro #", "devices": [
            latencies(json!([{"name": "Auto"}, {"name": "Dry"}])), tuner("+AUTOTUNE:0")]},
        {"name": "Plain #", "devices": [tuner("+AUTOTUNE")]},
        {"name": "Twice #", "devices": [
            latencies(json!([])), latencies(json!([])), tuner("+AUTOTUNE")]},
    ]});
    let path = dir.join("autotune-site.json");
    std::fs::write(&path, set.to_string()).unwrap();
    path
}

/// One page whose rail holds the AUTOTUNE button.
fn frame(dir: &Path) {
    let frame = json!({"schema": 2, "default_page": "p", "pages": [{"id": "p", "title": "P",
        "rail": [{"kind": "autotune", "label": "AUTOTUNE"}]}]});
    std::fs::write(dir.join("layout.json"), frame.to_string()).unwrap();
}

/// The status once the four targets are composed and walked: the stream
/// and repro racks found, the others missing.
fn walked(s: &HubStatus) -> bool {
    let t = &s.autotune.targets;
    t.len() == 4
        && t[0].path.is_some()
        && t[1].path.is_some()
        && t[2].problem.is_some()
        && t[3].problem.is_some()
}

/// Walked, and the band's transport heard.
fn ready(s: &HubStatus) -> bool {
    walked(s) && s.instances[0].playing.is_some()
}

/// Live's value of the macro at `path`, read with a command.
async fn macro_value(c: &mut Client, path: &str) -> f64 {
    c.get("band", path, "value").await.as_f64().unwrap()
}

/// Both macros as Live holds them (stream, repro).
async fn macros(c: &mut Client) -> (f64, f64) {
    (
        macro_value(c, STREAM_MACRO).await,
        macro_value(c, REPRO_MACRO).await,
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

async fn set_hub(c: &mut Client, key: &str, value: bool) {
    c.send(&ClientMsg::SetHub {
        key: key.into(),
        value: json!(value),
    })
    .await;
}

/// The hub's UTC clock now (the event records' `ts`).
fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// The `autotune` records written at or after `since`.
fn autotune_since(records: &[Value], since: i64) -> Vec<Value> {
    records
        .iter()
        .filter(|r| r["ev"] == "autotune" && r["ts"].as_i64().is_some_and(|ts| ts >= since))
        .cloned()
        .collect()
}

/// How many of `records` are the step `step`.
fn count(records: &[Value], step: &str) -> usize {
    records.iter().filter(|r| r["step"] == step).count()
}

#[test]
fn both_kinds_of_rack_are_found_and_only_dry_is_written() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let host = Host::start_site("band", &site(dir.path()), 0, 0.0);
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let status = hub.status_until(SECS_5, ready).await;
        let targets = &status.autotune.targets;
        assert_eq!(targets[0].path.as_deref(), Some(STREAM_MACRO));
        assert_eq!(targets[1].path.as_deref(), Some(REPRO_MACRO));
        assert_eq!((targets[0].on, targets[1].on), (127, 0));
        assert_eq!(
            targets[2].problem.as_deref(),
            Some("no rack named Latencies")
        );
        assert_eq!(
            targets[3].problem.as_deref(),
            Some("2 racks named Latencies")
        );
        assert!(!status.autotune.aut);
        let mut a = hub.client().await;
        // AUT off: the repro rack stands on its chain (0), so the hub
        // writes it DRY (127) once (#140 PR 3b); the stream rack is DRY.
        let status = hub.status_until(SECS_5, |s| s.autotune.writes == 1).await;
        let decisions: Vec<_> = status
            .autotune
            .targets
            .iter()
            .map(|t| t.decision.map(|d| (d.on, d.why)))
            .collect();
        assert_eq!(
            decisions[..2],
            [Some((false, SwitchWhy::Aut)), Some((false, SwitchWhy::Aut))]
        );
        // AUT on is taken (#140 PR 3c; no song: nothing tunes), a tap is
        // refused (since PR 3b).
        set_hub(&mut a, "autotune_aut", true).await;
        set_hub(&mut a, "autotune", false).await;
        let records = hub
            .events_until(SECS_5, |r| count(&autotune_since(r, 0), "refused") == 1)
            .await;
        let refused: Vec<Value> = autotune_since(&records, 0)
            .into_iter()
            .filter(|r| r["step"] == "refused")
            .map(|r| r["what"].clone())
            .collect();
        assert_eq!(refused, vec![json!("tap")]);
        // Live plays, then stops: out of every song, nothing is written.
        transport(&mut a, "start_playing").await;
        hub.status_until(SECS_5, |s| s.instances[0].playing == Some(true))
            .await;
        transport(&mut a, "stop_playing").await;
        hub.status_until(SECS_5, |s| s.instances[0].playing == Some(false))
            .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(macros(&mut a).await, (0.0, 127.0), "only the DRY moved");
        let status = hub.status().await;
        assert_eq!(
            (
                status.autotune.aut,
                status.autotune.writes,
                status.autotune.taps
            ),
            (true, 1, 0)
        );
        let steps = autotune_since(&hub.events(), 0);
        assert_eq!(count(&steps, "found"), 2);
        assert_eq!(count(&steps, "missing"), 2);
        assert_eq!(count(&steps, "write"), 1);
        assert_eq!(count(&steps, "select"), 0, "never on");
        assert_eq!(count(&steps, "flag"), 1);
        assert_eq!(count(&steps, "tap"), 0);
        a.close().await;
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_saved_aut_on_is_kept() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let host = Host::start_site("band", &site(dir.path()), 0, 0.0);
        frame(dir.path());
        // AUTOTUNE AUT and a singer's off by hand saved on: both kept across
        // a restart (#140 PR 3b; AUT honoured again since PR 3c).
        let state = json!({"stage_aut": false, "autotune_aut": true, "autotune_off": ["VOX1"]});
        std::fs::write(dir.path().join("hub-state.json"), state.to_string()).unwrap();
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut a = hub.client().await;
        let first = a
            .wait(SECS_5, |m| match m {
                ServerMsg::Autotune(s) => Some(s.clone()),
                _ => None,
            })
            .await;
        assert!(first.aut, "told on at attach");
        let saved: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("hub-state.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved, state, "kept as it was");
        assert_eq!(hub.status().await.autotune.singers_off, ["VOX1"]);
        // Live plays once the racks are found: out of every song, only the
        // repro rack's DRY is written.
        hub.status_until(SECS_5, ready).await;
        transport(&mut a, "start_playing").await;
        hub.status_until(SECS_5, |s| s.instances[0].playing == Some(true))
            .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(macros(&mut a).await, (0.0, 127.0));
        let status = hub.status().await;
        assert_eq!((status.autotune.aut, status.autotune.writes), (true, 1));
        transport(&mut a, "stop_playing").await;
        a.close().await;
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_host_restart_walks_the_racks_again_and_writes_dry_once_more() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let set = site(dir.path());
        let host = Host::start_site("band", &set, 0, 0.0);
        let port = host.port;
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_5, |s| ready(s) && s.autotune.writes == 1)
            .await;
        let mut a = hub.client().await;
        // A set load: SimLive comes back as it was, stopped (the repro rack
        // on its chain again).
        host.stop();
        a.instance_state("band", false, None, SECS_5).await;
        let reloaded = now_ts();
        let host = Host::start_site("band", &set, port, 0.0);
        let records = hub
            .events_until(SECS_5, |r| {
                count(&autotune_since(r, reloaded), "found") == 2
            })
            .await;
        hub.status_until(SECS_5, |s| {
            s.instances[0].online && walked(s) && s.autotune.writes == 2
        })
        .await;
        let steps = autotune_since(&hub.events(), reloaded);
        assert_eq!(count(&steps, "failed"), 0, "{steps:?}");
        assert_eq!(count(&steps, "write"), 1, "{steps:?}");
        assert_eq!(macros(&mut a).await, (0.0, 127.0));
        let _ = records;
        a.close().await;
        hub.stop().await;
        host.stop();
    });
}
