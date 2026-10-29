//! The strips' colours (#164; the owner's approved mockup
//! `docs/mockups/strip-color-v1.html`): a strip's colour is the engineer's
//! choice in its channel detail, no longer the Ableton track's colour. The
//! hub keeps the document (`colors.json`), every page shows it: the strip's
//! 3 px edge at the left of its name button and the detail's name chip.
//!
//! A colour belongs to a fader, known by its [`StripRef`] (its Live set and
//! its label, as the sectors know it), so the same fader in two layouts has
//! one colour; a fader with none shows the neutral grey NONE. The palette
//! is fixed: twelve calm colours ([`ColorKey`]), defined here once for the
//! hub and the UI.
//!
//! The file is `{"colors": {"<instance>": {"<label>": "<key>"}}}`; an
//! unknown key or field is an error, not ignored (the file is edited by
//! hand, as the sectors' is).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::sectors::{Pooled, Refusal, StripRef};

/// A colour of the palette (the mockup's twelve, in its order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorKey {
    Red,
    Orange,
    Amber,
    Lime,
    Green,
    Teal,
    Cyan,
    Blue,
    Violet,
    Purple,
    Pink,
    Sand,
}

impl ColorKey {
    /// The palette in the detail's order.
    pub const ALL: [ColorKey; 12] = [
        ColorKey::Red,
        ColorKey::Orange,
        ColorKey::Amber,
        ColorKey::Lime,
        ColorKey::Green,
        ColorKey::Teal,
        ColorKey::Cyan,
        ColorKey::Blue,
        ColorKey::Violet,
        ColorKey::Purple,
        ColorKey::Pink,
        ColorKey::Sand,
    ];

    /// Its key in the document and the protocol: `red`, …, `sand`.
    pub fn as_str(self) -> &'static str {
        match self {
            ColorKey::Red => "red",
            ColorKey::Orange => "orange",
            ColorKey::Amber => "amber",
            ColorKey::Lime => "lime",
            ColorKey::Green => "green",
            ColorKey::Teal => "teal",
            ColorKey::Cyan => "cyan",
            ColorKey::Blue => "blue",
            ColorKey::Violet => "violet",
            ColorKey::Purple => "purple",
            ColorKey::Pink => "pink",
            ColorKey::Sand => "sand",
        }
    }

    /// Its name on the tablet: `RED`, …, `SAND`.
    pub fn name(self) -> &'static str {
        match self {
            ColorKey::Red => "RED",
            ColorKey::Orange => "ORANGE",
            ColorKey::Amber => "AMBER",
            ColorKey::Lime => "LIME",
            ColorKey::Green => "GREEN",
            ColorKey::Teal => "TEAL",
            ColorKey::Cyan => "CYAN",
            ColorKey::Blue => "BLUE",
            ColorKey::Violet => "VIOLET",
            ColorKey::Purple => "PURPLE",
            ColorKey::Pink => "PINK",
            ColorKey::Sand => "SAND",
        }
    }

    /// Its colour, `0xRRGGBB` (the mockup's values).
    pub fn rgb(self) -> u32 {
        match self {
            ColorKey::Red => 0xd4676d,
            ColorKey::Orange => 0xde8a4e,
            ColorKey::Amber => 0xd6b24e,
            ColorKey::Lime => 0xa9c45c,
            ColorKey::Green => 0x5db87c,
            ColorKey::Teal => 0x4bb3a6,
            ColorKey::Cyan => 0x5aaed8,
            ColorKey::Blue => 0x6d8fe2,
            ColorKey::Violet => 0x9b82e4,
            ColorKey::Purple => 0xc47acb,
            ColorKey::Pink => 0xe27fa8,
            ColorKey::Sand => 0xb59a7c,
        }
    }
}

/// The name of no colour on the tablet.
pub const NONE_NAME: &str = "NONE";
/// The colour of a strip with none chosen: a calm grey, the dim texts'
/// tone, so a grey edge reads as "no colour".
pub const NONE_RGB: u32 = 0x73738a;

/// The colour a strip shows: its key's, else NONE's.
pub fn shown_rgb(key: Option<ColorKey>) -> u32 {
    key.map_or(NONE_RGB, ColorKey::rgb)
}

/// The name a strip's colour shows: its key's, else NONE.
pub fn shown_name(key: Option<ColorKey>) -> &'static str {
    key.map_or(NONE_NAME, ColorKey::name)
}

/// A colour as CSS: `#rrggbb`.
pub fn css(rgb: u32) -> String {
    format!("#{rgb:06x}")
}

/// The document, `colors.json`: each Live set's faders by label, each with
/// its colour. A fader it does not name is NONE.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorsDoc {
    pub colors: BTreeMap<String, BTreeMap<String, ColorKey>>,
}

impl ColorsDoc {
    /// The empty document: every fader NONE.
    pub fn empty() -> Self {
        Self {
            colors: BTreeMap::new(),
        }
    }

    /// The colour of `strip`; none: NONE.
    pub fn of(&self, strip: &StripRef) -> Option<ColorKey> {
        self.colors
            .get(&strip.instance)
            .and_then(|labels| labels.get(&strip.label))
            .copied()
    }

    /// The document with `strip`'s colour set (none: removed; a Live set
    /// left with no colour is removed with it, so NONE everywhere is the
    /// empty document).
    pub fn with(&self, strip: &StripRef, color: Option<ColorKey>) -> Self {
        let mut colors = self.colors.clone();
        let labels = colors.entry(strip.instance.clone()).or_default();
        match color {
            Some(key) => {
                labels.insert(strip.label.clone(), key);
            }
            None => {
                labels.remove(&strip.label);
            }
        }
        colors.retain(|_, labels| !labels.is_empty());
        Self { colors }
    }

    /// How many faders have a colour.
    pub fn count(&self) -> usize {
        self.colors.values().map(BTreeMap::len).sum()
    }

    /// Every fader it names, with its colour.
    pub fn strips(&self) -> impl Iterator<Item = (StripRef, ColorKey)> + '_ {
        self.colors.iter().flat_map(|(instance, labels)| {
            labels.iter().map(move |(label, key)| {
                (
                    StripRef {
                        instance: instance.clone(),
                        label: label.clone(),
                    },
                    *key,
                )
            })
        })
    }

    /// Every problem of the document (empty: valid): a Live set or a label
    /// that is empty or only spaces (no fader is known by it).
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        for (instance, labels) in &self.colors {
            if instance.trim().is_empty() {
                errors.push("a Live set's name is empty".to_string());
            }
            for label in labels.keys() {
                if label.trim().is_empty() {
                    errors.push(format!("a fader of {instance:?} has an empty label"));
                }
            }
        }
        errors
    }
}

/// How many faders with a colour no pool fader carries (a track renamed in
/// Live: its colour waits, the strip shows NONE).
pub fn unknown(doc: &ColorsDoc, pool: &[Pooled]) -> usize {
    doc.strips()
        .filter(|(strip, _)| !pool.iter().any(|p| p.id == *strip))
        .count()
}

/// A page's choice of `color` for `strip` (none: NONE): a colour only for a
/// fader of the pool ([`Refusal::NotInPool`]; NONE clears any, as a tick
/// off does); `Ok(None)` when it changes nothing, else the document after.
pub fn set_color(
    doc: &ColorsDoc,
    strip: &StripRef,
    color: Option<ColorKey>,
    pool: &[Pooled],
) -> Result<Option<ColorsDoc>, Refusal> {
    if color.is_some() && !pool.iter().any(|p| p.id == *strip) {
        return Err(Refusal::NotInPool);
    }
    if doc.of(strip) == color {
        return Ok(None);
    }
    Ok(Some(doc.with(strip, color)))
}

#[cfg(test)]
mod tests;
