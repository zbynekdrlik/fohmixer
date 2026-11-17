//! Pushes kept for a subscription batch still in flight (#243), pure.
//!
//! The script answers a batch whole, at the end of its last tick, while the
//! listeners it already registered push their changes on the ticks between.
//! Such a push names a Live key the table does not know yet; it is the newest
//! value, the result's is the one read at registration. So while a batch of
//! the instance is in flight a push for an unknown key is kept here, by its
//! frame's arrival number, and after each result the ones for the keys that
//! result made known are applied again in arrival order: those that arrived
//! after the batch was sent; one from before comes from an older listener and
//! is older than the result's read, so it goes (the table also drops, at
//! any arrival, those of a key whose older listener a `remove_listener` was
//! still ending). At most `KEPT_MAX` are kept; the oldest goes first.

use std::collections::VecDeque;

use super::super::LiveValue;

/// Pushes kept per instance at most.
pub const KEPT_MAX: usize = 1024;

/// What one `keep` had to drop: the oldest pushes, beyond `KEPT_MAX`.
#[derive(Debug, Clone, PartialEq)]
pub struct Overflow {
    /// Dropped by this `keep`.
    pub dropped: usize,
    /// Dropped since the table was made.
    pub total: u64,
}

impl Overflow {
    /// Its record's fields (the router's `kept_pushes`, warn class).
    pub fn fields(&self, instance: &str) -> serde_json::Value {
        serde_json::json!({
            "instance": instance,
            "dropped": self.dropped,
            "total": self.total,
            "max": KEPT_MAX,
        })
    }
}

/// One instance's kept pushes.
#[derive(Debug, Default)]
pub struct Kept {
    /// Each item with its frame's arrival number, in arrival order.
    pushes: VecDeque<(u64, LiveValue)>,
    /// Frames kept so far (never reset: a batch's mark stays comparable).
    frames: u64,
    dropped: u64,
}

impl Kept {
    /// The arrival number of the last frame kept: a batch sent now is newer
    /// than every push kept so far.
    pub fn mark(&self) -> u64 {
        self.frames
    }

    /// Keeps the items of one push frame; what it dropped to stay within
    /// `KEPT_MAX`, if anything.
    pub fn keep(&mut self, items: Vec<LiveValue>) -> Option<Overflow> {
        self.frames += 1;
        let mut dropped = 0;
        for item in items {
            if self.pushes.len() >= KEPT_MAX {
                self.pushes.pop_front();
                dropped += 1;
            }
            self.pushes.push_back((self.frames, item));
        }
        self.dropped += dropped as u64;
        (dropped > 0).then_some(Overflow {
            dropped,
            total: self.dropped,
        })
    }

    /// Takes the pushes for the Live keys `made_known` names: those that
    /// arrived after `mark`, grouped by their frames in arrival order; the
    /// older ones are dropped. The rest stay.
    pub fn take(&mut self, mark: u64, made_known: impl Fn(&str) -> bool) -> Vec<Vec<LiveValue>> {
        let mut frames: Vec<(u64, Vec<LiveValue>)> = Vec::new();
        let mut rest = VecDeque::new();
        for (frame, item) in std::mem::take(&mut self.pushes) {
            if !made_known(&item.key) {
                rest.push_back((frame, item));
            } else if frame > mark {
                match frames.last_mut() {
                    Some((last, items)) if *last == frame => items.push(item),
                    _ => frames.push((frame, vec![item])),
                }
            }
        }
        self.pushes = rest;
        frames.into_iter().map(|(_, items)| items).collect()
    }

    /// Forgets every kept push (no batch in flight any more, a disconnect).
    pub fn clear(&mut self) {
        self.pushes.clear();
    }

    /// Pushes kept now.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.pushes.len()
    }
}

#[cfg(test)]
#[path = "kept_tests.rs"]
mod tests;
