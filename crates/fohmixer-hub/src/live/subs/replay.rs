//! The table's glue of the pushes around a batch in flight: those kept for
//! a Live key it does not know yet (#243) and those heard for one it knows
//! (#244), on `kept::Kept`'s pure decisions.

use std::collections::BTreeSet;

use super::kept::{KEPT_MAX, Overflow};
use super::{Batch, Cached, Subs, Tag};

impl Subs {
    /// The pushes kept for `instance` now (#243).
    #[cfg(test)]
    pub(super) fn kept(&self, instance: &str) -> usize {
        self.instances.get(instance).map_or(0, |i| i.kept.len())
    }

    /// The Live keys a `remove_listener` of `instance` still in flight ends.
    pub(super) fn releasing(&self, instance: &str) -> BTreeSet<String> {
        self.inflight
            .values()
            .filter(|b| b.instance == instance)
            .flat_map(|b| &b.tags)
            .filter_map(|t| match t {
                Tag::Release { live } => Some(live.clone()),
                Tag::Resolve { .. } => None,
            })
            .collect()
    }

    /// Whether an `add_listener` batch of `instance` is in flight.
    pub(super) fn resolving(&self, instance: &str) -> bool {
        self.inflight.values().any(|b| {
            b.instance == instance && b.tags.iter().any(|t| matches!(t, Tag::Resolve { .. }))
        })
    }

    /// After a result (#243): the kept pushes for the Live keys it made known
    /// are applied again, in arrival order (those from before the batch was
    /// sent go, and so do those of a key whose older listener was being
    /// removed meanwhile); once no batch is in flight, the rest go.
    pub(super) fn replay_kept(
        &mut self,
        instance: &str,
        batch: &Batch,
        made_known: &BTreeSet<String>,
    ) {
        let Some(inst) = self.instances.get_mut(instance) else {
            return;
        };
        // A released key's pushes are dropped whatever their arrival.
        inst.kept.take(u64::MAX, |key| {
            batch.released.contains(key) && made_known.contains(key)
        });
        let frames = inst.kept.take(batch.mark, |key| made_known.contains(key));
        for frame in frames {
            self.apply_values(instance, &frame);
        }
        if !self.resolving(instance)
            && let Some(inst) = self.instances.get_mut(instance)
        {
            inst.kept.clear();
        }
    }

    /// Forgets the push heard for `live` (#244): no entry holds it any more.
    pub(super) fn forget_heard(&mut self, instance: &str, live: &str) {
        if let Some(inst) = self.instances.get_mut(instance) {
            inst.kept.forget(live);
        }
    }

    /// The state a key joining the Live key `live` takes at the result of a
    /// batch sent at `mark` (#244), if not the result's: the push heard for
    /// it after the send (`kept::newer_push`), only when another entry holds
    /// that Live key (a sibling).
    pub(super) fn joined_push(
        &self,
        instance: &str,
        key: &str,
        live: &str,
        mark: u64,
        wants_display: bool,
    ) -> Option<Cached> {
        let joins = self
            .by_live
            .get(&(instance.to_string(), live.to_string()))
            .is_some_and(|keys| keys.iter().any(|k| k != key));
        let push = self
            .instances
            .get(instance)?
            .kept
            .heard_after(live, mark, wants_display)
            .filter(|_| joins)?;
        tracing::debug!(
            instance,
            key,
            live,
            "a new subscription takes its Live key's push that came after its batch was sent"
        );
        Some(Cached::Value {
            value: push.value.clone(),
            display: push.display.clone(),
        })
    }
}

/// The warn line of pushes dropped beyond `KEPT_MAX` (#243).
pub(super) fn warn_overflow(instance: &str, overflow: &Overflow) {
    tracing::warn!(
        instance,
        dropped = overflow.dropped,
        total = overflow.total,
        max = KEPT_MAX,
        "a subscription batch is still in flight and more Live pushes came than the hub keeps for it: the oldest are dropped"
    );
}
