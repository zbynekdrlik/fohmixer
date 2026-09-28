//! A client connection's outbox (S3 design note §3; spec §2.4 fan-out): what
//! waits to be written to one client, bounded.
//!
//! Values keep only the latest item per subscription while a send is
//! pending (a slow client costs memory bounded by its subscriptions, never
//! a queue), and so do the instance states and hub values. Replies
//! (`result`, `subbed`, `error`) keep their order; more than
//! [`MAX_REPLIES`] waiting means the client stopped reading: the outbox
//! closes and the connection ends (the client reconnects and resyncs).
//! The router writes here under a short lock and never waits for a client.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};

use fohmixer_proto::client::{ServerMsg, ValueItem};
use serde_json::Value;
use tokio::sync::Notify;

/// Replies waiting for one client before it counts as stuck.
pub const MAX_REPLIES: usize = 1024;

#[derive(Debug, Default)]
struct Inner {
    replies: VecDeque<ServerMsg>,
    instances: BTreeMap<String, ServerMsg>,
    hub: BTreeMap<String, Value>,
    layout: Option<u64>,
    values: BTreeMap<String, ValueItem>,
    closed: bool,
}

impl Inner {
    fn is_empty(&self) -> bool {
        self.replies.is_empty()
            && self.instances.is_empty()
            && self.hub.is_empty()
            && self.layout.is_none()
            && self.values.is_empty()
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
        self.put(|inner| {
            if inner.replies.len() >= MAX_REPLIES {
                tracing::warn!(
                    waiting = inner.replies.len(),
                    "a client stopped reading its replies: closing it"
                );
                inner.closed = true;
            } else {
                inner.replies.push_back(msg);
            }
        });
    }

    /// The latest item of a subscription.
    pub fn value(&self, item: ValueItem) {
        self.put(|inner| {
            inner.values.insert(item.sub.clone(), item);
        });
    }

    /// Drops a pending item of a subscription the client left.
    pub fn forget(&self, sub: &str) {
        self.lock().values.remove(sub);
    }

    /// The latest state of an instance (an `instance` message).
    pub fn instance(&self, name: &str, msg: ServerMsg) {
        self.put(|inner| {
            inner.instances.insert(name.to_string(), msg);
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

    /// Everything waiting, in write order (replies, instance states, hub
    /// values, the layout revision, then one `values` message); `None` once
    /// closed.
    pub fn take(&self) -> Option<Vec<ServerMsg>> {
        let mut inner = self.lock();
        if inner.closed {
            return None;
        }
        let mut out: Vec<ServerMsg> = inner.replies.drain(..).collect();
        out.extend(std::mem::take(&mut inner.instances).into_values());
        out.extend(
            std::mem::take(&mut inner.hub)
                .into_iter()
                .map(|(key, value)| ServerMsg::Hub { key, value }),
        );
        if let Some(rev) = inner.layout.take() {
            out.push(ServerMsg::Layout { rev });
        }
        if !inner.values.is_empty() {
            out.push(ServerMsg::Values {
                items: std::mem::take(&mut inner.values).into_values().collect(),
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
    fn the_write_order_is_replies_states_hub_layout_values() {
        let outbox = Outbox::new();
        outbox.value(ValueItem::value("a", json!(1), None));
        outbox.layout(1);
        outbox.layout(2);
        outbox.hub("stage_aut", json!(false));
        outbox.hub("stage_aut", json!(true));
        outbox.instance("band", instance(false));
        outbox.instance("band", instance(true));
        outbox.reply(result("1"));
        outbox.reply(result("2"));
        assert_eq!(
            outbox.take().unwrap(),
            vec![
                result("1"),
                result("2"),
                instance(true),
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
