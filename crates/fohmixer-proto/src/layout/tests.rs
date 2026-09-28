use super::*;
use serde_json::json;

/// A small valid layout: one page with a strip and a nested pager, an
/// overlay with the TechAlert box and REFRESH ALL.
fn sample() -> Value {
    json!({
        "schema": 1,
        "canvas": {"w": 2360, "h": 1640},
        "background": "#9D9DA0FF",
        "tabbar": {"orientation": "top", "bar_size": 59, "default_page": 0},
        "pages": [{
            "id": "main",
            "title": "FOH",
            "tab": {"color": "#404040", "text_size": 33},
            "items": [
                {"kind": "area", "frame": {"x": 0, "y": 59, "w": 400, "h": 400},
                 "style": {"bg": "#646464", "text": "EFFECTS", "vertical": true}, "title": "EFFECTS"},
                {"kind": "strip", "id": "s1", "frame": {"x": 10, "y": 100, "w": 160, "h": 710}, "z": 2,
                 "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Klavir #"}},
                 "strip_kind": "standard",
                 "children": {"fader": {"x": 37, "y": 183, "w": 106, "h": 553},
                              "mute": {"x": 37, "y": 748, "w": 106, "h": 52}},
                 "mute_guard": true},
                {"kind": "stage", "frame": {"x": 27, "y": 78, "w": 115, "h": 85},
                 "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Mics Stage #"}}, "aut": true},
                {"kind": "hub_toggle", "frame": {"x": 27, "y": 180, "w": 115, "h": 85},
                 "key": "stage_aut", "label": "STAGE AUT"},
                {"kind": "param_toggle", "frame": {"x": 27, "y": 300, "w": 150, "h": 120},
                 "label": "REVERB", "press": "toggle",
                 "targets": [{"binding": {"instance": "band", "anchor": {"kind": "track", "name": "Rev #"}},
                              "prop": "mute", "on": false, "off": true}]},
                {"kind": "param_fader", "frame": {"x": 800, "y": 300, "w": 101, "h": 419},
                 "label": "Podklady All",
                 "targets": [{"binding": {"instance": "band", "anchor": {"kind": "track", "name": "Stems grp#"},
                                          "path": "mixer_device volume"},
                              "prop": "value", "scale": "cc_linear"}]},
                {"kind": "solo", "frame": {"x": 27, "y": 450, "w": 161, "h": 65},
                 "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Vocals Repro grp#"}}}
            ],
            "pager": {
                "frame": {"x": 229, "y": 61, "w": 1746, "h": 773},
                "tabbar": {"orientation": "left", "bar_size": 65, "default_page": 0},
                "background": "#000000FF",
                "pages": [{"id": "stage", "title": "STAGE", "background": "#000000F9",
                           "tab": {"color": "#404040FF", "color_on": "#BBFFA656", "text_size": 36, "text_size_on": 51},
                           "items": []},
                          {"id": "others", "title": "OTHERS", "items": [
                             {"kind": "label", "frame": {"x": 300, "y": 100, "w": 50, "h": 20}, "text": "HANDS"}]}]
            }
        }, {"id": "conf", "title": "Conf", "items": []}],
        "overlay": [
            {"kind": "alert", "frame": {"x": 0, "y": 80, "w": 2360, "h": 1553},
             "style": {"bg": "#FF00001F"},
             "binding": {"instance": "band", "anchor": {"kind": "track", "name": "TechAlert #"}}, "period_ms": 300},
            {"kind": "refresh", "frame": {"x": 21, "y": 1300, "w": 209, "h": 54}, "label": "REFRESH ALL"}
        ],
        "config": {"unfold": [{"instance": "band", "name": "Vocals Repro grp#"}], "fader_shaping": true},
        "report": {"dropped": []}
    })
}

fn parse(value: Value) -> Layout {
    serde_json::from_value(value).expect("the layout parses")
}

fn errors(value: Value) -> Vec<String> {
    parse(value)
        .validate()
        .into_iter()
        .map(|e| e.to_string())
        .collect()
}

#[test]
fn the_sample_parses_validates_and_round_trips() {
    let layout = parse(sample());
    assert_eq!(layout.validate(), vec![]);
    assert_eq!(layout.pages.len(), 2);
    assert_eq!(layout.pages[0].pager.as_ref().unwrap().pages.len(), 2);
    assert_eq!(layout.overlay.len(), 2);
    let ItemKind::Strip(strip) = &layout.pages[0].items[1].kind else {
        panic!("a strip")
    };
    assert_eq!(strip.strip_kind, StripKind::Standard);
    assert!(strip.mute_guard);
    assert_eq!(layout.pages[0].items[1].z, 2);
    assert_eq!(layout.background.as_deref(), Some("#9D9DA0FF"));
    let pager = layout.pages[0].pager.as_ref().unwrap();
    assert_eq!(pager.background.as_deref(), Some("#000000FF"));
    let stage = &pager.pages[0];
    assert_eq!(stage.background.as_deref(), Some("#000000F9"));
    assert_eq!(
        stage.tab,
        Tab {
            color: Some("#404040FF".into()),
            color_on: Some("#BBFFA656".into()),
            text_size: Some(36.0),
            text_size_on: Some(51.0),
        }
    );
    assert_eq!(pager.pages[1].background, None);
    assert_eq!(pager.pages[1].tab, Tab::default());
    let again: Layout = serde_json::from_value(serde_json::to_value(&layout).unwrap()).unwrap();
    assert_eq!(again, layout);
    // Absent fields stay absent when written back.
    let written = serde_json::to_value(&layout).unwrap();
    assert!(
        written["pages"][0]["pager"]["pages"][1]
            .get("background")
            .is_none()
    );
    assert!(written["pages"][1]["tab"].get("color_on").is_none());
}

#[test]
fn an_unknown_kind_does_not_parse() {
    let mut v = sample();
    v["overlay"][1]["kind"] = json!("battery");
    assert!(serde_json::from_value::<Layout>(v).is_err());
}

#[test]
fn a_duplicate_page_id_is_an_error() {
    let mut v = sample();
    v["pages"][1]["id"] = json!("stage");
    assert_eq!(
        errors(v),
        vec![r#"pages[1]: duplicate page id "stage""#.to_string()]
    );
}

#[test]
fn a_duplicate_item_id_is_an_error() {
    let mut v = sample();
    v["overlay"][1]["id"] = json!("s1");
    assert_eq!(
        errors(v),
        vec![r#"overlay[1]: duplicate item id "s1""#.to_string()]
    );
}

#[test]
fn a_frame_outside_the_canvas_is_an_error() {
    let mut v = sample();
    v["pages"][0]["items"][0]["frame"] = json!({"x": 2000, "y": 59, "w": 400, "h": 400});
    assert_eq!(
        errors(v),
        vec![
            "pages[0].items[0].frame: frame (2000, 59, 400×400) is not inside the 2360×1640 canvas"
                .to_string()
        ]
    );
    for frame in [
        json!({"x": -1, "y": 0, "w": 10, "h": 10}),
        json!({"x": 0, "y": -1, "w": 10, "h": 10}),
        json!({"x": 0, "y": 1631, "w": 10, "h": 10}),
        json!({"x": 0, "y": 0, "w": 0, "h": 10}),
        json!({"x": 0, "y": 0, "w": 10, "h": 0}),
    ] {
        let mut v = sample();
        v["overlay"][1]["frame"] = frame.clone();
        assert_eq!(errors(v).len(), 1, "{frame}");
    }
    let mut v = sample();
    v["overlay"][1]["frame"] = json!({"x": 2350.4, "y": 0, "w": 10, "h": 1640.4});
    assert_eq!(errors(v), Vec::<String>::new(), "within the rounding slack");
}

#[test]
fn a_non_finite_frame_is_an_error() {
    let mut layout = parse(sample());
    layout.overlay[1].frame.w = f64::NAN;
    assert_eq!(layout.validate().len(), 1);
    layout.overlay[1].frame.w = 10.0;
    layout.overlay[1].frame.x = f64::INFINITY;
    assert_eq!(layout.validate().len(), 1);
    let mut layout = parse(sample());
    layout.canvas.w = f64::INFINITY;
    let found: Vec<String> = layout.validate().iter().map(|e| e.at.clone()).collect();
    assert_eq!(
        found,
        vec!["canvas".to_string()],
        "every frame fits an infinite canvas"
    );
    layout.canvas.w = 2360.0;
    layout.canvas.h = f64::NAN;
    assert!(layout.validate().iter().any(|e| e.at == "canvas"));
}

#[test]
fn strip_children_and_pagers_are_inside_the_canvas_too() {
    let mut v = sample();
    v["pages"][0]["items"][1]["children"]["mute"] = json!({"x": 37, "y": 1600, "w": 106, "h": 52});
    v["pages"][0]["pager"]["frame"] = json!({"x": 1000, "y": 61, "w": 1746, "h": 773});
    assert_eq!(
        errors(v),
        vec![
            "pages[0].items[1].children.mute: frame (37, 1600, 106×52) is not inside the 2360×1640 canvas"
                .to_string(),
            "pages[0].pager: frame (1000, 61, 1746×773) is not inside the 2360×1640 canvas"
                .to_string(),
        ]
    );
}

#[test]
fn a_bad_path_is_an_error() {
    let mut v = sample();
    v["pages"][0]["items"][5]["targets"][0]["binding"]["path"] =
        json!("devices[name=Latencies parameters 1");
    assert_eq!(
        errors(v),
        vec!["pages[0].items[5].targets[0]: path: syntax: unterminated [name=".to_string()]
    );
}

#[test]
fn a_param_toggle_without_targets_is_an_error() {
    let mut v = sample();
    v["pages"][0]["items"][4]["targets"] = json!([]);
    assert_eq!(errors(v), vec!["pages[0].items[4]: no targets".to_string()]);
}

#[test]
fn param_targets_need_a_prop_and_their_values_or_scale() {
    let mut v = sample();
    v["pages"][0]["items"][4]["targets"][0]["prop"] = json!("");
    v["pages"][0]["items"][4]["targets"][0]
        .as_object_mut()
        .unwrap()
        .remove("off");
    v["pages"][0]["items"][5]["targets"][0]
        .as_object_mut()
        .unwrap()
        .remove("scale");
    v["pages"][0]["items"][5]["targets"] = json!([]);
    assert_eq!(
        errors(v),
        vec![
            "pages[0].items[4].targets[0]: target without a prop".to_string(),
            "pages[0].items[4].targets[0]: a toggle target needs on and off".to_string(),
            "pages[0].items[5]: no targets".to_string(),
        ]
    );
    let mut v = sample();
    v["pages"][0]["items"][5]["targets"][0]
        .as_object_mut()
        .unwrap()
        .remove("scale");
    v["pages"][0]["items"][4]["targets"][0]
        .as_object_mut()
        .unwrap()
        .remove("on");
    assert_eq!(
        errors(v),
        vec![
            "pages[0].items[4].targets[0]: a toggle target needs on and off".to_string(),
            "pages[0].items[5].targets[0]: a fader target needs a scale".to_string(),
        ]
    );
}

#[test]
fn bindings_need_an_instance_and_a_name() {
    let mut v = sample();
    v["pages"][0]["items"][6]["binding"]["instance"] = json!("");
    v["overlay"][0]["binding"]["anchor"] = json!({"kind": "return", "name": ""});
    v["pages"][0]["items"][2]["binding"]["anchor"] = json!({"kind": "track", "name": ""});
    assert_eq!(
        errors(v),
        vec![
            "pages[0].items[2].binding: anchor without a name".to_string(),
            "pages[0].items[6].binding: binding without an instance".to_string(),
            "overlay[0].binding: anchor without a name".to_string(),
        ]
    );
}

#[test]
fn schema_canvas_pages_and_misc_are_checked() {
    let mut v = sample();
    v["schema"] = json!(2);
    v["overlay"][0]["period_ms"] = json!(0);
    v["pages"][0]["items"][3]["key"] = json!("tempo");
    v["pages"][0]["tab"]["color"] = json!("gray");
    v["pages"][0]["tab"]["color_on"] = json!("#12345");
    v["pages"][0]["background"] = json!("black");
    v["pages"][0]["pager"]["background"] = json!("#00000G");
    v["background"] = json!("#9D9DA0F");
    v["overlay"][0]["style"]["bg"] = json!("#FF00001");
    v["pages"][0]["pager"]["tabbar"]["default_page"] = json!(2);
    v["pages"][1]["id"] = json!("");
    v["config"]["unfold"][0]["name"] = json!("");
    assert_eq!(
        errors(v),
        vec![
            "schema: schema 2 is not 1".to_string(),
            r##"background: "#9D9DA0F" is not #RRGGBB or #RRGGBBAA"##.to_string(),
            r#"pages[0].tab.color: "gray" is not #RRGGBB or #RRGGBBAA"#.to_string(),
            r##"pages[0].tab.color_on: "#12345" is not #RRGGBB or #RRGGBBAA"##.to_string(),
            r#"pages[0].background: "black" is not #RRGGBB or #RRGGBBAA"#.to_string(),
            r#"pages[0].items[3]: unknown hub value "tempo""#.to_string(),
            r##"pages[0].pager.background: "#00000G" is not #RRGGBB or #RRGGBBAA"##.to_string(),
            "pages[0].pager.pages: default page 2 of 2 pages".to_string(),
            "pages[1]: empty page id".to_string(),
            r##"overlay[0].style.bg: "#FF00001" is not #RRGGBB or #RRGGBBAA"##.to_string(),
            "overlay[0]: alert period 0 ms".to_string(),
            "config.unfold[0]: needs an instance and a name".to_string(),
        ]
    );
    let mut v = sample();
    v["canvas"] = json!({"w": 0, "h": 1640});
    v["pages"] = json!([]);
    v["tabbar"]["bar_size"] = json!(-1);
    let found = errors(v);
    assert!(
        found.contains(&"canvas: 0×1640 is not a canvas size".to_string()),
        "{found:?}"
    );
    assert!(found.contains(&"pages: no pages".to_string()), "{found:?}");
    assert!(
        found.contains(&"pages: tab bar size -1".to_string()),
        "{found:?}"
    );
    let mut v = sample();
    v["tabbar"]["default_page"] = json!(5);
    v["canvas"] = json!({"w": 2360, "h": -1});
    let found = errors(v);
    assert!(
        found.contains(&"pages: default page 5 of 2 pages".to_string()),
        "{found:?}"
    );
    assert!(
        found.contains(&"canvas: 2360×-1 is not a canvas size".to_string()),
        "{found:?}"
    );
    let mut v = sample();
    v["canvas"] = json!({"w": 2360, "h": 0});
    let found = errors(v);
    assert!(
        found.contains(&"canvas: 2360×0 is not a canvas size".to_string()),
        "{found:?}"
    );
}

/// A mistyped field in a hand edit (D4) is an error, never silently
/// ignored: at the top, in a page, in an item of every shape (a struct kind,
/// the boxed strip), in a binding, a frame, a style and the config.
#[test]
fn unknown_fields_do_not_parse() {
    for place in [
        "",
        "/canvas",
        "/tabbar",
        "/pages/0",
        "/pages/0/tab",
        "/pages/0/items/0",
        "/pages/0/items/0/frame",
        "/pages/0/items/0/style",
        "/pages/0/items/1",
        "/pages/0/items/1/binding",
        "/pages/0/items/1/children",
        "/pages/0/items/4/targets/0",
        "/config",
    ] {
        let mut v = sample();
        v.pointer_mut(place).expect(place)["colour"] = json!("#FF0000");
        let error = match serde_json::from_value::<Layout>(v) {
            Ok(_) => panic!("an unknown field at {place:?} parsed"),
            Err(e) => e.to_string(),
        };
        assert!(
            error.contains("unknown field `colour`"),
            "{place:?}: {error}"
        );
    }
    // The pager, an overlay item and an unfold target too.
    let mut v = sample();
    v["pages"][0]["pager"]["colour"] = json!(1);
    assert!(serde_json::from_value::<Layout>(v).is_err());
    let mut v = sample();
    v["overlay"][1]["colour"] = json!(1);
    assert!(serde_json::from_value::<Layout>(v).is_err());
    let mut v = sample();
    v["config"]["unfold"][0]["colour"] = json!(1);
    assert!(serde_json::from_value::<Layout>(v).is_err());
}

#[test]
fn colors_are_six_or_eight_hex_digits() {
    for ok in ["#000000", "#bbffa656", "#A0B1C2"] {
        assert!(is_color(ok), "{ok}");
    }
    for bad in ["000000", "#00000", "#0000000", "#GG0000", "#000000000", ""] {
        assert!(!is_color(bad), "{bad}");
    }
}

#[test]
fn binding_targets_follow_the_anchor() {
    let b = |anchor: Anchor, path: Option<&str>| Binding {
        instance: "band".into(),
        anchor,
        path: path.map(str::to_string),
    };
    assert_eq!(
        b(
            Anchor::Track {
                name: "Klavir #".into()
            },
            None
        )
        .target()
        .unwrap(),
        "live_set tracks[name=Klavir #]"
    );
    assert_eq!(
        b(
            Anchor::Track {
                name: "Vocal 1 repro#".into()
            },
            Some("devices[name=EQ Eight]  parameters 1")
        )
        .target()
        .unwrap(),
        "live_set tracks[name=Vocal 1 repro#] devices[name=EQ Eight] parameters 1"
    );
    assert_eq!(
        b(
            Anchor::Return {
                name: "A-Rev]x".into()
            },
            Some("mixer_device volume")
        )
        .target()
        .unwrap(),
        r"live_set return_tracks[name=A-Rev\]x] mixer_device volume"
    );
    assert_eq!(
        b(Anchor::Master, Some("mixer_device volume"))
            .target()
            .unwrap(),
        "live_set master_track mixer_device volume"
    );
    assert_eq!(b(Anchor::Song, Some("")).target().unwrap(), "live_set");
    assert_eq!(b(Anchor::Song, None).target().unwrap(), "live_set");
    assert!(b(Anchor::Master, Some("_x")).target().is_err());
    assert_eq!(
        serde_json::to_value(b(Anchor::Master, None)).unwrap(),
        json!({"instance": "band", "anchor": {"kind": "master"}})
    );
}

#[test]
fn bindings_are_listed_in_document_order() {
    let layout = parse(sample());
    let names: Vec<String> = layout
        .bindings()
        .iter()
        .map(|b| b.target().unwrap())
        .collect();
    assert_eq!(
        names,
        vec![
            "live_set tracks[name=Klavir #]",
            "live_set tracks[name=Mics Stage #]",
            "live_set tracks[name=Rev #]",
            "live_set tracks[name=Stems grp#] mixer_device volume",
            "live_set tracks[name=Vocals Repro grp#]",
            "live_set tracks[name=TechAlert #]",
        ]
    );
}

#[test]
fn the_stage_aut_binding_is_the_first_stage_item_with_aut() {
    let layout = parse(sample());
    assert_eq!(
        layout.stage_aut_binding().unwrap().target().unwrap(),
        "live_set tracks[name=Mics Stage #]"
    );
    let mut v = sample();
    v["pages"][0]["items"][2]["aut"] = json!(false);
    assert!(parse(v.clone()).stage_aut_binding().is_none());
    // One in a nested pager page is found too, and one in the overlay.
    v["pages"][0]["pager"]["pages"][1]["items"] = json!([
        {"kind": "stage", "frame": {"x": 300, "y": 100, "w": 50, "h": 20}, "aut": true,
         "binding": {"instance": "master", "anchor": {"kind": "track", "name": "Stage2"}}}]);
    assert_eq!(
        parse(v.clone()).stage_aut_binding().unwrap().instance,
        "master"
    );
    v["pages"][0]["pager"]["pages"][1]["items"] = json!([]);
    v["overlay"] = json!([
        {"kind": "stage", "frame": {"x": 300, "y": 100, "w": 50, "h": 20}, "aut": true,
         "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Overlay stage"}}}]);
    assert_eq!(
        parse(v).stage_aut_binding().unwrap().target().unwrap(),
        "live_set tracks[name=Overlay stage]"
    );
}

/// The TouchOSC import tool's output for its synthetic fixtures
/// (`tools/import-tosc/fixtures/expected-layout.json`, which the tool's own
/// tests compare with its import) is a layout this schema reads and accepts.
#[test]
fn imported_layout_parses_and_validates() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/import-tosc/fixtures/expected-layout.json"
    );
    let text = std::fs::read_to_string(path).expect("the import tool's fixture output");
    let layout: Layout = serde_json::from_str(&text).expect("it parses");
    assert_eq!(layout.validate(), vec![]);
    let titles: Vec<&str> = layout.pages.iter().map(|p| p.title.as_str()).collect();
    assert_eq!(titles, vec!["Cue", "FOH", "Conf"]);
    assert_eq!(layout.tabbar.orientation, Orientation::Top);
    let pager = layout.pages[1].pager.as_ref().expect("the nested pager");
    assert_eq!(pager.tabbar.orientation, Orientation::Left);
    assert_eq!(
        layout.stage_aut_binding().unwrap().target().unwrap(),
        "live_set tracks[name=Mics Stage #]"
    );
    let kinds: Vec<&str> = layout
        .overlay
        .iter()
        .map(|i| match i.kind {
            ItemKind::Strip(_) => "strip",
            ItemKind::Refresh { .. } => "refresh",
            ItemKind::Alert { .. } => "alert",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, vec!["strip", "refresh", "alert"]);
    let targets: Vec<String> = layout
        .bindings()
        .iter()
        .map(|b| b.target().unwrap())
        .collect();
    assert!(targets.contains(
        &"live_set tracks[name=Vocal 1 repro#] devices[name=Vox Chain] chains[name=Main] devices[name=Latencies] parameters 1"
            .to_string()
    ));
    // Re-serialised, it reads back the same.
    let again: Layout = serde_json::from_value(serde_json::to_value(&layout).unwrap()).unwrap();
    assert_eq!(again, layout);
}

#[test]
fn strip_children_are_listed_by_name() {
    let f = Frame {
        x: 1.0,
        y: 2.0,
        w: 3.0,
        h: 4.0,
    };
    let children = StripChildren {
        meter: Some(f),
        instance_label: Some(f),
        ..StripChildren::default()
    };
    assert_eq!(children.frames(), vec![("meter", f), ("instance_label", f)]);
    let all = StripChildren {
        fader: Some(f),
        pan: Some(f),
        mute: Some(f),
        meter: Some(f),
        status: Some(f),
        db: Some(f),
        label: Some(f),
        instance_label: Some(f),
    };
    let names: Vec<&str> = all.frames().into_iter().map(|(n, _)| n).collect();
    assert_eq!(
        names,
        vec![
            "fader",
            "pan",
            "mute",
            "meter",
            "status",
            "db",
            "label",
            "instance_label"
        ]
    );
}

/// The meter source switch (spec X2, #5): absent is `level` (TouchOSC
/// parity), `lr` names the two bars, anything else does not parse.
#[test]
fn the_meter_source_is_level_unless_lr_is_named() {
    let layout = parse(sample());
    assert_eq!(layout.config.meter_source, None);
    assert_eq!(MeterSource::default(), MeterSource::Level);
    let mut v = sample();
    v["config"]["meter_source"] = json!("lr");
    let layout = parse(v);
    assert_eq!(layout.config.meter_source, Some(MeterSource::Lr));
    assert_eq!(
        serde_json::to_value(&layout.config).unwrap()["meter_source"],
        json!("lr")
    );
    let mut v = sample();
    v["config"]["meter_source"] = json!("level");
    assert_eq!(parse(v).config.meter_source, Some(MeterSource::Level));
    let mut v = sample();
    v["config"]["meter_source"] = json!("stereo");
    assert!(serde_json::from_value::<Layout>(v).is_err());
    let absent = serde_json::to_value(&parse(sample()).config).unwrap();
    assert!(absent.get("meter_source").is_none(), "{absent}");
}
