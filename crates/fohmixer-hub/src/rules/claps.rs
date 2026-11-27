//! Claps from the surface (#220; the design comment on #220 of 2026-11-16,
//! mockup `docs/mockups/claps-v2.html`): the pure rule. The glue is
//! `router/claps.rs`; the `FohMixer Claps` device plays what this decides
//! on Live's own grid (`fohmixer_proto::claps`).
//!
//! - A tap of the Claps strip's TAP is a moment of the page's clock, mapped
//!   to the hub's (`clock.rs`), then to Live's song position along the
//!   playhead and tempo last heard ([`position_at`]): the timing stays
//!   Live's, the taps only choose the beats (the corrected findings, comment
//!   6909085169).
//! - Taps gather while the engineer taps (the device is silent then); a bar
//!   with no tap ends the tapping ([`Claps::position`]). The taps are judged
//!   together (#278, [`pattern_of`]): the period from their median interval
//!   (below [`EVERY_BEAT_MAX`] every beat, below [`ONE_A_BAR_MIN`] a beat in
//!   two, else one a bar), the phase from the circular mean of their
//!   positions less the eye's lag [`TAP_LAG_BEATS`]; one tap alone is its
//!   own snapped beat ([`beat_of`]). A tap once a bar while Live plays is a
//!   tapping of its own (the bar after it passes before the next, late tap
//!   comes), so one a bar serves taps 3 to 4 beats apart. An engineer tapping by eye taps late
//!   and uneven, so a per-tap snap scattered over neighbouring beats. The
//!   pattern goes to the device, switched on, which starts it at the next
//!   bar (its own `@quantize`). A new tapping replaces the pattern.
//! - A beat dot switches one beat ([`Claps::toggle`]; a tapping ends first):
//!   set while nothing claps, it starts that beat from the next bar; the
//!   last beat cleared stops it. A 1 s hold of TAP stops everything
//!   ([`Claps::stop`]).
//! - One clap source: while an active arrangement clip of the Claps track
//!   covers the playhead (half a beat ahead), the device is silent
//!   ([`covers`]); the clip claps.
//! - 4/4 only: another time signature claps nothing and takes no tap.

use fohmixer_proto::claps::{BEATS, ClapsPhase, ClapsWords, Pattern};

/// A bar, in beats (4/4 only).
pub const BAR: f64 = 4.0;
/// A playhead heard longer ago than this (ms) gives no tap a position:
/// Live sends it some 30 times a second while it plays.
pub const STALE_MS: f64 = 2000.0;
/// No beat claps.
pub const NONE: Pattern = [false; BEATS];
/// How far ahead of the playhead heard an active clip silences the device
/// (beats) while Live plays: the playhead is up to a script tick old when
/// it arrives, and the word takes the router and UDP to the device, so a
/// tick ahead could let the device clap a clip's first beat too. The
/// device claps whole beats only, so half a beat early silences no beat
/// before the clip, and half a beat before the clip's end (a bar line)
/// none of the clip's.
pub const SILENCE_AHEAD_BEATS: f64 = 0.5;

/// A tapping whose median interval is below this (beats) claps every beat.
pub const EVERY_BEAT_MAX: f64 = 1.4;
/// From this median interval on (beats) a tapping claps one beat a bar;
/// between the two, a beat in two (1&3 or 2&4).
pub const ONE_A_BAR_MIN: f64 = 3.0;
/// How late the engineer's eye taps after the clap he sees (beats): taken
/// off the taps' phase before it snaps to a beat.
pub const TAP_LAG_BEATS: f64 = 0.15;

/// The whole beat of the bar nearest `pos` (beats from the song's start; a
/// bar starts at a multiple of 4), 0 to 3.
pub fn beat_of(pos: f64) -> usize {
    // A song position is far below i64's range; `as` saturates anyway.
    (pos.round() as i64).rem_euclid(BEATS as i64) as usize
}

/// The median of `values` (the mean of the middle two for an even count);
/// none for none.
pub fn median(values: &[f64]) -> Option<f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mid = sorted.len() / 2;
    match sorted.len() {
        0 => None,
        n if n % 2 == 1 => Some(sorted[mid]),
        _ => Some((sorted[mid - 1] + sorted[mid]) / 2.0),
    }
}

/// The beats between two claps a tapping's median interval means: 1, 2 or
/// a bar.
pub fn period_of(median: f64) -> usize {
    if median < EVERY_BEAT_MAX {
        1
    } else if median < ONE_A_BAR_MIN {
        2
    } else {
        BEATS
    }
}

/// The beat (0 to `period` − 1) the `taps` fall on together: the circular
/// mean of their positions modulo the period, less [`TAP_LAG_BEATS`],
/// snapped to the nearest whole beat. Accepted (#278 review): taps spread
/// evenly round the period (1.5 beats apart in a period of 2) cancel out
/// and the phase is whatever the sums' rounding leaves; a clapper's steady
/// taps never do that.
pub fn phase_of(taps: &[f64], period: usize) -> usize {
    let span = period as f64;
    let (sin, cos) = taps.iter().fold((0.0, 0.0), |(sin, cos), &pos| {
        let angle = std::f64::consts::TAU * pos.rem_euclid(span) / span;
        (sin + angle.sin(), cos + angle.cos())
    });
    let mean = f64::atan2(sin, cos) * span / std::f64::consts::TAU;
    ((mean - TAP_LAG_BEATS).round() as i64).rem_euclid(period as i64) as usize
}

/// The pattern a tapping's `taps` (playhead positions, in order) mean,
/// judged together (#278): the period from the median interval between
/// consecutive taps (only forward ones), the phase from all the taps. A
/// tapping with no forward interval (one tap) is its last tap's beat.
pub fn pattern_of(taps: &[f64]) -> Pattern {
    let intervals: Vec<f64> = taps
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|gap| *gap > 0.0)
        .collect();
    let Some(median) = median(&intervals) else {
        let mut pattern = NONE;
        if let Some(&last) = taps.last() {
            pattern[beat_of(last)] = true;
        }
        return pattern;
    };
    let period = period_of(median);
    let phase = phase_of(taps, period);
    std::array::from_fn(|beat| beat % period == phase)
}

/// The bar `pos` lies in (0 for the first).
pub fn bar_of(pos: f64) -> i64 {
    (pos / BAR).floor() as i64
}

/// Live's song position at hub time `at_ms`, from the playhead `heard` at
/// `heard_ms` and the `tempo` (bpm), Live playing; none without a tempo
/// (not finite, not above 0) or from a playhead older than [`STALE_MS`]
/// either way.
pub fn position_at(heard: f64, heard_ms: f64, tempo: Option<f64>, at_ms: f64) -> Option<f64> {
    let tempo = tempo.filter(|t| t.is_finite() && *t > 0.0)?;
    let dt = at_ms - heard_ms;
    (dt.abs() <= STALE_MS).then(|| heard + dt * tempo / 60_000.0)
}

/// Whether Live's time signature is 4/4; none while either part is not
/// known.
pub fn four_four(numerator: Option<f64>, denominator: Option<f64>) -> Option<bool> {
    Some(numerator? == 4.0 && denominator? == 4.0)
}

/// The signature in words for the status (`4/4`); none while not known.
pub fn signature_text(numerator: Option<f64>, denominator: Option<f64>) -> Option<String> {
    Some(format!("{}/{}", numerator?, denominator?))
}

/// One arrangement clip of the Claps track (beats; `muted`: Live's
/// "deactivated").
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clip {
    pub start: f64,
    pub end: f64,
    pub muted: bool,
}

/// Whether an active clip covers `pos`: from its start, up to its end.
pub fn covers(clips: &[Clip], pos: f64) -> bool {
    clips
        .iter()
        .any(|c| !c.muted && c.start <= pos && pos < c.end)
}

/// The active clips.
pub fn active(clips: &[Clip]) -> usize {
    clips.iter().filter(|c| !c.muted).count()
}

/// Why a tap claps nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// No `+CLAPS` track in the served layout.
    NoTrack,
    /// Live's time signature is not 4/4, or not known.
    Signature,
    /// Live does not play (or its transport is not known).
    Stopped,
    /// No playhead or tempo heard lately.
    NoPosition,
}

impl Refusal {
    /// Its words in the record and the log.
    pub fn words(self) -> &'static str {
        match self {
            Refusal::NoTrack => "no +CLAPS track",
            Refusal::Signature => "not 4/4",
            Refusal::Stopped => "Live does not play",
            Refusal::NoPosition => "no playhead heard",
        }
    }
}

/// Where the rule is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    /// Nothing claps.
    Off,
    /// The engineer taps: the last tap's position (the taps themselves are
    /// the rule's `tapped`).
    Tapping { last: f64 },
    /// The pattern is set and the device switched on: it starts at the
    /// next bar after `bar` (none: the bar is not known yet, Live stopped).
    Armed { bar: Option<i64> },
    /// The device claps the pattern.
    On,
}

/// The rule's state.
#[derive(Debug, Clone, PartialEq)]
pub struct Claps {
    mode: Mode,
    pattern: Pattern,
    playing: Option<bool>,
    four_four: Option<bool>,
    /// The positions of this tapping's taps, in order (empty outside one).
    tapped: Vec<f64>,
}

impl Default for Claps {
    fn default() -> Self {
        Self::new()
    }
}

impl Claps {
    /// Nothing claps, nothing is known of Live.
    pub fn new() -> Self {
        Self {
            mode: Mode::Off,
            pattern: NONE,
            playing: None,
            four_four: None,
            tapped: Vec::new(),
        }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The pattern the device plays (or will).
    pub fn pattern(&self) -> Pattern {
        self.pattern
    }

    /// A tap at song position `pos` (none: not known): its own snapped beat
    /// (for the record; the pattern comes from all the taps together at the
    /// tapping's end), or why it was refused.
    pub fn tap(&mut self, pos: Option<f64>) -> Result<usize, Refusal> {
        if self.four_four != Some(true) {
            return Err(Refusal::Signature);
        }
        if self.playing != Some(true) {
            return Err(Refusal::Stopped);
        }
        let pos = pos.ok_or(Refusal::NoPosition)?;
        if !matches!(self.mode, Mode::Tapping { .. }) {
            self.tapped.clear();
        }
        self.tapped.push(pos);
        self.mode = Mode::Tapping { last: pos };
        Ok(beat_of(pos))
    }

    /// The tapping ends: its taps judged together ([`pattern_of`]) are the
    /// pattern, switched on (it starts at the next bar after `pos`'s; none:
    /// not known).
    fn end_tapping(&mut self, pos: Option<f64>) {
        let pattern = pattern_of(&self.tapped);
        self.tapped.clear();
        self.pattern = pattern;
        self.mode = if pattern.contains(&true) {
            Mode::Armed {
                bar: pos.map(bar_of),
            }
        } else {
            Mode::Off
        };
    }

    /// Live's playhead heard at `pos` while it plays: a bar with no tap
    /// (or a jump back before the last tap) ends the tapping, and a new bar
    /// after the arming means the device claps. Whether the mode changed.
    pub fn position(&mut self, pos: f64) -> bool {
        let before = self.mode;
        match self.mode {
            Mode::Tapping { last } if pos >= last + BAR || pos < last - BAR => {
                self.end_tapping(Some(pos));
            }
            Mode::Armed { bar: None } if self.playing == Some(true) => {
                self.mode = Mode::Armed {
                    bar: Some(bar_of(pos)),
                };
            }
            Mode::Armed { bar: Some(bar) } if bar_of(pos) != bar => self.mode = Mode::On,
            _ => {}
        }
        self.mode != before
    }

    /// Live's transport (none: not known): a stop ends a tapping (the taps
    /// wait for the play), and an armed pattern waits for the next bar
    /// after the play's start. Whether the mode changed.
    pub fn transport(&mut self, playing: Option<bool>) -> bool {
        let before = self.mode;
        self.playing = playing;
        if playing != Some(true) {
            match self.mode {
                Mode::Tapping { .. } => self.end_tapping(None),
                Mode::Armed { .. } => self.mode = Mode::Armed { bar: None },
                Mode::Off | Mode::On => {}
            }
        }
        self.mode != before
    }

    /// Live's time signature is 4/4 (`Some(true)`), another, or not known:
    /// leaving 4/4 ends a tapping, and what claps waits (the device is
    /// switched off, so it starts again at a bar once it is back).
    pub fn signature(&mut self, four_four: Option<bool>) -> bool {
        let before = (self.mode, self.four_four);
        self.four_four = four_four;
        if four_four != Some(true) {
            match self.mode {
                Mode::Tapping { .. } => self.end_tapping(None),
                Mode::On => self.mode = Mode::Armed { bar: None },
                Mode::Off | Mode::Armed { .. } => {}
            }
        }
        (self.mode, self.four_four) != before
    }

    /// A beat dot set (`on`) or cleared: a tapping ends first (its taps are
    /// the pattern, #278), then the dot switches that beat of it; nothing
    /// clapping starts that beat (from the next bar); the last beat cleared
    /// stops. Whether anything changed; a beat past the bar is none.
    pub fn toggle(&mut self, beat: usize, on: bool) -> bool {
        if beat >= BEATS {
            return false;
        }
        let ended = matches!(self.mode, Mode::Tapping { .. });
        if ended {
            self.end_tapping(None);
        }
        self.switch(beat, on) || ended
    }

    /// A beat of what claps (or nothing) switched: whether it changed (a
    /// tapping, ended by the caller first, switches nothing).
    fn switch(&mut self, beat: usize, on: bool) -> bool {
        match self.mode {
            Mode::Off if on => {
                self.pattern = NONE;
                self.pattern[beat] = true;
                self.mode = Mode::Armed { bar: None };
                true
            }
            Mode::Off | Mode::Tapping { .. } => false,
            Mode::Armed { .. } | Mode::On => {
                if self.pattern[beat] == on {
                    return false;
                }
                self.pattern[beat] = on;
                if !self.pattern.contains(&true) {
                    self.mode = Mode::Off;
                }
                true
            }
        }
    }

    /// Nothing claps, the pattern clears. Whether anything changed.
    pub fn stop(&mut self) -> bool {
        let changed = self.mode != Mode::Off || self.pattern != NONE;
        self.mode = Mode::Off;
        self.pattern = NONE;
        self.tapped.clear();
        changed
    }

    /// Live went away: its transport and signature are not known (a
    /// tapping ends; what claps waits for them).
    pub fn offline(&mut self) {
        self.transport(None);
        self.signature(None);
    }

    /// What the device is told, `silent` while a clip covers the playhead
    /// and with the `notes` it plays: it claps only while armed or on, in
    /// 4/4.
    pub fn words(&self, silent: bool, notes: &[u8]) -> ClapsWords {
        ClapsWords {
            pattern: self.pattern,
            on: matches!(self.mode, Mode::Armed { .. } | Mode::On) && self.four_four == Some(true),
            silent,
            notes: notes.to_vec(),
        }
    }

    /// What the Claps strip shows, `silent` while a clip covers the
    /// playhead.
    pub fn phase(&self, silent: bool) -> ClapsPhase {
        if self.four_four == Some(false) {
            return ClapsPhase::Signature;
        }
        let playing = self.playing == Some(true);
        match self.mode {
            Mode::Off => ClapsPhase::Off,
            Mode::Tapping { .. } => ClapsPhase::Tapping,
            Mode::Armed { .. } if playing => ClapsPhase::NextBar,
            Mode::Armed { .. } => ClapsPhase::Waits,
            Mode::On if !playing => ClapsPhase::Waits,
            Mode::On if silent => ClapsPhase::Clip,
            Mode::On => ClapsPhase::On,
        }
    }

    /// The pattern the strip shows: while tapping, the one the taps so far
    /// would give.
    pub fn shown(&self) -> Pattern {
        match self.mode {
            Mode::Tapping { .. } => pattern_of(&self.tapped),
            _ => self.pattern,
        }
    }
}

/// Whether the pages' ring would be off by more than a quarter beat: the
/// playhead `heard` against the one told (`told`, beats, at `told_ms`, at
/// `tempo`) moved on to `at_ms`. A jump, a locate, a tempo change or a
/// loop's return is told again; a playhead never told is news.
pub fn ring_off(told: Option<(f64, f64, f64)>, heard: f64, at_ms: f64) -> bool {
    let Some((pos, told_ms, tempo)) = told else {
        return true;
    };
    let expected = pos + (at_ms - told_ms) * tempo / 60_000.0;
    (heard - expected).abs() > RING_SLACK
}

/// How far the pages' ring may drift before the playhead is told again
/// (beats).
pub const RING_SLACK: f64 = 0.25;

#[cfg(test)]
#[path = "claps/tests.rs"]
mod tests;
