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
//! - **Take:** `SetWindowPos(HWND_TOPMOST)` without a move or a size; the
//!   picture is Pro-Q's own child `FF_UIWindow` ([`PICTURE_CLASS`]), else
//!   the window's client area.
//! - **Grab:** a `BitBlt` of the picture's screen rectangle from the
//!   screen's DC (the window is on top, so the screen holds it; nothing runs
//!   on the window's own thread, which is Live's).
//! - **Touch:** `InjectTouchInput` (one contact, `TOUCH_FEEDBACK_NONE`; never
//!   `POINTER_FLAG_PRIMARY`, which fails every call with 87): a down
//!   `INRANGE|INCONTACT|DOWN`, an update `INRANGE|INCONTACT|UPDATE`, an up
//!   `UP`, a cancel `UP|CANCELED`. A down or an update first checks the
//!   point's root window (`WindowFromPoint` → `GetAncestor(GA_ROOT)`) is the
//!   editor and its process the editor's. The cursor saved at the down is
//!   put back (`SetCursorPos`) when the contact ends. A call too soon after
//!   the last one (`ERROR_NOT_READY`) is tried again after 1 ms, twice.
//! - **Release:** the z-order as it was (`HWND_NOTOPMOST`, or topmost again
//!   when it was).

use std::time::Duration;

use windows::Win32::Foundation::{ERROR_NOT_READY, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, ClientToScreen, CreateCompatibleBitmap,
    CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, RGBQUAD,
    ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext,
};
use windows::Win32::UI::Input::Pointer::{
    InitializeTouchInjection, InjectTouchInput, POINTER_FLAG_CANCELED, POINTER_FLAG_DOWN,
    POINTER_FLAG_INCONTACT, POINTER_FLAG_INRANGE, POINTER_FLAG_UP, POINTER_FLAG_UPDATE,
    POINTER_FLAGS, POINTER_INFO, POINTER_TOUCH_INFO, TOUCH_FEEDBACK_NONE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GA_ROOT, GWL_EXSTYLE, GetAncestor, GetClassNameW, GetClientRect,
    GetCursorPos, GetWindowLongW, GetWindowThreadProcessId, HWND_NOTOPMOST, HWND_TOPMOST, IsWindow,
    IsWindowVisible, PT_TOUCH, PostMessageW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetCursorPos,
    SetWindowPos, TOUCH_FLAG_NONE, TOUCH_MASK_CONTACTAREA, WM_CLOSE, WS_EX_TOPMOST,
    WindowFromPoint,
};
use windows::core::{BOOL, HRESULT};

use super::{Backend, Phase, Pixels, Taken, WindowId};

/// The class of Live's top-level plug-in editor windows.
pub const EDITOR_CLASS: &str = "Vst3PlugWindow";
/// The class of Pro-Q's own child window: the picture.
pub const PICTURE_CLASS: &str = "FF_UIWindow";
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

/// A window's client area's size.
fn client_size(window: HWND) -> Result<(i32, i32), String> {
    let mut rect = RECT::default();
    // SAFETY: `rect` outlives the call.
    unsafe { GetClientRect(window, &mut rect) }.map_err(|e| format!("GetClientRect: {e}"))?;
    Ok((rect.right - rect.left, rect.bottom - rect.top))
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
/// with no move, no size and no activation.
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
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
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

/// One touch contact at a screen point.
fn contact(phase: Phase, point: POINT) -> POINTER_TOUCH_INFO {
    POINTER_TOUCH_INFO {
        pointerInfo: POINTER_INFO {
            pointerType: PT_TOUCH,
            pointerId: 0,
            pointerFlags: flags(phase),
            ptPixelLocation: point,
            ..Default::default()
        },
        touchFlags: TOUCH_FLAG_NONE,
        touchMask: TOUCH_MASK_CONTACTAREA,
        rcContact: RECT {
            left: point.x - CONTACT_RADIUS,
            top: point.y - CONTACT_RADIUS,
            right: point.x + CONTACT_RADIUS,
            bottom: point.y + CONTACT_RADIUS,
        },
        ..Default::default()
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

/// The Windows backend: the cursor saved at a contact's down.
#[derive(Debug, Default)]
pub struct Win {
    cursor: Option<POINT>,
}

impl Win {
    pub fn new() -> Self {
        Self::default()
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

    fn take(&mut self, window: WindowId) -> Result<Taken, String> {
        let handle = hwnd(window);
        if !exists(handle) {
            return Err("no such window".to_string());
        }
        // SAFETY: a plain query of a live handle.
        let style = unsafe { GetWindowLongW(handle, GWL_EXSTYLE) } as u32;
        let was_topmost = (style & WS_EX_TOPMOST.0) != 0;
        place(handle, true)?;
        let picture = children(handle)
            .into_iter()
            .find(|c| class_of(*c) == PICTURE_CLASS)
            .unwrap_or(handle);
        let (width, height) = client_size(picture)?;
        Ok(Taken {
            window,
            picture: id_of(picture),
            pid: pid_of(handle),
            was_topmost,
            width: u32::try_from(width).unwrap_or(0),
            height: u32::try_from(height).unwrap_or(0),
        })
    }

    fn alive(&mut self, taken: &Taken) -> bool {
        let (window, picture) = (hwnd(taken.window), hwnd(taken.picture));
        exists(window) && exists(picture) && visible(window)
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
        let point = to_screen(hwnd(taken.picture), at)?;
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
        if phase.ends()
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
    }
}
