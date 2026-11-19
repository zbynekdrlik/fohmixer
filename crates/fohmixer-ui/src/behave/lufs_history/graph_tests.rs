use super::*;

/// A sample at `t` with the reading `v` and no fader.
fn at(t: i64, v: Option<i16>) -> Sample {
    Sample { t, v, f: None }
}

/// A column with the reading `v` and the fader `f`.
fn col(v: Option<i16>, f: Option<i16>) -> Column {
    Column { v, f }
}

/// The readings of `cols`.
fn vs(cols: &[Column]) -> Vec<Option<i16>> {
    cols.iter().map(|c| c.v).collect()
}

#[test]
fn the_series_gives_a_sample_per_bucket_with_its_fader_and_the_tail_keeps_the_window() {
    let s = HistorySeries {
        from: 1000,
        step_ms: 4000,
        values: vec![Some(-230), None, Some(-700)],
        fader: vec![Some(-45), Some(-40), None],
    };
    assert_eq!(
        samples(&s),
        vec![
            Sample {
                t: 1000,
                v: Some(-230),
                f: Some(-45)
            },
            Sample {
                t: 5000,
                v: None,
                f: Some(-40)
            },
            Sample {
                t: 9000,
                v: Some(-700),
                f: None
            },
        ]
    );
    // An older hub's series: no fader.
    let old = HistorySeries {
        from: 1000,
        step_ms: 4000,
        values: vec![Some(-230), None, Some(-700)],
        fader: Vec::new(),
    };
    assert!(samples(&old).iter().all(|x| x.f.is_none()));
    assert_eq!(samples(&old)[0].v, Some(-230));
    assert_eq!(series_end(&s), 13_000);
    assert_eq!(series_end(&HistorySeries::default()), 0);
    let mut tail = vec![at(1, Some(-1)), at(2, None), at(3, Some(-3))];
    keep_tail(&mut tail, 2);
    assert_eq!(tail, vec![at(2, None), at(3, Some(-3))]);
}

#[test]
fn a_window_has_120_columns_or_one_a_second_when_shorter() {
    assert_eq!(COLUMNS, 120);
    assert_eq!(columns_for(600_000), 120);
    assert_eq!(columns_for(120_000), 120);
    assert_eq!(columns_for(119_999), 119);
    assert_eq!(columns_for(60_000), 60);
    assert_eq!(columns_for(999), 1);
    assert_eq!(columns_for(0), 1);
}

#[test]
fn a_column_takes_the_hubs_bucket_rule_a_breath_gated_out() {
    // Sung at −20 with a breath at −34 in one column (5 s in 10 MIN): the
    // breath is gated out; half silent is silence; none without a reading.
    let sung: Vec<Sample> = [-200, -200, -200, -340]
        .iter()
        .enumerate()
        .map(|(i, v)| at(i as i64 * 1000, Some(*v)))
        .collect();
    assert_eq!(columns(&sung, 0, 600_000)[0].v, Some(-200));
    let half = [at(0, Some(-700)), at(1000, Some(-200))];
    assert_eq!(columns(&half, 0, 600_000)[0].v, Some(-700));
    assert_eq!(columns(&half, 0, 600_000)[1].v, None);
}

#[test]
fn a_column_takes_its_readings_and_its_faders_median() {
    let cols = columns(
        &[
            Sample {
                t: 0,
                v: Some(-700),
                f: Some(-40),
            },
            Sample {
                t: 1,
                v: Some(-200),
                f: None,
            },
            Sample {
                t: 2,
                v: Some(-200),
                f: Some(-50),
            },
            Sample {
                t: 3,
                v: None,
                f: Some(-45),
            },
            at(5000, Some(-250)),
        ],
        0,
        600_000,
    );
    assert_eq!(cols[0], col(Some(-200), Some(-45)));
    assert_eq!(cols[1], col(Some(-250), None));
    assert_eq!(cols[2], col(None, None));
}

#[test]
fn a_short_past_day_draws_one_line() {
    // A minute of readings a second apart: 60 columns, one line.
    let samples: Vec<Sample> = (0..60).map(|i| at(i * 1000, Some(-230))).collect();
    let cols = drawn_columns(&samples, 0, 60_000);
    assert_eq!(cols.len(), 60);
    assert!(cols.iter().all(|c| c.v == Some(-230)));
    let l = lines(&vs(&cols), &scale(Some(-23), &[]));
    assert_eq!(l.len(), 1);
    assert_eq!(l[0].matches('C').count(), 59);
    assert!(l[0].starts_with("M8.3,500.0C"), "{}", l[0]);
}

#[test]
fn samples_fall_in_their_columns_inside_the_window_only() {
    // 600 000 ms in 120 columns: 5000 ms each.
    let samples = vec![
        at(-1, Some(-100)),
        at(0, Some(-200)),
        at(4999, Some(-200)),
        at(5000, Some(-300)),
        at(10_000, None),
        at(599_999, Some(-400)),
        at(600_000, Some(-500)),
    ];
    let cols = columns(&samples, 0, 600_000);
    assert_eq!(cols.len(), COLUMNS);
    assert_eq!(cols[0].v, Some(-200));
    assert_eq!(cols[1].v, Some(-300));
    assert_eq!(cols[2].v, None);
    assert_eq!(cols[119].v, Some(-400));
    assert_eq!(cols.iter().filter(|c| c.v.is_some()).count(), 3);
    // An empty window has one column.
    assert_eq!(columns(&samples, 5, 5).len(), 1);
}

#[test]
fn the_glide_averages_one_two_one_and_never_across_a_gap() {
    assert_eq!(
        glide(&[Some(-200), Some(-240), Some(-200), Some(-260)]),
        // −200: (2·−200 − 240) / 3 = −213.3; −240: (−200 − 480 − 200) / 4
        // = −220; −200: (−240 − 400 − 260) / 4 = −225; −260: (−200 − 520)
        // / 3 = −240.
        vec![Some(-213), Some(-220), Some(-225), Some(-240)]
    );
    // A gap and silence cut it: the neighbours across them are left out,
    // and they stay as they are.
    assert_eq!(
        glide(&[Some(-200), None, Some(-230), Some(-700), Some(-210)]),
        vec![Some(-200), None, Some(-230), Some(-700), Some(-210)]
    );
    assert_eq!(
        glide(&[Some(-200), Some(-210)]),
        vec![Some(-203), Some(-207)]
    );
    assert_eq!(glide(&[]), Vec::<Option<i16>>::new());
    // Rounded to the nearest tenth: (−200·2 − 201) / 3 = −200.33.
    assert_eq!(glide(&[Some(-200), Some(-201)])[0], Some(-200));
    assert_eq!(glide(&[Some(-200), Some(-202)])[0], Some(-201));
}

#[test]
fn the_drawn_columns_are_glided_and_keep_their_faders() {
    let samples = vec![
        Sample {
            t: 0,
            v: Some(-200),
            f: Some(-30),
        },
        at(5000, Some(-240)),
    ];
    let cols = drawn_columns(&samples, 0, 600_000);
    assert_eq!(cols[0], col(Some(-213), Some(-30)));
    assert_eq!(cols[1], col(Some(-227), None));
    assert_eq!(cols[2], col(None, None));
}

#[test]
fn an_input_point_shows_its_target_plus_minus_12_lu() {
    let s = scale(Some(-23), &[Some(-100)]);
    assert_eq!(
        s,
        Scale {
            lo: -350,
            hi: -110,
            ticks: vec![-350, -320, -290, -260, -230, -200, -170, -140, -110],
            target: Some(-230),
        }
    );
    assert_eq!(y_of(-230, &s), 500.0);
    assert_eq!(y_of(-110, &s), 0.0);
    assert_eq!(y_of(-350, &s), 1000.0);
    // Past an edge: on the edge.
    assert_eq!(y_of(-50, &s), 0.0);
    assert_eq!(y_of(-700, &s), 1000.0);
    assert_eq!(y_of(-170, &s), 250.0);
    let (top, bottom) = band(&s).unwrap();
    assert!((top - 458.333).abs() < 0.001, "{top}");
    assert!((bottom - 541.667).abs() < 0.001, "{bottom}");
    assert_eq!(
        (IN_HALF, TICK_STEP, OUT_STEP, OUT_MIN, OUT_EMPTY),
        (120, 30, 60, 180, (-300, -120))
    );
}

#[test]
fn an_output_point_fits_its_readings_in_6_lu_steps_at_least_18_lu() {
    assert_eq!(out_range(-300, -60), (-300, -60));
    assert_eq!(out_range(-300, -120), (-300, -120));
    assert_eq!(out_range(-301, -120), (-360, -120));
    assert_eq!(out_range(-300, -119), (-300, -60));
    assert_eq!(out_range(-170, -170), (-240, -60));
    assert_eq!(out_range(-200, -130), (-300, -120));
    assert_eq!(out_range(-180, -180), (-300, -120));
    assert_eq!(out_range(-185, -175), (-300, -120));
    let s = scale(None, &[Some(-170), None, Some(-700), Some(-800)]);
    assert_eq!(
        s,
        Scale {
            lo: -240,
            hi: -60,
            ticks: vec![-240, -210, -180, -150, -120, -90, -60],
            target: None,
        }
    );
    assert_eq!(band(&s), None);
    let empty = scale(None, &[None, Some(-700)]);
    assert_eq!((empty.lo, empty.hi), (-300, -120));
    assert_eq!(empty.ticks, vec![-300, -270, -240, -210, -180, -150, -120]);
}

#[test]
fn every_output_range_holds_its_readings_and_is_whole_steps() {
    for min in -720..=0 {
        for width in [0, 1, 59, 60, 61, 119, 120, 179, 180, 181, 300] {
            let max = min + width;
            let (lo, hi) = out_range(min, max);
            assert!(lo <= min && max <= hi, "{min}..{max} in {lo}..{hi}");
            assert_eq!(lo.rem_euclid(60), 0);
            assert_eq!(hi.rem_euclid(60), 0);
            assert!(hi - lo >= 180);
        }
    }
}

#[test]
fn tick_labels_are_whole_lu_with_a_typographic_minus() {
    assert_eq!(tick_label(-230), "\u{2212}23");
    assert_eq!(tick_label(-120), "\u{2212}12");
    assert_eq!(tick_label(0), "0");
    assert_eq!(tick_label(60), "6");
}

#[test]
fn a_curve_passes_its_points_and_never_swings_past_them() {
    assert_eq!(curve(&[]), "");
    assert_eq!(curve(&[(1.0, 2.0)]), "");
    // Two points: a straight cubic.
    assert_eq!(
        curve(&[(0.0, 0.0), (30.0, 30.0)]),
        "M0.0,0.0C10.0,10.0 20.0,20.0 30.0,30.0"
    );
    // A peak: its tangent is flat (the slopes change sign), the ends take
    // their own slope.
    assert_eq!(
        curve(&[(0.0, 100.0), (30.0, 40.0), (60.0, 100.0)]),
        "M0.0,100.0C10.0,80.0 20.0,40.0 30.0,40.0C40.0,40.0 50.0,80.0 60.0,100.0"
    );
    // A rise: the middle tangent is the slopes' mean (1 and 3: 2).
    assert_eq!(
        curve(&[(0.0, 0.0), (30.0, 30.0), (60.0, 120.0)]),
        "M0.0,0.0C10.0,10.0 20.0,10.0 30.0,30.0C40.0,50.0 50.0,90.0 60.0,120.0"
    );
    // A rise into a flat stretch: the middle tangent of the slopes 1 and
    // 0 is 0 (they do not keep their sign), and the rise's scaled one is
    // 3.0 (5.5 against the slope 1, hypot 5.5, scaled by 3 / 5.5).
    assert_eq!(
        curve(&[
            (0.0, 0.0),
            (100.0, 1000.0),
            (200.0, 1100.0),
            (300.0, 1100.0)
        ]),
        "M0.0,0.0C33.3,333.3 66.7,900.0 100.0,1000.0C133.3,1100.0 166.7,1100.0 200.0,1100.0C233.3,1100.0 266.7,1100.0 300.0,1100.0"
    );
    // A flat stretch stays flat.
    assert_eq!(
        curve(&[(0.0, 50.0), (30.0, 50.0), (60.0, 50.0)]),
        "M0.0,50.0C10.0,50.0 20.0,50.0 30.0,50.0C40.0,50.0 50.0,50.0 60.0,50.0"
    );
    // A step: the tangents past a radius of 3 are scaled back onto it, so
    // the curve never overshoots (slopes 0.1 then 10: tangents 0.1, 5.05,
    // 10; on the first segment 1 and 50.5 against its slope, scaled by
    // 3 / 50.51 to 0.006 and 0.300; the second, 0.03 and 1, stays).
    let steep = curve(&[(0.0, 0.0), (10.0, 1.0), (11.0, 11.0)]);
    assert_eq!(
        steep,
        "M0.0,0.0C3.3,0.0 6.7,0.0 10.0,1.0C10.3,1.1 10.7,7.7 11.0,11.0"
    );
    // A slope of 1 between two of 9: (1 + 9) / 2 = 5 on each side of the
    // middle segment: hypot(5, 5) = 7.07 > 3, scaled by 3 / 7.07.
    let scaled = curve(&[(0.0, 0.0), (1.0, 9.0), (2.0, 10.0), (3.0, 19.0)]);
    assert_eq!(
        scaled,
        "M0.0,0.0C0.3,3.0 0.7,8.3 1.0,9.0C1.3,9.7 1.7,9.3 2.0,10.0C2.3,10.7 2.7,16.0 3.0,19.0"
    );
}

#[test]
fn a_monotone_curves_control_points_stay_between_their_ends() {
    // Every segment's control points lie between its two values, whatever
    // the data: the curve never swings past its points.
    let ys = [
        0.0, 900.0, 880.0, 10.0, 12.0, 500.0, 499.0, 999.0, 0.0, 0.0, 5.0,
    ];
    let pts: Vec<(f64, f64)> = ys
        .iter()
        .enumerate()
        .map(|(i, y)| (i as f64 * 8.0, *y))
        .collect();
    let path = curve(&pts);
    let segs: Vec<&str> = path.split('C').skip(1).collect();
    assert_eq!(segs.len(), ys.len() - 1);
    for (i, seg) in segs.iter().enumerate() {
        let nums: Vec<f64> = seg.split([' ', ',']).map(|n| n.parse().unwrap()).collect();
        let (lo, hi) = (ys[i].min(ys[i + 1]) - 0.05, ys[i].max(ys[i + 1]) + 0.05);
        for y in [nums[1], nums[3]] {
            assert!(lo <= y && y <= hi, "segment {i}: {y} outside {lo}..{hi}");
        }
    }
}

#[test]
fn the_line_is_a_curve_per_run_cut_at_silence_and_no_reading() {
    let s = scale(Some(-23), &[]);
    let mut values = vec![None; COLUMNS];
    values[..8].copy_from_slice(&[
        Some(-230),
        Some(-230),
        None,
        Some(-230),
        Some(-200),
        Some(-200),
        Some(-700),
        Some(-240),
    ]);
    // 120 columns: column i's middle at (i + 0.5) · 8.333.
    assert_eq!(
        lines(&values, &s),
        vec![
            "M4.2,500.0C6.9,500.0 9.7,500.0 12.5,500.0".to_string(),
            "M29.2,500.0C31.9,458.3 34.7,375.0 37.5,375.0C40.3,375.0 43.1,375.0 45.8,375.0"
                .to_string(),
        ]
    );
    // A lone point draws nothing; neither does silence.
    assert!(lines(&[Some(-230)], &s).is_empty());
    assert!(lines(&[None, Some(-700)], &s).is_empty());
    assert!(lines(&[Some(-230), None, Some(-231)], &s).is_empty());
    // A run at the window's end is drawn too.
    assert_eq!(lines(&[None, Some(-230), Some(-230)], &s).len(), 1);
}

#[test]
fn the_dot_sits_at_the_last_column_that_sounds() {
    assert_eq!(live_end(true, &[]), None);
    assert_eq!(live_end(true, &[None, Some(-700)]), None);
    assert_eq!(
        live_end(true, &[Some(-200), Some(-230), Some(-700), None]),
        Some((1, -230))
    );
    assert_eq!(
        live_end(true, &[Some(-200), None, Some(-210)]),
        Some((2, -210))
    );
    // A window dragged back or a past day: no dot.
    assert_eq!(live_end(false, &[Some(-200), None, Some(-210)]), None);
}

#[test]
fn the_fader_scale_puts_0_db_on_the_target_or_the_middle_1_db_as_1_lu() {
    let input = scale(Some(-23), &[]);
    assert_eq!(fader_zero(&input), -230);
    assert_eq!(fader_y(0, &input), 500.0);
    // −3 dB sits on the −26 LU line, +12 dB on the top.
    assert_eq!(fader_y(-30, &input), y_of(-260, &input));
    assert_eq!(fader_y(120, &input), 0.0);
    // −∞ and anything past the scale at its edge.
    assert_eq!(fader_y(-700, &input), 1000.0);
    assert_eq!(fader_y(200, &input), 0.0);
    let labels: Vec<String> = input
        .ticks
        .iter()
        .map(|t| fader_label(*t, &input))
        .collect();
    assert_eq!(
        labels,
        [
            "\u{2212}12",
            "\u{2212}9",
            "\u{2212}6",
            "\u{2212}3",
            "0",
            "+3",
            "+6",
            "+9",
            "+12"
        ]
        .map(String::from)
    );
    // An output point: the scale's middle, a whole LU.
    let output = scale(None, &[Some(-170)]);
    assert_eq!((output.lo, output.hi), (-240, -60));
    assert_eq!(fader_zero(&output), -150);
    assert_eq!(fader_label(-150, &output), "0");
    assert_eq!(fader_label(-60, &output), "+9");
    assert_eq!(fader_label(-240, &output), "\u{2212}9");
    let wide = Scale {
        lo: -360,
        hi: -120,
        ticks: Vec::new(),
        target: None,
    };
    assert_eq!(fader_zero(&wide), -240);
}

#[test]
fn the_fader_line_runs_where_the_fader_is_known() {
    let s = scale(Some(-23), &[]);
    let cols = [
        col(Some(-230), Some(0)),
        col(None, Some(-30)),
        col(None, None),
        col(None, Some(30)),
        col(None, Some(30)),
        col(None, Some(-700)),
        col(None, None),
        col(None, Some(0)),
    ];
    // 8 columns: middles at 62.5, 187.5, ...
    assert_eq!(
        fader_lines(&cols, &s),
        vec![
            "62.5,500.0 187.5,625.0".to_string(),
            "437.5,375.0 562.5,375.0 687.5,1000.0".to_string(),
        ]
    );
    assert!(fader_lines(&[col(None, Some(0))], &s).is_empty());
    assert!(fader_lines(&[], &s).is_empty());
}

#[test]
fn a_faders_db_reads_with_one_decimal_and_its_sign() {
    assert_eq!(db_text(-45), "\u{2212}4.5 dB");
    assert_eq!(db_text(30), "+3.0 dB");
    assert_eq!(db_text(0), "0.0 dB");
    assert_eq!(db_text(-5), "\u{2212}0.5 dB");
    assert_eq!(db_text(60), "+6.0 dB");
    assert_eq!(db_text(-699), "\u{2212}69.9 dB");
    assert_eq!(db_text(-700), "\u{2212}\u{221e} dB");
    assert_eq!(db_text(-701), "\u{2212}\u{221e} dB");
    assert_eq!(
        fader_text(Some(-45)).as_deref(),
        Some("FADER \u{2212}4.5 dB")
    );
    assert_eq!(fader_text(None), None);
}

#[test]
fn green_is_within_one_lu_of_an_input_points_target() {
    assert!(green(-220, Some(-23)));
    assert!(green(-240, Some(-23)));
    assert!(!green(-219, Some(-23)));
    assert!(!green(-241, Some(-23)));
    assert!(!green(-230, None));
}

#[test]
fn columns_sit_at_their_middles() {
    assert!((column_x(0, 300) - 1.6667).abs() < 0.001);
    assert_eq!(column_x(1, 300), 5.0);
    assert!((column_x(299, 300) - 998.333).abs() < 0.001);
    assert_eq!(column_x(0, 1), 500.0);
    assert_eq!(column_x(0, 0), 500.0);
    assert_eq!(column_x(3, 4), 875.0);
    assert_eq!(BOX, 1000.0);
}
