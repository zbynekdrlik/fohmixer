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
//!   grab ([`Arrivals::changed`], the worker's change-driven rate), any at
//!   all, and the counts the probe prints ([`CaptureCounts`]). A window
//!   that stays the same sends no frame, so silence is never a lost
//!   session: the worker's 1 s refresh sends the newest frame again, and a
//!   session is started anew only when Windows ends it (the window closed)
//!   or the GPU device is lost, keeping its newest frame meanwhile (the #215
//!   review).
//! - **[`picture_box`]:** Pro-Q's picture (its child window's client area)
//!   in a frame of Live's top-level window, whose origin is the window's
//!   extended frame bounds (as OBS places a window's client area in its
//!   WGC frames); **[`frame_current`]:** whether the newest frame is of the
//!   window's size now (a frame from before a resize is not: its box would
//!   be cut, or show the old layout).

use serde::Deserialize;

pub mod display;

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
    /// Windows Graphics Capture of the MONITOR the editor is on, cut to the
    /// editor's picture at a grab (#215: Windows 10 makes no capture item of
    /// Live's owned editor window, and Live's window styles are never
    /// touched). Nothing of ours runs per screen change: the frames are read
    /// only at a grab ([`DisplayPick`]).
    #[serde(rename = "wgc-display")]
    WgcDisplay,
}

impl Capture {
    /// Its name in the config and the probe's lines.
    pub fn name(self) -> &'static str {
        match self {
            Self::Wgc => "wgc",
            Self::Blt => "blt",
            Self::WgcDisplay => "wgc-display",
        }
    }

    /// The capture a name names (`eq-probe --capture`).
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "wgc" => Some(Self::Wgc),
            "blt" => Some(Self::Blt),
            "wgc-display" => Some(Self::WgcDisplay),
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

/// How far a frame's size may lie from the window's frame bounds (px,
/// each side) and still be of the window's size now ([`frame_current`]).
pub const FRAME_SLACK: u32 = 2;

/// A capture's own counts (the probe's `arrived=` and `lag_ms_max=`): the
/// frames delivered and the largest delay from a frame's composition to
/// its arrival (ms); a display capture's grabs by outcome too (`fresh=`,
/// `current=`, `unsure=`, [`display::Tally`]).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CaptureCounts {
    pub arrived: u64,
    pub lag_ms_max: f64,
    pub display: Option<display::Tally>,
}

/// The frames a capture delivered.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Arrivals {
    arrived: u64,
    taken: u64,
    lag_max: f64,
}

impl Arrivals {
    /// A frame arrived, `lag_ms` after its composition.
    pub fn arrived(&mut self, lag_ms: f64) {
        self.arrived += 1;
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

    /// Its counts so far.
    pub fn counts(&self) -> CaptureCounts {
        CaptureCounts {
            arrived: self.arrived,
            lag_ms_max: self.lag_max,
            display: None,
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

/// Whether a frame of `content` size is of the window's size now
/// (`window`: its extended frame bounds' size), within [`FRAME_SLACK`] on
/// each side.
pub fn frame_current(content: (u32, u32), window: (u32, u32)) -> bool {
    content.0.abs_diff(window.0) <= FRAME_SLACK && content.1.abs_diff(window.1) <= FRAME_SLACK
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

/// What decides whether Windows Graphics Capture can start on a window
/// (#215, the PC's 19044 answered `CreateForWindow` with `E_INVALIDARG` for
/// Live's editor): the window's facts and the capturing thread's, read
/// before the start. The class name's length only, never a title (a title
/// holds a track name).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureDiag {
    /// The window class name's length (0: none read).
    pub class_len: usize,
    /// `IsWindow`.
    pub is_window: bool,
    /// `GetAncestor(GA_ROOT)` is the window itself.
    pub root: bool,
    /// `GetWindow(GW_OWNER)` names a window.
    pub owned: bool,
    /// `WS_EX_TOOLWINDOW`.
    pub tool: bool,
    /// `WS_EX_APPWINDOW`.
    pub app: bool,
    /// `WS_EX_NOACTIVATE`.
    pub noactivate: bool,
    /// `DWMWA_CLOAKED` (0: not cloaked; `None`: not read).
    pub cloaked: Option<u32>,
    /// The thread's apartment (`CoGetApartmentType`: `mta`, `sta`,
    /// `main_sta`, `na`, `none` or the error).
    pub apartment: String,
    /// The thread's DPI awareness (`unaware`, `system`, `per_monitor`,
    /// `invalid`).
    pub dpi: String,
    /// `GraphicsCaptureSession::IsSupported()` (`None`: the call failed).
    pub supported: Option<bool>,
}

/// A flag as the diagnostic line writes it.
fn yes(on: bool) -> &'static str {
    if on { "yes" } else { "no" }
}

impl CaptureDiag {
    /// Whether Alt+Tab would list the window: what Windows 10 asks of a
    /// window before it makes a capture item of it (OyasumiVR PR 468,
    /// measured on 19045: "no WS_EX_TOOLWINDOW, no WS_EX_NOACTIVATE and no
    /// owner, unless WS_EX_APPWINDOW is set"; Windows 11 takes any).
    pub fn alt_tab(&self) -> bool {
        self.app || (!self.tool && !self.noactivate && !self.owned)
    }

    /// Its facts as one `key=value` line's value (the probe's
    /// `capture_diag=`, the hub's failed start).
    pub fn text(&self) -> String {
        let cloaked = self.cloaked.map_or("-".to_string(), |c| c.to_string());
        let supported = self.supported.map_or("-", yes);
        format!(
            "class_len={} is_window={} root={} owned={} tool={} app={} noactivate={} cloaked={cloaked} apartment={} dpi={} supported={supported} alt_tab={}",
            self.class_len,
            yes(self.is_window),
            yes(self.root),
            yes(self.owned),
            yes(self.tool),
            yes(self.app),
            yes(self.noactivate),
            self.apartment,
            self.dpi,
            yes(self.alt_tab()),
        )
    }
}

/// A thread's apartment as the diagnostic line writes it:
/// `CoGetApartmentType`'s `APTTYPE` (0 STA, 1 MTA, `mta_implicit` when
/// its qualifier says the thread only joined the MTA implicitly, 2 NA, 3
/// main STA), else its number.
pub fn apartment_name(apttype: i32, implicit: bool) -> String {
    match apttype {
        0 => "sta".to_string(),
        1 if implicit => "mta_implicit".to_string(),
        1 => "mta".to_string(),
        2 => "na".to_string(),
        3 => "main_sta".to_string(),
        other => format!("apttype_{other}"),
    }
}

/// A thread's DPI awareness as the diagnostic line writes it:
/// `GetAwarenessFromDpiAwarenessContext`'s `DPI_AWARENESS` (0 unaware, 1
/// system, 2 per monitor; `v2` when the context is per-monitor v2), else
/// `invalid`.
pub fn awareness_name(awareness: i32, v2: bool) -> &'static str {
    match (awareness, v2) {
        (0, _) => "unaware",
        (1, _) => "system",
        (2, true) => "per_monitor_v2",
        (2, false) => "per_monitor",
        _ => "invalid",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_threads_apartment_and_dpi_awareness_have_their_names() {
        assert_eq!(apartment_name(0, false), "sta");
        assert_eq!(apartment_name(0, true), "sta");
        assert_eq!(apartment_name(1, false), "mta");
        assert_eq!(apartment_name(1, true), "mta_implicit");
        assert_eq!(apartment_name(2, false), "na");
        assert_eq!(apartment_name(3, false), "main_sta");
        assert_eq!(apartment_name(-1, false), "apttype_-1");
        assert_eq!(apartment_name(4, true), "apttype_4");
        assert_eq!(awareness_name(0, false), "unaware");
        assert_eq!(awareness_name(0, true), "unaware");
        assert_eq!(awareness_name(1, false), "system");
        assert_eq!(awareness_name(1, true), "system");
        assert_eq!(awareness_name(2, true), "per_monitor_v2");
        assert_eq!(awareness_name(2, false), "per_monitor");
        assert_eq!(awareness_name(-1, false), "invalid");
        assert_eq!(awareness_name(3, true), "invalid");
    }

    /// A plain top-level window's facts on an MTA thread.
    fn plain() -> CaptureDiag {
        CaptureDiag {
            class_len: 15,
            is_window: true,
            root: true,
            owned: false,
            tool: false,
            app: false,
            noactivate: false,
            cloaked: Some(0),
            apartment: "mta".to_string(),
            dpi: "per_monitor".to_string(),
            supported: Some(true),
        }
    }

    #[test]
    fn alt_tab_lists_a_window_with_no_owner_tool_or_noactivate_style_or_with_appwindow() {
        assert!(plain().alt_tab());
        let mut owned = plain();
        owned.owned = true;
        assert!(!owned.alt_tab(), "an owned window: Live's editor");
        let mut tool = plain();
        tool.tool = true;
        assert!(!tool.alt_tab());
        let mut quiet = plain();
        quiet.noactivate = true;
        assert!(!quiet.alt_tab());
        let mut app = plain();
        app.owned = true;
        app.tool = true;
        app.noactivate = true;
        app.app = true;
        assert!(app.alt_tab(), "WS_EX_APPWINDOW lists any");
    }

    #[test]
    fn the_diagnostic_line_names_every_fact_and_no_title() {
        assert_eq!(
            plain().text(),
            "class_len=15 is_window=yes root=yes owned=no tool=no app=no noactivate=no cloaked=0 apartment=mta dpi=per_monitor supported=yes alt_tab=yes"
        );
        let other = CaptureDiag {
            class_len: 0,
            is_window: false,
            root: false,
            owned: true,
            tool: true,
            app: true,
            noactivate: true,
            cloaked: None,
            apartment: "sta".to_string(),
            dpi: "unaware".to_string(),
            supported: None,
        };
        assert_eq!(
            other.text(),
            "class_len=0 is_window=no root=no owned=yes tool=yes app=yes noactivate=yes cloaked=- apartment=sta dpi=unaware supported=- alt_tab=yes"
        );
        let mut cloaked = plain();
        cloaked.cloaked = Some(2);
        cloaked.supported = Some(false);
        cloaked.owned = true;
        assert!(
            cloaked
                .text()
                .ends_with(" cloaked=2 apartment=mta dpi=per_monitor supported=no alt_tab=no")
        );
    }

    #[test]
    fn the_capture_is_wgc_unless_the_config_names_blt() {
        assert_eq!(Capture::default(), Capture::Wgc);
        assert_eq!(Capture::named("wgc"), Some(Capture::Wgc));
        assert_eq!(Capture::named("blt"), Some(Capture::Blt));
        assert_eq!(Capture::named("gdi"), None);
        assert_eq!(Capture::named(""), None);
        assert_eq!(Capture::Wgc.name(), "wgc");
        assert_eq!(Capture::Blt.name(), "blt");
        assert_eq!(Capture::named("wgc-display"), Some(Capture::WgcDisplay));
        assert_eq!(Capture::named("wgc_display"), None);
        assert_eq!(Capture::WgcDisplay.name(), "wgc-display");
        assert_eq!(WGC_LAG_MS, 34.0);
        assert_eq!(FRAME_SLACK, 2);
    }

    #[test]
    fn a_frame_since_the_last_grab_is_a_change_and_a_grab_takes_it() {
        let mut arrivals = Arrivals::default();
        assert!(!arrivals.any());
        assert!(!arrivals.changed(), "nothing delivered yet");
        arrivals.arrived(3.0);
        assert!(arrivals.any());
        assert!(arrivals.changed());
        arrivals.taken();
        assert!(!arrivals.changed(), "taken");
        arrivals.arrived(9.5);
        arrivals.arrived(2.0);
        assert!(arrivals.changed());
        assert_eq!(
            arrivals.counts(),
            CaptureCounts {
                arrived: 3,
                lag_ms_max: 9.5,
                display: None
            }
        );
        arrivals.taken();
        assert!(!arrivals.changed());
        assert_eq!(arrivals.counts().arrived, 3, "a grab counts no frame");
    }

    #[test]
    fn a_frame_is_of_the_windows_size_now_within_2_px_each_side() {
        assert!(frame_current((1365, 848), (1365, 848)));
        assert!(frame_current((1367, 846), (1365, 848)));
        assert!(frame_current((1363, 850), (1365, 848)));
        assert!(!frame_current((1368, 848), (1365, 848)), "wider");
        assert!(!frame_current((1362, 848), (1365, 848)), "narrower");
        assert!(!frame_current((1365, 851), (1365, 848)), "taller");
        assert!(!frame_current((1365, 845), (1365, 848)), "shorter");
        assert!(!frame_current((776, 1310), (1365, 848)), "before a resize");
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
