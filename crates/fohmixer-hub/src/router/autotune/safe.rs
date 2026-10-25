//! The fall back to DRY when the hub is gone (#140 PR 3a, spec §8): the
//! router gives each Live set's FohMixer script the DRY write of every
//! found selector (its macro's `$ref` and its target's off value, whatever
//! AUT is), and the script makes them when its connection to the hub ends
//! or the hub said nothing on it for 1.5 s (`live-script/FohMixer/safe.py`;
//! the hub pings every 500 ms, `live::client::KEEPALIVE_MS`).
//!
//! - **When:** after every walk's answer (a macro found or lost), a layout
//!   (a target gone), and where the set's markers settle; a list goes only
//!   when it is not what the script holds ([`safe_news`]).
//! - **Never while the set's writes are held** (`may_write`, the macros'
//!   own rule): a walk by a stale index may have found another track's
//!   rack, whose DRY value is not the target's. Meanwhile the script keeps
//!   the list it holds: its refs are the objects, never an index. The
//!   hold's release registers the set as it stands.
//! - **The connection:** the script keeps a list per connection, so a
//!   disconnect forgets the registration (`autotune_offline`), and the
//!   connect's walks register afresh.
//! - **An answer:** one `safe` record (warn when it failed). Only the
//!   script's OK makes a registration `kept` (PR #146 review I2: #140 PR 3b
//!   switches a selector on only under a kept one); until then it is
//!   `sent`. A failed one leaves the hub not knowing what the script holds
//!   (`failed`), so the next change sends the list again, whatever it is;
//!   one Live did not answer (`retries::transient`: a timeout, offline) is
//!   also sent again `RETRY_MS` after it, up to `CONNECT_RETRIES` in a row
//!   (a due retry waits, uncounted, while Live is busy), a refusal never.
//!   `/api/status` `autotune.safe` shows each set's state.
//! - **A ref gone (PR #146 review N1):** the script refuses a list with a
//!   target that does not resolve whole (a rack deleted after a walk kept
//!   its ref). The hub then asks the script for each listed ref's value:
//!   the targets whose ref it answers stale or unknown are dropped (and
//!   walked again), and the rest are registered again; one stale ref
//!   never leaves the whole set unregistered. A probe Live did not answer
//!   is asked again (`retries::transient`, up to `CONNECT_RETRIES`; PR
//!   #147 review N1).
//! - **The decision (PR #146 review I2, refined by the PR #147 ruling on
//!   I2):** a selector is armed while both the list the script last kept
//!   (`Safe.kept`) and the newest one sent hold its DRY write, so a
//!   registration that only adds a target takes no selector DRY, and one
//!   that drops a selector's write does: every change decides the set.
//! - **It waits** while a DRY write of a target let go is on its way (PR
//!   #147 review I3) and while any target, found or not, waits for its
//!   walk unconfirmed (I4, re-review R1): the script keeps the list it
//!   holds until then. An unconfirmed target whose walk gave up is left
//!   out of the list (re-review R8).
//!
//! Nothing here writes a selector.

use fohmixer_proto::client::{SafeState, SafeStatus};
use serde_json::{Value, json};

use super::super::retries::{Due, RETRY_MS, transient};
use super::super::{Router, RouterMsg};
use super::{AfterWalk, AutotuneIo, PARAMETER_CLASS, after_walk, gave_up_problem, may_write};
use crate::eq::close::{STALE_REF, UNKNOWN_REF};
use crate::eq::walk::refused;
use crate::rules::autotune::{Cause, Step};

/// The script's command that keeps a connection's DRY writes.
pub const SAFE_WRITES: &str = "safe_writes";
/// The script's words when a listed target does not resolve (`<n> safe
/// write target(s) do not resolve`): a ref gone.
pub const DO_NOT_RESOLVE: &str = "do not resolve";
/// A target's problem once its ref is gone (PR #146 review N1).
pub const REF_GONE: &str = "its rack's selector is gone from Live";

/// What the hub registered with a Live set's script since its connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::router) struct Safe {
    /// The DRY writes sent last: each found macro's `$ref` and its target's
    /// off value, in the layout's order.
    pub list: Vec<(String, u8)>,
    /// That registration's number: an answer or a retry of another is void.
    pub seq: u64,
    /// Where it stands: on its way, kept by the script, or failed (what the
    /// script holds is not known).
    pub state: SafeState,
    /// Why it failed (with the give-up once its retries are spent).
    pub problem: Option<String>,
    /// Its failures in a row Live did not answer (the retries' count).
    pub failures: u32,
    /// The list the script last kept (its OK) since the connect, through
    /// the registrations sent or failed since (PR #147 ruling on I2).
    pub kept: Option<Vec<(String, u8)>>,
    /// Its refs-gone probes Live did not answer in a row (PR #147 review
    /// N1: the retries' count).
    pub probes: u32,
    /// Whether it failed because Live did not answer (a timeout, offline):
    /// the script may still take it (PR #147 re-review R4).
    pub lost: bool,
}

/// The list the script last kept, as far as the hub may rely on it once a
/// new list goes after `previous` (PR #147 re-review R4): a registration
/// still on its way, or one Live did not answer, may still become the
/// script's list without the hub hearing of it, so only the pairs it
/// shares with the kept list stay; after a kept or refused one, the kept
/// list as it is.
pub(super) fn kept_after(previous: Option<&Safe>) -> Option<Vec<(String, u8)>> {
    let previous = previous?;
    let kept = previous.kept.as_ref()?;
    let open =
        previous.state == SafeState::Sent || (previous.state == SafeState::Failed && previous.lost);
    if !open {
        return Some(kept.clone());
    }
    Some(
        kept.iter()
            .filter(|pair| previous.list.contains(pair))
            .cloned()
            .collect(),
    )
}

/// The script's `safe_writes` of `writes` (each macro's `$ref` and DRY
/// value): one command, the list it keeps for this connection.
pub(super) fn safe_commands(writes: &[(&str, u8)]) -> Vec<Value> {
    let writes: Vec<Value> = writes
        .iter()
        .map(|(id, off)| {
            json!({
                "target": {"$ref": id, "class": PARAMETER_CLASS},
                "value": f64::from(*off),
            })
        })
        .collect();
    vec![json!({"target": "live_set", "name": SAFE_WRITES, "args": {"writes": writes}})]
}

/// Whether `list` must be registered: it is not what the script holds
/// (`held`; none since the connect, when the script holds nothing, so an
/// empty list needs no registration; after a failure that is not known,
/// so any list goes).
pub(super) fn safe_news(held: Option<&Safe>, list: &[(String, u8)]) -> bool {
    match held {
        None => !list.is_empty(),
        Some(safe) => safe.state == SafeState::Failed || safe.list != list,
    }
}

/// Why a registration failed: the call's error (Live offline, a timeout),
/// or the script's refusal of its one command (an old script: no such
/// function; a target it cannot resolve); none when the script keeps the
/// list.
pub(super) fn safe_problem(outcome: &Result<Vec<Value>, String>) -> Option<String> {
    match outcome {
        Err(why) => Some(why.clone()),
        Ok(slots) => refused(slots.first()),
    }
}

/// Whether a registration's refusal names a target that does not resolve
/// (a ref gone: the hub asks which).
pub(super) fn does_not_resolve(problem: &str) -> bool {
    problem.contains(DO_NOT_RESOLVE)
}

/// The probe of a refused registration's refs: each one's `value` through
/// its `$ref` and class, in one call.
pub(super) fn probe_commands(ids: &[String]) -> Vec<Value> {
    ids.iter()
        .map(|id| {
            json!({
                "target": {"$ref": id, "class": PARAMETER_CLASS},
                "name": "get_prop",
                "args": {"prop": "value"},
            })
        })
        .collect()
}

/// The refs of `ids` the probe's answer names gone: the script answered
/// them stale (the object was deleted) or unknown; none when the call
/// failed (nothing is known).
pub(super) fn gone_refs(ids: &[String], outcome: &Result<Vec<Value>, String>) -> Vec<String> {
    let Ok(slots) = outcome else {
        return Vec::new();
    };
    ids.iter()
        .zip(slots)
        .filter(|(_, slot)| {
            matches!(
                slot.get("errorType").and_then(Value::as_str),
                Some(STALE_REF | UNKNOWN_REF)
            )
        })
        .map(|(id, _)| id.clone())
        .collect()
}

/// Whether `safe` is still the failed registration `seq` (a retry or a
/// probe's answer of another, or of one kept since, is void).
pub(super) fn failed_current(safe: Option<&Safe>, seq: u64) -> bool {
    safe.is_some_and(|safe| safe.seq == seq && safe.state == SafeState::Failed)
}

/// What `/api/status` says of a failed registration: Live's words, and
/// once its retries are spent (`after`), that it gave up after `tries`.
pub(super) fn failed_problem(why: &str, after: AfterWalk, tries: u32) -> String {
    match after {
        AfterWalk::GiveUp => gave_up_problem(why, tries),
        AfterWalk::Retry | AfterWalk::Wait => why.to_string(),
    }
}

impl AutotuneIo {
    /// Each Live set's registration as `/api/status` shows it.
    pub(super) fn safe_status(&self) -> Vec<SafeStatus> {
        self.safe
            .iter()
            .map(|(instance, safe)| SafeStatus {
                instance: instance.clone(),
                state: safe.state,
                targets: safe.list.len(),
                problem: safe.problem.clone(),
            })
            .collect()
    }

    /// `set`'s DRY writes: each found and confirmed target's macro `$ref`
    /// with its off value, in the layout's order (Live's), whatever AUT is.
    pub(super) fn safe_list(&self, set: &str) -> Vec<(String, u8)> {
        self.keys_in_order(set)
            .iter()
            .filter_map(|key| {
                let target = self.targets.get(key)?;
                // An unconfirmed one is left out (PR #147 re-review R8):
                // its ref may be of a track an older layout named there,
                // also once its walk gave up; it is DRY and written nothing.
                let found = target.found.as_ref().filter(|_| target.sure)?;
                Some((found.id.clone(), target.spec.off))
            })
            .collect()
    }
}

impl Router {
    /// Registers `instance`'s DRY writes with its script when they are not
    /// what it holds ([`safe_news`]), a change point: the failures' count
    /// starts over. Not while the set's writes are held or its markers are
    /// not settled (`may_write`); the hold's release, or the read that
    /// settles the markers, registers then. The answer comes back as
    /// [`RouterMsg::AutotuneSafe`].
    pub(in crate::router) fn autotune_safe(&mut self, instance: &str) {
        self.autotune_safe_now(instance, 0);
    }

    /// [`Router::autotune_safe`] with `failures` already counted (a retry).
    /// It also waits while a DRY write of a target let go is on its way
    /// (PR #147 review I3) or any target waits for its walk unconfirmed
    /// (`Target::awaits_walk`, I4, re-review R1): the script keeps the list
    /// it holds meanwhile.
    fn autotune_safe_now(&mut self, instance: &str, failures: u32) {
        let holding = self.autotune.held.contains(instance);
        if !may_write(holding, self.reverb_settled(instance))
            || self.autotune.retiring.contains_key(instance)
            || self
                .autotune
                .keys_in_order(instance)
                .iter()
                .any(|key| self.autotune.targets[key].awaits_walk())
        {
            return;
        }
        let list = self.autotune.safe_list(instance);
        if !safe_news(self.autotune.safe.get(instance), &list) {
            return;
        }
        let Some(live) = self.live.get(instance) else {
            return;
        };
        self.autotune.safe_seq += 1;
        let seq = self.autotune.safe_seq;
        let pairs: Vec<(&str, u8)> = list.iter().map(|(id, off)| (id.as_str(), *off)).collect();
        let result = live.call(safe_commands(&pairs));
        let targets = list.len();
        let kept = kept_after(self.autotune.safe.get(instance));
        self.autotune.safe.insert(
            instance.to_string(),
            Safe {
                list,
                seq,
                state: SafeState::Sent,
                problem: None,
                failures,
                kept,
                probes: 0,
                lost: false,
            },
        );
        let tx = self.io.tx.clone();
        let set = instance.to_string();
        tokio::spawn(async move {
            let outcome = result.await.map_err(|e| e.to_string());
            let _ = tx.send(RouterMsg::AutotuneSafe {
                instance: set,
                seq,
                targets,
                outcome,
            });
        });
        // Not kept until the script says so: a selector the new list drops
        // goes DRY.
        self.autotune_decide(instance, Cause::Safe);
    }

    /// A registration's answer: void unless it is `instance`'s newest
    /// since its connect; one `safe` record (warn when it failed). The
    /// script's OK makes it `kept`; a failure `failed` (the next change
    /// sends the list again, whatever it is), and one Live did not answer
    /// is sent again after [`RETRY_MS`] while retries are left
    /// ([`after_walk`], the walks' rule), a refusal never.
    pub(in crate::router) fn autotune_safe_answered(
        &mut self,
        instance: &str,
        seq: u64,
        targets: usize,
        outcome: &Result<Vec<Value>, String>,
    ) {
        let Some(safe) = self
            .autotune
            .safe
            .get_mut(instance)
            .filter(|safe| safe.seq == seq)
        else {
            return;
        };
        let problem = safe_problem(outcome);
        let retry = match &problem {
            None => {
                safe.state = SafeState::Kept;
                safe.problem = None;
                safe.failures = 0;
                safe.kept = Some(safe.list.clone());
                safe.lost = false;
                false
            }
            Some(why) => {
                let retryable = transient(why);
                safe.lost = retryable;
                if retryable {
                    safe.failures += 1;
                }
                let after = after_walk(retryable, safe.failures);
                safe.state = SafeState::Failed;
                safe.problem = Some(failed_problem(why, after, safe.failures));
                after == AfterWalk::Retry
            }
        };
        let gone = problem.as_deref().is_some_and(does_not_resolve);
        self.autotune_step(
            Step::Safe {
                instance: instance.to_string(),
                targets,
                problem,
            },
            None,
        );
        if retry {
            self.autotune_safe_retry_later(instance, seq);
        }
        if gone {
            self.autotune_safe_probe(instance, seq);
        }
        self.autotune_decide(instance, Cause::Safe);
    }

    /// Asks the script which refs of `instance`'s refused registration
    /// `seq` are gone; the answer comes back as
    /// [`RouterMsg::AutotuneSafeProbed`].
    fn autotune_safe_probe(&self, instance: &str, seq: u64) {
        let (Some(safe), Some(live)) = (self.autotune.safe.get(instance), self.live.get(instance))
        else {
            return;
        };
        let ids: Vec<String> = safe.list.iter().map(|(id, _)| id.clone()).collect();
        let result = live.call(probe_commands(&ids));
        let (instance, tx) = (instance.to_string(), self.io.tx.clone());
        tokio::spawn(async move {
            let outcome = result.await.map_err(|e| e.to_string());
            let _ = tx.send(RouterMsg::AutotuneSafeProbed {
                instance,
                seq,
                ids,
                outcome,
            });
        });
    }

    /// The probe's answer (void unless `instance`'s newest registration is
    /// still the refused `seq`): every target whose ref is gone is dropped
    /// (missing, walked again: a rack replaced is found anew), and the rest
    /// is registered again. A probe Live did not answer is asked again after
    /// [`RETRY_MS`] while retries are left (PR #147 review N1, the walks'
    /// rule), then the status says it gave up.
    pub(in crate::router) fn autotune_safe_probed(
        &mut self,
        instance: &str,
        seq: u64,
        ids: &[String],
        outcome: &Result<Vec<Value>, String>,
    ) {
        let Some(safe) = self
            .autotune
            .safe
            .get_mut(instance)
            .filter(|safe| failed_current(Some(&**safe), seq))
        else {
            return;
        };
        if let Err(why) = outcome {
            if !transient(why) {
                return;
            }
            safe.probes += 1;
            let probes = safe.probes;
            if after_walk(true, probes) == AfterWalk::Retry {
                self.autotune_safe_probe_later(instance, seq);
            } else {
                safe.problem = Some(gave_up_problem(why, probes));
            }
            return;
        }
        let gone = gone_refs(ids, outcome);
        if gone.is_empty() {
            return;
        }
        let keys: Vec<String> = self
            .autotune
            .keys_in_order(instance)
            .into_iter()
            .filter(|key| {
                self.autotune.targets[key]
                    .found
                    .as_ref()
                    .is_some_and(|found| gone.contains(&found.id))
            })
            .collect();
        for key in keys {
            self.autotune_ref_gone(&key);
            self.autotune_walk(&key);
        }
        self.autotune_safe(instance);
    }

    /// Asks for the probe of `instance`'s refused registration `seq` again
    /// after [`RETRY_MS`] ([`RouterMsg::AutotuneSafeProbeRetry`]).
    fn autotune_safe_probe_later(&self, instance: &str, seq: u64) {
        let (instance, tx) = (instance.to_string(), self.io.tx.clone());
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(RETRY_MS)).await;
            let _ = tx.send(RouterMsg::AutotuneSafeProbeRetry { instance, seq });
        });
    }

    /// A probe's retry is due: asked again, unless the set's newest
    /// registration is no longer the refused `seq` (void); while Live is
    /// busy it waits another [`RETRY_MS`], uncounted.
    pub(in crate::router) fn autotune_safe_probe_retry(&mut self, instance: &str, seq: u64) {
        if !failed_current(self.autotune.safe.get(instance), seq) {
            return;
        }
        if self.retry_due(instance) == Due::Wait {
            self.autotune_safe_probe_later(instance, seq);
            return;
        }
        self.autotune_safe_probe(instance, seq);
    }

    /// Asks for a retry of `instance`'s failed registration `seq` after
    /// [`RETRY_MS`] ([`RouterMsg::AutotuneSafeRetry`]).
    fn autotune_safe_retry_later(&self, instance: &str, seq: u64) {
        let (instance, tx) = (instance.to_string(), self.io.tx.clone());
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(RETRY_MS)).await;
            let _ = tx.send(RouterMsg::AutotuneSafeRetry { instance, seq });
        });
    }

    /// A failed registration's retry is due: sent again with its failures
    /// counted, unless it is no longer the set's newest or failed (a newer
    /// registration, a disconnect: void); while Live is busy it waits
    /// another [`RETRY_MS`], uncounted (as the walks' retries).
    pub(in crate::router) fn autotune_safe_retry(&mut self, instance: &str, seq: u64) {
        let Some(failures) = self
            .autotune
            .safe
            .get(instance)
            .filter(|safe| failed_current(Some(*safe), seq))
            .map(|safe| safe.failures)
        else {
            return;
        };
        if self.retry_due(instance) == Due::Wait {
            self.autotune_safe_retry_later(instance, seq);
            return;
        }
        self.autotune_safe_now(instance, failures);
    }
}

#[cfg(test)]
#[path = "safe_tests.rs"]
mod tests;
