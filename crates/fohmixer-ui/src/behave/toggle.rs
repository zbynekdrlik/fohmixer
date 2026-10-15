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

use super::Down;

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

    /// A press by `by` at `now` on a toggle showing `state`: the write, if
    /// any.
    pub fn down(&mut self, _by: Down, press: Press, state: ToggleState, now: f64) -> Option<Write> {
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

    /// A pointer comes down (`by`, #124): a pulse whose press's release the
    /// browser never delivered ends when this is a primary pointer of that
    /// press's type (no other of its type is down): the press's pointer,
    /// and the pulse's `Off` goes (a pulse left on is worse than a stuck
    /// fader). None otherwise, a second finger's down included.
    pub fn missed(&mut self, _by: Down) -> Option<i32> {
        None
    }

    /// The page went hidden (#124): a pulse still running ends, its
    /// press's release lost (a hidden Home Screen app keeps no touch
    /// going): the press's pointer, and the pulse's `Off` goes.
    pub fn hidden(&mut self) -> Option<i32> {
        None
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
    use crate::diag::perf::Pointer;
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
            t.down(Down::touch(1, true), Press::Toggle, ToggleState::Off, 0.0),
            Some(Write::On)
        );
        assert_eq!(
            t.down(Down::touch(1, true), Press::Toggle, ToggleState::Mixed, 1.0),
            Some(Write::On)
        );
        assert_eq!(
            t.down(Down::touch(1, true), Press::Toggle, ToggleState::On, 2.0),
            Some(Write::Off)
        );
        assert_eq!(t.up(), None);
    }

    #[test]
    fn double_tap_latch_ignores_single_presses() {
        let mut t = ToggleCtl::default();
        let press = Press::DoubleTapLatch;
        assert_eq!(
            t.down(Down::touch(1, true), press, ToggleState::Off, 0.0),
            None
        );
        assert_eq!(t.up(), None);
        assert_eq!(
            t.down(Down::touch(1, true), press, ToggleState::Off, 199.0),
            Some(Write::On)
        );
        assert_eq!(
            t.down(Down::touch(1, true), press, ToggleState::On, 300.0),
            None
        );
        assert_eq!(
            t.down(Down::touch(1, true), press, ToggleState::On, 500.0),
            None,
            "200 ms is too slow"
        );
        assert_eq!(
            t.down(Down::touch(1, true), press, ToggleState::On, 650.0),
            Some(Write::Off)
        );
        assert_eq!(
            t.down(Down::touch(1, true), press, ToggleState::Off, 700.0),
            None,
            "a double tap resets"
        );
    }

    #[test]
    fn a_pulse_press_turns_on_until_the_release() {
        let mut t = ToggleCtl::default();
        let press = Press::PulseAndDoubleTapLatch;
        assert_eq!(
            t.down(Down::touch(1, true), press, ToggleState::Off, 0.0),
            Some(Write::On)
        );
        assert_eq!(t.up(), Some(Write::Off));
        assert_eq!(t.up(), None, "one release, one off");
    }

    #[test]
    fn a_double_tap_latches_the_pulse_and_a_press_unlatches() {
        let mut t = ToggleCtl::default();
        let press = Press::PulseAndDoubleTapLatch;
        assert_eq!(
            t.down(Down::touch(1, true), press, ToggleState::Off, 0.0),
            Some(Write::On)
        );
        assert_eq!(t.up(), Some(Write::Off));
        assert_eq!(
            t.down(Down::touch(1, true), press, ToggleState::Off, 150.0),
            Some(Write::On)
        );
        assert_eq!(t.up(), None, "latched");
        assert_eq!(
            t.down(Down::touch(1, true), press, ToggleState::On, 1000.0),
            Some(Write::Off)
        );
        assert_eq!(t.up(), None, "an unlatch is no pulse");
        assert_eq!(
            t.down(Down::touch(1, true), press, ToggleState::Mixed, 2000.0),
            Some(Write::On)
        );
        assert_eq!(t.up(), Some(Write::Off), "a mixed state pulses");
    }

    #[test]
    fn a_primary_down_ends_a_pulse_whose_release_never_came() {
        // #124 M2: pointer 1's pulse is on and its release never came. A
        // second finger or a primary mouse ends nothing; a primary finger
        // does (its Off goes), and the lost release then ends nothing.
        let mut t = ToggleCtl::default();
        let press = Press::PulseAndDoubleTapLatch;
        assert_eq!(
            t.down(Down::touch(1, true), press, ToggleState::Off, 0.0),
            Some(Write::On)
        );
        assert_eq!(t.missed(Down::touch(2, false)), None, "a second finger");
        let mouse = Down {
            id: 3,
            kind: Some(Pointer::Mouse),
            primary: true,
        };
        assert_eq!(t.missed(mouse), None, "a mouse never ends a finger's");
        assert_eq!(t.missed(Down::touch(2, true)), Some(1));
        assert_eq!(t.missed(Down::touch(4, true)), None, "once");
        assert_eq!(t.up(), None, "the pulse already ended");
    }

    #[test]
    fn a_hidden_page_ends_a_running_pulse_only() {
        let mut t = ToggleCtl::default();
        let press = Press::PulseAndDoubleTapLatch;
        assert_eq!(t.hidden(), None, "nothing pressed");
        assert_eq!(
            t.down(Down::touch(5, true), press, ToggleState::Off, 0.0),
            Some(Write::On)
        );
        assert_eq!(t.hidden(), Some(5));
        assert_eq!(t.hidden(), None, "once");
        assert_eq!(t.up(), None);
        // A latched pulse is no press: a hide leaves it on.
        assert_eq!(
            t.down(Down::touch(6, true), press, ToggleState::Off, 1_000.0),
            Some(Write::On)
        );
        assert_eq!(t.up(), Some(Write::Off));
        assert_eq!(
            t.down(Down::touch(7, true), press, ToggleState::Off, 1_150.0),
            Some(Write::On)
        );
        assert_eq!(t.hidden(), None, "latched");
        assert_eq!(t.missed(Down::touch(8, true)), None, "latched");
        // A plain toggle holds nothing.
        let mut t = ToggleCtl::default();
        assert_eq!(
            t.down(Down::touch(9, true), Press::Toggle, ToggleState::Off, 0.0),
            Some(Write::On)
        );
        assert_eq!(t.hidden(), None);
        assert_eq!(t.missed(Down::touch(10, true)), None);
    }
}
