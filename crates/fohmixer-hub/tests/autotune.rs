//! AUTOTUNE AUT end to end (#104, D19) on SimLive: the `+AUTOTUNE` Tuners
//! of a set of our own make the rule's targets; the hub finds each track's
//! `Latencies` rack (one rack deep on a stream track, on the track itself
//! on a repro track), says why a track has none or two, and writes each
//! rack's first macro through its `$ref`: AUT's turn on writes the
//! transport, a start writes on and a stop off. A tap overrides until the
//! next change; AUT survives a hub restart; a host restart (a set load)
//! writes once after the walk found the racks again.
#![cfg(unix)]

mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fohmixer_proto::client::{AutotuneState, ClientMsg, HubStatus, ServerMsg, TargetState};
use serde_json::{Value, json};
use support::{Client, Host, TestHub, runtime, serial};

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

/// Walked, and the band's transport heard (AUT's turn on writes it).
fn ready(s: &HubStatus) -> bool {
    walked(s) && s.instances[0].playing.is_some()
}

/// Live's value of the macro at `path`, read with a command.
async fn macro_value(c: &mut Client, path: &str) -> f64 {
    c.get("band", path, "value").await.as_f64().unwrap()
}

/// Polls the macro at `path` every 50 ms until it reads `want` (5 s at
/// most).
async fn until_macro(c: &mut Client, path: &str, want: f64) {
    let deadline = Instant::now() + SECS_5;
    loop {
        let v = macro_value(c, path).await;
        if v == want {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{path} stayed at {v}, not {want}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
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

/// The first `autotune` message that passes `check`.
async fn autotune_until(
    client: &mut Client,
    check: impl Fn(&AutotuneState) -> bool,
) -> AutotuneState {
    client
        .wait(SECS_5, |m| match m {
            ServerMsg::Autotune(s) if check(s) => Some(s.clone()),
            _ => None,
        })
        .await
}

fn states(s: &AutotuneState) -> Vec<TargetState> {
    s.targets.iter().map(|t| t.state).collect()
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
fn with_aut_on_both_kinds_of_rack_follow_the_transport() {
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
        // The stream rack's macro between its values, so the flag's write
        // shows.
        a.set("band", STREAM_MACRO, "value", json!(64.0)).await;
        set_hub(&mut a, "autotune_aut", true).await;
        until_macro(&mut a, STREAM_MACRO, 0.0).await;
        until_macro(&mut a, REPRO_MACRO, 127.0).await;
        let off = autotune_until(&mut a, |s| {
            s.aut && s.targets.len() == 4 && s.targets[1].state == TargetState::Off
        })
        .await;
        assert_eq!(
            states(&off),
            vec![
                TargetState::Off,
                TargetState::Off,
                TargetState::Missing,
                TargetState::Missing
            ]
        );
        transport(&mut a, "start_playing").await;
        until_macro(&mut a, STREAM_MACRO, 127.0).await;
        until_macro(&mut a, REPRO_MACRO, 0.0).await;
        let on = autotune_until(&mut a, |s| {
            s.playing == Some(true)
                && s.targets.len() == 4
                && s.targets[..2].iter().all(|t| t.state == TargetState::On)
        })
        .await;
        assert!(on.aut);
        transport(&mut a, "stop_playing").await;
        until_macro(&mut a, STREAM_MACRO, 0.0).await;
        until_macro(&mut a, REPRO_MACRO, 127.0).await;
        let status = hub.status_until(SECS_5, |s| s.autotune.writes == 3).await;
        assert!(status.autotune.aut);
        let records = hub
            .events_until(SECS_5, |r| count(&autotune_since(r, 0), "write") == 3)
            .await;
        let steps = autotune_since(&records, 0);
        assert_eq!(count(&steps, "flag"), 1);
        assert_eq!(count(&steps, "found"), 2);
        assert_eq!(count(&steps, "missing"), 2);
        assert_eq!(count(&steps, "failed"), 0);
        let whys: Vec<(Value, Value)> = steps
            .iter()
            .filter(|r| r["step"] == "write")
            .map(|r| (r["why"].clone(), r["on"].clone()))
            .collect();
        assert_eq!(
            whys,
            vec![
                (json!("flag"), json!(false)),
                (json!("transport"), json!(true)),
                (json!("transport"), json!(false)),
            ]
        );
        assert!(
            records
                .iter()
                .filter(|r| r["ev"] == "transport" && r["instance"] == "band")
                .count()
                >= 2
        );
        a.close().await;
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_tap_overrides_until_the_next_change_and_aut_survives_a_hub_restart() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let host = Host::start_site("band", &site(dir.path()), 0, 0.0);
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_5, ready).await;
        let mut a = hub.client().await;
        set_hub(&mut a, "autotune_aut", true).await;
        transport(&mut a, "start_playing").await;
        until_macro(&mut a, STREAM_MACRO, 127.0).await;
        until_macro(&mut a, REPRO_MACRO, 0.0).await;
        // A tap turns them off while Live plays.
        set_hub(&mut a, "autotune", false).await;
        until_macro(&mut a, STREAM_MACRO, 0.0).await;
        until_macro(&mut a, REPRO_MACRO, 127.0).await;
        // Live told to play again (it plays): no change, the tap holds.
        transport(&mut a, "start_playing").await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(macro_value(&mut a, STREAM_MACRO).await, 0.0);
        assert_eq!(macro_value(&mut a, REPRO_MACRO).await, 127.0);
        // The stop writes off (as they are), the start on.
        transport(&mut a, "stop_playing").await;
        hub.status_until(SECS_5, |s| s.autotune.writes == 3).await;
        transport(&mut a, "start_playing").await;
        until_macro(&mut a, STREAM_MACRO, 127.0).await;
        until_macro(&mut a, REPRO_MACRO, 0.0).await;
        let status = hub.status_until(SECS_5, |s| s.autotune.writes == 4).await;
        assert_eq!(status.autotune.taps, 1);
        // Both off by hand while Live plays: the rule does not follow Live's
        // values, only the transport.
        a.set("band", STREAM_MACRO, "value", json!(0.0)).await;
        a.set("band", REPRO_MACRO, "value", json!(127.0)).await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(macro_value(&mut a, STREAM_MACRO).await, 0.0);
        a.close().await;
        hub.stop().await;
        let saved: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("hub-state.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(saved["autotune_aut"], json!(true));
        // A new hub on the same data: AUT is on at once and writes the
        // transport it finds (Live plays) once the racks are found.
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut b = hub.client().await;
        let first = b
            .wait(SECS_5, |m| match m {
                ServerMsg::Autotune(s) => Some(s.clone()),
                _ => None,
            })
            .await;
        assert!(first.aut, "told at attach");
        until_macro(&mut b, STREAM_MACRO, 127.0).await;
        until_macro(&mut b, REPRO_MACRO, 0.0).await;
        let status = hub
            .status_until(SECS_5, |s| walked(s) && s.autotune.writes == 1)
            .await;
        assert!(status.autotune.aut);
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(hub.status().await.autotune.writes, 1, "once");
        transport(&mut b, "stop_playing").await;
        b.close().await;
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_host_restart_writes_once_after_the_walk_found_the_racks_again() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let set = site(dir.path());
        let host = Host::start_site("band", &set, 0, 0.0);
        let port = host.port;
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_5, ready).await;
        let mut a = hub.client().await;
        set_hub(&mut a, "autotune_aut", true).await;
        until_macro(&mut a, REPRO_MACRO, 127.0).await;
        hub.status_until(SECS_5, |s| s.autotune.writes == 1).await;
        // A set load: SimLive comes back with its racks at 0 and stopped.
        host.stop();
        a.instance_state("band", false, None, SECS_5).await;
        let reloaded = now_ts();
        let host = Host::start_site("band", &set, port, 0.0);
        let status = hub
            .status_until(SECS_5, |s| {
                s.instances[0].online && walked(s) && s.autotune.writes == 2
            })
            .await;
        assert!(status.autotune.aut);
        // Stopped: off, written through the new connection's refs.
        until_macro(&mut a, REPRO_MACRO, 127.0).await;
        assert_eq!(macro_value(&mut a, STREAM_MACRO).await, 0.0);
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(hub.status().await.autotune.writes, 2, "once");
        let records = hub
            .events_until(SECS_5, |r| {
                count(&autotune_since(r, reloaded), "found") == 2
            })
            .await;
        let steps = autotune_since(&records, reloaded);
        assert_eq!(count(&steps, "failed"), 0, "{steps:?}");
        assert!(count(&steps, "write") >= 1, "{steps:?}");
        a.close().await;
        hub.stop().await;
        host.stop();
    });
}
