//! A set of marked faders kept by the hub, one per per-strip switch of the
//! channel detail: the mute alarm's (#188, [`crate::mute_alarms`]) and the
//! mute guard's (#203, [`crate::mute_guards`]). Each document is
//! `{"<field>": {"<instance>": ["<label>", …]}}`; this is their one logic,
//! so the two switches cannot drift apart.
//!
//! A mark belongs to a fader, known by its [`StripRef`] (its Live set and
//! its label, as the sectors and the colours know it), so the same fader in
//! two layouts is marked once.

use std::collections::{BTreeMap, BTreeSet};

use crate::sectors::{Pooled, Refusal, StripRef};

/// Each Live set's marked faders by label.
pub type Marks = BTreeMap<String, BTreeSet<String>>;

/// A document of marked faders: it reads its marks and is built from marks.
pub trait StripMarks: Sized {
    /// Its marks.
    fn marks(&self) -> &Marks;

    /// The document of `marks`.
    fn of_marks(marks: Marks) -> Self;

    /// Whether `strip` is marked.
    fn is_on(&self, strip: &StripRef) -> bool {
        self.marks()
            .get(&strip.instance)
            .is_some_and(|labels| labels.contains(&strip.label))
    }

    /// The document with `strip` marked (`on`) or not (a Live set left with
    /// no mark is removed with it, so no mark anywhere is the empty
    /// document).
    fn with(&self, strip: &StripRef, on: bool) -> Self {
        let mut marks = self.marks().clone();
        let labels = marks.entry(strip.instance.clone()).or_default();
        if on {
            labels.insert(strip.label.clone());
        } else {
            labels.remove(&strip.label);
        }
        marks.retain(|_, labels| !labels.is_empty());
        Self::of_marks(marks)
    }

    /// How many faders are marked.
    fn count(&self) -> usize {
        self.marks().values().map(BTreeSet::len).sum()
    }

    /// Every fader it marks.
    fn strips(&self) -> Vec<StripRef> {
        self.marks()
            .iter()
            .flat_map(|(instance, labels)| {
                labels.iter().map(move |label| StripRef {
                    instance: instance.clone(),
                    label: label.clone(),
                })
            })
            .collect()
    }

    /// The document with only the Live set `instance`'s marks (a role that
    /// reaches that set alone).
    fn only(&self, instance: &str) -> Self {
        Self::of_marks(
            self.marks()
                .iter()
                .filter(|(set, _)| set.as_str() == instance)
                .map(|(set, labels)| (set.clone(), labels.clone()))
                .collect(),
        )
    }

    /// Every problem of the document (empty: valid): a Live set or a label
    /// that is empty or only spaces (no fader is known by it).
    fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        for (instance, labels) in self.marks() {
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
/// its mark waits, the strip shows nothing).
pub fn unknown<D: StripMarks>(doc: &D, pool: &[Pooled]) -> usize {
    doc.strips()
        .iter()
        .filter(|strip| !pool.iter().any(|p| p.id == **strip))
        .count()
}

/// A page's switch of `strip`'s mark to `on`: on only for a fader of the
/// pool ([`Refusal::NotInPool`]; off clears any, as COLOR's NONE does);
/// `Ok(None)` when it changes nothing, else the document after.
pub fn switch<D: StripMarks>(
    doc: &D,
    strip: &StripRef,
    on: bool,
    pool: &[Pooled],
) -> Result<Option<D>, Refusal> {
    if on && !pool.iter().any(|p| p.id == *strip) {
        return Err(Refusal::NotInPool);
    }
    if doc.is_on(strip) == on {
        return Ok(None);
    }
    Ok(Some(doc.with(strip, on)))
}
