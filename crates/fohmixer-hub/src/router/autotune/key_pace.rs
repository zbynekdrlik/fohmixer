//! The key writes paced by the transport (#190, the design of 2026-11-20;
//! dasio#22: a batch of every Auto-Tune's Key and Scale while the set stood
//! and the playhead jumped from song to song stalled band Live). The pure
//! decision is `rules::autotune::keys::key_pace`; the writes themselves are
//! `key_writes.rs`'.
//!
//! - **Playing:** one batch at once, as before (the singers stay in step,
//!   `LATE_BOUND_MS` holds); a key still to come is dropped.
//! - **Stopped** (or not known to play): the write waits until the playhead
//!   stayed `KEY_SETTLE_MS` in the song (a newer song starts the wait over,
//!   so a run of jumps writes only the song it stops in), then goes one
//!   target per `STOP_SPREAD_MS`, each its own call and read-back, Live's
//!   bar last; a settle that ends with Live playing (a play start heard as
//!   a first value) is one batch.
//! - **A play start:** a key still to come, or one the start's song needs,
//!   goes spread from [`KEY_AFTER_START_MS`] after it.
//! - A key still to come that is no longer the key in force, a held set
//!   (the immediate held path: owed, the release writes it `found`) and a
//!   disconnect drop it; `found` and `retry` stay immediate. Records
//!   `key_settle` (`waits`, `restarted`, `ends`, `after_start`, `dropped`,
//!   `offline`) and one `key_write` per target.

use std::collections::VecDeque;
use std::time::Duration;

use fohmixer_proto::songmap::Key;

use super::super::{Router, RouterMsg};
use super::key_writes::{BarWrite, elapsed_ms};
use super::may_write;
use crate::rules::autotune::Step;
use crate::rules::autotune::keys::{
    KEY_AFTER_START_MS, KeyPace, KeySettleAct, KeyWhy, key_pace, settle_left,
};
use crate::rules::autotune::switch::STOP_SPREAD_MS;

/// One turn of a paced key write's spread (#190): a target's write, or
/// Live's bar's, last.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Turn {
    Target(String),
    Bar,
}

/// Where a paced key write stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Stage {
    /// Waiting for the playhead to stay in the song it came into at `since`.
    Settle { since: tokio::time::Instant },
    /// Spread: the turns still to come (none: every target of the set in
    /// Live's order, then the bar, taken at the first turn).
    Spread(Option<VecDeque<Turn>>),
}

/// A Live set's key write paced by its transport (#190): its turn's
/// generation, the key writes' counter when it began (`base`: a target
/// written since with its key, a `found` write after a reconnect, is not
/// written again), the song and key it writes, why, and where it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::router) struct KeyPacing {
    pub(super) generation: u64,
    pub(super) base: u64,
    pub(super) song: Option<u16>,
    pub(super) key: Key,
    pub(super) why: KeyWhy,
    pub(super) stage: Stage,
}

/// Whether a target whose newest key write is `newest`, wanting `wanted`,
/// was written `key` after the key writes' counter stood at `base`.
pub(super) fn written_since(newest: u64, base: u64, wanted: Option<Key>, key: Key) -> bool {
    newest > base && wanted == Some(key)
}

/// A wait of `ms` (never below 0) as a timer's duration, to the µs.
pub(super) fn wait_of(ms: f64) -> Duration {
    Duration::from_micros((ms.max(0.0) * 1000.0).round() as u64)
}

impl Router {
    /// A song start or a key change of `set` (song `song`'s `key` in
    /// force, `keys` its targets in Live's order) written at its
    /// transport's pace ([`key_pace`]; #190, dasio#22): playing, one batch
    /// now (a key still to come is dropped: the batch writes every
    /// target); stopped, it waits for the playhead to stay
    /// `KEY_SETTLE_MS` in the song (a newer song starts the wait over);
    /// within [`KEY_AFTER_START_MS`] of a play start, spread from then. A
    /// set whose writes may not go now takes the held path at once (every
    /// target and the bar owe the key; the release writes it `found`).
    pub(super) fn autotune_key_paced(
        &mut self,
        set: &str,
        keys: &[String],
        song: Option<u16>,
        key: Key,
        why: KeyWhy,
    ) {
        let holding = self.autotune.held.contains(set);
        let (stage, wait, act) = match self.autotune_key_pace_now(set) {
            Some(KeyPace::Settle { until }) if may_write(holding, self.reverb_settled(set)) => {
                let act = if self.autotune.key_paces.contains_key(set) {
                    KeySettleAct::Restarted
                } else {
                    KeySettleAct::Waits
                };
                let since = tokio::time::Instant::now();
                (Stage::Settle { since }, until, act)
            }
            Some(KeyPace::Spread { from }) if may_write(holding, self.reverb_settled(set)) => {
                (Stage::Spread(None), from, KeySettleAct::AfterStart)
            }
            _ => {
                self.autotune_key_unpace(set, KeySettleAct::Dropped);
                self.autotune_key_write(set, keys, key, why);
                return;
            }
        };
        let pacing = KeyPacing {
            generation: 0,
            base: self.autotune.key_writes,
            song,
            key,
            why,
            stage,
        };
        self.autotune_key_arm(set, pacing, wait, Some(act));
    }

    /// `set`'s pace now ([`key_pace`]): its transport and the time since
    /// its play start; none without a position.
    fn autotune_key_pace_now(&self, set: &str) -> Option<KeyPace> {
        self.autotune.positions.get(set).map(|p| {
            let since_start = p.started.map(|at| elapsed_ms(at.elapsed()));
            key_pace(p.playing == Some(true), since_start)
        })
    }

    /// A play start of `set` (`prev`: its transport before; a start is one
    /// heard after a stop, as the engage's): its time is kept for
    /// [`key_pace`], and a key still to come (waiting, or spreading) goes
    /// [`KEY_AFTER_START_MS`] after it, so nothing lands on the start.
    pub(super) fn autotune_key_start(&mut self, set: &str, prev: Option<bool>) {
        if prev != Some(false) {
            return;
        }
        if let Some(position) = self.autotune.positions.get_mut(set) {
            position.started = Some(tokio::time::Instant::now());
        }
        let Some(mut pacing) = self.autotune.key_paces.remove(set) else {
            return;
        };
        if let Stage::Settle { .. } = pacing.stage {
            pacing.stage = Stage::Spread(None);
        }
        self.autotune_key_arm(
            set,
            pacing,
            KEY_AFTER_START_MS,
            Some(KeySettleAct::AfterStart),
        );
    }

    /// Puts `pacing` in `set`'s place and arms its turn `wait` ms from now
    /// (a turn armed before it is stale then), recorded with `act`.
    fn autotune_key_arm(
        &mut self,
        set: &str,
        mut pacing: KeyPacing,
        wait: f64,
        act: Option<KeySettleAct>,
    ) {
        self.autotune.key_turns += 1;
        let generation = self.autotune.key_turns;
        pacing.generation = generation;
        if let Some(act) = act {
            self.autotune_key_settle_step(set, &pacing, act);
        }
        self.autotune.key_paces.insert(set.to_string(), pacing);
        self.autotune_turn_in(
            wait_of(wait),
            RouterMsg::AutotuneKeyTurn {
                instance: set.to_string(),
                generation,
            },
        );
    }

    /// `set`'s paced key write's turn `generation` came: a settle ends once
    /// the playhead stayed (else it waits the rest), then the spread's next
    /// turn goes (one with nothing to send passes it on at once) and the
    /// next one is armed `STOP_SPREAD_MS` on while any is left. A set whose
    /// writes may not go now takes the held path with every target still
    /// to come. A stale turn does nothing.
    pub(in crate::router) fn autotune_key_turn(&mut self, set: &str, generation: u64) {
        if self.autotune.key_paces.get(set).map(|p| p.generation) != Some(generation) {
            return;
        }
        let Some(mut pacing) = self.autotune.key_paces.remove(set) else {
            return;
        };
        let (queue, settled) = match &pacing.stage {
            Stage::Settle { since } => {
                if let Some(left) = settle_left(elapsed_ms(since.elapsed())) {
                    self.autotune_key_arm(set, pacing, left, None);
                    return;
                }
                (None, true)
            }
            Stage::Spread(queue) => (queue.clone(), false),
        };
        let mut queue = queue.unwrap_or_else(|| {
            let keys = self.autotune.keys_in_order(set).into_iter();
            keys.map(Turn::Target).chain([Turn::Bar]).collect()
        });
        let (key, why) = (pacing.key, pacing.why);
        let holding = self.autotune.held.contains(set);
        // A settle that ends with Live playing (a start heard as a first
        // value, after a lost transport listener): one batch, as playing.
        let playing = settled && matches!(self.autotune_key_pace_now(set), Some(KeyPace::Batch));
        if playing || !may_write(holding, self.reverb_settled(set)) {
            let keys: Vec<String> = queue
                .into_iter()
                .filter_map(|turn| match turn {
                    Turn::Target(k) => Some(k),
                    Turn::Bar => None,
                })
                .filter(|k| !self.autotune_key_written_since(k, pacing.base, key))
                .collect();
            self.autotune_key_write(set, &keys, key, why);
            return;
        }
        if settled {
            self.autotune_key_settle_step(set, &pacing, KeySettleAct::Ends);
        }
        while let Some(turn) = queue.pop_front() {
            let sent = match turn {
                Turn::Target(k) if self.autotune_key_written_since(&k, pacing.base, key) => false,
                Turn::Target(k) => self.autotune_key_send(set, &[k], key, why, BarWrite::Later),
                Turn::Bar => self.autotune_key_send(set, &[], key, why, BarWrite::Own),
            };
            if sent {
                break;
            }
        }
        if !queue.is_empty() {
            pacing.stage = Stage::Spread(Some(queue));
            self.autotune_key_arm(set, pacing, STOP_SPREAD_MS as f64, None);
        }
    }

    /// Whether target `k` was written `key` since the key writes' counter
    /// stood at `base` (a `found` write while its set's key waited).
    fn autotune_key_written_since(&self, k: &str, base: u64, key: Key) -> bool {
        self.autotune
            .targets
            .get(k)
            .is_some_and(|t| written_since(t.key_write, base, t.keyt.wanted(), key))
    }

    /// Drops `set`'s key still to come, recorded with `act` (none: nothing).
    pub(super) fn autotune_key_unpace(&mut self, set: &str, act: KeySettleAct) {
        if let Some(pacing) = self.autotune.key_paces.remove(set) {
            self.autotune_key_settle_step(set, &pacing, act);
        }
    }

    /// The `key_settle` record of `set`'s `pacing`.
    fn autotune_key_settle_step(&self, set: &str, pacing: &KeyPacing, act: KeySettleAct) {
        self.autotune_step(
            Step::KeySettle {
                instance: set.to_string(),
                song: pacing.song,
                key: pacing.key.name(),
                act,
            },
            None,
        );
    }
}

/// The router tests' hands on the pace (#190) for a test on tokio's real
/// clock, which cannot wait out a settle (the spreads' `next_turn`'s
/// kind).
#[cfg(test)]
impl Router {
    /// `set`'s paced key write's next turn handed over at once, a settle's
    /// as if the playhead had stayed `KEY_SETTLE_MS`: whether one was
    /// there.
    pub(super) fn autotune_key_now(&mut self, set: &str) -> bool {
        let Some(pacing) = self.autotune.key_paces.get_mut(set) else {
            return false;
        };
        if let Stage::Settle { since } = &mut pacing.stage {
            *since -= Duration::from_millis(500);
        }
        let generation = pacing.generation;
        self.autotune_key_turn(set, generation);
        true
    }

    /// `set`'s play start `KEY_AFTER_START_MS` back: Live has played on,
    /// so a key change is one batch again.
    pub(super) fn autotune_played_on(&mut self, set: &str) {
        if let Some(position) = self.autotune.positions.get_mut(set) {
            position.started = position.started.map(|at| at - Duration::from_millis(300));
        }
    }
}
