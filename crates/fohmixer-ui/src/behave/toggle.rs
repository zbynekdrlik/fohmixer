//! The former MIDI toggles (spec F17, F18, D10): a toggle writes its
//! targets' `on`/`off` values directly and shows their real state. Its
//! press mode is the TouchOSC button's (S3 design note §5):
//!
//! - `toggle`: a press turns every target on, or all off when all are on;
//! - `double_tap_latch`: only two presses within 200 ms flip it;
//! - `pulse_and_double_tap_latch`: a press turns it on and the release off
//!   again (a pulse), two presses within 200 ms latch it on, and a press
//!   while it is on turns it off.

use fohmixer_proto::layout::Press;
use serde_json::Value;

/// Two presses closer than this are a double tap.
pub const DOUBLE_TAP_MS: f64 = 200.0;
/// How close a number must be to the target's `on` value to count as on.
const EPSILON: f64 = 1e-6;

/// What a toggle's targets show together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleState {
    /// Every target is on.
    On,
    /// Every target is off.
    Off,
    /// Some are on, some are not.
    Mixed,
    /// A target has no value from Live (yet).
    Unknown,
}

/// A write to every target of a toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Write {
    On,
    Off,
}

/// Whether Live's `value` is the target's `on` value (numbers within 1e-6,
/// anything else exactly).
pub fn is_on(value: &Value, on: &Value) -> bool {
    match (value.as_f64(), on.as_f64()) {
        (Some(v), Some(o)) => within_epsilon(v - o),
        _ => value == on,
    }
}

/// Whether a difference is small enough to be the same number.
fn within_epsilon(diff: f64) -> bool {
    diff.abs() < EPSILON
}

/// The state of a toggle whose targets are on (`Some(true)`), off
/// (`Some(false)`) or have no value (`None`).
pub fn aggregate(targets: &[Option<bool>]) -> ToggleState {
    if targets.is_empty() || targets.iter().any(Option::is_none) {
        ToggleState::Unknown
    } else if targets.iter().all(|t| *t == Some(true)) {
        ToggleState::On
    } else if targets.iter().all(|t| *t == Some(false)) {
        ToggleState::Off
    } else {
        ToggleState::Mixed
    }
}

/// One toggle's press state.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ToggleCtl {
    last_press: Option<f64>,
    pulsing: bool,
}

impl ToggleCtl {
    /// Whether a press at `now` completes a double tap (and forgets it).
    fn double(&mut self, now: f64) -> bool {
        let double = self.last_press.is_some_and(|t| now - t < DOUBLE_TAP_MS);
        self.last_press = if double { None } else { Some(now) };
        double
    }

    /// A press at `now` on a toggle showing `state`: the write, if any.
    pub fn down(&mut self, press: Press, state: ToggleState, now: f64) -> Option<Write> {
        let flip = if state == ToggleState::On {
            Write::Off
        } else {
            Write::On
        };
        match press {
            Press::Toggle => Some(flip),
            Press::DoubleTapLatch => self.double(now).then_some(flip),
            Press::PulseAndDoubleTapLatch => {
                if self.double(now) {
                    self.pulsing = false;
                    Some(Write::On)
                } else {
                    self.pulsing = flip == Write::On;
                    Some(flip)
                }
            }
        }
    }

    /// The release: the end of a pulse, if one is running.
    pub fn up(&mut self) -> Option<Write> {
        let pulsing = self.pulsing;
        self.pulsing = false;
        pulsing.then_some(Write::Off)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_value_is_on_when_it_is_the_on_value() {
        assert!(is_on(&json!(false), &json!(false)));
        assert!(!is_on(&json!(true), &json!(false)));
        assert!(is_on(&json!(127.0), &json!(127)));
        assert!(is_on(&json!(1.0000005), &json!(1.0)));
        assert!(!is_on(&json!(1.000002), &json!(1.0)));
        assert!(!is_on(&json!(0.999998), &json!(1.0)));
        assert!(!is_on(&json!(0.0), &json!(1.0)));
        assert!(!is_on(&Value::Null, &json!(1.0)));
        assert!(!is_on(&json!("on"), &json!(1.0)));
        assert!(!within_epsilon(1e-6) && !within_epsilon(-1e-6));
        assert!(within_epsilon(9.999999999999997e-7) && within_epsilon(-9.999999999999997e-7));
    }

    #[test]
    fn the_state_is_on_off_mixed_or_unknown() {
        assert_eq!(aggregate(&[Some(true), Some(true)]), ToggleState::On);
        assert_eq!(aggregate(&[Some(false), Some(false)]), ToggleState::Off);
        assert_eq!(aggregate(&[Some(true), Some(false)]), ToggleState::Mixed);
        assert_eq!(aggregate(&[Some(false), Some(true)]), ToggleState::Mixed);
        assert_eq!(aggregate(&[Some(true), None]), ToggleState::Unknown);
        assert_eq!(aggregate(&[None]), ToggleState::Unknown);
        assert_eq!(aggregate(&[]), ToggleState::Unknown);
        assert_eq!(aggregate(&[Some(true)]), ToggleState::On);
        assert_eq!(aggregate(&[Some(false)]), ToggleState::Off);
    }

    #[test]
    fn toggle_turns_all_on_unless_all_are_on() {
        let mut t = ToggleCtl::default();
        assert_eq!(
            t.down(Press::Toggle, ToggleState::Off, 0.0),
            Some(Write::On)
        );
        assert_eq!(
            t.down(Press::Toggle, ToggleState::Mixed, 1.0),
            Some(Write::On)
        );
        assert_eq!(
            t.down(Press::Toggle, ToggleState::On, 2.0),
            Some(Write::Off)
        );
        assert_eq!(t.up(), None);
    }

    #[test]
    fn double_tap_latch_ignores_single_presses() {
        let mut t = ToggleCtl::default();
        let press = Press::DoubleTapLatch;
        assert_eq!(t.down(press, ToggleState::Off, 0.0), None);
        assert_eq!(t.up(), None);
        assert_eq!(t.down(press, ToggleState::Off, 199.0), Some(Write::On));
        assert_eq!(t.down(press, ToggleState::On, 300.0), None);
        assert_eq!(
            t.down(press, ToggleState::On, 500.0),
            None,
            "200 ms is too slow"
        );
        assert_eq!(t.down(press, ToggleState::On, 650.0), Some(Write::Off));
        assert_eq!(
            t.down(press, ToggleState::Off, 700.0),
            None,
            "a double tap resets"
        );
    }

    #[test]
    fn a_pulse_press_turns_on_until_the_release() {
        let mut t = ToggleCtl::default();
        let press = Press::PulseAndDoubleTapLatch;
        assert_eq!(t.down(press, ToggleState::Off, 0.0), Some(Write::On));
        assert_eq!(t.up(), Some(Write::Off));
        assert_eq!(t.up(), None, "one release, one off");
    }

    #[test]
    fn a_double_tap_latches_the_pulse_and_a_press_unlatches() {
        let mut t = ToggleCtl::default();
        let press = Press::PulseAndDoubleTapLatch;
        assert_eq!(t.down(press, ToggleState::Off, 0.0), Some(Write::On));
        assert_eq!(t.up(), Some(Write::Off));
        assert_eq!(t.down(press, ToggleState::Off, 150.0), Some(Write::On));
        assert_eq!(t.up(), None, "latched");
        assert_eq!(t.down(press, ToggleState::On, 1000.0), Some(Write::Off));
        assert_eq!(t.up(), None, "an unlatch is no pulse");
        assert_eq!(t.down(press, ToggleState::Mixed, 2000.0), Some(Write::On));
        assert_eq!(t.up(), Some(Write::Off), "a mixed state pulses");
    }
}
