//! Windows Graphics Capture of a Pro-Q 4 editor (#215; what it rests on:
//! `.claude/rules/plugin-window.md`, Capture), the Windows backend's grab
//! when `[eq] capture = "wgc"` (the default). Compiled on Windows only and
//! out of the mutation gate, as `win.rs`: what it decides is the tested
//! `capture.rs` and the worker's rate (`plugwin::grab_due`); the `windows`
//! CI job lints and tests it, and `eq-probe --capture wgc` drives it on the
//! PC.
//!
//! The desktop-DC `BitBlt` it replaces read the composed screen back from
//! the GPU at every grab, about 13 a second however still the editor was
//! (dasio's sampler: the system cores' DPCs and the master Live's cores).
//! Here:
//!
//! - **One session per taken editor:** a `GraphicsCaptureItem` of Live's
//!   top-level window (`IGraphicsCaptureItemInterop::CreateForWindow`), a
//!   free-threaded frame pool of [`BUFFERS`] BGRA frames, the cursor left
//!   out, Windows' yellow border off where Windows allows it (from 20348;
//!   the PC's 19044 keeps it, accepted on #215).
//! - **A frame only when the window changed**, delivered on a system
//!   thread-pool thread (nothing of ours runs between frames): it is copied
//!   on the GPU into the session's newest-frame texture and counted
//!   ([`Arrivals`]); nothing is read back then. A frame of another content
//!   size recreates the pool (the newest-frame texture follows).
//! - **A grab** reads the newest frame back: the picture's box (Pro-Q's
//!   child client area in the frame, [`picture_box`]) copied on the GPU into
//!   a staging texture of its size, mapped once, its rows into the
//!   [`Pixels`] the encoder takes. The device's immediate context is shared
//!   by the frame thread and the worker under one lock (and the device is
//!   multithread-protected).
//! - **No frames:** a window that stays the same sends none, and that is
//!   no loss: the grab reads the newest frame again (the worker's 1 s
//!   refresh). A minimised window's grab fails (no stale picture); a frame
//!   of another size than the window's now (one from before a resize) is
//!   not taken, and the grab fails until the resized one comes
//!   ([`frame_current`]). A session Windows ended (the item's `Closed`) is
//!   started anew at the next grab, keeping its newest frame until the new
//!   one's first; a lost GPU device drops every session and the device,
//!   made anew at the next grab. A session that failed to start is tried
//!   again no sooner than [`START_AGAIN`]. The worker never waits for a
//!   frame (the #215 review).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{CO_E_NOTINITIALIZED, HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT,
    D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::Com::{
    APTTYPE, APTTYPEQUALIFIER, APTTYPEQUALIFIER_IMPLICIT_MTA, CoGetApartmentType, IAgileObject,
};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::HiDpi::{
    AreDpiAwarenessContextsEqual, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    GetAwarenessFromDpiAwarenessContext, GetThreadDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GA_ROOT, GW_OWNER, GWL_EXSTYLE, GetAncestor, GetClassNameW, GetWindow, GetWindowLongW,
    IsIconic, IsWindow, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};
use windows::core::{IInspectable, Interface};

use super::Pixels;
use super::capture::{
    Arrivals, CaptureCounts, CaptureDiag, apartment_name, awareness_name, frame_current,
    picture_box,
};

/// The frame pool's buffers: one DWM writes while the other is copied.
pub const BUFFERS: i32 = 2;
/// A session that failed to start is tried again no sooner than this (the
/// worker asks at every step while it has none).
pub const START_AGAIN: Duration = Duration::from_secs(1);
/// The frames' format: what the encoder reads (BGRA, 4 bytes a pixel).
const FORMAT: DirectXPixelFormat = DirectXPixelFormat::B8G8R8A8UIntNormalized;

/// A COM call's error as the backend's.
fn text(what: &str, error: windows::core::Error) -> String {
    format!("{what}: {error}")
}

/// Takes a lock, poisoned or not (a panic on the frame thread leaves the
/// state as it was: the next frame or grab goes on).
fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The Direct3D device the captures share: its immediate context under one
/// lock (the frame threads and the worker both use it), and its WinRT face
/// for the frame pools.
struct Device {
    device: ID3D11Device,
    context: Arc<Mutex<ID3D11DeviceContext>>,
    direct3d: Direct3D,
}

/// The device's WinRT face, sent to the frame thread (a pool's `Recreate`
/// needs it there).
#[derive(Clone)]
struct Direct3D(IDirect3DDevice);

// SAFETY: a WinRT Direct3D device is agile (it may be used from any
// thread; `Device::new` refuses one that does not say `IAgileObject`); the
// windows crate does not mark the interface so.
unsafe impl Send for Direct3D {}
// SAFETY: as above.
unsafe impl Sync for Direct3D {}

impl Direct3D {
    /// The device (a closure that calls this captures the whole wrapper,
    /// never its field alone, which is not `Send`).
    fn get(&self) -> &IDirect3DDevice {
        &self.0
    }
}

impl Device {
    /// A hardware device with BGRA support, multithread-protected.
    fn new() -> Result<Self, String> {
        let mut device = None;
        let mut context = None;
        // SAFETY: the out pointers outlive the call.
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(|e| text("D3D11CreateDevice", e))?;
        let device: ID3D11Device = device.ok_or("D3D11CreateDevice gave no device")?;
        let context: ID3D11DeviceContext = context.ok_or("D3D11CreateDevice gave no context")?;
        // The frame threads and the worker share the context: protected
        // besides our own lock.
        if let Ok(multithread) = device.cast::<ID3D11Multithread>() {
            // SAFETY: a plain switch of the device's own lock.
            let _ = unsafe { multithread.SetMultithreadProtected(true) };
        }
        let dxgi: IDXGIDevice = device.cast().map_err(|e| text("IDXGIDevice", e))?;
        // SAFETY: wraps the DXGI device for WinRT; no pointer is kept.
        let inspectable: IInspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
            .map_err(|e| text("CreateDirect3D11DeviceFromDXGIDevice", e))?;
        let direct3d: IDirect3DDevice =
            inspectable.cast().map_err(|e| text("IDirect3DDevice", e))?;
        // The frame threads use it (a pool's `Recreate`): it must be agile.
        direct3d
            .cast::<IAgileObject>()
            .map_err(|e| text("the WinRT Direct3D device is not agile", e))?;
        Ok(Self {
            device,
            context: Arc::new(Mutex::new(context)),
            direct3d: Direct3D(direct3d),
        })
    }
}

/// A texture of `width` × `height` BGRA pixels, on the GPU (`staging`:
/// false) or readable by the CPU (true).
fn texture(
    device: &ID3D11Device,
    width: u32,
    height: u32,
    staging: bool,
) -> Result<ID3D11Texture2D, String> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: width,
        Height: height,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: if staging {
            D3D11_USAGE_STAGING
        } else {
            D3D11_USAGE_DEFAULT
        },
        BindFlags: 0,
        CPUAccessFlags: if staging {
            D3D11_CPU_ACCESS_READ.0 as u32
        } else {
            0
        },
        MiscFlags: 0,
    };
    let mut made = None;
    // SAFETY: the description and the out pointer outlive the call.
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut made)) }
        .map_err(|e| text("CreateTexture2D", e))?;
    made.ok_or_else(|| "CreateTexture2D gave no texture".to_string())
}

/// A texture's size.
fn size_of(texture: &ID3D11Texture2D) -> (u32, u32) {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    // SAFETY: fills `desc`, which outlives the call.
    unsafe { texture.GetDesc(&mut desc) };
    (desc.Width, desc.Height)
}

/// The performance counter now, in 100 ns units (a frame's
/// `SystemRelativeTime` is on that clock).
fn qpc_100ns() -> Option<i64> {
    let (mut count, mut frequency) = (0_i64, 0_i64);
    // SAFETY: plain reads into locals.
    unsafe { QueryPerformanceCounter(&mut count).ok()? };
    // SAFETY: as above.
    unsafe { QueryPerformanceFrequency(&mut frequency).ok()? };
    (frequency > 0).then(|| (i128::from(count) * 10_000_000 / i128::from(frequency)) as i64)
}

/// What a session's frame thread and the worker share: the newest frame
/// (on the GPU), the size of what it holds, its bookkeeping, and whether
/// Windows ended the session (the window closed).
#[derive(Default)]
struct Newest {
    frame: Option<ID3D11Texture2D>,
    content: (u32, u32),
    arrivals: Arrivals,
    ended: bool,
    /// The staging texture of the last grab's box (kept for the next one
    /// of that size).
    staging: Option<ID3D11Texture2D>,
}

/// One editor's capture.
struct Session {
    item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    frames: i64,
    closed: i64,
    newest: Arc<Mutex<Newest>>,
}

impl Session {
    /// Ends it: no handler runs after its removal, the pool's buffers go.
    fn stop(&self) {
        let _ = self.pool.RemoveFrameArrived(self.frames);
        let _ = self.item.RemoveClosed(self.closed);
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}

/// A session's parts made so far: on a failure every handler is removed
/// and the pool closed before the error goes back.
struct Partial {
    item: GraphicsCaptureItem,
    pool: Direct3D11CaptureFramePool,
    frames: Option<i64>,
    closed: Option<i64>,
}

impl Partial {
    fn fail(self, why: String) -> String {
        if let Some(token) = self.frames {
            let _ = self.pool.RemoveFrameArrived(token);
        }
        if let Some(token) = self.closed {
            let _ = self.item.RemoveClosed(token);
        }
        let _ = self.pool.Close();
        why
    }
}

/// The captures: one session per taken editor's window, on one device,
/// started when first asked for; a start that failed, when and why.
#[derive(Default)]
pub struct Wgc {
    device: Option<Device>,
    sessions: HashMap<usize, Session>,
    failed: HashMap<usize, (Instant, String)>,
}

impl std::fmt::Debug for Wgc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wgc")
            .field("sessions", &self.sessions.len())
            .finish()
    }
}

/// The newest frame `texture` of `content` size kept: copied on the GPU
/// into the session's own texture (made anew for another size) and
/// counted, `lag_ms` after its composition.
fn keep(
    device: &ID3D11Device,
    context: &Mutex<ID3D11DeviceContext>,
    newest: &Mutex<Newest>,
    frame: &ID3D11Texture2D,
    content: SizeInt32,
    lag_ms: f64,
) -> Result<(), String> {
    let size = size_of(frame);
    let mut newest = locked(newest);
    let reuse = newest
        .frame
        .as_ref()
        .is_some_and(|kept| size_of(kept) == size);
    if !reuse {
        newest.frame = Some(texture(device, size.0, size.1, false)?);
    }
    let Some(kept) = newest.frame.as_ref() else {
        return Err("no texture for the newest frame".to_string());
    };
    // SAFETY: two textures of the same size and format on one device.
    unsafe { locked(context).CopyResource(kept, frame) };
    let side = |v: i32, max: u32| u32::try_from(v).unwrap_or(0).min(max);
    newest.content = (side(content.Width, size.0), side(content.Height, size.1));
    newest.arrivals.arrived(lag_ms);
    Ok(())
}

/// What decides whether a capture can start on `window` (#215): its own
/// facts (the class name's length, never a title) and this thread's
/// apartment and DPI awareness, read with calls that send the window no
/// message.
pub fn diag(window: HWND) -> CaptureDiag {
    let mut class = [0_u16; 256];
    // SAFETY: plain reads of a handle into a local buffer; a stale handle
    // reads empty.
    let class_len = unsafe { GetClassNameW(window, &mut class) };
    // SAFETY: as above.
    let is_window = unsafe { IsWindow(Some(window)) }.as_bool();
    // SAFETY: as above.
    let root = unsafe { GetAncestor(window, GA_ROOT) } == window;
    // SAFETY: as above.
    let owned = unsafe { GetWindow(window, GW_OWNER) }.is_ok_and(|owner| !owner.is_invalid());
    // SAFETY: as above.
    let ex = unsafe { GetWindowLongW(window, GWL_EXSTYLE) } as u32;
    let mut cloaked = 0_u32;
    // SAFETY: `cloaked` outlives the call and is the attribute's size.
    let cloaked = unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_CLOAKED,
            (&mut cloaked as *mut u32).cast(),
            std::mem::size_of::<u32>() as u32,
        )
    }
    .ok()
    .map(|()| cloaked);
    let (mut apttype, mut qualifier) = (APTTYPE(0), APTTYPEQUALIFIER(0));
    // SAFETY: reads this thread's apartment into locals.
    let apartment = match unsafe { CoGetApartmentType(&mut apttype, &mut qualifier) } {
        Ok(()) => apartment_name(apttype.0, qualifier == APTTYPEQUALIFIER_IMPLICIT_MTA),
        Err(e) if e.code() == CO_E_NOTINITIALIZED => "none".to_string(),
        Err(e) => format!("error_{:08x}", e.code().0),
    };
    // SAFETY: reads this thread's DPI awareness context.
    let context = unsafe { GetThreadDpiAwarenessContext() };
    // SAFETY: plain reads of a context handle.
    let (awareness, v2) = unsafe {
        (
            GetAwarenessFromDpiAwarenessContext(context).0,
            AreDpiAwarenessContextsEqual(context, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)
                .as_bool(),
        )
    };
    CaptureDiag {
        class_len: usize::try_from(class_len).unwrap_or(0),
        is_window,
        root,
        owned,
        tool: ex & WS_EX_TOOLWINDOW.0 != 0,
        app: ex & WS_EX_APPWINDOW.0 != 0,
        noactivate: ex & WS_EX_NOACTIVATE.0 != 0,
        cloaked,
        apartment,
        dpi: awareness_name(awareness, v2).to_string(),
        supported: GraphicsCaptureSession::IsSupported().ok(),
    }
}

/// A window's extended frame bounds (its visible rectangle, the origin of
/// its captured frames).
fn frame_bounds(window: HWND) -> Result<RECT, String> {
    let mut bounds = RECT::default();
    // SAFETY: `bounds` outlives the call and is the attribute's size.
    unsafe {
        DwmGetWindowAttribute(
            window,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&mut bounds as *mut RECT).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
    }
    .map_err(|e| text("DwmGetWindowAttribute", e))?;
    Ok(bounds)
}

impl Wgc {
    fn device(&mut self) -> Result<&Device, String> {
        if self.device.is_none() {
            self.device = Some(Device::new()?);
        }
        self.device
            .as_ref()
            .ok_or_else(|| "no Direct3D device".to_string())
    }

    /// A session for `window` started, its frames kept in `newest` (a new
    /// one, or the ended session's: its newest frame serves until the new
    /// session's first).
    fn start(&mut self, window: HWND, newest: Arc<Mutex<Newest>>) -> Result<Session, String> {
        if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
            return Err("Windows Graphics Capture is not supported here".to_string());
        }
        let device = self.device()?;
        let (d3d, context, direct3d) = (
            device.device.clone(),
            Arc::clone(&device.context),
            device.direct3d.clone(),
        );
        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(|e| text("IGraphicsCaptureItemInterop", e))?;
        // SAFETY: a capture item of a window handle; a stale one fails.
        // Windows 10 refuses a window Alt+Tab would not list (#215: Live's
        // editor on the PC's 19044): the window's facts go with the error.
        let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(window) }
            .map_err(|e| format!("{} ({})", text("CreateForWindow", e), diag(window).text()))?;
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
        let theirs = Arc::clone(&newest);
        let pool_size = Mutex::new(size);
        let handler =
            TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |pool, _| {
                let Some(pool) = pool.as_ref() else {
                    return Ok(());
                };
                let frame = pool.TryGetNextFrame()?;
                let composed = frame.SystemRelativeTime()?.Duration;
                let lag_ms = qpc_100ns().map_or(0.0, |now| (now - composed) as f64 / 10_000.0);
                let content = frame.ContentSize()?;
                let access: IDirect3DDxgiInterfaceAccess = frame.Surface()?.cast()?;
                // SAFETY: the frame's surface as its Direct3D texture.
                let surface: ID3D11Texture2D = unsafe { access.GetInterface()? };
                let kept = keep(&d3d, &context, &theirs, &surface, content, lag_ms);
                if let Err(why) = &kept {
                    tracing::debug!(why = %why, "a captured Pro-Q 4 frame could not be kept");
                }
                drop(surface);
                frame.Close()?;
                // Another content size: the pool's buffers follow it (as the
                // windows-capture crate does), the frames after it fit.
                let mut last = locked(&pool_size);
                if (content.Width, content.Height) != (last.Width, last.Height)
                    && content.Width > 0
                    && content.Height > 0
                {
                    pool.Recreate(direct3d.get(), FORMAT, BUFFERS, content)?;
                    *last = content;
                }
                Ok(())
            });
        match pool.FrameArrived(&handler) {
            Ok(token) => partial.frames = Some(token),
            Err(e) => return Err(partial.fail(text("FrameArrived", e))),
        }
        // Windows ends the capture when the window closes: started anew at
        // the next grab (a gone window is the worker's `alive`).
        let flagged = Arc::clone(&newest);
        let on_closed = TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
            locked(&flagged).ended = true;
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
        // go (from 20348: the PC's 19044 refuses, accepted on #215).
        let _ = session.SetIsCursorCaptureEnabled(false);
        let _ = session.SetIsBorderRequired(false);
        if let Err(e) = session.StartCapture() {
            let _ = session.Close();
            return Err(partial.fail(text("StartCapture", e)));
        }
        locked(&newest).ended = false;
        Ok(Session {
            item,
            pool,
            session,
            frames: partial.frames.unwrap_or_default(),
            closed: partial.closed.unwrap_or_default(),
            newest,
        })
    }

    /// The session of `window`: started when it has none (a start that
    /// failed less than [`START_AGAIN`] ago answers its error again) or
    /// when Windows ended it (its newest frame kept).
    fn session(&mut self, window: HWND) -> Result<&Session, String> {
        let key = window.0 as usize;
        let ended = self
            .sessions
            .get(&key)
            .is_some_and(|s| locked(&s.newest).ended);
        let mut kept = None;
        if ended && let Some(old) = self.sessions.remove(&key) {
            tracing::info!("a Pro-Q 4 capture ended by Windows: started anew");
            old.stop();
            kept = Some(old.newest);
        }
        if !self.sessions.contains_key(&key) {
            if let Some((at, why)) = self.failed.get(&key)
                && at.elapsed() < START_AGAIN
            {
                return Err(why.clone());
            }
            match self.start(window, kept.unwrap_or_default()) {
                Ok(session) => {
                    self.failed.remove(&key);
                    self.sessions.insert(key, session);
                }
                Err(why) => {
                    self.failed.insert(key, (Instant::now(), why.clone()));
                    return Err(why);
                }
            }
        }
        self.sessions
            .get(&key)
            .ok_or_else(|| "no capture session".to_string())
    }

    /// Whether `window`'s capture runs: started now when it has none (a
    /// start that failed less than [`START_AGAIN`] ago answers its error).
    pub fn started(&mut self, window: HWND) -> Result<(), String> {
        self.session(window).map(|_| ())
    }

    /// Starts the capture of a taken editor's `window` (its first frame
    /// comes at once); a failure is said and tried again at a grab.
    pub fn begin(&mut self, window: HWND) {
        if let Err(why) = self.session(window) {
            tracing::warn!(why = %why, "a Pro-Q 4 editor's capture could not start: tried again at its grab");
        }
    }

    /// Whether `window`'s picture may have changed since the last grab: a
    /// frame arrived, Windows ended its session, or it has none (the grab
    /// starts it).
    pub fn changed(&mut self, window: HWND) -> bool {
        self.sessions.get(&(window.0 as usize)).is_none_or(|s| {
            let newest = locked(&s.newest);
            newest.arrivals.changed() || newest.ended
        })
    }

    /// The counts of `window`'s session (none: no session).
    pub fn counts(&self, window: HWND) -> Option<CaptureCounts> {
        self.sessions
            .get(&(window.0 as usize))
            .map(|s| locked(&s.newest).arrivals.counts())
    }

    /// The picture of `window` (its picture's client area at the screen
    /// point `origin`, `size`) from its newest frame. A lost device drops
    /// every session and the device (made anew at the next grab).
    pub fn grab(
        &mut self,
        window: HWND,
        origin: (i32, i32),
        size: (u32, u32),
    ) -> Result<Pixels, String> {
        // SAFETY: a plain query of a handle.
        if unsafe { IsIconic(window) }.as_bool() {
            return Err("the editor's window is minimised: it gives no frames".to_string());
        }
        let bounds = frame_bounds(window)?;
        let session = self.session(window)?;
        let newest = Arc::clone(&session.newest);
        let device = self
            .device
            .as_ref()
            .ok_or_else(|| "no Direct3D device".to_string())?;
        let read = read(device, &newest, bounds, origin, size);
        // SAFETY: a plain query of the device's state.
        if read.is_err()
            && let Err(e) = unsafe { device.device.GetDeviceRemovedReason() }
        {
            tracing::warn!(error = %e, "the Pro-Q 4 capture's Direct3D device was lost: made anew at the next grab");
            self.reset();
        }
        read
    }

    /// Every session ended and the device dropped.
    fn reset(&mut self) {
        for (_, session) in self.sessions.drain() {
            session.stop();
        }
        self.device = None;
    }

    /// The capture of `window` ends (its release, a lost window, the
    /// stop).
    pub fn end(&mut self, window: HWND) {
        let key = window.0 as usize;
        self.failed.remove(&key);
        if let Some(session) = self.sessions.remove(&key) {
            session.stop();
        }
    }
}

impl Drop for Wgc {
    fn drop(&mut self) {
        self.reset();
    }
}

/// The picture's box of the newest frame read back: copied on the GPU into
/// a staging texture of its size, mapped once, its rows into [`Pixels`].
/// A frame of another size than the window's `bounds` now (one from before
/// a resize) is not taken: the grab fails, and the next frame is a change.
fn read(
    device: &Device,
    newest: &Mutex<Newest>,
    bounds: RECT,
    origin: (i32, i32),
    size: (u32, u32),
) -> Result<Pixels, String> {
    let mut newest = locked(newest);
    if !newest.arrivals.any() {
        return Err("no frame of the editor yet".to_string());
    }
    let side = |from: i32, to: i32| u32::try_from(to - from).unwrap_or(0);
    let window = (
        side(bounds.left, bounds.right),
        side(bounds.top, bounds.bottom),
    );
    if !frame_current(newest.content, window) {
        return Err(format!(
            "the newest frame is {}x{}, the window {}x{}: one of its size awaited",
            newest.content.0, newest.content.1, window.0, window.1
        ));
    }
    let within = picture_box((bounds.left, bounds.top), origin, size, newest.content)
        .ok_or("the picture lies outside the captured window")?;
    let Some(frame) = newest.frame.clone() else {
        return Err("no frame of the editor yet".to_string());
    };
    let fits = newest
        .staging
        .as_ref()
        .is_some_and(|s| size_of(s) == (within.width, within.height));
    if !fits {
        newest.staging = Some(texture(&device.device, within.width, within.height, true)?);
    }
    let staging = newest.staging.clone().ok_or("no staging texture")?;
    let source = D3D11_BOX {
        left: within.left,
        top: within.top,
        front: 0,
        right: within.left + within.width,
        bottom: within.top + within.height,
        back: 1,
    };
    let context = locked(&device.context);
    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    // SAFETY: the box lies inside `frame` (`picture_box` cut it to the
    // content, which `keep` cut to the texture), the staging texture is the
    // box's size, and the mapped rows are read within the pitch Direct3D
    // gives before the unmap.
    let bgra = unsafe {
        context.CopySubresourceRegion(&staging, 0, 0, 0, 0, &frame, 0, Some(&source));
        context
            .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .map_err(|e| text("Map", e))?;
        let row = within.width as usize * 4;
        let pitch = mapped.RowPitch as usize;
        let mut bgra = Vec::with_capacity(row * within.height as usize);
        let base = mapped.pData.cast::<u8>();
        for y in 0..within.height as usize {
            bgra.extend_from_slice(std::slice::from_raw_parts(base.add(y * pitch), row));
        }
        context.Unmap(&staging, 0);
        bgra
    };
    drop(context);
    newest.arrivals.taken();
    Ok(Pixels {
        width: within.width,
        height: within.height,
        bgra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_that_is_none_gives_no_picture_and_no_session() {
        // A CI runner has a GPU or WARP or neither; whatever it has, a
        // handle that names no window gives no capture and no picture,
        // and the captures keep nothing of it.
        let mut wgc = Wgc::default();
        let none = HWND(std::ptr::null_mut());
        assert!(wgc.changed(none), "no session: a grab would start one");
        assert!(wgc.grab(none, (0, 0), (10, 10)).is_err());
        assert!(wgc.counts(none).is_none());
        wgc.end(none);
        assert!(wgc.sessions.is_empty());
        assert!(
            wgc.failed.is_empty(),
            "its failed start forgotten at its end"
        );
        assert_eq!(START_AGAIN, Duration::from_secs(1));
        assert!(qpc_100ns().is_some_and(|t| t > 0));
    }
}
