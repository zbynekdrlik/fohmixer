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
use fohmixer_proto::markers::migrate::MigrationStatus;
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

        // A plain Tuner added (a read of the devices, the same markers) and
        // a rename elsewhere leave the composition as it is: no revision.
        let rev = layout_until(&hub, "settled", |_| true).await.rev;
        assert_eq!(host.tuner("track", 3, "add", "Tuner"), 1);
        assert_eq!(host.rename("Hand2 #", "Hand2 renamed #"), 1);
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(layout_until(&hub, "unchanged", |_| true).await.rev, rev);
        hub.status_until(SECS_5, |s| s.layout.markers.found == 3)
            .await;

        // A track deleted above the markers: a key bound by index follows the
        // object now at that index (a list change resolves it again), and
        // the markers follow their tracks.
        let volume8 = "live_set tracks 8 mixer_device volume";
        let key = a.sub_key("band", volume8, "value", false).await;
        a.set("band", volume8, "value", json!(0.7)).await;
        a.value_until(&key, SECS_5, |i| i.value == Some(json!(0.7)))
            .await;
        assert_eq!(host.delete_track(1), 14);
        // Index 8 holds what was 9 now (Stems grp#, at 0.85).
        a.value_until(&key, SECS_5, |i| i.value == Some(json!(0.85)))
            .await;
        let served = layout_until(&hub, "the markers moved", |s| {
            strips(s, "p")
                .iter()
                .map(|(_, anchor, _)| anchor.clone())
                .collect::<Vec<_>>()
                == vec![Anchor::TrackAt { index: 7 }, Anchor::TrackAt { index: 11 }]
        })
        .await;
        assert_eq!(strips(&served, "view-TALK").len(), 1);

        // Removed: the strip goes.
        assert_eq!(host.tuner("track", 11, "remove", ""), 0);
        assert_eq!(host.tuner("track", 7, "remove", ""), 0);
        let served = layout_until(&hub, "the strips gone", |s| strips(s, "p").is_empty()).await;
        assert_eq!(strips(&served, "view-TALK").len(), 1);
        hub.status_until(SECS_5, |s| s.layout.markers.found == 1)
            .await;

        hub.stop().await;
        host.stop();
    });
}

/// The migration's rows once `check` holds on them (polled).
async fn migration_until(
    hub: &TestHub,
    what: &str,
    check: impl Fn(&MigrationStatus) -> bool,
) -> MigrationStatus {
    let deadline = Instant::now() + SECS_5;
    loop {
        let (code, body) = hub.get("/api/markers/migration").await;
        assert_eq!(code, 200, "{body}");
        let status: MigrationStatus = serde_json::from_value(body).unwrap();
        if check(&status) {
            return status;
        }
        assert!(Instant::now() < deadline, "{what}: {status:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A row's (matches, index, plain, markers, done), by track.
fn row(status: &MigrationStatus, track: &str) -> (u32, Option<u32>, u32, u32, bool) {
    let r = status
        .rows
        .iter()
        .find(|r| r.track == track)
        .unwrap_or_else(|| panic!("no row {track}"));
    (r.matches, r.index, r.plain, r.markers, r.done)
}

#[test]
fn the_migration_names_the_owners_plain_tuners_with_their_planned_markers() {
    // #68 PR C on the test site: Hand1 # is track 0, Hand2 # track 1, Keys 1
    // is two tracks, A-Reverb # is return 0.
    let _serial = serial();
    runtime().block_on(async {
        let mut host = Host::start("band");
        let dir = tempfile::tempdir().unwrap();
        let strip = |kind: &str, name: &str, pinned: bool| {
            json!({"kind": "strip", "strip_kind": if kind == "return" { "return" } else { "standard" },
                   "pinned": pinned,
                   "binding": {"instance": "band", "anchor": {"kind": kind, "name": name}}})
        };
        let frame = json!({
            "schema": 2,
            "default_page": "p",
            "pages": [{"id": "p", "title": "P", "rows": [{"sections": [
                {"kind": "group", "id": "hands", "title": "Hands", "controls": [
                    strip("track", "Hand1 #", true), strip("track", "Hand2 #", false)]},
                {"kind": "group", "id": "fx", "title": "FX", "controls": [
                    strip("return", "A-Reverb #", false), {"kind": "text", "text": "t"}]},
                {"kind": "group", "id": "keys", "title": "Keys", "controls": [
                    strip("track", "Keys 1", false)]}]}]}]
        });
        std::fs::write(
            dir.path().join("layout.json"),
            serde_json::to_vec(&frame).unwrap(),
        )
        .unwrap();
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        // The keeper's lists are read: every track found by name, none with
        // a Tuner yet.
        let status = migration_until(&hub, "the lists read", |s| {
            s.rows.len() == 4 && s.rows.iter().all(|r| r.matches > 0)
        })
        .await;
        assert_eq!(row(&status, "Hand1 #"), (1, Some(0), 0, 0, false));
        assert_eq!(row(&status, "Keys 1"), (2, None, 0, 0, false));
        let markers: Vec<&str> = status.rows.iter().map(|r| r.marker.as_str()).collect();
        assert_eq!(
            markers,
            [
                r#""Hand1" +G:HANDS:1 +PIN"#,
                r#""Hand2" +G:HANDS:2"#,
                r#""Reverb" +G:FX:1"#,
                r#""Keys" +G:KEYS:1"#,
            ]
        );
        // The owner adds plain Tuners: one on Hand1 # and the reverb, two on
        // Hand2 #.
        assert_eq!(host.tuner("track", 0, "add", "Tuner"), 1);
        assert_eq!(host.tuner("track", 1, "add", "Tuner"), 1);
        assert_eq!(host.tuner("track", 1, "add", "Tuner"), 2);
        assert_eq!(host.tuner("return", 0, "add", "Tuner"), 1);
        let status = migration_until(&hub, "the Tuners found", |s| {
            row(s, "Hand1 #").2 == 1 && row(s, "Hand2 #").2 == 2 && row(s, "A-Reverb #").2 == 1
        })
        .await;
        assert!(status.rows.iter().all(|r| !r.done));
        // The renames: only the ready tracks.
        let (code, body) = hub.post("/api/markers/migration").await;
        assert_eq!(code, 200, "{body}");
        let posted: MigrationStatus = serde_json::from_value(body).unwrap();
        assert_eq!(posted.renamed, 2);
        let status = migration_until(&hub, "the renames done", |s| {
            row(s, "Hand1 #").4 && row(s, "A-Reverb #").4
        })
        .await;
        assert_eq!(row(&status, "Hand1 #"), (1, Some(0), 0, 1, true));
        assert_eq!(row(&status, "Hand2 #"), (1, Some(1), 2, 0, false));
        // The frame still names its strips: the tag groups are views.
        let served = layout_until(&hub, "the views", |l| {
            !strips(l, "view-HANDS").is_empty() && !strips(l, "view-FX").is_empty()
        })
        .await;
        assert_eq!(
            strips(&served, "view-FX")[0],
            ("Reverb".to_string(), Anchor::ReturnAt { index: 0 }, None)
        );
        assert_eq!(
            strips(&served, "view-HANDS")[0],
            ("Hand1".to_string(), Anchor::TrackAt { index: 0 }, None)
        );
        // Nothing is ready any more.
        let (code, body) = hub.post("/api/markers/migration").await;
        assert_eq!(code, 200, "{body}");
        let posted: MigrationStatus = serde_json::from_value(body).unwrap();
        assert_eq!(posted.renamed, 0);
        assert!(posted.problems.is_empty());
        // Live fires no list listener on a rename: the names are read
        // afresh, so a renamed track is no longer found under its old name.
        assert_eq!(host.rename("Hand2 #", "Hand2b #"), 1);
        let status = migration_until(&hub, "the rename seen", |s| row(s, "Hand2 #").0 == 0).await;
        assert_eq!(row(&status, "Hand2 #"), (0, None, 0, 0, false));
        assert_eq!(host.rename("Hand2b #", "Hand2 #"), 1);
        let status = migration_until(&hub, "the name back", |s| {
            row(s, "Hand2 #") == (1, Some(1), 2, 0, false) && !s.reading
        })
        .await;
        assert_eq!(row(&status, "Hand1 #"), (1, Some(0), 0, 1, true));
        hub.stop().await;
        host.stop();
    });
}
