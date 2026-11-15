//! The scene read from Companion end to end (#207, `[scenes]`): the hub
//! reads a fake Companion's custom variable every 500 ms; its first value
//! moves nothing, a change is a scene that glides a SimLive fader by its
//! rule, an outage is said once, and the value it reads when Companion is
//! back moves nothing until it changes. No automation token: the poll turns
//! the scenes on by itself.
#![cfg(unix)]

use std::time::Duration;

use crate::scenes::{SECS_5, glides, heard, layout, set_rule, volume};
use crate::support::companion_http::FakeCompanionHttp;
use crate::support::{Host, TestHub, runtime, serial};
use fohmixer_hub::config::{CompanionCfg, Config, ScenesCfg};
use fohmixer_proto::scenes::SceneRule;
use fohmixer_proto::volume::{value2db, volume_of};
use serde_json::{Value, json};

const SECS_10: Duration = Duration::from_secs(10);

/// The `scene` records, in order.
fn scenes(records: &[Value]) -> Vec<Value> {
    records
        .iter()
        .filter(|r| r["ev"] == "scene")
        .cloned()
        .collect()
}

/// The `scene_source` records, in order.
fn sources(records: &[Value]) -> Vec<Value> {
    records
        .iter()
        .filter(|r| r["ev"] == "scene_source")
        .cloned()
        .collect()
}

/// Waits until the fake answered `more` reads after now.
async fn reads_more(fake: &FakeCompanionHttp, more: u64) {
    let target = fake.reads() + more;
    let start = std::time::Instant::now();
    while fake.reads() < target {
        assert!(start.elapsed() < SECS_10, "the hub stopped reading");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[test]
fn companions_variable_moves_faders_only_when_it_changes() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        layout(dir.path());
        let mut fake = FakeCompanionHttp::start("scena", "start").await;
        let mut config = Config::defaults(dir.path());
        config.instances = vec![host.cfg()];
        config.layout_poll_ms = 100;
        config.lufs.port = 0;
        // Companion's Satellite port is closed (the Stream Deck stays
        // offline); its web server is the fake.
        config.companion = Some(CompanionCfg {
            host: "127.0.0.1".into(),
            port: 1,
            columns: 8,
            rows: 4,
            bitmap_px: 144,
            title: "Stream Deck".into(),
        });
        config.scenes = Some(ScenesCfg {
            companion_variable: "scena".into(),
            companion_http_port: fake.addr.port(),
        });
        let peer = format!("127.0.0.1:{}", fake.addr.port());
        let hub = TestHub::start_config(config).await;
        let status = hub
            .status_until(SECS_5, |s| {
                s.instances[0].online
                    && s.scenes.companion.ok == Some(true)
                    && s.scenes.companion.last.as_deref() == Some("start")
            })
            .await;
        // The poll turns the scenes on without a token.
        assert!(status.scenes.on);
        assert!(status.scenes.companion.on);
        assert_eq!(status.scenes.companion.error, None);
        assert!(status.scenes.companion.since.is_some());
        // The first value moves nothing and learns nothing.
        reads_more(&fake, 2).await;
        let records = hub.events();
        assert!(scenes(&records).is_empty(), "{records:?}");
        let up = sources(&records);
        assert_eq!(up.len(), 1, "{up:?}");
        assert_eq!(up[0]["up"], true);
        assert_eq!(up[0]["peer"], json!(peer));
        assert_eq!(hub.status().await.scenes.known, 0);

        // A change is a scene: learned, nothing to move yet.
        fake.set("intro");
        let records = hub.events_until(SECS_5, |r| !scenes(r).is_empty()).await;
        let first = &scenes(&records)[0];
        assert_eq!(first["scene"], "intro");
        assert_eq!(first["source"], "companion");
        assert_eq!(first["peer"], json!(peer));
        assert_eq!(first["moved"], 0);
        let mut page = hub.client().await;
        set_rule(
            &mut page,
            "r1",
            "intro",
            Some(SceneRule {
                db: Some(-10.0),
                mute: None,
                time_ms: 1000,
            }),
        )
        .await;
        heard(&mut page).await;
        assert!(
            value2db(volume(&mut page).await).abs() < 0.05,
            "starts at 0 dB"
        );
        // Away and back: the rule's glide.
        fake.set("talk");
        hub.events_until(SECS_5, |r| scenes(r).len() >= 2).await;
        fake.set("intro");
        let records = hub
            .events_until(SECS_5, |r| {
                glides(r)
                    .iter()
                    .any(|g| g["what"] == "stop" && g["why"] == "done")
            })
            .await;
        let names: Vec<Value> = scenes(&records)
            .iter()
            .map(|s| s["scene"].clone())
            .collect();
        assert_eq!(names, vec!["intro", "talk", "intro"]);
        assert_eq!(scenes(&records)[2]["moved"], 1);
        let v = volume(&mut page).await;
        assert_eq!(v, volume_of(-10.0), "lands at −10 dB exactly");
        set_rule(
            &mut page,
            "r2",
            "talk",
            Some(SceneRule {
                db: Some(0.0),
                mute: None,
                time_ms: 500,
            }),
        )
        .await;

        // An outage: said once, the status says why.
        fake.down();
        let status = hub
            .status_until(SECS_5, |s| s.scenes.companion.ok == Some(false))
            .await;
        assert!(status.scenes.companion.error.is_some());
        assert_eq!(status.scenes.companion.last.as_deref(), Some("intro"));
        tokio::time::sleep(Duration::from_millis(1200)).await;
        let records = hub.events();
        let down: Vec<Value> = sources(&records)
            .into_iter()
            .filter(|r| r["up"] == false)
            .collect();
        assert_eq!(down.len(), 1, "one record per outage: {down:?}");
        assert_eq!(down[0]["failures"], 2);
        // Back with the same value: up again, nothing moves.
        fake.up().await;
        hub.status_until(SECS_5, |s| s.scenes.companion.ok == Some(true))
            .await;
        reads_more(&fake, 3).await;
        let records = hub.events();
        assert_eq!(
            scenes(&records).len(),
            3,
            "the value after the outage moved"
        );
        let back = sources(&records).last().cloned().unwrap();
        assert_eq!(back["up"], true);
        assert!(back["down_ms"].as_u64().unwrap() >= 1000, "{back}");
        assert_eq!(volume(&mut page).await, volume_of(-10.0));
        // A new value after the outage moves.
        fake.set("talk");
        let records = hub
            .events_until(SECS_5, |r| {
                glides(r)
                    .iter()
                    .filter(|g| g["what"] == "stop" && g["why"] == "done")
                    .count()
                    >= 2
            })
            .await;
        let last = scenes(&records).last().cloned().unwrap();
        assert_eq!(
            (&last["scene"], &last["moved"]),
            (&json!("talk"), &json!(1))
        );
        assert_eq!(volume(&mut page).await, volume_of(0.0));
        let status = hub.status().await.scenes;
        assert_eq!(status.last.as_deref(), Some("talk"));
        assert_eq!(status.known, 2);
        assert_eq!(status.companion.last.as_deref(), Some("talk"));
        hub.stop().await;
        drop(host);
    });
}
