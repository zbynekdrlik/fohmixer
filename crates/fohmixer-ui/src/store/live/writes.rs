//! The controls' writes (#43): the glue between the controls, the intent
//! store (`store/intent.rs`, where every decision is made and tested) and
//! the socket — a write, its ack, its release and touch, the resend when an
//! instance is back, and what a control reads of its open write.

use fohmixer_proto::client::{AckItem, set_key};
use leptos::prelude::{UpdateValue, WithValue};
use serde_json::Value;

use super::{FailFn, LiveStore};
use crate::dom;
use crate::store::intent::{Acked, RESEND_MAX_AGE_MS, State};
use crate::store::write_key;

impl LiveStore {
    /// The hub's acks of the controls' writes: a failed write shows on its
    /// control (spec I6: shown, never retried).
    pub(super) fn on_ack(self, items: &[AckItem]) {
        for item in items {
            let Some(acked) = self.inner.try_update_value(|i| i.intents.ack(item)) else {
                return;
            };
            if let Acked::Failed(why, failed) = acked {
                dom::log(&format!("set {} failed: {why}", item.key));
                if let Some(failed) = failed {
                    failed(why);
                }
            }
        }
    }

    /// `instance` is back (#43, L4): its open writes go again, as new
    /// `set`s; a release too old to send blindly is `not_sent` now.
    pub(super) fn resend(self, instance: &str) {
        let Some(resend) = self
            .inner
            .try_update_value(|i| i.intents.resend(instance, dom::epoch_now()))
        else {
            return;
        };
        for key in &resend.not_sent {
            dom::log(&format!(
                "set {key} not sent again: released {RESEND_MAX_AGE_MS} ms ago or more"
            ));
        }
        let taken = resend.sets.iter().filter(|msg| self.send(msg)).count();
        if !resend.sets.is_empty() || !resend.not_sent.is_empty() {
            dom::log(&format!(
                "instance {instance} is back: {} writes sent again ({taken} taken), {} not sent",
                resend.sets.len(),
                resend.not_sent.len()
            ));
        }
    }

    /// A control's write (#43): `value` to `prop` of `target` on
    /// `instance`, through the intent store as a `set` (`is_final`: a
    /// release, a toggle or a tap); `failed` hears why the hub refused it
    /// (spec I6: shown, never retried). The intent stays open until its ack;
    /// one the socket cannot take now is kept and sent when its instance is
    /// back (L1, `resend`).
    pub fn set(
        self,
        instance: &str,
        target: &str,
        prop: &str,
        value: Value,
        is_final: bool,
        failed: Option<FailFn>,
    ) {
        let t = dom::epoch_now();
        let Some((key, msg)) = self.inner.try_update_value(|i| {
            i.intents
                .set((instance, target, prop), value, t, is_final, failed)
        }) else {
            return;
        };
        if !self.send(&msg) {
            dom::log(&format!(
                "set {key} kept: not connected to the hub, sent when it is back"
            ));
        }
    }

    /// The controls of `keys` were let go without a final write (#43, L4:
    /// the release time of their open writes).
    pub fn release(self, keys: &[String]) {
        let t = dom::epoch_now();
        let _ = self.inner.try_update_value(|i| {
            for key in keys {
                i.intents.release(key, t);
            }
        });
    }

    /// The controls of `keys` were touched again: a write not sent after a
    /// reconnect is dropped, one on its way is held again (L4).
    pub fn touch(self, keys: &[String]) {
        let _ = self.inner.try_update_value(|i| {
            for key in keys {
                i.intents.touch(key);
            }
        });
    }

    /// The state of `key`'s write now and, while one is open, its value as
    /// a number (a fader's cap and look, §4.2, §4.3).
    pub fn intent_view(self, key: &str) -> (State, Option<f64>) {
        self.inner
            .try_with_value(|i| {
                let state = i.intents.state(key, dom::epoch_now());
                let value = i.intents.open(key).and_then(|w| w.value.as_f64());
                (state, value)
            })
            .unwrap_or((State::Confirmed, None))
    }

    /// The value the write of `prop` of `target` on `instance` still on its
    /// way will leave (a toggle's tap inverts it); none for a not-sent one.
    pub fn pending_value(self, instance: &str, target: &str, prop: &str) -> Option<Value> {
        let key = set_key(instance, target, prop);
        self.inner
            .try_with_value(|i| i.intents.pending(&key).cloned())
            .flatten()
    }

    /// Live's fresh `value` of the subscription `sub_key` arrived: a not-sent
    /// write it equals is closed (Live holds it already).
    pub(super) fn live_value(self, sub_key: &str, value: &Value) {
        let _ = self
            .inner
            .try_update_value(|i| i.intents.live_value(write_key(sub_key), value));
    }
}
