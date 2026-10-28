use super::*;
use crate::arrange::METRICS;
use fohmixer_proto::layout::Layout;
use fohmixer_proto::sectors::pool;
use serde_json::{Value, json};

fn strip(name: &str) -> Value {
    json!({"kind": "strip", "binding": {"instance": "band", "anchor": {"kind": "track", "name": name}},
           "strip_kind": "standard"})
}

/// One page of band strips `Vox 1`..`Vox 8`, then `PA Out` (wide).
fn layout() -> Layout {
    let mut controls: Vec<Value> = (1..=8).map(|n| strip(&format!("Vox {n}"))).collect();
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
}

fn band(label: &str) -> StripRef {
    StripRef {
        instance: "band".to_string(),
        label: label.to_string(),
    }
}

fn vox(numbers: &[u32]) -> Vec<String> {
    numbers.iter().map(|n| format!("Vox {n}")).collect()
}

/// A layout of these slots, `""` an empty slot.
fn sl(id: &str, strips: &[String]) -> SectorLayout {
    SectorLayout {
        id: id.to_string(),
        title: id.to_uppercase(),
        strips: strips
            .iter()
            .map(|s| (!s.is_empty()).then(|| band(s)))
            .collect(),
    }
}

/// These names as a layout's slots, `""` an empty slot.
fn names(all: &[&str]) -> Vec<String> {
    all.iter().map(|s| s.to_string()).collect()
}

fn doc(sectors: Vec<(SectorId, Vec<SectorLayout>)>) -> SectorsDoc {
    SectorsDoc {
        rev: 1,
        sectors: sectors.into_iter().collect(),
    }
}

/// The iPad's side at 1194 px: (1194 − 12 − 132 − 12) / 2.
const IPAD_SIDE: f64 = 519.0;
/// A height that keeps two lines.
const TALL: f64 = 822.0;

/// The cells' labels, `""` an empty cell.
fn labels(cells: &[SectorCell]) -> Vec<String> {
    cells
        .iter()
        .map(|c| c.strip.as_ref().map_or(String::new(), |s| s.label.clone()))
        .collect()
}

fn arranged(doc: &SectorsDoc, picks: &Picks, sides: Sides, height: f64) -> SectorArrangement {
    let layout = layout();
    let pool = pool(&layout);
    arrange_sectors(doc, &pool, picks, sides, height, |_| 0, false, &METRICS)
}

/// The page while ARRANGE is on.
fn arranging(doc: &SectorsDoc, picks: &Picks, sides: Sides, height: f64) -> SectorArrangement {
    let layout = layout();
    let pool = pool(&layout);
    arrange_sectors(doc, &pool, picks, sides, height, |_| 0, true, &METRICS)
}

#[test]
fn an_unknown_ref_is_left_out_of_the_cells() {
    let layout = layout();
    let pool = pool(&layout);
    let shown = sl(
        "lu-1",
        &[
            "Vox 1".to_string(),
            "Gone".to_string(),
            "PA Out".to_string(),
        ],
    );
    let cells = layout_cells(SectorId::Lu, &shown, &pool, &METRICS);
    assert_eq!(labels(&cells), vec!["Vox 1", "PA Out"]);
    assert_eq!(cells[0].units, 1.0);
    assert_eq!(cells[1].units, 1.1);
    assert!(cells.iter().all(|c| c.sector == SectorId::Lu));
}

#[test]
fn a_cell_key_names_its_sector_and_ref() {
    let cell = |sector| SectorCell {
        sector,
        slot: 2,
        strip: Some(band("Vox 1")),
        units: 1.0,
    };
    assert_eq!(cell(SectorId::Lu).key(), "lu:band|Vox 1");
    assert_eq!(cell(SectorId::Ru).key(), "ru:band|Vox 1");
    // An empty slot by its sector and its place.
    assert_eq!(SectorCell::empty(SectorId::Ru, 3).key(), "ru:#3");
    assert_eq!(SectorCell::empty(SectorId::Lu, 0).key(), "lu:#0");
    assert_eq!(SectorCell::empty(SectorId::Lu, 0).units, 1.0);
}

#[test]
fn two_lines_when_both_keep_340_px_else_one() {
    use SectorId::*;
    let two = 2.0 * 340.0 + 6.0;
    assert_eq!(
        sector_lines(two, &METRICS),
        vec![(vec![Lu], vec![Ru]), (vec![Ld], vec![Rd])]
    );
    assert_eq!(
        sector_lines(two.next_down(), &METRICS),
        vec![(vec![Lu, Ld], vec![Ru, Rd])]
    );
}

#[test]
fn the_width_fits_every_layout_of_every_sector_and_paging_keeps_it() {
    // lu's widest layout holds 6 strips, its other 3; ru 2.
    let doc = doc(vec![
        (
            SectorId::Lu,
            vec![
                sl("lu-1", &vox(&[1, 2, 3])),
                sl("lu-2", &vox(&[1, 2, 3, 4, 5, 6])),
            ],
        ),
        (SectorId::Ru, vec![sl("ru-1", &vox(&[7, 8]))]),
    ]);
    let mut picks = Picks::new();
    let first = arranged(&doc, &picks, Sides::both(525.0), TALL);
    assert_eq!(first.width, (525.0 - 20.0) / 6.0);
    assert_eq!(labels(&first.lines[0].left.cells), vox(&[1, 2, 3]));
    assert_eq!(labels(&first.lines[0].right.cells), vox(&[7, 8]));
    assert_eq!(first.lines[0].left.shift, None);
    picks.insert(SectorId::Lu, "lu-2".to_string());
    let second = arranged(&doc, &picks, Sides::both(525.0), TALL);
    assert_eq!(second.width, first.width);
    assert_eq!(
        labels(&second.lines[0].left.cells),
        vox(&[1, 2, 3, 4, 5, 6])
    );
    // The other sides are as they were.
    assert_eq!(second.lines[0].right, first.lines[0].right);
    assert_eq!(second.lines[1], first.lines[1]);
    // The width is a side's own (the iPad's side, 6 strips).
    let ipad = arranged(&doc, &picks, Sides::both(IPAD_SIDE), TALL);
    assert_eq!(ipad.width, (IPAD_SIDE - 20.0) / 6.0);
    // A side's width counts on `sides.width`, its fit on `sides.count`.
    let wider = arranged(
        &doc,
        &picks,
        Sides {
            count: 525.0,
            width: 625.0,
        },
        TALL,
    );
    assert_eq!(wider.width, (625.0 - 20.0) / 6.0);
}

#[test]
fn a_wide_strip_takes_1_1_units() {
    let doc = doc(vec![(
        SectorId::Ld,
        vec![sl(
            "ld-1",
            &["Vox 1", "Vox 2", "Vox 3", "Vox 4", "Vox 5", "PA Out"].map(String::from),
        )],
    )]);
    let a = arranged(&doc, &Picks::new(), Sides::both(525.0), TALL);
    assert!(
        (a.width - 505.0 / 6.1).abs() < 1e-9,
        "{} against {}",
        a.width,
        505.0 / 6.1
    );
    let cells = &a.lines[1].left.cells;
    assert_eq!(cells.last().map(|c| c.units), Some(1.1));
}

#[test]
fn the_width_is_clamped_to_64_120() {
    use SectorId::*;
    let doc = doc([Lu, Ld, Ru, Rd]
        .into_iter()
        .zip(1..)
        .map(|(sector, n)| {
            (
                sector,
                vec![sl(&format!("{}-1", sector.as_str()), &vox(&[n]))],
            )
        })
        .collect());
    let a = arranged(&doc, &Picks::new(), Sides::both(525.0), TALL);
    assert_eq!(a.width, 120.0);
    // Nothing at all: no limit but the widest.
    let none = arranged(
        &SectorsDoc::empty(),
        &Picks::new(),
        Sides::both(525.0),
        TALL,
    );
    assert_eq!(none.width, 120.0);
    assert!(none.windows().is_empty());
}

#[test]
fn a_side_that_does_not_fit_at_64_gets_a_window_and_keeps_64() {
    // A phone's side, 200 px: three strips fit at 64 px exactly
    // (3 × 64 + 2 × 4 = 200), the fourth does not.
    let doc = doc(vec![
        (SectorId::Lu, vec![sl("lu-1", &vox(&[1, 2, 3, 4, 5, 6]))]),
        (SectorId::Ru, vec![sl("ru-1", &vox(&[7]))]),
    ]);
    let layout = layout();
    let pool = pool(&layout);
    let at = |offset: usize, sides: Sides| {
        arrange_sectors(
            &doc,
            &pool,
            &Picks::new(),
            sides,
            TALL,
            |_| offset,
            false,
            &METRICS,
        )
    };
    let a = at(0, Sides::both(200.0));
    assert_eq!(a.width, 64.0);
    let lu = WindowKey {
        sectors: vec![SectorId::Lu],
    };
    let shift = a.shift_of(&lu).expect("a window");
    assert_eq!(shift.label(), "1–3/6");
    assert_eq!((shift.offset, shift.free, shift.last), (0, 3, 3));
    assert_eq!(labels(&a.lines[0].left.cells), vox(&[1, 2, 3]));
    // The right side fits: no window, its cells all shown.
    let ru = WindowKey {
        sectors: vec![SectorId::Ru],
    };
    assert_eq!(a.shift_of(&ru), None);
    assert_eq!(a.windows(), vec![lu.clone()]);
    // An offset past the last window is clamped to it.
    let far = at(9, Sides::both(200.0));
    let shift = far.shift_of(&lu).expect("a window");
    assert_eq!((shift.offset, shift.label()), (3, "4–6/6".to_string()));
    assert_eq!(labels(&far.lines[0].left.cells), vox(&[4, 5, 6]));
    // A side with a window fills a wider side with the narrowest strip
    // scaled (the phone's column closed).
    let stretched = at(
        0,
        Sides {
            count: 200.0,
            width: 300.0,
        },
    );
    assert_eq!(stretched.width, 96.0);
}

#[test]
fn one_line_puts_up_then_down_on_each_side() {
    let doc = doc(vec![
        (SectorId::Lu, vec![sl("lu-1", &vox(&[1]))]),
        (SectorId::Ld, vec![sl("ld-1", &vox(&[2, 3]))]),
        (SectorId::Ru, vec![sl("ru-1", &vox(&[4]))]),
        (SectorId::Rd, vec![sl("rd-1", &vox(&[5]))]),
    ]);
    let a = arranged(&doc, &Picks::new(), Sides::both(525.0), 500.0);
    assert_eq!(a.lines.len(), 1);
    let line = &a.lines[0];
    assert_eq!(labels(&line.left.cells), vox(&[1, 2, 3]));
    assert_eq!(
        line.left.cells.iter().map(|c| c.sector).collect::<Vec<_>>(),
        vec![SectorId::Lu, SectorId::Ld, SectorId::Ld]
    );
    assert_eq!(labels(&line.right.cells), vox(&[4, 5]));
    assert_eq!(line.left.key.sectors, vec![SectorId::Lu, SectorId::Ld]);
    // Both sides' needs together: 3 strips on the left.
    assert_eq!(a.width, 120.0);
    let narrow = arranged(&doc, &Picks::new(), Sides::both(300.0), 500.0);
    assert_eq!(narrow.width, (300.0 - 8.0) / 3.0);
}

#[test]
fn an_empty_sector_gives_no_width_limit() {
    let doc = doc(vec![
        (SectorId::Lu, vec![sl("lu-1", &vox(&[1, 2, 3, 4, 5, 6]))]),
        (SectorId::Ru, vec![sl("ru-1", &[])]),
        (SectorId::Rd, vec![]),
    ]);
    let a = arranged(&doc, &Picks::new(), Sides::both(525.0), TALL);
    assert_eq!(a.width, 505.0 / 6.0);
    assert!(a.lines[0].right.cells.is_empty());
    assert_eq!(a.lines[0].right.shift, None);
    assert!(a.windows().is_empty());
}

#[test]
fn the_widest_layout_wins_and_a_tie_takes_the_first() {
    let layout = layout();
    let pool = pool(&layout);
    let doc = doc(vec![(
        SectorId::Lu,
        vec![
            sl("lu-1", &vox(&[1, 2])),
            sl("lu-2", &vox(&[3, 4, 5])),
            sl("lu-3", &vox(&[6, 7, 8])),
        ],
    )]);
    let cells = widest(&doc, &pool, SectorId::Lu, &METRICS);
    assert_eq!(labels(&cells), vox(&[3, 4, 5]));
    assert_eq!(widest(&doc, &pool, SectorId::Rd, &METRICS), Vec::new());
}

#[test]
fn a_span_and_a_fit_count_the_units_and_the_gaps_between_cells() {
    let cell = |label: &str, units: f64| SectorCell {
        sector: SectorId::Lu,
        slot: 0,
        strip: Some(band(label)),
        units,
    };
    let cells = vec![cell("Vox 1", 1.0), cell("PA Out", 1.1), cell("Vox 2", 1.0)];
    // 3.1 strips of 50 px and two gaps of 4.
    assert!((span(&cells, 50.0, &METRICS) - 163.0).abs() < 1e-9);
    assert_eq!(span(&cells[..1], 50.0, &METRICS), 50.0);
    assert_eq!(span(&[], 50.0, &METRICS), 0.0);
    assert!((fit_width(&cells, 163.0, &METRICS).expect("cells") - 50.0).abs() < 1e-9);
    assert_eq!(fit_width(&cells[..1], 70.0, &METRICS), Some(70.0));
    assert_eq!(fit_width(&[], 70.0, &METRICS), None);
    // At the narrowest strip exactly: fits; a hair narrower: no.
    let two = vec![cell("Vox 1", 1.0), cell("Vox 2", 1.0)];
    assert!(fits(&two, 132.0, &METRICS));
    assert!(!fits(&two, 132.0_f64.next_down(), &METRICS));
    assert!(fits(&[], 0.0, &METRICS));
}

#[test]
fn an_empty_slot_is_a_blank_cell_one_unit_wide_in_its_place() {
    let layout = layout();
    let pool = pool(&layout);
    let shown = sl("ru-1", &names(&["Vox 1", "", "Gone", "", "PA Out"]));
    let cells = layout_cells(SectorId::Ru, &shown, &pool, &METRICS);
    assert_eq!(labels(&cells), names(&["Vox 1", "", "", "PA Out"]));
    let slots: Vec<usize> = cells.iter().map(|c| c.slot).collect();
    assert_eq!(slots, vec![0, 1, 3, 4], "Gone shows nowhere, its slot kept");
    let units: Vec<f64> = cells.iter().map(|c| c.units).collect();
    assert_eq!(units, vec![1.0, 1.0, 1.0, 1.1]);
    assert_eq!(cells[1], SectorCell::empty(SectorId::Ru, 1));
}

#[test]
fn the_width_counts_the_gaps_of_every_layout() {
    // ru-2 holds two strips but its last sits in the sixth slot: 6 units.
    let doc = doc(vec![
        (SectorId::Lu, vec![sl("lu-1", &vox(&[1, 2]))]),
        (
            SectorId::Ru,
            vec![
                sl("ru-1", &vox(&[3])),
                sl("ru-2", &names(&["Vox 4", "", "", "", "", "Vox 5"])),
            ],
        ),
    ]);
    let page = arranged(&doc, &Picks::new(), Sides::both(IPAD_SIDE), TALL);
    assert_eq!(page.width, (IPAD_SIDE - 20.0) / 6.0);
    let mut picks = Picks::new();
    picks.insert(SectorId::Ru, "ru-2".to_string());
    let paged = arranged(&doc, &picks, Sides::both(IPAD_SIDE), TALL);
    assert_eq!(paged.width, page.width, "paging resizes nothing");
    // From the column: Vox 4, four blanks, Vox 5 at the side's far edge.
    let right = &paged.lines[0].right.cells;
    assert_eq!(labels(right), names(&["Vox 4", "", "", "", "", "Vox 5"]));
    assert!((span(right, paged.width, &METRICS) - IPAD_SIDE).abs() < 1e-9);
}

#[test]
fn arrange_shows_every_further_position_up_to_6_units_as_empty_cells() {
    // lu's widest holds 6 strips, so the width fits 6 a side.
    let doc = doc(vec![
        (SectorId::Lu, vec![sl("lu-1", &vox(&[1, 2, 3, 4, 5, 6]))]),
        (
            SectorId::Ru,
            vec![sl("ru-1", &names(&["Vox 7", "", "Vox 8"]))],
        ),
        (SectorId::Rd, vec![sl("rd-1", &names(&["PA Out"]))]),
    ]);
    let off = arranged(&doc, &Picks::new(), Sides::both(IPAD_SIDE), TALL);
    let on = arranging(&doc, &Picks::new(), Sides::both(IPAD_SIDE), TALL);
    assert_eq!(on.width, off.width, "ARRANGE resizes nothing");
    assert_eq!(
        labels(&off.lines[0].right.cells),
        names(&["Vox 7", "", "Vox 8"])
    );
    assert_eq!(
        labels(&on.lines[0].right.cells),
        names(&["Vox 7", "", "Vox 8", "", "", ""])
    );
    let slots: Vec<usize> = on.lines[0].right.cells.iter().map(|c| c.slot).collect();
    assert_eq!(slots, vec![0, 1, 2, 3, 4, 5]);
    // A full sector gets none; a wide strip's sector stops at 6 units
    // (PA Out 1.1 and four blanks: 5.1, a fifth would be 6.1).
    assert_eq!(on.lines[0].left.cells, off.lines[0].left.cells);
    assert_eq!(
        labels(&on.lines[1].right.cells),
        names(&["PA Out", "", "", "", ""])
    );
    // A sector without a layout: nothing to drop into.
    assert_eq!(on.lines[1].left.cells, Vec::<SectorCell>::new());
    // An empty layout: six blanks from slot 0.
    let blank = doc_with_empty_ld();
    let on = arranging(&blank, &Picks::new(), Sides::both(IPAD_SIDE), TALL);
    assert_eq!(
        labels(&on.lines[1].left.cells),
        names(&["", "", "", "", "", ""])
    );
    assert_eq!(
        on.lines[1].left.cells[0],
        SectorCell::empty(SectorId::Ld, 0)
    );
}

#[test]
fn arrange_adds_only_what_fits_the_side_at_the_strip_width() {
    // Every layout small: the width is the 120 px top, so a 519 px side
    // fits 4 cells (4 × 120 + 3 × 4 = 492), not 5 (616).
    let doc = doc(vec![(SectorId::Lu, vec![sl("lu-1", &vox(&[1, 2]))])]);
    let on = arranging(&doc, &Picks::new(), Sides::both(IPAD_SIDE), TALL);
    assert_eq!(on.width, 120.0);
    assert_eq!(
        labels(&on.lines[0].left.cells),
        names(&["Vox 1", "Vox 2", "", ""])
    );
    // with_spaces at the boundary: 3 strips of 100 px and 2 gaps of 4 =
    // 308; one more is 412: fits 412 + 0.5, not a hair less.
    let three: Vec<SectorCell> = (0..3).map(|n| SectorCell::empty(SectorId::Lu, n)).collect();
    let fit = |side: f64| with_spaces(three.clone(), SectorId::Lu, 3, 100.0, side, &METRICS).len();
    assert_eq!(fit(412.0 - FIT_SLACK), 4);
    assert_eq!(fit((412.0 - FIT_SLACK).next_down()), 3);
    assert_eq!(FIT_SLACK, 0.5);
    // Five units leave one; six none.
    let five: Vec<SectorCell> = (0..5).map(|n| SectorCell::empty(SectorId::Lu, n)).collect();
    assert_eq!(
        with_spaces(five, SectorId::Lu, 5, 10.0, 1e6, &METRICS).len(),
        6
    );
    let six: Vec<SectorCell> = (0..6).map(|n| SectorCell::empty(SectorId::Lu, n)).collect();
    assert_eq!(
        with_spaces(six, SectorId::Lu, 6, 10.0, 1e6, &METRICS).len(),
        6
    );
}

#[test]
fn arrange_adds_no_cells_to_a_side_of_two_sectors_or_with_a_window() {
    let doc = doc(vec![
        (SectorId::Lu, vec![sl("lu-1", &vox(&[1]))]),
        (SectorId::Ld, vec![sl("ld-1", &vox(&[2]))]),
    ]);
    // One line (a low screen): each side holds up then down.
    let low = arranging(&doc, &Picks::new(), Sides::both(IPAD_SIDE), 400.0);
    assert_eq!(low.lines.len(), 1);
    assert_eq!(labels(&low.lines[0].left.cells), vox(&[1, 2]));
    // A phone's side with a window keeps its window's cells.
    let doc = doc_of_seven();
    let phone = arranging(&doc, &Picks::new(), Sides::both(200.0), TALL);
    assert!(phone.lines[0].left.shift.is_some());
    assert_eq!(labels(&phone.lines[0].left.cells), vox(&[1, 2, 3]));
}

fn doc_of_seven() -> SectorsDoc {
    doc(vec![(
        SectorId::Lu,
        vec![sl("lu-1", &vox(&[1, 2, 3, 4, 5, 6, 7]))],
    )])
}

fn doc_with_empty_ld() -> SectorsDoc {
    doc(vec![
        (SectorId::Lu, vec![sl("lu-1", &vox(&[1, 2, 3, 4, 5, 6]))]),
        (SectorId::Ld, vec![sl("ld-1", &[])]),
    ])
}

#[test]
fn a_cell_is_found_by_its_key_on_its_line() {
    let doc = doc(vec![
        (SectorId::Lu, vec![sl("lu-1", &vox(&[1, 2]))]),
        (SectorId::Rd, vec![sl("rd-1", &names(&["", "Vox 3"]))]),
    ]);
    let page = arranged(&doc, &Picks::new(), Sides::both(IPAD_SIDE), TALL);
    assert_eq!(page.cell_of(0, "lu:band|Vox 2").map(|c| c.slot), Some(1));
    assert_eq!(page.cell_of(1, "rd:band|Vox 3").map(|c| c.slot), Some(1));
    assert_eq!(page.cell_of(1, "rd:#0").map(|c| c.slot), Some(0));
    assert_eq!(page.cell_of(1, "lu:band|Vox 2"), None, "another line");
    assert_eq!(page.cell_of(2, "lu:band|Vox 2"), None);
}
