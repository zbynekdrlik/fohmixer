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
//! decides; a closed tab drops what waits, `forget_deck`).

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};

use fohmixer_proto::client::{AckItem, DeckKey, ServerMsg, ValueItem};
use serde_json::Value;
use tokio::sync::Notify;

/// Replies waiting for one client before it counts as stuck.
pub const MAX_REPLIES: usize = 1024;

#[derive(Debug, Default)]
struct Inner {
    replies: VecDeque<ServerMsg>,
    hub: BTreeMap<String, Value>,
    layout: Option<u64>,
    links: BTreeMap<String, ServerMsg>,
    values: BTreeMap<String, ValueItem>,
    acks: BTreeMap<String, AckItem>,
    deck: BTreeMap<u32, DeckKey>,
    closed: bool,
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

    fn is_empty(&self) -> bool {
        self.replies.is_empty()
            && self.hub.is_empty()
            && self.layout.is_none()
            && self.links.is_empty()
            && self.values.is_empty()
            && self.acks.is_empty()
            && self.deck.is_empty()
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

    /// The client closed the tab: the keys waiting for it are dropped.
    pub fn forget_deck(&self) {
        self.lock().deck.clear();
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
    /// message, one `ack` message, then one `deck_keys` message); `None` once
    /// closed.
    pub fn take(&self) -> Option<Vec<ServerMsg>> {
        let mut inner = self.lock();
        if inner.closed {
            return None;
        }
        let mut out: Vec<ServerMsg> = inner.replies.drain(..).collect();
        out.extend(
            std::mem::take(&mut inner.hub)
                .into_iter()
                .map(|(key, value)| ServerMsg::Hub { key, value }),
        );
        if let Some(rev) = inner.layout.take() {
            out.push(ServerMsg::Layout { rev });
        }
        out.extend(std::mem::take(&mut inner.links).into_values());
        if !inner.values.is_empty() {
            out.push(ServerMsg::Values {
                items: std::mem::take(&mut inner.values).into_values().collect(),
            });
        }
        if !inner.acks.is_empty() {
            out.push(ServerMsg::Ack {
                items: std::mem::take(&mut inner.acks).into_values().collect(),
            });
        }
        if !inner.deck.is_empty() {
            out.push(ServerMsg::DeckKeys {
                items: std::mem::take(&mut inner.deck).into_values().collect(),
            });
        }
        Some(out)
    }

    /// Waits until something is waiting (or the outbox closed) and takes it.
    pub async fn next_batch(&self) -> Option<Vec<ServerMsg>> {
        loop {
            {
                let inner = self.lock();
                if inner.closed || !inner.is_empty() {
                    drop(inner);
                    return self.take();
                }
            }
            self.ready.notified().await;
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
            .unwrap();
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
        assert_eq!(got, Some(vec![ServerMsg::Layout { rev: 4 }]));
        // Something already waiting is taken at once.
        outbox.layout(5);
        assert_eq!(
            outbox.next_batch().await,
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
}
