//! The mute alarm (#188; the owner's approved mockup
//! `docs/mockups/mute-alert-v1.html`): a fader whose mute can stop the show
//! (the band's feed at an event) is marked in its channel detail's MUTE
//! ALARM switch; while such a fader is muted its name button blinks red and
//! a red frame blinks round the strip, so a mute that is not the normal
//! state cannot look calm. The hub keeps the marks (`mute_alarms.json`),
//! every page shows them; nothing is written to Live.
//!
//! A mark belongs to a fader, known by its [`StripRef`] (its Live set and
//! its label, as the sectors and the colours know it), so the same fader in
//! two layouts is marked once.
//!
//! The file is `{"mute_alarms": {"<instance>": ["<label>", …]}}`; an
//! unknown field is an error, not ignored (the file can be edited by hand,
//! as the colours' is).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::sectors::{Pooled, Refusal, StripRef};

/// The document, `mute_alarms.json`: each Live set's marked faders by
/// label. A fader it does not name has no alarm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MuteAlarmsDoc {
    pub mute_alarms: BTreeMap<String, BTreeSet<String>>,
}

impl MuteAlarmsDoc {
    /// The empty document: no fader marked.
    pub fn empty() -> Self {
        Self {
            mute_alarms: BTreeMap::new(),
        }
    }

    /// Whether `strip` is marked.
    pub fn is_on(&self, strip: &StripRef) -> bool {
        self.mute_alarms
            .get(&strip.instance)
            .is_some_and(|labels| labels.contains(&strip.label))
    }

    /// The document with `strip` marked (`on`) or not (a Live set left with
    /// no mark is removed with it, so no mark anywhere is the empty
    /// document).
    pub fn with(&self, strip: &StripRef, on: bool) -> Self {
        let mut mute_alarms = self.mute_alarms.clone();
        let labels = mute_alarms.entry(strip.instance.clone()).or_default();
        if on {
            labels.insert(strip.label.clone());
        } else {
            labels.remove(&strip.label);
        }
        mute_alarms.retain(|_, labels| !labels.is_empty());
        Self { mute_alarms }
    }

    /// How many faders are marked.
    pub fn count(&self) -> usize {
        self.mute_alarms.values().map(BTreeSet::len).sum()
    }

    /// Every fader it marks.
    pub fn strips(&self) -> impl Iterator<Item = StripRef> + '_ {
        self.mute_alarms.iter().flat_map(|(instance, labels)| {
            labels.iter().map(move |label| StripRef {
                instance: instance.clone(),
                label: label.clone(),
            })
        })
    }

    /// Every problem of the document (empty: valid): a Live set or a label
    /// that is empty or only spaces (no fader is known by it).
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        for (instance, labels) in &self.mute_alarms {
            if instance.trim().is_empty() {
                errors.push("a Live set's name is empty".to_string());
            }
            for label in labels {
                if label.trim().is_empty() {
                    errors.push(format!("a fader of {instance:?} has an empty label"));
                }
            }
        }
        errors
    }
}

/// How many marked faders no pool fader carries (a track renamed in Live:
/// its mark waits, the strip shows no alarm).
pub fn unknown(doc: &MuteAlarmsDoc, pool: &[Pooled]) -> usize {
    doc.strips()
        .filter(|strip| !pool.iter().any(|p| p.id == *strip))
        .count()
}

/// A page's switch of `strip`'s alarm to `on`: on only for a fader of the
/// pool ([`Refusal::NotInPool`]; off clears any, as COLOR's NONE does);
/// `Ok(None)` when it changes nothing, else the document after.
pub fn set_mute_alarm(
    doc: &MuteAlarmsDoc,
    strip: &StripRef,
    on: bool,
    pool: &[Pooled],
) -> Result<Option<MuteAlarmsDoc>, Refusal> {
    if on && !pool.iter().any(|p| p.id == *strip) {
        return Err(Refusal::NotInPool);
    }
    if doc.is_on(strip) == on {
        return Ok(None);
    }
    Ok(Some(doc.with(strip, on)))
}

#[cfg(test)]
mod tests;
