//! The pan control (spec F11; TouchOSC `pan_control.lua` 1.5.3): position
//! `p` ↔ Live panning `2p − 1`, relative drag, two releases within 300 ms
//! centre it, grey when centred and cyan otherwise.

use super::{Motion, Start};

/// The colour of a centred pan and of an off-centre one.
pub const CENTERED: &str = "#646464";
pub const OFF_CENTER: &str = "#34C1DC";
/// How far from the centre still counts as centred.
const CENTER_TOLERANCE: f64 = 0.01;
/// Two releases closer than this centre the pan.
pub const DOUBLE_TAP_MS: f64 = 300.0;
/// After a release the pan shows its own position this long, so Live's
/// echo of the last value lands before Live's value is shown again
/// (TouchOSC synced at once; see the S4 decisions on the ticket).
pub const HOLD_MS: f64 = 100.0;

/// Live's panning (−1..1) of position `p`.
pub fn to_live(p: f64) -> f64 {
    p.clamp(0.0, 1.0) * 2.0 - 1.0
}

/// The position of Live's panning `v`.
pub fn to_pos(v: f64) -> f64 {
    ((v + 1.0) / 2.0).clamp(0.0, 1.0)
}

/// Whether position `p` shows as centred.
pub fn is_centered(p: f64) -> bool {
    near_centre(p - 0.5)
}

/// Whether an offset from the centre still counts as centred.
fn near_centre(offset: f64) -> bool {
    offset.abs() <= CENTER_TOLERANCE
}

/// The pan's colour at position `p`.
pub fn color(p: f64) -> &'static str {
    if is_centered(p) { CENTERED } else { OFF_CENTER }
}

/// One pan's input state: one pointer, the relative drag, the double tap
/// to the centre and the post-release hold.
#[derive(Debug, Clone, PartialEq)]
pub struct PanCtl {
    pointer: Option<i32>,
    last_x: f64,
    travel: f64,
    pos: f64,
    unsent: bool,
    last_release: Option<f64>,
    hold_until: f64,
}

impl Default for PanCtl {
    fn default() -> Self {
        Self {
            pointer: None,
            last_x: 0.0,
            travel: 1.0,
            pos: 0.5,
            unsent: false,
            last_release: None,
            hold_until: f64::NEG_INFINITY,
        }
    }
}

impl PanCtl {
    fn local(&self, now: f64) -> bool {
        self.pointer.is_some() || now < self.hold_until
    }

    /// Pointer `id` pressed at `x` (px) on a pan `travel` px wide, Live's
    /// panning being `live`: whether this pointer now drives the pan.
    pub fn down(&mut self, id: i32, x: f64, travel: f64, now: f64, live: f64) -> bool {
        self.press(id, x, travel, now, live).is_some()
    }

    /// [`PanCtl::down`], with where the touch started when this pointer now
    /// drives the pan (#43 PR D), in positions: the one it showed, Live's
    /// panning's, whether it showed its own (a finger, the hold) and the one
    /// the touch starts from.
    pub fn press(&mut self, id: i32, x: f64, travel: f64, now: f64, live: f64) -> Option<Start> {
        if self.pointer.is_some() {
            return None;
        }
        let shown = self.pos;
        let local = self.local(now);
        if !local {
            self.pos = to_pos(live);
        }
        self.pointer = Some(id);
        self.last_x = x;
        self.travel = travel.max(1.0);
        Some(Start {
            shown,
            live: to_pos(live),
            local,
            from: self.pos,
        })
    }

    /// Whether pointer `id` drives this pan (its release is this pan's).
    pub fn drives(&self, id: i32) -> bool {
        self.pointer == Some(id)
    }

    /// Pointer `id` moved to `x`: whether it moved this pan.
    pub fn moved(&mut self, id: i32, x: f64) -> bool {
        if self.pointer != Some(id) {
            return false;
        }
        self.pos = (self.pos + (x - self.last_x) / self.travel).clamp(0.0, 1.0);
        self.last_x = x;
        self.unsent = true;
        true
    }

    /// Pointer `id` lifted: the panning still to send, if any. The second
    /// release within 300 ms centres the pan.
    pub fn up(&mut self, id: i32, now: f64) -> Option<f64> {
        if self.pointer != Some(id) {
            return None;
        }
        self.pointer = None;
        self.hold_until = now + HOLD_MS;
        let double = self.last_release.is_some_and(|t| now - t < DOUBLE_TAP_MS);
        if double {
            self.last_release = None;
            self.pos = 0.5;
            self.unsent = false;
            return Some(0.0);
        }
        self.last_release = Some(now);
        self.take_unsent()
    }

    /// Pointer `id` was cancelled: a release that never centres.
    pub fn cancel(&mut self, id: i32, now: f64) -> Option<f64> {
        if self.pointer != Some(id) {
            return None;
        }
        self.pointer = None;
        self.hold_until = now + HOLD_MS;
        self.last_release = None;
        self.take_unsent()
    }

    fn take_unsent(&mut self) -> Option<f64> {
        let unsent = self.unsent;
        self.unsent = false;
        unsent.then(|| to_live(self.pos))
    }

    /// The frame at `now`, Live's panning being `live`.
    pub fn frame(&mut self, now: f64, live: Option<f64>) -> Motion {
        if self.pointer.is_some() {
            return Motion {
                pos: Some(self.pos),
                send: self.take_unsent(),
            };
        }
        if self.local(now) {
            return Motion {
                pos: Some(self.pos),
                send: None,
            };
        }
        let pos = live.map(to_pos);
        if let Some(p) = pos {
            self.pos = p;
        }
        Motion { pos, send: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_position_maps_to_minus_one_to_one() {
        assert_eq!(to_live(0.5), 0.0);
        assert_eq!(to_live(0.0), -1.0);
        assert_eq!(to_live(1.0), 1.0);
        assert_eq!(to_live(0.75), 0.5);
        assert_eq!(to_live(1.5), 1.0);
        assert_eq!(to_live(-0.5), -1.0);
        assert_eq!(to_pos(0.0), 0.5);
        assert_eq!(to_pos(-0.25), 0.375);
        assert_eq!(to_pos(1.0), 1.0);
        assert_eq!(to_pos(-3.0), 0.0);
        assert_eq!(to_pos(3.0), 1.0);
    }

    #[test]
    fn grey_when_centred_cyan_otherwise() {
        assert_eq!(color(0.5), CENTERED);
        assert_eq!(color(0.6), OFF_CENTER);
        assert_eq!(color(0.4), OFF_CENTER);
        // 0.01 either side is still centred (0.25 steps are exact in binary).
        assert!(is_centered(0.5 + 0.0078125));
        assert!(is_centered(0.5 - 0.0078125));
        assert!(!is_centered(0.5 + 0.015625));
        assert!(!is_centered(0.5 - 0.015625));
        // The tolerance itself is still centred.
        assert!(near_centre(0.01) && near_centre(-0.01));
        assert!(!near_centre(0.010000000000000002) && !near_centre(-0.010000000000000002));
    }

    fn touched(live: f64) -> PanCtl {
        let mut p = PanCtl::default();
        assert!(p.down(1, 100.0, 200.0, 0.0, live));
        p
    }

    #[test]
    fn a_drag_moves_the_pan_relative_to_where_it_was() {
        let mut p = touched(0.0);
        assert!(p.moved(1, 150.0));
        let frame = p.frame(10.0, Some(0.0));
        assert_eq!(frame.pos, Some(0.75));
        assert_eq!(frame.send, Some(0.5));
        assert_eq!(p.frame(20.0, Some(0.0)).send, None, "sent once");
        assert!(p.moved(1, -1000.0));
        assert_eq!(p.frame(30.0, Some(0.0)).send, Some(-1.0));
        // Another pointer does nothing to it.
        assert!(!p.down(2, 0.0, 200.0, 40.0, 0.0));
        assert!(!p.moved(2, 50.0));
        assert_eq!(p.up(2, 50.0), None);
        assert_eq!(p.cancel(2, 50.0), None);
    }

    #[test]
    fn two_releases_within_300_ms_centre_it() {
        let mut p = touched(0.5);
        p.moved(1, 120.0);
        assert_eq!(p.up(1, 10.0), Some(to_live(to_pos(0.5) + 0.1)));
        assert!(p.down(1, 100.0, 200.0, 200.0, 0.5));
        assert_eq!(p.up(1, 309.0), Some(0.0), "299 ms after the last release");
        assert_eq!(p.frame(310.0, Some(0.5)).pos, Some(0.5));
        // A third release starts over; 300 ms is too late.
        assert!(p.down(1, 100.0, 200.0, 500.0, 0.0));
        assert_eq!(p.up(1, 510.0), None);
        assert!(p.down(1, 100.0, 200.0, 700.0, 0.0));
        assert_eq!(p.up(1, 810.0), None, "300 ms apart");
    }

    #[test]
    fn a_press_says_where_the_touch_started() {
        let mut p = PanCtl::default();
        let _ = p.frame(0.0, Some(0.0));
        let start = |shown, live, local, from| {
            Some(Start {
                shown,
                live,
                local,
                from,
            })
        };
        assert_eq!(
            p.press(1, 100.0, 200.0, 10.0, 0.5),
            start(0.5, 0.75, false, 0.75),
            "from Live's panning, as a position"
        );
        assert_eq!(p.press(2, 0.0, 200.0, 11.0, 0.5), None);
        assert!(p.moved(1, 125.0));
        assert_eq!(p.up(1, 20.0), Some(to_live(0.875)));
        assert_eq!(
            p.press(1, 100.0, 200.0, 119.0, 0.0),
            start(0.875, 0.5, true, 0.875),
            "within the hold"
        );
        assert_eq!(p.cancel(1, 150.0), None);
        let _ = p.frame(251.0, Some(-0.5));
        assert_eq!(
            p.press(1, 100.0, 200.0, 252.0, 1.0),
            start(0.25, 1.0, false, 1.0)
        );
    }

    #[test]
    fn a_cancel_never_centres() {
        let mut p = touched(0.5);
        assert_eq!(p.up(1, 10.0), None);
        assert!(p.down(1, 100.0, 200.0, 50.0, 0.5));
        assert_eq!(p.cancel(1, 60.0), None);
        assert!(p.down(1, 100.0, 200.0, 70.0, 0.5));
        assert_eq!(p.up(1, 80.0), None, "the cancel forgot the first release");
        assert!(p.down(1, 100.0, 200.0, 90.0, 0.5));
        p.moved(1, 110.0);
        assert!(p.cancel(1, 95.0).is_some(), "a cancel sends the last move");
    }

    #[test]
    fn after_a_release_the_pan_holds_100_ms_then_shows_live() {
        let mut p = touched(0.0);
        p.moved(1, 140.0);
        p.frame(5.0, Some(0.0));
        assert_eq!(p.up(1, 10.0), None);
        assert_eq!(p.frame(109.0, Some(-1.0)).pos, Some(0.7));
        assert_eq!(p.frame(110.0, Some(-1.0)).pos, Some(0.0));
        // A touch after the hold starts from Live's value, during it from
        // the shown one.
        assert!(p.down(1, 0.0, 200.0, 400.0, 0.5));
        assert_eq!(p.frame(401.0, None).pos, Some(0.75));
        assert_eq!(p.up(1, 402.0), None, "392 ms after the last release");
        assert!(p.down(1, 0.0, 200.0, 450.0, -1.0));
        assert_eq!(p.frame(451.0, None).pos, Some(0.75));
        p.cancel(1, 460.0);
        assert_eq!(
            p.frame(559.0, Some(-1.0)).pos,
            Some(0.75),
            "a cancel holds too"
        );
        assert_eq!(p.frame(600.0, None).pos, None, "no value from Live");
        assert_eq!(p.frame(600.0, Some(-1.0)).pos, Some(0.0));
    }

    #[test]
    fn the_pan_says_which_pointer_drives_it() {
        let mut p = PanCtl::default();
        assert!(!p.drives(1), "untouched");
        assert!(p.down(1, 100.0, 200.0, 0.0, 0.0));
        assert!(p.drives(1) && !p.drives(2));
        assert_eq!(p.up(2, 5.0), None);
        assert!(p.drives(1), "another pointer's release is not its");
        assert_eq!(p.up(1, 10.0), None, "nothing unsent");
        assert!(!p.drives(1), "released");
        assert!(p.down(3, 100.0, 200.0, 500.0, 0.0));
        p.cancel(3, 510.0);
        assert!(!p.drives(3), "cancelled");
    }

    #[test]
    fn a_touchs_first_move_only_anchors_the_pan() {
        // #43 PR F: the iPad's first pointer event of a touch comes late and
        // several px away from the down; it moves nothing and sends nothing,
        // and the drag goes on from it.
        let mut p = PanCtl::default();
        assert!(p.down(1, 100.0, 64.0, 0.0, 0.0));
        assert!(p.moved(1, 106.0));
        assert_eq!(
            p.frame(80.0, Some(0.0)),
            Motion {
                pos: Some(0.5),
                send: None
            },
            "no jump on the first move's frame"
        );
        assert!(p.moved(1, 114.0));
        assert_eq!(
            p.frame(100.0, Some(0.0)),
            Motion {
                pos: Some(0.625),
                send: Some(0.25)
            },
            "8 px of 64 from the anchor"
        );
        // Another pointer's move is not the touch's first; each touch anchors
        // its own first move.
        assert_eq!(p.up(1, 120.0), None);
        assert!(p.down(2, 100.0, 64.0, 500.0, 0.25));
        assert!(!p.moved(3, 140.0));
        assert!(p.moved(2, 92.0));
        assert_eq!(p.frame(520.0, Some(0.25)).pos, Some(0.625));
        assert!(p.moved(2, 84.0));
        assert_eq!(p.frame(540.0, Some(0.25)).send, Some(0.0));
    }

    #[test]
    fn a_pan_touch_whose_finger_only_anchored_sends_nothing() {
        let mut p = PanCtl::default();
        assert!(p.down(1, 100.0, 64.0, 0.0, 0.0));
        assert!(p.moved(1, 94.0));
        assert_eq!(p.up(1, 50.0), None, "nothing unsent");
        assert_eq!(p.frame(60.0, Some(0.0)).pos, Some(0.5));
    }

    #[test]
    fn a_zero_width_counts_as_one_pixel() {
        let mut p = PanCtl::default();
        p.down(1, 0.0, 0.0, 0.0, -1.0);
        p.moved(1, 0.25);
        assert_eq!(p.frame(1.0, None).pos, Some(0.25));
    }
}
