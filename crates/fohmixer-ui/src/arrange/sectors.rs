//! The sectors page's arrangement (#148; spec
//! `docs/superpowers/specs/2026-10-28-sectors-design.md` §4): two lines,
//! up and down, each cut by the column at a fixed point, the left sector's
//! cells on its left and the right sector's on its right (a low screen: one
//! line, each side up then down). `split`, pins, blanks and the pager's
//! region do not apply.
//!
//! One strip width serves the whole page and depends on the document and
//! the screen only, never on the picks, so paging a sector resizes nothing:
//! each side's need is its sectors' widest layouts side by side, and the
//! width is the widest at which every side's need fits it, 64–120 px. A
//! side whose need does not fit at the narrowest strip (a phone) shows a
//! window over its shown cells, keyed by its sectors, which the column's
//! arrows move. This module is pure; `pages::sectors` measures and renders.
//!
//! A layout is a row of slots (1.24.0, the slot model): an empty slot is a
//! blank cell one strip wide in its place, counted in the width like a
//! strip. While ARRANGE is on, every further position a one-sector side can
//! show at the page's strip width, up to [`SECTOR_UNITS`], is an empty cell
//! too (`arranging`), so a strip can be dropped there; the width never
//! depends on them.

use fohmixer_proto::sectors::{Pooled, SECTOR_UNITS, SectorId, SectorLayout, SectorsDoc, StripRef};

use super::{Metrics, Shift, Sides, lines, stretch};
use crate::binding::sectors::{Picks, picked};

#[cfg(test)]
mod tests;

/// One cell of a sector on the page: its sector, its slot in the layout,
/// its fader (none: an empty slot) and its width in strips (a wide strip
/// `Metrics::wide`, an empty slot 1).
#[derive(Debug, Clone, PartialEq)]
pub struct SectorCell {
    pub sector: SectorId,
    pub slot: usize,
    pub strip: Option<StripRef>,
    pub units: f64,
}

impl SectorCell {
    /// Its key in the line's keyed list: its sector and its fader, so a
    /// strip that stays when its sector pages keeps its element, and the
    /// same fader in two sectors is two elements; an empty slot by its
    /// sector and its place (`lu:#3`).
    pub fn key(&self) -> String {
        match &self.strip {
            Some(strip) => format!(
                "{}:{}|{}",
                self.sector.as_str(),
                strip.instance,
                strip.label
            ),
            None => format!("{}:#{}", self.sector.as_str(), self.slot),
        }
    }

    /// An empty cell of `sector` at `slot`.
    pub fn empty(sector: SectorId, slot: usize) -> Self {
        Self {
            sector,
            slot,
            strip: None,
            units: 1.0,
        }
    }
}

/// A layout's cells in its slots' order, an empty slot an empty cell; a ref
/// no pool control carries is left out (it shows nowhere and takes no
/// room).
pub fn layout_cells(
    sector: SectorId,
    layout: &SectorLayout,
    pool: &[Pooled],
    m: &Metrics,
) -> Vec<SectorCell> {
    layout
        .strips
        .iter()
        .enumerate()
        .filter_map(|(slot, strip)| match strip {
            None => Some(SectorCell::empty(sector, slot)),
            Some(strip) => pool.iter().find(|p| p.id == *strip).map(|p| SectorCell {
                sector,
                slot,
                strip: Some(p.id.clone()),
                units: if p.strip().is_some_and(|s| s.wide) {
                    m.wide
                } else {
                    1.0
                },
            }),
        })
        .collect()
}

/// How far (px) a side's cells may pass its room and still fit at a strip
/// width (the width is a quotient: six strips at it can sum a hair over).
pub const FIT_SLACK: f64 = 0.5;

/// Whether `cells` fit `side` px at strips `width` wide ([`FIT_SLACK`]).
fn fits_at(cells: &[SectorCell], width: f64, side: f64, m: &Metrics) -> bool {
    span(cells, width, m) <= side + FIT_SLACK
}

/// ARRANGE's cells of a one-sector side: its `cells`, then an empty cell
/// for every further position that fits `side` px at `width` while the
/// sector stays within [`SECTOR_UNITS`] (each empty one a unit), numbered
/// after its layout's last slot (`slots`).
pub fn with_spaces(
    mut cells: Vec<SectorCell>,
    sector: SectorId,
    slots: usize,
    width: f64,
    side: f64,
    m: &Metrics,
) -> Vec<SectorCell> {
    for slot in slots..slots + SECTOR_UNITS as usize {
        let units: f64 = cells.iter().map(|c| c.units).sum();
        let mut next = cells.clone();
        next.push(SectorCell::empty(sector, slot));
        if units + 1.0 > SECTOR_UNITS || !fits_at(&next, width, side, m) {
            break;
        }
        cells = next;
    }
    cells
}

/// The px `cells` take side by side with strips `width` wide.
fn span(cells: &[SectorCell], width: f64, m: &Metrics) -> f64 {
    let units: f64 = cells.iter().map(|c| c.units).sum();
    units * width + cells.len().saturating_sub(1) as f64 * m.gap
}

/// The widest strip at which `cells` fit `side` px (none while they hold
/// nothing).
fn fit_width(cells: &[SectorCell], side: f64, m: &Metrics) -> Option<f64> {
    let units: f64 = cells.iter().map(|c| c.units).sum();
    (!cells.is_empty()).then(|| (side - (cells.len() - 1) as f64 * m.gap) / units)
}

/// Whether `cells` fit `side` px at the narrowest strip.
fn fits(cells: &[SectorCell], side: f64, m: &Metrics) -> bool {
    fit_width(cells, side, m).is_none_or(|width| width >= m.min)
}

/// A sector's widest layout's cells, at the narrowest strip (ties: the
/// first); none for a sector without layouts.
fn widest(doc: &SectorsDoc, pool: &[Pooled], sector: SectorId, m: &Metrics) -> Vec<SectorCell> {
    doc.layouts(sector)
        .iter()
        .rev()
        .map(|layout| layout_cells(sector, layout, pool, m))
        .max_by(|a, b| span(a, m.min, m).total_cmp(&span(b, m.min, m)))
        .unwrap_or_default()
}

/// What a side's window is kept for: its sectors (a phone keeps one window
/// per side whatever its sectors show).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowKey {
    pub sectors: Vec<SectorId>,
}

/// One side of a line: its window's key, the cells it shows and its window
/// (none while its need fits).
#[derive(Debug, Clone, PartialEq)]
pub struct SectorSide {
    pub key: WindowKey,
    pub cells: Vec<SectorCell>,
    pub shift: Option<Shift>,
}

/// One line: its left and right sides.
#[derive(Debug, Clone, PartialEq)]
pub struct SectorLine {
    pub left: SectorSide,
    pub right: SectorSide,
}

/// The sectors page as arranged: the strip width and the lines.
#[derive(Debug, Clone, PartialEq)]
pub struct SectorArrangement {
    pub width: f64,
    pub lines: Vec<SectorLine>,
}

impl SectorArrangement {
    /// The window of the side kept by `key` (none: no such side, or it
    /// fits): what its arrows move.
    pub fn shift_of(&self, key: &WindowKey) -> Option<Shift> {
        self.lines
            .iter()
            .flat_map(|line| [&line.left, &line.right])
            .find(|side| side.key == *key)
            .and_then(|side| side.shift)
    }

    /// The cell of line `line` kept by `key` (`SectorCell::key`): a kept
    /// strip's element reads its slot now through it.
    pub fn cell_of(&self, line: usize, key: &str) -> Option<&SectorCell> {
        let line = self.lines.get(line)?;
        line.left
            .cells
            .iter()
            .chain(line.right.cells.iter())
            .find(|cell| cell.key() == key)
    }

    /// The keys of the sides with a window, line by line, left first: the
    /// column's arrows.
    pub fn windows(&self) -> Vec<WindowKey> {
        self.lines
            .iter()
            .flat_map(|line| [&line.left, &line.right])
            .filter(|side| side.shift.is_some())
            .map(|side| side.key.clone())
            .collect()
    }
}

/// The page's lines in `height` px, each its left and right sectors: two
/// lines (`lu` | `ru`, `ld` | `rd`) while each keeps `m.line_min` of the
/// height (`arrange::lines` of two equal rows); otherwise one, each side up
/// then down.
pub fn sector_lines(height: f64, m: &Metrics) -> Vec<(Vec<SectorId>, Vec<SectorId>)> {
    lines(&[1.0, 1.0], height, m)
        .into_iter()
        .map(|rows| {
            let side = |left: bool| -> Vec<SectorId> {
                rows.iter()
                    .map(|row| SectorId::of(*row == 0, left))
                    .collect()
            };
            (side(true), side(false))
        })
        .collect()
}

/// A window over `cells` from `offset` on a side `side` px wide: as many as
/// fit at the narrowest strip; the last window is as many of the last cells
/// as fit, so the last cell can always be reached, and `offset` is clamped
/// to its start (`arrange::shown`'s window, with no pins).
fn window(cells: &[SectorCell], side: f64, offset: usize, m: &Metrics) -> (Vec<SectorCell>, Shift) {
    let total = cells.len();
    let tail = (1..=total)
        .rev()
        .find(|&t| fits(&cells[total - t..], side, m))
        .unwrap_or(0);
    let last = total - tail;
    let offset = offset.min(last);
    let free = (1..=total - offset)
        .rev()
        .find(|&f| fits(&cells[offset..offset + f], side, m))
        .unwrap_or(0);
    let shift = Shift {
        offset,
        free,
        step: free.max(1),
        last,
        total,
    };
    (cells[offset..offset + free].to_vec(), shift)
}

/// One side of a line and the strip width it allows (none: it holds
/// nothing). Its need (each sector's widest layout) decides whether it
/// fits; it shows its picked layouts' cells, in its sectors' order.
fn arrange_side(
    doc: &SectorsDoc,
    pool: &[Pooled],
    picks: &Picks,
    sectors: Vec<SectorId>,
    sides: Sides,
    offset: &dyn Fn(&WindowKey) -> usize,
    m: &Metrics,
) -> (SectorSide, Option<f64>) {
    let need: Vec<SectorCell> = sectors
        .iter()
        .flat_map(|sector| widest(doc, pool, *sector, m))
        .collect();
    let cells: Vec<SectorCell> = sectors
        .iter()
        .flat_map(|sector| {
            picked(doc, picks, *sector)
                .map(|layout| layout_cells(*sector, layout, pool, m))
                .unwrap_or_default()
        })
        .collect();
    let key = WindowKey { sectors };
    if fits(&need, sides.count, m) {
        let width = fit_width(&need, sides.width, m);
        return (
            SectorSide {
                key,
                cells,
                shift: None,
            },
            width,
        );
    }
    let (shown, shift) = window(&cells, sides.count, offset(&key), m);
    (
        SectorSide {
            key,
            cells: shown,
            shift: Some(shift),
        },
        Some(m.min * stretch(sides)),
    )
}

/// The sectors page arranged on `sides` and `height`: its lines, each
/// side's cells and window (at `offset(its key)`), and the one strip width,
/// the narrowest any side allows, within `m.min`..`m.max` (a side with a
/// window allows the narrowest strip, scaled to the side it fills). While
/// `arranging` (ARRANGE on), a one-sector side without a window shows its
/// further positions as empty cells ([`with_spaces`]).
#[allow(clippy::too_many_arguments)]
pub fn arrange_sectors(
    doc: &SectorsDoc,
    pool: &[Pooled],
    picks: &Picks,
    sides: Sides,
    height: f64,
    offset: impl Fn(&WindowKey) -> usize,
    arranging: bool,
    m: &Metrics,
) -> SectorArrangement {
    let mut width = f64::INFINITY;
    let mut out = Vec::new();
    for (left, right) in sector_lines(height, m) {
        let (left, left_width) = arrange_side(doc, pool, picks, left, sides, &offset, m);
        let (right, right_width) = arrange_side(doc, pool, picks, right, sides, &offset, m);
        width = [left_width, right_width]
            .into_iter()
            .flatten()
            .fold(width, f64::min);
        out.push(SectorLine { left, right });
    }
    let width = width.clamp(m.min, m.max);
    if arranging {
        for side in out.iter_mut().flat_map(|l| [&mut l.left, &mut l.right]) {
            add_spaces(side, doc, picks, width, sides.width, m);
        }
    }
    SectorArrangement { width, lines: out }
}

/// ARRANGE's further positions of `side` ([`with_spaces`]): only a side of
/// one sector with a layout that shows no window (a phone's sides keep
/// their cells).
fn add_spaces(
    side: &mut SectorSide,
    doc: &SectorsDoc,
    picks: &Picks,
    width: f64,
    room: f64,
    m: &Metrics,
) {
    let [sector] = side.key.sectors.as_slice() else {
        return;
    };
    let Some(layout) = picked(doc, picks, *sector) else {
        return;
    };
    if side.shift.is_some() {
        return;
    }
    let slots = layout.strips.len();
    side.cells = with_spaces(
        std::mem::take(&mut side.cells),
        *sector,
        slots,
        width,
        room,
        m,
    );
}
