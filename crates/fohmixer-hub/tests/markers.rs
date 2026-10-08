//! The hub serves the strips the Tuner markers name (#68, spec D16), end to
//! end on a real FohMixer script: a Tuner added, renamed or removed in Live
//! changes the served layout; two tracks with one name are two strips; an
//! equal label is a conflict; a change that leaves the composition as it is
//! bumps no revision (I10).
#![cfg(unix)]

mod support;

use std::time::{Duration, Instant};

use fohmixer_proto::layout::{Anchor, Control, LayoutResponse, StripMark};
use fohmixer_proto::markers::TagProblem;
use serde_json::{Value, json};
use support::{Host, TestHub, runtime, serial};

const SECS_5: Duration = Duration::from_secs(5);

/// One page with one group that shows the tag group VOCALS.
fn frame() -> Value {
    json!({
        "schema": 2,
        "default_page": "p",
        "pages": [{"id": "p", "title": "P", "rows": [{"sections": [
            {"kind": "group", "id": "vocals", "title": "VOCALS", "tags": "VOCALS"}
        ]}]}]
    })
}

/// The served layout once `check` holds on it (polled).
async fn layout_until(
    hub: &TestHub,
    what: &str,
    check: impl Fn(&LayoutResponse) -> bool,
) -> LayoutResponse {
    let deadline = Instant::now() + SECS_5;
    loop {
        let (code, body) = hub.get("/api/layout").await;
        assert_eq!(code, 200, "{body}");
        let served: LayoutResponse = serde_json::from_value(body).unwrap();
        if check(&served) {
            return served;
        }
        assert!(Instant::now() < deadline, "{what}: {served:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The marker strips of the page `page`: (label, anchor, mark).
fn strips(served: &LayoutResponse, page: &str) -> Vec<(String, Anchor, Option<StripMark>)> {
    served
        .layout
        .pages
        .iter()
        .filter(|p| p.id == page)
        .flat_map(|p| p.controls())
        .filter_map(|c| match c {
            Control::Strip(s) => Some((
                s.label.clone().unwrap_or_default(),
                s.binding.anchor.clone(),
                s.mark,
            )),
            _ => None,
        })
        .collect()
}

#[test]
fn the_served_strips_follow_the_tuner_markers() {
    let _serial = serial();
    runtime().block_on(async {
        let mut host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("layout.json"),
            serde_json::to_vec(&frame()).unwrap(),
        )
        .unwrap();
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut a = hub.client().await;
        a.instance_state("band", true, Some(false), SECS_5).await;
        layout_until(&hub, "the frame served", |s| s.rev > 0).await;

        // A Tuner added on the first track: a strip with its label.
        assert_eq!(
            host.tuner("track", 0, "set", r#""Lead vox" +G:VOCALS +PIN"#),
            1
        );
        let served = layout_until(&hub, "the marker strip", |s| !strips(s, "p").is_empty()).await;
        assert_eq!(
            strips(&served, "p"),
            vec![("Lead vox".to_string(), Anchor::TrackAt { index: 0 }, None)]
        );
        let status = hub
            .status_until(SECS_5, |s| s.layout.markers.found == 1)
            .await;
        assert_eq!(status.layout.markers.problems, vec![]);

        // The two tracks named `Keys 1` (indices 8 and 12 in the test site),
        // each with its own Tuner: two strips, each on its own track.
        assert_eq!(
            host.tuner("track", 8, "set", r#""Keys repro" +G:VOCALS:2"#),
            1
        );
        assert_eq!(
            host.tuner("track", 12, "set", r#""Keys stems" +G:VOCALS:3"#),
            1
        );
        let served = layout_until(&hub, "three strips", |s| strips(s, "p").len() == 3).await;
        let labels: Vec<(String, Anchor)> = strips(&served, "p")
            .into_iter()
            .map(|(label, anchor, _)| (label, anchor))
            .collect();
        assert_eq!(
            labels,
            vec![
                ("Keys repro".to_string(), Anchor::TrackAt { index: 8 }),
                ("Keys stems".to_string(), Anchor::TrackAt { index: 12 }),
                ("Lead vox".to_string(), Anchor::TrackAt { index: 0 }),
            ]
        );
        assert_eq!(
            a.get("band", "live_set tracks 12", "name").await,
            json!("Keys 1")
        );

        // Renamed into a group the frame does not show: a view.
        assert_eq!(host.tuner("track", 0, "set", r#""Lead vox" +G:TALK"#), 1);
        let served = layout_until(&hub, "the view", |s| {
            s.layout.pages.iter().any(|p| p.id == "view-TALK")
        })
        .await;
        assert_eq!(
            strips(&served, "view-TALK"),
            vec![("Lead vox".to_string(), Anchor::TrackAt { index: 0 }, None)]
        );
        assert_eq!(strips(&served, "p").len(), 2);

        // An equal label: a conflict on both strips, listed in the status.
        assert_eq!(
            host.tuner("track", 12, "set", r#""Keys repro" +G:VOCALS:3"#),
            1
        );
        let served = layout_until(&hub, "the conflict", |s| {
            strips(s, "p")
                .iter()
                .all(|(_, _, mark)| *mark == Some(StripMark::Conflict))
        })
        .await;
        assert_eq!(strips(&served, "p").len(), 2);
        let status = hub
            .status_until(SECS_5, |s| s.layout.markers.problems.len() == 2)
            .await;
        assert_eq!(
            status.layout.markers.problems[0].problems,
            vec![TagProblem::Conflict {
                label: "Keys repro".into()
            }]
        );

        // A rename elsewhere leaves the composition as it is: no revision.
        let rev = layout_until(&hub, "settled", |_| true).await.rev;
        assert_eq!(host.rename("Hand2 #", "Hand2 renamed #"), 1);
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(layout_until(&hub, "unchanged", |_| true).await.rev, rev);

        // Removed: the strip goes.
        assert_eq!(host.tuner("track", 12, "remove", ""), 0);
        assert_eq!(host.tuner("track", 8, "remove", ""), 0);
        let served = layout_until(&hub, "the strips gone", |s| strips(s, "p").is_empty()).await;
        assert_eq!(strips(&served, "view-TALK").len(), 1);
        hub.status_until(SECS_5, |s| s.layout.markers.found == 1)
            .await;

        hub.stop().await;
        host.stop();
    });
}
