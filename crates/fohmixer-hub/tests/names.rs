//! The layout's unresolved names end to end (spec §2.5 D4, I5): after a
//! layout is accepted and whenever an instance connects, `/api/status` lists
//! the bindings that do not resolve on a real FohMixer script — a missing
//! name, and two tracks named alike (ambiguous).
#![cfg(unix)]

mod support;

use std::path::Path;
use std::time::Duration;

use fohmixer_hub::config::InstanceCfg;
use fohmixer_proto::client::Unresolved;
use serde_json::{Value, json};
use support::{Host, TestHub, runtime, serial};

const SECS_3: Duration = Duration::from_secs(3);

/// The test layout plus a solo bound to a track the set does not have.
fn layout_with_nobody() -> Value {
    let text =
        std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/layout-ok.json"))
            .unwrap();
    let mut layout: Value = serde_json::from_slice(&text).unwrap();
    layout["pages"][0]["items"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "kind": "solo",
            "frame": {"x": 1000, "y": 900, "w": 100, "h": 60},
            "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Nobody #"}}
        }));
    layout
}

fn band(target: &str, error: &str) -> Unresolved {
    Unresolved {
        instance: "band".into(),
        target: target.into(),
        error: error.into(),
    }
}

#[test]
fn the_status_lists_the_bindings_that_do_not_resolve() {
    let _serial = serial();
    runtime().block_on(async {
        let mut host = Host::start("band");
        let port = host.port;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layout.json");
        let mut layout = layout_with_nobody();
        std::fs::write(&path, serde_json::to_vec(&layout).unwrap()).unwrap();
        // Nothing listens for master: its bindings are not checked.
        let instances = vec![
            host.cfg(),
            InstanceCfg {
                name: "master".into(),
                port: 1,
            },
        ];
        let hub = TestHub::start(instances, dir.path()).await;
        let nobody = band(
            "live_set tracks[name=Nobody #]",
            "not found: tracks[name=Nobody #]",
        );
        hub.status_until(SECS_3, |s| s.layout.unresolved == [nobody.clone()])
            .await;
        // Two tracks named alike: ambiguous, found by the check of the next
        // accepted layout.
        assert_eq!(host.rename("Hand2 #", "Mics Stage #"), 1);
        layout["pages"][0]["title"] = json!("FOH 2");
        std::fs::write(&path, serde_json::to_vec(&layout).unwrap()).unwrap();
        let ambiguous = band(
            "live_set tracks[name=Mics Stage #]",
            "ambiguous: tracks[name=Mics Stage #]",
        );
        hub.status_until(SECS_3, |s| {
            s.layout.rev == 2 && s.layout.unresolved == [ambiguous.clone(), nobody.clone()]
        })
        .await;
        // Offline: no report for the band. A fresh set (the names as they
        // were) is checked again when it connects.
        host.stop();
        hub.status_until(SECS_3, |s| {
            !s.instances[0].online && s.layout.unresolved.is_empty()
        })
        .await;
        let host = Host::start_with("band", port, 0.0);
        hub.status_until(SECS_3, |s| {
            s.instances[0].online && s.layout.unresolved == [nobody.clone()]
        })
        .await;
        hub.stop().await;
        host.stop();
    });
}
