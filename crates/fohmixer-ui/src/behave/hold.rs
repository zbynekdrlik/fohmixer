//! The hold on a strip's ☰ (spec F27, D17; #71): a press that lasts
//! `HOLD_MS` opens the channel detail, once; a shorter one is a tap, which
//! only shows the hint. One press at a time: a second finger's down is not
//! one, a primary pointer's down starts over (an end the browser never
//! delivered leaves nothing stuck). A finger may slide; only its release
//! ends the press, and a cancel or a lost capture ends it without a tap.

/// How long a press on ☰ lasts before it opens the channel detail.
pub const HOLD_MS: f64 = 500.0;
/// How long a tap's hint shows.
pub const HINT_MS: f64 = 1200.0;

/// Whether a press that started at `start` has lasted the hold at `now`.
pub fn held_long(start: f64, now: f64) -> bool {
    now - start >= HOLD_MS
}

/// One press: its pointer, when it started and whether it opened the
/// detail.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Press {
    pointer: i32,
    at: f64,
    opened: bool,
}

/// What a lift ends ([`Hold::up`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lift {
    /// Lifted before the hold: a tap (the hint).
    Tap,
    /// Lifted after the hold, which no check has seen yet (a late timer):
    /// the detail opens now.
    Open,
    /// Another pointer's lift, or the press already opened the detail.
    Nothing,
}

/// The ☰ button's press.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Hold {
    press: Option<Press>,
}

impl Hold {
    /// Pointer `pointer` went down at `now` (`primary`: no other pointer is
    /// down, the browser's `isPrimary`): whether it is the press now. A
    /// second finger's down while one presses is not; the same pointer, or
    /// a primary one, starts the press over.
    pub fn down(&mut self, pointer: i32, primary: bool, now: f64) -> bool {
        let other = self.press.is_some_and(|p| p.pointer != pointer);
        if other && !primary {
            return false;
        }
        self.press = Some(Press {
            pointer,
            at: now,
            opened: false,
        });
        true
    }

    /// Whether a press is down that has not opened the detail (the button
    /// fills meanwhile).
    pub fn holding(&self) -> bool {
        self.press.is_some_and(|p| !p.opened)
    }

    /// Whether the press has lasted the hold at `now` and not opened the
    /// detail yet.
    pub fn due(&self, now: f64) -> bool {
        self.press
            .is_some_and(|p| !p.opened && held_long(p.at, now))
    }

    /// The hold's check at `now`: whether the press opens the detail now
    /// (it is due); a press opens it once.
    pub fn open(&mut self, now: f64) -> bool {
        if !self.due(now) {
            return false;
        }
        if let Some(press) = self.press.as_mut() {
            press.opened = true;
        }
        true
    }

    /// Pointer `pointer` lifted at `now`: what its press ends.
    pub fn up(&mut self, pointer: i32, now: f64) -> Lift {
        let Some(press) = self.press.filter(|p| p.pointer == pointer) else {
            return Lift::Nothing;
        };
        self.press = None;
        if press.opened {
            Lift::Nothing
        } else if held_long(press.at, now) {
            Lift::Open
        } else {
            Lift::Tap
        }
    }

    /// Pointer `pointer` was cancelled or lost its capture: its press ends,
    /// no tap, nothing opens.
    pub fn cancel(&mut self, pointer: i32) {
        if self.press.is_some_and(|p| p.pointer == pointer) {
            self.press = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_has_lasted_the_hold_at_exactly_500_ms() {
        assert!(held_long(1000.0, 1500.0));
        assert!(!held_long(1000.0, 1500.0_f64.next_down()));
        assert!(held_long(0.0, 500.0));
        assert!(!held_long(0.0, 499.999));
        assert!(held_long(1000.0, 2400.0));
        assert!(!held_long(1000.0, 1499.0));
        assert_eq!(HOLD_MS, 500.0);
        assert_eq!(HINT_MS, 1200.0);
    }

    #[test]
    fn a_press_opens_the_detail_once_at_its_hold() {
        let mut h = Hold::default();
        assert!(!h.holding() && !h.due(1000.0));
        assert!(h.down(7, true, 1000.0));
        assert!(h.holding());
        assert!(!h.due(1499.999));
        assert!(!h.open(1499.999), "not before the hold");
        assert!(h.holding());
        assert!(h.due(1500.0));
        assert!(h.open(1500.0));
        assert!(!h.holding(), "the button stops filling");
        assert!(!h.due(1600.0));
        assert!(!h.open(1600.0), "once");
        // Its lift after that ends nothing.
        assert_eq!(h.up(7, 1700.0), Lift::Nothing);
        assert!(!h.open(1800.0));
    }

    #[test]
    fn a_lift_before_the_hold_is_a_tap() {
        let mut h = Hold::default();
        assert!(h.down(3, true, 200.0));
        assert_eq!(h.up(3, 699.999), Lift::Tap);
        assert!(!h.holding());
        assert!(!h.open(800.0), "a tap opens nothing later");
        assert_eq!(h.up(3, 900.0), Lift::Nothing, "the press is over");
    }

    #[test]
    fn a_lift_after_the_hold_no_check_has_seen_opens_it() {
        let mut h = Hold::default();
        assert!(h.down(3, true, 200.0));
        assert_eq!(h.up(3, 700.0), Lift::Open);
        assert!(!h.holding());
        assert!(!h.open(800.0));
    }

    #[test]
    fn a_second_finger_is_no_press_and_its_lift_ends_nothing() {
        let mut h = Hold::default();
        assert!(h.down(1, true, 0.0));
        assert!(!h.down(2, false, 100.0), "a second finger");
        assert_eq!(h.up(2, 150.0), Lift::Nothing);
        h.cancel(2);
        assert!(h.holding(), "the first press goes on");
        assert!(!h.open(499.0));
        assert!(h.open(500.0), "from the first finger's down");
    }

    #[test]
    fn a_primary_down_or_the_same_pointer_starts_the_press_over() {
        // An up the browser never delivered: the next primary down is a
        // new press, from its own time.
        let mut h = Hold::default();
        assert!(h.down(1, true, 0.0));
        assert!(h.down(2, true, 1000.0));
        assert!(!h.open(1499.0));
        assert_eq!(h.up(1, 1200.0), Lift::Nothing, "the old pointer's lift");
        assert!(h.open(1500.0));
        // The same pointer pressing again starts over too.
        let mut h = Hold::default();
        assert!(h.down(4, false, 0.0), "no press yet: any down is one");
        assert!(h.down(4, false, 300.0));
        assert!(!h.open(799.0));
        assert!(h.open(800.0));
        // After an opened press, a new press holds again.
        assert_eq!(h.up(4, 900.0), Lift::Nothing);
        assert!(h.down(5, false, 1000.0));
        assert!(h.holding());
        assert_eq!(h.up(5, 1100.0), Lift::Tap);
    }

    #[test]
    fn a_cancel_or_a_lost_capture_ends_the_press_without_a_tap() {
        let mut h = Hold::default();
        assert!(h.down(9, true, 0.0));
        h.cancel(8);
        assert!(h.holding(), "another pointer's cancel");
        h.cancel(9);
        assert!(!h.holding());
        assert!(!h.due(600.0));
        assert!(!h.open(600.0), "nothing opens after a cancel");
        assert_eq!(h.up(9, 100.0), Lift::Nothing, "no tap after it");
        // A re-press after the cancel is a press of its own.
        assert!(h.down(9, true, 1000.0));
        assert_eq!(h.up(9, 1100.0), Lift::Tap);
    }
}
