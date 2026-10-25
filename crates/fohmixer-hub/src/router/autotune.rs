//! The router's half of AUTOTUNE (#104, D19): it finds each `+AUTOTUNE`
//! target's `Latencies` rack (`rules::autotune::find`, a bounded walk per
//! target, a newer walk voiding an older answer), follows its first macro's
//! value (its own subscriber, [`AUTOTUNE_CLIENT`]), hands the rule
//! (`rules::autotune`) the layout's Live sets, their transport and the
//! pages' flag, and decides every target (`autotune/switch.rs`, #140 PR
//! 3b): each Live set's selectors whose decision differs from what Live
//! shows are written through their `$ref` in one `live.call` batch (never
//! by an index path: a device inserted above would move it), each step an
//! `autotune` record and a log line, AUTOTUNE AUT and the singers off by
//! hand in `hub-state.json` (saved whole), and the state to every page
//! when it changed.
//!
//! The refs die with the script's connection: a disconnect drops every
//! found ref and value of that Live set and voids its walks in flight
//! before anything else; its connect walks them again. A write that failed
//! walks its target again (a stale ref, a deleted rack); a track-list
//! change walks the set's targets again (an index then names another
//! track), keeping what they found until the answer. A walk's answer read
//! by an index the hub's track list no longer names (its list's push still
//! on its way, PR #146 review M2) is no answer: it is walked again
//! ([`MOVED_TRACK`]).
//!
//! **The keys (#140 PR 2a, `autotune/keys.rs`):** each found rack's key
//! device (the find's chain 0) has its Key and Scale values followed, and
//! each Live set with targets its playhead (`current_song_time`, `tempo`):
//! the key in force along the set's newest song map, one `key` record per
//! change. A page's AUTOTUNE tap is refused (a `refused` record, the state
//! told again to that page).
//!
//! **The key writes (#140 PR 2b, `autotune/key_writes.rs`):** the key in
//! force is written to every found key device (through its parameters'
//! `$ref`s) and Live's bar at every song start and key change, to a target
//! found anew, and after a hold; each write is read back, a mismatch
//! retried once, another writer shown and never fought.
//!
//! **The fall back to DRY (#140 PR 3a, `autotune/safe.rs`):** every
//! found selector's DRY write (its macro's `$ref` and off value, whatever
//! AUT is) is registered with the set's script, which makes them when the
//! hub is gone; a registration goes when the found set changed, never
//! while the set's writes are held. A selector is on only while both the
//! list the script last kept and the newest one sent have it (PR #147
//! review I2). A target let go while its selector may be on (a layout
//! without it, a definite missing walk) is written DRY through its last
//! `$ref` first, and its set registered only once that write answered
//! (PR #147 review I3).
//!
//! **Nothing is written by a stale index (PR #139 review I1, REVERB's
//! window):** from a disconnect or a track-list change, and whenever a
//! write meets the set's markers unsettled (`reverb_settled`: the marker
//! keeper's read runs or a composition waits), the set's writes are held
//! (one `held` record): a hold first writes DRY through the last known
//! refs to every selector the rule had on (spec §7), then nothing goes;
//! once the markers settled (the keeper's read, the quiet time, a layout
//! served) the hold ends (one record), its targets are walked again and
//! the set is decided again; a target the hold left unconfirmed is DRY and
//! written nothing until its walk answers (PR #147 review I4), and a kept
//! target whose on and off values changed (a PA track where a stream track
//! was) is walked afresh before anything is written or registered for it.
//!
//! **AUTOTUNE AUT (#140 PR 3c lifted 3b's refusal):** a page's AUT on or
//! off is taken (the new AUTOTUNE button's 1 s hold), recorded (`flag`) and
//! saved as `hub-state.json` `autotune_aut`; a saved on is honoured at the
//! hub's start. Only the tap (`autotune`) is refused: a tap opens the
//! singers' panel and sends nothing.

use std::collections::{BTreeMap, BTreeSet};

use fohmixer_proto::client::{
    AutotuneState, AutotuneStatus, AutotuneTargetState, AutotuneTargetStatus, HUB_AUTOTUNE,
    HUB_AUTOTUNE_AUT, HUB_AUTOTUNE_SINGER, KeyInForce, ServerMsg, TargetState, hub_key,
};
use fohmixer_proto::layout::{Anchor, AutotuneTarget};
use fohmixer_proto::songmap::Counts;
use serde_json::{Value, json};

use super::markers::{RETURNS, TRACKS};
use super::retries::{CONNECT_RETRIES, Due, RETRY_MS, transient};
use super::{Router, RouterMsg, transport_heard};
use crate::eq::close::{STALE_REF, UNKNOWN_REF};
use crate::eq::walk::{answered, get, refused};
use crate::live::client::LiveHandle;
use crate::live::subs::{Cached, ClientId};
use crate::outbox::Outbox;
use crate::rules::autotune::find::{self, Find, Found, KeyFound};
use crate::rules::autotune::keys::KeyTarget;
use crate::rules::autotune::switch::{
    Decision, Flying, Refusal, SelectorGuard, answered_flying, needs_write, still_flying,
};
use crate::rules::autotune::{
    AutotuneAut, Cause, REFUSED_WHY, Step, shows, step_line, step_warn, target_state,
};

#[path = "autotune/keys.rs"]
mod keys;
use keys::{Position, key_status};
#[path = "autotune/key_writes.rs"]
mod key_writes;
#[path = "autotune/safe.rs"]
mod safe;
use safe::Safe;
#[path = "autotune/switch.rs"]
mod switch;

/// AUTOTUNE's subscriber of its targets' values (clients count up from 1;
/// the unfold and marker keepers and REVERB are `MAX`, `MAX − 1`,
/// `MAX − 2`).
pub const AUTOTUNE_CLIENT: ClientId = ClientId::MAX - 3;
/// A walk that read [`find::MAX_READS`] times and did not end.
pub const TOO_DEEP: &str = "the devices nest too deep";
/// The `$ref` class of a rack's macro (the script refuses a ref to anything
/// else).
pub const PARAMETER_CLASS: &str = "DeviceParameter";
/// Why a walk's answer is void (PR #146 review M2): the track at its index
/// is another than the hub's track list says, so the list's push is on its
/// way (its change walks the target again); walked again like a read Live
/// did not answer.
pub const MOVED_TRACK: &str = "the track at its index is another than the hub's track list";

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
    /// Its last decision (#140 PR 3b): none until its rack is found, and
    /// again from a disconnect or another macro found.
    pub decision: Option<Decision>,
    /// The guard against another writer on its selector (spec §7).
    pub guard: SelectorGuard,
    /// The hub's writes of its selector on their way, in order, until Live
    /// is heard after them or the answer says it failed (PR #147 review I1:
    /// two may be).
    pub writing: Vec<Flying>,
    /// Whether its rack was confirmed by a walk since its set's last hold
    /// or track-list change (PR #147 review I4, re-review R1): false from
    /// each until its walk's answer (its ref and values may be of the track
    /// the index named before), for a kept target whose on and off values
    /// changed until its walk, and after a selector write whose ref is gone
    /// until the walk that follows answers. Unconfirmed it is DRY
    /// (`unconfirmed`), written nothing, and its set's registration waits.
    pub sure: bool,
    /// Live's refusal of its selector's write (PR #147 review I5): one
    /// retry, then no more of that value to that `$ref` until a change.
    pub refusal: Option<Refusal>,
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

impl Target {
    /// Where its selector will be once the hub's writes on their way land:
    /// the last one's value.
    pub fn writing_now(&self) -> Option<bool> {
        self.writing.last().map(|f| f.on)
    }

    /// Where its selector shows: near its on value, its off value, or
    /// neither ([`shows`]).
    pub fn shows(&self) -> Option<bool> {
        shows(self.value, self.spec.on, self.spec.off)
    }

    /// Whether, let go now, it may leave its selector off DRY (PR #147
    /// review I3): Live heard off its off value, a write of its chain on
    /// its way, or its decision on; and no DRY write on its way already.
    pub fn may_be_on(&self) -> bool {
        let heard_or_on = self.value.is_some()
            || self.writing_now() == Some(true)
            || self.decision.is_some_and(|d| d.on);
        heard_or_on && needs_write(false, self.shows(), self.writing_now(), false)
    }

    /// Whether its set's registration waits for its walk: unconfirmed,
    /// found or not, with no give-up (PR #147 re-review R1: the script then
    /// keeps the list it last accepted, whose pairs stay true, until every
    /// newest walk answered).
    pub fn awaits_walk(&self) -> bool {
        !self.sure && self.problem.is_none()
    }
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
    /// Each Live set's song map counts, as last read (the pages' state).
    pub counts: BTreeMap<String, Counts>,
    /// The selector writes' own counter (PR #147 review I1).
    pub selects: u64,
    /// Each Live set's DRY writes of targets let go, on their way: its
    /// registration waits for their answers (PR #147 review I3).
    pub retiring: BTreeMap<String, BTreeSet<u64>>,
}

impl AutotuneIo {
    /// AUTOTUNE with AUT and the singers off by hand as persisted (#140 PR
    /// 3b: kept across a restart; a saved AUT on honoured since PR 3c). Its
    /// state as every page gets it at attach is the one told (a layout
    /// without targets tells nothing new).
    pub fn new(saved: bool, off: Vec<String>) -> Self {
        let mut io = Self {
            rule: AutotuneAut::new(saved, off),
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
            counts: BTreeMap::new(),
            selects: 0,
            retiring: BTreeMap::new(),
        };
        io.told = Some(io.state());
        io
    }

    /// The targets in the layout's order (Live's).
    fn ordered(&self) -> Vec<&Target> {
        self.order
            .iter()
            .filter_map(|key| self.targets.get(key))
            .collect()
    }

    /// The state every page is told: AUT, the first target's Live set's
    /// transport and each target's state, in Live's order; since #140 PR
    /// 3b that set's song and key in force, whether a key is being
    /// checked, the singers, its song map's counts and Live's bar.
    pub fn state(&self) -> AutotuneState {
        let targets = self.ordered();
        let first = targets.first().map(|t| t.instance.as_str());
        let position = first.and_then(|set| self.positions.get(set));
        let (song, key) = position.map_or((None, None), |p| p.follow.now());
        let counts = first
            .and_then(|set| self.counts.get(set))
            .copied()
            .unwrap_or_default();
        let playing = first.and_then(|set| self.rule.playing(set));
        AutotuneState {
            aut: self.rule.is_on(),
            playing,
            targets: targets
                .iter()
                .map(|t| AutotuneTargetState {
                    label: t.spec.label.clone(),
                    state: state_of(t),
                })
                .collect(),
            song,
            key: key.map(|key| key.name()),
            checking: switch::checking(
                self.rule.is_on(),
                playing,
                key,
                targets
                    .iter()
                    .filter(|t| switch::checked(t))
                    .map(|t| self.check_of(t)),
            ),
            singers: self.singers(),
            map_problems: counts.problems,
            map_changes: counts.changes,
            map_keyed: counts.keyed,
            bar: position.and_then(|p| p.bar.clone()),
        }
    }

    /// AUTOTUNE in `/api/status`.
    pub fn status(&self) -> AutotuneStatus {
        AutotuneStatus {
            aut: self.rule.is_on(),
            writes: self.rule.writes(),
            taps: 0,
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
                    decision: t.decision,
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
            singers_off: self.rule.singers_off(),
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
/// for all when the batch failed), and whether Live refused the write for
/// good (PR #147 review I5): the script's refusal of the value or of the
/// call, never a ref gone ([`refused_for_good`]) or a call Live did not
/// answer ([`transient`]).
pub(super) fn slot_failures(
    outcome: &Result<Vec<Value>, String>,
    keys: &[String],
) -> Vec<(String, String, bool)> {
    match outcome {
        Err(why) => keys
            .iter()
            .map(|k| (k.clone(), why.clone(), !transient(why)))
            .collect(),
        Ok(slots) => keys
            .iter()
            .enumerate()
            .filter_map(|(i, key)| {
                let slot = slots.get(i);
                refused(slot).map(|why| (key.clone(), why, refused_for_good(slot)))
            })
            .collect(),
    }
}

/// Whether a selector write's call went unanswered (a timeout, offline:
/// [`transient`]): Live may still take it (PR #147 re-review R3).
pub(super) fn unanswered(outcome: &Result<Vec<Value>, String>) -> bool {
    matches!(outcome, Err(why) if transient(why))
}

/// Whether a refused slot is Live's refusal of the value (a held
/// parameter: "Value cannot be set, the parameter is disabled"), not a ref
/// the script no longer resolves ([`STALE_REF`], [`UNKNOWN_REF`]: its
/// target is walked again) nor a slot missing.
pub(super) fn refused_for_good(slot: Option<&Value>) -> bool {
    slot.is_some_and(|slot| {
        !matches!(
            slot.get("errorType").and_then(Value::as_str),
            Some(STALE_REF | UNKNOWN_REF)
        )
    })
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
/// busy, a timeout, offline, or [`MOVED_TRACK`]) keeps its ref and its DRY
/// registration, since the script holds the object; a definite refusal
/// drops them.
pub(super) fn keeps_found(retryable: bool, found: bool) -> bool {
    retryable && found
}

/// Whether a failed walk is walked again (#140): Live did not answer or
/// was offline ([`transient`]), or the track at its index moved
/// ([`MOVED_TRACK`]: the list's push is on its way).
pub(super) fn walk_retryable(why: &str) -> bool {
    transient(why) || why == MOVED_TRACK
}

/// The track a walk must find at its target's index (PR #146 review M2):
/// the list (`tracks` or `return_tracks`), the index and the track's `$ref`
/// as the hub's list holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TrackCheck {
    pub list: &'static str,
    pub index: usize,
    pub id: String,
}

/// The `$ref` of item `index` of a list value as the script encodes it
/// (`[{"$ref", "path", "class", "name"}, …]`).
pub(super) fn item_ref(list: &Value, index: usize) -> Option<&str> {
    list.as_array()?.get(index)?.get("$ref")?.as_str()
}

/// The check of a walk of a target at `anchor` in `instance`: its list and
/// index, and the track the hub's list (`lists`, by hub key: the marker
/// keeper's) names there; none for another anchor or a list not heard (a
/// set not settled yet holds its writes anyway).
pub(super) fn track_check(
    lists: &BTreeMap<String, Value>,
    instance: &str,
    anchor: &Anchor,
) -> Option<TrackCheck> {
    let (list, index) = match anchor {
        Anchor::TrackAt { index } => (TRACKS, *index),
        Anchor::ReturnAt { index } => (RETURNS, *index),
        _ => return None,
    };
    let index = usize::try_from(index).ok()?;
    let held = lists.get(&hub_key(instance, "live_set", list, false))?;
    let id = item_ref(held, index)?.to_string();
    Some(TrackCheck { list, index, id })
}

/// Whether the list Live answered (`slots`) still has the checked track at
/// its index.
pub(super) fn same_track(slots: &[Value], check: &TrackCheck) -> bool {
    slots
        .first()
        .and_then(answered)
        .and_then(|list| item_ref(list, check.index))
        == Some(check.id.as_str())
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
/// read that failed. A macro found is checked against `check` (PR #146
/// review M2): read after the find, the track at the target's index must
/// still be the one the hub's list names there, else the walk failed
/// ([`MOVED_TRACK`]).
async fn find_rack(live: LiveHandle, target: String, check: Option<TrackCheck>) -> Walked {
    let (mut find, mut commands) = Find::start(&target);
    for _ in 0..find::MAX_READS {
        let slots = match live.call(commands).await {
            Ok(slots) => slots,
            Err(e) => return Walked::Failed(e.to_string()),
        };
        match find.answer(&slots) {
            find::Step::Read(next) => commands = next,
            find::Step::Done(Ok(found)) => return checked(live, check, found).await,
            find::Step::Done(Err(missing)) => return Walked::Answered(Err(missing.to_string())),
            find::Step::Failed(why) => return Walked::Failed(why),
        }
    }
    Walked::Answered(Err(TOO_DEEP.to_string()))
}

/// A macro found, once its track is checked (none to check: as found).
async fn checked(live: LiveHandle, check: Option<TrackCheck>, found: Found) -> Walked {
    let Some(check) = check else {
        return Walked::Answered(Ok(found));
    };
    match live.call(vec![get("live_set", check.list)]).await {
        Ok(slots) if same_track(&slots, &check) => Walked::Answered(Ok(found)),
        Ok(_) => Walked::Failed(MOVED_TRACK.to_string()),
        Err(e) => Walked::Failed(e.to_string()),
    }
}

impl Router {
    /// A new layout's targets: gone ones let go (a selector that may be on
    /// written DRY first through its last ref, PR #147 review I3), new ones
    /// walked, kept ones keep what they found and take the new layout's
    /// values, unless their on and off values changed (PR #147 review I4: a
    /// PA track where a stream track was): those are walked afresh before
    /// anything is written or registered for them. The rule is told the
    /// Live sets, every set decided again.
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
                decision: None,
                guard: SelectorGuard::new(),
                writing: Vec::new(),
                sure: true,
                refusal: None,
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
        self.autotune_retire(&gone, Cause::Layout);
        for key in gone {
            self.autotune_unsub(&key);
            self.autotune_key_unsub(&key);
            self.autotune.targets.remove(&key);
        }
        let mut new = Vec::new();
        let mut flipped = Vec::new();
        self.autotune.order = wanted.iter().map(|(key, _)| key.clone()).collect();
        for (key, target) in wanted {
            if let Some(kept) = self.autotune.targets.get_mut(&key) {
                if (kept.spec.on, kept.spec.off) != (target.spec.on, target.spec.off) {
                    flipped.push(key.clone());
                }
                kept.spec = target.spec;
                continue;
            }
            self.autotune.targets.insert(key.clone(), target);
            new.push(key);
        }
        for key in flipped {
            self.autotune_unconfirm(&key);
        }
        tracing::info!(
            targets = self.autotune.order.len(),
            new = new.len(),
            "AUTOTUNE follows the layout's tracks"
        );
        for key in new {
            self.autotune_walk(&key);
        }
        let sets = self.autotune_sets();
        self.autotune_positions(&sets);
        let heard: BTreeMap<String, Option<bool>> = sets
            .iter()
            .map(|s| (s.clone(), self.transport_now(s)))
            .collect();
        self.autotune
            .rule
            .follow(&sets, |s| heard.get(s).copied().flatten());
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
        // A kept target may carry new values or another singer.
        for set in sets {
            self.autotune_decide(&set, Cause::Layout);
        }
        self.autotune_tell();
    }

    /// Writes DRY to each of `keys`' selectors that may be on before the
    /// hub lets them go (PR #147 review I3: a layout without them, a
    /// definite missing walk), through its last ref with its own off value,
    /// one batch per Live set (`cause`); only where the set's writes may go
    /// (a hold already wrote them DRY). Each set's registration waits for
    /// that write's answer, so the script keeps their DRY writes until then.
    fn autotune_retire(&mut self, keys: &[String], cause: Cause) {
        // The markers unsettled with no hold open (PR #147 re-review R2):
        // the hold opens first, so its DRY goes through the targets' last
        // refs while they are still here.
        let unsettled: BTreeSet<String> = keys
            .iter()
            .filter_map(|key| self.autotune.targets.get(key))
            .filter(|t| t.found.is_some())
            .map(|t| t.instance.clone())
            .filter(|set| !self.autotune.held.contains(set) && !self.reverb_settled(set))
            .collect();
        for set in unsettled {
            self.autotune_hold(&set, "markers");
        }
        let mut sets: BTreeMap<String, Vec<switch::Going>> = BTreeMap::new();
        for key in keys {
            let Some(target) = self.autotune.targets.get(key) else {
                continue;
            };
            let Some(found) = &target.found else {
                continue;
            };
            let holding = self.autotune.held.contains(&target.instance);
            if !may_write(holding, self.reverb_settled(&target.instance)) || !target.may_be_on() {
                continue;
            }
            sets.entry(target.instance.clone())
                .or_default()
                .push(switch::Going {
                    key: key.clone(),
                    id: found.id.clone(),
                    label: target.spec.label.clone(),
                    on: false,
                    value: target.spec.off,
                });
        }
        for (set, going) in sets {
            self.autotune_select(&set, cause, going, true);
        }
    }

    /// A kept target whose on and off values changed (PR #147 review I4):
    /// what it found is let go unwritten (its old ref's DRY value is the
    /// other polarity's), it is unconfirmed, so its set's registration waits
    /// for its walk, and walked afresh.
    fn autotune_unconfirm(&mut self, key: &str) {
        self.autotune_unsub(key);
        self.autotune_key_unsub(key);
        self.autotune_key_forget(key);
        let Some(target) = self.autotune.targets.get_mut(key) else {
            return;
        };
        target.found = None;
        target.key = None;
        target.problem = None;
        target.decision = None;
        target.writing.clear();
        target.guard.reset();
        target.refusal = None;
        target.sure = false;
        self.autotune_walk(key);
    }

    /// The Live sets of the layout's targets.
    fn autotune_sets(&self) -> BTreeSet<String> {
        self.autotune
            .targets
            .values()
            .map(|t| t.instance.clone())
            .collect()
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
    /// void), while its Live set is online; its connect walks it else. The
    /// walk checks the track at its index against the hub's track list as
    /// it stands now (PR #146 review M2, [`track_check`]): a list change
    /// handled later walks it again, so its answer is void then anyway.
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
        let check = track_check(
            &self.reverb.lists,
            &target.instance,
            &target.spec.binding.anchor,
        );
        let (live, track, walk) = (live.clone(), target.track.clone(), target.walk);
        let key = key.to_string();
        let tx = self.io.tx.clone();
        tokio::spawn(async move {
            let msg = match find_rack(live, track, check).await {
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
    /// when Live did not answer, was offline or the track at its index
    /// moved ([`walk_retryable`]), walked again after [`RETRY_MS`] while
    /// retries are left, or one `gave_up` record (warn) waits for the next
    /// trigger ([`after_walk`]); a definite refusal waits for the next
    /// trigger.
    pub(super) fn autotune_walk_failed(&mut self, key: &str, walk: u64, why: String) {
        let Some(target) = self.autotune.targets.get(key).filter(|t| t.walk == walk) else {
            return;
        };
        let retryable = walk_retryable(&why);
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
                        instance: instance.clone(),
                        target: track,
                        problem: why,
                        tries: failures,
                    },
                    None,
                );
                // A problem: its selector goes DRY.
                self.autotune_decide(&instance, Cause::Found);
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
    /// nothing; a found macro is followed and its set decided, a missing
    /// one says why.
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
        // A walk's answer confirms it (PR #147 review I4).
        target.sure = true;
        let was = target.found.replace(found.clone());
        let instance = target.instance.clone();
        // Another path, or the same path with a new `$ref` (a rack replaced
        // in place; PR #139 review M1): its value is followed afresh, which
        // also gives up a listener the script ended, and its selector is
        // decided afresh.
        if !same_macro(was.as_ref(), &found) {
            target.decision = None;
            target.writing.clear();
            target.guard.reset();
            target.refusal = None;
            self.autotune_follow(key, &found);
        }
        self.autotune_key_found(key);
        self.autotune_safe(&instance);
        self.autotune_decide(&instance, Cause::Found);
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

    /// A definite answer that it has no rack (or a refused walk): a selector
    /// that may be on is written DRY first (PR #147 review I3), then it is
    /// dropped and its set registered again (once that write answered).
    fn autotune_target_missing(&mut self, key: &str, why: String) {
        self.autotune_retire(&[key.to_string()], Cause::Found);
        if let Some(instance) = self.autotune_drop(key, why) {
            self.autotune_safe(&instance);
        }
    }

    /// A target whose selector's ref the script no longer knows (PR #146
    /// review N1): missing like a rack gone; its set's registration goes
    /// again once every gone one is dropped.
    pub(super) fn autotune_ref_gone(&mut self, key: &str) {
        self.autotune_drop(key, safe::REF_GONE.to_string());
    }

    /// Drops what a target found (its subscriptions, its key device, its
    /// decision): missing with `why`, recorded when that is news; its Live
    /// set.
    fn autotune_drop(&mut self, key: &str, why: String) -> Option<String> {
        self.autotune_unsub(key);
        self.autotune_key_unsub(key);
        self.autotune_key_forget(key);
        let target = self.autotune.targets.get_mut(key)?;
        let news = missing_news(target.problem.as_deref(), &why);
        target.found = None;
        target.key = None;
        target.value = None;
        target.decision = None;
        target.writing.clear();
        target.refusal = None;
        // A definite answer: nothing waits for its walk.
        target.sure = true;
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
        Some(instance)
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

    /// The answer to selector write `write` (`selectors`: each target's
    /// key, the `$ref` written and the value's side): a write that failed
    /// is no longer on its way, one that went is answered
    /// ([`answered_flying`]); each failure is recorded, a refusal for good
    /// counted (PR #147 review I5: its one retry is the next decision's,
    /// then that value to that `$ref` stops), any other failure (a stale
    /// ref, a deleted rack, no answer) walks its target again. A set whose
    /// registration waited for this DRY write (a target let go, PR #147
    /// review I3) registers now. Then the set is decided again (I1: after
    /// every answer).
    pub(super) fn autotune_written(
        &mut self,
        instance: &str,
        write: u64,
        selectors: &[(String, String, bool)],
        outcome: &Result<Vec<Value>, String>,
    ) {
        let keys: Vec<String> = selectors.iter().map(|(key, _, _)| key.clone()).collect();
        let failures = slot_failures(outcome, &keys);
        // Live did not answer the call (a timeout, offline): it may still
        // take it, so every write of it stays on its way (PR #147
        // re-review R3).
        let lost = unanswered(outcome);
        for (key, id, on) in selectors {
            let failed = failures.iter().find(|(k, _, _)| k == key);
            let Some(target) = self.autotune.targets.get_mut(key) else {
                continue;
            };
            if !lost {
                target.writing = answered_flying(&target.writing, write, failed.is_none());
            }
            let same_ref = target.found.as_ref().is_some_and(|found| found.id == *id);
            if let (Some((_, why, true)), true) = (failed, same_ref) {
                target.refusal = Some(Refusal::after(target.refusal.as_ref(), id, *on, why));
            }
        }
        for (key, problem, for_good) in failures {
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
            if !for_good && !lost {
                // Its ref is gone: in doubt until the walk answers, no
                // write meanwhile (the decision after this answer would
                // write it again at once otherwise).
                if let Some(target) = self.autotune.targets.get_mut(&key) {
                    target.sure = false;
                }
                self.autotune_walk(&key);
            }
        }
        let retired = self
            .autotune
            .retiring
            .get_mut(instance)
            .is_some_and(|writes| writes.remove(&write));
        if retired {
            if self
                .autotune
                .retiring
                .get(instance)
                .is_some_and(BTreeSet::is_empty)
            {
                self.autotune.retiring.remove(instance);
            }
            self.autotune_safe(instance);
        }
        self.autotune_decide(instance, Cause::Written);
    }

    /// `instance` connected: its targets are walked (the script's refs start
    /// over with each connection); each answer decides it.
    pub(super) fn autotune_connected(&mut self, instance: &str) {
        for key in self.autotune.keys_of(Some(instance), false) {
            self.autotune_walk(&key);
        }
        self.autotune_tell();
    }

    /// `instance` went offline: first its walks in flight are voided, then
    /// every ref, value and decision it found dropped and the rule told;
    /// its next session starts afresh (the script made its DRY writes).
    pub(super) fn autotune_offline(&mut self, instance: &str) {
        let keys = self.autotune.keys_of(Some(instance), false);
        for key in &keys {
            self.autotune.walks += 1;
            let walk = self.autotune.walks;
            if let Some(target) = self.autotune.targets.get_mut(key) {
                target.walk = walk;
                target.found = None;
                target.problem = None;
                target.decision = None;
                target.writing.clear();
                target.guard.reset();
                target.refusal = None;
                target.sure = true;
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
        self.autotune.retiring.remove(instance);
        self.autotune_position_offline(instance);
        self.autotune_hold(instance, "offline");
        self.autotune.rule.on_disconnect(instance);
        self.autotune_tell();
    }

    /// `instance`'s track list changed: its writes are held (DRY first
    /// through the last known refs) and its targets walked again (an index
    /// may name another track now), keeping what they found until the
    /// answer.
    pub(super) fn autotune_tracks(&mut self, instance: &str) {
        self.autotune_hold(instance, "tracks");
        for key in self.autotune.keys_of(Some(instance), false) {
            // Its key device's values may be another track's until its walk
            // answers (PR 2a review M3), and so may its rack: every change
            // unconfirms it, a second one within the hold too (PR #147
            // re-review R1).
            if let Some(target) = self.autotune.targets.get_mut(&key) {
                target.key_sure = false;
                if target.found.is_some() {
                    target.sure = false;
                }
            }
            self.autotune_walk(&key);
        }
    }

    /// Holds `instance`'s writes (`why`: the cause), recorded once; only
    /// for a Live set with AUTOTUNE targets. A selector the rule had on is
    /// written DRY first, through its last known ref (spec §7: a hold never
    /// leaves a singer on autotune; refs are objects, never an index).
    fn autotune_hold(&mut self, instance: &str, why: &str) {
        if self.autotune.keys_of(Some(instance), false).is_empty()
            || self.autotune.held.contains(instance)
        {
            return;
        }
        let going = self.autotune_judge(instance, Cause::Hold, true);
        self.autotune_select(instance, Cause::Hold, going, false);
        self.autotune.held.insert(instance.to_string());
        // What each target found is of the track its index named before:
        // unconfirmed until its walk answers (PR #147 review I4).
        for key in self.autotune.keys_of(Some(instance), false) {
            let Some(target) = self.autotune.targets.get_mut(&key) else {
                continue;
            };
            if target.found.is_some() {
                target.sure = false;
            }
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
    /// targets (#140 PR 3a), its targets are walked again and the set is
    /// decided again (each target unconfirmed since the hold DRY until its
    /// walk answers: PR #147 review I4). Settled with no hold, a
    /// registration a walk's answer left to the markers goes now, and the
    /// set is decided again too (PR #147 review N2: a decision judged held
    /// for unsettled markers is lost otherwise).
    pub(super) fn autotune_release(&mut self, instance: &str, why: &str) {
        if !self.reverb_settled(instance) {
            return;
        }
        if !self.autotune.held.remove(instance) {
            self.autotune_safe(instance);
            self.autotune_decide(instance, Cause::Release);
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
        self.autotune_decide(instance, Cause::Release);
    }

    /// A transport change of `instance`: the set's unfound targets walked
    /// again, the key in force judged, every target decided (a stop is DRY
    /// at once).
    pub(super) fn autotune_transport(&mut self, instance: &str, playing: bool) {
        self.autotune.rule.on_playing(instance, playing);
        for key in self.autotune.keys_of(Some(instance), true) {
            self.autotune_walk(&key);
        }
        self.autotune_position_playing(instance, Some(playing));
        self.autotune_decide(instance, Cause::Transport);
        self.autotune_tell();
    }

    /// The rules' transport listener of `instance` failed: AUTOTUNE forgets
    /// its transport, so every target goes DRY (an unknown transport is no
    /// play).
    pub(super) fn autotune_transport_lost(&mut self, instance: &str) {
        self.autotune.rule.transport_lost(instance);
        self.autotune_position_playing(instance, None);
        self.autotune_decide(instance, Cause::Transport);
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

    /// A delivery of a target's macro value: a value (the echo of the
    /// hub's own write on its way, or judged for another writer: #140 PR
    /// 3b), or an error (the macro's path no longer resolves: its value
    /// dropped and the target walked again).
    fn autotune_macro_value(&mut self, key: &str, value: Option<Value>, error: Option<String>) {
        match transport_heard(value, error) {
            Some(Ok(value)) => {
                let Some(target) = self.autotune.targets.get_mut(key) else {
                    return;
                };
                let was = target.value;
                target.value = value.as_f64();
                // The writes on their way before it, then those it did not
                // land (PR #147 review I1).
                let flying: Vec<bool> = target.writing.iter().map(|f| f.on).collect();
                target.writing = still_flying(&target.writing, target.shows());
                self.autotune_heard(key, was, &flying);
            }
            Some(Err(error)) => {
                tracing::warn!(key = %key, error = ?error, "AUTOTUNE lost a rack's macro: it looks again");
                if let Some(target) = self.autotune.targets.get_mut(key) {
                    target.value = None;
                    target.writing.clear();
                }
                self.autotune_walk(key);
            }
            None => {}
        }
    }

    /// A page's `set_hub` of an AUTOTUNE key: whether it was one (another,
    /// or a value it does not take, falls through to the error reply).
    /// `autotune_aut` takes a bool (#140 PR 3c: on and off), `autotune_singer`
    /// a singer's `{id, off}` (#140 PR 3b), and the tap `autotune` is
    /// refused.
    pub(super) fn autotune_set_hub(&mut self, client: ClientId, key: &str, value: &Value) -> bool {
        match key {
            HUB_AUTOTUNE_AUT => match value.as_bool() {
                Some(on) => self.autotune_flag(client, on),
                None => return false,
            },
            HUB_AUTOTUNE => self.autotune_refused(client, "tap", REFUSED_WHY),
            HUB_AUTOTUNE_SINGER => {
                let Some((id, off)) = switch::singer_value(value) else {
                    return false;
                };
                if !self.autotune_takes_singer(&id, off) {
                    return false;
                }
                self.autotune_singer(client, &id, off);
            }
            _ => return false,
        }
        true
    }

    /// AUTOTUNE AUT set (a page's hold, #140 PR 3c): recorded; saved when
    /// it differs from `hub-state.json`; every set
    /// decided at once (on: what the rule allows, a selected track only
    /// once its key is confirmed; off: every target DRY).
    pub(super) fn autotune_flag(&mut self, client: ClientId, on: bool) {
        let changed = on != self.state.autotune_aut;
        self.autotune_step(Step::Flag { on }, Some(client));
        self.autotune.rule.set_flag(on);
        if changed {
            self.state.autotune_aut = on;
            self.save_state();
        }
        self.autotune_tell();
        for set in self.autotune_sets() {
            self.autotune_decide(&set, Cause::Flag);
        }
        for key in self.autotune.keys_of(None, true) {
            self.autotune_walk(&key);
        }
    }

    /// Whether a page may switch the singer `id` (`off`): a target of the
    /// layout belongs to it, or it is off by hand and goes back on (PR #147
    /// review N6: an id a marker edit left behind is cleared, never kept).
    fn autotune_takes_singer(&self, id: &str, off: bool) -> bool {
        let has = self
            .autotune
            .targets
            .values()
            .any(|t| fohmixer_proto::layout::singer_of(&t.spec).as_deref() == Some(id));
        has || (!off && self.autotune.rule.is_off(id))
    }

    /// A page switched a singer off by hand or back on (spec §2.2: both of
    /// its tracks): recorded; saved only on a change; told; every set
    /// decided.
    fn autotune_singer(&mut self, client: ClientId, id: &str, off: bool) {
        self.autotune_step(
            Step::Singer {
                id: id.to_string(),
                off,
            },
            Some(client),
        );
        if self.autotune.rule.set_singer(id, off) {
            self.state.autotune_off = self.autotune.rule.singers_off();
            self.save_state();
        }
        self.autotune_tell();
        for set in self.autotune_sets() {
            self.autotune_decide(&set, Cause::Singer);
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
