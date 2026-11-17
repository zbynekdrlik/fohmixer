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
//! page's `trace`: a batch of its flight recorder (PR C: touches, sends and
//! acks, pongs, socket transitions, long frames, visibility, the full ring's
//! `overflow` notes, the dropouts and the counter's resets), written as the
//! page sent it. The Stream Deck's (#52): `deck_link`, `deck_press`,
//! `deck_ok`, `deck_release`, `deck_key`, `deck_keys`, `deck_view` (an image
//! only as its hash and size). A page going idle or waking (#51): `idle`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

use crate::day_files::UtcDate;
use serde_json::{Map, Value, json};

use crate::day_files::{self, DayFiles, Sink};
pub use crate::day_files::{Limits, SWEEP_EVERY, date_of, expired, over_cap};

/// Records waiting for the writer before new ones are dropped.
pub const QUEUE: usize = 8192;
/// Day files older than this many days are deleted.
pub const KEEP_DAYS: i64 = 60;
/// A day file's size past which only warn-class records are written.
pub const DAY_CAP: u64 = 256 * 1024 * 1024;
/// The event log's limits.
pub const LIMITS: Limits = Limits {
    keep_days: KEEP_DAYS,
    day_cap: DAY_CAP,
};
/// The day files' stem: `events-YYYY-MM-DD.jsonl`.
pub const STEM: &str = "events";

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
        Self::start_with(dir, LIMITS).map(|(log, _)| log)
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
            .spawn(move || day_files::run(Writer::new(dir, limits).with_dropped(dropped), &rx))?;
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

/// The day file of `date`.
pub fn file_name(date: UtcDate) -> String {
    day_files::file_name(STEM, date)
}

/// The date of a day file's name; `None` for any other file.
pub fn date_of_file(name: &str) -> Option<UtcDate> {
    day_files::date_of_file(STEM, name)
}

/// Whether a record is still written past a day file's cap: a socket or a
/// link change, the Stream Deck's link changes, its presses with
/// Companion's answers, its tab opened or closed and the releases the hub
/// made itself (#52: rare, tiny, and the most consequential deck records),
/// the Pro-Q 4 editors' opens, closes, locks and contacts (#71 PR E, `eq`),
/// a page going idle or waking (#51, `idle`: rare, and it explains a page's
/// slow pings and paced meters), a page's `trace` that holds a dropout or a counter reset (the owner's
/// priority on #43), the cap and dropped notes, and anything that carries
/// an error. The flight recorder's other batches stop at the cap like the
/// pings, the writes and the deck's key images (`deck_key`, `deck_keys`):
/// a page left open for a day would otherwise write past it without bound
/// (PR C). AbleSet's name (#90 B3h): an `ableset_proxy` record of an
/// AbleSet outage carries `error` (only its warned one, at most one per
/// 10 s), a client's fault `why` (rationed), and an `ableset_ws` open or
/// close never an `error`, nor an `ableset_lock` (the tab's lock page could
/// not store AbleSet's lock, #90: the page's own word, `why`, rationed):
/// only the outage outlives the cap, so no client fills the warn class. Its link's changes (`ableset_link`, #90 B3: at
/// most one per 5 s read, they explain a page's AbleSet tab) do too, and
/// so does a page's `trace` that holds its AbleSet tab opened or closed
/// (`ableset_view`, #90 design §6: a few per open, beside the dropouts),
/// and one that holds a rail button's step (`rail`, #130 PR 6: a few per
/// press, the press the rules' records below answer) or a pick in the
/// column (`tab`, #212: a few an hour, what the column's use is counted
/// from).
/// The rules' records (#104, #130, D18): every instance's `transport`
/// change, STAGE AUT's `stage_aut` (its flag, its writes and their
/// failures), REVERB's `reverb` steps, each fader link's switch
/// (`fader_link`) and AUTOTUNE's `autotune` steps (#104): a few per song,
/// and they explain what the stage mics, the reverbs, the links and the
/// singers' autotune did. A `songmap` read (#140) only when its problems
/// changed since the set's last read (`problems_changed`; the first read
/// after a connect: when it has any) or it failed (its `error`): a read at
/// every transport change whose problems are known and unchanged is bulk
/// (review M8). The LUFS history's day cap (`lufs_history`, #219): at most
/// one a day.
pub fn is_warn(record: &Value) -> bool {
    let ev = record.get("ev").and_then(Value::as_str).unwrap_or("");
    let failed = record.get("error").is_some_and(|e| !e.is_null())
        || record
            .get("errors")
            .and_then(Value::as_u64)
            .is_some_and(|n| n > 0);
    failed
        || matches!(
            ev,
            "sock"
                | "link"
                | "cap"
                | "dropped"
                | "deck_link"
                | "deck_release"
                | "deck_press"
                | "deck_ok"
                | "deck_view"
                | "eq"
                | "idle"
                | "ableset_link"
                | "presenter_link"
                | "presenter_switch"
                | "transport"
                | "stage_aut"
                | "reverb"
                | "rec"
                | "fader_link"
                | "autotune"
                | "sectors"
                | "colors"
                | "mute_alarms"
                | "mute_guards"
                | "scene"
                | "scene_rule"
                | "scene_glide"
                | "scene_source"
                | "script"
                | "listen"
                | "lufs_history"
                | "claps"
                | "kept_pushes"
        )
        || (ev == "trace" && holds_warn_event(record))
        || (ev == "songmap" && problems_changed(record))
        || (ev == "lufs" && record.get("warn") == Some(&Value::Bool(true)))
}

/// Whether a song map read's record says its problems changed.
fn problems_changed(record: &Value) -> bool {
    record.get("problems_changed").and_then(Value::as_bool) == Some(true)
}

/// Whether a page's `trace` holds a dropout, a reset of its counter, its
/// AbleSet tab opened or closed, a rail button's step, or a pick in the
/// column (`tab`, #212).
fn holds_warn_event(record: &Value) -> bool {
    record
        .get("events")
        .and_then(Value::as_array)
        .is_some_and(|events| {
            events.iter().any(|e| {
                matches!(
                    e.get("ev").and_then(Value::as_str),
                    Some("dropout" | "reset" | "ableset_view" | "presenter" | "rail" | "tab")
                )
            })
        })
}

/// The hub log's note on a write error: only the first of a run of
/// failures is noted (`was_failing`: the previous write failed too).
pub fn failure_note(was_failing: bool, error: &std::io::Error) -> Option<String> {
    day_files::failure_note("event log", was_failing, error)
}

/// The writer: the event log's day files.
pub struct Writer {
    files: DayFiles,
    /// The producers' count of dropped records (the thread's; a test hands
    /// its own to [`Writer::take`]).
    dropped: Arc<AtomicU64>,
}

impl Writer {
    pub fn new(dir: PathBuf, limits: Limits) -> Self {
        Self {
            files: DayFiles::new(dir, STEM, "event log", limits),
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }

    /// The writer with the producers' count of dropped records.
    fn with_dropped(mut self, dropped: Arc<AtomicU64>) -> Self {
        self.dropped = dropped;
        self
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

    /// Writes `record`; at the cap a `cap` record goes first, and past it
    /// only warn-class records ([`is_warn`]).
    fn write(&mut self, record: &Value) {
        let ts = ts_of(record);
        let cap = self.files.limits().day_cap;
        let note = format!("{}\n", stamp("cap", ts, json!({"cap_bytes": cap})));
        let line = format!("{record}\n");
        self.files.append(ts, &line, Some(&note), is_warn(record));
    }

    /// Writes out what the open day file buffers.
    pub fn flush(&mut self) {
        self.files.flush();
    }

    /// Deletes the day files past retention on `today`; how many it
    /// deleted.
    pub fn sweep(&mut self, today: UtcDate) -> usize {
        self.files.sweep(today)
    }
}

impl Sink<Value> for Writer {
    fn take(&mut self, record: Value) {
        let dropped = Arc::clone(&self.dropped);
        Writer::take(self, &record, &dropped);
    }

    fn flush(&mut self) {
        Writer::flush(self);
    }

    fn sweep(&mut self, today: UtcDate) {
        Writer::sweep(self, today);
    }
}

#[cfg(test)]
#[path = "events/tests.rs"]
mod tests;
