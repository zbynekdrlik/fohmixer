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
//!   full (a change may have been dropped), keeps the newest frame it has
//!   ([`Frames`]), and reads back
//!   only the editor's box of it ([`super::wgc::read_box`]: a GPU copy into
//!   a staging texture, one map).
//! - A session Windows ended (the item's `Closed`: the monitor went) is
//!   started anew at the next grab; a start that failed is tried again no
//!   sooner than [`START_AGAIN`]; a frame of another size (the resolution
//!   changed) recreates the pool and fails that grab; a lost GPU device
//!   drops every session and the device, made anew at the next grab.

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
use windows::Win32::UI::WindowsAndMessaging::IsIconic;
use windows::core::{IInspectable, Interface};

use super::Pixels;
use super::capture::CaptureCounts;
use super::capture::display::{BUFFERS, Frame, Frames, Monitors, Outcome, WAIT_MS, display_box};
use super::wgc::{Device, FORMAT, Partial, locked, qpc_100ns, read_box, text};

/// A monitor session that failed to start is tried again no sooner than
/// this (the worker asks at every grab while it has none).
pub const START_AGAIN: Duration = Duration::from_secs(1);

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
    failed: HashMap<isize, (Instant, String)>,
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
    fn device(&mut self) -> Result<&Device, String> {
        if self.device.is_none() {
            self.device = Some(Device::new()?);
        }
        self.device
            .as_ref()
            .ok_or_else(|| "no Direct3D device".to_string())
    }

    /// A session of `monitor` started.
    fn start(&mut self, monitor: HMONITOR) -> Result<Screen, String> {
        if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
            return Err("Windows Graphics Capture is not supported here".to_string());
        }
        let direct3d = self.device()?.direct3d.clone();
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(|e| text("IGraphicsCaptureItemInterop", e))?;
        // SAFETY: a capture item of a monitor handle; a stale one fails.
        let item: GraphicsCaptureItem = unsafe { interop.CreateForMonitor(monitor) }
            .map_err(|e| text("CreateForMonitor", e))?;
        let size = item
            .Size()
            .map_err(|e| text("GraphicsCaptureItem.Size", e))?;
        let pool =
            Direct3D11CaptureFramePool::CreateFreeThreaded(direct3d.get(), FORMAT, BUFFERS, size)
                .map_err(|e| text("CreateFreeThreaded", e))?;
        let mut partial = Partial {
            item: item.clone(),
            pool: pool.clone(),
            frames: None,
            closed: None,
        };
        let wake = Arc::new(Wake::default());
        // Only a count and a wake: no frame is taken or copied here.
        let theirs = Arc::clone(&wake);
        let on_frame =
            TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |_, _| {
                theirs.arrived();
                Ok(())
            });
        match pool.FrameArrived(&on_frame) {
            Ok(token) => partial.frames = Some(token),
            Err(e) => return Err(partial.fail(text("FrameArrived", e))),
        }
        let flagged = Arc::clone(&wake);
        let on_closed = TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
            flagged.ended();
            Ok(())
        });
        match item.Closed(&on_closed) {
            Ok(token) => partial.closed = Some(token),
            Err(e) => return Err(partial.fail(text("Closed", e))),
        }
        let session = match pool.CreateCaptureSession(&item) {
            Ok(session) => session,
            Err(e) => return Err(partial.fail(text("CreateCaptureSession", e))),
        };
        // The cursor never in the picture; no border where Windows lets it
        // go (from 20348: the PC's 19044 frames the whole monitor).
        let _ = session.SetIsCursorCaptureEnabled(false);
        let _ = session.SetIsBorderRequired(false);
        if let Err(e) = session.StartCapture() {
            let _ = session.Close();
            return Err(partial.fail(text("StartCapture", e)));
        }
        Ok(Screen {
            item,
            pool,
            session,
            frames_token: partial.frames.unwrap_or_default(),
            closed_token: partial.closed.unwrap_or_default(),
            wake,
            size,
            held: None,
            frames: Frames::default(),
            lag_max: 0.0,
            staging: None,
        })
    }

    /// The session of `monitor`: started when it has none (a start that
    /// failed less than [`START_AGAIN`] ago answers its error again) or
    /// when Windows ended it.
    fn screen(&mut self, monitor: HMONITOR) -> Result<&mut Screen, String> {
        let key = key_of(monitor);
        if self.screens.get(&key).is_some_and(|s| s.wake.is_ended())
            && let Some(mut old) = self.screens.remove(&key)
        {
            tracing::info!("a Pro-Q 4 display capture ended by Windows: started anew");
            old.stop();
        }
        if !self.screens.contains_key(&key) {
            if let Some((at, why)) = self.failed.get(&key)
                && at.elapsed() < START_AGAIN
            {
                return Err(why.clone());
            }
            match self.start(monitor) {
                Ok(screen) => {
                    self.failed.remove(&key);
                    self.screens.insert(key, screen);
                }
                Err(why) => {
                    self.failed.insert(key, (Instant::now(), why.clone()));
                    return Err(why);
                }
            }
        }
        self.screens
            .get_mut(&key)
            .ok_or_else(|| "no capture session".to_string())
    }

    /// `window`'s monitor now: its session in use (a monitor it left ends
    /// when no other editor is on it).
    fn follow(&mut self, window: HWND) -> Result<(HMONITOR, RECT), String> {
        let (monitor, rect) = monitor_of(window)?;
        if let Some(left) = self.monitors.on(window.0 as usize, key_of(monitor)) {
            self.end_screen(left);
        }
        Ok((monitor, rect))
    }

    fn end_screen(&mut self, key: isize) {
        self.failed.remove(&key);
        if let Some(mut screen) = self.screens.remove(&key) {
            screen.stop();
        }
    }

    /// Whether `window`'s monitor capture runs: started now when it has
    /// none (a start that failed less than [`START_AGAIN`] ago answers its
    /// error).
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
        // SAFETY: a plain query of the device's state.
        if read.is_err()
            && let Err(e) = unsafe { device.device.GetDeviceRemovedReason() }
        {
            tracing::warn!(error = %e, "the Pro-Q 4 display capture's Direct3D device was lost: made anew at the next grab");
            self.reset();
        }
        read
    }

    /// Every session ended and the device dropped (the monitors forgotten:
    /// each grab looks its window's up again).
    fn reset(&mut self) {
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
        self.reset();
    }
}

/// A grab of the editor's box from `screen`: the pool drained to its
/// newest frame, a fresh one awaited when a change was queued (at most
/// [`WAIT_MS`]), the frame [`Frames::settle`] names read back and held.
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
        Some(Frame::Fresh) => close(fresh.take()),
        None => {}
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
    if settled.outcome == Outcome::Fresh
        && let (Ok(time), Some(now)) = (used.SystemRelativeTime(), qpc_100ns())
    {
        screen.lag_max = screen.lag_max.max((now - time.Duration) as f64 / 10_000.0);
    }
    match picture_of(device, screen, &used, monitor, origin, size) {
        Err(Recreated(why)) => {
            close(Some(used));
            screen.frames.dropped();
            Err(why)
        }
        Ok(picture) => {
            screen.held = Some(used);
            picture
        }
    }
}

/// An error after which the frame used is given back (the pool was
/// recreated for another size).
struct Recreated(String);

/// The editor's box of `frame` read back; a frame of another size than
/// the pool's recreates the pool (`Err(Recreated)`), any other failure is
/// `Ok(Err)` (the frame stays held).
fn picture_of(
    device: &Device,
    screen: &mut Screen,
    frame: &Direct3D11CaptureFrame,
    monitor: RECT,
    origin: (i32, i32),
    size: (u32, u32),
) -> Result<Result<Pixels, String>, Recreated> {
    let content = frame
        .ContentSize()
        .map_err(|e| Recreated(text("ContentSize", e)))?;
    if (content.Width, content.Height) != (screen.size.Width, screen.size.Height)
        && content.Width > 0
        && content.Height > 0
    {
        let why = format!(
            "the screen's frames are {}x{} now, its pool {}x{}: made anew",
            content.Width, content.Height, screen.size.Width, screen.size.Height
        );
        screen
            .pool
            .Recreate(device.direct3d.get(), FORMAT, BUFFERS, content)
            .map_err(|e| Recreated(text("Recreate", e)))?;
        screen.size = content;
        return Err(Recreated(why));
    }
    let side = |v: i32| u32::try_from(v).unwrap_or(0);
    let within = match display_box(
        (monitor.left, monitor.top, monitor.right, monitor.bottom),
        (side(content.Width), side(content.Height)),
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
    fn a_window_that_is_none_gives_no_picture_and_no_session() {
        // Whatever the runner has (a GPU, WARP or neither), a handle that
        // names no window gives no picture and keeps no window of it.
        let mut display = Display::default();
        let none = HWND(std::ptr::null_mut());
        assert!(display.counts(none).is_none());
        let _ = display.grab(none, (0, 0), (10, 10));
        display.end(none);
        assert_eq!(display.monitors.monitor(0), None);
        assert_eq!(START_AGAIN, Duration::from_secs(1));
    }
}
