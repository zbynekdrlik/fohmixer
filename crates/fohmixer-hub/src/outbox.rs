//! A client connection's outbox (S3 design note §3; spec §2.4 fan-out): what
//! waits to be written to one client, bounded.
//!
//! Values keep only the latest item per subscription while a send is
//! pending (a slow client costs memory bounded by its subscriptions, never
//! a queue), and so do the hub values. Replies (`result`, `subbed`,
//! `error`) and instance states keep their order: a client sees an
//! instance go offline and come back even when its writer was stuck across
//! the reconnect, and a `subbed` of the new session never comes before the
//! old session's `offline`. An instance going offline drops its pending
//! values (they belong to the old session). More than [`MAX_REPLIES`]
//! waiting means the client stopped reading: the outbox closes and the
//! connection ends (the client reconnects and resyncs). The router writes
//! here under a short lock and never waits for a client.
//!
//! The acks of a client's writes (#43) are coalesced like the values: per
//! write key the ack of the highest `seq` waits (a late result of an older
//! write never hides that a newer one was superseded), sent after the
//! values (a page then sees Live's new value before the ack that closes its
//! intent). Each instance's `link` (Live's health) is coalesced too: only
//! the latest one waits.
//!
//! The Stream Deck's keys (#52) are coalesced too, the latest state per key,
//! and written last, after the acks: a viewer's key images never hold up a
//! fader's ack, and only a client viewing the tab gets them (the router
//! decides; a closed tab drops what waits, `forget_deck`). They go at most
//! [`DECK_KEYS_PER_TAKE`] per batch, round the key numbers from the one
//! after the last written, the rest left for the next batch: the keys come
//! last in each batch, so a pong, an ack or a value queued meanwhile is not
//! kept behind a whole page of key images (25–200 KB). The writer's send
//! returns once a frame is in the TCP send buffer, so on a slow link a pong
//! can still wait behind about one send buffer (~64 KB) of key frames, far
//! less than a page; and a key that keeps changing never holds the others
//! back.
//!
//! The picture of the client's open Pro-Q 4 editor (#71 PR E) waits in one
//! slot, the newest frame replacing one not yet written (a slow link gets
//! fewer frames, never old ones), and goes as a binary message after the
//! batch it is taken with ([`Batch`]: taken together, so a message put
//! before a frame is always written before it; the PR I review found the
//! session's `eq` open written after its first frame when the frame was
//! taken after the batch's write). One frame is in flight at a time (PR I,
//! [`flow`]): the next one goes only once the page acked the one before
//! (`eq_frame_ack`, [`Outbox::frame_acked`]) or, with one in flight, the
//! ack's wait ran out, so frames never pile up in the socket's send buffer,
//! and a pong, an ack or a value waits behind one frame at most (two after
//! a lost ack). The frames are numbered in their header
//! (`fohmixer_proto::eq::FrameHead`, written as the frame is taken). Meanwhile the
//! newest frame waits in its slot, and the writer wakes for the ack, for
//! anything else, or when the wait runs out ([`Outbox::next_batch`]).
//!
//! An idle page's meters (#51, its `idle` message, [`Outbox::pace_meters`])
//! wait in a map of their own, per key the louder number (a peak shorter
//! than the period still reaches the page; a value that is not a number
//! replaces the one waiting), and join the batch's one `values` message once
//! per period ([`Pace::Every`]); a hidden page's ([`Pace::Paused`]) wait
//! until it wakes. Waking (full rate) puts every waiting meter with the
//! values at once. Every other value, reply, ack and frame goes as before,
//! and an instance going offline or a subscription left drops its waiting
//! meters as it drops its values.

pub mod flow;

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use bytes::Bytes;
use fohmixer_proto::client::{AckItem, DeckKey, Pace, ServerMsg, ValueItem, is_meter_key};
use fohmixer_proto::eq::{FrameHead, frame};
use serde_json::Value;
use tokio::sync::Notify;

use flow::{Flight, Trips};

/// Replies waiting for one client before it counts as stuck.
pub const MAX_REPLIES: usize = 1024;
/// The Stream Deck keys one batch carries at most (#52).
pub const DECK_KEYS_PER_TAKE: usize = 4;

#[derive(Debug, Default)]
struct Inner {
    replies: VecDeque<ServerMsg>,
    hub: BTreeMap<String, Value>,
    layout: Option<u64>,
    links: BTreeMap<String, ServerMsg>,
    values: BTreeMap<String, ValueItem>,
    acks: BTreeMap<String, AckItem>,
    deck: BTreeMap<u32, DeckKey>,
    /// The key the next batch's deck keys start from (the one after the
    /// last written; the round wraps to the lowest).
    deck_next: u32,
    /// The newest picture of the client's Pro-Q 4 editor (#71 PR E): its
    /// session, the page's input it shows (PR J) and its JPEG.
    frame: Option<(FrameHead, Bytes)>,
    /// Its frames on their way (PR I): the next goes once acked.
    flight: Flight,
    /// The frames' round trips since the last `rate` record (PR I).
    trips: Trips,
    /// The page's meter values held back while it is idle (#51): per key
    /// the louder number.
    meters: BTreeMap<String, ValueItem>,
    /// How the page's meter values go (#51): at once, paced or paused.
    pace: Pace,
    /// When the held meters go next while paced (#51).
    meters_at: Option<Instant>,
    closed: bool,
}

/// What the writer writes next (#71 PR I): the messages, then the frame
/// that may go with them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Batch {
    pub messages: Vec<ServerMsg>,
    pub frame: Option<Bytes>,
}

/// When the writer has something to write (#71 PR I).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wake {
    /// Now: a message, a frame that may go, or the close.
    Now,
    /// When the ack's wait of the frame waiting runs out (or something
    /// comes first).
    At(Instant),
    /// When something comes.
    Woken,
}

impl Inner {
    /// Queues an ordered message; a client that let [`MAX_REPLIES`] pile up
    /// is closed.
    fn push_reply(&mut self, msg: ServerMsg) {
        if self.replies.len() >= MAX_REPLIES {
            tracing::warn!(
                waiting = self.replies.len(),
                "a client stopped reading: closing it"
            );
            self.closed = true;
        } else {
            self.replies.push_back(msg);
        }
    }

    /// The next deck keys to write: at most [`DECK_KEYS_PER_TAKE`], from
    /// `deck_next` on, round to the lowest; the rest stay for the next
    /// batch.
    fn take_deck(&mut self) -> Vec<DeckKey> {
        let from = self.deck_next;
        let keys: Vec<u32> = self
            .deck
            .range(from..)
            .chain(self.deck.range(..from))
            .map(|(key, _)| *key)
            .take(DECK_KEYS_PER_TAKE)
            .collect();
        if let Some(last) = keys.last() {
            self.deck_next = last.saturating_add(1);
        }
        keys.iter()
            .filter_map(|key| self.deck.remove(key))
            .collect()
    }

    /// Whether a message waits (anything but the frame).
    fn has_messages(&self) -> bool {
        !(self.replies.is_empty()
            && self.hub.is_empty()
            && self.layout.is_none()
            && self.links.is_empty()
            && self.values.is_empty()
            && self.acks.is_empty()
            && self.deck.is_empty())
    }

    /// Whether nothing waits at all (the tests' view).
    #[cfg(test)]
    fn is_empty(&self) -> bool {
        !self.has_messages() && self.frame.is_none() && self.meters.is_empty()
    }

    /// A subscription's latest item; while the page is idle a meter's is
    /// held back with the one already held, the louder kept (#51).
    fn put_value(&mut self, item: ValueItem) {
        if self.pace == Pace::Full || !is_meter_key(&item.sub) {
            self.values.insert(item.sub.clone(), item);
            return;
        }
        let kept = match self.meters.remove(&item.sub) {
            Some(held) => louder(held, item),
            None => item,
        };
        self.meters.insert(kept.sub.clone(), kept);
    }

    /// The page's meter pace from `now` on (#51): full rate puts every held
    /// meter with the values (they go at once); a pace starts its first
    /// period, unless one runs already; a pause holds them.
    fn pace_meters(&mut self, pace: Pace, now: Instant) {
        match pace {
            Pace::Full => {
                self.values.append(&mut self.meters);
                self.meters_at = None;
            }
            Pace::Every(period) => {
                self.meters_at = self.meters_at.or(Some(now + period));
            }
            Pace::Paused => {}
        }
        self.pace = pace;
    }

    /// When the held meters go by themselves (#51): at once when paced and
    /// due, at their release when paced and not; never (until something
    /// wakes the writer) when none are held or they are paused.
    fn meters_wake(&self, now: Instant) -> Wake {
        match (self.pace, self.meters_at) {
            _ if self.meters.is_empty() => Wake::Woken,
            (Pace::Every(_), Some(at)) if now < at => Wake::At(at),
            (Pace::Every(_), _) => Wake::Now,
            _ => Wake::Woken,
        }
    }

    /// Puts the held meters with the values when they are due at `now`
    /// (#51); the next release is one period later.
    fn release_meters(&mut self, now: Instant) {
        if let (Wake::Now, Pace::Every(period)) = (self.meters_wake(now), self.pace) {
            self.values.append(&mut self.meters);
            self.meters_at = Some(now + period);
        }
    }

    /// When the writer has something to write at `now`: a message or the
    /// close at once; a frame waiting at once when it may go
    /// ([`Flight::due`]), else when its ack's wait runs out; held meters
    /// when they are due (#51); the sooner of those two; nothing until
    /// woken (a frame that may not go with no wait to count, which the
    /// flight never gives, waits for the next ack or message: the writer
    /// never loops without awaiting).
    fn wake(&self, now: Instant) -> Wake {
        if self.closed || self.has_messages() {
            return Wake::Now;
        }
        let frame = match &self.frame {
            None => Wake::Woken,
            Some((head, _)) if self.flight.due(head.session, now) => Wake::Now,
            Some((head, _)) => self
                .flight
                .wait_until(head.session)
                .map_or(Wake::Woken, Wake::At),
        };
        sooner(frame, self.meters_wake(now))
    }

    /// The messages waiting at `now`, in write order (see [`Outbox::take`]);
    /// held meters that are due go with the values (#51).
    fn take_messages(&mut self, now: Instant) -> Vec<ServerMsg> {
        self.release_meters(now);
        let mut out: Vec<ServerMsg> = self.replies.drain(..).collect();
        out.extend(
            std::mem::take(&mut self.hub)
                .into_iter()
                .map(|(key, value)| ServerMsg::Hub { key, value }),
        );
        if let Some(rev) = self.layout.take() {
            out.push(ServerMsg::Layout { rev });
        }
        out.extend(std::mem::take(&mut self.links).into_values());
        if !self.values.is_empty() {
            out.push(ServerMsg::Values {
                items: std::mem::take(&mut self.values).into_values().collect(),
            });
        }
        if !self.acks.is_empty() {
            out.push(ServerMsg::Ack {
                items: std::mem::take(&mut self.acks).into_values().collect(),
            });
        }
        let keys = self.take_deck();
        if !keys.is_empty() {
            out.push(ServerMsg::DeckKeys { items: keys });
        }
        out
    }

    /// The frame waiting, if it may go at `now` (PR I: one in flight, two
    /// after a lost ack), with its header (the session and the frame's
    /// number); none once closed. Taking it counts it in flight, and one
    /// that went without the ack before it counts as a wait that ran out.
    fn take_frame(&mut self, now: Instant) -> Option<Bytes> {
        let session = self.frame.as_ref()?.0.session;
        if self.closed || !self.flight.due(session, now) {
            return None;
        }
        let (mut head, jpeg) = self.frame.take()?;
        let (seq, waited_out) = self.flight.sent(session, now);
        if waited_out {
            self.trips.waited_out += 1;
        }
        head.seq = seq;
        Some(Bytes::from(frame(head, &jpeg)))
    }
}

/// The sooner of two wakes (#51: a frame's and the held meters').
fn sooner(a: Wake, b: Wake) -> Wake {
    match (a, b) {
        (Wake::Now, _) | (_, Wake::Now) => Wake::Now,
        (Wake::At(x), Wake::At(y)) => Wake::At(x.min(y)),
        (Wake::At(at), Wake::Woken) | (Wake::Woken, Wake::At(at)) => Wake::At(at),
        (Wake::Woken, Wake::Woken) => Wake::Woken,
    }
}

/// The meter item kept of a held one and a newer one (#51): the louder
/// when both are numbers (a peak shorter than the period still reaches the
/// page: the clip light and the bar's peak), else the newer.
fn louder(held: ValueItem, new: ValueItem) -> ValueItem {
    let level = |item: &ValueItem| item.value.as_ref().and_then(Value::as_f64);
    match (level(&held), level(&new)) {
        (Some(was), Some(now)) if was >= now => held,
        _ => new,
    }
}

/// One client's outbox.
#[derive(Debug, Default)]
pub struct Outbox {
    inner: Mutex<Inner>,
    ready: Notify,
}

impl Outbox {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs `f` on the open outbox and wakes the writer.
    fn put(&self, f: impl FnOnce(&mut Inner)) {
        let mut inner = self.lock();
        if inner.closed {
            return;
        }
        f(&mut inner);
        drop(inner);
        self.ready.notify_one();
    }

    /// A reply, in order; a client that let [`MAX_REPLIES`] pile up is
    /// closed.
    pub fn reply(&self, msg: ServerMsg) {
        self.put(|inner| inner.push_reply(msg));
    }

    /// The latest item of a subscription; while the page is idle a meter's
    /// is held back (#51, [`Outbox::pace_meters`]).
    pub fn value(&self, item: ValueItem) {
        self.put(|inner| inner.put_value(item));
    }

    /// The page's meter pace from `now` on (#51, its `idle` message): full
    /// rate (awake) sends every held meter at once; [`Pace::Every`] holds
    /// them, the louder kept per key, and lets them go once per period, the
    /// first one period from the first pacing; [`Pace::Paused`] holds them
    /// until the page wakes.
    pub fn pace_meters(&self, pace: Pace, now: Instant) {
        self.put(|inner| inner.pace_meters(pace, now));
    }

    /// The latest `link` of an instance (#43); any other message is
    /// ignored.
    pub fn link(&self, msg: ServerMsg) {
        let ServerMsg::Link { instance, .. } = &msg else {
            return;
        };
        let instance = instance.clone();
        self.put(|inner| {
            inner.links.insert(instance, msg);
        });
    }

    /// The outcome of one of the client's writes (#43): per key the one of
    /// the newest write waits (a late result of an older write never hides
    /// that a newer one was superseded).
    pub fn ack(&self, item: AckItem) {
        self.put(|inner| {
            if inner
                .acks
                .get(&item.key)
                .is_none_or(|held| held.seq <= item.seq)
            {
                inner.acks.insert(item.key.clone(), item);
            }
        });
    }

    /// A Stream Deck key's latest state (#52), written after everything else.
    pub fn deck_key(&self, key: DeckKey) {
        self.put(|inner| {
            inner.deck.insert(key.key, key);
        });
    }

    /// The newest picture of the client's Pro-Q 4 editor (#71 PR E), a JPEG
    /// and its header (PR J: the input it shows; PR L: where it lies in the
    /// picture): it replaces one still waiting. The header's `seq` is the
    /// flight's, written as it is taken ([`Batch`]).
    pub fn eq_frame(&self, head: FrameHead, jpeg: Bytes) {
        self.put(|inner| inner.frame = Some((head, jpeg)));
    }

    /// The client's editor closed: a frame still waiting is dropped, and
    /// its frames' flight is forgotten (PR I: a late ack of them is nothing,
    /// the next editor's first frame goes at once). Its round trips since
    /// the last `rate` record are handed back, for the close's record (the
    /// PR I review: a session's last minute was lost).
    pub fn forget_eq(&self) -> Trips {
        let mut inner = self.lock();
        inner.frame = None;
        inner.flight = Flight::default();
        std::mem::take(&mut inner.trips)
    }

    /// The frame that may go at `now`, taken alone (the tests' view: the
    /// writer takes it with its batch, [`Outbox::next_batch`]).
    #[cfg(test)]
    pub fn take_frame(&self, now: Instant) -> Option<Bytes> {
        self.lock().take_frame(now)
    }

    /// The page acked the frames of `session` up to `seq` at `now` (PR I):
    /// a round trip is counted, and the writer wakes (the next frame may
    /// go).
    pub fn frame_acked(&self, session: u32, seq: u32, now: Instant) {
        let mut inner = self.lock();
        if let Some(trip) = inner.flight.acked(session, seq, now) {
            inner.trips.trip(crate::plugwin::millis(trip));
        }
        drop(inner);
        self.ready.notify_one();
    }

    /// The frames' round trips since the last call (PR I: the editor's
    /// `rate` record).
    pub fn take_trips(&self) -> Trips {
        std::mem::take(&mut self.lock().trips)
    }

    /// The client closed the tab: the keys waiting for it are dropped.
    pub fn forget_deck(&self) {
        let mut inner = self.lock();
        inner.deck.clear();
        // A reopened tab starts its round from the lowest key (#54).
        inner.deck_next = 0;
    }

    /// Drops a pending item of a subscription the client left (a held
    /// meter's too, #51).
    pub fn forget(&self, sub: &str) {
        let mut inner = self.lock();
        inner.values.remove(sub);
        inner.meters.remove(sub);
    }

    /// A state of an instance (an `instance` message), in order with the
    /// replies; `online` false drops the instance's pending values (and its
    /// held meters, #51).
    pub fn instance(&self, name: &str, online: bool, msg: ServerMsg) {
        self.put(|inner| {
            if !online {
                let prefix = format!("{name}|");
                inner.values.retain(|sub, _| !sub.starts_with(&prefix));
                inner.meters.retain(|sub, _| !sub.starts_with(&prefix));
            }
            inner.push_reply(msg);
        });
    }

    /// The latest value of a hub key.
    pub fn hub(&self, key: &str, value: Value) {
        self.put(|inner| {
            inner.hub.insert(key.to_string(), value);
        });
    }

    /// The latest layout revision.
    pub fn layout(&self, rev: u64) {
        self.put(|inner| inner.layout = Some(rev));
    }

    /// Closes the outbox: the writer ends after it.
    pub fn close(&self) {
        self.lock().closed = true;
        self.ready.notify_one();
    }

    pub fn is_closed(&self) -> bool {
        self.lock().closed
    }

    /// Everything waiting, in write order (replies and instance states,
    /// hub values, the layout revision, the instances' links, one `values`
    /// message (with the held meters when due, #51), one `ack` message,
    /// then one `deck_keys` message of at most [`DECK_KEYS_PER_TAKE`] keys,
    /// the others left for the next take); `None` once closed. The frame
    /// stays.
    pub fn take(&self) -> Option<Vec<ServerMsg>> {
        let mut inner = self.lock();
        (!inner.closed).then(|| inner.take_messages(Instant::now()))
    }

    /// Everything waiting at `now`, the frame that may go with the messages
    /// ([`Batch`]); `None` once closed.
    pub fn take_batch(&self, now: Instant) -> Option<Batch> {
        let mut inner = self.lock();
        if inner.closed {
            return None;
        }
        let messages = inner.take_messages(now);
        let frame = inner.take_frame(now);
        Some(Batch { messages, frame })
    }

    /// Waits until something is waiting (or the outbox closed) and takes it;
    /// deck keys left by the last take are something waiting, so the writer
    /// takes them at once, after what came meanwhile. A frame alone is
    /// something waiting too once it may go (PR I: its predecessor acked, or
    /// the ack's wait over, which wakes the writer by itself): the batch's
    /// messages are then none. So are an idle page's held meters once their
    /// period ran out (#51): the writer wakes by itself for them.
    pub async fn next_batch(&self) -> Option<Batch> {
        loop {
            let wake = self.lock().wake(Instant::now());
            match wake {
                Wake::Now => return self.take_batch(Instant::now()),
                Wake::At(at) => {
                    let at = tokio::time::Instant::from_std(at);
                    let _ = tokio::time::timeout_at(at, self.ready.notified()).await;
                }
                Wake::Woken => self.ready.notified().await,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::client::DeckKey;
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;

    fn result(id: &str) -> ServerMsg {
        ServerMsg::Result {
            id: id.into(),
            data: vec![],
        }
    }

    fn instance(online: bool) -> ServerMsg {
        ServerMsg::Instance {
            name: "band".into(),
            online,
            busy: false,
            set_name: String::new(),
        }
    }

    fn put_instance(outbox: &Outbox, online: bool) {
        outbox.instance("band", online, instance(online));
    }

    #[test]
    fn values_keep_only_the_latest_per_subscription() {
        let outbox = Outbox::new();
        for v in 0..100 {
            outbox.value(ValueItem::value("a", json!(v), None));
        }
        outbox.value(ValueItem::value("b", json!(1), None));
        assert_eq!(
            outbox.take().unwrap(),
            vec![ServerMsg::Values {
                items: vec![
                    ValueItem::value("a", json!(99), None),
                    ValueItem::value("b", json!(1), None)
                ]
            }]
        );
        assert_eq!(outbox.take().unwrap(), vec![], "taken once");
    }

    #[test]
    fn the_write_order_is_replies_and_states_hub_layout_values() {
        let outbox = Outbox::new();
        outbox.value(ValueItem::value("a", json!(1), None));
        outbox.layout(1);
        outbox.layout(2);
        outbox.hub("stage_aut", json!(false));
        outbox.hub("stage_aut", json!(true));
        outbox.reply(result("1"));
        put_instance(&outbox, true);
        outbox.reply(result("2"));
        assert_eq!(
            outbox.take().unwrap(),
            vec![
                result("1"),
                instance(true),
                result("2"),
                ServerMsg::Hub {
                    key: "stage_aut".into(),
                    value: json!(true)
                },
                ServerMsg::Layout { rev: 2 },
                ServerMsg::Values {
                    items: vec![ValueItem::value("a", json!(1), None)]
                },
            ]
        );
    }

    #[test]
    fn an_instance_going_offline_is_seen_and_drops_its_old_values() {
        let outbox = Outbox::new();
        outbox.value(ValueItem::value(
            "band|live_set|tempo|false",
            json!(120),
            None,
        ));
        outbox.value(ValueItem::value(
            "master|live_set|tempo|false",
            json!(90),
            None,
        ));
        outbox.value(ValueItem::value(
            "bandx|live_set|tempo|false",
            json!(1),
            None,
        ));
        // Offline and back before the writer ran: both are written, in
        // order, and the old session's values of `band` are gone.
        put_instance(&outbox, false);
        put_instance(&outbox, true);
        outbox.reply(result("after"));
        outbox.value(ValueItem::value(
            "band|live_set|tempo|false",
            json!(100),
            None,
        ));
        assert_eq!(
            outbox.take().unwrap(),
            vec![
                instance(false),
                instance(true),
                result("after"),
                ServerMsg::Values {
                    items: vec![
                        // Key order: `x` sorts before `|`.
                        ValueItem::value("bandx|live_set|tempo|false", json!(1), None),
                        ValueItem::value("band|live_set|tempo|false", json!(100), None),
                        ValueItem::value("master|live_set|tempo|false", json!(90), None),
                    ]
                },
            ]
        );
        // Coming online keeps what waits.
        outbox.value(ValueItem::value(
            "band|live_set|tempo|false",
            json!(101),
            None,
        ));
        put_instance(&outbox, true);
        assert_eq!(
            outbox.take().unwrap(),
            vec![
                instance(true),
                ServerMsg::Values {
                    items: vec![ValueItem::value(
                        "band|live_set|tempo|false",
                        json!(101),
                        None
                    )]
                },
            ]
        );
    }

    #[test]
    fn too_many_states_close_the_outbox_too() {
        let outbox = Outbox::new();
        for i in 0..MAX_REPLIES {
            put_instance(&outbox, i % 2 == 0);
        }
        assert!(!outbox.is_closed());
        put_instance(&outbox, true);
        assert!(outbox.is_closed());
    }

    #[test]
    fn acks_keep_only_the_latest_per_key_and_come_after_the_values() {
        let outbox = Outbox::new();
        outbox.ack(AckItem::applied("band|live_set|tempo", 1, None));
        outbox.ack(AckItem::failed(
            "band|live_set|tempo",
            2,
            "instance offline",
        ));
        outbox.ack(AckItem::superseded("band|live_set|metronome", 4));
        outbox.value(ValueItem::value("a", json!(1), None));
        put_instance(&outbox, false);
        assert_eq!(
            outbox.take().unwrap(),
            vec![
                instance(false),
                ServerMsg::Values {
                    items: vec![ValueItem::value("a", json!(1), None)]
                },
                ServerMsg::Ack {
                    items: vec![
                        AckItem::superseded("band|live_set|metronome", 4),
                        AckItem::failed("band|live_set|tempo", 2, "instance offline"),
                    ]
                },
            ],
            "an instance going offline keeps the acks"
        );
        assert_eq!(outbox.take().unwrap(), vec![], "taken once");
        // A late result of an older write does not hide a newer write's
        // outcome; the same write's later outcome replaces it.
        outbox.ack(AckItem::superseded("k", 7));
        outbox.ack(AckItem::applied("k", 5, None));
        outbox.ack(AckItem::failed("j", 3, "late"));
        outbox.ack(AckItem::applied("j", 3, None));
        assert_eq!(
            outbox.take().unwrap(),
            vec![ServerMsg::Ack {
                items: vec![AckItem::applied("j", 3, None), AckItem::superseded("k", 7)]
            }]
        );
        // An ack alone wakes the writer.
        outbox.ack(AckItem::applied("k", 9, None));
        assert!(!outbox.lock().is_empty());
        assert_eq!(
            outbox.take().unwrap(),
            vec![ServerMsg::Ack {
                items: vec![AckItem::applied("k", 9, None)]
            }]
        );
        assert!(outbox.lock().is_empty());
    }

    fn deck_key(key: u32, img: &str) -> DeckKey {
        DeckKey {
            key,
            img: Some(img.to_string()),
            color: None,
            pressed: false,
        }
    }

    #[test]
    fn deck_keys_keep_the_latest_per_key_and_go_last() {
        let outbox = Outbox::new();
        outbox.deck_key(deck_key(5, "data:a"));
        outbox.deck_key(deck_key(1, "data:b"));
        outbox.deck_key(deck_key(5, "data:c"));
        outbox.ack(AckItem::applied("band|x|value", 3, None));
        outbox.value(ValueItem::value("a", json!(1), None));
        outbox.reply(result("1"));
        assert_eq!(
            outbox.take().unwrap(),
            vec![
                result("1"),
                ServerMsg::Values {
                    items: vec![ValueItem::value("a", json!(1), None)]
                },
                ServerMsg::Ack {
                    items: vec![AckItem::applied("band|x|value", 3, None)]
                },
                ServerMsg::DeckKeys {
                    items: vec![deck_key(1, "data:b"), deck_key(5, "data:c")]
                },
            ]
        );
        assert_eq!(outbox.take().unwrap(), vec![], "taken once");
    }

    /// The keys of a batch's `deck_keys` message, and the batch's other
    /// messages before it.
    fn deck_part(batch: Vec<ServerMsg>) -> (Vec<ServerMsg>, Vec<u32>) {
        let mut others = Vec::new();
        let mut keys = Vec::new();
        for msg in batch {
            match msg {
                ServerMsg::DeckKeys { items } => keys.extend(items.iter().map(|k| k.key)),
                other => others.push(other),
            }
        }
        (others, keys)
    }

    #[test]
    fn deck_keys_go_four_at_a_time_and_a_reply_goes_between() {
        assert_eq!(DECK_KEYS_PER_TAKE, 4);
        let outbox = Outbox::new();
        for key in 0..32 {
            outbox.deck_key(deck_key(key, "data:a"));
        }
        let mut sent = Vec::new();
        for round in 0..8_u32 {
            // A pong queued after the last take goes before the next keys.
            outbox.reply(result(&format!("pong {round}")));
            let batch = outbox.take().unwrap();
            assert_eq!(
                batch
                    .last()
                    .map(|m| matches!(m, ServerMsg::DeckKeys { .. })),
                Some(true),
                "the keys go last"
            );
            let (others, keys) = deck_part(batch);
            assert_eq!(others, vec![result(&format!("pong {round}"))]);
            assert_eq!(keys.len(), 4, "round {round}: {keys:?}");
            sent.extend(keys);
            assert_eq!(
                outbox.lock().is_empty(),
                round == 7,
                "keys left wake the writer at once"
            );
        }
        assert_eq!(sent, (0..32).collect::<Vec<u32>>(), "in order, each once");
        assert_eq!(outbox.take().unwrap(), vec![]);
    }

    #[test]
    fn keys_left_for_later_go_round_the_keys_at_their_latest() {
        let outbox = Outbox::new();
        for key in 0..8 {
            outbox.deck_key(deck_key(key, "data:a"));
        }
        assert_eq!(deck_part(outbox.take().unwrap()).1, vec![0, 1, 2, 3]);
        // Keys just written change again, and one waiting changes: the
        // round goes on from key 4, each key at its latest.
        outbox.deck_key(deck_key(3, "data:b"));
        outbox.deck_key(deck_key(1, "data:b"));
        outbox.deck_key(deck_key(6, "data:b"));
        assert_eq!(
            outbox.take().unwrap(),
            vec![ServerMsg::DeckKeys {
                items: vec![
                    deck_key(4, "data:a"),
                    deck_key(5, "data:a"),
                    deck_key(6, "data:b"),
                    deck_key(7, "data:a")
                ]
            }]
        );
        assert_eq!(
            outbox.take().unwrap(),
            vec![ServerMsg::DeckKeys {
                items: vec![deck_key(1, "data:b"), deck_key(3, "data:b")]
            }]
        );
        assert_eq!(outbox.take().unwrap(), vec![]);
        // A key after the last one written goes first, then the round
        // starts over from the lowest.
        for key in [0, 2, 9] {
            outbox.deck_key(deck_key(key, "data:c"));
        }
        assert_eq!(deck_part(outbox.take().unwrap()).1, vec![9, 0, 2]);
    }

    #[tokio::test]
    async fn the_writer_takes_the_keys_left_at_once() {
        let outbox = Outbox::new();
        for key in 0..6 {
            outbox.deck_key(deck_key(key, "data:a"));
        }
        let first = outbox.next_batch().await.unwrap().messages;
        assert_eq!(deck_part(first).1, vec![0, 1, 2, 3]);
        let second = tokio::time::timeout(Duration::from_millis(500), outbox.next_batch())
            .await
            .expect("keys left: no wait for a wake-up")
            .unwrap()
            .messages;
        assert_eq!(deck_part(second).1, vec![4, 5]);
    }

    #[test]
    fn a_waiting_deck_key_wakes_the_writer_and_a_closed_tab_drops_them() {
        let outbox = Outbox::new();
        outbox.deck_key(deck_key(2, "data:a"));
        assert!(
            !outbox.lock().is_empty(),
            "a deck key is something to write"
        );
        outbox.forget_deck();
        assert!(outbox.lock().is_empty());
        assert_eq!(outbox.take().unwrap(), vec![]);
        outbox.close();
        outbox.deck_key(deck_key(2, "data:b"));
        assert_eq!(outbox.take(), None);
    }

    #[test]
    fn a_reopened_tab_starts_its_first_batch_at_the_lowest_key() {
        let outbox = Outbox::new();
        for key in 0..6 {
            outbox.deck_key(deck_key(key, "data:a"));
        }
        let first = outbox.take().unwrap();
        assert_eq!(deck_part(first).1, vec![0, 1, 2, 3]);
        // The tab closes and opens again: the round starts over.
        outbox.forget_deck();
        for key in 0..6 {
            outbox.deck_key(deck_key(key, "data:b"));
        }
        let again = outbox.take().unwrap();
        assert_eq!(deck_part(again).1, vec![0, 1, 2, 3]);
    }

    #[tokio::test]
    async fn the_writer_wakes_for_a_deck_key() {
        let outbox = Arc::new(Outbox::new());
        let writer = Arc::clone(&outbox);
        let batch = tokio::spawn(async move { writer.next_batch().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        outbox.deck_key(deck_key(7, "data:a"));
        let batch = tokio::time::timeout(Duration::from_secs(2), batch)
            .await
            .unwrap()
            .unwrap()
            .unwrap()
            .messages;
        assert_eq!(
            batch,
            vec![ServerMsg::DeckKeys {
                items: vec![deck_key(7, "data:a")]
            }]
        );
    }

    fn link(instance: &str, tick_age_ms: f64) -> ServerMsg {
        ServerMsg::Link {
            instance: instance.into(),
            tick_age_ms,
            busy: true,
        }
    }

    #[test]
    fn links_keep_only_the_latest_per_instance() {
        let outbox = Outbox::new();
        outbox.link(link("master", 300.0));
        outbox.link(link("band", 250.0));
        outbox.link(link("band", 500.0));
        // Only a link is taken as one.
        outbox.link(ServerMsg::Layout { rev: 9 });
        assert!(!outbox.lock().is_empty());
        outbox.value(ValueItem::value("a", json!(1), None));
        outbox.layout(2);
        assert_eq!(
            outbox.take().unwrap(),
            vec![
                ServerMsg::Layout { rev: 2 },
                link("band", 500.0),
                link("master", 300.0),
                ServerMsg::Values {
                    items: vec![ValueItem::value("a", json!(1), None)]
                },
            ]
        );
        outbox.link(ServerMsg::Pong {
            n: 1,
            t: 2.0,
            h: 3.0,
        });
        assert!(outbox.lock().is_empty(), "nothing else is a link");
    }

    #[test]
    fn a_left_subscription_is_forgotten() {
        let outbox = Outbox::new();
        outbox.value(ValueItem::value("a", json!(1), None));
        outbox.value(ValueItem::value("b", json!(1), None));
        outbox.forget("a");
        assert_eq!(
            outbox.take().unwrap(),
            vec![ServerMsg::Values {
                items: vec![ValueItem::value("b", json!(1), None)]
            }]
        );
    }

    #[test]
    fn too_many_replies_close_the_outbox() {
        let outbox = Outbox::new();
        for i in 0..MAX_REPLIES {
            outbox.reply(result(&i.to_string()));
        }
        assert!(!outbox.is_closed());
        outbox.reply(result("one too many"));
        assert!(outbox.is_closed());
        assert_eq!(outbox.take(), None);
        // A closed outbox takes nothing more.
        outbox.value(ValueItem::value("a", json!(1), None));
        assert_eq!(outbox.take(), None);
    }

    #[tokio::test]
    async fn next_waits_for_something_and_ends_when_closed() {
        let outbox = Arc::new(Outbox::new());
        let waiting = {
            let outbox = Arc::clone(&outbox);
            tokio::spawn(async move { outbox.next_batch().await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiting.is_finished(), "nothing to write yet");
        outbox.layout(4);
        let got = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("woken")
            .unwrap();
        assert_eq!(
            got,
            Some(Batch {
                messages: vec![ServerMsg::Layout { rev: 4 }],
                frame: None,
            })
        );
        // Something already waiting is taken at once.
        outbox.layout(5);
        assert_eq!(
            outbox.next_batch().await.map(|b| b.messages),
            Some(vec![ServerMsg::Layout { rev: 5 }])
        );
        let waiting = {
            let outbox = Arc::clone(&outbox);
            tokio::spawn(async move { outbox.next_batch().await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        outbox.close();
        let got = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("woken by the close")
            .unwrap();
        assert_eq!(got, None);
        assert_eq!(outbox.next_batch().await, None);
    }

    /// A frame of `session` numbered `seq` with `jpeg`, as the writer sends it.
    fn framed(session: u32, seq: u32, jpeg: &[u8]) -> Option<Bytes> {
        let mut head = head_of(session);
        head.seq = seq;
        Some(Bytes::from(frame(head, jpeg)))
    }

    /// The header of a whole 1349 × 809 picture of `session` (its `seq` the
    /// flight's).
    fn head_of(session: u32) -> FrameHead {
        FrameHead {
            session,
            seq: 0,
            shown: 0,
            x: 0,
            y: 0,
            width: 1349,
            height: 809,
        }
    }

    /// A batch of no messages with `frame`.
    fn frame_only(frame: Option<Bytes>) -> Option<Batch> {
        Some(Batch {
            messages: vec![],
            frame,
        })
    }

    #[tokio::test]
    async fn the_newest_frame_waits_alone_and_wakes_the_writer() {
        let outbox = Arc::new(Outbox::new());
        let t = Instant::now();
        assert_eq!(outbox.take_frame(t), None);
        let writer = Arc::clone(&outbox);
        let batch = tokio::spawn(async move { writer.next_batch().await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!batch.is_finished(), "nothing to write yet");
        outbox.eq_frame(head_of(1), Bytes::from_static(b"old"));
        outbox.eq_frame(head_of(1), Bytes::from_static(b"new"));
        // A frame alone wakes the writer: a batch of it alone, numbered 1.
        let batch = tokio::time::timeout(Duration::from_secs(2), batch)
            .await
            .expect("woken by a frame")
            .unwrap();
        assert_eq!(batch, frame_only(framed(1, 1, b"new")));
        assert_eq!(outbox.take_frame(t), None, "taken once");
        assert!(outbox.lock().is_empty());
        // The messages and the frame go together, the messages first.
        outbox.frame_acked(1, 1, t);
        outbox.eq_frame(head_of(1), Bytes::from_static(b"next"));
        outbox.layout(3);
        assert_eq!(
            outbox.take_batch(t),
            Some(Batch {
                messages: vec![ServerMsg::Layout { rev: 3 }],
                frame: framed(1, 2, b"next"),
            })
        );
        // A closed editor's frame is dropped.
        outbox.eq_frame(head_of(1), Bytes::from_static(b"gone"));
        outbox.forget_eq();
        assert!(outbox.lock().is_empty());
        assert_eq!(outbox.take_frame(t), None);
        // A closed outbox writes no frame.
        outbox.eq_frame(head_of(1), Bytes::from_static(b"late"));
        outbox.close();
        assert_eq!(outbox.take_frame(t), None);
        assert_eq!(outbox.take_batch(t), None);
        outbox.eq_frame(head_of(1), Bytes::from_static(b"after"));
        assert_eq!(
            outbox.lock().frame.as_ref().map(|(_, f)| &f[..]),
            Some(&b"late"[..]),
            "a closed outbox takes nothing new"
        );
    }

    #[test]
    fn a_message_put_before_a_frame_is_written_before_it() {
        // The PR I review: the writer took the frame after writing its
        // batch, so the `eq` open put during that write went after the
        // session's first frame, and the page, still opening, dropped it.
        let outbox = Outbox::new();
        let t = Instant::now();
        outbox.layout(1);
        assert_eq!(
            outbox.take_batch(t),
            Some(Batch {
                messages: vec![ServerMsg::Layout { rev: 1 }],
                frame: None,
            })
        );
        // While that batch is written: a reply, then a frame.
        outbox.reply(result("open"));
        outbox.eq_frame(head_of(1), Bytes::from_static(b"a"));
        assert_eq!(
            outbox.take_batch(t),
            Some(Batch {
                messages: vec![result("open")],
                frame: framed(1, 1, b"a"),
            })
        );
        // A frame that may not go stays for a later batch.
        outbox.eq_frame(head_of(1), Bytes::from_static(b"b"));
        outbox.layout(2);
        assert_eq!(
            outbox.take_batch(t),
            Some(Batch {
                messages: vec![ServerMsg::Layout { rev: 2 }],
                frame: None,
            })
        );
        assert!(!outbox.lock().is_empty(), "the frame still waits");
        // `take` leaves the frame too.
        outbox.frame_acked(1, 1, t);
        assert_eq!(outbox.take(), Some(vec![]));
        assert_eq!(outbox.take_frame(t), framed(1, 2, b"b"));
    }

    /// `n` ms after `t`.
    fn at(t: Instant, n: u64) -> Instant {
        t + Duration::from_millis(n)
    }

    #[test]
    fn a_frame_goes_only_once_the_one_before_was_acked_or_its_wait_ran_out() {
        let outbox = Outbox::new();
        let t = Instant::now();
        outbox.eq_frame(head_of(1), Bytes::from_static(b"a"));
        assert_eq!(outbox.take_frame(t), framed(1, 1, b"a"));
        // The next waits for the ack; a newer one replaces it meanwhile.
        outbox.eq_frame(head_of(1), Bytes::from_static(b"b"));
        assert_eq!(outbox.take_frame(at(t, 10)), None);
        outbox.eq_frame(head_of(1), Bytes::from_static(b"c"));
        assert_eq!(outbox.take_frame(at(t, 20)), None);
        // An ack of another session is nothing; the page's ack lets it go.
        outbox.frame_acked(2, 1, at(t, 25));
        assert_eq!(outbox.take_frame(at(t, 26)), None);
        outbox.frame_acked(1, 1, at(t, 30));
        assert_eq!(outbox.take_frame(at(t, 31)), framed(1, 2, b"c"));
        // No ack: the next goes once the wait (1 s) ran out, counted.
        outbox.eq_frame(head_of(1), Bytes::from_static(b"d"));
        assert_eq!(outbox.take_frame(at(t, 1030)), None);
        assert_eq!(outbox.take_frame(at(t, 1031)), framed(1, 3, b"d"));
        // Two in flight: the next waits for an ack, however long.
        outbox.eq_frame(head_of(1), Bytes::from_static(b"e"));
        assert_eq!(outbox.take_frame(at(t, 5000)), None);
        assert_eq!(
            outbox.take_trips(),
            flow::Trips {
                n: 1,
                total_ms: 30.0,
                max_ms: 30.0,
                waited_out: 1,
            }
        );
        assert_eq!(outbox.take_trips(), flow::Trips::default(), "taken");
        // The ack of the newest acks both: the next goes at once.
        outbox.frame_acked(1, 3, at(t, 5010));
        assert_eq!(outbox.take_frame(at(t, 5011)), framed(1, 4, b"e"));
        // Another session's frame goes at once (a new editor), numbered 1.
        outbox.eq_frame(head_of(2), Bytes::from_static(b"f"));
        assert_eq!(outbox.take_frame(at(t, 5012)), framed(2, 1, b"f"));
    }

    #[test]
    fn a_closed_editor_forgets_its_frames_flight_and_hands_back_its_round_trips() {
        let outbox = Outbox::new();
        let t = Instant::now();
        outbox.eq_frame(head_of(1), Bytes::from_static(b"a"));
        outbox.take_frame(t);
        outbox.frame_acked(1, 1, at(t, 20));
        outbox.eq_frame(head_of(1), Bytes::from_static(b"b"));
        outbox.take_frame(at(t, 30));
        outbox.eq_frame(head_of(1), Bytes::from_static(b"c"));
        // The round trips of the last minute go to the close's record.
        assert_eq!(
            outbox.forget_eq(),
            flow::Trips {
                n: 1,
                total_ms: 20.0,
                max_ms: 20.0,
                waited_out: 0,
            }
        );
        assert_eq!(outbox.take_trips(), flow::Trips::default());
        assert_eq!(outbox.lock().flight, flow::Flight::default());
        assert_eq!(outbox.forget_eq(), flow::Trips::default(), "handed once");
        // The same session's next frame goes at once (nothing in flight).
        outbox.eq_frame(head_of(1), Bytes::from_static(b"d"));
        assert_eq!(outbox.take_frame(at(t, 40)), framed(1, 1, b"d"));
    }

    #[test]
    fn only_a_message_is_a_message_the_frame_is_not() {
        let outbox = Outbox::new();
        assert!(!outbox.lock().has_messages());
        outbox.eq_frame(head_of(1), Bytes::from_static(b"a"));
        assert!(!outbox.lock().has_messages(), "a frame alone");
        // Nor is a held meter (#51): it goes when its period ran out.
        outbox.pace_meters(Pace::Paused, Instant::now());
        outbox.value(meter("Kick", json!(0.5)));
        assert!(!outbox.lock().has_messages(), "a held meter");
        let kinds: [fn(&Outbox); 7] = [
            |o: &Outbox| o.reply(result("1")),
            |o: &Outbox| o.hub("stage_aut", json!(true)),
            |o: &Outbox| o.layout(1),
            |o: &Outbox| o.link(link("band", 1.0)),
            |o: &Outbox| o.value(ValueItem::value("a", json!(1), None)),
            |o: &Outbox| o.ack(AckItem::applied("k", 1, None)),
            |o: &Outbox| o.deck_key(deck_key(1, "data:a")),
        ];
        for (i, put) in kinds.iter().enumerate() {
            let outbox = Outbox::new();
            put(&outbox);
            assert!(outbox.lock().has_messages(), "kind {i}");
        }
    }

    #[test]
    fn the_writer_wakes_for_a_message_a_frame_that_may_go_or_its_wait() {
        let t = Instant::now();
        let outbox = Outbox::new();
        assert_eq!(outbox.lock().wake(t), Wake::Woken, "nothing");
        outbox.eq_frame(head_of(1), Bytes::from_static(b"a"));
        assert_eq!(outbox.lock().wake(t), Wake::Now, "a frame that may go");
        outbox.take_frame(t);
        assert_eq!(outbox.lock().wake(t), Wake::Woken, "nothing waits");
        outbox.eq_frame(head_of(1), Bytes::from_static(b"b"));
        assert_eq!(
            outbox.lock().wake(at(t, 10)),
            Wake::At(at(t, 1000)),
            "a frame waiting for the ack"
        );
        assert_eq!(
            outbox.lock().wake(at(t, 1000)),
            Wake::Now,
            "its wait ran out"
        );
        // A message goes at once whatever the frame does.
        outbox.layout(2);
        assert_eq!(outbox.lock().wake(at(t, 10)), Wake::Now);
        outbox.take();
        // Two in flight: only an ack or a message wakes the writer.
        outbox.take_frame(at(t, 1000));
        outbox.eq_frame(head_of(1), Bytes::from_static(b"c"));
        assert_eq!(
            outbox.lock().wake(at(t, 9000)),
            Wake::Woken,
            "two in flight"
        );
        outbox.frame_acked(1, 2, at(t, 9010));
        assert_eq!(outbox.lock().wake(at(t, 9010)), Wake::Now, "acked");
        // Closed: at once (the writer ends).
        let closed = Outbox::new();
        closed.close();
        assert_eq!(closed.lock().wake(t), Wake::Now);
    }

    #[tokio::test]
    async fn the_writer_sleeps_while_a_frame_waits_for_its_ack_and_wakes_for_the_ack() {
        let outbox = Arc::new(Outbox::new());
        outbox.eq_frame(head_of(1), Bytes::from_static(b"a"));
        assert_eq!(outbox.next_batch().await, frame_only(framed(1, 1, b"a")));
        outbox.eq_frame(head_of(1), Bytes::from_static(b"b"));
        let writer = Arc::clone(&outbox);
        let batch = tokio::spawn(async move { writer.next_batch().await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!batch.is_finished(), "the frame waits for its ack");
        outbox.frame_acked(1, 1, Instant::now());
        // Woken by the ack, well before the 1 s wait would wake it (the PR
        // I review: a 2 s timeout let the wait's end pass for the ack).
        let batch = tokio::time::timeout(Duration::from_millis(300), batch)
            .await
            .expect("woken by the ack")
            .unwrap();
        assert_eq!(batch, frame_only(framed(1, 2, b"b")));
    }

    /// A meter value (Live's `output_meter_level`) of `track` on `band`.
    fn meter(track: &str, level: Value) -> ValueItem {
        ValueItem::value(&meter_key("band", track), level, None)
    }

    /// The hub key of `track`'s level meter on `instance`.
    fn meter_key(instance: &str, track: &str) -> String {
        format!("{instance}|live_set tracks[name={track}]|output_meter_level|false")
    }

    /// A pace of `ms` (#51).
    fn every(ms: u64) -> Pace {
        Pace::Every(Duration::from_millis(ms))
    }

    fn values(items: Vec<ValueItem>) -> ServerMsg {
        ServerMsg::Values { items }
    }

    /// The messages a writer takes at `now`.
    fn taken(outbox: &Outbox, now: Instant) -> Vec<ServerMsg> {
        outbox.take_batch(now).unwrap().messages
    }

    #[test]
    fn a_paced_meter_waits_for_its_period_and_goes_with_the_values() {
        let outbox = Outbox::new();
        let t = Instant::now();
        // Awake (the default), a meter goes at once.
        outbox.value(meter("Kick", json!(0.1)));
        assert_eq!(
            taken(&outbox, t),
            vec![values(vec![meter("Kick", json!(0.1))])]
        );
        // Idle from t at 250 ms: the first release is one period later.
        outbox.pace_meters(every(250), t);
        outbox.value(meter("Kick", json!(0.5)));
        let due = at(t, 250);
        let before = due - Duration::from_nanos(1);
        assert_eq!(outbox.lock().wake(before), Wake::At(due));
        assert_eq!(taken(&outbox, before), vec![], "one tick before: held");
        assert_eq!(outbox.lock().wake(due), Wake::Now);
        assert_eq!(
            taken(&outbox, due),
            vec![values(vec![meter("Kick", json!(0.5))])]
        );
        assert!(outbox.lock().is_empty(), "released once");
        // A late release (the writer was busy): the next one is a period
        // after it, not after the one it was due at.
        outbox.value(meter("Kick", json!(0.6)));
        let late = at(t, 600);
        assert_eq!(
            taken(&outbox, late),
            vec![values(vec![meter("Kick", json!(0.6))])]
        );
        outbox.value(meter("Kick", json!(0.7)));
        let next = at(t, 850);
        assert_eq!(outbox.lock().wake(late), Wake::At(next));
        assert_eq!(taken(&outbox, next - Duration::from_nanos(1)), vec![]);
        // With another value: the batch's one `values` message holds both,
        // in key order (`mute` before `output_meter_level`).
        let mute = ValueItem::value("band|live_set tracks[name=Kick]|mute|false", json!(1), None);
        outbox.value(mute.clone());
        assert_eq!(
            taken(&outbox, next),
            vec![values(vec![mute, meter("Kick", json!(0.7))])]
        );
        // Nothing held: the writer sleeps until woken, its period or not.
        assert_eq!(outbox.lock().wake(at(t, 5000)), Wake::Woken);
    }

    #[test]
    fn a_held_meter_keeps_the_louder_number_and_a_non_number_replaces_it() {
        let outbox = Outbox::new();
        let t = Instant::now();
        outbox.pace_meters(every(250), t);
        for level in [json!(0.3), json!(0.9), json!(0.5), json!(0.9)] {
            outbox.value(meter("Kick", level));
        }
        // Another meter of another kind is held on its own.
        let left = ValueItem::value(
            "band|live_set tracks[name=Kick]|output_meter_left|false",
            json!(0.2),
            None,
        );
        outbox.value(left.clone());
        let due = at(t, 250);
        assert_eq!(
            taken(&outbox, due),
            vec![values(vec![left, meter("Kick", json!(0.9))])],
            "the loudest of the period"
        );
        // A quieter one after a louder one is still the louder; a louder one
        // replaces.
        outbox.value(meter("Kick", json!(0.4)));
        outbox.value(meter("Kick", json!(1)));
        outbox.value(meter("Kick", json!(0.8)));
        assert_eq!(
            taken(&outbox, at(t, 500)),
            vec![values(vec![meter("Kick", json!(1))])]
        );
        // An error replaces a number, and a number then replaces the error
        // (the newer: the error is no level).
        let gone = ValueItem::error(&meter_key("band", "Kick"), "no such track");
        outbox.value(meter("Kick", json!(0.9)));
        outbox.value(gone.clone());
        assert_eq!(taken(&outbox, at(t, 750)), vec![values(vec![gone.clone()])]);
        outbox.value(gone);
        outbox.value(meter("Kick", json!(0.1)));
        assert_eq!(
            taken(&outbox, at(t, 1000)),
            vec![values(vec![meter("Kick", json!(0.1))])]
        );
        // A null replaces a number too.
        outbox.value(meter("Kick", json!(0.9)));
        outbox.value(meter("Kick", Value::Null));
        assert_eq!(
            taken(&outbox, at(t, 1250)),
            vec![values(vec![meter("Kick", Value::Null)])]
        );
    }

    #[test]
    fn the_louder_of_two_meter_items() {
        let kick = |v: Value| meter("Kick", v);
        assert_eq!(louder(kick(json!(0.5)), kick(json!(0.4))), kick(json!(0.5)));
        assert_eq!(louder(kick(json!(0.4)), kick(json!(0.5))), kick(json!(0.5)));
        assert_eq!(louder(kick(json!(0)), kick(json!(1))), kick(json!(1)));
        // Equal: the held one stays (the same number).
        assert_eq!(louder(kick(json!(0.5)), kick(json!(0.5))), kick(json!(0.5)));
        let gone = ValueItem::error(&meter_key("band", "Kick"), "gone");
        assert_eq!(louder(kick(json!(0.5)), gone.clone()), gone);
        assert_eq!(louder(gone, kick(json!(0.1))), kick(json!(0.1)));
        assert_eq!(louder(kick(json!(0.5)), kick(json!("x"))), kick(json!("x")));
    }

    #[test]
    fn everything_but_a_meter_goes_at_once_while_paced_or_paused() {
        let t = Instant::now();
        let mute = ValueItem::value("band|live_set tracks[name=Kick]|mute|false", json!(1), None);
        let pong = ServerMsg::Pong {
            n: 1,
            t: 2.0,
            h: 3.0,
        };
        for (pace, alone) in [
            (every(250), Wake::At(at(t, 250))),
            (Pace::Paused, Wake::Woken),
        ] {
            let outbox = Outbox::new();
            outbox.pace_meters(pace, t);
            outbox.value(meter("Kick", json!(0.5)));
            assert_eq!(outbox.lock().wake(t), alone, "{pace:?}: a held meter alone");
            assert!(!outbox.lock().has_messages(), "{pace:?}: no message");
            assert!(!outbox.lock().is_empty(), "{pace:?}: but held");
            outbox.value(mute.clone());
            outbox.ack(AckItem::applied("k", 1, None));
            outbox.reply(pong.clone());
            outbox.layout(4);
            assert_eq!(outbox.lock().wake(t), Wake::Now, "{pace:?}");
            assert_eq!(
                taken(&outbox, t),
                vec![
                    pong.clone(),
                    ServerMsg::Layout { rev: 4 },
                    values(vec![mute.clone()]),
                    ServerMsg::Ack {
                        items: vec![AckItem::applied("k", 1, None)]
                    },
                ],
                "{pace:?}"
            );
            // A frame goes at once too.
            outbox.eq_frame(head_of(1), Bytes::from_static(b"a"));
            assert_eq!(outbox.take_frame(t), framed(1, 1, b"a"), "{pace:?}");
            assert_eq!(outbox.lock().meters.len(), 1, "{pace:?}: still held");
        }
    }

    #[test]
    fn a_paused_meter_is_never_released_and_waking_releases_it_at_once() {
        let outbox = Outbox::new();
        let t = Instant::now();
        // A meter written before the pause, still waiting with the values,
        // goes; one written after it is held, however long.
        outbox.value(meter("Kick", json!(0.1)));
        outbox.pace_meters(Pace::Paused, t);
        outbox.value(meter("Snare", json!(0.2)));
        assert_eq!(
            taken(&outbox, t),
            vec![values(vec![meter("Kick", json!(0.1))])]
        );
        outbox.value(meter("Kick", json!(0.3)));
        for n in [250, 2000, 3_600_000] {
            assert_eq!(outbox.lock().wake(at(t, n)), Wake::Woken, "{n}");
            assert_eq!(taken(&outbox, at(t, n)), vec![], "{n}");
        }
        // Waking: every held meter goes with the next batch.
        outbox.pace_meters(Pace::Full, at(t, 5000));
        assert_eq!(outbox.lock().wake(at(t, 5000)), Wake::Now);
        assert_eq!(
            taken(&outbox, at(t, 5000)),
            vec![values(vec![
                meter("Kick", json!(0.3)),
                meter("Snare", json!(0.2))
            ])]
        );
        // Awake again: a meter goes at once.
        outbox.value(meter("Kick", json!(0.4)));
        assert_eq!(
            taken(&outbox, at(t, 5001)),
            vec![values(vec![meter("Kick", json!(0.4))])]
        );
    }

    #[test]
    fn waking_a_paced_page_releases_before_its_period_and_a_new_pace_starts_afresh() {
        let outbox = Outbox::new();
        let t = Instant::now();
        // A meter waiting with the values when the page went idle is the
        // older one: the held one replaces it at the wake.
        outbox.value(meter("Kick", json!(0.9)));
        outbox.pace_meters(every(250), t);
        outbox.value(meter("Kick", json!(0.2)));
        outbox.pace_meters(Pace::Full, at(t, 10));
        assert_eq!(
            taken(&outbox, at(t, 10)),
            vec![values(vec![meter("Kick", json!(0.2))])]
        );
        // Idle again: the first period starts at this pacing.
        outbox.pace_meters(every(250), at(t, 20));
        outbox.value(meter("Kick", json!(0.3)));
        assert_eq!(outbox.lock().wake(at(t, 30)), Wake::At(at(t, 270)));
        // Another `idle` while paced keeps the period running; the new period
        // counts from the next release.
        outbox.pace_meters(every(1000), at(t, 100));
        assert_eq!(outbox.lock().wake(at(t, 100)), Wake::At(at(t, 270)));
        assert_eq!(
            taken(&outbox, at(t, 270)),
            vec![values(vec![meter("Kick", json!(0.3))])]
        );
        outbox.value(meter("Kick", json!(0.4)));
        assert_eq!(outbox.lock().wake(at(t, 270)), Wake::At(at(t, 1270)));
        // Paused, then paced: the period that ran goes on.
        outbox.pace_meters(Pace::Paused, at(t, 300));
        assert_eq!(outbox.lock().wake(at(t, 1270)), Wake::Woken);
        outbox.pace_meters(every(250), at(t, 400));
        assert_eq!(outbox.lock().wake(at(t, 400)), Wake::At(at(t, 1270)));
        // Awake, then paused, then paced: a fresh period.
        outbox.pace_meters(Pace::Full, at(t, 500));
        taken(&outbox, at(t, 500));
        outbox.pace_meters(Pace::Paused, at(t, 600));
        outbox.pace_meters(every(250), at(t, 700));
        outbox.value(meter("Kick", json!(0.5)));
        assert_eq!(outbox.lock().wake(at(t, 700)), Wake::At(at(t, 950)));
    }

    #[test]
    fn an_instance_going_offline_and_a_left_subscription_drop_held_meters() {
        let outbox = Outbox::new();
        let t = Instant::now();
        outbox.pace_meters(Pace::Paused, t);
        outbox.value(meter("Kick", json!(0.5)));
        outbox.value(meter("Snare", json!(0.5)));
        let master = ValueItem::value(&meter_key("master", "Kick"), json!(0.7), None);
        outbox.value(master.clone());
        let bandx = ValueItem::value(&meter_key("bandx", "Kick"), json!(0.6), None);
        outbox.value(bandx.clone());
        outbox.forget(&meter_key("band", "Snare"));
        put_instance(&outbox, false);
        outbox.pace_meters(Pace::Full, t);
        assert_eq!(
            taken(&outbox, t),
            vec![instance(false), values(vec![bandx, master])]
        );
    }

    #[test]
    fn the_writer_wakes_for_the_sooner_of_a_frame_and_the_held_meters() {
        let t = Instant::now();
        let outbox = Outbox::new();
        outbox.pace_meters(every(250), t);
        assert_eq!(outbox.lock().wake(at(t, 300)), Wake::Woken, "nothing held");
        outbox.value(meter("Kick", json!(0.5)));
        assert_eq!(outbox.lock().wake(t), Wake::At(at(t, 250)));
        // A frame in flight and the next waiting for its ack until 1000.
        outbox.eq_frame(head_of(1), Bytes::from_static(b"a"));
        assert_eq!(outbox.lock().wake(t), Wake::Now, "a frame that may go");
        outbox.take_frame(t);
        outbox.eq_frame(head_of(1), Bytes::from_static(b"b"));
        assert_eq!(
            outbox.lock().wake(t),
            Wake::At(at(t, 250)),
            "the meters first"
        );
        assert_eq!(outbox.lock().wake(at(t, 250)), Wake::Now, "the meters due");
        taken(&outbox, at(t, 250));
        outbox.value(meter("Kick", json!(0.5)));
        assert_eq!(
            outbox.lock().wake(at(t, 300)),
            Wake::At(at(t, 500)),
            "the meters' next period first"
        );
        // A longer period: the frame's wait first.
        outbox.pace_meters(Pace::Full, at(t, 300));
        taken(&outbox, at(t, 300));
        outbox.pace_meters(every(2000), at(t, 300));
        outbox.value(meter("Kick", json!(0.5)));
        assert_eq!(
            outbox.lock().wake(at(t, 300)),
            Wake::At(at(t, 1000)),
            "the frame first"
        );
        // Paused: only the frame's wait counts.
        outbox.pace_meters(Pace::Paused, at(t, 300));
        assert_eq!(outbox.lock().wake(at(t, 300)), Wake::At(at(t, 1000)));
        assert_eq!(
            outbox.lock().wake(at(t, 1000)),
            Wake::Now,
            "its wait ran out"
        );
    }

    #[test]
    fn the_sooner_of_two_wakes() {
        let t = Instant::now();
        let (early, late) = (Wake::At(t), Wake::At(at(t, 1)));
        for (a, b, sooner_one) in [
            (Wake::Now, Wake::Now, Wake::Now),
            (Wake::Now, early, Wake::Now),
            (early, Wake::Now, Wake::Now),
            (Wake::Now, Wake::Woken, Wake::Now),
            (Wake::Woken, Wake::Now, Wake::Now),
            (early, late, early),
            (late, early, early),
            (late, Wake::Woken, late),
            (Wake::Woken, late, late),
            (Wake::Woken, Wake::Woken, Wake::Woken),
        ] {
            assert_eq!(sooner(a, b), sooner_one, "{a:?} {b:?}");
        }
    }

    #[tokio::test]
    async fn the_writer_wakes_by_itself_for_the_held_meters_and_for_the_wake_up() {
        let outbox = Arc::new(Outbox::new());
        let paced = Instant::now();
        outbox.pace_meters(every(100), paced);
        outbox.value(meter("Kick", json!(0.5)));
        let batch = tokio::time::timeout(Duration::from_secs(3), outbox.next_batch())
            .await
            .expect("woken when the period ran out")
            .unwrap();
        assert!(
            paced.elapsed() >= Duration::from_millis(100),
            "not before the period: {:?}",
            paced.elapsed()
        );
        assert_eq!(
            batch.messages,
            vec![values(vec![meter("Kick", json!(0.5))])]
        );
        // Paused: the writer sleeps until the page wakes.
        outbox.pace_meters(Pace::Paused, Instant::now());
        outbox.value(meter("Kick", json!(0.6)));
        let writer = Arc::clone(&outbox);
        let batch = tokio::spawn(async move { writer.next_batch().await });
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!batch.is_finished(), "held while paused");
        outbox.pace_meters(Pace::Full, Instant::now());
        let batch = tokio::time::timeout(Duration::from_millis(500), batch)
            .await
            .expect("woken by the wake")
            .unwrap()
            .unwrap();
        assert_eq!(
            batch.messages,
            vec![values(vec![meter("Kick", json!(0.6))])]
        );
    }

    #[tokio::test]
    async fn the_writer_wakes_by_itself_once_a_frames_wait_ran_out() {
        let outbox = Arc::new(Outbox::new());
        outbox.eq_frame(head_of(1), Bytes::from_static(b"a"));
        let sent = Instant::now();
        assert!(outbox.take_frame(sent).is_some());
        outbox.eq_frame(head_of(1), Bytes::from_static(b"b"));
        let writer = Arc::clone(&outbox);
        let batch = tokio::spawn(async move { writer.next_batch().await });
        let batch = tokio::time::timeout(Duration::from_secs(3), batch)
            .await
            .expect("woken when the wait ran out")
            .unwrap();
        assert!(
            sent.elapsed() >= flow::FRAME_ACK_WAIT,
            "not before the wait: {:?}",
            sent.elapsed()
        );
        assert_eq!(batch, frame_only(framed(1, 2, b"b")));
    }
}
