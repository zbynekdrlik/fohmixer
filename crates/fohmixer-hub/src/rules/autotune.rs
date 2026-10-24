//! AUTOTUNE AUT (#104, D19): while the flag is on, every `+AUTOTUNE`
//! track's autotune follows its Live set's transport: playing → on,
//! stopped → off; once per change, at the flag's turn on, and once after a
//! reconnect (STAGE AUT's rules, `rules.rs`). A tap sets every target by
//! hand until the next change.
//!
//! Pure: the router's glue (`router/autotune.rs`) finds each target's
//! `Latencies` rack ([`find`]), writes its first macro through its `$ref`
//! and records every [`Step`]. The keys (#140 PR 2) are [`keys`]: until
//! #140 PR 3 a page can neither turn AUTOTUNE AUT on nor tap AUTOTUNE
//! ([`Step::Refused`]), so nothing writes the macros.

use std::collections::{BTreeMap, BTreeSet};

use fohmixer_proto::client::TargetState;
use serde::Serialize;

use super::stage_rule;
use keys::{BarState, KeyDevice, KeyState, KeyWhy, LATE_BOUND_MS, state_word};

#[path = "autotune/find.rs"]
pub mod find;
#[path = "autotune/keys.rs"]
pub mod keys;

/// A target's value within half a step of its on value is on; any other is
/// off (today's repro "off" values, 11 to 16 set by hand, read off).
pub const SAME_VALUE: f64 = 0.5;

/// Where a target stands: `value` its macro's (none: not heard yet), `on`
/// its on value.
pub fn target_state(value: Option<f64>, on: u8) -> TargetState {
    match value {
        None => TargetState::Waiting,
        Some(v) if (v - f64::from(on)).abs() < SAME_VALUE => TargetState::On,
        Some(_) => TargetState::Off,
    }
}

/// One followed Live set.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Followed {
    /// Its latest `is_playing` this session.
    playing: Option<bool>,
    /// What the rule last wrote this session (a tap never sets it).
    written: Option<bool>,
    /// The last write the set's targets got, by the rule or a tap: a target
    /// found later gets it.
    last: Option<bool>,
}

/// The rule's state between events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutotuneAut {
    on: bool,
    /// The Live sets of the layout's targets.
    sets: BTreeMap<String, Followed>,
    /// The rule's writes since the hub started.
    writes: u64,
    /// The pages' taps since the hub started.
    taps: u64,
}

impl AutotuneAut {
    /// The rule with the persisted flag, following no set yet.
    pub fn new(on: bool) -> Self {
        Self {
            on,
            sets: BTreeMap::new(),
            writes: 0,
            taps: 0,
        }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    pub fn writes(&self) -> u64 {
        self.writes
    }

    pub fn taps(&self) -> u64 {
        self.taps
    }

    /// A new layout's Live sets: the ones followed stay as they are, a new
    /// one takes `transport(set)` as last heard (applied at once), a gone one
    /// is dropped. The writes to make.
    pub fn follow(
        &mut self,
        sets: &BTreeSet<String>,
        transport: impl Fn(&str) -> Option<bool>,
    ) -> Vec<(String, bool)> {
        self.sets.retain(|set, _| sets.contains(set));
        let new: Vec<String> = sets
            .iter()
            .filter(|set| !self.sets.contains_key(*set))
            .cloned()
            .collect();
        for set in &new {
            self.sets.insert(
                set.clone(),
                Followed {
                    playing: transport(set),
                    written: None,
                    last: None,
                },
            );
        }
        new.into_iter()
            .filter_map(|set| self.decide(&set).map(|on| (set, on)))
            .collect()
    }

    /// A set's new `is_playing`: the write to make, if any.
    pub fn on_playing(&mut self, set: &str, playing: bool) -> Option<bool> {
        self.sets.get_mut(set)?.playing = Some(playing);
        self.decide(set)
    }

    /// The flag changed: turning it on applies each set's transport at once.
    /// The writes to make.
    pub fn set_flag(&mut self, on: bool) -> Vec<(String, bool)> {
        if on && !self.on {
            self.sets.values_mut().for_each(|f| f.written = None);
        }
        self.on = on;
        let sets: Vec<String> = self.sets.keys().cloned().collect();
        sets.into_iter()
            .filter_map(|set| self.decide(&set).map(|on| (set, on)))
            .collect()
    }

    /// A page's tap: every set's targets on or off by hand, until the next
    /// change of its transport (the rule's `written` stays, as STAGE AUT's).
    pub fn tap(&mut self, on: bool) -> Vec<(String, bool)> {
        self.taps += 1;
        self.sets
            .iter_mut()
            .map(|(set, f)| {
                f.last = Some(on);
                (set.clone(), on)
            })
            .collect()
    }

    /// The set's Live went away: its next session starts afresh (its first
    /// transport value writes once).
    pub fn on_disconnect(&mut self, set: &str) {
        if let Some(f) = self.sets.get_mut(set) {
            *f = Followed::default();
        }
    }

    /// The set's transport listener failed: its transport is not known until
    /// the next value (what was written stays).
    pub fn transport_lost(&mut self, set: &str) {
        if let Some(f) = self.sets.get_mut(set) {
            f.playing = None;
        }
    }

    /// What a target of `set` found later gets: the set's last write.
    pub fn wanted(&self, set: &str) -> Option<bool> {
        self.sets.get(set).and_then(|f| f.last)
    }

    /// The set's transport as last heard.
    pub fn playing(&self, set: &str) -> Option<bool> {
        self.sets.get(set).and_then(|f| f.playing)
    }

    /// The write the rule wants for `set` now, once per change.
    fn decide(&mut self, set: &str) -> Option<bool> {
        let followed = self.sets.get_mut(set)?;
        let want = stage_rule(self.on, followed.playing?)?;
        if followed.written == Some(want) {
            return None;
        }
        followed.written = Some(want);
        followed.last = Some(want);
        self.writes += 1;
        Some(want)
    }
}

/// Why AUTOTUNE writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    /// The followed transport changed (or was heard first).
    Transport,
    /// AUTOTUNE AUT turned on.
    Flag,
    /// A page's tap.
    Tap,
    /// A target found after its set's last write.
    Found,
}

impl Why {
    fn word(self) -> &'static str {
        match self {
            Why::Transport => "transport",
            Why::Flag => "flag",
            Why::Tap => "tap",
            Why::Found => "found",
        }
    }
}

/// One step of AUTOTUNE: an `autotune` record and a log line.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum Step {
    /// A page set AUTOTUNE AUT.
    Flag { on: bool },
    /// A page tapped AUTOTUNE.
    Tap { on: bool },
    /// The macros of a Live set's found targets written (`targets`: their
    /// labels).
    Write {
        instance: String,
        on: bool,
        why: Why,
        targets: Vec<String>,
    },
    /// A target's write failed.
    Failed {
        instance: String,
        target: String,
        problem: String,
    },
    /// A target's rack found: its first macro's path.
    Found {
        instance: String,
        target: String,
        path: String,
    },
    /// A target has no rack to switch, and why.
    Missing {
        instance: String,
        target: String,
        problem: String,
    },
    /// A target's walks failed `tries` times in a row (#140: Live did not
    /// answer or was offline): no more retries until the next trigger.
    /// `tries` counts the first walk and its retries, the unit of every
    /// give-up record (PR #145 review M10).
    GaveUp {
        instance: String,
        target: String,
        problem: String,
        tries: u32,
    },
    /// A Live set's writes are held (`on`) while its markers may not be
    /// where Live has them (#104, PR #139 review I1): it went `offline`, its
    /// track list changed (`tracks`), or a write met its markers unsettled
    /// (`markers`); the hold ends (not `on`) once they settled with no new
    /// layout (`settled`) or a layout of them was served (`layout`).
    Held {
        instance: String,
        on: bool,
        why: String,
    },
    /// The key in force of a Live set changed (#140 PR 2): its song (none:
    /// out of every song) and key (none: no key in force, a keyless song
    /// too), and why.
    Key {
        instance: String,
        song: Option<u16>,
        key: Option<String>,
        why: KeyWhy,
    },
    /// A target's key device found: which device and its path.
    KeyDevice {
        instance: String,
        target: String,
        device: KeyDevice,
        path: String,
    },
    /// A target's rack has no key device, and why (its macro is found).
    KeyMissing {
        instance: String,
        target: String,
        problem: String,
    },
    /// A page's AUTOTUNE AUT on (`what` `aut`) or tap (`tap`) refused, and
    /// why ([`REFUSED_WHY`]).
    Refused { what: String, why: String },
    /// The key `key` written to a Live set's found key devices (`targets`:
    /// their labels) and, with `bar`, to Live's bar (#140 PR 2b), and why.
    KeyWrite {
        instance: String,
        key: String,
        why: KeyWhy,
        targets: Vec<String>,
        bar: bool,
    },
    /// A target's write of `key` and its read-back: where the key stands
    /// (`state`), the key Live's values show, and Live's words when it
    /// refused (a Max map holds the parameter).
    Readback {
        instance: String,
        target: String,
        key: String,
        state: KeyState,
        shows: Option<String>,
        problem: Option<String>,
    },
    /// A target's values changed since the hub confirmed its key: shown,
    /// never written against (spec §5).
    AnotherWriter {
        instance: String,
        target: String,
        shows: Option<String>,
    },
    /// How late song `song`'s key change was written after its clip's
    /// start, in ms (below 0: before it; spec §6).
    Late {
        instance: String,
        song: u16,
        ms: f64,
    },
    /// Live's bar's read-back after the key `key` was written there: the
    /// key it shows (it gates nothing).
    Bar {
        instance: String,
        key: String,
        state: BarState,
        shows: Option<String>,
    },
}

/// Why a page's AUTOTUNE AUT on and tap are refused (#140 PR 2: the keys
/// first, nothing switches).
pub const REFUSED_WHY: &str = "keys only until #140 PR 3";

/// A key in force in words: its song and key, a keyless song, or no song.
fn key_words(song: Option<u16>, key: Option<&str>) -> String {
    match (song, key) {
        (Some(song), Some(key)) => format!("song {song} {key}"),
        (Some(song), None) => format!("song {song}, no key"),
        (None, _) => "no song".to_string(),
    }
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

/// A key Live's values show, in words (none: no key).
fn shown(shows: Option<&str>) -> &str {
    shows.unwrap_or("no key")
}

/// `n` key devices in words.
fn key_devices(n: usize) -> String {
    if n == 1 {
        "1 key device".to_string()
    } else {
        format!("{n} key devices")
    }
}

/// A key change's lateness in words: ms after its clip's start, or before
/// it for a write the lookahead made early.
fn lateness_words(ms: f64) -> String {
    if ms < 0.0 {
        format!("{:.1} ms before its start", -ms)
    } else {
        format!("{ms:.1} ms after its start")
    }
}

/// A step's log line.
pub fn step_line(step: &Step) -> String {
    match step {
        Step::Flag { on } => format!("AUTOTUNE AUT turned {}", on_off(*on)),
        Step::Tap { on } => format!("AUTOTUNE tapped: {}", on_off(*on)),
        Step::Write {
            instance,
            on,
            why,
            targets,
        } => format!(
            "AUTOTUNE AUT writes {instance} {}: {} tracks ({})",
            on_off(*on),
            targets.len(),
            why.word()
        ),
        Step::Failed {
            instance,
            target,
            problem,
        } => format!("AUTOTUNE could not write {instance} {target}: {problem}"),
        Step::Found {
            instance,
            target,
            path,
        } => format!("AUTOTUNE found {instance} {target}'s rack: {path}"),
        Step::Missing {
            instance,
            target,
            problem,
        } => format!("AUTOTUNE has no rack for {instance} {target}: {problem}"),
        Step::GaveUp {
            instance,
            target,
            problem,
            tries,
        } => format!(
            "AUTOTUNE gave up walking {instance} {target} after {tries} failed walks, until the next trigger: {problem}"
        ),
        Step::Held {
            instance,
            on: true,
            why,
        } => format!("AUTOTUNE writes nothing to {instance} until its markers are settled ({why})"),
        Step::Held { instance, why, .. } => {
            format!("AUTOTUNE writes {instance} again: its markers are settled ({why})")
        }
        Step::Key {
            instance,
            song,
            key,
            why,
        } => format!(
            "AUTOTUNE key in force in {instance}: {} ({})",
            key_words(*song, key.as_deref()),
            why.word()
        ),
        Step::KeyDevice {
            instance,
            target,
            device,
            path,
        } => format!(
            "AUTOTUNE found {instance} {target}'s key device ({}): {path}",
            device.name()
        ),
        Step::KeyMissing {
            instance,
            target,
            problem,
        } => format!("AUTOTUNE has no key device for {instance} {target}: {problem}"),
        Step::Refused { what, why } => format!("AUTOTUNE refuses a page's {what}: {why}"),
        Step::KeyWrite {
            instance,
            key,
            why,
            targets,
            bar,
        } => format!(
            "AUTOTUNE writes {key} to {instance}: {}{} ({})",
            key_devices(targets.len()),
            if *bar { " and Live's bar" } else { "" },
            why.word()
        ),
        Step::Readback {
            instance,
            target,
            key,
            state,
            problem: Some(problem),
            ..
        } => format!(
            "AUTOTUNE {instance} {target}: {} for {key}, nothing read back: {problem}",
            state_word(*state)
        ),
        Step::Readback {
            instance,
            target,
            key,
            state,
            shows,
            problem: None,
        } => format!(
            "AUTOTUNE read back {instance} {target}: {} for {key}, it shows {}",
            state_word(*state),
            shown(shows.as_deref())
        ),
        Step::AnotherWriter {
            instance,
            target,
            shows,
        } => format!(
            "AUTOTUNE sees another writer on {instance} {target}: it shows {}, not written against",
            shown(shows.as_deref())
        ),
        Step::Late { instance, song, ms } => format!(
            "AUTOTUNE wrote {instance} song {song}'s key change {}",
            lateness_words(*ms)
        ),
        Step::Bar {
            instance,
            key,
            state,
            shows,
        } => format!(
            "AUTOTUNE read back {instance}'s Live bar: {} for {key}, it shows {}",
            match state {
                BarState::Confirmed => "confirmed",
                BarState::Shows => "another key",
            },
            shown(shows.as_deref())
        ),
    }
}

/// Whether a step is logged as a warning: a failed write, a missing rack,
/// a missing key device, a key not confirmed, another writer, a key change
/// written later than [`LATE_BOUND_MS`].
pub fn step_warn(step: &Step) -> bool {
    match step {
        Step::Failed { .. }
        | Step::Missing { .. }
        | Step::GaveUp { .. }
        | Step::KeyMissing { .. }
        | Step::AnotherWriter { .. } => true,
        Step::Readback { state, .. } => *state != KeyState::Confirmed,
        Step::Late { ms, .. } => *ms > LATE_BOUND_MS,
        _ => false,
    }
}

#[cfg(test)]
#[path = "autotune/tests.rs"]
mod tests;
