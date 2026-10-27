//! Loudness on the PA mics and the PA output (#151, spec
//! `docs/superpowers/specs/2026-10-26-lufs-design.md`).
//!
//! The `FohMixer LUFS` Max for Live device measures (§3.3) and sends its
//! readings to the hub over localhost (§4.4). Its numbers are checked on the
//! PC against Sonible trueLevel's short-term on the same signal (§9, the
//! owner's ruling of 27.10), not against a reference of our own.
//!
//! - [`judge`] says how a reading looks on its strip (§6.1), [`text`] is
//!   the one-decimal number it shows (`−∞` in silence) and [`caption`] the
//!   words over it (§6.2), shared by the UI and the hub's log. Since the
//!   owner's ruling of 27.10 evening a strip behaves like trueLevel: short-
//!   term `S` everywhere, green within ±1 LU of an input point's target.
//! - [`wire`] is the devices' datagram and its parser (§4.4).
//! - [`LufsProblem`] is a loudness point's problem in words (§6.5).

use serde::{Deserialize, Serialize};

use crate::layout::LufsKind;

pub mod wire;

/// Within this many tenths of its target (±1.0 LU, inclusive) an input
/// point's reading is in the band, as Sonible trueLevel's green band (the
/// owner's ruling of 27.10).
pub const BAND_TENTHS: i16 = 10;
/// At or under this many tenths a reading is silence (−70 LUFS), shown as
/// `−∞`.
pub const SILENT_TENTHS: i16 = -700;

/// How a strip shows its reading (§6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LufsState {
    /// An input point within [`BAND_TENTHS`] of its target: green.
    In,
    /// An input point outside its band: calm.
    Off,
    /// An output point's reading: no target to judge it by, calm.
    Plain,
    /// Silence (−70 or under): shown as `−∞`, calm.
    Silent,
    /// No reading (no device, stale, offline): nothing shown.
    None,
}

impl LufsState {
    /// The state's word (the strip's `data-l`, the log).
    pub fn as_str(self) -> &'static str {
        match self {
            LufsState::In => "in",
            LufsState::Off => "off",
            LufsState::Plain => "plain",
            LufsState::Silent => "silent",
            LufsState::None => "none",
        }
    }
}

/// The text a reading in tenths shows: one decimal with a typographic
/// minus (`−25.9`, `0.0`, `1.5`), as trueLevel shows it; silence (at or
/// under [`SILENT_TENTHS`]) is `−∞`.
pub fn text(tenths: i16) -> String {
    if tenths <= SILENT_TENTHS {
        return "\u{2212}\u{221e}".to_string();
    }
    let whole = tenths.unsigned_abs() / 10;
    let tenth = tenths.unsigned_abs() % 10;
    let sign = if tenths < 0 { "\u{2212}" } else { "" };
    format!("{sign}{whole}.{tenth}")
}

/// Whether `tenths` lies within [`BAND_TENTHS`] of `target` (whole LU),
/// both ends included.
pub fn in_band(tenths: i16, target: i8) -> bool {
    (i32::from(tenths) - i32::from(target) * 10).abs() <= i32::from(BAND_TENTHS)
}

/// How a reading (`tenths`, none when there is none) looks against its
/// point's `target` (none: an output point).
pub fn judge(tenths: Option<i16>, target: Option<i8>) -> LufsState {
    let Some(tenths) = tenths else {
        return LufsState::None;
    };
    if tenths <= SILENT_TENTHS {
        return LufsState::Silent;
    }
    match target {
        None => LufsState::Plain,
        Some(target) if in_band(tenths, target) => LufsState::In,
        Some(_) => LufsState::Off,
    }
}

/// The caption's two lines over the number: `LUFS` over `IN` or `OUT`.
pub fn caption(kind: LufsKind) -> [&'static str; 2] {
    match kind {
        LufsKind::In => ["LUFS", "IN"],
        LufsKind::Out => ["LUFS", "OUT"],
    }
}

/// A loudness point's problem (spec §4.4, §5.2, §6.5), shown with the
/// strip and in `/api/status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LufsProblem {
    /// No matched LUFS device sends for the strip (3 s after its Live
    /// connected or the layout was served).
    NoLufsMeter,
    /// Two devices claim the strip: both Lives hold one at its place, or
    /// its track holds two.
    TwoLufsMeters,
    /// An armed input point without a meter device (§3.4, #151 Task 13).
    NoMeterDevice,
}

impl LufsProblem {
    /// Its words (English, #141).
    pub fn text(self) -> &'static str {
        match self {
            LufsProblem::NoLufsMeter => "no LUFS meter",
            LufsProblem::TwoLufsMeters => "two LUFS meters",
            LufsProblem::NoMeterDevice => "no meter device",
        }
    }
}

#[cfg(test)]
mod tests;
