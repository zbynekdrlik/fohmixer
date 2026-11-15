//! The channel detail's LAYOUTS (#208, the owner's approved mockup
//! `docs/mockups/scenes-v3.html`, frame 1): the block lists only the
//! layouts that hold the strip, one row each with a ✕ that takes it out,
//! and ADD TO LAYOUT opens a sheet of the other layouts, a FULL one
//! disabled. Pure: what the rows and the picks are, from the strip's
//! [`DetailTicks`]; `components::detail_layouts` draws them.

use fohmixer_proto::sectors::SectorId;

use super::sectors::{DetailTicks, sector_name};

/// A layout that holds the strip: one row of the block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutRow {
    pub layout: String,
    pub title: String,
    /// The sector's tag (`LU`, `LD`, `RU`, `RD`).
    pub sector: &'static str,
}

/// A layout that does not hold the strip: one pick of the sheet; `full`
/// when the strip would not fit (FULL, disabled).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutPick {
    pub layout: String,
    pub title: String,
    pub sector: &'static str,
    pub full: bool,
}

impl LayoutPick {
    /// Whether a tap on this pick asks the hub to put the strip in: never
    /// for a FULL one.
    pub fn asks(&self) -> bool {
        !self.full
    }

    /// The pick's second line: its sector, and `· FULL` when full.
    pub fn sub(&self) -> String {
        if self.full {
            format!("{} · FULL", self.sector)
        } else {
            self.sector.to_string()
        }
    }
}

/// Every layout of `ticks`, sector by sector in the sectors' order, with
/// its tag and whether it holds the strip and is full.
fn every(ticks: &DetailTicks) -> Vec<(&'static str, &super::sectors::TickRow)> {
    let DetailTicks::Rows { groups, .. } = ticks else {
        return Vec::new();
    };
    SectorId::ALL
        .iter()
        .flat_map(|sector| {
            groups
                .iter()
                .filter(move |g| g.sector == *sector)
                .flat_map(|g| g.rows.iter().map(move |row| (sector_name(g.sector), row)))
        })
        .collect()
}

/// The block's rows: the layouts that hold the strip, in the sectors'
/// order (none outside `Rows`).
pub fn held(ticks: &DetailTicks) -> Vec<LayoutRow> {
    every(ticks)
        .into_iter()
        .filter(|(_, row)| row.on)
        .map(|(sector, row)| LayoutRow {
            layout: row.layout.clone(),
            title: row.title.clone(),
            sector,
        })
        .collect()
}

/// The sheet's picks: the layouts that do not hold the strip, in the
/// sectors' order, a full one marked (none outside `Rows`).
pub fn picks(ticks: &DetailTicks) -> Vec<LayoutPick> {
    every(ticks)
        .into_iter()
        .filter(|(_, row)| !row.on)
        .map(|(sector, row)| LayoutPick {
            layout: row.layout.clone(),
            title: row.title.clone(),
            sector,
            full: row.full,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binding::sectors::{TickGroup, TickRow};
    use fohmixer_proto::sectors::StripRef;

    fn row(layout: &str, title: &str, on: bool, full: bool) -> TickRow {
        TickRow {
            layout: layout.to_string(),
            title: title.to_string(),
            on,
            full,
        }
    }

    fn group(sector: SectorId, rows: Vec<TickRow>) -> TickGroup {
        TickGroup {
            sector,
            name: sector_name(sector),
            rows,
        }
    }

    /// The groups out of the sectors' order, so the order is the
    /// functions' own.
    fn ticks() -> DetailTicks {
        DetailTicks::Rows {
            id: StripRef {
                instance: "band".to_string(),
                label: "Vox 1".to_string(),
            },
            groups: vec![
                group(
                    SectorId::Rd,
                    vec![
                        row("rd-1", "PA", true, false),
                        row("rd-2", "MBC", false, true),
                    ],
                ),
                group(
                    SectorId::Lu,
                    vec![
                        row("lu-1", "BAND", true, false),
                        row("lu-2", "OTHERS", false, false),
                    ],
                ),
                group(SectorId::Ld, vec![]),
                group(SectorId::Ru, vec![row("ru-1", "MAIN", false, false)]),
            ],
        }
    }

    fn held_row(layout: &str, title: &str, sector: &'static str) -> LayoutRow {
        LayoutRow {
            layout: layout.to_string(),
            title: title.to_string(),
            sector,
        }
    }

    fn pick(layout: &str, title: &str, sector: &'static str, full: bool) -> LayoutPick {
        LayoutPick {
            layout: layout.to_string(),
            title: title.to_string(),
            sector,
            full,
        }
    }

    #[test]
    fn the_rows_are_only_the_layouts_that_hold_the_strip_in_sector_order() {
        assert_eq!(
            held(&ticks()),
            vec![held_row("lu-1", "BAND", "LU"), held_row("rd-1", "PA", "RD")]
        );
    }

    #[test]
    fn the_picks_are_the_other_layouts_a_full_one_marked() {
        assert_eq!(
            picks(&ticks()),
            vec![
                pick("lu-2", "OTHERS", "LU", false),
                pick("ru-1", "MAIN", "RU", false),
                pick("rd-2", "MBC", "RD", true),
            ]
        );
    }

    #[test]
    fn a_full_pick_asks_nothing_and_says_full() {
        let free = pick("lu-2", "OTHERS", "LU", false);
        let full = pick("rd-2", "MBC", "RD", true);
        assert!(free.asks());
        assert!(!full.asks());
        assert_eq!(free.sub(), "LU");
        assert_eq!(full.sub(), "RD · FULL");
    }

    #[test]
    fn a_strip_in_no_layout_has_no_rows_and_every_layout_to_pick() {
        let DetailTicks::Rows { id, groups } = ticks() else {
            panic!("rows");
        };
        let groups = groups
            .into_iter()
            .map(|g| TickGroup {
                rows: g
                    .rows
                    .into_iter()
                    .map(|r| TickRow { on: false, ..r })
                    .collect(),
                ..g
            })
            .collect();
        let none = DetailTicks::Rows { id, groups };
        assert_eq!(held(&none), Vec::new());
        assert_eq!(picks(&none).len(), 5);
        // A strip in every layout: nothing to pick (no ADD).
        let all = DetailTicks::Rows {
            id: StripRef {
                instance: "band".to_string(),
                label: "Vox 1".to_string(),
            },
            groups: vec![group(SectorId::Lu, vec![row("lu-1", "BAND", true, false)])],
        };
        assert_eq!(picks(&all), Vec::new());
        assert_eq!(held(&all), vec![held_row("lu-1", "BAND", "LU")]);
    }

    #[test]
    fn no_section_or_no_identity_has_neither() {
        assert_eq!(held(&DetailTicks::Off), Vec::new());
        assert_eq!(picks(&DetailTicks::Off), Vec::new());
        assert_eq!(held(&DetailTicks::NoRef), Vec::new());
        assert_eq!(picks(&DetailTicks::NoRef), Vec::new());
    }
}
