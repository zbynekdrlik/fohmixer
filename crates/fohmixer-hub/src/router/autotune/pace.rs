//! The walks paced (#235, the design of 2026-11-17): a hub restart walked
//! every AUTOTUNE target of band Live in one tick (each track's devices,
//! the rack's parameters and chains, chain 0's devices, the plug-in's
//! parameters), and band Live lost 10 audio blocks; #239's inventory read
//! 7,235 values one at a time 60 ms apart and lost none. So the burst is
//! the cost, never the reads.
//!
//! - **One target's walk at a time per Live set,** in the order asked
//!   (Live's order at a connect, a set load, a track-list change): the next
//!   starts [`WALK_GAP_MS`] after the previous walk's last answer (its
//!   `AutotuneFound` or `AutotuneWalkFailed`), through
//!   [`RouterMsg::AutotuneWalkNext`] with the gap's generation (a stale one
//!   does nothing).
//! - **A walk's number is taken when it is asked** (`autotune_walk_now`),
//!   so an older walk's answer is void as before; only its start waits. A
//!   target asked again while queued keeps its place.
//! - **A disconnect drops the queue** (`autotune_offline`): the connect
//!   walks every target again from the first.
//! - A walk's own reads are unchanged, and so are the decisions: a target
//!   not yet found keeps its last state until its walk answers.

use std::collections::VecDeque;

use super::super::{Router, RouterMsg};

/// The gap between one walk's last answer and the next walk's first read.
pub const WALK_GAP_MS: u64 = 100;

/// A Live set's walks: the one on its way, the gap that follows it and the
/// targets still to walk, in the order asked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::router) struct Pace {
    /// The walk on its way: its target's key and walk number.
    pub(super) walking: Option<(String, u64)>,
    /// The gap's generation while it runs.
    pub(super) gap: Option<u64>,
    /// The targets still to walk.
    pub(super) queue: VecDeque<String>,
}

impl Pace {
    /// `key` asked to be walked: queued once, keeping its place.
    pub(super) fn ask(&mut self, key: &str) {
        if !self.queue.iter().any(|k| k == key) {
            self.queue.push_back(key.to_string());
        }
    }

    /// Whether a walk may start now: none on its way and no gap running.
    pub(super) fn free(&self) -> bool {
        self.walking.is_none() && self.gap.is_none()
    }

    /// Whether `key`'s walk `walk` is the one on its way; if so it ended.
    pub(super) fn ended(&mut self, key: &str, walk: u64) -> bool {
        let ended = self
            .walking
            .as_ref()
            .is_some_and(|(k, w)| k == key && *w == walk);
        if ended {
            self.walking = None;
        }
        ended
    }

    /// Whether gap `generation` is the one running; if so it is over.
    pub(super) fn gap_over(&mut self, generation: u64) -> bool {
        let over = self.gap == Some(generation);
        if over {
            self.gap = None;
        }
        over
    }
}

impl Router {
    /// `key` (of `set`) asked to be walked: queued, and the set's next walk
    /// started when it is free.
    pub(super) fn autotune_pace_ask(&mut self, set: &str, key: &str) {
        let pace = self.autotune.paces.entry(set.to_string()).or_default();
        pace.ask(key);
        tracing::debug!(instance = %set, queued = pace.queue.len(), "AUTOTUNE queues a walk");
        self.autotune_pace_next(set);
    }

    /// Starts `set`'s next walk while it is free: the first queued target
    /// still in the layout, online, with its newest walk number. A target
    /// gone or offline is passed over (it read nothing: no gap).
    fn autotune_pace_next(&mut self, set: &str) {
        let free = self.autotune.paces.get(set).is_some_and(Pace::free);
        if !free {
            return;
        }
        while let Some(key) = self
            .autotune
            .paces
            .get_mut(set)
            .and_then(|p| p.queue.pop_front())
        {
            if let Some(walk) = self.autotune_walk_start(&key) {
                tracing::debug!(instance = %set, key = %key, walk, "AUTOTUNE walks a target");
                if let Some(pace) = self.autotune.paces.get_mut(set) {
                    pace.walking = Some((key, walk));
                }
                return;
            }
        }
    }

    /// A walk of `key` ended (its last answer, `walk`): when it is its set's
    /// walk on its way, the gap starts.
    pub(super) fn autotune_pace_ended(&mut self, key: &str, walk: u64) {
        let Some(set) = self
            .autotune
            .paces
            .iter_mut()
            .find_map(|(set, pace)| pace.ended(key, walk).then(|| set.clone()))
        else {
            return;
        };
        self.autotune.pace_gaps += 1;
        let generation = self.autotune.pace_gaps;
        if let Some(pace) = self.autotune.paces.get_mut(&set) {
            pace.gap = Some(generation);
        }
        // Counted from now, not from the task's first poll.
        let at = tokio::time::Instant::now() + std::time::Duration::from_millis(WALK_GAP_MS);
        let tx = self.io.tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep_until(at).await;
            let _ = tx.send(RouterMsg::AutotuneWalkNext {
                instance: set,
                generation,
            });
        });
    }

    /// `set`'s gap `generation` is over: its next walk starts. A stale gap
    /// (a disconnect dropped the queue since) does nothing.
    pub(in crate::router) fn autotune_pace_gap(&mut self, set: &str, generation: u64) {
        let over = self
            .autotune
            .paces
            .get_mut(set)
            .is_some_and(|p| p.gap_over(generation));
        if over {
            self.autotune_pace_next(set);
        }
    }

    /// `set` went offline: its walks still to come are dropped.
    pub(super) fn autotune_pace_offline(&mut self, set: &str) {
        if let Some(pace) = self.autotune.paces.remove(set) {
            tracing::debug!(instance = %set, dropped = pace.queue.len(), "AUTOTUNE drops the walks still to come");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gap_is_100_ms() {
        assert_eq!(WALK_GAP_MS, 100);
    }

    #[test]
    fn a_pace_queues_once_ends_only_its_walk_and_only_its_gap() {
        let mut pace = Pace::default();
        assert!(pace.free());
        pace.ask("a");
        pace.ask("b");
        pace.ask("a");
        assert_eq!(pace.queue, ["a", "b"], "asked again keeps its place");
        pace.walking = Some(("a".into(), 3));
        assert!(!pace.free(), "a walk on its way");
        assert!(!pace.ended("a", 2), "an older walk");
        assert!(!pace.ended("b", 3), "another target");
        assert!(pace.ended("a", 3));
        assert_eq!(pace.walking, None);
        assert!(!pace.ended("a", 3), "once");
        assert!(pace.free());
        pace.gap = Some(7);
        assert!(!pace.free(), "the gap runs");
        assert!(!pace.gap_over(6), "a stale gap");
        assert_eq!(pace.gap, Some(7));
        assert!(pace.gap_over(7));
        assert!(pace.free());
        assert!(!pace.gap_over(7), "once");
    }
}
