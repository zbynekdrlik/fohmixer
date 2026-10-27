//! REVERB end to end (#130, D18): the `+REVERB` markers make the reverb
//! faders; the hub serves them as reverb strips and tells every page the
//! rule's state. REVERB AUT lowers them on a stop of SimLive's transport and
//! returns them exactly at the start; the hand wins; the lowered state
//! survives a hub restart; a host restart (a set load) lowers nothing again.
#![cfg(unix)]

use std::path::Path;
use std::time::{Duration, Instant};

use crate::support::{Client, Host, TestHub, runtime, serial};
use fohmixer_proto::client::{ClientMsg, Low, ReverbState, ServerMsg, hub_key};
use fohmixer_proto::layout::{Control, LayoutResponse};
use fohmixer_proto::volume::{value2db, volume_of};
use serde_json::{Value, json};

const SECS_5: Duration = Duration::from_secs(5);

/// One page whose group shows the tag group FX, and the REVERB button.
fn frame(dir: &Path) {
    let frame = json!({"schema": 2, "default_page": "p", "pages": [{"id": "p", "title": "P",
        "rail": [{"kind": "reverb", "label": "REVERB"}],
        "rows": [{"sections": [{"kind": "group", "id": "fx", "title": "FX", "tags": "FX"}]}]}]});
    std::fs::write(dir.join("layout.json"), frame.to_string()).unwrap();
}

/// The two reverb faders on SimLive's band: Hand3 # and Hand4 #.
fn tag(host: &mut Host) {
    assert_eq!(
        host.tuner("track", 2, "set", r#""Rev ST" +G:FX +REVERB"#),
        1
    );
    assert_eq!(
        host.tuner("track", 3, "set", r#""Rev PA" +G:FX +REVERB"#),
        1
    );
}

/// The first `reverb` message that passes `check`.
async fn reverb_until(client: &mut Client, check: impl Fn(&ReverbState) -> bool) -> ReverbState {
    client
        .wait(SECS_5, |m| match m {
            ServerMsg::Reverb(s) if check(s) => Some(s.clone()),
            _ => None,
        })
        .await
}

/// SimLive's band track `index`: its volume's target.
fn volume_target(index: u32) -> String {
    format!("live_set tracks {index} mixer_device volume")
}

/// Live's volume of band track `index`, read with a command.
async fn volume(c: &mut Client, index: u32) -> f64 {
    c.get("band", &volume_target(index), "value")
        .await
        .as_f64()
        .unwrap()
}

/// Polls track `index`'s volume every 50 ms until `ok` (6 s at most).
async fn until_volume(c: &mut Client, index: u32, ok: impl Fn(f64) -> bool) -> f64 {
    let deadline = Instant::now() + Duration::from_secs(6);
    loop {
        let v = volume(c, index).await;
        if ok(v) {
            return v;
        }
        assert!(
            Instant::now() < deadline,
            "track {index}'s volume stayed at {v}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The level 10 dB under `v` on the strips' law (the rule's own sum).
fn lowered(v: f64) -> f64 {
    volume_of(value2db(v) - 10.0)
}

fn db_close(a: f64, b: f64) -> bool {
    (value2db(a) - value2db(b)).abs() < 0.05
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

/// Waits until the hub holds Live's value of track `index`'s volume. The
/// rule's subscription is the same entry, so it had the value in the same
/// flush.
async fn heard(c: &mut Client, index: u32) {
    let target = volume_target(index);
    if let ServerMsg::Subbed { value: Some(_), .. } = c.sub("band", &target, "value", false).await {
        return;
    }
    let sub = hub_key("band", &target, "value", false);
    c.value_until(&sub, SECS_5, |i| i.value.is_some()).await;
}

/// The hub's UTC clock now (the event records' `ts`).
fn now_ts() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// The `reverb` records' steps written at or after `since` (hub UTC ms).
fn steps_since(records: &[Value], since: i64) -> Vec<Value> {
    records
        .iter()
        .filter(|r| r["ev"] == "reverb" && r["ts"].as_i64().is_some_and(|ts| ts >= since))
        .map(|r| r["step"].clone())
        .collect()
}

#[test]
fn the_reverb_faders_are_told_and_served() {
    let _serial = serial();
    runtime().block_on(async {
        let mut host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_5, |s| s.instances[0].online).await;
        tag(&mut host);
        let mut a = hub.client().await;
        let state = reverb_until(&mut a, |s| s.faders.len() == 2).await;
        assert_eq!(
            state
                .faders
                .iter()
                .map(|f| f.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Rev ST", "Rev PA"]
        );
        assert_eq!(
            state.faders[0].key,
            "band|live_set tracks 2 mixer_device volume|value"
        );
        assert!(!state.aut && state.low.is_none() && !state.waiting);
        let (_, body) = hub.get("/api/layout").await;
        let layout: LayoutResponse = serde_json::from_value(body).unwrap();
        let strips: Vec<_> = layout.layout.pages[0]
            .controls()
            .into_iter()
            .filter_map(|c| match c {
                Control::Strip(s) => Some(s.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(strips.len(), 2);
        assert!(strips.iter().all(|s| s.reverb));
        let status = hub.status().await;
        assert_eq!(status.reverb.faders, 2);
        assert!(!status.reverb.aut);
        a.close().await;
        hub.stop().await;
        host.tuner("track", 2, "remove", "");
        host.tuner("track", 3, "remove", "");
        host.stop();
    });
}

#[test]
fn reverb_aut_lowers_on_a_stop_returns_on_a_start_and_the_hand_wins() {
    let _serial = serial();
    runtime().block_on(async {
        let mut host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_5, |s| s.instances[0].online).await;
        tag(&mut host);
        let mut a = hub.client().await;
        reverb_until(&mut a, |s| s.faders.len() == 2).await;
        a.set("band", &volume_target(2), "value", json!(0.85)).await;
        a.set("band", &volume_target(3), "value", json!(0.6)).await;
        transport(&mut a, "start_playing").await;
        set_hub(&mut a, "reverb_aut", true).await;
        reverb_until(&mut a, |s| s.aut && s.playing == Some(true)).await;
        transport(&mut a, "stop_playing").await;
        reverb_until(&mut a, |s| s.waiting).await;
        reverb_until(&mut a, |s| s.low == Some(Low::Stop)).await;
        until_volume(&mut a, 2, |v| db_close(v, lowered(0.85))).await;
        until_volume(&mut a, 3, |v| db_close(v, lowered(0.6))).await;
        // A page's write of Rev PA: the hand.
        a.write("band", &volume_target(3), "value", json!(0.5), 1, true)
            .await;
        reverb_until(&mut a, |s| s.faders.len() == 2 && s.faders[1].hand).await;
        transport(&mut a, "start_playing").await;
        until_volume(&mut a, 2, |v| v == 0.85).await;
        tokio::time::sleep(Duration::from_millis(700)).await;
        assert_eq!(volume(&mut a, 3).await, 0.5, "the hand's level stays");
        let records = hub
            .events_until(SECS_5, |r| steps_since(r, 0).contains(&json!("return")))
            .await;
        let steps = steps_since(&records, 0);
        for want in ["aut", "wait", "lower", "arrived", "hand", "return"] {
            assert!(steps.contains(&json!(want)), "{want} in {steps:?}");
        }
        assert!(
            records
                .iter()
                .any(|r| r["ev"] == "set" && r["peer"] == json!("rule:reverb"))
        );
        let status = hub.status().await;
        assert_eq!(status.reverb.hands, 1);
        assert!(status.reverb.writes > 0);
        a.close().await;
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn the_lowered_state_survives_a_hub_restart_and_returns_exactly() {
    let _serial = serial();
    runtime().block_on(async {
        let mut host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_5, |s| s.instances[0].online).await;
        tag(&mut host);
        let mut a = hub.client().await;
        reverb_until(&mut a, |s| s.faders.len() == 2 && s.playing == Some(false)).await;
        a.set("band", &volume_target(2), "value", json!(0.85)).await;
        a.set("band", &volume_target(3), "value", json!(0.6)).await;
        // AUT on while Live stands starts the wait; a tap takes over: the
        // faders are low for the stop (they return at the start).
        set_hub(&mut a, "reverb_aut", true).await;
        reverb_until(&mut a, |s| s.aut && s.waiting).await;
        set_hub(&mut a, "reverb_low", true).await;
        reverb_until(&mut a, |s| s.low == Some(Low::Stop) && !s.waiting).await;
        until_volume(&mut a, 2, |v| db_close(v, lowered(0.85))).await;
        until_volume(&mut a, 3, |v| db_close(v, lowered(0.6))).await;
        a.close().await;
        hub.stop().await;
        let saved: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("hub-state.json")).unwrap(),
        )
        .unwrap();
        let mut keys: Vec<&str> = saved
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["reverb_aut", "reverb_low", "stage_aut"]);
        assert_eq!(saved["reverb_aut"], json!(true));
        assert_eq!(saved["reverb_low"]["by"], json!("stop"));
        let backs: Vec<(Value, Value)> = saved["reverb_low"]["faders"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| (f["label"].clone(), f["back"].clone()))
            .collect();
        assert_eq!(
            backs,
            vec![
                (json!("Rev ST"), json!(0.85)),
                (json!("Rev PA"), json!(0.6))
            ]
        );
        let restarted = now_ts();
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut a = hub.client().await;
        let state = reverb_until(&mut a, |s| {
            s.low == Some(Low::Stop)
                && s.faders.len() == 2
                && s.faders.iter().all(|f| f.back.is_some())
        })
        .await;
        assert!(state.aut);
        assert_eq!(
            state.faders.iter().map(|f| f.back).collect::<Vec<_>>(),
            vec![Some(0.85), Some(0.6)]
        );
        heard(&mut a, 2).await;
        heard(&mut a, 3).await;
        assert_eq!(
            volume(&mut a, 2).await,
            lowered(0.85),
            "Live's first transport value lowered nothing again"
        );
        transport(&mut a, "start_playing").await;
        until_volume(&mut a, 2, |v| v == 0.85).await;
        until_volume(&mut a, 3, |v| v == 0.6).await;
        let records = hub
            .events_until(SECS_5, |r| {
                steps_since(r, restarted).contains(&json!("return"))
            })
            .await;
        let steps = steps_since(&records, restarted);
        assert!(steps.contains(&json!("restored")), "{steps:?}");
        assert!(
            !steps.contains(&json!("wait")) && !steps.contains(&json!("lower")),
            "{steps:?}"
        );
        a.close().await;
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_host_restart_while_lowered_lowers_nothing_again() {
    let _serial = serial();
    runtime().block_on(async {
        let mut host = Host::start("band");
        let port = host.port;
        let dir = tempfile::tempdir().unwrap();
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        hub.status_until(SECS_5, |s| s.instances[0].online).await;
        tag(&mut host);
        let mut a = hub.client().await;
        reverb_until(&mut a, |s| s.faders.len() == 2).await;
        transport(&mut a, "start_playing").await;
        set_hub(&mut a, "reverb_aut", true).await;
        reverb_until(&mut a, |s| s.aut && s.playing == Some(true)).await;
        transport(&mut a, "stop_playing").await;
        reverb_until(&mut a, |s| s.low == Some(Low::Stop)).await;
        // SimLive's fixture: Hand3 # at 0.75, Hand4 # at 0.7.
        until_volume(&mut a, 2, |v| db_close(v, lowered(0.75))).await;
        until_volume(&mut a, 3, |v| db_close(v, lowered(0.7))).await;
        // A set load: SimLive comes back with the fixture's volumes, no
        // Tuner.
        host.stop();
        a.instance_state("band", false, None, SECS_5).await;
        let reloaded = now_ts();
        let mut host = Host::start_with("band", port, 0.0);
        hub.status_until(SECS_5, |s| s.instances[0].online).await;
        tag(&mut host);
        reverb_until(&mut a, |s| {
            s.faders.len() == 2 && s.faders.iter().all(|f| f.hand && f.back.is_none())
        })
        .await;
        assert_eq!(
            volume(&mut a, 2).await,
            0.75,
            "the set's level, never lowered again"
        );
        assert_eq!(volume(&mut a, 3).await, 0.7);
        let records = hub
            .events_until(SECS_5, |r| {
                steps_since(r, reloaded)
                    .iter()
                    .filter(|s| **s == json!("hand"))
                    .count()
                    == 2
            })
            .await;
        let steps = steps_since(&records, reloaded);
        assert!(
            !steps.contains(&json!("wait")) && !steps.contains(&json!("lower")),
            "{steps:?}"
        );
        a.close().await;
        hub.stop().await;
        host.stop();
    });
}
