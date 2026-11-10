//! A stop's DRY writes spread out (#140, the DECIDED comment of
//! 2026-11-09): dasio measured band Live's audio callback at 5.8-11.5 ms at
//! every stop while the hub wrote every selector in one batch (each write
//! swaps a rack's chain, all in one Live tick, one audio block; 11.45 ms
//! dropped a block, a click), about 3 ms without them.
//!
//! - **One selector per batch,** [`STOP_SPREAD_MS`] apart, in Live's order:
//!   the first at once, each next one when its turn's timer fires
//!   ([`RouterMsg::AutotuneSpread`] with the spread's generation; a stale
//!   one does nothing).
//! - **A write still to come is on its way already** (`Target.writing`,
//!   numbered when the spread starts), so every decision meanwhile (a write's
//!   answer, an echo) judges that selector written and writes it nothing.
//! - **A start drops the rest** (`start`) before it decides: its selectors
//!   were never written, Live still shows their chain, and the start decides
//!   them afresh (one batch, nothing written twice).
//! - **A hold and a target let go write DRY at once** (`hold`, `retire`):
//!   the rest is dropped first, so their own DRY batch takes those
//!   selectors through their last refs. A disconnect drops it (`offline`).
//! - **A write no longer owed** when its turn comes (its target's writes
//!   were dropped: a new macro, a walk's answer, a value at DRY heard, a
//!   later write answered) is dropped (`owed`) and the next one goes.
//! - The hub-loss fall back (the script's `safe_writes`) is the script's and
//!   stays immediate.
//!
//! Every step is an `autotune` record and a log line: one `spread` when it
//! starts, one `write` per selector, one `unspread` per drop.

use std::collections::VecDeque;

use super::super::{Router, RouterMsg};
use super::may_write;
use super::switch::Going;
use crate::rules::autotune::switch::{Flying, STOP_SPREAD_MS};
use crate::rules::autotune::{Cause, Step};

/// One DRY write of a spread still to come: what it sends and its write's
/// number (on its way since the spread started).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Pending {
    pub going: Going,
    pub write: u64,
}

/// A Live set's spread: the timer's generation that is its turn, and the
/// writes still to come in Live's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::router) struct Spread {
    pub(super) generation: u64,
    pub(super) cause: Cause,
    pub(super) queue: VecDeque<Pending>,
}

/// Whether `pending` is still owed: its target still holds the macro it
/// names (`found`: its `$ref`; none without a target or a macro), and its
/// write is still on its way unanswered (`writing`: the target's).
fn owed(found: Option<&str>, writing: &[Flying], pending: &Pending) -> bool {
    found == Some(pending.going.id.as_str())
        && writing
            .iter()
            .any(|f| f.write == pending.write && !f.answered)
}

impl Router {
    /// A stop's DRY writes of `set` (`going`, Live's order; its decision
    /// never hands over none): the first now, the rest one per
    /// [`STOP_SPREAD_MS`]; while a spread runs, all wait behind it (a lost
    /// transport within a stop's spread: never two writes in one turn).
    pub(super) fn autotune_spread(&mut self, set: &str, cause: Cause, mut going: Vec<Going>) {
        let running = self.autotune.spreads.contains_key(set);
        let rest = if running {
            going
        } else {
            let rest = going.split_off(going.len().min(1));
            self.autotune_select(set, cause, going, false);
            rest
        };
        if rest.is_empty() {
            return;
        }
        let labels: Vec<String> = rest.iter().map(|g| g.label.clone()).collect();
        let queue: VecDeque<Pending> = rest
            .into_iter()
            .map(|g| {
                let write = self.autotune_on_its_way(std::slice::from_ref(&g));
                Pending { going: g, write }
            })
            .collect();
        tracing::debug!(instance = %set, queued = queue.len(), running, "AUTOTUNE queues a stop's DRY writes");
        self.autotune_step(
            Step::Spread {
                instance: set.to_string(),
                targets: labels,
                ms: STOP_SPREAD_MS,
            },
            None,
        );
        match self.autotune.spreads.get_mut(set) {
            Some(spread) => spread.queue.extend(queue),
            None => {
                self.autotune.spreads.insert(
                    set.to_string(),
                    Spread {
                        generation: 0,
                        cause,
                        queue,
                    },
                );
                self.autotune_spread_later(set);
            }
        }
    }

    /// Arms `set`'s next turn [`STOP_SPREAD_MS`] from now.
    fn autotune_spread_later(&mut self, set: &str) {
        self.autotune.spread_turns += 1;
        let generation = self.autotune.spread_turns;
        if let Some(spread) = self.autotune.spreads.get_mut(set) {
            spread.generation = generation;
        }
        // Counted from now, not from the task's first poll.
        let at = tokio::time::Instant::now() + std::time::Duration::from_millis(STOP_SPREAD_MS);
        let (instance, tx) = (set.to_string(), self.io.tx.clone());
        tokio::spawn(async move {
            tokio::time::sleep_until(at).await;
            let _ = tx.send(RouterMsg::AutotuneSpread {
                instance,
                generation,
            });
        });
    }

    /// `set`'s turn `generation` came: its next write still owed goes (the
    /// ones no longer owed before it are dropped), the next turn is armed
    /// while any is left. A stale turn does nothing; a set whose writes may
    /// not go now (its markers unsettled) is held, which writes the rest DRY
    /// at once.
    pub(in crate::router) fn autotune_spread_turn(&mut self, set: &str, generation: u64) {
        let Some(spread) = self.autotune.spreads.get(set) else {
            return;
        };
        if spread.generation != generation {
            return;
        }
        let cause = spread.cause;
        let holding = self.autotune.held.contains(set);
        if !may_write(holding, self.reverb_settled(set)) {
            self.autotune_hold(set, "markers");
            self.autotune_unspread(set, None, "hold");
            self.autotune_tell();
            return;
        }
        let mut dropped = Vec::new();
        let mut next = None;
        while let Some(pending) = self
            .autotune
            .spreads
            .get_mut(set)
            .and_then(|s| s.queue.pop_front())
        {
            let target = self.autotune.targets.get(&pending.going.key);
            let found = target.and_then(|t| t.found.as_ref()).map(|f| f.id.as_str());
            let writing = target.map_or(&[][..], |t| t.writing.as_slice());
            if owed(found, writing, &pending) {
                next = Some(pending);
                break;
            }
            dropped.push(pending.going.label);
        }
        self.autotune_unspread_step(set, dropped, "owed");
        if let Some(pending) = next {
            self.autotune_send(
                set,
                cause,
                std::slice::from_ref(&pending.going),
                pending.write,
            );
        }
        if self
            .autotune
            .spreads
            .get(set)
            .is_some_and(|s| !s.queue.is_empty())
        {
            self.autotune_spread_later(set);
        } else {
            self.autotune.spreads.remove(set);
        }
    }

    /// Drops `set`'s spread writes still to come (`keys`: only those
    /// targets', else all), each taken off its target's writes on their
    /// way (never sent, so Live never takes it), recorded with `why`.
    pub(super) fn autotune_unspread(&mut self, set: &str, keys: Option<&[String]>, why: &str) {
        let Some(spread) = self.autotune.spreads.get_mut(set) else {
            return;
        };
        let (gone, kept): (VecDeque<Pending>, VecDeque<Pending>) = spread
            .queue
            .drain(..)
            .partition(|p| keys.is_none_or(|keys| keys.contains(&p.going.key)));
        if kept.is_empty() {
            self.autotune.spreads.remove(set);
        } else {
            spread.queue = kept;
        }
        let mut labels = Vec::new();
        for pending in gone {
            if let Some(target) = self.autotune.targets.get_mut(&pending.going.key) {
                target.writing.retain(|f| f.write != pending.write);
            }
            labels.push(pending.going.label);
        }
        self.autotune_unspread_step(set, labels, why);
    }

    /// The `unspread` record of `targets` (none: nothing).
    fn autotune_unspread_step(&self, set: &str, targets: Vec<String>, why: &str) {
        if targets.is_empty() {
            return;
        }
        self.autotune_step(
            Step::Unspread {
                instance: set.to_string(),
                targets,
                why: why.to_string(),
            },
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(id: &str, write: u64) -> Pending {
        Pending {
            going: Going {
                key: "band|live_set tracks 7".into(),
                id: id.into(),
                label: "#8".into(),
                on: false,
                value: 0,
            },
            write,
        }
    }

    #[test]
    fn a_spread_write_is_owed_only_while_on_its_way_to_the_macro_it_names() {
        let flying = |write, answered| Flying {
            write,
            on: false,
            answered,
        };
        let p = pending("p7", 3);
        assert!(owed(Some("p7"), &[flying(2, true), flying(3, false)], &p));
        assert!(!owed(None, &[flying(3, false)], &p), "no target or macro");
        assert!(!owed(Some("p9"), &[flying(3, false)], &p), "another macro");
        assert!(!owed(Some("p7"), &[], &p), "dropped");
        assert!(!owed(Some("p7"), &[flying(3, true)], &p), "answered");
        assert!(!owed(Some("p7"), &[flying(4, false)], &p), "another write");
    }
}
