use super::*;

const NOW: i64 = 10_000_000_000;

fn day(first_ms: i64, last_ms: i64) -> HistoryDay {
    HistoryDay { first_ms, last_ms }
}

#[test]
fn the_lufs_tab_is_only_for_a_lufs_strip_and_a_single_tab_shows_no_row() {
    assert_eq!(tabs(false), vec![Tab::Eq]);
    assert_eq!(tabs(true), vec![Tab::Eq, Tab::Lufs]);
    assert!(!tab_row(false));
    assert!(tab_row(true));
    assert_eq!(shown_tab(Tab::Lufs, true), Tab::Lufs);
    assert_eq!(shown_tab(Tab::Lufs, false), Tab::Eq);
    assert_eq!(shown_tab(Tab::Eq, true), Tab::Eq);
    assert_eq!(shown_tab(Tab::Eq, false), Tab::Eq);
    assert_eq!((Tab::Eq.name(), Tab::Eq.label()), ("eq", "EQ"));
    assert_eq!((Tab::Lufs.name(), Tab::Lufs.label()), ("lufs", "LUFS"));
}

#[test]
fn the_windows_are_ten_minutes_one_hour_and_three_hours() {
    assert_eq!(
        WINDOWS,
        [("10 MIN", 600_000), ("1 H", 3_600_000), ("3 H", 10_800_000)]
    );
    assert_eq!(DEFAULT_SPAN_MS, 600_000);
    assert_eq!(MAX_SPAN_MS, 10_800_000);
    assert_eq!(MIN_DAY_SPAN_MS, 60_000);
    assert_eq!(LIVE_EDGE_MS, 1000);
    assert_eq!(
        (SECOND_MS, MINUTE_MS, HOUR_MS, DAY_MS),
        (1000, 60_000, 3_600_000, 86_400_000)
    );
}

#[test]
fn a_live_window_ends_now_and_shows_no_now_chip() {
    let v = View::live(600_000);
    assert!(v.is_live());
    assert_eq!(v.window(1_000_000), (400_000, 1_000_000));
    assert!(v.chip_on(600_000));
    assert!(!v.chip_on(3_600_000));
    let back = View {
        span: 600_000,
        end: Some(500_000),
        day: None,
    };
    assert!(!back.is_live());
    assert_eq!(back.window(1_000_000), (-100_000, 500_000));
    let past = View::live(600_000).pick_day(day(1, 2), false);
    assert!(!past.is_live());
    assert!(!past.chip_on(600_000));
}

#[test]
fn a_window_chip_keeps_the_end_and_from_a_day_starts_at_its_first_reading() {
    let live = View::live(600_000).pick_span(3_600_000, NOW);
    assert_eq!(live, View::live(3_600_000));
    let back = View {
        span: 600_000,
        end: Some(500_000),
        day: None,
    }
    .pick_span(10_800_000, NOW);
    assert_eq!(back.end, Some(500_000));
    assert_eq!(back.span, 10_800_000);
    let past = View::live(600_000).pick_day(day(100_000, 5_000_000), false);
    let picked = past.pick_span(3_600_000, NOW);
    assert_eq!(
        picked,
        View {
            span: 3_600_000,
            end: Some(3_700_000),
            day: None
        }
    );
    // A day whose start plus the window reaches now is the live edge.
    assert_eq!(past.pick_span(3_600_000, 3_701_000).end, None);
    assert_eq!(past.pick_span(3_600_000, 3_701_001).end, Some(3_700_000));
}

#[test]
fn now_and_today_go_back_to_the_live_edge_keeping_the_window() {
    let back = View {
        span: 3_600_000,
        end: Some(500_000),
        day: None,
    };
    assert_eq!(back.to_now(), View::live(3_600_000));
    let d = day(100_000, 5_000_000);
    assert_eq!(back.pick_day(d, true), View::live(3_600_000));
    let past = back.pick_day(d, false);
    assert_eq!(
        past,
        View {
            span: 3_600_000,
            end: None,
            day: Some(d)
        }
    );
    assert!(past.day_on(d));
    assert!(!past.day_on(day(1, 2)));
    assert!(!back.day_on(d));
}

#[test]
fn a_past_day_shows_its_whole_span_at_least_a_minute() {
    let v = |first, last| View::live(600_000).pick_day(day(first, last), false);
    assert_eq!(v(100_000, 5_000_000).window(NOW), (100_000, 5_000_000));
    assert_eq!(v(100_000, 100_000).window(NOW), (100_000, 160_000));
    assert_eq!(v(100_000, 159_999).window(NOW), (100_000, 160_000));
    assert_eq!(v(100_000, 160_001).window(NOW), (100_000, 160_001));
}

#[test]
fn the_live_edge_is_within_a_second_of_now() {
    assert_eq!(settle(9_000, 10_000), None);
    assert_eq!(settle(8_999, 10_000), Some(8_999));
    assert_eq!(settle(10_000, 10_000), None);
}

#[test]
fn a_drag_right_goes_back_in_time_and_never_past_now() {
    let live = View::live(600_000);
    // 600 000 ms over 300 px: 2000 ms a px.
    let back = View::panned(live, 10.0, 300.0, 1_000_000);
    assert_eq!(
        back,
        View {
            span: 600_000,
            end: Some(980_000),
            day: None
        }
    );
    assert_eq!(View::panned(live, -10.0, 300.0, 1_000_000), live);
    assert_eq!(
        View::panned(back, -5.0, 300.0, 1_000_000).end,
        Some(990_000)
    );
    // Half a px is 1000 ms: still the live edge; a hair more is not.
    assert_eq!(View::panned(live, 0.5, 300.0, 1_000_000).end, None);
    assert_eq!(
        View::panned(live, 0.5005, 300.0, 1_000_000).end,
        Some(998_999)
    );
    // A width of 0 px counts as 1 px.
    assert_eq!(View::panned(live, 1.0, 0.0, 1_000_000).end, Some(400_000));
}

#[test]
fn a_dragged_day_becomes_a_window_of_its_span_at_most_three_hours() {
    let long = View::live(600_000).pick_day(day(0, 14_400_000), false);
    let v = View::panned(long, 1.0, 300.0, NOW);
    assert_eq!(v.span, 10_800_000);
    assert_eq!(v.day, None);
    // 14 400 000 ms over 300 px: 48 000 ms a px.
    assert_eq!(v.end, Some(14_352_000));
    let short = View::live(600_000).pick_day(day(0, 7_200_000), false);
    assert_eq!(View::panned(short, 0.0, 300.0, NOW).span, 7_200_000);
}

#[test]
fn a_press_is_a_drag_after_six_px_and_a_hold_after_350_ms() {
    let mut p = Press::down(100.0, 0.0);
    assert_eq!(p.x0(), 100.0);
    assert_eq!(p.moved(105.999), Gesture::Still);
    assert_eq!(p.moved(94.001), Gesture::Still);
    assert!(!p.dragged());
    assert_eq!(p.moved(106.0), Gesture::Pan(6.0));
    assert!(p.dragged());
    // Once dragging, every move pans.
    assert_eq!(p.moved(101.0), Gesture::Pan(1.0));
    assert!(!p.check(1000.0));
    assert!(!p.holding());

    let mut left = Press::down(100.0, 0.0);
    assert_eq!(left.moved(94.0), Gesture::Pan(-6.0));

    let mut h = Press::down(100.0, 1000.0);
    assert!(!h.check(1349.999));
    assert!(h.check(1350.0));
    assert!(h.holding());
    assert!(!h.check(2000.0));
    assert_eq!(h.moved(150.0), Gesture::Cursor(150.0));
    assert!(!h.dragged());
    assert_eq!(HOLD_MS, 350.0);
    assert_eq!(DRAG_PX, 6.0);
}

#[test]
fn the_series_gives_a_sample_per_bucket_and_the_tail_keeps_the_window() {
    let s = HistorySeries {
        from: 1000,
        step_ms: 4000,
        values: vec![Some(-230), None, Some(-700)],
    };
    assert_eq!(
        samples(&s),
        vec![(1000, Some(-230)), (5000, None), (9000, Some(-700))]
    );
    assert_eq!(series_end(&s), 13_000);
    assert_eq!(series_end(&HistorySeries::default()), 0);
    let mut tail = vec![(1, Some(-1)), (2, None), (3, Some(-3))];
    keep_tail(&mut tail, 2);
    assert_eq!(tail, vec![(2, None), (3, Some(-3))]);
}

#[test]
fn a_column_is_the_median_above_silence_or_silence_when_half_are_silent() {
    assert_eq!(COLUMNS, 300);
    assert_eq!(column_value(&[]), None);
    assert_eq!(column_value(&[-700]), Some(SILENT_TENTHS));
    assert_eq!(column_value(&[-701, -200]), Some(SILENT_TENTHS));
    assert_eq!(column_value(&[-700, -200, -210]), Some(-200));
    assert_eq!(column_value(&[-250, -200, -210]), Some(-210));
    assert_eq!(column_value(&[-699]), Some(-699));
    assert_eq!(column_value(&[-200, -250]), Some(-200));
}

#[test]
fn samples_fall_in_their_columns_inside_the_window_only() {
    // 600 000 ms in 300 columns: 2000 ms each.
    let samples = vec![
        (-1, Some(-100)),
        (0, Some(-200)),
        (1999, Some(-210)),
        (2000, Some(-300)),
        (4000, None),
        (599_999, Some(-400)),
        (600_000, Some(-500)),
    ];
    let cols = columns(&samples, 0, 600_000);
    assert_eq!(cols.len(), COLUMNS);
    assert_eq!(cols[0], Some(-200));
    assert_eq!(cols[1], Some(-300));
    assert_eq!(cols[2], None);
    assert_eq!(cols[299], Some(-400));
    assert_eq!(cols.iter().flatten().count(), 3);
    // An empty window keeps its columns.
    assert_eq!(columns(&samples, 5, 5).len(), COLUMNS);
}

#[test]
fn an_input_point_shows_its_target_plus_minus_12_lu() {
    let s = scale(Some(-23), &[Some(-100)]);
    assert_eq!(
        s,
        Scale {
            lo: -350,
            hi: -110,
            ticks: vec![-350, -290, -230, -170, -110],
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
        (IN_HALF, OUT_STEP, OUT_MIN, OUT_EMPTY),
        (120, 60, 180, (-300, -120))
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
            ticks: vec![-240, -180, -120, -60],
            target: None,
        }
    );
    assert_eq!(band(&s), None);
    let empty = scale(None, &[None, Some(-700)]);
    assert_eq!((empty.lo, empty.hi), (-300, -120));
    assert_eq!(empty.ticks, vec![-300, -240, -180, -120]);
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
fn the_line_is_green_in_the_band_and_cut_at_silence_and_no_reading() {
    let s = scale(Some(-23), &[]);
    let values = [
        Some(-230),
        Some(-231),
        None,
        Some(-230),
        Some(-250),
        Some(-260),
        Some(-700),
        Some(-240),
    ];
    assert_eq!(
        runs(&values, &s, Some(-23)),
        vec![
            Run {
                green: true,
                points: "1.7,500.0 5.0,504.2".to_string()
            },
            Run {
                green: true,
                points: "11.7,500.0 15.0,583.3".to_string()
            },
            Run {
                green: false,
                points: "15.0,583.3 18.3,625.0".to_string()
            },
        ]
    );
    // An output point is never green; a lone point draws nothing.
    let out = scale(None, &[Some(-230)]);
    let r = runs(&[Some(-230), Some(-231), None, Some(-200)], &out, None);
    assert_eq!(r.len(), 1);
    assert!(!r[0].green);
    assert!(runs(&[Some(-230)], &s, Some(-23)).is_empty());
    assert!(runs(&[None, Some(-700)], &s, Some(-23)).is_empty());
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
    assert!((column_x(0) - 1.6667).abs() < 0.001);
    assert_eq!(column_x(1), 5.0);
    assert!((column_x(299) - 998.333).abs() < 0.001);
    assert_eq!(BOX, 1000.0);
}

#[test]
fn the_time_step_is_calm_with_at_most_seven_ticks() {
    assert_eq!(time_step(600_000), 120_000);
    assert_eq!(time_step(839_999), 120_000);
    assert_eq!(time_step(840_000), 300_000);
    assert_eq!(time_step(419_999), 60_000);
    assert_eq!(time_step(420_000), 120_000);
    assert_eq!(time_step(3_600_000), 600_000);
    assert_eq!(time_step(10_800_000), 1_800_000);
    assert_eq!(time_step(i64::MAX / 8), 720 * 60_000);
    assert_eq!(MAX_TICKS, 7);
    assert_eq!(TICK_MINUTES, [1, 2, 5, 10, 15, 30, 60, 120, 180, 360, 720]);
    for span in (60_000..=86_400_000).step_by(37_000) {
        for t0 in [0, 1, 59_999, 1_234_567] {
            for offset in [0, 3_600_000, 7_200_000, -18_000_000] {
                let n = time_ticks(t0, t0 + span, offset).len();
                assert!(n <= 7, "{span} from {t0}: {n} ticks");
                assert!(n >= 1, "{span} from {t0}: no tick");
            }
        }
    }
}

#[test]
fn time_ticks_fall_on_whole_local_steps() {
    assert_eq!(
        time_ticks(0, 600_000, 0),
        vec![0, 120_000, 240_000, 360_000, 480_000, 600_000]
    );
    assert_eq!(
        time_ticks(1, 600_000, 0),
        vec![120_000, 240_000, 360_000, 480_000, 600_000]
    );
    assert_eq!(time_ticks(0, 600_000, 3_600_000)[0], 0);
    // One minute east of UTC: local whole two minutes.
    assert_eq!(time_ticks(0, 600_000, 60_000)[0], 60_000);
    assert_eq!(time_ticks(-1, 600_000, 0)[0], 0);
    assert_eq!(time_ticks(-120_001, 0, 0)[0], -120_000);
}

#[test]
fn a_time_lies_across_the_box() {
    assert_eq!(x_of(0, 0, 1000), 0.0);
    assert_eq!(x_of(250, 0, 1000), 250.0);
    assert_eq!(x_of(2000, 1000, 5000), 250.0);
    assert_eq!(x_of(1000, 1000, 1000), 0.0);
}

#[test]
fn times_read_as_the_local_clock() {
    assert_eq!(hh_mm(0, 3_600_000), "01:00");
    assert_eq!(hh_mm(3_723_000, 0), "01:02");
    assert_eq!(hh_mm(-1000, 0), "23:59");
    assert_eq!(hh_mm(86_399_000, 0), "23:59");
    assert_eq!(hh_mm_ss(3_723_000, 0), "01:02:03");
    assert_eq!(hh_mm_ss(3_723_999, 3_600_000), "02:02:03");
    assert_eq!(hh_mm_ss(-1, 0), "23:59:59");
}

#[test]
fn a_local_day_turns_at_local_midnight() {
    assert_eq!(local_day(0, 0), 0);
    assert_eq!(local_day(-1, 0), -1);
    assert_eq!(local_day(86_399_999, 0), 0);
    assert_eq!(local_day(86_400_000, 0), 1);
    assert_eq!(local_day(82_800_000, 3_600_000), 1);
    assert_eq!(local_day(82_799_999, 3_600_000), 0);
}

#[test]
fn the_calendar_dates_are_right() {
    assert_eq!(civil(0), (1970, 1, 1));
    assert_eq!(civil(-1), (1969, 12, 31));
    assert_eq!(civil(20_772), (2026, 11, 15));
    assert_eq!(civil(19_782), (2024, 2, 29));
    assert_eq!(civil(11_017), (2000, 3, 1));
    assert_eq!(civil(11_016), (2000, 2, 29));
    assert_eq!(civil(58), (1970, 2, 28));
    assert_eq!(civil(59), (1970, 3, 1));
    assert_eq!(civil(364), (1970, 12, 31));
    assert_eq!(civil(365), (1971, 1, 1));
}

#[test]
fn a_day_chip_says_today_or_its_weekday_and_date() {
    assert_eq!(day_label(20_773, 20_773), "TODAY");
    assert_eq!(day_label(20_772, 20_773), "SUN 15.11");
    assert_eq!(day_label(19_782, 20_773), "THU 29.2");
    assert_eq!(day_label(11_017, 20_773), "WED 1.3");
    assert_eq!(day_label(-1, 0), "WED 31.12");
    assert_eq!(day_label(20_774, 20_773), "TUE 17.11");
}

#[test]
fn the_cursor_reads_the_column_under_the_finger() {
    assert_eq!(cursor_time(0.5, 0, 1000), 500);
    assert_eq!(cursor_time(-1.0, 0, 1000), 0);
    assert_eq!(cursor_time(2.0, 0, 1000), 1000);
    assert_eq!(cursor_time(0.0004, 0, 1000), 0);
    assert_eq!(cursor_time(0.0005, 0, 1000), 1);
    assert_eq!(column_at(0, 0, 600_000), 0);
    assert_eq!(column_at(-5, 0, 600_000), 0);
    assert_eq!(column_at(1999, 0, 600_000), 0);
    assert_eq!(column_at(2000, 0, 600_000), 1);
    assert_eq!(column_at(600_000, 0, 600_000), 299);
    assert_eq!(column_at(900_000, 0, 600_000), 299);
    assert_eq!(column_at(5, 5, 5), 0);
}

#[test]
fn the_cursor_text_is_its_time_and_value() {
    assert_eq!(
        cursor_text(3_723_000, Some(-228), 0),
        "01:02:03 \u{b7} \u{2212}22.8"
    );
    assert_eq!(
        cursor_text(3_723_000, None, 0),
        "01:02:03 \u{b7} no reading"
    );
    assert_eq!(
        cursor_text(0, Some(-700), 0),
        "00:00:00 \u{b7} \u{2212}\u{221e}"
    );
}

#[test]
fn an_input_point_names_its_target() {
    assert_eq!(target_text(Some(-23)).as_deref(), Some("TARGET \u{2212}23"));
    assert_eq!(target_text(Some(-14)).as_deref(), Some("TARGET \u{2212}14"));
    assert_eq!(target_text(Some(0)).as_deref(), Some("TARGET 0"));
    assert_eq!(target_text(Some(5)).as_deref(), Some("TARGET 5"));
    assert_eq!(
        target_text(Some(-128)).as_deref(),
        Some("TARGET \u{2212}128")
    );
    assert_eq!(target_text(None), None);
}

#[test]
fn the_api_urls_encode_the_point() {
    assert_eq!(
        history_url("band", "live_set tracks 1", 1000, 2000),
        "/api/lufs/history?instance=band&target=live_set%20tracks%201&from=1000&to=2000"
    );
    assert_eq!(
        days_url("band", "live_set tracks 1"),
        "/api/lufs/days?instance=band&target=live_set%20tracks%201"
    );
}

#[test]
fn the_api_answers_read_as_the_series_or_why_not() {
    let ok = series_answer(200, r#"{"from":1000,"step_ms":1000,"values":[-230,null]}"#);
    assert_eq!(
        ok,
        Ok(HistorySeries {
            from: 1000,
            step_ms: 1000,
            values: vec![Some(-230), None]
        })
    );
    assert!(
        series_answer(200, "{")
            .unwrap_err()
            .starts_with("unreadable history")
    );
    assert_eq!(
        series_answer(400, r#"{"code":"BAD_WINDOW","message":"too long"}"#),
        Err("HTTP 400: too long".to_string())
    );
    assert_eq!(
        days_answer(200, r#"{"days":[{"first_ms":1,"last_ms":2}]}"#),
        Ok(vec![day(1, 2)])
    );
    assert!(
        days_answer(200, "[")
            .unwrap_err()
            .starts_with("unreadable days")
    );
    assert_eq!(
        days_answer(403, " nope "),
        Err("HTTP 403: nope".to_string())
    );
}
