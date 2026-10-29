use super::*;
use fohmixer_proto::sectors::StripRef;

fn r(label: &str) -> StripRef {
    StripRef {
        instance: "band".to_string(),
        label: label.to_string(),
    }
}

fn sl(id: &str, title: &str, labels: &[&str]) -> SectorLayout {
    SectorLayout {
        id: id.to_string(),
        title: title.to_string(),
        strips: labels.iter().map(|l| Some(r(l))).collect(),
    }
}

/// lu: lu-1 VOX [Vox 1, Vox 2], lu-3 KEYS [Klavir]; ru: ru-1 BAND [Bass]
/// at revision 5 (lu-2 removed before; new ids carry the next revision,
/// 6).
fn doc() -> SectorsDoc {
    SectorsDoc {
        rev: 5,
        sectors: [
            (
                SectorId::Lu,
                vec![
                    sl("lu-1", "VOX", &["Vox 1", "Vox 2"]),
                    sl("lu-3", "KEYS", &["Klavir"]),
                ],
            ),
            (SectorId::Ru, vec![sl("ru-1", "BAND", &["Bass"])]),
        ]
        .into_iter()
        .collect(),
    }
}

fn titles(edit: &Edit, sector: SectorId) -> Vec<String> {
    edit.sectors[&sector]
        .iter()
        .map(|l| l.title.clone())
        .collect()
}

#[test]
fn an_edit_starts_as_the_document_and_is_no_change() {
    let doc = doc();
    let edit = Edit::of(&doc);
    assert_eq!(edit.sectors, doc.sectors);
    assert!(!edit.changed(&doc));
    assert_eq!(edit.renaming, None);
    assert_eq!(edit.confirming, None);
    // Trailing empty slots change nothing.
    let mut trailing = Edit::of(&doc);
    trailing.moved("ru-1", vec![Some(r("Bass")), None]);
    assert!(!trailing.changed(&doc));
    trailing.moved("ru-1", vec![None, Some(r("Bass"))]);
    assert!(trailing.changed(&doc));
    assert_eq!(
        trailing.slots_of("ru-1"),
        Some([None, Some(r("Bass"))].as_slice())
    );
    assert_eq!(trailing.slots_of("gone"), None);
    // A layout the edit does not hold takes no slots.
    trailing.moved("gone", vec![Some(r("Bass"))]);
    assert_eq!(trailing.slots_of("gone"), None);
}

#[test]
fn the_field_keeps_8_capitals_and_a_rename_keeps_its_trimmed_text() {
    assert_eq!(field_text("monitors"), "MONITORS");
    assert_eq!(field_text("monitors2"), "MONITORS", "cut to 8");
    assert_eq!(field_text("žive"), "ŽIVE");
    assert_eq!(field_text("a\u{7}b"), "AB", "no control character");
    assert_eq!(committed(" NEW "), Some("NEW".to_string()));
    assert_eq!(committed("   "), None);
    assert_eq!(committed(""), None);
    let doc = doc();
    let mut edit = Edit::of(&doc);
    assert_eq!(edit.rename_start("lu-3"), Some("KEYS".to_string()));
    assert_eq!(
        edit.renaming,
        Some(("lu-3".to_string(), "KEYS".to_string()))
    );
    assert_eq!(edit.typed("monitors2"), "MONITORS");
    assert_eq!(
        edit.renaming,
        Some(("lu-3".to_string(), "MONITORS".to_string()))
    );
    edit.rename_end(true);
    assert_eq!(edit.renaming, None);
    assert_eq!(titles(&edit, SectorId::Lu), ["VOX", "MONITORS"]);
    assert!(edit.changed(&doc));
    // Escape keeps the old title; a blank field does too.
    assert_eq!(edit.rename_start("lu-1"), Some("VOX".to_string()));
    let _ = edit.typed("drums");
    edit.rename_end(false);
    assert_eq!(titles(&edit, SectorId::Lu), ["VOX", "MONITORS"]);
    let _ = edit.rename_start("lu-1");
    let _ = edit.typed("  ");
    edit.rename_end(true);
    assert_eq!(titles(&edit, SectorId::Lu), ["VOX", "MONITORS"]);
    // No rename running: typing and an end change nothing.
    assert_eq!(edit.typed("x"), "X");
    edit.rename_end(true);
    assert_eq!(edit.renaming, None);
    assert_eq!(edit.rename_start("gone"), None);
    assert_eq!(edit.renaming, None);
    // A rename drops a pending REMOVE?.
    edit.ask_remove("lu-1");
    let _ = edit.rename_start("lu-3");
    assert_eq!(edit.confirming, None);
}

#[test]
fn a_plus_adds_new_with_an_id_never_used_up_to_4_layouts() {
    let doc = doc();
    let mut edit = Edit::of(&doc);
    assert!(edit.can_add(SectorId::Lu));
    assert_eq!(edit.add(SectorId::Lu), Some("lu-r6-1".to_string()));
    assert_eq!(titles(&edit, SectorId::Lu), ["VOX", "KEYS", "NEW"]);
    assert_eq!(edit.sectors[&SectorId::Lu][2].strips, Vec::<Slot>::new());
    assert_eq!(edit.add(SectorId::Lu), Some("lu-r6-2".to_string()));
    assert_eq!(titles(&edit, SectorId::Lu), ["VOX", "KEYS", "NEW", "NEW 2"]);
    assert!(!edit.can_add(SectorId::Lu), "4 layouts: the + is gone");
    assert_eq!(edit.add(SectorId::Lu), None);
    assert_eq!(edit.sectors[&SectorId::Lu].len(), MAX_LAYOUTS);
    // A sector the document does not name gets none.
    assert!(!edit.can_add(SectorId::Rd));
    assert_eq!(edit.add(SectorId::Rd), None);
    // An id removed in this edit is not used again.
    edit.ask_remove("lu-r6-2");
    assert_eq!(edit.tab_tapped("lu-r6-2"), TabTap::Removed);
    assert_eq!(edit.add(SectorId::Lu), Some("lu-r6-3".to_string()));
    assert_eq!(titles(&edit, SectorId::Lu), ["VOX", "KEYS", "NEW", "NEW 2"]);
    // An add drops a pending REMOVE?.
    let mut ru = Edit::of(&doc);
    ru.add(SectorId::Ru);
    ru.ask_remove("ru-1");
    assert_eq!(ru.add(SectorId::Ru), Some("ru-r6-2".to_string()));
    assert_eq!(ru.confirming, None);
}

#[test]
fn a_new_title_counts_from_what_is_taken_and_an_id_carries_its_revision() {
    let new = |titles: &[&str]| {
        new_title(
            &titles
                .iter()
                .map(|t| sl("x", t, &[]))
                .collect::<Vec<SectorLayout>>(),
        )
    };
    assert_eq!(new(&[]), "NEW");
    assert_eq!(new(&["VOX"]), "NEW");
    assert_eq!(new(&["NEW"]), "NEW 2");
    assert_eq!(new(&["NEW", "NEW 2", "NEW 3"]), "NEW 4");
    assert_eq!(new(&["NEW 2", "NEW"]), "NEW 3");
    assert_eq!(new(&["NEW 2"]), "NEW");
    assert_eq!(new(&["NEW", "NEW 2", "NEW 3", "NEW 4"]), "NEW 5");
    assert_eq!(
        new(&["NEW", "NEW 2", "NEW 3", "NEW 4", "NEW 5"]),
        "NEW",
        "none free (a hand edit's five): NEW again"
    );
    assert_eq!(new_id(SectorId::Lu, 6, 1), "lu-r6-1");
    assert_eq!(new_id(SectorId::Ru, 6, 2), "ru-r6-2");
    assert_eq!(new_id(SectorId::Rd, 12, 10), "rd-r12-10");
}

/// `doc` with `id` (an empty NEW layout) added to lu, one revision on: the
/// hub's document after a save of that add.
fn saved_with(doc: &SectorsDoc, id: &str) -> SectorsDoc {
    let mut next = doc.clone();
    next.rev += 1;
    if let Some(lu) = next.sectors.get_mut(&SectorId::Lu) {
        lu.push(sl(id, "NEW", &[]));
    }
    next
}

/// `doc` without the layout `id`, one revision on (a save of its removal).
fn saved_without(doc: &SectorsDoc, id: &str) -> SectorsDoc {
    let mut next = doc.clone();
    next.rev += 1;
    for layouts in next.sectors.values_mut() {
        layouts.retain(|l| l.id != id);
    }
    next
}

#[test]
fn an_id_removed_in_an_earlier_revision_is_never_used_again() {
    // Added and saved (revision 6), removed and saved (7), then a later
    // session adds: never the removed id.
    let first = doc();
    let added = Edit::of(&first).add(SectorId::Lu).expect("an id");
    let with = saved_with(&first, &added);
    let removed = saved_without(&with, &added);
    assert_eq!(removed.rev, 7);
    let again = Edit::of(&removed).add(SectorId::Lu).expect("an id");
    assert_ne!(again, added);
    assert_eq!(again, "lu-r8-1");
    // The review's case: a layout of the document (lu-3) removed and saved,
    // then a new session's add: not lu-3 again.
    let gone = saved_without(&first, "lu-3");
    let next = Edit::of(&gone).add(SectorId::Lu).expect("an id");
    assert_ne!(next, "lu-3");
    assert_eq!(next, "lu-r7-1");
}

#[test]
fn a_hand_edit_back_to_older_layouts_then_an_add_never_uses_the_taken_out_id_again() {
    // The add saved at revision 6; a hand edit of sectors.json brings back
    // revision 5's layouts, served at revision 7 (a change of its own).
    let first = doc();
    let added = Edit::of(&first).add(SectorId::Lu).expect("an id");
    assert_eq!(added, "lu-r6-1");
    let with = saved_with(&first, &added);
    let back = SectorsDoc {
        rev: with.rev + 1,
        sectors: first.sectors.clone(),
    };
    let again = Edit::of(&back).add(SectorId::Lu).expect("an id");
    assert_ne!(again, added);
    assert_eq!(again, "lu-r8-1");
}

#[test]
fn a_cross_asks_first_and_the_tab_then_removes_but_never_the_last() {
    let doc = doc();
    let mut edit = Edit::of(&doc);
    assert!(edit.removable(SectorId::Lu));
    assert!(!edit.removable(SectorId::Ru), "the last layout has no ✕");
    assert!(!edit.removable(SectorId::Rd));
    // ✕ asks; ✕ again takes the question back.
    edit.ask_remove("lu-3");
    assert_eq!(edit.confirming, Some("lu-3".to_string()));
    edit.ask_remove("lu-3");
    assert_eq!(edit.confirming, None);
    // Another tab while one asks: no removal, it pages and the ask goes.
    edit.ask_remove("lu-3");
    assert_eq!(edit.tab_tapped("lu-1"), TabTap::Page);
    assert_eq!(edit.confirming, None);
    assert_eq!(edit.sectors[&SectorId::Lu].len(), 2);
    // A tap elsewhere drops it; on the REMOVE? tab or a ✕ it stays.
    edit.ask_remove("lu-3");
    edit.outside_down(true, false);
    assert_eq!(edit.confirming, Some("lu-3".to_string()));
    edit.outside_down(false, false);
    assert_eq!(edit.confirming, None);
    // The REMOVE? tab removes it.
    edit.ask_remove("lu-3");
    assert_eq!(edit.tab_tapped("lu-3"), TabTap::Removed);
    assert_eq!(titles(&edit, SectorId::Lu), ["VOX"]);
    assert_eq!(edit.confirming, None);
    assert!(edit.changed(&doc));
    // Now lu's last: its ✕ asks nothing.
    assert!(!edit.removable(SectorId::Lu));
    edit.ask_remove("lu-1");
    assert_eq!(edit.confirming, None);
    assert_eq!(edit.tab_tapped("lu-1"), TabTap::Page);
    assert_eq!(titles(&edit, SectorId::Lu), ["VOX"]);
    // ru's only layout likewise.
    edit.ask_remove("ru-1");
    assert_eq!(edit.confirming, None);
    // An unknown id asks nothing.
    edit.ask_remove("gone");
    assert_eq!(edit.confirming, None);
}

#[test]
fn a_field_ends_only_its_own_rename() {
    let doc = doc();
    let mut edit = Edit::of(&doc);
    let _ = edit.rename_start("lu-1");
    let _ = edit.typed("lead");
    // lu-3's field losing focus ends nothing of lu-1's.
    edit.rename_end_of("lu-3", true);
    assert_eq!(
        edit.renaming,
        Some(("lu-1".to_string(), "LEAD".to_string()))
    );
    edit.rename_end_of("lu-1", false);
    assert_eq!(edit.renaming, None);
    assert_eq!(titles(&edit, SectorId::Lu), ["VOX", "KEYS"]);
    let _ = edit.rename_start("lu-1");
    let _ = edit.typed("lead");
    edit.rename_end_of("lu-1", true);
    assert_eq!(titles(&edit, SectorId::Lu), ["LEAD", "KEYS"]);
}

#[test]
fn a_tap_elsewhere_ends_a_rename_keeping_its_text() {
    let doc = doc();
    let mut edit = Edit::of(&doc);
    let _ = edit.rename_start("ru-1");
    let _ = edit.typed("band b");
    // On the field itself: it goes on.
    edit.outside_down(false, true);
    assert!(edit.renaming.is_some());
    edit.outside_down(false, false);
    assert_eq!(edit.renaming, None);
    assert_eq!(titles(&edit, SectorId::Ru), ["BAND B"]);
}

#[test]
fn the_tabs_show_the_edit_the_pick_the_ask_and_the_field() {
    let doc = doc();
    let mut edit = Edit::of(&doc);
    let picks: Picks = [(SectorId::Lu, "lu-3".to_string())].into_iter().collect();
    let row = |id: &str, title: &str, selected: bool| TabRow {
        id: id.to_string(),
        title: title.to_string(),
        selected,
        asks: false,
        renaming: false,
        removable: true,
    };
    // Outside ARRANGE: the document's tabs, the pick marked.
    assert_eq!(
        tab_rows(&doc, &picks, SectorId::Lu, None),
        vec![row("lu-1", "VOX", false), row("lu-3", "KEYS", true)]
    );
    // A sector with no pick marks its first; its only layout is not
    // removable.
    assert_eq!(
        tab_rows(&doc, &picks, SectorId::Ru, None),
        vec![TabRow {
            removable: false,
            ..row("ru-1", "BAND", true)
        }]
    );
    assert_eq!(
        tab_rows(&doc, &picks, SectorId::Rd, None),
        Vec::<TabRow>::new()
    );
    // In ARRANGE: the ask and the field show on their tab.
    edit.ask_remove("lu-1");
    let shown = SectorsDoc {
        rev: doc.rev,
        sectors: edit.sectors.clone(),
    };
    assert_eq!(
        tab_rows(&shown, &picks, SectorId::Lu, Some(&edit)),
        vec![
            TabRow {
                asks: true,
                ..row("lu-1", "VOX", false)
            },
            row("lu-3", "KEYS", true)
        ]
    );
    let _ = edit.rename_start("lu-3");
    assert_eq!(
        tab_rows(&shown, &picks, SectorId::Lu, Some(&edit)),
        vec![
            row("lu-1", "VOX", false),
            TabRow {
                renaming: true,
                ..row("lu-3", "KEYS", true)
            }
        ]
    );
    assert_eq!(REMOVE_ASK, "REMOVE?");
    assert_eq!(NEW_TITLE, "NEW");
}
