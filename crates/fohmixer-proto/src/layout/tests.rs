use super::*;
use serde_json::json;

fn track(name: &str) -> Value {
    json!({"instance": "band", "anchor": {"kind": "track", "name": name}})
}

/// A small valid layout: a cue page of toggles; the FOH page with a rail,
/// a nested pager beside fixed strips and a second row; the Conf page's
/// text; TechAlert on every page.
fn sample() -> Value {
    json!({
        "schema": 2,
        "default_page": "foh",
        "pages": [
            {"id": "cue", "title": "Cue", "rows": [{"sections": [
                {"kind": "group", "id": "cue-1", "controls": [
                    {"kind": "param_toggle", "label": "Vox 1 TU", "press": "toggle",
                     "targets": [{"binding": track("Vox 1 stream"), "prop": "mute", "on": false, "off": true}]}]}]}]},
            {"id": "foh", "title": "FOH",
             "rail": [
                {"kind": "stage", "binding": track("Mics Stage #"), "aut": true, "label": "STAGE"},
                {"kind": "hub_toggle", "key": "stage_aut", "label": "STAGE AUT"},
                {"kind": "solo", "binding": track("Vocals Repro grp#"), "label": "Vocals"},
                {"kind": "param_toggle", "label": "REVERB", "press": "toggle", "color": "#F39420",
                 "targets": [{"binding": track("Rev #"), "prop": "mute", "on": false, "off": true}]}],
             "rows": [
                {"sections": [
                    {"kind": "pager", "id": "foh-pager", "default_page": "stage", "pages": [
                        {"id": "stage", "title": "STAGE", "sections": [
                            {"kind": "group", "id": "stage-1", "title": "STAGE", "color": "#1E3A1E", "controls": [
                                {"kind": "strip", "binding": track("Klavir #"), "strip_kind": "standard", "mute_guard": true}]}]},
                        {"id": "others", "title": "OTHERS", "sections": []}]},
                    {"kind": "group", "id": "foh-2", "controls": [
                        {"kind": "strip", "binding": track("Podklady #"), "strip_kind": "standard", "wide": true}]}]},
                {"sections": [
                    {"kind": "group", "id": "foh-3", "title": "EFFECTS", "color": "#636363", "controls": [
                        {"kind": "strip",
                         "binding": {"instance": "master", "anchor": {"kind": "return", "name": "A-Rev"}},
                         "strip_kind": "return"},
                        {"kind": "param_fader", "label": "Podklady All",
                         "targets": [{"binding": {"instance": "band", "anchor": {"kind": "track", "name": "Stems grp#"},
                                                  "path": "mixer_device volume"},
                                      "prop": "value", "scale": "cc_linear"}]}]}],
                 "weight": 0.8}]},
            {"id": "conf", "title": "Conf", "rows": [{"sections": [
                {"kind": "group", "id": "conf-1", "controls": [{"kind": "text", "text": "unfold_band: 'Vocals Repro grp#'"}]}]}]}
        ],
        "global": [
            {"kind": "alert", "binding": track("TechAlert #"), "period_ms": 300, "label": "TechAlert"}
        ],
        "config": {"fader_shaping": true},
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

fn foh_row(v: &mut Value, row: usize) -> &mut Value {
    &mut v["pages"][1]["rows"][row]["sections"]
}

#[test]
fn the_sample_parses_validates_and_round_trips() {
    let layout = parse(sample());
    assert_eq!(layout.validate(), vec![]);
    assert_eq!(layout.default_page, "foh");
    let foh = &layout.pages[1];
    assert_eq!(foh.rail.len(), 4);
    assert_eq!(foh.rows.len(), 2);
    assert_eq!(foh.rows[0].weight, 1.0);
    assert_eq!(foh.rows[1].weight, 0.8);
    let pager = foh.pager().expect("the nested pager");
    assert_eq!(pager.id, "foh-pager");
    assert_eq!(pager.pages.len(), 2);
    assert_eq!(layout.pages[0].pager(), None);
    // Defaults are omitted, so the file reads back exactly as written.
    assert_eq!(serde_json::to_value(&layout).unwrap(), sample());
}

#[test]
fn defaults_are_filled_and_omitted() {
    let layout = parse(json!({
        "schema": 2, "default_page": "p",
        "pages": [{"id": "p", "title": "P", "rows": [{"sections": [{"kind": "group", "controls": [
            {"kind": "strip", "binding": track("A"), "strip_kind": "standard"},
            {"kind": "stage", "binding": track("M")}]}]}]}]
    }));
    assert_eq!(layout.validate(), vec![]);
    let page = &layout.pages[0];
    assert!(page.rail.is_empty());
    assert_eq!(page.rows[0].weight, 1.0);
    let Section::Group(group) = &page.rows[0].sections[0] else {
        panic!("a group")
    };
    assert_eq!(group.id, None);
    let Control::Strip(strip) = &group.controls[0] else {
        panic!("a strip")
    };
    assert!(!strip.wide);
    assert!(!strip.mute_guard);
    assert_eq!(layout.global, vec![]);
    assert_eq!(layout.config, LayoutConfig::default());
    let back = serde_json::to_value(&layout).unwrap();
    assert!(back.get("global").is_none(), "{back}");
    assert!(back["pages"][0].get("rail").is_none(), "{back}");
    assert!(
        back["pages"][0]["rows"][0].get("weight").is_none(),
        "{back}"
    );
    let strip = &back["pages"][0]["rows"][0]["sections"][0]["controls"][0];
    assert!(
        strip.get("wide").is_none() && strip.get("mute_guard").is_none(),
        "{strip}"
    );
    let stage = &back["pages"][0]["rows"][0]["sections"][0]["controls"][1];
    assert!(
        stage.get("aut").is_none() && stage.get("label").is_none(),
        "{stage}"
    );
    // An alert without a guard; with one, it is written.
    let mut v = sample();
    let unguarded = serde_json::to_value(parse(v.clone())).unwrap();
    assert!(unguarded["global"][0].get("mute_guard").is_none());
    v["global"][0]["mute_guard"] = json!(true);
    let guarded = parse(v);
    assert!(matches!(
        guarded.global[0],
        Control::Alert {
            mute_guard: true,
            ..
        }
    ));
    assert_eq!(
        serde_json::to_value(&guarded).unwrap()["global"][0]["mute_guard"],
        json!(true)
    );
    // A weight other than 1 is written.
    let mut v = sample();
    v["pages"][1]["rows"][0]["weight"] = json!(2.0);
    let back = serde_json::to_value(parse(v)).unwrap();
    assert_eq!(back["pages"][1]["rows"][0]["weight"], json!(2.0));
}

#[test]
fn an_unknown_kind_does_not_parse() {
    let mut v = sample();
    v["global"][0]["kind"] = json!("battery");
    assert!(serde_json::from_value::<Layout>(v).is_err());
    // REFRESH ALL is gone (#58): the hub keeps every binding current itself.
    let mut v = sample();
    v["global"]
        .as_array_mut()
        .unwrap()
        .push(json!({"kind": "refresh", "label": "REFRESH ALL"}));
    let refused = serde_json::from_value::<Layout>(v).unwrap_err();
    assert!(
        refused.to_string().contains("unknown variant `refresh`"),
        "{refused}"
    );
    let mut v = sample();
    foh_row(&mut v, 1)[0]["kind"] = json!("area");
    assert!(serde_json::from_value::<Layout>(v).is_err());
    let mut v = sample();
    foh_row(&mut v, 1)[0]["controls"][0]["strip_kind"] = json!("narrow");
    assert!(serde_json::from_value::<Layout>(v).is_err());
}

/// Every object of the sample that denies unknown fields, by number.
fn place(v: &mut Value, i: usize) -> &mut Value {
    match i {
        0 => v,
        1 => &mut v["pages"][1],
        2 => &mut v["pages"][1]["rows"][0],
        3 => &mut v["pages"][1]["rows"][0]["sections"][0],
        4 => &mut v["pages"][1]["rows"][0]["sections"][0]["pages"][0],
        5 => &mut v["pages"][1]["rows"][0]["sections"][1],
        6 => &mut v["pages"][1]["rows"][0]["sections"][1]["controls"][0],
        7 => &mut v["pages"][1]["rail"][0],
        8 => &mut v["pages"][1]["rail"][1],
        9 => &mut v["pages"][1]["rail"][3],
        10 => &mut v["global"][0],
        _ => &mut v["pages"][2]["rows"][0]["sections"][0]["controls"][0],
    }
}

#[test]
fn unknown_fields_do_not_parse() {
    for i in 0..=11 {
        let mut v = sample();
        place(&mut v, i)["frame"] = json!({"x": 0});
        assert!(serde_json::from_value::<Layout>(v).is_err(), "place {i}");
    }
}

#[test]
fn the_schema_pages_and_default_pages_are_checked() {
    let mut v = sample();
    v["schema"] = json!(1);
    v["default_page"] = json!("worship");
    assert_eq!(
        errors(v),
        vec![
            "schema: schema 1 is not 2".to_string(),
            r#"default_page: "worship" is not a page"#.to_string(),
        ]
    );
    let mut v = sample();
    v["pages"] = json!([]);
    assert_eq!(errors(v), vec!["pages: no pages".to_string()]);
    let mut v = sample();
    foh_row(&mut v, 0)[0]["default_page"] = json!("band-b");
    assert_eq!(
        errors(v),
        vec![r#"pages[1].rows[0].sections[0]: default page "band-b" is not a page"#.to_string()]
    );
    let mut v = sample();
    foh_row(&mut v, 0)[0]["pages"] = json!([]);
    assert_eq!(
        errors(v),
        vec!["pages[1].rows[0].sections[0]: no pages".to_string()]
    );
}

#[test]
fn ids_are_unique_across_pages_sub_pages_and_sections() {
    // A sub-page named like a top-level page.
    let mut v = sample();
    foh_row(&mut v, 0)[0]["pages"][1]["id"] = json!("conf");
    assert_eq!(
        errors(v),
        vec![r#"pages[2]: duplicate id "conf""#.to_string()]
    );
    // Two groups, and a group named like the pager.
    let mut v = sample();
    foh_row(&mut v, 1)[0]["id"] = json!("foh-2");
    foh_row(&mut v, 0)[1]["id"] = json!("foh-pager");
    assert_eq!(
        errors(v),
        vec![r#"pages[1].rows[0].sections[1]: duplicate id "foh-pager""#.to_string()]
    );
    let mut v = sample();
    foh_row(&mut v, 1)[0]["id"] = json!("foh-2");
    assert_eq!(
        errors(v),
        vec![r#"pages[1].rows[1].sections[0]: duplicate id "foh-2""#.to_string()]
    );
    // An empty id is never allowed.
    let mut v = sample();
    v["pages"][0]["id"] = json!("");
    v["default_page"] = json!("foh");
    foh_row(&mut v, 0)[0]["id"] = json!("");
    assert_eq!(
        errors(v),
        vec![
            "pages[0]: empty id".to_string(),
            "pages[1].rows[0].sections[0]: empty id".to_string(),
        ]
    );
}

#[test]
fn a_page_has_at_most_one_pager_and_a_pager_holds_none() {
    let mut v = sample();
    let mut second = foh_row(&mut v, 0)[0].clone();
    second["id"] = json!("foh-pager-2");
    second["pages"][0]["id"] = json!("stage-2");
    second["pages"][1]["id"] = json!("others-2");
    second["default_page"] = json!("stage-2");
    second["pages"][0]["sections"] = json!([]);
    foh_row(&mut v, 1)
        .as_array_mut()
        .unwrap()
        .push(second.clone());
    assert_eq!(
        errors(v),
        vec!["pages[1].rows[1].sections[1]: a second pager on the page".to_string()]
    );
    let mut v = sample();
    foh_row(&mut v, 0)[0]["pages"][1]["sections"] = json!([second]);
    assert_eq!(
        errors(v),
        vec![
            "pages[1].rows[0].sections[0].pages[1].sections[0]: a pager inside a pager".to_string()
        ]
    );
    // One pager on each of two pages is fine.
    let mut v = sample();
    v["pages"][0]["rows"][0]["sections"]
        .as_array_mut()
        .unwrap()
        .push(second);
    assert_eq!(errors(v), Vec::<String>::new());
}

#[test]
fn a_row_weight_is_above_zero() {
    for bad in [json!(0.0), json!(-1.0)] {
        let mut v = sample();
        v["pages"][1]["rows"][1]["weight"] = bad.clone();
        assert_eq!(
            errors(v),
            vec![format!(
                "pages[1].rows[1]: weight {} is not above 0",
                bad.as_f64().unwrap()
            )]
        );
    }
    // Not writable in JSON, but a layout built in code is checked too.
    let mut layout = parse(sample());
    layout.pages[1].rows[1].weight = f64::NAN;
    let errors: Vec<String> = layout.validate().iter().map(|e| e.to_string()).collect();
    assert_eq!(errors, vec!["pages[1].rows[1]: weight NaN is not above 0"]);
    let mut layout = parse(sample());
    layout.pages[1].rows[1].weight = 0.001;
    assert_eq!(layout.validate(), vec![]);
}

#[test]
fn a_bad_path_is_an_error() {
    let mut v = sample();
    foh_row(&mut v, 1)[0]["controls"][1]["targets"][0]["binding"]["path"] =
        json!("devices[name=Latencies parameters 1");
    assert_eq!(
        errors(v),
        vec![
            "pages[1].rows[1].sections[0].controls[1].targets[0]: path: syntax: unterminated [name="
                .to_string()
        ]
    );
}

#[test]
fn param_targets_need_a_prop_and_their_values_or_scale() {
    let mut v = sample();
    v["pages"][1]["rail"][3]["targets"][0]["prop"] = json!("");
    v["pages"][1]["rail"][3]["targets"][0]
        .as_object_mut()
        .unwrap()
        .remove("off");
    foh_row(&mut v, 1)[0]["controls"][1]["targets"] = json!([]);
    assert_eq!(
        errors(v),
        vec![
            "pages[1].rail[3].targets[0]: target without a prop".to_string(),
            "pages[1].rail[3].targets[0]: a toggle target needs on and off".to_string(),
            "pages[1].rows[1].sections[0].controls[1]: no targets".to_string(),
        ]
    );
    let mut v = sample();
    foh_row(&mut v, 1)[0]["controls"][1]["targets"][0]
        .as_object_mut()
        .unwrap()
        .remove("scale");
    v["pages"][1]["rail"][3]["targets"][0]
        .as_object_mut()
        .unwrap()
        .remove("on");
    assert_eq!(
        errors(v),
        vec![
            "pages[1].rail[3].targets[0]: a toggle target needs on and off".to_string(),
            "pages[1].rows[1].sections[0].controls[1].targets[0]: a fader target needs a scale"
                .to_string(),
        ]
    );
    let mut v = sample();
    v["pages"][0]["rows"][0]["sections"][0]["controls"][0]["targets"] = json!([]);
    assert_eq!(
        errors(v),
        vec!["pages[0].rows[0].sections[0].controls[0]: no targets".to_string()]
    );
}

#[test]
fn bindings_need_an_instance_and_a_name() {
    let mut v = sample();
    v["pages"][1]["rail"][2]["binding"]["instance"] = json!("");
    v["global"][0]["binding"]["anchor"] = json!({"kind": "return", "name": ""});
    v["pages"][1]["rail"][0]["binding"]["anchor"] = json!({"kind": "track", "name": ""});
    foh_row(&mut v, 1)[0]["controls"][0]["binding"]["instance"] = json!("");
    assert_eq!(
        errors(v),
        vec![
            "pages[1].rail[0].binding: anchor without a name".to_string(),
            "pages[1].rail[2].binding: binding without an instance".to_string(),
            "pages[1].rows[1].sections[0].controls[0].binding: binding without an instance"
                .to_string(),
            "global[0].binding: anchor without a name".to_string(),
        ]
    );
}

#[test]
fn colors_alerts_and_hub_values_are_checked() {
    let mut v = sample();
    foh_row(&mut v, 1)[0]["color"] = json!("grey");
    v["pages"][1]["rail"][3]["color"] = json!("#F3942");
    v["global"][0]["period_ms"] = json!(0);
    v["pages"][1]["rail"][1]["key"] = json!("battery");
    assert_eq!(
        errors(v),
        vec![
            r#"pages[1].rail[1]: unknown hub value "battery""#.to_string(),
            r##"pages[1].rail[3].color: "#F3942" is not #RRGGBB or #RRGGBBAA"##.to_string(),
            r#"pages[1].rows[1].sections[0].color: "grey" is not #RRGGBB or #RRGGBBAA"#.to_string(),
            "global[0]: alert period 0 ms".to_string(),
        ]
    );
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
            "live_set tracks[name=Vox 1 stream]",
            "live_set tracks[name=Mics Stage #]",
            "live_set tracks[name=Vocals Repro grp#]",
            "live_set tracks[name=Rev #]",
            "live_set tracks[name=Klavir #]",
            "live_set tracks[name=Podklady #]",
            "live_set return_tracks[name=A-Rev]",
            "live_set tracks[name=Stems grp#] mixer_device volume",
            "live_set tracks[name=TechAlert #]",
        ]
    );
    // Every sub-page of the pager counts, not only the one shown first.
    let mut v = sample();
    foh_row(&mut v, 0)[0]["pages"][1]["sections"] = json!([{"kind": "group", "controls": [
        {"kind": "strip", "binding": track("Hand1 #"), "strip_kind": "standard"}]}]);
    let layout = parse(v);
    let names: Vec<String> = layout
        .bindings()
        .iter()
        .map(|b| b.target().unwrap())
        .collect();
    assert_eq!(names[5], "live_set tracks[name=Hand1 #]");
    // 11 on the pages, TechAlert on every page (#58: REFRESH ALL is gone).
    assert_eq!(layout.controls().len(), 12);
}

#[test]
fn a_page_lists_its_controls_rail_first() {
    let layout = parse(sample());
    let kinds: Vec<&str> = layout.pages[1]
        .controls()
        .into_iter()
        .map(|c| match c {
            Control::Strip(_) => "strip",
            Control::Solo { .. } => "solo",
            Control::Stage { .. } => "stage",
            Control::HubToggle { .. } => "hub_toggle",
            Control::ParamToggle { .. } => "param_toggle",
            Control::ParamFader { .. } => "param_fader",
            Control::Alert { .. } => "alert",
            Control::Text { .. } => "text",
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            "stage",
            "hub_toggle",
            "solo",
            "param_toggle",
            "strip",
            "strip",
            "strip",
            "param_fader"
        ]
    );
    let section = &layout.pages[1].rows[0].sections[0];
    assert_eq!(section.groups().len(), 1);
    assert_eq!(layout.pages[1].rows[0].sections[1].groups().len(), 1);
    assert_eq!(
        layout.pages[2].controls()[0].bindings(),
        Vec::<&Binding>::new()
    );
    assert_eq!(layout.pages[2].controls().len(), 1);
}

#[test]
fn the_strip_tracks_are_each_track_a_strip_shows_once() {
    let mut v = sample();
    // The same track twice, on another sub-page too: listed once.
    foh_row(&mut v, 0)[0]["pages"][1]["sections"] = json!([{"kind": "group", "controls": [
        {"kind": "strip", "binding": track("Podklady #"), "strip_kind": "standard"}]}]);
    let layout = parse(v);
    // The return strip and the other controls' tracks are not strips' tracks.
    assert_eq!(
        layout.strip_tracks(),
        vec![
            ("band".to_string(), "Klavir #".to_string()),
            ("band".to_string(), "Podklady #".to_string()),
        ]
    );
}

#[test]
fn the_stage_aut_binding_is_the_first_stage_control_with_aut() {
    let layout = parse(sample());
    assert_eq!(
        layout.stage_aut_binding().unwrap().target().unwrap(),
        "live_set tracks[name=Mics Stage #]"
    );
    // A stage control without aut is skipped; one in a sub-page counts.
    let mut v = sample();
    v["pages"][1]["rail"][0]["aut"] = json!(false);
    foh_row(&mut v, 0)[0]["pages"][1]["sections"] = json!([{"kind": "group", "controls": [
        {"kind": "stage", "binding": track("Sub stage"), "aut": true}]}]);
    assert_eq!(
        parse(v).stage_aut_binding().unwrap().target().unwrap(),
        "live_set tracks[name=Sub stage]"
    );
    // Then the global controls.
    let mut v = sample();
    v["pages"][1]["rail"][0]["aut"] = json!(false);
    v["global"] = json!([{"kind": "stage", "binding": track("Global stage"), "aut": true}]);
    assert_eq!(
        parse(v).stage_aut_binding().unwrap().target().unwrap(),
        "live_set tracks[name=Global stage]"
    );
    let mut v = sample();
    v["pages"][1]["rail"][0]["aut"] = json!(false);
    assert_eq!(parse(v).stage_aut_binding(), None);
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
    assert_eq!(titles, vec!["Cue", "FOH"], "no Conf page (#58)");
    assert_eq!(layout.default_page, layout.pages[1].id);
    let pager = layout.pages[1].pager().expect("the nested pager");
    let sub: Vec<&str> = pager.pages.iter().map(|p| p.title.as_str()).collect();
    assert_eq!(sub, vec!["STAGE", "OTHERS"]);
    assert!(!layout.pages[1].rail.is_empty());
    assert_eq!(
        layout.stage_aut_binding().unwrap().target().unwrap(),
        "live_set tracks[name=Mics Stage #]"
    );
    let global: Vec<&str> = layout
        .global
        .iter()
        .map(|c| match c {
            Control::Alert { .. } => "alert",
            _ => "other",
        })
        .collect();
    assert_eq!(global, vec!["alert"]);
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
