//! A sheet over the channel detail (#208's ADD TO LAYOUT, #205's channel
//! selector): its keys act at their lift, and a finger dragged over its
//! list scrolls it. Pure; `components::sheet` carries it out.
//!
//! The detail takes no native touch scroll (`detail.css`: `.detail`
//! `touch-action: none`, and every key prevents its `touchstart`), so a
//! list longer than its panel scrolls by the finger's own moves: each move
//! of a pressed pointer scrolls it by the distance since the pointer's
//! last move.

use super::hold::{Hold, Lift};
use super::rail::RAIL_HOLD_MS;

/// The press of a sheet's key and of a LAYOUTS row's ✕ (the #208 review;
/// AUTOTUNE's singer switch's, PR #150 review M1): it acts at the lift of
/// a tap or a long press, never at its down; a slide past 10 px, a cancel
/// or a lost capture ends it with nothing done, so a finger that scrolls
/// the list or lands on a key on its way somewhere else changes nothing.
pub fn key_press() -> Hold {
    Hold::lifted(RAIL_HOLD_MS)
}

/// Whether a key's lift acts.
pub fn key_acts(lift: Lift) -> bool {
    matches!(lift, Lift::Tap | Lift::Open)
}

/// A finger scrolling a sheet's list: the pointer that drives it and where
/// it was last.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DragScroll {
    at: Option<(i32, f64)>,
}

impl DragScroll {
    /// Pointer `pointer` moved to `y` (`pressed`: a button or a finger
    /// down): how far to scroll the list down (px). A pointer's first move
    /// only anchors it, as does the first move of another pointer; a move
    /// with nothing pressed (a mouse hovering) forgets the drag.
    pub fn moved(&mut self, pointer: i32, y: f64, pressed: bool) -> f64 {
        if !pressed {
            self.at = None;
            return 0.0;
        }
        let step = match self.at {
            Some((p, last)) if p == pointer => last - y,
            _ => 0.0,
        };
        self.at = Some((pointer, y));
        step
    }

    /// Pointer `pointer` lifted, was cancelled or lost its capture.
    pub fn end(&mut self, pointer: i32) {
        if self.at.is_some_and(|(p, _)| p == pointer) {
            self.at = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One press of a key: its down at 0 ms, a move to `slide` px right of
    /// its anchor (none: no move), then its lift at `lift_ms`.
    fn press(slide: Option<f64>, lift_ms: f64) -> bool {
        let mut key = key_press();
        assert!(key.down(7, true, 0.0));
        if let Some(px) = slide {
            key.moved(7, 100.0, 100.0);
            key.moved(7, 100.0 + px, 100.0);
        }
        key_acts(key.up(7, lift_ms))
    }

    #[test]
    fn a_key_acts_at_the_lift_of_a_tap_or_a_long_press_and_never_after_a_slide() {
        assert!(press(None, 80.0));
        assert!(press(Some(10.0), 80.0));
        assert!(press(None, 2500.0));
        assert!(!press(Some(11.0), 80.0));
        assert!(!press(Some(-30.0), 2500.0));
        // A cancel or a lost capture: nothing.
        let mut key = key_press();
        assert!(key.down(7, true, 0.0));
        key.cancel(7);
        assert!(!key_acts(key.up(7, 80.0)));
        // Another pointer's lift: nothing.
        let mut key = key_press();
        assert!(key.down(7, true, 0.0));
        assert!(!key_acts(key.up(8, 80.0)));
        assert!(!key_acts(Lift::Nothing));
        assert!(key_acts(Lift::Tap));
        assert!(key_acts(Lift::Open));
        // A press a hold's check found full (1 s) still acts at its lift,
        // never at the check: the key is a lifted press, not SEL's.
        let mut key = key_press();
        assert!(key.down(7, true, 0.0));
        assert!(!key.open(999.0));
        assert!(key.open(1020.0));
        assert!(key_acts(key.up(7, 1100.0)));
    }

    #[test]
    fn a_dragged_finger_scrolls_by_its_moves_after_the_first() {
        let mut drag = DragScroll::default();
        // The first move anchors; up 30 px scrolls 30 down, back 10 px up.
        assert_eq!(drag.moved(3, 400.0, true), 0.0);
        assert_eq!(drag.moved(3, 370.0, true), 30.0);
        assert_eq!(drag.moved(3, 380.0, true), -10.0);
        // Another pointer takes over from its own first move.
        assert_eq!(drag.moved(4, 100.0, true), 0.0);
        assert_eq!(drag.moved(4, 90.0, true), 10.0);
        // A lift of another pointer leaves the drag; its own ends it.
        drag.end(3);
        assert_eq!(drag.moved(4, 85.0, true), 5.0);
        drag.end(4);
        assert_eq!(drag, DragScroll::default());
        assert_eq!(drag.moved(4, 60.0, true), 0.0);
        // Nothing pressed (a hovering mouse): no scroll, the drag forgotten.
        assert_eq!(drag.moved(4, 20.0, false), 0.0);
        assert_eq!(drag, DragScroll::default());
        assert_eq!(drag.moved(4, 10.0, true), 0.0);
    }
}
