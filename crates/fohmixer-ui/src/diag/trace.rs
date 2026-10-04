//! The page's flight recorder (#43, design note §5.2): a ring of what the
//! page itself saw, uploaded to the hub's event log as `trace` batches, so an
//! outage is recorded from the page's side too (the hub cannot log what never
//! reached it).
//!
//! Its events, each with `ev` and the page clock `t` (ms since the epoch, the
//! clock of `set` and `ping`): a [`touch`] (down, up, cancel; a toggle's
//! tap) with the control's keys, and for a finger on a fader or a pan where
//! its touch started and each frame's moves ([`moves`], PR D); a [`send`]
//! only when the socket did not take it (a set the socket took is the hub's
//! own `set` record, same `t` and `seq`); each [`ack`] as its arrival (`t`,
//! `seq`: the hub's `ack` record has the rest); a per-second summary of the
//! pongs' round trips ([`rtt`]: the hub logs every ping, so a page event per
//! pong only doubled the volume, 204 812 of them at the service of
//! 2026-10-04); the socket's transitions ([`sock`]), a frame longer than
//! [`LONG_FRAME_MS`] (the page's main thread stalled, [`Recorder::frame`]), a
//! visibility change ([`Recorder::visibility`]), and the dropout watch's
//! `dropout` and `reset` (`behave::link`). No page event has a field named
//! `ts`: the forensics timeline reads a record's hub `ts` as the first
//! `"ts":` of its line, and a `trace` line's `events` come before it.
//!
//! **Upload** ([`Recorder::upload`], from the link's 100 ms tick): one batch
//! of at most [`BATCH_BYTES`] with its envelope (one small WebSocket frame:
//! every set behind it waits for it) and at most [`RATE_BYTES_PER_S`] (each
//! batch waits for the previous one's bytes at that rate, a full one less
//! than a tick), only when
//!
//! - no set went onto the socket since the previous tick
//!   ([`Recorder::set_went`]): while a finger moves a fader the recorder
//!   waits, and its events go when the fingers rest;
//! - the socket holds at most [`BUFFERED_MAX`] unsent: a ping still leaving
//!   never holds it (at the service the iPad's WebKit reported a ping's bytes
//!   in `bufferedAmount` at every tick for minutes, and a gate on 0 sent
//!   nothing for up to 9 minutes), a backed-up socket does;
//! - something waits and it is due: a full batch (events that fill a
//!   message) as soon as the rate lets it, smaller amounts every
//!   [`UPLOAD_MS`], and the first batch after a hello, a dropout or a reset
//!   ([`Recorder::soon`]) as soon as the rate lets it.
//!
//! Oldest first, so after a reconnect the backlog goes first. A batch stays
//! in the ring until the pong of a ping sent after it: the hub reads a
//! socket's messages in order, so that pong proves the batch reached the
//! event log ([`Recorder::proved`]). A socket lost first sends it again after
//! the next hello ([`Recorder::requeue`]); the log may then hold an exact
//! duplicate, which the forensics timeline drops. A reload loses what was not
//! proved (§5.2: the page keeps nothing in browser storage).
//!
//! **Bounds:** past [`BACKLOG_BYTES`] of unsent events the oldest
//! non-essential unsent events go (so the newest reaches the hub within
//! seconds: [`BACKLOG_BYTES`] at [`RATE_BYTES_PER_S`]); the essential ones
//! ([`is_essential`]: touches, dropouts, resets, socket transitions,
//! visibility, long frames, an unsent write, and a touch's first
//! [`moves::FIRST_MOVES`] frames, [`Recorder::push_essential`]) stay. Past the
//! ring's hard bounds ([`MAX_EVENTS`], [`MAX_BYTES`]) the oldest unsent event
//! goes whatever it is. The next batch says what went, per kind (an
//! `overflow` event).
//!
//! Pure, tested natively: `diag.rs` keeps the page's one recorder, the store
//! (`store/live/link.rs`) uploads it.

use std::collections::{BTreeMap, VecDeque};

use fohmixer_proto::client::{AckItem, ClientMsg, set_key};
use serde_json::{Value, json};

pub mod moves;
pub mod rtt;

use moves::round1;
use rtt::RttWindow;

/// The most events the ring keeps.
pub const MAX_EVENTS: usize = 20_000;
/// The most bytes of event JSON the ring keeps.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
/// Smaller amounts go at most this often (ms), unless a batch is due at once.
pub const UPLOAD_MS: f64 = 2000.0;
/// A batch's message is at most this many bytes, its events joined plus
/// the `trace` envelope (at least one event, however large): one WebSocket
/// frame, and a set sent right after it waits for it, so it is small (about
/// one TCP segment). At [`RATE_BYTES_PER_S`] a full batch waits 97.7 ms, so
/// it goes at the next 100 ms tick with a margin for a coarse clock.
pub const BATCH_BYTES: usize = 1000;
/// The bytes of a `trace` message around its events ([`batch_text`]).
pub const ENVELOPE: usize = 28;
/// The recorder's upload never exceeds this many bytes a second (each batch
/// waits for the previous one's bytes at this rate): far below a slow
/// Wi-Fi link, and it sends nothing while a finger moves a fader.
pub const RATE_BYTES_PER_S: f64 = 10_240.0;
/// Past this many bytes of unsent events the oldest non-essential ones go,
/// so the newest event reaches the hub within about 5 s of the fingers
/// resting (48 KB at [`RATE_BYTES_PER_S`]).
pub const BACKLOG_BYTES: usize = 48 * 1024;
/// A socket that holds more unsent bytes than this is backed up: no batch
/// goes. A ping or two still leaving is less.
pub const BUFFERED_MAX: u32 = 1024;
/// A frame longer than this (ms) is recorded: the page's main thread
/// stalled.
pub const LONG_FRAME_MS: f64 = 50.0;

/// Whether a frame gap of `gap` ms is long.
pub fn is_long_frame(gap: f64) -> bool {
    gap > LONG_FRAME_MS
}

/// Whether a socket holding `buffered` unsent bytes takes a batch.
pub fn takes_batch(buffered: u32) -> bool {
    buffered <= BUFFERED_MAX
}

/// Whether an event of kind `ev` stays when the backlog is over its bound:
/// what the forensics need of every touch and outage (touches, dropouts,
/// resets, socket transitions, visibility, long frames, the notes of what
/// went, and a write the socket did not take). Moves, acks and round trips
/// go first; a touch's first moves are pushed as essential
/// ([`Recorder::push_essential`]).
pub fn is_essential(ev: &str) -> bool {
    matches!(
        ev,
        "touch" | "dropout" | "reset" | "sock" | "visibility" | "frame" | "overflow" | "send"
    )
}

/// A touch's event: `what` (`down`, `up`, `cancel`; `tap` for a toggle,
/// which no up follows) on the control writing `keys`, by pointer `pointer`.
/// A fader's or pan's taken down is [`moves::touch_start`].
pub fn touch(t: f64, what: &str, keys: &[String], pointer: i32) -> Value {
    json!({"ev": "touch", "t": t, "what": what, "keys": keys, "pointer": pointer})
}

/// A write's event, only when the socket did not take it (`sent` false: it
/// is kept and sent again later as a new `set`, and the hub never saw it):
/// its key, sequence, value and whether it was final. A taken write is the
/// hub's `set` record (the same `t` and `seq`); nothing for another message.
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
    if sent {
        return None;
    }
    Some(json!({
        "ev": "send",
        "t": t,
        "key": set_key(instance, target, prop),
        "seq": seq,
        "value": value,
        "final": is_final,
        "sent": false,
    }))
}

/// The sequence number of a `set`; none for another message.
pub fn seq_of(msg: &ClientMsg) -> Option<u64> {
    match msg {
        ClientMsg::Set { seq, .. } => Some(*seq),
        _ => None,
    }
}

/// An ack item's arrival at `t`: its sequence only (per page, it names the
/// write; the hub's `ack` record holds its key, error and superseded).
pub fn ack(t: f64, item: &AckItem) -> Value {
    json!({"ev": "ack", "t": round1(t), "seq": item.seq})
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

/// One event in the ring: its JSON, its kind and whether it stays when the
/// backlog is over its bound.
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    text: String,
    kind: String,
    essential: bool,
}

/// The page's flight recorder.
#[derive(Debug, Default)]
pub struct Recorder {
    /// The events not proved logged yet, oldest first.
    events: VecDeque<Entry>,
    /// The size of their JSON.
    bytes: usize,
    /// The size of the unsent ones' JSON (after the first `sent`).
    unsent: usize,
    /// How many of the unsent ones are not essential (the backlog's bound
    /// has nothing to drop without one).
    optional: usize,
    /// The first `sent` events went in batches not proved yet.
    sent: usize,
    /// Those batches, oldest first: the ping whose pong proves each, and its
    /// number of events.
    flights: VecDeque<(u32, usize)>,
    /// Unsent events dropped since the last batch, per kind.
    overflow: BTreeMap<String, u64>,
    /// When the last batch went (page ms); none yet.
    uploaded: Option<f64>,
    /// The rate cap: no batch before this (page ms).
    paced_until: Option<f64>,
    /// A batch goes as soon as the rate lets it (a hello, a dropout, a reset).
    soon: bool,
    /// A set went onto the socket since the last tick.
    set_went: bool,
    /// The next frame's gap follows a visibility change: it is the time the
    /// page was hidden, not a stall.
    after_visibility: bool,
    /// The pongs of the current second.
    rtt: RttWindow,
}

impl Recorder {
    /// Records `event`, essential by its kind ([`is_essential`]).
    pub fn push(&mut self, event: &Value) {
        let essential = is_essential(event["ev"].as_str().unwrap_or_default());
        self.add(event, essential);
    }

    /// Records `event` as essential whatever its kind (a touch's first
    /// moves).
    pub fn push_essential(&mut self, event: &Value) {
        self.add(event, true);
    }

    fn add(&mut self, event: &Value, essential: bool) {
        let text = event.to_string();
        self.make_room(text.len());
        self.bytes += text.len();
        self.unsent += text.len();
        self.optional += usize::from(!essential);
        self.events.push_back(Entry {
            kind: event["ev"].as_str().unwrap_or_default().to_string(),
            text,
            essential,
        });
        self.bound_backlog();
    }

    /// Whether `len` more bytes overfill the ring.
    fn full(&self, len: usize) -> bool {
        self.events.len() >= MAX_EVENTS || self.bytes + len > MAX_BYTES
    }

    /// The ring's hard bounds: drops the oldest unsent events, whatever they
    /// are, until `len` more bytes fit (a batch on its way stays: it is at
    /// most a batch or two over the bound).
    fn make_room(&mut self, len: usize) {
        for _ in 0..self.events.len() {
            if !self.full(len) {
                return;
            }
            let Some(gone) = self.events.remove(self.sent) else {
                return;
            };
            self.forget(&gone);
        }
    }

    /// An unsent event went: its bytes leave, its kind is counted.
    fn forget(&mut self, gone: &Entry) {
        self.bytes -= gone.text.len();
        self.unsent -= gone.text.len();
        self.optional -= usize::from(!gone.essential);
        *self.overflow.entry(gone.kind.clone()).or_default() += 1;
    }

    /// Past [`BACKLOG_BYTES`] of unsent events, the oldest non-essential
    /// unsent ones go until the backlog is within it again.
    fn bound_backlog(&mut self) {
        let mut excess = self.unsent.saturating_sub(BACKLOG_BYTES);
        if excess == 0 {
            return;
        }
        // Nothing to drop: every unsent event is essential.
        if self.optional == 0 {
            return;
        }
        let sent = self.sent;
        let mut gone = Vec::new();
        let mut index = 0;
        self.events.retain(|e| {
            let keep = index < sent || e.essential || excess == 0;
            index += 1;
            if !keep {
                excess = excess.saturating_sub(e.text.len());
                gone.push(e.clone());
            }
            keep
        });
        for entry in &gone {
            self.forget(entry);
        }
    }

    /// A batch goes as soon as the rate lets it (after a hello, a dropout, a
    /// reset).
    pub fn soon(&mut self) {
        self.soon = true;
    }

    /// A set went onto the socket: no batch goes at the next tick (it would
    /// wait in front of the next frame's set).
    pub fn set_went(&mut self) {
        self.set_went = true;
    }

    /// The pong of a ping came at `t`, `rtt` ms after the ping: into the
    /// second's summary (the one it closes goes into the ring).
    pub fn pong(&mut self, t: f64, rtt: f64) {
        if let Some(summary) = self.rtt.add(t, rtt) {
            self.push(&summary);
        }
    }

    /// Whether something waits: unsent events, or a note of what went.
    fn waiting(&self) -> bool {
        self.events.len() > self.sent || !self.overflow.is_empty()
    }

    /// Whether a batch is due at `now`: the rate lets it, and it goes at
    /// once ([`Recorder::soon`]), a full batch waits, or [`UPLOAD_MS`]
    /// passed since the last one.
    fn due(&self, now: f64) -> bool {
        let paced = self.paced_until.is_none_or(|at| now >= at);
        // The unsent events fill a message: their JSON, the commas between
        // them and the envelope.
        let commas = (self.events.len() - self.sent).saturating_sub(1);
        let full = self.unsent + commas + ENVELOPE >= BATCH_BYTES;
        let time = self.soon || full || self.uploaded.is_none_or(|at| now - at >= UPLOAD_MS);
        paced && time
    }

    /// How many unsent events the next batch takes: as many as fit
    /// [`BATCH_BYTES`] joined by commas inside the envelope, at least one.
    fn batch_count(&self) -> usize {
        let mut size = ENVELOPE;
        let mut count = 0;
        for event in self.events.iter().skip(self.sent) {
            let joined = size + event.text.len() + usize::from(count > 0);
            if count > 0 && joined > BATCH_BYTES {
                break;
            }
            size = joined;
            count += 1;
        }
        count
    }

    /// The next batch at `now` (a `trace` message), when one goes: the
    /// socket said hello (`ready`), no set went since the last tick, the
    /// socket holds at most [`BUFFERED_MAX`] unsent (`buffered`), something
    /// waits and it is due ([`Recorder::due`]). `proof` is the number of the
    /// next ping, whose pong proves the batch was logged. Each tick calls it
    /// once: it closes a second of round trips that is over, and forgets the
    /// sets of the tick. What went is said first, as an `overflow` event.
    pub fn upload(&mut self, now: f64, ready: bool, buffered: u32, proof: u32) -> Option<String> {
        if let Some(summary) = self.rtt.close(now) {
            self.push(&summary);
        }
        let quiet = !std::mem::take(&mut self.set_went);
        if !(ready && quiet && takes_batch(buffered) && self.waiting() && self.due(now)) {
            return None;
        }
        if !self.overflow.is_empty() {
            let kinds = std::mem::take(&mut self.overflow);
            let n: u64 = kinds.values().sum();
            let text = json!({"ev": "overflow", "t": now, "n": n, "kinds": kinds}).to_string();
            self.bytes += text.len();
            self.unsent += text.len();
            self.events.insert(
                self.sent,
                Entry {
                    text,
                    kind: "overflow".to_string(),
                    essential: true,
                },
            );
        }
        let count = self.batch_count();
        let batch: Vec<&Entry> = self.events.iter().skip(self.sent).take(count).collect();
        let size: usize = batch.iter().map(|e| e.text.len()).sum();
        let optional = batch.iter().filter(|e| !e.essential).count();
        let events: Vec<&str> = batch.iter().map(|e| e.text.as_str()).collect();
        let text = batch_text(&events);
        self.sent += count;
        self.unsent -= size;
        self.optional -= optional;
        self.flights.push_back((proof, count));
        self.uploaded = Some(now);
        self.paced_until = Some(now + text.len() as f64 * 1000.0 / RATE_BYTES_PER_S);
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
                self.bytes -= gone.text.len();
            }
            self.sent -= count;
        }
    }

    /// The socket closed, or a batch could not be sent: the batches not
    /// proved go again, first, as soon as a socket takes them.
    pub fn requeue(&mut self) {
        let unproved = self.events.iter().take(self.sent);
        self.unsent += unproved.clone().map(|e| e.text.len()).sum::<usize>();
        self.optional += unproved.filter(|e| !e.essential).count();
        self.sent = 0;
        self.flights.clear();
        self.soon = true;
        self.bound_backlog();
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

    /// The bytes of unsent event JSON (the backlog).
    pub fn backlog(&self) -> usize {
        self.unsent
    }
}

#[cfg(test)]
mod tests;
