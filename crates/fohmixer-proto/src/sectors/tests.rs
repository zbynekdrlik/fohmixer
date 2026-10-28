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

/// These faders as slots, none empty.
fn full(strips: Vec<StripRef>) -> Vec<Slot> {
    strips.into_iter().map(Some).collect()
}

fn sl(id: &str, title: &str, strips: Vec<StripRef>) -> SectorLayout {
    slotted(id, title, full(strips))
}

fn slotted(id: &str, title: &str, strips: Vec<Slot>) -> SectorLayout {
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
    let six: Vec<Slot> = (1..=6).map(|n| Some(band(&format!("S{n}")))).collect();
    assert_eq!(units(&six, &pooled), 6.0);
    assert!(!is_full(units(&six, &pooled)));
    let mut wide: Vec<Slot> = (1..=5).map(|n| Some(band(&format!("S{n}")))).collect();
    wide.push(Some(band("W")));
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
    assert_eq!(units(&full(strips.clone()), &pooled), 6.0);
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
fn a_tick_off_empties_its_slot_even_for_a_ref_the_pool_lost() {
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
    // The slot model (1.24.0): the strip's slot stays, empty, so Bass keeps
    // its place.
    assert_eq!(
        after.layouts(SectorId::Rd),
        &[slotted("rd-1", "B", vec![None, Some(band("Bass"))])]
    );
    // The last strip ticked out leaves no trailing empty slot.
    let last = tick(&after, "rd-1", &band("Bass"), false, &pooled)
        .unwrap()
        .unwrap();
    assert_eq!(last.layouts(SectorId::Rd), &[slotted("rd-1", "B", vec![])]);
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
    assert_eq!(off.layouts(SectorId::Lu)[0].refs().count(), 5);
    // Klavir's slot stays empty: it counts a unit, so Bass still fits (it
    // takes that slot) and a seventh after it does not.
    assert_eq!(off.layouts(SectorId::Lu)[0].strips[4], None);
    let back = tick(&off, "lu-1", &band("Bass"), true, &pooled)
        .unwrap()
        .unwrap();
    assert_eq!(back.layouts(SectorId::Lu)[0].strips[4], Some(band("Bass")));
    assert_eq!(
        tick(&back, "lu-1", &band("Klavir"), true, &pooled),
        Err(Refusal::Full)
    );
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

/// `before`'s sectors with these layouts' slots replaced, `""` an empty
/// slot (ARRANGE's drags).
fn reordered(before: &SectorsDoc, orders: &[(&str, Vec<&str>)]) -> Sectors {
    let mut sectors = before.sectors.clone();
    for (id, labels) in orders {
        for layout in sectors.values_mut().flatten() {
            if layout.id == *id {
                layout.strips = labels
                    .iter()
                    .map(|s| (!s.is_empty()).then(|| band(s)))
                    .collect();
            }
        }
    }
    sectors
}

/// The pool of `ordered`'s strips and a wide `Wide`.
fn ordered_pool_layout() -> Layout {
    let mut controls: Vec<Value> = ["Vox 1", "Vox 2", "Vox 3", "Klavir", "Bass"]
        .iter()
        .map(|n| strip("band", track(n)))
        .collect();
    let mut wide = strip("band", track("Wide"));
    wide["wide"] = json!(true);
    controls.push(wide);
    layout_of(
        json!([{"sections": [group("g", None, Value::Array(controls))]}]),
        json!([]),
    )
}

#[test]
fn an_edit_takes_the_same_set_at_the_documents_rev() {
    let before = ordered();
    let layout = ordered_pool_layout();
    let pooled = pool(&layout);
    let after = edit(
        &before,
        &reordered(
            &before,
            &[
                ("lu-1", vec!["Vox 3", "Vox 1", "Vox 2"]),
                ("ru-1", vec!["Bass", "Klavir"]),
            ],
        ),
        7,
        &pooled,
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
    let one = edit(
        &before,
        &reordered(&before, &[("ru-1", vec!["Bass", "Klavir"])]),
        7,
        &pooled,
    )
    .unwrap();
    assert_eq!(one.layouts(SectorId::Lu), before.layouts(SectorId::Lu));
    assert_eq!(one.rev, 8);
}

#[test]
fn an_edit_is_stale_for_another_rev_another_set_or_other_sectors() {
    let before = ordered();
    let layout = ordered_pool_layout();
    let pooled = pool(&layout);
    let stale = |sectors: &Sectors, rev: u64| edit(&before, sectors, rev, &pooled);
    let good = reordered(&before, &[("ru-1", vec!["Bass", "Klavir"])]);
    assert_eq!(stale(&good, 6), Err(Refusal::Stale));
    assert_eq!(stale(&good, 8), Err(Refusal::Stale));
    assert!(stale(&good, 7).is_ok());
    assert_eq!(
        stale(&reordered(&before, &[("ru-1", vec!["Bass"])]), 7),
        Err(Refusal::Stale),
        "one missing"
    );
    assert_eq!(
        stale(
            &reordered(&before, &[("ru-1", vec!["Bass", "Klavir", "Vox 1"])]),
            7
        ),
        Err(Refusal::Stale),
        "one more"
    );
    assert_eq!(
        stale(&reordered(&before, &[("ru-1", vec!["Bass", "Bass"])]), 7),
        Err(Refusal::Stale),
        "the same length, another set"
    );
    // A sector the document does not name, or one it names left out.
    let mut more = good.clone();
    more.insert(SectorId::Rd, vec![sl("rd-1", "NEW", Vec::new())]);
    assert_eq!(stale(&more, 7), Err(Refusal::Stale), "a sector more");
    let mut less = good.clone();
    less.remove(&SectorId::Lu);
    assert_eq!(stale(&less, 7), Err(Refusal::Stale), "a sector less");
    // A layout moved to another sector.
    let mut moved = good.clone();
    let lu = moved.remove(&SectorId::Lu).unwrap();
    moved.insert(SectorId::Lu, vec![sl("lu-9", "NEW", Vec::new())]);
    moved.get_mut(&SectorId::Ru).unwrap().extend(lu);
    assert_eq!(stale(&moved, 7), Err(Refusal::Stale), "lu-1 moved to ru");
    // A new layout starts empty: one with a strip, even a gap before it,
    // is no edit a page makes.
    let mut filled = good.clone();
    filled.get_mut(&SectorId::Lu).unwrap().push(slotted(
        "lu-2",
        "NEW",
        vec![None, Some(band("Bass"))],
    ));
    assert_eq!(
        stale(&filled, 7),
        Err(Refusal::Stale),
        "a new layout filled"
    );
}

#[test]
fn an_edit_that_changes_nothing_is_refused_empty() {
    let before = ordered();
    assert_eq!(
        edit(&before, &before.sectors, 7, &[])
            .map_err(Refusal::text)
            .map(|d| d.rev),
        Err("EMPTY"),
        "nothing to change takes no revision and no undo step"
    );
    // Trailing empty slots change nothing either.
    let trailing = reordered(&before, &[("ru-1", vec!["Klavir", "Bass", "", ""])]);
    assert_eq!(edit(&before, &trailing, 7, &[]), Err(Refusal::Empty));
}

#[test]
fn an_edit_puts_a_strip_at_the_sectors_right_edge_with_gaps_before_it() {
    let before = ordered();
    let layout = ordered_pool_layout();
    let pooled = pool(&layout);
    // BAND's case: Klavir stays first, Bass goes to the sixth slot.
    let after = edit(
        &before,
        &reordered(&before, &[("ru-1", vec!["Klavir", "", "", "", "", "Bass"])]),
        7,
        &pooled,
    )
    .unwrap();
    assert_eq!(after.rev, 8);
    let bass = Some(band("Bass"));
    let klavir = Some(band("Klavir"));
    assert_eq!(
        after.layouts(SectorId::Ru),
        &[slotted(
            "ru-1",
            "BAND",
            vec![klavir.clone(), None, None, None, None, bass.clone()]
        )]
    );
    // The file says null for an empty slot.
    let file = serde_json::to_value(&after).unwrap();
    assert_eq!(
        file["sectors"]["ru"][0]["strips"],
        json!([{"instance": "band", "label": "Klavir"}, null, null, null, null,
               {"instance": "band", "label": "Bass"}])
    );
    // Trailing empty slots are dropped; the gaps do not count for the set.
    let trailing = edit(
        &before,
        &reordered(&before, &[("ru-1", vec!["", "Bass", "Klavir", "", ""])]),
        7,
        &pooled,
    )
    .unwrap();
    assert_eq!(
        trailing.layouts(SectorId::Ru)[0].strips,
        vec![None, bass, klavir]
    );
    assert_eq!(
        edit(
            &before,
            &reordered(&before, &[("ru-1", vec!["Bass", "", "Vox 1"])]),
            7,
            &pooled
        ),
        Err(Refusal::Stale),
        "a gap never stands in for a strip"
    );
}

#[test]
fn an_edit_past_6_units_is_full_unless_it_grows_nothing() {
    let layout = ordered_pool_layout();
    let pooled = pool(&layout);
    let before = ordered();
    // Six slots hold 6 units: taken; a seventh is FULL.
    assert!(
        edit(
            &before,
            &reordered(&before, &[("ru-1", vec!["Klavir", "", "", "", "", "Bass"])]),
            7,
            &pooled
        )
        .is_ok()
    );
    assert_eq!(
        edit(
            &before,
            &reordered(
                &before,
                &[("ru-1", vec!["Klavir", "", "", "", "", "", "Bass"])]
            ),
            7,
            &pooled
        ),
        Err(Refusal::Full)
    );
    // A layout a hand edit put past 6 units can be reordered at its size,
    // never grown.
    let mut over = before.clone();
    over.sectors.insert(
        SectorId::Ru,
        vec![slotted(
            "ru-1",
            "BAND",
            vec![
                Some(band("Klavir")),
                None,
                None,
                None,
                None,
                None,
                Some(band("Bass")),
            ],
        )],
    );
    assert!(
        edit(
            &over,
            &reordered(
                &over,
                &[("ru-1", vec!["Bass", "", "", "", "", "", "Klavir"])]
            ),
            7,
            &pooled
        )
        .is_ok()
    );
    assert_eq!(
        edit(
            &over,
            &reordered(
                &over,
                &[("ru-1", vec!["Bass", "", "", "", "", "", "", "Klavir"])]
            ),
            7,
            &pooled
        ),
        Err(Refusal::Full)
    );
    assert!(grows_past_full(5.0, 6.0_f64.next_up()));
    assert!(!grows_past_full(5.0, 6.0));
    assert!(!grows_past_full(7.0, 7.0));
    assert!(grows_past_full(7.0, 7.0_f64.next_up()));
}

#[test]
fn an_edit_renames_adds_and_removes_layouts_as_one_change() {
    let before = ordered();
    let layout = ordered_pool_layout();
    let pooled = pool(&layout);
    // lu-1 renamed, lu-2 added (empty), ru-1 reordered: one revision.
    let mut sectors = reordered(&before, &[("ru-1", vec!["Bass", "Klavir"])]);
    let lu = sectors.get_mut(&SectorId::Lu).unwrap();
    lu[0].title = "VOCALS".to_string();
    lu.push(sl("lu-2", "NEW", Vec::new()));
    let after = edit(&before, &sectors, 7, &pooled).unwrap();
    assert_eq!(after.rev, 8);
    assert_eq!(
        after.layouts(SectorId::Lu),
        &[
            sl(
                "lu-1",
                "VOCALS",
                vec![band("Vox 1"), band("Vox 2"), band("Vox 3")]
            ),
            sl("lu-2", "NEW", Vec::new()),
        ]
    );
    assert_eq!(
        edit_counts(&before, &after),
        EditCounts {
            added: 1,
            removed: 0,
            renamed: 1,
            reordered: 1,
        }
    );
    assert_eq!(
        serde_json::to_value(edit_counts(&before, &after)).unwrap(),
        json!({"added": 1, "removed": 0, "renamed": 1, "reordered": 1})
    );
    // lu-1 removed (its strips simply leave it): lu-2 stays, alone.
    let mut gone = after.sectors.clone();
    gone.get_mut(&SectorId::Lu).unwrap().remove(0);
    let removed = edit(&after, &gone, 8, &pooled).unwrap();
    assert_eq!(removed.rev, 9);
    assert_eq!(
        removed.layouts(SectorId::Lu),
        &[sl("lu-2", "NEW", Vec::new())]
    );
    assert_eq!(removed.strip_count(), 2, "Klavir and Bass");
    assert_eq!(
        edit_counts(&after, &removed),
        EditCounts {
            added: 0,
            removed: 1,
            renamed: 0,
            reordered: 0,
        }
    );
    // A new layout's gaps are trimmed away.
    let mut gaps = before.sectors.clone();
    gaps.get_mut(&SectorId::Ru)
        .unwrap()
        .push(slotted("ru-2", "NEW 2", vec![None, None]));
    let added = edit(&before, &gaps, 7, &pooled).unwrap();
    assert_eq!(
        added.layouts(SectorId::Ru)[1],
        sl("ru-2", "NEW 2", Vec::new())
    );
}

#[test]
fn a_new_or_changed_title_is_1_to_8_upper_case_characters() {
    assert!(title_ok("MONITORS"));
    assert!(title_ok("NEW 2"));
    assert!(title_ok("A"));
    assert!(title_ok("ŽIVÉ"));
    assert!(!title_ok("MONITORS1"), "9 characters");
    assert!(!title_ok(""));
    assert!(!title_ok(" "));
    assert!(!title_ok(" A"), "a space before");
    assert!(!title_ok("A "), "a space after");
    assert!(!title_ok("Vox"), "lower case");
    assert!(!title_ok("A\u{7}"), "a control character");
    let before = ordered();
    let renamed = |title: &str| {
        let mut sectors = before.sectors.clone();
        sectors.get_mut(&SectorId::Lu).unwrap()[0].title = title.to_string();
        edit(&before, &sectors, 7, &[])
    };
    assert!(renamed("MONITORS").is_ok());
    assert_eq!(renamed("Vocals"), Err(Refusal::Invalid));
    assert_eq!(renamed("MONITORS1"), Err(Refusal::Invalid));
    assert_eq!(renamed(""), Err(Refusal::Invalid));
    let added = |title: &str| {
        let mut sectors = before.sectors.clone();
        sectors
            .get_mut(&SectorId::Lu)
            .unwrap()
            .push(sl("lu-2", title, Vec::new()));
        edit(&before, &sectors, 7, &[])
    };
    assert!(added("NEW").is_ok());
    assert_eq!(added("new"), Err(Refusal::Invalid));
    // A title the edit leaves as it was (a seed's from a group) is kept
    // whatever its case.
    let mut seeded = before.clone();
    seeded.sectors.get_mut(&SectorId::Ru).unwrap()[0].title = "Band".to_string();
    assert!(
        edit(
            &seeded,
            &reordered(&seeded, &[("ru-1", vec!["Bass", "Klavir"])]),
            7,
            &[]
        )
        .is_ok()
    );
    assert_eq!(Refusal::Invalid.text(), "invalid");
}

#[test]
fn a_sector_keeps_1_to_4_layouts_and_every_id_once() {
    assert_eq!(MAX_LAYOUTS, 4);
    assert!(!layouts_fit(0, 1));
    assert!(layouts_fit(1, 1));
    assert!(layouts_fit(4, 1));
    assert!(!layouts_fit(5, 1));
    assert!(!layouts_fit(5, 4));
    assert!(layouts_fit(5, 5), "a hand edit's five kept");
    assert!(!layouts_fit(6, 5));
    let before = ordered();
    let with = |extra: Vec<SectorLayout>| {
        let mut sectors = before.sectors.clone();
        sectors.get_mut(&SectorId::Lu).unwrap().extend(extra);
        edit(&before, &sectors, 7, &[])
    };
    let new = |id: &str| sl(id, "NEW", Vec::new());
    assert!(with(vec![new("lu-2"), new("lu-3"), new("lu-4")]).is_ok());
    assert_eq!(
        with(vec![new("lu-2"), new("lu-3"), new("lu-4"), new("lu-5")]),
        Err(Refusal::Invalid),
        "a fifth layout"
    );
    assert_eq!(
        with(vec![new("lu-2"), new("lu-2")]),
        Err(Refusal::Invalid),
        "an id twice"
    );
    assert_eq!(with(vec![new("")]), Err(Refusal::Invalid), "an empty id");
    let mut none = before.sectors.clone();
    none.insert(SectorId::Lu, Vec::new());
    assert_eq!(
        edit(&before, &none, 7, &[]),
        Err(Refusal::Invalid),
        "no layout left"
    );
}

#[test]
fn an_empty_slot_counts_one_unit_and_a_tick_fills_the_first() {
    let layout = units_layout();
    let pooled = pool(&layout);
    let gaps = vec![Some(band("S1")), None, Some(band("W")), None];
    assert_eq!(units(&gaps, &pooled), 4.1);
    assert_eq!(units(&[None], &pooled), 1.0);
    assert_eq!(
        ticked_in(&gaps, &band("S2")),
        vec![Some(band("S1")), Some(band("S2")), Some(band("W")), None]
    );
    let packed = full(vec![band("S1")]);
    assert_eq!(
        ticked_in(&packed, &band("S2")),
        full(vec![band("S1"), band("S2")])
    );
    assert_eq!(
        ticked_out(&gaps, &band("W")),
        vec![Some(band("S1"))],
        "its slot and the empty one after it trimmed"
    );
    assert_eq!(
        ticked_out(&gaps, &band("S1")),
        vec![None, None, Some(band("W"))]
    );
    // FULL counts the empty slots: five strips and a gap hold 6 units, a
    // tick fills the gap (6 units, fits); a wide one there would be 6.1.
    let five_gap = slotted(
        "lu-1",
        "A",
        vec![
            Some(band("S1")),
            None,
            Some(band("S2")),
            Some(band("S3")),
            Some(band("S4")),
            Some(band("S5")),
        ],
    );
    assert!(!would_be_full(&five_gap, &band("S6"), &pooled));
    assert!(would_be_full(&five_gap, &band("W"), &pooled));
    let six_gaps = slotted("lu-1", "A", vec![None; 6]);
    assert!(!would_be_full(&six_gaps, &band("S1"), &pooled));
    let seven = slotted(
        "lu-1",
        "A",
        full((1..=6).map(|n| band(&format!("S{n}"))).collect()),
    );
    assert!(would_be_full(&seven, &band("Bass"), &pooled));
}

#[test]
fn a_file_with_null_slots_reads_and_a_strip_is_held_once_its_gaps_aside() {
    let text = json!({"rev": 2, "sectors": {"ru": [{"id": "ru-1", "title": "BAND",
        "strips": [{"instance": "band", "label": "Klavir"}, null, null,
                   {"instance": "band", "label": "Bass"}, null]}]}});
    let read: SectorsDoc = serde_json::from_value(text).unwrap();
    assert_eq!(
        read.validate(),
        Vec::<String>::new(),
        "two gaps are no strip twice"
    );
    let layout = &read.layouts(SectorId::Ru)[0];
    assert!(layout.holds(&band("Bass")));
    assert!(!layout.holds(&band("Vox 1")));
    assert_eq!(
        layout.refs().cloned().collect::<Vec<_>>(),
        vec![band("Klavir"), band("Bass")]
    );
    assert_eq!(read.strip_count(), 2, "the gaps are no strips");
    assert_eq!(read.layout_count(), 1);
    let pool_layout = ordered_pool_layout();
    let pooled = pool(&pool_layout);
    assert_eq!(unknown(&read, &pooled), 0, "a gap is no unknown ref");
    // A hand edit's trailing empty slots are dropped.
    let trimmed = read.trimmed();
    assert_eq!(trimmed.layouts(SectorId::Ru)[0].strips.len(), 4);
    assert_eq!(trim(vec![None, None]), Vec::<Slot>::new());
    assert_eq!(
        trim(vec![None, Some(band("A"))]),
        vec![None, Some(band("A"))]
    );
    // An old file without null reads as before.
    let old: SectorsDoc = serde_json::from_value(json!({"rev": 1, "sectors": {"lu": [
        {"id": "lu-1", "title": "VOX", "strips": [{"instance": "band", "label": "Vox 1"}]}]}}))
    .unwrap();
    assert_eq!(
        old.layouts(SectorId::Lu),
        &[sl("lu-1", "VOX", vec![band("Vox 1")])]
    );
}

#[test]
fn the_seed_never_makes_a_gap() {
    let seeded = seed(&fixture());
    for layout in seeded.sectors.values().flatten() {
        assert!(layout.strips.iter().all(Option::is_some), "{}", layout.id);
    }
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

/// A group of band strips with these names.
fn band_group(id: &str, title: Option<&str>, names: &[&str]) -> Value {
    let controls: Vec<Value> = names.iter().map(|n| strip("band", track(n))).collect();
    group(id, title, Value::Array(controls))
}

fn bands(names: &[&str]) -> Vec<StripRef> {
    names.iter().copied().map(band).collect()
}

fn names(prefix: &str, n: usize) -> Vec<String> {
    (1..=n).map(|i| format!("{prefix} {i}")).collect()
}

fn as_strs(all: &[String]) -> Vec<&str> {
    all.iter().map(String::as_str).collect()
}

#[test]
fn the_seed_moves_a_layouts_overflow_to_the_front_of_its_rows_right_sector() {
    // Row 0 as the real FOH page: a pager whose STAGE holds 6 singers and
    // 2 pinned strips, then a group of 2; row 1 two groups of 8 and 4.
    let singers = names("Vox", 6);
    let effects = names("Fx", 8);
    let hands = names("Hand", 4);
    let pager = json!({"kind": "pager", "id": "pg", "default_page": "stage", "pages": [
        {"id": "stage", "title": "STAGE", "sections": [
            band_group("singers", Some("VOCALS"), &as_strs(&singers)),
            band_group("pinned", None, &["Klavir", "Vocals"])]},
        {"id": "band", "title": "BAND", "sections": [
            band_group("band", None, &["Bass", "Klavir", "Vocals"])]}]});
    let rows = json!([
        {"sections": [pager, band_group("backing", Some("BACKING"), &["Backing", "Band"])]},
        {"sections": [
            band_group("effects", Some("EFFECTS"), &as_strs(&effects)),
            band_group("hands", Some("HANDS"), &as_strs(&hands))]}
    ]);
    let layout = layout_of(rows, json!([]));
    let seeded = seed(&layout);
    assert_eq!(seeded.validate(), Vec::<String>::new());
    assert_eq!(
        seeded.layouts(SectorId::Lu),
        &[
            sl("lu-1", "STAGE", bands(&as_strs(&singers))),
            sl("lu-2", "BAND", bands(&["Bass", "Klavir", "Vocals"])),
        ]
    );
    assert_eq!(
        seeded.layouts(SectorId::Ru),
        &[sl(
            "ru-1",
            "BACKING",
            bands(&["Klavir", "Vocals", "Backing", "Band"])
        )]
    );
    assert_eq!(
        seeded.layouts(SectorId::Ld),
        &[sl("ld-1", "EFFECTS", bands(&as_strs(&effects[..6])))]
    );
    let mut rd = as_strs(&effects[6..]);
    rd.extend(as_strs(&hands));
    assert_eq!(
        seeded.layouts(SectorId::Rd),
        &[sl("rd-1", "HANDS", bands(&rd))]
    );
    let pool = pool(&layout);
    for layout in seeded.sectors.values().flatten() {
        assert!(!is_full(units(&layout.strips, &pool)), "{}", layout.id);
    }
}

#[test]
fn the_seed_puts_a_fader_two_sub_pages_overflow_into_the_right_sector_once() {
    // Klavir and Vocals pinned on both sub-pages, each full with 6 others:
    // both overflow twice, and the right sector holds each once.
    let singers = names("Vox", 6);
    let players = names("Bass", 6);
    let pager = json!({"kind": "pager", "id": "pg", "default_page": "stage", "pages": [
        {"id": "stage", "title": "STAGE", "sections": [
            band_group("singers", None, &as_strs(&singers)),
            band_group("pin1", None, &["Klavir", "Vocals"])]},
        {"id": "band", "title": "BAND", "sections": [
            band_group("players", None, &as_strs(&players)),
            band_group("pin2", None, &["Klavir", "Vocals"])]}]});
    let rows = json!([{"sections": [pager, band_group("main", Some("MAIN"), &["Band"])]}]);
    let seeded = seed(&layout_of(rows, json!([])));
    assert_eq!(seeded.validate(), Vec::<String>::new());
    assert_eq!(
        seeded.layouts(SectorId::Ru),
        &[sl("ru-1", "MAIN", bands(&["Klavir", "Vocals", "Band"]))]
    );
}

#[test]
fn the_seed_counts_a_wide_strip_and_carries_a_full_right_side_into_more_layouts() {
    // Left: 5 strips and a wide one (6.1 units): the wide one moves right,
    // to the front of an empty right sector's new MAIN layout.
    let mut controls: Vec<Value> = (1..=5)
        .map(|n| strip("band", track(&format!("Vox {n}"))))
        .collect();
    let mut wide = strip("band", track("Klavir"));
    wide["wide"] = json!(true);
    controls.push(wide);
    let pager = json!({"kind": "pager", "id": "pg", "default_page": "a", "pages": [
        {"id": "a", "title": "STAGE", "sections": [group("g", None, Value::Array(controls))]}]});
    let seeded = seed(&layout_of(json!([{"sections": [pager]}]), json!([])));
    assert_eq!(
        seeded.layouts(SectorId::Lu),
        &[sl(
            "lu-1",
            "STAGE",
            bands(&["Vox 1", "Vox 2", "Vox 3", "Vox 4", "Vox 5"])
        )]
    );
    assert_eq!(
        seeded.layouts(SectorId::Ru),
        &[sl("ru-1", "MAIN", bands(&["Klavir"]))]
    );
    // Right: 3 overflowing strips before a group of 6 hold 9: the right
    // sector gets 6, then a second layout of the same title with 3.
    let left = names("Vox", 9);
    let right = names("Fx", 6);
    let rows = json!([{"sections": [
        band_group("l", Some("STAGE"), &as_strs(&left)),
        band_group("r", Some("EFFECTS"), &as_strs(&right))]}]);
    let seeded = seed(&layout_of(rows, json!([])));
    assert_eq!(
        seeded.layouts(SectorId::Lu),
        &[sl("lu-1", "STAGE", bands(&as_strs(&left[..6])))]
    );
    let mut first = as_strs(&left[6..]);
    first.extend(as_strs(&right[..3]));
    assert_eq!(
        seeded.layouts(SectorId::Ru),
        &[
            sl("ru-1", "EFFECTS", bands(&first)),
            sl("ru-2", "EFFECTS", bands(&as_strs(&right[3..]))),
        ]
    );
    assert_eq!(seeded.validate(), Vec::<String>::new());
}
