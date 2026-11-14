//! The scene automation (#207; the owner's rulings of 2026-11-14 and the
//! approved mockup `docs/mockups/scenes-v3.html`): Companion tells the hub
//! the service's scene (`POST /api/scene`), and the rules set per fader in
//! its channel detail move that fader: to a dB over a time (2 s unless
//! changed), and a mute or an unmute.
//!
//! The document is the hub's `scenes.json`: every scene name the hub has
//! heard (`known`, in the order first heard: the detail's ADD SCENE picks
//! from them) and each fader's rules by scene, a fader known by its
//! [`StripRef`] (its Live set and label) as the colours know it:
//! `{"known": ["intro"], "rules": {"<instance>": {"<label>": {"<scene>":
//! {"db": -10.0, "mute": "unmute", "time_ms": 2000}}}}}`. An unknown field
//! is an error, not ignored (the file can be edited by hand).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::sectors::{Pooled, StripRef};
use crate::volume::{FLOOR_DB, TOP_DB};

/// The longest scene name, in characters.
pub const MAX_NAME_CHARS: usize = 64;
/// The most scene names the document keeps: a scene past it still runs,
/// its name is not learned.
pub const MAX_KNOWN: usize = 128;
/// A rule's glide time when none was chosen (the owner: "default 2 s").
pub const DEFAULT_TIME_MS: u32 = 2000;
/// The longest glide a rule may ask for.
pub const MAX_TIME_MS: u32 = 60_000;

/// What a rule does to the fader's mute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SceneMute {
    Mute,
    Unmute,
}

impl SceneMute {
    /// Live's `mute` value it writes.
    pub fn value(self) -> bool {
        self == SceneMute::Mute
    }

    /// Its word on the tablet.
    pub fn word(self) -> &'static str {
        match self {
            SceneMute::Mute => "MUTE",
            SceneMute::Unmute => "UNMUTE",
        }
    }
}

/// One fader's rule for one scene: the dB its fader glides to (none: the
/// volume stays; the floor [`FLOOR_DB`] is −∞), its mute (none: it stays)
/// and the glide's time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneRule {
    #[serde(default)]
    pub db: Option<f64>,
    #[serde(default)]
    pub mute: Option<SceneMute>,
    #[serde(default = "default_time")]
    pub time_ms: u32,
}

fn default_time() -> u32 {
    DEFAULT_TIME_MS
}

impl SceneRule {
    /// Whether it does anything (a rule that does nothing is removed).
    pub fn does_something(&self) -> bool {
        self.db.is_some() || self.mute.is_some()
    }

    /// Its problem, if any: a dB that is no number or outside the law's
    /// floor and top, a time past [`MAX_TIME_MS`], or nothing to do.
    pub fn problem(&self) -> Option<&'static str> {
        if !self.does_something() {
            return Some("a rule moves nothing");
        }
        if self
            .db
            .is_some_and(|db| !db.is_finite() || !(FLOOR_DB..=TOP_DB).contains(&db))
        {
            return Some("a rule's dB is outside -70..+6");
        }
        if self.time_ms > MAX_TIME_MS {
            return Some("a rule's time is longer than 60 s");
        }
        None
    }
}

/// A scene name as the API takes it: trimmed, 1 to [`MAX_NAME_CHARS`]
/// characters, no control character. `Err` says why not.
pub fn scene_name(raw: &str) -> Result<String, &'static str> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("the scene name is empty");
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err("the scene name is longer than 64 characters");
    }
    if name.chars().any(char::is_control) {
        return Err("the scene name holds a control character");
    }
    Ok(name.to_string())
}

/// One fader's rules, by scene.
pub type FaderRules = BTreeMap<String, SceneRule>;

/// The document, `scenes.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScenesDoc {
    /// Every scene name heard, in the order first heard.
    #[serde(default)]
    pub known: Vec<String>,
    /// Each Live set's faders by label, each with its rules by scene.
    #[serde(default)]
    pub rules: BTreeMap<String, BTreeMap<String, FaderRules>>,
}

impl ScenesDoc {
    /// The empty document: no scene heard, no rule.
    pub fn empty() -> Self {
        Self {
            known: Vec::new(),
            rules: BTreeMap::new(),
        }
    }

    /// `strip`'s rules (none: an empty map).
    pub fn of(&self, strip: &StripRef) -> FaderRules {
        self.rules
            .get(&strip.instance)
            .and_then(|labels| labels.get(&strip.label))
            .cloned()
            .unwrap_or_default()
    }

    /// `strip`'s rule for `scene`.
    pub fn rule(&self, strip: &StripRef, scene: &str) -> Option<SceneRule> {
        self.rules
            .get(&strip.instance)
            .and_then(|labels| labels.get(&strip.label))
            .and_then(|scenes| scenes.get(scene))
            .copied()
    }

    /// The document with `strip`'s rule for `scene` set (none, or a rule
    /// that does nothing: removed; a fader or a Live set left with no rule
    /// is removed with it).
    pub fn with(&self, strip: &StripRef, scene: &str, rule: Option<SceneRule>) -> Self {
        let mut rules = self.rules.clone();
        let scenes = rules
            .entry(strip.instance.clone())
            .or_default()
            .entry(strip.label.clone())
            .or_default();
        match rule.filter(SceneRule::does_something) {
            Some(rule) => {
                scenes.insert(scene.to_string(), rule);
            }
            None => {
                scenes.remove(scene);
            }
        }
        for labels in rules.values_mut() {
            labels.retain(|_, scenes| !scenes.is_empty());
        }
        rules.retain(|_, labels| !labels.is_empty());
        Self {
            known: self.known.clone(),
            rules,
        }
    }

    /// Whether the hub has heard `scene`.
    pub fn knows(&self, scene: &str) -> bool {
        self.known.iter().any(|k| k == scene)
    }

    /// Every rule of `scene`, with its fader, in the faders' order.
    pub fn rules_of(&self, scene: &str) -> Vec<(StripRef, SceneRule)> {
        self.rules
            .iter()
            .flat_map(|(instance, labels)| {
                labels.iter().filter_map(move |(label, scenes)| {
                    scenes.get(scene).map(|rule| {
                        (
                            StripRef {
                                instance: instance.clone(),
                                label: label.clone(),
                            },
                            *rule,
                        )
                    })
                })
            })
            .collect()
    }

    /// Every fader that has a rule.
    pub fn strips(&self) -> Vec<StripRef> {
        self.rules
            .iter()
            .flat_map(|(instance, labels)| {
                labels.keys().map(move |label| StripRef {
                    instance: instance.clone(),
                    label: label.clone(),
                })
            })
            .collect()
    }

    /// How many faders have a rule.
    pub fn count(&self) -> usize {
        self.rules.values().map(BTreeMap::len).sum()
    }

    /// The rules of the Live set `instance` only, every name known kept
    /// (an MBC login's view, #184).
    pub fn only(&self, instance: &str) -> Self {
        Self {
            known: self.known.clone(),
            rules: self
                .rules
                .iter()
                .filter(|(set, _)| set.as_str() == instance)
                .map(|(set, labels)| (set.clone(), labels.clone()))
                .collect(),
        }
    }

    /// Every problem of the document (empty: valid): a known name that is
    /// no scene name or twice, an empty Live set or label, a rule for a
    /// name that is no scene name, a rule with a problem
    /// ([`SceneRule::problem`]).
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        for (i, name) in self.known.iter().enumerate() {
            if scene_name(name).as_deref() != Ok(name.as_str()) {
                errors.push(format!("known scene {name:?} is no scene name"));
            }
            if self.known[..i].contains(name) {
                errors.push(format!("known scene {name:?} is listed twice"));
            }
        }
        for (instance, labels) in &self.rules {
            if instance.trim().is_empty() {
                errors.push("a Live set's name is empty".to_string());
            }
            for (label, scenes) in labels {
                if label.trim().is_empty() {
                    errors.push(format!("a fader of {instance:?} has an empty label"));
                }
                for (scene, rule) in scenes {
                    if scene_name(scene).as_deref() != Ok(scene.as_str()) {
                        errors.push(format!("{instance}|{label}: {scene:?} is no scene name"));
                    }
                    if let Some(problem) = rule.problem() {
                        errors.push(format!("{instance}|{label} {scene}: {problem}"));
                    }
                }
            }
        }
        errors
    }
}

/// The document after the hub heard `scene`: its name learned (none: it
/// was known, or [`MAX_KNOWN`] names are kept already).
pub fn heard(doc: &ScenesDoc, scene: &str) -> Option<ScenesDoc> {
    if doc.knows(scene) || doc.known.len() >= MAX_KNOWN {
        return None;
    }
    let mut next = doc.clone();
    next.known.push(scene.to_string());
    Some(next)
}

/// Whether a scene name is past what the document keeps (it runs, it is
/// not learned): one warn line.
pub fn is_full(doc: &ScenesDoc, scene: &str) -> bool {
    !doc.knows(scene) && doc.known.len() >= MAX_KNOWN
}

/// Why the hub refuses a page's rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleRefusal {
    /// A rule for a fader the served layout does not carry.
    NotInPool,
    /// A rule for a scene the hub never heard.
    UnknownScene,
    /// A rule with a problem ([`SceneRule::problem`]).
    Invalid,
}

impl RuleRefusal {
    /// The words of the hub's `error` answer.
    pub fn text(self) -> &'static str {
        match self {
            RuleRefusal::NotInPool => "not in the pool",
            RuleRefusal::UnknownScene => "unknown scene",
            RuleRefusal::Invalid => "invalid rule",
        }
    }
}

/// A page's rule for `strip` and `scene` (none: removed): a rule only for
/// a fader of the pool and a scene heard, and one without a problem (a
/// rule that does nothing removes, as none does); a removal is always
/// taken. `Ok(None)` when it changes nothing, else the document after.
pub fn set_rule(
    doc: &ScenesDoc,
    strip: &StripRef,
    scene: &str,
    rule: Option<SceneRule>,
    pool: &[Pooled],
) -> Result<Option<ScenesDoc>, RuleRefusal> {
    let rule = rule.filter(SceneRule::does_something);
    if let Some(rule) = rule {
        if !pool.iter().any(|p| p.id == *strip) {
            return Err(RuleRefusal::NotInPool);
        }
        if !doc.knows(scene) {
            return Err(RuleRefusal::UnknownScene);
        }
        if rule.problem().is_some() {
            return Err(RuleRefusal::Invalid);
        }
    }
    if doc.rule(strip, scene) == rule {
        return Ok(None);
    }
    Ok(Some(doc.with(strip, scene, rule)))
}

/// How many faders with a rule no pool fader carries (a track renamed in
/// Live: its rules wait, they move nothing).
pub fn unknown(doc: &ScenesDoc, pool: &[Pooled]) -> usize {
    doc.strips()
        .iter()
        .filter(|strip| !pool.iter().any(|p| p.id == **strip))
        .count()
}

#[cfg(test)]
mod tests;
