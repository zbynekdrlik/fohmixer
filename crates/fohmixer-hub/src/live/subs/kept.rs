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
//!
//! A push for a Live key the table already knows is applied to its entries at
//! once, but a NEW hub key joining that Live key in the batch (#244: a
//! `…|true` beside a `…|false`) is not linked yet and misses it. So for each
//! known key the last push of the frames numbered here is heard too (its
//! value and display, one per key), and at the result the joining key takes
//! it instead of the result's value when it arrived after the batch was sent
//! (`newer_push`): one Live key, one value across its hub keys.

use std::collections::{HashMap, VecDeque};

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
    /// The last push of each Live key the table knew, with its frame's
    /// arrival number (#244).
    heard: HashMap<String, (u64, LiveValue)>,
}

/// What a new hub key joining a Live key another entry already holds takes
/// at its batch's result (#244): the last push heard for that Live key when
/// it arrived after the batch was sent (`mark`) and carries what the key
/// shows; else none, and the key takes the result's value. A push from
/// before the send may be older than the result's read. So is one without a
/// display for a key that wants it: the script reads the display from the
/// first `add_listener` that asks for it on (`Subscriptions.add`), so such a
/// push was read before this batch's registration.
pub fn newer_push(
    heard: Option<&(u64, LiveValue)>,
    mark: u64,
    wants_display: bool,
) -> Option<&LiveValue> {
    heard
        .filter(|(frame, item)| *frame > mark && (item.display.is_some() || !wants_display))
        .map(|(_, item)| item)
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

    /// Numbers one push frame while a batch is in flight: its items for Live
    /// keys the table does not know (`known` false) are kept (`keep`), and
    /// for each key it knows the frame's last item without an error is heard
    /// (#244). What it dropped beyond `KEPT_MAX`, if anything.
    pub fn frame(&mut self, items: &[LiveValue], known: impl Fn(&str) -> bool) -> Option<Overflow> {
        let unknown = items.iter().filter(|i| !known(&i.key)).cloned().collect();
        let overflow = self.keep(unknown);
        for item in items.iter().filter(|i| i.error.is_none() && known(&i.key)) {
            self.heard
                .insert(item.key.clone(), (self.frames, item.clone()));
        }
        overflow
    }

    /// The push a key joining the Live key `live` takes at the result of a
    /// batch sent at `mark` (`newer_push`), if any.
    pub fn heard_after(&self, live: &str, mark: u64, wants_display: bool) -> Option<&LiveValue> {
        newer_push(self.heard.get(live), mark, wants_display)
    }

    /// Forgets what was heard for `live`: no entry holds it any more, or its
    /// listener ended (an error pushed).
    pub fn forget(&mut self, live: &str) {
        self.heard.remove(live);
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

    /// Forgets every kept and heard push (no batch in flight any more, a
    /// disconnect).
    pub fn clear(&mut self) {
        self.pushes.clear();
        self.heard.clear();
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
