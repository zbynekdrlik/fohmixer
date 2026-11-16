//! The LUFS history's store (#219, the design on the ticket): the pure
//! recorder (`rules/lufs/history.rs`) behind a lock the router samples once
//! a second and the API reads, and its day files.
//!
//! - `<data>/lufs/lufs-YYYY-MM-DD.jsonl` (the UTC date of each minute), one
//!   minute line each, written by a thread of its own through the event
//!   log's machinery (`day_files.rs`): [`KEEP_DAYS`] days kept, a day file
//!   capped at [`DAY_CAP`] (past it nothing more that day, one warn
//!   `lufs_history` record).
//! - Producers never wait: the router puts each minute line into a queue of
//!   [`QUEUE`]; a full queue counts the line as dropped (`/api/status`
//!   `lufs.history.dropped`).
//! - A window ([`LufsHistory::history`]): the last hour from RAM, anything
//!   older read from the day files line by line (never a whole file in
//!   memory), bucketed to at most 900 values.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use chrono::NaiveDate;
use fohmixer_proto::client::{LufsHistoryStatus, LufsItem};
use fohmixer_proto::lufs::{HistoryDay, HistorySeries};
use serde_json::{Value, json};

use crate::day_files::{self, Appended, DayFiles, Limits, Sink};
use crate::events::EventLog;
use crate::rules::lufs::history::{
    HOLE, MINUTE_MS, MinuteLine, Point, Recorder, WindowError, bucket, span_of, window,
};

/// The day files' folder in the data folder, and their stem.
pub const FOLDER: &str = "lufs";
pub const STEM: &str = "lufs";
/// Day files older than this many days are deleted (two Sundays back).
pub const KEEP_DAYS: i64 = 14;
/// A day file's size past which nothing more is written that day.
pub const DAY_CAP: u64 = 32 * 1024 * 1024;
/// Minute lines waiting for the writer before new ones are dropped.
pub const QUEUE: usize = 64;
/// The history's limits.
pub const LIMITS: Limits = Limits {
    keep_days: KEEP_DAYS,
    day_cap: DAY_CAP,
};

/// The history: cheap to clone, one per hub (the router samples it, the
/// API reads it).
#[derive(Debug, Clone)]
pub struct LufsHistory {
    recorder: Arc<Mutex<Recorder>>,
    tx: Option<SyncSender<MinuteLine>>,
    dropped: Arc<AtomicU64>,
    /// The day files' folder; none: RAM only.
    dir: Option<PathBuf>,
}

impl Default for LufsHistory {
    fn default() -> Self {
        Self::off()
    }
}

impl LufsHistory {
    /// A history that keeps the last hour in RAM and writes nothing.
    pub fn off() -> Self {
        Self {
            recorder: Arc::new(Mutex::new(Recorder::new())),
            tx: None,
            dropped: Arc::new(AtomicU64::new(0)),
            dir: None,
        }
    }

    /// The history of the data folder `data`, its writer a thread of its
    /// own that ends when the last handle is dropped.
    pub fn start(data: &Path, events: EventLog) -> std::io::Result<Self> {
        Self::start_with(data.join(FOLDER), LIMITS, events).map(|(history, _)| history)
    }

    /// [`LufsHistory::start`] in `dir` with other limits; the writer
    /// thread's handle.
    pub fn start_with(
        dir: PathBuf,
        limits: Limits,
        events: EventLog,
    ) -> std::io::Result<(Self, std::thread::JoinHandle<()>)> {
        let (history, rx) = Self::channel(Some(dir.clone()), QUEUE);
        let writer = std::thread::Builder::new()
            .name("lufs-history".into())
            .spawn(move || day_files::run(LufsWriter::new(dir, limits, events), &rx))?;
        Ok((history, writer))
    }

    /// A history reading `dir` whose lines go to the returned receiver, at
    /// most `capacity` waiting.
    pub fn channel(dir: Option<PathBuf>, capacity: usize) -> (Self, Receiver<MinuteLine>) {
        let (tx, rx) = sync_channel(capacity);
        let history = Self {
            recorder: Arc::new(Mutex::new(Recorder::new())),
            tx: Some(tx),
            dropped: Arc::new(AtomicU64::new(0)),
            dir,
        };
        (history, rx)
    }

    fn recorder(&self) -> MutexGuard<'_, Recorder> {
        self.recorder.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Samples every served point at the second of `ms` (the hub's UTC
    /// clock) when that second is not sampled yet; `items` (the points'
    /// readings, `Lufs::items`) is asked only then. A minute that ended
    /// goes to the writer; never waits.
    pub fn sample(&self, ms: i64, items: impl FnOnce() -> Vec<LufsItem>) {
        let line = {
            let mut recorder = self.recorder();
            if !recorder.due(ms) {
                return;
            }
            let readings: Vec<(Point, Option<i16>)> = items()
                .into_iter()
                .map(|item| ((item.instance, item.target), item.tenths))
                .collect();
            recorder.sample(ms, &readings)
        };
        if let Some(line) = line {
            self.send(line);
        }
    }

    /// Hands a minute line to the writer; a full queue counts it dropped.
    fn send(&self, line: MinuteLine) {
        let Some(tx) = &self.tx else {
            return;
        };
        if let Err(TrySendError::Full(_)) = tx.try_send(line) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// `/api/status` `lufs.history`.
    pub fn status(&self) -> LufsHistoryStatus {
        LufsHistoryStatus {
            points: self.recorder().points(),
            dropped: self.dropped.load(Ordering::Relaxed),
        }
    }

    /// `GET /api/lufs/history`: the point's readings from `from` to `to`
    /// (UTC ms), bucketed. Reads files: call it off the async runtime.
    pub fn history(
        &self,
        instance: &str,
        target: &str,
        from: i64,
        to: i64,
    ) -> Result<HistorySeries, WindowError> {
        let (start, n) = window(from, to)?;
        let end = start + n as i64 * 1000;
        let point: Point = (instance.to_string(), target.to_string());
        let covers = self.recorder().covers_from();
        let mut slots = vec![HOLE; n];
        let files_end = files_until(covers, end);
        if let Some(dir) = &self.dir {
            read_window(dir, &point, start, files_end, &mut slots);
        }
        if let Some(covers) = covers {
            let recorder = self.recorder();
            for (i, slot) in slots.iter_mut().enumerate() {
                let t = start + i as i64 * 1000;
                if t >= covers {
                    *slot = recorder.at(&point, t);
                }
            }
        }
        Ok(bucket(start, &slots))
    }

    /// `GET /api/lufs/days`: the days that hold the point, newest first;
    /// the minute not written yet counts for its day.
    pub fn days(&self, instance: &str, target: &str) -> Vec<HistoryDay> {
        let mut days: Vec<(NaiveDate, HistoryDay)> = Vec::new();
        if let Some(dir) = &self.dir {
            for (date, path) in day_files_in(dir) {
                if let Some(day) = day_of(&path, instance, target) {
                    days.push((date, day));
                }
            }
        }
        let current = self.recorder().current();
        if let Some((m, span)) = current.and_then(|line| {
            let slots = line.slots(instance, target)?;
            Some((line.m, span_of(line.m, &slots)?))
        }) {
            add_span(&mut days, day_files::date_of(m), span);
        }
        days.sort_by_key(|(date, _)| std::cmp::Reverse(*date));
        days.into_iter().map(|(_, day)| day).collect()
    }
}

/// Where the day files stop answering a window ending at `end`: the first
/// second the hour in RAM answers for, or the end when RAM holds nothing.
pub fn files_until(covers: Option<i64>, end: i64) -> i64 {
    covers.map_or(end, |c| c.min(end))
}

/// A day widened by the span `(first, last)` (the span alone when none).
pub fn widen(day: Option<HistoryDay>, span: (i64, i64)) -> HistoryDay {
    match day {
        Some(day) => HistoryDay {
            first_ms: day.first_ms.min(span.0),
            last_ms: day.last_ms.max(span.1),
        },
        None => HistoryDay {
            first_ms: span.0,
            last_ms: span.1,
        },
    }
}

/// Adds the span `(first, last)` to `date`'s day (a new day when none).
pub fn add_span(days: &mut Vec<(NaiveDate, HistoryDay)>, date: NaiveDate, span: (i64, i64)) {
    match days.iter_mut().find(|(d, _)| *d == date) {
        Some((_, day)) => *day = widen(Some(*day), span),
        None => days.push((date, widen(None, span))),
    }
}

/// The day files of `dir` with their dates (none when it cannot be read).
fn day_files_in(dir: &Path) -> Vec<(NaiveDate, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            day_files::date_of_file(STEM, &name).map(|date| (date, entry.path()))
        })
        .collect()
}

/// The minute lines of a day file, one at a time (none when it cannot be
/// opened; a line that does not parse is skipped).
fn lines_of(path: &Path) -> impl Iterator<Item = MinuteLine> {
    std::fs::File::open(path)
        .ok()
        .map(BufReader::new)
        .into_iter()
        .flat_map(|reader| reader.lines().map_while(Result::ok))
        .filter_map(|line| MinuteLine::parse(&line))
}

/// The point's day in a day file: its first and last second heard.
fn day_of(path: &Path, instance: &str, target: &str) -> Option<HistoryDay> {
    lines_of(path)
        .filter_map(|line| span_of(line.m, &line.slots(instance, target)?))
        .fold(None, |day, span| Some(widen(day, span)))
}

/// Fills `slots` (one a second from `start`) from the day files of `dir`
/// for the seconds before `end`.
fn read_window(dir: &Path, point: &Point, start: i64, end: i64, slots: &mut [i16]) {
    if end <= start {
        return;
    }
    let first = day_files::date_of(start);
    let last = day_files::date_of(end - 1);
    let days = last.signed_duration_since(first).num_days();
    for k in 0..=days {
        let Some(date) = first.checked_add_days(chrono::Days::new(k as u64)) else {
            continue;
        };
        let path = dir.join(day_files::file_name(STEM, date));
        for line in lines_of(&path) {
            if line.m + MINUTE_MS <= start || line.m >= end {
                continue;
            }
            if let Some(values) = line.slots(&point.0, &point.1) {
                place(line.m, &values, start, end, slots);
            }
        }
    }
}

/// Puts a minute's values starting at `m` into `slots` (one a second from
/// `start`), the seconds from `start` to `end` only.
pub fn place(m: i64, values: &[i16], start: i64, end: i64, slots: &mut [i16]) {
    for (i, value) in values.iter().enumerate() {
        let t = m + i as i64 * 1000;
        if t < start || t >= end {
            continue;
        }
        if let Some(slot) = slots.get_mut(((t - start) / 1000) as usize) {
            *slot = *value;
        }
    }
}

/// The `lufs_history` record of a minute line's write: one when the line
/// took its day file past the cap (warn class), none otherwise.
pub fn cap_record(appended: Appended, date: NaiveDate, cap: u64) -> Option<Value> {
    (appended == Appended::CapReached).then(|| {
        json!({
            "what": "cap",
            "date": date.format("%Y-%m-%d").to_string(),
            "cap_bytes": cap,
        })
    })
}

/// The writer: the history's day files.
struct LufsWriter {
    files: DayFiles,
    events: EventLog,
}

impl LufsWriter {
    fn new(dir: PathBuf, limits: Limits, events: EventLog) -> Self {
        Self {
            files: DayFiles::new(dir, STEM, "LUFS history", limits),
            events,
        }
    }
}

impl Sink<MinuteLine> for LufsWriter {
    fn take(&mut self, line: MinuteLine) {
        let text = format!("{}\n", line.text());
        let appended = self.files.append(line.m, &text, None, false);
        let date = day_files::date_of(line.m);
        if let Some(record) = cap_record(appended, date, self.files.limits().day_cap) {
            tracing::warn!(date = %date, "LUFS history: the day file reached its cap; nothing more is kept that day");
            self.events.record("lufs_history", record);
        }
    }

    fn flush(&mut self) {
        self.files.flush();
    }

    fn sweep(&mut self, today: NaiveDate) {
        self.files.sweep(today);
    }
}

#[cfg(test)]
#[path = "lufs_history/tests.rs"]
mod tests;
