//! The page's write intents (#43, design note §4.1; the minimal PR A
//! store): a control's write goes here, never straight to the socket. Per
//! key (`instance|target|prop`) it keeps the latest value the engineer set,
//! with its sequence number (strictly increasing over all of this page's
//! sets), its page time and whether it was final, until the hub acks a
//! sequence number at least as new. An `error` ack fails it (the control
//! flashes red, as before); a `superseded` ack (another client's newer
//! write) closes it.
//!
//! PR B adds the states (`unconfirmed`, `not_sent`), the resend after a
//! reconnect and what a fader draws from them; here an intent only waits
//! for its ack.

use std::collections::BTreeMap;

use fohmixer_proto::client::{AckItem, ClientMsg, set_key};
use serde_json::Value;

/// One key's latest write.
#[derive(Debug, Clone, PartialEq)]
pub struct Intent {
    pub seq: u64,
    pub value: Value,
    /// The page's clock when it was set (ms since the time origin's epoch).
    pub t: f64,
    pub is_final: bool,
}

/// What an ack did to its key's intent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Acked {
    /// It answers an older write (a newer one is on its way), or no intent
    /// is open: nothing changes.
    Stale,
    /// Live ran it: the intent is closed.
    Confirmed,
    /// Another client's newer write replaced it: the intent is closed.
    Superseded,
    /// Live refused it, or it never got there: the intent is closed and the
    /// control shows why.
    Failed(String),
}

/// The page's open intents.
#[derive(Debug, Default)]
pub struct Intents {
    last_seq: u64,
    open: BTreeMap<String, Intent>,
}

impl Intents {
    /// A control's write of `value` to `prop` of `target` on `instance` at
    /// page time `t`: its key and the `set` to send.
    pub fn set(
        &mut self,
        instance: &str,
        target: &str,
        prop: &str,
        value: Value,
        t: f64,
        is_final: bool,
    ) -> (String, ClientMsg) {
        self.last_seq += 1;
        let key = set_key(instance, target, prop);
        self.open.insert(
            key.clone(),
            Intent {
                seq: self.last_seq,
                value: value.clone(),
                t,
                is_final,
            },
        );
        let msg = ClientMsg::Set {
            instance: instance.to_string(),
            target: target.to_string(),
            prop: prop.to_string(),
            value,
            seq: self.last_seq,
            t,
            is_final,
        };
        (key, msg)
    }

    /// An ack item from the hub.
    pub fn ack(&mut self, item: &AckItem) -> Acked {
        let Some(intent) = self.open.get(&item.key) else {
            return Acked::Stale;
        };
        if item.seq < intent.seq {
            return Acked::Stale;
        }
        self.open.remove(&item.key);
        match (&item.error, item.superseded) {
            (Some(why), _) => Acked::Failed(why.clone()),
            (None, true) => Acked::Superseded,
            (None, false) => Acked::Confirmed,
        }
    }

    /// The open intent of `key`.
    pub fn open(&self, key: &str) -> Option<&Intent> {
        self.open.get(key)
    }

    /// How many intents wait for their ack.
    pub fn len(&self) -> usize {
        self.open.len()
    }

    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const TARGET: &str = "live_set tracks[name=Hand1 #] mixer_device volume";

    fn key() -> String {
        set_key("band", TARGET, "value")
    }

    #[test]
    fn a_write_is_a_set_with_the_next_sequence_number() {
        let mut intents = Intents::default();
        assert!(intents.is_empty());
        let (key, msg) = intents.set("band", TARGET, "value", json!(0.5), 1_000.5, false);
        assert_eq!(
            key,
            "band|live_set tracks[name=Hand1 #] mixer_device volume|value"
        );
        assert_eq!(
            msg,
            ClientMsg::Set {
                instance: "band".into(),
                target: TARGET.into(),
                prop: "value".into(),
                value: json!(0.5),
                seq: 1,
                t: 1_000.5,
                is_final: false,
            }
        );
        // The sequence is the page's, over every key.
        let (_, mute) = intents.set(
            "band",
            "live_set tracks 1",
            "mute",
            json!(true),
            1_001.0,
            true,
        );
        assert!(matches!(
            mute,
            ClientMsg::Set {
                seq: 2,
                is_final: true,
                ..
            }
        ));
        let (_, again) = intents.set("band", TARGET, "value", json!(0.6), 1_002.0, true);
        assert!(matches!(again, ClientMsg::Set { seq: 3, .. }));
        // One intent per key: the latest.
        assert_eq!(intents.len(), 2);
        assert_eq!(
            intents.open(&key),
            Some(&Intent {
                seq: 3,
                value: json!(0.6),
                t: 1_002.0,
                is_final: true,
            })
        );
    }

    #[test]
    fn an_ack_at_least_as_new_closes_the_intent() {
        let mut intents = Intents::default();
        intents.set("band", TARGET, "value", json!(0.5), 1.0, false);
        intents.set("band", TARGET, "value", json!(0.6), 2.0, false);
        // The ack of seq 1: a newer write is on its way.
        assert_eq!(
            intents.ack(&AckItem::applied(&key(), 1, None)),
            Acked::Stale
        );
        assert_eq!(intents.open(&key()).map(|i| i.seq), Some(2));
        assert_eq!(
            intents.ack(&AckItem::applied(&key(), 2, None)),
            Acked::Confirmed
        );
        assert!(intents.is_empty());
        // Nothing open: stale.
        assert_eq!(
            intents.ack(&AckItem::applied(&key(), 2, None)),
            Acked::Stale
        );
        // A newer seq than the intent's closes it too.
        intents.set("band", TARGET, "value", json!(0.7), 3.0, true);
        assert_eq!(
            intents.ack(&AckItem::applied(&key(), 9, Some(json!(0.7)))),
            Acked::Confirmed
        );
        assert!(intents.is_empty());
    }

    #[test]
    fn an_error_fails_and_a_superseded_ack_closes_only_at_its_sequence() {
        let mut intents = Intents::default();
        intents.set("band", TARGET, "value", json!(0.5), 1.0, true);
        assert_eq!(
            intents.ack(&AckItem::failed(&key(), 1, "instance offline")),
            Acked::Failed("instance offline".into())
        );
        assert!(intents.is_empty());
        intents.set("band", TARGET, "value", json!(0.5), 2.0, false);
        intents.set("band", TARGET, "value", json!(0.6), 3.0, true);
        // Seq 2 lost to another client, but this page's seq 3 is on its way.
        assert_eq!(intents.ack(&AckItem::superseded(&key(), 2)), Acked::Stale);
        assert_eq!(
            intents.ack(&AckItem::failed(&key(), 2, "late")),
            Acked::Stale
        );
        assert_eq!(intents.len(), 1);
        assert_eq!(
            intents.ack(&AckItem::superseded(&key(), 3)),
            Acked::Superseded
        );
        assert!(intents.is_empty());
        // Another key's ack leaves an intent alone.
        intents.set("band", TARGET, "value", json!(0.1), 4.0, true);
        assert_eq!(
            intents.ack(&AckItem::applied("band|live_set|tempo", 4, None)),
            Acked::Stale
        );
        assert_eq!(intents.len(), 1);
    }
}
