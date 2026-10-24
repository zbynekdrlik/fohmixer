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
//! - **An answer:** one `safe` record (warn when it failed). A failed one
//!   leaves the hub not knowing what the script holds, so the next change
//!   sends the list again, whatever it is.
//!
//! Nothing here writes a selector.

use serde_json::{Value, json};

use super::super::{Router, RouterMsg};
use super::{AutotuneIo, PARAMETER_CLASS, may_write};
use crate::eq::walk::refused;
use crate::rules::autotune::Step;

/// The script's command that keeps a connection's DRY writes.
pub const SAFE_WRITES: &str = "safe_writes";

/// What the hub registered with a Live set's script since its connect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::router) struct Safe {
    /// The DRY writes sent last: each found macro's `$ref` and its target's
    /// off value, in the layout's order; none when that registration failed
    /// (what the script holds is not known).
    pub list: Option<Vec<(String, u8)>>,
    /// That registration's number: an answer of another is void.
    pub seq: u64,
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
/// empty list needs no registration).
pub(super) fn safe_news(held: Option<&Safe>, list: &[(String, u8)]) -> bool {
    match held {
        None => !list.is_empty(),
        Some(safe) => safe.list.as_deref() != Some(list),
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

impl AutotuneIo {
    /// `set`'s DRY writes: each found target's macro `$ref` with its off
    /// value, in the layout's order (Live's), whatever AUT is.
    pub(super) fn safe_list(&self, set: &str) -> Vec<(String, u8)> {
        self.keys_in_order(set)
            .iter()
            .filter_map(|key| {
                let target = self.targets.get(key)?;
                let found = target.found.as_ref()?;
                Some((found.id.clone(), target.spec.off))
            })
            .collect()
    }
}

impl Router {
    /// Registers `instance`'s DRY writes with its script when they are not
    /// what it holds ([`safe_news`]). Not while the set's writes are held
    /// or its markers are not settled (`may_write`); the hold's release, or
    /// the read that settles the markers, registers then. The answer comes
    /// back as [`RouterMsg::AutotuneSafe`].
    pub(in crate::router) fn autotune_safe(&mut self, instance: &str) {
        let holding = self.autotune.held.contains(instance);
        if !may_write(holding, self.reverb_settled(instance)) {
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
        self.autotune.safe.insert(
            instance.to_string(),
            Safe {
                list: Some(list),
                seq,
            },
        );
        let (instance, tx) = (instance.to_string(), self.io.tx.clone());
        tokio::spawn(async move {
            let outcome = result.await.map_err(|e| e.to_string());
            let _ = tx.send(RouterMsg::AutotuneSafe {
                instance,
                seq,
                targets,
                outcome,
            });
        });
    }

    /// A registration's answer: void unless it is `instance`'s newest
    /// since its connect; one `safe` record (warn when it failed). A failed
    /// one leaves what the script holds unknown, so the next change sends
    /// the list again, whatever it is.
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
        if problem.is_some() {
            safe.list = None;
        }
        self.autotune_step(
            Step::Safe {
                instance: instance.to_string(),
                targets,
                problem,
            },
            None,
        );
    }
}

#[cfg(test)]
#[path = "safe_tests.rs"]
mod tests;
