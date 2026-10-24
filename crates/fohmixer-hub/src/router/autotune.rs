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
//!
//! **The keys (#140 PR 2a, `autotune/keys.rs`):** each found rack's key
//! device (the find's chain 0) has its Key and Scale values followed, and
//! each Live set with targets its playhead (`current_song_time`, `tempo`):
//! the key in force along the set's newest song map, one `key` record per
//! change. Until #140 PR 3 a page's AUTOTUNE AUT on and tap are refused (a
//! `refused` record, the state told again to that page) and a saved AUT on
//! starts off, so nothing writes the macros.
//!
//! **The key writes (#140 PR 2b, `autotune/key_writes.rs`):** the key in
//! force is written to every found key device (through its parameters'
//! `$ref`s) and Live's bar at every song start and key change, to a target
//! found anew, and after a hold; each write is read back, a mismatch
//! retried once, another writer shown and never fought. No selector moves.
//!
//! **The fall back to DRY (#140 PR 3a, `autotune/safe.rs`):** every
//! found selector's DRY write (its macro's `$ref` and off value, whatever
//! AUT is) is registered with the set's script, which makes them when the
//! hub is gone; a registration goes when the found set changed, never
//! while the set's writes are held.
//!
//! **Nothing is written by a stale index (PR #139 review I1, REVERB's
//! window):** from a disconnect or a track-list change, and whenever a
//! write meets the set's markers unsettled (`reverb_settled`: the marker
//! keeper's read runs or a composition waits), the set's writes are held
//! (one `held` record), the rule's decisions kept; once the markers
//! settled (the keeper's read, the quiet time, a layout served) the hold
//! ends (one record) and its targets are walked again, and each answer
//! writes the set's decision once to a macro not yet written with it
//! (`found_write`: another `$ref`, or none written since the hold or the
//! disconnect).

use std::collections::{BTreeMap, BTreeSet};

use fohmixer_proto::client::{
    AutotuneState, AutotuneStatus, AutotuneTargetState, AutotuneTargetStatus, HUB_AUTOTUNE,
    HUB_AUTOTUNE_AUT, KeyInForce, ServerMsg, TargetState,
};
use fohmixer_proto::layout::AutotuneTarget;
use serde_json::{Value, json};

use super::retries::{CONNECT_RETRIES, Due, RETRY_MS, transient};
use super::{Router, RouterMsg, transport_heard};
use crate::eq::walk::refused;
use crate::live::client::LiveHandle;
use crate::live::subs::{Cached, ClientId};
use crate::outbox::Outbox;
use crate::rules::autotune::find::{self, Find, Found, KeyFound};
use crate::rules::autotune::keys::KeyTarget;
use crate::rules::autotune::{AutotuneAut, Step, Why, step_line, step_warn, target_state};

#[path = "autotune/keys.rs"]
mod keys;
use keys::{Position, key_status};
#[path = "autotune/key_writes.rs"]
mod key_writes;
#[path = "autotune/safe.rs"]
mod safe;
use safe::Safe;

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
    /// The `$ref` the set's decision was last written to; none since a
    /// disconnect or a held write (the decision is owed).
    pub wrote: Option<String>,
    /// Its key device (#140 PR 2): none until its rack is found, else the
    /// device or why there is none (the find's words).
    pub key: Option<Result<KeyFound, String>>,
    /// The key device's Key and Scale value subscriptions (hub keys).
    pub key_sub: Option<String>,
    pub scale_sub: Option<String>,
    /// Their values as last heard: Key, Scale.
    pub values: (Option<f64>, Option<f64>),
    /// Its key writes and their read-backs (#140 PR 2b).
    pub keyt: KeyTarget,
    /// The newest key write that holds it: an answer of another is void (0:
    /// none since a disconnect or another device).
    pub key_write: u64,
    /// The Key and Scale `$ref`s the key in force was last written to; none
    /// since a disconnect, a held write or another device (it is owed).
    pub key_wrote: Option<(String, String)>,
    /// Whether its newest walk confirmed its key device: false from a
    /// track-list change (and a disconnect) until then, when a value
    /// delivery may be another track's (PR 2a review M3).
    pub key_sure: bool,
    /// Its walks that failed in a row since its last trigger or answer
    /// (#140: a set still loading answers no read for a while).
    pub walk_failures: u32,
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
    /// The Live sets whose writes are held until their markers settle.
    pub held: BTreeSet<String>,
    /// Each Live set with targets: its playhead and key in force (#140 PR
    /// 2).
    pub positions: BTreeMap<String, Position>,
    /// The Live sets whose newest song map was read since their connect:
    /// only such a map gives a key in force (PR 2a review M1: the map from
    /// before a disconnect may be another set's).
    pub current_maps: BTreeSet<String>,
    /// The key writes' own counter (#140 PR 2b).
    pub key_writes: u64,
    /// Each Live set's DRY writes registered with its script since its
    /// connect (#140 PR 3a); none: the script holds none.
    pub safe: BTreeMap<String, Safe>,
    /// The registrations' own counter.
    pub safe_seq: u64,
}

impl AutotuneIo {
    /// AUTOTUNE with AUT off, whatever was persisted (`saved`): until #140
    /// PR 3 nothing switches, so a saved AUT on is logged and starts off
    /// (the router saves it off). Its state as every page gets it at attach
    /// is the one told (a layout without targets tells nothing new).
    pub fn new(saved: bool) -> Self {
        if saved {
            tracing::warn!("AUTOTUNE AUT stays off until #140 PR 3");
        }
        let mut io = Self {
            rule: AutotuneAut::new(false),
            targets: BTreeMap::new(),
            order: Vec::new(),
            walks: 0,
            told: None,
            held: BTreeSet::new(),
            positions: BTreeMap::new(),
            current_maps: BTreeSet::new(),
            key_writes: 0,
            safe: BTreeMap::new(),
            safe_seq: 0,
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
                    key: t.key.as_ref().map(|k| key_status(k, t.values, &t.keyt)),
                })
                .collect(),
            keys: self
                .positions
                .iter()
                .map(|(instance, p)| {
                    let (song, key) = p.follow.now();
                    KeyInForce {
                        instance: instance.clone(),
                        song,
                        key: key.map(|key| key.name()),
                        time: p.time,
                        playing: p.playing,
                        tempo: p.tempo,
                        bar: p.bar.clone(),
                    }
                })
                .collect(),
            safe: self.safe_status(),
        }
    }

    /// The keys of `set`'s targets in the layout's order (Live's).
    fn keys_in_order(&self, set: &str) -> Vec<String> {
        self.order
            .iter()
            .filter(|key| self.targets.get(*key).is_some_and(|t| t.instance == set))
            .cloned()
            .collect()
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

/// Whether a walk found the macro it found before (`was`): the same path
/// and `$ref` (its key device may differ; it is followed on its own).
pub(super) fn same_macro(was: Option<&Found>, now: &Found) -> bool {
    was.is_some_and(|was| was.path == now.path && was.id == now.id)
}

/// The write a found target gets: its set's decision, only when the macro
/// found (`id`) is not the one it was last written to (`wrote`: none since
/// a disconnect or a held write). The same macro found again keeps what
/// Live holds.
pub(super) fn found_write(wrote: Option<&str>, id: &str, wanted: Option<bool>) -> Option<bool> {
    wanted.filter(|_| wrote != Some(id))
}

/// Whether a Live set's writes may go now: its hold is not open and its
/// markers are settled (`reverb_settled`; PR #139 review I1).
pub(super) fn may_write(holding: bool, settled: bool) -> bool {
    !holding && settled
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

/// Whether a target whose walks failed `failures` times in a row (this
/// one included) is walked again after [`RETRY_MS`] (#140): up to
/// [`CONNECT_RETRIES`] retries, the song map's bound; then it waits for the
/// next trigger.
pub(super) fn walk_again(failures: u32) -> bool {
    failures <= CONNECT_RETRIES
}

/// What follows a failed walk (#140).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AfterWalk {
    /// Walked again after [`RETRY_MS`].
    Retry,
    /// Its retries spent: one `gave_up` record, then the next trigger.
    GiveUp,
    /// A definite refusal (PR #145 review M2): shown as missing, it waits
    /// for the next trigger.
    Wait,
}

/// What follows a target's failed walk: a retry while Live did not answer
/// or was offline (`retryable`, [`transient`]) with retries left
/// ([`walk_again`]), a give-up once they are spent, and nothing more for a
/// definite refusal.
pub(super) fn after_walk(retryable: bool, failures: u32) -> AfterWalk {
    match (retryable, walk_again(failures)) {
        (false, _) => AfterWalk::Wait,
        (true, true) => AfterWalk::Retry,
        (true, false) => AfterWalk::GiveUp,
    }
}

/// Whether a target keeps what it found through a failed walk (PR #146
/// review I3): a found one whose walk Live did not answer (`retryable`:
/// busy, a timeout, offline) keeps its ref and its DRY registration, since
/// the script holds the object; a definite refusal drops them.
pub(super) fn keeps_found(retryable: bool, found: bool) -> bool {
    retryable && found
}

/// A target's problem once its walks gave up (PR #145 review M7): Live's
/// words, and that it gave up after `tries` failed walks, so `/api/status`
/// tells a stopped retry from a running one.
pub(super) fn gave_up_problem(why: &str, tries: u32) -> String {
    format!("{why} (gave up after {tries} tries, until the next trigger)")
}

/// How a walk ended: its definite answer (the macro, or why there is none
/// in words), or a read that failed (Live offline, a timeout, a refused
/// read: #140; only the first two are worth another walk, [`after_walk`]).
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Walked {
    Answered(Result<Found, String>),
    Failed(String),
}

/// Finds the `Latencies` rack of the track at `target` (at most
/// [`find::MAX_READS`] reads): its first macro, or why not in words, or the
/// read that failed.
async fn find_rack(live: LiveHandle, target: String) -> Walked {
    let (mut find, mut commands) = Find::start(&target);
    for _ in 0..find::MAX_READS {
        let slots = match live.call(commands).await {
            Ok(slots) => slots,
            Err(e) => return Walked::Failed(e.to_string()),
        };
        match find.answer(&slots) {
            find::Step::Read(next) => commands = next,
            find::Step::Done(done) => return Walked::Answered(done.map_err(|m| m.to_string())),
            find::Step::Failed(why) => return Walked::Failed(why),
        }
    }
    Walked::Answered(Err(TOO_DEEP.to_string()))
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
                wrote: None,
                key: None,
                key_sub: None,
                scale_sub: None,
                values: (None, None),
                keyt: KeyTarget::new(),
                key_write: 0,
                key_wrote: None,
                key_sure: false,
                walk_failures: 0,
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
            self.autotune_key_unsub(&key);
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
        self.autotune_positions(&sets);
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
        // A layout served ends the holds of the sets whose markers are
        // settled (as REVERB's window).
        let held: Vec<String> = self.autotune.held.iter().cloned().collect();
        for set in held {
            self.autotune_release(&set, "layout");
        }
        // The DRY writes follow the found set (#140 PR 3a): a target gone
        // leaves its set's list, a set without targets left disarms.
        let names: Vec<String> = self.live.keys().cloned().collect();
        for name in names {
            self.autotune_safe(&name);
        }
        self.autotune_tell();
    }

    /// Walks a target again for a trigger (a layout, a connect, a transport
    /// change, a track-list change, a failed write or value): its failed
    /// walks' count starts over (#140).
    pub(super) fn autotune_walk(&mut self, key: &str) {
        if let Some(target) = self.autotune.targets.get_mut(key) {
            target.walk_failures = 0;
        }
        self.autotune_walk_now(key);
    }

    /// Walks a target (a new walk number: an answer of an older walk is
    /// void), while its Live set is online; its connect walks it else.
    fn autotune_walk_now(&mut self, key: &str) {
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
            let msg = match find_rack(live, track).await {
                Walked::Answered(outcome) => RouterMsg::AutotuneFound { key, walk, outcome },
                Walked::Failed(why) => RouterMsg::AutotuneWalkFailed { key, walk, why },
            };
            let _ = tx.send(msg);
        });
    }

    /// A walk's read failed (#140): void unless it is the target's newest
    /// walk; else the target is missing with Live's words (recorded when
    /// news), unless it was found and Live only did not answer (PR #146
    /// review I3, [`keeps_found`]: its ref and its DRY write stay), and,
    /// when Live did not answer or was offline, walked again after
    /// [`RETRY_MS`] while retries are left, or one `gave_up` record (warn)
    /// waits for the next trigger ([`after_walk`]); a definite refusal waits
    /// for the next trigger.
    pub(super) fn autotune_walk_failed(&mut self, key: &str, walk: u64, why: String) {
        let Some(target) = self.autotune.targets.get(key).filter(|t| t.walk == walk) else {
            return;
        };
        let retryable = transient(&why);
        if !keeps_found(retryable, target.found.is_some()) {
            self.autotune_target_missing(key, why.clone());
        }
        let Some(target) = self.autotune.targets.get_mut(key) else {
            return;
        };
        target.walk_failures += 1;
        let failures = target.walk_failures;
        let (instance, track) = (target.instance.clone(), target.track.clone());
        match after_walk(retryable, failures) {
            AfterWalk::Retry => self.autotune_walk_retry_later(key, walk),
            AfterWalk::GiveUp => {
                target.problem = Some(gave_up_problem(&why, failures));
                self.autotune_step(
                    Step::GaveUp {
                        instance,
                        target: track,
                        problem: why,
                        tries: failures,
                    },
                    None,
                );
            }
            AfterWalk::Wait => {}
        }
        self.autotune_tell();
    }

    /// Asks for a retry of the target's failed walk `walk` after
    /// [`RETRY_MS`] ([`RouterMsg::AutotuneWalkRetry`]).
    fn autotune_walk_retry_later(&self, key: &str, walk: u64) {
        let (key, tx) = (key.to_string(), self.io.tx.clone());
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(RETRY_MS)).await;
            let _ = tx.send(RouterMsg::AutotuneWalkRetry { key, walk });
        });
    }

    /// A failed walk's retry is due: the target is walked again, unless it
    /// was walked since (a trigger, a disconnect: the retry is void); while
    /// its Live is busy it waits another [`RETRY_MS`], uncounted (PR #145
    /// review M1).
    pub(super) fn autotune_walk_retry(&mut self, key: &str, walk: u64) {
        let Some(target) = self.autotune.targets.get(key).filter(|t| t.walk == walk) else {
            return;
        };
        if self.retry_due(&target.instance) == Due::Wait {
            self.autotune_walk_retry_later(key, walk);
            return;
        }
        self.autotune_walk_now(key);
    }

    /// A walk's answer: a stale one (a newer walk, a disconnect) does
    /// nothing; a found macro is followed (and gets its set's last write
    /// when newly found), a missing one says why.
    pub(super) fn autotune_found(&mut self, key: &str, walk: u64, outcome: Result<Found, String>) {
        let Some(target) = self.autotune.targets.get_mut(key) else {
            return;
        };
        if target.walk != walk {
            return;
        }
        // A definite answer: its failed walks are over (#140).
        target.walk_failures = 0;
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
        let wrote = target.wrote.clone();
        let instance = target.instance.clone();
        // Another path, or the same path with a new `$ref` (a rack replaced
        // in place; PR #139 review M1): its value is followed afresh, which
        // also gives up a listener the script ended.
        if !same_macro(was.as_ref(), &found) {
            self.autotune_follow(key, &found);
        }
        self.autotune_key_found(key);
        if let Some(on) = found_write(
            wrote.as_deref(),
            &found.id,
            self.autotune.rule.wanted(&instance),
        ) {
            self.autotune_write_keys(&instance, &[key.to_string()], on, Why::Found);
        }
        self.autotune_safe(&instance);
    }

    /// Follows a found macro's value afresh (a new subscription at the found
    /// path), recorded `found`.
    fn autotune_follow(&mut self, key: &str, found: &Found) {
        let Some(target) = self.autotune.targets.get(key) else {
            return;
        };
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
                instance,
                target: track,
                path: found.path.clone(),
            },
            None,
        );
    }

    fn autotune_target_missing(&mut self, key: &str, why: String) {
        self.autotune_unsub(key);
        self.autotune_key_unsub(key);
        self.autotune_key_forget(key);
        let Some(target) = self.autotune.targets.get_mut(key) else {
            return;
        };
        let news = missing_news(target.problem.as_deref(), &why);
        target.found = None;
        target.key = None;
        target.value = None;
        target.problem = Some(why.clone());
        let (instance, track) = (target.instance.clone(), target.track.clone());
        if news {
            self.autotune_step(
                Step::Missing {
                    instance: instance.clone(),
                    target: track,
                    problem: why,
                },
                None,
            );
        }
        self.autotune_safe(&instance);
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
        let keys = self.autotune.keys_in_order(set);
        self.autotune_write_keys(set, &keys, on, why);
    }

    /// Writes `on` to the found ones of `keys` (targets of `set`), each its
    /// own on or off value, in one batch through their `$ref`s: recorded
    /// first; the answer comes back as [`RouterMsg::AutotuneWritten`].
    /// Nothing found: no record, no call.
    fn autotune_write_keys(&mut self, set: &str, keys: &[String], on: bool, why: Why) {
        let holding = self.autotune.held.contains(set);
        if !may_write(holding, self.reverb_settled(set)) {
            // Held: the decision is owed to every one of them.
            for key in keys {
                if let Some(target) = self.autotune.targets.get_mut(key) {
                    target.wrote = None;
                }
            }
            self.autotune_hold(set, "markers");
            return;
        }
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
        for (key, id, ..) in &found {
            if let Some(target) = self.autotune.targets.get_mut(key) {
                target.wrote = Some(id.clone());
            }
        }
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
                target.wrote = None;
                target.key = None;
            }
            self.autotune_unsub(key);
            self.autotune_key_unsub(key);
            self.autotune_key_forget(key);
        }
        // The script's DRY writes died with its connection (#140 PR 3a):
        // an answer still on its way is void, and the connect's walks
        // register afresh.
        self.autotune.safe.remove(instance);
        self.autotune_position_offline(instance);
        self.autotune_hold(instance, "offline");
        self.autotune.rule.on_disconnect(instance);
        self.autotune_tell();
    }

    /// `instance`'s track list changed: its targets are walked again (an
    /// index may name another track now), keeping what they found until the
    /// answer.
    pub(super) fn autotune_tracks(&mut self, instance: &str) {
        self.autotune_hold(instance, "tracks");
        for key in self.autotune.keys_of(Some(instance), false) {
            // Its key device's values may be another track's until its walk
            // answers (PR 2a review M3).
            if let Some(target) = self.autotune.targets.get_mut(&key) {
                target.key_sure = false;
            }
            self.autotune_walk(&key);
        }
    }

    /// Holds `instance`'s writes (`why`: the cause), recorded once; only
    /// for a Live set with AUTOTUNE targets.
    fn autotune_hold(&mut self, instance: &str, why: &str) {
        if self.autotune.keys_of(Some(instance), false).is_empty()
            || !self.autotune.held.insert(instance.to_string())
        {
            return;
        }
        self.autotune_step(
            Step::Held {
                instance: instance.to_string(),
                on: true,
                why: why.to_string(),
            },
            None,
        );
    }

    /// Ends `instance`'s hold once its markers are settled (`why`: how),
    /// recorded; its DRY writes are registered as the hold kept its found
    /// targets (#140 PR 3a), its targets are walked again, and each answer
    /// writes the set's decision once where it is owed. Settled with no
    /// hold, a registration a walk's answer left to the markers goes now.
    pub(super) fn autotune_release(&mut self, instance: &str, why: &str) {
        if !self.reverb_settled(instance) {
            return;
        }
        if !self.autotune.held.remove(instance) {
            self.autotune_safe(instance);
            return;
        }
        self.autotune_step(
            Step::Held {
                instance: instance.to_string(),
                on: false,
                why: why.to_string(),
            },
            None,
        );
        self.autotune_safe(instance);
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
        self.autotune_position_playing(instance, Some(playing));
        self.autotune_tell();
    }

    /// The rules' transport listener of `instance` failed: AUTOTUNE forgets
    /// its transport.
    pub(super) fn autotune_transport_lost(&mut self, instance: &str) {
        self.autotune.rule.transport_lost(instance);
        self.autotune_position_playing(instance, None);
        self.autotune_tell();
    }

    /// A delivery of one of AUTOTUNE's subscriptions (its hub key `sub`): a
    /// target's macro, its key device's Key or Scale, or a set's playhead or
    /// tempo (#140 PR 2), each to its own; anything else does nothing.
    pub(super) fn autotune_delivery(
        &mut self,
        sub: &str,
        value: Option<Value>,
        error: Option<String>,
    ) {
        match self.autotune.route(sub) {
            Some(keys::Route::Macro(key)) => self.autotune_macro_value(&key, value, error),
            Some(keys::Route::Key(key, part)) => self.autotune_key_value(&key, part, value, error),
            Some(keys::Route::Position(instance, part)) => {
                self.autotune_position_value(&instance, part, value, error);
            }
            None => return,
        }
        self.autotune_tell();
    }

    /// A delivery of a target's macro value: a value, or an error (the
    /// macro's path no longer resolves: its value dropped and the target
    /// walked again).
    fn autotune_macro_value(&mut self, key: &str, value: Option<Value>, error: Option<String>) {
        match transport_heard(value, error) {
            Some(Ok(value)) => {
                if let Some(target) = self.autotune.targets.get_mut(key) {
                    target.value = value.as_f64();
                }
            }
            Some(Err(error)) => {
                tracing::warn!(key = %key, error = ?error, "AUTOTUNE lost a rack's macro: it looks again");
                if let Some(target) = self.autotune.targets.get_mut(key) {
                    target.value = None;
                }
                self.autotune_walk(key);
            }
            None => {}
        }
    }

    /// A page's `set_hub` of an AUTOTUNE key (`autotune_aut`, `autotune`):
    /// whether it was one. Until #140 PR 3 only AUT off is taken: AUT on
    /// and any tap are refused, and nothing is written.
    pub(super) fn autotune_set_hub(&mut self, client: ClientId, key: &str, value: &Value) -> bool {
        match (key, value.as_bool()) {
            (HUB_AUTOTUNE_AUT, Some(false)) => self.autotune_flag(client, false),
            (HUB_AUTOTUNE_AUT, Some(true)) => self.autotune_refused(client, "aut"),
            (HUB_AUTOTUNE, _) => self.autotune_refused(client, "tap"),
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
