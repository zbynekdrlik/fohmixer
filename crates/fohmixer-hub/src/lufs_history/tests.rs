use super::*;
use crate::rules::lufs::history::{encode, minute_of};
use fohmixer_proto::lufs::SILENT_TENTHS;

/// 2026-11-15 10:00:00 UTC.
const T0: i64 = 1_794_736_800_000;
const VOX: &str = "live_set tracks 1";

fn item(instance: &str, target: &str, tenths: Option<i16>) -> Sample {
    with_fader(instance, target, tenths, None)
}

fn with_fader(instance: &str, target: &str, tenths: Option<i16>, fader: Option<i16>) -> Sample {
    Sample {
        point: (instance.into(), target.into()),
        tenths,
        fader,
    }
}

fn date(y: i32, m: u8, d: u8) -> UtcDate {
    time::Date::from_calendar_date(y, time::Month::try_from(m).unwrap(), d).unwrap()
}

/// One minute line of `band`'s VOX at `m` with `slots`.
fn line(m: i64, slots: &[i16]) -> String {
    MinuteLine {
        m,
        points: vec![crate::rules::lufs::history::PointMinute {
            instance: "band".into(),
            target: VOX.into(),
            values: encode(slots),
            faders: None,
        }],
    }
    .text()
}

#[test]
fn the_limits_are_fourteen_days_32_mb_and_a_queue_of_64() {
    assert_eq!(KEEP_DAYS, 14);
    assert_eq!(DAY_CAP, 33_554_432);
    assert_eq!(QUEUE, 64);
    assert_eq!(FOLDER, "lufs");
    assert_eq!(STEM, "lufs");
    assert_eq!(
        LIMITS,
        Limits {
            keep_days: 14,
            day_cap: 33_554_432
        }
    );
}

#[test]
fn a_minute_goes_to_the_writer_once_it_ended_and_a_full_queue_counts_it_dropped() {
    let (history, rx) = LufsHistory::channel(None, 1);
    let mut asked = 0;
    for s in 0..=120 {
        history.sample(T0 + s * 1000 + 300, || {
            asked += 1;
            vec![item("band", VOX, Some(-230))]
        });
        // The same second again asks nothing.
        history.sample(T0 + s * 1000 + 600, || {
            asked += 1;
            Vec::new()
        });
    }
    assert_eq!(asked, 121, "once a second");
    // Two minutes ended; the queue holds one.
    let first = rx.try_recv().unwrap();
    assert_eq!(first.m, T0);
    assert!(rx.try_recv().is_err());
    assert_eq!(
        history.status(),
        LufsHistoryStatus {
            points: 1,
            dropped: 1
        }
    );
}

#[test]
fn the_last_hour_is_read_from_ram_bucketed() {
    let history = LufsHistory::off();
    for s in 0..600 {
        let tenths = if s < 300 { Some(-230) } else { Some(-800) };
        history.sample(T0 + s * 1000, || vec![item("band", VOX, tenths)]);
    }
    let series = history.history("band", VOX, T0, T0 + 600_000).unwrap();
    assert_eq!(series.from, T0);
    assert_eq!(series.step_ms, 1000);
    assert_eq!(series.values.len(), 600);
    assert_eq!(series.values[0], Some(-230));
    assert_eq!(series.values[299], Some(-230));
    assert_eq!(series.values[300], Some(SILENT_TENTHS));
    // Another point and a window past the last second: no readings.
    let other = history
        .history("band", "live_set tracks 2", T0, T0 + 60_000)
        .unwrap();
    assert!(other.values.iter().all(Option::is_none));
    let later = history
        .history("band", VOX, T0 + 600_000, T0 + 660_000)
        .unwrap();
    assert!(later.values.iter().all(Option::is_none));
    assert_eq!(
        history.history("band", VOX, T0, T0).unwrap_err(),
        WindowError::Empty
    );
}

#[test]
fn an_older_window_is_read_from_the_day_files_and_ram_answers_its_own_hour() {
    let dir = tempfile::tempdir().unwrap();
    let day = date(2026, 11, 15);
    // Yesterday 23:59 and today 00:00 UTC, two files.
    let midnight = day.midnight().assume_utc().unix_timestamp() * 1000;
    let mut slots = vec![-250_i16; 60];
    slots[0] = HOLE;
    std::fs::write(
        dir.path().join("lufs-2026-11-14.jsonl"),
        format!("{}\n", line(midnight - 60_000, &slots)),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("lufs-2026-11-15.jsonl"),
        format!("{}\n{}\n", "not a line", line(T0 - 120_000, &[-240; 60])),
    )
    .unwrap();
    let (history, _rx) = LufsHistory::channel(Some(dir.path().to_path_buf()), 8);
    // Across midnight: both files.
    let night = history
        .history("band", VOX, midnight - 60_000, midnight + 60_000)
        .unwrap();
    assert_eq!(night.values.len(), 120);
    assert_eq!(night.values[0], None);
    assert_eq!(night.values[1], Some(-250));
    assert_eq!(night.values[59], Some(-250));
    assert_eq!(night.values[60], None);
    // RAM from T0 on: the file answers the minutes before it.
    history.sample(T0, || vec![item("band", VOX, Some(-200))]);
    let both = history
        .history("band", VOX, T0 - 120_000, T0 + 1000)
        .unwrap();
    assert_eq!(both.values.len(), 121);
    assert_eq!(both.values[0], Some(-240));
    assert_eq!(both.values[59], Some(-240));
    assert_eq!(both.values[60], None);
    assert_eq!(both.values[120], Some(-200));
}

#[test]
fn a_window_carries_the_fader_from_the_day_files_and_from_ram_beside_the_readings() {
    let dir = tempfile::tempdir().unwrap();
    // Two minutes before T0 in the file: one with `f`, one an older hub's.
    let mut faders = vec![-45_i16; 60];
    faders[1] = HOLE;
    let with_f = MinuteLine {
        m: T0 - 120_000,
        points: vec![crate::rules::lufs::history::PointMinute {
            instance: "band".into(),
            target: VOX.into(),
            values: encode(&[-240; 60]),
            faders: Some(encode(&faders)),
        }],
    }
    .text();
    std::fs::write(
        dir.path().join("lufs-2026-11-15.jsonl"),
        format!("{with_f}\n{}\n", line(T0 - 60_000, &[-230; 60])),
    )
    .unwrap();
    let (history, _rx) = LufsHistory::channel(Some(dir.path().to_path_buf()), 8);
    history.sample(T0, || vec![with_fader("band", VOX, Some(-200), Some(-100))]);
    let series = history
        .history("band", VOX, T0 - 120_000, T0 + 1000)
        .unwrap();
    assert_eq!(series.step_ms, 1000);
    assert_eq!(series.fader.len(), 121);
    assert_eq!(series.fader[0], Some(-45));
    assert_eq!(series.fader[1], None);
    assert_eq!(series.fader[59], Some(-45));
    assert_eq!(series.values[59], Some(-240));
    // The older hub's minute: readings, no fader.
    assert_eq!(series.values[60], Some(-230));
    assert_eq!(series.fader[60], None);
    assert_eq!(series.fader[119], None);
    // RAM's second.
    assert_eq!(series.values[120], Some(-200));
    assert_eq!(series.fader[120], Some(-100));
}

#[test]
fn a_window_whose_arithmetic_would_overflow_is_a_window_error_never_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let (history, _rx) = LufsHistory::channel(Some(dir.path().to_path_buf()), 8);
    history.sample(T0, || vec![item("band", VOX, Some(-230))]);
    for (from, to) in [
        (i64::MIN, 0),
        (9_223_372_036_854_775_000, 9_223_372_036_854_775_800),
    ] {
        assert!(
            history.history("band", VOX, from, to).is_err(),
            "{from}..{to}"
        );
    }
}

#[test]
fn a_bad_line_in_a_day_file_is_skipped_and_the_lines_after_it_are_read() {
    let dir = tempfile::tempdir().unwrap();
    let m = T0 - 7_200_000;
    let mut bytes = format!("{}\n", line(m, &[-250; 60])).into_bytes();
    // A line that is no UTF-8 (a torn write), then a line that is no JSON.
    bytes.extend_from_slice(b"{\"m\":\xff\xfe\n");
    bytes.extend_from_slice(b"not a line\n");
    bytes.extend_from_slice(format!("{}\n", line(m + 60_000, &[-240; 60])).as_bytes());
    std::fs::write(dir.path().join("lufs-2026-11-15.jsonl"), bytes).unwrap();
    let (history, _rx) = LufsHistory::channel(Some(dir.path().to_path_buf()), 8);
    let series = history.history("band", VOX, m, m + 120_000).unwrap();
    assert_eq!(series.values[0], Some(-250));
    assert_eq!(series.values[60], Some(-240), "the line after the bad ones");
    assert_eq!(
        history.days("band", VOX),
        [HistoryDay {
            first_ms: m,
            last_ms: m + 119_000,
        }]
    );
}

#[test]
fn a_day_files_bad_lines_are_noted_once_with_their_count() {
    assert_eq!(bad_lines_note("lufs-2026-11-15.jsonl", 0), None);
    assert_eq!(
        bad_lines_note("lufs-2026-11-15.jsonl", 1).as_deref(),
        Some("LUFS history: skipped 1 unreadable line in lufs-2026-11-15.jsonl")
    );
    assert_eq!(
        bad_lines_note("lufs-2026-11-15.jsonl", 3).as_deref(),
        Some("LUFS history: skipped 3 unreadable lines in lufs-2026-11-15.jsonl")
    );
}

#[test]
fn the_files_answer_up_to_the_first_second_ram_holds() {
    assert_eq!(files_until(None, T0), T0);
    assert_eq!(files_until(Some(T0 - 1000), T0), T0 - 1000);
    assert_eq!(files_until(Some(T0 + 1000), T0), T0);
}

#[test]
fn a_minute_is_placed_only_within_the_window() {
    let mut slots = vec![HOLE; 3];
    let values: Vec<i16> = (0..60).map(|i| -300 + i).collect();
    // The window is seconds 58, 59 and 60 of the minute: the last two.
    place(T0, &values, T0 + 58_000, T0 + 61_000, &mut slots);
    assert_eq!(slots, [-242, -241, HOLE]);
    let mut slots = vec![HOLE; 2];
    place(T0, &values, T0 - 1000, T0 + 1000, &mut slots);
    assert_eq!(slots, [HOLE, -300]);
    let mut slots = vec![HOLE; 2];
    place(T0, &values, T0, T0 + 1000, &mut slots);
    assert_eq!(slots, [-300, HOLE], "the end is not included");
}

#[test]
fn the_days_hold_only_the_points_spans_newest_first_with_the_unwritten_minute() {
    let dir = tempfile::tempdir().unwrap();
    let mut slots = vec![HOLE; 60];
    slots[5] = -230;
    slots[40] = -700;
    let sunday = (date(2026, 11, 8).midnight().assume_utc().unix_timestamp() + 9 * 3600) * 1000;
    std::fs::write(
        dir.path().join("lufs-2026-11-08.jsonl"),
        format!(
            "{}\n{}\n",
            line(sunday, &slots),
            line(sunday + 3_600_000, &[-240; 60])
        ),
    )
    .unwrap();
    // A day file without this point, and another stem's file.
    std::fs::write(
        dir.path().join("lufs-2026-11-10.jsonl"),
        MinuteLine {
            m: sunday + 2 * 86_400_000,
            points: Vec::new(),
        }
        .text()
            + "\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("events-2026-11-12.jsonl"),
        line(sunday, &slots),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("lufs-2026-11-15.jsonl"),
        format!("{}\n", line(T0 - 60_000, &[-240; 60])),
    )
    .unwrap();
    let (history, _rx) = LufsHistory::channel(Some(dir.path().to_path_buf()), 8);
    history.sample(T0 + 7000, || vec![item("band", VOX, Some(-200))]);
    assert_eq!(
        history.days("band", VOX),
        [
            HistoryDay {
                first_ms: T0 - 60_000,
                last_ms: T0 + 7000,
            },
            HistoryDay {
                first_ms: sunday + 5000,
                last_ms: sunday + 3_600_000 + 59_000,
            },
        ]
    );
    assert!(history.days("band", "live_set tracks 9").is_empty());
    assert!(LufsHistory::off().days("band", VOX).is_empty());
}

#[test]
fn a_day_widens_to_every_span() {
    let one = widen(None, (10, 20));
    assert_eq!(
        one,
        HistoryDay {
            first_ms: 10,
            last_ms: 20
        }
    );
    assert_eq!(
        widen(Some(one), (5, 15)),
        HistoryDay {
            first_ms: 5,
            last_ms: 20
        }
    );
    assert_eq!(
        widen(Some(one), (12, 30)),
        HistoryDay {
            first_ms: 10,
            last_ms: 30
        }
    );
    let mut days = Vec::new();
    add_span(&mut days, date(2026, 11, 1), (10, 20));
    add_span(&mut days, date(2026, 11, 2), (100, 200));
    add_span(&mut days, date(2026, 11, 1), (30, 40));
    assert_eq!(
        days,
        [
            (date(2026, 11, 1), widen(None, (10, 40))),
            (date(2026, 11, 2), widen(None, (100, 200))),
        ]
    );
}

#[test]
fn only_the_line_that_reaches_the_cap_makes_a_record() {
    let day = date(2026, 11, 15);
    assert_eq!(
        cap_record(Appended::CapReached, day, 32),
        Some(json!({"what": "cap", "date": "2026-11-15", "cap_bytes": 32}))
    );
    for other in [Appended::Written, Appended::Capped, Appended::Failed] {
        assert_eq!(cap_record(other, day, 32), None, "{other:?}");
    }
    assert!(crate::events::is_warn(&json!({"ev": "lufs_history"})));
}

#[test]
fn the_writer_thread_writes_each_minute_stops_at_the_cap_and_sweeps_at_start() {
    let dir = tempfile::tempdir().unwrap();
    let lufs = dir.path().join("lufs");
    std::fs::create_dir_all(&lufs).unwrap();
    let old = lufs.join("lufs-2020-01-01.jsonl");
    std::fs::write(&old, "x\n").unwrap();
    let (events, records) = EventLog::channel(8);
    // Room for one minute line of one point (a ~240 byte line).
    let limits = Limits {
        keep_days: 14,
        day_cap: 300,
    };
    let (history, writer) = LufsHistory::start_with(lufs.clone(), limits, events).unwrap();
    for s in 0..=180 {
        history.sample(T0 + s * 1000, || vec![item("band", VOX, Some(-230))]);
    }
    drop(history);
    writer.join().unwrap();
    assert!(!old.exists(), "swept at start");
    let text = std::fs::read_to_string(lufs.join("lufs-2026-11-15.jsonl")).unwrap();
    let lines: Vec<MinuteLine> = text.lines().filter_map(MinuteLine::parse).collect();
    assert_eq!(lines.len(), 1, "{text}");
    assert_eq!(lines[0].m, minute_of(T0));
    assert_eq!(lines[0].slots("band", VOX), Some(vec![-230; 60]));
    let record = records.try_recv().unwrap();
    assert_eq!(record["ev"], "lufs_history");
    assert_eq!(record["what"], "cap");
    assert_eq!(record["date"], "2026-11-15");
    assert!(records.try_recv().is_err(), "one record a day");
}

#[test]
fn start_writes_into_the_data_folders_lufs() {
    let dir = tempfile::tempdir().unwrap();
    let history = LufsHistory::start(dir.path(), EventLog::off()).unwrap();
    assert_eq!(
        history.dir.as_deref(),
        Some(dir.path().join("lufs").as_path())
    );
    assert_eq!(LufsHistory::default().dir, None);
}

#[test]
fn a_day_file_reports_how_many_lines_it_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lufs-2026-11-15.jsonl");
    let mut bytes = format!("{}\n", line(T0, &[-250; 60])).into_bytes();
    bytes.extend_from_slice(b"\xff\n{\"m\":\n");
    bytes.extend_from_slice(format!("{}\n", line(T0 + 60_000, &[-240; 60])).as_bytes());
    std::fs::write(&path, bytes).unwrap();
    let mut read = Vec::new();
    assert_eq!(each_line(&path, |l| read.push(l.m)), 2);
    assert_eq!(read, [T0, T0 + 60_000]);
    assert_eq!(each_line(&dir.path().join("none.jsonl"), |_| ()), 0);
}
