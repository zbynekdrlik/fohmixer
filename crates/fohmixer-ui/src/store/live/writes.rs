//! The controls' writes (#43): the glue between the controls, the intent
//! store (`store/intent.rs`, where every decision is made and tested) and
//! the socket — a write, its ack, its release and touch, the resend when an
//! instance is back, and what a fader reads of its open write. A send the
//! socket took tells the page's flight recorder (`diag::trace`) to wait
//! (`Recorder::set_went`, #43 PR D: no batch in front of the next frame's
//! set); a write's turn to `unconfirmed` or `not_sent` is an event of the
//! recorder (`note_intents`, #43 PR E). Sends and acks are not: the hub's
//! own `set` and `ack` records hold them (PR E).

use fohmixer_proto::client::{AckItem, ClientMsg};
use leptos::prelude::{UpdateValue, WithValue};
use serde_json::Value;

use super::{FailFn, LiveStore};
use crate::diag::{self, trace, trace::Recorder};
use crate::dom;
use crate::store::intent::{Acked, RESEND_MAX_AGE_MS, State};
use crate::store::write_key;

impl LiveStore {
    /// The hub's acks of the controls' writes: a failed write shows on its
    /// control (spec I6: shown, never retried).
    pub(super) fn on_ack(self, items: &[AckItem]) {
        // A write these acks close may have turned unconfirmed since the
        // last tick: the recorder hears it before it closes.
        self.note_intents();
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
        // Each write kept back is `not_sent` now: the recorder hears it at
        // once (Live's value replayed after the hello can close it within
        // milliseconds, before the next tick).
        self.note_intents();
        let mut taken = 0;
        for msg in &resend.sets {
            let sent = self.sent_set(msg);
            taken += usize::from(sent);
        }
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
    /// back (L1, `resend`). Its sequence number (a fader's move record names
    /// it, #43 PR D).
    pub fn set(
        self,
        instance: &str,
        target: &str,
        prop: &str,
        value: Value,
        is_final: bool,
        failed: Option<FailFn>,
    ) -> Option<u64> {
        // A new write replaces the key's open one: the recorder hears that
        // one's turn first.
        self.note_intents();
        let t = dom::epoch_now();
        let (key, msg) = self.inner.try_update_value(|i| {
            i.intents
                .set((instance, target, prop), value, t, is_final, failed)
        })?;
        if !self.sent_set(&msg) {
            dom::log(&format!(
                "set {key} kept: not connected to the hub, sent when it is back"
            ));
        }
        trace::seq_of(&msg)
    }

    /// Sends a `set`: whether the socket took it. One it took holds the
    /// flight recorder's next batch (#43 PR D); one it did not take stays
    /// open in the intent store, its frame's move record names it, and the
    /// recorder hears if it is not sent again (`note_intents`).
    fn sent_set(self, msg: &ClientMsg) -> bool {
        let sent = self.send(msg);
        if sent {
            let _ = diag::with_trace(Recorder::set_went);
        }
        sent
    }

    /// The writes that turned `unconfirmed` or `not_sent` since the last
    /// call go into the flight recorder (#43 PR E, `Intents::changes`): only
    /// the page knows when they turned. From the link's tick, after a
    /// resend, and before acks, a set or a touch close or reset a write.
    pub(super) fn note_intents(self) {
        let now = dom::epoch_now();
        let changes = self
            .inner
            .try_update_value(|i| i.intents.changes(now))
            .unwrap_or_default();
        for change in &changes {
            diag::record(&trace::intent(
                change.at,
                &change.key,
                change.seq,
                change.state.name(),
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
        // A touch drops a not-sent write or holds one again (its turn is
        // told anew after the next release): the recorder hears it first.
        self.note_intents();
        let _ = self.inner.try_update_value(|i| {
            for key in keys {
                i.intents.touch(key);
            }
        });
    }

    /// The state of `key`'s write now (the look of a pan or a toggle, #43
    /// PR C).
    pub fn intent_state(self, key: &str) -> State {
        self.inner
            .try_with_value(|i| i.intents.state(key, dom::epoch_now()))
            .unwrap_or(State::Confirmed)
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

    /// Live's fresh `value` of the subscription `sub_key` arrived: a not-sent
    /// write it equals is closed (Live holds it already).
    pub(super) fn live_value(self, sub_key: &str, value: &Value) {
        let _ = self
            .inner
            .try_update_value(|i| i.intents.live_value(write_key(sub_key), value));
    }
}
