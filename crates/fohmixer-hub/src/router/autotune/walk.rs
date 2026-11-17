//! The walks of AUTOTUNE's targets (#104, #140; split from
//! `router/autotune.rs`, #234): each walk finds a target's `Latencies` rack
//! ([`find_rack`], at most `find::MAX_READS` reads, its track checked
//! against the hub's list), a newer walk voiding an older answer; a failed
//! walk is retried while Live did not answer ([`after_walk`]) and gives up
//! after [`CONNECT_RETRIES`]; an answer follows the found macro or drops a
//! missing target. The walks' pacing is `pace.rs` (#235).

use std::collections::BTreeMap;

use fohmixer_proto::client::hub_key;
use fohmixer_proto::layout::Anchor;
use serde_json::Value;

use super::super::markers::{RETURNS, TRACKS};
use super::super::retries::{CONNECT_RETRIES, Due, RETRY_MS, transient};
use super::super::{Router, RouterMsg};
use super::{AUTOTUNE_CLIENT, MOVED_TRACK, TOO_DEEP, same_macro};
use crate::eq::walk::{answered, get};
use crate::live::client::LiveHandle;
use crate::live::subs::Cached;
use crate::rules::autotune::find::{self, Find, Found};
use crate::rules::autotune::{Cause, Step};

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
    /// Walks a target again for a trigger (a layout, a connect, a transport
    /// change, a track-list change, a failed write or value): its failed
    /// walks' count starts over (#140).
    pub(in crate::router) fn autotune_walk(&mut self, key: &str) {
        if let Some(target) = self.autotune.targets.get_mut(key) {
            target.walk_failures = 0;
        }
        self.autotune_walk_now(key);
    }

    /// Walks a target (a new walk number: an answer of an older walk is
    /// void), while its Live set is online; its connect walks it else. The
    /// walk waits its turn in its set's queue (#235, `autotune/pace.rs`),
    /// whose start passes over a target of a set offline.
    fn autotune_walk_now(&mut self, key: &str) {
        let Some(target) = self.autotune.targets.get_mut(key) else {
            return;
        };
        self.autotune.walks += 1;
        target.walk = self.autotune.walks;
        let instance = target.instance.clone();
        self.autotune_pace_ask(&instance, key);
    }

    /// Starts a target's newest walk now, while its Live set is online: its
    /// walk number, or none when nothing went. The walk checks the track at
    /// its index against the hub's track list as it stands now (PR #146
    /// review M2, [`track_check`]): a list change handled later walks it
    /// again, so its answer is void then anyway.
    pub(super) fn autotune_walk_start(&self, key: &str) -> Option<u64> {
        let target = self.autotune.targets.get(key)?;
        let online = self.states.get(&target.instance).is_some_and(|s| s.online);
        let live = self.live.get(&target.instance).filter(|_| online)?;
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
        Some(walk)
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
    pub(in crate::router) fn autotune_walk_failed(&mut self, key: &str, walk: u64, why: String) {
        // Its last answer: the set's next walk follows the gap (#235).
        self.autotune_pace_ended(key, walk);
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
    pub(in crate::router) fn autotune_walk_retry(&mut self, key: &str, walk: u64) {
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
    pub(in crate::router) fn autotune_found(
        &mut self,
        key: &str,
        walk: u64,
        outcome: Result<Found, String>,
    ) {
        // Its last answer: the set's next walk follows the gap (#235).
        self.autotune_pace_ended(key, walk);
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
}
