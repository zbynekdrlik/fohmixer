use super::*;

/// The test pictures' field: the dark of Pro-Q's top bar around its ↶
/// (luma 32, under [`GREY`]).
const DARK: u8 = 0x20;

/// A picture `width` × `height` px of the dark field.
fn field(width: u32, height: u32) -> Pixels {
    Pixels {
        width,
        height,
        bgra: [DARK, DARK, DARK, 0xFF].repeat((width * height) as usize),
    }
}

/// `pixels` with the rectangle `columns` × `rows` painted `bgra`.
fn painted(
    mut pixels: Pixels,
    columns: RangeInclusive<i32>,
    rows: RangeInclusive<i32>,
    bgra: [u8; 4],
) -> Pixels {
    let stride = pixels.width as usize;
    for row in rows {
        for column in columns.clone() {
            let at = (row as usize * stride + column as usize) * 4;
            pixels.bgra[at..at + 4].copy_from_slice(&bgra);
        }
    }
    pixels
}

/// A grey pixel of brightness `luma` (blue, green and red equal).
fn grey(luma: u8) -> [u8; 4] {
    [luma, luma, luma, 0xFF]
}

/// A 1349 px wide picture (the ↶'s spot at x 570) with a run `width`
/// columns wide whose centre (rounded) is `centre`, in the ↶'s rows (15 to
/// 21), of brightness `luma`.
fn glyph(centre: i32, width: i32, luma: u8) -> Pixels {
    let first = centre - width / 2;
    painted(
        field(1349, 40),
        first..=first + width - 1,
        15..=21,
        grey(luma),
    )
}

#[test]
fn the_measured_table_and_the_reads_figures() {
    assert_eq!(TABLE.len(), 14);
    assert_eq!(TABLE[0], (760, 311.5));
    assert_eq!(TABLE[10], (1349, 569.5));
    assert_eq!(TABLE[13], (2400, 1095.5));
    assert!(TABLE.is_sorted_by_key(|(width, _)| *width));
    assert_eq!((Y, ROWS, REACH, SLOP), (18, (12, 24), 16, 4));
    assert_eq!((LIT, GREY, WIDTHS, GAP), (140, 70, (8, 16), 1));
}

#[test]
fn the_spot_is_the_measured_one_at_each_point_and_linear_between() {
    // Each point exactly, x rounded half away from zero.
    let points = [
        (760, 312),
        (780, 314),
        (800, 315),
        (820, 317),
        (840, 318),
        (860, 324),
        (880, 334),
        (900, 344),
        (1100, 446),
        (1291, 541),
        (1349, 570),
        (1535, 663),
        (1810, 801),
        (2400, 1096),
    ];
    for (width, x) in points {
        assert_eq!(spot(width), Some((x, 18)), "{width}");
    }
    // Between: 311.5 + 2 × 10 / 20 = 312.5; 343.5 + 102 × 100 / 200 =
    // 394.5; 445.5 + 95 × 100 / 191 = 495.24; 662.5 + 138 × 165 / 275 =
    // 745.3; 800.5 + 295 × 190 / 590 = 895.5.
    for (width, x) in [
        (770, 313),
        (1000, 395),
        (1200, 495),
        (1700, 745),
        (2000, 896),
    ] {
        assert_eq!(spot(width), Some((x, 18)), "{width}");
    }
    // Outside the table: no spot.
    assert_eq!(spot(759), None);
    assert_eq!(spot(2401), None);
    assert_eq!(spot(0), None);
    assert_eq!(spot(u32::MAX), None);
}

#[test]
fn a_pixels_luma_weighs_red_green_and_blue() {
    // Blue, green, red, alpha.
    assert_eq!(luma(&[0, 0, 255, 0xFF]), 76, "red: 299 × 255 / 1000");
    assert_eq!(luma(&[0, 255, 0, 0xFF]), 149, "green: 587 × 255 / 1000");
    assert_eq!(luma(&[255, 0, 0, 0xFF]), 29, "blue: 114 × 255 / 1000");
    assert_eq!(luma(&[255, 255, 255, 0]), 255, "alpha is not read");
    assert_eq!(
        luma(&[10, 20, 30, 0xFF]),
        21,
        "(8970 + 11740 + 1140) / 1000"
    );
    assert_eq!(luma(&grey(240)), 240);
    assert_eq!(luma(&[0, 0, 0, 0xFF]), 0);
    // The pixel the slice starts with.
    assert_eq!(luma(&[0, 0, 255, 0xFF, 255, 255, 255, 0xFF]), 76);
}

#[test]
fn the_read_looks_at_its_reach_and_rows_within_the_picture() {
    assert_eq!(reach(1349, 570), 554..=586);
    assert_eq!(reach(760, 312), 296..=328);
    // Within the picture: from its first column to its last.
    assert_eq!(reach(20, 3), 0..=19);
    assert_eq!(reach(20, 10), 0..=19);
    assert_eq!(reach(40, 30), 14..=39);
    assert_eq!(rows(809), 12..=24);
    assert_eq!(rows(25), 12..=24);
    assert_eq!(rows(20), 12..=19);
    assert!(rows(12).is_empty(), "no row to read");
}

#[test]
fn a_run_bridges_one_column_below_the_threshold_and_no_more() {
    let columns = [
        (10, 0),
        (11, 200),
        (12, 139),
        (13, 140),
        (14, 0),
        (15, 0),
        (16, 255),
        (17, 255),
        (18, 0),
        (19, 0),
        (20, 0),
        (21, 150),
    ];
    assert_eq!(runs(&columns, 140), [(11, 13), (16, 17), (21, 21)]);
    // At 141 the column of 140 is below it too: 11 stands alone.
    assert_eq!(runs(&columns, 141), [(11, 11), (16, 17), (21, 21)]);
    assert_eq!(runs(&columns, 201), [(16, 17)]);
    assert_eq!(runs(&columns, 0), [(10, 21)]);
    assert!(runs(&[], 140).is_empty());
}

#[test]
fn a_lit_undo_reads_lit_at_its_own_centre_and_a_grey_one_grey() {
    assert_eq!(read(&glyph(570, 12, 240)), Undo::Lit(570, 18));
    assert_eq!(read(&glyph(570, 12, LIT)), Undo::Lit(570, 18), "at 140");
    assert_eq!(read(&glyph(570, 12, LIT - 1)), Undo::Grey, "at 139");
    assert_eq!(read(&glyph(570, 12, 100)), Undo::Grey);
    assert_eq!(read(&glyph(570, 12, GREY)), Undo::Grey, "at 70");
    assert_eq!(read(&glyph(570, 12, GREY - 1)), Undo::Missing, "at 69");
    assert_eq!(read(&field(1349, 40)), Undo::Missing, "the bar alone");
    // A lit ↶ split by a thinner column is still lit.
    let split = painted(glyph(570, 12, 240), 569..=569, 15..=21, grey(180));
    assert_eq!(read(&split), Undo::Lit(570, 18));
    // At the table's ends, and outside it.
    let at = |width: u32, x: i32| painted(field(width, 40), x - 6..=x + 5, 15..=21, grey(240));
    assert_eq!(read(&at(760, 312)), Undo::Lit(312, 18));
    assert_eq!(read(&at(2400, 1096)), Undo::Lit(1096, 18));
    assert_eq!(read(&at(759, 312)), Undo::Missing, "no spot at 759 px");
    assert_eq!(read(&at(2401, 1096)), Undo::Missing, "no spot at 2401 px");
}

#[test]
fn only_a_run_as_wide_as_the_undo_reads() {
    for (luma, state) in [(240, Undo::Lit(570, 18)), (100, Undo::Grey)] {
        assert_eq!(read(&glyph(570, 7, luma)), Undo::Missing, "7 at {luma}");
        assert_eq!(read(&glyph(570, 8, luma)), state, "8 at {luma}");
        assert_eq!(read(&glyph(570, 16, luma)), state, "16 at {luma}");
        assert_eq!(read(&glyph(570, 17, luma)), Undo::Missing, "17 at {luma}");
    }
}

#[test]
fn only_a_run_centred_within_four_px_of_the_spot_reads() {
    // The tap goes to the run's own centre, not to the table's spot.
    assert_eq!(read(&glyph(574, 12, 240)), Undo::Lit(574, 18));
    assert_eq!(read(&glyph(566, 12, 240)), Undo::Lit(566, 18));
    assert_eq!(read(&glyph(575, 12, 240)), Undo::Missing);
    assert_eq!(read(&glyph(565, 12, 240)), Undo::Missing);
    assert_eq!(read(&glyph(574, 12, 100)), Undo::Grey);
    assert_eq!(read(&glyph(575, 12, 100)), Undo::Missing);
    // A narrow run before it hides nothing.
    let stray = painted(glyph(570, 12, 240), 556..=558, 15..=21, grey(240));
    assert_eq!(read(&stray), Undo::Lit(570, 18));
}

#[test]
fn a_one_column_gap_is_bridged_and_a_two_column_gap_splits() {
    let one = painted(glyph(570, 12, 240), 569..=569, 12..=24, grey(DARK));
    assert_eq!(read(&one), Undo::Lit(570, 18));
    let one = painted(glyph(570, 12, 100), 569..=569, 12..=24, grey(DARK));
    assert_eq!(read(&one), Undo::Grey);
    // Two halves of 5 px: neither is as wide as the ↶.
    let two = painted(glyph(570, 12, 240), 569..=570, 12..=24, grey(DARK));
    assert_eq!(read(&two), Undo::Missing);
}

#[test]
fn only_the_undos_rows_and_reach_are_read() {
    let row = |y: i32| painted(field(1349, 40), 564..=575, y..=y, grey(240));
    assert_eq!(read(&row(11)), Undo::Missing);
    assert_eq!(read(&row(12)), Undo::Lit(570, 18));
    assert_eq!(read(&row(24)), Undo::Lit(570, 18));
    assert_eq!(read(&row(25)), Undo::Missing);
    // Past the reach (586): never read.
    let past = painted(field(1349, 40), 587..=598, 15..=21, grey(240));
    assert_eq!(read(&past), Undo::Missing);
    // A picture shorter than the rows: the rows it has.
    let short = painted(field(1349, 20), 564..=575, 15..=19, grey(240));
    assert_eq!(read(&short), Undo::Lit(570, 18));
    let none = Pixels {
        width: 1349,
        height: 12,
        bgra: vec![0xFF; 1349 * 12 * 4],
    };
    assert_eq!(read(&none), Undo::Missing, "no row to read");
}

#[test]
fn each_colour_is_weighed() {
    let coloured = |bgra: [u8; 4]| painted(field(1349, 40), 564..=575, 15..=21, bgra);
    assert_eq!(
        read(&coloured([255, 0, 0, 0xFF])),
        Undo::Missing,
        "blue: 29"
    );
    assert_eq!(read(&coloured([0, 0, 255, 0xFF])), Undo::Grey, "red: 76");
    assert_eq!(
        read(&coloured([0, 255, 0, 0xFF])),
        Undo::Lit(570, 18),
        "green: 149"
    );
}
