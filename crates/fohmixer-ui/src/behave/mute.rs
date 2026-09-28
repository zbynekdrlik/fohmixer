//! The mute button's protection (spec F12; TouchOSC `mute_button.lua`
//! 2.7.2): a strip listed in `double_click_mute` changes its mute only on a
//! second tap within 500 ms. The first tap arms (the button pulses) and
//! changes nothing; a tap after the window arms again.

/// The window for the confirming second tap.
pub const GUARD_MS: f64 = 500.0;

/// What a tap on a mute button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardAction {
    /// Send the new mute.
    Apply,
    /// Only arm the guard (and show it).
    Arm,
}

/// The guard of one mute button.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MuteGuard {
    armed_at: Option<f64>,
}

impl MuteGuard {
    /// A tap at `now` on a button that is `guarded` (or not).
    pub fn on_tap(&mut self, guarded: bool, now: f64) -> GuardAction {
        if !guarded {
            return GuardAction::Apply;
        }
        if self.armed(now) {
            self.armed_at = None;
            GuardAction::Apply
        } else {
            self.armed_at = Some(now);
            GuardAction::Arm
        }
    }

    /// Whether the guard waits for the confirming tap at `now`.
    pub fn armed(&self, now: f64) -> bool {
        self.armed_at.is_some_and(|t| now - t <= GUARD_MS)
    }
}

/// Whether a mute button is lit: it is lit while the track is audible.
pub fn lit(muted: bool) -> bool {
    !muted
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unguarded_tap_applies_at_once() {
        let mut g = MuteGuard::default();
        assert_eq!(g.on_tap(false, 0.0), GuardAction::Apply);
        assert_eq!(g.on_tap(false, 10.0), GuardAction::Apply);
        assert!(!g.armed(10.0));
    }

    #[test]
    fn a_guarded_mute_needs_a_second_tap_within_500_ms() {
        let mut g = MuteGuard::default();
        assert_eq!(g.on_tap(true, 1000.0), GuardAction::Arm);
        assert!(g.armed(1000.0) && g.armed(1500.0) && !g.armed(1501.0));
        assert_eq!(g.on_tap(true, 1400.0), GuardAction::Apply);
        assert!(!g.armed(1400.0), "applied: disarmed");
        // A second tap at 600 ms arms again instead.
        assert_eq!(g.on_tap(true, 2000.0), GuardAction::Arm);
        assert_eq!(g.on_tap(true, 2600.0), GuardAction::Arm);
        assert_eq!(
            g.on_tap(true, 3100.0),
            GuardAction::Apply,
            "500 ms still counts"
        );
    }

    #[test]
    fn a_mute_button_is_lit_while_the_track_is_audible() {
        assert!(lit(false));
        assert!(!lit(true));
    }
}
