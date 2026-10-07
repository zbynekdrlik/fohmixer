//! The fader (spec F8–F10, X3): the position law, the TouchOSC touch
//! shaping, tap and double-tap detection, the glide to 0 dB, and the
//! controller that ties them to one pointer.
//!
//! Ported from the TouchOSC `fader_script.lua` 2.5.4 (`abl-touchosc`). The
//! TouchOSC fader is relative: every move event adds the finger's delta to
//! the fader's current value, and the script then reshapes that value. The
//! reference traces in the tests were produced by running that script's own
//! `applyFirstMovementScaling` under Lua 5.4.
//!
//! A touch's start (#43 PR F): the iPad's first pointer event of a touch
//! comes late and several px away from the down (median 83 ms and 6 px, up
//! to 15 px, on the FOH iPad), so it only anchors the drag; and the
//! shaping's size thresholds are TouchOSC's finger distances on that iPad
//! ([`REFERENCE_TRAVEL`]), not fractions of our shorter travel.

use fohmixer_proto::layout::FaderLaw;

use super::{Motion, Start};

/// TouchOSC's position ↔ Live volume exponent: `v = p^0.515` (−6 dB at
/// half travel).
pub const EXPONENT: f64 = 0.515;
/// Live's volume at 0 dB: the double-tap target.
pub const UNITY: f64 = 0.85;
/// The double-tap glide speed, in positions per second (TouchOSC: 0.005 per
/// frame; here time-based, so 60 and 120 Hz behave the same).
pub const GLIDE_SPEED: f64 = 0.3;
/// The post-release delay before the fader shows Live's value again, with
/// touch shaping on (TouchOSC's `delay`) and off (spec X3).
pub const HOLD_SHAPED_MS: f64 = 1000.0;
pub const HOLD_PLAIN_MS: f64 = 100.0;

/// The smallest first step of a touch, in dB.
const MIN_DB_STEP: f64 = 0.1;
/// A move larger than this (a fraction of [`REFERENCE_TRAVEL`]) bypasses
/// all shaping.
const EMERGENCY: f64 = 0.03;
/// The imported TouchOSC strips' travel on the FOH iPad, in px (710 of the
/// layout's 1640 canvas units; #43 PR F). The shaping's size thresholds are
/// fractions of it, so they are the same finger distance on a fader of any
/// travel: a shorter fader does not bypass the fine first step sooner.
pub const REFERENCE_TRAVEL: f64 = 355.0;
/// Moves after a forced first step, and the first one's scale.
const REACTION_MOVES: u32 = 3;
const REACTION_SCALE: f64 = 0.3;
const REACTION_SCALE_LAST: f64 = 0.7;
/// Moves scaled gradually from `INITIAL_SCALE` to `FINAL_SCALE`.
const SCALED_MOVES: u32 = 10;
const INITIAL_SCALE: f64 = 0.9;
const FINAL_SCALE: f64 = 1.0;
/// The extra precision factor in the linear range of the volume law
/// (Live value 0.7–1.0, about −6 to +6 dB).
const LINEAR_SCALE: f64 = 0.85;
const LINEAR_FROM: f64 = 0.7;
const LINEAR_TO: f64 = 1.0;
/// TouchOSC writes a reshaped value back only when it differs by more.
const WRITE_BACK: f64 = 0.0001;
/// The bisection steps of `audio_for_db_change`: the Lua halves [0, 1]
/// until it is at most 1e-4 wide, which takes 14 halvings.
const BISECT_STEPS: u32 = 14;

/// Tap detection (TouchOSC): a tap moves less than this and lasts less
/// than `TAP_MS`; a finger that strayed more than `STRAY` must have been
/// still for `STILL_MS` before the release.
const TAP_MOVE: f64 = 0.01;
const TAP_MS: f64 = 200.0;
const STRAY: f64 = 0.015;
const STILL_MS: f64 = 100.0;
/// Two taps between these many ms apart are a double tap.
const DOUBLE_MIN_MS: f64 = 50.0;
const DOUBLE_MAX_MS: f64 = 250.0;

/// The Live volume of fader position `p` on TouchOSC's law (clamped to
/// 0..1; the power keeps 0 and 1 where they are).
pub fn to_live(p: f64) -> f64 {
    p.clamp(0.0, 1.0).powf(EXPONENT)
}

/// The fader position of Live volume `v` on TouchOSC's law (clamped to
/// 0..1).
pub fn to_pos(v: f64) -> f64 {
    v.clamp(0.0, 1.0).powf(1.0 / EXPONENT)
}

/// How a volume fader's position maps to Live's volume (#63; the layout's
/// `config.fader_law`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VolumeLaw {
    /// Live's own: the position is Live's volume value, as Live's mixer
    /// draws its fader (0 dB at 0.85, −20 dB at 0.36, −40 dB at 0.16: close
    /// to a console fader). No conversion between the fader and Live.
    #[default]
    Live,
    /// TouchOSC's: `v = p^0.515` (0 dB at 0.7294, −40 dB at 0.024).
    TouchOsc,
}

impl VolumeLaw {
    /// The Live volume of position `p` (clamped to 0..1).
    pub fn to_live(self, p: f64) -> f64 {
        match self {
            Self::Live => p.clamp(0.0, 1.0),
            Self::TouchOsc => to_live(p),
        }
    }

    /// The position of Live volume `v` (clamped to 0..1).
    pub fn to_pos(self, v: f64) -> f64 {
        match self {
            Self::Live => v.clamp(0.0, 1.0),
            Self::TouchOsc => to_pos(v),
        }
    }

    /// This law's position of position `p` on TouchOSC's scale (the meter's
    /// calibration places Live's levels there).
    pub fn from_touchosc(self, p: f64) -> f64 {
        self.to_pos(to_live(p))
    }
}

impl From<FaderLaw> for VolumeLaw {
    fn from(law: FaderLaw) -> Self {
        match law {
            FaderLaw::Live => Self::Live,
            FaderLaw::Touchosc => Self::TouchOsc,
        }
    }
}

/// The position of `value` on a parameter's `(min, max)` range: the
/// former MIDI faders' `cc_linear` scale (spec F18, S3 design note §5).
pub fn linear_pos(value: f64, (min, max): (f64, f64)) -> f64 {
    ((value - min) / (max - min)).clamp(0.0, 1.0)
}

/// The parameter value at position `p` of its `(min, max)` range.
pub fn linear_value(p: f64, (min, max): (f64, f64)) -> f64 {
    min + p.clamp(0.0, 1.0) * (max - min)
}

/// TouchOSC's `value2db`, the dB approximation of a Live volume. Only the
/// touch shaping and the meter colour use it; every dB text on screen is
/// Live's own display string (spec X1).
pub fn value2db(v: f64) -> f64 {
    // The Lua's ranges, tested from the top: above 1 (or no number) is 0 dB.
    if v > 1.0 || v.is_nan() {
        0.0
    } else if v >= 0.4 {
        40.0 * v - 34.0
    } else if v >= 0.15 {
        -((399.751894 * v - 201.871345).powi(2) + 12630.61132) / 799.503788
    } else {
        // Spelled as the Lua computes it (gamma = 7504/5567, then 1/gamma).
        let gamma = 7504.0 / 5567.0;
        let db = 118.426374 * v.powf(1.0 / gamma) - 70.0;
        if db <= -70.0 { f64::NEG_INFINITY } else { db }
    }
}

/// The Live volume `db_change` dB away from `start` (TouchOSC's
/// `dbChangeToAudioChange`: a bisection over 0..1).
pub fn audio_for_db_change(start: f64, db_change: f64) -> f64 {
    let target = value2db(start) + db_change;
    let (mut low, mut high) = (0.0_f64, 1.0_f64);
    for _ in 0..BISECT_STEPS {
        let mid = (low + high) / 2.0;
        if value2db(mid) < target {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) / 2.0
}

/// The size of a move of `delta` positions on a fader `travel` px long, as a
/// fraction of [`REFERENCE_TRAVEL`]: the same finger distance as on
/// TouchOSC's strip (#43 PR F). The ratio first, so a travel of exactly
/// [`REFERENCE_TRAVEL`] leaves the size as it is.
fn finger_size(delta: f64, travel: f64) -> f64 {
    delta.abs() * (travel / REFERENCE_TRAVEL)
}

/// Whether a move (a fraction of [`REFERENCE_TRAVEL`]) is large enough to
/// bypass the shaping.
fn is_emergency(size: f64) -> bool {
    size > EMERGENCY
}

/// Whether a move is small enough to end an emergency.
fn is_calm(size: f64) -> bool {
    size < EMERGENCY * 0.5
}

/// Whether a change (dB) falls short of the smallest first step.
fn short_of_step(change_db: f64) -> bool {
    change_db.abs() < MIN_DB_STEP
}

/// Whether TouchOSC writes a reshaped value back to the fader (a
/// difference this large).
fn writes_back(diff: f64) -> bool {
    diff.abs() > WRITE_BACK
}

/// The TouchOSC touch shaping of one touch (spec F9, X3): a 0.1 dB first
/// step, reaction compensation after it, a gradual 0.9 → 1.0 speed over the
/// first moves (× 0.85 in the linear range), and a bypass for large moves
/// (measured as the finger distance on TouchOSC's strip, #43 PR F). It holds
/// the fader's value as TouchOSC does (`x`) and the script's own last
/// position.
#[derive(Debug, Clone, PartialEq)]
pub struct Shaper {
    enabled: bool,
    /// The volume law the dB steps are measured on.
    law: VolumeLaw,
    /// The fader's travel (px): [`REFERENCE_TRAVEL`] until told.
    travel: f64,
    x: f64,
    last: f64,
    start_audio: f64,
    first_done: bool,
    reaction: bool,
    reaction_count: u32,
    emergency: bool,
    processed: u32,
}

impl Shaper {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            law: VolumeLaw::TouchOsc,
            travel: REFERENCE_TRAVEL,
            x: 0.0,
            last: 0.0,
            start_audio: 0.0,
            first_done: false,
            reaction: false,
            reaction_count: 0,
            emergency: false,
            processed: 0,
        }
    }

    /// Measures its dB steps on volume law `law` (TouchOSC's until told,
    /// #63).
    pub fn set_law(&mut self, law: VolumeLaw) {
        self.law = law;
    }

    /// The fader's travel in px (at least 1, `FaderCtl`'s): the size
    /// thresholds are the finger distances of TouchOSC's strip on it.
    pub fn set_travel(&mut self, travel: f64) {
        self.travel = travel;
    }

    /// A touch starts at position `p0`.
    pub fn start(&mut self, p0: f64) {
        let p0 = p0.clamp(0.0, 1.0);
        self.x = p0;
        self.last = p0;
        self.start_audio = self.law.to_live(p0);
        self.first_done = false;
        self.reaction = false;
        self.reaction_count = 0;
        self.emergency = false;
        self.processed = 0;
    }

    /// The finger moved by `delta` positions: the fader's new value.
    pub fn move_by(&mut self, delta: f64) -> f64 {
        let raw = (self.x + delta).clamp(0.0, 1.0);
        if !self.enabled {
            self.x = raw;
            return raw;
        }
        let shaped = self.shape(raw);
        self.x = if writes_back(shaped - raw) {
            shaped
        } else {
            raw
        };
        self.x
    }

    /// The smallest step from where the touch started, in the direction of
    /// `delta` (never 0 here).
    fn forced(&self, delta: f64) -> f64 {
        self.law.to_pos(audio_for_db_change(
            self.start_audio,
            MIN_DB_STEP.copysign(delta),
        ))
    }

    /// `applyFirstMovementScaling` for one move to `raw`.
    fn shape(&mut self, raw: f64) -> f64 {
        let delta = raw - self.last;
        let size = finger_size(delta, self.travel);
        if size == 0.0 {
            return raw;
        }
        if !self.first_done {
            self.first_done = true;
            if is_emergency(size) {
                self.emergency = true;
                self.last = raw;
                return raw;
            }
            let start_db = value2db(self.start_audio);
            if short_of_step(value2db(self.law.to_live(raw)) - start_db) {
                let forced = self.forced(delta);
                self.reaction = true;
                self.reaction_count = 0;
                self.last = forced;
                return forced;
            }
            let mut scaled = self.last + delta * INITIAL_SCALE;
            if short_of_step(value2db(self.law.to_live(scaled)) - start_db) {
                scaled = self.forced(delta);
            }
            self.last = scaled;
            self.processed = 1;
            return scaled;
        }
        if is_emergency(size) {
            self.emergency = true;
            self.last = raw;
            return raw;
        }
        if is_calm(size) {
            self.emergency = false;
        }
        if self.emergency {
            self.last = raw;
            return raw;
        }
        // The reaction moves after a forced first step (the flag clears on
        // the last of them).
        if self.reaction {
            self.reaction_count += 1;
            let progress = f64::from(self.reaction_count - 1) / f64::from(REACTION_MOVES - 1);
            let scale = REACTION_SCALE + (REACTION_SCALE_LAST - REACTION_SCALE) * progress;
            let next = (self.last + delta * scale).clamp(0.0, 1.0);
            if self.reaction_count >= REACTION_MOVES {
                self.reaction = false;
            }
            self.last = next;
            return next;
        }
        if self.processed <= SCALED_MOVES {
            self.processed += 1;
            let progress = f64::from(self.processed - 1) / f64::from(SCALED_MOVES - 1);
            let base = INITIAL_SCALE + (FINAL_SCALE - INITIAL_SCALE) * progress;
            let audio = self.law.to_live(raw);
            let linear = (LINEAR_FROM..=LINEAR_TO).contains(&audio);
            let scale = if linear { base * LINEAR_SCALE } else { base };
            let next = (self.last + delta * scale).clamp(0.0, 1.0);
            self.last = next;
            return next;
        }
        self.last = raw;
        raw
    }
}

/// Tap and double-tap detection over a touch (TouchOSC's release check).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TapTracker {
    start_pos: f64,
    start_t: f64,
    strayed: bool,
    last_move_t: f64,
    last_tap: Option<f64>,
}

impl TapTracker {
    /// A touch starts at `pos`.
    pub fn down(&mut self, pos: f64, now: f64) {
        self.start_pos = pos;
        self.start_t = now;
        self.strayed = false;
    }

    /// The touch moved the fader to `pos`.
    pub fn moved(&mut self, pos: f64, now: f64) {
        if (pos - self.start_pos).abs() > STRAY {
            self.strayed = true;
            self.last_move_t = now;
        }
    }

    /// The touch ended at `pos`: whether it completed a double tap.
    pub fn up(&mut self, pos: f64, now: f64) -> bool {
        let still = !self.strayed || now - self.last_move_t > STILL_MS;
        let tap = (pos - self.start_pos).abs() < TAP_MOVE && now - self.start_t < TAP_MS && still;
        if !tap {
            self.last_tap = None;
            return false;
        }
        let double = self
            .last_tap
            .is_some_and(|t| now - t < DOUBLE_MAX_MS && now - t > DOUBLE_MIN_MS);
        self.last_tap = if double { None } else { Some(now) };
        double
    }
}

/// A constant-speed glide (the double tap's move to 0 dB, a pan's to the
/// centre).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glide {
    from: f64,
    to: f64,
    t0: f64,
    /// Positions per second.
    speed: f64,
}

impl Glide {
    /// A fader's glide, at [`GLIDE_SPEED`].
    pub fn new(from: f64, to: f64, t0: f64) -> Self {
        Self::at_speed(from, to, t0, GLIDE_SPEED)
    }

    /// A glide at `speed` positions per second.
    pub fn at_speed(from: f64, to: f64, t0: f64, speed: f64) -> Self {
        Self {
            from,
            to,
            t0,
            speed,
        }
    }

    /// The position at time `t` and whether the glide has arrived.
    pub fn at(&self, t: f64) -> (f64, bool) {
        let travelled = self.speed * (t - self.t0).max(0.0) / 1000.0;
        let gap = self.to - self.from;
        if travelled >= gap.abs() {
            (self.to, true)
        } else {
            (self.from + travelled.copysign(gap), false)
        }
    }
}

/// The post-release delay for a fader with shaping on or off (spec X3).
pub fn hold_ms(shaping: bool) -> f64 {
    if shaping {
        HOLD_SHAPED_MS
    } else {
        HOLD_PLAIN_MS
    }
}

/// One fader's input state, in positions (0..1; the component maps them to
/// Live's values by its law): one pointer at a time (other pointers move
/// other faders), the shaping, the double tap and its glide, the
/// post-release hold (spec I4: a touched fader shows the finger, then snaps
/// to Live's value) and, since #43 (L3), its write's intent: while the store
/// holds it open the fader shows its own position, never Live's. A touch's
/// first pointer move only anchors the drag (#43 PR F), unless the fader
/// keeps TouchOSC's first move ([`FaderCtl::anchoring`]).
#[derive(Debug, Clone, PartialEq)]
pub struct FaderCtl {
    shaper: Shaper,
    taps: TapTracker,
    glide_to: Option<f64>,
    glide: Option<Glide>,
    pointer: Option<i32>,
    last_y: f64,
    travel: f64,
    pos: f64,
    unsent: bool,
    hold: f64,
    hold_until: f64,
    /// The store holds this fader's write open (#43).
    open: bool,
    /// A glide ended since the store last heard of it.
    ended: bool,
    /// A touch's first pointer move only anchors the drag (#43 PR F).
    anchors: bool,
    /// The touch's next pointer move is its first.
    first_move: bool,
    /// The finger's travel the touch's anchored first move did not apply
    /// (positions), cut at the travel's ends as that move would have been:
    /// the tap check still counts it.
    slip: f64,
}

impl FaderCtl {
    /// A fader with shaping on or off; a double tap glides to `glide_to`
    /// (a position), or does nothing when it is `None`.
    pub fn new(shaping: bool, glide_to: Option<f64>) -> Self {
        Self {
            shaper: Shaper::new(shaping),
            taps: TapTracker::default(),
            glide_to,
            glide: None,
            pointer: None,
            last_y: 0.0,
            travel: 1.0,
            pos: 0.0,
            unsent: false,
            hold: hold_ms(shaping),
            hold_until: f64::NEG_INFINITY,
            open: false,
            ended: false,
            anchors: true,
            first_move: false,
            slip: 0.0,
        }
    }

    /// Whether a touch's first pointer move only anchors the drag (#43 PR F,
    /// on by default): the iPad's first event of a touch comes late and
    /// several px from the down, and taken as a move it jumped the cap. The
    /// imported multi-target parameter faders keep TouchOSC's first move
    /// (the owner's call on #43).
    pub fn anchoring(mut self, on: bool) -> Self {
        self.anchors = on;
        self
    }

    /// A volume fader on law `law`: the touch shaping measures its dB steps
    /// there (#63; TouchOSC's law until told, as the Lua's port).
    pub fn law(mut self, law: VolumeLaw) -> Self {
        self.shaper.set_law(law);
        self
    }

    /// Whether a finger or a glide drives the fader (#63: the strip lights
    /// while it does, above the finger).
    pub fn held(&self) -> bool {
        self.pointer.is_some() || self.glide.is_some()
    }

    /// The position the fader holds now: the finger's, a glide's, a write's
    /// or Live's last (the flight recorder's `a` after a touch's first move,
    /// #43 PR F).
    pub fn pos(&self) -> f64 {
        self.pos
    }

    /// Whether the fader shows its own position (touched, gliding, its write
    /// open, or holding after a release) at `now`.
    fn local(&self, now: f64) -> bool {
        self.pointer.is_some() || self.glide.is_some() || self.open || now < self.hold_until
    }

    /// The store's word on this fader's write at `now` (#43, L3): while its
    /// intent is open (sending, unconfirmed or not sent) the fader shows its
    /// own position and Live's value only moves the ghost, and a touch starts
    /// from the cap; when the intent closes (an ack at least as new, or
    /// another client's newer write) the post-release hold runs again from
    /// then, so Live's echo of the write lands before Live's value shows.
    pub fn intent(&mut self, open: bool, now: f64) {
        if self.open && !open {
            self.hold_until = now + self.hold;
        }
        self.open = open;
    }

    /// The open write's position (#43, §4.2: the cap shows the write): the
    /// fader takes it when no finger and no glide drive it, so a fader built
    /// while its write is open (a page switch) shows the write, not 0, and a
    /// touch starts from it. The frame loop calls it while the intent is open.
    pub fn write_at(&mut self, pos: f64) {
        if self.pointer.is_none() && self.glide.is_none() {
            self.pos = pos;
        }
    }

    /// Whether a glide ended (arrived, or stopped because Live's value went)
    /// since the last call: the store then marks its last write released.
    pub fn take_ended(&mut self) -> bool {
        std::mem::take(&mut self.ended)
    }

    /// Pointer `id` pressed at `y` (px) on a fader `travel` px long, while
    /// Live's value sits at position `live`: whether this pointer now drives
    /// the fader.
    pub fn down(&mut self, id: i32, y: f64, travel: f64, now: f64, live: f64) -> bool {
        self.press(id, y, travel, now, live).is_some()
    }

    /// [`FaderCtl::down`], with where the touch started when this pointer
    /// now drives the fader (#43 PR D): the position it showed, Live's,
    /// whether it showed its own (a finger, a glide, an open write, the
    /// hold) and the position the touch starts from.
    pub fn press(&mut self, id: i32, y: f64, travel: f64, now: f64, live: f64) -> Option<Start> {
        if self.pointer.is_some() {
            return None;
        }
        let shown = self.pos;
        let local = self.local(now);
        if !local {
            self.pos = live;
        }
        self.glide = None;
        self.pointer = Some(id);
        self.last_y = y;
        self.travel = travel.max(1.0);
        self.first_move = self.anchors;
        self.slip = 0.0;
        self.shaper.set_travel(self.travel);
        self.shaper.start(self.pos);
        self.taps.down(self.pos, now);
        Some(Start {
            shown,
            live,
            local,
            from: self.pos,
        })
    }

    /// Whether pointer `id` drives this fader (its release is this fader's).
    pub fn drives(&self, id: i32) -> bool {
        self.pointer == Some(id)
    }

    /// Pointer `id` moved to `y`: whether it drives this fader. The touch's
    /// first move only anchors the drag there (#43 PR F): nothing moves and
    /// nothing is left to send.
    pub fn moved(&mut self, id: i32, y: f64, now: f64) -> bool {
        if self.pointer != Some(id) {
            return false;
        }
        let delta = (self.last_y - y) / self.travel;
        self.last_y = y;
        if std::mem::take(&mut self.first_move) {
            // Cut where the move would have stopped: a slide past an end
            // that the next moves come back from is no tap.
            self.slip = (self.pos + delta).clamp(0.0, 1.0) - self.pos;
        } else {
            self.pos = self.shaper.move_by(delta);
            self.unsent = true;
        }
        self.taps.moved(self.tap_pos(), now);
        true
    }

    /// Where the tap check sees the fader: its position plus the slide the
    /// touch's anchored first move did not apply (`slip`, cut at the ends
    /// when it came), within the travel as TouchOSC's value is. The
    /// finger's travel counts against a tap, as the fader's own move did
    /// before #43 PR F, so a quick nudge whose one event slid the finger is
    /// no tap (two of them would glide the fader to 0 dB).
    fn tap_pos(&self) -> f64 {
        (self.pos + self.slip).clamp(0.0, 1.0)
    }

    /// Pointer `id` lifted: the position still to send, if any. A double
    /// tap starts the glide.
    pub fn up(&mut self, id: i32, now: f64) -> Option<f64> {
        let last = self.release(id, now)?;
        if self.taps.up(self.tap_pos(), now)
            && let Some(to) = self.glide_to
        {
            self.glide = Some(Glide::new(self.pos, to, now));
        }
        last
    }

    /// Pointer `id` was cancelled by the browser: a release without a tap.
    pub fn cancel(&mut self, id: i32, now: f64) -> Option<f64> {
        self.release(id, now)?
    }

    /// Ends pointer `id`'s touch: `None` when it was not this fader's, else
    /// the unsent position.
    fn release(&mut self, id: i32, now: f64) -> Option<Option<f64>> {
        if self.pointer != Some(id) {
            return None;
        }
        self.pointer = None;
        self.hold_until = now + self.hold;
        Some(self.take_unsent())
    }

    fn take_unsent(&mut self) -> Option<f64> {
        let unsent = self.unsent;
        self.unsent = false;
        unsent.then_some(self.pos)
    }

    /// The frame at `now`, Live's value sitting at position `live`
    /// (`None`: no value yet).
    pub fn frame(&mut self, now: f64, live: Option<f64>) -> Motion {
        if self.glide.is_some() && live.is_none() {
            // Live's value is gone (its instance went offline; a lost hub
            // connection keeps it, stale, since #43): the glide stops where
            // it is and sends nothing more.
            self.glide = None;
            self.ended = true;
            self.hold_until = now + self.hold;
            return Motion {
                pos: Some(self.pos),
                send: None,
            };
        }
        if let Some(glide) = self.glide {
            let (pos, done) = glide.at(now);
            self.pos = pos;
            if done {
                self.glide = None;
                self.ended = true;
                self.hold_until = now + self.hold;
            }
            return Motion {
                pos: Some(pos),
                send: Some(pos),
            };
        }
        if self.pointer.is_some() {
            return Motion {
                pos: Some(self.pos),
                send: self.take_unsent(),
            };
        }
        if self.local(now) {
            return Motion {
                pos: Some(self.pos),
                send: None,
            };
        }
        if let Some(p) = live {
            self.pos = p;
            return Motion {
                pos: Some(p),
                send: None,
            };
        }
        Motion {
            pos: None,
            send: None,
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod touch_start_tests;
