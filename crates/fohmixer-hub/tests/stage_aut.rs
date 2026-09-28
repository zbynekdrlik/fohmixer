//! STAGE AUT end to end (S3 plan, Task 5; spec F15, X9): with the flag on,
//! the band's stage-mic track (the layout's `stage` item with `aut`) is
//! muted while the transport plays and live when it stops; the flag is kept
//! across a hub restart; a host restart gives exactly one write after the
//! reconnect.
#![cfg(unix)]

mod support;

use std::path::Path;
use std::time::Duration;

use fohmixer_hub::config::InstanceCfg;
use fohmixer_proto::client::{ClientMsg, ServerMsg};
use serde_json::json;
use support::{Client, Host, TestHub, runtime, serial};

const SECS_3: Duration = Duration::from_secs(3);
const STAGE: &str = "live_set tracks[name=Mics Stage #]";

fn with_layout(dir: &Path) {
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/layout-ok.json"),
        dir.join("layout.json"),
    )
    .unwrap();
}

/// The band host and a master nothing listens for (the layout binds both
/// instances; STAGE AUT acts on the band only).
fn instances(band: &Host) -> Vec<InstanceCfg> {
    vec![
        band.cfg(),
        InstanceCfg {
            name: "master".into(),
            port: 1,
        },
    ]
}

async fn transport(client: &mut Client, verb: &str) {
    let slots = client
        .cmd(
            "band",
            vec![json!({"target": "live_set", "name": verb, "args": []})],
        )
        .await
        .unwrap();
    assert_eq!(slots[0]["ok"], json!(true), "{slots:?}");
}

async fn set_flag(client: &mut Client, on: bool) {
    client
        .send(&ClientMsg::SetHub {
            key: "stage_aut".into(),
            value: json!(on),
        })
        .await;
    client
        .wait(SECS_3, move |m| {
            matches!(m, ServerMsg::Hub { key, value } if key == "stage_aut" && *value == json!(on))
                .then_some(())
        })
        .await;
}

#[test]
fn with_the_flag_on_the_stage_mics_mute_while_playing() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        with_layout(dir.path());
        let hub = TestHub::start(instances(&host), dir.path()).await;
        hub.status_until(SECS_3, |s| s.layout.rev == 1 && s.instances[0].online)
            .await;
        let mut a = hub.client().await;
        let mute = a.sub_key("band", STAGE, "mute", false).await;
        a.value_until(&mute, SECS_3, |i| i.value == Some(json!(false)))
            .await;
        // Turning the flag on applies the rule at once: stopped → live.
        set_flag(&mut a, true).await;
        hub.status_until(SECS_3, |s| s.stage_aut.on && s.stage_aut.writes == 1)
            .await;
        transport(&mut a, "start_playing").await;
        a.value_until(&mute, SECS_3, |i| i.value == Some(json!(true)))
            .await;
        hub.status_until(SECS_3, |s| s.stage_aut.writes == 2).await;
        transport(&mut a, "stop_playing").await;
        a.value_until(&mute, SECS_3, |i| i.value == Some(json!(false)))
            .await;
        hub.status_until(SECS_3, |s| s.stage_aut.writes == 3).await;
        // Flag off: the transport is not followed.
        set_flag(&mut a, false).await;
        transport(&mut a, "start_playing").await;
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(a.get("band", STAGE, "mute").await, json!(false));
        let status = hub.status().await;
        assert!(!status.stage_aut.on);
        assert_eq!(status.stage_aut.writes, 3);
        let saved = std::fs::read_to_string(dir.path().join("hub-state.json")).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&saved).unwrap(),
            json!({"stage_aut": false})
        );
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn the_flag_survives_a_hub_restart_and_a_host_restart_writes_once() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let port = host.port;
        let dir = tempfile::tempdir().unwrap();
        with_layout(dir.path());
        let hub = TestHub::start(instances(&host), dir.path()).await;
        let mut a = hub.client().await;
        set_flag(&mut a, true).await;
        hub.status_until(SECS_3, |s| s.stage_aut.writes == 1).await;
        a.close().await;
        hub.stop().await;
        // A new hub on the same data folder: the flag is on, the rule writes
        // once for the transport state it finds.
        let hub = TestHub::start(instances(&host), dir.path()).await;
        let mut a = hub.client().await;
        a.wait(SECS_3, |m| {
            matches!(m, ServerMsg::Hub { key, value } if key == "stage_aut" && *value == json!(true))
                .then_some(())
        })
        .await;
        hub.status_until(SECS_3, |s| s.stage_aut.on && s.stage_aut.writes == 1)
            .await;
        // A host restart (a set load): exactly one write after the reconnect.
        host.stop();
        a.instance_state("band", false, None, SECS_3).await;
        let host = Host::start_with("band", port, 0.0);
        hub.status_until(SECS_3, |s| s.instances[0].online && s.stage_aut.writes == 2)
            .await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(hub.status().await.stage_aut.writes, 2, "one write per reconnect");
        transport(&mut a, "start_playing").await;
        hub.status_until(SECS_3, |s| s.stage_aut.writes == 3).await;
        assert_eq!(a.get("band", STAGE, "mute").await, json!(true));
        hub.stop().await;
        host.stop();
    });
}
