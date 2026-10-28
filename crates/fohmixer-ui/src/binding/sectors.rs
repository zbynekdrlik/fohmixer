//! The fader sectors on the device (#148; spec
//! `docs/superpowers/specs/2026-10-28-sectors-design.md` §4): which layout
//! each sector shows here (the picks, remembered in localStorage under
//! [`PICKS_KEY`]; a layout gone falls back to its sector's first), whether
//! the sectors replace the default page's rows, the controls they put on
//! screen (what the surface subscribes, `binding::wanted_subs`), a strip's
//! ticks in its channel detail and the detail's steps through the pool.
//! Everything here is pure: `pages::sectors` and the detail render it.

use std::collections::BTreeMap;

use fohmixer_proto::layout::{Control, Layout, Strip};
use fohmixer_proto::sectors::{
    Pooled, SectorId, SectorLayout, SectorsDoc, StripRef, pool, would_be_full,
};

use crate::flow::shared_instance;

/// Where the device keeps its picks (a JSON map: sector id → layout id).
pub const PICKS_KEY: &str = "fohmixer_sectors";

/// The layout each sector shows on this device, by id.
pub type Picks = BTreeMap<SectorId, String>;

/// The picks stored as `text` (none when it does not read).
pub fn parse_picks(text: &str) -> Picks {
    serde_json::from_str(text).unwrap_or_default()
}

/// The layout `sector` shows: the picked one while the document still has
/// it, else the sector's first (none for a sector without layouts). The
/// pick itself stays as stored until the user pages.
pub fn picked<'a>(
    doc: &'a SectorsDoc,
    picks: &Picks,
    sector: SectorId,
) -> Option<&'a SectorLayout> {
    let layouts = doc.layouts(sector);
    picks
        .get(&sector)
        .and_then(|id| layouts.iter().find(|layout| layout.id == *id))
        .or_else(|| layouts.first())
}

/// Whether the sectors replace the rows of the page shown (`path`'s
/// first): only the layout's default page, and only while the document
/// holds a layout.
pub fn sectors_on(layout: &Layout, path: &[usize], doc: Option<&SectorsDoc>) -> bool {
    doc.is_some_and(SectorsDoc::is_on)
        && path
            .first()
            .and_then(|index| layout.pages.get(*index))
            .is_some_and(|page| page.id == layout.default_page)
}

/// The pool's controls of the picked layouts, sector by sector in
/// [`SectorId::ALL`]'s order (a ref no pool control carries shows nowhere).
pub fn sector_controls<'a>(
    layout: &'a Layout,
    doc: &SectorsDoc,
    picks: &Picks,
) -> Vec<&'a Control> {
    let pool = pool(layout);
    SectorId::ALL
        .iter()
        .filter_map(|sector| picked(doc, picks, *sector))
        .flat_map(|shown| shown.strips.iter())
        .filter_map(|strip| pool.iter().find(|p| p.id == *strip).map(|p| p.control))
        .collect()
}

/// The Live set every strip of `sector`'s shown layout shares (#148: the
/// instance tag on its strips, `flow::shared_instance` over that layout's
/// pool controls); none when they differ or it holds no strip.
pub fn shown_instance(
    doc: &SectorsDoc,
    picks: &Picks,
    sector: SectorId,
    pool: &[Pooled],
) -> Option<String> {
    let shown = picked(doc, picks, sector)?;
    let controls: Vec<Control> = shown
        .strips
        .iter()
        .filter_map(|strip| pool.iter().find(|p| p.id == *strip))
        .map(|p| p.control.clone())
        .collect();
    shared_instance(&controls)
}

/// The sectors as the surface reads them: the document and this device's
/// picks.
#[derive(Debug, Clone, Copy)]
pub struct Sectored<'a> {
    pub doc: &'a SectorsDoc,
    pub picks: &'a Picks,
}

/// The controls on screen while the sectors replace the rows: the page's
/// rail, the picked layouts' controls, then the global controls.
pub fn sectored_controls<'a>(
    layout: &'a Layout,
    path: &[usize],
    sectors: Sectored<'_>,
) -> Vec<&'a Control> {
    let mut out: Vec<&Control> = path
        .first()
        .and_then(|index| layout.pages.get(*index))
        .map(|page| page.rail.iter().collect())
        .unwrap_or_default();
    out.extend(sector_controls(layout, sectors.doc, sectors.picks));
    out.extend(layout.global.iter());
    out
}

/// One layout's checkbox in a strip's channel detail: ticked (`on`) when
/// the layout holds the strip; `full` when it does not and the strip would
/// not fit (FULL, disabled).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickRow {
    pub layout: String,
    pub title: String,
    pub on: bool,
    pub full: bool,
}

/// A sector's checkboxes, under its name (`LU`, `LD`, `RU`, `RD`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TickGroup {
    pub sector: SectorId,
    pub name: &'static str,
    pub rows: Vec<TickRow>,
}

/// A sector's name in the detail.
pub fn sector_name(sector: SectorId) -> &'static str {
    match sector {
        SectorId::Lu => "LU",
        SectorId::Ld => "LD",
        SectorId::Ru => "RU",
        SectorId::Rd => "RD",
    }
}

/// Every layout of every sector with `strip`'s tick, in the sectors' order.
pub fn tick_rows(doc: &SectorsDoc, pool: &[Pooled], strip: &StripRef) -> Vec<TickGroup> {
    SectorId::ALL
        .iter()
        .map(|&sector| TickGroup {
            sector,
            name: sector_name(sector),
            rows: doc
                .layouts(sector)
                .iter()
                .map(|layout| {
                    let on = layout.strips.contains(strip);
                    TickRow {
                        layout: layout.id.clone(),
                        title: layout.title.clone(),
                        on,
                        full: !on && would_be_full(layout, strip, pool),
                    }
                })
                .collect(),
        })
        .collect()
}

/// The detail's ◀ ▶: the pool's strip before or after `now` (`forward`),
/// the parameter faders skipped; none at either end, and none when `now`
/// is no strip of the pool.
pub fn pool_step(pool: &[Pooled], now: &StripRef, forward: bool) -> Option<Strip> {
    let strips: Vec<&Pooled> = pool.iter().filter(|p| p.strip().is_some()).collect();
    let at = strips.iter().position(|p| p.id == *now)?;
    let next = if forward { at + 1 } else { at.checked_sub(1)? };
    strips.get(next).and_then(|p| p.strip()).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binding::wanted_subs;
    use serde_json::{Value, json};

    fn strip(name: &str) -> Value {
        json!({"kind": "strip",
               "binding": {"instance": "band", "anchor": {"kind": "track", "name": name}},
               "strip_kind": "standard"})
    }

    fn group(id: &str, controls: Value) -> Value {
        json!({"kind": "group", "id": id, "controls": controls})
    }

    /// The default page `foh`: a solo in its rail, a row of Vox 1, Vox 2,
    /// Podklady (a linked parameter fader) and Bass; the page `cue` holds
    /// Keys 1; a global solo.
    fn layout() -> Layout {
        let solo = |name: &str| {
            json!({"kind": "solo", "label": name,
                   "binding": {"instance": "band", "anchor": {"kind": "track", "name": name}}})
        };
        let pod = json!({"kind": "param_fader", "label": "Podklady",
                         "link": {"instance": "band", "id": "POD"}});
        serde_json::from_value(json!({
            "schema": 2,
            "default_page": "foh",
            "pages": [
                {"id": "foh", "title": "FOH", "rail": [solo("Vocals grp")],
                 "rows": [{"sections": [group("g", json!([strip("Vox 1"), strip("Vox 2"), pod, strip("Bass")]))]}]},
                {"id": "cue", "title": "CUE",
                 "rows": [{"sections": [group("c", json!([strip("Keys 1")]))]}]}
            ],
            "global": [solo("Alert grp")]
        }))
        .expect("the layout parses")
    }

    fn band(label: &str) -> StripRef {
        StripRef {
            instance: "band".to_string(),
            label: label.to_string(),
        }
    }

    fn sl(id: &str, title: &str, strips: &[&str]) -> SectorLayout {
        SectorLayout {
            id: id.to_string(),
            title: title.to_string(),
            strips: strips.iter().map(|s| band(s)).collect(),
        }
    }

    fn doc(sectors: Vec<(SectorId, Vec<SectorLayout>)>) -> SectorsDoc {
        SectorsDoc {
            rev: 3,
            sectors: sectors.into_iter().collect(),
        }
    }

    fn sample() -> SectorsDoc {
        doc(vec![
            (
                SectorId::Lu,
                vec![
                    sl("lu-1", "VOX", &["Vox 1"]),
                    sl("lu-2", "KEYS", &["Keys 1"]),
                ],
            ),
            (
                SectorId::Ru,
                vec![sl("ru-1", "MAIN", &["Bass", "Podklady"])],
            ),
        ])
    }

    #[test]
    fn picks_round_trip_and_bad_text_is_empty() {
        let picks: Picks = [
            (SectorId::Lu, "lu-2".to_string()),
            (SectorId::Rd, "rd-1".to_string()),
        ]
        .into_iter()
        .collect();
        let text = serde_json::to_string(&picks).expect("picks write");
        assert_eq!(text, r#"{"lu":"lu-2","rd":"rd-1"}"#);
        assert_eq!(parse_picks(&text), picks);
        assert_eq!(parse_picks("{"), Picks::new());
        assert_eq!(parse_picks(r#"{"lx":"a"}"#), Picks::new());
        assert_eq!(PICKS_KEY, "fohmixer_sectors");
    }

    #[test]
    fn a_pick_whose_layout_is_gone_falls_back_to_the_first() {
        let doc = sample();
        let mut picks = Picks::new();
        assert_eq!(
            picked(&doc, &picks, SectorId::Lu).map(|l| l.id.as_str()),
            Some("lu-1")
        );
        picks.insert(SectorId::Lu, "lu-2".to_string());
        assert_eq!(
            picked(&doc, &picks, SectorId::Lu).map(|l| l.id.as_str()),
            Some("lu-2")
        );
        picks.insert(SectorId::Lu, "x".to_string());
        assert_eq!(
            picked(&doc, &picks, SectorId::Lu).map(|l| l.id.as_str()),
            Some("lu-1")
        );
        // A pick of another sector's layout is no pick of this one.
        picks.insert(SectorId::Ru, "lu-2".to_string());
        assert_eq!(
            picked(&doc, &picks, SectorId::Ru).map(|l| l.id.as_str()),
            Some("ru-1")
        );
        assert_eq!(picked(&doc, &picks, SectorId::Ld), None);
    }

    #[test]
    fn sectors_are_on_only_on_the_default_page_with_a_layout() {
        let layout = layout();
        let shown = sample();
        assert!(sectors_on(&layout, &[0], Some(&shown)));
        assert!(!sectors_on(&layout, &[1], Some(&shown)));
        assert!(!sectors_on(&layout, &[], Some(&shown)));
        assert!(!sectors_on(&layout, &[0], None));
        assert!(!sectors_on(&layout, &[0], Some(&SectorsDoc::empty())));
        // A sector named with no layout is still off.
        assert!(!sectors_on(
            &layout,
            &[0],
            Some(&doc(vec![(SectorId::Lu, vec![])]))
        ));
    }

    fn targets(subs: &[crate::binding::SubSpec]) -> Vec<String> {
        subs.iter().map(|s| s.target.clone()).collect()
    }

    fn has(subs: &[crate::binding::SubSpec], name: &str) -> bool {
        let needle = format!("[name={name}]");
        targets(subs).iter().any(|t| t.contains(&needle))
    }

    #[test]
    fn the_wanted_set_is_the_rail_the_picked_layouts_and_global() {
        let layout = layout();
        let doc = sample();
        let mut picks = Picks::new();
        let on =
            |picks: &Picks| wanted_subs(&layout, &[0], None, Some(Sectored { doc: &doc, picks }));
        let first = on(&picks);
        assert!(has(&first, "Vox 1"));
        assert!(has(&first, "Bass"));
        assert!(has(&first, "Vocals grp"), "the rail");
        assert!(has(&first, "Alert grp"), "the global controls");
        assert!(!has(&first, "Vox 2"), "the rows' strips are not on screen");
        assert!(!has(&first, "Keys 1"), "lu-2 is not shown");
        picks.insert(SectorId::Lu, "lu-2".to_string());
        let second = on(&picks);
        assert!(has(&second, "Keys 1"));
        assert!(!has(&second, "Vox 1"));
        // The sector controls in the sectors' order, Podklady by its link.
        let controls = sector_controls(&layout, &doc, &picks);
        let refs: Vec<_> = controls
            .iter()
            .filter_map(|c| fohmixer_proto::sectors::strip_ref(c))
            .collect();
        assert_eq!(refs, vec![band("Keys 1"), band("Bass"), band("Podklady")]);
        // Another page, or an empty document: the page as without sectors.
        let cue = wanted_subs(
            &layout,
            &[1],
            None,
            Some(Sectored {
                doc: &doc,
                picks: &picks,
            }),
        );
        assert_eq!(cue, wanted_subs(&layout, &[1], None, None));
        let empty = SectorsDoc::empty();
        let off = wanted_subs(
            &layout,
            &[0],
            None,
            Some(Sectored {
                doc: &empty,
                picks: &picks,
            }),
        );
        assert_eq!(off, wanted_subs(&layout, &[0], None, None));
        assert!(has(&off, "Vox 2"));
    }

    #[test]
    fn tick_rows_list_every_layout_with_on_and_full() {
        // The pool: Vox 1, Vox 2, Podklady, Bass, then the cue page's
        // Keys 1, Drums, Keys 2 (seven, one unit each).
        let mut layout = layout();
        layout.pages[1].rows[0].sections = serde_json::from_value(json!([group(
            "c",
            json!([strip("Keys 1"), strip("Drums"), strip("Keys 2")])
        )]))
        .expect("sections");
        let pool = pool(&layout);
        // lu-1 holds 6 units; lu-2 5 (a ref no pool control carries takes
        // no room), so one more fits exactly.
        let doc = doc(vec![
            (
                SectorId::Lu,
                vec![
                    sl(
                        "lu-1",
                        "SIX",
                        &["Vox 1", "Vox 2", "Bass", "Podklady", "Keys 1", "Drums"],
                    ),
                    sl(
                        "lu-2",
                        "FIVE",
                        &["Vox 1", "Bass", "Podklady", "Keys 1", "Gone 1", "Drums"],
                    ),
                ],
            ),
            (SectorId::Rd, vec![sl("rd-1", "MAIN", &[])]),
        ]);
        let row = |layout: &str, title: &str, on: bool, full: bool| TickRow {
            layout: layout.to_string(),
            title: title.to_string(),
            on,
            full,
        };
        let rows = tick_rows(&doc, &pool, &band("Keys 2"));
        let names: Vec<(SectorId, &str)> = rows.iter().map(|g| (g.sector, g.name)).collect();
        assert_eq!(
            names,
            vec![
                (SectorId::Lu, "LU"),
                (SectorId::Ld, "LD"),
                (SectorId::Ru, "RU"),
                (SectorId::Rd, "RD"),
            ]
        );
        assert_eq!(
            rows[0].rows,
            vec![
                row("lu-1", "SIX", false, true),
                row("lu-2", "FIVE", false, false)
            ]
        );
        assert!(rows[1].rows.is_empty());
        assert!(rows[2].rows.is_empty());
        assert_eq!(rows[3].rows, vec![row("rd-1", "MAIN", false, false)]);
        // A full layout that holds the strip: on, never FULL.
        let held = tick_rows(&doc, &pool, &band("Vox 2"));
        assert_eq!(
            held[0].rows,
            vec![
                row("lu-1", "SIX", true, false),
                row("lu-2", "FIVE", false, false)
            ]
        );
    }

    #[test]
    fn the_pool_step_goes_to_the_next_strip_and_stops_at_the_ends() {
        let layout = layout();
        let pool = pool(&layout);
        let label = |s: Option<Strip>| s.map(|s| s.binding.anchor.clone());
        let named = |name: &str| {
            Some(fohmixer_proto::layout::Anchor::Track {
                name: name.to_string(),
            })
        };
        assert_eq!(
            label(pool_step(&pool, &band("Vox 1"), true)),
            named("Vox 2")
        );
        assert_eq!(
            label(pool_step(&pool, &band("Vox 2"), false)),
            named("Vox 1")
        );
        // Podklady (a parameter fader) is skipped both ways.
        assert_eq!(label(pool_step(&pool, &band("Vox 2"), true)), named("Bass"));
        assert_eq!(
            label(pool_step(&pool, &band("Bass"), false)),
            named("Vox 2")
        );
        assert_eq!(
            label(pool_step(&pool, &band("Bass"), true)),
            named("Keys 1")
        );
        // The ends.
        assert_eq!(pool_step(&pool, &band("Vox 1"), false), None);
        assert_eq!(pool_step(&pool, &band("Keys 1"), true), None);
        // No strip of the pool: nowhere to step from.
        assert_eq!(pool_step(&pool, &band("Podklady"), true), None);
        assert_eq!(pool_step(&pool, &band("Gone"), true), None);
    }

    #[test]
    fn a_sectors_instance_tag_is_the_set_its_shown_strips_share() {
        let mut layout = layout();
        layout.pages[1].rows[0].sections = serde_json::from_value(json!([group(
            "c",
            json!([{"kind": "strip", "strip_kind": "standard",
                    "binding": {"instance": "master", "anchor": {"kind": "track", "name": "Hand1"}}}])
        )]))
        .expect("sections");
        let pool = pool(&layout);
        let hand = StripRef {
            instance: "master".to_string(),
            label: "Hand1".to_string(),
        };
        let mut mixed = sl("ld-2", "MIXED", &["Vox 1"]);
        mixed.strips.push(hand);
        let doc = doc(vec![(
            SectorId::Ld,
            vec![sl("ld-1", "VOX", &["Vox 1", "Bass"]), mixed],
        )]);
        let mut picks = Picks::new();
        assert_eq!(
            shown_instance(&doc, &picks, SectorId::Ld, &pool).as_deref(),
            Some("band")
        );
        picks.insert(SectorId::Ld, "ld-2".to_string());
        assert_eq!(shown_instance(&doc, &picks, SectorId::Ld, &pool), None);
        assert_eq!(shown_instance(&doc, &picks, SectorId::Lu, &pool), None);
    }

    #[test]
    fn the_sector_names_read_as_the_detail_shows_them() {
        let names: Vec<&str> = SectorId::ALL.iter().map(|s| sector_name(*s)).collect();
        assert_eq!(names, vec!["LU", "LD", "RU", "RD"]);
    }
}
