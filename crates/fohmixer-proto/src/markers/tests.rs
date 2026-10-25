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

/// A former MIDI fader of invented names: a control a tags group keeps
/// after its marker strips (PR C).
fn fader() -> serde_json::Value {
    json!({"kind": "param_fader", "label": "Hands All",
           "targets": [{"binding": {"instance": "master", "anchor": {"kind": "track", "name": "Hands grp#"},
                                    "path": "mixer_device volume"},
                        "prop": "value", "scale": "cc_linear"}]})
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
    frame["pages"][0]["rows"][1]["sections"][0]["controls"] = json!([fader()]);
    let frame: Layout = serde_json::from_value(frame).unwrap();
    assert_eq!(frame_problems(&frame), vec![]);
    let own: Control = serde_json::from_value(fader()).unwrap();
    let composed = compose(&frame, &sample());
    let hands = group(&composed.layout, "hands");
    assert_eq!(labels(&hands.controls), ["Hand 1"]);
    assert_eq!(hands.controls.len(), 2);
    assert!(matches!(hands.controls[0], Control::Strip(_)));
    assert_eq!(hands.controls[1], own);
    // Without markers it shows its own controls alone.
    let bare = compose(&frame, &[]);
    assert_eq!(group(&bare.layout, "hands").controls, vec![own]);
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
    let rail = |label: &str| {
        json!([{"kind": "stage", "label": label,
                "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Mics Stage #"}}}])
    };
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
fn a_frame_holds_no_view_no_view_id_and_no_strip_in_a_tags_group() {
    assert_eq!(frame_problems(&frame()), vec![]);
    let bad: Layout = serde_json::from_value(json!({
        "schema": 2,
        "default_page": "p",
        "pages": [
            {"id": "p", "title": "P", "rows": [{"sections": [
                {"kind": "group", "id": "view-x", "tags": "A", "controls": [
                    fader(),
                    {"kind": "strip", "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Bass #"}},
                     "strip_kind": "standard"}]},
                {"kind": "pager", "id": "view-pager", "default_page": "view-sub", "pages": [
                    {"id": "view-sub", "title": "S"}]},
                {"kind": "group", "id": "own", "tags": "B", "controls": [
                    fader()]}]}]},
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

#[test]
fn a_frame_strip_carries_no_linked_mute() {
    // #127 review M4: a linked mute is the Tuner markers' (`+ID` and
    // `+MUTE`, the owner's ruling), never the frame's: a frame strip with
    // `mute_links`, on a page or among the global controls, is refused as a
    // view page is.
    let link = json!([{"instance": "band", "anchor": {"kind": "return_at", "index": 0}}]);
    let mut v = serde_json::to_value(frame()).unwrap();
    v["pages"][0]["rows"][0]["sections"][1]["controls"][0]["mute_links"] = link.clone();
    v["global"] = json!([
        fader(),
        {"kind": "strip",
         "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Keys #"}},
         "strip_kind": "standard",
         "mute_links": link}
    ]);
    let linked: Layout = serde_json::from_value(v).expect("parses");
    assert_eq!(linked.validate(), vec![], "a valid layout otherwise");
    let problems: Vec<String> = frame_problems(&linked)
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        problems,
        vec![
            "pages[0]: a linked mute: the Tuner markers make those".to_string(),
            "global[1]: a linked mute: the Tuner markers make those".to_string(),
        ]
    );
    // The same strips without it pass.
    let mut plain = serde_json::to_value(frame()).unwrap();
    plain["global"] = json!([fader()]);
    let plain: Layout = serde_json::from_value(plain).expect("parses");
    assert_eq!(frame_problems(&plain), vec![]);
}

#[test]
fn a_marker_without_a_label_shows_its_tracks_number_and_is_known_by_it() {
    assert_eq!(placeholder_label(0), "#1");
    assert_eq!(placeholder_label(7), "#8");
    // #71: compose's strips say which label is a placeholder.
    let composed = compose(
        &frame(),
        &[
            found("band", TrackKind::Track, 3, r#""Vox 1" +G:VOCALS"#),
            found("band", TrackKind::Track, 7, "+G:VOCALS"),
            found("band", TrackKind::Return, 2, r#""" +G:VOCALS"#),
        ],
    );
    let vocals = group(&composed.layout, "vocals");
    let shown: Vec<(String, bool)> = strips(&vocals.controls)
        .iter()
        .map(|s| {
            (
                s.label.clone().unwrap_or_default(),
                has_placeholder_label(s),
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            ("Vox 1".to_string(), false),
            ("#8".to_string(), true),
            ("#3".to_string(), true)
        ]
    );
    // Only on an index anchor, and only its own index's number.
    let bound = |anchor: Anchor, label: &str| Strip {
        binding: Binding {
            instance: "band".into(),
            anchor,
            path: None,
        },
        strip_kind: StripKind::Standard,
        wide: false,
        mute_guard: false,
        pinned: false,
        label: Some(label.into()),
        mark: None,
        mute_links: Vec::new(),
        reverb: false,
        fader_link: None,
        fader_follows: Vec::new(),
    };
    assert!(has_placeholder_label(&bound(
        Anchor::TrackAt { index: 2 },
        "#3"
    )));
    assert!(has_placeholder_label(&bound(
        Anchor::ReturnAt { index: 0 },
        "#1"
    )));
    assert!(!has_placeholder_label(&bound(
        Anchor::TrackAt { index: 4 },
        "#3"
    )));
    assert!(!has_placeholder_label(&bound(
        Anchor::TrackAt { index: 2 },
        "Vox"
    )));
    let named = Anchor::Track {
        name: "Hand1 #".into(),
    };
    assert!(!has_placeholder_label(&bound(named, "#1")));
    let mut unlabelled = bound(Anchor::TrackAt { index: 0 }, "#1");
    unlabelled.label = None;
    assert!(!has_placeholder_label(&unlabelled));
}

#[test]
fn a_strip_carries_an_id_and_any_tuner_mutes_with_one() {
    // #127: `+ID:<id>` on a strip's Tuner, `+MUTE:<id>` on any track's.
    let m = parse(r#""Vox 1" +G:VOCALS +ID:VOX1 +MG"#);
    assert_eq!(m.id.as_deref(), Some("VOX1"));
    assert_eq!(m.mutes, Vec::<String>::new());
    assert!(m.mute_guard);
    assert_eq!(m.problems, vec![]);
    // A Tuner with only `+MUTE` makes no strip: no label and no group to miss.
    let m = parse("+MUTE:VOX1");
    assert_eq!(m.label, None);
    assert_eq!(m.id, None);
    assert_eq!(m.groups, vec![]);
    assert_eq!(m.mutes, vec!["VOX1".to_string()]);
    assert_eq!(m.problems, vec![]);
    // With a label it is a strip of its own (and mutes with VOX1 and V-2_B).
    let m = parse(r#""Vox 1 S" +G:STREAM +MUTE:VOX1 +MUTE:V-2_B +ID:VOXS"#);
    assert_eq!(m.label.as_deref(), Some("Vox 1 S"));
    assert_eq!(m.groups, vec![group_tag("STREAM", None)]);
    assert_eq!(m.mutes, vec!["VOX1".to_string(), "V-2_B".to_string()]);
    assert_eq!(m.id.as_deref(), Some("VOXS"));
    assert_eq!(m.problems, vec![]);
    // A label and no group: a strip that shows nowhere, as before.
    let m = parse(r#""Vox" +MUTE:A"#);
    assert_eq!(m.problems, vec![TagProblem::NoGroup]);
}

#[test]
fn only_a_tuner_of_nothing_but_mutes_is_spared_its_label_and_group() {
    let problems = |name: &str| parse(name).problems;
    let none = Vec::<TagProblem>::new();
    assert_eq!(problems("+MUTE:A +MUTE:B"), none);
    // Any other tag wants a strip, so its label and group count.
    for name in ["+MUTE:A +PIN", "+MUTE:A +MG", "+MUTE:A +ID:B"] {
        assert_eq!(
            problems(name),
            vec![TagProblem::NoLabel, TagProblem::NoGroup],
            "{name}"
        );
    }
    assert_eq!(problems("+MUTE:A +G:X"), vec![TagProblem::NoLabel]);
    // No mutes: a name without a quote or a tag is as before.
    assert_eq!(
        problems("Vox"),
        vec![
            TagProblem::NoLabel,
            TagProblem::StrayText { text: "Vox".into() },
            TagProblem::NoGroup
        ]
    );
    assert_eq!(
        problems("+PIN"),
        vec![TagProblem::NoLabel, TagProblem::NoGroup]
    );
    // A label that is there but empty or unclosed stays a problem.
    assert_eq!(
        problems(r#""" +MUTE:A"#),
        vec![TagProblem::EmptyLabel, TagProblem::NoGroup]
    );
    assert_eq!(
        problems(r#""Vox +MUTE:A"#),
        vec![
            TagProblem::UnclosedLabel,
            TagProblem::StrayText { text: "Vox".into() },
            TagProblem::NoGroup
        ]
    );
    // Stray text beside it is still shown.
    assert_eq!(
        problems("Vox +MUTE:A"),
        vec![TagProblem::StrayText { text: "Vox".into() }]
    );
}

#[test]
fn autotune_tags_name_where_autotune_is_on() {
    // #104: `+AUTOTUNE` on 127 (the stream racks), `+AUTOTUNE:0` on 0 (repro).
    let m = parse(r#""Vox 1" +G:STAGE +AUTOTUNE"#);
    assert_eq!(m.autotune, Some(AutotuneOn::Max));
    assert_eq!(m.label.as_deref(), Some("Vox 1"));
    assert_eq!(m.groups, vec![group_tag("STAGE", None)]);
    assert_eq!(m.problems, vec![]);
    let m = parse(r#""Vox 1" +G:STAGE +AUTOTUNE:0"#);
    assert_eq!(m.autotune, Some(AutotuneOn::Zero));
    assert_eq!(m.problems, vec![]);
    assert_eq!(parse(r#""Vox 1" +G:STAGE"#).autotune, None);
    for tag in ["+AUTOTUNE:5", "+AUTOTUNE:", "+AUTOTUNE:00", "+AUTOTUNE:127"] {
        let m = parse(&format!(r#""Vox 1" +G:STAGE {tag}"#));
        assert_eq!(m.autotune, None, "{tag}");
        assert_eq!(
            m.problems,
            vec![TagProblem::BadAutotune { tag: tag.into() }],
            "{tag}"
        );
    }
    // A second one is a problem, and the first counts.
    let m = parse(r#""Vox 1" +G:STAGE +AUTOTUNE +AUTOTUNE:0"#);
    assert_eq!(m.autotune, Some(AutotuneOn::Max));
    assert_eq!(
        m.problems,
        vec![TagProblem::SecondAutotune {
            tag: "+AUTOTUNE:0".into()
        }]
    );
    let m = parse(r#""Vox 1" +G:STAGE +AUTOTUNE:0 +AUTOTUNE"#);
    assert_eq!(m.autotune, Some(AutotuneOn::Zero));
    assert_eq!(
        m.problems,
        vec![TagProblem::SecondAutotune {
            tag: "+AUTOTUNE".into()
        }]
    );
    // A bad one after a good one is bad, not second.
    let m = parse(r#""Vox 1" +G:STAGE +AUTOTUNE +AUTOTUNE:x"#);
    assert_eq!(m.autotune, Some(AutotuneOn::Max));
    assert_eq!(
        m.problems,
        vec![TagProblem::BadAutotune {
            tag: "+AUTOTUNE:x".into()
        }]
    );
    // Lowercase is no tag of ours.
    assert_eq!(
        parse(r#""Vox 1" +G:STAGE +autotune"#).problems,
        vec![TagProblem::UnknownTag {
            tag: "+autotune".into()
        }]
    );
    assert_eq!(AutotuneOn::Max.on(), 127);
    assert_eq!(AutotuneOn::Max.off(), 0);
    assert_eq!(AutotuneOn::Zero.on(), 0);
    assert_eq!(AutotuneOn::Zero.off(), 127);
}

#[test]
fn a_tuner_of_links_and_autotune_makes_no_strip() {
    let problems = |name: &str| parse(name).problems;
    let none = Vec::<TagProblem>::new();
    // A stream track's Tuner: `+MUTE:<id> +AUTOTUNE`, or the tag alone.
    assert_eq!(problems("+MUTE:A +AUTOTUNE"), none);
    assert_eq!(problems("+AUTOTUNE"), none);
    assert_eq!(problems("+AUTOTUNE:0"), none);
    assert_eq!(problems("+FADER:B +AUTOTUNE"), none);
    assert_eq!(parse("+AUTOTUNE").autotune, Some(AutotuneOn::Max));
    // Any strip tag beside it wants a strip: its label and group count.
    for name in [
        "+AUTOTUNE +PIN",
        "+AUTOTUNE +MG",
        "+AUTOTUNE +REVERB",
        "+AUTOTUNE:0 +ID:X",
    ] {
        assert_eq!(
            problems(name),
            vec![TagProblem::NoLabel, TagProblem::NoGroup],
            "{name}"
        );
    }
    // A bad autotune tag alone is no autotune: no strip's tag, no link.
    assert_eq!(
        problems("+AUTOTUNE:5"),
        vec![
            TagProblem::NoLabel,
            TagProblem::BadAutotune {
                tag: "+AUTOTUNE:5".into()
            },
            TagProblem::NoGroup
        ]
    );
    // Stray text beside it is still shown.
    assert_eq!(
        problems("Vox +AUTOTUNE"),
        vec![TagProblem::StrayText { text: "Vox".into() }]
    );
    // A label with it: a strip that shows nowhere without a group.
    assert_eq!(problems(r#""Vox" +AUTOTUNE"#), vec![TagProblem::NoGroup]);
}

#[test]
fn the_autotune_problems_serialize_with_their_codes() {
    let problems = vec![
        TagProblem::BadAutotune {
            tag: "+AUTOTUNE:5".into(),
        },
        TagProblem::SecondAutotune {
            tag: "+AUTOTUNE:0".into(),
        },
    ];
    assert_eq!(
        serde_json::to_value(&problems).unwrap(),
        json!([
            {"code": "bad_autotune", "tag": "+AUTOTUNE:5"},
            {"code": "second_autotune", "tag": "+AUTOTUNE:0"},
        ])
    );
}

#[test]
fn a_bad_second_or_repeated_id_is_a_problem() {
    let ok = "A".repeat(MAX_GROUP_NAME);
    let long = "A".repeat(MAX_GROUP_NAME + 1);
    let m = parse(&format!(
        r#""Vox" +G:A +ID:vox +ID: +ID:{long} +ID:A1 +ID:B2 +MUTE:x +MUTE:{ok} +MUTE:{ok} +MUTE: +MUTE +ID"#
    ));
    assert_eq!(m.id.as_deref(), Some("A1"));
    assert_eq!(m.mutes, vec![ok.clone()]);
    let bad = |tag: &str| TagProblem::BadId { tag: tag.into() };
    assert_eq!(
        m.problems,
        vec![
            bad("+ID:vox"),
            bad("+ID:"),
            bad(&format!("+ID:{long}")),
            TagProblem::SecondId {
                tag: "+ID:B2".into()
            },
            bad("+MUTE:x"),
            TagProblem::DuplicateMute { id: ok },
            bad("+MUTE:"),
            TagProblem::UnknownTag {
                tag: "+MUTE".into()
            },
            TagProblem::UnknownTag { tag: "+ID".into() },
        ]
    );
}

/// The strip of `label` in `layout`'s group `id`.
fn strip_in<'a>(layout: &'a Layout, id: &str, label: &str) -> &'a Strip {
    strips(&group(layout, id).controls)
        .into_iter()
        .find(|s| s.label.as_deref() == Some(label))
        .unwrap_or_else(|| panic!("no strip {label} in {id}"))
}

fn at(instance: &str, anchor: Anchor) -> Binding {
    Binding {
        instance: instance.into(),
        anchor,
        path: None,
    }
}

#[test]
fn a_strip_whose_id_others_name_mutes_their_tracks_with_its_own() {
    // A link of 3: the strip's track, a track and a return, each by index.
    let found = vec![
        found("band", TrackKind::Track, 3, r#""Vox 1" +G:VOCALS +ID:VOX1"#),
        found("band", TrackKind::Track, 9, "+MUTE:VOX1"),
        found("band", TrackKind::Return, 1, "+MUTE:VOX1"),
        found("band", TrackKind::Track, 4, r#""Vox 2" +G:VOCALS"#),
    ];
    let composed = compose(&frame(), &found);
    assert_eq!(composed.problems, vec![]);
    assert_eq!(composed.layout.validate(), vec![]);
    // The partners make no strip; only Vox 1 links.
    assert_eq!(
        labels(&group(&composed.layout, "vocals").controls),
        ["Vox 1", "Vox 2"]
    );
    let vox1 = strip_in(&composed.layout, "vocals", "Vox 1");
    assert_eq!(vox1.binding, at("band", Anchor::TrackAt { index: 3 }));
    assert_eq!(
        vox1.mute_links,
        vec![
            at("band", Anchor::TrackAt { index: 9 }),
            at("band", Anchor::ReturnAt { index: 1 })
        ]
    );
    assert_eq!(vox1.mark, None);
    assert_eq!(
        strip_in(&composed.layout, "vocals", "Vox 2").mute_links,
        vec![]
    );
    assert_eq!(composed.layout.pages.len(), 1, "no view of its own");
    // In any order the hub finds them.
    let mut reversed = found.clone();
    reversed.reverse();
    assert_eq!(compose(&frame(), &reversed), composed);
}

#[test]
fn a_partner_may_be_a_strip_of_its_own_with_a_link_of_its_own() {
    let found = vec![
        found("band", TrackKind::Track, 3, r#""Vox 1" +G:VOCALS +ID:VOX1"#),
        found(
            "band",
            TrackKind::Track,
            5,
            r#""Vox 1 S" +G:HANDS +ID:VOXS +MUTE:VOX1"#,
        ),
        found("band", TrackKind::Track, 7, "+MUTE:VOXS"),
    ];
    let composed = compose(&frame(), &found);
    assert_eq!(composed.problems, vec![]);
    assert_eq!(
        strip_in(&composed.layout, "vocals", "Vox 1").mute_links,
        vec![at("band", Anchor::TrackAt { index: 5 })]
    );
    assert_eq!(
        strip_in(&composed.layout, "hands", "Vox 1 S").mute_links,
        vec![at("band", Anchor::TrackAt { index: 7 })]
    );
}

#[test]
fn an_id_on_two_strips_marks_both_and_links_neither() {
    let found = vec![
        found("band", TrackKind::Track, 3, r#""A" +G:VOCALS +ID:VOX1"#),
        found("band", TrackKind::Track, 4, r#""B" +G:VOCALS +ID:VOX1"#),
        found("band", TrackKind::Track, 6, "+MUTE:VOX1"),
        // The same id in the other Live set is no duplicate: each set's
        // own (its +MUTE links within its set).
        found("master", TrackKind::Track, 0, r#""C" +G:HANDS +ID:VOX1"#),
        found("master", TrackKind::Return, 2, "+MUTE:VOX1"),
    ];
    let composed = compose(&frame(), &found);
    let duplicate = TagProblem::DuplicateId { id: "VOX1".into() };
    let problems: Vec<(&str, u32, Vec<TagProblem>)> = composed
        .problems
        .iter()
        .map(|r| (r.instance.as_str(), r.index, r.problems.clone()))
        .collect();
    assert_eq!(
        problems,
        vec![
            ("band", 3, vec![duplicate.clone()]),
            ("band", 4, vec![duplicate]),
        ]
    );
    for label in ["A", "B"] {
        let strip = strip_in(&composed.layout, "vocals", label);
        assert_eq!(strip.mark, Some(StripMark::Problem), "{label}");
        assert_eq!(strip.mute_links, vec![], "{label}");
    }
    let c = strip_in(&composed.layout, "hands", "C");
    assert_eq!(c.mark, None);
    assert_eq!(
        c.mute_links,
        vec![at("master", Anchor::ReturnAt { index: 2 })]
    );
}

#[test]
fn a_mute_naming_no_strip_or_its_own_is_a_problem_listed_with_its_marker() {
    let found = vec![
        found(
            "band",
            TrackKind::Track,
            2,
            r#""Self" +G:VOCALS +ID:SELF +MUTE:SELF"#,
        ),
        found("band", TrackKind::Track, 7, "+MUTE:NOBODY"),
        found(
            "band",
            TrackKind::Track,
            8,
            r#""Lone" +G:VOCALS +MUTE:NOBODY"#,
        ),
        // An id of the other Live set is no strip of this one.
        found("master", TrackKind::Track, 1, r#""Far" +G:HANDS +ID:FAR"#),
        found("band", TrackKind::Return, 0, "+MUTE:FAR"),
    ];
    let composed = compose(&frame(), &found);
    assert_eq!(
        composed.problems,
        vec![
            MarkerReport {
                instance: "band".into(),
                kind: TrackKind::Track,
                index: 2,
                name: r#""Self" +G:VOCALS +ID:SELF +MUTE:SELF"#.into(),
                problems: vec![TagProblem::SelfMute { id: "SELF".into() }],
            },
            MarkerReport {
                instance: "band".into(),
                kind: TrackKind::Track,
                index: 7,
                name: "+MUTE:NOBODY".into(),
                problems: vec![TagProblem::OrphanMute {
                    id: "NOBODY".into()
                }],
            },
            MarkerReport {
                instance: "band".into(),
                kind: TrackKind::Track,
                index: 8,
                name: r#""Lone" +G:VOCALS +MUTE:NOBODY"#.into(),
                problems: vec![TagProblem::OrphanMute {
                    id: "NOBODY".into()
                }],
            },
            MarkerReport {
                instance: "band".into(),
                kind: TrackKind::Return,
                index: 0,
                name: "+MUTE:FAR".into(),
                problems: vec![TagProblem::OrphanMute { id: "FAR".into() }],
            },
        ]
    );
    let own = strip_in(&composed.layout, "vocals", "Self");
    assert_eq!(own.mark, Some(StripMark::Problem));
    assert_eq!(own.mute_links, vec![]);
    assert_eq!(
        strip_in(&composed.layout, "vocals", "Lone").mark,
        Some(StripMark::Problem)
    );
    assert_eq!(
        strip_in(&composed.layout, "hands", "Far").mute_links,
        vec![]
    );
}

#[test]
fn the_linked_mutes_problems_serialize_with_their_codes() {
    let problems = vec![
        TagProblem::BadId {
            tag: "+ID:x".into(),
        },
        TagProblem::SecondId {
            tag: "+ID:B".into(),
        },
        TagProblem::DuplicateMute { id: "A".into() },
        TagProblem::DuplicateId { id: "A".into() },
        TagProblem::OrphanMute { id: "A".into() },
        TagProblem::SelfMute { id: "A".into() },
    ];
    assert_eq!(
        serde_json::to_value(&problems).unwrap(),
        json!([
            {"code": "bad_id", "tag": "+ID:x"},
            {"code": "second_id", "tag": "+ID:B"},
            {"code": "duplicate_mute", "id": "A"},
            {"code": "duplicate_id", "id": "A"},
            {"code": "orphan_mute", "id": "A"},
            {"code": "self_mute", "id": "A"},
        ])
    );
}

#[test]
fn reverb_marks_a_strip_of_the_rule_and_never_one_in_conflict() {
    let marker = parse(r#""Rev ST" +G:VOCALS +REVERB"#);
    assert!(marker.reverb);
    assert!(marker.problems.is_empty(), "{:?}", marker.problems);
    assert!(!parse(r#""Vox 1" +G:VOCALS"#).reverb);
    let composed = compose(
        &frame(),
        &[
            found("band", TrackKind::Track, 4, r#""Rev ST" +G:VOCALS +REVERB"#),
            found("band", TrackKind::Track, 6, r#""Vox 1" +G:VOCALS"#),
            found("band", TrackKind::Track, 7, r#""Rev" +G:VOCALS +REVERB"#),
            found("band", TrackKind::Track, 8, r#""Rev" +G:HANDS +REVERB"#),
        ],
    );
    assert!(strip_in(&composed.layout, "vocals", "Rev ST").reverb);
    assert!(!strip_in(&composed.layout, "vocals", "Vox 1").reverb);
    assert!(
        !strip_in(&composed.layout, "vocals", "Rev").reverb,
        "a conflict's label is no identity"
    );
    assert!(!strip_in(&composed.layout, "hands", "Rev").reverb);
}

#[test]
fn a_frame_strip_cannot_be_a_reverb_fader() {
    let frame: Layout = serde_json::from_value(
        json!({"schema": 2, "default_page": "p", "pages": [{"id": "p", "title": "P",
        "rows": [{"sections": [{"kind": "group", "id": "g", "controls": [{"kind": "strip",
            "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Rev #"}},
            "strip_kind": "standard", "reverb": true}]}]}]}]}),
    )
    .unwrap();
    let messages: Vec<String> = frame_problems(&frame)
        .into_iter()
        .map(|e| e.message)
        .collect();
    assert_eq!(messages, ["a reverb fader: the Tuner markers make those"]);
}

#[test]
fn fader_tags_parse_each_id_once_by_the_group_name_rule() {
    let marker = parse("+FADER:REV +FADER:VOX1 +FADER:REV +FADER:rev");
    assert_eq!(marker.faders, vec!["REV".to_string(), "VOX1".to_string()]);
    assert_eq!(
        marker.problems,
        vec![
            TagProblem::DuplicateFader { id: "REV".into() },
            TagProblem::BadId {
                tag: "+FADER:rev".into()
            },
        ]
    );
    // Only links: no strip, so no label or group to miss.
    assert_eq!(parse("+FADER:REV").problems, vec![]);
    assert_eq!(parse("+MUTE:VOX1 +FADER:REV").problems, vec![]);
    // Anything else beside them: a strip's marker, which misses both.
    for name in [
        "+FADER:REV +PIN",
        "+FADER:REV +MG",
        "+FADER:REV +REVERB",
        "+FADER:REV +ID:X",
    ] {
        assert_eq!(
            parse(name).problems,
            vec![TagProblem::NoLabel, TagProblem::NoGroup],
            "{name}"
        );
    }
}

#[test]
fn a_leaders_id_named_by_fader_tags_links_their_volumes_in_lives_order() {
    let found = [
        found(
            "band",
            TrackKind::Track,
            3,
            r#""Rev PA" +G:VOCALS +ID:REV +REVERB"#,
        ),
        found(
            "band",
            TrackKind::Track,
            2,
            r#""Rev ST" +G:VOCALS +FADER:REV +REVERB"#,
        ),
        found("band", TrackKind::Return, 1, "+FADER:REV"),
        found("band", TrackKind::Track, 4, r#""Vox 1" +G:VOCALS"#),
    ];
    let composed = compose(&frame(), &found);
    assert_eq!(composed.problems, vec![]);
    assert_eq!(composed.layout.validate(), vec![]);
    assert_eq!(
        labels(&group(&composed.layout, "vocals").controls),
        ["Rev ST", "Rev PA", "Vox 1"]
    );
    let pa = strip_in(&composed.layout, "vocals", "Rev PA");
    assert_eq!(
        pa.fader_link,
        Some(FaderLink {
            id: "REV".into(),
            partners: vec![
                LinkPartner {
                    binding: at("band", Anchor::TrackAt { index: 2 }),
                    label: "Rev ST".into()
                },
                LinkPartner {
                    binding: at("band", Anchor::ReturnAt { index: 1 }),
                    label: "#2".into()
                },
            ]
        })
    );
    assert_eq!(pa.fader_follows, vec![]);
    assert_eq!(pa.mute_links, vec![], "a fader link is no mute link");
    let st = strip_in(&composed.layout, "vocals", "Rev ST");
    assert_eq!(st.fader_link, None);
    assert_eq!(
        st.fader_follows,
        vec![FaderFollow {
            id: "REV".into(),
            leader: "Rev PA".into(),
            fixed: false
        }]
    );
    assert_eq!(
        strip_in(&composed.layout, "vocals", "Vox 1").fader_follows,
        vec![]
    );
    assert_eq!(
        composed.links,
        vec![LinkReport {
            instance: "band".into(),
            id: "REV".into(),
            leader: Leader::Strip,
            partners: 2
        }]
    );
    let mut reversed = found.clone();
    reversed.reverse();
    assert_eq!(compose(&frame(), &reversed), composed);
}

#[test]
fn one_id_serves_a_mute_link_and_a_fader_link() {
    let composed = compose(
        &frame(),
        &[
            found("band", TrackKind::Track, 3, r#""Vox 1" +G:VOCALS +ID:VOX1"#),
            found("band", TrackKind::Track, 9, "+MUTE:VOX1 +FADER:VOX1"),
        ],
    );
    assert_eq!(composed.problems, vec![]);
    let vox = strip_in(&composed.layout, "vocals", "Vox 1");
    assert_eq!(
        vox.mute_links,
        vec![at("band", Anchor::TrackAt { index: 9 })]
    );
    assert_eq!(vox.fader_link.as_ref().map(|l| l.partners.len()), Some(1));
    assert_eq!(
        labels(&group(&composed.layout, "vocals").controls),
        ["Vox 1"],
        "the partner makes no strip"
    );
}

#[test]
fn a_fader_tags_problems_mirror_the_linked_mutes() {
    let found = [
        found(
            "band",
            TrackKind::Track,
            1,
            r#""A" +G:VOCALS +ID:REV +FADER:REV"#,
        ),
        found("band", TrackKind::Track, 2, "+FADER:NONE +FADER:NONE"),
        found("band", TrackKind::Track, 4, r#""B" +G:VOCALS +ID:TWO"#),
        found("band", TrackKind::Track, 5, r#""C" +G:VOCALS +ID:TWO"#),
        found("band", TrackKind::Track, 6, "+FADER:TWO"),
        // An id holds in its own Live set only.
        found("master", TrackKind::Track, 0, "+FADER:REV"),
    ];
    let composed = compose(&frame(), &found);
    let problems: Vec<(&str, u32, Vec<TagProblem>)> = composed
        .problems
        .iter()
        .map(|r| (r.instance.as_str(), r.index, r.problems.clone()))
        .collect();
    assert_eq!(
        problems,
        vec![
            ("band", 1, vec![TagProblem::SelfFader { id: "REV".into() }]),
            (
                "band",
                2,
                vec![
                    TagProblem::DuplicateFader { id: "NONE".into() },
                    TagProblem::OrphanFader { id: "NONE".into() }
                ]
            ),
            (
                "band",
                4,
                vec![TagProblem::DuplicateId { id: "TWO".into() }]
            ),
            (
                "band",
                5,
                vec![TagProblem::DuplicateId { id: "TWO".into() }]
            ),
            (
                "master",
                0,
                vec![TagProblem::OrphanFader { id: "REV".into() }]
            ),
        ]
    );
    for label in ["A", "B", "C"] {
        assert_eq!(
            strip_in(&composed.layout, "vocals", label).fader_link,
            None,
            "{label}"
        );
    }
    assert_eq!(composed.links, vec![]);
    assert_eq!(
        serde_json::to_value(TagProblem::OrphanFader { id: "X".into() }).unwrap(),
        json!({"code": "orphan_fader", "id": "X"})
    );
    assert_eq!(
        serde_json::to_value(TagProblem::SelfFader { id: "X".into() }).unwrap(),
        json!({"code": "self_fader", "id": "X"})
    );
    assert_eq!(
        serde_json::to_value(TagProblem::DuplicateFader { id: "X".into() }).unwrap(),
        json!({"code": "duplicate_fader", "id": "X"})
    );
}

/// A frame whose `bt` group holds Podklady All linked to band's `PODKLADY`
/// (#130) beside the VOCALS tags group.
fn podklady_frame() -> Layout {
    serde_json::from_value(json!({"schema": 2, "default_page": "foh", "pages": [{"id": "foh", "title": "FOH",
        "rows": [{"sections": [
            {"kind": "group", "id": "vocals", "title": "VOCALS", "tags": "VOCALS"},
            {"kind": "group", "id": "bt", "controls": [
                {"kind": "param_fader", "label": "Podklady All", "link": {"instance": "band", "id": "PODKLADY"}}]}]}]}]}))
    .expect("the frame parses")
}

/// Podklady All in a composed layout.
fn podklady(layout: &Layout) -> &Control {
    group(layout, "bt").controls.first().expect("Podklady All")
}

/// A track's volume by index, as the markers fill a linked fader.
fn volume_at(index: u32) -> ParamTarget {
    ParamTarget {
        binding: Binding {
            instance: "band".into(),
            anchor: Anchor::TrackAt { index },
            path: Some("mixer_device volume".into()),
        },
        prop: "value".into(),
        on: None,
        off: None,
        scale: Some(Scale::CcLinear),
    }
}

#[test]
fn the_frames_linked_fader_takes_the_fader_tags_tracks_of_its_set() {
    let composed = compose(
        &podklady_frame(),
        &[
            found("band", TrackKind::Track, 11, "+FADER:PODKLADY"),
            found(
                "band",
                TrackKind::Track,
                10,
                r#""Drums" +G:VOCALS +FADER:PODKLADY"#,
            ),
            found("band", TrackKind::Track, 12, "+MUTE:PODKLADY"),
            found("master", TrackKind::Track, 2, "+FADER:PODKLADY"),
        ],
    );
    let Control::ParamFader { targets, link, .. } = podklady(&composed.layout) else {
        panic!("no param_fader")
    };
    assert_eq!(targets, &vec![volume_at(10), volume_at(11)]);
    assert_eq!(
        link,
        &Some(LinkId {
            instance: "band".into(),
            id: "PODKLADY".into()
        })
    );
    assert_eq!(composed.layout.validate(), vec![]);
    assert_eq!(
        strip_in(&composed.layout, "vocals", "Drums").fader_follows,
        vec![FaderFollow {
            id: "PODKLADY".into(),
            leader: "Podklady All".into(),
            fixed: true
        }]
    );
    assert_eq!(
        composed.links,
        vec![LinkReport {
            instance: "band".into(),
            id: "PODKLADY".into(),
            leader: Leader::Frame,
            partners: 2
        }]
    );
    let problems: Vec<(&str, u32, Vec<TagProblem>)> = composed
        .problems
        .iter()
        .map(|r| (r.instance.as_str(), r.index, r.problems.clone()))
        .collect();
    assert_eq!(
        problems,
        vec![
            // No strip of its set carries the id: the frame's fader has no mute.
            (
                "band",
                12,
                vec![TagProblem::OrphanMute {
                    id: "PODKLADY".into()
                }]
            ),
            (
                "master",
                2,
                vec![TagProblem::OrphanFader {
                    id: "PODKLADY".into()
                }]
            ),
        ]
    );
}

#[test]
fn a_linked_fader_in_a_pager_on_the_rail_or_global_is_filled_too() {
    // fill_links walks every place a frame control can be: a pager's
    // sub-pages, the rail and the global controls.
    let fader = |id: &str| json!({"kind": "param_fader", "label": id, "link": {"instance": "band", "id": id}});
    let frame: Layout = serde_json::from_value(json!({"schema": 2, "default_page": "foh",
        "pages": [{"id": "foh", "title": "FOH", "rail": [fader("RAIL")], "rows": [{"sections": [
            {"kind": "pager", "id": "pg", "default_page": "a", "pages": [
                {"id": "a", "title": "A", "sections": [{"kind": "group", "id": "ga", "controls": [fader("PAGER")]}]}]}]}]}],
        "global": [fader("GLOBAL")]}))
    .expect("the frame parses");
    let composed = compose(
        &frame,
        &[
            found("band", TrackKind::Track, 1, "+FADER:RAIL"),
            found("band", TrackKind::Track, 2, "+FADER:PAGER"),
            found("band", TrackKind::Track, 3, "+FADER:GLOBAL"),
        ],
    );
    let filled: Vec<(String, Vec<ParamTarget>)> = composed
        .layout
        .controls()
        .into_iter()
        .filter_map(|c| match c {
            Control::ParamFader { label, targets, .. } => Some((label.clone(), targets.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        filled,
        vec![
            ("RAIL".to_string(), vec![volume_at(1)]),
            ("PAGER".to_string(), vec![volume_at(2)]),
            ("GLOBAL".to_string(), vec![volume_at(3)]),
        ]
    );
    assert_eq!(composed.problems, vec![]);
}

#[test]
fn a_linked_fader_with_no_partner_or_a_taken_id_has_no_targets() {
    let alone = compose(&podklady_frame(), &[]);
    assert!(
        matches!(podklady(&alone.layout), Control::ParamFader { targets, .. } if targets.is_empty())
    );
    assert_eq!(
        alone.layout.validate(),
        vec![],
        "a linked fader may wait for its tracks"
    );
    assert_eq!(
        alone.links,
        vec![LinkReport {
            instance: "band".into(),
            id: "PODKLADY".into(),
            leader: Leader::Frame,
            partners: 0
        }]
    );
    let taken = compose(
        &podklady_frame(),
        &[
            found(
                "band",
                TrackKind::Track,
                3,
                r#""Pad" +G:VOCALS +ID:PODKLADY"#,
            ),
            found("band", TrackKind::Track, 10, "+FADER:PODKLADY"),
            found("band", TrackKind::Track, 12, "+MUTE:PODKLADY"),
        ],
    );
    assert!(
        matches!(podklady(&taken.layout), Control::ParamFader { targets, .. } if targets.is_empty())
    );
    let pad = strip_in(&taken.layout, "vocals", "Pad");
    assert_eq!(
        (pad.fader_link.clone(), pad.mute_links.clone()),
        (None, vec![])
    );
    let problems: Vec<(u32, Vec<TagProblem>)> = taken
        .problems
        .iter()
        .map(|r| (r.index, r.problems.clone()))
        .collect();
    assert_eq!(
        problems,
        vec![(
            3,
            vec![TagProblem::DuplicateId {
                id: "PODKLADY".into()
            }]
        )]
    );
}

#[test]
fn a_frame_holds_no_fader_link_and_names_a_linked_faders_tracks_by_its_link_only() {
    let frame = |controls: Vec<serde_json::Value>| -> Layout {
        serde_json::from_value(
            json!({"schema": 2, "default_page": "p", "pages": [{"id": "p", "title": "P",
            "rows": [{"sections": [{"kind": "group", "id": "g", "controls": controls}]}]}]}),
        )
        .unwrap()
    };
    let strip = |field: &str, value: serde_json::Value| {
        let mut s = json!({"kind": "strip", "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Rev #"}}, "strip_kind": "standard"});
        s[field] = value;
        s
    };
    let messages = |f: &Layout| {
        frame_problems(f)
            .into_iter()
            .map(|e| e.message)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        messages(&frame(vec![strip(
            "fader_link",
            json!({"id": "REV", "partners": []})
        )])),
        ["a fader link: the Tuner markers make those"]
    );
    assert_eq!(
        messages(&frame(vec![strip(
            "fader_follows",
            json!([{"id": "REV", "leader": "Rev PA"}])
        )])),
        ["a fader link: the Tuner markers make those"]
    );
    let linked = json!({"kind": "param_fader", "label": "Podklady All", "link": {"instance": "band", "id": "PODKLADY"}});
    assert_eq!(messages(&frame(vec![linked.clone()])), Vec::<String>::new());
    let mut both = linked.clone();
    both["targets"] = json!([{"binding": {"instance": "band", "anchor": {"kind": "track", "name": "Drums #"}, "path": "mixer_device volume"}, "prop": "value", "scale": "cc_linear"}]);
    assert_eq!(
        messages(&frame(vec![both])),
        ["a linked fader's tracks: the Tuner markers make those"]
    );
    // One link on two faders (review M4): at the second one, by its label,
    // on a page or among the global controls.
    let mut again = linked.clone();
    again["label"] = json!("Podklady 2");
    let errors = |f: &Layout| {
        frame_problems(f)
            .into_iter()
            .map(|e| (e.at, e.message))
            .collect::<Vec<_>>()
    };
    let twice = "the fader link band/PODKLADY is named twice: again by \"Podklady 2\"";
    assert_eq!(
        errors(&frame(vec![linked.clone(), again.clone()])),
        [("pages[0]".to_string(), twice.to_string())]
    );
    let mut global = frame(vec![linked]);
    global.global = vec![serde_json::from_value(again).unwrap()];
    assert_eq!(
        errors(&global),
        [("global[0]".to_string(), twice.to_string())]
    );
}

#[test]
fn a_reverb_strip_without_a_label_of_its_own_is_no_reverb_fader() {
    // #130 review I1: a placeholder label names another track after a
    // track-list change, so the REVERB rule could not know the fader by it.
    // The strip keeps its label problem, its only mark.
    let composed = compose(
        &frame(),
        &[
            found("band", TrackKind::Track, 4, r#""Rev ST" +G:VOCALS +REVERB"#),
            found("band", TrackKind::Track, 9, "+G:VOCALS +REVERB"),
            found("band", TrackKind::Track, 10, r#""" +G:VOCALS +REVERB"#),
            found("band", TrackKind::Track, 11, r#""Rev +G:VOCALS +REVERB"#),
        ],
    );
    for label in ["#10", "#11", "#12"] {
        let strip = strip_in(&composed.layout, "vocals", label);
        assert!(!strip.reverb, "{label}");
        assert_eq!(strip.mark, Some(StripMark::Problem), "{label}");
    }
    assert!(strip_in(&composed.layout, "vocals", "Rev ST").reverb);
    let faders: Vec<String> = composed
        .layout
        .reverb_faders()
        .into_iter()
        .map(|f| f.label)
        .collect();
    assert_eq!(faders, ["Rev ST"]);
    let problems: Vec<(u32, Vec<TagProblem>)> = composed
        .problems
        .iter()
        .map(|r| (r.index, r.problems.clone()))
        .collect();
    assert_eq!(
        problems,
        vec![
            (9, vec![TagProblem::NoLabel]),
            (10, vec![TagProblem::EmptyLabel]),
            (
                11,
                vec![
                    TagProblem::UnclosedLabel,
                    TagProblem::StrayText { text: "Rev".into() }
                ]
            ),
        ]
    );
}

#[test]
fn a_leader_shown_nowhere_leads_nothing() {
    // #130 review M1: the hub keeps a switch only for a strip of the served
    // layout (`Layout::fader_leaders`), so a leader without `+G:` (its own
    // `NoGroup`) gives no follow and is in no link.
    let composed = compose(
        &frame(),
        &[
            found("band", TrackKind::Track, 3, r#""Rev PA" +ID:REV"#),
            found(
                "band",
                TrackKind::Track,
                2,
                r#""Rev ST" +G:VOCALS +FADER:REV"#,
            ),
        ],
    );
    assert_eq!(
        strip_in(&composed.layout, "vocals", "Rev ST").fader_follows,
        vec![]
    );
    assert_eq!(composed.links, vec![]);
    assert_eq!(composed.layout.fader_leaders(), vec![]);
    let problems: Vec<(u32, Vec<TagProblem>)> = composed
        .problems
        .iter()
        .map(|r| (r.index, r.problems.clone()))
        .collect();
    assert_eq!(problems, vec![(3, vec![TagProblem::NoGroup])]);
}

#[test]
fn a_strip_may_lead_and_follow_and_a_tuner_may_follow_two_leaders() {
    // #130's C table: a strip that leads and follows carries both; a partner
    // of two linked leaders is a partner of each (two follows on its strip).
    let composed = compose(
        &frame(),
        &[
            found("band", TrackKind::Track, 1, r#""A" +G:VOCALS +ID:A"#),
            found(
                "band",
                TrackKind::Track,
                2,
                r#""B" +G:VOCALS +ID:B +FADER:A"#,
            ),
            found(
                "band",
                TrackKind::Track,
                3,
                r#""C" +G:VOCALS +FADER:A +FADER:B"#,
            ),
            found("band", TrackKind::Track, 4, "+FADER:A +FADER:B"),
        ],
    );
    assert_eq!(composed.problems, vec![]);
    let partner = |index: u32, label: &str| LinkPartner {
        binding: at("band", Anchor::TrackAt { index }),
        label: label.into(),
    };
    let follow = |id: &str| FaderFollow {
        id: id.into(),
        leader: id.into(),
        fixed: false,
    };
    let a = strip_in(&composed.layout, "vocals", "A");
    assert_eq!(
        a.fader_link,
        Some(FaderLink {
            id: "A".into(),
            partners: vec![partner(2, "B"), partner(3, "C"), partner(4, "#5")]
        })
    );
    assert_eq!(a.fader_follows, vec![]);
    let b = strip_in(&composed.layout, "vocals", "B");
    assert_eq!(
        b.fader_link,
        Some(FaderLink {
            id: "B".into(),
            partners: vec![partner(3, "C"), partner(4, "#5")]
        }),
        "it leads"
    );
    assert_eq!(b.fader_follows, vec![follow("A")], "and it follows");
    let c = strip_in(&composed.layout, "vocals", "C");
    assert_eq!(c.fader_link, None);
    assert_eq!(c.fader_follows, vec![follow("A"), follow("B")]);
    let report = |id: &str, partners| LinkReport {
        instance: "band".into(),
        id: id.into(),
        leader: Leader::Strip,
        partners,
    };
    assert_eq!(composed.links, vec![report("A", 3), report("B", 2)]);
}

/// The frame with an autotune control (#104) on the FOH rail, in a pager's
/// sub-page, in a row's group and among the global controls.
fn autotune_frame() -> Layout {
    let autotune = json!({"kind": "autotune", "label": "AUTOTUNE"});
    serde_json::from_value(json!({
        "schema": 2,
        "default_page": "foh",
        "pages": [{"id": "foh", "title": "FOH", "rail": [autotune.clone()], "rows": [
            {"sections": [
                {"kind": "group", "id": "vocals", "title": "VOCALS", "tags": "VOCALS"},
                {"kind": "group", "id": "fx", "controls": [autotune.clone()]}]},
            {"sections": [{"kind": "pager", "id": "pager", "default_page": "one", "pages": [
                {"id": "one", "title": "ONE", "sections": [
                    {"kind": "group", "id": "deep", "controls": [autotune.clone()]}]}]}]}]}],
        "global": [autotune]
    }))
    .expect("the frame parses")
}

/// The autotune targets of every autotune control of `layout`, in
/// document order.
fn autotune_controls(layout: &Layout) -> Vec<Vec<AutotuneTarget>> {
    layout
        .controls()
        .into_iter()
        .filter_map(|c| match c {
            Control::Autotune { targets, .. } => Some(targets.clone()),
            _ => None,
        })
        .collect()
}

fn autotune_at(
    anchor: Anchor,
    label: &str,
    (on, off): (u8, u8),
    singer: Option<&str>,
) -> AutotuneTarget {
    AutotuneTarget {
        binding: at("band", anchor),
        label: label.into(),
        on,
        off,
        singer: singer.map(str::to_string),
        two_singers: false,
    }
}

#[test]
fn compose_fills_every_autotune_control_with_the_tagged_tracks_in_lives_order() {
    let tagged = [
        found("band", TrackKind::Track, 9, "+MUTE:V1 +AUTOTUNE"),
        found("band", TrackKind::Return, 0, "+AUTOTUNE"),
        found(
            "band",
            TrackKind::Track,
            3,
            r#""Vox 1" +G:VOCALS +ID:V1 +AUTOTUNE:0"#,
        ),
        found("band", TrackKind::Track, 5, r#""Hand" +G:TALKSHOW"#),
    ];
    let composed = compose(&autotune_frame(), &tagged);
    // Track 3's `+ID` and track 9's `+MUTE` name one singer (#140 PR 3b).
    let want = vec![
        autotune_at(Anchor::TrackAt { index: 3 }, "Vox 1", (0, 127), Some("V1")),
        autotune_at(Anchor::TrackAt { index: 9 }, "#10", (127, 0), Some("V1")),
        autotune_at(Anchor::ReturnAt { index: 0 }, "#1", (127, 0), None),
    ];
    let pages: Vec<&str> = composed
        .layout
        .pages
        .iter()
        .map(|p| p.id.as_str())
        .collect();
    assert_eq!(pages, ["foh", "view-TALKSHOW"]);
    // The FOH rail, its row's group, the pager's group, the view's copied
    // rail and the global control.
    assert_eq!(autotune_controls(&composed.layout), vec![want.clone(); 5]);
    assert_eq!(composed.layout.autotune_targets(), want);
    assert_eq!(composed.layout.validate(), vec![]);
    assert_eq!(composed.problems, vec![]);
    // The strips are as without the tags: track 3's, linked to track 9;
    // none for track 9.
    let untagged = compose(
        &autotune_frame(),
        &[
            found("band", TrackKind::Track, 9, "+MUTE:V1"),
            found("band", TrackKind::Track, 3, r#""Vox 1" +G:VOCALS +ID:V1"#),
            found("band", TrackKind::Track, 5, r#""Hand" +G:TALKSHOW"#),
        ],
    );
    assert_eq!(
        group(&composed.layout, "vocals"),
        group(&untagged.layout, "vocals")
    );
    assert_eq!(
        labels(&group(&composed.layout, "vocals").controls),
        ["Vox 1"]
    );
    assert_eq!(
        strip_in(&composed.layout, "vocals", "Vox 1").mute_links,
        vec![at("band", Anchor::TrackAt { index: 9 })]
    );
}

/// The autotune targets `compose` makes of these band tracks' Tuners (each
/// `(index, name)`), and every marker problem.
fn autotune_of(tuners: &[(u32, &str)]) -> (Vec<AutotuneTarget>, Vec<MarkerReport>) {
    let tracks: Vec<Found> = tuners
        .iter()
        .map(|(index, name)| found("band", TrackKind::Track, *index, name))
        .collect();
    let composed = compose(&autotune_frame(), &tracks);
    (composed.layout.autotune_targets(), composed.problems)
}

#[test]
fn a_pa_tuner_names_its_singer_by_its_id() {
    // #140 PR 3b, spec §7: a PA strip's `+ID` is its singer.
    let (targets, _) = autotune_of(&[(1, "+G:STAGE:2 +ID:VOX1 +AUTOTUNE:0")]);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].singer.as_deref(), Some("VOX1"));
    assert!(!targets[0].two_singers);
    assert_eq!(
        crate::layout::singer_of(&targets[0]).as_deref(),
        Some("VOX1")
    );
    // Its own `+ID` comes first, also beside two `+MUTE` ids.
    let m = parse(r#""Vox 1" +G:VOCALS +ID:VOX1 +MUTE:A +MUTE:B +AUTOTUNE:0"#);
    assert_eq!(singer(&m), Some("VOX1".to_string()));
    assert!(!names_two_singers(&m));
    assert!(!m.problems.contains(&TagProblem::TwoSingers));
}

#[test]
fn a_stream_tuner_names_its_singer_by_its_one_mute() {
    // #127's pair: the PA strip `+ID:VOX1` and the stream track's Tuner
    // `+MUTE:VOX1 +AUTOTUNE` are one singer.
    let (targets, _) = autotune_of(&[
        (3, r#""Vox 1" +G:VOCALS +ID:VOX1 +AUTOTUNE:0"#),
        (9, "+MUTE:VOX1 +AUTOTUNE"),
    ]);
    let singers: Vec<Option<&str>> = targets.iter().map(|t| t.singer.as_deref()).collect();
    assert_eq!(singers, [Some("VOX1"), Some("VOX1")]);
    assert!(targets.iter().all(|t| !t.two_singers));
    let m = parse("+MUTE:VOX1 +AUTOTUNE");
    assert_eq!(singer(&m), Some("VOX1".to_string()));
    assert!(!names_two_singers(&m));
}

#[test]
fn two_mutes_on_an_autotune_tuner_are_a_problem() {
    // Spec §7: which singer it is is not known, so no autotune for it.
    assert_eq!(
        parse("+MUTE:A +MUTE:B +AUTOTUNE").problems,
        vec![TagProblem::TwoSingers]
    );
    let m = parse("+MUTE:A +MUTE:B +MUTE:C +AUTOTUNE");
    assert!(names_two_singers(&m));
    assert_eq!(singer(&m), None);
    // Without `+AUTOTUNE` two mutes are #127's links, no problem.
    assert_eq!(parse("+MUTE:A +MUTE:B").problems, vec![]);
    assert!(!names_two_singers(&parse("+MUTE:A +MUTE:B")));
    // One mute is a singer.
    assert!(!names_two_singers(&parse("+MUTE:A +AUTOTUNE")));
    let (targets, problems) = autotune_of(&[
        (3, r#""A" +G:VOCALS +ID:A"#),
        (4, r#""B" +G:VOCALS +ID:B"#),
        (9, "+MUTE:A +MUTE:B +AUTOTUNE"),
    ]);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].singer, None);
    assert!(targets[0].two_singers);
    let reported: Vec<&MarkerReport> = problems.iter().filter(|r| r.index == 9).collect();
    assert_eq!(reported.len(), 1);
    assert_eq!(reported[0].problems, vec![TagProblem::TwoSingers]);
    assert_eq!(
        serde_json::to_value(TagProblem::TwoSingers).unwrap(),
        json!({"code": "two_singers"})
    );
    // The target says so on the wire; a plain one carries neither field.
    assert_eq!(
        serde_json::to_value(&targets[0]).unwrap(),
        json!({"binding": {"instance": "band", "anchor": {"kind": "track_at", "index": 9}},
               "label": "#10", "on": 127, "off": 0, "two_singers": true})
    );
}

#[test]
fn a_tagged_track_without_ids_is_a_singer_of_its_own() {
    let (targets, _) = autotune_of(&[(6, r#""Vox 7" +G:VOCALS +AUTOTUNE"#)]);
    assert_eq!(targets[0].singer, None);
    assert!(!targets[0].two_singers);
    assert_eq!(
        crate::layout::singer_of(&targets[0]).as_deref(),
        Some("#Vox 7")
    );
    assert_eq!(singer(&parse("+AUTOTUNE")), None);
    let mut named = targets[0].clone();
    named.singer = Some("VOX7".into());
    assert_eq!(crate::layout::singer_of(&named).as_deref(), Some("VOX7"));
    assert_eq!(
        serde_json::to_value(&named).unwrap()["singer"],
        json!("VOX7")
    );
    // A target from before #140 PR 3b (no singer fields) still reads.
    let older: AutotuneTarget = serde_json::from_value(json!({
        "binding": {"instance": "band", "anchor": {"kind": "track_at", "index": 6}},
        "label": "Vox 7", "on": 127, "off": 0}))
    .unwrap();
    assert_eq!(older, targets[0]);
}

#[test]
fn a_placeholder_label_names_no_singer() {
    // PR #147 review N6: `#<index + 1>` follows the track's index, never a
    // singer, so a Tuner with no id and no label of its own names none.
    let (targets, _) = autotune_of(&[(6, "+AUTOTUNE")]);
    assert_eq!(targets[0].label, "#7");
    assert_eq!(targets[0].singer, None);
    assert_eq!(crate::layout::singer_of(&targets[0]), None);
    // An id still names one, a placeholder label or not.
    let mut paired = targets[0].clone();
    paired.singer = Some("VOX1".into());
    assert_eq!(crate::layout::singer_of(&paired).as_deref(), Some("VOX1"));
}

#[test]
fn a_frame_autotune_control_names_no_tracks() {
    let target = json!({"binding": {"instance": "band", "anchor": {"kind": "track_at", "index": 1}},
                        "label": "#2", "on": 127, "off": 0});
    let frame: Layout = serde_json::from_value(json!({"schema": 2, "default_page": "p",
        "pages": [{"id": "p", "title": "P",
                   "rail": [{"kind": "autotune", "targets": [target.clone()]}]},
                  {"id": "q", "title": "Q", "rail": [{"kind": "autotune"}]}],
        "global": [{"kind": "autotune"}, {"kind": "autotune", "targets": [target]}]}))
    .unwrap();
    let problems: Vec<(String, String)> = frame_problems(&frame)
        .into_iter()
        .map(|e| (e.at, e.message))
        .collect();
    let message = "the autotune tracks: the Tuner markers make those".to_string();
    assert_eq!(
        problems,
        vec![
            ("pages[0]".to_string(), message.clone()),
            ("global[1]".to_string(), message)
        ]
    );
    assert_eq!(frame_problems(&autotune_frame()), vec![]);
}

#[test]
fn autotune_tags_without_a_control_fill_nothing() {
    let tagged = [
        found("band", TrackKind::Track, 9, "+MUTE:V1 +AUTOTUNE"),
        found(
            "band",
            TrackKind::Track,
            3,
            r#""Vox 1" +G:VOCALS +ID:V1 +AUTOTUNE:0"#,
        ),
    ];
    let composed = compose(&frame(), &tagged);
    assert_eq!(composed.problems, vec![]);
    assert!(composed.layout.autotune_targets().is_empty());
    assert!(autotune_controls(&composed.layout).is_empty());
    let untagged = compose(
        &frame(),
        &[
            found("band", TrackKind::Track, 9, "+MUTE:V1"),
            found("band", TrackKind::Track, 3, r#""Vox 1" +G:VOCALS +ID:V1"#),
        ],
    );
    assert_eq!(composed.layout, untagged.layout);
    // An autotune control and no tag: no targets.
    let composed = compose(&autotune_frame(), &[]);
    assert_eq!(autotune_controls(&composed.layout), vec![Vec::new(); 4]);
}
