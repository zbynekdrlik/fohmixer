//! The router's half of AUTOTUNE (#104, D19): it finds each `+AUTOTUNE`
//! target's `Latencies` rack (`rules::autotune::find`, a bounded walk per
//! target, a newer walk voiding an older answer), follows its first macro's
//! value (its own subscriber, [`AUTOTUNE_CLIENT`]), hands the rule
//! (`rules::autotune`) the layout's Live sets, their transport, the pages'
//! flag and taps, and carries out its writes: each Live set's found macros
//! written through their `$ref` in one `live.call` batch (never by an index
//! path: a device inserted above would move it), each step an `autotune`
//! record and a log line, AUTOTUNE AUT in `hub-state.json` (saved whole),
//! and the state to every page when it changed.
//!
//! The refs die with the script's connection: a disconnect drops every
//! found ref and value of that Live set and voids its walks in flight
//! before anything else; its connect walks them again. A write that failed
//! walks its target again (a stale ref, a deleted rack); a track-list
//! change walks the set's targets again (an index then names another
//! track), keeping what they found until the answer.

use std::collections::{BTreeMap, BTreeSet};

use fohmixer_proto::client::{
    AutotuneState, AutotuneStatus, AutotuneTargetState, AutotuneTargetStatus, HUB_AUTOTUNE,
    HUB_AUTOTUNE_AUT, ServerMsg, TargetState,
};
use fohmixer_proto::layout::AutotuneTarget;
use serde_json::{Value, json};

use super::{Router, RouterMsg, transport_heard};
use crate::eq::walk::refused;
use crate::live::client::LiveHandle;
use crate::live::subs::{Cached, ClientId};
use crate::outbox::Outbox;
use crate::rules::autotune::find::{self, Find, Found};
use crate::rules::autotune::{AutotuneAut, Step, Why, step_line, step_warn, target_state};

/// AUTOTUNE's subscriber of its targets' values (clients count up from 1;
/// the unfold and marker keepers and REVERB are `MAX`, `MAX − 1`,
/// `MAX − 2`).
pub const AUTOTUNE_CLIENT: ClientId = ClientId::MAX - 3;
/// A walk that read [`find::MAX_READS`] times and did not end.
pub const TOO_DEEP: &str = "the devices nest too deep";
/// The `$ref` class of a rack's macro (the script refuses a ref to anything
/// else).
pub const PARAMETER_CLASS: &str = "DeviceParameter";

/// One target of the layout, and what the hub found of it.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Target {
    pub spec: AutotuneTarget,
    pub instance: String,
    /// Its track's LOM target (by index).
    pub track: String,
    /// Its newest walk's number: an answer of another is void.
    pub walk: u64,
    /// The rack's first macro, while the hub holds its ref.
    pub found: Option<Found>,
    /// Why it has no rack to switch (a find's words).
    pub problem: Option<String>,
    /// The macro's value subscription (hub key).
    pub sub: Option<String>,
    /// The macro's value as last heard.
    pub value: Option<f64>,
}

/// The router's AUTOTUNE.
pub(super) struct AutotuneIo {
    pub rule: AutotuneAut,
    /// By key `<instance>|<track target>` ([`target_key`]).
    pub targets: BTreeMap<String, Target>,
    /// The keys in the layout's order (Live's).
    pub order: Vec<String>,
    /// The walks' own counter.
    pub walks: u64,
    /// The state the pages were last told.
    pub told: Option<AutotuneState>,
}

impl AutotuneIo {
    /// AUTOTUNE with the persisted flag; its state as every page gets it at
    /// attach is the one told (a layout without targets tells nothing new).
    pub fn new(aut: bool) -> Self {
        let mut io = Self {
            rule: AutotuneAut::new(aut),
            targets: BTreeMap::new(),
            order: Vec::new(),
            walks: 0,
            told: None,
        };
        io.told = Some(io.state());
        io
    }

    /// The state every page is told: AUT, the first target's Live set's
    /// transport and each target's state, in Live's order.
    pub fn state(&self) -> AutotuneState {
        let targets: Vec<&Target> = self
            .order
            .iter()
            .filter_map(|key| self.targets.get(key))
            .collect();
        AutotuneState {
            aut: self.rule.is_on(),
            playing: targets.first().and_then(|t| self.rule.playing(&t.instance)),
            targets: targets
                .iter()
                .map(|t| AutotuneTargetState {
                    label: t.spec.label.clone(),
                    state: state_of(t),
                })
                .collect(),
        }
    }

    /// AUTOTUNE in `/api/status`.
    pub fn status(&self) -> AutotuneStatus {
        AutotuneStatus {
            aut: self.rule.is_on(),
            writes: self.rule.writes(),
            taps: self.rule.taps(),
            targets: self
                .order
                .iter()
                .filter_map(|key| self.targets.get(key))
                .map(|t| AutotuneTargetStatus {
                    instance: t.instance.clone(),
                    track: t.track.clone(),
                    label: t.spec.label.clone(),
                    on: t.spec.on,
                    state: state_of(t),
                    path: t.found.as_ref().map(|f| f.path.clone()),
                    problem: t.problem.clone(),
                })
                .collect(),
        }
    }

    /// The keys of `instance`'s targets (all with `None`), the unfound ones
    /// only with `unfound`.
    fn keys_of(&self, instance: Option<&str>, unfound: bool) -> Vec<String> {
        self.targets
            .iter()
            .filter(|(_, t)| instance.is_none_or(|i| t.instance == i))
            .filter(|(_, t)| !unfound || t.found.is_none())
            .map(|(key, _)| key.clone())
            .collect()
    }
}

/// A target's key: its Live set and its track's LOM target.
pub(super) fn target_key(instance: &str, track: &str) -> String {
    format!("{instance}|{track}")
}

/// Where a target stands: no rack (`missing`), not found yet (`waiting`),
/// else its macro's value against its on value.
pub(super) fn state_of(target: &Target) -> TargetState {
    match (&target.problem, &target.found) {
        (Some(_), _) => TargetState::Missing,
        (None, None) => TargetState::Waiting,
        (None, Some(_)) => target_state(target.value, target.spec.on),
    }
}

/// The `set_prop` of each found macro's value through its `$ref` (the
/// script checks its class): one batch, applied in one Live tick.
pub(super) fn write_commands(writes: &[(&str, u8)]) -> Vec<Value> {
    writes
        .iter()
        .map(|(id, value)| {
            json!({
                "target": {"$ref": id, "class": PARAMETER_CLASS},
                "name": "set_prop",
                "args": {"prop": "value", "value": f64::from(*value)},
            })
        })
        .collect()
}

/// Each target whose slot did not answer `ok`, with why (the call's error
/// for all when the batch failed).
pub(super) fn slot_failures(
    outcome: &Result<Vec<Value>, String>,
    keys: &[String],
) -> Vec<(String, String)> {
    match outcome {
        Err(why) => keys.iter().map(|k| (k.clone(), why.clone())).collect(),
        Ok(slots) => keys
            .iter()
            .enumerate()
            .filter_map(|(i, key)| refused(slots.get(i)).map(|why| (key.clone(), why)))
            .collect(),
    }
}

/// The value a target's macro is written for `on`: its on value, else its
/// off value.
pub(super) fn macro_value(target: &AutotuneTarget, on: bool) -> u8 {
    if on { target.on } else { target.off }
}

/// Whether a missing target's problem is news (another than last recorded).
pub(super) fn missing_news(before: Option<&str>, now: &str) -> bool {
    before != Some(now)
}

/// The write a newly found target gets: its set's last write, only when it
/// was not found before (a target found again at another path keeps what
/// Live holds).
pub(super) fn found_write(was_found: bool, wanted: Option<bool>) -> Option<bool> {
    wanted.filter(|_| !was_found)
}

/// The `autotune` record of `step`, with the page that caused it.
pub(super) fn autotune_fields(step: &Step, client: Option<ClientId>, peer: Option<&str>) -> Value {
    let mut fields = serde_json::to_value(step).unwrap_or(Value::Null);
    if let Some(client) = client {
        fields["client"] = json!(client);
        fields["peer"] = json!(peer);
    }
    fields
}

/// Finds the `Latencies` rack of the track at `target` (at most
/// [`find::MAX_READS`] reads): its first macro, or why not in words.
async fn find_rack(live: LiveHandle, target: String) -> Result<Found, String> {
    let (mut find, mut commands) = Find::start(&target);
    for _ in 0..find::MAX_READS {
        let slots = live.call(commands).await.map_err(|e| e.to_string())?;
        match find.answer(&slots) {
            find::Step::Read(next) => commands = next,
            find::Step::Done(done) => return done.map_err(|m| m.to_string()),
            find::Step::Failed(why) => return Err(why),
        }
    }
    Err(TOO_DEEP.to_string())
}

impl Router {
    /// A new layout's targets: gone ones let go, new ones walked (kept ones
    /// keep what they found), the rule told the Live sets, its writes made.
    pub(super) fn autotune_layout(&mut self, targets: Vec<AutotuneTarget>) {
        let mut wanted: Vec<(String, Target)> = Vec::new();
        for spec in targets {
            let Ok(track) = spec.binding.target() else {
                tracing::warn!(instance = %spec.binding.instance, "AUTOTUNE cannot name a target's track");
                continue;
            };
            let key = target_key(&spec.binding.instance, &track);
            if wanted.iter().any(|(k, _)| *k == key) {
                continue;
            }
            let target = Target {
                instance: spec.binding.instance.clone(),
                spec,
                track,
                walk: 0,
                found: None,
                problem: None,
                sub: None,
                value: None,
            };
            wanted.push((key, target));
        }
        let gone: Vec<String> = self
            .autotune
            .targets
            .keys()
            .filter(|k| !wanted.iter().any(|(key, _)| key == *k))
            .cloned()
            .collect();
        for key in gone {
            self.autotune_unsub(&key);
            self.autotune.targets.remove(&key);
        }
        let mut new = Vec::new();
        self.autotune.order = wanted.iter().map(|(key, _)| key.clone()).collect();
        for (key, target) in wanted {
            if let Some(kept) = self.autotune.targets.get_mut(&key) {
                kept.spec = target.spec;
                continue;
            }
            self.autotune.targets.insert(key.clone(), target);
            new.push(key);
        }
        tracing::info!(
            targets = self.autotune.order.len(),
            new = new.len(),
            "AUTOTUNE follows the layout's tracks"
        );
        for key in new {
            self.autotune_walk(&key);
        }
        let sets: BTreeSet<String> = self
            .autotune
            .targets
            .values()
            .map(|t| t.instance.clone())
            .collect();
        let heard: BTreeMap<String, Option<bool>> = sets
            .iter()
            .map(|s| (s.clone(), self.transport_now(s)))
            .collect();
        let writes = self
            .autotune
            .rule
            .follow(&sets, |s| heard.get(s).copied().flatten());
        for (set, on) in writes {
            self.autotune_write(&set, on, Why::Transport);
        }
        self.autotune_tell();
    }

    /// Walks a target again (a new walk number: an answer of an older walk
    /// is void), while its Live set is online; its connect walks it else.
    pub(super) fn autotune_walk(&mut self, key: &str) {
        let Some(target) = self.autotune.targets.get_mut(key) else {
            return;
        };
        self.autotune.walks += 1;
        target.walk = self.autotune.walks;
        let online = self.states.get(&target.instance).is_some_and(|s| s.online);
        let (true, Some(live)) = (online, self.live.get(&target.instance)) else {
            return;
        };
        let (live, track, walk) = (live.clone(), target.track.clone(), target.walk);
        let key = key.to_string();
        let tx = self.io.tx.clone();
        tokio::spawn(async move {
            let outcome = find_rack(live, track).await;
            let _ = tx.send(RouterMsg::AutotuneFound { key, walk, outcome });
        });
    }

    /// A walk's answer: a stale one (a newer walk, a disconnect) does
    /// nothing; a found macro is followed (and gets its set's last write
    /// when newly found), a missing one says why.
    pub(super) fn autotune_found(&mut self, key: &str, walk: u64, outcome: Result<Found, String>) {
        if self.autotune.targets.get(key).map(|t| t.walk) != Some(walk) {
            return;
        }
        match outcome {
            Ok(found) => self.autotune_target_found(key, found),
            Err(why) => self.autotune_target_missing(key, why),
        }
        self.autotune_tell();
    }

    fn autotune_target_found(&mut self, key: &str, found: Found) {
        let Some(target) = self.autotune.targets.get_mut(key) else {
            return;
        };
        target.problem = None;
        let was = target.found.replace(found.clone());
        if was.as_ref().map(|f| f.path.as_str()) == Some(found.path.as_str()) {
            return;
        }
        let (instance, track) = (target.instance.clone(), target.track.clone());
        self.autotune_unsub(key);
        match self
            .subs
            .subscribe(AUTOTUNE_CLIENT, &instance, &found.path, "value", false)
        {
            Ok(reply) => {
                let value = match &reply.cached {
                    Some(Cached::Value { value, .. }) => value.as_f64(),
                    _ => None,
                };
                if let Some(target) = self.autotune.targets.get_mut(key) {
                    target.sub = Some(reply.key);
                    target.value = value;
                }
            }
            Err((_, error)) => {
                tracing::warn!(instance = %instance, path = %found.path, error = %error, "AUTOTUNE cannot follow a rack's macro");
            }
        }
        self.autotune_step(
            Step::Found {
                instance: instance.clone(),
                target: track,
                path: found.path.clone(),
            },
            None,
        );
        if let Some(on) = found_write(was.is_some(), self.autotune.rule.wanted(&instance)) {
            self.autotune_write_keys(&instance, &[key.to_string()], on, Why::Found);
        }
    }

    fn autotune_target_missing(&mut self, key: &str, why: String) {
        self.autotune_unsub(key);
        let Some(target) = self.autotune.targets.get_mut(key) else {
            return;
        };
        let news = missing_news(target.problem.as_deref(), &why);
        target.found = None;
        target.value = None;
        target.problem = Some(why.clone());
        let (instance, track) = (target.instance.clone(), target.track.clone());
        if news {
            self.autotune_step(
                Step::Missing {
                    instance,
                    target: track,
                    problem: why,
                },
                None,
            );
        }
    }

    /// Ends a target's value subscription (its value with it).
    fn autotune_unsub(&mut self, key: &str) {
        let Some(target) = self.autotune.targets.get_mut(key) else {
            return;
        };
        target.value = None;
        if let Some(sub) = target.sub.take() {
            self.subs.unsubscribe(AUTOTUNE_CLIENT, &sub);
        }
    }

    /// Writes `on` to every found target of `set`.
    fn autotune_write(&mut self, set: &str, on: bool, why: Why) {
        let keys: Vec<String> = self
            .autotune
            .order
            .iter()
            .filter(|key| {
                self.autotune
                    .targets
                    .get(*key)
                    .is_some_and(|t| t.instance == set)
            })
            .cloned()
            .collect();
        self.autotune_write_keys(set, &keys, on, why);
    }

    /// Writes `on` to the found ones of `keys` (targets of `set`), each its
    /// own on or off value, in one batch through their `$ref`s: recorded
    /// first; the answer comes back as [`RouterMsg::AutotuneWritten`].
    /// Nothing found: no record, no call.
    fn autotune_write_keys(&mut self, set: &str, keys: &[String], on: bool, why: Why) {
        let found: Vec<(String, String, String, u8)> = keys
            .iter()
            .filter_map(|key| {
                let t = self.autotune.targets.get(key)?;
                let f = t.found.as_ref()?;
                Some((
                    key.clone(),
                    f.id.clone(),
                    t.spec.label.clone(),
                    macro_value(&t.spec, on),
                ))
            })
            .collect();
        if found.is_empty() {
            return;
        }
        let Some(live) = self.live.get(set) else {
            return;
        };
        let labels: Vec<String> = found.iter().map(|(_, _, label, _)| label.clone()).collect();
        self.autotune_step(
            Step::Write {
                instance: set.to_string(),
                on,
                why,
                targets: labels,
            },
            None,
        );
        let pairs: Vec<(&str, u8)> = found
            .iter()
            .map(|(_, id, _, value)| (id.as_str(), *value))
            .collect();
        let result = live.call(write_commands(&pairs));
        let written: Vec<String> = found.iter().map(|(key, ..)| key.clone()).collect();
        let instance = set.to_string();
        let tx = self.io.tx.clone();
        tokio::spawn(async move {
            let outcome = result.await.map_err(|e| e.to_string());
            let _ = tx.send(RouterMsg::AutotuneWritten {
                instance,
                keys: written,
                outcome,
            });
        });
    }

    /// A write's answer: each target that failed is recorded and walked
    /// again (a stale ref, a deleted rack); no retry of the write.
    pub(super) fn autotune_written(
        &mut self,
        instance: &str,
        keys: &[String],
        outcome: &Result<Vec<Value>, String>,
    ) {
        for (key, problem) in slot_failures(outcome, keys) {
            let target = self
                .autotune
                .targets
                .get(&key)
                .map_or_else(|| key.clone(), |t| t.track.clone());
            self.autotune_step(
                Step::Failed {
                    instance: instance.to_string(),
                    target,
                    problem,
                },
                None,
            );
            self.autotune_walk(&key);
        }
    }

    /// `instance` connected: its targets are walked (the script's refs start
    /// over with each connection).
    pub(super) fn autotune_connected(&mut self, instance: &str) {
        for key in self.autotune.keys_of(Some(instance), false) {
            self.autotune_walk(&key);
        }
        self.autotune_tell();
    }

    /// `instance` went offline: first its walks in flight are voided, then
    /// every ref and value it found dropped and the rule told; its next
    /// session starts afresh.
    pub(super) fn autotune_offline(&mut self, instance: &str) {
        let keys = self.autotune.keys_of(Some(instance), false);
        for key in &keys {
            self.autotune.walks += 1;
            let walk = self.autotune.walks;
            if let Some(target) = self.autotune.targets.get_mut(key) {
                target.walk = walk;
                target.found = None;
                target.problem = None;
            }
            self.autotune_unsub(key);
        }
        self.autotune.rule.on_disconnect(instance);
        self.autotune_tell();
    }

    /// `instance`'s track list changed: its targets are walked again (an
    /// index may name another track now), keeping what they found until the
    /// answer.
    pub(super) fn autotune_tracks(&mut self, instance: &str) {
        for key in self.autotune.keys_of(Some(instance), false) {
            self.autotune_walk(&key);
        }
    }

    /// A transport change of `instance`: the rule's write, and the set's
    /// unfound targets walked again (found, they get the write).
    pub(super) fn autotune_transport(&mut self, instance: &str, playing: bool) {
        if let Some(on) = self.autotune.rule.on_playing(instance, playing) {
            self.autotune_write(instance, on, Why::Transport);
        }
        for key in self.autotune.keys_of(Some(instance), true) {
            self.autotune_walk(&key);
        }
        self.autotune_tell();
    }

    /// The rules' transport listener of `instance` failed: AUTOTUNE forgets
    /// its transport.
    pub(super) fn autotune_transport_lost(&mut self, instance: &str) {
        self.autotune.rule.transport_lost(instance);
        self.autotune_tell();
    }

    /// A delivery of a target's value subscription (its hub key `sub`): a
    /// value, or an error (the macro's path no longer resolves: its value
    /// dropped and the target walked again).
    pub(super) fn autotune_delivery(
        &mut self,
        sub: &str,
        value: Option<Value>,
        error: Option<String>,
    ) {
        let Some(key) = self
            .autotune
            .targets
            .iter()
            .find(|(_, t)| t.sub.as_deref() == Some(sub))
            .map(|(key, _)| key.clone())
        else {
            return;
        };
        match transport_heard(value, error) {
            Some(Ok(value)) => {
                if let Some(target) = self.autotune.targets.get_mut(&key) {
                    target.value = value.as_f64();
                }
            }
            Some(Err(error)) => {
                tracing::warn!(key = %key, error = ?error, "AUTOTUNE lost a rack's macro: it looks again");
                if let Some(target) = self.autotune.targets.get_mut(&key) {
                    target.value = None;
                }
                self.autotune_walk(&key);
            }
            None => {}
        }
        self.autotune_tell();
    }

    /// A page's `set_hub` of an AUTOTUNE key (`autotune_aut`, `autotune`):
    /// whether it was one.
    pub(super) fn autotune_set_hub(&mut self, client: ClientId, key: &str, value: &Value) -> bool {
        match (key, value.as_bool()) {
            (HUB_AUTOTUNE_AUT, Some(on)) => self.autotune_flag(client, on),
            (HUB_AUTOTUNE, Some(on)) => self.autotune_tap(client, on),
            _ => return false,
        }
        true
    }

    /// A page set AUTOTUNE AUT: recorded; saved only on a change; turning it
    /// on writes each set's transport at once.
    fn autotune_flag(&mut self, client: ClientId, on: bool) {
        let changed = on != self.autotune.rule.is_on();
        self.autotune_step(Step::Flag { on }, Some(client));
        let writes = self.autotune.rule.set_flag(on);
        if changed {
            self.state.autotune_aut = on;
            self.save_state();
        }
        self.autotune_tell();
        for (set, on) in writes {
            self.autotune_write(&set, on, Why::Flag);
        }
        for key in self.autotune.keys_of(None, true) {
            self.autotune_walk(&key);
        }
    }

    /// A page tapped AUTOTUNE: every set's targets on or off.
    fn autotune_tap(&mut self, client: ClientId, on: bool) {
        self.autotune_step(Step::Tap { on }, Some(client));
        for (set, on) in self.autotune.rule.tap(on) {
            self.autotune_write(&set, on, Why::Tap);
        }
        for key in self.autotune.keys_of(None, true) {
            self.autotune_walk(&key);
        }
        self.autotune_tell();
    }

    /// One step: its log line and its `autotune` record.
    fn autotune_step(&self, step: Step, client: Option<ClientId>) {
        let line = step_line(&step);
        if step_warn(&step) {
            tracing::warn!("{line}");
        } else {
            tracing::info!("{line}");
        }
        let peer = client.and_then(|c| self.peers.get(&c)).map(String::as_str);
        self.io
            .events
            .record("autotune", autotune_fields(&step, client, peer));
    }

    /// Every page is told the state, when it changed.
    pub(super) fn autotune_tell(&mut self) {
        let state = self.autotune.state();
        if self.autotune.told.as_ref() == Some(&state) {
            return;
        }
        for outbox in self.clients.values() {
            outbox.reply(ServerMsg::Autotune(state.clone()));
        }
        self.autotune.told = Some(state);
    }

    /// A new page is told AUTOTUNE's state.
    pub(super) fn autotune_attach(&self, outbox: &Outbox) {
        outbox.reply(ServerMsg::Autotune(self.autotune.state()));
    }
}

#[cfg(test)]
#[path = "autotune/tests.rs"]
mod tests;
