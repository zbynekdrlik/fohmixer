//! Live's track meter and pan law as probe P2 measured them on the PC
//! (#151, spec §3.4, §4.5; the main session, 27.10), and the post-fader
//! level of a `FohMixer Meter` reading: the device sits at the end of an
//! armed track's chain, before Live's mixer, so the hub adds the fader's dB
//! and the pan's gain itself and turns the sum into the 0..1 value the strip
//! draws.
//!
//! - **The scale:** Live's `output_meter_left` / `_right` of a signal whose
//!   peak is `d` dBFS reads `(d + 70) / 76`, linear in dB, 0 at or below
//!   −70 dB, 1 at +6 dB ([`live_value`]; every measured point within 0.001,
//!   kept as the tests' vectors).
//! - **The pan law:** constant power, normalised to unity at the centre:
//!   `√2·cos((p+1)·π/4)` on the left and `√2·sin((p+1)·π/4)` on the right,
//!   `p` from −1 to +1 ([`pan_gains_db`]; measured: centre 0 dB both, −1
//!   L +3.0 dB and R silent, −0.5 L +2.32 dB and R −5.33 dB, mirrored).

use crate::volume::value2db;

/// The meter's floor: a peak at or below it reads 0.
pub const FLOOR_DB: f64 = -70.0;
/// The meter's top: a peak at or above it reads 1.
pub const TOP_DB: f64 = 6.0;
/// A `FohMixer Meter` reading at or below this (tenths of dBFS) is
/// silence: the device's floor, whatever the fader adds.
pub const SILENT_TENTHS: i16 = -700;

/// Live's meter value (0..1) of a peak of `db` dBFS: linear from
/// [`FLOOR_DB`] (0) to [`TOP_DB`] (1), clamped; NaN or −∞ reads 0.
pub fn live_value(db: f64) -> f64 {
    let value = (db - FLOOR_DB) / (TOP_DB - FLOOR_DB);
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// A pan gain at or below this (linear) is silence: `cos(π/2)` is 6e-17 in
/// floating point, not 0, so a hard-panned side would read −321 dB.
pub const SILENT_GAIN: f64 = 1e-6;

/// Live's pan law: each side's gain in dB at pan `pan` (−1 left .. +1
/// right, clamped); a silent side (a gain at or below [`SILENT_GAIN`]) is
/// −∞.
pub fn pan_gains_db(pan: f64) -> (f64, f64) {
    let pan = if pan.is_nan() {
        0.0
    } else {
        pan.clamp(-1.0, 1.0)
    };
    let angle = (pan + 1.0) * std::f64::consts::FRAC_PI_4;
    (gain_db(angle.cos()), gain_db(angle.sin()))
}

/// One side's pan gain in dB: `√2·g`, silence at or below [`SILENT_GAIN`].
fn gain_db(g: f64) -> f64 {
    let gain = std::f64::consts::SQRT_2 * g;
    if gain <= SILENT_GAIN {
        f64::NEG_INFINITY
    } else {
        20.0 * gain.log10()
    }
}

/// One side's meter value: the device's peak (tenths of dBFS) plus the
/// fader's and the pan's dB; the device's silence stays silence.
fn side(tenths: i16, fader_db: f64, pan_db: f64) -> f64 {
    if tenths <= SILENT_TENTHS {
        return 0.0;
    }
    live_value(f64::from(tenths) / 10.0 + fader_db + pan_db)
}

/// The meter values Live would show after the track's mixer for a meter
/// device's L/R peaks (`dev_l`, `dev_r`, tenths of dBFS), the track's
/// volume value (0..1), its pan (−1..+1) and mute: `(0, 0)` when muted.
pub fn post_fader(dev_l: i16, dev_r: i16, volume_value: f64, pan: f64, muted: bool) -> (f64, f64) {
    if muted {
        return (0.0, 0.0);
    }
    let fader_db = value2db(volume_value);
    let (pan_l, pan_r) = pan_gains_db(pan);
    (side(dev_l, fader_db, pan_l), side(dev_r, fader_db, pan_r))
}

#[cfg(test)]
#[path = "meter/tests.rs"]
mod tests;
