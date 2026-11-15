//! Windows Graphics Capture of the MONITOR a Pro-Q 4 editor is on, cut to
//! the editor's picture at a grab (#215, `[eq] capture = "wgc-display"`;
//! what it rests on: `.claude/rules/plugin-window.md`, Capture). Windows 10
//! makes no capture item of Live's owned editor window (the Alt+Tab rule,
//! `CreateForWindow` E_INVALIDARG on the PC's 19044); a monitor has no such
//! rule, and Live's window styles are never touched. Compiled on Windows
//! only and out of the mutation gate, as `wgc.rs`: what it decides is the
//! tested `capture/display.rs` ([`Monitors`], [`Frames`], [`display_box`]).
//!
//! - **One session per monitor** (`MonitorFromWindow`, the nearest),
//!   shared by the editors on it; looked up at every grab, so a window
//!   moved to another monitor follows, and a monitor no editor is on ends
//!   its session. A free-threaded pool of [`BUFFERS`] BGRA buffers, the
//!   cursor left out, the yellow border off where Windows allows it (from
//!   20348; the PC's 19044 frames the whole monitor while it runs).
//! - **Nothing of ours per screen change:** the pool's `FrameArrived`
//!   handler only counts and wakes a grab that waits. The frame the last
//!   grab used stays HELD, so DWM writes at most one further change into
//!   the pool between two grabs and drops the rest.
//! - **A grab** drains the pool to its newest frame (closing the older),
//!   waits for one fresh arrival at most [`WAIT_MS`] only when the pool was
//!   full (a change may have been dropped) or no frame is there at all (a
//!   session's first grab, one after a frame given back), keeps the newest
//!   frame it has
//!   ([`Frames`]), and reads back
//!   only the editor's box of it ([`super::wgc::read_box`]: a GPU copy into
//!   a staging texture, one map).
//! - A session Windows ended (the item's `Closed`: the monitor went) is
//!   started anew at the next grab; a start that failed is tried again no
//!   sooner than `wgc::START_AGAIN`; a frame of another size (the resolution
//!   changed) recreates the pool and fails that grab (a frame of the old
//!   size queued before is given back, never read); a lost GPU device
//!   (after a failed read or start) drops every session and the device,
//!   made anew at the next grab. The start, the device and the start's
//!   retry are `wgc.rs`' own (`start_capture`, `device_in`, `Failures`).

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::System::WinRT::Direct3D11::IDirect3DDxgiInterfaceAccess;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::{IsIconic, IsWindow};
use windows::core::{IInspectable, Interface};

use super::Pixels;
use super::capture::CaptureCounts;
use super::capture::display::{
    BUFFERS, Frame, Frames, Monitors, Outcome, PoolAct, WAIT_MS, display_box, pool_act,
};
use super::wgc::{
    Device, FORMAT, Failures, device_in, locked, lost, qpc_100ns, read_box, start_capture,
    supported, text,
};

/// What the pool's handlers tell the grabs: the frames delivered, and
/// whether Windows ended the session.
#[derive(Default)]
struct Signal {
    arrived: u64,
    ended: bool,
}

/// The handlers' side, shared with the grabs: a count, a flag, a wake.
#[derive(Default)]
struct Wake {
    signal: Mutex<Signal>,
    woken: Condvar,
}

impl Wake {
    fn arrived(&self) {
        locked(&self.signal).arrived += 1;
        self.woken.notify_all();
    }

    fn ended(&self) {
        locked(&self.signal).ended = true;
        self.woken.notify_all();
    }

    fn count(&self) -> u64 {
        locked(&self.signal).arrived
    }

    fn is_ended(&self) -> bool {
        locked(&self.signal).ended
    }

    /// Waits until a frame arrived after `seen` was counted, at most
    /// `left`.
    fn wait(&self, seen: u64, left: Duration) {
        let guard = locked(&self.signal);
        let _ = self
            .woken
            .wait_timeout_while(guard, left, |s| s.arrived == seen && !s.ended);
    }
}

/// One monitor's capture.
struct Screen {
    item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    frames_token: i64,
    closed_token: i64,
    wake: Arc<Wake>,
    /// The pool's buffers' size (a frame of another content size recreates
    /// it).
    size: SizeInt32,
    /// The frame the last grab used, held in its buffer.
    held: Option<Direct3D11CaptureFrame>,
    frames: Frames,
    /// The largest delay from a fresh frame's composition to its use (ms).
    lag_max: f64,
    staging: Option<ID3D11Texture2D>,
}

impl Screen {
    /// Ends it: the held frame closed, no handler runs after its removal,
    /// the pool's buffers go.
    fn stop(&mut self) {
        close(self.held.take());
        let _ = self.pool.RemoveFrameArrived(self.frames_token);
        let _ = self.item.RemoveClosed(self.closed_token);
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }

    fn counts(&self) -> CaptureCounts {
        CaptureCounts {
            arrived: self.wake.count(),
            lag_ms_max: self.lag_max,
            display: Some(self.frames.tally()),
        }
    }
}

/// A frame given back to its pool (none: nothing).
fn close(frame: Option<Direct3D11CaptureFrame>) {
    if let Some(frame) = frame {
        let _ = frame.Close();
    }
}

/// The newest frame queued in `pool`, every older one closed (bounded by
/// the pool's buffers: `TryGetNextFrame` hands the oldest first), and how
/// many were queued.
fn newest(pool: &Direct3D11CaptureFramePool) -> (Option<Direct3D11CaptureFrame>, u32) {
    let (mut newest, mut queued) = (None, 0);
    for _ in 0..=BUFFERS {
        match pool.TryGetNextFrame() {
            Ok(frame) => {
                close(newest.replace(frame));
                queued += 1;
            }
            Err(_) => break,
        }
    }
    (newest, queued)
}

/// A window's monitor (the nearest) and that monitor's rectangle.
fn monitor_of(window: HWND) -> Result<(HMONITOR, RECT), String> {
    // SAFETY: a plain query of a handle; the nearest monitor always answers.
    let monitor = unsafe { MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        rcMonitor: RECT::default(),
        rcWork: RECT::default(),
        dwFlags: 0,
    };
    // SAFETY: `info` outlives the call, its size set as the call needs.
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        Ok((monitor, info.rcMonitor))
    } else {
        Err("GetMonitorInfoW failed".to_string())
    }
}

/// A monitor as the maps key it.
fn key_of(monitor: HMONITOR) -> isize {
    monitor.0 as isize
}

/// The display captures: one session per monitor an editor is on, on one
/// device; a start that failed, when and why.
#[derive(Default)]
pub struct Display {
    device: Option<Device>,
    screens: HashMap<isize, Screen>,
    monitors: Monitors,
    failed: Failures<isize>,
}

impl std::fmt::Debug for Display {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Display")
            .field("screens", &self.screens.len())
            .field("monitors", &self.monitors)
            .finish()
    }
}

impl Display {
    /// A session of `monitor` started (`wgc::start_capture`, as a window's).
    fn start(&mut self, monitor: HMONITOR) -> Result<Screen, String> {
        supported()?;
        let direct3d = device_in(&mut self.device)?.direct3d.clone();
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(|e| text("IGraphicsCaptureItemInterop", e))?;
        // SAFETY: a capture item of a monitor handle; a stale one fails.
        let item: GraphicsCaptureItem = unsafe { interop.CreateForMonitor(monitor) }
            .map_err(|e| text("CreateForMonitor", e))?;
        let size = item
            .Size()
            .map_err(|e| text("GraphicsCaptureItem.Size", e))?;
        let wake = Arc::new(Wake::default());
        // Only a count and a wake: no frame is taken or copied here.
        let theirs = Arc::clone(&wake);
        let on_frame =
            TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |_, _| {
                theirs.arrived();
                Ok(())
            });
        let flagged = Arc::clone(&wake);
        let on_closed = TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
            flagged.ended();
            Ok(())
        });
        let started = start_capture(&direct3d, &item, BUFFERS, size, &on_frame, &on_closed)?;
        Ok(Screen {
            item,
            pool: started.pool,
            session: started.session,
            frames_token: started.frames,
            closed_token: started.closed,
            wake,
            size,
            held: None,
            frames: Frames::default(),
            lag_max: 0.0,
            staging: None,
        })
    }

    /// The session of `monitor`: started when it has none (a start that
    /// failed less than `wgc::START_AGAIN` ago answers its error again) or
    /// when Windows ended it. A start that failed on a lost device drops
    /// every session and the device.
    fn screen(&mut self, monitor: HMONITOR) -> Result<&mut Screen, String> {
        let key = key_of(monitor);
        if self.screens.get(&key).is_some_and(|s| s.wake.is_ended())
            && let Some(mut old) = self.screens.remove(&key)
        {
            tracing::info!("a Pro-Q 4 display capture ended by Windows: started anew");
            old.stop();
        }
        if !self.screens.contains_key(&key) {
            if let Some(why) = self.failed.recent(&key) {
                return Err(why);
            }
            match self.start(monitor) {
                Ok(screen) => {
                    self.failed.forget(&key);
                    self.screens.insert(key, screen);
                }
                Err(why) => {
                    if let Some(lost) = lost(&self.device) {
                        tracing::warn!(error = %lost, "the Pro-Q 4 display capture's Direct3D device was lost: made anew at the next grab");
                        self.drop_device();
                    }
                    self.failed.failed(key, why.clone());
                    return Err(why);
                }
            }
        }
        self.screens
            .get_mut(&key)
            .ok_or_else(|| "no capture session".to_string())
    }

    /// `window`'s monitor now: its session in use (a monitor it left ends
    /// when no other editor is on it). A handle that names no window has
    /// none.
    fn follow(&mut self, window: HWND) -> Result<(HMONITOR, RECT), String> {
        // SAFETY: a plain query of a handle.
        if !unsafe { IsWindow(Some(window)) }.as_bool() {
            return Err("no such window".to_string());
        }
        let (monitor, rect) = monitor_of(window)?;
        if let Some(left) = self.monitors.on(window.0 as usize, key_of(monitor)) {
            self.end_screen(left);
        }
        Ok((monitor, rect))
    }

    fn end_screen(&mut self, key: isize) {
        self.failed.forget(&key);
        if let Some(mut screen) = self.screens.remove(&key) {
            screen.stop();
        }
    }

    /// Whether `window`'s monitor capture runs: started now when it has
    /// none (a start that failed less than `wgc::START_AGAIN` ago answers
    /// its error).
    pub fn started(&mut self, window: HWND) -> Result<(), String> {
        let (monitor, _) = self.follow(window)?;
        self.screen(monitor).map(|_| ())
    }

    /// Starts the capture of a taken editor's monitor (its first frame
    /// comes at once); a failure is said and tried again at a grab.
    pub fn begin(&mut self, window: HWND) {
        if let Err(why) = self.started(window) {
            tracing::warn!(why = %why, "a Pro-Q 4 editor's display capture could not start: tried again at its grab");
        }
    }

    /// The counts of `window`'s monitor session (none: no session).
    pub fn counts(&self, window: HWND) -> Option<CaptureCounts> {
        let key = self.monitors.monitor(window.0 as usize)?;
        self.screens.get(&key).map(Screen::counts)
    }

    /// The picture of `window` (its picture's client area at the screen
    /// point `origin`, `size`) from its monitor's newest frame. A lost
    /// device drops every session and the device (made anew at the next
    /// grab).
    pub fn grab(
        &mut self,
        window: HWND,
        origin: (i32, i32),
        size: (u32, u32),
    ) -> Result<Pixels, String> {
        // SAFETY: a plain query of a handle.
        if unsafe { IsIconic(window) }.as_bool() {
            return Err("the editor's window is minimised: it is not on the screen".to_string());
        }
        let (monitor, rect) = self.follow(window)?;
        self.screen(monitor)?;
        let key = key_of(monitor);
        let (Some(device), Some(screen)) = (self.device.as_ref(), self.screens.get_mut(&key))
        else {
            return Err("no capture session".to_string());
        };
        let read = read(device, screen, rect, origin, size);
        if read.is_err()
            && let Some(lost) = lost(&self.device)
        {
            tracing::warn!(error = %lost, "the Pro-Q 4 display capture's Direct3D device was lost: made anew at the next grab");
            self.drop_device();
        }
        read
    }

    /// Every session ended and the device dropped (the monitors forgotten:
    /// each grab looks its window's up again).
    fn drop_device(&mut self) {
        for (_, mut screen) in self.screens.drain() {
            screen.stop();
        }
        self.monitors.clear();
        self.device = None;
    }

    /// The capture of `window` ends (its release, a lost window, the stop):
    /// its monitor's session too when no other editor is on it.
    pub fn end(&mut self, window: HWND) {
        if let Some(left) = self.monitors.off(window.0 as usize) {
            self.end_screen(left);
        }
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        self.drop_device();
    }
}

/// A grab of the editor's box from `screen`: the pool drained to its
/// newest frame, a fresh one awaited when the pool was full (at most
/// [`WAIT_MS`]), the frame [`Frames::settle`] names read back and held,
/// the grab counted only once its picture was read.
fn read(
    device: &Device,
    screen: &mut Screen,
    monitor: RECT,
    origin: (i32, i32),
    size: (u32, u32),
) -> Result<Pixels, String> {
    let (mut queued, count) = newest(&screen.pool);
    let plan = screen.frames.plan(count);
    if plan.close_held {
        close(screen.held.take());
    }
    let mut fresh = None;
    if plan.wait {
        fresh = wait_fresh(screen);
    }
    let Some(settled) = screen.frames.settle(count, fresh.is_some()) else {
        return Err("no frame of the screen yet".to_string());
    };
    match settled.close {
        Some(Frame::Queued) => close(queued.take()),
        Some(Frame::Held) => close(screen.held.take()),
        // `settle` never closes the fresh frame it uses.
        Some(Frame::Fresh) | None => {}
    }
    let used = match settled.frame {
        Frame::Fresh => fresh.take(),
        Frame::Queued => queued.take(),
        Frame::Held => screen.held.take(),
    };
    close(fresh);
    close(queued);
    let Some(used) = used else {
        screen.frames.dropped();
        return Err("the frame to read is gone".to_string());
    };
    let age = (settled.outcome == Outcome::Fresh)
        .then(|| {
            let composed = used.SystemRelativeTime().ok()?.Duration;
            qpc_100ns().map(|now| (now - composed) as f64 / 10_000.0)
        })
        .flatten();
    match picture_of(device, screen, &used, monitor, origin, size) {
        Err(Retired(why)) => {
            close(Some(used));
            screen.frames.dropped();
            Err(why)
        }
        Ok(picture) => {
            screen.held = Some(used);
            if picture.is_ok() {
                screen.frames.counted(settled.outcome);
                if let Some(age) = age {
                    screen.lag_max = screen.lag_max.max(age);
                }
            }
            picture
        }
    }
}

/// An error after which the frame used is given back, never held: it is of
/// another size than the monitor's now, or the pool was made anew.
struct Retired(String);

/// The editor's box of `frame` read back. The pool's act is
/// [`pool_act`]'s, from the monitor's live rectangle: a frame of another
/// size than the monitor (queued before a change of resolution) is given
/// back, a new size makes the pool anew (`Err(Retired)`: the next grab
/// takes a frame of the new size). Any other failure is `Ok(Err)` (the
/// frame stays held).
fn picture_of(
    device: &Device,
    screen: &mut Screen,
    frame: &Direct3D11CaptureFrame,
    monitor: RECT,
    origin: (i32, i32),
    size: (u32, u32),
) -> Result<Result<Pixels, String>, Retired> {
    let content = frame
        .ContentSize()
        .map_err(|e| Retired(text("ContentSize", e)))?;
    let side = |v: i32| u32::try_from(v).unwrap_or(0);
    let span = |from: i32, to: i32| u32::try_from(i64::from(to) - i64::from(from)).unwrap_or(0);
    let content = (side(content.Width), side(content.Height));
    let pool = (side(screen.size.Width), side(screen.size.Height));
    let now = (
        span(monitor.left, monitor.right),
        span(monitor.top, monitor.bottom),
    );
    match pool_act(pool, content, now) {
        PoolAct::Keep => {}
        PoolAct::Retire => {
            return Err(Retired(format!(
                "a frame of {}x{}, the screen {}x{}: one of its size awaited",
                content.0, content.1, now.0, now.1
            )));
        }
        PoolAct::Recreate((width, height)) => {
            let to = SizeInt32 {
                Width: i32::try_from(width).unwrap_or(i32::MAX),
                Height: i32::try_from(height).unwrap_or(i32::MAX),
            };
            screen
                .pool
                .Recreate(device.direct3d.get(), FORMAT, BUFFERS, to)
                .map_err(|e| Retired(text("Recreate", e)))?;
            screen.size = to;
            return Err(Retired(format!(
                "the screen's frames are {width}x{height} now, its pool {}x{}: made anew",
                pool.0, pool.1
            )));
        }
    }
    let within = match display_box(
        (monitor.left, monitor.top, monitor.right, monitor.bottom),
        content,
        origin,
        size,
    ) {
        Ok(within) => within,
        Err(why) => return Ok(Err(why)),
    };
    let texture = (|| -> windows::core::Result<ID3D11Texture2D> {
        let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
        // SAFETY: the frame's surface as its Direct3D texture.
        unsafe { access.GetInterface() }
    })();
    let texture = match texture {
        Ok(texture) => texture,
        Err(e) => return Ok(Err(text("the frame's surface", e))),
    };
    Ok(read_box(device, &mut screen.staging, &texture, within))
}

/// Waits for a frame newer than any drained, at most [`WAIT_MS`] (the loop
/// bounded by its milliseconds: a spurious wake looks again). The count is
/// read before each look, so a frame that arrives between the look and the
/// wait ends the wait at once.
fn wait_fresh(screen: &Screen) -> Option<Direct3D11CaptureFrame> {
    let deadline = Instant::now() + Duration::from_millis(WAIT_MS);
    for _ in 0..=WAIT_MS {
        let counted = screen.wake.count();
        if let (Some(frame), _) = newest(&screen.pool) {
            return Some(frame);
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || screen.wake.is_ended() {
            return None;
        }
        screen.wake.wait(counted, left);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_handle_that_names_no_window_starts_no_capture_and_gives_no_picture() {
        // Whatever the runner has (a GPU, WARP or neither, a desktop or
        // none), a handle that names no window is refused before any
        // monitor is looked up: no session, no monitor, no device.
        let mut display = Display::default();
        let none = HWND(std::ptr::null_mut());
        assert_eq!(
            display.grab(none, (0, 0), (10, 10)).err(),
            Some("no such window".to_string())
        );
        assert_eq!(display.started(none), Err("no such window".to_string()));
        assert!(display.counts(none).is_none());
        display.begin(none);
        display.end(none);
        assert!(display.screens.is_empty());
        assert_eq!(display.monitors, Monitors::default());
        assert!(display.failed.is_empty());
        assert!(display.device.is_none());
    }
}
