use super::*;

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

fn read(dir: &Path, stem: &str, day: NaiveDate) -> String {
    std::fs::read_to_string(dir.join(file_name(stem, day))).unwrap_or_default()
}

#[test]
fn a_stem_names_its_own_files_only() {
    assert_eq!(
        file_name("lufs", date(2026, 11, 3)),
        "lufs-2026-11-03.jsonl"
    );
    assert_eq!(
        date_of_file("lufs", "lufs-2026-11-03.jsonl"),
        Some(date(2026, 11, 3))
    );
    for other in [
        "events-2026-11-03.jsonl",
        "lufs2026-11-03.jsonl",
        "lufs-2026-11-03.json",
        "lufs-2026-13-03.jsonl",
        "xlufs-2026-11-03.jsonl",
    ] {
        assert_eq!(date_of_file("lufs", other), None, "{other}");
    }
}

#[test]
fn the_failure_note_names_the_files() {
    let error = std::io::Error::other("disk full");
    assert_eq!(
        failure_note("LUFS history", false, &error).as_deref(),
        Some("cannot write the LUFS history: disk full")
    );
    assert_eq!(failure_note("LUFS history", true, &error), None);
}

#[test]
fn past_the_cap_a_line_goes_only_when_let_through_and_the_note_once() {
    let dir = tempfile::tempdir().unwrap();
    let limits = Limits {
        keep_days: 14,
        day_cap: 12,
    };
    let mut files = DayFiles::new(dir.path().to_path_buf(), "lufs", "LUFS history", limits);
    let day = date(2026, 11, 3);
    assert_eq!(
        files.append(noon(day), "aaaaa\n", None, false),
        Appended::Written
    );
    assert_eq!(
        files.append(noon(day), "bbbbb\n", None, false),
        Appended::Written
    );
    // 12 bytes: the next byte is past the cap.
    assert_eq!(
        files.append(noon(day), "c\n", None, false),
        Appended::CapReached
    );
    assert_eq!(
        files.append(noon(day), "d\n", None, false),
        Appended::Capped
    );
    assert_eq!(
        files.append(noon(day), "e\n", None, true),
        Appended::Written
    );
    files.flush();
    assert_eq!(read(dir.path(), "lufs", day), "aaaaa\nbbbbb\ne\n");

    // The note goes first, at the crossing line, which passes when let
    // through.
    let next = date(2026, 11, 4);
    assert_eq!(
        files.append(noon(next), "0123456789\n", None, false),
        Appended::Written
    );
    assert_eq!(
        files.append(noon(next), "xx\n", Some("cap\n"), true),
        Appended::CapReached
    );
    files.flush();
    assert_eq!(read(dir.path(), "lufs", next), "0123456789\ncap\nxx\n");
}

#[test]
fn a_line_a_crash_left_unfinished_is_ended_before_the_next_one() {
    let dir = tempfile::tempdir().unwrap();
    let limits = Limits {
        keep_days: 14,
        day_cap: 1000,
    };
    let day = date(2026, 11, 3);
    let torn = dir.path().join(file_name("lufs", day));
    std::fs::write(&torn, "{\"m\":1").unwrap();
    let mut files = DayFiles::new(dir.path().to_path_buf(), "lufs", "LUFS history", limits);
    assert_eq!(
        files.append(noon(day), "line\n", None, false),
        Appended::Written
    );
    files.flush();
    assert_eq!(read(dir.path(), "lufs", day), "{\"m\":1\nline\n");
    // A whole file and an empty one get nothing added.
    let next = date(2026, 11, 4);
    std::fs::write(dir.path().join(file_name("lufs", next)), "whole\n").unwrap();
    assert_eq!(
        files.append(noon(next), "line\n", None, false),
        Appended::Written
    );
    files.flush();
    assert_eq!(read(dir.path(), "lufs", next), "whole\nline\n");
    let last = date(2026, 11, 5);
    std::fs::write(dir.path().join(file_name("lufs", last)), "").unwrap();
    assert_eq!(
        files.append(noon(last), "line\n", None, false),
        Appended::Written
    );
    files.flush();
    assert_eq!(read(dir.path(), "lufs", last), "line\n");
}

#[test]
fn only_a_file_whose_last_byte_is_no_newline_needs_one() {
    assert!(!needs_newline(None));
    assert!(!needs_newline(Some(b'\n')));
    assert!(needs_newline(Some(b'}')));
    assert!(needs_newline(Some(0)));
}

#[test]
fn a_sweep_deletes_only_its_own_stems_files_past_retention() {
    let dir = tempfile::tempdir().unwrap();
    let today = date(2026, 11, 16);
    for name in [
        "lufs-2026-11-02.jsonl",
        "lufs-2026-11-01.jsonl",
        "events-2026-01-01.jsonl",
        "notes.txt",
    ] {
        std::fs::write(dir.path().join(name), "x").unwrap();
    }
    let limits = Limits {
        keep_days: 14,
        day_cap: 1024,
    };
    let mut files = DayFiles::new(dir.path().to_path_buf(), "lufs", "LUFS history", limits);
    assert_eq!(files.sweep(today), 1);
    let mut left: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(
        left,
        [
            "events-2026-01-01.jsonl",
            "lufs-2026-11-02.jsonl",
            "notes.txt"
        ]
    );
    assert_eq!(files.dir(), dir.path());
    assert_eq!(files.limits(), limits);
}

/// A sink that keeps what the thread did.
struct Kept(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

impl Sink<u32> for Kept {
    fn take(&mut self, item: u32) {
        self.0.lock().unwrap().push(format!("take {item}"));
    }
    fn flush(&mut self) {
        self.0.lock().unwrap().push("flush".into());
    }
    fn sweep(&mut self, _today: NaiveDate) {
        self.0.lock().unwrap().push("sweep".into());
    }
}

#[test]
fn the_thread_sweeps_first_flushes_each_burst_and_ends_with_its_handles() {
    let kept = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let (tx, rx) = std::sync::mpsc::sync_channel(8);
    tx.send(1).unwrap();
    tx.send(2).unwrap();
    drop(tx);
    run(Kept(std::sync::Arc::clone(&kept)), &rx);
    assert_eq!(
        *kept.lock().unwrap(),
        ["sweep", "take 1", "take 2", "flush", "flush"]
    );
    assert_eq!(SWEEP_EVERY, Duration::from_secs(3600));
}

#[test]
fn the_newline_that_ends_a_torn_line_counts_toward_the_cap() {
    let dir = tempfile::tempdir().unwrap();
    let limits = Limits {
        keep_days: 14,
        day_cap: 12,
    };
    let day = date(2026, 11, 3);
    // 7 bytes and the newline added: 8; a line of 5 takes it past 12.
    std::fs::write(dir.path().join(file_name("lufs", day)), "partial").unwrap();
    let mut files = DayFiles::new(dir.path().to_path_buf(), "lufs", "LUFS history", limits);
    assert_eq!(
        files.append(noon(day), "line\n", None, false),
        Appended::CapReached
    );
    files.flush();
    assert_eq!(read(dir.path(), "lufs", day), "partial\n");
}
