//! The capture's pure parts (#215): which grab the Windows backend uses,
//! the bookkeeping of a Windows Graphics Capture session's frames, and
//! where the picture lies in a captured frame. The Win32 and Direct3D glue
//! is `wgc.rs` (Windows only, out of the mutation gate); what it decides is
//! here, tested on every platform.
//!
//! - **[`Capture`]** (`[eq] capture`): `wgc`, the default, reads the
//!   editor window's own DWM surface on the GPU and gets a frame only when
//!   the window changed; `blt`, the old costly path, a `BitBlt` of the
//!   screen's DC at every grab (dasio's sampler, #215: 16-28 ms each, the
//!   system cores' DPCs and the master Live's cores loaded while the EQ
//!   screen was open). `blt` stays so the PC can switch back without a
//!   rebuild.
//! - **[`Arrivals`]:** what a session delivered: a frame since the last
//!   grab ([`Arrivals::changed`], the worker's change-driven rate), none for
//!   [`STALE_MS`] ([`Arrivals::stale`]: the session is started anew, which
//!   always delivers the current content first), and the counts the probe
//!   prints ([`CaptureCounts`]).
//! - **[`picture_box`]:** Pro-Q's picture (its child window's client area)
//!   in a frame of Live's top-level window, whose origin is the window's
//!   extended frame bounds (as OBS places a window's client area in its
//!   WGC frames).

use serde::Deserialize;

/// How the Windows backend grabs an editor's picture (`[eq] capture`,
/// #215).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capture {
    /// Windows Graphics Capture: the editor window's own DWM surface, a
    /// frame only when it changed, read back only when the worker grabs.
    #[default]
    Wgc,
    /// The old costly path: a `BitBlt` of the picture's rectangle from the
    /// screen's DC at every grab.
    Blt,
}

impl Capture {
    /// Its name in the config and the probe's lines.
    pub fn name(self) -> &'static str {
        match self {
            Self::Wgc => "wgc",
            Self::Blt => "blt",
        }
    }

    /// The capture a name names (`eq-probe --capture`).
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "wgc" => Some(Self::Wgc),
            "blt" => Some(Self::Blt),
            _ => None,
        }
    }
}

/// How far behind the screen a WGC picture may be (ms): DWM delivers a
/// changed window's frame after its composition, so with no frame since
/// the newest one the window had not changed up to about one composition
/// ago; two 60 Hz compositions, conservative. The worker takes it off a
/// grab's clock before it says which input the picture shows
/// ([`super::Shown`]). The probe prints the delivery lag measured on the PC
/// (`lag_ms_max=`).
pub const WGC_LAG_MS: f64 = 34.0;

/// A session that delivered no frame for this long (ms) is started anew
/// at the next grab: a new session delivers the window's current content
/// first, so a session that stopped delivering (a lost device, a display
/// change) never leaves a still picture for good.
pub const STALE_MS: f64 = 5000.0;

/// A capture's own counts (the probe's `arrived=` and `lag_ms_max=`): the
/// frames delivered and the largest delay from a frame's composition to
/// its arrival (ms).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CaptureCounts {
    pub arrived: u64,
    pub lag_ms_max: f64,
}

/// The frames a capture session delivered (ms on the capture's clock).
#[derive(Debug, Clone, PartialEq)]
pub struct Arrivals {
    started: f64,
    last: Option<f64>,
    arrived: u64,
    taken: u64,
    lag_max: f64,
}

impl Arrivals {
    /// A session started at `started`, no frame yet.
    pub fn new(started: f64) -> Self {
        Self {
            started,
            last: None,
            arrived: 0,
            taken: 0,
            lag_max: 0.0,
        }
    }

    /// A frame arrived at `at`, `lag_ms` after its composition.
    pub fn arrived(&mut self, at: f64, lag_ms: f64) {
        self.arrived += 1;
        self.last = Some(at);
        self.lag_max = self.lag_max.max(lag_ms);
    }

    /// Whether a frame arrived since the last grab ([`Arrivals::taken`]).
    pub fn changed(&self) -> bool {
        self.arrived != self.taken
    }

    /// A grab took the newest frame.
    pub fn taken(&mut self) {
        self.taken = self.arrived;
    }

    /// Whether any frame arrived (a grab before the first has no picture).
    pub fn any(&self) -> bool {
        self.arrived > 0
    }

    /// Whether the session delivered nothing for [`STALE_MS`] at `now`,
    /// counted from its last frame or, before one, from its start.
    pub fn stale(&self, now: f64) -> bool {
        now - self.last.unwrap_or(self.started) >= STALE_MS
    }

    /// Its counts so far.
    pub fn counts(&self) -> CaptureCounts {
        CaptureCounts {
            arrived: self.arrived,
            lag_ms_max: self.lag_max,
        }
    }
}

/// A box in a captured frame (px).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameBox {
    pub left: u32,
    pub top: u32,
    pub width: u32,
    pub height: u32,
}

/// The picture's box in a frame of `content` size whose top-left corner
/// is the screen point `frame` (the window's extended frame bounds), the
/// picture's client area at the screen point `picture` with the size
/// `size`: cut at the frame's right and bottom edge (as OBS cuts a
/// window's client area); none when the picture starts outside the frame
/// or nothing of it lies inside.
pub fn picture_box(
    frame: (i32, i32),
    picture: (i32, i32),
    size: (u32, u32),
    content: (u32, u32),
) -> Option<FrameBox> {
    let left = u32::try_from(i64::from(picture.0) - i64::from(frame.0)).ok()?;
    let top = u32::try_from(i64::from(picture.1) - i64::from(frame.1)).ok()?;
    let width = size.0.min(content.0.checked_sub(left)?);
    let height = size.1.min(content.1.checked_sub(top)?);
    (width > 0 && height > 0).then_some(FrameBox {
        left,
        top,
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_capture_is_wgc_unless_the_config_names_blt() {
        assert_eq!(Capture::default(), Capture::Wgc);
        assert_eq!(Capture::named("wgc"), Some(Capture::Wgc));
        assert_eq!(Capture::named("blt"), Some(Capture::Blt));
        assert_eq!(Capture::named("gdi"), None);
        assert_eq!(Capture::named(""), None);
        assert_eq!(Capture::Wgc.name(), "wgc");
        assert_eq!(Capture::Blt.name(), "blt");
        assert_eq!(WGC_LAG_MS, 34.0);
        assert_eq!(STALE_MS, 5000.0);
    }

    #[test]
    fn a_frame_since_the_last_grab_is_a_change_and_a_grab_takes_it() {
        let mut arrivals = Arrivals::new(100.0);
        assert!(!arrivals.any());
        assert!(!arrivals.changed(), "nothing delivered yet");
        arrivals.arrived(120.0, 3.0);
        assert!(arrivals.any());
        assert!(arrivals.changed());
        arrivals.taken();
        assert!(!arrivals.changed(), "taken");
        arrivals.arrived(140.0, 9.5);
        arrivals.arrived(150.0, 2.0);
        assert!(arrivals.changed());
        assert_eq!(
            arrivals.counts(),
            CaptureCounts {
                arrived: 3,
                lag_ms_max: 9.5
            }
        );
        arrivals.taken();
        assert!(!arrivals.changed());
        assert_eq!(arrivals.counts().arrived, 3, "a grab counts no frame");
    }

    #[test]
    fn a_session_with_no_frame_for_5_s_is_stale_from_its_start_or_its_last_frame() {
        // The bounds a millisecond's thousandth apart (a sum's rounding
        // would swallow `next_down`).
        let mut arrivals = Arrivals::new(1000.0);
        assert!(!arrivals.stale(1000.0 + STALE_MS - 0.001));
        assert!(arrivals.stale(1000.0 + STALE_MS));
        arrivals.arrived(4000.0, 0.0);
        assert!(!arrivals.stale(4000.0 + STALE_MS - 0.001));
        assert!(arrivals.stale(4000.0 + STALE_MS));
        assert!(!arrivals.stale(6000.0), "a frame since its start");
    }

    #[test]
    fn the_picture_lies_in_the_frame_at_its_offset_cut_at_the_frames_edges() {
        // Live's window: its frame bounds at (100, 50), Pro-Q's picture at
        // (108, 81), 1349 x 809, in a 1365 x 848 frame.
        assert_eq!(
            picture_box((100, 50), (108, 81), (1349, 809), (1365, 848)),
            Some(FrameBox {
                left: 8,
                top: 31,
                width: 1349,
                height: 809
            })
        );
        // A frame smaller than the picture's offset and size: cut.
        assert_eq!(
            picture_box((100, 50), (108, 81), (1349, 809), (1300, 800)),
            Some(FrameBox {
                left: 8,
                top: 31,
                width: 1292,
                height: 769
            })
        );
        // Exactly at the frame's corner, exactly its size.
        assert_eq!(
            picture_box((-20, -30), (-20, -30), (10, 20), (10, 20)),
            Some(FrameBox {
                left: 0,
                top: 0,
                width: 10,
                height: 20
            })
        );
        // Left of or above the frame, or past it: none.
        assert_eq!(picture_box((100, 50), (99, 81), (10, 10), (100, 100)), None);
        assert_eq!(
            picture_box((100, 50), (108, 49), (10, 10), (100, 100)),
            None
        );
        assert_eq!(picture_box((0, 0), (100, 0), (10, 10), (100, 100)), None);
        assert_eq!(picture_box((0, 0), (0, 100), (10, 10), (100, 100)), None);
        assert_eq!(picture_box((0, 0), (101, 0), (10, 10), (100, 100)), None);
        // An empty picture: none.
        assert_eq!(picture_box((0, 0), (0, 0), (0, 10), (100, 100)), None);
        assert_eq!(picture_box((0, 0), (0, 0), (10, 0), (100, 100)), None);
        // One pixel inside the right and bottom edges.
        assert_eq!(
            picture_box((0, 0), (99, 99), (10, 10), (100, 100)),
            Some(FrameBox {
                left: 99,
                top: 99,
                width: 1,
                height: 1
            })
        );
    }
}
