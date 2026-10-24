//! AUTOTUNE AUT (#104, D19; #140 PR 3b): the flag, each followed Live
//! set's transport, the singers off by hand, and every step's record and
//! log line. The decision per track is [`switch`] (spec §7: AUT, Live
//! playing, a keyed song, the singer, the target, the fall back armed, the
//! key confirmed), the find [`find`] and the keys [`keys`].
//!
//! Pure: the router's glue (`router/autotune.rs`) finds each target's
//! `Latencies` rack, decides every target (`router/autotune/switch.rs`),
//! writes its first macro through its `$ref` when the decision differs
//! from what Live shows and records every [`Step`]. 1.8.0's transport-only
//! writes and its tap are gone (PR 2a review M10): a page's tap is refused
//! ([`Step::Refused`], [`REFUSED_WHY`]).

use std::collections::{BTreeMap, BTreeSet};

use fohmixer_proto::client::TargetState;
use serde::Serialize;

use keys::{BarState, KeyDevice, KeyState, KeyWhy, LATE_BOUND_MS, state_word};
use switch::{SwitchWhy, why_word};

#[path = "autotune/find.rs"]
pub mod find;
#[path = "autotune/keys.rs"]
pub mod keys;
#[path = "autotune/switch.rs"]
pub mod switch;

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

/// The rule's state between events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutotuneAut {
    on: bool,
    /// The Live sets of the layout's targets: each one's `is_playing` as
    /// last heard this session.
    sets: BTreeMap<String, Option<bool>>,
    /// The selector batches the rule sent since the hub started.
    writes: u64,
    /// The singers off by hand (#140 PR 3b, spec §2.2), by
    /// `layout::singer_of`: kept in `hub-state.json`.
    singers_off: BTreeSet<String>,
}

impl AutotuneAut {
    /// The rule with the persisted flag and singers off, following no set
    /// yet (#140 PR 3b: AUT is kept across a restart).
    pub fn new(on: bool, off: Vec<String>) -> Self {
        Self {
            on,
            sets: BTreeMap::new(),
            writes: 0,
            singers_off: off.into_iter().collect(),
        }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    pub fn writes(&self) -> u64 {
        self.writes
    }

    /// One batch of selector writes went out.
    pub fn wrote(&mut self) {
        self.writes += 1;
    }

    /// A new layout's Live sets: the ones followed stay as they are, a new
    /// one takes `transport(set)` as last heard, a gone one is dropped.
    pub fn follow(&mut self, sets: &BTreeSet<String>, transport: impl Fn(&str) -> Option<bool>) {
        self.sets.retain(|set, _| sets.contains(set));
        for set in sets {
            if !self.sets.contains_key(set) {
                self.sets.insert(set.clone(), transport(set));
            }
        }
    }

    /// A set's new `is_playing` (a set no layout names is not followed).
    pub fn on_playing(&mut self, set: &str, playing: bool) {
        if let Some(heard) = self.sets.get_mut(set) {
            *heard = Some(playing);
        }
    }

    /// The flag (the router decides every target then).
    pub fn set_flag(&mut self, on: bool) {
        self.on = on;
    }

    /// The set's Live went away: its transport is not known until its next
    /// session's first value.
    pub fn on_disconnect(&mut self, set: &str) {
        self.transport_lost(set);
    }

    /// The set's transport listener failed: its transport is not known
    /// until the next value.
    pub fn transport_lost(&mut self, set: &str) {
        if let Some(heard) = self.sets.get_mut(set) {
            *heard = None;
        }
    }

    /// The set's transport as last heard.
    pub fn playing(&self, set: &str) -> Option<bool> {
        self.sets.get(set).copied().flatten()
    }

    /// A singer switched off by hand or back on: whether that changed what
    /// the rule holds.
    pub fn set_singer(&mut self, id: &str, off: bool) -> bool {
        if off {
            self.singers_off.insert(id.to_string())
        } else {
            self.singers_off.remove(id)
        }
    }

    /// Whether the singer `id` is off by hand.
    pub fn is_off(&self, id: &str) -> bool {
        self.singers_off.contains(id)
    }

    /// The singers off by hand, sorted (as saved).
    pub fn singers_off(&self) -> Vec<String> {
        self.singers_off.iter().cloned().collect()
    }
}

/// What made the hub decide its targets again (#140 PR 3b; the `select`
/// and `write` records' `cause`, spec §7's `found`, `reconnect` and the
/// others).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    /// The set's transport changed.
    Transport,
    /// The song or the key in force changed (a song start, a key clip, a
    /// jump, out of every song).
    Position,
    /// A key's write was read back or refused.
    Key,
    /// AUTOTUNE AUT switched.
    Flag,
    /// A singer switched off by hand or back on.
    Singer,
    /// A target found (after a connect too), or its selector's first value
    /// heard.
    Found,
    /// Another writer moved a selector.
    Guard,
    /// A hold ended.
    Release,
    /// A hold started: DRY first, through the last known refs.
    Hold,
    /// The set's registration of its DRY writes changed.
    Safe,
    /// A layout changed a target.
    Layout,
}

impl Cause {
    /// Its word in a log line.
    pub fn word(self) -> &'static str {
        match self {
            Cause::Transport => "transport",
            Cause::Position => "position",
            Cause::Key => "key",
            Cause::Flag => "flag",
            Cause::Singer => "singer",
            Cause::Found => "found",
            Cause::Guard => "guard",
            Cause::Release => "release",
            Cause::Hold => "hold",
            Cause::Safe => "safe",
            Cause::Layout => "layout",
        }
    }
}

/// What the hub did about another writer on a selector (spec §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriterAct {
    /// It moved to the autotune chain while the hub wants DRY: DRY again.
    DryAgain,
    /// It moved again within 10 s: the hub stops writing it until the next
    /// song start.
    GaveUp,
    /// It moved to DRY while the hub wants the chain: left so.
    LeftDry,
}

impl WriterAct {
    fn words(self) -> &'static str {
        match self {
            WriterAct::DryAgain => "DRY again",
            WriterAct::GaveUp => "the hub stops writing it until the next song start",
            WriterAct::LeftDry => "left on DRY",
        }
    }
}

/// One step of AUTOTUNE: an `autotune` record and a log line.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum Step {
    /// A page set AUTOTUNE AUT.
    Flag { on: bool },
    /// The selectors of a Live set's targets written in one batch (#140 PR
    /// 3b): the labels written to their autotune chain (`on`) and to DRY
    /// (`off`), and what made the hub decide.
    Write {
        instance: String,
        cause: Cause,
        on: Vec<String>,
        off: Vec<String>,
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
    /// A page's AUTOTUNE tap (`what` `tap`; 1.8.0's, gone since #140 PR
    /// 3b) refused, and why ([`REFUSED_WHY`]).
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
    /// The answer to a registration of a Live set's DRY writes with its
    /// script (#140 PR 3a: `targets` selectors it makes DRY when the hub is
    /// gone), and why it failed (none: the script keeps them).
    Safe {
        instance: String,
        targets: usize,
        problem: Option<String>,
    },
    /// A target's decision switched (#140 PR 3b): its autotune chain
    /// selected (`on`) or DRY, why, and what made the hub decide.
    Select {
        instance: String,
        target: String,
        on: bool,
        why: SwitchWhy,
        cause: Cause,
    },
    /// Another writer moved a target's selector, and what the hub did.
    SelectorWriter {
        instance: String,
        target: String,
        act: WriterAct,
    },
    /// A page switched a singer off by hand or back on.
    Singer { id: String, off: bool },
}

/// Why a page's AUTOTUNE tap is refused (#140 PR 3b: 1.8.0's tap is gone;
/// in the new surface, PR 3c, a tap opens the singers' panel).
pub const REFUSED_WHY: &str = "the AUTOTUNE tap is gone; a tap opens the singers' panel";

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
        Step::Write {
            instance,
            cause,
            on,
            off,
        } => format!(
            "AUTOTUNE writes {instance}'s selectors: {} on, {} to DRY ({})",
            on.len(),
            off.len(),
            cause.word()
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
        Step::Safe {
            instance,
            targets,
            problem: None,
        } => format!(
            "AUTOTUNE gave {instance}'s script the DRY writes of {targets} selectors, for when the hub is gone"
        ),
        Step::Safe {
            instance,
            targets,
            problem: Some(problem),
        } => format!(
            "AUTOTUNE could not give {instance}'s script the DRY writes of {targets} selectors: {problem}"
        ),
        Step::Select {
            instance,
            target,
            on,
            why,
            cause,
        } => format!(
            "AUTOTUNE switches {instance} {target} {} ({}, {})",
            if *on { "on" } else { "to DRY" },
            why_word(*why),
            cause.word()
        ),
        Step::SelectorWriter {
            instance,
            target,
            act,
        } => format!(
            "AUTOTUNE sees another writer on {instance} {target}'s selector: {}",
            act.words()
        ),
        Step::Singer { id, off } => format!(
            "AUTOTUNE singer {id} {}",
            if *off {
                "off by hand"
            } else {
                "back on the rule"
            }
        ),
    }
}

/// Whether a step is logged as a warning: a failed write, a missing rack,
/// a missing key device, a key not confirmed, another writer (on a key or
/// a selector), a key change written later than [`LATE_BOUND_MS`], a
/// failed registration of the DRY writes.
pub fn step_warn(step: &Step) -> bool {
    match step {
        Step::Failed { .. }
        | Step::Missing { .. }
        | Step::GaveUp { .. }
        | Step::KeyMissing { .. }
        | Step::AnotherWriter { .. }
        | Step::SelectorWriter { .. }
        | Step::Safe {
            problem: Some(_), ..
        } => true,
        Step::Readback { state, .. } => *state != KeyState::Confirmed,
        Step::Late { ms, .. } => *ms > LATE_BOUND_MS,
        _ => false,
    }
}

#[cfg(test)]
#[path = "autotune/tests.rs"]
mod tests;
