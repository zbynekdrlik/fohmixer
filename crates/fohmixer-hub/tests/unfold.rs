//! The hub keeps the groups of the strips' tracks unfolded (#58), end to
//! end on a real FohMixer script: Live sends no meter of a track inside a
//! folded group, so a group the layout's strips sit in is unfolded at once
//! whenever it is folded, and when Live comes back; any other group is left
//! as it is.
#![cfg(unix)]

mod support;

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::{Client, Host, TestHub, runtime, serial};

const SECS_3: Duration = Duration::from_secs(3);
const STEMS: &str = "live_set tracks[name=Stems grp#]";
const VOCALS: &str = "live_set tracks[name=Vocals Repro grp#]";

/// One page with one strip, on `Drums #` (inside `Stems grp#` in the test
/// site); the layout names no group.
fn layout() -> Value {
    json!({
        "schema": 2,
        "default_page": "p",
        "pages": [{"id": "p", "title": "P", "rows": [{"sections": [{"kind": "group", "controls": [
            {"kind": "strip", "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Drums #"}},
             "strip_kind": "standard"}
        ]}]}]}]
    })
}

/// The test site with `Drums #` moved from `Stems grp#` into `Vocals Repro
/// grp#`.
fn moved_site() -> Value {
    let text = std::fs::read(support::repo().join("sim/fixtures/test-site.json")).unwrap();
    let mut site: Value = serde_json::from_slice(&text).unwrap();
    let tracks = site["tracks"].as_array_mut().unwrap();
    let mut drums = None;
    // `get_mut`, not an index: indexing a missing key inserts a null, which
    // the site builder reads as a group with no children list.
    for track in tracks.iter_mut() {
        if let Some(children) = track.get_mut("children").and_then(Value::as_array_mut)
            && let Some(i) = children.iter().position(|c| c["name"] == "Drums #")
        {
            drums = Some(children.remove(i));
        }
    }
    let drums = drums.expect("the fixture has Drums # in a group");
    for track in tracks.iter_mut() {
        if track["name"] == "Vocals Repro grp#" {
            track
                .get_mut("children")
                .and_then(Value::as_array_mut)
                .expect("the group's children")
                .push(drums.clone());
        }
    }
    site
}

async fn fold_until(client: &mut Client, target: &str, want: i64, what: &str) {
    let deadline = Instant::now() + SECS_3;
    loop {
        let fold = client.get("band", target, "fold_state").await;
        if fold == json!(want) {
            return;
        }
        assert!(Instant::now() < deadline, "{what}: fold_state {fold}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[test]
fn a_strip_track_s_group_is_unfolded_whenever_it_is_folded() {
    let _serial = serial();
    runtime().block_on(async {
        let host = Host::start("band");
        let port = host.port;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("layout.json"),
            serde_json::to_vec(&layout()).unwrap(),
        )
        .unwrap();
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut a = hub.client().await;
        a.instance_state("band", true, Some(false), SECS_3).await;
        // The status names the group the hub holds.
        hub.status_until(SECS_3, |s| s.instances[0].unfolded == ["Stems grp#"])
            .await;
        // Someone folds the strip track's group: the hub unfolds it.
        a.set("band", STEMS, "fold_state", json!(true)).await;
        fold_until(&mut a, STEMS, 0, "the strips' group unfolded").await;
        // A group no strip track sits in stays as it is left.
        a.set("band", VOCALS, "fold_state", json!(true)).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(a.get("band", VOCALS, "fold_state").await, json!(1));
        a.set("band", VOCALS, "fold_state", json!(false)).await;
        // Live restarts on a set where the strip's track sits in the other
        // group: the hub reads the groups again and holds the new one.
        host.stop();
        let site = dir.path().join("moved-site.json");
        std::fs::write(&site, serde_json::to_vec(&moved_site()).unwrap()).unwrap();
        let host = Host::start_site("band", &site, port, 0.0);
        a.instance_state("band", true, Some(false), Duration::from_secs(10))
            .await;
        hub.status_until(Duration::from_secs(10), |s| {
            s.instances[0].unfolded == ["Vocals Repro grp#"]
        })
        .await;
        a.set("band", VOCALS, "fold_state", json!(true)).await;
        fold_until(&mut a, VOCALS, 0, "the new group unfolded").await;
        // The old one is let go.
        a.set("band", STEMS, "fold_state", json!(true)).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(a.get("band", STEMS, "fold_state").await, json!(1));
        hub.stop().await;
        host.stop();
    });
}
