//! Dated day files written by one thread (#43; generalised for #219): the
//! machinery the event log (`events.rs`, `<data>/logs/events-*.jsonl`) and
//! the LUFS history (`lufs_history.rs`, `<data>/lufs/lufs-*.jsonl`) share.
//!
//! - `<dir>/<stem>-YYYY-MM-DD.jsonl`, the UTC date of each line's time.
//! - Retention: files more than `keep_days` old are deleted at start, when
//!   the day turns and at least once every [`SWEEP_EVERY`] while idle.
//! - A day file is capped at `day_cap`: past it only what the writer's own
//!   rule lets through is written ([`DayFiles::append`]'s `past_cap`), once
//!   after an optional note line.
//! - One thread writes ([`run`]): it flushes after every burst it drained
//!   and ends when every producer's handle is gone. Producers never wait:
//!   each writer counts what a full queue drops on its own.

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use chrono::{DateTime, NaiveDate};

/// How long a writer waits idle before it checks the retention again.
pub const SWEEP_EVERY: Duration = Duration::from_secs(3600);

/// A writer's limits (the tests shrink them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub keep_days: i64,
    pub day_cap: u64,
}

/// The UTC date of `ts` (ms since the Unix epoch).
pub fn date_of(ts: i64) -> NaiveDate {
    DateTime::from_timestamp_millis(ts)
        .unwrap_or(DateTime::UNIX_EPOCH)
        .date_naive()
}

/// The day file of `date` in the folder of `stem`.
pub fn file_name(stem: &str, date: NaiveDate) -> String {
    format!("{stem}-{}.jsonl", date.format("%Y-%m-%d"))
}

/// The date of a day file's name of `stem`; `None` for any other file.
pub fn date_of_file(stem: &str, name: &str) -> Option<NaiveDate> {
    let day = name
        .strip_prefix(stem)?
        .strip_prefix('-')?
        .strip_suffix(".jsonl")?;
    NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()
}

/// Whether the day file of `file` is past retention on `today`: more than
/// `keep_days` days old.
pub fn expired(file: NaiveDate, today: NaiveDate, keep_days: i64) -> bool {
    today.signed_duration_since(file).num_days() > keep_days
}

/// Whether a line of `len` bytes takes a day file of `size` bytes past
/// `cap`.
pub fn over_cap(size: u64, len: u64, cap: u64) -> bool {
    size + len > cap
}

/// The hub log's note on a write error of the files of `what`: only the
/// first of a run of failures is noted (`was_failing`: the previous write
/// failed too).
pub fn failure_note(what: &str, was_failing: bool, error: &std::io::Error) -> Option<String> {
    (!was_failing).then(|| format!("cannot write the {what}: {error}"))
}

/// The open day file.
struct Day {
    date: NaiveDate,
    out: BufWriter<File>,
    size: u64,
    capped: bool,
}

/// What [`DayFiles::append`] did with a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appended {
    /// The line is in its day file.
    Written,
    /// The line was left out: its day file is past the cap.
    Capped,
    /// This line took its day file past the cap (the note, if any, went;
    /// the line only when `past_cap` let it).
    CapReached,
    /// The write failed (noted in the hub log once per run of failures).
    Failed,
}

/// The day files of one folder.
pub struct DayFiles {
    dir: PathBuf,
    stem: &'static str,
    /// The files' name in the hub log's lines (`event log`, `LUFS history`).
    what: &'static str,
    limits: Limits,
    day: Option<Day>,
    /// The date the retention was last checked for.
    swept: Option<NaiveDate>,
    /// The last write failed.
    failing: bool,
}

impl DayFiles {
    pub fn new(dir: PathBuf, stem: &'static str, what: &'static str, limits: Limits) -> Self {
        Self {
            dir,
            stem,
            what,
            limits,
            day: None,
            swept: None,
            failing: false,
        }
    }

    /// The folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The limits.
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Whether the last write failed.
    pub fn failing(&self) -> bool {
        self.failing
    }

    /// Appends `line` (one line, its `\n` included) to the day file of
    /// `ts`. At the line that takes the file past its cap, `cap_note` (a
    /// line of its own) goes first and is not counted; past the cap a line
    /// is written only when `past_cap`.
    pub fn append(
        &mut self,
        ts: i64,
        line: &str,
        cap_note: Option<&str>,
        past_cap: bool,
    ) -> Appended {
        match self.try_append(ts, line, cap_note, past_cap) {
            Ok(done) => {
                self.failing = false;
                done
            }
            Err(error) => {
                self.note_failure(&error);
                Appended::Failed
            }
        }
    }

    fn note_failure(&mut self, error: &std::io::Error) {
        if let Some(note) = failure_note(self.what, self.failing, error) {
            tracing::warn!(dir = %self.dir.display(), "{note}");
        }
        self.failing = true;
    }

    fn try_append(
        &mut self,
        ts: i64,
        line: &str,
        cap_note: Option<&str>,
        past_cap: bool,
    ) -> std::io::Result<Appended> {
        let date = date_of(ts);
        if self.swept != Some(date) {
            self.sweep(date);
        }
        let cap = self.limits.day_cap;
        let day = self.open(date)?;
        let mut done = Appended::Written;
        if !day.capped && over_cap(day.size, line.len() as u64, cap) {
            if let Some(note) = cap_note {
                day.out.write_all(note.as_bytes())?;
            }
            // Past the cap the size is never compared again.
            day.capped = true;
            done = Appended::CapReached;
        }
        if day.capped && !past_cap {
            return Ok(if done == Appended::CapReached {
                done
            } else {
                Appended::Capped
            });
        }
        day.out.write_all(line.as_bytes())?;
        day.size += line.len() as u64;
        Ok(done)
    }

    /// The day file of `date`, opened (appending) when another one is open.
    fn open(&mut self, date: NaiveDate) -> std::io::Result<&mut Day> {
        if self.day.as_ref().is_some_and(|d| d.date != date) {
            self.flush();
            self.day = None;
        }
        if self.day.is_none() {
            std::fs::create_dir_all(&self.dir)?;
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.dir.join(file_name(self.stem, date)))?;
            let size = file.metadata()?.len();
            self.day = Some(Day {
                date,
                out: BufWriter::new(file),
                size,
                capped: false,
            });
        }
        Ok(self.day.as_mut().expect("opened above"))
    }

    /// Writes out what the open day file buffers.
    pub fn flush(&mut self) {
        let failed = self.day.as_mut().and_then(|d| d.out.flush().err());
        if let Some(error) = failed {
            self.note_failure(&error);
        }
    }

    /// Deletes the day files past retention on `today`; how many it
    /// deleted.
    pub fn sweep(&mut self, today: NaiveDate) -> usize {
        self.swept = Some(today);
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        let mut deleted = 0;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(date) = date_of_file(self.stem, &name) else {
                continue;
            };
            if !expired(date, today, self.limits.keep_days) {
                continue;
            }
            match std::fs::remove_file(entry.path()) {
                Ok(()) => {
                    deleted += 1;
                    tracing::info!(file = %name, keep_days = self.limits.keep_days, "{}: deleted a day file past retention", self.what);
                }
                Err(error) => {
                    tracing::warn!(file = %name, error = %error, "{}: cannot delete a day file past retention", self.what);
                }
            }
        }
        deleted
    }
}

/// A writer the thread of [`run`] drives.
pub trait Sink<T> {
    /// Writes one item.
    fn take(&mut self, item: T);
    /// Writes out what it buffers.
    fn flush(&mut self);
    /// Deletes the day files past retention on `today`.
    fn sweep(&mut self, today: NaiveDate);
}

/// Today's UTC date by the hub's clock.
pub fn today() -> NaiveDate {
    date_of(crate::events::now_ms())
}

/// The writer thread: writes every item it receives, flushes after each
/// burst, checks the retention at start and every [`SWEEP_EVERY`] while
/// idle, and ends when every handle is gone.
pub fn run<T, S: Sink<T>>(mut sink: S, rx: &Receiver<T>) {
    sink.sweep(today());
    loop {
        match rx.recv_timeout(SWEEP_EVERY) {
            Ok(item) => {
                sink.take(item);
                for more in rx.try_iter() {
                    sink.take(more);
                }
                sink.flush();
            }
            Err(RecvTimeoutError::Timeout) => {
                sink.sweep(today());
            }
            Err(RecvTimeoutError::Disconnected) => {
                sink.flush();
                return;
            }
        }
    }
}

#[cfg(test)]
#[path = "day_files/tests.rs"]
mod tests;
