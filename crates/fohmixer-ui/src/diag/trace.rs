//! The page's flight recorder (#43, design note §5.2): a ring of what the
//! page itself saw, uploaded to the hub's event log as `trace` batches, so an
//! outage is recorded from the page's side too (the hub cannot log what never
//! reached it).
//!
//! Its events, each with `ev` and the page clock `t` (ms since the epoch, the
//! clock of `set` and `ping`): a [`touch`] (down, up, cancel) with the
//! control's keys, each [`send`] (whether the socket took it) and [`ack`],
//! each [`ping`] and [`pong`] with its round trip, the socket's transitions
//! ([`sock`]), a frame longer than [`LONG_FRAME_MS`] (the page's main thread
//! stalled, [`Recorder::frame`]), a visibility change
//! ([`Recorder::visibility`]), and the dropout watch's `dropout` and `reset`
//! (`behave::link`). The ring keeps at most [`MAX_EVENTS`] events and
//! [`MAX_BYTES`] of their JSON; past either the oldest unsent event goes, and
//! the next batch says how many went (an `overflow` event).
//!
//! Upload ([`Recorder::upload`]): one batch of at most [`BATCH_BYTES`] every
//! [`UPLOAD_MS`] while the socket said hello, and at once after a hello, a
//! dropout or a reset ([`Recorder::soon`]), oldest first, so after a
//! reconnect the backlog goes first. Only while the socket holds nothing
//! unsent: the recorder never queues in front of the moves on a slow link. A
//! batch stays in the ring until the pong of a ping sent after it: the hub
//! reads a socket's messages in order, so that pong proves the batch reached
//! the event log ([`Recorder::proved`]). A socket lost first sends it again
//! after the next hello ([`Recorder::requeue`]); the log may then hold an
//! exact duplicate, which the forensics timeline drops. A reload loses what
//! was not proved (§5.2: the page keeps nothing in browser storage).
//!
//! Pure, tested natively: `diag.rs` keeps the page's one recorder, and the
//! store (`store/live/link.rs`) uploads it.

use std::collections::VecDeque;

use fohmixer_proto::client::{AckItem, ClientMsg, set_key};
use serde_json::{Value, json};

/// The most events the ring keeps.
pub const MAX_EVENTS: usize = 20_000;
/// The most bytes of event JSON the ring keeps.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
/// A batch goes at most this often (ms), unless one is due at once.
pub const UPLOAD_MS: f64 = 2000.0;
/// A batch carries at most this many bytes of event JSON (at least one
/// event): about 0.13 s of a 2 Mbit/s link, so it never holds the moves up
/// for long.
pub const BATCH_BYTES: usize = 32 * 1024;
/// A frame longer than this (ms) is recorded: the page's main thread
/// stalled.
pub const LONG_FRAME_MS: f64 = 50.0;

/// Whether a frame gap of `gap` ms is long.
pub fn is_long_frame(gap: f64) -> bool {
    gap > LONG_FRAME_MS
}

/// A touch's event: `what` (`down`, `up`, `cancel`) on the control writing
/// `keys`, by pointer `pointer`.
pub fn touch(t: f64, what: &str, keys: &[String], pointer: i32) -> Value {
    json!({"ev": "touch", "t": t, "what": what, "keys": keys, "pointer": pointer})
}

/// A write's event: the `set` it sent (its own page time), and whether the
/// socket took it (an unsent write is kept and sent again later as a new
/// `set`). Nothing for another message.
pub fn send(msg: &ClientMsg, sent: bool) -> Option<Value> {
    let ClientMsg::Set {
        instance,
        target,
        prop,
        value,
        seq,
        t,
        is_final,
    } = msg
    else {
        return None;
    };
    Some(json!({
        "ev": "send",
        "t": t,
        "key": set_key(instance, target, prop),
        "seq": seq,
        "value": value,
        "final": is_final,
        "sent": sent,
    }))
}

/// An ack item's event, as it arrived at `t`: its key and sequence, and why
/// it failed or that it was superseded, when so.
pub fn ack(t: f64, item: &AckItem) -> Value {
    let mut event = json!({"ev": "ack", "t": t, "key": item.key, "seq": item.seq});
    if let Some(error) = &item.error {
        event["error"] = json!(error);
    }
    if item.superseded {
        event["superseded"] = json!(true);
    }
    event
}

/// Ping `n` went at `t`.
pub fn ping(t: f64, n: u32) -> Value {
    json!({"ev": "ping", "t": t, "n": n})
}

/// The pong of ping `n` came at `t`, `rtt` ms after the ping.
pub fn pong(t: f64, n: u32, rtt: f64) -> Value {
    json!({"ev": "pong", "t": t, "n": n, "rtt": rtt})
}

/// A socket transition at `t`: `what` (`open`, `hello`, `close`, `drop`,
/// `fail`), the socket's number, the close code, why.
pub fn sock(
    t: f64,
    what: &str,
    socket: Option<u64>,
    code: Option<u16>,
    reason: Option<&str>,
) -> Value {
    let mut event = json!({"ev": "sock", "t": t, "what": what});
    if let Some(socket) = socket {
        event["socket"] = json!(socket);
    }
    if let Some(code) = code {
        event["code"] = json!(code);
    }
    if let Some(reason) = reason {
        event["reason"] = json!(reason);
    }
    event
}

/// A `trace` message of `events` (each one event's JSON).
pub fn batch_text(events: &[&str]) -> String {
    format!(r#"{{"type":"trace","events":[{}]}}"#, events.join(","))
}

/// The page's flight recorder.
#[derive(Debug, Default)]
pub struct Recorder {
    /// The events not proved logged yet, oldest first, as JSON.
    events: VecDeque<String>,
    /// The size of their JSON.
    bytes: usize,
    /// The first `sent` events went in batches not proved yet.
    sent: usize,
    /// Those batches, oldest first: the ping whose pong proves each, and its
    /// number of events.
    flights: VecDeque<(u32, usize)>,
    /// Unsent events the full ring dropped since the last batch.
    overflow: u64,
    /// When the last batch went (page ms); none yet.
    uploaded: Option<f64>,
    /// A batch goes at once (a hello, a dropout, a reset).
    soon: bool,
    /// The next frame's gap follows a visibility change: it is the time the
    /// page was hidden, not a stall.
    after_visibility: bool,
}

impl Recorder {
    /// Records `event`; when the ring is full the oldest unsent events go
    /// (counted for the next batch's `overflow`).
    pub fn push(&mut self, event: &Value) {
        let text = event.to_string();
        self.make_room(text.len());
        self.bytes += text.len();
        self.events.push_back(text);
    }

    /// Whether `len` more bytes overfill the ring.
    fn full(&self, len: usize) -> bool {
        self.events.len() >= MAX_EVENTS || self.bytes + len > MAX_BYTES
    }

    /// Drops the oldest unsent events until `len` more bytes fit (a batch on
    /// its way stays: it is at most a batch or two over the bound).
    fn make_room(&mut self, len: usize) {
        for _ in 0..self.events.len() {
            if !self.full(len) {
                return;
            }
            let Some(gone) = self.events.remove(self.sent) else {
                return;
            };
            self.bytes -= gone.len();
            self.overflow += 1;
        }
    }

    /// A batch goes at once (after a hello, a dropout, a reset).
    pub fn soon(&mut self) {
        self.soon = true;
    }

    /// Whether a batch goes at `now`: the socket said hello (`ready`) and
    /// holds nothing unsent (`buffered` bytes), something waits, and it is
    /// due ([`UPLOAD_MS`] since the last one, or at once).
    fn due(&self, now: f64, ready: bool, buffered: u32) -> bool {
        let waiting = self.events.len() > self.sent || self.overflow > 0;
        let time = self.soon || self.uploaded.is_none_or(|at| now - at >= UPLOAD_MS);
        ready && buffered == 0 && waiting && time
    }

    /// How many unsent events the next batch takes: as many as fit
    /// [`BATCH_BYTES`] of JSON joined by commas, at least one.
    fn batch_count(&self) -> usize {
        let mut size = 0;
        let mut count = 0;
        for event in self.events.iter().skip(self.sent) {
            let joined = size + event.len() + usize::from(count > 0);
            if count > 0 && joined > BATCH_BYTES {
                break;
            }
            size = joined;
            count += 1;
        }
        count
    }

    /// The next batch at `now` (a `trace` message), when one is due
    /// ([`Recorder::due`]); `proof` is the number of the next ping, whose
    /// pong proves the batch was logged. The full ring's drops go first, as
    /// an `overflow` event.
    pub fn upload(&mut self, now: f64, ready: bool, buffered: u32, proof: u32) -> Option<String> {
        if !self.due(now, ready, buffered) {
            return None;
        }
        if self.overflow > 0 {
            let note = json!({"ev": "overflow", "t": now, "n": self.overflow}).to_string();
            self.bytes += note.len();
            self.events.insert(self.sent, note);
            self.overflow = 0;
        }
        let count = self.batch_count();
        let events: Vec<&str> = self
            .events
            .iter()
            .skip(self.sent)
            .take(count)
            .map(String::as_str)
            .collect();
        let text = batch_text(&events);
        self.sent += count;
        self.flights.push_back((proof, count));
        self.uploaded = Some(now);
        self.soon = false;
        Some(text)
    }

    /// The pong of ping `n` came: every batch sent before a ping up to `n`
    /// reached the event log, and its events leave the ring.
    pub fn proved(&mut self, n: u32) {
        for _ in 0..self.flights.len() {
            let Some(&(proof, count)) = self.flights.front() else {
                return;
            };
            if proof > n {
                return;
            }
            self.flights.pop_front();
            for gone in self.events.drain(..count) {
                self.bytes -= gone.len();
            }
            self.sent -= count;
        }
    }

    /// The socket closed, or a batch could not be sent: the batches not
    /// proved go again, first, as soon as a socket takes them.
    pub fn requeue(&mut self) {
        self.sent = 0;
        self.flights.clear();
        self.soon = true;
    }

    /// A frame `gap` ms after the one before, at page time `t`: recorded
    /// when long, unless it is the first after a visibility change.
    pub fn frame(&mut self, t: f64, gap: f64) {
        if std::mem::take(&mut self.after_visibility) {
            return;
        }
        if is_long_frame(gap) {
            self.push(&json!({"ev": "frame", "t": t, "ms": gap}));
        }
    }

    /// The page was hidden or shown at `t`.
    pub fn visibility(&mut self, t: f64, hidden: bool) {
        self.after_visibility = true;
        self.push(&json!({"ev": "visibility", "t": t, "hidden": hidden}));
    }

    /// How many events the ring holds (proved ones leave it).
    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[cfg(test)]
mod tests;
