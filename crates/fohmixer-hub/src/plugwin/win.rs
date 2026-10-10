//! The Windows window backend (#71 PR E; what it rests on:
//! `.claude/rules/plugin-window.md`), Win32 through the `windows` crate.
//! Compiled on Windows only, so it is out of the mutation gate (as
//! `dpapi.rs`): the `windows` CI job lints, builds and tests it, and
//! `fohmixer-hub eq-probe` drives it on the PC against Carla's bridge
//! before any Live (#71, D17).
//!
//! - Its calls run on the window worker's thread, made per-monitor aware
//!   ([`Backend::start`]): every coordinate is a physical pixel.
//! - **Editors:** Live's top-level `Vst3PlugWindow`s ([`EDITOR_CLASS`]).
//! - **Take:** `SetWindowPos(HWND_TOPMOST)` without a move or a size,
//!   posted (`SWP_ASYNCWINDOWPOS`: the window is Live's, and a z-order
//!   change of another thread's window would otherwise wait for Live's busy
//!   UI thread while a resting contact needs its keep-alive); whether it
//!   landed is the window's `WS_EX_TOPMOST` style (`GetWindowLongW`, a read
//!   that sends no message; [`Backend::on_top`]), which the worker waits for
//!   before the take answers. The picture is Pro-Q's own child
//!   `FF_UIWindow` ([`PICTURE_CLASS`]). The
//!   hub's take ([`Win::for_hub`]) refuses a window without it (no Pro-Q
//!   editor: Live's other instance's window or a plug-in opened by hand at
//!   that moment), before it touches the window; the probe's
//!   ([`Win::new`]) takes the window's client area then.
//! - **Grab:** a `BitBlt` of the picture's screen rectangle from the
//!   screen's DC (the window is on top, so the screen holds it; nothing runs
//!   on the window's own thread, which is Live's).
//! - **Touch:** `InjectTouchInput` (one contact, `TOUCH_FEEDBACK_NONE`; never
//!   `POINTER_FLAG_PRIMARY`, which fails every call with 87): a down
//!   `INRANGE|INCONTACT|DOWN`, an update `INRANGE|INCONTACT|UPDATE`, an up
//!   `UP`, a cancel `UP|CANCELED`. A down or an update first checks the
//!   point's root window (`WindowFromPoint` → `GetAncestor(GA_ROOT)`) is the
//!   editor and its process the editor's. An up or a cancel goes at the last
//!   point injected, without the window's mapping or that check: a window
//!   gone or moved still ends the contact where it was. The cursor saved at
//!   the down is put back (`SetCursorPos`) when the contact ends (the window
//!   gone included) and when a down fails with no contact before it
//!   ([`restores`]); a failed update or keep-alive keeps it saved, as the
//!   worker's cancel at the last point follows and moves the cursor there
//!   again. A call too soon after the last one (`ERROR_NOT_READY`) is tried
//!   again after 1 ms, twice.
//! - **Release:** the z-order as it was (`HWND_NOTOPMOST`, or topmost again
//!   when it was), posted as the take's.
//! - **Size (PR G):** the take keeps the window's rectangle
//!   (`GetWindowRect`): what it has beyond the picture is added to a picture
//!   size asked for. A resize is `SetWindowPos` of a rectangle, its place
//!   and its size (#74 review: the window moves into the work area when it
//!   does not fit where it stands, `plugwin::inside`), with
//!   `SWP_ASYNCWINDOWPOS | SWP_NOZORDER | SWP_NOACTIVATE` (posted to Live's
//!   thread as the z-order changes are); whether it landed is the picture's
//!   client size (`GetClientRect`, a read that sends no message), and the
//!   window's rectangle now is `GetWindowRect` (no message either). The
//!   work area is the window's monitor's (`MonitorFromWindow`,
//!   `GetMonitorInfoW` `rcWork`: the screen less the taskbar).

use std::time::Duration;

use windows::Win32::Foundation::{ERROR_NOT_READY, HANDLE, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, ClientToScreen, CreateCompatibleBitmap,
    CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, GetMonitorInfoW,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, RGBQUAD, ReleaseDC, SRCCOPY,
    SelectObject,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
};
use windows::Win32::UI::Input::Pointer::{
    InitializeTouchInjection, InjectTouchInput, POINTER_CHANGE_NONE, POINTER_FLAG_CANCELED,
    POINTER_FLAG_DOWN, POINTER_FLAG_INCONTACT, POINTER_FLAG_INRANGE, POINTER_FLAG_UP,
    POINTER_FLAG_UPDATE, POINTER_FLAGS, POINTER_INFO, POINTER_TOUCH_INFO, TOUCH_FEEDBACK_NONE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GA_ROOT, GWL_EXSTYLE, GetAncestor, GetClassNameW, GetClientRect,
    GetCursorPos, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, HWND_NOTOPMOST,
    HWND_TOPMOST, IsWindow, IsWindowVisible, PT_TOUCH, PostMessageW, SWP_ASYNCWINDOWPOS,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetCursorPos, SetWindowPos,
    TOUCH_FLAG_NONE, TOUCH_MASK_CONTACTAREA, WM_CLOSE, WS_EX_TOPMOST, WindowFromPoint,
};
use windows::core::{BOOL, HRESULT};

use super::{Backend, Phase, Pixels, Rect, Taken, WindowId};

/// The class of Live's top-level plug-in editor windows.
pub const EDITOR_CLASS: &str = "Vst3PlugWindow";
/// The class of Pro-Q's own child window: the picture.
pub const PICTURE_CLASS: &str = "FF_UIWindow";
/// Why the hub's take refuses a window (a reason its page reads).
pub const NO_PICTURE: &str = fohmixer_proto::eq::reason::NO_PICTURE;
/// The half-size of an injected contact's area (px).
const CONTACT_RADIUS: i32 = 2;

fn hwnd(id: WindowId) -> HWND {
    HWND(id.0 as usize as *mut core::ffi::c_void)
}

fn id_of(window: HWND) -> WindowId {
    WindowId(window.0 as usize as u64)
}

unsafe extern "system" fn collect(window: HWND, data: LPARAM) -> BOOL {
    // SAFETY: `data` is the `Vec<HWND>` the enumerating call lent for its
    // duration (`top_windows`, `children`).
    let list = unsafe { &mut *(data.0 as *mut Vec<HWND>) };
    list.push(window);
    BOOL(1)
}

/// Every top-level window.
fn top_windows() -> Result<Vec<HWND>, String> {
    let mut list: Vec<HWND> = Vec::new();
    // SAFETY: `collect` writes only into `list`, which outlives the call.
    unsafe { EnumWindows(Some(collect), LPARAM(&mut list as *mut Vec<HWND> as isize)) }
        .map_err(|e| format!("EnumWindows: {e}"))?;
    Ok(list)
}

/// Every child window of `parent`, at any depth.
fn children(parent: HWND) -> Vec<HWND> {
    let mut list: Vec<HWND> = Vec::new();
    // SAFETY: as in `top_windows`. Its BOOL says nothing useful.
    let _ = unsafe {
        EnumChildWindows(
            Some(parent),
            Some(collect),
            LPARAM(&mut list as *mut Vec<HWND> as isize),
        )
    };
    list
}

fn class_of(window: HWND) -> String {
    let mut name = [0_u16; 256];
    // SAFETY: the buffer is ours; the call writes at most its length.
    let length = unsafe { GetClassNameW(window, &mut name) };
    String::from_utf16_lossy(&name[..usize::try_from(length).unwrap_or(0)])
}

fn pid_of(window: HWND) -> u32 {
    let mut pid = 0_u32;
    // SAFETY: `pid` outlives the call.
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    pid
}

fn exists(window: HWND) -> bool {
    // SAFETY: a plain query of a handle (any value is allowed).
    unsafe { IsWindow(Some(window)) }.as_bool()
}

fn visible(window: HWND) -> bool {
    // SAFETY: as `exists`.
    unsafe { IsWindowVisible(window) }.as_bool()
}

/// Whether `window` is on top of every window (`WS_EX_TOPMOST`): a read of
/// its style, no message to its thread.
fn topmost(window: HWND) -> bool {
    // SAFETY: a plain query of a handle (a stale one reads 0).
    let style = unsafe { GetWindowLongW(window, GWL_EXSTYLE) } as u32;
    (style & WS_EX_TOPMOST.0) != 0
}

/// A window's client area's size.
fn client_size(window: HWND) -> Result<(i32, i32), String> {
    let mut rect = RECT::default();
    // SAFETY: `rect` outlives the call.
    unsafe { GetClientRect(window, &mut rect) }.map_err(|e| format!("GetClientRect: {e}"))?;
    Ok((rect.right - rect.left, rect.bottom - rect.top))
}

/// A Win32 rectangle as the backend's.
fn rect_of(rect: RECT) -> Rect {
    Rect {
        left: rect.left,
        top: rect.top,
        width: rect.right - rect.left,
        height: rect.bottom - rect.top,
    }
}

/// A window's rectangle on the screen (its frame included).
fn window_rect(window: HWND) -> Result<Rect, String> {
    let mut rect = RECT::default();
    // SAFETY: `rect` outlives the call.
    unsafe { GetWindowRect(window, &mut rect) }.map_err(|e| format!("GetWindowRect: {e}"))?;
    Ok(rect_of(rect))
}

/// The work area of the monitor `window` is on (the screen less the
/// taskbar).
fn work_area(window: HWND) -> Result<Rect, String> {
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
        Ok(rect_of(info.rcWork))
    } else {
        Err("GetMonitorInfoW failed".to_string())
    }
}

/// A point of a window's client area on the screen.
fn to_screen(window: HWND, at: (i32, i32)) -> Result<POINT, String> {
    let mut point = POINT { x: at.0, y: at.1 };
    // SAFETY: `point` outlives the call.
    if unsafe { ClientToScreen(window, &mut point) }.as_bool() {
        Ok(point)
    } else {
        Err("ClientToScreen failed".to_string())
    }
}

/// Puts `window` on top of every window (`top`) or back among the others,
/// with no move, no size and no activation. The change is posted to the
/// window's thread (Live's UI thread): the worker never waits for it.
fn place(window: HWND, top: bool) -> Result<(), String> {
    let after = if top { HWND_TOPMOST } else { HWND_NOTOPMOST };
    // SAFETY: a z-order change of a handle; a stale one fails the call.
    unsafe {
        SetWindowPos(
            window,
            Some(after),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        )
    }
    .map_err(|e| format!("SetWindowPos: {e}"))
}

/// The flags of a contact phase (never `POINTER_FLAG_PRIMARY`: the system
/// sets it, and asking for it fails every call).
fn flags(phase: Phase) -> POINTER_FLAGS {
    match phase {
        Phase::Down => POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT | POINTER_FLAG_DOWN,
        Phase::Update => POINTER_FLAG_INRANGE | POINTER_FLAG_INCONTACT | POINTER_FLAG_UPDATE,
        Phase::Up => POINTER_FLAG_UP,
        Phase::Cancel => POINTER_FLAG_UP | POINTER_FLAG_CANCELED,
    }
}

/// One touch contact at a screen point (every field spelled out: the
/// system fills in the time, the frame and the raw locations).
fn contact(phase: Phase, point: POINT) -> POINTER_TOUCH_INFO {
    POINTER_TOUCH_INFO {
        pointerInfo: POINTER_INFO {
            pointerType: PT_TOUCH,
            pointerId: 0,
            frameId: 0,
            pointerFlags: flags(phase),
            sourceDevice: HANDLE::default(),
            hwndTarget: HWND::default(),
            ptPixelLocation: point,
            ptHimetricLocation: POINT::default(),
            ptPixelLocationRaw: POINT::default(),
            ptHimetricLocationRaw: POINT::default(),
            dwTime: 0,
            historyCount: 0,
            InputData: 0,
            dwKeyStates: 0,
            PerformanceCount: 0,
            ButtonChangeType: POINTER_CHANGE_NONE,
        },
        touchFlags: TOUCH_FLAG_NONE,
        touchMask: TOUCH_MASK_CONTACTAREA,
        rcContact: RECT {
            left: point.x - CONTACT_RADIUS,
            top: point.y - CONTACT_RADIUS,
            right: point.x + CONTACT_RADIUS,
            bottom: point.y + CONTACT_RADIUS,
        },
        rcContactRaw: RECT::default(),
        orientation: 0,
        pressure: 0,
    }
}

/// Where an up or a cancel goes: the last point injected (the window may
/// be gone or moved; the contact still ends where it was). None for a down
/// or an update (its own point), or when no point was injected.
fn end_point(phase: Phase, last: Option<POINT>) -> Option<POINT> {
    if phase.ends() { last } else { None }
}

/// The last point injected after `phase` at `point` (`went`: it was
/// injected): a down's or an update's point; none once the contact ended.
fn last_after(phase: Phase, went: bool, point: POINT, last: Option<POINT>) -> Option<POINT> {
    if phase.ends() {
        None
    } else if went {
        Some(point)
    } else {
        last
    }
}

/// Whether the cursor saved at the down goes back after `phase` (`failed`:
/// it was not injected; `down`: a contact was down before it): when the
/// contact ends, and when a down fails with no contact before it. A failed
/// phase of a contact still down keeps it saved: the worker follows with a
/// cancel at the contact's last point, which moves the cursor there again,
/// and that end puts it back.
fn restores(phase: Phase, failed: bool, down: bool) -> bool {
    phase.ends() || (failed && !down)
}

/// Pro-Q's own child window of `window` (its picture), if it is there yet.
fn picture_child(window: HWND) -> Option<HWND> {
    children(window)
        .into_iter()
        .find(|c| class_of(*c) == PICTURE_CLASS)
}

/// The window whose client area is the picture: Pro-Q's own child
/// (`child`), else the window itself, unless the take needs the child.
fn picture_of(child: Option<HWND>, window: HWND, needs_child: bool) -> Result<HWND, String> {
    match child {
        Some(child) => Ok(child),
        None if needs_child => Err(NO_PICTURE.to_string()),
        None => Ok(window),
    }
}

/// Injects one contact; a call too soon after the last one is tried again.
fn inject(info: &POINTER_TOUCH_INFO) -> Result<(), String> {
    let not_ready = HRESULT::from_win32(ERROR_NOT_READY.0);
    let mut last = String::new();
    for _ in 0..3 {
        // SAFETY: one initialised contact, read by the call only.
        match unsafe { InjectTouchInput(std::slice::from_ref(info)) } {
            Ok(()) => return Ok(()),
            Err(e) if e.code() == not_ready => {
                last = e.to_string();
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(e) => return Err(format!("InjectTouchInput: {e}")),
        }
    }
    Err(format!("InjectTouchInput: {last}"))
}

/// The Windows backend: the cursor saved at a contact's down, the last
/// point injected while a contact is down, and whether a take needs Pro-Q's
/// own child window.
#[derive(Debug, Default)]
pub struct Win {
    cursor: Option<POINT>,
    last: Option<POINT>,
    needs_child: bool,
}

/// The `eq-probe` backend: it takes any window of the process it is given.
pub fn probe_backend() -> Result<Box<dyn Backend>, String> {
    Ok(Box::new(Win::new()))
}

/// The hub's backend (`[eq] backend = "windows"`): it takes only a window
/// with Pro-Q's own child.
pub fn hub_backend() -> anyhow::Result<Box<dyn Backend>> {
    Ok(Box::new(Win::for_hub()))
}

impl Win {
    /// The probe's: a take of a window without Pro-Q's child shows its
    /// client area.
    pub fn new() -> Self {
        Self::default()
    }

    /// The hub's: a take refuses a window without Pro-Q's child.
    pub fn for_hub() -> Self {
        Self {
            cursor: None,
            last: None,
            needs_child: true,
        }
    }

    /// One phase at picture point `at`: a down or an update only on the
    /// editor's own window; an up or a cancel at the last point injected; a
    /// down saves the cursor first.
    fn inject_at(&mut self, taken: &Taken, phase: Phase, at: (i32, i32)) -> Result<(), String> {
        let point = match end_point(phase, self.last) {
            Some(last) => last,
            None => to_screen(hwnd(taken.picture), at)?,
        };
        if phase.checked() {
            // SAFETY: plain queries of the window at a point.
            let root = unsafe { GetAncestor(WindowFromPoint(point), GA_ROOT) };
            if root != hwnd(taken.window) || pid_of(root) != taken.pid {
                return Err("the point is not the editor's window".to_string());
            }
        }
        if phase == Phase::Down {
            let mut saved = POINT::default();
            // SAFETY: `saved` outlives the call.
            if unsafe { GetCursorPos(&mut saved) }.is_ok() {
                self.cursor = Some(saved);
            }
        }
        let injected = inject(&contact(phase, point));
        self.last = last_after(phase, injected.is_ok(), point, self.last);
        injected
    }
}

impl Backend for Win {
    fn start(&mut self) -> Result<(), String> {
        // SAFETY: changes this thread's DPI awareness only.
        unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        // SAFETY: sets up touch injection for this process.
        unsafe { InitializeTouchInjection(1, TOUCH_FEEDBACK_NONE) }
            .map_err(|e| format!("InitializeTouchInjection: {e}"))
    }

    fn editors(&mut self) -> Result<Vec<WindowId>, String> {
        Ok(top_windows()?
            .into_iter()
            .filter(|w| visible(*w) && class_of(*w) == EDITOR_CLASS)
            .map(id_of)
            .collect())
    }

    fn windows_of(&mut self, pid: u32) -> Result<Vec<WindowId>, String> {
        Ok(top_windows()?
            .into_iter()
            .filter(|w| visible(*w) && pid_of(*w) == pid)
            .map(id_of)
            .collect())
    }

    fn ready(&mut self, window: WindowId) -> bool {
        // Live shows the editor's window before Pro-Q attaches its view:
        // the hub's take waits for it (the worker polls on).
        !self.needs_child || picture_child(hwnd(window)).is_some()
    }

    fn take(&mut self, window: WindowId) -> Result<Taken, String> {
        let handle = hwnd(window);
        if !exists(handle) {
            return Err("no such window".to_string());
        }
        // The picture first: a refused window is left as it was.
        let picture = picture_of(picture_child(handle), handle, self.needs_child)?;
        let was_topmost = topmost(handle);
        let rect = window_rect(handle)?;
        place(handle, true)?;
        let (width, height) = client_size(picture)?;
        Ok(Taken {
            window,
            picture: id_of(picture),
            pid: pid_of(handle),
            was_topmost,
            width: u32::try_from(width).unwrap_or(0),
            height: u32::try_from(height).unwrap_or(0),
            rect,
        })
    }

    fn on_top(&mut self, taken: &Taken) -> bool {
        topmost(hwnd(taken.window))
    }

    fn alive(&mut self, taken: &Taken) -> bool {
        let (window, picture) = (hwnd(taken.window), hwnd(taken.picture));
        exists(window) && exists(picture) && visible(window)
    }

    fn work(&mut self, taken: &Taken) -> Result<Rect, String> {
        let window = hwnd(taken.window);
        // The nearest monitor answers for any handle: a window gone has none.
        if !exists(window) {
            return Err("no such window".to_string());
        }
        work_area(window)
    }

    fn rect(&mut self, taken: &Taken) -> Result<Rect, String> {
        window_rect(hwnd(taken.window))
    }

    fn resize(&mut self, taken: &Taken, rect: Rect) -> Result<(), String> {
        // SAFETY: a place and size change of a handle, posted; a stale one
        // fails.
        unsafe {
            SetWindowPos(
                hwnd(taken.window),
                None,
                rect.left,
                rect.top,
                rect.width,
                rect.height,
                SWP_ASYNCWINDOWPOS | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        }
        .map_err(|e| format!("SetWindowPos: {e}"))
    }

    fn client(&mut self, taken: &Taken) -> Result<(u32, u32), String> {
        let (width, height) = client_size(hwnd(taken.picture))?;
        Ok((
            u32::try_from(width).unwrap_or(0),
            u32::try_from(height).unwrap_or(0),
        ))
    }

    fn grab(&mut self, taken: &Taken) -> Result<Pixels, String> {
        let picture = hwnd(taken.picture);
        let (width, height) = client_size(picture)?;
        if width <= 0 || height <= 0 {
            return Err("the picture is empty".to_string());
        }
        let origin = to_screen(picture, (0, 0))?;
        let mut bgra = vec![0_u8; width as usize * height as usize * 4];
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // Negative: rows top-down.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                biSizeImage: 0,
                biXPelsPerMeter: 0,
                biYPelsPerMeter: 0,
                biClrUsed: 0,
                biClrImportant: 0,
            },
            bmiColors: [RGBQUAD::default()],
        };
        // SAFETY: every handle made here is freed below on every path; the
        // pixel buffer holds `width * height` 32-bit pixels, what `info`
        // asks GetDIBits for.
        let (blit, lines) = unsafe {
            let screen = GetDC(None);
            if screen.is_invalid() {
                return Err("GetDC failed".to_string());
            }
            let memory = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            let old = SelectObject(memory, bitmap.into());
            let blit = BitBlt(
                memory,
                0,
                0,
                width,
                height,
                Some(screen),
                origin.x,
                origin.y,
                SRCCOPY,
            );
            let _ = SelectObject(memory, old);
            let lines = GetDIBits(
                memory,
                bitmap,
                0,
                height as u32,
                Some(bgra.as_mut_ptr().cast()),
                &mut info,
                DIB_RGB_COLORS,
            );
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory);
            let _ = ReleaseDC(None, screen);
            (blit, lines)
        };
        blit.map_err(|e| format!("BitBlt: {e}"))?;
        if lines != height {
            return Err(format!("GetDIBits copied {lines} of {height} rows"));
        }
        Ok(Pixels {
            width: width as u32,
            height: height as u32,
            bgra,
        })
    }

    fn touch(&mut self, taken: &Taken, phase: Phase, at: (i32, i32)) -> Result<(), String> {
        let down = self.last.is_some();
        let injected = self.inject_at(taken, phase, at);
        // An end puts the cursor back even when it failed (the window gone);
        // a failed down with no contact before it too. A contact still down
        // keeps it saved for its end (the worker's cancel follows).
        if restores(phase, injected.is_err(), down)
            && let Some(saved) = self.cursor.take()
        {
            // SAFETY: moves the cursor back where it was.
            let _ = unsafe { SetCursorPos(saved.x, saved.y) };
        }
        injected
    }

    fn release(&mut self, taken: &Taken) {
        let window = hwnd(taken.window);
        if exists(window)
            && let Err(why) = place(window, taken.was_topmost)
        {
            tracing::warn!(why = %why, "a plug-in editor's z-order could not be put back");
        }
    }

    fn close_window(&mut self, taken: &Taken) {
        // SAFETY: posts a close request to a handle; a stale one fails.
        let _ = unsafe { PostMessageW(Some(hwnd(taken.window)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flags_never_ask_for_primary() {
        use windows::Win32::UI::Input::Pointer::POINTER_FLAG_PRIMARY;
        for phase in [Phase::Down, Phase::Update, Phase::Up, Phase::Cancel] {
            assert!(!flags(phase).contains(POINTER_FLAG_PRIMARY), "{phase:?}");
        }
        assert_eq!(flags(Phase::Down).0, 0x2 | 0x4 | 0x10000);
        assert_eq!(flags(Phase::Update).0, 0x2 | 0x4 | 0x20000);
        assert_eq!(flags(Phase::Up).0, 0x40000);
        assert_eq!(flags(Phase::Cancel).0, 0x40000 | 0x8000);
        assert_eq!(std::mem::size_of::<POINTER_TOUCH_INFO>(), 144);
        assert_eq!(std::mem::size_of::<POINTER_INFO>(), 96);
        let one = contact(Phase::Down, POINT { x: 100, y: 50 });
        assert_eq!(one.pointerInfo.ptPixelLocation, POINT { x: 100, y: 50 });
        assert_eq!(
            (one.rcContact.left, one.rcContact.right),
            (98, 102),
            "a 4 px contact"
        );
    }

    #[test]
    fn the_screen_lists_and_a_window_id_round_trips() {
        // A CI runner may show no plug-in editor; the calls work all the
        // same, and no window of process 0 is listed.
        let mut backend = Win::new();
        assert!(backend.editors().is_ok());
        assert_eq!(backend.windows_of(0).unwrap(), Vec::new());
        let window = WindowId(0x1234);
        assert_eq!(id_of(hwnd(window)), window);
        assert!(!exists(hwnd(WindowId(0))));
        assert!(backend.take(WindowId(0)).is_err());
        assert!(Win::for_hub().take(WindowId(0)).is_err());
        // The hub's take waits for Pro-Q's picture; the probe's takes any.
        assert!(!Win::for_hub().ready(WindowId(0)));
        assert!(backend.ready(WindowId(0)));
        // No window is on top.
        let none = Taken {
            window: WindowId(0),
            picture: WindowId(0),
            pid: 0,
            was_topmost: false,
            width: 0,
            height: 0,
            rect: Rect::default(),
        };
        assert!(!backend.on_top(&none));
        assert!(!topmost(hwnd(WindowId(0))));
        // No window has no size, work area, place or resize (PR G).
        assert!(backend.client(&none).is_err());
        assert!(backend.work(&none).is_err());
        assert!(backend.rect(&none).is_err());
        let rect = Rect {
            left: 0,
            top: 0,
            width: 800,
            height: 600,
        };
        assert!(backend.resize(&none, rect).is_err());
        assert!(window_rect(hwnd(WindowId(0))).is_err());
        assert_eq!(
            rect_of(RECT {
                left: 10,
                top: 20,
                right: 1375,
                bottom: 868
            }),
            Rect {
                left: 10,
                top: 20,
                width: 1365,
                height: 848
            }
        );
    }

    #[test]
    fn an_end_goes_at_the_last_point_injected() {
        let (a, b) = (POINT { x: 1, y: 2 }, POINT { x: 3, y: 4 });
        assert_eq!(end_point(Phase::Up, Some(a)), Some(a));
        assert_eq!(end_point(Phase::Cancel, Some(a)), Some(a));
        assert_eq!(end_point(Phase::Up, None), None);
        assert_eq!(end_point(Phase::Down, Some(a)), None);
        assert_eq!(end_point(Phase::Update, Some(a)), None);
        assert_eq!(last_after(Phase::Down, true, b, None), Some(b));
        assert_eq!(last_after(Phase::Update, true, b, Some(a)), Some(b));
        assert_eq!(
            last_after(Phase::Update, false, b, Some(a)),
            Some(a),
            "a refused update leaves the contact where it was"
        );
        assert_eq!(last_after(Phase::Up, true, b, Some(a)), None);
        assert_eq!(last_after(Phase::Cancel, false, b, Some(a)), None);
    }

    #[test]
    fn the_cursor_goes_back_when_the_contact_ends_or_never_began() {
        for phase in [Phase::Up, Phase::Cancel] {
            for (failed, down) in [(false, true), (true, true), (false, false), (true, false)] {
                assert!(restores(phase, failed, down), "{phase:?} {failed} {down}");
            }
        }
        assert!(restores(Phase::Down, true, false), "a down that failed");
        assert!(!restores(Phase::Down, false, false), "a down that went");
        assert!(!restores(Phase::Update, false, true), "an update that went");
        // A failed phase of a contact still down: the worker's cancel at
        // its last point follows, and that end puts the cursor back.
        assert!(!restores(Phase::Update, true, true));
        assert!(!restores(Phase::Down, true, true));
        assert!(restores(Phase::Update, true, false), "no contact down");
    }

    #[test]
    fn the_hubs_take_needs_pro_qs_own_picture_the_probes_does_not() {
        let (child, window) = (hwnd(WindowId(2)), hwnd(WindowId(1)));
        assert_eq!(picture_of(Some(child), window, true), Ok(child));
        assert_eq!(picture_of(Some(child), window, false), Ok(child));
        assert_eq!(picture_of(None, window, false), Ok(window));
        assert_eq!(picture_of(None, window, true), Err(NO_PICTURE.to_string()));
        assert!(Win::for_hub().needs_child);
        assert!(!Win::new().needs_child);
    }
}
