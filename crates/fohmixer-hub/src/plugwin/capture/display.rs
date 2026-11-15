//! The display capture's pure parts (#215, `[eq] capture = "wgc-display"`):
//! Windows Graphics Capture of the MONITOR an editor is on, cut to the
//! editor's picture at a grab. Windows 10 makes no capture item of Live's
//! owned editor window (the Alt+Tab rule), a monitor has no such rule, and
//! Live's window styles are never touched. The Win32 and Direct3D glue is
//! `wgc_display.rs` (Windows only, out of the mutation gate); what it
//! decides is here.
//!
//! - **[`Monitors`]:** one session per monitor, shared by the editors on
//!   it; a window moved to another monitor follows, and a monitor no editor
//!   is on any more ends its session.
//! - **[`Frames`]:** a screen change must cost nothing of ours. The pool has
//!   [`BUFFERS`] buffers and the last frame a grab used is HELD in one of
//!   them, so DWM writes at most one further change between two grabs (the
//!   other buffer) and drops the rest; the pool's handler only counts and
//!   wakes. A grab drains the pool to its newest frame ([`Frames::plan`]).
//!   With a buffer still free since the newest frame, no change can have
//!   been dropped: the newest frame is current and is used at once (nothing
//!   queued: the held one). With the pool FULL (the held frame and a queued
//!   one), changes after the queued frame may have been dropped: the held
//!   frame is closed, and the grab waits at most [`WAIT_MS`] for a fresh
//!   arrival ([`Frames::settle`]). Only the editor's box of the frame used
//!   is read back, and that frame stays held.
//! - **The doubt:** no source says whether DWM delivers a change it dropped
//!   once a buffer frees, so the code assumes it does not. A queued frame
//!   used because no fresh one came within the wait may miss a later
//!   change; that picture and every one from the same held frame are
//!   counted `unsure` until a fresh frame, or a queued one with a buffer
//!   free beside it, comes.
//! - **[`display_box`]:** the editor's picture in a frame of its monitor
//!   (both physical pixels under the worker's per-monitor-v2 awareness).

use std::collections::BTreeMap;

use super::{FrameBox, frame_current, picture_box};

/// The frame pool's buffers: the held frame, and one for the first change
/// after it.
pub const BUFFERS: i32 = 2;

/// How long a grab waits for a fresh frame once a change was queued (ms):
/// two 60 Hz compositions, under the worker's 40 ms step.
pub const WAIT_MS: u64 = 34;

/// Which editors' windows are on which monitor (the glue's handles as
/// numbers): one capture session per monitor in use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Monitors {
    of: BTreeMap<usize, isize>,
}

impl Monitors {
    /// `window` is on `monitor` now: the monitor it left, when no other
    /// window is on that one (its session ends).
    pub fn on(&mut self, window: usize, monitor: isize) -> Option<isize> {
        let left = self.of.insert(window, monitor)?;
        (left != monitor && !self.used(left)).then_some(left)
    }

    /// `window` is no editor any more (released): its monitor, when no
    /// other window is on it.
    pub fn off(&mut self, window: usize) -> Option<isize> {
        let left = self.of.remove(&window)?;
        (!self.used(left)).then_some(left)
    }

    /// The monitor `window` is on, as last seen.
    pub fn monitor(&self, window: usize) -> Option<isize> {
        self.of.get(&window).copied()
    }

    /// Whether any window is on `monitor`.
    pub fn used(&self, monitor: isize) -> bool {
        self.of.values().any(|m| *m == monitor)
    }

    /// Every monitor and window forgotten (a lost device ends every
    /// session).
    pub fn clear(&mut self) {
        self.of.clear();
    }
}

/// A frame a grab has: the one held since the last grab, the newest one
/// queued in the pool since, or a fresh one that came while it waited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frame {
    Held,
    Queued,
    Fresh,
}

/// What a grab does before it waits: the held frame closed (a queued one
/// is newer: its buffer is freed for a fresh frame), and whether it waits
/// for a fresh one at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plan {
    pub close_held: bool,
    pub wait: bool,
}

/// How a grab's picture stands to the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// A frame that came while the grab waited.
    Fresh,
    /// The newest frame, and no change can have been dropped since it (a
    /// buffer was free all along).
    Current,
    /// A frame a change dropped while the pool was full may be missing
    /// from.
    Unsure,
}

/// What a grab uses: its frame, the one it closes, how it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settled {
    pub frame: Frame,
    pub close: Option<Frame>,
    pub outcome: Outcome,
}

/// The grabs by outcome (the probe's `fresh=`, `current=`, `unsure=`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub fresh: u64,
    pub current: u64,
    pub unsure: u64,
}

/// One monitor session's frames as the grabs see them: whether one is
/// held, whether it may miss a change, and the grabs' outcomes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frames {
    held: bool,
    doubt: bool,
    tally: Tally,
}

impl Frames {
    /// Whether the pool was full with `queued` frames drained (the held one
    /// counts): DWM may have dropped a change after the newest.
    pub fn full(&self, queued: u32) -> bool {
        i64::from(queued) + i64::from(self.held) >= i64::from(BUFFERS)
    }

    /// Before the wait, `queued` the frames drained from the pool (the
    /// newest kept).
    pub fn plan(&self, queued: u32) -> Plan {
        Plan {
            close_held: self.held && queued > 0,
            wait: self.full(queued) || (queued == 0 && !self.held),
        }
    }

    /// After the wait (`fresh`: a frame came): the frame the grab uses, the
    /// one it closes then (the held one is closed already when a frame was
    /// queued, [`Frames::plan`]), how it stands; none without any frame
    /// (the session's first has not come yet). The frame used stays held.
    pub fn settle(&mut self, queued: u32, fresh: bool) -> Option<Settled> {
        let settled = if fresh {
            let close = if queued > 0 {
                Some(Frame::Queued)
            } else {
                self.held.then_some(Frame::Held)
            };
            self.doubt = false;
            Settled {
                frame: Frame::Fresh,
                close,
                outcome: Outcome::Fresh,
            }
        } else if queued > 0 {
            self.doubt = self.full(queued);
            Settled {
                frame: Frame::Queued,
                close: None,
                outcome: self.outcome(),
            }
        } else if self.held {
            Settled {
                frame: Frame::Held,
                close: None,
                outcome: self.outcome(),
            }
        } else {
            return None;
        };
        self.held = true;
        match settled.outcome {
            Outcome::Fresh => self.tally.fresh += 1,
            Outcome::Current => self.tally.current += 1,
            Outcome::Unsure => self.tally.unsure += 1,
        }
        Some(settled)
    }

    /// A frame not fresh: unsure while the doubt stands.
    fn outcome(&self) -> Outcome {
        if self.doubt {
            Outcome::Unsure
        } else {
            Outcome::Current
        }
    }

    /// The held frame was given up (the pool made anew).
    pub fn dropped(&mut self) {
        self.held = false;
        self.doubt = false;
    }

    /// Whether a frame is held.
    pub fn held(&self) -> bool {
        self.held
    }

    /// The grabs' outcomes so far.
    pub fn tally(&self) -> Tally {
        self.tally
    }
}

/// The editor's picture (its client area at the screen point `picture`,
/// `size`) in a frame of `content` size of the monitor whose rectangle is
/// `monitor` (left, top, right, bottom): the frame must be of the
/// monitor's size now ([`frame_current`]: a frame from before a change of
/// resolution is not taken), the picture cut at the frame's right and
/// bottom edges ([`picture_box`]).
pub fn display_box(
    monitor: (i32, i32, i32, i32),
    content: (u32, u32),
    picture: (i32, i32),
    size: (u32, u32),
) -> Result<FrameBox, String> {
    let side = |from: i32, to: i32| u32::try_from(i64::from(to) - i64::from(from)).unwrap_or(0);
    let screen = (side(monitor.0, monitor.2), side(monitor.1, monitor.3));
    if !frame_current(content, screen) {
        return Err(format!(
            "the screen's newest frame is {}x{}, the screen {}x{}: one of its size awaited",
            content.0, content.1, screen.0, screen.1
        ));
    }
    picture_box((monitor.0, monitor.1), picture, size, content)
        .ok_or_else(|| "the editor's picture lies outside its monitor".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_constants_are_the_designs() {
        assert_eq!(BUFFERS, 2);
        assert_eq!(WAIT_MS, 34);
    }

    #[test]
    fn editors_share_their_monitors_session_and_a_moved_one_follows() {
        let mut monitors = Monitors::default();
        assert_eq!(monitors.on(1, 10), None, "a first monitor leaves none");
        assert_eq!(monitors.monitor(1), Some(10));
        assert!(monitors.used(10));
        assert!(!monitors.used(20));
        assert_eq!(monitors.on(1, 10), None, "the same monitor again");
        assert_eq!(monitors.on(2, 10), None, "a second editor shares it");
        assert_eq!(monitors.on(1, 20), None, "10 is still the second's");
        assert_eq!(monitors.monitor(1), Some(20));
        assert_eq!(monitors.on(2, 20), Some(10), "nobody on 10 any more");
        assert_eq!(monitors.off(1), None, "20 is still the second's");
        assert_eq!(monitors.monitor(1), None);
        assert_eq!(monitors.off(2), Some(20));
        assert_eq!(monitors.off(2), None, "released twice");
        assert!(!monitors.used(20));
        monitors.on(3, 30);
        monitors.clear();
        assert_eq!(monitors, Monitors::default());
    }

    #[test]
    fn the_first_grab_waits_for_the_first_frame() {
        let mut frames = Frames::default();
        assert!(!frames.held());
        assert!(!frames.full(0));
        assert_eq!(
            frames.plan(0),
            Plan {
                close_held: false,
                wait: true
            }
        );
        assert_eq!(frames.settle(0, false), None, "no frame yet");
        assert!(!frames.held());
        assert_eq!(
            frames.settle(0, true),
            Some(Settled {
                frame: Frame::Fresh,
                close: None,
                outcome: Outcome::Fresh
            })
        );
        assert!(frames.held());
        assert_eq!(
            frames.tally(),
            Tally {
                fresh: 1,
                current: 0,
                unsure: 0
            }
        );
    }

    #[test]
    fn the_first_frame_queued_beside_a_free_buffer_is_current_at_once() {
        // The session's first frame came before the first grab: nothing
        // held, so a buffer stayed free and no change was dropped.
        let mut frames = Frames::default();
        assert!(!frames.full(1));
        assert_eq!(
            frames.plan(1),
            Plan {
                close_held: false,
                wait: false
            }
        );
        assert_eq!(
            frames.settle(1, false),
            Some(Settled {
                frame: Frame::Queued,
                close: None,
                outcome: Outcome::Current
            })
        );
        // Two queued with nothing held: the pool was full.
        let mut frames = Frames::default();
        assert!(frames.full(2));
        assert_eq!(
            frames.plan(2),
            Plan {
                close_held: false,
                wait: true
            }
        );
        assert_eq!(
            frames.settle(2, false).map(|s| s.outcome),
            Some(Outcome::Unsure)
        );
    }

    #[test]
    fn a_still_monitor_uses_the_held_frame_at_once() {
        let mut frames = Frames::default();
        frames.settle(0, true);
        assert!(!frames.full(0), "the held frame and a free buffer");
        assert_eq!(
            frames.plan(0),
            Plan {
                close_held: false,
                wait: false
            },
            "nothing queued: the monitor did not change"
        );
        assert_eq!(
            frames.settle(0, false),
            Some(Settled {
                frame: Frame::Held,
                close: None,
                outcome: Outcome::Current
            })
        );
        assert_eq!(frames.tally().current, 1);
    }

    #[test]
    fn a_change_frees_the_held_buffer_and_a_fresh_frame_replaces_the_queued_one() {
        let mut frames = Frames::default();
        frames.settle(0, true);
        assert!(frames.full(1), "the held frame and a queued one");
        assert_eq!(
            frames.plan(1),
            Plan {
                close_held: true,
                wait: true
            }
        );
        assert_eq!(
            frames.settle(1, true),
            Some(Settled {
                frame: Frame::Fresh,
                close: Some(Frame::Queued),
                outcome: Outcome::Fresh
            })
        );
        assert_eq!(frames.tally().fresh, 2);
    }

    #[test]
    fn a_queued_frame_of_a_full_pool_with_no_fresh_one_is_unsure_until_a_fresh_one_comes() {
        let mut frames = Frames::default();
        frames.settle(0, true);
        assert_eq!(
            frames.settle(1, false),
            Some(Settled {
                frame: Frame::Queued,
                close: None,
                outcome: Outcome::Unsure
            })
        );
        assert!(frames.held());
        // The monitor still since: the same held frame, still unsure.
        assert_eq!(
            frames.settle(0, false).map(|s| (s.frame, s.outcome)),
            Some((Frame::Held, Outcome::Unsure))
        );
        // A fresh frame ends the doubt; the held one goes.
        assert_eq!(
            frames.settle(0, true),
            Some(Settled {
                frame: Frame::Fresh,
                close: Some(Frame::Held),
                outcome: Outcome::Fresh
            })
        );
        assert_eq!(
            frames.settle(0, false).map(|s| s.outcome),
            Some(Outcome::Current)
        );
        assert_eq!(
            frames.tally(),
            Tally {
                fresh: 2,
                current: 1,
                unsure: 2
            }
        );
    }

    #[test]
    fn a_dropped_frame_is_no_frame_and_no_doubt() {
        let mut frames = Frames::default();
        frames.settle(0, true);
        frames.settle(1, false);
        frames.dropped();
        assert!(!frames.held());
        assert!(frames.plan(0).wait);
        assert_eq!(frames.settle(0, false), None);
        // A queued frame beside a free buffer ends the doubt too.
        assert_eq!(
            frames.settle(1, false).map(|s| (s.close, s.outcome)),
            Some((None, Outcome::Current))
        );
        assert_eq!(
            frames.settle(0, false).map(|s| s.outcome),
            Some(Outcome::Current),
            "the doubt went with the frame"
        );
    }

    #[test]
    fn the_editors_box_lies_in_its_monitors_frame() {
        // A second monitor right of a 2560-wide first one.
        let monitor = (2560, 0, 4480, 1080);
        assert_eq!(
            display_box(monitor, (1920, 1080), (2568, 31), (1349, 809)),
            Ok(FrameBox {
                left: 8,
                top: 31,
                width: 1349,
                height: 809
            })
        );
        // Cut at the monitor's right edge.
        assert_eq!(
            display_box(monitor, (1920, 1080), (4000, 100), (1349, 809)),
            Ok(FrameBox {
                left: 1440,
                top: 100,
                width: 480,
                height: 809
            })
        );
        // Left of the monitor (the window straddles the first one): none.
        assert_eq!(
            display_box(monitor, (1920, 1080), (2559, 31), (10, 10)),
            Err("the editor's picture lies outside its monitor".to_string())
        );
        // A monitor left of and above the primary one.
        assert_eq!(
            display_box((-1920, -1080, 0, 0), (1920, 1080), (-10, -20), (10, 20)),
            Ok(FrameBox {
                left: 1910,
                top: 1060,
                width: 10,
                height: 20
            })
        );
        // A frame of another size (the resolution changed): not taken.
        assert_eq!(
            display_box(monitor, (2560, 1440), (2568, 31), (10, 10)),
            Err(
                "the screen's newest frame is 2560x1440, the screen 1920x1080: one of its size awaited"
                    .to_string()
            )
        );
        assert!(display_box(monitor, (1922, 1078), (2568, 31), (10, 10)).is_ok());
        assert!(display_box(monitor, (1923, 1080), (2568, 31), (10, 10)).is_err());
        // A monitor rectangle upside down reads as empty.
        assert!(display_box((10, 10, 0, 0), (0, 0), (10, 10), (1, 1)).is_err());
    }
}
