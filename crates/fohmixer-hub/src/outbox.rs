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

pub mod flow;

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use bytes::Bytes;
use fohmixer_proto::client::{AckItem, DeckKey, ServerMsg, ValueItem};
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
    frame: Option<(u32, u32, Bytes)>,
    /// Its frames on their way (PR I): the next goes once acked.
    flight: Flight,
    /// The frames' round trips since the last `rate` record (PR I).
    trips: Trips,
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
        !self.has_messages() && self.frame.is_none()
    }

    /// When the writer has something to write at `now`: a message or the
    /// close at once; a frame waiting at once when it may go
    /// ([`Flight::due`]), else when its ack's wait runs out; nothing until
    /// woken (a frame that may not go with no wait to count, which the
    /// flight never gives, waits for the next ack or message: the writer
    /// never loops without awaiting).
    fn wake(&self, now: Instant) -> Wake {
        if self.closed || self.has_messages() {
            return Wake::Now;
        }
        match &self.frame {
            None => Wake::Woken,
            Some((session, _, _)) if self.flight.due(*session, now) => Wake::Now,
            Some((session, _, _)) => self
                .flight
                .wait_until(*session)
                .map_or(Wake::Woken, Wake::At),
        }
    }

    /// The messages waiting, in write order (see [`Outbox::take`]).
    fn take_messages(&mut self) -> Vec<ServerMsg> {
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
        let &(session, _, _) = self.frame.as_ref()?;
        if self.closed || !self.flight.due(session, now) {
            return None;
        }
        let (_, shown, jpeg) = self.frame.take()?;
        let (seq, waited_out) = self.flight.sent(session, now);
        if waited_out {
            self.trips.waited_out += 1;
        }
        let head = FrameHead {
            session,
            seq,
            shown,
        };
        Some(Bytes::from(frame(head, &jpeg)))
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

    /// The latest item of a subscription.
    pub fn value(&self, item: ValueItem) {
        self.put(|inner| {
            inner.values.insert(item.sub.clone(), item);
        });
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

    /// The newest picture of the client's Pro-Q 4 editor (#71 PR E) of
    /// `session`, a JPEG that shows the page's input `shown` (PR J): it
    /// replaces one still waiting. Its header goes on as it is taken
    /// ([`Batch`]).
    pub fn eq_frame(&self, session: u32, shown: u32, jpeg: Bytes) {
        self.put(|inner| inner.frame = Some((session, shown, jpeg)));
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

    /// Drops a pending item of a subscription the client left.
    pub fn forget(&self, sub: &str) {
        self.lock().values.remove(sub);
    }

    /// A state of an instance (an `instance` message), in order with the
    /// replies; `online` false drops the instance's pending values.
    pub fn instance(&self, name: &str, online: bool, msg: ServerMsg) {
        self.put(|inner| {
            if !online {
                let prefix = format!("{name}|");
                inner.values.retain(|sub, _| !sub.starts_with(&prefix));
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
    /// message, one `ack` message, then one `deck_keys` message of at most
    /// [`DECK_KEYS_PER_TAKE`] keys, the others left for the next take);
    /// `None` once closed. The frame stays.
    pub fn take(&self) -> Option<Vec<ServerMsg>> {
        let mut inner = self.lock();
        (!inner.closed).then(|| inner.take_messages())
    }

    /// Everything waiting at `now`, the frame that may go with the messages
    /// ([`Batch`]); `None` once closed.
    pub fn take_batch(&self, now: Instant) -> Option<Batch> {
        let mut inner = self.lock();
        if inner.closed {
            return None;
        }
        let messages = inner.take_messages();
        let frame = inner.take_frame(now);
        Some(Batch { messages, frame })
    }

    /// Waits until something is waiting (or the outbox closed) and takes it;
    /// deck keys left by the last take are something waiting, so the writer
    /// takes them at once, after what came meanwhile. A frame alone is
    /// something waiting too once it may go (PR I: its predecessor acked, or
    /// the ack's wait over, which wakes the writer by itself): the batch's
    /// messages are then none.
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
        let head = FrameHead {
            session,
            seq,
            shown: 0,
        };
        Some(Bytes::from(frame(head, jpeg)))
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
        outbox.eq_frame(1, 0, Bytes::from_static(b"old"));
        outbox.eq_frame(1, 0, Bytes::from_static(b"new"));
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
        outbox.eq_frame(1, 0, Bytes::from_static(b"next"));
        outbox.layout(3);
        assert_eq!(
            outbox.take_batch(t),
            Some(Batch {
                messages: vec![ServerMsg::Layout { rev: 3 }],
                frame: framed(1, 2, b"next"),
            })
        );
        // A closed editor's frame is dropped.
        outbox.eq_frame(1, 0, Bytes::from_static(b"gone"));
        outbox.forget_eq();
        assert!(outbox.lock().is_empty());
        assert_eq!(outbox.take_frame(t), None);
        // A closed outbox writes no frame.
        outbox.eq_frame(1, 0, Bytes::from_static(b"late"));
        outbox.close();
        assert_eq!(outbox.take_frame(t), None);
        assert_eq!(outbox.take_batch(t), None);
        outbox.eq_frame(1, 0, Bytes::from_static(b"after"));
        assert_eq!(
            outbox.lock().frame.as_ref().map(|(_, _, f)| &f[..]),
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
        outbox.eq_frame(1, 0, Bytes::from_static(b"a"));
        assert_eq!(
            outbox.take_batch(t),
            Some(Batch {
                messages: vec![result("open")],
                frame: framed(1, 1, b"a"),
            })
        );
        // A frame that may not go stays for a later batch.
        outbox.eq_frame(1, 0, Bytes::from_static(b"b"));
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
        outbox.eq_frame(1, 0, Bytes::from_static(b"a"));
        assert_eq!(outbox.take_frame(t), framed(1, 1, b"a"));
        // The next waits for the ack; a newer one replaces it meanwhile.
        outbox.eq_frame(1, 0, Bytes::from_static(b"b"));
        assert_eq!(outbox.take_frame(at(t, 10)), None);
        outbox.eq_frame(1, 0, Bytes::from_static(b"c"));
        assert_eq!(outbox.take_frame(at(t, 20)), None);
        // An ack of another session is nothing; the page's ack lets it go.
        outbox.frame_acked(2, 1, at(t, 25));
        assert_eq!(outbox.take_frame(at(t, 26)), None);
        outbox.frame_acked(1, 1, at(t, 30));
        assert_eq!(outbox.take_frame(at(t, 31)), framed(1, 2, b"c"));
        // No ack: the next goes once the wait (1 s) ran out, counted.
        outbox.eq_frame(1, 0, Bytes::from_static(b"d"));
        assert_eq!(outbox.take_frame(at(t, 1030)), None);
        assert_eq!(outbox.take_frame(at(t, 1031)), framed(1, 3, b"d"));
        // Two in flight: the next waits for an ack, however long.
        outbox.eq_frame(1, 0, Bytes::from_static(b"e"));
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
        outbox.eq_frame(2, 0, Bytes::from_static(b"f"));
        assert_eq!(outbox.take_frame(at(t, 5012)), framed(2, 1, b"f"));
    }

    #[test]
    fn a_closed_editor_forgets_its_frames_flight_and_hands_back_its_round_trips() {
        let outbox = Outbox::new();
        let t = Instant::now();
        outbox.eq_frame(1, 0, Bytes::from_static(b"a"));
        outbox.take_frame(t);
        outbox.frame_acked(1, 1, at(t, 20));
        outbox.eq_frame(1, 0, Bytes::from_static(b"b"));
        outbox.take_frame(at(t, 30));
        outbox.eq_frame(1, 0, Bytes::from_static(b"c"));
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
        outbox.eq_frame(1, 0, Bytes::from_static(b"d"));
        assert_eq!(outbox.take_frame(at(t, 40)), framed(1, 1, b"d"));
    }

    #[test]
    fn only_a_message_is_a_message_the_frame_is_not() {
        let outbox = Outbox::new();
        assert!(!outbox.lock().has_messages());
        outbox.eq_frame(1, 0, Bytes::from_static(b"a"));
        assert!(!outbox.lock().has_messages(), "a frame alone");
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
        outbox.eq_frame(1, 0, Bytes::from_static(b"a"));
        assert_eq!(outbox.lock().wake(t), Wake::Now, "a frame that may go");
        outbox.take_frame(t);
        assert_eq!(outbox.lock().wake(t), Wake::Woken, "nothing waits");
        outbox.eq_frame(1, 0, Bytes::from_static(b"b"));
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
        outbox.eq_frame(1, 0, Bytes::from_static(b"c"));
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
        outbox.eq_frame(1, 0, Bytes::from_static(b"a"));
        assert_eq!(outbox.next_batch().await, frame_only(framed(1, 1, b"a")));
        outbox.eq_frame(1, 0, Bytes::from_static(b"b"));
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

    #[tokio::test]
    async fn the_writer_wakes_by_itself_once_a_frames_wait_ran_out() {
        let outbox = Arc::new(Outbox::new());
        outbox.eq_frame(1, 0, Bytes::from_static(b"a"));
        let sent = Instant::now();
        assert!(outbox.take_frame(sent).is_some());
        outbox.eq_frame(1, 0, Bytes::from_static(b"b"));
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
