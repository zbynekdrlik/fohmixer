use super::*;
use chrono::Days;

fn date(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

/// `ts` (ms) at noon UTC of a date.
fn noon(day: NaiveDate) -> i64 {
    day.and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp_millis()
}

/// The lines of a day file, parsed.
fn lines(dir: &std::path::Path, day: NaiveDate) -> Vec<Value> {
    std::fs::read_to_string(dir.join(file_name(day)))
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

fn evs(records: &[Value]) -> Vec<&str> {
    records.iter().map(|r| r["ev"].as_str().unwrap()).collect()
}

#[test]
fn the_limits_are_sixty_days_and_256_mb() {
    assert_eq!(KEEP_DAYS, 60);
    assert_eq!(DAY_CAP, 268_435_456);
    assert_eq!(QUEUE, 8192);
    assert_eq!(SWEEP_EVERY, Duration::from_secs(3600));
    assert_eq!(
        Limits::default(),
        Limits {
            keep_days: 60,
            day_cap: 268_435_456
        }
    );
}

#[test]
fn a_record_is_stamped_with_its_event_and_time() {
    assert_eq!(
        stamp("set", 1_790_000_000_123, json!({"client": 3, "seq": 41})),
        json!({"ev": "set", "ts": 1_790_000_000_123_i64, "client": 3, "seq": 41})
    );
    assert_eq!(
        stamp("cap", 5, json!("not an object")),
        json!({"ev": "cap", "ts": 5})
    );
    assert_eq!(ts_of(&json!({"ts": 77})), 77);
    assert_eq!(ts_of(&json!({"ev": "x"})), 0);
    // The hub's clock: milliseconds of today, not seconds.
    let now = now_ms();
    assert!(now > 1_788_000_000_000 && now < 4_102_444_800_000, "{now}");
}

#[test]
fn day_files_are_named_by_the_utc_date() {
    // 2026-10-03 23:59:59.999 UTC and the next millisecond.
    let last = noon(date(2026, 10, 3)) + 12 * 3_600_000 - 1;
    assert_eq!(date_of(last), date(2026, 10, 3));
    assert_eq!(date_of(last + 1), date(2026, 10, 4));
    assert_eq!(date_of(0), date(1970, 1, 1));
    assert_eq!(date_of(i64::MAX), date(1970, 1, 1), "out of range");
    assert_eq!(file_name(date(2026, 1, 9)), "events-2026-01-09.jsonl");
    assert_eq!(
        date_of_file("events-2026-01-09.jsonl"),
        Some(date(2026, 1, 9))
    );
    for other in [
        "hub.out.log",
        "events-2026-01-09.json",
        "x-events-2026-01-09.jsonl",
        "events-2026-13-01.jsonl",
        "events-.jsonl",
    ] {
        assert_eq!(date_of_file(other), None, "{other}");
    }
}

#[test]
fn a_file_expires_after_sixty_days() {
    let today = date(2026, 10, 3);
    assert!(!expired(date(2026, 10, 3), today, 60));
    assert!(!expired(date(2026, 8, 4), today, 60), "60 days old");
    assert!(expired(date(2026, 8, 3), today, 60), "61 days old");
    assert!(!expired(date(2026, 10, 4), today, 60), "a day ahead");
}

#[test]
fn warn_class_records_are_socket_link_cap_dropped_and_errors() {
    for ev in [
        "sock",
        "link",
        "cap",
        "dropped",
        "deck_link",
        "deck_release",
        "deck_press",
        "deck_ok",
        "deck_view",
        "eq",
    ] {
        assert!(is_warn(&json!({"ev": ev})), "{ev}");
    }
    for ev in [
        "set",
        "batch",
        "ping",
        "ack",
        "applied",
        "trace",
        "deck_key",
        "deck_keys",
    ] {
        assert!(!is_warn(&json!({"ev": ev})), "{ev}");
    }
    assert!(is_warn(&json!({"ev": "ack", "error": "instance offline"})));
    assert!(!is_warn(&json!({"ev": "ack", "error": null})));
    // A press and Companion's answer outlive the cap, refused or not (#52):
    // the incident logs need every press.
    assert!(is_warn(
        &json!({"ev": "deck_ok", "ok": false, "error": "Invalid KEY"})
    ));
    assert!(is_warn(
        &json!({"ev": "deck_ok", "ok": true, "error": null})
    ));
    assert!(is_warn(
        &json!({"ev": "deck_press", "forwarded": true, "reason": null})
    ));
    // A key's image change stays bulk, with or without a colour.
    assert!(!is_warn(
        &json!({"ev": "deck_key", "key": 3, "pressed": true, "color": "#00aa00"})
    ));
    assert!(is_warn(&json!({"ev": "applied", "errors": 1})));
    assert!(!is_warn(&json!({"ev": "applied", "errors": 0})));
    assert!(!is_warn(&json!({})));
}

#[test]
fn a_trace_is_warn_class_only_with_a_dropout_or_a_counter_reset() {
    let trace = |events: Value| json!({"ev": "trace", "client": 3, "events": events});
    assert!(is_warn(&trace(json!([
        {"ev": "ping", "t": 1.0, "n": 4},
        {"ev": "dropout", "t": 2.0, "ms": 420.0, "socket_lost": false, "rtts": []}
    ]))));
    assert!(is_warn(&trace(json!([
        {"ev": "reset", "t": 3.0, "count": 2, "active": false}
    ]))));
    assert!(!is_warn(&trace(json!([
        {"ev": "ping", "t": 1.0, "n": 4},
        {"ev": "pong", "t": 1.5, "n": 4, "rtt": 0.5},
        {"ev": "touch", "t": 2.0, "what": "down", "keys": ["a|b|c"], "pointer": 1}
    ]))));
    assert!(!is_warn(&trace(json!([]))));
    assert!(!is_warn(&json!({"ev": "trace"})), "no events");
    assert!(
        !is_warn(&json!({"ev": "trace", "events": {"ev": "dropout"}})),
        "not a list"
    );
    // Another record holding a dropout-shaped list is not a trace.
    assert!(!is_warn(
        &json!({"ev": "set", "events": [{"ev": "dropout"}]})
    ));
}

#[test]
fn the_cap_is_crossed_by_a_line_past_it() {
    assert!(!over_cap(10, 5, 15));
    assert!(over_cap(10, 6, 15));
    assert!(over_cap(0, 16, 15));
    assert!(!over_cap(15, 0, 15));
}

#[test]
fn only_the_first_write_failure_of_a_run_is_noted() {
    let error = std::io::Error::other("disk full");
    assert_eq!(
        failure_note(false, &error).as_deref(),
        Some("cannot write the event log: disk full")
    );
    assert_eq!(failure_note(true, &error), None);
}

#[test]
fn records_go_to_the_file_of_their_day() {
    let dir = tempfile::tempdir().unwrap();
    let logs = dir.path().join("logs");
    let mut writer = Writer::new(logs.clone(), Limits::default());
    let dropped = AtomicU64::new(0);
    let day1 = date(2026, 10, 3);
    let day2 = date(2026, 10, 4);
    writer.take(&stamp("set", noon(day1), json!({"seq": 1})), &dropped);
    writer.take(&stamp("ack", noon(day1) + 1, json!({"seq": 1})), &dropped);
    writer.take(&stamp("set", noon(day2), json!({"seq": 2})), &dropped);
    writer.flush();
    let first = lines(&logs, day1);
    assert_eq!(evs(&first), vec!["set", "ack"]);
    assert_eq!(first[0]["seq"], 1);
    assert_eq!(evs(&lines(&logs, day2)), vec!["set"]);
    // A new writer (a hub restart) appends to the day's file.
    let mut again = Writer::new(logs.clone(), Limits::default());
    again.take(&stamp("ping", noon(day2) + 5, json!({"n": 0})), &dropped);
    again.flush();
    assert_eq!(evs(&lines(&logs, day2)), vec!["set", "ping"]);
}

#[test]
fn dropped_records_are_counted_before_the_next_one() {
    let (log, rx) = EventLog::channel(1);
    log.record("set", json!({"seq": 1}));
    log.record("set", json!({"seq": 2}));
    log.record("set", json!({"seq": 3}));
    assert_eq!(log.dropped.load(Ordering::Relaxed), 2);
    let first = rx.try_recv().unwrap();
    assert_eq!(first["seq"], 1);
    assert!(rx.try_recv().is_err(), "the rest was dropped");
    let dir = tempfile::tempdir().unwrap();
    let mut writer = Writer::new(dir.path().to_path_buf(), Limits::default());
    writer.take(&first, &log.dropped);
    writer.take(
        &stamp("set", ts_of(&first), json!({"seq": 4})),
        &log.dropped,
    );
    writer.flush();
    let written = lines(dir.path(), date_of(ts_of(&first)));
    assert_eq!(evs(&written), vec!["dropped", "set", "set"]);
    assert_eq!(written[0]["n"], 2);
    assert_eq!(written[0]["ts"], first["ts"]);
    assert_eq!(log.dropped.load(Ordering::Relaxed), 0, "counted once");
    // A log that is off records nothing and never fails.
    EventLog::off().record("set", json!({"seq": 5}));
}

#[test]
fn past_the_cap_only_warn_class_records_are_written() {
    let dir = tempfile::tempdir().unwrap();
    let limits = Limits {
        keep_days: 60,
        day_cap: 200,
    };
    let mut writer = Writer::new(dir.path().to_path_buf(), limits);
    let dropped = AtomicU64::new(0);
    let day = date(2026, 10, 3);
    let set = |seq: u64| stamp("set", noon(day), json!({"seq": seq, "pad": "x".repeat(40)}));
    for seq in 1..=6 {
        writer.take(&set(seq), &dropped);
    }
    writer.take(
        &stamp("sock", noon(day), json!({"what": "close"})),
        &dropped,
    );
    writer.take(&set(7), &dropped);
    writer.take(
        &stamp(
            "ack",
            noon(day),
            json!({"seq": 7, "error": "instance offline"}),
        ),
        &dropped,
    );
    // A Stream Deck press still goes in past the cap, a key's image does
    // not (#52).
    writer.take(
        &stamp("deck_key", noon(day), json!({"key": 3, "pressed": true})),
        &dropped,
    );
    writer.take(
        &stamp("deck_press", noon(day), json!({"key": 3, "down": true})),
        &dropped,
    );
    writer.flush();
    let written = lines(dir.path(), day);
    assert_eq!(
        evs(&written),
        vec!["set", "set", "cap", "sock", "ack", "deck_press"],
        "{written:?}"
    );
    assert_eq!(written[2]["cap_bytes"], 200);
    assert_eq!(written[1]["seq"], 2);
    // The next day starts a file of its own, under the cap again.
    let next = date(2026, 10, 4);
    writer.take(&stamp("set", noon(next), json!({"seq": 8})), &dropped);
    writer.flush();
    assert_eq!(evs(&lines(dir.path(), next)), vec!["set"]);
}

#[test]
fn a_sweep_deletes_only_day_files_past_retention() {
    let dir = tempfile::tempdir().unwrap();
    let today = date(2026, 10, 3);
    for name in [
        "events-2026-08-03.jsonl",
        "events-2026-01-01.jsonl",
        "events-2026-08-04.jsonl",
        "events-2026-10-03.jsonl",
        "hub.out.log",
        "events-garbage.jsonl",
    ] {
        std::fs::write(dir.path().join(name), "{}\n").unwrap();
    }
    let mut writer = Writer::new(dir.path().to_path_buf(), Limits::default());
    assert_eq!(writer.sweep(today), 2);
    let mut left: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(
        left,
        vec![
            "events-2026-08-04.jsonl",
            "events-2026-10-03.jsonl",
            "events-garbage.jsonl",
            "hub.out.log",
        ]
    );
    // A folder that does not exist yet has nothing to sweep.
    let mut nowhere = Writer::new(dir.path().join("missing"), Limits::default());
    assert_eq!(nowhere.sweep(today), 0);
}

#[test]
fn the_first_record_of_a_day_sweeps_the_folder() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("events-2026-01-01.jsonl");
    std::fs::write(&old, "{}\n").unwrap();
    let mut writer = Writer::new(dir.path().to_path_buf(), Limits::default());
    let dropped = AtomicU64::new(0);
    writer.take(&stamp("set", noon(date(2026, 3, 1)), json!({})), &dropped);
    assert!(old.exists(), "59 days: kept");
    writer.take(&stamp("set", noon(date(2026, 3, 3)), json!({})), &dropped);
    assert!(!old.exists(), "61 days: the day turned and swept it");
}

#[test]
fn a_write_failure_is_noted_and_the_writer_goes_on() {
    let dir = tempfile::tempdir().unwrap();
    // A file where the folder should be: every open fails.
    let blocked = dir.path().join("logs");
    std::fs::write(&blocked, "not a folder").unwrap();
    let mut writer = Writer::new(blocked.clone(), Limits::default());
    let dropped = AtomicU64::new(0);
    let day = date(2026, 10, 3);
    writer.take(&stamp("set", noon(day), json!({"seq": 1})), &dropped);
    assert!(writer.failing);
    writer.take(&stamp("set", noon(day), json!({"seq": 2})), &dropped);
    assert!(writer.failing);
    // The folder comes back: the next record is written.
    std::fs::remove_file(&blocked).unwrap();
    writer.take(&stamp("set", noon(day), json!({"seq": 3})), &dropped);
    assert!(!writer.failing);
    writer.flush();
    let written = lines(&blocked, day);
    assert_eq!(written.len(), 1);
    assert_eq!(written[0]["seq"], 3);
}

#[test]
fn the_writer_thread_writes_sweeps_at_start_and_ends_with_its_handles() {
    let dir = tempfile::tempdir().unwrap();
    let logs = dir.path().join("logs");
    std::fs::create_dir_all(&logs).unwrap();
    let today = date_of(now_ms());
    let expired_day = today - Days::new(61);
    let old = logs.join(file_name(expired_day));
    std::fs::write(&old, "{}\n").unwrap();
    let (log, writer) = EventLog::start_with(logs.clone(), Limits::default()).unwrap();
    // No record yet: only the start's sweep can delete it.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while old.exists() {
        assert!(std::time::Instant::now() < deadline, "swept at start");
        std::thread::sleep(Duration::from_millis(10));
    }
    log.record("sock", json!({"client": 1, "what": "open"}));
    let copy = log.clone();
    copy.record("sock", json!({"client": 1, "what": "close"}));
    drop(log);
    drop(copy);
    writer.join().unwrap();
    let written = lines(&logs, today);
    assert!(written.len() >= 2, "{written:?}");
    let tail = &written[written.len() - 2..];
    assert_eq!(tail[0]["what"], "open");
    assert_eq!(tail[1]["what"], "close");
    assert_eq!(tail[0]["client"], 1);
    assert!(ts_of(&tail[0]) > 0);
}
