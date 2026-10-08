use serde_json::{Value, json};

use super::*;
use crate::layout::{Group, Strip, StripKind};
use crate::markers::{Found, compose, frame_problems, parse};

fn strip(instance: &str, kind: &str, name: &str, pinned: bool, mute_guard: bool) -> Value {
    json!({"kind": "strip",
           "binding": {"instance": instance, "anchor": {"kind": kind, "name": name}},
           "strip_kind": if kind == "return" { "return" } else { "standard" },
           "pinned": pinned, "mute_guard": mute_guard})
}

/// A frame of invented names: a pager of two sub-pages, a group ending with
/// a text, two groups titled alike, and a group of the master strip.
fn frame() -> Layout {
    serde_json::from_value(json!({
        "schema": 2,
        "default_page": "foh",
        "pages": [{"id": "foh", "title": "FOH", "rows": [
            {"sections": [
                {"kind": "pager", "id": "sub", "default_page": "stage", "pages": [
                    {"id": "stage", "title": "STAGE", "sections": [
                        {"kind": "group", "id": "stage-1", "title": "Štage", "controls": [
                            strip("band", "track", "Vox 1 #", true, false),
                            strip("band", "track", "Vox 2 #", false, false)]}]},
                    {"id": "others", "title": "OTHERS", "sections": [
                        {"kind": "group", "id": "others-1", "controls": [
                            strip("band", "track", "Keys #", false, false)]}]}]},
                {"kind": "group", "id": "fx", "title": "EFFECTS", "controls": [
                    strip("band", "return", "A-Reverb #", false, false),
                    {"kind": "text", "text": "t"}]}]},
            {"sections": [
                {"kind": "group", "id": "hands", "title": "HANDS", "controls": [
                    strip("master", "track", "Hand1 #", true, true),
                    strip("master", "track", "Hand2 #", false, false)]},
                {"kind": "group", "id": "hands-b", "title": "hands", "controls": [
                    strip("master", "track", "Hand1 #", true, true)]},
                {"kind": "group", "id": "main", "controls": [
                    {"kind": "strip", "binding": {"instance": "master", "anchor": {"kind": "master"}},
                     "strip_kind": "standard"}]}]}]}]
    }))
    .expect("the frame parses")
}

/// Every group of the layout, the pagers' sub-pages' too, by id.
fn groups(layout: &Layout) -> Vec<&Group> {
    layout
        .pages
        .iter()
        .flat_map(|p| &p.rows)
        .flat_map(|r| &r.sections)
        .flat_map(|s| s.groups())
        .collect()
}

fn group<'a>(layout: &'a Layout, id: &str) -> &'a Group {
    groups(layout)
        .into_iter()
        .find(|g| g.id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no group {id}"))
}

fn strips(group: &Group) -> Vec<&Strip> {
    group
        .controls
        .iter()
        .filter_map(|c| match c {
            Control::Strip(s) => Some(s.as_ref()),
            _ => None,
        })
        .collect()
}

#[test]
fn every_group_of_strips_by_name_becomes_a_tags_group() {
    let m = migration(&frame());
    let tags = |id: &str| group(&m.frame, id).tags.clone();
    assert_eq!(tags("stage-1").as_deref(), Some("STAGE"));
    assert_eq!(tags("others-1").as_deref(), Some("OTHERS-1"));
    assert_eq!(tags("fx").as_deref(), Some("EFFECTS"));
    assert_eq!(tags("hands").as_deref(), Some("HANDS"));
    assert_eq!(tags("hands-b").as_deref(), Some("HANDS_2"));
    // The master strip's group stays as it was.
    assert_eq!(group(&m.frame, "main"), group(&frame(), "main"));
    // Its strips go; its other controls and its title stay.
    assert_eq!(
        group(&m.frame, "fx").controls,
        vec![Control::Text { text: "t".into() }]
    );
    assert_eq!(group(&m.frame, "stage-1").controls, vec![]);
    assert_eq!(group(&m.frame, "stage-1").title.as_deref(), Some("Štage"));
    // The converted frame passes the markers' rules and the layout's own.
    assert!(frame_problems(&m.frame).is_empty());
    assert_eq!(m.frame.validate(), vec![]);
    // A text after its strips, a track pinned and guarded in both of its
    // groups: nothing a marker cannot carry.
    assert_eq!(m.problems, Vec::<String>::new());
}

#[test]
fn each_track_gets_the_marker_of_its_groups_places_and_flags() {
    let m = migration(&frame());
    let planned: Vec<(&str, TrackKind, &str, &str)> = m
        .planned
        .iter()
        .map(|p| {
            (
                p.instance.as_str(),
                p.kind,
                p.track.as_str(),
                p.marker.as_str(),
            )
        })
        .collect();
    assert_eq!(
        planned,
        vec![
            // Both would show "Vox": the name without its marks.
            (
                "band",
                TrackKind::Track,
                "Vox 1 #",
                r#""Vox 1" +G:STAGE:1 +PIN"#
            ),
            ("band", TrackKind::Track, "Vox 2 #", r#""Vox 2" +G:STAGE:2"#),
            (
                "band",
                TrackKind::Track,
                "Keys #",
                r#""Keys" +G:OTHERS-1:1"#
            ),
            (
                "band",
                TrackKind::Return,
                "A-Reverb #",
                r#""Reverb" +G:EFFECTS:1"#
            ),
            (
                "master",
                TrackKind::Track,
                "Hand1 #",
                r#""Hand1" +G:HANDS:1 +G:HANDS_2:1 +PIN +MG"#
            ),
            (
                "master",
                TrackKind::Track,
                "Hand2 #",
                r#""Hand2" +G:HANDS:2"#
            ),
        ]
    );
    // Each marker parses back without a problem.
    for p in &m.planned {
        let marker = parse(&p.marker);
        assert_eq!(marker.problems, vec![], "{}", p.marker);
    }
}

#[test]
fn the_migrated_frame_with_its_markers_shows_the_same_strips() {
    let original = frame();
    let m = migration(&original);
    // Live's tracks: each planned track at an index of its own kind.
    let index_of = |p: &Planned| -> u32 {
        let same: Vec<&Planned> = m
            .planned
            .iter()
            .filter(|q| q.instance == p.instance && q.kind == p.kind)
            .collect();
        u32::try_from(same.iter().position(|q| q.track == p.track).unwrap()).unwrap() + 2
    };
    let found: Vec<Found> = m
        .planned
        .iter()
        .map(|p| Found {
            instance: p.instance.clone(),
            kind: p.kind,
            index: index_of(p),
            name: p.marker.clone(),
            tuners: 1,
        })
        .collect();
    let composed = compose(&m.frame, &found);
    assert_eq!(composed.problems, vec![]);
    assert_eq!(composed.layout.validate(), vec![]);
    // No view: every tag group has its frame group.
    assert!(composed.layout.pages.iter().all(|p| !p.view));
    // Each group: the same tracks in the same order, kind, pin and guard.
    let track_of = |s: &Strip| -> (String, String, StripKind, bool, bool) {
        let name = match s.binding.anchor {
            Anchor::Track { ref name } | Anchor::Return { ref name } => name.clone(),
            Anchor::TrackAt { index } | Anchor::ReturnAt { index } => {
                let kind = if matches!(s.binding.anchor, Anchor::TrackAt { .. }) {
                    TrackKind::Track
                } else {
                    TrackKind::Return
                };
                m.planned
                    .iter()
                    .find(|p| {
                        p.instance == s.binding.instance && p.kind == kind && index_of(p) == index
                    })
                    .map(|p| p.track.clone())
                    .unwrap()
            }
            _ => "master".to_string(),
        };
        (
            s.binding.instance.clone(),
            name,
            s.strip_kind,
            s.pinned,
            s.mute_guard,
        )
    };
    for before in groups(&original) {
        let id = before.id.as_deref().unwrap();
        let after = group(&composed.layout, id);
        let a: Vec<_> = strips(before).into_iter().map(&track_of).collect();
        let b: Vec<_> = strips(after).into_iter().map(&track_of).collect();
        assert_eq!(a, b, "group {id}");
        assert_eq!(after.controls.len(), before.controls.len(), "group {id}");
    }
}

#[test]
fn a_group_name_is_its_title_or_id_in_capitals_without_accents() {
    assert_eq!(tag_name("Master A"), "MASTER_A");
    assert_eq!(tag_name("  Štage   left! "), "STAGE_LEFT");
    assert_eq!(tag_name("Ďaľšia úroveň Č"), "DALSIA_UROVEN_C");
    assert_eq!(tag_name("foh-3"), "FOH-3");
    // Every accented capital of the table, once.
    assert_eq!(tag_name("ÁČĎÉÍĽŇÓŔŠŤÚÝŽ"), "ACDEILNORSTUYZ");
    assert_eq!(tag_name("äěĺôöřůü"), "AELOORUU");
    assert_eq!(tag_name("a_ b"), "A_B");
    assert_eq!(tag_name("  "), "");
    assert_eq!(tag_name("!!"), "");
    let long = "a".repeat(40);
    assert_eq!(tag_name(&long), "A".repeat(32));
    // Cut at 32, no `_` left at the end.
    let cut = format!("{} b", "a".repeat(31));
    assert_eq!(tag_name(&cut), "A".repeat(31));
    for name in ["MASTER_A", "STAGE_LEFT", "DALSIA_UROVEN_C", "FOH-3"] {
        assert!(crate::markers::valid_group_name(name), "{name}");
    }
}

#[test]
fn a_taken_group_name_gets_a_number_within_the_length() {
    let mut used = BTreeSet::new();
    assert_eq!(unique("X", &mut used), "X");
    assert_eq!(unique("X", &mut used), "X_2");
    assert_eq!(unique("X", &mut used), "X_3");
    let long = "A".repeat(32);
    assert_eq!(unique(&long, &mut used), long);
    assert_eq!(unique(&long, &mut used), format!("{}_2", "A".repeat(30)));
}

#[test]
fn a_group_without_a_usable_title_or_id_is_named_group() {
    let frame: Layout = serde_json::from_value(json!({
        "schema": 2,
        "default_page": "p",
        "pages": [{"id": "p", "title": "P", "rows": [{"sections": [
            {"kind": "group", "title": "!!", "controls": [strip("band", "track", "Bass #", false, false)]},
            {"kind": "group", "id": "g", "title": "", "controls": [strip("band", "track", "Kick #", false, false)]}]}]}]
    }))
    .unwrap();
    let m = migration(&frame);
    let tags: Vec<Option<String>> = groups(&m.frame).iter().map(|g| g.tags.clone()).collect();
    assert_eq!(tags, vec![Some("GROUP".into()), Some("G".into())]);
}

#[test]
fn a_group_with_a_strip_not_bound_by_name_or_with_tags_or_no_strip_stays() {
    let frame: Layout = serde_json::from_value(json!({
        "schema": 2,
        "default_page": "p",
        "pages": [{"id": "p", "title": "P", "rows": [{"sections": [
            {"kind": "group", "id": "mixed", "controls": [
                strip("band", "track", "Bass #", false, false),
                {"kind": "strip", "binding": {"instance": "band", "anchor": {"kind": "track_at", "index": 3}},
                 "strip_kind": "standard"}]},
            {"kind": "group", "id": "tagged", "tags": "X"},
            {"kind": "group", "id": "texts", "controls": [{"kind": "text", "text": "t"}]}]}]}]
    }))
    .unwrap();
    let m = migration(&frame);
    assert_eq!(m.frame, frame);
    assert_eq!(m.planned, vec![]);
}

#[test]
fn equal_labels_take_the_names_then_a_number() {
    let frame: Layout = serde_json::from_value(json!({
        "schema": 2,
        "default_page": "p",
        "pages": [{"id": "p", "title": "P", "rows": [{"sections": [
            {"kind": "group", "id": "g", "title": "G", "controls": [
                strip("band", "track", "Vox 1 #", false, false),
                strip("master", "track", "Vox 1 #", false, false),
                strip("band", "track", "B-", false, false),
                strip("band", "track", "Vox 1 2", false, false),
                strip("band", "track", "Say \"hi\" #", false, false)]}]}]}]
    }))
    .unwrap();
    let labels: Vec<String> = migration(&frame)
        .planned
        .iter()
        .map(|p| parse(&p.marker).label.unwrap())
        .collect();
    // "Vox" thrice: the names; "Vox 1" twice and a track named "Vox 1 2":
    // the second "Vox 1" skips "Vox 1 2"; "B-" shows nothing: "?".
    assert_eq!(labels, vec!["Vox 1", "Vox 1 3", "?", "Vox 1 2", "Say"]);
}

#[test]
fn a_name_without_its_marks() {
    assert_eq!(full_label("Vocal 1 repro#"), "Vocal 1 repro");
    assert_eq!(full_label("Hand1 #"), "Hand1");
    assert_eq!(full_label("A-Reverb #"), "Reverb");
    assert_eq!(full_label("  B-Main repro # "), "Main repro");
    assert_eq!(full_label("A-\"Q\" voice #"), "Q voice");
    assert_eq!(full_label("1-Mic"), "1-Mic");
    assert_eq!(full_label("#"), "");
    // A quote alone is no word: no space left where it stood.
    assert_eq!(full_label("A \" B #"), "A B");
}

#[test]
fn a_marker_name_lists_its_groups_then_its_flags() {
    let groups = vec![("A".to_string(), 1), ("B".to_string(), 999)];
    assert_eq!(
        marker_name("V", &groups, true, true),
        r#""V" +G:A:1 +G:B:999 +PIN +MG"#
    );
    assert_eq!(
        marker_name("V", &[("A".to_string(), 1000)], false, false),
        r#""V" +G:A"#
    );
    assert_eq!(marker_name("V", &[], false, true), r#""V" +MG"#);
}

#[test]
fn a_row_is_ready_with_one_track_one_plain_tuner_and_no_marker() {
    let row = |matches, plain, markers| MigrationRow {
        instance: "band".into(),
        kind: TrackKind::Track,
        track: "Vox 1 #".into(),
        marker: r#""Vox" +G:A:1"#.into(),
        matches,
        index: Some(4),
        plain,
        markers,
        done: false,
    };
    assert!(row(1, 1, 0).ready());
    assert!(!row(0, 1, 0).ready());
    assert!(!row(2, 1, 0).ready());
    assert!(!row(1, 0, 0).ready());
    assert!(!row(1, 2, 0).ready());
    assert!(!row(1, 1, 1).ready());
    // On the wire: no index when none.
    let mut none = row(0, 0, 0);
    none.index = None;
    let text = serde_json::to_value(&none).unwrap();
    assert!(text.get("index").is_none(), "{text}");
    let status: MigrationStatus = serde_json::from_value(json!({"rows": [text]})).unwrap();
    assert_eq!(status.renamed, 0);
    assert!(!status.reading);
    assert_eq!(status.problems, Vec::<String>::new());
    assert_eq!(status.rows, vec![none]);
}

/// One group of `controls`, titled G, migrated: its problems.
fn problems_of(controls: Value) -> Vec<String> {
    let frame: Layout = serde_json::from_value(json!({
        "schema": 2,
        "default_page": "p",
        "pages": [{"id": "p", "title": "P", "rows": [{"sections": [
            {"kind": "group", "id": "g", "title": "G", "controls": controls}]}]}]
    }))
    .unwrap();
    migration(&frame).problems
}

#[test]
fn what_a_marker_cannot_carry_is_a_problem() {
    let text = json!({"kind": "text", "text": "t"});
    let plain = strip("band", "track", "Bass #", false, false);
    // A control before or between the strips would move after them.
    assert_eq!(
        problems_of(json!([text, plain])),
        vec!["group G: a control before or between its strips would move after them"]
    );
    assert_eq!(
        problems_of(json!([
            plain,
            text,
            strip("band", "track", "Kick #", false, false)
        ])),
        vec!["group G: a control before or between its strips would move after them"]
    );
    assert_eq!(problems_of(json!([plain, text])), Vec::<String>::new());
    // A track twice in one group shows once.
    assert_eq!(
        problems_of(json!([plain, plain])),
        vec![r#""Bass #" (band): twice in group G (a marker shows it once)"#]
    );
    // The same name on another instance or as a return is another track.
    assert_eq!(
        problems_of(json!([
            plain,
            strip("master", "track", "Bass #", false, false),
            strip("band", "return", "Bass #", false, false)
        ])),
        Vec::<String>::new()
    );
    // Its width, label or path would be lost.
    let lost = vec![r#""Bass #" (band): its width, label, path or kind would be lost"#];
    let with = |field: &str, value: Value| {
        let mut s = plain.clone();
        s[field] = value;
        s
    };
    assert_eq!(problems_of(json!([with("wide", json!(true))])), lost);
    assert_eq!(problems_of(json!([with("label", json!("B"))])), lost);
    let mut path = plain.clone();
    path["binding"]["path"] = json!("mixer_device volume");
    assert_eq!(problems_of(json!([path])), lost);
    // A strip of another kind than its track's.
    assert_eq!(
        problems_of(json!([with("strip_kind", json!("return"))])),
        lost
    );
    let mut ret = strip("band", "return", "Bass #", false, false);
    assert_eq!(problems_of(json!([ret.clone()])), Vec::<String>::new());
    ret["strip_kind"] = json!("standard");
    assert_eq!(
        problems_of(json!([ret])),
        vec![r#""Bass #" (band): its width, label, path or kind would be lost"#]
    );
}

#[test]
fn a_track_pinned_or_guarded_in_only_some_groups_is_a_problem() {
    let frame: Layout = serde_json::from_value(json!({
        "schema": 2,
        "default_page": "p",
        "pages": [{"id": "p", "title": "P", "rows": [{"sections": [
            {"kind": "group", "id": "a", "title": "A", "controls": [
                strip("band", "track", "Bass #", true, false),
                strip("band", "track", "Kick #", false, true)]},
            {"kind": "group", "id": "b", "title": "B", "controls": [
                strip("band", "track", "Bass #", false, false),
                strip("band", "track", "Kick #", false, false)]}]}]}]
    }))
    .unwrap();
    let m = migration(&frame);
    assert_eq!(
        m.problems,
        vec![
            r#""Bass #" (band): pinned in one group and not in another (a marker pins it in all)"#,
            r#""Kick #" (band): mute-guarded in one group and not in another (a marker guards it in all)"#,
        ]
    );
    // The plan still says what a marker would do: pinned, guarded.
    assert_eq!(m.planned[0].marker, r#""Bass" +G:A:1 +G:B:1 +PIN"#);
    assert_eq!(m.planned[1].marker, r#""Kick" +G:A:2 +G:B:2 +MG"#);
}

#[test]
fn a_frames_own_tag_group_keeps_its_name() {
    let frame: Layout = serde_json::from_value(json!({
        "schema": 2,
        "default_page": "p",
        "pages": [{"id": "p", "title": "P", "rows": [{"sections": [
            {"kind": "pager", "id": "sub", "default_page": "s", "pages": [
                {"id": "s", "title": "S", "sections": [
                    {"kind": "group", "id": "own", "tags": "HANDS"}]}]},
            {"kind": "group", "id": "g", "title": "Hands", "controls": [
                strip("band", "track", "Bass #", false, false)]}]}]}]
    }))
    .unwrap();
    let m = migration(&frame);
    assert_eq!(group(&m.frame, "g").tags.as_deref(), Some("HANDS_2"));
    assert_eq!(m.planned[0].marker, r#""Bass" +G:HANDS_2:1"#);
}

#[test]
fn the_e2e_fixture_migrates_and_composes_the_same_strips_but_their_width() {
    // The suite's frame (tools/import-tosc's expected output): three of its
    // strips are wide (the PC's frame has none), two labels are equal
    // across the instances.
    let fixture: Layout = serde_json::from_str(include_str!(
        "../../../../../tools/import-tosc/fixtures/expected-layout.json"
    ))
    .unwrap();
    let m = migration(&fixture);
    assert_eq!(
        m.problems,
        vec![
            r#""B-Main repro #" (band): its width, label, path or kind would be lost"#,
            r#""Hand2 #" (band): its width, label, path or kind would be lost"#,
            r#""A-Echo" (master): its width, label, path or kind would be lost"#,
        ]
    );
    assert!(frame_problems(&m.frame).is_empty());
    assert_eq!(m.frame.validate(), vec![]);
    let labels: Vec<String> = m
        .planned
        .iter()
        .map(|p| parse(&p.marker).label.unwrap())
        .collect();
    assert_eq!(
        labels,
        [
            "Vocal", "Keys", "Hand1", "Main", "Hand2", "Reverb", "Reverb 2", "Hand4", "Hand2 2",
            "Echo"
        ]
    );
    // Composed with markers named as planned, every group shows the same
    // tracks in the same order, without a problem.
    let found: Vec<Found> = m
        .planned
        .iter()
        .enumerate()
        .map(|(i, p)| Found {
            instance: p.instance.clone(),
            kind: p.kind,
            index: u32::try_from(i).unwrap(),
            name: p.marker.clone(),
            tuners: 1,
        })
        .collect();
    let composed = compose(&m.frame, &found);
    assert_eq!(composed.problems, vec![]);
    let track = |s: &Strip| -> String {
        match &s.binding.anchor {
            Anchor::Track { name } | Anchor::Return { name } => name.clone(),
            Anchor::TrackAt { index } | Anchor::ReturnAt { index } => {
                m.planned[usize::try_from(*index).unwrap()].track.clone()
            }
            other => format!("{other:?}"),
        }
    };
    for before in groups(&fixture) {
        let id = before.id.as_deref().unwrap();
        let after = group(&composed.layout, id);
        let a: Vec<String> = strips(before).into_iter().map(&track).collect();
        let b: Vec<String> = strips(after).into_iter().map(&track).collect();
        assert_eq!(a, b, "group {id}");
    }
}
