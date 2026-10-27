//! Loudness on the PA mics and the PA output (#151, spec
//! `docs/superpowers/specs/2026-10-26-lufs-design.md`).
//!
//! The `FohMixer LUFS` Max for Live device measures (§3.3) and sends its
//! readings to the hub over localhost (§4.4). Its numbers are checked on the
//! PC against Sonible trueLevel's short-term on the same signal (§9, the
//! owner's ruling of 27.10), not against a reference of our own.
//!
//! - [`judge`] says how a reading looks on its strip (§6.1), [`shown`] is
//!   the integer it shows and [`caption`] the words over it (§6.2), shared
//!   by the UI and the hub's log.
//! - [`wire`] is the devices' datagram and its parser (§4.4).

use serde::Serialize;

use crate::layout::LufsKind;

pub mod wire;

/// Within this many LU of the target a reading is fine (the owner's rule).
pub const FINE_LU: i16 = 3;
/// Past this many LU from the target a reading is far.
pub const FAR_LU: i16 = 6;
/// At or under this many tenths a reading is silence (−70 LUFS).
pub const SILENT_TENTHS: i16 = -700;

/// How a strip shows its reading (§6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LufsState {
    /// An input point within [`FINE_LU`] of its target.
    Fine,
    /// An input point within [`FAR_LU`].
    Warn,
    /// An input point further away.
    Far,
    /// An output point's reading: no target to judge it by.
    Plain,
    /// Silence (−70 or under): nothing shown.
    Silent,
    /// No reading (no device, stale, offline): nothing shown.
    None,
}

impl LufsState {
    /// The state's word (the strip's `data-l`, the log).
    pub fn as_str(self) -> &'static str {
        match self {
            LufsState::Fine => "fine",
            LufsState::Warn => "warn",
            LufsState::Far => "far",
            LufsState::Plain => "plain",
            LufsState::Silent => "silent",
            LufsState::None => "none",
        }
    }
}

/// The integer LU a reading in tenths shows: rounded half away from zero
/// (−21.5 shows −22).
pub fn shown(tenths: i16) -> i16 {
    let tenths = i32::from(tenths);
    // In i32, so i16's ends cannot overflow; the result fits i16 again.
    ((tenths + 5 * tenths.signum()) / 10) as i16
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
    let Some(target) = target else {
        return LufsState::Plain;
    };
    let off = (shown(tenths) - i16::from(target)).abs();
    if off <= FINE_LU {
        LufsState::Fine
    } else if off <= FAR_LU {
        LufsState::Warn
    } else {
        LufsState::Far
    }
}

/// The caption's two lines over the number: `LUFS` over `IN` or `OUT`.
pub fn caption(kind: LufsKind) -> [&'static str; 2] {
    match kind {
        LufsKind::In => ["LUFS", "IN"],
        LufsKind::Out => ["LUFS", "OUT"],
    }
}

#[cfg(test)]
mod tests;
