//! The meter's peak hold and clip light (#21, new with the redesign; TouchOSC
//! had neither): a bar's highest position stays 1.5 s, then falls to the
//! bar; a level at the calibration's 0 dB point lights the clip light,
//! which stays lit until it is tapped.

/// How long a peak holds (ms).
pub const HOLD_MS: f64 = 1500.0;
/// How fast a peak falls after its hold (fader positions per ms).
pub const FALL_PER_MS: f64 = 0.0005;
/// Live's meter level at 0 dB on the fader scale (the meter calibration's
/// point at position 0.729): from here the clip light lights.
pub const CLIP_LEVEL: f64 = 0.92;

/// A bar's peak (positions on the fader scale).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Peak {
    held: f64,
    at: f64,
}

impl Peak {
    /// The peak shown at `now`: the held position while it holds, then
    /// falling.
    fn shown(&self, now: f64) -> f64 {
        let over = (now - self.at - HOLD_MS).max(0.0);
        (self.held - FALL_PER_MS * over).max(0.0)
    }

    /// The peak after the bar is at `bar` at `now`: a bar at or above the
    /// peak holds anew.
    pub fn step(&mut self, bar: f64, now: f64) -> f64 {
        let shown = self.shown(now);
        if bar >= shown {
            self.held = bar;
            self.at = now;
            bar
        } else {
            shown
        }
    }
}

/// Whether Live's meter level lights the clip light.
pub fn clips(level: f64) -> bool {
    level >= CLIP_LEVEL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_peak_holds_then_falls_to_the_bar() {
        let mut p = Peak::default();
        assert_eq!(p.step(0.6, 0.0), 0.6);
        // The bar drops: the peak holds 1.5 s.
        assert_eq!(p.step(0.2, 100.0), 0.6);
        assert_eq!(p.step(0.2, 1500.0), 0.6);
        // Then falls 0.0005 per ms.
        assert!((p.step(0.2, 1600.0) - 0.55).abs() < 1e-12);
        assert!((p.step(0.2, 2000.0) - 0.35).abs() < 1e-12);
        // Down to the bar, which then holds it anew.
        assert_eq!(p.step(0.2, 2400.0), 0.2);
        assert_eq!(p.step(0.1, 2500.0), 0.2);
        // A higher bar holds at once.
        assert_eq!(p.step(0.9, 2600.0), 0.9);
    }

    #[test]
    fn a_peak_never_falls_below_the_bottom() {
        let mut p = Peak::default();
        p.step(0.3, 0.0);
        assert_eq!(p.step(-1.0, 1_000_000.0), 0.0);
    }

    #[test]
    fn a_bar_equal_to_the_peak_holds_it_anew() {
        let mut p = Peak::default();
        p.step(0.6, 0.0);
        // The bar back at the peak at 1000 ms: the hold starts again there,
        // so at 2600 ms the peak has fallen 100 ms' worth, not 1100's.
        assert_eq!(p.step(0.6, 1000.0), 0.6);
        assert!((p.step(0.0, 2600.0) - 0.55).abs() < 1e-12);
    }

    #[test]
    fn the_clip_light_lights_from_0_db() {
        assert!(clips(0.92));
        assert!(clips(1.0));
        assert!(!clips(0.919_999_999_999_999_9));
        assert!(!clips(0.5));
    }
}
