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
//! PR 2b adds the writes' pure parts: the value to write ([`write_value`],
//! [`key_values`]: a step's centre, or a nudge beside it when Live
//! already holds the centre), the read-back's judgement ([`judge`],
//! [`Readback`]), each target's write state with its one retry
//! ([`KeyTarget`], [`KeyState`]), a key change's lateness ([`late_ms`])
//! and what Live's bar shows ([`bar_shows`], [`BarState`]).
//!
//! The router's glue is `router/autotune/keys.rs` (the reads) and
//! `router/autotune/key_writes.rs` (the writes and read-backs).

use fohmixer_proto::songmap::key::{KEY_STEPS, SCALE_STEPS, key_index_of, scale_index_of};
use fohmixer_proto::songmap::{Key, SongMap, key_at, song_at};
use serde::Serialize;

pub use fohmixer_proto::client::KeyState;

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
/// How far beside a step's centre (in steps) a value Live already holds is
/// written (spec §5: a write always changes Live's value). 0.05 stays in
/// the step under both a rounding plug-in (`round(v × n)`, Auto-Tune
/// Artist's own names) and VST3's discrete mapping (`floor(v × (n + 1))`),
/// for every key and both scales; a quarter step left it there for G#, A
/// and Bb (PR #144 review I2).
pub const NUDGE: f64 = 0.05;
/// Within this of a step's centre a value is that centre: Live keeps a
/// parameter's value as a float32 (the PC read 2/11 back as
/// 0.1818181872367859).
pub const SAME_VALUE_EPS: f64 = 1e-6;
/// A key change's lateness bound after its clip's start, in ms (spec §6):
/// a `late` record beyond it is a warning.
pub const LATE_BOUND_MS: f64 = 100.0;
/// The longest step forward of the playhead between two deliveries that is
/// still continuous play, in beats: ~30 deliveries a second move it ~0.07
/// beat at 120 bpm, and a stalled half second still stays under one beat;
/// a longer step is a jump or a locate (PR #144 review M1).
pub const CROSSING_BEATS: f64 = 1.0;

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

/// Why a device's parameter list could not be read (#140): Live answered it
/// as text, as the FohMixer script does for a Live vector class its codec
/// did not know (a device's `Device.ATimeableValueVector`, the PC with
/// 1.10.0's script). The script loaded in Live is older than the fix, and
/// a Live restart loads the new one.
pub const PARAMETERS_AS_TEXT: &str = "Live answered the parameter list as text: the FohMixer \
                                      script loaded in Live predates the fix for it; restart Live";

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
    /// The plug-in's parameter list came as text ([`PARAMETERS_AS_TEXT`]).
    ParametersAsText,
}

impl std::fmt::Display for KeyMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyMissing::NoChain => write!(f, "the Latencies rack has no chain"),
            KeyMissing::Plugins(n) => write!(f, "chain 0 holds {n} plug-ins, not one"),
            KeyMissing::NoKeyParams => write!(f, "the plug-in has no Key/Scale parameters"),
            KeyMissing::ParametersAsText => write!(f, "{PARAMETERS_AS_TEXT}"),
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

/// The lookahead a set's key in force is judged with (beats; spec §6, PR
/// 2a review M4 as ruled): a playing set's one tick at its tempo; a set
/// not known to play keeps the one it was last judged with (`kept`) until
/// its playhead moves (`moved`: a jump, a locate), then none. So a stop
/// (or a lost transport) alone never judges the key again: a stop one
/// tick before a key clip keeps the clip's key.
pub fn kept_ahead(playing: bool, tempo: Option<f64>, kept: f64, moved: bool) -> f64 {
    if playing {
        lookahead_beats(tempo)
    } else if moved {
        0.0
    } else {
        kept
    }
}

/// Where a position is: its song (none outside every song) and the key in
/// force there (none outside every song, or in a keyless song: PR 2a review
/// M2 tells the two apart).
pub type InForce = (Option<u16>, Option<Key>);

/// The song and key in force at `time` (beats): one tick ahead while
/// `playing` (a key clip's change is due when the position reaches its
/// start, spec §6), at the position itself while stopped.
pub fn in_force(map: &SongMap, time: f64, playing: bool, tempo: Option<f64>) -> InForce {
    let ahead = if playing { lookahead_beats(tempo) } else { 0.0 };
    let at = time + ahead;
    (song_at(map, at), key_at(map, at).map(|(_, key)| key))
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

/// Why the key in force changed (the `key` record's `why`; spec §7's
/// names).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyWhy {
    /// A song other than the last one in force (a song start, a jump into
    /// it), keyed or not.
    Song,
    /// The same song, another key (a key clip passed, a jump back before
    /// it).
    Change,
    /// Into a keyless song from out of every song: no key in force.
    NoKey,
    /// Out of every song.
    Span,
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
            KeyWhy::Span => "span",
            KeyWhy::Found => "found",
            KeyWhy::Retry => "retry",
        }
    }
}

/// Why the position went from `before` to `now`, if anything changed:
/// another key in the same song is a change; another song entered from a
/// song is a song start, keyed or not; a song entered from out of every
/// song is a song start when it has a key, else no key; out of every song
/// after a song is `span`.
fn why_of(before: InForce, now: InForce) -> Option<KeyWhy> {
    let ((was_song, was_key), (song, key)) = (before, now);
    match (was_song, song) {
        (_, None) => was_song.map(|_| KeyWhy::Span),
        (Some(was), Some(song)) if was == song => (was_key != key).then_some(KeyWhy::Change),
        (Some(_), Some(_)) => Some(KeyWhy::Song),
        (None, Some(_)) => Some(key.map_or(KeyWhy::NoKey, |_| KeyWhy::Song)),
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

    /// The position's song and key in force now (`in_force`'s answer): why
    /// it changed, if it did. Leaving every song forgets the last one, so
    /// coming back into it is a song start again.
    pub fn on_position(&mut self, now: InForce) -> Option<KeyWhy> {
        let why = why_of(self.now(), now);
        (self.song, self.key) = now;
        why
    }

    /// The song and key in force, as last told.
    pub fn now(&self) -> InForce {
        (self.song, self.key)
    }

    /// A reconnect forgets the last song: its next position is a start.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// The value to write for step `index` of `steps` (spec §5, A.4): the
/// step's centre `index / steps`; when Live already holds it (`current`
/// within [`SAME_VALUE_EPS`] of it: a rewrite of an equal value may never
/// reach a remote plug-in), a nudge ([`NUDGE`] of a step) above it, or below
/// it for the last step. Both read back as the same step (half up).
pub fn write_value(index: u8, steps: u8, current: Option<f64>) -> f64 {
    let (i, n) = (f64::from(index), f64::from(steps));
    let centre = i / n;
    if !current.is_some_and(|c| (c - centre).abs() < SAME_VALUE_EPS) {
        return centre;
    }
    let beside = if index == steps { i - NUDGE } else { i + NUDGE };
    beside / n
}

/// The Key and Scale values to write for `key` against Live's `current`
/// ones: its root over [`KEY_STEPS`], Major (0) or Minor (1) over
/// [`SCALE_STEPS`], each by [`write_value`].
pub fn key_values(key: Key, current: (Option<f64>, Option<f64>)) -> (f64, f64) {
    (
        write_value(key.root, KEY_STEPS, current.0),
        write_value(u8::from(key.minor), SCALE_STEPS, current.1),
    )
}

/// What a target's write and its read-back came to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Readback {
    /// The values read back show the key written.
    Confirmed,
    /// They show another key, or none (Chromatic, a step out of range).
    Shows(Option<Key>),
    /// Live refused the write or the read (the script's words: a Max map
    /// holds the parameter, a stale `$ref`, Live offline).
    Refused(String),
}

/// The read-back of `values` (Key, Scale) against the key `want` written.
pub fn judge(want: Key, values: (f64, f64)) -> Readback {
    let seen = shows(values.0, values.1);
    if seen == Some(want) {
        Readback::Confirmed
    } else {
        Readback::Shows(seen)
    }
}

/// One target's key writes (spec §5): the key wanted there (the last one
/// written), whether its one retry was spent on that key, whether a write
/// is on its way (until its read-back), the last outcome, and whether
/// another writer changed it since the hub confirmed it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyTarget {
    want: Option<Key>,
    retried: bool,
    writing: bool,
    last: Option<Readback>,
    another: bool,
}

impl KeyTarget {
    /// Nothing written yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The key to write now: another key than the one wanted gets its own
    /// retry. The caller writes either way (a song start and a target found
    /// write an equal key too).
    pub fn want(&mut self, key: Key) {
        if self.want != Some(key) {
            self.retried = false;
        }
        self.want = Some(key);
    }

    /// The key wanted there, if any.
    pub fn wanted(&self) -> Option<Key> {
        self.want
    }

    /// A write went out: it is on its way until its outcome, and what
    /// another writer did before it is past.
    pub fn sent(&mut self) {
        self.writing = true;
        self.another = false;
    }

    /// The write's outcome (its refusal, or the read-back's judgement): the
    /// write is over; `Some(Retry)` once per key when it is not confirmed
    /// (once at once, then at the next key; never a write loop).
    pub fn written(&mut self, outcome: Readback) -> Option<KeyWhy> {
        self.writing = false;
        let retry = outcome != Readback::Confirmed && !self.retried;
        self.last = Some(outcome);
        if retry {
            self.retried = true;
        }
        retry.then_some(KeyWhy::Retry)
    }

    /// Whether `values` heard between writes are another writer's: no write
    /// on its way, the key wanted was confirmed there, and the values no
    /// longer show it. A refused or mismatched target stays as it is until
    /// its next write.
    pub fn heard(&self, values: (f64, f64)) -> bool {
        !self.writing
            && self.last == Some(Readback::Confirmed)
            && self
                .want
                .is_some_and(|want| judge(want, values) != Readback::Confirmed)
    }

    /// Another writer was recorded: shown until the next write.
    pub fn another(&mut self) {
        self.another = true;
    }

    /// Where the key stands.
    pub fn state(&self) -> KeyState {
        if self.want.is_none() {
            return KeyState::NoKey;
        }
        if self.writing {
            return KeyState::Writing;
        }
        if self.another {
            return KeyState::AnotherWriter;
        }
        match self.last {
            Some(Readback::Confirmed) => KeyState::Confirmed,
            Some(Readback::Shows(_)) => KeyState::Mismatch,
            Some(Readback::Refused(_)) => KeyState::Refused,
            None => KeyState::Writing,
        }
    }
}

/// A key state's word (as `/api/status` names it).
pub fn state_word(state: KeyState) -> &'static str {
    match state {
        KeyState::NoKey => "no_key",
        KeyState::Writing => "writing",
        KeyState::Confirmed => "confirmed",
        KeyState::Mismatch => "mismatch",
        KeyState::Refused => "refused",
        KeyState::AnotherWriter => "another_writer",
    }
}

/// Whether the playhead crossed a key change's start `change` in continuous
/// play (spec §6; PR #144 review M1, as ruled): the position heard before
/// (`before`), one lookahead (`ahead`) on, was not yet at it, and the step
/// to `time` went forward by at most [`CROSSING_BEATS`]. A jump or a locate
/// past it, playing or not, is no crossing and has no lateness.
pub fn crossed(before: Option<f64>, time: f64, change: f64, ahead: f64) -> bool {
    before.is_some_and(|b| b + ahead < change && time > b && time - b <= CROSSING_BEATS)
}

/// How late a key change was written, in ms after its clip's start (spec
/// §6): the playhead `time` at the decision against the change's start
/// `change` (beats) at `tempo` (bpm), plus the write's round trip; below 0
/// when the lookahead wrote it before the start.
pub fn late_ms(time: f64, change: f64, tempo: f64, rtt_ms: f64) -> f64 {
    (time - change) * 60_000.0 / tempo + rtt_ms
}

/// The key Live's bar shows (spec A.5): `root_note` C = 0 to B = 11 and
/// `scale_name` "Major" or "Minor"; none for anything else (another scale,
/// a value missing).
pub fn bar_shows(root: Option<u64>, scale: Option<&str>) -> Option<Key> {
    let root = u8::try_from(root?).ok().filter(|r| *r <= 11)?;
    let minor = match scale? {
        "Major" => false,
        "Minor" => true,
        _ => return None,
    };
    Some(Key { root, minor })
}

/// What Live's bar's read-back came to: the key written, or something
/// else (it gates nothing; spec §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BarState {
    Confirmed,
    Shows,
}

/// The bar's read-back of `shows` against the key `want` written.
pub fn bar_state(want: Key, shows: Option<Key>) -> BarState {
    if shows == Some(want) {
        BarState::Confirmed
    } else {
        BarState::Shows
    }
}

#[cfg(test)]
#[path = "keys_tests.rs"]
mod tests;
