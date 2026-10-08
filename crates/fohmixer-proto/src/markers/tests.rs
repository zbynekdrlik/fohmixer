use super::*;
use crate::layout::{Control, Group, Layout};
use serde_json::json;

fn group_tag(name: &str, place: Option<u32>) -> GroupTag {
    GroupTag {
        name: name.to_string(),
        place,
    }
}

#[test]
fn a_full_marker_names_its_label_groups_pin_and_mute_guard() {
    let m = parse(r#""Vox 1" +G:VOCALS:2 +G:TALKSHOW:1 +PIN +MG"#);
    assert_eq!(m.label.as_deref(), Some("Vox 1"));
    assert_eq!(
        m.groups,
        vec![group_tag("VOCALS", Some(2)), group_tag("TALKSHOW", Some(1))]
    );
    assert!(m.pin);
    assert!(m.mute_guard);
    assert_eq!(m.problems, vec![]);
}

#[test]
fn the_label_takes_any_words_and_is_trimmed() {
    let m = parse(r#"  " Vox 1 mastered "   +G:A  "#);
    assert_eq!(m.label.as_deref(), Some("Vox 1 mastered"));
    assert_eq!(m.groups, vec![group_tag("A", None)]);
    assert!(!m.pin);
    assert!(!m.mute_guard);
    assert_eq!(m.problems, vec![]);
}

#[test]
fn a_missing_empty_or_unclosed_label_is_a_problem() {
    let m = parse("Vox +G:A");
    assert_eq!(m.label, None);
    assert_eq!(
        m.problems,
        vec![
            TagProblem::NoLabel,
            TagProblem::StrayText { text: "Vox".into() }
        ]
    );
    assert_eq!(m.groups, vec![group_tag("A", None)]);

    let m = parse(r#""  " +G:A"#);
    assert_eq!(m.label, None);
    assert_eq!(m.problems, vec![TagProblem::EmptyLabel]);

    let m = parse(r#""Vox +G:A"#);
    assert_eq!(m.label, None);
    assert_eq!(
        m.problems,
        vec![
            TagProblem::UnclosedLabel,
            TagProblem::StrayText { text: "Vox".into() }
        ]
    );
    assert_eq!(m.groups, vec![group_tag("A", None)]);
}

#[test]
fn an_unknown_or_lowercase_tag_is_a_problem() {
    let m = parse(r#""Vox" +G:A +g:b +PINNED +GX +mg"#);
    assert_eq!(m.groups, vec![group_tag("A", None)]);
    assert!(!m.pin);
    assert!(!m.mute_guard);
    assert_eq!(
        m.problems,
        vec![
            TagProblem::UnknownTag { tag: "+g:b".into() },
            TagProblem::UnknownTag {
                tag: "+PINNED".into()
            },
            TagProblem::UnknownTag { tag: "+GX".into() },
            TagProblem::UnknownTag { tag: "+mg".into() },
        ]
    );
}

#[test]
fn a_group_name_takes_up_to_32_of_a_to_z_digits_dash_and_underscore() {
    let ok = "A".repeat(MAX_GROUP_NAME);
    let long = "A".repeat(MAX_GROUP_NAME + 1);
    assert!(valid_group_name(&ok));
    assert!(!valid_group_name(&long));
    assert!(valid_group_name("MASTER_A-2"));
    assert!(valid_group_name("A"));
    assert!(!valid_group_name(""));
    assert!(!valid_group_name("Vocals"));
    assert!(!valid_group_name("VOX 1"));
    assert!(!valid_group_name("HLASOVÉ"));
    let m = parse(&format!(r#""Vox" +G:{ok} +G:{long}"#));
    assert_eq!(m.groups, vec![group_tag(&ok, None)]);
    assert_eq!(
        m.problems,
        vec![TagProblem::BadGroup {
            tag: format!("+G:{long}")
        }]
    );
}

#[test]
fn a_place_is_1_to_999() {
    let m = parse(r#""Vox" +G:A:1 +G:B:999 +G:C:0 +G:D:1000 +G:E:x +G:F: +G:G:1:2 +G:"#);
    assert_eq!(
        m.groups,
        vec![group_tag("A", Some(1)), group_tag("B", Some(999))]
    );
    let bad: Vec<TagProblem> = ["+G:C:0", "+G:D:1000", "+G:E:x", "+G:F:", "+G:G:1:2", "+G:"]
        .iter()
        .map(|t| TagProblem::BadGroup { tag: t.to_string() })
        .collect();
    assert_eq!(m.problems, bad);
}

#[test]
fn a_group_named_twice_counts_once() {
    let m = parse(r#""Vox" +G:A:1 +G:A:2 +PIN +PIN"#);
    assert_eq!(m.groups, vec![group_tag("A", Some(1))]);
    assert!(m.pin);
    assert_eq!(
        m.problems,
        vec![TagProblem::DuplicateGroup { name: "A".into() }]
    );
}

#[test]
fn a_marker_without_a_group_is_a_problem() {
    let m = parse(r#""Vox" +PIN"#);
    assert_eq!(m.problems, vec![TagProblem::NoGroup]);
}

#[test]
fn a_tuner_is_a_marker_by_a_quote_or_a_plus_tag() {
    assert!(!is_marker("Tuner"));
    assert!(!is_marker("Tuner 2"));
    assert!(!is_marker("a+b"));
    assert!(!is_marker(""));
    assert!(is_marker(r#""Vox""#));
    assert!(is_marker("x +PIN"));
    assert!(is_marker("+G:A"));
}

#[test]
fn a_group_title_shows_underscores_as_spaces() {
    assert_eq!(group_title("MASTER_A"), "MASTER A");
    assert_eq!(group_title("HANDS"), "HANDS");
}

fn found(instance: &str, kind: TrackKind, index: u32, name: &str) -> Found {
    Found {
        instance: instance.to_string(),
        kind,
        index,
        name: name.to_string(),
        tuners: 1,
    }
}

/// The frame: the FOH page with a VOCALS group (and a strip bound by name
/// beside it) in its first row and a HANDS group in its second.
fn frame() -> Layout {
    serde_json::from_value(json!({
        "schema": 2,
        "default_page": "foh",
        "pages": [{"id": "foh", "title": "FOH", "rows": [
            {"sections": [
                {"kind": "group", "id": "vocals", "title": "VOCALS", "tags": "VOCALS"},
                {"kind": "group", "id": "fixed", "controls": [
                    {"kind": "strip",
                     "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Bass #"}},
                     "strip_kind": "standard"}]}]},
            {"sections": [{"kind": "group", "id": "hands", "title": "HANDS", "tags": "HANDS"}]}]}]
    }))
    .expect("the frame parses")
}

fn strips(controls: &[Control]) -> Vec<&Strip> {
    controls
        .iter()
        .filter_map(|c| match c {
            Control::Strip(s) => Some(s.as_ref()),
            _ => None,
        })
        .collect()
}

fn labels(controls: &[Control]) -> Vec<String> {
    strips(controls)
        .iter()
        .map(|s| s.label.clone().unwrap_or_default())
        .collect()
}

fn group<'a>(layout: &'a Layout, id: &str) -> &'a Group {
    layout
        .pages
        .iter()
        .flat_map(|p| &p.rows)
        .flat_map(|r| &r.sections)
        .flat_map(|s| s.groups())
        .find(|g| g.id.as_deref() == Some(id))
        .unwrap_or_else(|| panic!("no group {id}"))
}

fn sample() -> Vec<Found> {
    vec![
        found("band", TrackKind::Track, 3, r#""Vox 1" +G:VOCALS:2"#),
        found("band", TrackKind::Track, 1, r#""Vox 2" +G:VOCALS:1 +PIN"#),
        found("band", TrackKind::Track, 5, r#""Vox 3" +G:VOCALS"#),
        found(
            "master",
            TrackKind::Track,
            0,
            r#""Hand 1" +G:HANDS +G:TALKSHOW:1 +MG"#,
        ),
        found("band", TrackKind::Return, 0, r#""Hall" +G:TALKSHOW:2"#),
    ]
}

#[test]
fn a_frame_group_shows_its_tag_group_by_place_then_in_lives_order() {
    let composed = compose(&frame(), &sample());
    assert_eq!(composed.problems, vec![]);
    assert_eq!(composed.layout.validate(), vec![]);
    let vocals = group(&composed.layout, "vocals");
    assert_eq!(labels(&vocals.controls), ["Vox 2", "Vox 1", "Vox 3"]);
    let first = strips(&vocals.controls)[0];
    assert_eq!(first.binding.instance, "band");
    assert_eq!(first.binding.anchor, Anchor::TrackAt { index: 1 });
    assert_eq!(first.binding.path, None);
    assert_eq!(first.strip_kind, StripKind::Standard);
    assert!(first.pinned);
    assert!(!first.mute_guard);
    assert!(!first.wide);
    assert_eq!(first.mark, None);
    let hands = group(&composed.layout, "hands");
    assert_eq!(labels(&hands.controls), ["Hand 1"]);
    let hand = strips(&hands.controls)[0];
    assert_eq!(hand.binding.instance, "master");
    assert!(hand.mute_guard);
    assert!(!hand.pinned);
    // The strip bound by name stays as the frame has it.
    let original = frame();
    assert_eq!(group(&composed.layout, "fixed"), group(&original, "fixed"));
}

#[test]
fn a_tags_groups_own_controls_follow_its_marker_strips() {
    // PR C: a migrated group that ended with a parameter fader keeps it.
    let mut frame = serde_json::to_value(frame()).unwrap();
    frame["pages"][0]["rows"][1]["sections"][0]["controls"] =
        json!([{"kind": "text", "text": "t"}]);
    let frame: Layout = serde_json::from_value(frame).unwrap();
    assert_eq!(frame_problems(&frame), vec![]);
    let composed = compose(&frame, &sample());
    let hands = group(&composed.layout, "hands");
    assert_eq!(labels(&hands.controls), ["Hand 1"]);
    assert_eq!(hands.controls.len(), 2);
    assert!(matches!(hands.controls[0], Control::Strip(_)));
    assert_eq!(hands.controls[1], Control::Text { text: "t".into() });
    // Without markers it shows its own controls alone.
    let bare = compose(&frame, &[]);
    assert_eq!(
        group(&bare.layout, "hands").controls,
        vec![Control::Text { text: "t".into() }]
    );
}

#[test]
fn a_tag_group_the_frame_does_not_show_is_a_view_with_the_pins() {
    let composed = compose(&frame(), &sample());
    let ids: Vec<&str> = composed
        .layout
        .pages
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    assert_eq!(ids, ["foh", "view-TALKSHOW"]);
    let view = &composed.layout.pages[1];
    assert!(view.view);
    assert!(!composed.layout.pages[0].view);
    assert_eq!(view.title, "TALKSHOW");
    assert_eq!(view.rows.len(), 1);
    let own = group(&composed.layout, "view-TALKSHOW-strips");
    assert_eq!(own.title.as_deref(), Some("TALKSHOW"));
    assert_eq!(labels(&own.controls), ["Hand 1", "Hall"]);
    let hall = strips(&own.controls)[1];
    assert_eq!(hall.binding.anchor, Anchor::ReturnAt { index: 0 });
    assert_eq!(hall.strip_kind, StripKind::Return);
    let pins = group(&composed.layout, "view-TALKSHOW-pins-VOCALS");
    assert_eq!(pins.title.as_deref(), Some("VOCALS"));
    assert_eq!(labels(&pins.controls), ["Vox 2"]);
    assert_eq!(view.rows[0].sections.len(), 2);
}

#[test]
fn a_view_keeps_the_rail_of_the_frames_default_page() {
    let rail = |label: &str| json!([{"kind": "hub_toggle", "key": "stage_aut", "label": label}]);
    let mut frame = serde_json::to_value(frame()).unwrap();
    frame["pages"][0]["rail"] = rail("FOH RAIL");
    let mut cue = frame["pages"][0].clone();
    cue["id"] = json!("cue");
    cue["rail"] = rail("CUE RAIL");
    cue["rows"] = json!([]);
    frame["pages"].as_array_mut().unwrap().insert(0, cue);
    let mut frame: Layout = serde_json::from_value(frame).unwrap();
    let rail_of = |layout: &Layout, id: &str| -> Vec<Control> {
        layout
            .pages
            .iter()
            .find(|p| p.id == id)
            .unwrap()
            .rail
            .clone()
    };
    assert_ne!(rail_of(&frame, "foh"), rail_of(&frame, "cue"));
    let composed = compose(&frame, &sample());
    assert_eq!(
        rail_of(&composed.layout, "view-TALKSHOW"),
        rail_of(&frame, "foh")
    );
    // Without a default page: the first page's.
    frame.default_page = "missing".into();
    let composed = compose(&frame, &sample());
    assert_eq!(
        rail_of(&composed.layout, "view-TALKSHOW"),
        rail_of(&frame, "cue")
    );
}

#[test]
fn views_come_in_the_order_of_their_first_marker() {
    let found = vec![
        found("band", TrackKind::Track, 0, r#""A" +G:ZED_2"#),
        found("band", TrackKind::Track, 1, r#""B" +G:ALPHA"#),
    ];
    let composed = compose(&frame(), &found);
    let ids: Vec<&str> = composed
        .layout
        .pages
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    assert_eq!(ids, ["foh", "view-ZED_2", "view-ALPHA"]);
    assert_eq!(composed.layout.pages[1].title, "ZED 2");
    assert_eq!(composed.layout.validate(), vec![]);
}

#[test]
fn an_equal_label_on_two_markers_is_a_conflict_on_both() {
    let found = vec![
        found("band", TrackKind::Track, 7, r#""Klavir" +G:VOCALS"#),
        found("master", TrackKind::Track, 2, r#""Klavir" +G:HANDS"#),
        found("band", TrackKind::Track, 0, r#""Vox" +G:VOCALS"#),
    ];
    let composed = compose(&frame(), &found);
    let vocals = strips(&group(&composed.layout, "vocals").controls)
        .into_iter()
        .map(|s| (s.label.clone().unwrap_or_default(), s.mark))
        .collect::<Vec<_>>();
    assert_eq!(
        vocals,
        [
            ("Vox".to_string(), None),
            ("Klavir".to_string(), Some(StripMark::Conflict))
        ]
    );
    let hands = strips(&group(&composed.layout, "hands").controls)[0];
    assert_eq!(hands.mark, Some(StripMark::Conflict));
    let conflict = TagProblem::Conflict {
        label: "Klavir".into(),
    };
    assert_eq!(
        composed.problems,
        vec![
            MarkerReport {
                instance: "band".into(),
                kind: TrackKind::Track,
                index: 7,
                name: r#""Klavir" +G:VOCALS"#.into(),
                problems: vec![conflict.clone()],
            },
            MarkerReport {
                instance: "master".into(),
                kind: TrackKind::Track,
                index: 2,
                name: r#""Klavir" +G:HANDS"#.into(),
                problems: vec![conflict],
            },
        ]
    );
}

#[test]
fn a_double_tuner_a_shared_place_and_a_missing_label_mark_the_strip() {
    let mut double = found("band", TrackKind::Track, 2, r#""Vox" +G:VOCALS:1"#);
    double.tuners = 2;
    let found = vec![
        double,
        found("band", TrackKind::Track, 4, r#""Vox B" +G:VOCALS:1"#),
        found("band", TrackKind::Track, 7, "+G:VOCALS:3"),
    ];
    let composed = compose(&frame(), &found);
    let vocals = group(&composed.layout, "vocals");
    assert_eq!(labels(&vocals.controls), ["Vox", "Vox B", "#8"]);
    for strip in strips(&vocals.controls) {
        assert_eq!(strip.mark, Some(StripMark::Problem));
    }
    let problems: Vec<(u32, Vec<TagProblem>)> = composed
        .problems
        .iter()
        .map(|r| (r.index, r.problems.clone()))
        .collect();
    let same = TagProblem::SamePlace {
        group: "VOCALS".into(),
        place: 1,
    };
    assert_eq!(
        problems,
        vec![
            (2, vec![TagProblem::DoubleTuner, same.clone()]),
            (4, vec![same]),
            (7, vec![TagProblem::NoLabel]),
        ]
    );
}

#[test]
fn one_tuner_per_track_and_distinct_places_are_no_problem() {
    let found = vec![
        found("band", TrackKind::Track, 2, r#""Vox" +G:VOCALS:1"#),
        found("band", TrackKind::Track, 4, r#""Vox B" +G:VOCALS:2"#),
    ];
    let composed = compose(&frame(), &found);
    assert_eq!(composed.problems, vec![]);
    for strip in strips(&group(&composed.layout, "vocals").controls) {
        assert_eq!(strip.mark, None);
    }
}

#[test]
fn an_unknown_tag_marks_a_strip_that_still_shows() {
    let found = vec![found(
        "band",
        TrackKind::Track,
        1,
        r#""Gitara" +G:VOCALS +GX"#,
    )];
    let composed = compose(&frame(), &found);
    let strip = strips(&group(&composed.layout, "vocals").controls)[0];
    assert_eq!(strip.label.as_deref(), Some("Gitara"));
    assert_eq!(strip.mark, Some(StripMark::Problem));
}

#[test]
fn a_marker_without_a_group_shows_nowhere_but_is_reported() {
    let found = vec![found("band", TrackKind::Track, 9, r#""Lost" +PIN"#)];
    let composed = compose(&frame(), &found);
    let shown = composed
        .layout
        .controls()
        .into_iter()
        .filter(|c| matches!(c, Control::Strip(s) if s.label.as_deref() == Some("Lost")))
        .count();
    assert_eq!(shown, 0);
    assert_eq!(composed.layout.pages.len(), 1);
    assert_eq!(composed.problems.len(), 1);
    assert_eq!(composed.problems[0].problems, vec![TagProblem::NoGroup]);
}

#[test]
fn the_composition_does_not_depend_on_the_order_markers_were_found() {
    let found = sample();
    let mut reversed = found.clone();
    reversed.reverse();
    assert_eq!(compose(&frame(), &found), compose(&frame(), &reversed));
}

#[test]
fn no_markers_leave_the_frame_as_it_is_but_the_tag_groups_empty() {
    let composed = compose(&frame(), &[]);
    assert_eq!(composed.layout, frame());
    assert_eq!(composed.problems, vec![]);
}

#[test]
fn a_problem_report_serializes_with_its_code() {
    let report = MarkerReport {
        instance: "band".into(),
        kind: TrackKind::Return,
        index: 3,
        name: "x".into(),
        problems: vec![
            TagProblem::NoLabel,
            TagProblem::SamePlace {
                group: "A".into(),
                place: 2,
            },
        ],
    };
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        json!({"instance": "band", "kind": "return", "index": 3, "name": "x",
               "problems": [{"code": "no_label"}, {"code": "same_place", "group": "A", "place": 2}]})
    );
}

#[test]
fn a_views_pins_are_grouped_by_their_first_group_in_the_markers_order() {
    let found = vec![
        found("band", TrackKind::Track, 0, r#""A" +G:SOLO"#),
        found("band", TrackKind::Track, 1, r#""P1" +G:VOCALS +PIN"#),
        found(
            "band",
            TrackKind::Track,
            2,
            r#""P2" +G:VOCALS:1 +G:HANDS +PIN"#,
        ),
        found("band", TrackKind::Track, 3, r#""P3" +G:HANDS +PIN"#),
    ];
    let composed = compose(&frame(), &found);
    assert_eq!(composed.layout.validate(), vec![]);
    let view = composed
        .layout
        .pages
        .iter()
        .find(|p| p.id == "view-SOLO")
        .expect("the view");
    let groups: Vec<(Option<String>, Vec<String>)> = view.rows[0]
        .sections
        .iter()
        .flat_map(Section::groups)
        .map(|g| (g.id.clone(), labels(&g.controls)))
        .collect();
    assert_eq!(
        groups,
        vec![
            (Some("view-SOLO-strips".into()), vec!["A".to_string()]),
            (
                Some("view-SOLO-pins-VOCALS".into()),
                vec!["P1".to_string(), "P2".to_string()]
            ),
            (Some("view-SOLO-pins-HANDS".into()), vec!["P3".to_string()]),
        ]
    );
}

#[test]
fn a_frame_holds_no_view_no_view_id_and_no_tags_group_with_controls() {
    assert_eq!(frame_problems(&frame()), vec![]);
    let bad: Layout = serde_json::from_value(json!({
        "schema": 2,
        "default_page": "p",
        "pages": [
            {"id": "p", "title": "P", "rows": [{"sections": [
                {"kind": "group", "id": "view-x", "tags": "A", "controls": [
                    {"kind": "text", "text": "t"},
                    {"kind": "strip", "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Bass #"}},
                     "strip_kind": "standard"}]},
                {"kind": "pager", "id": "view-pager", "default_page": "view-sub", "pages": [
                    {"id": "view-sub", "title": "S"}]},
                {"kind": "group", "id": "own", "tags": "B", "controls": [
                    {"kind": "text", "text": "t"}]}]}]},
            {"id": "view-A", "title": "A", "view": true}
        ]
    }))
    .expect("parses");
    let problems: Vec<String> = frame_problems(&bad)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        problems,
        vec![
            r#"pages[0].rows[0].sections[0]: "view-x" starts with "view-", the Tuner markers' views"#
                .to_string(),
            r#"pages[0].rows[0].sections[1]: "view-pager" starts with "view-", the Tuner markers' views"#
                .to_string(),
            r#"pages[0].rows[0].sections[1]: "view-sub" starts with "view-", the Tuner markers' views"#
                .to_string(),
            r#"pages[1]: "view-A" starts with "view-", the Tuner markers' views"#.to_string(),
            r#"pages[0]: the tags group "view-x" holds a strip of its own (the markers' strips fill it)"#
                .to_string(),
            "pages[1]: a view page: the Tuner markers make those".to_string(),
        ]
    );
}
