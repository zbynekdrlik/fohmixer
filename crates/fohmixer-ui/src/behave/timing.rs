//! Small timing rules: the REFRESH ALL debounce (spec F6) and the
//! TechAlert blink (spec F16).

/// REFRESH ALL ignores presses closer than this to the last one…
pub const REFRESH_DEBOUNCE_MS: f64 = 500.0;
/// …and flashes yellow this long when it fires.
pub const REFRESH_FLASH_MS: f64 = 300.0;
/// The automatic refresh after the app loaded.
pub const AUTO_REFRESH_MS: f64 = 1000.0;

/// A debounce: fires at most once per `REFRESH_DEBOUNCE_MS`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Debounce {
    last: Option<f64>,
}

impl Debounce {
    /// A press at `now`: whether it fires.
    pub fn fire(&mut self, now: f64) -> bool {
        if self.last.is_some_and(|t| now - t < REFRESH_DEBOUNCE_MS) {
            return false;
        }
        self.last = Some(now);
        true
    }

    /// Whether the flash of the last firing still shows at `now`.
    pub fn flashing(&self, now: f64) -> bool {
        self.last.is_some_and(|t| now - t < REFRESH_FLASH_MS)
    }
}

/// Whether a blinking overlay shows at `now`: `active`, and in the first
/// half of every `2 × period_ms`.
pub fn blink_on(active: bool, now: f64, period_ms: f64) -> bool {
    active && (now / period_ms).floor().rem_euclid(2.0) < 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_fires_at_most_every_half_second_and_flashes_300_ms() {
        let mut d = Debounce::default();
        assert!(!d.flashing(0.0));
        assert!(d.fire(1000.0));
        assert!(d.flashing(1000.0) && d.flashing(1299.0) && !d.flashing(1300.0));
        assert!(!d.fire(1499.0), "debounced");
        assert!(d.fire(1500.0));
        assert!(!d.fire(1600.0));
        assert!(d.fire(2000.0), "measured from the last firing");
    }

    #[test]
    fn the_alert_blinks_every_period_while_active() {
        assert!(blink_on(true, 0.0, 300.0));
        assert!(blink_on(true, 299.0, 300.0));
        assert!(!blink_on(true, 300.0, 300.0));
        assert!(!blink_on(true, 599.0, 300.0));
        assert!(blink_on(true, 600.0, 300.0));
        assert!(!blink_on(true, 900.0, 300.0));
        assert!(!blink_on(false, 0.0, 300.0));
        assert!(!blink_on(false, 600.0, 300.0));
    }
}
