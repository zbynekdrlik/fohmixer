use super::*;
use fohmixer_proto::layout::{Anchor, StripKind};
use serde_json::json;

fn track(name: &str, path: Option<&str>) -> Binding {
    Binding {
        instance: "band".into(),
        anchor: Anchor::Track { name: name.into() },
        path: path.map(str::to_string),
    }
}

#[test]
fn targets_follow_the_anchor_and_the_path() {
    assert_eq!(
        target_of(&track("Klavir #", None), "").as_deref(),
        Some("live_set tracks[name=Klavir #]")
    );
    assert_eq!(
        target_of(&track("Klavir #", None), "mixer_device volume").as_deref(),
        Some("live_set tracks[name=Klavir #] mixer_device volume")
    );
    let eq = track("Klavir #", Some("devices[name=EQ Eight] parameters 1"));
    assert_eq!(
        target_of(&eq, "").as_deref(),
        Some("live_set tracks[name=Klavir #] devices[name=EQ Eight] parameters 1")
    );
    let ret = Binding {
        instance: "band".into(),
        anchor: Anchor::Return {
            name: "A-Reverb #".into(),
        },
        path: None,
    };
    assert_eq!(
        target_of(&ret, "mixer_device panning").as_deref(),
        Some("live_set return_tracks[name=A-Reverb #] mixer_device panning")
    );
    let master = Binding {
        instance: "master".into(),
        anchor: Anchor::Master,
        path: Some("mixer_device volume".into()),
    };
    assert_eq!(
        target_of(&master, "").as_deref(),
        Some("live_set master_track mixer_device volume")
    );
    let song = Binding {
        instance: "band".into(),
        anchor: Anchor::Song,
        path: None,
    };
    assert_eq!(target_of(&song, "").as_deref(), Some("live_set"));
    // A `]` inside a name is escaped; a path that does not parse has none.
    assert_eq!(
        target_of(&track("a]b", None), "mute").as_deref(),
        Some(r"live_set tracks[name=a\]b] mute")
    );
    assert_eq!(target_of(&track("x", Some("devices[name=")), ""), None);
}

#[test]
fn a_subscription_key_is_the_hubs() {
    let spec = SubSpec::new(
        "band",
        "live_set tracks[name=Hand2 #] mixer_device volume".into(),
        "value",
        true,
    );
    assert_eq!(
        spec.key(),
        "band|live_set tracks[name=Hand2 #] mixer_device volume|value|true"
    );
    let plain = SubSpec::new("master", "live_set".into(), "is_playing", false);
    assert_eq!(plain.key(), "master|live_set|is_playing|false");
}

fn sample() -> Layout {
    let tr = |name: &str| json!({"instance": "band", "anchor": {"kind": "track", "name": name}});
    serde_json::from_value(json!({
        "schema": 2,
        "default_page": "foh",
        "pages": [
            {"id": "cue", "title": "Cue", "rows": [{"sections": [{"kind": "group", "controls": [
                {"kind": "param_toggle", "label": "Vox 1 TU", "press": "toggle",
                 "targets": [{"binding": tr("Vox 1 stream"), "prop": "mute", "on": false, "off": true}]}]}]}]},
            {"id": "foh", "title": "FOH",
             "rail": [
                {"kind": "stage", "binding": tr("Mics #"), "aut": true},
                {"kind": "solo", "binding": tr("Vocals grp")},
                {"kind": "hub_toggle", "key": "stage_aut", "label": "STAGE AUT"}],
             "rows": [
                {"sections": [
                    {"kind": "pager", "id": "foh-pager", "default_page": "others", "pages": [
                        {"id": "stage", "title": "STAGE", "sections": [{"kind": "group", "controls": [
                            {"kind": "strip", "binding": tr("A"), "strip_kind": "standard"}]}]},
                        {"id": "others", "title": "OTHERS", "sections": [{"kind": "group", "controls": [
                            {"kind": "strip", "binding": tr("B"), "strip_kind": "standard"}]}]}]},
                    {"kind": "group", "controls": [
                        {"kind": "strip", "binding": tr("C"), "strip_kind": "standard"},
                        {"kind": "strip", "binding": tr("A"), "strip_kind": "standard"}]}]}]},
            {"id": "conf", "title": "Conf", "rows": [{"sections": [{"kind": "group", "controls": [
                {"kind": "text", "text": "unfold_band: 'Vocals grp'"}]}]}]}
        ],
        "global": [
            {"kind": "alert", "binding": tr("TechAlert #"), "period_ms": 300}
        ]
    }))
    .expect("the sample parses")
}

fn strip(name: &str) -> Strip {
    Strip {
        binding: track(name, None),
        strip_kind: StripKind::Standard,
        wide: false,
        mute_guard: false,
        pinned: false,
        label: None,
        mark: None,
    }
}

fn keys(specs: &[SubSpec]) -> Vec<String> {
    specs.iter().map(SubSpec::key).collect()
}

#[test]
fn a_strip_subscribes_every_part_and_its_colour() {
    let subs = strip_subs(&strip("Hand2 #"), MeterSource::Level);
    let t = "live_set tracks[name=Hand2 #]";
    assert_eq!(
        keys(&subs.all()),
        vec![
            format!("band|{t} mixer_device volume|value|true"),
            format!("band|{t} mixer_device panning|value|false"),
            format!("band|{t}|mute|false"),
            format!("band|{t}|output_meter_level|false"),
            format!("band|{t}|color|false"),
        ]
    );
    assert_eq!(
        subs.color.as_ref().map(SubSpec::key),
        Some(format!("band|{t}|color|false"))
    );
    // Two bars with `lr`.
    let lr = strip_subs(&strip("Hand2 #"), MeterSource::Lr);
    assert_eq!(
        keys(&lr.meters),
        vec![
            format!("band|{t}|output_meter_left|false"),
            format!("band|{t}|output_meter_right|false"),
        ]
    );
    assert_eq!(lr.all().len(), 6);
    // A binding whose path does not parse subscribes nothing.
    let mut bad = strip("X");
    bad.binding.path = Some("devices[name=".into());
    assert_eq!(strip_subs(&bad, MeterSource::Level).all(), vec![]);
    assert_eq!(StripSubs::default().all(), vec![]);
}

#[test]
fn every_control_kind_subscribes_what_it_shows() {
    let src = MeterSource::Level;
    let sub = |c: &Control| keys(&control_subs(c, src));
    let t = |name: &str| format!("live_set tracks[name={name}]");
    assert_eq!(
        sub(&Control::Solo {
            binding: track("G", None),
            label: None
        }),
        vec![format!("band|{}|solo|false", t("G"))]
    );
    assert_eq!(
        sub(&Control::Stage {
            binding: track("M", None),
            aut: true,
            label: None
        }),
        vec![format!("band|{}|mute|false", t("M"))]
    );
    assert_eq!(
        sub(&Control::Alert {
            binding: track("T", None),
            period_ms: 300,
            label: None,
            mute_guard: false
        }),
        vec![format!("band|{}|mute|false", t("T"))]
    );
    let target = |name: &str, path: Option<&str>, prop: &str| fohmixer_proto::layout::ParamTarget {
        binding: track(name, path),
        prop: prop.into(),
        on: Some(json!(1)),
        off: Some(json!(0)),
        scale: None,
    };
    let targets = vec![
        target("P", Some("mixer_device volume"), "value"),
        target("Q", Some("devices["), "value"),
        target("R", None, "mute"),
    ];
    assert_eq!(
        sub(&Control::ParamToggle {
            label: "x".into(),
            targets: targets.clone(),
            press: fohmixer_proto::layout::Press::Toggle,
            color: None
        }),
        vec![
            format!("band|{} mixer_device volume|value|false", t("P")),
            format!("band|{}|mute|false", t("R")),
        ]
    );
    assert_eq!(
        sub(&Control::ParamFader {
            label: "x".into(),
            targets
        }),
        vec![
            format!("band|{} mixer_device volume|value|true", t("P")),
            format!("band|{}|mute|false", t("R")),
        ]
    );
    assert_eq!(
        sub(&Control::Strip(Box::new(strip("S")))).len(),
        5,
        "a strip: volume, pan, mute, one meter, colour"
    );
    for none in [
        Control::HubToggle {
            key: "stage_aut".into(),
            label: "A".into(),
        },
        Control::Text { text: "x".into() },
    ] {
        assert_eq!(sub(&none), Vec::<String>::new());
    }
}

/// The names of the strips and the kinds of the other controls on screen.
fn shown(layout: &Layout, path: &[usize]) -> Vec<String> {
    visible_controls(layout, path)
        .into_iter()
        .map(|c| match c {
            Control::Strip(s) => match &s.binding.anchor {
                Anchor::Track { name } => name.clone(),
                _ => "strip".into(),
            },
            Control::Solo { .. } => "solo".into(),
            Control::Stage { .. } => "stage".into(),
            Control::HubToggle { .. } => "hub".into(),
            Control::ParamToggle { label, .. } => label.clone(),
            Control::ParamFader { .. } => "fader".into(),
            Control::Alert { .. } => "alert".into(),
            Control::Text { .. } => "text".into(),
        })
        .collect()
}

#[test]
fn the_controls_on_screen_are_the_rail_the_rows_with_the_sub_page_and_the_global_ones() {
    let layout = sample();
    assert_eq!(
        shown(&layout, &[1, 0]),
        vec!["stage", "solo", "hub", "A", "C", "A", "alert"]
    );
    assert_eq!(
        shown(&layout, &[1, 1]),
        vec!["stage", "solo", "hub", "B", "C", "A", "alert"]
    );
    // No sub-page chosen (or one that is gone): the pager shows nothing.
    assert_eq!(
        shown(&layout, &[1]),
        vec!["stage", "solo", "hub", "C", "A", "alert"]
    );
    assert_eq!(shown(&layout, &[1, 7]), shown(&layout, &[1]));
    assert_eq!(shown(&layout, &[0]), vec!["Vox 1 TU", "alert"]);
    assert_eq!(shown(&layout, &[2]), vec!["text", "alert"]);
    // No page: the global controls only.
    assert_eq!(shown(&layout, &[]), vec!["alert"]);
    assert_eq!(shown(&layout, &[9]), vec!["alert"]);
}

#[test]
fn a_pinned_strip_of_another_sub_page_stays_on_screen() {
    // #63: STAGE's strip A pinned; OTHERS shown: A stays, in the pager's
    // place, before the shown sub-page's strips.
    let mut layout = sample();
    let Section::Pager(pager) = &mut layout.pages[1].rows[0].sections[0] else {
        panic!("the pager")
    };
    let Section::Group(group) = &mut pager.pages[0].sections[0] else {
        panic!("STAGE's group")
    };
    let Control::Strip(a) = &mut group.controls[0] else {
        panic!("the strip A")
    };
    a.pinned = true;
    assert_eq!(
        shown(&layout, &[1, 1]),
        vec!["stage", "solo", "hub", "A", "B", "C", "A", "alert"]
    );
    // On its own sub-page, once.
    assert_eq!(
        shown(&layout, &[1, 0]),
        vec!["stage", "solo", "hub", "A", "C", "A", "alert"]
    );
    // No sub-page chosen: the pinned strip still shows.
    assert_eq!(
        shown(&layout, &[1]),
        vec!["stage", "solo", "hub", "A", "C", "A", "alert"]
    );
}

#[test]
fn every_key_on_screen_is_subscribed_once() {
    let layout = sample();
    let subs = visible_subs(&layout, &[1, 0]);
    // stage + solo, strips A and C (A twice on screen), TechAlert.
    assert_eq!(subs.len(), 2 + 5 + 5 + 1);
    let unique: std::collections::BTreeSet<String> = subs.iter().map(SubSpec::key).collect();
    assert_eq!(unique.len(), subs.len());
    assert!(
        subs.iter()
            .any(|s| s.target == "live_set tracks[name=C]" && s.prop == "color")
    );
    assert!(!subs.iter().any(|s| s.target.contains("name=B]")));
    // The other sub-page: B instead of A (A stays, the fixed group holds it).
    let others = visible_subs(&layout, &[1, 1]);
    assert_eq!(others.len(), 2 + 5 + 5 + 5 + 1);
    // The cue page: its toggle's target and TechAlert.
    assert_eq!(visible_subs(&layout, &[0]).len(), 2);
}

#[test]
fn the_pills_solos_are_every_solo_of_the_page() {
    let layout = sample();
    let names: Vec<String> = page_solos(&layout.pages[1])
        .iter()
        .map(|b| b.target().unwrap())
        .collect();
    assert_eq!(names, vec!["live_set tracks[name=Vocals grp]"]);
    assert_eq!(page_solos(&layout.pages[0]), Vec::<Binding>::new());
}

#[test]
fn the_meter_source_switch_applies_to_every_strip() {
    let mut layout = sample();
    let level = visible_subs(&layout, &[1, 0]);
    layout.config.meter_source = Some(MeterSource::Lr);
    let lr = visible_subs(&layout, &[1, 0]);
    assert_eq!(lr.len(), level.len() + 2, "one more bar per strip (A, C)");
    assert!(lr.iter().any(|s| s.prop == "output_meter_right"));
    assert!(!lr.iter().any(|s| s.prop == "output_meter_level"));
}

#[test]
fn the_selected_pages_are_the_remembered_ones_or_the_defaults() {
    let layout = sample();
    let mut remembered = BTreeMap::new();
    // The defaults: FOH and its pager's OTHERS.
    assert_eq!(selected_path(&layout, &remembered), vec![1, 1]);
    remembered.insert("foh".to_string(), "stage".to_string());
    assert_eq!(selected_path(&layout, &remembered), vec![1, 0]);
    remembered.insert(String::new(), "cue".to_string());
    assert_eq!(selected_path(&layout, &remembered), vec![0]);
    remembered.insert(String::new(), "conf".to_string());
    assert_eq!(selected_path(&layout, &remembered), vec![2]);
    // Remembered pages that are gone fall back to the defaults.
    remembered.insert(String::new(), "worship".to_string());
    remembered.insert("foh".to_string(), "band-b".to_string());
    assert_eq!(selected_path(&layout, &remembered), vec![1, 1]);
    // A default page that is not there: the first page.
    let mut odd = sample();
    odd.default_page = "gone".into();
    assert_eq!(selected_path(&odd, &BTreeMap::new()), vec![0]);
    // A pager default that is not there: its first sub-page.
    let mut odd = sample();
    if let Section::Pager(p) = &mut odd.pages[1].rows[0].sections[0] {
        p.default_page = "gone".into();
    }
    assert_eq!(selected_path(&odd, &BTreeMap::new()), vec![1, 0]);
    // A pager without sub-pages is no level; no pages, no path.
    if let Section::Pager(p) = &mut odd.pages[1].rows[0].sections[0] {
        p.pages.clear();
    }
    assert_eq!(selected_path(&odd, &BTreeMap::new()), vec![1]);
    odd.pages.clear();
    assert_eq!(selected_path(&odd, &BTreeMap::new()), Vec::<usize>::new());
}

#[test]
fn choosing_a_tab_remembers_it_for_its_pager() {
    let layout = sample();
    let mut remembered = BTreeMap::new();
    choose(&layout, &mut remembered, &[1, 1], 0, 2);
    assert_eq!(remembered.get(""), Some(&"conf".to_string()));
    choose(&layout, &mut remembered, &[1, 1], 1, 0);
    assert_eq!(remembered.get("foh"), Some(&"stage".to_string()));
    assert_eq!(selected_path(&layout, &remembered), vec![2]);
    // Out of range, or a page without a pager: nothing changes.
    let before = remembered.clone();
    choose(&layout, &mut remembered, &[1, 1], 0, 9);
    choose(&layout, &mut remembered, &[1, 1], 1, 9);
    choose(&layout, &mut remembered, &[0], 1, 0);
    choose(&layout, &mut remembered, &[], 1, 0);
    assert_eq!(remembered, before);
}

/// A layout of two pages and two views (#68).
fn with_views() -> Layout {
    serde_json::from_value(serde_json::json!({
        "schema": 2,
        "default_page": "foh",
        "pages": [
            {"id": "cue", "title": "Cue"},
            {"id": "foh", "title": "FOH"},
            {"id": "view-TALK", "title": "TALK", "view": true},
            {"id": "view-SOLO", "title": "SOLO", "view": true}
        ]
    }))
    .expect("the layout parses")
}

#[test]
fn a_views_button_shows_it_and_a_second_tap_returns_to_the_page_before() {
    let layout = with_views();
    // From the cue page: the view, the cue page remembered.
    assert_eq!(view_tap(&layout, Some(0), 2, None), (2, Some("cue".into())));
    // Again: back to the cue page.
    assert_eq!(view_tap(&layout, Some(2), 2, Some("cue")), (0, None));
    // From one view to another: the page before the first stays.
    assert_eq!(
        view_tap(&layout, Some(2), 3, Some("cue")),
        (3, Some("cue".into()))
    );
    // Nothing remembered (or a view, or a page gone): the default page.
    assert_eq!(view_tap(&layout, Some(3), 3, None), (1, None));
    assert_eq!(view_tap(&layout, Some(3), 3, Some("view-TALK")), (1, None));
    assert_eq!(view_tap(&layout, Some(3), 3, Some("gone")), (1, None));
    // Nothing shown yet: the view, nothing to remember.
    assert_eq!(view_tap(&layout, None, 2, None), (2, None));
}

#[test]
fn a_view_returns_to_the_first_page_without_a_default() {
    let mut layout = with_views();
    layout.default_page = "missing".into();
    assert_eq!(view_tap(&layout, Some(2), 2, None), (0, None));
}
