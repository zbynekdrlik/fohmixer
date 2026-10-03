//! The page's write intents (#43, design note §4.1): a control's write goes
//! here, never straight to the socket. Per key (`instance|target|prop`) it
//! keeps the latest value the engineer set, with its sequence number
//! (strictly increasing over all of this page's sets), its page time and
//! whether it was final, until the hub acks a sequence number at least as
//! new. An `error` ack fails it (the control flashes red); a `superseded`
//! ack (another client's newer write) closes it.
//!
//! Each key also keeps what hears its latest write's failure (`F`: the
//! store's flash callback, a plain value in the tests), handed back with a
//! failed ack.
//!
//! The states (PR B, L1–L4): an open intent is `sending`, `unconfirmed` once
//! its release is `UNCONFIRMED_MS` old without an ack, or `not_sent` when it
//! was not sent again after a reconnect because its release was
//! `RESEND_MAX_AGE_MS` old or older; a closed one is `confirmed`. A control
//! still held (no release) is never `unconfirmed` and is always sent again.
//! When an instance is back (after a reconnect's hello, or online again) the
//! store sends its intents again (`resend`), each as a new `set`.

use std::collections::BTreeMap;

use fohmixer_proto::client::{AckItem, ClientMsg, set_key};
use serde_json::Value;

/// A release this old (ms) without its ack is `unconfirmed` (L3, §4.3).
pub const UNCONFIRMED_MS: f64 = 1000.0;
/// After a reconnect a release is sent again only while younger than this
/// (ms); an older one is `not_sent` (L4).
pub const RESEND_MAX_AGE_MS: f64 = 2000.0;

/// What a key's write is waiting for (§4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// No open intent: the last write was acked (or there was none).
    Confirmed,
    /// On its way, or held under a finger.
    Sending,
    /// Released `UNCONFIRMED_MS` ago or more, no ack yet.
    Unconfirmed,
    /// Its release was too old to send again after a reconnect (L4): kept
    /// for the look until the control is touched again.
    NotSent,
}

impl State {
    /// Whether an intent is open (the fader shows it, not Live's value).
    pub fn is_open(self) -> bool {
        self != Self::Confirmed
    }

    /// Whether the fader shows a ghost at Live's value (§4.3).
    pub fn shows_ghost(self) -> bool {
        matches!(self, Self::Unconfirmed | Self::NotSent)
    }

    /// The control's `data-intent`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Confirmed => "confirmed",
            Self::Sending => "sending",
            Self::Unconfirmed => "unconfirmed",
            Self::NotSent => "not_sent",
        }
    }
}

/// What a resend does (`Intents::resend`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resend {
    /// The `set`s to send, one per key.
    pub sets: Vec<ClientMsg>,
    /// The keys whose release was too old: `not_sent` now.
    pub not_sent: Vec<String>,
}

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
pub enum Acked<F> {
    /// It answers an older write (a newer one is on its way), or no intent
    /// is open: nothing changes.
    Stale,
    /// Live ran it: the intent is closed.
    Confirmed,
    /// Another client's newer write replaced it: the intent is closed.
    Superseded,
    /// Live refused it, or it never got there: the intent is closed, and
    /// its failure handler shows why.
    Failed(String, Option<F>),
}

/// An open intent: the write, where it goes, and how far it got.
#[derive(Debug, Clone, PartialEq)]
struct Open {
    intent: Intent,
    instance: String,
    target: String,
    prop: String,
    /// When the control was let go (page ms); `None` while it is held.
    released_at: Option<f64>,
    /// Not sent again after a reconnect: its release was too old (L4).
    not_sent: bool,
}

impl Open {
    /// The `set` of this intent.
    fn set_msg(&self) -> ClientMsg {
        ClientMsg::Set {
            instance: self.instance.clone(),
            target: self.target.clone(),
            prop: self.prop.clone(),
            value: self.intent.value.clone(),
            seq: self.intent.seq,
            t: self.intent.t,
            is_final: self.intent.is_final,
        }
    }
}

/// Whether a release `age` ms old without its ack is `unconfirmed`.
fn is_unconfirmed(age: f64) -> bool {
    age >= UNCONFIRMED_MS
}

/// Two numbers this close (relative above 1, absolute below) are one value:
/// Live keeps a value as float32.
const SAME_VALUE: f64 = 1e-6;

/// Whether Live's number `live` is the write's `wrote`.
fn same_number(wrote: f64, live: f64) -> bool {
    (wrote - live).abs() <= SAME_VALUE * wrote.abs().max(1.0)
}

/// Whether Live's `live` is the write's `wrote` (numbers as `same_number`,
/// anything else exactly).
fn same_value(wrote: &Value, live: &Value) -> bool {
    match (wrote.as_f64(), live.as_f64()) {
        (Some(w), Some(l)) => same_number(w, l),
        _ => wrote == live,
    }
}

/// Whether a release `age` ms old is sent again after a reconnect (L4).
fn may_resend(age: f64) -> bool {
    age < RESEND_MAX_AGE_MS
}

/// The page's open intents.
#[derive(Debug)]
pub struct Intents<F> {
    last_seq: u64,
    open: BTreeMap<String, Open>,
    /// What hears the failure of each key's latest write.
    fails: BTreeMap<String, F>,
}

impl<F> Default for Intents<F> {
    fn default() -> Self {
        Self {
            last_seq: 0,
            open: BTreeMap::new(),
            fails: BTreeMap::new(),
        }
    }
}

impl<F> Intents<F> {
    /// A control's write of `value` to `(instance, target, prop)` at page
    /// time `t`, `fail` hearing its failure: its key and the `set` to send.
    pub fn set(
        &mut self,
        (instance, target, prop): (&str, &str, &str),
        value: Value,
        t: f64,
        is_final: bool,
        fail: Option<F>,
    ) -> (String, ClientMsg) {
        self.last_seq += 1;
        let key = set_key(instance, target, prop);
        let open = Open {
            intent: Intent {
                seq: self.last_seq,
                value,
                t,
                is_final,
            },
            instance: instance.to_string(),
            target: target.to_string(),
            prop: prop.to_string(),
            released_at: is_final.then_some(t),
            not_sent: false,
        };
        let msg = open.set_msg();
        self.open.insert(key.clone(), open);
        match fail {
            Some(fail) => self.fails.insert(key.clone(), fail),
            None => self.fails.remove(&key),
        };
        (key, msg)
    }

    /// The control of `key` was let go at page time `t` without a final
    /// write (its last move had gone out in a frame, or a glide ended): its
    /// open intent counts as released from then. A second release keeps the
    /// first time.
    pub fn release(&mut self, key: &str, t: f64) {
        if let Some(open) = self.open.get_mut(key)
            && open.released_at.is_none()
        {
            open.released_at = Some(t);
            open.intent.is_final = true;
        }
    }

    /// The control of `key` was touched again: a `not_sent` intent is dropped
    /// with its handler (L4); one still on its way is held again (no release
    /// time until the touch's own release, so a reconnect meanwhile sends it
    /// as a held control's value).
    pub fn touch(&mut self, key: &str) {
        let Some(open) = self.open.get_mut(key) else {
            return;
        };
        if open.not_sent {
            self.open.remove(key);
            self.fails.remove(key);
        } else {
            open.released_at = None;
            open.intent.is_final = false;
        }
    }

    /// Live's fresh `value` of `key` arrived (#43): a `not_sent` write it
    /// equals is closed (Live holds it already: the hub applied it before
    /// the link went, and its ack was lost with the socket).
    pub fn live_value(&mut self, key: &str, value: &Value) {
        if self
            .open
            .get(key)
            .is_some_and(|open| open.not_sent && same_value(&open.intent.value, value))
        {
            self.open.remove(key);
            self.fails.remove(key);
        }
    }

    /// The value `key`'s write still on its way will leave (a toggle's tap
    /// inverts it, #43): none when nothing is open or the write is
    /// `not_sent` (it never reached Live).
    pub fn pending(&self, key: &str) -> Option<&Value> {
        self.open
            .get(key)
            .filter(|open| !open.not_sent)
            .map(|open| &open.intent.value)
    }

    /// The state of `key`'s write at page time `now`.
    pub fn state(&self, key: &str, now: f64) -> State {
        let Some(open) = self.open.get(key) else {
            return State::Confirmed;
        };
        if open.not_sent {
            return State::NotSent;
        }
        match open.released_at {
            Some(at) if is_unconfirmed(now - at) => State::Unconfirmed,
            _ => State::Sending,
        }
    }

    /// `instance` is back (a reconnect's hello, or online again) at page
    /// time `now`: its intents as new `set`s (L4: a held control's, a release
    /// younger than `RESEND_MAX_AGE_MS`); an older release is `not_sent`.
    pub fn resend(&mut self, instance: &str, now: f64) -> Resend {
        let Self { last_seq, open, .. } = self;
        let mut out = Resend::default();
        let back = open
            .iter_mut()
            .filter(|(_, o)| o.instance == instance && !o.not_sent);
        for (key, o) in back {
            if o.released_at.is_some_and(|at| !may_resend(now - at)) {
                o.not_sent = true;
                out.not_sent.push(key.clone());
            } else {
                *last_seq += 1;
                o.intent.seq = *last_seq;
                o.intent.t = now;
                out.sets.push(o.set_msg());
            }
        }
        out
    }

    /// An ack item from the hub.
    pub fn ack(&mut self, item: &AckItem) -> Acked<F> {
        let Some(open) = self.open.get(&item.key) else {
            return Acked::Stale;
        };
        if item.seq < open.intent.seq {
            return Acked::Stale;
        }
        self.open.remove(&item.key);
        let fail = self.fails.remove(&item.key);
        match (&item.error, item.superseded) {
            (Some(why), _) => Acked::Failed(why.clone(), fail),
            (None, true) => Acked::Superseded,
            (None, false) => Acked::Confirmed,
        }
    }

    /// The open intent of `key`.
    pub fn open(&self, key: &str) -> Option<&Intent> {
        self.open.get(key).map(|open| &open.intent)
    }

    /// What hears `key`'s failure (the tests' view of the handlers).
    #[cfg(test)]
    fn handler(&self, key: &str) -> Option<&F> {
        self.fails.get(key)
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
mod tests;
