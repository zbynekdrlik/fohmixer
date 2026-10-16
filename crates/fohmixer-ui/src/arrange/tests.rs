use super::*;
use serde_json::{Value, json};

fn strip(name: &str) -> Value {
    json!({"kind": "strip", "binding": {"instance": "band", "anchor": {"kind": "track", "name": name}},
           "strip_kind": "standard"})
}

fn pinned(name: &str) -> Value {
    let mut v = strip(name);
    v["pinned"] = json!(true);
    v
}

fn wide(name: &str) -> Value {
    let mut v = strip(name);
    v["wide"] = json!(true);
    v
}

fn toggle(label: &str) -> Value {
    json!({"kind": "param_toggle", "label": label, "press": "toggle",
           "targets": [{"binding": {"instance": "band", "anchor": {"kind": "track", "name": "T"}},
                        "prop": "mute", "on": false, "off": true}]})
}

fn group(id: &str, controls: Vec<Value>) -> Value {
    json!({"kind": "group", "id": id, "title": id, "controls": controls})
}

fn pager(subs: Vec<(&str, Vec<Value>)>) -> Value {
    let pages: Vec<Value> = subs
        .into_iter()
        .map(|(id, sections)| json!({"id": id, "title": id, "sections": sections}))
        .collect();
    json!({"kind": "pager", "id": "pg", "default_page": "stage", "pages": pages})
}

fn page(rows: Vec<Vec<Value>>) -> Page {
    let rows: Vec<Value> = rows
        .into_iter()
        .map(|sections| json!({"sections": sections}))
        .collect();
    serde_json::from_value(json!({"id": "p", "title": "P", "rows": rows})).expect("a page")
}

/// The FOH page's shape (the counts and pins of the owner's, invented
/// names): groups stage 0, bandb 1, others 2, band 3, effects 4, master 5,
/// hands 6.
fn foh() -> Page {
    let stage = (0..10)
        .map(|i| {
            if i == 9 {
                pinned("Vox bus")
            } else {
                strip(&format!("S{i}"))
            }
        })
        .collect();
    let band_b = (0..5).map(|i| strip(&format!("B{i}"))).collect();
    let others = (0..2).map(|i| strip(&format!("O{i}"))).collect();
    page(vec![
        vec![
            pager(vec![
                ("stage", vec![group("stage", stage)]),
                ("bandb", vec![group("bandb", band_b)]),
                ("others", vec![group("others", others)]),
            ]),
            group("band", vec![pinned("Playback"), pinned("Band ret")]),
        ],
        vec![
            group("effects", vec![strip("E0"), strip("E1"), strip("E2")]),
            group(
                "master",
                vec![
                    strip("M0"),
                    strip("M1"),
                    strip("M2"),
                    pinned("Out C"),
                    pinned("Speakers"),
                ],
            ),
            group(
                "hands",
                vec![pinned("Wireless 1"), strip("H1"), strip("H2"), strip("H3")],
            ),
        ],
    ])
}

fn c(group: usize, control: usize) -> Cell {
    Cell::Control {
        at: At { group, control },
        units: 1.0,
        pinned: false,
    }
}

fn p(group: usize, control: usize) -> Cell {
    Cell::Control {
        at: At { group, control },
        units: 1.0,
        pinned: true,
    }
}

fn w(group: usize, control: usize) -> Cell {
    Cell::Control {
        at: At { group, control },
        units: 1.1,
        pinned: false,
    }
}

fn keys(cells: &[Cell]) -> Vec<String> {
    cells.iter().map(Cell::key).collect()
}

fn ones(n: usize) -> Vec<Cell> {
    (0..n).map(|i| c(0, i)).collect()
}

#[test]
fn the_page_model_holds_every_group_once_in_document_order() {
    let model = PageModel::of(&foh());
    let ids: Vec<String> = model.groups.iter().map(|g| g.id.clone().unwrap()).collect();
    assert_eq!(
        ids,
        [
            "stage", "bandb", "others", "band", "effects", "master", "hands"
        ]
    );
    assert_eq!(model.rows(), 2);
    assert!(matches!(
        model.control(At { group: 3, control: 1 }),
        Some(Control::Strip(s)) if s.pinned
    ));
    assert!(matches!(
        model.control(At { group: 4, control: 2 }),
        Some(Control::Strip(s)) if !s.pinned
    ));
    assert_eq!(
        model.control(At {
            group: 3,
            control: 2
        }),
        None
    );
    assert_eq!(
        model.control(At {
            group: 9,
            control: 0
        }),
        None
    );
}

#[test]
fn a_groups_controls_are_its_cells_a_wide_strip_wider_and_buttons_one_block() {
    let model = PageModel::of(&page(vec![
        vec![
            group("a", vec![strip("A0"), wide("A1"), pinned("A2")]),
            group("buttons", vec![toggle("T0"), toggle("T1")]),
            group("empty", vec![]),
            group("mixed", vec![strip("X0"), toggle("X1")]),
        ],
        vec![],
    ]));
    let cells = row_cells(&model, 0, None, &METRICS);
    assert_eq!(
        cells,
        vec![
            c(0, 0),
            w(0, 1),
            p(0, 2),
            Cell::Block {
                group: 1,
                units: 2.4
            },
            c(3, 0),
            c(3, 1),
        ]
    );
    assert_eq!(
        cells.iter().map(|c| c.units()).collect::<Vec<_>>(),
        vec![1.0, 1.1, 1.0, 2.4, 1.0, 1.0]
    );
    assert_eq!(
        cells.iter().map(Cell::pinned).collect::<Vec<_>>(),
        vec![false, false, true, false, false, false]
    );
    assert_eq!(
        cells.iter().map(Cell::group).collect::<Vec<_>>(),
        vec![Some(0), Some(0), Some(0), Some(1), Some(3), Some(3)]
    );
    assert_eq!(keys(&cells), ["c0-0", "c0-1", "c0-2", "b1", "c3-0", "c3-1"]);
    // An empty row, and a row the page lacks.
    assert_eq!(row_cells(&model, 1, None, &METRICS), vec![]);
    assert_eq!(row_cells(&model, 2, None, &METRICS), vec![]);
    let blank = Cell::Blank { slot: 3 };
    assert_eq!(blank.units(), 1.0);
    assert_eq!(blank.group(), None);
    assert_eq!(blank.key(), "s3");
    assert!(!blank.pinned());
}

#[test]
fn a_pager_region_keeps_one_width_and_a_pinned_strip_its_slot() {
    let model = PageModel::of(&foh());
    let stage = row_cells(&model, 0, Some(0), &METRICS);
    assert_eq!(
        keys(&stage),
        [
            "c0-0", "c0-1", "c0-2", "c0-3", "c0-4", "c0-5", "c0-6", "c0-7", "c0-8", "c0-9", "c3-0",
            "c3-1"
        ]
    );
    // BAND B fills the free slots, blanks follow, the pinned strip and BAND
    // keep their places.
    assert_eq!(
        keys(&row_cells(&model, 0, Some(1), &METRICS)),
        [
            "c1-0", "c1-1", "c1-2", "c1-3", "c1-4", "s5", "s6", "s7", "s8", "c0-9", "c3-0", "c3-1"
        ]
    );
    // No sub-page: blanks and the pinned strip.
    assert_eq!(
        keys(&row_cells(&model, 0, None, &METRICS)),
        [
            "s0", "s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "c0-9", "c3-0", "c3-1"
        ]
    );
}

#[test]
fn a_pinned_strip_whose_slot_is_taken_takes_the_next_free_one_and_region_strips_are_one_unit() {
    // Two sub-pages pinning their first strip: two others each and both
    // pins, four slots.
    let model = PageModel::of(&page(vec![vec![pager(vec![
        (
            "a",
            vec![group("a", vec![pinned("A0"), strip("A1"), strip("A2")])],
        ),
        (
            "b",
            vec![group("b", vec![pinned("B0"), wide("B1"), strip("B2")])],
        ),
    ])]]));
    assert_eq!(
        row_cells(&model, 0, Some(0), &METRICS),
        vec![p(0, 0), p(1, 0), c(0, 1), c(0, 2)]
    );
    // B1 is wide, but a region's slots are one unit each.
    assert_eq!(
        row_cells(&model, 0, Some(1), &METRICS),
        vec![p(0, 0), p(1, 0), c(1, 1), c(1, 2)]
    );
}

#[test]
fn rows_are_lines_of_their_own_while_each_keeps_340_px_of_its_share() {
    // Two even rows: 340 px each and the 6 px gap.
    assert_eq!(lines(&[1.0, 1.0], 686.0, &METRICS), vec![vec![0], vec![1]]);
    assert_eq!(
        lines(&[1.0, 1.0], 686.0_f64.next_down(), &METRICS),
        vec![vec![0, 1]]
    );
    assert_eq!(
        lines(&[1.0, 1.0, 1.0], 1032.0, &METRICS),
        vec![vec![0], vec![1], vec![2]]
    );
    // A row of weight 1 beside one of 1.5 gets 2/5 of the room: 850 px
    // leave it 340 (a 3:2 split's lighter row is the one that must fit).
    assert_eq!(lines(&[1.5, 1.0], 856.0, &METRICS), vec![vec![0], vec![1]]);
    assert_eq!(
        lines(&[1.5, 1.0], 856.0_f64.next_down(), &METRICS),
        vec![vec![0, 1]]
    );
    // The lightest weight counts as a weight, not as 1: the lighter row of
    // 2:3 gets 2/5 of 850 px, each row of 0.5:0.5 half of 680 px.
    assert_eq!(lines(&[2.0, 3.0], 856.0, &METRICS), vec![vec![0], vec![1]]);
    assert_eq!(
        lines(&[2.0, 3.0], 856.0_f64.next_down(), &METRICS),
        vec![vec![0, 1]]
    );
    assert_eq!(lines(&[0.5, 0.5], 686.0, &METRICS), vec![vec![0], vec![1]]);
    assert_eq!(
        lines(&[0.5, 0.5], 686.0_f64.next_down(), &METRICS),
        vec![vec![0, 1]]
    );
    assert_eq!(lines(&[1.0], 200.0, &METRICS), vec![vec![0]]);
    assert_eq!(lines(&[], 900.0, &METRICS), Vec::<Vec<usize>>::new());
}

#[test]
fn a_half_spans_its_cells_and_gaps_and_the_widest_fitting_strip_fills_the_side() {
    assert_eq!(span(&ones(6), 83.0, &METRICS), 6.0 * 83.0 + 5.0 * 4.0);
    assert_eq!(span(&ones(1), 83.0, &METRICS), 83.0);
    assert_eq!(span(&[], 83.0, &METRICS), 0.0);
    assert!((span(&[c(0, 0), w(0, 1)], 100.0, &METRICS) - 214.0).abs() < 1e-9);
    assert_eq!(
        fit(&ones(6), &ones(6), 519.0, &METRICS),
        (519.0 - 20.0) / 6.0
    );
    assert_eq!(
        fit(&ones(6), &ones(4), 519.0, &METRICS),
        (519.0 - 20.0) / 6.0
    );
    assert_eq!(
        fit(&ones(2), &ones(3), 200.0, &METRICS),
        (200.0 - 8.0) / 3.0
    );
    assert_eq!(
        fit(&ones(3), &ones(2), 200.0, &METRICS),
        (200.0 - 8.0) / 3.0
    );
    assert_eq!(fit(&[], &ones(2), 200.0, &METRICS), 98.0);
    assert!((fit(&[w(0, 0)], &[], 110.0, &METRICS) - 100.0).abs() < 1e-9);
    assert_eq!(fit(&[], &[], 200.0, &METRICS), f64::INFINITY);
}

#[test]
fn a_line_is_cut_where_its_wider_half_is_the_narrowest() {
    assert_eq!(split(&ones(12), &METRICS), 6);
    // 3 | 2 and 2 | 3 are as wide: the odd cell goes left.
    assert_eq!(split(&ones(5), &METRICS), 3);
    assert_eq!(split(&ones(3), &METRICS), 2);
    assert_eq!(split(&ones(1), &METRICS), 1);
    assert_eq!(split(&[], &METRICS), 0);
    // 2 | 2 (132 | 144.8 px) beats 3 | 1 (206.4 | 70.4).
    assert_eq!(split(&[c(0, 0), c(0, 1), w(1, 0), w(1, 1)], &METRICS), 2);
    // A block counts 2.4 strips: 1 | 3 (153.6 | 200) beats 2 | 2 (221.6 | 132).
    assert_eq!(
        split(
            &[
                Cell::Block {
                    group: 2,
                    units: 2.4
                },
                c(0, 0),
                c(0, 1),
                c(0, 2)
            ],
            &METRICS
        ),
        1
    );
}

#[test]
fn a_line_that_fits_shows_every_cell_and_needs_no_arrows() {
    // Two strips a side at exactly 64 px.
    assert_eq!(shown(&ones(4), 132.0, 0, &METRICS), (ones(4), None));
    // A hair narrower: two of them and arrows.
    assert_eq!(
        shown(&ones(4), 132.0_f64.next_down(), 0, &METRICS),
        (
            ones(2),
            Some(Shift {
                offset: 0,
                free: 2,
                step: 2,
                last: 2,
                total: 4
            })
        )
    );
}

#[test]
fn a_line_first_drops_its_blanks_and_pinned_cells_alone_show_as_they_are() {
    let cells = [
        c(0, 0),
        Cell::Blank { slot: 1 },
        Cell::Blank { slot: 2 },
        c(0, 3),
    ];
    assert_eq!(
        shown(&cells, 64.0, 0, &METRICS),
        (vec![c(0, 0), c(0, 3)], None)
    );
    let pins = [p(0, 0), p(0, 1), p(0, 2)];
    assert_eq!(shown(&pins, 64.0, 0, &METRICS), (pins.to_vec(), None));
}

#[test]
fn a_line_that_does_not_fit_shows_its_pinned_cells_and_a_window_the_arrows_move() {
    // Ten strips, the 3rd and the 8th pinned; sides of 199 px hold two strips
    // at 64 px: the pins and two others.
    let cells: Vec<Cell> = (0..10)
        .map(|i| if i == 2 || i == 7 { p(0, i) } else { c(0, i) })
        .collect();
    let shift = |offset| Shift {
        offset,
        free: 2,
        step: 2,
        last: 6,
        total: 8,
    };
    let (cells0, shift0) = shown(&cells, 199.0, 0, &METRICS);
    assert_eq!(keys(&cells0), ["c0-0", "c0-1", "c0-2", "c0-7"]);
    assert_eq!(shift0, Some(shift(0)));
    let (cells3, shift3) = shown(&cells, 199.0, 3, &METRICS);
    assert_eq!(keys(&cells3), ["c0-2", "c0-4", "c0-5", "c0-7"]);
    assert_eq!(shift3, Some(shift(3)));
    // Past the end: the last window.
    let (cells9, shift9) = shown(&cells, 199.0, 99, &METRICS);
    assert_eq!(keys(&cells9), ["c0-2", "c0-7", "c0-8", "c0-9"]);
    assert_eq!(shift9, Some(shift(6)));
}

#[test]
fn a_window_over_a_wider_strip_shows_fewer_and_the_last_window_reaches_the_last_cell() {
    // Sides of 136 px: two strips at 64 px, but not a strip and a wide one.
    // The last window: the last other cell alone fits beside the pin.
    let cells = [p(0, 0), c(0, 1), c(0, 2), w(0, 3), c(0, 4)];
    let shift = |offset, step| {
        Some(Shift {
            offset,
            free: step,
            step,
            last: 3,
            total: 4,
        })
    };
    assert_eq!(
        shown(&cells, 136.0, 0, &METRICS),
        (vec![p(0, 0), c(0, 1), c(0, 2)], shift(0, 2))
    );
    assert_eq!(
        shown(&cells, 136.0, 2, &METRICS),
        (vec![p(0, 0), w(0, 3)], shift(2, 1))
    );
    assert_eq!(
        shown(&cells, 136.0, 3, &METRICS),
        (vec![p(0, 0), c(0, 4)], shift(3, 1))
    );
    assert_eq!(
        shown(&cells, 136.0, 9, &METRICS),
        (vec![p(0, 0), c(0, 4)], shift(3, 1))
    );
}

#[test]
fn pinned_cells_that_do_not_fit_with_one_other_show_alone() {
    // Three pins and one other on 64 px sides: not even the pins fit, the
    // other never shows.
    let cells = [p(0, 0), p(0, 1), p(0, 2), c(0, 3)];
    assert_eq!(
        shown(&cells, 64.0, 0, &METRICS),
        (
            vec![p(0, 0), p(0, 1), p(0, 2)],
            Some(Shift {
                offset: 0,
                free: 0,
                step: 1,
                last: 1,
                total: 1
            })
        )
    );
}

#[test]
fn the_last_window_is_the_last_cells_that_fit() {
    // Sides of 136 px: two strips a side, but a strip and a wide one not.
    // From the end the two wide ones fit, three cells do not: the last
    // window starts at the fifth cell (from the third, three would fit).
    let cells = [c(0, 0), c(0, 1), c(0, 2), c(0, 3), w(0, 4), w(0, 5)];
    assert_eq!(
        shown(&cells, 136.0, 99, &METRICS),
        (
            vec![w(0, 4), w(0, 5)],
            Some(Shift {
                offset: 4,
                free: 2,
                step: 2,
                last: 4,
                total: 6
            })
        )
    );
}

#[test]
fn the_last_window_ends_at_the_last_cell_even_where_a_window_from_the_start_holds_more() {
    // The fixture's FOH page on a phone on its side (sides of 336 px): ten
    // cells, three of them wide. From the start nine fit, at the end eight:
    // the last window starts at the third.
    let cells = [
        c(0, 0),
        c(0, 1),
        w(1, 0),
        w(1, 1),
        c(2, 0),
        c(2, 1),
        c(3, 0),
        c(4, 0),
        c(4, 1),
        w(5, 0),
    ];
    let (first, shift) = shown(&cells, 336.0, 0, &METRICS);
    assert_eq!(first, cells[..9].to_vec());
    assert_eq!(
        shift,
        Some(Shift {
            offset: 0,
            free: 9,
            step: 9,
            last: 2,
            total: 10
        })
    );
    let (end, shift) = shown(&cells, 336.0, 2, &METRICS);
    assert_eq!(end, cells[2..].to_vec());
    assert_eq!(
        shift,
        Some(Shift {
            offset: 2,
            free: 8,
            step: 8,
            last: 2,
            total: 10
        })
    );
}

#[test]
fn the_tablet_shows_both_rows_whole_and_a_sub_page_switch_moves_nothing_after_the_region() {
    let model = PageModel::of(&foh());
    let stage = arrange(&model, Some(0), 519.0, 822.0, |_| 0, &METRICS);
    assert_eq!(stage.width, (519.0 - 20.0) / 6.0);
    assert_eq!(stage.lines.len(), 2);
    let line = &stage.lines[0];
    assert_eq!(line.rows, vec![0]);
    assert_eq!(line.shift, None);
    assert_eq!(
        keys(&line.left),
        ["c0-0", "c0-1", "c0-2", "c0-3", "c0-4", "c0-5"]
    );
    assert_eq!(
        keys(&line.right),
        ["c0-6", "c0-7", "c0-8", "c0-9", "c3-0", "c3-1"]
    );
    let lower = &stage.lines[1];
    assert_eq!(lower.rows, vec![1]);
    assert_eq!(
        keys(&lower.left),
        ["c4-0", "c4-1", "c4-2", "c5-0", "c5-1", "c5-2"]
    );
    assert_eq!(
        keys(&lower.right),
        ["c5-3", "c5-4", "c6-0", "c6-1", "c6-2", "c6-3"]
    );
    let band_b = arrange(&model, Some(1), 519.0, 822.0, |_| 0, &METRICS);
    assert_eq!(band_b.width, stage.width);
    assert_eq!(
        keys(&band_b.lines[0].left),
        ["c1-0", "c1-1", "c1-2", "c1-3", "c1-4", "s5"]
    );
    assert_eq!(
        keys(&band_b.lines[0].right),
        ["s6", "s7", "s8", "c0-9", "c3-0", "c3-1"]
    );
    assert_eq!(band_b.lines[1], stage.lines[1]);
}

#[test]
fn a_low_screen_shows_one_line_of_the_pinned_strips_and_a_window_over_the_rest() {
    let model = PageModel::of(&foh());
    // Sides of 336 px: five strips a side at 64 px, the six pins and four others.
    let first = arrange(&model, Some(0), 336.0, 378.0, |_| 0, &METRICS);
    assert_eq!(first.width, 64.0);
    assert_eq!(first.lines.len(), 1);
    let line = &first.lines[0];
    assert_eq!(line.rows, vec![0, 1]);
    assert_eq!(keys(&line.left), ["c0-0", "c0-1", "c0-2", "c0-3", "c0-9"]);
    assert_eq!(keys(&line.right), ["c3-0", "c3-1", "c5-3", "c5-4", "c6-0"]);
    assert_eq!(
        line.shift,
        Some(Shift {
            offset: 0,
            free: 4,
            step: 4,
            last: 14,
            total: 18
        })
    );
    // The window moved by eight: the region's last other and the effects.
    let moved = arrange(&model, Some(0), 336.0, 378.0, |_| 8, &METRICS);
    let line = &moved.lines[0];
    assert_eq!(keys(&line.left), ["c0-8", "c0-9", "c3-0", "c3-1", "c4-0"]);
    assert_eq!(keys(&line.right), ["c4-1", "c4-2", "c5-3", "c5-4", "c6-0"]);
}

#[test]
fn the_strip_width_stays_within_its_bounds() {
    let model = PageModel::of(&page(vec![vec![group(
        "a",
        vec![strip("A0"), strip("A1")],
    )]]));
    let wide_screen = arrange(&model, None, 519.0, 822.0, |_| 0, &METRICS);
    assert_eq!(wide_screen.width, 120.0);
    assert_eq!(keys(&wide_screen.lines[0].left), ["c0-0"]);
    assert_eq!(keys(&wide_screen.lines[0].right), ["c0-1"]);
    // A page without rows has no line.
    let empty = PageModel::of(&page(vec![]));
    assert_eq!(
        arrange(&empty, None, 519.0, 822.0, |_| 0, &METRICS),
        Arrangement {
            width: 120.0,
            lines: vec![]
        }
    );
}

#[test]
fn a_halfs_cells_run_by_group_keyed_by_group_and_turn() {
    let half = [
        c(0, 0),
        c(0, 1),
        Cell::Blank { slot: 2 },
        Cell::Blank { slot: 3 },
        p(0, 9),
        c(3, 0),
        Cell::Block {
            group: 4,
            units: 2.4,
        },
    ];
    let all = runs(&half);
    assert_eq!(
        all.iter().map(|r| r.key.as_str()).collect::<Vec<_>>(),
        ["g0-0", "blank-0", "g0-1", "g3-0", "g4-0"]
    );
    assert_eq!(all[0].cells, vec![c(0, 0), c(0, 1)]);
    assert_eq!(all[0].group, Some(0));
    assert_eq!(all[1].group, None);
    assert_eq!(
        all[1].cells,
        vec![Cell::Blank { slot: 2 }, Cell::Blank { slot: 3 }]
    );
    assert_eq!(all[2].cells, vec![p(0, 9)]);
    assert_eq!(runs(&[]), vec![]);
}

#[test]
fn an_arrow_moves_the_window_a_step_within_the_line_and_the_column_says_where() {
    let at = |offset| Shift {
        offset,
        free: 4,
        step: 4,
        last: 14,
        total: 18,
    };
    assert_eq!(at(0).moved(true), 4);
    assert_eq!(at(12).moved(true), 14);
    assert_eq!(at(14).moved(true), 14);
    assert_eq!(at(8).moved(false), 4);
    assert_eq!(at(3).moved(false), 0);
    assert_eq!(at(0).moved(false), 0);
    assert_eq!(at(0).label(), "1–4/18");
    assert_eq!(at(14).label(), "15–18/18");
    // No other cell fits beside the pins: none shown of how many.
    let none = Shift {
        offset: 0,
        free: 0,
        step: 1,
        last: 1,
        total: 1,
    };
    assert_eq!(none.label(), "0/1");
    assert_eq!(none.moved(true), 1);
    // A last window shorter than a step ends at the last cell.
    let short = Shift {
        offset: 6,
        free: 3,
        step: 3,
        last: 6,
        total: 8,
    };
    assert_eq!(short.label(), "7–8/8");
}

#[test]
fn a_group_of_buttons_in_a_pager_region_is_one_unit_like_every_slot() {
    let model = PageModel::of(&page(vec![vec![pager(vec![
        (
            "a",
            vec![group("a", vec![toggle("T0"), toggle("T1"), toggle("T2")])],
        ),
        (
            "b",
            vec![group("b", vec![strip("B0"), strip("B1"), strip("B2")])],
        ),
    ])]]));
    let buttons = row_cells(&model, 0, Some(0), &METRICS);
    assert_eq!(
        buttons,
        vec![
            Cell::Block {
                group: 0,
                units: 1.0
            },
            Cell::Blank { slot: 1 },
            Cell::Blank { slot: 2 },
        ]
    );
    let units = |cells: &[Cell]| cells.iter().map(|c| c.units()).sum::<f64>();
    assert_eq!(units(&buttons), 3.0);
    assert_eq!(units(&row_cells(&model, 0, Some(1), &METRICS)), 3.0);
    // Outside a region a block is 2.4 strips.
    let fixed = PageModel::of(&page(vec![vec![group("t", vec![toggle("T0")])]]));
    assert_eq!(
        row_cells(&fixed, 0, None, &METRICS),
        vec![Cell::Block {
            group: 0,
            units: 2.4
        }]
    );
}

#[test]
fn every_line_takes_the_strip_width_of_the_tightest_line() {
    let four = || group("u", (0..4).map(|i| strip(&format!("U{i}"))).collect());
    let twelve = || group("l", (0..12).map(|i| strip(&format!("L{i}"))).collect());
    // Four strips alone take the widest strip; twelve need 83 px.
    let alone = PageModel::of(&page(vec![vec![four()]]));
    assert_eq!(
        arrange(&alone, None, 519.0, 822.0, |_| 0, &METRICS).width,
        120.0
    );
    let tight = (519.0 - 20.0) / 6.0;
    let upper_first = PageModel::of(&page(vec![vec![four()], vec![twelve()]]));
    let lower_first = PageModel::of(&page(vec![vec![twelve()], vec![four()]]));
    for model in [upper_first, lower_first] {
        let a = arrange(&model, None, 519.0, 822.0, |_| 0, &METRICS);
        assert_eq!(a.lines.len(), 2);
        assert_eq!(a.width, tight);
    }
}

#[test]
fn each_line_moves_its_own_window() {
    // A phone upright (sides of 135 px: two strips a side): both rows are
    // lines with a window, each with three pins and room for one other.
    let model = PageModel::of(&foh());
    let a = arrange(
        &model,
        Some(0),
        135.0,
        832.0,
        |k: &LineKey| if k.rows == [1] { 2 } else { 0 },
        &METRICS,
    );
    assert_eq!(a.lines.len(), 2);
    let upper = &a.lines[0];
    assert_eq!(keys(&upper.left), ["c0-0", "c0-9"]);
    assert_eq!(keys(&upper.right), ["c3-0", "c3-1"]);
    assert_eq!(upper.shift.map(|s| (s.offset, s.last)), Some((0, 8)));
    let lower = &a.lines[1];
    assert_eq!(keys(&lower.left), ["c4-2", "c5-3"]);
    assert_eq!(keys(&lower.right), ["c5-4", "c6-0"]);
    assert_eq!(lower.shift.map(|s| (s.offset, s.last)), Some((2, 8)));
}

#[test]
fn a_line_with_a_window_takes_the_narrowest_strip_wherever_its_window_stands() {
    // Ten strips, three wide, on sides of 336 px: from the third on eight
    // would fit at 77 px, but the strips stay at 64 px.
    let model = ten_with_three_wide();
    for offset in [0, 2] {
        let a = arrange(&model, None, 336.0, 378.0, |_| offset, &METRICS);
        assert_eq!(a.width, 64.0, "offset {offset}");
        assert_eq!(a.lines[0].shift.map(|s| s.offset), Some(offset));
    }
}

/// Ten strips, three of them wide, in one group: the FOH page's first row
/// on a phone on its side.
fn ten_with_three_wide() -> PageModel {
    let strips = (0..10)
        .map(|i| {
            if i == 2 || i == 3 || i == 9 {
                wide(&format!("W{i}"))
            } else {
                strip(&format!("S{i}"))
            }
        })
        .collect();
    PageModel::of(&page(vec![vec![group("g", strips)]]))
}

#[test]
fn a_phones_thin_strip_widens_the_strips_and_keeps_the_windows() {
    // #63, the approved mockup phone-burger-v1: on a phone on its side the
    // column (148 px) leaves sides of 336 px, the thin strip (34 px) 393.
    // The same strips show, the same windows, 64 × 393 / 336 px wide
    // wherever the window stands.
    let model = ten_with_three_wide();
    let sides = Sides {
        count: 336.0,
        width: 393.0,
    };
    for offset in [0, 2] {
        let column = arrange(&model, None, 336.0, 378.0, |_| offset, &METRICS);
        let thin = arrange_in(&model, None, sides, 378.0, |_| offset, &METRICS);
        assert_eq!(thin.width, 64.0 * 393.0 / 336.0, "offset {offset}");
        assert!((74.8..74.9).contains(&thin.width));
        assert_eq!(thin.lines, column.lines, "offset {offset}");
    }
    // Upright (the column 96 px: sides of 135; the thin strip: 166): two
    // strips a side, 78.7 px wide.
    let model = PageModel::of(&foh());
    let offset = |k: &LineKey| if k.rows == [1] { 2 } else { 0 };
    let column = arrange(&model, Some(0), 135.0, 832.0, offset, &METRICS);
    let sides = Sides {
        count: 135.0,
        width: 166.0,
    };
    let thin = arrange_in(&model, Some(0), sides, 832.0, offset, &METRICS);
    assert_eq!(thin.lines, column.lines);
    assert_eq!(thin.width, 64.0 * 166.0 / 135.0);
}

#[test]
fn a_line_without_a_window_fills_the_wider_side() {
    // Eight strips, four a side: 81 px on sides of 336, 95.25 on 393.
    let strips = (0..8).map(|i| strip(&format!("S{i}"))).collect();
    let model = PageModel::of(&page(vec![vec![group("g", strips)]]));
    let column = arrange(&model, None, 336.0, 378.0, |_| 0, &METRICS);
    assert_eq!(column.width, 81.0);
    assert_eq!(column.lines[0].shift, None);
    let sides = Sides {
        count: 336.0,
        width: 393.0,
    };
    let thin = arrange_in(&model, None, sides, 378.0, |_| 0, &METRICS);
    assert_eq!(thin.width, 95.25);
    assert_eq!(thin.lines, column.lines);
    // One side for both is `arrange`.
    assert_eq!(
        arrange_in(&model, None, Sides::both(336.0), 378.0, |_| 0, &METRICS),
        column
    );
}

#[test]
fn a_strip_stretches_by_the_wider_side_over_the_counted_one() {
    let sides = |count, width| Sides { count, width };
    assert_eq!(stretch(sides(336.0, 393.0)), 393.0 / 336.0);
    assert_eq!(stretch(sides(336.0, 336.0)), 1.0);
    // Never narrower, and 1 for a side not laid out yet.
    assert_eq!(stretch(sides(400.0, 300.0)), 1.0);
    assert_eq!(stretch(sides(0.0, 393.0)), 1.0);
    assert_eq!(stretch(sides(-1.0, 393.0)), 1.0);
}

#[test]
fn the_thin_strip_moves_the_first_line_with_a_window() {
    let model = PageModel::of(&foh());
    // Upright: both lines have a window.
    let upright = arrange(&model, Some(0), 135.0, 832.0, |_| 0, &METRICS);
    assert_eq!(thin_line(&upright), Some(0));
    // What a line's arrows move: its window and its key.
    let lower = &upright.lines[1];
    assert_eq!(
        upright.shift_of(1),
        Some((lower.key.clone(), lower.shift.unwrap()))
    );
    assert_eq!(upright.shift_of(2), None, "no such line");
    let mut lower_only = upright.clone();
    lower_only.lines[0].shift = None;
    assert_eq!(thin_line(&lower_only), Some(1));
    lower_only.lines[1].shift = None;
    assert_eq!(thin_line(&lower_only), None);
    assert_eq!(lower_only.shift_of(1), None, "it fits");
}

#[test]
fn a_css_length_in_px_is_read_from_a_custom_property() {
    assert_eq!(px_value(" 148px"), Some(148.0));
    assert_eq!(px_value("96px"), Some(96.0));
    assert_eq!(px_value(" 34.5 px "), Some(34.5));
    assert_eq!(px_value("0px"), Some(0.0));
    for none in ["", "  ", "3em", "px", "-2px", "NaNpx", "infpx", "148"] {
        assert_eq!(px_value(none), None, "{none:?}");
    }
}

#[test]
fn a_line_is_one_list_of_its_cells_after_their_groups_titles_each_on_its_side() {
    let model = PageModel::of(&foh());
    // BAND B: its five strips and a blank on the left; blanks, the pinned
    // strip of STAGE and BAND on the right.
    let band_b = arrange(&model, Some(1), 519.0, 822.0, |_| 0, &METRICS);
    let line = items(&model, &band_b.lines[0]);
    let shape: Vec<(String, Side)> = line.iter().map(|i| (i.key(), i.side())).collect();
    let l = |k: &str| (k.to_string(), Side::Left);
    let r = |k: &str| (k.to_string(), Side::Right);
    assert_eq!(
        shape,
        vec![
            l("t1-0"),
            l("c1-0"),
            l("c1-1"),
            l("c1-2"),
            l("c1-3"),
            l("c1-4"),
            l("s5"),
            r("s6"),
            r("s7"),
            r("s8"),
            r("t0-0"),
            r("c0-9"),
            r("t3-0"),
            r("c3-0"),
            r("c3-1"),
        ]
    );
    assert_eq!(
        line[12],
        Item::Title {
            key: "t3-0".into(),
            group: 3,
            side: Side::Right,
            cells: 2,
            units: 2.0,
            caption: false
        }
    );
    // STAGE cut by the column: a title on each side, the second keyed apart.
    let stage = arrange(&model, Some(0), 519.0, 822.0, |_| 0, &METRICS);
    let titles: Vec<Item> = items(&model, &stage.lines[0])
        .into_iter()
        .filter(|i| matches!(i, Item::Title { .. }))
        .collect();
    assert_eq!(
        titles,
        vec![
            Item::Title {
                key: "t0-0".into(),
                group: 0,
                side: Side::Left,
                cells: 6,
                units: 6.0,
                caption: false
            },
            Item::Title {
                key: "t0-1".into(),
                group: 0,
                side: Side::Right,
                cells: 4,
                units: 4.0,
                caption: false
            },
            Item::Title {
                key: "t3-0".into(),
                group: 3,
                side: Side::Right,
                cells: 2,
                units: 2.0,
                caption: false
            },
        ]
    );
    // A title spans its cells' units: a wide strip counts 1.1.
    let mixed = PageModel::of(&page(vec![vec![group("m", vec![strip("A"), wide("B")])]]));
    let wide_line = arrange(&mixed, None, 519.0, 822.0, |_| 0, &METRICS);
    let all = items(&mixed, &wide_line.lines[0]);
    let units: f64 = all
        .iter()
        .filter_map(|i| match i {
            Item::Title { units, .. } => Some(*units),
            Item::Cell { .. } => None,
        })
        .sum();
    assert!((units - 2.1).abs() < 1e-9, "{units}");
    assert_eq!(
        items(
            &model,
            &LineArrangement {
                rows: vec![],
                key: LineKey {
                    rows: vec![],
                    sub: None
                },
                left: vec![],
                right: vec![],
                shift: None
            }
        ),
        vec![]
    );
}

#[test]
fn every_region_holds_every_pin_in_one_slot_and_the_shown_sub_pages_others_in_order() {
    // Every pair of sub-pages of up to four strips, each pinned or not.
    let sub = |name: &str, len: usize, mask: u32| -> Vec<Value> {
        (0..len)
            .map(|i| {
                let name = format!("{name}{i}");
                if mask & (1 << i) != 0 {
                    pinned(&name)
                } else {
                    strip(&name)
                }
            })
            .collect()
    };
    for len_a in 0..=4usize {
        for len_b in 0..=4usize {
            for mask_a in 0..(1u32 << len_a) {
                for mask_b in 0..(1u32 << len_b) {
                    let model = PageModel::of(&page(vec![vec![pager(vec![
                        ("a", vec![group("a", sub("A", len_a, mask_a))]),
                        ("b", vec![group("b", sub("B", len_b, mask_b))]),
                    ])]]));
                    let pins = (mask_a.count_ones() + mask_b.count_ones()) as usize;
                    let others = [
                        len_a - mask_a.count_ones() as usize,
                        len_b - mask_b.count_ones() as usize,
                    ];
                    let size = others[0].max(others[1]) + pins;
                    let shown: Vec<Vec<Cell>> = (0..2)
                        .map(|s| row_cells(&model, 0, Some(s), &METRICS))
                        .collect();
                    let case = format!("A {len_a}/{mask_a:b}, B {len_b}/{mask_b:b}");
                    for (s, cells) in shown.iter().enumerate() {
                        assert_eq!(cells.len(), size, "{case}: slots on {s}");
                        // Every pin once, in the same slot whichever sub-page shows.
                        let pinned: Vec<(usize, String)> = cells
                            .iter()
                            .enumerate()
                            .filter(|(_, c)| c.pinned())
                            .map(|(i, c)| (i, c.key()))
                            .collect();
                        assert_eq!(pinned.len(), pins, "{case}: pins on {s}");
                        let other_pins: Vec<(usize, String)> = shown[1 - s]
                            .iter()
                            .enumerate()
                            .filter(|(_, c)| c.pinned())
                            .map(|(i, c)| (i, c.key()))
                            .collect();
                        assert_eq!(pinned, other_pins, "{case}: pin slots");
                        // The shown sub-page's others, in order, then blanks.
                        let mine: Vec<String> = cells
                            .iter()
                            .filter(|c| !c.pinned() && c.group() == Some(s))
                            .map(Cell::key)
                            .collect();
                        let expected: Vec<String> = (0..[len_a, len_b][s])
                            .filter(|&i| [mask_a, mask_b][s] & (1u32 << i) == 0)
                            .map(|i| format!("c{s}-{i}"))
                            .collect();
                        assert_eq!(mine, expected, "{case}: others on {s}");
                        let blanks = cells.iter().filter(|c| c.group().is_none()).count();
                        assert_eq!(blanks, size - pins - others[s], "{case}: blanks on {s}");
                    }
                }
            }
        }
    }
}

#[test]
fn a_lines_window_is_kept_per_rows_and_per_sub_page_only_where_the_pager_is() {
    let model = PageModel::of(&foh());
    assert_eq!(model.pager_row(), Some(0));
    assert_eq!(
        model.line_key(&[0], Some(1)),
        LineKey {
            rows: vec![0],
            sub: Some(1)
        }
    );
    // The lower row holds no pager: its window does not follow the sub-page.
    assert_eq!(
        model.line_key(&[1], Some(1)),
        LineKey {
            rows: vec![1],
            sub: None
        }
    );
    assert_eq!(
        model.line_key(&[0, 1], Some(2)),
        LineKey {
            rows: vec![0, 1],
            sub: Some(2)
        }
    );
    let plain = PageModel::of(&page(vec![vec![group("a", vec![strip("A")])]]));
    assert_eq!(plain.pager_row(), None);
    assert_eq!(
        plain.line_key(&[0], Some(1)),
        LineKey {
            rows: vec![0],
            sub: None
        }
    );
    // The arrangement asks for each line's window by its key.
    let a = arrange(
        &model,
        Some(1),
        135.0,
        832.0,
        |k: &LineKey| if k.sub == Some(1) { 3 } else { 1 },
        &METRICS,
    );
    assert_eq!(a.lines[0].key, model.line_key(&[0], Some(1)));
    assert_eq!(a.lines[0].shift.map(|s| s.offset), Some(3));
    assert_eq!(a.lines[1].shift.map(|s| s.offset), Some(1));
}

#[test]
fn a_block_wider_than_a_side_at_the_narrowest_strip_is_narrowed_to_it() {
    // A phone upright: sides of 135 px hold 2.1 strips at 64 px, a block
    // 2.4: it narrows and shows.
    let model = PageModel::of(&page(vec![vec![group(
        "t",
        vec![toggle("T0"), toggle("T1")],
    )]]));
    let a = arrange(&model, None, 135.0, 832.0, |_| 0, &METRICS);
    let block = Cell::Block {
        group: 0,
        units: 135.0 / 64.0,
    };
    assert_eq!(a.lines[0].left, vec![block]);
    assert_eq!(a.lines[0].shift, None);
    // A wide side keeps it 2.4 strips.
    let wide = arrange(&model, None, 519.0, 822.0, |_| 0, &METRICS);
    assert_eq!(
        wide.lines[0].left,
        vec![Cell::Block {
            group: 0,
            units: 2.4
        }]
    );
}

fn strip_on(instance: &str, name: &str) -> Value {
    let mut v = strip(name);
    v["binding"]["instance"] = json!(instance);
    v
}

fn param_fader(label: &str) -> Value {
    json!({"kind": "param_fader", "label": label,
           "targets": [{"binding": {"instance": "band", "anchor": {"kind": "track", "name": "Drums #"},
                                    "path": "mixer_device volume"},
                        "prop": "value", "scale": "cc_linear"}]})
}

fn titled(id: &str, title: Option<&str>, controls: Vec<Value>) -> Value {
    let mut v = group(id, controls);
    v["title"] = json!(title);
    v
}

fn group_of(value: Value) -> Group {
    let Section::Group(group) = serde_json::from_value::<Section>(value).expect("a section") else {
        panic!("not a group");
    };
    group
}

/// Each cell carrying the group's label: its key, the text and instance.
fn labels(items: &[Item]) -> Vec<(String, String, Option<String>)> {
    items
        .iter()
        .filter_map(|i| match i {
            Item::Cell {
                cell,
                label: Some(l),
                ..
            } => Some((cell.key(), l.text.clone(), l.instance.clone())),
            _ => None,
        })
        .collect()
}

/// Each run anchor: its key and whether its title shows over the run.
fn anchors(items: &[Item]) -> Vec<(String, bool)> {
    items
        .iter()
        .filter_map(|i| match i {
            Item::Title { key, caption, .. } => Some((key.clone(), *caption)),
            Item::Cell { .. } => None,
        })
        .collect()
}

fn one_line(model: &PageModel) -> Vec<Item> {
    let a = arrange(model, Some(0), 519.0, 822.0, |_| 0, &METRICS);
    assert_eq!(a.lines.len(), 1, "the premise: one line");
    items(model, &a.lines[0])
}

/// Whether `one_line`'s line keeps the titles' room.
fn one_room(model: &PageModel) -> bool {
    let all = rooms(model, Sides::both(519.0), 822.0, &METRICS);
    assert_eq!(all.len(), 1, "the premise: one line");
    all[0]
}

#[test]
fn a_groups_label_is_its_title_in_the_colour_of_the_instance_its_strips_share() {
    let band = |name: &str| strip_on("band", name);
    let master = |name: &str| strip_on("master", name);
    let label = |text: &str, instance: Option<&str>| {
        Some(GroupLabel {
            text: text.into(),
            instance: instance.map(str::to_string),
        })
    };
    assert_eq!(
        group_label(&group_of(titled(
            "s",
            Some("STAGE"),
            vec![band("A"), band("B")]
        ))),
        label("STAGE", Some("band"))
    );
    assert_eq!(
        group_label(&group_of(titled("h", Some("Hands 2"), vec![master("A")]))),
        label("Hands 2", Some("master")),
        "the title as written"
    );
    assert_eq!(
        group_label(&group_of(titled(
            "e",
            Some("EFFECTS"),
            vec![band("A"), master("B")]
        ))),
        label("EFFECTS", None),
        "strips of both instances: no instance"
    );
    assert_eq!(
        group_label(&group_of(titled(
            "t",
            Some("BAND"),
            vec![toggle("T"), master("A")]
        ))),
        label("BAND", Some("master")),
        "only the strips count"
    );
    // Untitled (a title of blanks too): the set its strips share, in
    // capitals (F4: the set is named once for the group); none when they
    // differ or there is no strip.
    for title in [None, Some(""), Some("  ")] {
        assert_eq!(
            group_label(&group_of(titled("u", title, vec![band("A"), band("B")]))),
            label("BAND", Some("band")),
            "untitled band: {title:?}"
        );
        assert_eq!(
            group_label(&group_of(titled(
                "u",
                title,
                vec![toggle("T"), master("A")]
            ))),
            label("MASTER", Some("master")),
            "untitled master: {title:?}"
        );
        assert_eq!(
            group_label(&group_of(titled("u", title, vec![band("A"), master("B")]))),
            None,
            "untitled, both sets: {title:?}"
        );
        assert_eq!(
            group_label(&group_of(titled(
                "u",
                title,
                vec![toggle("T"), param_fader("P")]
            ))),
            None,
            "untitled, no strip: {title:?}"
        );
    }
}

#[test]
fn a_run_that_starts_with_a_strip_names_its_group_on_that_strip_and_shows_no_title() {
    let model = PageModel::of(&page(vec![vec![
        titled(
            "s",
            Some("STAGE"),
            vec![
                strip_on("band", "A"),
                strip_on("band", "B"),
                strip_on("band", "C"),
            ],
        ),
        titled(
            "e",
            Some("EFFECTS"),
            vec![strip_on("band", "R"), strip_on("master", "R")],
        ),
        titled(
            "h",
            Some("HANDS"),
            vec![
                strip_on("master", "H1"),
                toggle("T"),
                strip_on("master", "H2"),
            ],
        ),
        titled(
            "u",
            None,
            vec![strip_on("band", "U1"), strip_on("band", "U2")],
        ),
    ]]));
    // Five cells a side: no group is cut by the column.
    let line = one_line(&model);
    // Each run's first strip, nothing else; the untitled group by its set.
    assert_eq!(
        labels(&line),
        vec![
            ("c0-0".into(), "STAGE".into(), Some("band".into())),
            ("c1-0".into(), "EFFECTS".into(), None),
            ("c2-0".into(), "HANDS".into(), Some("master".into())),
            ("c3-0".into(), "BAND".into(), Some("band".into())),
        ]
    );
    // Every run keeps its anchor (the tests find a strip's group by it),
    // with no title over it: the line gives the title's room to its strips.
    assert_eq!(
        anchors(&line),
        vec![
            ("t0-0".into(), false),
            ("t1-0".into(), false),
            ("t2-0".into(), false),
            ("t3-0".into(), false),
        ]
    );
    assert!(!captions(&line));
    assert!(!one_room(&model));
    // An untitled group whose strips differ names nothing.
    let mixed = PageModel::of(&page(vec![vec![titled(
        "m",
        None,
        vec![strip_on("band", "R"), strip_on("master", "R")],
    )]]));
    let line = one_line(&mixed);
    assert_eq!(labels(&line), vec![]);
    // (Cut by the column: a run a side, neither with a title.)
    assert_eq!(
        anchors(&line),
        vec![("t0-0".into(), false), ("t0-1".into(), false)]
    );
}

#[test]
fn a_run_that_does_not_start_with_a_strip_keeps_its_title_and_its_line_the_room() {
    // A parameter fader, a button cell before strips, a block of buttons:
    // each keeps its title over its run, and no strip of it is labelled.
    let model = PageModel::of(&page(vec![vec![
        titled("p", Some("PODKLADY"), vec![param_fader("All")]),
        titled(
            "b",
            Some("BUTTONS"),
            vec![toggle("T"), strip_on("band", "A")],
        ),
        titled(
            "s",
            Some("STAGE"),
            vec![
                strip_on("band", "S0"),
                strip_on("band", "S1"),
                strip_on("band", "S2"),
            ],
        ),
    ]]));
    // Three cells a side: no group is cut by the column.
    let line = one_line(&model);
    assert_eq!(
        anchors(&line),
        vec![
            ("t0-0".into(), true),
            ("t1-0".into(), true),
            ("t2-0".into(), false)
        ]
    );
    assert_eq!(
        labels(&line),
        vec![("c2-0".into(), "STAGE".into(), Some("band".into()))]
    );
    assert!(captions(&line));
    assert!(one_room(&model));
    // A titled block keeps its title; an untitled one has none to show and
    // keeps none: its line gives the room to its faders.
    let block = |title: Option<&str>| {
        PageModel::of(&page(vec![vec![titled(
            "cue",
            title,
            vec![toggle("T0"), toggle("T1")],
        )]]))
    };
    let line = one_line(&block(Some("CUE")));
    assert_eq!(anchors(&line), vec![("t0-0".into(), true)]);
    assert!(matches!(
        line[1],
        Item::Cell {
            cell: Cell::Block { .. },
            label: None,
            ..
        }
    ));
    assert!(captions(&line));
    assert!(one_room(&block(Some("CUE"))));
    let line = one_line(&block(None));
    assert_eq!(anchors(&line), vec![("t0-0".into(), false)]);
    assert!(!captions(&line));
    assert!(!one_room(&block(None)));
    // Untitled runs that start with no strip: a parameter fader shows
    // nothing (no title, no strip: none), a button before a band strip the
    // set's pill (kept). Three cells a side: no group is cut.
    let untitled = PageModel::of(&page(vec![vec![
        titled("p", None, vec![param_fader("All")]),
        titled("b", None, vec![toggle("T"), strip_on("band", "A")]),
        group(
            "s",
            vec![
                strip_on("band", "S0"),
                strip_on("band", "S1"),
                strip_on("band", "S2"),
            ],
        ),
    ]]));
    let line = one_line(&untitled);
    assert_eq!(
        anchors(&line),
        vec![
            ("t0-0".into(), false),
            ("t1-0".into(), true),
            ("t2-0".into(), false)
        ]
    );
    assert_eq!(
        labels(&line),
        vec![("c2-0".into(), "s".into(), Some("band".into()))]
    );
    assert!(one_room(&untitled));
    // A line of blanks only (a pager whose shown sub-page is empty) has no
    // run anchor and no title.
    assert!(!captions(&[Item::Cell {
        cell: Cell::Blank { slot: 0 },
        side: Side::Left,
        label: None
    }]));
    assert!(!captions(&[]));
}

#[test]
fn a_group_cut_into_runs_names_itself_at_the_start_of_each_run() {
    // By the column: STAGE (group 0, titled "stage") on both sides, and the
    // band group's pinned strips on the right.
    let model = PageModel::of(&foh());
    let stage = arrange(&model, Some(0), 519.0, 822.0, |_| 0, &METRICS);
    let line = items(&model, &stage.lines[0]);
    assert_eq!(
        labels(&line),
        vec![
            ("c0-0".into(), "stage".into(), Some("band".into())),
            ("c0-6".into(), "stage".into(), Some("band".into())),
            ("c3-0".into(), "band".into(), Some("band".into())),
        ]
    );
    // The lower row's line: MASTER cut by the column too.
    let lower = items(&model, &stage.lines[1]);
    assert_eq!(
        labels(&lower)
            .into_iter()
            .map(|(key, text, _)| (key, text))
            .collect::<Vec<_>>(),
        vec![
            ("c4-0".into(), "effects".into()),
            ("c5-0".into(), "master".into()),
            ("c5-3".into(), "master".into()),
            ("c6-0".into(), "hands".into()),
        ]
    );
    // By another group's pinned strip in a pager's region: A's strips
    // around B's pin, which keeps its slot (1) on A's sub-page (and by the
    // column after it).
    let region = PageModel::of(&page(vec![vec![pager(vec![
        (
            "a",
            vec![group("a", vec![strip("A0"), strip("A1"), strip("A2")])],
        ),
        ("b", vec![group("b", vec![strip("B0"), pinned("Bp")])]),
    ])]]));
    let line = one_line(&region);
    assert_eq!(
        labels(&line),
        vec![
            ("c0-0".into(), "a".into(), Some("band".into())),
            ("c1-1".into(), "b".into(), Some("band".into())),
            ("c0-1".into(), "a".into(), Some("band".into())),
        ]
    );
    assert_eq!(
        anchors(&line),
        vec![
            ("t0-0".into(), false),
            ("t1-0".into(), false),
            ("t0-1".into(), false)
        ]
    );
    // A low screen's one line of both rows: still each run's first strip.
    let low = arrange(&model, Some(0), 519.0, 400.0, |_| 0, &METRICS);
    assert_eq!(low.lines.len(), 1, "the premise: one line");
    let firsts: Vec<String> = labels(&items(&model, &low.lines[0]))
        .into_iter()
        .map(|(key, _, _)| key)
        .collect();
    let count: usize = [&low.lines[0].left, &low.lines[0].right]
        .iter()
        .map(|half| runs(half).iter().filter(|r| r.group.is_some()).count())
        .sum();
    assert!(count > 1, "the premise: runs on the line");
    assert_eq!(firsts.len(), count, "{firsts:?}");
}

#[test]
fn a_lines_room_is_the_same_on_every_sub_page() {
    // A pager whose second sub-page holds a titled block: the line keeps
    // the title's room on the first sub-page too, where no title shows, so
    // a switch moves no strip.
    let pages = |title: Option<&str>| {
        PageModel::of(&page(vec![vec![
            pager(vec![
                ("a", vec![group("a", vec![strip("A0"), strip("A1")])]),
                (
                    "b",
                    vec![titled("b", title, vec![toggle("T0"), toggle("T1")])],
                ),
            ]),
            group("s", vec![strip("S0"), strip("S1")]),
        ]]))
    };
    let model = pages(Some("CUE"));
    let on_a = arrange(&model, Some(0), 519.0, 822.0, |_| 0, &METRICS);
    let on_b = arrange(&model, Some(1), 519.0, 822.0, |_| 0, &METRICS);
    assert!(!captions(&items(&model, &on_a.lines[0])), "no title on a");
    assert!(
        captions(&items(&model, &on_b.lines[0])),
        "b's block's title"
    );
    // The room: one value for the line, whatever the sub-page shown.
    assert_eq!(
        rooms(&model, Sides::both(519.0), 822.0, &METRICS),
        vec![true]
    );
    // The block untitled: no title anywhere, no room.
    let model = pages(None);
    assert_eq!(
        rooms(&model, Sides::both(519.0), 822.0, &METRICS),
        vec![false]
    );
    // The titled block on the first of three sub-pages: the room on the
    // others too.
    let first = PageModel::of(&page(vec![vec![pager(vec![
        (
            "b",
            vec![titled("b", Some("CUE"), vec![toggle("T0"), toggle("T1")])],
        ),
        ("a", vec![group("a", vec![strip("A0"), strip("A1")])]),
        ("c", vec![group("c", vec![strip("C0")])]),
    ])]]));
    for sub in [Some(1), Some(2)] {
        let a = arrange(&first, sub, 519.0, 822.0, |_| 0, &METRICS);
        assert!(!captions(&items(&first, &a.lines[0])), "{sub:?}: no title");
    }
    assert_eq!(
        rooms(&first, Sides::both(519.0), 822.0, &METRICS),
        vec![true]
    );
}

#[test]
fn each_line_has_its_room_in_the_order_of_the_lines() {
    // Two rows, two lines on a tall side: the upper of strips, the lower
    // with a titled block; one line of both on a low one.
    let model = PageModel::of(&page(vec![
        vec![group("s", vec![strip("S0"), strip("S1")])],
        vec![titled("b", Some("CUE"), vec![toggle("T0"), toggle("T1")])],
    ]));
    assert_eq!(
        rooms(&model, Sides::both(519.0), 822.0, &METRICS),
        vec![false, true]
    );
    assert_eq!(
        rooms(&model, Sides::both(519.0), 400.0, &METRICS),
        vec![true]
    );
    let upper = PageModel::of(&page(vec![
        vec![titled("b", Some("CUE"), vec![toggle("T0"), toggle("T1")])],
        vec![group("s", vec![strip("S0"), strip("S1")])],
    ]));
    assert_eq!(
        rooms(&upper, Sides::both(519.0), 822.0, &METRICS),
        vec![true, false]
    );
    // The FOH shape: strips only on both lines.
    assert_eq!(
        rooms(&PageModel::of(&foh()), Sides::both(519.0), 822.0, &METRICS),
        vec![false, false]
    );
    // The same lines as the arrangement's, which the surface reads them by
    // (the #127 m3 review's m4): two lines, one, and a phone's side.
    for (page, sides, height) in [
        (&model, Sides::both(519.0), 822.0),
        (&model, Sides::both(519.0), 400.0),
        (&upper, Sides::both(519.0), 822.0),
        (
            &PageModel::of(&foh()),
            Sides {
                count: 336.0,
                width: 393.0,
            },
            378.0,
        ),
    ] {
        let lines = arrange_in(page, Some(0), sides, height, |_| 0, &METRICS)
            .lines
            .len();
        assert!(lines > 0, "the premise: lines");
        assert_eq!(rooms(page, sides, height, &METRICS).len(), lines);
    }
}

#[test]
fn a_lines_room_is_counted_on_the_side_the_strips_are_counted_on() {
    // A phone (the #127 m3 review's m1): the strips are counted with the
    // column (`count`) and drawn wider (`width`). Five cells, a button
    // second: on 135 px the window from one step on starts its left run
    // with the button (its title shows); on 519 px all fit, cut after the
    // third, and the button sits inside the first strip's run.
    let model = PageModel::of(&page(vec![vec![group(
        "a",
        vec![
            strip("S0"),
            toggle("T"),
            strip("S1"),
            strip("S2"),
            strip("S3"),
        ],
    )]]));
    let narrow = Sides {
        count: 135.0,
        width: 519.0,
    };
    let wide = Sides {
        count: 519.0,
        width: 135.0,
    };
    assert_eq!(rooms(&model, narrow, 822.0, &METRICS), vec![true]);
    assert_eq!(rooms(&model, wide, 822.0, &METRICS), vec![false]);
    // The premises, on one side for both.
    let window = arrange(&model, None, 135.0, 822.0, |_| 1, &METRICS);
    assert!(
        captions(&items(&model, &window.lines[0])),
        "135 px, one step on"
    );
    let whole = arrange(&model, None, 519.0, 822.0, |_| 0, &METRICS);
    assert_eq!(whole.lines[0].shift, None);
    assert!(!captions(&items(&model, &whole.lines[0])), "519 px");
}

#[test]
fn a_page_is_measured_once_a_side_has_width() {
    assert!(!Sides::both(0.0).measured());
    assert!(!Sides::both(-1.0).measured());
    assert!(Sides::both(0.0_f64.next_up()).measured());
    assert!(Sides::both(135.0).measured());
    // The count decides (a phone's thin strip leaves the width wider).
    assert!(
        !Sides {
            count: 0.0,
            width: 135.0
        }
        .measured()
    );
    assert!(
        Sides {
            count: 135.0,
            width: 0.0
        }
        .measured()
    );
}

#[test]
fn a_line_can_show_each_of_its_pagers_sub_pages() {
    // The pager's row: each of its three sub-pages; a row without one, and
    // two rows on one line with the pager in the second, likewise.
    let model = PageModel::of(&page(vec![
        vec![
            pager(vec![
                ("a", vec![group("a", vec![strip("A")])]),
                ("b", vec![group("b", vec![strip("B")])]),
                ("c", vec![group("c", vec![strip("C")])]),
            ]),
            group("s", vec![strip("S")]),
        ],
        vec![group("t", vec![strip("T")])],
    ]));
    let all = vec![Some(0), Some(1), Some(2)];
    assert_eq!(model.line_subs(&[0]), all);
    assert_eq!(model.line_subs(&[1]), vec![None]);
    assert_eq!(model.line_subs(&[1, 0]), all);
    // A pager with no sub-page shows none.
    let empty = PageModel::of(&page(vec![vec![pager(vec![])]]));
    assert_eq!(empty.line_subs(&[0]), vec![None]);
}

#[test]
fn a_lines_room_is_the_same_in_every_window() {
    // Sides of 135 px hold four cells at 64 px: the group's five cells show
    // in a window. From the start, both runs start with a strip; one step
    // on, the right one starts with the button (its title shows). The line
    // keeps the room in both windows, so an arrow moves no strip.
    let model = PageModel::of(&page(vec![vec![group(
        "a",
        vec![
            strip("S0"),
            strip("S1"),
            strip("S2"),
            toggle("T"),
            strip("S3"),
        ],
    )]]));
    let start = arrange(&model, None, 135.0, 822.0, |_| 0, &METRICS);
    let next = arrange(&model, None, 135.0, 822.0, |_| 1, &METRICS);
    assert_eq!(start.lines[0].shift.map(|s| s.last), Some(1), "the premise");
    assert!(
        !captions(&items(&model, &start.lines[0])),
        "no title from the start"
    );
    assert!(
        captions(&items(&model, &next.lines[0])),
        "the title one step on"
    );
    assert_eq!(
        rooms(&model, Sides::both(135.0), 822.0, &METRICS),
        vec![true],
        "the room, whichever window shows"
    );
    // Strips only: no room in any window.
    let strips = PageModel::of(&page(vec![vec![group(
        "a",
        (0..5).map(|i| strip(&format!("S{i}"))).collect(),
    )]]));
    let a = arrange(&strips, None, 135.0, 822.0, |_| 0, &METRICS);
    assert!(a.lines[0].shift.is_some(), "the premise: a window");
    assert_eq!(
        rooms(&strips, Sides::both(135.0), 822.0, &METRICS),
        vec![false]
    );
}
