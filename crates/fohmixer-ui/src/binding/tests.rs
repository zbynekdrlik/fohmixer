use super::*;
use fohmixer_proto::layout::StripKind;
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
    // #71: the strip on the page draws no pan (the channel detail does), so
    // it subscribes every part but the pan.
    assert_eq!(
        keys(&subs.shown()),
        vec![
            format!("band|{t} mixer_device volume|value|true"),
            format!("band|{t}|mute|false"),
            format!("band|{t}|output_meter_level|false"),
            format!("band|{t}|color|false"),
        ]
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
    assert_eq!(lr.shown().len(), 5);
    // A binding whose path does not parse subscribes nothing.
    let mut bad = strip("X");
    bad.binding.path = Some("devices[name=".into());
    assert_eq!(strip_subs(&bad, MeterSource::Level).all(), vec![]);
    assert_eq!(strip_subs(&bad, MeterSource::Level).shown(), vec![]);
    assert_eq!(StripSubs::default().all(), vec![]);
    assert_eq!(StripSubs::default().shown(), vec![]);
}

#[test]
fn a_channel_detail_subscribes_every_part_its_pan_with_lives_display() {
    let t = "live_set tracks[name=Hand2 #]";
    let subs = detail_subs(&strip("Hand2 #"), MeterSource::Level);
    assert_eq!(
        keys(&subs.all()),
        vec![
            format!("band|{t} mixer_device volume|value|true"),
            format!("band|{t} mixer_device panning|value|true"),
            format!("band|{t}|mute|false"),
            format!("band|{t}|output_meter_level|false"),
            format!("band|{t}|color|false"),
        ]
    );
    // The parts are the strip's, but for the pan's display string.
    let page = strip_subs(&strip("Hand2 #"), MeterSource::Level);
    assert_eq!(subs.volume, page.volume);
    assert_eq!(subs.mute, page.mute);
    assert_eq!(subs.color, page.color);
    assert_eq!(subs.meters, page.meters);
    assert_eq!(
        subs.pan.map(|p| (p.target, p.prop, p.display)),
        Some((
            format!("{t} mixer_device panning"),
            "value".to_string(),
            true
        ))
    );
    assert_eq!(
        detail_subs(&strip("Hand2 #"), MeterSource::Lr).meters.len(),
        2
    );
    // A binding whose path does not parse: nothing.
    let mut bad = strip("X");
    bad.binding.path = Some("devices[name=".into());
    assert_eq!(detail_subs(&bad, MeterSource::Level), StripSubs::default());
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
        4,
        "a strip: volume, mute, one meter, colour (#71: its pan is the detail's)"
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
    // stage + solo, strips A and C (A twice on screen; no pan, #71),
    // TechAlert.
    assert_eq!(subs.len(), 2 + 4 + 4 + 1);
    let unique: std::collections::BTreeSet<String> = subs.iter().map(SubSpec::key).collect();
    assert_eq!(unique.len(), subs.len());
    assert!(
        subs.iter()
            .any(|s| s.target == "live_set tracks[name=C]" && s.prop == "color")
    );
    assert!(!subs.iter().any(|s| s.target.contains("name=B]")));
    // The other sub-page: B instead of A (A stays, the fixed group holds it).
    let others = visible_subs(&layout, &[1, 1]);
    assert_eq!(others.len(), 2 + 4 + 4 + 4 + 1);
    assert!(
        !others
            .iter()
            .any(|s| s.target.ends_with("mixer_device panning"))
    );
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
    assert_eq!(
        view_tap(&layout, Some(0), false, 2, None),
        (2, Some("cue".into()))
    );
    // Again: back to the cue page.
    assert_eq!(view_tap(&layout, Some(2), false, 2, Some("cue")), (0, None));
    // From one view to another: the page before the first stays.
    assert_eq!(
        view_tap(&layout, Some(2), false, 3, Some("cue")),
        (3, Some("cue".into()))
    );
    // Nothing remembered (or a view, or a page gone): the default page.
    assert_eq!(view_tap(&layout, Some(3), false, 3, None), (1, None));
    assert_eq!(
        view_tap(&layout, Some(3), false, 3, Some("view-TALK")),
        (1, None)
    );
    assert_eq!(
        view_tap(&layout, Some(3), false, 3, Some("gone")),
        (1, None)
    );
    // Nothing shown yet: the view, nothing to remember.
    assert_eq!(view_tap(&layout, None, false, 2, None), (2, None));
}

#[test]
fn a_views_button_under_the_stream_deck_tab_shows_the_view() {
    let layout = with_views();
    // The view is the page under the deck tab: a tap shows it, never back.
    assert_eq!(
        view_tap(&layout, Some(2), true, 2, Some("cue")),
        (2, Some("cue".into()))
    );
    // The cue page under it is remembered as without the deck.
    assert_eq!(
        view_tap(&layout, Some(0), true, 2, None),
        (2, Some("cue".into()))
    );
}

#[test]
fn a_view_shown_is_never_stored_the_page_before_it_is() {
    let layout = with_views();
    let pages = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    };
    let shown = pages(&[("", "view-SOLO"), ("foh", "others")]);
    assert_eq!(
        stored_pages(&layout, &shown, Some("cue")),
        pages(&[("", "cue"), ("foh", "others")])
    );
    assert_eq!(
        stored_pages(&layout, &shown, None),
        pages(&[("foh", "others")])
    );
    // A page is stored as it is.
    let page = pages(&[("", "cue")]);
    assert_eq!(stored_pages(&layout, &page, Some("foh")), page);
}

#[test]
fn a_view_returns_to_the_first_page_without_a_default() {
    let mut layout = with_views();
    layout.default_page = "missing".into();
    assert_eq!(view_tap(&layout, Some(2), false, 2, None), (0, None));
}

/// The sample's strip `name` bound by name in the band (as a detail holds
/// it).
fn held(name: &str) -> Strip {
    strip(name)
}

#[test]
fn a_detail_adds_its_pan_with_lives_display_to_a_page_that_shows_its_strip() {
    let layout = sample();
    let page = visible_subs(&layout, &[1, 0]);
    // No detail: the controls on screen.
    assert_eq!(wanted_subs(&layout, &[1, 0], None), page);
    // C is on screen: its detail adds exactly its pan with the display.
    let wanted = wanted_subs(&layout, &[1, 0], Some(&held("C")));
    let added: Vec<String> = keys(&wanted)
        .into_iter()
        .filter(|k| !keys(&page).contains(k))
        .collect();
    assert_eq!(
        added,
        vec!["band|live_set tracks[name=C] mixer_device panning|value|true".to_string()]
    );
    assert_eq!(wanted.len(), page.len() + 1);
    let unique: std::collections::BTreeSet<String> = keys(&wanted).into_iter().collect();
    assert_eq!(unique.len(), wanted.len(), "each key once");
}

#[test]
fn a_detail_of_a_strip_on_no_page_shown_adds_all_its_subscriptions() {
    let layout = sample();
    // The cue page shows its toggle and TechAlert; B's detail adds B.
    let page = visible_subs(&layout, &[0]);
    let wanted = wanted_subs(&layout, &[0], Some(&held("B")));
    assert_eq!(wanted.len(), page.len() + 5);
    for key in keys(&detail_subs(&held("B"), MeterSource::Level).all()) {
        assert!(keys(&wanted).contains(&key), "{key}");
    }
    for key in keys(&page) {
        assert!(keys(&wanted).contains(&key), "{key}");
    }
    // The layout's meter source applies to the detail too.
    let mut lr = sample();
    lr.config.meter_source = Some(MeterSource::Lr);
    let wanted = wanted_subs(&lr, &[0], Some(&held("B")));
    assert_eq!(wanted.len(), page.len() + 6);
    assert!(
        wanted
            .iter()
            .any(|s| s.target == "live_set tracks[name=B]" && s.prop == "output_meter_right")
    );
}

/// The sample's first group that holds strip A (the pager's STAGE).
fn stage_group(layout: &mut Layout) -> &mut fohmixer_proto::layout::Group {
    let Section::Pager(pager) = &mut layout.pages[1].rows[0].sections[0] else {
        panic!("the pager")
    };
    let Section::Group(group) = &mut pager.pages[0].sections[0] else {
        panic!("STAGE's group")
    };
    group
}

#[test]
fn a_detail_shows_the_layouts_strip_with_the_same_binding() {
    let mut layout = sample();
    // A new layout: A's label, guard and mark changed.
    let Control::Strip(a) = &mut stage_group(&mut layout).controls[0] else {
        panic!("the strip A")
    };
    a.label = Some("Lead".into());
    a.mute_guard = true;
    a.mark = Some(StripMark::Problem);
    let shown = detail_strip(&layout, &held("A")).expect("A is in the layout");
    assert_eq!(shown.binding, held("A").binding);
    assert_eq!(shown.label.as_deref(), Some("Lead"));
    assert!(shown.mute_guard);
    assert_eq!(shown.mark, Some(StripMark::Problem));
    // C as it is; a strip the layout does not hold: none.
    assert_eq!(detail_strip(&layout, &held("C")), Some(held("C")));
    assert_eq!(detail_strip(&layout, &held("Gone")), None);
    // Another instance's track of the same name is another strip.
    let mut master = held("C");
    master.binding.instance = "master".into();
    assert_eq!(detail_strip(&layout, &master), None);
}

#[test]
fn a_strip_in_conflict_has_no_detail() {
    let mut layout = sample();
    // B is only on OTHERS: in conflict, its detail closes.
    let Section::Pager(pager) = &mut layout.pages[1].rows[0].sections[0] else {
        panic!("the pager")
    };
    let Section::Group(group) = &mut pager.pages[1].sections[0] else {
        panic!("OTHERS' group")
    };
    let Control::Strip(b) = &mut group.controls[0] else {
        panic!("the strip B")
    };
    b.mark = Some(StripMark::Conflict);
    assert_eq!(detail_strip(&layout, &held("B")), None);
    // A in conflict on STAGE but whole in the fixed group: that one.
    let Control::Strip(a) = &mut stage_group(&mut layout).controls[0] else {
        panic!("the strip A")
    };
    a.mark = Some(StripMark::Conflict);
    assert_eq!(detail_strip(&layout, &held("A")), Some(held("A")));
}

#[test]
fn a_details_title_is_its_first_titled_group() {
    let mut layout = sample();
    let a = held("A").binding;
    assert_eq!(group_title(&layout, &a), None, "no group has a title");
    // The fixed group titled: A's first group (STAGE) has none, so that one.
    let Section::Group(fixed) = &mut layout.pages[1].rows[0].sections[1] else {
        panic!("the fixed group")
    };
    fixed.title = Some("BAND".into());
    assert_eq!(group_title(&layout, &a).as_deref(), Some("BAND"));
    stage_group(&mut layout).title = Some("STAGE".into());
    assert_eq!(group_title(&layout, &a).as_deref(), Some("STAGE"));
    // B sits only on OTHERS (untitled); a track on no strip has none.
    assert_eq!(group_title(&layout, &held("B").binding), None);
    assert_eq!(group_title(&layout, &held("Gone").binding), None);
    // A group holding only other controls is no strip's group.
    assert_eq!(group_title(&layout, &track("Vocals grp", None)), None);
}

/// A marker strip (#68): the band's track at `index` whose Tuner says
/// `label`.
fn marker(index: u32, label: &str) -> Strip {
    Strip {
        binding: Binding {
            instance: "band".into(),
            anchor: Anchor::TrackAt { index },
            path: None,
        },
        strip_kind: StripKind::Standard,
        wide: false,
        mute_guard: false,
        pinned: false,
        label: Some(label.into()),
        mark: None,
    }
}

/// A layout of one page whose one group holds `strips`.
fn with_strips(strips: &[Strip]) -> Layout {
    let controls: Vec<Control> = strips
        .iter()
        .cloned()
        .map(|s| Control::Strip(Box::new(s)))
        .collect();
    serde_json::from_value(json!({
        "schema": 2,
        "default_page": "foh",
        "pages": [{"id": "foh", "title": "FOH", "rows": [{"sections": [
            {"kind": "group", "id": "markers", "title": "MARKERS", "controls": controls}]}]}]
    }))
    .expect("the layout parses")
}

#[test]
fn a_marker_strips_detail_follows_its_label_when_the_indices_shift() {
    let vox = marker(4, "Vox 1");
    let before = with_strips(&[marker(3, "Klavir"), marker(4, "Vox 1"), marker(5, "Vox 2")]);
    assert_eq!(detail_strip(&before, &vox), Some(marker(4, "Vox 1")));
    // A track deleted above: Vox 1 at 3, Vox 2 at the held index.
    let deleted = with_strips(&[marker(3, "Vox 1"), marker(4, "Vox 2")]);
    assert_eq!(detail_strip(&deleted, &vox), Some(marker(3, "Vox 1")));
    // Then one inserted above: Vox 1 at 5, a strip without it at 4.
    let inserted = with_strips(&[marker(4, "New"), marker(5, "Vox 1"), marker(6, "Vox 2")]);
    assert_eq!(detail_strip(&inserted, &vox), Some(marker(5, "Vox 1")));
    // Its guard and mark as the layout has them now.
    let mut guarded = marker(7, "Vox 1");
    guarded.mute_guard = true;
    guarded.mark = Some(StripMark::Problem);
    let marked = with_strips(&[guarded.clone()]);
    assert_eq!(detail_strip(&marked, &vox), Some(guarded));
}

#[test]
fn a_different_marker_at_the_held_index_never_matches() {
    let vox = marker(4, "Vox 1");
    assert_eq!(
        detail_strip(&with_strips(&[marker(4, "Klavir")]), &vox),
        None
    );
    // The same label in the other instance, or on a return, is another strip.
    let mut master = marker(4, "Vox 1");
    master.binding.instance = "master".into();
    let mut ret = marker(4, "Vox 1");
    ret.binding.anchor = Anchor::ReturnAt { index: 4 };
    ret.strip_kind = StripKind::Return;
    assert_eq!(
        detail_strip(&with_strips(&[master, ret.clone()]), &vox),
        None
    );
    // A return's marker follows its label among the returns.
    let mut moved = ret.clone();
    moved.binding.anchor = Anchor::ReturnAt { index: 2 };
    assert_eq!(
        detail_strip(&with_strips(&[marker(2, "Vox 1"), moved.clone()]), &ret),
        Some(moved)
    );
    // A strip bound by name with that label's track is not the marker.
    let mut named = strip("Vox 1");
    named.label = None;
    assert_eq!(detail_strip(&with_strips(&[named]), &vox), None);
}

#[test]
fn a_marker_in_conflict_closes_its_detail_and_says_so() {
    let vox = marker(4, "Vox 1");
    let mut first = marker(4, "Vox 1");
    first.mark = Some(StripMark::Conflict);
    let mut second = marker(6, "Vox 1");
    second.mark = Some(StripMark::Conflict);
    let conflict = with_strips(&[first, second.clone()]);
    assert_eq!(detail_strip(&conflict, &vox), None);
    assert_eq!(
        detail_update(&conflict, Some(&vox), false),
        Some(DetailChange::Close("conflict"))
    );
    // Gone, beside another marker in conflict: the layout closed it.
    let mut other = marker(2, "Klavir");
    other.mark = Some(StripMark::Conflict);
    assert_eq!(
        detail_update(
            &with_strips(&[marker(4, "Vox 2"), other]),
            Some(&vox),
            false
        ),
        Some(DetailChange::Close("layout"))
    );
    // A held strip marked in conflict before: the layout's strip as it is now.
    assert_eq!(
        detail_update(&with_strips(&[marker(6, "Vox 1")]), Some(&second), false),
        Some(DetailChange::Follow(marker(6, "Vox 1")))
    );
    assert_eq!(
        (CLOSE_EXIT, CLOSE_LAYOUT, CLOSE_CONFLICT),
        ("exit", "layout", "conflict")
    );
}

#[test]
fn a_held_strip_is_written_back_only_when_the_layout_changes_it() {
    let vox = marker(4, "Vox 1");
    let same = with_strips(&[marker(4, "Vox 1")]);
    // No detail, or the same strip: nothing to write.
    assert_eq!(detail_update(&same, None, false), None);
    assert_eq!(detail_update(&same, None, true), None);
    assert_eq!(detail_update(&same, Some(&vox), false), None);
    assert_eq!(
        detail_update(&same, Some(&vox), true),
        None,
        "carried, labelled"
    );
    // Moved: the strip at its new index, so the wanted set follows it.
    let moved = with_strips(&[marker(3, "Vox 1")]);
    assert_eq!(
        detail_update(&moved, Some(&vox), true),
        Some(DetailChange::Follow(marker(3, "Vox 1")))
    );
    // Gone: it closes.
    let gone = with_strips(&[marker(4, "Vox 2")]);
    assert_eq!(
        detail_update(&gone, Some(&vox), false),
        Some(DetailChange::Close("layout"))
    );
    // A strip bound by name: the same binding is the same strip.
    let layout = sample();
    assert_eq!(detail_update(&layout, Some(&held("C")), true), None);
    assert_eq!(
        detail_update(&layout, Some(&held("Gone")), false),
        Some(DetailChange::Close("layout"))
    );
}

#[test]
fn a_placeholder_labelled_marker_carried_into_a_new_layout_closes() {
    // A Tuner without a label shows its track's number (`#5` at index 4):
    // no identity, the same label names another track after a shift.
    let five = marker(4, "#5");
    let shifted = with_strips(&[marker(3, "#4"), marker(4, "#5")]);
    assert_eq!(
        detail_update(&shifted, Some(&five), true),
        Some(DetailChange::Close("layout")),
        "never the other track now labelled #5"
    );
    // Even where it still sits: a new layout cannot tell.
    let same = with_strips(&[marker(4, "#5")]);
    assert_eq!(
        detail_update(&same, Some(&five), true),
        Some(DetailChange::Close("layout"))
    );
    // A detail opened within the layout is not carried: it stays.
    assert_eq!(detail_update(&same, Some(&five), false), None);
    // A real label that is no placeholder of its index is followed.
    let named = marker(4, "#9");
    assert_eq!(
        detail_update(&with_strips(&[marker(2, "#9")]), Some(&named), true),
        Some(DetailChange::Follow(marker(2, "#9")))
    );
}

#[test]
fn a_labelled_strip_bound_by_name_is_its_binding() {
    // A hand-made layout: two tracks bound by name, the same label.
    let label = |name: &str| {
        let mut s = strip(name);
        s.label = Some("VOC".into());
        s
    };
    let layout = with_strips(&[label("Vocal 3 repro#"), label("Vocal 3#")]);
    assert_eq!(
        detail_strip(&layout, &label("Vocal 3#")),
        Some(label("Vocal 3#")),
        "never the first strip of that label"
    );
    assert_eq!(detail_update(&layout, Some(&label("Vocal 3#")), true), None);
    assert_eq!(
        detail_strip(&with_strips(&[label("Vocal 3 repro#")]), &label("Vocal 3#")),
        None
    );
}

#[test]
fn a_details_events_name_its_volume_and_mute_keys() {
    assert_eq!(
        detail_keys(&strip("Hand2 #")),
        vec![
            "band|live_set tracks[name=Hand2 #] mixer_device volume|value".to_string(),
            "band|live_set tracks[name=Hand2 #]|mute".to_string(),
        ]
    );
    assert_eq!(
        detail_keys(&marker(4, "Vox 1")),
        vec![
            "band|live_set tracks 4 mixer_device volume|value".to_string(),
            "band|live_set tracks 4|mute".to_string(),
        ]
    );
    let mut bad = strip("X");
    bad.binding.path = Some("devices[name=".into());
    assert_eq!(detail_keys(&bad), Vec::<String>::new());
}
