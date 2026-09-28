//! The served layout end to end (S3 plan, Task 4): `/api/layout` serves the
//! file, a valid replacement bumps the revision, is pushed to the clients
//! and backed up; an invalid one keeps the last good layout served and
//! `/api/status` says why. No Live host: these run on Windows too.

mod support;

use std::path::Path;
use std::time::Duration;

use fohmixer_hub::config::InstanceCfg;
use fohmixer_proto::client::ServerMsg;
use fohmixer_proto::layout::LayoutResponse;
use support::{TestHub, runtime, serial};

const SECS_3: Duration = Duration::from_secs(3);

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// Two instances nothing listens for (the layout needs their names only).
fn instances() -> Vec<InstanceCfg> {
    vec![
        InstanceCfg {
            name: "band".into(),
            port: 1,
        },
        InstanceCfg {
            name: "master".into(),
            port: 2,
        },
    ]
}

async fn layout(hub: &TestHub) -> LayoutResponse {
    let (code, body) = hub.get("/api/layout").await;
    assert_eq!(code, 200, "{body}");
    serde_json::from_value(body).unwrap()
}

#[test]
fn the_layout_is_served_replaced_and_kept_through_a_bad_edit() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layout.json");
        std::fs::write(&path, fixture("layout-ok.json")).unwrap();
        let hub = TestHub::start(instances(), dir.path()).await;
        hub.status_until(SECS_3, |s| s.layout.rev == 1).await;
        let served = layout(&hub).await;
        assert_eq!(served.rev, 1);
        assert_eq!(served.layout.pages[0].title, "FOH");
        assert!(served.layout.stage_aut_binding().is_some());
        let mut client = hub.client().await;
        client
            .wait(SECS_3, |m| {
                matches!(m, ServerMsg::Layout { rev: 1 }).then_some(())
            })
            .await;
        // A valid edit: served, pushed, backed up.
        let mut edited: serde_json::Value =
            serde_json::from_slice(&fixture("layout-ok.json")).unwrap();
        edited["pages"][0]["title"] = serde_json::json!("FOH 2");
        std::fs::write(&path, serde_json::to_vec(&edited).unwrap()).unwrap();
        client
            .wait(SECS_3, |m| {
                matches!(m, ServerMsg::Layout { rev: 2 }).then_some(())
            })
            .await;
        assert_eq!(layout(&hub).await.layout.pages[0].title, "FOH 2");
        let backups = std::fs::read_dir(dir.path().join("layout-backups"))
            .unwrap()
            .count();
        assert_eq!(backups, 2);
        // An invalid edit: the last good layout stays, the status says why.
        std::fs::write(&path, fixture("layout-bad.json")).unwrap();
        let status = hub.status_until(SECS_3, |s| s.layout.error.is_some()).await;
        assert_eq!(status.layout.rev, 2);
        let error = status.layout.error.unwrap();
        assert!(error.starts_with("layout is invalid: "), "{error}");
        assert!(
            error.contains("pages[0].rows[0].sections[0].color"),
            "{error}"
        );
        let still = layout(&hub).await;
        assert_eq!(still.rev, 2);
        assert_eq!(still.layout.pages[0].title, "FOH 2");
        assert!(
            client
                .gets(Duration::from_millis(400), |m| matches!(
                    m,
                    ServerMsg::Layout { .. }
                )
                .then_some(()))
                .await
                .is_none(),
            "no push for a rejected layout"
        );
        assert_eq!(
            std::fs::read_dir(dir.path().join("layout-backups"))
                .unwrap()
                .count(),
            2
        );
        hub.stop().await;
    });
}

#[test]
fn without_a_layout_file_nothing_is_served_and_the_status_says_why() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start(instances(), dir.path()).await;
        let status = hub.status_until(SECS_3, |s| s.layout.error.is_some()).await;
        assert_eq!(status.layout.rev, 0);
        assert!(status.layout.error.unwrap().contains("layout.json"));
        let (code, body) = hub.get("/api/layout").await;
        assert_eq!(code, 503);
        assert_eq!(body["code"], "NO_LAYOUT");
        assert!(
            body["message"].as_str().unwrap().contains("layout.json"),
            "{body}"
        );
        // An instance nothing listens for is simply offline, and the status
        // says why and how often it was tried.
        assert!(status.instances.iter().all(|i| !i.online));
        assert_eq!(status.instances[0].port, 1);
        let tried = hub
            .status_until(Duration::from_secs(10), |s| {
                s.instances.iter().all(|i| i.connect_failures >= 1)
            })
            .await;
        assert!(
            tried.instances.iter().all(|i| i.last_error.is_some()),
            "{tried:?}"
        );
        assert!(tried.layout.unresolved.is_empty(), "nothing to check");
        hub.stop().await;
    });
}
