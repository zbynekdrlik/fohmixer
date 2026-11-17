use std::sync::LazyLock;

use super::*;
use crate::behave::arrange_edit::TabTap;
use fohmixer_proto::layout::Layout;
use fohmixer_proto::sectors::SectorLayout;
use serde_json::{Value, json};

fn strip(name: &str) -> Value {
    json!({"kind": "strip", "binding": {"instance": "band", "anchor": {"kind": "track", "name": name}},
           "strip_kind": "standard"})
}

/// One page of the band strips the tests name (one unit each), then the
/// wide `PA Out` (1.1 units, #236).
static LAYOUT: LazyLock<Layout> = LazyLock::new(|| {
    let mut controls: Vec<Value> = [
        "Vox 1", "Vox 2", "Vox 3", "Vox 4", "Vox 5", "Vox 6", "Bass", "Keys 1", "Klavir", "PA",
    ]
    .into_iter()
    .map(strip)
    .collect();
    let mut wide = strip("PA Out");
    wide["wide"] = json!(true);
    controls.push(wide);
    serde_json::from_value(json!({
        "schema": 2,
        "default_page": "foh",
        "pages": [{"id": "foh", "title": "FOH",
                   "rows": [{"sections": [{"kind": "group", "id": "g", "controls": controls}]}]}]
    }))
    .expect("the layout parses")
});

/// The served layout's pool, the units a drop is judged by (#236).
fn pool() -> Vec<Pooled<'static>> {
    fohmixer_proto::sectors::pool(&LAYOUT)
}

fn r(label: &str) -> StripRef {
    StripRef {
        instance: "band".to_string(),
        label: label.to_string(),
    }
}

/// These labels as slots, `""` an empty slot.
fn refs(labels: &[&str]) -> Vec<Slot> {
    labels
        .iter()
        .map(|l| (!l.is_empty()).then(|| r(l)))
        .collect()
}

/// What a down on `label`'s cover in `sector`, slot `slot`, holds.
fn held(sector: SectorId, label: &str, slot: usize, spots: Vec<Spot>) -> Held {
    Held {
        sector,
        strip: r(label),
        slot,
        spots,
    }
}

/// A cell of `sector`, slot `slot`, centred at `x` on the line from
/// `top` to `top + 300`.
fn spot(sector: SectorId, slot: usize, x: f64, top: f64) -> Spot {
    Spot {
        sector,
        slot,
        x,
        top,
        bottom: top + 300.0,
    }
}

fn sl(id: &str, labels: &[&str]) -> SectorLayout {
    SectorLayout {
        id: id.to_string(),
        title: id.to_uppercase(),
        strips: refs(labels),
    }
}

/// lu-1 [Vox 1, Vox 2, Bass], ru-1 [Keys 1, Klavir] at revision 3.
fn doc() -> SectorsDoc {
    SectorsDoc {
        rev: 3,
        sectors: [
            (SectorId::Lu, vec![sl("lu-1", &["Vox 1", "Vox 2", "Bass"])]),
            (SectorId::Ru, vec![sl("ru-1", &["Keys 1", "Klavir"])]),
        ]
        .into_iter()
        .collect(),
    }
}

/// The mode on since `doc()`, nothing edited yet.
fn on() -> Arrange {
    Arrange::On {
        rev: 3,
        edit: Edit::of(&doc()),
    }
}

/// The mode saving these sectors at revision `rev`.
fn saving_at(rev: u64) -> Arrange {
    Arrange::Saving {
        rev,
        sectors: doc().sectors,
    }
}

/// `doc()`'s sectors with `layout`'s slots replaced.
fn with_slots(layout: &str, slots: Vec<Slot>) -> Sectors {
    let mut sectors = doc().sectors;
    for l in sectors.values_mut().flatten() {
        if l.id == layout {
            l.strips = slots.clone();
        }
    }
    sectors
}

/// A drag gave `layout` these slots (only while the mode is on).
fn moved(mode: &mut Arrange, layout: &str, slots: Vec<Slot>) {
    if let Some(edit) = mode.edit() {
        edit.moved(layout, slots);
    }
}

#[test]
fn a_first_tap_starts_and_a_second_without_changes_leaves() {
    let doc = doc();
    let mut mode = Arrange::Off;
    assert!(!mode.is_on());
    assert_eq!(mode.tap(&doc), Some(Tap::Start));
    assert_eq!(mode, on());
    assert!(mode.is_on());
    assert_eq!(mode.tap(&doc), Some(Tap::Leave));
    assert_eq!(mode, Arrange::Off);
    // An edit back to the layout's own order is no change.
    let _ = mode.tap(&doc);
    moved(&mut mode, "lu-1", refs(&["Vox 1", "Vox 2", "Bass"]));
    assert_eq!(mode.tap(&doc), Some(Tap::Leave));
    assert_eq!(mode, Arrange::Off);
    // So is a layout added and removed again.
    let _ = mode.tap(&doc);
    let added = mode.edit().and_then(|e| e.add(SectorId::Ru));
    assert_eq!(added.as_deref(), Some("ru-r4-1"));
    if let Some(edit) = mode.edit() {
        edit.ask_remove("ru-r4-1");
        assert_eq!(edit.tab_tapped("ru-r4-1"), TabTap::Removed);
    }
    assert_eq!(mode.tap(&doc), Some(Tap::Leave));
    // Off, there is nothing to edit.
    assert_eq!(mode.edit(), None);
    assert_eq!(mode.edit_now(), None);
}

#[test]
fn a_second_tap_with_changes_sends_every_sector_as_one_edit() {
    let doc = doc();
    let mut mode = Arrange::Off;
    let _ = mode.tap(&doc);
    moved(&mut mode, "lu-1", refs(&["Vox 2", "Vox 1", "Bass"]));
    assert!(mode.edit_now().is_some());
    let sent = mode.tap(&doc);
    assert_eq!(
        sent,
        Some(Tap::Send(
            with_slots("lu-1", refs(&["Vox 2", "Vox 1", "Bass"])),
            3
        ))
    );
    assert!(mode.is_on(), "saving keeps the page inert");
    // The edit stays shown while saving; a tap meanwhile does nothing,
    // and nothing can be edited.
    assert_eq!(
        mode.applied(&doc).sectors[&SectorId::Lu][0].strips,
        refs(&["Vox 2", "Vox 1", "Bass"])
    );
    assert_eq!(mode.tap(&doc), None);
    assert_eq!(mode.edit(), None);
    assert_eq!(mode.edit_now(), None);
}

#[test]
fn a_rename_an_add_and_a_remove_go_in_the_one_save() {
    let doc = doc();
    let mut mode = Arrange::Off;
    let _ = mode.tap(&doc);
    let edit = mode.edit().expect("on");
    assert_eq!(edit.add(SectorId::Lu), Some("lu-r4-1".to_string()));
    edit.ask_remove("lu-1");
    assert_eq!(edit.tab_tapped("lu-1"), TabTap::Removed);
    // A rename still in its field is taken by the tap that saves.
    let _ = edit.rename_start("ru-1");
    let _ = edit.typed("keys");
    let Some(Tap::Send(sectors, rev)) = mode.tap(&doc) else {
        panic!("a save");
    };
    assert_eq!(rev, 3);
    assert_eq!(
        sectors[&SectorId::Lu],
        vec![SectorLayout {
            id: "lu-r4-1".to_string(),
            title: "NEW".to_string(),
            strips: Vec::new(),
        }]
    );
    assert_eq!(sectors[&SectorId::Ru][0].title, "KEYS");
    assert_eq!(
        sectors[&SectorId::Ru][0].strips,
        refs(&["Keys 1", "Klavir"])
    );
    // A rename alone is a change too.
    let mut mode = Arrange::Off;
    let _ = mode.tap(&doc);
    if let Some(edit) = mode.edit() {
        let _ = edit.rename_start("lu-1");
        let _ = edit.typed("vocals");
        edit.rename_end(true);
    }
    assert!(matches!(mode.tap(&doc), Some(Tap::Send(..))));
}

#[test]
fn a_taken_save_keeps_its_edits_shown_until_its_document_arrives() {
    let doc = doc();
    let saving = || {
        let mut mode = Arrange::Off;
        let _ = mode.tap(&doc);
        moved(&mut mode, "lu-1", refs(&["Bass", "Vox 1", "Vox 2"]));
        let _ = mode.tap(&doc);
        mode
    };
    // The hub sends its result before the new document: between the
    // two the page must still show the edits, never the old order.
    let mut taken = saving();
    assert_eq!(taken.answered(true), None);
    assert!(taken.is_on(), "still saving");
    assert_eq!(
        taken.applied(&doc).sectors[&SectorId::Lu][0].strips,
        refs(&["Bass", "Vox 1", "Vox 2"])
    );
    taken.doc_arrived(3);
    assert!(taken.is_on(), "the same revision is not the save's");
    taken.doc_arrived(4);
    assert_eq!(taken, Arrange::Off, "its document ends it");
    // A refusal ends it at once with its note.
    let mut refused = saving();
    assert_eq!(refused.answered(false), Some(REFUSED_NOTE));
    assert_eq!(refused, Arrange::Off);
    assert_eq!(REFUSED_NOTE, "Not saved: the layouts changed");
    // An answer while on (a stale one) leaves the mode on.
    let mut mode = on();
    assert_eq!(mode.answered(true), None);
    assert_eq!(mode, on());
    assert_eq!(mode.answered(false), Some(REFUSED_NOTE));
    assert_eq!(mode, on());
}

#[test]
fn a_save_that_never_reached_the_hub_ends_at_once_saying_so() {
    let doc = doc();
    let saving = || {
        let mut mode = Arrange::Off;
        let _ = mode.tap(&doc);
        moved(&mut mode, "lu-1", refs(&["Bass", "Vox 1", "Vox 2"]));
        let _ = mode.tap(&doc);
        mode
    };
    let not_sent: Result<(), String> = Err(crate::store::NOT_CONNECTED.to_string());
    let closed: Result<(), String> = Err(crate::store::CONNECTION_CLOSED.to_string());
    for outcome in [not_sent, closed] {
        let mut mode = saving();
        assert_eq!(mode.heard(&outcome), Some(OFFLINE_NOTE), "{outcome:?}");
        assert_eq!(mode, Arrange::Off);
    }
    assert_eq!(OFFLINE_NOTE, "Not saved: the hub is not connected");
    assert_eq!(crate::store::NOT_CONNECTED, "not connected to the hub");
    assert_eq!(crate::store::CONNECTION_CLOSED, "the hub connection closed");
    // The hub's own refusal says the layouts changed.
    let mut mode = saving();
    assert_eq!(
        mode.heard(&Err::<(), _>("stale".to_string())),
        Some(REFUSED_NOTE)
    );
    assert_eq!(mode, Arrange::Off);
    // Taken: still saving until its document.
    let mut mode = saving();
    assert_eq!(mode.heard(&Ok::<(), String>(())), None);
    assert!(mode.is_on());
    // Heard while on (a stale answer): the mode stays.
    let mut mode = on();
    assert_eq!(
        mode.heard(&Err::<(), _>(crate::store::NOT_CONNECTED.to_string())),
        Some(OFFLINE_NOTE)
    );
    assert_eq!(mode, on());
}

#[test]
fn a_refusal_names_its_word_in_the_note() {
    assert_eq!(refusal_note("stale"), REFUSED_NOTE);
    assert_eq!(refusal_note("FULL"), FULL_NOTE);
    assert_eq!(refusal_note("invalid"), INVALID_NOTE);
    assert_eq!(refusal_note(crate::store::NOT_CONNECTED), OFFLINE_NOTE);
    assert_eq!(refusal_note(crate::store::CONNECTION_CLOSED), OFFLINE_NOTE);
    assert_eq!(refusal_note("EMPTY"), NOT_SAVED_NOTE);
    assert_eq!(refusal_note("cannot write sectors.json"), NOT_SAVED_NOTE);
    assert_eq!(refusal_note(""), NOT_SAVED_NOTE);
    // The words are the hub's, exactly: another case is another word.
    assert_eq!(refusal_note("Stale"), NOT_SAVED_NOTE);
    assert_eq!(refusal_note("full"), NOT_SAVED_NOTE);
    assert_eq!(REFUSED_NOTE, "Not saved: the layouts changed");
    assert_eq!(FULL_NOTE, "Not saved: a layout is full");
    assert_eq!(INVALID_NOTE, "Not saved: the layouts are not valid");
    assert_eq!(NOT_SAVED_NOTE, "Not saved");
}

#[test]
fn a_refused_save_ends_at_once_with_its_words_note() {
    let doc = doc();
    let saving = || {
        let mut mode = Arrange::Off;
        let _ = mode.tap(&doc);
        moved(&mut mode, "lu-1", refs(&["Bass", "Vox 1", "Vox 2"]));
        let _ = mode.tap(&doc);
        mode
    };
    for (word, note) in [
        ("stale", REFUSED_NOTE),
        ("FULL", FULL_NOTE),
        ("invalid", INVALID_NOTE),
        ("EMPTY", NOT_SAVED_NOTE),
        (crate::store::NOT_CONNECTED, OFFLINE_NOTE),
    ] {
        let mut mode = saving();
        assert_eq!(
            mode.heard(&Err::<(), _>(word.to_string())),
            Some(note),
            "{word}"
        );
        assert_eq!(mode, Arrange::Off, "{word}");
    }
}

#[test]
fn a_new_document_during_arrange_ends_it_unsaved() {
    let mut mode = on();
    moved(&mut mode, "lu-1", refs(&["Bass"]));
    let kept = mode.clone();
    mode.doc_arrived(3);
    assert_eq!(mode, kept, "the same revision (a reconnect) keeps it");
    mode.doc_arrived(4);
    assert_eq!(mode, Arrange::Off);
    let mut older = on();
    older.doc_arrived(2);
    assert_eq!(older, Arrange::Off);
    // Saving at 3: its own answer's document is 4; 3 again keeps it.
    let mut mode = saving_at(3);
    mode.doc_arrived(3);
    assert_eq!(mode, saving_at(3));
    mode.doc_arrived(4);
    assert_eq!(mode, Arrange::Off);
    let mut off = Arrange::Off;
    off.doc_arrived(9);
    assert_eq!(off, Arrange::Off);
}

#[test]
fn a_hide_ends_it_unsaved() {
    let mut mode = on();
    moved(&mut mode, "lu-1", refs(&["Bass"]));
    assert_eq!(mode.ended_by(ArrangeEnd::Hidden), Some(ArrangeEnd::Hidden));
    assert_eq!(mode, Arrange::Off);
    assert_eq!(mode.ended_by(ArrangeEnd::Hidden), None);
    let mut saving = saving_at(3);
    assert_eq!(
        saving.ended_by(ArrangeEnd::NewDoc),
        Some(ArrangeEnd::NewDoc)
    );
    assert_eq!(saving, Arrange::Off);
}

#[test]
fn the_page_shows_the_edit() {
    let doc = doc();
    let mut mode = Arrange::Off;
    assert_eq!(mode.applied(&doc), doc);
    let _ = mode.tap(&doc);
    moved(&mut mode, "ru-1", refs(&["Klavir", "Keys 1"]));
    let added = mode.edit().and_then(|e| e.add(SectorId::Ru));
    assert_eq!(added.as_deref(), Some("ru-r4-1"));
    let shown = mode.applied(&doc);
    assert_eq!(shown.rev, 3);
    assert_eq!(
        shown.sectors[&SectorId::Ru][0].strips,
        refs(&["Klavir", "Keys 1"])
    );
    assert_eq!(shown.sectors[&SectorId::Ru][0].title, "RU-1");
    assert_eq!(shown.sectors[&SectorId::Ru][1].title, "NEW");
    assert_eq!(shown.sectors[&SectorId::Lu], doc.sectors[&SectorId::Lu]);
}

#[test]
fn a_drag_starts_past_6_px() {
    assert_eq!(DRAG_SLOP, 6.0);
    assert!(!drags(0.0, 6.0));
    assert!(drags(0.0, 6.0_f64.next_up()));
    assert!(!drags(0.0, -6.0));
    assert!(drags(0.0, -(6.0_f64.next_up())));
    assert!(!drags(100.0, 106.0));
    assert!(drags(100.0, 93.0));
}

#[test]
fn an_older_notes_timer_never_clears_a_newer_note() {
    let mut notes = NoteTimer::default();
    let first = notes.show();
    assert!(notes.expires(first), "alone, its timer clears it");
    let second = notes.show();
    assert_ne!(first, second);
    assert!(!notes.expires(first), "the first refusal's timer, 4 s on");
    assert!(notes.expires(second));
    let third = notes.show();
    assert!(!notes.expires(second));
    assert!(notes.expires(third));
}

#[test]
fn the_note_says_the_mode_or_the_refusal() {
    assert_eq!(arrange_note(false, None), "");
    assert_eq!(arrange_note(true, None), ARRANGE_NOTE);
    assert_eq!(arrange_note(false, Some(REFUSED_NOTE)), REFUSED_NOTE);
    assert_eq!(arrange_note(true, Some(REFUSED_NOTE)), REFUSED_NOTE);
    assert_eq!(ARRANGE_NOTE, "drag · ✎ ✕ + · tap to save");
}

#[test]
fn a_strip_lands_on_the_cell_of_its_line_whose_centre_is_nearest() {
    use SectorId::{Ld, Lu, Ru};
    // The up line from 0 to 300 (LU left, RU right), the down line
    // from 310 to 610.
    let spots = [
        spot(Lu, 0, 40.0, 0.0),
        spot(Lu, 1, 124.0, 0.0),
        spot(Ru, 0, 460.0, 0.0),
        spot(Ld, 0, 40.0, 310.0),
        spot(Ld, 2, 208.0, 310.0),
    ];
    let at = |x: f64, y: f64| drop_spot(&spots, x, y).map(|s| (s.sector, s.slot));
    assert_eq!(at(0.0, 100.0), Some((Lu, 0)));
    assert_eq!(at(83.0, 100.0), Some((Lu, 1)));
    assert_eq!(at(500.0, 100.0), Some((Ru, 0)), "another sector's cell");
    // Halfway: the first of the two.
    assert_eq!(at(82.0, 100.0), Some((Lu, 0)));
    assert_eq!(at(82.0_f64.next_up(), 100.0), Some((Lu, 1)));
    // The down line: its own cells first, however near an up one's x.
    assert_eq!(at(124.0, 400.0), Some((Ld, 0)));
    assert_eq!(at(200.0, 310.0), Some((Ld, 2)), "its top edge");
    assert_eq!(at(200.0, 300.0), Some((Lu, 1)), "the up line's foot");
    // Between the lines: the nearer line's box.
    assert_eq!(at(124.0, 304.0), Some((Lu, 1)));
    assert_eq!(at(124.0, 306.0), Some((Ld, 0)));
    // Above every line and below it: the nearest line.
    assert_eq!(at(460.0, -50.0), Some((Ru, 0)));
    assert_eq!(at(40.0, 900.0), Some((Ld, 0)));
    assert_eq!(drop_spot(&[], 10.0, 10.0), None);
    // How far off a cell's box a finger is.
    let cell = spot(Lu, 0, 40.0, 100.0);
    assert_eq!(cell.off_line(100.0), 0.0);
    assert_eq!(cell.off_line(400.0), 0.0);
    assert_eq!(cell.off_line(250.0), 0.0);
    assert_eq!(cell.off_line(90.0), 10.0);
    assert_eq!(cell.off_line(410.5), 10.5);
}

#[test]
fn a_swap_or_a_move_into_an_empty_slot_keeps_every_strip_once() {
    let order = refs(&["A", "B", "", "D"]);
    // Onto another strip: the two swap.
    assert_eq!(swapped(&order, 0, 3), refs(&["D", "B", "", "A"]));
    // Into an empty slot: its old slot is left empty.
    assert_eq!(swapped(&order, 0, 2), refs(&["", "B", "A", "D"]));
    // Past the end: empty slots before it.
    assert_eq!(swapped(&order, 1, 5), refs(&["A", "", "", "D", "", "B"]));
    // The last strip moved left: the trailing empty slots are dropped.
    assert_eq!(swapped(&order, 3, 2), refs(&["A", "B", "D"]));
    assert_eq!(swapped(&refs(&["A"]), 0, 0), refs(&["A"]));
}

#[test]
fn a_drop_puts_the_strip_at_the_right_edge_with_gaps_before_it() {
    // BAND's case: RU shows Klavir, Bass, Keys 1, PA and two empty
    // cells after them in ARRANGE; PA goes to the sixth.
    let order = refs(&["Klavir", "Bass", "Keys 1", "PA"]);
    assert_eq!(
        landing(&order, None, 3, 5, &r("PA")),
        Some(Landing::Within(refs(&[
            "Klavir", "Bass", "Keys 1", "", "", "PA"
        ])))
    );
    // Onto Klavir: a swap.
    assert_eq!(
        landing(&order, None, 3, 0, &r("PA")),
        Some(Landing::Within(refs(&["PA", "Bass", "Keys 1", "Klavir"])))
    );
    // Where it was, a slot that no longer holds it: nothing.
    assert_eq!(landing(&order, None, 3, 3, &r("PA")), None);
    assert_eq!(landing(&order, None, 2, 0, &r("PA")), None);
    assert_eq!(landing(&order, None, 9, 0, &r("PA")), None);
    // Into the mode: only while it is on, onto its own edit.
    let layout = SectorLayout {
        id: "ru-1".to_string(),
        title: "BAND".to_string(),
        strips: order.clone(),
    };
    let doc = SectorsDoc {
        rev: 3,
        sectors: [(SectorId::Ru, vec![layout.clone()])].into_iter().collect(),
    };
    let mut off = Arrange::Off;
    assert!(!off.dropped_in("ru-1", 3, "ru-1", 5, &r("PA"), &pool()));
    assert_eq!(off, Arrange::Off);
    let mut mode = Arrange::Off;
    let _ = mode.tap(&doc);
    assert!(mode.dropped_in("ru-1", 3, "ru-1", 5, &r("PA"), &pool()));
    let first = refs(&["Klavir", "Bass", "Keys 1", "", "", "PA"]);
    assert_eq!(mode.applied(&doc).sectors[&SectorId::Ru][0].strips, first);
    assert!(mode.dropped_in("ru-1", 0, "ru-1", 3, &r("Klavir"), &pool()));
    let second = refs(&["", "Bass", "Keys 1", "Klavir", "", "PA"]);
    assert_eq!(mode.applied(&doc).sectors[&SectorId::Ru][0].strips, second);
    // A layout the edit does not hold takes nothing.
    let before = mode.clone();
    assert!(!mode.dropped_in("gone", 3, "gone", 0, &r("Klavir"), &pool()));
    assert!(!mode.dropped_in("ru-1", 3, "gone", 0, &r("Klavir"), &pool()));
    assert_eq!(mode, before);
    // The save carries the empty slots.
    let mut sent = doc.sectors.clone();
    sent.get_mut(&SectorId::Ru).unwrap()[0].strips = second;
    assert_eq!(mode.tap(&doc), Some(Tap::Send(sent, 3)));
    let mut saving = Arrange::Saving {
        rev: 3,
        sectors: doc.sectors.clone(),
    };
    assert!(!saving.dropped_in("ru-1", 3, "ru-1", 5, &r("PA"), &pool()));
    assert_eq!(saving.applied(&doc), doc);
}

#[test]
fn a_drop_on_another_sectors_cell_moves_the_strip_there() {
    // #233: Vox 2 from LU's layout onto RU's.
    let lu = refs(&["Vox 1", "Vox 2", "Bass"]);
    let ru = refs(&["Keys 1", "", "Klavir"]);
    // An empty cell takes it; its old slot is left empty.
    assert_eq!(
        landing(&lu, Some(&ru), 1, 1, &r("Vox 2")),
        Some(Landing::Across {
            from: refs(&["Vox 1", "", "Bass"]),
            to: refs(&["Keys 1", "Vox 2", "Klavir"]),
        })
    );
    // A strip there takes its old slot: a swap between the layouts.
    assert_eq!(
        landing(&lu, Some(&ru), 1, 0, &r("Vox 2")),
        Some(Landing::Across {
            from: refs(&["Vox 1", "Keys 1", "Bass"]),
            to: refs(&["Vox 2", "", "Klavir"]),
        })
    );
    // Past the other layout's end: empty slots before it; the last
    // strip of its own layout leaves no trailing gap.
    assert_eq!(
        landing(&lu, Some(&ru), 2, 4, &r("Bass")),
        Some(Landing::Across {
            from: refs(&["Vox 1", "Vox 2"]),
            to: refs(&["Keys 1", "", "Klavir", "", "Bass"]),
        })
    );
    // Into an empty layout.
    assert_eq!(
        landing(&lu, Some(&[]), 0, 0, &r("Vox 1")),
        Some(Landing::Across {
            from: refs(&["", "Vox 2", "Bass"]),
            to: refs(&["Vox 1"]),
        })
    );
    // A layout that already holds it takes no drop, on any cell.
    let holding = refs(&["Keys 1", "Vox 2"]);
    assert_eq!(landing(&lu, Some(&holding), 1, 0, &r("Vox 2")), None);
    assert_eq!(landing(&lu, Some(&holding), 1, 1, &r("Vox 2")), None);
    assert_eq!(landing(&lu, Some(&holding), 1, 2, &r("Vox 2")), None);
    // Nor does a swap with a strip its own layout already holds.
    let has_bass = refs(&["Bass", "Keys 1"]);
    assert_eq!(landing(&lu, Some(&has_bass), 1, 0, &r("Vox 2")), None);
    assert_eq!(
        landing(&lu, Some(&has_bass), 1, 1, &r("Vox 2")),
        Some(Landing::Across {
            from: refs(&["Vox 1", "Keys 1", "Bass"]),
            to: refs(&["Bass", "Vox 2"]),
        })
    );
    // A slot that no longer holds it: nothing.
    assert_eq!(landing(&lu, Some(&ru), 0, 1, &r("Vox 2")), None);
    // In the mode: both layouts change in the one edit, saved as one.
    let doc = doc();
    let mut mode = Arrange::Off;
    assert_eq!(
        mode.lands("lu-1", 1, "ru-1", 0, &r("Vox 2"), &pool()),
        None,
        "off"
    );
    let _ = mode.tap(&doc);
    assert_eq!(
        mode.lands("lu-1", 1, "ru-1", 2, &r("Vox 2"), &pool()),
        Some(Landing::Across {
            from: refs(&["Vox 1", "", "Bass"]),
            to: refs(&["Keys 1", "Klavir", "Vox 2"]),
        })
    );
    assert_eq!(mode.edit_now(), on().edit_now(), "lands changes nothing");
    assert!(mode.dropped_in("lu-1", 1, "ru-1", 2, &r("Vox 2"), &pool()));
    let shown = mode.applied(&doc);
    assert_eq!(
        shown.sectors[&SectorId::Lu][0].strips,
        refs(&["Vox 1", "", "Bass"])
    );
    assert_eq!(
        shown.sectors[&SectorId::Ru][0].strips,
        refs(&["Keys 1", "Klavir", "Vox 2"])
    );
    // RU holds Vox 2 now: a second drop of it there takes nothing.
    assert!(!mode.dropped_in("ru-1", 2, "ru-1", 2, &r("Vox 2"), &pool()));
    assert_eq!(mode.lands("lu-1", 0, "ru-1", 2, &r("Vox 2"), &pool()), None);
    let mut sent = doc.sectors.clone();
    sent.get_mut(&SectorId::Lu).unwrap()[0].strips = refs(&["Vox 1", "", "Bass"]);
    sent.get_mut(&SectorId::Ru).unwrap()[0].strips = refs(&["Keys 1", "Klavir", "Vox 2"]);
    assert_eq!(mode.tap(&doc), Some(Tap::Send(sent, 3)));
}

fn touch(id: i32, primary: bool) -> Down {
    Down::touch(id, primary)
}

#[test]
fn a_drag_carries_one_strip_past_the_slop_and_drops_it_at_the_lift() {
    let mut drag = Drag::default();
    let cells = vec![
        spot(SectorId::Lu, 0, 40.0, 0.0),
        spot(SectorId::Ru, 0, 460.0, 0.0),
    ];
    assert!(drag.down(
        touch(1, true),
        held(SectorId::Lu, "Vox 1", 0, cells.clone()),
        100.0,
        50.0
    ));
    // Another finger while one holds a strip: refused.
    assert!(!drag.down(
        touch(2, false),
        held(SectorId::Lu, "Vox 2", 0, Vec::new()),
        0.0,
        0.0
    ));
    assert_eq!(drag.held().map(|c| c.pointer), Some(1));
    assert_eq!(drag.moved(2, 300.0, 50.0), None);
    assert_eq!(drag.moved(1, 106.0, 56.0), None, "within the slop");
    assert_eq!(drag.moved(1, 120.0, 50.0), Some((20.0, 0.0)));
    assert_eq!(
        drag.moved(1, 104.0, 30.0),
        Some((4.0, -20.0)),
        "carried, it follows"
    );
    assert_eq!(drag.up(2, 0.0, 0.0), None);
    let carry = drag.up(1, 210.0, 50.0).expect("a drop");
    assert_eq!(carry.strip, r("Vox 1"));
    assert_eq!(carry.sector, SectorId::Lu);
    assert_eq!(carry.spots, cells);
    assert_eq!(carry.slot, 0);
    assert!(carry.dragging);
    assert_eq!(drag, Drag::default());
    // A tap moves nothing; a lift past the slop with no move does,
    // across or down.
    assert!(drag.down(
        touch(3, true),
        held(SectorId::Lu, "Vox 1", 0, Vec::new()),
        100.0,
        50.0
    ));
    assert_eq!(drag.up(3, 106.0, 44.0), None);
    assert_eq!(drag.held(), None);
    assert!(drag.down(
        touch(4, true),
        held(SectorId::Lu, "Vox 1", 0, Vec::new()),
        100.0,
        50.0
    ));
    assert!(drag.up(4, 93.0, 50.0).is_some());
    assert!(drag.down(
        touch(5, true),
        held(SectorId::Lu, "Vox 1", 0, Vec::new()),
        100.0,
        50.0
    ));
    assert!(drag.up(5, 100.0, 57.0).is_some(), "down to another line");
    // Moved down only: carried.
    assert!(drag.down(
        touch(6, true),
        held(SectorId::Lu, "Vox 1", 0, Vec::new()),
        100.0,
        50.0
    ));
    assert_eq!(drag.moved(6, 100.0, 43.0), Some((0.0, -7.0)));
}

#[test]
fn a_carry_whose_end_never_came_ends_at_the_next_primary_down_of_its_type() {
    let mut drag = Drag::default();
    assert!(drag.down(
        touch(1, true),
        held(SectorId::Lu, "Vox 1", 0, Vec::new()),
        100.0,
        0.0
    ));
    // A mouse never ends a finger's carry.
    let mouse = Down {
        id: 9,
        kind: Some(Pointer::Mouse),
        primary: true,
    };
    assert!(!drag.down(mouse, held(SectorId::Ru, "Keys 1", 0, Vec::new()), 0.0, 0.0));
    // The next primary finger takes over.
    assert!(drag.down(
        touch(5, true),
        held(SectorId::Ru, "Keys 1", 0, Vec::new()),
        50.0,
        0.0
    ));
    assert_eq!(
        drag.held().map(|c| (c.pointer, c.sector)),
        Some((5, SectorId::Ru))
    );
    assert_eq!(drag.held().map(|c| c.dragging), Some(false));
    // Its own pointer down again takes it too.
    assert!(drag.down(
        touch(5, false),
        held(SectorId::Lu, "Vox 1", 0, Vec::new()),
        10.0,
        20.0
    ));
    assert_eq!(
        drag.held().map(|c| (c.start_x, c.start_y)),
        Some((10.0, 20.0))
    );
    // A cancel ends only its own pointer's; a leave ends any.
    assert!(!drag.cancel(1));
    assert!(drag.cancel(5));
    assert!(!drag.cancel(5));
    assert!(drag.down(
        touch(6, true),
        held(SectorId::Lu, "Vox 1", 0, Vec::new()),
        10.0,
        0.0
    ));
    assert!(drag.leave());
    assert!(!drag.leave());
}

#[test]
fn a_keys_press_is_named_by_what_it_did() {
    assert_eq!(tap_act(Some(&Tap::Start)), "start");
    assert_eq!(tap_act(Some(&Tap::Send(Sectors::new(), 3))), "save");
    assert_eq!(tap_act(Some(&Tap::Leave)), "leave");
    assert_eq!(tap_act(None), "busy");
}

#[test]
fn the_cell_under_a_carried_strip_is_marked_only_off_its_own_slot() {
    let doc = doc();
    let mut mode = Arrange::Off;
    assert_eq!(
        mode.aim("lu-1", 1, "ru-1", 1, &r("Vox 2"), &pool()),
        Some(false),
        "off"
    );
    let _ = mode.tap(&doc);
    // Over its own slot: no mark (it only goes back there).
    assert_eq!(mode.aim("lu-1", 1, "lu-1", 1, &r("Vox 2"), &pool()), None);
    // Another slot of its own layout, a cell of another layout (the same
    // slot number there included): it lands.
    assert_eq!(
        mode.aim("lu-1", 1, "lu-1", 0, &r("Vox 2"), &pool()),
        Some(true)
    );
    assert_eq!(
        mode.aim("lu-1", 1, "ru-1", 1, &r("Vox 2"), &pool()),
        Some(true)
    );
    assert_eq!(
        mode.aim("lu-1", 1, "ru-1", 4, &r("Vox 2"), &pool()),
        Some(true)
    );
    // Klavir in both layouts (LAYOUTS > ADD put it in lu-1 too): ru-1
    // takes no drop of it, and no swap that would bring it into lu-1 again.
    moved(&mut mode, "lu-1", refs(&["Vox 1", "Klavir", "Bass"]));
    assert_eq!(
        mode.aim("lu-1", 1, "ru-1", 0, &r("Klavir"), &pool()),
        Some(false)
    );
    assert_eq!(
        mode.aim("lu-1", 1, "ru-1", 2, &r("Klavir"), &pool()),
        Some(false)
    );
    assert_eq!(
        mode.aim("lu-1", 0, "ru-1", 1, &r("Vox 1"), &pool()),
        Some(false)
    );
    assert_eq!(
        mode.aim("lu-1", 0, "ru-1", 0, &r("Vox 1"), &pool()),
        Some(true)
    );
    // A slot that no longer holds the strip: no drop.
    assert_eq!(
        mode.aim("lu-1", 2, "ru-1", 0, &r("Vox 1"), &pool()),
        Some(false)
    );
}

/// lu-1 and ru-1 with these slots, the mode on since then.
fn on_with(lu: &[&str], ru: &[&str]) -> (SectorsDoc, Arrange) {
    let doc = SectorsDoc {
        rev: 3,
        sectors: [
            (SectorId::Lu, vec![sl("lu-1", lu)]),
            (SectorId::Ru, vec![sl("ru-1", ru)]),
        ]
        .into_iter()
        .collect(),
    };
    let mode = Arrange::On {
        rev: 3,
        edit: Edit::of(&doc),
    };
    (doc, mode)
}

#[test]
fn a_move_into_a_full_layout_takes_no_drop() {
    // #236: ru-1 holds 6 units; Vox 1 dropped on its empty cell past its
    // end would make it 7, which the hub refuses FULL (and the refused
    // save would end ARRANGE, losing the session's other changes).
    let six = ["Vox 2", "Vox 3", "Vox 4", "Vox 5", "Vox 6", "Bass"];
    let (_, mut mode) = on_with(&["Vox 1", "Keys 1"], &six);
    let before = mode.clone();
    assert_eq!(mode.lands("lu-1", 0, "ru-1", 6, &r("Vox 1"), &pool()), None);
    assert_eq!(
        mode.aim("lu-1", 0, "ru-1", 6, &r("Vox 1"), &pool()),
        Some(false)
    );
    assert!(!mode.dropped_in("lu-1", 0, "ru-1", 6, &r("Vox 1"), &pool()));
    assert_eq!(mode, before, "nothing moved");
    // A swap with one of its strips keeps it at 6 units: it lands.
    assert_eq!(
        mode.aim("lu-1", 0, "ru-1", 0, &r("Vox 1"), &pool()),
        Some(true)
    );
    // Five strips and a gap are 6 units too: the gap takes the strip and
    // the layout stays at 6.
    let gap = ["Vox 2", "Vox 3", "", "Vox 5", "Vox 6", "Bass"];
    let (_, gapped) = on_with(&["Vox 1", "Keys 1"], &gap);
    assert_eq!(
        gapped.lands("lu-1", 0, "ru-1", 2, &r("Vox 1"), &pool()),
        Some(Landing::Across {
            from: refs(&["", "Keys 1"]),
            to: refs(&["Vox 2", "Vox 3", "Vox 1", "Vox 5", "Vox 6", "Bass"]),
        })
    );
}

#[test]
fn a_swap_that_overfills_the_source_with_a_wider_strip_takes_no_drop() {
    // lu-1 holds 6 units; Vox 1 swapped with ru-1's wide PA Out brings
    // 1.1 units into lu-1 (6.1: FULL), while ru-1 only shrinks.
    let six = ["Vox 1", "Vox 2", "Vox 3", "Vox 4", "Vox 5", "Vox 6"];
    let (_, mut mode) = on_with(&six, &["PA Out", "Bass"]);
    let before = mode.clone();
    assert_eq!(mode.lands("lu-1", 0, "ru-1", 0, &r("Vox 1"), &pool()), None);
    assert_eq!(
        mode.aim("lu-1", 0, "ru-1", 0, &r("Vox 1"), &pool()),
        Some(false)
    );
    assert!(!mode.dropped_in("lu-1", 0, "ru-1", 0, &r("Vox 1"), &pool()));
    assert_eq!(mode, before, "nothing moved");
    // The other way, PA Out onto Vox 1, grows lu-1 the same: no drop.
    assert_eq!(
        mode.aim("ru-1", 0, "lu-1", 0, &r("PA Out"), &pool()),
        Some(false)
    );
    // Swapped with a strip of one unit, lu-1 stays at 6: it lands.
    assert_eq!(
        mode.aim("lu-1", 0, "ru-1", 1, &r("Vox 1"), &pool()),
        Some(true)
    );
}

#[test]
fn a_move_that_fits_lands_and_a_drop_past_the_end_counts_its_gaps() {
    // ru-1 at 5 units takes one more strip of one unit.
    let five = ["Vox 2", "Vox 3", "Vox 4", "Vox 5", "Vox 6"];
    let (doc, mut mode) = on_with(&["Vox 1", "Keys 1"], &five);
    assert!(mode.dropped_in("lu-1", 0, "ru-1", 5, &r("Vox 1"), &pool()));
    let shown = mode.applied(&doc);
    assert_eq!(
        shown.sectors[&SectorId::Ru][0].strips,
        refs(&["Vox 2", "Vox 3", "Vox 4", "Vox 5", "Vox 6", "Vox 1"])
    );
    assert_eq!(
        shown.sectors[&SectorId::Lu][0].strips,
        refs(&["", "Keys 1"])
    );
    // A wide strip past 6 never fits: 5 + 1.1 units.
    let (_, wide) = on_with(&["PA Out"], &five);
    assert_eq!(
        wide.aim("lu-1", 0, "ru-1", 5, &r("PA Out"), &pool()),
        Some(false)
    );
    // Within a layout: a drop past its end leaves gaps before the strip,
    // and each gap is a unit. lu-1 at 5 takes a drop on its sixth cell,
    // never on its seventh (a gap and the strip's old slot: 7 units).
    let (_, within) = on_with(&five, &["Bass"]);
    assert_eq!(
        within.aim("lu-1", 0, "lu-1", 5, &r("Vox 2"), &pool()),
        Some(true)
    );
    assert_eq!(
        within.aim("lu-1", 0, "lu-1", 6, &r("Vox 2"), &pool()),
        Some(false)
    );
    // A layout already past 6 by a hand edit still reorders (the hub
    // refuses only a layout that grows) and takes a swap that keeps its
    // units.
    let seven = ["Vox 1", "Vox 2", "Vox 3", "Vox 4", "Vox 5", "Vox 6", "Bass"];
    let (_, over) = on_with(&seven, &["Keys 1"]);
    assert_eq!(
        over.aim("lu-1", 0, "lu-1", 6, &r("Vox 1"), &pool()),
        Some(true)
    );
    assert_eq!(
        over.aim("lu-1", 0, "ru-1", 0, &r("Vox 1"), &pool()),
        Some(true)
    );
}
