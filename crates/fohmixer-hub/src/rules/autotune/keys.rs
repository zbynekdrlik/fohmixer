//! The autotune keys (#140 PR 2; spec §2.1, §5, §6, Appendix A.4–A.5),
//! pure: which device of a target's `Latencies` rack holds its Key and
//! Scale ([`device_of`]: the native Auto-Tune Artist by its parameters'
//! names, AudioGridder by its first two automation slots), the key a pair
//! of values shows ([`shows`], the step indexes half up), the key in force
//! at a position of the playhead with one script tick of lookahead while
//! playing ([`in_force`], [`lookahead_beats`]), where the key change in
//! force started ([`change_time`]), and the per-set follower that says why
//! the key in force changed ([`KeyFollow`], [`KeyWhy`]).
//!
//! The router's glue is `router/autotune/keys.rs`. PR 2a only reads: it
//! finds the key devices, follows their values and the key in force; the
//! writes are PR 2b's.

use fohmixer_proto::songmap::key::{key_index_of, scale_index_of};
use fohmixer_proto::songmap::{Key, SongMap, key_at};
use serde::Serialize;

/// The Auto-Tune Artist's Key parameter's name.
pub const KEY_NAME: &str = "Key";
/// The Auto-Tune Artist's Scale parameter's name.
pub const SCALE_NAME: &str = "Scale";
/// AudioGridder's automation slot 0 (Key): LOM parameter 1 (0 is `Device
/// On`), named `0:…` by Live.
pub const GRIDDER_KEY: usize = 1;
/// AudioGridder's automation slot 1 (Scale): LOM parameter 2, named `1:…`.
pub const GRIDDER_SCALE: usize = 2;
/// One script tick (~30 Hz) of lookahead while playing, in ms (spec §6).
pub const LOOKAHEAD_MS: f64 = 35.0;
/// The lookahead at most, in beats: never earlier than one beat.
pub const MAX_LOOKAHEAD_BEATS: f64 = 1.0;

/// The device that holds a target's Key and Scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyDevice {
    /// The native Auto-Tune Artist (the PA racks): its `Key` and `Scale`.
    AutoTune,
    /// AudioGridder (the stream racks): its automation slots 0 and 1.
    Gridder,
}

impl KeyDevice {
    /// Its name in `/api/status` and the records (`auto_tune`, `gridder`).
    pub fn name(self) -> &'static str {
        match self {
            KeyDevice::AutoTune => "auto_tune",
            KeyDevice::Gridder => "gridder",
        }
    }
}

/// A key device and the indexes of its Key and Scale parameters (in its
/// `parameters` list).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyParams {
    pub device: KeyDevice,
    pub key: usize,
    pub scale: usize,
}

/// Why a target has no key device. It never makes the selector's find
/// missing: the target's macro is still found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyMissing {
    /// The `Latencies` rack has no chain.
    NoChain,
    /// Its chain 0 holds this many plug-ins (only one is the key device).
    Plugins(usize),
    /// The plug-in has neither the Auto-Tune Artist's Key and Scale nor
    /// AudioGridder's slots.
    NoKeyParams,
}

impl std::fmt::Display for KeyMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyMissing::NoChain => write!(f, "the Latencies rack has no chain"),
            KeyMissing::Plugins(n) => write!(f, "chain 0 holds {n} plug-ins, not one"),
            KeyMissing::NoKeyParams => write!(f, "the plug-in has no Key/Scale parameters"),
        }
    }
}

/// The index of the one parameter named `name`; none when none or several
/// are (which one is meant is not known).
fn only(names: &[&str], name: &str) -> Option<usize> {
    let mut found = names
        .iter()
        .enumerate()
        .filter(|(_, n)| **n == name)
        .map(|(i, _)| i);
    match (found.next(), found.next()) {
        (Some(i), None) => Some(i),
        _ => None,
    }
}

/// The key device a plug-in's parameter names (in its `parameters` order)
/// make: exactly one `Key` and one `Scale` is the Auto-Tune Artist with
/// their indexes; else parameter 1 named `0:…` and parameter 2 named
/// `1:…` is AudioGridder (spec §5, the spike of 2026-10-24); else none.
/// The Auto-Tune Artist wins when both match.
pub fn device_of(names: &[&str]) -> Result<KeyParams, KeyMissing> {
    if let (Some(key), Some(scale)) = (only(names, KEY_NAME), only(names, SCALE_NAME)) {
        return Ok(KeyParams {
            device: KeyDevice::AutoTune,
            key,
            scale,
        });
    }
    let slot = |at: usize, prefix: &str| names.get(at).is_some_and(|n| n.starts_with(prefix));
    if slot(GRIDDER_KEY, "0:") && slot(GRIDDER_SCALE, "1:") {
        return Ok(KeyParams {
            device: KeyDevice::Gridder,
            key: GRIDDER_KEY,
            scale: GRIDDER_SCALE,
        });
    }
    Err(KeyMissing::NoKeyParams)
}

/// The key a Key and a Scale value show (spec A.4): the steps `p × 11` and
/// `p × 28`, half up; a scale step of 0 is Major, 1 Minor, any other
/// (Chromatic, Auto-Tune's default, among them) no key.
pub fn shows(key_value: f64, scale_value: f64) -> Option<Key> {
    let minor = match scale_index_of(scale_value) {
        0 => false,
        1 => true,
        _ => return None,
    };
    Some(Key {
        root: key_index_of(key_value),
        minor,
    })
}

/// The lookahead in beats at `tempo` (bpm): one tick, at most a beat; none
/// without a tempo, or for one that is not finite or not above 0 (no
/// division, no NaN).
pub fn lookahead_beats(tempo: Option<f64>) -> f64 {
    match tempo {
        Some(t) if t.is_finite() && t.is_sign_positive() => {
            (LOOKAHEAD_MS * t / 60_000.0).min(MAX_LOOKAHEAD_BEATS)
        }
        _ => 0.0,
    }
}

/// The song and key in force at `time` (beats): one tick ahead while
/// `playing` (a key clip's change is due when the position reaches its
/// start, spec §6), at the position itself while stopped.
pub fn in_force(map: &SongMap, time: f64, playing: bool, tempo: Option<f64>) -> Option<(u16, Key)> {
    let ahead = if playing { lookahead_beats(tempo) } else { 0.0 };
    key_at(map, time + ahead)
}

/// Where the key change of song `song` in force at `time` started: its
/// last change at or before `time`; none before its first change, for a
/// song with none, or a song not in the map.
pub fn change_time(map: &SongMap, song: u16, time: f64) -> Option<f64> {
    map.spans
        .iter()
        .find(|span| span.number == song)?
        .changes
        .iter()
        .rev()
        .find(|change| change.time <= time)
        .map(|change| change.time)
}

/// Why the key in force changed (the `key` record's `why`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyWhy {
    /// A song other than the last one in force (a song start, a jump into
    /// it).
    Song,
    /// The same song, another key (a key clip passed, a jump back before
    /// it).
    Change,
    /// No key in force any more (out of every song, a keyless song).
    NoKey,
    /// A target found anew (PR 2b's writes).
    Found,
    /// A write's one retry (PR 2b).
    Retry,
}

impl KeyWhy {
    /// Its word in a log line.
    pub fn word(self) -> &'static str {
        match self {
            KeyWhy::Song => "song",
            KeyWhy::Change => "change",
            KeyWhy::NoKey => "no_key",
            KeyWhy::Found => "found",
            KeyWhy::Retry => "retry",
        }
    }
}

/// Why the key in force went from `before` to `now`, if it changed: a span
/// other than the last one in force is a song start (its key equal or
/// not), another key in the same span a change, none after one a no key.
fn why_of(before: Option<(u16, Key)>, now: Option<(u16, Key)>) -> Option<KeyWhy> {
    match (before, now) {
        (Some((was_song, was_key)), Some((song, key))) if was_song == song => {
            (was_key != key).then_some(KeyWhy::Change)
        }
        (_, Some(_)) => Some(KeyWhy::Song),
        (Some(_), None) => Some(KeyWhy::NoKey),
        (None, None) => None,
    }
}

/// One Live set's key in force as the playhead moves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyFollow {
    song: Option<u16>,
    key: Option<Key>,
}

impl KeyFollow {
    /// Nothing in force yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The key in force now (`in_force`'s answer): why it changed, if it
    /// did. Leaving every song forgets the last one, so coming back into it
    /// is a song start again.
    pub fn on_position(&mut self, now: Option<(u16, Key)>) -> Option<KeyWhy> {
        let why = why_of(self.now(), now);
        self.song = now.map(|(song, _)| song);
        self.key = now.map(|(_, key)| key);
        why
    }

    /// The song and key in force, as last told.
    pub fn now(&self) -> Option<(u16, Key)> {
        self.song.zip(self.key)
    }

    /// A reconnect forgets the last song: its next position is a start.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
#[path = "keys_tests.rs"]
mod tests;
