//! The hold on a strip's ☰ (spec F27, D17; #71): a press that lasts
//! `HOLD_MS` opens the channel detail, once; a shorter one is a tap, which
//! only shows the hint. One press at a time: a second finger's down is not
//! one, a primary pointer's down starts over (an end the browser never
//! delivered leaves nothing stuck). A finger may slide up to `SLIDE_PX`
//! from where its first move put it (the iPad's first move comes late and
//! 6–15 px away, so it only anchors the press, as a fader's does, #43 PR F);
//! further, the press ends with neither a tap nor an open (a fader grab
//! that lands on ☰ never opens the detail). Else only
//! its release ends the press, and a cancel or a lost capture ends it
//! without a tap. Right after a hold opens the detail, its parts take no
//! touch for `OPEN_GUARD_MS` (a second tap at ☰'s spot would land on its
//! MUTE; `guard_left`: a new layout's remount does not guard again).
//!
//! A rail button with an automatic mode (#104, #130, D18) presses the same
//! way with its own hold (`behave::rail::RAIL_HOLD_MS`) but acts at its
//! lift (`Hold::lifted`, the owner's ruling on #104 of 2026-10-17): its
//! check only makes the press full, a lift while full acts (`Lift::Open`)
//! unless the full mark was refused (`Hold::refuse`), and a full press still
//! ends with nothing done at a slide, a cancel, a lost capture or a hidden
//! page (`Hold::leave`), so a lost end never acts. A lift before the hold
//! is the button's tap.

/// How long a press on ☰ lasts before it opens the channel detail.
pub const HOLD_MS: f64 = 500.0;
/// How long a tap's hint shows.
pub const HINT_MS: f64 = 1200.0;
/// How far a press's finger may slide from its down point (px).
pub const SLIDE_PX: f64 = 10.0;
/// How long the detail's parts take no touch after it opens.
pub const OPEN_GUARD_MS: f64 = 400.0;

/// Whether a press that started at `start` has lasted ☰'s hold at `now`.
pub fn held_long(start: f64, now: f64) -> bool {
    lasted(start, HOLD_MS, now)
}

/// Whether `ms` have passed from `start` at `now`.
fn lasted(start: f64, ms: f64, now: f64) -> bool {
    now - start >= ms
}

/// Whether a finger `dx`, `dy` px from its down point has slid off the
/// press (further than `SLIDE_PX`, in a straight line).
pub fn slid_off(dx: f64, dy: f64) -> bool {
    dx.hypot(dy) > SLIDE_PX
}

/// How long the detail's parts still take no touch at `now`, the detail
/// opened by a hold at `opened_at` (the page clock): `OPEN_GUARD_MS` from
/// the open, 0 once it passed. A new layout's remount of an open detail
/// guards only what is left, never again.
pub fn guard_left(opened_at: f64, now: f64) -> f64 {
    (OPEN_GUARD_MS - (now - opened_at)).max(0.0)
}

/// One press: its pointer, when it went down, where its first move
/// anchored it (none before that move), whether it passed its check (☰'s
/// opened the detail; a rail press is full) and whether that full mark was
/// refused (a rail press that could not act: its lift does nothing).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Press {
    pointer: i32,
    at: f64,
    anchor: Option<(f64, f64)>,
    opened: bool,
    refused: bool,
}

/// When a held press acts: ☰'s at its check (opening the detail is
/// harmless), a rail button's at its lift once full (#104: a lost end never
/// acts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acts {
    AtCheck,
    AtLift,
}

/// What a lift ends ([`Hold::up`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lift {
    /// Lifted before the hold: a tap (the hint).
    Tap,
    /// Lifted after the hold, which no check has seen yet (a late timer):
    /// the detail opens now; a rail press's lift while full (it acts now).
    Open,
    /// Another pointer's lift, a press that already opened the detail, or a
    /// rail press whose full mark was refused.
    Nothing,
}

/// A press that lasts its hold: the ☰ button's ([`HOLD_MS`], the default)
/// and a rail button's with an automatic mode (#104, #130, D18:
/// `behave::rail::RAIL_HOLD_MS`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hold {
    press: Option<Press>,
    /// How long a press lasts before it holds (ms).
    ms: f64,
    acts: Acts,
}

impl Default for Hold {
    fn default() -> Self {
        Self::with(HOLD_MS)
    }
}

impl Hold {
    /// A press that holds after `ms` and acts at its check (☰'s).
    pub fn with(ms: f64) -> Self {
        Self {
            press: None,
            ms,
            acts: Acts::AtCheck,
        }
    }

    /// A press that is full after `ms` and acts at its lift once full (a
    /// rail button's, #104).
    pub fn lifted(ms: f64) -> Self {
        Self {
            press: None,
            ms,
            acts: Acts::AtLift,
        }
    }

    /// Whether the press is full: past its check and not refused.
    pub fn full(&self) -> bool {
        self.press.is_some_and(|p| p.opened && !p.refused)
    }

    /// The full mark could not act (#104: the page not connected, the hub's
    /// flag not known): the press is dead, its lift does nothing.
    pub fn refuse(&mut self) {
        if let Some(press) = self.press.as_mut().filter(|p| p.opened) {
            press.refused = true;
        }
    }

    /// The press's pointer, opened or not.
    pub fn pressing(&self) -> Option<i32> {
        self.press.map(|p| p.pointer)
    }

    /// The page went hidden: any press ends, with neither a tap nor a hold;
    /// the pointer it ended.
    pub fn leave(&mut self) -> Option<i32> {
        self.press.take().map(|p| p.pointer)
    }

    /// Pointer `pointer` went down at `now` (`primary`: no other pointer
    /// is down, the browser's `isPrimary`): whether it is the press now. A
    /// second finger's down while one presses is not; the same pointer, or
    /// a primary one, starts the press over. Where it went down does not
    /// count: its first move anchors it.
    pub fn down(&mut self, pointer: i32, primary: bool, now: f64) -> bool {
        let other = self.press.is_some_and(|p| p.pointer != pointer);
        if other && !primary {
            return false;
        }
        self.press = Some(Press {
            pointer,
            at: now,
            anchor: None,
            opened: false,
            refused: false,
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
            .is_some_and(|p| !p.opened && lasted(p.at, self.ms, now))
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

    /// Pointer `pointer` moved to `x`, `y`: whether that ended its press
    /// (slid off it: no tap, nothing opens, the fill clears). The press's
    /// first move only anchors it there; a press that opened the detail
    /// already has nothing left to end (its finger moving on is no slide),
    /// while a full rail press still slides off (nothing acts).
    pub fn moved(&mut self, pointer: i32, x: f64, y: f64) -> bool {
        let at_lift = self.acts == Acts::AtLift;
        let Some(press) = self
            .press
            .as_mut()
            .filter(|p| p.pointer == pointer && (!p.opened || at_lift))
        else {
            return false;
        };
        let Some((ax, ay)) = press.anchor else {
            press.anchor = Some((x, y));
            return false;
        };
        if !slid_off(x - ax, y - ay) {
            return false;
        }
        self.press = None;
        true
    }

    /// Pointer `pointer` lifted at `now`: what its press ends.
    pub fn up(&mut self, pointer: i32, now: f64) -> Lift {
        let Some(press) = self.press.filter(|p| p.pointer == pointer) else {
            return Lift::Nothing;
        };
        self.press = None;
        if press.opened {
            if self.acts == Acts::AtLift && !press.refused {
                Lift::Open
            } else {
                Lift::Nothing
            }
        } else if lasted(press.at, self.ms, now) {
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
    fn a_rail_press_holds_at_exactly_3000_ms() {
        let mut h = Hold::with(3000.0);
        assert!(h.down(1, true, 0.0));
        assert!(!h.open(3000.0_f64.next_down()));
        assert!(h.holding());
        assert!(h.open(3000.0));
        assert!(!h.open(3500.0), "once");
        assert_eq!(h.up(1, 3600.0), Lift::Nothing);
        // A lift a hair before is a tap; one after that no check saw is the hold.
        assert!(h.down(2, true, 0.0));
        assert_eq!(h.up(2, 3000.0_f64.next_down()), Lift::Tap);
        assert!(h.down(3, true, 0.0));
        assert_eq!(h.up(3, 3000.0), Lift::Open);
        // From the press's own down, not from 0.
        assert!(h.down(4, true, 1000.0));
        assert!(!h.due(4000.0_f64.next_down()));
        assert!(h.due(4000.0));
        // ☰'s stays 500 ms.
        let mut menu = Hold::default();
        assert!(menu.down(5, true, 0.0));
        assert!(!menu.open(500.0_f64.next_down()));
        assert!(menu.open(500.0));
        assert_eq!(Hold::default(), Hold::with(HOLD_MS));
    }

    #[test]
    fn a_rail_press_is_full_at_exactly_3000_ms_and_acts_at_its_lift() {
        let mut h = Hold::lifted(3000.0);
        assert!(h.down(1, true, 0.0));
        assert!(!h.full());
        assert!(!h.open(3000.0_f64.next_down()));
        assert!(h.holding() && !h.full(), "filling");
        assert!(h.open(3000.0));
        assert!(h.full() && !h.holding(), "full");
        assert!(!h.open(3500.0), "once");
        assert_eq!(h.pressing(), Some(1));
        // Another pointer's lift ends nothing; its own acts.
        assert_eq!(h.up(2, 3600.0), Lift::Nothing);
        assert!(h.full());
        assert_eq!(h.up(1, 3600.0), Lift::Open);
        assert!(!h.full());
        // A lift a hair before is a tap; one after that no check saw acts.
        assert!(h.down(2, true, 0.0));
        assert_eq!(h.up(2, 3000.0_f64.next_down()), Lift::Tap);
        assert!(h.down(3, true, 0.0));
        assert_eq!(h.up(3, 3000.0), Lift::Open);
        assert_eq!(Hold::lifted(3000.0).acts, Acts::AtLift);
        assert_eq!(Hold::with(3000.0).acts, Acts::AtCheck);
    }

    #[test]
    fn a_refused_full_mark_makes_the_lift_do_nothing_and_never_a_tap() {
        let mut h = Hold::lifted(3000.0);
        assert!(h.down(1, true, 0.0));
        h.refuse();
        assert!(h.holding(), "no full mark yet: nothing to refuse");
        assert!(h.open(3000.0));
        h.refuse();
        assert!(!h.full() && !h.holding());
        assert_eq!(h.pressing(), Some(1), "still down until its end");
        assert_eq!(h.up(1, 3500.0), Lift::Nothing);
        // A new press starts afresh.
        assert!(h.down(1, true, 4000.0));
        assert!(h.open(7000.0));
        assert!(h.full());
        assert_eq!(h.up(1, 7100.0), Lift::Open);
    }

    #[test]
    fn a_full_rail_press_ends_with_nothing_done_at_a_slide_a_cancel_or_a_hide() {
        let full = || {
            let mut h = Hold::lifted(3000.0);
            assert!(h.down(1, true, 0.0));
            assert!(h.open(3000.0));
            h
        };
        // A slide past 10 px from the first move.
        let mut h = full();
        assert!(!h.moved(1, 50.0, 50.0), "the anchor");
        assert!(!h.moved(1, 60.0, 50.0), "exactly 10 px");
        assert!(h.moved(1, 60.0_f64.next_up(), 50.0));
        assert_eq!((h.full(), h.up(1, 3500.0)), (false, Lift::Nothing));
        // A cancel or a lost capture.
        let mut h = full();
        h.cancel(2);
        assert!(h.full(), "another pointer's cancel");
        h.cancel(1);
        assert_eq!((h.full(), h.up(1, 3500.0)), (false, Lift::Nothing));
        // A hidden page.
        let mut h = full();
        assert_eq!(h.leave(), Some(1));
        assert_eq!(h.up(1, 3500.0), Lift::Nothing);
        // ☰'s opened press keeps sliding nothing off.
        let mut menu = Hold::default();
        assert!(menu.down(1, true, 0.0));
        assert!(menu.open(500.0));
        assert!(!menu.moved(1, 0.0, 0.0));
        assert!(!menu.moved(1, 0.0, 300.0));
        assert_eq!(menu.up(1, 600.0), Lift::Nothing);
    }

    #[test]
    fn a_press_says_its_pointer_and_a_hidden_page_ends_it() {
        let mut h = Hold::with(3000.0);
        assert_eq!(h.pressing(), None);
        assert_eq!(h.leave(), None);
        assert!(h.down(7, true, 0.0));
        assert_eq!(h.pressing(), Some(7));
        assert_eq!(h.leave(), Some(7));
        assert_eq!(h.pressing(), None);
        assert!(!h.holding());
        assert!(!h.open(4000.0), "nothing to open");
        assert_eq!(h.up(7, 4000.0), Lift::Nothing);
        // An opened press still says its pointer until it ends.
        assert!(h.down(8, true, 0.0));
        assert!(h.open(3000.0));
        assert_eq!(h.pressing(), Some(8));
        assert_eq!(h.leave(), Some(8));
    }

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
        assert_eq!(SLIDE_PX, 10.0);
        assert_eq!(OPEN_GUARD_MS, 400.0);
    }

    #[test]
    fn the_guard_counts_400_ms_from_the_open_never_from_a_remount() {
        assert_eq!(guard_left(1000.0, 1000.0), 400.0);
        assert_eq!(guard_left(1000.0, 1100.0), 300.0);
        assert!(guard_left(1000.0, 1400.0_f64.next_down()) > 0.0);
        assert_eq!(guard_left(1000.0, 1400.0), 0.0);
        assert_eq!(guard_left(1000.0, 5000.0), 0.0, "a remount long after");
        assert_eq!(
            guard_left(f64::NEG_INFINITY, 0.0),
            0.0,
            "never opened by a hold"
        );
    }

    #[test]
    fn a_finger_slides_off_the_press_past_exactly_10_px() {
        assert!(!slid_off(0.0, 0.0));
        assert!(!slid_off(10.0, 0.0));
        assert!(slid_off(10.0_f64.next_up(), 0.0));
        assert!(!slid_off(0.0, -10.0));
        assert!(slid_off(0.0, -(10.0_f64.next_up())));
        // In a straight line, not per axis.
        assert!(!slid_off(-6.0, 7.9));
        assert!(slid_off(-6.0, 8.01));
        assert!(slid_off(8.0, -7.0));
    }

    #[test]
    fn a_slide_past_10_px_from_the_first_move_ends_the_press_with_neither_a_tap_nor_an_open() {
        let mut h = Hold::default();
        assert!(h.down(1, true, 0.0));
        // The first move only anchors the press, however far (the iPad's
        // comes 6–15 px away).
        assert!(!h.moved(1, 115.0, 200.0));
        // Up to 10 px from the anchor: the press goes on.
        assert!(!h.moved(1, 125.0, 200.0));
        assert!(!h.moved(1, 115.0, 190.0));
        assert!(!h.moved(1, 109.0, 192.1));
        // Another pointer's move ends nothing.
        assert!(!h.moved(2, 300.0, 300.0));
        assert!(h.holding());
        assert!(h.moved(1, 115.0, 210.5), "past 10 px from the anchor");
        assert!(!h.holding(), "the fill clears");
        assert!(!h.open(600.0), "nothing opens");
        assert_eq!(h.up(1, 700.0), Lift::Nothing, "no tap");
        assert!(!h.moved(1, 0.0, 0.0), "no press to end");
        // Within the slide the press still opens at its hold.
        assert!(h.down(3, true, 1000.0));
        assert!(!h.moved(3, 50.0, 40.0));
        assert!(!h.moved(3, 57.0, 47.0));
        assert!(h.open(1500.0));
        // A fader grab that lands on ☰ and pulls down: no detail, no hint.
        let mut grab = Hold::default();
        assert!(grab.down(4, true, 0.0));
        assert!(!grab.moved(4, 60.0, 60.0), "the anchor");
        assert!(grab.moved(4, 60.0, 80.0));
        assert!(!grab.open(520.0));
        assert_eq!(grab.up(4, 900.0), Lift::Nothing);
        // A new press anchors again at its own first move.
        let mut again = Hold::default();
        assert!(again.down(5, true, 0.0));
        assert!(!again.moved(5, 3.0, 0.0));
        assert!(again.down(5, true, 50.0));
        assert!(!again.moved(5, 140.0, 100.0), "the new press's anchor");
        assert!(!again.moved(5, 150.0, 100.0), "exactly 10 px");
        assert!(again.moved(5, 150.0_f64.next_up(), 100.0));
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
        // Its finger moving on, however far, is no slide.
        assert!(!h.moved(7, 0.0, 0.0), "the anchor");
        assert!(
            !h.moved(7, 0.0, 300.0),
            "an opened press slides off nothing"
        );
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
