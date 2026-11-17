//! ARRANGE, the sectors page's edit mode (#148; spec
//! `docs/superpowers/specs/2026-10-28-sectors-design.md` §4, the owner's
//! approved mockups `docs/mockups/sectors-v1.html` and `sectors-v2.html`):
//! a tap on ARRANGE starts it, the strips are dragged within and between
//! the sectors' shown layouts and the layouts renamed, added and removed
//! (`behave::arrange_edit::Edit`, the page shows the edit), and a second
//! tap sends every sector as ONE `SectorEdit` (one change)
//! and leaves once the hub's new document arrives (a refusal: at once);
//! with nothing changed it just leaves. A hide of the page, or a
//! new document from the hub during the mode, ends it unsaved. Nothing here
//! touches Live. The drag itself (`Drag`): a strip is carried once the
//! finger moved past `DRAG_SLOP`; it lands on the cell nearest the finger
//! among every visible sector's cells (`Spot`s, strips and empty ones, read
//! once at the down; #233: another sector's too): the cells of the finger's
//! line first, then the nearest centre. An empty cell takes it and its old
//! slot is left empty, another strip swaps places with it (the slot model,
//! 1.24.0: the owner could not put BAND at RU's right edge), also between
//! two layouts (a MOVE: the strip leaves the layout it came from); a layout
//! that already holds the strip, or a swap that would put a strip twice in
//! the strip's own layout, takes no drop (`landing`); nor does one that
//! would grow a layout past FULL, as the hub's save would refuse it (#236,
//! `fits`: the strip's layout on a swap with a wider strip, the target on a
//! move, a layout's own gaps past its end). A carry whose end
//! never came ends at the next primary down of its type (#124). All of it
//! is pure; `pages::sectors` carries it out.

use fohmixer_proto::sectors::{
    Pooled, Refusal, SectorId, Sectors, SectorsDoc, Slot, StripRef, grows_past_full, trim, units,
};

use super::Down;
use super::arrange_edit::Edit;
use crate::diag::perf::Pointer;
use crate::store::{CONNECTION_CLOSED, NOT_CONNECTED};

/// How far (px) a finger moves before the strip under it is carried.
pub const DRAG_SLOP: f64 = 6.0;

/// The note under ARRANGE while the mode is on (the mockup v2's).
pub const ARRANGE_NOTE: &str = "drag · ✎ ✕ + · tap to save";

/// The note under ARRANGE when the hub refused the save as `stale` (it
/// shows its own order again).
pub const REFUSED_NOTE: &str = "Not saved: the layouts changed";

/// The note under ARRANGE when the hub refused the save as `FULL`.
pub const FULL_NOTE: &str = "Not saved: a layout is full";

/// The note under ARRANGE when the hub refused the save as `invalid`.
pub const INVALID_NOTE: &str = "Not saved: the layouts are not valid";

/// The note under ARRANGE for any other failure of the save.
pub const NOT_SAVED_NOTE: &str = "Not saved";

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

    /// What the save's request heard: the hub's answer (`answered`); a
    /// failure, the hub's refusal or one the hub never gave
    /// (`never_reached`), ends a save at once with its word's note
    /// ([`refusal_note`]).
    pub fn heard<T>(&mut self, outcome: &Result<T, String>) -> Option<&'static str> {
        match outcome {
            Ok(_) => self.answered(true),
            Err(why) => {
                let _ = self.answered(false);
                Some(refusal_note(why))
            }
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

    /// Where the strip `held` of layout `from`, carried from its slot
    /// `at`, lands in slot `to` of layout `into` (the edit's layouts; none
    /// while the mode is not on, see [`landing`]). None too when a layout
    /// it changes would grow past FULL by the served layout's `pool`
    /// (#236, [`fits`]): the hub's save would refuse it, and a refused save
    /// ends the mode with every other change of the session.
    pub fn lands(
        &self,
        from: &str,
        at: usize,
        into: &str,
        to: usize,
        held: &StripRef,
        pool: &[Pooled],
    ) -> Option<Landing> {
        let edit = self.edit_now()?;
        let order = edit.slots_of(from)?;
        let target = if into == from {
            None
        } else {
            Some(edit.slots_of(into)?)
        };
        let landed = landing(order, target, at, to, held)?;
        // An `Across` landing always has its target.
        let fit = match &landed {
            Landing::Within(after) => fits(order, after, pool),
            Landing::Across {
                from: back,
                to: there,
            } => fits(order, back, pool) && fits(target.unwrap_or_default(), there, pool),
        };
        fit.then_some(landed)
    }

    /// The mark of the cell (slot `to` of layout `into`) a carried strip
    /// `held` of layout `from`, from slot `at`, is over: none over its own
    /// slot (it only goes back there), else whether it [`lands`] there.
    ///
    /// [`lands`]: Arrange::lands
    pub fn aim(
        &self,
        from: &str,
        at: usize,
        into: &str,
        to: usize,
        held: &StripRef,
        pool: &[Pooled],
    ) -> Option<bool> {
        if into == from && to == at {
            return None;
        }
        Some(self.lands(from, at, into, to, held, pool).is_some())
    }

    /// The strip `held` of layout `from`, carried from its slot `at`, is
    /// dropped in slot `to` of layout `into`: the layouts' new slots kept
    /// in the edit (only while the mode is on and it [`lands`]). Whether
    /// anything moved.
    ///
    /// [`lands`]: Arrange::lands
    pub fn dropped_in(
        &mut self,
        from: &str,
        at: usize,
        into: &str,
        to: usize,
        held: &StripRef,
        pool: &[Pooled],
    ) -> bool {
        let landed = self.lands(from, at, into, to, held, pool);
        let (Some(landed), Some(edit)) = (landed, self.edit()) else {
            return false;
        };
        match landed {
            Landing::Within(order) => edit.moved(from, order),
            Landing::Across {
                from: back,
                to: there,
            } => {
                edit.moved(from, back);
                edit.moved(into, there);
            }
        }
        true
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

/// The note for a save that failed with `why`: the hub's refusal word
/// (`stale`, `FULL`, `invalid`; any other, such as `EMPTY` or a write that
/// failed, plain "Not saved"), or the page's own not connected.
pub fn refusal_note(why: &str) -> &'static str {
    if never_reached(why) {
        OFFLINE_NOTE
    } else if why == Refusal::Stale.text() {
        REFUSED_NOTE
    } else if why == Refusal::Full.text() {
        FULL_NOTE
    } else if why == Refusal::Invalid.text() {
        INVALID_NOTE
    } else {
        NOT_SAVED_NOTE
    }
}

/// What a press of the ⇄ key did, for the recorder's `tab` event (#212):
/// `start` the mode, `save` the edit sent, `leave` with nothing to save,
/// `busy` a press while a save is on its way (`Arrange::tap`'s none).
pub fn tap_act(tap: Option<&Tap>) -> &'static str {
    match tap {
        Some(Tap::Start) => "start",
        Some(Tap::Send(..)) => "save",
        Some(Tap::Leave) => "leave",
        None => "busy",
    }
}

/// The note under ARRANGE: a refusal's while it shows, else the mode's.
pub fn arrange_note(on: bool, refused: Option<&'static str>) -> &'static str {
    refused.unwrap_or(if on { ARRANGE_NOTE } else { "" })
}

/// A cell on screen a carried strip can land in (#233): its sector, its
/// layout slot, its centre's x and its box's top and bottom (px, the
/// viewport's).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spot {
    pub sector: SectorId,
    pub slot: usize,
    pub x: f64,
    pub top: f64,
    pub bottom: f64,
}

impl Spot {
    /// How far `y` lies above or below the cell's box (0 inside it).
    pub fn off_line(&self, y: f64) -> f64 {
        (self.top - y).max(y - self.bottom).max(0.0)
    }
}

/// The cell a strip dropped at (`x`, `y`) lands in: among the cells whose
/// box is nearest `y` (the finger's line: 0 inside it), the one whose
/// centre is nearest `x`, the first on a tie; none without a cell.
pub fn drop_spot(spots: &[Spot], x: f64, y: f64) -> Option<Spot> {
    spots
        .iter()
        .min_by(|a, b| {
            a.off_line(y)
                .total_cmp(&b.off_line(y))
                .then((a.x - x).abs().total_cmp(&(b.x - x).abs()))
        })
        .copied()
}

/// Where a dropped strip goes: within its own layout (its new slots), or
/// into another layout (#233: the strip's layout's and the target's new
/// slots, the strip moved out of its own).
#[derive(Debug, Clone, PartialEq)]
pub enum Landing {
    Within(Vec<Slot>),
    Across { from: Vec<Slot>, to: Vec<Slot> },
}

/// Whether a layout's slots `before` may become `after` by the hub's FULL
/// rule (#236, `fohmixer_proto::sectors::grows_past_full`, judged on the
/// `pool`'s units: an empty slot 1, a wide strip 1.1): never grown past
/// [`SECTOR_UNITS`](fohmixer_proto::sectors::SECTOR_UNITS).
pub fn fits(before: &[Slot], after: &[Slot], pool: &[Pooled]) -> bool {
    !grows_past_full(units(before, pool), units(after, pool))
}

/// Whether `slots` hold `strip`.
fn holds(slots: &[Slot], strip: &StripRef) -> bool {
    slots.iter().any(|s| s.as_ref() == Some(strip))
}

/// The strip `held`, carried from slot `from` of its layout (`order`), is
/// dropped in slot `to`: of its own layout (`target` none: the two slots
/// swap, see [`swapped`]) or of another layout (`target`'s slots: an empty
/// slot takes it and its old slot is left empty, a strip there takes its
/// old slot, a swap). None when `from` no longer holds `held` (the order
/// changed meanwhile), when it lands where it was, when `target` already
/// holds `held`, or when the strip it would swap with is already in its own
/// layout (a layout never holds a strip twice; the hub refuses that too).
pub fn landing(
    order: &[Slot],
    target: Option<&[Slot]>,
    from: usize,
    to: usize,
    held: &StripRef,
) -> Option<Landing> {
    if order.get(from)?.as_ref() != Some(held) {
        return None;
    }
    let Some(target) = target else {
        return (to != from).then(|| Landing::Within(swapped(order, from, to)));
    };
    let comes = target.get(to).cloned().flatten();
    if holds(target, held) || comes.as_ref().is_some_and(|c| holds(order, c)) {
        return None;
    }
    let mut back = order.to_vec();
    back[from] = comes;
    let mut there = target.to_vec();
    there.resize(there.len().max(to + 1), None);
    there[to] = Some(held.clone());
    Some(Landing::Across {
        from: trim(back),
        to: trim(there),
    })
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

/// Whether a finger `x` px from where it went down carries the strip (one
/// axis; [`Drag`] checks both).
pub fn drags(start_x: f64, x: f64) -> bool {
    (x - start_x).abs() > DRAG_SLOP
}

/// A strip held under a pointer: the pointer and its type, the strip, its
/// sector and its slot, where the finger went down, every visible sector's
/// cells then, and whether it is carried yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Carry {
    pub pointer: i32,
    pub kind: Option<Pointer>,
    pub sector: SectorId,
    pub strip: StripRef,
    pub slot: usize,
    pub start_x: f64,
    pub start_y: f64,
    pub spots: Vec<Spot>,
    pub dragging: bool,
}

impl Carry {
    /// Whether the finger at (`x`, `y`) carries the strip: past the slop
    /// across or down.
    fn past_slop(&self, x: f64, y: f64) -> bool {
        drags(self.start_x, x) || drags(self.start_y, y)
    }
}

/// What a down on a strip's cover holds: its sector, its strip and its
/// slot, and every visible sector's cells (#233).
#[derive(Debug, Clone, PartialEq)]
pub struct Held {
    pub sector: SectorId,
    pub strip: StripRef,
    pub slot: usize,
    pub spots: Vec<Spot>,
}

/// The page's one drag at a time.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drag {
    held: Option<Carry>,
}

impl Drag {
    /// A pointer went down on a strip's cover at (`x`, `y`) (`held`: the
    /// strip, its sector, its slot and the cells). Taken unless another
    /// pointer holds a strip; a primary down of the held pointer's type (or
    /// that pointer again) ends a carry whose end never came (#124) and
    /// takes this one.
    pub fn down(&mut self, down: Down, held: Held, x: f64, y: f64) -> bool {
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
            start_y: y,
            spots: held.spots,
            dragging: false,
        });
        true
    }

    /// `pointer` moved to (`x`, `y`): the carried strip's offset from its
    /// place, once past the slop; none for another pointer or within it.
    pub fn moved(&mut self, pointer: i32, x: f64, y: f64) -> Option<(f64, f64)> {
        let held = self.held.as_mut().filter(|h| h.pointer == pointer)?;
        held.dragging = held.dragging || held.past_slop(x, y);
        held.dragging
            .then_some((x - held.start_x, y - held.start_y))
    }

    /// `pointer` lifted: the carry ends, and comes back when it moved the
    /// strip (to be dropped where the finger lifted); a tap moves nothing.
    pub fn up(&mut self, pointer: i32, x: f64, y: f64) -> Option<Carry> {
        let held = self.held.take_if(|h| h.pointer == pointer)?;
        (held.dragging || held.past_slop(x, y)).then_some(held)
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
mod tests;
