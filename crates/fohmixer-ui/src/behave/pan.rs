//! The pan control (spec F11; TouchOSC `pan_control.lua` 1.5.3): position
//! `p` ↔ Live panning `2p − 1`, relative drag, two releases within 300 ms
//! glide it to the centre (#63: at iemmixer's 0.4 positions a second; the
//! TouchOSC script jumped), grey when centred and cyan otherwise.

use super::fader::Glide;
use super::{Down, Lost, Motion, Start, Taken, TouchEnd, touch_end};
use crate::diag::perf::Pointer;

/// The colour of a centred pan and of an off-centre one.
pub const CENTERED: &str = "#646464";
pub const OFF_CENTER: &str = "#34C1DC";
/// How far from the centre still counts as centred.
const CENTER_TOLERANCE: f64 = 0.01;
/// Two releases closer than this centre the pan.
pub const DOUBLE_TAP_MS: f64 = 300.0;
/// The double tap's glide to the centre, in positions per second (iemmixer's
/// pan: 0.02 per 50 ms tick; a full side in 1.25 s).
pub const GLIDE_SPEED: f64 = 0.4;
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

/// One pan's input state: one pointer, the relative drag (a touch's first
/// pointer move only anchors it, #43 PR F), the double tap's glide to the
/// centre and the post-release hold.
#[derive(Debug, Clone, PartialEq)]
pub struct PanCtl {
    pointer: Option<i32>,
    /// The held pointer's type (#124 M1: only a primary down of that type
    /// ends a touch whose end never came).
    kind: Option<Pointer>,
    /// The touch's next pointer move is its first.
    first_move: bool,
    last_x: f64,
    travel: f64,
    pos: f64,
    unsent: bool,
    last_release: Option<f64>,
    hold_until: f64,
    glide: Option<Glide>,
    /// A glide ended since the store last heard of it.
    ended: bool,
}

impl Default for PanCtl {
    fn default() -> Self {
        Self {
            pointer: None,
            kind: None,
            first_move: false,
            last_x: 0.0,
            travel: 1.0,
            pos: 0.5,
            unsent: false,
            last_release: None,
            hold_until: f64::NEG_INFINITY,
            glide: None,
            ended: false,
        }
    }
}

impl PanCtl {
    fn local(&self, now: f64) -> bool {
        self.pointer.is_some() || self.glide.is_some() || now < self.hold_until
    }

    /// Pointer `id` pressed at `x` (px) on a pan `travel` px wide, Live's
    /// panning being `live` (`primary`: the browser's `isPrimary`): whether
    /// this pointer now drives the pan.
    pub fn down(
        &mut self,
        id: i32,
        primary: bool,
        x: f64,
        travel: f64,
        now: f64,
        live: f64,
    ) -> bool {
        self.press(Down::touch(id, primary), x, travel, now, live)
            .is_some()
    }

    /// [`PanCtl::down`], with where the touch started when this pointer now
    /// drives the pan (#43 PR D), in positions: the one it showed, Live's
    /// panning's, whether it showed its own (a finger, the hold) and the one
    /// the touch starts from. A held pan refuses a second finger; a primary
    /// down of the held pointer's type ([`Down::ends`]) ends the held touch,
    /// whose end the browser never delivered (#124), as a cancel (`lost`),
    /// and the new finger drives the pan: from Live's panning when that
    /// touch had nothing left to send, else from the dot during the hold.
    pub fn press(&mut self, down: Down, x: f64, travel: f64, now: f64, live: f64) -> Option<Taken> {
        let id = down.id;
        let lost = match self.pointer {
            Some(_) if !down.ends(self.kind) => return None,
            Some(held) => Some(self.lose(held, now)),
            None => None,
        };
        let shown = self.pos;
        let local = self.local(now);
        if !local {
            self.pos = to_pos(live);
        }
        // A touch stops a glide where it is.
        self.glide = None;
        self.pointer = Some(id);
        self.kind = down.kind;
        self.first_move = true;
        self.last_x = x;
        self.travel = travel.max(1.0);
        Some(Taken {
            start: Start {
                shown,
                live: to_pos(live),
                local,
                from: self.pos,
            },
            lost,
        })
    }

    /// Ends pointer `id`'s touch, whose end the browser never delivered
    /// (#124): as a cancel (no release that centres), the panning it left
    /// unsent its end. With nothing left to send no post-release hold starts
    /// from now (#124 I1): the next touch starts from Live's panning; a
    /// panning it left unsent keeps the dot there for the hold.
    fn lose(&mut self, id: i32, now: f64) -> Lost {
        let hold_until = self.hold_until;
        let end = touch_end(true, self.cancel(id, now));
        if end == TouchEnd::Released {
            self.hold_until = hold_until;
        }
        Lost { pointer: id, end }
    }

    /// The page went hidden (#124): a pointer still held lost its end (a
    /// hidden Home Screen app keeps no touch going), and its touch ends as
    /// one a primary down ends, whatever `isPrimary` the next finger
    /// brings. None when no pointer is held.
    pub fn hidden(&mut self, now: f64) -> Option<Lost> {
        let held = self.pointer?;
        Some(self.lose(held, now))
    }

    /// Whether pointer `id` drives this pan (its release is this pan's).
    pub fn drives(&self, id: i32) -> bool {
        self.pointer == Some(id)
    }

    /// The position the pan holds now (the flight recorder's `a` after a
    /// touch's first move, #43 PR F).
    pub fn pos(&self) -> f64 {
        self.pos
    }

    /// Pointer `id` moved to `x`: whether it drives this pan. The touch's
    /// first move only anchors the drag there (#43 PR F, as a fader's: the
    /// iPad's first event of a touch comes late and several px from the
    /// down): nothing moves and nothing is left to send.
    pub fn moved(&mut self, id: i32, x: f64) -> bool {
        if self.pointer != Some(id) {
            return false;
        }
        if std::mem::take(&mut self.first_move) {
            self.last_x = x;
            return true;
        }
        self.pos = (self.pos + (x - self.last_x) / self.travel).clamp(0.0, 1.0);
        self.last_x = x;
        self.unsent = true;
        true
    }

    /// Pointer `id` lifted: the panning still to send, if any. The second
    /// release within 300 ms starts the glide to the centre (its frames
    /// send it).
    pub fn up(&mut self, id: i32, now: f64) -> Option<f64> {
        if self.pointer != Some(id) {
            return None;
        }
        self.pointer = None;
        self.hold_until = now + HOLD_MS;
        let double = self.last_release.is_some_and(|t| now - t < DOUBLE_TAP_MS);
        if double {
            self.last_release = None;
            self.unsent = false;
            self.glide = Some(Glide::at_speed(self.pos, 0.5, now, GLIDE_SPEED));
            return None;
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

    /// Whether a glide ended (arrived, or stopped because Live's value went)
    /// since the last call: the store then marks its last write released.
    pub fn take_ended(&mut self) -> bool {
        std::mem::take(&mut self.ended)
    }

    /// The frame at `now`, Live's panning being `live`.
    pub fn frame(&mut self, now: f64, live: Option<f64>) -> Motion {
        if self.glide.is_some() && live.is_none() {
            // Live's value is gone (its instance went offline): the glide
            // stops where it is and sends nothing more.
            self.glide = None;
            self.ended = true;
            self.hold_until = now + HOLD_MS;
            return Motion {
                pos: Some(self.pos),
                send: None,
            };
        }
        if let Some(glide) = self.glide {
            let (pos, done) = glide.at(now);
            self.pos = pos;
            if done {
                self.glide = None;
                self.ended = true;
                self.hold_until = now + HOLD_MS;
            }
            return Motion {
                pos: Some(pos),
                send: Some(to_live(pos)),
            };
        }
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
    use crate::behave::{Lost, TouchEnd};
    use crate::diag::perf::Pointer;

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
        assert!(p.down(1, true, 100.0, 200.0, 0.0, live));
        p
    }

    /// The touch's first pointer move, at the down's own coordinate: it only
    /// anchors the drag (#43 PR F), so the moves after it are the drag.
    #[track_caller]
    fn anchor(p: &mut PanCtl, id: i32, x: f64) {
        assert!(p.moved(id, x), "pointer {id} drives the pan");
    }

    #[test]
    fn a_drag_moves_the_pan_relative_to_where_it_was() {
        let mut p = touched(0.0);
        anchor(&mut p, 1, 100.0);
        assert!(p.moved(1, 150.0));
        let frame = p.frame(10.0, Some(0.0));
        assert_eq!(frame.pos, Some(0.75));
        assert_eq!(frame.send, Some(0.5));
        assert_eq!(p.frame(20.0, Some(0.0)).send, None, "sent once");
        assert!(p.moved(1, -1000.0));
        assert_eq!(p.frame(30.0, Some(0.0)).send, Some(-1.0));
        // Another pointer does nothing to it.
        assert!(!p.down(2, false, 0.0, 200.0, 40.0, 0.0));
        assert!(!p.moved(2, 50.0));
        assert_eq!(p.up(2, 50.0), None);
        assert_eq!(p.cancel(2, 50.0), None);
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
    }

    #[test]
    fn two_releases_within_300_ms_glide_it_to_the_centre() {
        let mut p = touched(0.5);
        anchor(&mut p, 1, 100.0);
        p.moved(1, 120.0);
        assert_eq!(p.up(1, 10.0), Some(to_live(to_pos(0.5) + 0.1)));
        assert!(p.down(1, true, 100.0, 200.0, 200.0, 0.5));
        assert_eq!(
            p.up(1, 309.0),
            None,
            "299 ms after the last release: no jump"
        );
        assert!(!p.take_ended());
        // 0.4 positions a second from 0.75, sent each frame: 0.65 after
        // 250 ms.
        let frame = p.frame(559.0, Some(0.5));
        assert!(close(frame.pos.unwrap(), 0.65), "{frame:?}");
        assert!(close(frame.send.unwrap(), to_live(0.65)), "{frame:?}");
        assert!(!p.take_ended(), "still on its way");
        // At the centre after 625 ms; the end is told once, then the hold.
        assert_eq!(
            p.frame(934.0, Some(0.5)),
            Motion {
                pos: Some(0.5),
                send: Some(0.0)
            }
        );
        assert!(p.take_ended());
        assert!(!p.take_ended(), "told once");
        assert_eq!(p.frame(1033.0, Some(1.0)).pos, Some(0.5), "the hold");
        assert_eq!(p.frame(1033.0, Some(1.0)).send, None);
        assert_eq!(p.frame(1034.0, Some(1.0)).pos, Some(1.0));
        // A third release starts over; 300 ms is too late: no glide.
        assert!(p.down(1, true, 100.0, 200.0, 1500.0, 0.5));
        assert_eq!(p.up(1, 1510.0), None);
        assert!(p.down(1, true, 100.0, 200.0, 1700.0, 0.5));
        assert_eq!(p.up(1, 1810.0), None, "300 ms apart");
        assert_eq!(
            p.frame(1811.0, Some(0.5)),
            Motion {
                pos: Some(0.75),
                send: None
            },
            "no glide"
        );
        assert!(!p.take_ended());
    }

    #[test]
    fn a_pan_glide_stops_where_it_is_when_lives_value_goes() {
        let mut p = touched(0.5);
        assert_eq!(p.up(1, 10.0), None);
        assert!(p.down(1, true, 100.0, 200.0, 50.0, 0.5));
        assert_eq!(p.up(1, 60.0), None);
        let gliding = p.frame(310.0, Some(0.5));
        assert!(close(gliding.pos.unwrap(), 0.65), "{gliding:?}");
        // The instance went offline: no more sends, the pan holds.
        let stopped = p.frame(320.0, None);
        assert!(close(stopped.pos.unwrap(), 0.65), "{stopped:?}");
        assert_eq!(stopped.send, None);
        assert!(p.take_ended(), "a stopped glide ends");
        let held = p.frame(419.0, Some(0.5));
        assert!(close(held.pos.unwrap(), 0.65), "{held:?}");
        assert_eq!(held.send, None, "the glide does not resume");
        assert_eq!(
            p.frame(420.0, Some(0.5)).pos,
            Some(0.75),
            "then Live's value"
        );
    }

    #[test]
    fn a_touch_stops_the_pan_glide_where_it_is() {
        let mut p = touched(0.5);
        assert_eq!(p.up(1, 10.0), None);
        assert!(p.down(1, true, 100.0, 200.0, 50.0, 0.5));
        assert_eq!(p.up(1, 60.0), None);
        let _ = p.frame(310.0, Some(0.5));
        let at = p.pos();
        assert!(close(at, 0.65), "{at}");
        assert_eq!(
            p.press(Down::touch(2, true), 100.0, 200.0, 320.0, 0.5),
            Some(Taken {
                start: Start {
                    shown: at,
                    live: 0.75,
                    local: true,
                    from: at,
                },
                lost: None,
            }),
            "the touch starts where the glide was"
        );
        let held = p.frame(900.0, Some(0.5));
        assert!(close(held.pos.unwrap(), 0.65), "{held:?}");
        assert_eq!(held.send, None);
        assert!(!p.take_ended(), "a touch is no glide end");
    }

    #[test]
    fn a_press_says_where_the_touch_started() {
        let mut p = PanCtl::default();
        let _ = p.frame(0.0, Some(0.0));
        let start = |shown, live, local, from| {
            Some(Taken {
                start: Start {
                    shown,
                    live,
                    local,
                    from,
                },
                lost: None,
            })
        };
        assert_eq!(
            p.press(Down::touch(1, true), 100.0, 200.0, 10.0, 0.5),
            start(0.5, 0.75, false, 0.75),
            "from Live's panning, as a position"
        );
        assert_eq!(p.press(Down::touch(2, false), 0.0, 200.0, 11.0, 0.5), None);
        anchor(&mut p, 1, 100.0);
        assert!(p.moved(1, 125.0));
        assert_eq!(p.up(1, 20.0), Some(to_live(0.875)));
        assert_eq!(
            p.press(Down::touch(1, true), 100.0, 200.0, 119.0, 0.0),
            start(0.875, 0.5, true, 0.875),
            "within the hold"
        );
        assert_eq!(p.cancel(1, 150.0), None);
        let _ = p.frame(251.0, Some(-0.5));
        assert_eq!(
            p.press(Down::touch(1, true), 100.0, 200.0, 252.0, 1.0),
            start(0.25, 1.0, false, 1.0)
        );
    }

    #[test]
    fn a_second_finger_takes_a_free_pan() {
        let mut p = PanCtl::default();
        let taken = p.press(Down::touch(2, false), 100.0, 200.0, 0.0, 0.0);
        assert_eq!(taken.map(|t| t.lost), Some(None), "it ends no touch");
        assert!(p.drives(2));
    }

    #[test]
    fn a_primary_down_ends_a_touch_whose_end_never_came() {
        // #124, as a fader's: pointer 1's end never came. A second finger is
        // still refused; a primary down ends pointer 1's touch as a cancel
        // and drives the pan from Live's panning (#124 I1: nothing was left
        // to send, so no hold keeps the dot).
        let mut p = touched(0.0);
        anchor(&mut p, 1, 100.0);
        assert!(p.moved(1, 150.0));
        assert_eq!(p.frame(10.0, Some(0.0)).send, Some(0.5));
        assert_eq!(
            p.press(Down::touch(2, false), 0.0, 200.0, 20.0, 0.5),
            None,
            "a second finger"
        );
        assert!(p.drives(1));
        assert_eq!(
            p.press(Down::touch(2, true), 0.0, 200.0, 5_000.0, 0.5),
            Some(Taken {
                start: Start {
                    shown: 0.75,
                    live: 0.75,
                    local: false,
                    from: 0.75,
                },
                lost: Some(Lost {
                    pointer: 1,
                    end: TouchEnd::Released,
                }),
            })
        );
        assert!(p.drives(2) && !p.drives(1));
        // Pointer 1's late events are no longer its touch.
        assert!(!p.moved(1, 300.0));
        assert_eq!(p.up(1, 5_010.0), None);
        assert_eq!(p.cancel(1, 5_010.0), None);
        anchor(&mut p, 2, 0.0);
        assert!(p.moved(2, 50.0));
        assert_eq!(p.frame(5_020.0, Some(0.5)).send, Some(1.0));
        assert_eq!(p.up(2, 5_030.0), None);
    }

    #[test]
    fn a_lost_pan_touch_sends_its_unsent_move_and_is_no_release() {
        // Pointer 1's move was not sent yet when pointer 2 came: it goes as
        // the lost touch's end. The lost touch is no release, so pointer 2's
        // release 50 ms later is a first one and does not centre the pan.
        let mut p = touched(0.0);
        anchor(&mut p, 1, 100.0);
        assert!(p.moved(1, 150.0));
        let lost = p
            .press(Down::touch(2, true), 0.0, 200.0, 100.0, 0.0)
            .and_then(|t| t.lost);
        assert_eq!(
            lost,
            Some(Lost {
                pointer: 1,
                end: TouchEnd::Send(0.5),
            })
        );
        assert_eq!(
            p.frame(110.0, Some(0.0)).send,
            None,
            "sent once, as its end"
        );
        assert_eq!(p.up(2, 150.0), None);
        let after = p.frame(500.0, Some(0.5));
        assert_eq!(after.send, None, "no glide to the centre");
        assert_eq!(after.pos, Some(0.75), "Live's value after the hold");
    }

    /// A pan whose pointer 1 dragged from the centre to 0.75 and whose
    /// frame sent it, its end never delivered (#124).
    fn stuck() -> PanCtl {
        let mut p = touched(0.0);
        anchor(&mut p, 1, 100.0);
        assert!(p.moved(1, 150.0));
        assert_eq!(p.frame(10.0, Some(0.0)).send, Some(0.5));
        p
    }

    #[test]
    fn a_long_lost_touch_hands_the_next_finger_lives_panning() {
        // #124 I1: Live's panning moved to -0.5 (position 0.25) while the
        // dot stayed at 0.75; the next finger starts from Live's, and its
        // first move sends -0.25 (25 px on), never near the stale 0.5.
        let mut p = stuck();
        assert_eq!(p.frame(5_000.0, Some(-0.5)).pos, Some(0.75));
        let Taken { start, lost } = p
            .press(Down::touch(2, true), 0.0, 200.0, 5_000.0, -0.5)
            .expect("a primary down takes the pan");
        assert_eq!(lost.map(|l| l.end), Some(TouchEnd::Released));
        assert_eq!(
            start,
            Start {
                shown: 0.75,
                live: 0.25,
                local: false,
                from: 0.25,
            }
        );
        anchor(&mut p, 2, 0.0);
        assert!(p.moved(2, 25.0));
        assert_eq!(p.frame(5_010.0, Some(-0.5)).send, Some(-0.25));
        // Its unsent move goes as its end: Live is about to hold it, so the
        // next finger starts there.
        let mut p = touched(0.0);
        anchor(&mut p, 1, 100.0);
        assert!(p.moved(1, 150.0));
        let Taken { start, .. } = p
            .press(Down::touch(2, true), 0.0, 200.0, 5_000.0, -0.5)
            .expect("taken");
        assert!(start.local);
        assert_eq!(start.from, 0.75);
    }

    #[test]
    fn a_hidden_page_ends_a_held_pan_touch() {
        assert_eq!(PanCtl::default().hidden(0.0), None, "nothing held");
        let mut p = stuck();
        assert_eq!(
            p.hidden(5_000.0),
            Some(Lost {
                pointer: 1,
                end: TouchEnd::Released,
            })
        );
        assert!(!p.drives(1));
        assert_eq!(p.hidden(5_001.0), None, "once");
        // A second finger takes the free pan, from Live's panning.
        let start = p
            .press(Down::touch(2, false), 0.0, 200.0, 6_000.0, -0.5)
            .expect("taken")
            .start;
        assert_eq!(start.from, 0.25);
    }

    #[test]
    fn a_primary_down_of_another_type_never_ends_a_held_pan_touch() {
        let mut p = stuck();
        let mouse = Down {
            id: 9,
            kind: Some(Pointer::Mouse),
            primary: true,
        };
        assert_eq!(p.press(mouse, 0.0, 200.0, 5_000.0, 0.5), None);
        assert!(p.drives(1));
        assert!(p.down(2, true, 0.0, 200.0, 5_000.0, 0.5));
    }

    #[test]
    fn a_cancel_never_centres() {
        let mut p = touched(0.5);
        assert_eq!(p.up(1, 10.0), None);
        assert!(p.down(1, true, 100.0, 200.0, 50.0, 0.5));
        assert_eq!(p.cancel(1, 60.0), None);
        assert!(p.down(1, true, 100.0, 200.0, 70.0, 0.5));
        assert_eq!(p.up(1, 80.0), None, "the cancel forgot the first release");
        assert!(p.down(1, true, 100.0, 200.0, 90.0, 0.5));
        anchor(&mut p, 1, 100.0);
        p.moved(1, 110.0);
        assert!(p.cancel(1, 95.0).is_some(), "a cancel sends the last move");
    }

    #[test]
    fn after_a_release_the_pan_holds_100_ms_then_shows_live() {
        let mut p = touched(0.0);
        anchor(&mut p, 1, 100.0);
        p.moved(1, 140.0);
        p.frame(5.0, Some(0.0));
        assert_eq!(p.up(1, 10.0), None);
        assert_eq!(p.frame(109.0, Some(-1.0)).pos, Some(0.7));
        assert_eq!(p.frame(110.0, Some(-1.0)).pos, Some(0.0));
        // A touch after the hold starts from Live's value, during it from
        // the shown one.
        assert!(p.down(1, true, 0.0, 200.0, 400.0, 0.5));
        assert_eq!(p.frame(401.0, None).pos, Some(0.75));
        assert_eq!(p.up(1, 402.0), None, "392 ms after the last release");
        assert!(p.down(1, true, 0.0, 200.0, 450.0, -1.0));
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
        assert!(p.down(1, true, 100.0, 200.0, 0.0, 0.0));
        assert!(p.drives(1) && !p.drives(2));
        assert_eq!(p.up(2, 5.0), None);
        assert!(p.drives(1), "another pointer's release is not its");
        assert_eq!(p.up(1, 10.0), None, "nothing unsent");
        assert!(!p.drives(1), "released");
        assert!(p.down(3, true, 100.0, 200.0, 500.0, 0.0));
        p.cancel(3, 510.0);
        assert!(!p.drives(3), "cancelled");
    }

    #[test]
    fn a_touchs_first_move_only_anchors_the_pan() {
        // #43 PR F: the iPad's first pointer event of a touch comes late and
        // several px away from the down; it moves nothing and sends nothing,
        // and the drag goes on from it.
        let mut p = PanCtl::default();
        assert!(p.down(1, true, 100.0, 64.0, 0.0, 0.0));
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
        assert!(p.down(2, true, 100.0, 64.0, 500.0, 0.25));
        assert!(!p.moved(3, 140.0));
        assert!(p.moved(2, 92.0));
        assert_eq!(p.frame(520.0, Some(0.25)).pos, Some(0.625));
        assert!(p.moved(2, 84.0));
        assert_eq!(p.frame(540.0, Some(0.25)).send, Some(0.0));
    }

    #[test]
    fn the_pan_says_where_a_move_left_it() {
        // The flight recorder's `a` (#43 PR F): read right after the move.
        let mut p = PanCtl::default();
        assert!(p.down(1, true, 100.0, 64.0, 0.0, -0.5));
        assert_eq!(p.pos(), 0.25, "the touch's start");
        assert!(p.moved(1, 106.0));
        assert_eq!(p.pos(), 0.25, "the anchor moved nothing");
        assert!(p.moved(1, 122.0));
        assert_eq!(p.pos(), 0.5);
    }

    #[test]
    fn a_pan_touch_whose_finger_only_anchored_sends_nothing() {
        let mut p = PanCtl::default();
        assert!(p.down(1, true, 100.0, 64.0, 0.0, 0.0));
        assert!(p.moved(1, 94.0));
        assert_eq!(p.up(1, 50.0), None, "nothing unsent");
        assert_eq!(p.frame(60.0, Some(0.0)).pos, Some(0.5));
    }

    #[test]
    fn a_zero_width_counts_as_one_pixel() {
        let mut p = PanCtl::default();
        p.down(1, true, 0.0, 0.0, 0.0, -1.0);
        anchor(&mut p, 1, 0.0);
        p.moved(1, 0.25);
        assert_eq!(p.frame(1.0, None).pos, Some(0.25));
    }
}
