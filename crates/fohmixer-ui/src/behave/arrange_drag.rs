//! ARRANGE, the sectors page's edit mode (#148; spec
//! `docs/superpowers/specs/2026-10-28-sectors-design.md` §4, the owner's
//! approved mockups `docs/mockups/sectors-v1.html` and `sectors-v2.html`):
//! a tap on ARRANGE starts it, the strips are dragged within their sector's
//! shown layout and the layouts renamed, added and removed
//! (`behave::arrange_edit::Edit`, the page shows the edit), and a second
//! tap sends every sector as ONE `SectorEdit` (one change, one undo step)
//! and leaves once the hub's new document arrives (a refusal: at once);
//! with nothing changed it just leaves. A hide of the page, or a
//! new document from the hub during the mode, ends it unsaved. Nothing here
//! touches Live. The drag itself (`Drag`): a strip is carried once the
//! finger moved past `DRAG_SLOP`; it lands on the cell of its sector whose
//! centre is nearest the finger (the centres of every cell, strips and
//! empty ones, read once at the down): an empty cell takes it and its old
//! slot is left empty, another strip swaps places with it (the slot model,
//! 1.24.0: the owner could not put BAND at RU's right edge); a carry whose
//! end never came ends at the next primary down of its type (#124). All of
//! it is pure; `pages::sectors` carries it out.

use fohmixer_proto::sectors::{SectorId, Sectors, SectorsDoc, Slot, StripRef, trim};

use super::Down;
use super::arrange_edit::Edit;
use crate::diag::perf::Pointer;
use crate::store::{CONNECTION_CLOSED, NOT_CONNECTED};

/// How far (px) a finger moves before the strip under it is carried.
pub const DRAG_SLOP: f64 = 6.0;

/// The note under ARRANGE while the mode is on (the mockup v2's).
pub const ARRANGE_NOTE: &str = "drag · ✎ ✕ + · tap to save";

/// The note under ARRANGE when the hub refused the save (it shows its own
/// order again).
pub const REFUSED_NOTE: &str = "Not saved: the layouts changed";

/// The note under ARRANGE when the save never reached the hub (no socket,
/// or it closed before the answer).
pub const OFFLINE_NOTE: &str = "Not saved: the hub is not connected";

/// The mode: off; on since the document of revision `rev`, with its edit
/// so far; saving these sectors at `rev` (the edit stays shown until the
/// hub's answer or its new document).
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Arrange {
    #[default]
    Off,
    On {
        rev: u64,
        edit: Edit,
    },
    Saving {
        rev: u64,
        sectors: Sectors,
    },
}

/// Why the mode ended without a save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrangeEnd {
    /// The page was hidden.
    Hidden,
    /// The hub sent another document.
    NewDoc,
}

/// What a tap on ARRANGE does.
#[derive(Debug, Clone, PartialEq)]
pub enum Tap {
    /// The mode starts.
    Start,
    /// Every sector's layouts go to the hub as one edit at this revision.
    Send(Sectors, u64),
    /// The mode ends with nothing to save.
    Leave,
}

impl Arrange {
    /// A tap on ARRANGE with `doc` shown. None while a save is on its way.
    /// A rename still in its field is kept first, an ask dropped.
    pub fn tap(&mut self, doc: &SectorsDoc) -> Option<Tap> {
        match self {
            Arrange::Off => {
                *self = Arrange::On {
                    rev: doc.rev,
                    edit: Edit::of(doc),
                };
                Some(Tap::Start)
            }
            Arrange::On { rev, edit } => {
                edit.outside_down(false, false);
                let rev = *rev;
                if edit.changed(doc) {
                    let sectors = std::mem::take(&mut edit.sectors);
                    *self = Arrange::Saving {
                        rev,
                        sectors: sectors.clone(),
                    };
                    Some(Tap::Send(sectors, rev))
                } else {
                    *self = Arrange::Off;
                    Some(Tap::Leave)
                }
            }
            Arrange::Saving { .. } => None,
        }
    }

    /// The edit, while the mode is on (never while saving).
    pub fn edit(&mut self) -> Option<&mut Edit> {
        match self {
            Arrange::On { edit, .. } => Some(edit),
            _ => None,
        }
    }

    /// The edit as it stands, while the mode is on.
    pub fn edit_now(&self) -> Option<&Edit> {
        match self {
            Arrange::On { edit, .. } => Some(edit),
            _ => None,
        }
    }

    /// The hub answered the save (`ok`: taken). A refusal ends the mode at
    /// once, with its note; a taken save stays saving, its edit shown,
    /// until the hub's new document arrives (`doc_arrived`: the hub sends
    /// its result first, the document after it).
    pub fn answered(&mut self, ok: bool) -> Option<&'static str> {
        if ok {
            return None;
        }
        if matches!(self, Arrange::Saving { .. }) {
            *self = Arrange::Off;
        }
        Some(REFUSED_NOTE)
    }

    /// What the save's request heard: the hub's answer (`answered`), or a
    /// failure the hub never gave (`never_reached`), which ends a save at
    /// once with its own note.
    pub fn heard<T>(&mut self, outcome: &Result<T, String>) -> Option<&'static str> {
        match outcome {
            Ok(_) => self.answered(true),
            Err(why) if never_reached(why) => {
                if matches!(self, Arrange::Saving { .. }) {
                    *self = Arrange::Off;
                }
                Some(OFFLINE_NOTE)
            }
            Err(_) => self.answered(false),
        }
    }

    /// The hub's document of revision `rev` arrived: another revision ends
    /// the mode unsaved; while saving, only a newer one (the save's own).
    pub fn doc_arrived(&mut self, rev: u64) {
        let ends = match self {
            Arrange::Off => false,
            Arrange::On { rev: since, .. } => *since != rev,
            Arrange::Saving { rev: since, .. } => rev > *since,
        };
        if ends {
            *self = Arrange::Off;
        }
    }

    /// The mode ends unsaved (`why`); `why` again when it was on.
    pub fn ended_by(&mut self, why: ArrangeEnd) -> Option<ArrangeEnd> {
        let was = self.is_on();
        *self = Arrange::Off;
        was.then_some(why)
    }

    /// `doc` as the page shows it: the edit (its revision `doc`'s) while
    /// the mode is on or saving, else `doc`.
    pub fn applied(&self, doc: &SectorsDoc) -> SectorsDoc {
        let sectors = match self {
            Arrange::Off => &doc.sectors,
            Arrange::On { edit, .. } => &edit.sectors,
            Arrange::Saving { sectors, .. } => sectors,
        };
        SectorsDoc {
            rev: doc.rev,
            sectors: sectors.clone(),
        }
    }

    /// Whether the mode is on (saving included): the page is inert.
    pub fn is_on(&self) -> bool {
        !matches!(self, Arrange::Off)
    }

    /// The strip `held` of `layout`, carried from its slot `from` and
    /// dropped at `x` among its sector's cells on screen (`spots`: each
    /// cell's slot and centre): the layout's new slots kept in the edit
    /// (only while the mode is on).
    pub fn dropped_in(
        &mut self,
        layout: &str,
        spots: &[(usize, f64)],
        from: usize,
        held: &StripRef,
        x: f64,
    ) {
        let Some(edit) = self.edit() else {
            return;
        };
        let order = edit.slots_of(layout).map(<[Slot]>::to_vec);
        if let Some(order) = order.and_then(|o| dropped(&o, spots, from, held, x)) {
            edit.moved(layout, order);
        }
    }
}

/// The refusal note's showings, numbered: a note's timer clears it only
/// while that note is still the one shown (a second refusal within the
/// note's time keeps its full time).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoteTimer {
    shown: u64,
}

impl NoteTimer {
    /// A note shows: its number, for its timer.
    pub fn show(&mut self) -> u64 {
        self.shown += 1;
        self.shown
    }

    /// Whether the timer of note `mine` clears what shows: only its own.
    pub fn expires(&self, mine: u64) -> bool {
        self.shown == mine
    }
}

/// Whether a request's failure `why` is the page's own (not sent, or its
/// socket closed before the answer), never the hub's refusal.
pub fn never_reached(why: &str) -> bool {
    why == NOT_CONNECTED || why == CONNECTION_CLOSED
}

/// The note under ARRANGE: a refusal's while it shows, else the mode's.
pub fn arrange_note(on: bool, refused: Option<&'static str>) -> &'static str {
    refused.unwrap_or(if on { ARRANGE_NOTE } else { "" })
}

/// The slot a strip dropped at `x` lands in: the cell (`spots`: each
/// cell's slot and centre) whose centre is nearest, the first on a tie;
/// none without a cell.
pub fn drop_slot(spots: &[(usize, f64)], x: f64) -> Option<usize> {
    spots
        .iter()
        .min_by(|a, b| (a.1 - x).abs().total_cmp(&(b.1 - x).abs()))
        .map(|spot| spot.0)
}

/// `order` with its slots `from` and `to` swapped (a slot past its end is
/// empty, so a strip moved there leaves its old slot empty), the trailing
/// empty slots dropped.
pub fn swapped(order: &[Slot], from: usize, to: usize) -> Vec<Slot> {
    let mut out = order.to_vec();
    out.resize(out.len().max(from.max(to) + 1), None);
    out.swap(from, to);
    trim(out)
}

/// Whether a finger `x` px from where it went down carries the strip.
pub fn drags(start_x: f64, x: f64) -> bool {
    (x - start_x).abs() > DRAG_SLOP
}

/// The layout's slots after `held`, carried from slot `from`, is dropped
/// at `x` among `spots`; none when `from` no longer holds `held` (the
/// order changed meanwhile), there is no cell, or it lands where it was.
pub fn dropped(
    order: &[Slot],
    spots: &[(usize, f64)],
    from: usize,
    held: &StripRef,
    x: f64,
) -> Option<Vec<Slot>> {
    if order.get(from)?.as_ref() != Some(held) {
        return None;
    }
    let to = drop_slot(spots, x)?;
    (to != from).then(|| swapped(order, from, to))
}

/// A strip held under a pointer: the pointer and its type, the strip, its
/// sector and its slot, where the finger went down, its sector's cells
/// then (each cell's slot and centre), and whether it is carried yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Carry {
    pub pointer: i32,
    pub kind: Option<Pointer>,
    pub sector: SectorId,
    pub strip: StripRef,
    pub slot: usize,
    pub start_x: f64,
    pub spots: Vec<(usize, f64)>,
    pub dragging: bool,
}

/// What a down on a strip's cover holds: its sector, its strip and its
/// slot, and its sector's cells (each cell's slot and centre).
#[derive(Debug, Clone, PartialEq)]
pub struct Held {
    pub sector: SectorId,
    pub strip: StripRef,
    pub slot: usize,
    pub spots: Vec<(usize, f64)>,
}

/// The page's one drag at a time.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drag {
    held: Option<Carry>,
}

impl Drag {
    /// A pointer went down on a strip's cover at `x` (`held`: the strip,
    /// its sector, its slot and its sector's cells). Taken unless another
    /// pointer holds a strip; a primary down of the held pointer's type (or
    /// that pointer again) ends a carry whose end never came (#124) and
    /// takes this one.
    pub fn down(&mut self, down: Down, held: Held, x: f64) -> bool {
        if let Some(held) = &self.held
            && held.pointer != down.id
            && !down.ends(held.kind)
        {
            return false;
        }
        self.held = Some(Carry {
            pointer: down.id,
            kind: down.kind,
            sector: held.sector,
            strip: held.strip,
            slot: held.slot,
            start_x: x,
            spots: held.spots,
            dragging: false,
        });
        true
    }

    /// `pointer` moved to `x`: the carried strip's offset from its place,
    /// once past the slop; none for another pointer or within it.
    pub fn moved(&mut self, pointer: i32, x: f64) -> Option<f64> {
        let held = self.held.as_mut().filter(|h| h.pointer == pointer)?;
        held.dragging = held.dragging || drags(held.start_x, x);
        held.dragging.then_some(x - held.start_x)
    }

    /// `pointer` lifted at `x`: the carry ends, and comes back (with `x`)
    /// when it moved the strip; a tap moves nothing.
    pub fn up(&mut self, pointer: i32, x: f64) -> Option<(Carry, f64)> {
        let held = self.held.take_if(|h| h.pointer == pointer)?;
        (held.dragging || drags(held.start_x, x)).then_some((held, x))
    }

    /// `pointer`'s touch was cancelled (or its capture lost): the carry
    /// ends with nothing moved. Whether it held one.
    pub fn cancel(&mut self, pointer: i32) -> bool {
        self.held.take_if(|h| h.pointer == pointer).is_some()
    }

    /// Every carry ends with nothing moved (a hidden page, the mode
    /// ended). Whether one was held.
    pub fn leave(&mut self) -> bool {
        self.held.take().is_some()
    }

    /// The carry held, if any.
    pub fn held(&self) -> Option<&Carry> {
        self.held.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behave::arrange_edit::TabTap;
    use fohmixer_proto::sectors::SectorLayout;

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
    fn held(sector: SectorId, label: &str, slot: usize, spots: Vec<(usize, f64)>) -> Held {
        Held {
            sector,
            strip: r(label),
            slot,
            spots,
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
    fn a_strip_lands_on_the_cell_whose_centre_is_nearest() {
        let spots = [(0, 40.0), (1, 124.0), (2, 208.0), (5, 460.0)];
        assert_eq!(drop_slot(&spots, 0.0), Some(0));
        assert_eq!(drop_slot(&spots, 81.0), Some(0));
        assert_eq!(drop_slot(&spots, 83.0), Some(1));
        assert_eq!(drop_slot(&spots, 500.0), Some(5));
        // Halfway: the first of the two.
        assert_eq!(drop_slot(&spots, 82.0), Some(0));
        assert_eq!(drop_slot(&spots, 82.0_f64.next_up()), Some(1));
        assert_eq!(drop_slot(&[], 10.0), None);
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
        let spots = [
            (0, 40.0),
            (1, 124.0),
            (2, 208.0),
            (3, 292.0),
            (4, 376.0),
            (5, 460.0),
        ];
        assert_eq!(
            dropped(&order, &spots, 3, &r("PA"), 455.0),
            Some(refs(&["Klavir", "Bass", "Keys 1", "", "", "PA"]))
        );
        // Onto Klavir: a swap.
        assert_eq!(
            dropped(&order, &spots, 3, &r("PA"), 30.0),
            Some(refs(&["PA", "Bass", "Keys 1", "Klavir"]))
        );
        // Where it was, a slot that no longer holds it, or no cell: nothing.
        assert_eq!(dropped(&order, &spots, 3, &r("PA"), 300.0), None);
        assert_eq!(dropped(&order, &spots, 2, &r("PA"), 30.0), None);
        assert_eq!(dropped(&order, &spots, 9, &r("PA"), 30.0), None);
        assert_eq!(dropped(&order, &[], 3, &r("PA"), 30.0), None);
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
        off.dropped_in("ru-1", &spots, 3, &r("PA"), 455.0);
        assert_eq!(off, Arrange::Off);
        let mut mode = Arrange::Off;
        let _ = mode.tap(&doc);
        mode.dropped_in("ru-1", &spots, 3, &r("PA"), 455.0);
        let first = refs(&["Klavir", "Bass", "Keys 1", "", "", "PA"]);
        assert_eq!(mode.applied(&doc).sectors[&SectorId::Ru][0].strips, first);
        mode.dropped_in("ru-1", &spots, 0, &r("Klavir"), 300.0);
        let second = refs(&["", "Bass", "Keys 1", "Klavir", "", "PA"]);
        assert_eq!(mode.applied(&doc).sectors[&SectorId::Ru][0].strips, second);
        // A layout the edit does not hold takes nothing.
        let before = mode.clone();
        mode.dropped_in("gone", &spots, 0, &r("Klavir"), 300.0);
        assert_eq!(mode, before);
        // The save carries the empty slots.
        let mut sent = doc.sectors.clone();
        sent.get_mut(&SectorId::Ru).unwrap()[0].strips = second;
        assert_eq!(mode.tap(&doc), Some(Tap::Send(sent, 3)));
        let mut saving = Arrange::Saving {
            rev: 3,
            sectors: doc.sectors.clone(),
        };
        saving.dropped_in("ru-1", &spots, 3, &r("PA"), 455.0);
        assert_eq!(saving.applied(&doc), doc);
    }

    fn touch(id: i32, primary: bool) -> Down {
        Down::touch(id, primary)
    }

    #[test]
    fn a_drag_carries_one_strip_past_the_slop_and_drops_it_at_the_lift() {
        let mut drag = Drag::default();
        assert!(drag.down(
            touch(1, true),
            held(SectorId::Lu, "Vox 1", 0, vec![(0, 40.0), (1, 124.0)]),
            100.0
        ));
        // Another finger while one holds a strip: refused.
        assert!(!drag.down(
            touch(2, false),
            held(SectorId::Lu, "Vox 2", 0, Vec::new()),
            0.0
        ));
        assert_eq!(drag.held().map(|c| c.pointer), Some(1));
        assert_eq!(drag.moved(2, 300.0), None);
        assert_eq!(drag.moved(1, 106.0), None, "within the slop");
        assert_eq!(drag.moved(1, 120.0), Some(20.0));
        assert_eq!(drag.moved(1, 104.0), Some(4.0), "carried, it follows");
        assert_eq!(drag.up(2, 0.0), None);
        let (carry, x) = drag.up(1, 210.0).expect("a drop");
        assert_eq!(x, 210.0);
        assert_eq!(carry.strip, r("Vox 1"));
        assert_eq!(carry.sector, SectorId::Lu);
        assert_eq!(carry.spots, vec![(0, 40.0), (1, 124.0)]);
        assert_eq!(carry.slot, 0);
        assert!(carry.dragging);
        assert_eq!(drag, Drag::default());
        // A tap moves nothing; a lift past the slop with no move does.
        assert!(drag.down(
            touch(3, true),
            held(SectorId::Lu, "Vox 1", 0, Vec::new()),
            100.0
        ));
        assert_eq!(drag.up(3, 106.0), None);
        assert_eq!(drag.held(), None);
        assert!(drag.down(
            touch(4, true),
            held(SectorId::Lu, "Vox 1", 0, Vec::new()),
            100.0
        ));
        assert!(drag.up(4, 93.0).is_some());
    }

    #[test]
    fn a_carry_whose_end_never_came_ends_at_the_next_primary_down_of_its_type() {
        let mut drag = Drag::default();
        assert!(drag.down(
            touch(1, true),
            held(SectorId::Lu, "Vox 1", 0, Vec::new()),
            100.0
        ));
        // A mouse never ends a finger's carry.
        let mouse = Down {
            id: 9,
            kind: Some(Pointer::Mouse),
            primary: true,
        };
        assert!(!drag.down(mouse, held(SectorId::Ru, "Keys 1", 0, Vec::new()), 0.0));
        // The next primary finger takes over.
        assert!(drag.down(
            touch(5, true),
            held(SectorId::Ru, "Keys 1", 0, Vec::new()),
            50.0
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
            10.0
        ));
        assert_eq!(drag.held().map(|c| c.start_x), Some(10.0));
        // A cancel ends only its own pointer's; a leave ends any.
        assert!(!drag.cancel(1));
        assert!(drag.cancel(5));
        assert!(!drag.cancel(5));
        assert!(drag.down(
            touch(6, true),
            held(SectorId::Lu, "Vox 1", 0, Vec::new()),
            10.0
        ));
        assert!(drag.leave());
        assert!(!drag.leave());
    }
}
