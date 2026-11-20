use super::*;

/// 2026-11-15 10:00:00 UTC.
const T0: i64 = 1_794_736_800_000;

fn point(instance: &str, target: &str) -> Point {
    (instance.to_string(), target.to_string())
}

fn vox() -> Point {
    point("band", "live_set tracks 1")
}

fn pa() -> Point {
    point("master", "live_set master_track")
}

/// The samples of `items` (each point's reading), no fader known.
fn at(items: &[(Point, Option<i16>)]) -> Vec<Sample> {
    items
        .iter()
        .map(|(point, tenths)| Sample {
            point: point.clone(),
            tenths: *tenths,
            fader: None,
        })
        .collect()
}

/// A sample of `point` with its reading and fader.
fn with_fader(point: Point, tenths: Option<i16>, fader: Option<i16>) -> Sample {
    Sample {
        point,
        tenths,
        fader,
    }
}

#[test]
fn the_constants() {
    assert_eq!(RING_S, 3600);
    assert_eq!(HOLE, -32768);
    assert_eq!(MINUTE_S, 60);
    assert_eq!(MINUTE_MS, 60_000);
    assert_eq!(MAX_VALUES, 900);
    assert_eq!(MAX_SPAN_MS, 86_400_000);
    assert_eq!(T0 % 60_000, 0, "T0 is a whole minute");
}

#[test]
fn seconds_and_minutes_floor_the_clock() {
    assert_eq!(second_of(T0 + 999), T0);
    assert_eq!(second_of(T0 + 1000), T0 + 1000);
    assert_eq!(second_of(-1), -1000);
    assert_eq!(minute_of(T0 + 59_999), T0);
    assert_eq!(minute_of(T0 + 60_000), T0 + 60_000);
    assert_eq!(minute_of(-1), -60_000);
}

#[test]
fn a_slot_keeps_silence_as_silence_and_none_as_a_hole() {
    assert_eq!(slot_of(None), HOLE);
    assert_eq!(slot_of(Some(-231)), -231);
    assert_eq!(slot_of(Some(-700)), -700);
    assert_eq!(slot_of(Some(-950)), -700);
    assert!(sounds(-699));
    assert!(!sounds(-700));
    assert!(!sounds(HOLE));
    assert!(heard(-700));
    assert!(!heard(HOLE));
}

#[test]
fn slots_round_trip_through_base64_and_a_wrong_length_is_refused() {
    let slots: Vec<i16> = (0..60).map(|i| -300 + i).collect();
    let text = encode(&slots);
    assert_eq!(decode(&text), Some(slots));
    // 60 slots of two bytes: 120 bytes, 160 base64 characters.
    assert_eq!(text.len(), 160);
    assert_eq!(encode(&[1, -2]), "AQD+/w==");
    assert_eq!(decode(&encode(&[0; 59])), None);
    assert_eq!(decode(&encode(&[0; 61])), None);
    assert_eq!(decode("not base64!"), None);
}

#[test]
fn a_minute_line_reads_back_and_names_its_points() {
    let line = MinuteLine {
        m: T0,
        points: vec![PointMinute {
            instance: "band".into(),
            target: "live_set tracks 1".into(),
            values: encode(&[-230; 60]),
            faders: None,
        }],
    };
    let text = line.text();
    assert!(
        text.starts_with(r#"{"m":1794736800000,"p":[{"i":"band","t":"live_set tracks 1","v":""#),
        "{text}"
    );
    let back = MinuteLine::parse(&text).unwrap();
    assert_eq!(back, line);
    assert_eq!(
        back.slots("band", "live_set tracks 1"),
        Some(vec![-230; 60])
    );
    assert_eq!(back.slots("band", "live_set tracks 2"), None);
    assert_eq!(back.slots("master", "live_set tracks 1"), None);
    assert_eq!(MinuteLine::parse("{\"m\": 1"), None);
}

#[test]
fn a_span_is_the_first_and_last_second_heard() {
    let mut slots = vec![HOLE; 60];
    assert_eq!(span_of(T0, &slots), None);
    slots[3] = -700;
    slots[41] = -200;
    assert_eq!(span_of(T0, &slots), Some((T0 + 3000, T0 + 41_000)));
}

#[test]
fn each_second_is_sampled_once_and_a_clock_step_back_samples_nothing() {
    let mut r = Recorder::new();
    assert!(r.due(T0));
    assert_eq!(r.sample(T0 + 400, &at(&[(vox(), Some(-230))])), None);
    assert!(!r.due(T0 + 999));
    assert!(r.due(T0 + 1000));
    // The same second again and an earlier one change nothing.
    r.sample(T0 + 900, &at(&[(vox(), Some(-100))]));
    r.sample(T0 - 5000, &at(&[(vox(), Some(-100))]));
    assert_eq!(r.at(&vox(), T0), -230);
    assert_eq!(r.at(&vox(), T0 - 1000), HOLE);
    assert_eq!(r.at(&vox(), T0 + 1000), HOLE, "not sampled yet");
    assert_eq!(r.at(&pa(), T0), HOLE, "never served");
}

#[test]
fn a_missed_second_is_a_hole_and_a_point_not_served_gets_holes() {
    let mut r = Recorder::new();
    r.sample(T0, &at(&[(vox(), Some(-230)), (pa(), Some(-180))]));
    // Two seconds later, PA no longer served.
    r.sample(T0 + 2000, &at(&[(vox(), Some(-240))]));
    assert_eq!(r.at(&vox(), T0), -230);
    assert_eq!(r.at(&vox(), T0 + 1000), HOLE);
    assert_eq!(r.at(&vox(), T0 + 2000), -240);
    assert_eq!(r.at(&pa(), T0), -180);
    assert_eq!(r.at(&pa(), T0 + 2000), HOLE);
    assert_eq!(r.points(), 2, "PA's hour still holds a reading");
}

#[test]
fn the_gap_holes_are_bounded_by_the_hour() {
    assert_eq!(gap_holes(T0, T0 + 1000), 0);
    assert_eq!(gap_holes(T0, T0 + 2000), 1);
    assert_eq!(gap_holes(T0, T0 + 3_601_000), 3600);
    assert_eq!(gap_holes(T0, T0 + 9_000_000), 3600);
}

#[test]
fn the_hour_keeps_the_last_3600_seconds_and_lets_a_silent_point_go() {
    let mut r = Recorder::new();
    r.sample(T0, &at(&[(vox(), Some(-230)), (pa(), Some(-180))]));
    for s in 1..3600 {
        r.sample(T0 + s * 1000, &at(&[(vox(), Some(-230))]));
    }
    assert_eq!(r.at(&vox(), T0), -230, "3600 seconds back is still there");
    assert_eq!(r.at(&pa(), T0), -180);
    assert_eq!(r.covers_from(), Some(T0));
    r.sample(T0 + 3_600_000, &at(&[(vox(), Some(-230))]));
    assert_eq!(r.at(&vox(), T0), HOLE, "the oldest second left the hour");
    assert_eq!(r.at(&vox(), T0 + 1000), -230);
    assert_eq!(r.points(), 1, "PA's hour holds nothing any more");
    assert_eq!(r.covers_from(), Some(T0 + 1000));
}

#[test]
fn a_long_gap_leaves_only_holes_before_the_new_second() {
    let mut r = Recorder::new();
    r.sample(T0, &at(&[(vox(), Some(-230))]));
    r.sample(T0 + 7_200_000, &at(&[(vox(), Some(-210))]));
    assert_eq!(r.at(&vox(), T0 + 7_200_000), -210);
    assert_eq!(r.at(&vox(), T0 + 7_199_000), HOLE);
    assert_eq!(r.at(&vox(), T0 + 3_601_000), HOLE);
    assert_eq!(r.covers_from(), Some(T0 + 3_601_000));
    assert_eq!(Recorder::new().covers_from(), None);
}

#[test]
fn the_minute_line_comes_when_the_minute_ends_with_only_the_points_that_sounded() {
    let mut r = Recorder::new();
    for s in 0..60 {
        let line = r.sample(
            T0 + s * 1000,
            &at(&[(vox(), Some(-230 + s as i16)), (pa(), Some(-700))]),
        );
        assert_eq!(line, None, "second {s}");
    }
    assert_eq!(r.current().unwrap().m, T0);
    let line = r
        .sample(T0 + 60_000, &at(&[(vox(), Some(-100)), (pa(), Some(-700))]))
        .unwrap();
    assert_eq!(line.m, T0);
    assert_eq!(line.points.len(), 1, "PA was silent the whole minute");
    let slots = line.slots("band", "live_set tracks 1").unwrap();
    assert_eq!(slots[0], -230);
    assert_eq!(slots[59], -171);
    assert_eq!(r.current().unwrap().m, T0 + 60_000);
}

#[test]
fn a_gap_over_a_minute_writes_the_minute_before_it_with_its_holes() {
    let mut r = Recorder::new();
    r.sample(T0 + 10_000, &at(&[(vox(), Some(-230))]));
    let line = r
        .sample(T0 + 300_000, &at(&[(vox(), Some(-230))]))
        .expect("the minute of T0 ended");
    let slots = line.slots("band", "live_set tracks 1").unwrap();
    assert_eq!(slots[10], -230);
    assert_eq!(slots[9], HOLE);
    assert_eq!(slots[11], HOLE);
}

#[test]
fn an_idle_minute_is_no_line() {
    let mut r = Recorder::new();
    for s in 0..=60 {
        assert_eq!(r.sample(T0 + s * 1000, &at(&[(vox(), None)])), None);
    }
    assert_eq!(r.current(), None);
    assert_eq!(r.line(T0), None);
}

#[test]
fn a_window_is_whole_seconds_up_to_24_h() {
    assert_eq!(window(T0, T0 + 600_000), Ok((T0, 600)));
    assert_eq!(window(T0 + 500, T0 + 1500), Ok((T0, 2)));
    assert_eq!(window(T0, T0 + 1), Ok((T0, 1)));
    assert_eq!(window(T0, T0 + 86_400_000), Ok((T0, 86_400)));
    assert_eq!(window(T0, T0 + 86_400_001), Err(WindowError::TooLong));
    assert_eq!(window(T0, T0), Err(WindowError::Empty));
    assert_eq!(window(T0, T0 - 1), Err(WindowError::Empty));
    assert_eq!(
        WindowError::Empty.text(),
        "the window ends before it starts"
    );
    assert_eq!(
        WindowError::TooLong.text(),
        "the window is longer than 24 h"
    );
}

#[test]
fn a_window_whose_arithmetic_would_overflow_is_refused() {
    // PR #223's review: `to - from` wrapped negative, passed the 24 h check
    // and made a vector of the wrapped length; a start near the top made
    // `start + n * 1000` overflow.
    for (from, to) in [
        (i64::MIN, 0),
        (i64::MIN, i64::MIN + 1),
        (9_223_372_036_854_775_000, 9_223_372_036_854_775_800),
        (i64::MAX - 1000, i64::MAX),
    ] {
        assert!(window(from, to).is_err(), "{from}..{to}");
    }
    // The last whole second whose end still fits.
    let top = i64::MAX - i64::MAX.rem_euclid(1000) - 1000;
    assert_eq!(window(top, top + 1000), Ok((top, 1)));
}

#[test]
fn a_bucket_holds_at_most_900_values() {
    assert_eq!(step_for(0), 1);
    assert_eq!(step_for(600), 1);
    assert_eq!(step_for(900), 1);
    assert_eq!(step_for(901), 2);
    assert_eq!(step_for(3600), 4);
    assert_eq!(step_for(10_800), 12);
    assert_eq!(step_for(86_400), 96);
}

#[test]
fn a_bucket_is_the_gated_loudness_above_silence_silence_from_half_and_none_empty() {
    assert_eq!(bucket_value(&[]), None);
    assert_eq!(bucket_value(&[HOLE, HOLE]), None);
    assert_eq!(bucket_value(&[-230]), Some(-230));
    // The gated loudness of the sounding ones (#253, the page's column
    // rule: their energy mean here, nothing 10 LU under it).
    assert_eq!(bucket_value(&[-200, -240, -220, HOLE]), Some(-217));
    assert_eq!(bucket_value(&[-200, -240, -220, -210]), Some(-215));
    // One silent of three heard: the others' gated loudness.
    assert_eq!(bucket_value(&[-700, -200, -240, HOLE]), Some(-216));
    // Half silent: silence.
    assert_eq!(bucket_value(&[-700, -200, HOLE, HOLE]), Some(-700));
}

#[test]
fn a_window_is_bucketed_from_its_start() {
    let slots: Vec<i16> = (0..1800).map(|i| -300 + (i % 4) as i16).collect();
    let series = bucket(T0, &slots, &slots);
    assert_eq!(series.from, T0);
    assert_eq!(series.step_ms, 2000);
    assert_eq!(series.values.len(), 900);
    assert_eq!(series.values[0], Some(-299));
    assert_eq!(series.values[1], Some(-297));
    // The fader's buckets beside them, by the median of the same seconds.
    assert_eq!(series.fader.len(), 900);
    assert_eq!(series.fader[0], Some(-299));
    assert_eq!(series.fader[1], Some(-297));
    let short = bucket(T0, &[-230, HOLE, -700], &[-45, -40, HOLE]);
    assert_eq!(short.step_ms, 1000);
    assert_eq!(short.values, vec![Some(-230), None, Some(-700)]);
    assert_eq!(short.fader, vec![Some(-45), Some(-40), None]);
}

#[test]
fn a_window_past_the_clocks_range_says_so() {
    assert_eq!(window(i64::MIN, i64::MIN + 1), Err(WindowError::OutOfRange));
    assert_eq!(
        window(i64::MAX - 1000, i64::MAX),
        Err(WindowError::OutOfRange)
    );
    // A span that wraps is longer than 24 h.
    assert_eq!(window(i64::MIN, 0), Err(WindowError::TooLong));
    assert_eq!(WindowError::OutOfRange.text(), "the window is out of range");
}

#[test]
fn a_fader_slot_is_its_tenths_and_none_a_hole() {
    assert_eq!(fader_slot(None), HOLE);
    assert_eq!(fader_slot(Some(-45)), -45);
    assert_eq!(fader_slot(Some(-700)), -700);
    assert_eq!(fader_slot(Some(60)), 60);
    let s = with_fader(vox(), Some(-950), Some(-45));
    assert_eq!(slots_of(&s), (-700, -45));
    assert_eq!(slots_of(&with_fader(vox(), None, None)), (HOLE, HOLE));
}

#[test]
fn a_minutes_fader_is_written_only_when_it_knows_one() {
    assert_eq!(faders_text(&[HOLE; 60]), None);
    let mut faders = vec![HOLE; 60];
    faders[59] = -700;
    assert_eq!(faders_text(&faders), Some(encode(&faders)));
}

#[test]
fn a_buckets_fader_is_the_median_of_its_known_seconds() {
    assert_eq!(fader_value(&[HOLE, HOLE]), None);
    assert_eq!(fader_value(&[]), None);
    assert_eq!(fader_value(&[-45, HOLE, -40]), Some(-40));
    assert_eq!(fader_value(&[-45, -100, -40]), Some(-45));
    // −∞ (the floor) is a value, never a hole.
    assert_eq!(fader_value(&[-700, HOLE, HOLE]), Some(-700));
}

#[test]
fn the_fader_is_kept_beside_the_reading_each_second_and_written_as_f() {
    let mut r = Recorder::new();
    for s in 0..60 {
        let fader = (s < 30).then_some(-45 + s as i16);
        let line = r.sample(
            T0 + s * 1000,
            &[
                with_fader(vox(), Some(-230), fader),
                with_fader(pa(), Some(-180), None),
            ],
        );
        assert_eq!(line, None, "second {s}");
    }
    assert_eq!(r.fader_at(&vox(), T0), -45);
    assert_eq!(r.fader_at(&vox(), T0 + 29_000), -16);
    assert_eq!(r.fader_at(&vox(), T0 + 30_000), HOLE);
    assert_eq!(r.at(&vox(), T0 + 30_000), -230, "the reading goes on");
    assert_eq!(
        r.fader_at(&vox(), T0 + 60_000),
        HOLE,
        "after the last second"
    );
    assert_eq!(r.fader_at(&point("band", "nothing"), T0), HOLE);
    let line = r
        .sample(T0 + 60_000, &[with_fader(vox(), Some(-230), Some(-45))])
        .unwrap();
    let faders = line.fader_slots("band", "live_set tracks 1").unwrap();
    assert_eq!(faders[0], -45);
    assert_eq!(faders[29], -16);
    assert_eq!(faders[30], HOLE);
    assert_eq!(
        line.slots("band", "live_set tracks 1"),
        Some(vec![-230; 60])
    );
    // PA's minute knew no fader: no `f`, and none reads back.
    let pa_minute = line.points.iter().find(|p| p.instance == "master").unwrap();
    assert_eq!(pa_minute.faders, None);
    assert_eq!(line.fader_slots("master", "live_set master_track"), None);
    assert_eq!(line.fader_slots("band", "live_set tracks 2"), None);
    let text = line.text();
    assert!(
        text.contains(r#""v":""#) && text.matches(r#""f":""#).count() == 1,
        "{text}"
    );
    assert_eq!(MinuteLine::parse(&text), Some(line));
    // A point the hub no longer serves gets fader holes too.
    r.sample(T0 + 61_000, &[]);
    assert_eq!(r.fader_at(&vox(), T0 + 61_000), HOLE);
    assert_eq!(r.fader_at(&vox(), T0 + 60_000), -45);
}

#[test]
fn an_older_hubs_line_reads_as_no_fader() {
    let old = format!(
        r#"{{"m":{T0},"p":[{{"i":"band","t":"live_set tracks 1","v":"{}"}}]}}"#,
        encode(&[-230; 60])
    );
    let line = MinuteLine::parse(&old).unwrap();
    assert_eq!(line.points[0].faders, None);
    assert_eq!(line.fader_slots("band", "live_set tracks 1"), None);
    assert_eq!(
        line.slots("band", "live_set tracks 1"),
        Some(vec![-230; 60])
    );
    // An `f` that does not decode reads as no fader, the reading kept.
    let bad = old.replace(r#""}]}"#, r#"","f":"AAA"}]}"#);
    let line = MinuteLine::parse(&bad).unwrap();
    assert_eq!(line.fader_slots("band", "live_set tracks 1"), None);
    assert_eq!(
        line.slots("band", "live_set tracks 1"),
        Some(vec![-230; 60])
    );
}
