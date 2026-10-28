//! ARRANGE, the sectors page's reorder mode (#148; spec
//! `docs/superpowers/specs/2026-10-28-sectors-design.md` §4, the owner's
//! approved mockup `docs/mockups/sectors-v1.html`): a tap on ARRANGE starts
//! it, the strips are dragged within their sector's shown layout and the
//! page keeps the edited orders, and a second tap sends the changed layouts
//! as ONE `SectorOrders` (one change, one undo step) and leaves once the
//! hub's new document arrives (a refusal: at once); with nothing changed it
//! just leaves. A hide of the page, or a
//! new document from the hub during the mode, ends it unsaved. Nothing here
//! touches Live. The drag itself (`Drag`): a strip is carried once the
//! finger moved past `DRAG_SLOP`; where it lands is counted from the
//! centres of its sector's other strips, read once at the down; a carry
//! whose end never came ends at the next primary down of its type (#124).
//! All of it is pure; `pages::sectors` carries it out.

use std::collections::BTreeMap;

use fohmixer_proto::sectors::{SectorId, SectorLayout, SectorOrder, SectorsDoc, StripRef};

use super::Down;
use crate::arrange::sectors::SectorArrangement;
use crate::diag::perf::Pointer;

/// How far (px) a finger moves before the strip under it is carried.
pub const DRAG_SLOP: f64 = 6.0;

/// The note under ARRANGE while the mode is on.
pub const ARRANGE_NOTE: &str = "drag to reorder · tap to save";

/// The note under ARRANGE when the hub refused the save (it shows its own
/// order again).
pub const REFUSED_NOTE: &str = "Not saved: the layouts changed";

/// The mode: off; on since the document of revision `rev`, with the
/// orders edited so far (by layout id); saving them at `rev` (the edits
/// stay shown until the hub's answer or its new document).
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Arrange {
    #[default]
    Off,
    On {
        rev: u64,
        edits: BTreeMap<String, Vec<StripRef>>,
    },
    Saving {
        rev: u64,
        edits: BTreeMap<String, Vec<StripRef>>,
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
    /// The changed layouts' orders go to the hub at this revision.
    Send(Vec<SectorOrder>, u64),
    /// The mode ends with nothing to save.
    Leave,
}

impl Arrange {
    /// A tap on ARRANGE with `doc` shown. None while a save is on its way.
    pub fn tap(&mut self, doc: &SectorsDoc) -> Option<Tap> {
        match self {
            Arrange::Off => {
                *self = Arrange::On {
                    rev: doc.rev,
                    edits: BTreeMap::new(),
                };
                Some(Tap::Start)
            }
            Arrange::On { rev, edits } => {
                let (rev, edits) = (*rev, std::mem::take(edits));
                let orders = orders_of(&edits, doc);
                if orders.is_empty() {
                    *self = Arrange::Off;
                    Some(Tap::Leave)
                } else {
                    *self = Arrange::Saving { rev, edits };
                    Some(Tap::Send(orders, rev))
                }
            }
            Arrange::Saving { .. } => None,
        }
    }

    /// A drag gave `layout` this order (kept only while the mode is on).
    pub fn moved(&mut self, layout: &str, order: Vec<StripRef>) {
        if let Arrange::On { edits, .. } = self {
            edits.insert(layout.to_string(), order);
        }
    }

    /// The hub answered the save (`ok`: taken). A refusal ends the mode at
    /// once, with its note; a taken save stays saving, its edits shown,
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

    /// The order `layout` shows: its edit, else its own.
    pub fn order_of<'a>(&'a self, layout: &'a SectorLayout) -> &'a [StripRef] {
        match self {
            Arrange::On { edits, .. } | Arrange::Saving { edits, .. } => edits
                .get(&layout.id)
                .map_or(layout.strips.as_slice(), Vec::as_slice),
            Arrange::Off => &layout.strips,
        }
    }

    /// `doc` as the page shows it: every layout in its `order_of`.
    pub fn applied(&self, doc: &SectorsDoc) -> SectorsDoc {
        SectorsDoc {
            rev: doc.rev,
            sectors: doc
                .sectors
                .iter()
                .map(|(sector, layouts)| {
                    let layouts = layouts
                        .iter()
                        .map(|layout| SectorLayout {
                            id: layout.id.clone(),
                            title: layout.title.clone(),
                            strips: self.order_of(layout).to_vec(),
                        })
                        .collect();
                    (*sector, layouts)
                })
                .collect(),
        }
    }

    /// Whether the mode is on (saving included): the page is inert.
    pub fn is_on(&self) -> bool {
        !matches!(self, Arrange::Off)
    }

    /// The strip `held` of `layout`, shown among `shown` (its sector's
    /// strips on screen, in order, at `centres`), dropped at `x`: the
    /// layout's new order kept (only while the mode is on).
    pub fn dropped_in(
        &mut self,
        layout: &SectorLayout,
        shown: &[StripRef],
        centres: &[f64],
        held: &StripRef,
        x: f64,
    ) {
        if !matches!(self, Arrange::On { .. }) {
            return;
        }
        let order = self.order_of(layout).to_vec();
        if let Some(order) = dropped(&order, shown, centres, held, x) {
            self.moved(&layout.id, order);
        }
    }
}

/// The note under ARRANGE: a refusal's while it shows, else the mode's.
pub fn arrange_note(on: bool, refused: Option<&'static str>) -> &'static str {
    refused.unwrap_or(if on { ARRANGE_NOTE } else { "" })
}

/// The save: each edited layout whose order differs from `doc`'s, as one
/// order each (a layout gone from `doc` is left out).
pub fn orders_of(edits: &BTreeMap<String, Vec<StripRef>>, doc: &SectorsDoc) -> Vec<SectorOrder> {
    edits
        .iter()
        .filter(|(id, order)| {
            doc.find(id)
                .is_some_and(|(_, layout)| layout.strips != **order)
        })
        .map(|(id, order)| SectorOrder {
            layout: id.clone(),
            strips: order.clone(),
        })
        .collect()
}

/// The dragged strip's new index: how many of the other strips' centres
/// lie at or left of the finger (the strip goes before the first whose
/// middle is right of it, as the mockup's drag).
pub fn drop_index(centres: &[f64], from: usize, x: f64) -> usize {
    centres
        .iter()
        .enumerate()
        .filter(|(i, centre)| *i != from && **centre <= x)
        .count()
}

/// `order` with its item at `from` moved to `to` (clamped to the end);
/// unchanged for a `from` past it.
pub fn moved_to(order: &[StripRef], from: usize, to: usize) -> Vec<StripRef> {
    let mut out = order.to_vec();
    if from >= out.len() {
        return out;
    }
    let item = out.remove(from);
    let to = to.min(out.len());
    out.insert(to, item);
    out
}

/// Whether a finger `x` px from where it went down carries the strip.
pub fn drags(start_x: f64, x: f64) -> bool {
    (x - start_x).abs() > DRAG_SLOP
}

/// `order` with the places of `shown`'s strips taken by `reordered` in
/// turn: a strip not on screen (a ref no pool strip carries, one outside a
/// window) keeps its place.
pub fn placed_back(
    order: &[StripRef],
    shown: &[StripRef],
    reordered: &[StripRef],
) -> Vec<StripRef> {
    let mut next = reordered.iter();
    order
        .iter()
        .map(|strip| {
            if shown.contains(strip) {
                next.next().unwrap_or(strip).clone()
            } else {
                strip.clone()
            }
        })
        .collect()
}

/// The layout's order after `held`, one of `shown` (its sector's strips on
/// screen at `centres`), is dropped at `x`; none when `held` is not shown
/// or the screen's strips are not those `shown` names.
pub fn dropped(
    order: &[StripRef],
    shown: &[StripRef],
    centres: &[f64],
    held: &StripRef,
    x: f64,
) -> Option<Vec<StripRef>> {
    if shown.len() != centres.len() {
        return None;
    }
    let from = shown.iter().position(|strip| strip == held)?;
    let reordered = moved_to(shown, from, drop_index(centres, from, x));
    Some(placed_back(order, shown, &reordered))
}

/// `sector`'s strips on line `line`, in their order on screen.
pub fn sector_refs(arranged: &SectorArrangement, line: usize, sector: SectorId) -> Vec<StripRef> {
    arranged
        .lines
        .get(line)
        .map(|l| {
            l.left
                .cells
                .iter()
                .chain(l.right.cells.iter())
                .filter(|cell| cell.sector == sector)
                .map(|cell| cell.strip.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// A strip held under a pointer: the pointer and its type, the strip and
/// its sector, where the finger went down, its sector's strips' centres
/// then, and whether it is carried yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Carry {
    pub pointer: i32,
    pub kind: Option<Pointer>,
    pub sector: SectorId,
    pub strip: StripRef,
    pub start_x: f64,
    pub centres: Vec<f64>,
    pub dragging: bool,
}

/// The page's one drag at a time.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drag {
    held: Option<Carry>,
}

impl Drag {
    /// A pointer went down on `strip`'s cover at `x`, its sector's strips'
    /// centres `centres`. Taken unless another pointer holds a strip; a
    /// primary down of the held pointer's type (or that pointer again) ends
    /// a carry whose end never came (#124) and takes this one.
    pub fn down(
        &mut self,
        down: Down,
        sector: SectorId,
        strip: StripRef,
        x: f64,
        centres: Vec<f64>,
    ) -> bool {
        if let Some(held) = &self.held
            && held.pointer != down.id
            && !down.ends(held.kind)
        {
            return false;
        }
        self.held = Some(Carry {
            pointer: down.id,
            kind: down.kind,
            sector,
            strip,
            start_x: x,
            centres,
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

    fn r(label: &str) -> StripRef {
        StripRef {
            instance: "band".to_string(),
            label: label.to_string(),
        }
    }

    fn refs(labels: &[&str]) -> Vec<StripRef> {
        labels.iter().map(|l| r(l)).collect()
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

    fn on(rev: u64) -> Arrange {
        Arrange::On {
            rev,
            edits: BTreeMap::new(),
        }
    }

    #[test]
    fn a_first_tap_starts_and_a_second_without_changes_leaves() {
        let doc = doc();
        let mut mode = Arrange::Off;
        assert!(!mode.is_on());
        assert_eq!(mode.tap(&doc), Some(Tap::Start));
        assert_eq!(mode, on(3));
        assert!(mode.is_on());
        assert_eq!(mode.tap(&doc), Some(Tap::Leave));
        assert_eq!(mode, Arrange::Off);
        // An edit back to the layout's own order is no change.
        let _ = mode.tap(&doc);
        mode.moved("lu-1", refs(&["Vox 1", "Vox 2", "Bass"]));
        assert_eq!(mode.tap(&doc), Some(Tap::Leave));
        assert_eq!(mode, Arrange::Off);
    }

    #[test]
    fn a_second_tap_with_changes_sends_one_orders_message_of_the_changed_layouts_only() {
        let doc = doc();
        let mut mode = Arrange::Off;
        let _ = mode.tap(&doc);
        mode.moved("lu-1", refs(&["Vox 2", "Vox 1", "Bass"]));
        mode.moved("ru-1", refs(&["Keys 1", "Klavir"]));
        // A layout gone from the document is left out.
        mode.moved("gone", refs(&["Vox 1"]));
        let sent = mode.tap(&doc);
        assert_eq!(
            sent,
            Some(Tap::Send(
                vec![SectorOrder {
                    layout: "lu-1".to_string(),
                    strips: refs(&["Vox 2", "Vox 1", "Bass"]),
                }],
                3
            ))
        );
        assert!(mode.is_on(), "saving keeps the page inert");
        // The edits stay shown while saving; a tap meanwhile does nothing,
        // and so does a move.
        assert_eq!(
            mode.order_of(&doc.sectors[&SectorId::Lu][0]),
            refs(&["Vox 2", "Vox 1", "Bass"]).as_slice()
        );
        assert_eq!(mode.tap(&doc), None);
        let before = mode.clone();
        mode.moved("lu-1", refs(&["Bass", "Vox 1", "Vox 2"]));
        assert_eq!(mode, before);
    }

    #[test]
    fn a_taken_save_keeps_its_edits_shown_until_its_document_arrives() {
        let doc = doc();
        let saving = || {
            let mut mode = Arrange::Off;
            let _ = mode.tap(&doc);
            mode.moved("lu-1", refs(&["Bass", "Vox 1", "Vox 2"]));
            let _ = mode.tap(&doc);
            mode
        };
        // The hub sends its result before the new document: between the
        // two the page must still show the edits, never the old order.
        let mut taken = saving();
        assert_eq!(taken.answered(true), None);
        assert!(taken.is_on(), "still saving");
        assert_eq!(
            taken.order_of(&doc.sectors[&SectorId::Lu][0]),
            refs(&["Bass", "Vox 1", "Vox 2"]).as_slice()
        );
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
        let mut mode = on(3);
        assert_eq!(mode.answered(true), None);
        assert_eq!(mode, on(3));
        assert_eq!(mode.answered(false), Some(REFUSED_NOTE));
        assert_eq!(mode, on(3));
    }

    #[test]
    fn a_new_document_during_arrange_ends_it_unsaved() {
        let mut mode = on(3);
        mode.moved("lu-1", refs(&["Bass"]));
        let kept = mode.clone();
        mode.doc_arrived(3);
        assert_eq!(mode, kept, "the same revision (a reconnect) keeps it");
        mode.doc_arrived(4);
        assert_eq!(mode, Arrange::Off);
        let mut older = on(3);
        older.doc_arrived(2);
        assert_eq!(older, Arrange::Off);
        // Saving at 3: its own answer's document is 4; 3 again keeps it.
        let saving = |rev| Arrange::Saving {
            rev,
            edits: BTreeMap::new(),
        };
        let mut mode = saving(3);
        mode.doc_arrived(3);
        assert_eq!(mode, saving(3));
        mode.doc_arrived(4);
        assert_eq!(mode, Arrange::Off);
        let mut off = Arrange::Off;
        off.doc_arrived(9);
        assert_eq!(off, Arrange::Off);
    }

    #[test]
    fn a_hide_ends_it_unsaved() {
        let mut mode = on(3);
        mode.moved("lu-1", refs(&["Bass"]));
        assert_eq!(mode.ended_by(ArrangeEnd::Hidden), Some(ArrangeEnd::Hidden));
        assert_eq!(mode, Arrange::Off);
        assert_eq!(mode.ended_by(ArrangeEnd::Hidden), None);
        let mut saving = Arrange::Saving {
            rev: 3,
            edits: BTreeMap::new(),
        };
        assert_eq!(
            saving.ended_by(ArrangeEnd::NewDoc),
            Some(ArrangeEnd::NewDoc)
        );
        assert_eq!(saving, Arrange::Off);
    }

    #[test]
    fn the_page_shows_the_edited_orders() {
        let doc = doc();
        let mut mode = Arrange::Off;
        assert_eq!(mode.applied(&doc), doc);
        let _ = mode.tap(&doc);
        mode.moved("ru-1", refs(&["Klavir", "Keys 1"]));
        let shown = mode.applied(&doc);
        assert_eq!(shown.rev, 3);
        assert_eq!(
            shown.sectors[&SectorId::Ru][0].strips,
            refs(&["Klavir", "Keys 1"])
        );
        assert_eq!(shown.sectors[&SectorId::Ru][0].title, "RU-1");
        assert_eq!(shown.sectors[&SectorId::Lu], doc.sectors[&SectorId::Lu]);
    }

    #[test]
    fn the_drop_index_counts_the_other_centres_left_of_the_finger() {
        let centres = [40.0, 124.0, 208.0, 292.0];
        assert_eq!(drop_index(&centres, 0, 200.0), 1);
        assert_eq!(drop_index(&centres, 0, 210.0), 2);
        assert_eq!(drop_index(&centres, 0, 0.0), 0);
        assert_eq!(drop_index(&centres, 0, 400.0), 3);
        // A centre exactly under the finger counts; the float before not.
        assert_eq!(drop_index(&centres, 0, 124.0), 1);
        assert_eq!(drop_index(&centres, 0, 124.0_f64.next_down()), 0);
        // Its own centre never counts.
        assert_eq!(drop_index(&centres, 3, 400.0), 3);
        assert_eq!(drop_index(&centres, 1, 130.0), 1);
    }

    #[test]
    fn moved_to_keeps_every_ref_once() {
        let order = refs(&["A", "B", "C", "D"]);
        assert_eq!(moved_to(&order, 0, 2), refs(&["B", "C", "A", "D"]));
        assert_eq!(moved_to(&order, 3, 0), refs(&["D", "A", "B", "C"]));
        assert_eq!(moved_to(&order, 1, 1), order);
        assert_eq!(moved_to(&order, 0, 9), refs(&["B", "C", "D", "A"]));
        assert_eq!(moved_to(&order, 4, 0), order);
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
    fn a_drop_reorders_the_shown_strips_and_keeps_the_others_in_place() {
        // Gone 1 is no pool strip: it stays where it was.
        let order = refs(&["Vox 1", "Gone 1", "Vox 2", "Bass"]);
        let shown = refs(&["Vox 1", "Vox 2", "Bass"]);
        let centres = [40.0, 124.0, 208.0];
        assert_eq!(
            dropped(&order, &shown, &centres, &r("Vox 1"), 210.0),
            Some(refs(&["Vox 2", "Gone 1", "Bass", "Vox 1"]))
        );
        assert_eq!(
            dropped(&order, &shown, &centres, &r("Bass"), 0.0),
            Some(refs(&["Bass", "Gone 1", "Vox 1", "Vox 2"]))
        );
        // Not shown, or the screen holds other strips: nothing.
        assert_eq!(dropped(&order, &shown, &centres, &r("Gone 1"), 0.0), None);
        assert_eq!(
            dropped(&order, &shown, &centres[..2], &r("Vox 1"), 210.0),
            None
        );
        assert_eq!(
            placed_back(&order, &shown, &refs(&["Bass"])),
            refs(&["Bass", "Gone 1", "Vox 2", "Bass"])
        );
        // Into the mode: only while it is on, edited onto its own edit.
        let layout = SectorLayout {
            id: "lu-1".to_string(),
            title: "VOX".to_string(),
            strips: order.clone(),
        };
        let mut off = Arrange::Off;
        off.dropped_in(&layout, &shown, &centres, &r("Vox 1"), 210.0);
        assert_eq!(off, Arrange::Off);
        let mut mode = on(3);
        mode.dropped_in(&layout, &shown, &centres, &r("Vox 1"), 210.0);
        let first = refs(&["Vox 2", "Gone 1", "Bass", "Vox 1"]);
        assert_eq!(mode.order_of(&layout), first.as_slice());
        let shown = refs(&["Vox 2", "Bass", "Vox 1"]);
        mode.dropped_in(&layout, &shown, &centres, &r("Vox 1"), 0.0);
        assert_eq!(
            mode.order_of(&layout),
            refs(&["Vox 1", "Gone 1", "Vox 2", "Bass"]).as_slice()
        );
        let mut saving = Arrange::Saving {
            rev: 3,
            edits: BTreeMap::new(),
        };
        saving.dropped_in(&layout, &shown, &centres, &r("Vox 1"), 210.0);
        assert_eq!(saving.order_of(&layout), order.as_slice());
    }

    #[test]
    fn the_note_says_the_mode_or_the_refusal() {
        assert_eq!(arrange_note(false, None), "");
        assert_eq!(arrange_note(true, None), ARRANGE_NOTE);
        assert_eq!(arrange_note(false, Some(REFUSED_NOTE)), REFUSED_NOTE);
        assert_eq!(arrange_note(true, Some(REFUSED_NOTE)), REFUSED_NOTE);
        assert_eq!(ARRANGE_NOTE, "drag to reorder · tap to save");
    }

    #[test]
    fn a_sectors_strips_on_a_line_are_its_cells_in_order() {
        use crate::arrange::sectors::{SectorCell, SectorLine, SectorSide, WindowKey};
        let cell = |sector, label: &str| SectorCell {
            sector,
            strip: r(label),
            units: 1.0,
        };
        let side = |cells| SectorSide {
            key: WindowKey {
                sectors: vec![SectorId::Lu, SectorId::Ld],
            },
            cells,
            shift: None,
        };
        let arranged = SectorArrangement {
            width: 84.0,
            lines: vec![SectorLine {
                left: side(vec![
                    cell(SectorId::Lu, "Vox 1"),
                    cell(SectorId::Lu, "Vox 2"),
                    cell(SectorId::Ld, "Bass"),
                ]),
                right: side(vec![cell(SectorId::Ru, "Keys 1")]),
            }],
        };
        assert_eq!(
            sector_refs(&arranged, 0, SectorId::Lu),
            refs(&["Vox 1", "Vox 2"])
        );
        assert_eq!(sector_refs(&arranged, 0, SectorId::Ld), refs(&["Bass"]));
        assert_eq!(sector_refs(&arranged, 0, SectorId::Ru), refs(&["Keys 1"]));
        assert_eq!(sector_refs(&arranged, 1, SectorId::Lu), Vec::new());
    }

    fn touch(id: i32, primary: bool) -> Down {
        Down::touch(id, primary)
    }

    #[test]
    fn a_drag_carries_one_strip_past_the_slop_and_drops_it_at_the_lift() {
        let mut drag = Drag::default();
        assert!(drag.down(
            touch(1, true),
            SectorId::Lu,
            r("Vox 1"),
            100.0,
            vec![40.0, 124.0]
        ));
        // Another finger while one holds a strip: refused.
        assert!(!drag.down(touch(2, false), SectorId::Lu, r("Vox 2"), 0.0, Vec::new()));
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
        assert_eq!(carry.centres, vec![40.0, 124.0]);
        assert!(carry.dragging);
        assert_eq!(drag, Drag::default());
        // A tap moves nothing; a lift past the slop with no move does.
        assert!(drag.down(touch(3, true), SectorId::Lu, r("Vox 1"), 100.0, Vec::new()));
        assert_eq!(drag.up(3, 106.0), None);
        assert_eq!(drag.held(), None);
        assert!(drag.down(touch(4, true), SectorId::Lu, r("Vox 1"), 100.0, Vec::new()));
        assert!(drag.up(4, 93.0).is_some());
    }

    #[test]
    fn a_carry_whose_end_never_came_ends_at_the_next_primary_down_of_its_type() {
        let mut drag = Drag::default();
        assert!(drag.down(touch(1, true), SectorId::Lu, r("Vox 1"), 100.0, Vec::new()));
        // A mouse never ends a finger's carry.
        let mouse = Down {
            id: 9,
            kind: Some(Pointer::Mouse),
            primary: true,
        };
        assert!(!drag.down(mouse, SectorId::Ru, r("Keys 1"), 0.0, Vec::new()));
        // The next primary finger takes over.
        assert!(drag.down(touch(5, true), SectorId::Ru, r("Keys 1"), 50.0, Vec::new()));
        assert_eq!(
            drag.held().map(|c| (c.pointer, c.sector)),
            Some((5, SectorId::Ru))
        );
        assert_eq!(drag.held().map(|c| c.dragging), Some(false));
        // Its own pointer down again takes it too.
        assert!(drag.down(touch(5, false), SectorId::Lu, r("Vox 1"), 10.0, Vec::new()));
        assert_eq!(drag.held().map(|c| c.start_x), Some(10.0));
        // A cancel ends only its own pointer's; a leave ends any.
        assert!(!drag.cancel(1));
        assert!(drag.cancel(5));
        assert!(!drag.cancel(5));
        assert!(drag.down(touch(6, true), SectorId::Lu, r("Vox 1"), 10.0, Vec::new()));
        assert!(drag.leave());
        assert!(!drag.leave());
    }
}
