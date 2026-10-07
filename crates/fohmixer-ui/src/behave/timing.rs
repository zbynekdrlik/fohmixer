//! Small timing rules: the TechAlert blink (spec F16).

/// Whether a blinking overlay shows at `now`: `active`, and in the first
/// half of every `2 × period_ms`.
pub fn blink_on(active: bool, now: f64, period_ms: f64) -> bool {
    active && (now / period_ms).floor().rem_euclid(2.0) < 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

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
