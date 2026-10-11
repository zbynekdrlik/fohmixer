//! Pro-Q 4's own undo (#71 PR P): where its ↶ lies in the editor's picture
//! and whether it reads lit (something to undo), grey (nothing to undo) or
//! missing (not where it was measured). Measured in Carla's bridge on the
//! PC (0.1.0-dev.59, `eq-probe --size` at 13 widths; nothing in Live): the
//! ↶ is 12 × 7 px in Pro-Q's top bar, rows 15 to 21 at every size, its
//! centre x following the picture's width ([`TABLE`]). From 1100 px up the
//! top bar's group is centred (x = w/2 − 104.5), at 860 to 900 px it is
//! w/2 − 106.5, and under about 850 px the group stops at the left edge.
//!
//! Lit, its brightest pixel reads about 225 to 250 of 255 (a 12 px run of
//! columns at or above [`LIT`], sometimes split by a thinner column); grey,
//! about 99 (a 10 to 12 px run at or above [`GREY`]); the bar around it
//! stays under about 55. Its neighbours: the guard's inert spot 23 px to
//! the left (at 1349 px), the redo ↷ 29 px to the right, and A/B about
//! 70 px to the right (a tap there swaps the whole EQ), so a tap goes only
//! where a read found the ↶ ([`Undo::Lit`] carries the run's own centre),
//! never blind at the table's spot.

use std::ops::RangeInclusive;

use super::Pixels;

/// The ↶'s centre x (px) by the picture's width (px), as measured; linear
/// between the points ([`spot`]).
pub const TABLE: [(u32, f64); 14] = [
    (760, 311.5),
    (780, 313.5),
    (800, 314.5),
    (820, 316.5),
    (840, 317.5),
    (860, 323.5),
    (880, 333.5),
    (900, 343.5),
    (1100, 445.5),
    (1291, 540.5),
    (1349, 569.5),
    (1535, 662.5),
    (1810, 800.5),
    (2400, 1095.5),
];
/// The ↶'s row centre (px from the picture's top).
pub const Y: i32 = 18;
/// The rows the read looks at (inclusive).
pub const ROWS: (i32, i32) = (12, 24);
/// The columns the read looks at on each side of the spot.
pub const REACH: i32 = 16;
/// How far a run's centre may lie from the spot (px).
pub const SLOP: i32 = 4;
/// The brightness (luma, 0 to 255) of a lit ↶'s columns, and of a grey
/// one's.
pub const LIT: u8 = 140;
pub const GREY: u8 = 70;
/// A run's width (px, inclusive): the ↶'s own 12 px with a margin.
pub const WIDTHS: (i32, i32) = (8, 16);
/// The columns below the threshold a run bridges (a lit ↶'s thinner
/// column).
pub const GAP: i32 = 1;

/// What a read of the ↶ found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Undo {
    /// Lit, something to undo: a tap goes to the run's centre (x, [`Y`]).
    Lit(i32, i32),
    /// Grey: nothing to undo.
    Grey,
    /// Not where it was measured (a width the table does not know, a
    /// Pro-Q update, another layout): never tapped.
    Missing,
}

impl Undo {
    /// Its name in the hub log and the `eq` records (#71 PR P2a): `lit`,
    /// `grey` or `missing`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Lit(..) => "lit",
            Self::Grey => "grey",
            Self::Missing => "missing",
        }
    }

    /// The column the read found a lit ↶ centred on (none: not lit).
    pub fn x(self) -> Option<i32> {
        match self {
            Self::Lit(x, _) => Some(x),
            Self::Grey | Self::Missing => None,
        }
    }
}

/// The ↶'s spot (x, y) in a picture `width` px wide: linear between the
/// [`TABLE`]'s points (exact at each), x rounded; none outside the table
/// (760 to 2400 px).
pub fn spot(width: u32) -> Option<(i32, i32)> {
    let w = f64::from(width);
    TABLE
        .iter()
        .zip(&TABLE[1..])
        .find_map(|(&(w0, x0), &(w1, x1))| {
            let (w0, w1) = (f64::from(w0), f64::from(w1));
            (w0..=w1)
                .contains(&w)
                .then_some(x0 + (x1 - x0) * (w - w0) / (w1 - w0))
        })
        .map(|x| (x.round() as i32, Y))
}

/// The brightness (0 to 255) of the BGRA pixel `bgra` starts with (its
/// blue, green and red bytes): `(299 R + 587 G + 114 B) / 1000`.
pub fn luma(bgra: &[u8]) -> u8 {
    let [b, g, r] = [bgra[0], bgra[1], bgra[2]].map(u32::from);
    ((299 * r + 587 * g + 114 * b) / 1000) as u8
}

/// The columns the read looks at around `x` in a picture `width` px wide:
/// [`REACH`] on each side, within the picture.
fn reach(width: u32, x: i32) -> RangeInclusive<i32> {
    (x - REACH).max(0)..=(x + REACH).min(width as i32 - 1)
}

/// The rows the read looks at in a picture `height` px high: [`ROWS`],
/// within the picture.
fn rows(height: u32) -> RangeInclusive<i32> {
    ROWS.0..=ROWS.1.min(height as i32 - 1)
}

/// Each column around `x` ([`reach`]) with its brightest pixel over the
/// rows ([`rows`]), left to right.
fn columns(pixels: &Pixels, x: i32) -> Vec<(i32, u8)> {
    let stride = pixels.width as usize;
    reach(pixels.width, x)
        .map(|column| {
            let brightest = rows(pixels.height)
                .map(|row| luma(&pixels.bgra[(row as usize * stride + column as usize) * 4..]))
                .max()
                .unwrap_or(0);
            (column, brightest)
        })
        .collect()
}

/// The runs of `columns` (consecutive, left to right) at or above
/// `threshold`, each its first and last column: a gap of at most [`GAP`]
/// columns below it is bridged.
fn runs(columns: &[(i32, u8)], threshold: u8) -> Vec<(i32, i32)> {
    let mut runs: Vec<(i32, i32)> = Vec::new();
    for &(column, _) in columns.iter().filter(|(_, luma)| *luma >= threshold) {
        match runs.last_mut() {
            Some((_, last)) if column - *last - 1 <= GAP => *last = column,
            _ => runs.push((column, column)),
        }
    }
    runs
}

/// The centre of the first run at or above `threshold` that is as wide as
/// the ↶ ([`WIDTHS`]) and whose centre (`(first + last) / 2`, rounded)
/// lies within [`SLOP`] of `x`.
fn found(columns: &[(i32, u8)], threshold: u8, x: i32) -> Option<i32> {
    runs(columns, threshold)
        .into_iter()
        .find_map(|(first, last)| {
            let centre = (f64::from(first + last) / 2.0).round() as i32;
            let wide = (WIDTHS.0..=WIDTHS.1).contains(&(last - first + 1));
            (wide && (centre - x).abs() <= SLOP).then_some(centre)
        })
}

/// The ↶ in `pixels` (an editor's picture): at its [`spot`] for the
/// picture's width (none: missing), each column within [`REACH`] taken at
/// its brightest over [`ROWS`]; a run of columns at or above [`LIT`] as
/// wide as the ↶ and centred on the spot within [`SLOP`] is lit (its own
/// centre), else such a run at or above [`GREY`] is grey, else it is
/// missing.
pub fn read(pixels: &Pixels) -> Undo {
    let Some((x, _)) = spot(pixels.width) else {
        return Undo::Missing;
    };
    let seen = columns(pixels, x);
    match (found(&seen, LIT, x), found(&seen, GREY, x)) {
        (Some(centre), _) => Undo::Lit(centre, Y),
        (None, Some(_)) => Undo::Grey,
        (None, None) => Undo::Missing,
    }
}

#[cfg(test)]
mod tests;
