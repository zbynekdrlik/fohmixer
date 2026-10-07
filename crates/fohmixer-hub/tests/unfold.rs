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
        let mut host = Host::start("band");
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
        // Live restarts: the hub follows the new session's groups too.
        host.stop();
        let mut host = Host::start_with("band", port, 0.0);
        a.instance_state("band", true, Some(false), Duration::from_secs(10))
            .await;
        a.set("band", STEMS, "fold_state", json!(true)).await;
        fold_until(&mut a, STEMS, 0, "unfolded again after Live came back").await;
        hub.stop().await;
        host.stop();
    });
}
