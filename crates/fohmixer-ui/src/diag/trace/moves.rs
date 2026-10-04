//! A finger on a fader or a pan, as the flight recorder keeps it (#43 PR D):
//! where its touch started and, per frame that sends from it, the pointer
//! moves the frame carried. With the hub's `live_before` (Live's value before
//! the touch's first set) this tells a fader that jumped at its first touch
//! from a finger that moved fast, and shows a drag that stuttered (gaps
//! between the moves, a value held while the finger moved).
//!
//! - **The start** ([`touch_start`]), the `touch` event of a taken `down`:
//!   `dt` (the pointer event's own time minus the event's `t`), `c` (its
//!   coordinate along the control: `clientY` on a fader, `clientX` on a pan),
//!   `travel` (the control's travel, px), `pos` (the position the control
//!   showed), `live` (Live's value as the position the control used),
//!   `local` (it showed its own position: a finger, a glide, an open write or
//!   the post-release hold) and `from` (where the touch starts: `pos` when
//!   local, else `live`). Positions are 0..1 of the travel (a volume's Live
//!   value is p^0.515, a pan's 2p − 1).
//! - **A frame's moves** ([`Trail::take`]), one `mv` per frame that sends
//!   from the finger: `key` (the control's shown key), `p` (the pointer),
//!   `e` (`[dt, c]` of each pointer move since the last frame, `dt` the
//!   move's own time minus the record's `t`), `r` (the 1:1 finger position:
//!   `from` plus the finger's travel since the down, before the touch shaping),
//!   `s` (the position the frame sends) and `q` (the sequence of its set, the
//!   hub's `set` record). A release that sends the last move adds a last one.
//!
//! Numbers are rounded to keep the records small: ms and px to 0.1,
//! positions to 5 decimals.

use serde_json::{Value, json};

use crate::behave::Start;

/// A touch's first frames of moves stay in the recorder when its backlog is
/// over its bound (the first-touch diagnosis needs them).
pub const FIRST_MOVES: u32 = 8;

/// `x` to 0.1.
pub fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// `x` to 5 decimals.
pub fn round5(x: f64) -> f64 {
    (x * 100_000.0).round() / 100_000.0
}

/// Which way a control's position grows with its pointer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Axis {
    /// A fader: up the screen (`clientY` falls).
    #[default]
    Up,
    /// A pan: to the right (`clientX` grows).
    Right,
}

impl Axis {
    /// The positions a pointer moving from `c0` to `c` (px) travels on a
    /// control `travel` px long.
    pub fn travelled(self, c0: f64, c: f64, travel: f64) -> f64 {
        let px = match self {
            Self::Up => c0 - c,
            Self::Right => c - c0,
        };
        px / travel.max(1.0)
    }
}

/// The pointer event of a touch's down: its own time (page ms since the
/// epoch), its coordinate along the control (px) and the control's travel
/// (px).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Press {
    pub at: f64,
    pub c: f64,
    pub travel: f64,
}

/// The `touch` event of a taken down at `t`, with where it started.
pub fn touch_start(t: f64, keys: &[String], pointer: i32, press: Press, start: Start) -> Value {
    json!({
        "ev": "touch",
        "t": t,
        "what": "down",
        "keys": keys,
        "pointer": pointer,
        "dt": round1(press.at - t),
        "c": round1(press.c),
        "travel": round1(press.travel),
        "pos": round5(start.shown),
        "live": round5(start.live),
        "local": start.local,
        "from": round5(start.from),
    })
}

/// One finger's moves on one control, between its frames.
#[derive(Debug, Default)]
pub struct Trail {
    /// The finger, while it drives the control.
    pointer: Option<i32>,
    axis: Axis,
    /// Its coordinate at the down and the control's travel (px).
    c0: f64,
    travel: f64,
    /// Where the touch started (a position).
    from: f64,
    /// The moves since the last frame: (their own time, page ms since the
    /// epoch; their coordinate).
    moves: Vec<(f64, f64)>,
    /// Frames taken in this touch.
    frames: u32,
}

impl Trail {
    /// `pointer` went down at `press` and drives the control, from
    /// position `from`.
    pub fn start(&mut self, pointer: i32, axis: Axis, press: Press, from: f64) {
        *self = Self {
            pointer: Some(pointer),
            axis,
            c0: press.c,
            travel: press.travel,
            from,
            moves: Vec::new(),
            frames: 0,
        };
    }

    /// `pointer` moved to `c` at `at` (another pointer's move is not this
    /// touch's).
    pub fn moved(&mut self, pointer: i32, at: f64, c: f64) {
        if self.pointer == Some(pointer) {
            self.moves.push((at, c));
        }
    }

    /// The 1:1 finger position of coordinate `c`: where the touch started
    /// plus the finger's travel since the down (0..1).
    pub fn raw(&self, c: f64) -> f64 {
        (self.from + self.axis.travelled(self.c0, c, self.travel)).clamp(0.0, 1.0)
    }

    /// The `mv` record of the frame at `t` that sent position `sent` as the
    /// set `seq`, for the control's `key`: the moves since the last one, and
    /// whether it is one of the touch's first [`FIRST_MOVES`] (essential).
    /// None without a move (a glide, a frame of another finger).
    pub fn take(
        &mut self,
        t: f64,
        key: &str,
        sent: f64,
        seq: Option<u64>,
    ) -> Option<(Value, bool)> {
        let pointer = self.pointer?;
        let &(_, last) = self.moves.last()?;
        let moves: Vec<[f64; 2]> = self
            .moves
            .drain(..)
            .map(|(at, c)| [round1(at - t), round1(c)])
            .collect();
        self.frames += 1;
        let record = json!({
            "ev": "mv",
            "t": round1(t),
            "key": key,
            "p": pointer,
            "e": moves,
            "r": round5(self.raw(last)),
            "s": round5(sent),
            "q": seq,
        });
        Some((record, self.frames <= FIRST_MOVES))
    }

    /// `pointer`'s touch ended: its moves go (another pointer's end changes
    /// nothing).
    pub fn end(&mut self, pointer: i32) {
        if self.pointer == Some(pointer) {
            self.pointer = None;
            self.moves.clear();
        }
    }
}

#[cfg(test)]
mod tests;
