use super::*;
use fohmixer_proto::layout::{Frame, StripChildren, StripKind};

/// The layout the import tool makes from its synthetic fixture (the E2E
/// suite serves the same file).
fn imported() -> Layout {
    let text = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/import-tosc/fixtures/expected-layout.json"
    ));
    serde_json::from_str(text).expect("the imported layout parses")
}

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

fn frame() -> Option<Frame> {
    Some(Frame {
        x: 0.0,
        y: 0.0,
        w: 10.0,
        h: 10.0,
    })
}

fn strip(children: StripChildren) -> Strip {
    Strip {
        binding: track("Hand2 #", None),
        strip_kind: StripKind::Standard,
        children,
        mute_guard: false,
    }
}

#[test]
fn a_strip_subscribes_its_present_parts() {
    let full = strip(StripChildren {
        fader: frame(),
        pan: frame(),
        mute: frame(),
        meter: frame(),
        status: frame(),
        db: frame(),
        label: frame(),
        instance_label: frame(),
    });
    let subs = strip_subs(&full, MeterSource::Level);
    let t = "live_set tracks[name=Hand2 #]";
    assert_eq!(
        subs.volume,
        Some(SubSpec::new(
            "band",
            format!("{t} mixer_device volume"),
            "value",
            true
        ))
    );
    assert_eq!(
        subs.pan,
        Some(SubSpec::new(
            "band",
            format!("{t} mixer_device panning"),
            "value",
            false
        ))
    );
    assert_eq!(
        subs.mute,
        Some(SubSpec::new("band", t.into(), "mute", false))
    );
    assert_eq!(
        subs.meters,
        vec![SubSpec::new("band", t.into(), "output_meter_level", false)]
    );
    assert_eq!(subs.all().len(), 4);
    let lr = strip_subs(&full, MeterSource::Lr);
    assert_eq!(
        lr.meters,
        vec![
            SubSpec::new("band", t.into(), "output_meter_left", false),
            SubSpec::new("band", t.into(), "output_meter_right", false),
        ]
    );
    assert_eq!(lr.all().len(), 5);
}

#[test]
fn missing_parts_subscribe_nothing_and_a_db_text_needs_the_volume() {
    let meter_mute = strip(StripChildren {
        mute: frame(),
        meter: frame(),
        status: frame(),
        label: frame(),
        ..StripChildren::default()
    });
    let subs = strip_subs(&meter_mute, MeterSource::Level);
    assert_eq!((subs.volume, subs.pan), (None, None));
    assert!(subs.mute.is_some() && subs.meters.len() == 1);
    let db_only = strip(StripChildren {
        db: frame(),
        ..StripChildren::default()
    });
    let subs = strip_subs(&db_only, MeterSource::Level);
    assert!(subs.volume.is_some());
    assert_eq!(subs.all().len(), 1);
    let fader_only = strip(StripChildren {
        fader: frame(),
        ..StripChildren::default()
    });
    assert_eq!(strip_subs(&fader_only, MeterSource::Lr).all().len(), 1);
    assert!(
        strip_subs(&strip(StripChildren::default()), MeterSource::Lr)
            .all()
            .is_empty()
    );
}

#[test]
fn every_item_kind_subscribes_what_it_shows() {
    let layout = imported();
    let foh = &layout.pages[1];
    let keys = |i: usize| -> Vec<String> {
        item_subs(&foh.items[i], MeterSource::Level)
            .iter()
            .map(SubSpec::key)
            .collect()
    };
    assert!(keys(0).is_empty(), "an area");
    assert_eq!(
        keys(1),
        ["band|live_set tracks[name=Mics Stage #]|mute|false"]
    );
    assert!(keys(2).is_empty(), "the STAGE AUT hub toggle");
    assert_eq!(
        keys(3),
        ["band|live_set tracks[name=Vocals Repro grp#]|solo|false"]
    );
    assert_eq!(
        keys(6),
        [
            "band|live_set tracks[name=Vocal 1 repro#]|mute|false",
            "band|live_set tracks[name=Vocal 2 repro#]|mute|false"
        ],
        "VOC MIC: every target"
    );
    assert_eq!(
        keys(10),
        [
            "band|live_set tracks[name=Drums #] mixer_device volume|value|true",
            "band|live_set tracks[name=Bass #] mixer_device volume|value|false"
        ],
        "Podklady All: the first target's display string"
    );
    assert!(keys(12).is_empty(), "a label");
    // A toggle's targets never bring a display string; a fader's first does.
    let ItemKind::ParamToggle { targets, .. } = &foh.items[6].kind else {
        panic!("VOC MIC is a param toggle");
    };
    assert!(
        param_subs(targets, false)
            .iter()
            .all(|s| !s.as_ref().unwrap().display)
    );
    let fader: Vec<bool> = param_subs(targets, true)
        .iter()
        .map(|s| s.as_ref().unwrap().display)
        .collect();
    assert_eq!(fader, [true, false]);
    // A target whose path does not parse keeps its place, as `None`.
    let mut broken = targets.clone();
    broken[0].binding.path = Some("devices[name=".into());
    let subs = param_subs(&broken, false);
    assert_eq!(subs.len(), 2);
    assert_eq!(subs[0], None);
    assert!(subs[1].is_some());
    let overlay: Vec<Vec<String>> = layout
        .overlay
        .iter()
        .map(|i| {
            item_subs(i, MeterSource::Level)
                .iter()
                .map(SubSpec::key)
                .collect()
        })
        .collect();
    assert_eq!(overlay[1], Vec::<String>::new(), "REFRESH ALL");
    assert_eq!(
        overlay[2],
        ["band|live_set tracks[name=TechAlert #]|mute|false"]
    );
}

fn visible_keys(layout: &Layout, path: &[usize]) -> Vec<String> {
    visible_subs(layout, path)
        .iter()
        .map(SubSpec::key)
        .collect()
}

#[test]
fn only_the_visible_pages_and_the_overlay_are_subscribed() {
    let layout = imported();
    let stage = visible_keys(&layout, &[1, 0]);
    assert_eq!(stage.len(), 28);
    let sorted = {
        let mut s = stage.clone();
        s.sort();
        s.dedup();
        s
    };
    assert_eq!(sorted, stage, "each key once, sorted");
    let tech = "band|live_set tracks[name=TechAlert #]|mute|false".to_string();
    assert!(stage.contains(&tech), "the overlay");
    assert!(stage.contains(&"band|live_set tracks[name=Keys 1]|mute|false".to_string()));
    assert!(!stage.iter().any(|k| k.starts_with("master|")));
    let others = visible_keys(&layout, &[1, 1]);
    assert_eq!(others.len(), 25);
    assert!(others.contains(&"master|live_set tracks[name=Hand1 #]|mute|false".to_string()));
    assert!(
        !others.iter().any(|k| k.contains("Keys 1")),
        "STAGE is hidden"
    );
    assert!(others.contains(&tech));
    let cue = visible_keys(&layout, &[0]);
    assert_eq!(
        cue,
        [
            "band|live_set tracks[name=TechAlert #]|mute|false",
            "band|live_set tracks[name=TechAlert #]|output_meter_level|false",
            "band|live_set tracks[name=Vocal 3 repro#]|mute|false"
        ]
    );
    assert_eq!(
        visible_keys(&layout, &[2]).len(),
        2,
        "Conf: the overlay only"
    );
    assert_eq!(visible_keys(&layout, &[]).len(), 2);
    assert_eq!(
        visible_keys(&layout, &[7]).len(),
        2,
        "a stale index shows nothing"
    );
}

#[test]
fn the_meter_source_switch_applies_to_every_strip() {
    let mut layout = imported();
    layout.config.meter_source = Some(MeterSource::Lr);
    let keys = visible_keys(&layout, &[0]);
    assert!(
        keys.contains(
            &"band|live_set tracks[name=TechAlert #]|output_meter_left|false".to_string()
        )
    );
    assert!(
        keys.contains(
            &"band|live_set tracks[name=TechAlert #]|output_meter_right|false".to_string()
        )
    );
    assert_eq!(keys.len(), 4);
    assert_eq!(meter_props(MeterSource::Level), ["output_meter_level"]);
}

fn remember(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn the_selected_pages_are_the_remembered_ones_or_the_defaults() {
    let layout = imported();
    assert_eq!(
        selected_path(&layout, &remember(&[])),
        [1, 0],
        "FOH and its STAGE"
    );
    assert_eq!(selected_path(&layout, &remember(&[("", "cue")])), [0]);
    assert_eq!(
        selected_path(&layout, &remember(&[("foh", "others")])),
        [1, 1],
        "the pager keeps its page"
    );
    assert_eq!(
        selected_path(&layout, &remember(&[("", "conf"), ("foh", "others")])),
        [2]
    );
    assert_eq!(selected_path(&layout, &remember(&[("", "gone")])), [1, 0]);
    assert_eq!(
        selected_path(&layout, &remember(&[("foh", "gone")])),
        [1, 0]
    );
    assert_eq!(
        selected_path(&layout, &remember(&[("stage", "others")])),
        [1, 0],
        "keyed by the page holding the pager"
    );
    // A default beyond the pages picks the last one.
    let mut layout = imported();
    layout.tabbar.default_page = 5;
    assert_eq!(selected_path(&layout, &remember(&[])), [2]);
    layout.pages.clear();
    assert_eq!(selected_path(&layout, &remember(&[])), Vec::<usize>::new());
}

#[test]
fn choosing_a_tab_remembers_it_for_its_pager() {
    let layout = imported();
    let mut remembered = BTreeMap::new();
    choose(&layout, &mut remembered, &[1, 0], 1, 1);
    assert_eq!(remembered, remember(&[("foh", "others")]));
    let path = selected_path(&layout, &remembered);
    assert_eq!(path, [1, 1]);
    choose(&layout, &mut remembered, &path, 0, 0);
    assert_eq!(remembered, remember(&[("", "cue"), ("foh", "others")]));
    assert_eq!(selected_path(&layout, &remembered), [0]);
    // Back on FOH, its pager still shows OTHERS.
    choose(&layout, &mut remembered, &[0], 0, 1);
    assert_eq!(selected_path(&layout, &remembered), [1, 1]);
    // A tab that is not there, or a level without a pager, changes nothing.
    let before = remembered.clone();
    choose(&layout, &mut remembered, &[1, 1], 1, 5);
    choose(&layout, &mut remembered, &[0], 1, 0);
    choose(&layout, &mut remembered, &[1, 1], 2, 0);
    choose(&layout, &mut remembered, &[1, 1], 0, 9);
    assert_eq!(remembered, before);
}

#[test]
fn refresh_unfolds_the_configured_groups() {
    let layout = imported();
    assert_eq!(
        unfold_targets(&layout.config),
        [
            (
                "band".to_string(),
                "live_set tracks[name=Vocals Repro grp#]".to_string()
            ),
            (
                "band".to_string(),
                "live_set tracks[name=Old grp#]".to_string()
            ),
        ]
    );
    let config = LayoutConfig {
        unfold: vec![fohmixer_proto::layout::UnfoldTarget {
            instance: "master".into(),
            name: "G]1".into(),
        }],
        fader_shaping: None,
        meter_source: None,
    };
    assert_eq!(
        unfold_targets(&config),
        [(
            "master".to_string(),
            r"live_set tracks[name=G\]1]".to_string()
        )]
    );
}
