//! The strip meter (spec F13; TouchOSC `meter_script.lua` 2.5.2): Live's
//! meter level placed on the fader scale, a 300 ms ease-out rise and a
//! 200 ms linear fall, green / yellow / red by the level's dB on the fader
//! scale (≥ −3 red, ≥ −12 yellow), the colour smoothed by 0.3 per frame.

use super::Rgb;
use super::fader::{to_live, value2db};

/// TouchOSC's calibration: Live's meter level → position on the fader
/// scale, linear between the points. (TouchOSC also snapped any level
/// within 0.01 of a point to that point; the interpolation already passes
/// through every point, so the snap is not reproduced.)
const CALIBRATION: [(f64, f64); 7] = [
    (0.0, 0.0),
    (0.3945, 0.027),
    (0.6839, 0.169),
    (0.7629, 0.313),
    (0.8399, 0.5),
    (0.92, 0.729),
    (1.0, 1.0),
];
/// The rise and fall times of the bar.
pub const RISE_MS: f64 = 300.0;
pub const FALL_MS: f64 = 200.0;
/// The colours and their thresholds (dB on the fader scale).
pub const GREEN: Rgb = [0.0, 204.0, 0.0];
pub const YELLOW: Rgb = [255.0, 204.0, 0.0];
pub const RED: Rgb = [255.0, 0.0, 0.0];
const RED_DB: f64 = -3.0;
const YELLOW_DB: f64 = -12.0;
/// The colour moves this share of the way to its target per 60 Hz frame.
const COLOR_SMOOTHING: f64 = 0.3;
const FRAME_MS: f64 = 1000.0 / 60.0;

/// The fader-scale position of Live's meter level `level` (0..1).
pub fn level_to_pos(level: f64) -> f64 {
    let level = level.clamp(0.0, 1.0);
    // The segment that holds the level: after the points below it (at a
    // point both neighbouring segments give the point's own position). NaN
    // sorts above every point (total_cmp): the last segment, a NaN position.
    let above = CALIBRATION.partition_point(|&(x, _)| x.total_cmp(&level).is_lt());
    let i = above.clamp(1, CALIBRATION.len() - 1);
    let ((x0, y0), (x1, y1)) = (CALIBRATION[i - 1], CALIBRATION[i]);
    y0 + (level - x0) / (x1 - x0) * (y1 - y0)
}

/// The colour of a level in dB on the fader scale.
pub fn color_for_db(db: f64) -> Rgb {
    if db >= RED_DB {
        RED
    } else if db >= YELLOW_DB {
        YELLOW
    } else {
        GREEN
    }
}

/// The colour of a bar at fader position `pos`.
pub fn color_for_pos(pos: f64) -> Rgb {
    color_for_db(value2db(to_live(pos)))
}

/// `current` moved toward `target` for a frame of `dt_ms` (time-based: two
/// 120 Hz frames move as far as one 60 Hz frame).
pub fn smooth(current: Rgb, target: Rgb, dt_ms: f64) -> Rgb {
    let share = 1.0 - (1.0 - COLOR_SMOOTHING).powf(dt_ms / FRAME_MS);
    [0, 1, 2].map(|i| current[i] + (target[i] - current[i]) * share)
}

/// One meter bar: its animation toward the latest level and its colour.
#[derive(Debug, Clone, PartialEq)]
pub struct MeterBar {
    from: f64,
    to: f64,
    elapsed: f64,
    shown: f64,
    color: Rgb,
}

impl Default for MeterBar {
    fn default() -> Self {
        Self {
            from: 0.0,
            to: 0.0,
            elapsed: 0.0,
            shown: 0.0,
            color: GREEN,
        }
    }
}

impl MeterBar {
    /// A new position to animate to (a new level from Live); the same
    /// target again changes nothing.
    pub fn target(&mut self, to: f64) {
        if to != self.to {
            self.from = self.shown;
            self.to = to;
            self.elapsed = 0.0;
        }
    }

    /// Advances `dt_ms`: the position and colour to show.
    pub fn step(&mut self, dt_ms: f64) -> (f64, Rgb) {
        self.elapsed += dt_ms;
        let gap = self.to - self.from;
        let rising = gap.is_sign_positive();
        let duration = if rising { RISE_MS } else { FALL_MS };
        let linear = (self.elapsed / duration).min(1.0);
        let progress = if rising {
            1.0 - (1.0 - linear).powi(2)
        } else {
            linear
        };
        self.shown = self.from + gap * progress;
        self.color = smooth(self.color, color_for_pos(self.shown), dt_ms);
        (self.shown, self.color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behave::fader::to_pos;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn levels_are_placed_by_the_touchosc_calibration() {
        assert_eq!(level_to_pos(0.0), 0.0);
        assert_eq!(level_to_pos(-1.0), 0.0);
        assert_eq!(level_to_pos(1.0), 1.0);
        assert_eq!(level_to_pos(2.0), 1.0);
        for (x, y) in CALIBRATION {
            assert!(close(level_to_pos(x), y), "{x}");
        }
        // Halfway between two points.
        assert!(close(level_to_pos(0.87995), 0.6145));
        assert!(close(level_to_pos(0.19725), 0.0135));
        assert!(close(level_to_pos(0.96), 0.8645));
        // Not a number: no position (and no panic).
        assert!(level_to_pos(f64::NAN).is_nan());
    }

    #[test]
    fn the_colour_thresholds_are_minus_3_and_minus_12_db() {
        assert_eq!(color_for_db(-2.0), RED);
        assert_eq!(color_for_db(-3.0), RED);
        assert_eq!(color_for_db(-10.0), YELLOW);
        assert_eq!(color_for_db(-12.0), YELLOW);
        assert_eq!(color_for_db(-20.0), GREEN);
        assert_eq!(color_for_db(f64::NEG_INFINITY), GREEN);
        // On the fader scale: 0 dB (Live 0.85) is red, −6 dB yellow.
        assert_eq!(color_for_pos(to_pos(0.85)), RED);
        assert_eq!(color_for_pos(to_pos(0.7)), YELLOW);
        assert_eq!(color_for_pos(0.1), GREEN);
    }

    #[test]
    fn the_colour_moves_three_tenths_per_60_hz_frame() {
        let one = smooth(GREEN, RED, FRAME_MS);
        assert!(close(one[0], 76.5) && close(one[1], 142.8) && close(one[2], 0.0));
        let half = smooth(GREEN, RED, FRAME_MS / 2.0);
        let two = smooth(half, RED, FRAME_MS / 2.0);
        assert!(two.iter().zip(one).all(|(a, b)| close(*a, b)), "{two:?}");
        assert_eq!(smooth(RED, RED, 5.0), RED);
        // Three 60 Hz frames in one 50 ms frame: 1 − 0.7³ of the way.
        let three = smooth(GREEN, RED, 50.0);
        assert!(
            close(three[0], 167.535) && close(three[1], 69.972) && close(three[2], 0.0),
            "{three:?}"
        );
    }

    #[test]
    fn a_rise_takes_300_ms_easing_out() {
        let mut bar = MeterBar::default();
        bar.target(0.8);
        let (p, _) = bar.step(150.0);
        assert!(close(p, 0.8 * 0.75), "{p}");
        let (p, _) = bar.step(149.0);
        assert!(p < 0.8 && p > 0.79, "{p}");
        let (p, _) = bar.step(1.0);
        assert_eq!(p, 0.8);
        // The same level again does not restart the animation.
        bar.target(0.8);
        assert_eq!(bar.step(1.0).0, 0.8);
    }

    #[test]
    fn a_fall_takes_200_ms_linearly_from_where_the_bar_is() {
        let mut bar = MeterBar::default();
        bar.target(0.8);
        bar.step(300.0);
        bar.target(0.4);
        assert!(close(bar.step(100.0).0, 0.6), "halfway after 100 ms");
        assert!(close(bar.step(50.0).0, 0.5));
        assert_eq!(bar.step(50.0).0, 0.4);
        // A new target mid-way starts from the shown position.
        bar.target(0.6);
        assert!(close(bar.step(150.0).0, 0.4 + 0.2 * 0.75));
    }

    #[test]
    fn the_bar_colour_follows_its_level() {
        let mut bar = MeterBar::default();
        bar.target(to_pos(0.85));
        let mut color = GREEN;
        for _ in 0..60 {
            color = bar.step(FRAME_MS).1;
        }
        assert!(
            (color[0] - 255.0).abs() < 0.001 && color[1] < 0.001,
            "{color:?}"
        );
    }
}
