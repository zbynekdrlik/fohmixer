//! The router's part of the remembered markers (#178).

use std::sync::Arc;

use serde_json::json;

use super::super::RouterMsg;
use super::super::markers;
use super::super::tests::{next_of, offline_router};

#[tokio::test]
async fn a_live_not_read_since_the_start_keeps_the_markers_the_store_remembered() {
    // #178: the hub started while the Live was down. The keeper's set
    // (nothing of the band, never read) leaves the band's remembered
    // strip served.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("layout.json"),
        serde_json::to_vec(&json!({
            "schema": 2,
            "default_page": "p",
            "pages": [{"id": "p", "title": "P", "rows": [{"sections": [
                {"kind": "group", "id": "g", "tags": "A"}]}]}]
        }))
        .unwrap(),
    )
    .unwrap();
    let vox = fohmixer_proto::markers::Found {
        instance: "band".into(),
        kind: fohmixer_proto::markers::TrackKind::Track,
        index: 3,
        name: r#""Vox" +G:A"#.into(),
        tuners: 1,
    };
    std::fs::write(
        dir.path().join(crate::layout::MARKERS_FILE),
        serde_json::to_vec(&vec![vox.clone()]).unwrap(),
    )
    .unwrap();
    let store = Arc::new(crate::layout::store_in(
        dir.path(),
        std::path::Path::new("layout.json"),
        vec!["band".into()],
    ));
    assert_eq!(store.poll(), Some(1));
    let (router, mut rx, _records) = offline_router(dir.path());
    let mut router = router.with_layout(Arc::clone(&store));
    assert!(!router.markers.has_read("band"));
    router.markers_found(Vec::new());
    let settle = next_of(&mut rx, "the quiet time", |m| {
        matches!(m, RouterMsg::MarkersSettle { .. })
    })
    .await;
    router.handle(settle);
    assert_eq!(store.markers(), vec![vox]);
    assert_eq!(store.markers_status().found, 1);
    // The band Live answers its first whole read with no Tuner (its set
    // changed while the hub was down): the keeper's markers stay as
    // they were (none), yet the remembered strip goes.
    router.handle(RouterMsg::MarkersRetry {
        instance: "band".into(),
    });
    let read = next_of(&mut rx, "the read", |m| {
        matches!(m, RouterMsg::MarkersRead { .. })
    })
    .await;
    let RouterMsg::MarkersRead { seq, step, .. } = read else {
        unreachable!()
    };
    router.handle(RouterMsg::MarkersRead {
        instance: "band".into(),
        seq,
        step,
        outcome: Ok(Vec::new()),
    });
    assert!(router.markers.has_read("band"));
    let settle = next_of(&mut rx, "the quiet time", |m| {
        matches!(m, RouterMsg::MarkersSettle { .. })
    })
    .await;
    router.handle(settle);
    assert_eq!(store.markers(), Vec::new());
    assert_eq!(store.markers_status().found, 0);
    // Later reads that change nothing hand the store nothing.
    router.handle(RouterMsg::MarkersRetry {
        instance: "band".into(),
    });
    let read = next_of(&mut rx, "the read", |m| {
        matches!(m, RouterMsg::MarkersRead { .. })
    })
    .await;
    let RouterMsg::MarkersRead { seq, step, .. } = read else {
        unreachable!()
    };
    router.handle(RouterMsg::MarkersRead {
        instance: "band".into(),
        seq,
        step,
        outcome: Ok(Vec::new()),
    });
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(markers::SETTLE_MS + 200),
            async {
                loop {
                    if let Some(RouterMsg::MarkersSettle { .. }) = rx.recv().await {
                        return;
                    }
                }
            }
        )
        .await
        .is_err(),
        "no quiet time for a read that changed nothing"
    );
}
