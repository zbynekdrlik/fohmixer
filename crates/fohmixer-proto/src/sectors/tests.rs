use super::*;
use crate::markers::{Found, TrackKind, compose};
use serde_json::{Value, json};

fn track(name: &str) -> Value {
    json!({"kind": "track", "name": name})
}

/// A standard strip of `instance` at `anchor`.
fn strip(instance: &str, anchor: Value) -> Value {
    json!({"kind": "strip", "binding": {"instance": instance, "anchor": anchor},
           "strip_kind": "standard"})
}

fn control(value: Value) -> Control {
    serde_json::from_value(value).expect("the control parses")
}

/// A layout whose default page `foh` has these rows (and these pages after
/// it).
fn layout_of(rows: Value, more: Value) -> Layout {
    let mut pages = vec![json!({"id": "foh", "title": "FOH", "rows": rows})];
    pages.extend(more.as_array().cloned().unwrap_or_default());
    serde_json::from_value(json!({"schema": 2, "default_page": "foh", "pages": pages}))
        .expect("the layout parses")
}

fn group(id: &str, title: Option<&str>, controls: Value) -> Value {
    match title {
        Some(title) => json!({"kind": "group", "id": id, "title": title, "controls": controls}),
        None => json!({"kind": "group", "id": id, "controls": controls}),
    }
}

fn r(instance: &str, label: &str) -> StripRef {
    StripRef {
        instance: instance.to_string(),
        label: label.to_string(),
    }
}

fn band(label: &str) -> StripRef {
    r("band", label)
}

fn sl(id: &str, title: &str, strips: Vec<StripRef>) -> SectorLayout {
    SectorLayout {
        id: id.to_string(),
        title: title.to_string(),
        strips,
    }
}

fn doc(rev: u64, sectors: Vec<(SectorId, Vec<SectorLayout>)>) -> SectorsDoc {
    SectorsDoc {
        rev,
        sectors: sectors.into_iter().collect(),
    }
}

/// One row of band strips `S1`..`S6` and a wide `W`, then `Bass`.
fn units_layout() -> Layout {
    let mut controls: Vec<Value> = (1..=6)
        .map(|n| strip("band", track(&format!("S{n}"))))
        .collect();
    let mut wide = strip("band", track("W"));
    wide["wide"] = json!(true);
    controls.push(wide);
    controls.push(strip("band", track("Bass")));
    layout_of(
        json!([{"sections": [group("g", None, Value::Array(controls))]}]),
        json!([]),
    )
}

fn ids(pool: &[Pooled]) -> Vec<StripRef> {
    pool.iter().map(|p| p.id.clone()).collect()
}

#[test]
fn the_sector_ids_are_lu_ld_ru_rd() {
    assert_eq!(
        serde_json::to_value(SectorId::ALL).unwrap(),
        json!(["lu", "ld", "ru", "rd"])
    );
    let names: Vec<&str> = SectorId::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(names, ["lu", "ld", "ru", "rd"]);
    let left: Vec<bool> = SectorId::ALL.iter().map(|s| s.left()).collect();
    assert_eq!(left, [true, true, false, false]);
    let up: Vec<bool> = SectorId::ALL.iter().map(|s| s.up()).collect();
    assert_eq!(up, [true, false, true, false]);
    for sector in SectorId::ALL {
        assert_eq!(SectorId::of(sector.up(), sector.left()), sector);
    }
}

#[test]
fn the_empty_document_parses_and_is_off() {
    let empty: SectorsDoc = serde_json::from_str(r#"{"rev": 0, "sectors": {}}"#).unwrap();
    assert_eq!(empty, SectorsDoc::empty());
    assert!(!empty.is_on());
    assert_eq!(empty.layouts(SectorId::Lu), &[] as &[SectorLayout]);
    // A sector with an empty list is still off.
    let listed: SectorsDoc = serde_json::from_str(r#"{"rev": 3, "sectors": {"lu": []}}"#).unwrap();
    assert!(!listed.is_on());
    let on = doc(1, vec![(SectorId::Rd, vec![sl("rd-1", "MAIN", vec![])])]);
    assert!(on.is_on());
    assert_eq!(on.layouts(SectorId::Rd), &[sl("rd-1", "MAIN", vec![])]);
    assert_eq!(on.layouts(SectorId::Lu), &[] as &[SectorLayout]);
}

#[test]
fn an_unknown_field_or_sector_does_not_parse() {
    let good = json!({"rev": 1, "sectors": {"lu": [{"id": "lu-1", "title": "VOX",
        "strips": [{"instance": "band", "label": "Vox 1"}]}]}});
    assert!(serde_json::from_value::<SectorsDoc>(good.clone()).is_ok());
    let mut top = good.clone();
    top["x"] = json!(1);
    let mut layout = good.clone();
    layout["sectors"]["lu"][0]["x"] = json!(1);
    let mut strip_ref = good.clone();
    strip_ref["sectors"]["lu"][0]["strips"][0]["x"] = json!(1);
    let sector = json!({"rev": 1, "sectors": {"lx": []}});
    for bad in [top, layout, strip_ref, sector] {
        assert!(
            serde_json::from_value::<SectorsDoc>(bad.clone()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn validate_refuses_a_duplicate_id_across_sectors() {
    let twice = doc(
        1,
        vec![
            (SectorId::Lu, vec![sl("a", "ONE", vec![])]),
            (SectorId::Rd, vec![sl("a", "TWO", vec![])]),
        ],
    );
    assert_eq!(
        twice.validate(),
        vec![r#"the layout id "a" is used 2 times"#.to_string()]
    );
    let fine = doc(
        1,
        vec![
            (SectorId::Lu, vec![sl("a", "ONE", vec![])]),
            (SectorId::Rd, vec![sl("b", "TWO", vec![])]),
        ],
    );
    assert_eq!(fine.validate(), Vec::<String>::new());
}

#[test]
fn validate_takes_8_characters_and_refuses_9() {
    let of = |title: &str| doc(1, vec![(SectorId::Lu, vec![sl("lu-1", title, vec![])])]);
    assert_eq!(of("ABCDEFGH").validate(), Vec::<String>::new());
    assert_eq!(
        of("Klavír12").validate(),
        Vec::<String>::new(),
        "8 characters, 9 bytes"
    );
    assert_eq!(
        of("ABCDEFGHI").validate(),
        vec![r#"layout "lu-1": its title "ABCDEFGHI" is longer than 8 characters"#.to_string()]
    );
}

#[test]
fn validate_refuses_an_empty_id_or_title_and_a_strip_twice() {
    let empty_id = doc(1, vec![(SectorId::Lu, vec![sl(" ", "VOX", vec![])])]);
    assert_eq!(
        empty_id.validate(),
        vec!["a layout has an empty id".to_string()]
    );
    let empty_title = doc(1, vec![(SectorId::Lu, vec![sl("lu-1", " ", vec![])])]);
    assert_eq!(
        empty_title.validate(),
        vec![r#"layout "lu-1" has an empty title"#.to_string()]
    );
    let twice = doc(
        1,
        vec![(
            SectorId::Lu,
            vec![sl(
                "lu-1",
                "VOX",
                vec![band("Vox 1"), band("Vox 2"), band("Vox 1")],
            )],
        )],
    );
    assert_eq!(
        twice.validate(),
        vec![r#"layout "lu-1" holds band|Vox 1 twice"#.to_string()]
    );
    // The same strip in two layouts is fine.
    let two = doc(
        1,
        vec![(
            SectorId::Lu,
            vec![
                sl("lu-1", "VOX", vec![band("Vox 1")]),
                sl("lu-2", "ALL", vec![band("Vox 1")]),
            ],
        )],
    );
    assert_eq!(two.validate(), Vec::<String>::new());
}

#[test]
fn a_marker_strip_is_known_by_its_label() {
    let mut marker = strip("band", json!({"kind": "track_at", "index": 3}));
    marker["label"] = json!("Vox 1");
    assert_eq!(strip_ref(&control(marker)), Some(band("Vox 1")));
    // A labelled frame strip too: its label, never its track's name.
    let mut labelled = strip("master", track("Klavir #"));
    labelled["label"] = json!("Klavir");
    assert_eq!(strip_ref(&control(labelled)), Some(r("master", "Klavir")));
}

#[test]
fn a_placeholder_or_conflict_strip_has_no_ref() {
    let mut placeholder = strip("band", json!({"kind": "track_at", "index": 3}));
    placeholder["label"] = json!("#4");
    assert_eq!(strip_ref(&control(placeholder)), None);
    let mut conflict = strip("band", json!({"kind": "track_at", "index": 3}));
    conflict["label"] = json!("Vox 1");
    conflict["mark"] = json!("conflict");
    assert_eq!(strip_ref(&control(conflict)), None);
    // Another problem leaves it working, and known.
    let mut problem = strip("band", json!({"kind": "track_at", "index": 3}));
    problem["label"] = json!("Vox 1");
    problem["mark"] = json!("problem");
    assert_eq!(strip_ref(&control(problem)), Some(band("Vox 1")));
    // A marker strip without a label has no name to go by.
    let bare = strip("band", json!({"kind": "track_at", "index": 3}));
    assert_eq!(strip_ref(&control(bare)), None);
}

#[test]
fn a_frame_strip_takes_its_label_else_its_anchor_name() {
    assert_eq!(
        strip_ref(&control(strip("band", track("Klavir")))),
        Some(band("Klavir"))
    );
    assert_eq!(
        strip_ref(&control(strip(
            "master",
            json!({"kind": "return", "name": "A-Rev"})
        ))),
        Some(r("master", "A-Rev"))
    );
    assert_eq!(
        strip_ref(&control(strip("master", json!({"kind": "master"})))),
        Some(r("master", "Master"))
    );
    assert_eq!(
        strip_ref(&control(strip("band", json!({"kind": "song"})))),
        None
    );
}

#[test]
fn a_labelled_param_fader_is_in_the_pool() {
    let linked = json!({"kind": "param_fader", "label": "Podklady",
                        "link": {"instance": "band", "id": "POD"}});
    assert_eq!(strip_ref(&control(linked.clone())), Some(band("Podklady")));
    let targeted = json!({"kind": "param_fader", "label": "Stems",
        "targets": [{"binding": {"instance": "master", "anchor": track("Stems #"),
                                 "path": "mixer_device volume"},
                     "prop": "value", "scale": "cc_linear"}]});
    assert_eq!(strip_ref(&control(targeted)), Some(r("master", "Stems")));
    let nowhere = json!({"kind": "param_fader", "label": "Lost"});
    assert_eq!(strip_ref(&control(nowhere)), None);
    let toggle = json!({"kind": "param_toggle", "label": "REVERB", "press": "toggle",
        "targets": [{"binding": {"instance": "band", "anchor": track("Rev #")},
                     "prop": "mute", "on": false, "off": true}]});
    assert_eq!(strip_ref(&control(toggle)), None);
    let solo = json!({"kind": "solo", "binding": {"instance": "band", "anchor": track("Vocals grp#")},
                      "label": "Vocals"});
    assert_eq!(strip_ref(&control(solo)), None);
    let layout = layout_of(
        json!([{"sections": [group("g", None, json!([linked]))]}]),
        json!([]),
    );
    let pooled = pool(&layout);
    assert_eq!(ids(&pooled), vec![band("Podklady")]);
    assert_eq!(pooled[0].units(), 1.0);
    assert_eq!(pooled[0].strip(), None);
}

#[test]
fn the_pool_holds_each_ref_once_in_document_order() {
    let mut wide_again = strip("band", track("Vox 1"));
    wide_again["wide"] = json!(true);
    let solo = json!({"kind": "solo", "binding": {"instance": "band", "anchor": track("Vocals grp#")},
                      "label": "Vocals"});
    let layout = layout_of(
        json!([{"sections": [group("g", None, json!([strip("band", track("Vox 1")), solo]))]}]),
        json!([{"id": "view-X", "title": "X", "view": true, "rows": [{"sections": [
            group("v", None, json!([wide_again, strip("band", track("Bass"))]))]}]}]),
    );
    let pooled = pool(&layout);
    assert_eq!(ids(&pooled), vec![band("Vox 1"), band("Bass")]);
    assert_eq!(pooled[0].units(), 1.0, "the first place wins");
    assert!(pooled[0].strip().is_some_and(|s| !s.wide));
}

#[test]
fn units_count_a_wide_strip_1_1_and_full_is_past_6() {
    let layout = units_layout();
    let pooled = pool(&layout);
    let six: Vec<StripRef> = (1..=6).map(|n| band(&format!("S{n}"))).collect();
    assert_eq!(units(&six, &pooled), 6.0);
    assert!(!is_full(units(&six, &pooled)));
    let mut wide: Vec<StripRef> = (1..=5).map(|n| band(&format!("S{n}"))).collect();
    wide.push(band("W"));
    assert_eq!(units(&wide, &pooled), 6.1);
    assert!(is_full(units(&wide, &pooled)));
    assert!(!is_full(6.0));
    assert!(is_full(6.0f64.next_up()));
    let five = sl(
        "lu-1",
        "A",
        (1..=5).map(|n| band(&format!("S{n}"))).collect(),
    );
    assert!(!would_be_full(&five, &band("S6"), &pooled));
    assert!(would_be_full(&five, &band("W"), &pooled));
}

#[test]
fn a_ref_no_pool_strip_carries_takes_no_room_and_counts_unknown() {
    let layout = units_layout();
    let pooled = pool(&layout);
    let mut strips: Vec<StripRef> = (1..=6).map(|n| band(&format!("S{n}"))).collect();
    strips.push(band("Renamed"));
    assert_eq!(units(&strips, &pooled), 6.0);
    let held = doc(
        1,
        vec![
            (SectorId::Lu, vec![sl("lu-1", "A", strips)]),
            (
                SectorId::Rd,
                vec![sl(
                    "rd-1",
                    "B",
                    vec![band("Renamed"), band("Gone"), band("Bass")],
                )],
            ),
        ],
    );
    assert_eq!(unknown(&held, &pooled), 2, "each ref once");
    assert_eq!(held.layout_count(), 2);
    assert_eq!(held.strip_count(), 9);
}

/// lu-1 [Vox 1] and lu-2 [] over a pool of band strips.
fn ticked() -> (Layout, SectorsDoc) {
    let names = [
        "Vox 1", "Vox 2", "Vox 3", "Vox 4", "Klavir", "Bass", "Keys 1",
    ];
    let controls: Vec<Value> = names.iter().map(|n| strip("band", track(n))).collect();
    let layout = layout_of(
        json!([{"sections": [group("g", None, Value::Array(controls))]}]),
        json!([]),
    );
    let doc = doc(
        4,
        vec![(
            SectorId::Lu,
            vec![
                sl("lu-1", "VOX", vec![band("Vox 1")]),
                sl("lu-2", "ALL", vec![]),
            ],
        )],
    );
    (layout, doc)
}

#[test]
fn a_tick_on_appends_at_the_end_and_bumps_rev() {
    let (layout, before) = ticked();
    let pooled = pool(&layout);
    let after = tick(&before, "lu-1", &band("Bass"), true, &pooled)
        .unwrap()
        .unwrap();
    assert_eq!(after.rev, 5);
    assert_eq!(
        after.layouts(SectorId::Lu),
        &[
            sl("lu-1", "VOX", vec![band("Vox 1"), band("Bass")]),
            sl("lu-2", "ALL", vec![]),
        ]
    );
}

#[test]
fn a_tick_off_removes_even_a_ref_the_pool_lost() {
    let (layout, mut before) = ticked();
    before.sectors.insert(
        SectorId::Rd,
        vec![sl("rd-1", "B", vec![band("Gone"), band("Bass")])],
    );
    let pooled = pool(&layout);
    let after = tick(&before, "rd-1", &band("Gone"), false, &pooled)
        .unwrap()
        .unwrap();
    assert_eq!(after.rev, 5);
    assert_eq!(
        after.layouts(SectorId::Rd),
        &[sl("rd-1", "B", vec![band("Bass")])]
    );
    assert_eq!(after.layouts(SectorId::Lu), before.layouts(SectorId::Lu));
}

#[test]
fn a_tick_that_changes_nothing_is_none() {
    let (layout, before) = ticked();
    let pooled = pool(&layout);
    assert_eq!(
        tick(&before, "lu-1", &band("Vox 1"), true, &pooled),
        Ok(None)
    );
    assert_eq!(
        tick(&before, "lu-1", &band("Bass"), false, &pooled),
        Ok(None)
    );
    assert_eq!(
        tick(&before, "lu-2", &band("Gone"), false, &pooled),
        Ok(None)
    );
}

#[test]
fn a_tick_is_refused_for_an_unknown_layout_a_strip_not_in_the_pool_or_a_full_layout() {
    let (layout, mut before) = ticked();
    let pooled = pool(&layout);
    assert_eq!(
        tick(&before, "x", &band("Bass"), true, &pooled),
        Err(Refusal::UnknownLayout)
    );
    assert_eq!(
        tick(&before, "x", &band("Bass"), false, &pooled),
        Err(Refusal::UnknownLayout)
    );
    assert_eq!(
        tick(&before, "lu-1", &band("Nobody"), true, &pooled),
        Err(Refusal::NotInPool)
    );
    let full = ["Vox 1", "Vox 2", "Vox 3", "Vox 4", "Klavir", "Keys 1"];
    before.sectors.insert(
        SectorId::Lu,
        vec![sl("lu-1", "VOX", full.iter().map(|n| band(n)).collect())],
    );
    assert_eq!(
        tick(&before, "lu-1", &band("Bass"), true, &pooled),
        Err(Refusal::Full)
    );
    assert_eq!(
        tick(&before, "lu-1", &band("Klavir"), true, &pooled),
        Ok(None),
        "a strip already in a full layout"
    );
    let off = tick(&before, "lu-1", &band("Klavir"), false, &pooled)
        .unwrap()
        .unwrap();
    assert_eq!(off.layouts(SectorId::Lu)[0].strips.len(), 5);
}

#[test]
fn the_refusals_read_as_the_hub_sends_them() {
    assert_eq!(Refusal::UnknownLayout.text(), "unknown layout");
    assert_eq!(Refusal::NotInPool.text(), "not in the pool");
    assert_eq!(Refusal::Full.text(), "FULL");
    assert_eq!(Refusal::Stale.text(), "stale");
    assert_eq!(Refusal::NothingToUndo.text(), "nothing to undo");
    assert_eq!(Refusal::NoLayout.text(), "no layout served");
}

/// lu-1 [Vox 1, Vox 2, Vox 3] and ru-1 [Klavir, Bass] at revision 7.
fn ordered() -> SectorsDoc {
    doc(
        7,
        vec![
            (
                SectorId::Lu,
                vec![sl(
                    "lu-1",
                    "VOX",
                    vec![band("Vox 1"), band("Vox 2"), band("Vox 3")],
                )],
            ),
            (
                SectorId::Ru,
                vec![sl("ru-1", "BAND", vec![band("Klavir"), band("Bass")])],
            ),
        ],
    )
}

fn order(layout: &str, strips: &[&str]) -> SectorOrder {
    SectorOrder {
        layout: layout.to_string(),
        strips: strips.iter().map(|s| band(s)).collect(),
    }
}

#[test]
fn orders_take_the_same_set_at_the_documents_rev() {
    let before = ordered();
    let after = orders(
        &before,
        &[
            order("lu-1", &["Vox 3", "Vox 1", "Vox 2"]),
            order("ru-1", &["Bass", "Klavir"]),
        ],
        7,
    )
    .unwrap();
    assert_eq!(after.rev, 8);
    assert_eq!(
        after.layouts(SectorId::Lu),
        &[sl(
            "lu-1",
            "VOX",
            vec![band("Vox 3"), band("Vox 1"), band("Vox 2")]
        )]
    );
    assert_eq!(
        after.layouts(SectorId::Ru),
        &[sl("ru-1", "BAND", vec![band("Bass"), band("Klavir")])]
    );
    let one = orders(&before, &[order("ru-1", &["Bass", "Klavir"])], 7).unwrap();
    assert_eq!(one.layouts(SectorId::Lu), before.layouts(SectorId::Lu));
    assert_eq!(one.rev, 8);
}

#[test]
fn orders_are_stale_for_another_rev_another_set_a_layout_twice_or_an_unknown_layout() {
    let before = ordered();
    let stale = |list: &[SectorOrder], rev: u64| orders(&before, list, rev);
    let good = order("ru-1", &["Bass", "Klavir"]);
    assert_eq!(stale(&[good.clone()], 6), Err(Refusal::Stale));
    assert_eq!(stale(&[good.clone()], 8), Err(Refusal::Stale));
    assert_eq!(
        stale(&[order("ru-1", &["Bass"])], 7),
        Err(Refusal::Stale),
        "one missing"
    );
    assert_eq!(
        stale(&[order("ru-1", &["Bass", "Klavir", "Vox 1"])], 7),
        Err(Refusal::Stale),
        "one more"
    );
    assert_eq!(
        stale(&[order("ru-1", &["Bass", "Bass"])], 7),
        Err(Refusal::Stale),
        "the same length, another set"
    );
    assert_eq!(
        stale(&[good.clone(), order("ru-1", &["Klavir", "Bass"])], 7),
        Err(Refusal::Stale),
        "a layout twice"
    );
    assert_eq!(
        stale(&[good, order("x", &[])], 7),
        Err(Refusal::Stale),
        "an unknown layout"
    );
}

fn fixture() -> Layout {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tools/import-tosc/fixtures/expected-layout.json"
    );
    let text = std::fs::read_to_string(path).expect("the import tool's fixture output");
    serde_json::from_str(&text).expect("it parses")
}

#[test]
fn the_seed_makes_a_pagers_sub_pages_left_layouts_and_the_rest_one_right_layout() {
    let seeded = seed(&fixture());
    assert_eq!(seeded.rev, 1);
    assert_eq!(seeded.validate(), Vec::<String>::new());
    assert_eq!(
        seeded.layouts(SectorId::Lu),
        &[
            sl(
                "lu-1",
                "STAGE",
                vec![band("Vocal 1 repro#"), band("Keys 1")]
            ),
            sl("lu-2", "OTHERS", vec![r("master", "Hand1 #")]),
        ]
    );
    assert_eq!(
        seeded.layouts(SectorId::Ru),
        &[sl(
            "ru-1",
            "MAIN",
            vec![band("B-Main repro #"), band("Hand2 #")]
        )]
    );
    // Row 1 has no pager: EFFECTS 2, the labelled param fader 1, HANDS 2,
    // A-Echo 1; the cut after the fader leaves 3 a side.
    assert_eq!(
        seeded.layouts(SectorId::Ld),
        &[sl(
            "ld-1",
            "EFFECTS",
            vec![
                band("A-Reverb #"),
                r("master", "A-Reverb #"),
                band("Podklady All")
            ]
        )]
    );
    assert_eq!(
        seeded.layouts(SectorId::Rd),
        &[sl(
            "rd-1",
            "HANDS",
            vec![
                r("master", "Hand4 #"),
                r("master", "Hand2 #"),
                r("master", "A-Echo")
            ]
        )]
    );
}

#[test]
fn the_seed_cuts_where_the_larger_side_is_smallest_and_more_left_on_a_tie() {
    assert_eq!(seed_cut(&[]), 0);
    assert_eq!(seed_cut(&[2, 1, 2, 1]), 2);
    assert_eq!(seed_cut(&[2]), 1, "a tie: all left");
    assert_eq!(seed_cut(&[1, 1]), 1);
    assert_eq!(seed_cut(&[3, 1, 1, 1]), 1);
    assert_eq!(seed_cut(&[1, 1, 1, 3]), 3);
    assert_eq!(seed_cut(&[1, 0, 1]), 1, "the first of equal cuts");
}

#[test]
fn the_seed_cuts_a_title_to_8_and_gives_an_empty_sector_main() {
    let pager = json!({"kind": "pager", "id": "pg", "default_page": "a", "pages": [
        {"id": "a", "title": "BACKINGVOX", "sections": [
            group("ga", None, json!([strip("band", track("Klavir")), strip("band", track("Klavir"))]))]},
        {"id": "b", "title": "Klavír12", "sections": []},
        {"id": "c", "title": " ", "sections": []}]});
    let layout = layout_of(json!([{"sections": [pager]}]), json!([]));
    let seeded = seed(&layout);
    assert_eq!(seeded.validate(), Vec::<String>::new());
    assert_eq!(
        seeded.layouts(SectorId::Lu),
        &[
            sl("lu-1", "BACKINGV", vec![band("Klavir")]),
            sl("lu-2", "Klavír12", vec![]),
            sl("lu-3", "MAIN", vec![]),
        ]
    );
    assert_eq!(seeded.layouts(SectorId::Ru), &[sl("ru-1", "MAIN", vec![])]);
    assert_eq!(seeded.layouts(SectorId::Ld), &[sl("ld-1", "MAIN", vec![])]);
    assert_eq!(seeded.layouts(SectorId::Rd), &[sl("rd-1", "MAIN", vec![])]);
    // No default page: every sector one empty MAIN.
    let mut lost = layout.clone();
    lost.default_page = "nowhere".into();
    let empty = seed(&lost);
    for sector in SectorId::ALL {
        assert_eq!(
            empty.layouts(sector),
            &[sl(&format!("{}-1", sector.as_str()), "MAIN", vec![])]
        );
    }
    // Rows past row 1 are ignored.
    let rows = json!([
        {"sections": [group("a", Some("A"), json!([strip("band", track("Vox 1"))]))]},
        {"sections": [group("b", Some("B"), json!([strip("band", track("Vox 2"))]))]},
        {"sections": [group("c", Some("C"), json!([strip("band", track("Vox 3"))]))]}
    ]);
    let three = seed(&layout_of(rows, json!([])));
    assert_eq!(
        three.layouts(SectorId::Lu),
        &[sl("lu-1", "A", vec![band("Vox 1")])]
    );
    assert_eq!(
        three.layouts(SectorId::Ld),
        &[sl("ld-1", "B", vec![band("Vox 2")])]
    );
    assert_eq!(three.strip_count(), 2);
}

#[test]
fn the_seed_of_a_tagged_frame_uses_the_markers_labels() {
    let frame: Layout = serde_json::from_value(json!({
        "schema": 2, "default_page": "foh",
        "pages": [{"id": "foh", "title": "FOH", "rows": [{"sections": [
            {"kind": "group", "id": "vocals", "title": "VOCALS", "tags": "VOCALS"}]}]}]}))
    .unwrap();
    let found = |index: u32, name: &str| Found {
        instance: "band".into(),
        kind: TrackKind::Track,
        index,
        name: name.into(),
        tuners: 1,
    };
    let composed = compose(
        &frame,
        &[
            found(3, r#""Vox 1" +G:VOCALS"#),
            found(1, r#""Vox 2" +G:VOCALS"#),
            // No label: its placeholder `#6` is no identity.
            found(5, "+G:VOCALS"),
        ],
    );
    let seeded = seed(&composed.layout);
    assert_eq!(
        seeded.layouts(SectorId::Lu),
        &[sl("lu-1", "VOCALS", vec![band("Vox 2"), band("Vox 1")])]
    );
    assert_eq!(seeded.layouts(SectorId::Ru), &[sl("ru-1", "MAIN", vec![])]);
    assert_eq!(seeded.validate(), Vec::<String>::new());
}
