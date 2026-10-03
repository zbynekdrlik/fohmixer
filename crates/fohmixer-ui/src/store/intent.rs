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
    pub fn release(&mut self, _key: &str, _t: f64) {}

    /// The control of `key` was touched again: a `not_sent` intent is dropped
    /// (L4); one still on its way stays.
    pub fn touch(&mut self, _key: &str) {}

    /// The state of `key`'s write at page time `now`.
    pub fn state(&self, key: &str, _now: f64) -> State {
        if self.open.contains_key(key) {
            State::Sending
        } else {
            State::Confirmed
        }
    }

    /// `instance` is back (a reconnect's hello, or online again) at page
    /// time `now`: its intents as new `set`s (L4: a held control's, a release
    /// younger than `RESEND_MAX_AGE_MS`); an older release is `not_sent`.
    pub fn resend(&mut self, _instance: &str, _now: f64) -> Resend {
        Resend::default()
    }

    /// The latest write of `key` could not be sent: what hears its failure
    /// (the intent stays open).
    pub fn unsent(&mut self, key: &str) -> Option<F> {
        self.fails.remove(key)
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
mod tests {
    use super::*;
    use serde_json::json;

    const TARGET: &str = "live_set tracks[name=Hand1 #] mixer_device volume";
    const AT: (&str, &str, &str) = ("band", TARGET, "value");

    fn key() -> String {
        set_key("band", TARGET, "value")
    }

    #[test]
    fn a_write_is_a_set_with_the_next_sequence_number() {
        let mut intents = Intents::<&str>::default();
        assert!(intents.is_empty());
        let (key, msg) = intents.set(AT, json!(0.5), 1_000.5, false, None);
        assert!(!intents.is_empty());
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
            ("band", "live_set tracks 1", "mute"),
            json!(true),
            1_001.0,
            true,
            None,
        );
        assert!(matches!(
            mute,
            ClientMsg::Set {
                seq: 2,
                is_final: true,
                ..
            }
        ));
        let (_, again) = intents.set(AT, json!(0.6), 1_002.0, true, None);
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
        let mut intents = Intents::<&str>::default();
        intents.set(AT, json!(0.5), 1.0, false, Some("first"));
        intents.set(AT, json!(0.6), 2.0, false, Some("second"));
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
        assert_eq!(
            intents.handler(&key()),
            None,
            "a confirmed write's handler goes"
        );
        // Nothing open: stale.
        assert_eq!(
            intents.ack(&AckItem::applied(&key(), 2, None)),
            Acked::Stale
        );
        // A newer seq than the intent's closes it too.
        intents.set(AT, json!(0.7), 3.0, true, None);
        assert_eq!(
            intents.ack(&AckItem::applied(&key(), 9, Some(json!(0.7)))),
            Acked::Confirmed
        );
        assert!(intents.is_empty());
    }

    #[test]
    fn an_error_fails_with_the_latest_writes_handler() {
        let mut intents = Intents::<&str>::default();
        intents.set(AT, json!(0.4), 1.0, false, Some("old"));
        intents.set(AT, json!(0.5), 2.0, true, Some("flash"));
        assert_eq!(
            intents.ack(&AckItem::failed(&key(), 2, "instance offline")),
            Acked::Failed("instance offline".into(), Some("flash"))
        );
        assert!(intents.is_empty());
        // A write without a handler drops the one before.
        intents.set(AT, json!(0.5), 3.0, false, Some("flash"));
        intents.set(AT, json!(0.6), 4.0, false, None);
        assert_eq!(
            intents.ack(&AckItem::failed(&key(), 4, "refused")),
            Acked::Failed("refused".into(), None)
        );
    }

    #[test]
    fn a_superseded_ack_closes_only_at_its_sequence() {
        let mut intents = Intents::<&str>::default();
        intents.set(AT, json!(0.5), 2.0, false, Some("flash"));
        intents.set(AT, json!(0.6), 3.0, true, Some("flash"));
        // Seq 1 lost to another client, but this page's seq 2 is on its way.
        assert_eq!(intents.ack(&AckItem::superseded(&key(), 1)), Acked::Stale);
        assert_eq!(
            intents.ack(&AckItem::failed(&key(), 1, "late")),
            Acked::Stale
        );
        assert_eq!(intents.len(), 1);
        assert_eq!(
            intents.ack(&AckItem::superseded(&key(), 2)),
            Acked::Superseded
        );
        assert!(intents.is_empty());
        assert_eq!(
            intents.handler(&key()),
            None,
            "a superseded write's handler goes"
        );
        // Another key's ack leaves an intent alone.
        intents.set(AT, json!(0.1), 4.0, true, None);
        assert_eq!(
            intents.ack(&AckItem::applied("band|live_set|tempo", 3, None)),
            Acked::Stale
        );
        assert_eq!(intents.len(), 1);
    }

    #[test]
    fn a_write_the_socket_could_not_take_stays_open_with_its_handler() {
        let mut intents = Intents::<&str>::default();
        let (key, _) = intents.set(AT, json!(0.5), 1.0, true, Some("flash"));
        assert_eq!(intents.open(&key).map(|i| i.seq), Some(1), "still open");
        // A stale ack keeps the handler of the newer write.
        intents.set(AT, json!(0.6), 2.0, true, Some("newer"));
        assert_eq!(intents.ack(&AckItem::failed(&key, 1, "late")), Acked::Stale);
        assert_eq!(
            intents.ack(&AckItem::failed(&key, 2, "refused")),
            Acked::Failed("refused".into(), Some("newer"))
        );
    }

    #[test]
    fn each_state_has_its_name_its_openness_and_its_ghost() {
        let all = [
            State::Confirmed,
            State::Sending,
            State::Unconfirmed,
            State::NotSent,
        ];
        assert_eq!(
            all.map(State::name),
            ["confirmed", "sending", "unconfirmed", "not_sent"]
        );
        assert_eq!(all.map(State::is_open), [false, true, true, true]);
        assert_eq!(all.map(State::shows_ghost), [false, false, true, true]);
    }

    #[test]
    fn the_bounds_are_one_and_two_seconds() {
        assert_eq!(UNCONFIRMED_MS, 1000.0);
        assert_eq!(RESEND_MAX_AGE_MS, 2000.0);
    }

    #[test]
    fn a_release_is_unconfirmed_after_1000_ms_without_its_ack() {
        let mut intents = Intents::<&str>::default();
        assert_eq!(intents.state(&key(), 0.0), State::Confirmed, "no write");
        // A held control's write is on its way however long it waits.
        intents.set(AT, json!(0.5), 100.0, false, None);
        assert_eq!(intents.state(&key(), 100.0), State::Sending);
        assert_eq!(intents.state(&key(), 60_000.0), State::Sending, "held");
        // Let go at 200: 999 ms later it is still sending, 1000 ms later not.
        intents.release(&key(), 200.0);
        assert_eq!(intents.state(&key(), 1_199.0), State::Sending);
        assert_eq!(
            intents.state(&key(), 1_200.0_f64.next_down()),
            State::Sending
        );
        assert_eq!(intents.state(&key(), 1_200.0), State::Unconfirmed);
        assert_eq!(intents.state(&key(), 50_000.0), State::Unconfirmed);
        // A final write (a tap, a release with an unsent move) is let go at
        // its own time.
        let (mute, _) = intents.set(
            ("band", "live_set tracks 1", "mute"),
            json!(true),
            500.0,
            true,
            None,
        );
        assert_eq!(intents.state(&mute, 1_499.0), State::Sending);
        assert_eq!(intents.state(&mute, 1_500.0), State::Unconfirmed);
        // The ack closes it.
        intents.ack(&AckItem::applied(&mute, 2, None));
        assert_eq!(intents.state(&mute, 1_600.0), State::Confirmed);
        assert_eq!(intents.state(&key(), 1_600.0), State::Unconfirmed);
    }

    #[test]
    fn a_release_marks_only_an_open_write_and_only_once() {
        let mut intents = Intents::<&str>::default();
        intents.release(&key(), 50.0);
        assert_eq!(
            intents.state(&key(), 5_000.0),
            State::Confirmed,
            "nothing open"
        );
        assert!(intents.is_empty());
        intents.set(AT, json!(0.5), 100.0, false, None);
        assert_eq!(intents.open(&key()).map(|i| i.is_final), Some(false));
        intents.release(&key(), 200.0);
        intents.release(&key(), 900.0);
        assert_eq!(
            intents.state(&key(), 1_200.0),
            State::Unconfirmed,
            "the first release counts"
        );
        assert_eq!(
            intents.open(&key()).map(|i| i.is_final),
            Some(true),
            "a released write is final"
        );
        // A new touch's move is held again until its own release.
        intents.set(AT, json!(0.6), 1_300.0, false, None);
        assert_eq!(intents.state(&key(), 9_000.0), State::Sending);
    }

    /// The `set` of `(instance, target, prop)` in `sets`.
    fn set_of<'a>(sets: &'a [ClientMsg], prop_of: &str) -> &'a ClientMsg {
        sets.iter()
            .find(|m| matches!(m, ClientMsg::Set { prop, .. } if prop == prop_of))
            .expect("a set of that prop")
    }

    #[test]
    fn an_instance_back_gets_its_held_writes_and_releases_younger_than_2000_ms() {
        let mut intents = Intents::<&str>::default();
        let held_at = ("band", "live_set tracks 1", "panning");
        let (held, _) = intents.set(held_at, json!(-0.2), 10.0, false, None);
        let (young, _) = intents.set(AT, json!(0.6), 20.0, false, Some("flash"));
        intents.release(&young, 1_000.0);
        let (other, _) = intents.set(("master", TARGET, "value"), json!(0.3), 1_000.0, true, None);
        // The band instance is back 1999.x ms after the release.
        let now = 3_000.0_f64.next_down();
        let resend = intents.resend("band", now);
        assert!(resend.not_sent.is_empty());
        assert_eq!(resend.sets.len(), 2, "the master's write waits for its own");
        assert_eq!(
            set_of(&resend.sets, "panning"),
            &ClientMsg::Set {
                instance: "band".into(),
                target: "live_set tracks 1".into(),
                prop: "panning".into(),
                value: json!(-0.2),
                seq: 4,
                t: now,
                is_final: false,
            },
            "a held control's value, as a new set at the time it goes"
        );
        assert_eq!(
            set_of(&resend.sets, "value"),
            &ClientMsg::Set {
                instance: "band".into(),
                target: TARGET.into(),
                prop: "value".into(),
                value: json!(0.6),
                seq: 5,
                t: now,
                is_final: true,
            },
            "a release younger than 2000 ms, final"
        );
        // Only an ack of the new sequence numbers closes them.
        assert_eq!(
            intents.ack(&AckItem::applied(&young, 2, None)),
            Acked::Stale
        );
        assert_eq!(
            intents.ack(&AckItem::applied(&held, 4, None)),
            Acked::Confirmed
        );
        assert_eq!(
            intents.ack(&AckItem::failed(&young, 5, "refused")),
            Acked::Failed("refused".into(), Some("flash"))
        );
        // The master is back: its write goes, numbered after the others.
        let master = intents.resend("master", 1_500.0);
        assert!(matches!(
            master.sets.as_slice(),
            [ClientMsg::Set { seq: 6, .. }]
        ));
        assert_eq!(
            intents.open(&other).map(|i| (i.seq, i.t)),
            Some((6, 1_500.0))
        );
    }

    #[test]
    fn a_release_2000_ms_old_is_not_sent_again_but_kept_until_touched() {
        let mut intents = Intents::<&str>::default();
        let (key, _) = intents.set(AT, json!(0.6), 0.0, true, None);
        let resend = intents.resend("band", 2_000.0);
        assert!(resend.sets.is_empty(), "too old to send blindly (L4)");
        assert_eq!(resend.not_sent, vec![key.clone()]);
        assert_eq!(intents.state(&key, 2_000.0), State::NotSent);
        assert_eq!(intents.state(&key, 1e9), State::NotSent, "until touched");
        assert_eq!(
            intents.open(&key).map(|i| i.seq),
            Some(1),
            "unsent, not renumbered"
        );
        // A later reconnect neither sends it nor counts it again.
        assert_eq!(intents.resend("band", 2_100.0), Resend::default());
        // A touch on another key, or on a write still on its way, drops nothing.
        let (mute, _) = intents.set(
            ("band", "live_set tracks 1", "mute"),
            json!(true),
            2_200.0,
            true,
            None,
        );
        intents.touch(&mute);
        assert_eq!(intents.state(&mute, 2_300.0), State::Sending);
        assert_eq!(intents.len(), 2);
        // A touch on the not-sent control drops it: confirmed (nothing open).
        intents.touch(&key);
        assert_eq!(intents.state(&key, 2_300.0), State::Confirmed);
        assert_eq!(intents.len(), 1);
        // A release just younger than 2000 ms still goes.
        let resend = intents.resend("band", 4_200.0_f64.next_down());
        assert!(resend.not_sent.is_empty());
        assert!(matches!(
            resend.sets.as_slice(),
            [ClientMsg::Set {
                seq: 3,
                is_final: true,
                ..
            }]
        ));
        let resend = intents.resend("band", 4_200.0);
        assert_eq!(
            resend.not_sent,
            vec![mute.clone()],
            "and then it is too old"
        );
        // A new write on a not-sent key replaces it.
        intents.set(
            ("band", "live_set tracks 1", "mute"),
            json!(false),
            5_000.0,
            true,
            None,
        );
        assert_eq!(intents.state(&mute, 5_000.0), State::Sending);
    }
}
