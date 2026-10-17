//! The router's half of REVERB (#130, D18): it follows the reverb faders'
//! volumes (its own subscriber, [`REVERB_CLIENT`]), hands the rule Live's
//! values, the transport, every page's write of a fader, the acks of its
//! last writes, a changed track list and its wakes, and carries out what
//! the rule returns (`rules::reverb` decides): each glide step as a `set` of
//! `REVERB_CLIENT` through the setter (one batch per tick and instance; the
//! latest value per key against the pages' writes), each step as a `reverb`
//! record and a log line, `hub-state.json` (the hub state saved whole), and
//! the state to every page.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use fohmixer_proto::client::{
    AckItem, HUB_REVERB_AUT, HUB_REVERB_LOW, ServerMsg, set_key, write_key_of,
};
use fohmixer_proto::layout::RuleFader;
use serde_json::{Value, json};

use super::markers::{RETURNS, TRACKS};
use super::writes::SetMsg;
use super::{Router, RouterMsg};
use crate::live::subs::{Cached, ClientId};
use crate::outbox::Outbox;
use crate::rules::reverb::{Act, Reverb, Saved, Step, step_line, step_warn};

/// The REVERB rule's subscriber and writer (clients count up from 1; the
/// unfold and marker keepers are `MAX` and `MAX − 1`).
pub const REVERB_CLIENT: ClientId = ClientId::MAX - 2;
/// The peer its writes name in the event log.
pub const REVERB_PEER: &str = "rule:reverb";

/// Whether a wake at `at` must be armed: none is on its way, or the one on
/// its way comes later.
pub(super) fn rearm(armed: Option<f64>, at: f64) -> bool {
    armed.is_none_or(|armed| armed > at)
}

/// How long the wake for `at` sleeps from `now` (router ms; never
/// negative: a need already due wakes at once).
pub(super) fn wake_in(at: f64, now: f64) -> std::time::Duration {
    std::time::Duration::from_secs_f64((at - now).max(0.0) / 1000.0)
}

/// The instance of a hub key that is its track list or return-track list
/// (an index then names another track).
pub(super) fn list_instance(sub: &str) -> Option<String> {
    let mut parts = sub.split('|');
    let instance = parts.next()?;
    let (target, prop) = (parts.next()?, parts.next()?);
    (target == "live_set" && (prop == TRACKS || prop == RETURNS)).then(|| instance.to_string())
}

/// Whether a list's value is a change: one heard before in this session,
/// and another one now (a list's first value after a connect is none: a
/// reconnect's values are judged as Live's own, design B.5).
pub(super) fn list_changed(before: Option<&Value>, value: &Value) -> bool {
    before.is_some_and(|before| before != value)
}

/// The router's REVERB.
pub(super) struct ReverbIo {
    pub rule: Reverb,
    /// Each fader's volume subscription: write key → hub key.
    pub watched: BTreeMap<String, String>,
    /// Each fader's glide's last write (`seq`), until its ack.
    pub finals: HashMap<String, u64>,
    /// The rule's write counter (its own `seq`).
    pub seq: u64,
    /// The latest armed wake's number, and when it comes (router ms).
    pub generation: u64,
    pub armed: Option<f64>,
    /// Each instance's track lists as last heard in its session (hub key →
    /// value).
    pub lists: BTreeMap<String, Value>,
}

impl ReverbIo {
    pub fn new(saved: &Saved) -> Self {
        Self {
            rule: Reverb::new(saved),
            watched: BTreeMap::new(),
            finals: HashMap::new(),
            seq: 0,
            generation: 0,
            armed: None,
            lists: BTreeMap::new(),
        }
    }
}

/// The `reverb` record of `step`, with the page that caused it (`client`,
/// `peer`).
pub(super) fn reverb_fields(step: &Step, client: Option<ClientId>, peer: Option<&str>) -> Value {
    let mut fields = serde_json::to_value(step).unwrap_or(Value::Null);
    if let Some(client) = client {
        fields["client"] = json!(client);
        fields["peer"] = json!(peer);
    }
    fields
}

impl Router {
    /// A new layout's reverb faders: their volumes followed, the rule told,
    /// then their cached values and the followed transport.
    pub(super) fn reverb_layout(&mut self, faders: Vec<RuleFader>) {
        let wanted: BTreeMap<String, (String, String)> = faders
            .iter()
            .map(|f| {
                (
                    set_key(&f.instance, &f.target, "value"),
                    (f.instance.clone(), f.target.clone()),
                )
            })
            .collect();
        let gone: Vec<String> = self
            .reverb
            .watched
            .keys()
            .filter(|k| !wanted.contains_key(*k))
            .cloned()
            .collect();
        for key in gone {
            if let Some(sub) = self.reverb.watched.remove(&key) {
                self.subs.unsubscribe(REVERB_CLIENT, &sub);
            }
        }
        let mut cached: Vec<(String, f64)> = Vec::new();
        for (key, (instance, target)) in wanted {
            if self.reverb.watched.contains_key(&key) {
                continue;
            }
            match self
                .subs
                .subscribe(REVERB_CLIENT, &instance, &target, "value", false)
            {
                Ok(reply) => {
                    if let Some(Cached::Value { value, .. }) = &reply.cached
                        && let Some(v) = value.as_f64()
                    {
                        cached.push((key.clone(), v));
                    }
                    self.reverb.watched.insert(key, reply.key);
                }
                Err((_, error)) => {
                    tracing::warn!(instance = %instance, target = %target, error = %error, "REVERB cannot follow a reverb fader");
                }
            }
        }
        let now = self.now_ms();
        let acts = self.reverb.rule.set_faders(faders, now);
        self.reverb_apply(acts, None);
        for (key, value) in cached {
            let now = self.now_ms();
            let acts = self.reverb.rule.live(&key, value, now);
            self.reverb_apply(acts, None);
        }
        if let Some(instance) = self.reverb.rule.followed().map(str::to_string)
            && let Some(playing) = self.transport_now(&instance)
        {
            self.reverb_transport(&instance, playing);
        }
    }

    /// A value of a fader's volume subscription (its hub key `sub`).
    pub(super) fn reverb_value(&mut self, sub: &str, value: f64) {
        let now = self.now_ms();
        let acts = self.reverb.rule.live(write_key_of(sub), value, now);
        self.reverb_apply(acts, None);
    }

    /// A value of a list the marker keeper follows: a changed track list
    /// stops its instance's glides.
    pub(super) fn reverb_list(&mut self, sub: &str, value: &Value) {
        let Some(instance) = list_instance(sub) else {
            return;
        };
        let before = self.reverb.lists.insert(sub.to_string(), value.clone());
        if list_changed(before.as_ref(), value) {
            self.reverb_tracks(&instance);
        }
    }

    /// A transport change of `instance`.
    pub(super) fn reverb_transport(&mut self, instance: &str, playing: bool) {
        let now = self.now_ms();
        let acts = self.reverb.rule.transport(instance, playing, now);
        self.reverb_apply(acts, None);
    }

    /// A page's write of `key` (`take_set`; never the rule's own).
    pub(super) fn reverb_hand(&mut self, key: &str, client: ClientId) {
        let acts = self.reverb.rule.hand(key);
        self.reverb_apply(acts, Some(client));
    }

    /// The ack of one of the rule's writes: its glide's last one settles
    /// the fader.
    pub(super) fn reverb_acked(&mut self, ack: &AckItem) {
        if self.reverb.finals.get(&ack.key) == Some(&ack.seq) {
            self.reverb.finals.remove(&ack.key);
            self.reverb.rule.applied(&ack.key);
        }
    }

    /// The armed wake came (a stale one does nothing).
    pub(super) fn reverb_wake(&mut self, generation: u64) {
        if generation != self.reverb.generation {
            return;
        }
        self.reverb.armed = None;
        let now = self.now_ms();
        let acts = self.reverb.rule.tick(now);
        self.reverb_apply(acts, None);
    }

    /// An instance went offline (its setter forgot the batch in flight; its
    /// lists start afresh with the next session).
    pub(super) fn reverb_offline(&mut self, instance: &str) {
        let prefix = format!("{instance}|");
        self.reverb
            .finals
            .retain(|key, _| !key.starts_with(&prefix));
        self.reverb.lists.retain(|key, _| !key.starts_with(&prefix));
        let acts = self.reverb.rule.offline(instance);
        self.reverb_apply(acts, None);
    }

    /// An instance's track list changed.
    pub(super) fn reverb_tracks(&mut self, instance: &str) {
        let acts = self.reverb.rule.tracks_changed(instance);
        self.reverb_apply(acts, None);
    }

    /// A page's `set_hub` of a REVERB key (`reverb_aut`, `reverb_low`):
    /// whether it was one.
    pub(super) fn reverb_set_hub(&mut self, client: ClientId, key: &str, value: &Value) -> bool {
        let now = self.now_ms();
        let acts = match (key, value.as_bool()) {
            (HUB_REVERB_AUT, Some(on)) => self.reverb.rule.set_aut(on, now),
            (HUB_REVERB_LOW, Some(low)) => self.reverb.rule.tap(low, now),
            _ => return false,
        };
        self.reverb_apply(acts, Some(client));
        true
    }

    /// A new page is told REVERB's state.
    pub(super) fn reverb_attach(&self, outbox: &Outbox) {
        outbox.reply(ServerMsg::Reverb(self.reverb.rule.state()));
    }

    /// Carries out the rule's acts (`client`: the page that caused them),
    /// then arms the next wake.
    pub(super) fn reverb_apply(&mut self, acts: Vec<Act>, client: Option<ClientId>) {
        let (mut save, mut tell) = (false, false);
        let mut instances: BTreeSet<String> = BTreeSet::new();
        for act in acts {
            match act {
                Act::Write {
                    instance,
                    target,
                    key,
                    value,
                    last,
                } => {
                    self.reverb.seq += 1;
                    let seq = self.reverb.seq;
                    if last {
                        self.reverb.finals.insert(key, seq);
                    }
                    let t = crate::live::wall_ms().unwrap_or(0.0);
                    self.take_set(SetMsg {
                        client: REVERB_CLIENT,
                        instance: instance.clone(),
                        target,
                        prop: "value".to_string(),
                        value: json!(value),
                        seq,
                        t,
                        is_final: last,
                        hub_ms: t,
                        offset_ms: Some(0.0),
                    });
                    instances.insert(instance);
                }
                Act::Step(step) => {
                    let line = step_line(&step);
                    if step_warn(&step) {
                        tracing::warn!("{line}");
                    } else {
                        tracing::info!("{line}");
                    }
                    let peer = client.and_then(|c| self.peers.get(&c)).map(String::as_str);
                    self.io
                        .events
                        .record("reverb", reverb_fields(&step, client, peer));
                }
                Act::Save => save = true,
                Act::Tell => tell = true,
            }
        }
        for instance in &instances {
            self.write_next(instance);
        }
        if save {
            self.state.reverb = self.reverb.rule.saved();
            self.save_state();
        }
        if tell {
            let state = self.reverb.rule.state();
            for outbox in self.clients.values() {
                outbox.reply(ServerMsg::Reverb(state.clone()));
            }
        }
        self.reverb_arm();
    }

    /// Arms one wake for the rule's next need (a sooner one replaces the
    /// one on its way: a newer generation, so the older does nothing).
    fn reverb_arm(&mut self) {
        let now = self.now_ms();
        let Some(at) = self.reverb.rule.next_tick(now) else {
            return;
        };
        if !rearm(self.reverb.armed, at) {
            return;
        }
        self.reverb.generation += 1;
        self.reverb.armed = Some(at);
        let generation = self.reverb.generation;
        let tx = self.io.tx.clone();
        let wait = wake_in(at, now);
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            let _ = tx.send(RouterMsg::ReverbWake { generation });
        });
    }
}

#[cfg(test)]
#[path = "reverb/tests.rs"]
mod tests;
