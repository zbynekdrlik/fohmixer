//! The hub's event log (#43, design note §5.1): the audit trail of every
//! move on its way from a page to Live, in dated files kept for weeks.
//!
//! - `<data>/logs/events-YYYY-MM-DD.jsonl` (the UTC date of each record's
//!   `ts`), one JSON object per line. Every record has `ev` (what happened)
//!   and `ts` (the hub's UTC clock, ms), and where it applies `client`,
//!   `peer`, `instance`, `key`, `seq` and `t` (the page's clock).
//! - Producers never wait: [`EventLog::record`] puts the record into a
//!   bounded channel; a full channel counts the record as dropped, and the
//!   writer logs a `dropped` record with the count before its next record.
//! - One thread writes, flushing after every burst it drained.
//! - Retention: files older than [`KEEP_DAYS`] are deleted at start, when
//!   the day turns and at least once an hour while idle.
//! - A day file is capped at [`DAY_CAP`]: past it the writer writes one
//!   `cap` record, then only warn-class records ([`is_warn`]).
//!
//! Records: `sock`, `set`, `batch`, `applied`, `ack`, `ping`, `link` and the
//! page's `trace` (its dropouts since PR A; the flight recorder and the
//! counter's resets with PR C).

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel};
use std::time::Duration;

use chrono::{DateTime, NaiveDate};
use serde_json::{Map, Value, json};

/// Records waiting for the writer before new ones are dropped.
pub const QUEUE: usize = 8192;
/// Day files older than this many days are deleted.
pub const KEEP_DAYS: i64 = 60;
/// A day file's size past which only warn-class records are written.
pub const DAY_CAP: u64 = 256 * 1024 * 1024;
/// How long the writer waits idle before it checks the retention again.
pub const SWEEP_EVERY: Duration = Duration::from_secs(3600);

/// The writer's limits (the tests shrink them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub keep_days: i64,
    pub day_cap: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            keep_days: KEEP_DAYS,
            day_cap: DAY_CAP,
        }
    }
}

/// The producers' handle (cheap to clone).
#[derive(Debug, Clone)]
pub struct EventLog {
    tx: Option<SyncSender<Value>>,
    dropped: Arc<AtomicU64>,
}

impl EventLog {
    /// The log in `dir` (`<data>/logs`), written by a thread of its own
    /// that ends when the last handle is dropped.
    pub fn start(dir: PathBuf) -> std::io::Result<Self> {
        Self::start_with(dir, Limits::default()).map(|(log, _)| log)
    }

    /// [`EventLog::start`] with other limits; the writer thread's handle.
    pub fn start_with(
        dir: PathBuf,
        limits: Limits,
    ) -> std::io::Result<(Self, std::thread::JoinHandle<()>)> {
        let (log, rx) = Self::channel(QUEUE);
        let dropped = Arc::clone(&log.dropped);
        let writer = std::thread::Builder::new()
            .name("event-log".into())
            .spawn(move || run(Writer::new(dir, limits), &rx, &dropped))?;
        Ok((log, writer))
    }

    /// A log whose records go to the returned receiver, at most `capacity`
    /// waiting.
    pub fn channel(capacity: usize) -> (Self, Receiver<Value>) {
        let (tx, rx) = sync_channel(capacity);
        (
            Self {
                tx: Some(tx),
                dropped: Arc::new(AtomicU64::new(0)),
            },
            rx,
        )
    }

    /// A log that records nothing.
    pub fn off() -> Self {
        Self {
            tx: None,
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Records event `ev` with `fields` (an object), stamped now. Never
    /// waits: with the queue full the record is counted as dropped.
    pub fn record(&self, ev: &str, fields: Value) {
        let Some(tx) = &self.tx else {
            return;
        };
        if let Err(TrySendError::Full(_)) = tx.try_send(stamp(ev, now_ms(), fields)) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The hub's UTC clock in ms (0 if the clock is before 1970).
pub fn now_ms() -> i64 {
    crate::live::wall_ms().map_or(0, |ms| ms as i64)
}

/// A record: `ev`, `ts` (hub UTC ms), then the entries of `fields` (an
/// object; anything else adds nothing).
pub fn stamp(ev: &str, ts: i64, fields: Value) -> Value {
    let mut record = Map::new();
    record.insert("ev".into(), json!(ev));
    record.insert("ts".into(), json!(ts));
    if let Value::Object(entries) = fields {
        record.extend(entries);
    }
    Value::Object(record)
}

/// A record's `ts` (0 without one).
pub fn ts_of(record: &Value) -> i64 {
    record.get("ts").and_then(Value::as_i64).unwrap_or(0)
}

/// The UTC date of `ts` (ms since the Unix epoch).
pub fn date_of(ts: i64) -> NaiveDate {
    DateTime::from_timestamp_millis(ts)
        .unwrap_or(DateTime::UNIX_EPOCH)
        .date_naive()
}

/// The day file of `date`.
pub fn file_name(date: NaiveDate) -> String {
    format!("events-{}.jsonl", date.format("%Y-%m-%d"))
}

/// The date of a day file's name; `None` for any other file.
pub fn date_of_file(name: &str) -> Option<NaiveDate> {
    let day = name.strip_prefix("events-")?.strip_suffix(".jsonl")?;
    NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()
}

/// Whether the day file of `file` is past retention on `today`: more than
/// `keep_days` days old.
pub fn expired(file: NaiveDate, today: NaiveDate, keep_days: i64) -> bool {
    today.signed_duration_since(file).num_days() > keep_days
}

/// Whether a record is still written past a day file's cap: a socket or a
/// link change, a page's `trace` (its dropouts, the owner's priority on
/// #43), the cap and dropped notes, and anything that carries an error.
pub fn is_warn(record: &Value) -> bool {
    let ev = record.get("ev").and_then(Value::as_str).unwrap_or("");
    let failed = record.get("error").is_some_and(|e| !e.is_null())
        || record
            .get("errors")
            .and_then(Value::as_u64)
            .is_some_and(|n| n > 0);
    failed || matches!(ev, "sock" | "link" | "trace" | "cap" | "dropped")
}

/// Whether a line of `len` bytes takes a day file of `size` bytes past
/// `cap`.
pub fn over_cap(size: u64, len: u64, cap: u64) -> bool {
    size + len > cap
}

/// The hub log's note on a write error: only the first of a run of
/// failures is noted (`was_failing`: the previous write failed too).
pub fn failure_note(was_failing: bool, error: &std::io::Error) -> Option<String> {
    (!was_failing).then(|| format!("cannot write the event log: {error}"))
}

/// The open day file.
struct Day {
    date: NaiveDate,
    out: BufWriter<File>,
    size: u64,
    capped: bool,
}

/// The writer: the day files of one folder.
pub struct Writer {
    dir: PathBuf,
    limits: Limits,
    day: Option<Day>,
    /// The date the retention was last checked for.
    swept: Option<NaiveDate>,
    /// The last write failed.
    failing: bool,
}

impl Writer {
    pub fn new(dir: PathBuf, limits: Limits) -> Self {
        Self {
            dir,
            limits,
            day: None,
            swept: None,
            failing: false,
        }
    }

    /// Writes `record`; first a `dropped` record when producers dropped
    /// records since the last one.
    pub fn take(&mut self, record: &Value, dropped: &AtomicU64) {
        let lost = dropped.swap(0, Ordering::Relaxed);
        if lost > 0 {
            self.write(&stamp("dropped", ts_of(record), json!({"n": lost})));
        }
        self.write(record);
    }

    fn write(&mut self, record: &Value) {
        match self.try_write(record) {
            Ok(()) => self.failing = false,
            Err(error) => {
                if let Some(note) = failure_note(self.failing, &error) {
                    tracing::warn!(dir = %self.dir.display(), "{note}");
                }
                self.failing = true;
            }
        }
    }

    fn try_write(&mut self, record: &Value) -> std::io::Result<()> {
        let ts = ts_of(record);
        let date = date_of(ts);
        if self.swept != Some(date) {
            self.sweep(date);
        }
        let cap = self.limits.day_cap;
        let day = self.open(date)?;
        let line = format!("{record}\n");
        if !day.capped && over_cap(day.size, line.len() as u64, cap) {
            let note = format!("{}\n", stamp("cap", ts, json!({"cap_bytes": cap})));
            // Past the cap the size is never compared again.
            day.out.write_all(note.as_bytes())?;
            day.capped = true;
        }
        if day.capped && !is_warn(record) {
            return Ok(());
        }
        day.out.write_all(line.as_bytes())?;
        day.size += line.len() as u64;
        Ok(())
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
                .open(self.dir.join(file_name(date)))?;
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
            if let Some(note) = failure_note(self.failing, &error) {
                tracing::warn!(dir = %self.dir.display(), "{note}");
            }
            self.failing = true;
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
            let Some(date) = date_of_file(&name) else {
                continue;
            };
            if !expired(date, today, self.limits.keep_days) {
                continue;
            }
            match std::fs::remove_file(entry.path()) {
                Ok(()) => {
                    deleted += 1;
                    tracing::info!(file = %name, keep_days = self.limits.keep_days, "event log: deleted a day file past retention");
                }
                Err(error) => {
                    tracing::warn!(file = %name, error = %error, "event log: cannot delete a day file past retention");
                }
            }
        }
        deleted
    }
}

/// Today's UTC date.
fn today() -> NaiveDate {
    date_of(now_ms())
}

/// The writer thread: writes every record it receives, flushes after each
/// burst, checks the retention at start and every [`SWEEP_EVERY`] while
/// idle, and ends when every handle is gone.
fn run(mut writer: Writer, rx: &Receiver<Value>, dropped: &AtomicU64) {
    writer.sweep(today());
    loop {
        match rx.recv_timeout(SWEEP_EVERY) {
            Ok(record) => {
                writer.take(&record, dropped);
                for more in rx.try_iter() {
                    writer.take(&more, dropped);
                }
                writer.flush();
            }
            Err(RecvTimeoutError::Timeout) => {
                writer.sweep(today());
            }
            Err(RecvTimeoutError::Disconnected) => {
                writer.flush();
                return;
            }
        }
    }
}

#[cfg(test)]
#[path = "events/tests.rs"]
mod tests;
