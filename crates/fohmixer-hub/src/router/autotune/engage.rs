//! A play start's selector-on writes wait (#190, the DECIDED section of the
//! ticket): band Live's audio callback stalled within 1 s of three play
//! starts on 12.11, and the hub switches every Auto-Tune selector on in the
//! play start's millisecond. The hub's `[autotune] engage_delay_ms` (0, the
//! default: no wait, today's writes byte for byte) separates the two.
//!
//! - **A play start** (`autotune_transport` with Live playing) opens the
//!   set's wait for `engage_delay_ms` (one `engage` record, `waits`):
//!   while it runs, every selector the decision turns on is left unwritten
//!   ([`now_and_later`]); its decision is kept, so the pages and the status
//!   see the rule's choice and Live's DRY.
//! - **Its end** ([`RouterMsg::AutotuneEngage`] with the wait's generation;
//!   a stale one does nothing) records `ends` and decides the set again with
//!   the start's cause (`transport`): the delayed writes go then, each with
//!   its own `write` record.
//! - **A stop within it** (or a lost transport, `stopped`) and a disconnect
//!   (`offline`) end it: nothing that waited is written.
//! - **Immediate as before:** every DRY write (a stop's spread, AUT off, a
//!   singer off, a hold), the key writes, AUT and singer taps; only the
//!   writes to the chain wait.

use std::collections::BTreeMap;

use super::super::{Router, RouterMsg};
use super::switch::Going;
use crate::rules::autotune::{Cause, EngageAct, Step};

/// The router's waits of play starts: the setting and each Live set's
/// running wait, by its generation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::router) struct Engage {
    /// `[autotune] engage_delay_ms`: 0 waits for nothing.
    pub delay_ms: u64,
    /// Each Live set whose wait runs: its generation.
    pub waiting: BTreeMap<String, u64>,
    /// The waits' own counter (a stale end does nothing).
    pub turns: u64,
}

/// Whether a play start waits: a delay is set.
pub(super) fn waits(delay_ms: u64) -> bool {
    delay_ms > 0
}

/// `going` split into the writes that go now and the ones a running wait
/// (`waiting`) holds back: only a write to the chain waits.
pub(super) fn now_and_later(going: Vec<Going>, waiting: bool) -> (Vec<Going>, Vec<Going>) {
    going.into_iter().partition(|g| !(waiting && g.on))
}

impl Router {
    /// The hub's `[autotune] engage_delay_ms` (#190).
    pub fn with_engage_delay(mut self, delay_ms: u64) -> Self {
        self.autotune.engage.delay_ms = delay_ms;
        self
    }

    /// Live started playing in `set`: with a delay set, its wait opens (a
    /// wait still running starts over).
    pub(super) fn autotune_engage_start(&mut self, set: &str) {
        let ms = self.autotune.engage.delay_ms;
        if !waits(ms) {
            return;
        }
        self.autotune.engage.turns += 1;
        let generation = self.autotune.engage.turns;
        self.autotune
            .engage
            .waiting
            .insert(set.to_string(), generation);
        self.autotune_engage_step(set, EngageAct::Waits);
        // Counted from now, not from the task's first poll.
        let at = tokio::time::Instant::now() + std::time::Duration::from_millis(ms);
        let (instance, tx) = (set.to_string(), self.io.tx.clone());
        tokio::spawn(async move {
            tokio::time::sleep_until(at).await;
            let _ = tx.send(RouterMsg::AutotuneEngage {
                instance,
                generation,
            });
        });
    }

    /// `set`'s wait ends early (`act`: a stop, offline): nothing that
    /// waited is written. No wait running: nothing.
    pub(super) fn autotune_engage_cancel(&mut self, set: &str, act: EngageAct) {
        if self.autotune.engage.waiting.remove(set).is_some() {
            self.autotune_engage_step(set, act);
        }
    }

    /// `set`'s wait `generation` ran out: the set is decided again, so the
    /// selectors that waited go. A stale end does nothing.
    pub(in crate::router) fn autotune_engage_end(&mut self, set: &str, generation: u64) {
        if self.autotune.engage.waiting.get(set) != Some(&generation) {
            return;
        }
        self.autotune.engage.waiting.remove(set);
        self.autotune_engage_step(set, EngageAct::Ends);
        self.autotune_decide(set, Cause::Transport);
    }

    /// `going` of `set` less the writes its running wait holds back (their
    /// decision stays, so the wait's end writes them; the `engage` records
    /// and the end's `write` record trace them, no line per decision).
    pub(super) fn autotune_engage_filter(&self, set: &str, going: Vec<Going>) -> Vec<Going> {
        let waiting = self.autotune.engage.waiting.contains_key(set);
        now_and_later(going, waiting).0
    }

    /// The `engage` record of `set`'s wait.
    fn autotune_engage_step(&self, set: &str, act: EngageAct) {
        self.autotune_step(
            Step::Engage {
                instance: set.to_string(),
                ms: self.autotune.engage.delay_ms,
                act,
            },
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn going(label: &str, on: bool) -> Going {
        Going {
            key: format!("band|{label}"),
            id: label.into(),
            label: label.into(),
            on,
            value: if on { 127 } else { 0 },
        }
    }

    #[test]
    fn only_a_write_to_the_chain_waits_and_only_while_a_wait_runs() {
        assert!(!waits(0), "0: no wait, today's writes");
        assert!(waits(1));
        assert!(waits(1000));
        let all = vec![going("a", true), going("b", false), going("c", true)];
        assert_eq!(
            now_and_later(all.clone(), true),
            (
                vec![going("b", false)],
                vec![going("a", true), going("c", true)]
            )
        );
        assert_eq!(now_and_later(all.clone(), false), (all, Vec::new()));
    }
}
