//! The page's diagnostic reports (#26): what the page tells the hub about
//! itself, so the hub log and `/api/status` (`client_reports`) show what a
//! tablet does — a browser tab or a Home Screen app, its service worker,
//! wake lock, hub socket, visibility and errors — without anyone having to
//! look at the tablet.
//!
//! The pure part, tested natively: the report's fields from what the
//! browser says ([`fields`], [`display_mode`], [`screen_text`],
//! [`visibility`], [`error_event_text`]), which changed `<html>` attribute
//! means which report ([`attribute_kind`]), and [`Diag`]: the throttle (at
//! most one report per kind per [`REPORT_GAP_MS`], a change within the
//! window sent once when it ends with the state of then, an error message
//! only once) and the hellos (the first is `connected`, the later ones
//! `reconnect`). The `perf` report's numbers (#5, K4: the frame rate, the
//! longest frame, the most pointers at once and their type, and when a
//! `perf` report is due) are [`perf`]'s.
//!
//! The browser glue ([`install`], [`connected`], [`disconnected`],
//! [`frame`] and the helpers under them) reads the browser, asks [`Diag`]
//! and [`perf::Perf`] and posts to `/api/client-report` with `fetch`
//! `keepalive` (a report sent while the page reloads still arrives); it
//! decides nothing. The animation loop (`raf::tick`) hands [`frame`] every
//! frame's gap, and capture-phase `pointerdown` / `pointerup` /
//! `pointercancel` listeners on the window hand [`perf::Perf`] every
//! pointer.
//!
//! The page's flight recorder (#43, [`trace`]) lives here too: one per page
//! ([`record`], [`with_trace`]); the frame loop's long frames and the
//! visibility changes go into it from here, the store's and the controls'
//! events from there, and the store uploads it. So do the system's gestures
//! (#43 PR G, [`trace::sys`]): bubble-phase listeners on the window hear a
//! context menu, a selection, a drag, a pinch's start, a cancelled pointer
//! and a lost capture after the surface's own listeners ran, and the visual
//! viewport's `resize` its zoom; [`perf::Perf`] says which pointers are
//! down (a capture lost after a lift is no news, [`trace::sys::records`]).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::Duration;

use fohmixer_proto::client::ReportFields;
use leptos::prelude::set_timeout;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::dom;
use crate::lifecycle::truncate_for_display;

pub mod perf;
pub mod trace;

use perf::Perf;
use trace::Recorder;
use trace::sys::{self, ZoomWatch};

/// At most one report of a kind per this interval (page clock, ms).
pub const REPORT_GAP_MS: f64 = 5000.0;
/// The longest error text a report carries, in characters.
pub const ERROR_MAX_CHARS: usize = 300;
/// How many error messages are remembered as sent (older ones may be sent
/// again).
pub const ERRORS_REMEMBERED: usize = 32;
/// How many errors wait for the error window at most (more are dropped).
pub const ERRORS_WAITING: usize = 8;
/// Where reports go.
pub const REPORT_URL: &str = "/api/client-report";

/// What a report is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The app loaded.
    Load,
    /// The hub socket's first hello.
    Connected,
    /// The hub socket closed or went silent.
    Disconnected,
    /// A later hello.
    Reconnect,
    /// The page was hidden or shown.
    Visibility,
    /// The service worker's state changed (`data-sw`).
    Sw,
    /// The wake lock's state changed (`data-wake-lock`).
    WakeLock,
    /// A JavaScript error or an unhandled promise rejection.
    Error,
    /// The frame rate and the pointers at once (#5, K4): a window of
    /// frames closed a minute or more after the last `perf` report, or the
    /// page saw more pointers at once than ever ([`perf`]).
    Perf,
}

/// How many kinds there are (the throttle's slots).
const KINDS: usize = 9;

impl Kind {
    /// The name the hub logs.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Load => "load",
            Kind::Connected => "connected",
            Kind::Disconnected => "disconnected",
            Kind::Reconnect => "reconnect",
            Kind::Visibility => "visibility",
            Kind::Sw => "sw",
            Kind::WakeLock => "wake-lock",
            Kind::Error => "error",
            Kind::Perf => "perf",
        }
    }

    /// The throttle's slot of this kind.
    fn slot(self) -> usize {
        match self {
            Kind::Load => 0,
            Kind::Connected => 1,
            Kind::Disconnected => 2,
            Kind::Reconnect => 3,
            Kind::Visibility => 4,
            Kind::Sw => 5,
            Kind::WakeLock => 6,
            Kind::Error => 7,
            Kind::Perf => 8,
        }
    }
}

/// Whether a report last sent at `last` (page clock) lets another go at
/// `now`: none sent yet, [`REPORT_GAP_MS`] passed, or a clock that went
/// backwards.
fn gap_passed(now: f64, last: Option<f64>) -> bool {
    last.is_none_or(|t| now - t >= REPORT_GAP_MS || now < t)
}

/// What becomes of an offered report ([`Diag::offer`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    /// Send it now.
    Now,
    /// Its kind went less than [`REPORT_GAP_MS`] ago: arm one timer for
    /// this page time, when the kind's trailing report goes with the state
    /// of that moment ([`Diag::fire`]), so the hub never keeps a stale last
    /// state (hidden, the wake lock released) of a page that came back.
    Later(f64),
    /// Nothing to do: the error was sent already or waits its turn, or the
    /// kind's trailing report is armed and will carry this change too.
    Skip,
}

/// What an armed trailing report does when its timer fires.
#[derive(Debug, Clone, PartialEq)]
pub struct Fired {
    /// Whether the report goes.
    pub send: bool,
    /// The error it carries (an `error` report).
    pub error: Option<String>,
    /// Another error waits: arm the timer again for this page time.
    pub again: Option<f64>,
}

/// The page's report state: the throttle and the hellos.
#[derive(Debug, Default)]
pub struct Diag {
    /// When each kind last went (page clock, ms), by [`Kind::slot`].
    last: [Option<f64>; KINDS],
    /// A trailing report of the kind is armed.
    armed: [bool; KINDS],
    /// The error messages sent, the oldest first.
    errors: VecDeque<String>,
    /// Errors waiting for the error window, the oldest first.
    waiting: VecDeque<String>,
    /// Hub hellos since the page loaded.
    hellos: u32,
}

impl Diag {
    /// A report of `kind` (an error: with its `error` text) at `now`. A kind
    /// goes at most once per [`REPORT_GAP_MS`]: within the window it is
    /// sent later, once, with the state of then (an error: each waiting
    /// message in turn, at most [`ERRORS_WAITING`] of them). An error
    /// message already sent or waiting is never sent again.
    pub fn offer(&mut self, kind: Kind, now: f64, error: Option<&str>) -> Verdict {
        let slot = kind.slot();
        if let Some(message) = error
            && self
                .errors
                .iter()
                .chain(&self.waiting)
                .any(|known| known == message)
        {
            return Verdict::Skip;
        }
        if self.armed[slot] {
            self.wait(error);
            return Verdict::Skip;
        }
        match self.last[slot] {
            Some(last) if !gap_passed(now, Some(last)) => {
                self.wait(error);
                self.armed[slot] = true;
                Verdict::Later(last + REPORT_GAP_MS)
            }
            _ => {
                self.went(slot, now, error);
                Verdict::Now
            }
        }
    }

    /// The armed trailing report of `kind` fires at `now`.
    pub fn fire(&mut self, kind: Kind, now: f64) -> Fired {
        let slot = kind.slot();
        self.armed[slot] = false;
        if kind != Kind::Error {
            self.went(slot, now, None);
            return Fired {
                send: true,
                error: None,
                again: None,
            };
        }
        let Some(error) = self.waiting.pop_front() else {
            return Fired {
                send: false,
                error: None,
                again: None,
            };
        };
        self.went(slot, now, Some(&error));
        let again = if self.waiting.is_empty() {
            None
        } else {
            self.armed[slot] = true;
            Some(now + REPORT_GAP_MS)
        };
        Fired {
            send: true,
            error: Some(error),
            again,
        }
    }

    /// An error that waits for its window (dropped when
    /// [`ERRORS_WAITING`] already wait).
    fn wait(&mut self, error: Option<&str>) {
        if let Some(message) = error
            && self.waiting.len() < ERRORS_WAITING
        {
            self.waiting.push_back(message.to_string());
        }
    }

    /// A report of the kind in `slot` went at `now`.
    fn went(&mut self, slot: usize, now: f64, error: Option<&str>) {
        self.last[slot] = Some(now);
        if let Some(message) = error {
            self.errors.push_back(message.to_string());
            if self.errors.len() > ERRORS_REMEMBERED {
                self.errors.pop_front();
            }
        }
    }

    /// A hub hello: what it reports (`Connected` the first time,
    /// `Reconnect` after).
    pub fn hello(&mut self) -> Kind {
        self.hellos = self.hellos.saturating_add(1);
        if self.hellos == 1 {
            Kind::Connected
        } else {
            Kind::Reconnect
        }
    }

    /// The socket's reconnects so far: the hellos after the first.
    pub fn reconnects(&self) -> u32 {
        self.hellos.saturating_sub(1)
    }
}

/// What the browser says about the page (read by the glue).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Env {
    /// `matchMedia('(display-mode: standalone)')` matches.
    pub standalone_media: bool,
    /// `navigator.standalone` (iOS: a Home Screen app).
    pub navigator_standalone: bool,
    pub ua: String,
    /// `location.host`.
    pub host: String,
    /// `screen.width`, `screen.height` (CSS px) and `devicePixelRatio`.
    pub screen: (f64, f64, f64),
    /// `data-sw` / `data-wake-lock` on `<html>` (none off https).
    pub sw: Option<String>,
    pub wake_lock: Option<String>,
    /// `document.hidden`.
    pub hidden: bool,
}

/// `standalone` for a Home Screen app (either sign), else `browser`.
pub fn display_mode(standalone_media: bool, navigator_standalone: bool) -> &'static str {
    if standalone_media || navigator_standalone {
        "standalone"
    } else {
        "browser"
    }
}

/// The screen as `<width>x<height>@<device pixel ratio>` (`1194x834@2`).
pub fn screen_text(width: f64, height: f64, dpr: f64) -> String {
    format!("{width}x{height}@{dpr}")
}

/// `hidden` or `visible`.
pub fn visibility(hidden: bool) -> &'static str {
    if hidden { "hidden" } else { "visible" }
}

/// A window `error` event's text: the message, and where when the browser
/// says (`file:line:column`).
pub fn error_event_text(message: &str, file: &str, line: u32, column: u32) -> String {
    if file.is_empty() {
        message.to_string()
    } else {
        format!("{message} at {file}:{line}:{column}")
    }
}

/// The report a changed `<html>` attribute makes.
pub fn attribute_kind(name: &str) -> Option<Kind> {
    match name {
        "data-sw" => Some(Kind::Sw),
        "data-wake-lock" => Some(Kind::WakeLock),
        _ => None,
    }
}

/// A report's fields: the event, the page's state, this bundle's build,
/// the reconnects so far and the error's text (cut to [`ERROR_MAX_CHARS`]).
/// A `perf` report's numbers are added by [`perf::PerfReport::fill`].
pub fn fields(kind: Kind, env: &Env, reconnects: u32, error: Option<&str>) -> ReportFields {
    let (width, height, dpr) = env.screen;
    ReportFields {
        kind: Some(kind.name().to_string()),
        display: Some(display_mode(env.standalone_media, env.navigator_standalone).to_string()),
        ua: Some(env.ua.clone()),
        build: Some(fohmixer_proto::VERSION.to_string()),
        host: Some(env.host.clone()),
        screen: Some(screen_text(width, height, dpr)),
        sw: env.sw.clone(),
        wake_lock: env.wake_lock.clone(),
        visibility: Some(visibility(env.hidden).to_string()),
        reconnects: Some(reconnects.to_string()),
        error: error.map(|text| truncate_for_display(text, ERROR_MAX_CHARS)),
        fps: None,
        long_frame_ms: None,
        touches_max: None,
        pointer: None,
    }
}

// ---------------------------------------------------------------------------
// Browser glue
// ---------------------------------------------------------------------------

thread_local! {
    /// The page's one report state (WASM runs on one thread).
    static DIAG: RefCell<Diag> = RefCell::new(Diag::default());
    /// The page's frame and pointer counts (#5, K4).
    static PERF: RefCell<Perf> = RefCell::new(Perf::default());
    /// The page's flight recorder (#43).
    static TRACE: RefCell<Recorder> = RefCell::new(Recorder::default());
    /// The last zoom recorded (#43 PR G).
    static ZOOM: RefCell<ZoomWatch> = RefCell::new(ZoomWatch::default());
}

/// Records `event` in the page's flight recorder (#43, [`trace`]).
pub fn record(event: &serde_json::Value) {
    let _ = TRACE.try_with(|r| r.borrow_mut().push(event));
}

/// Records `event` as essential (#43 PR D: a touch's first moves stay when
/// the recorder's backlog is over its bound).
pub fn record_essential(event: &serde_json::Value) {
    let _ = TRACE.try_with(|r| r.borrow_mut().push_essential(event));
}

/// Runs `f` on the page's flight recorder (the store's uploads).
pub fn with_trace<T>(f: impl FnOnce(&mut Recorder) -> T) -> Option<T> {
    TRACE.try_with(|r| f(&mut r.borrow_mut())).ok()
}

/// Listens for errors, unhandled rejections, visibility changes, the
/// service worker's and wake lock's attributes and the pointers, then
/// reports the load. Called once, from `main`; the listeners live as long
/// as the page.
pub fn install() {
    let Some(window) = web_sys::window() else {
        return;
    };
    // Any `error` event reaches this listener, not only an `ErrorEvent`
    // (one dispatched as a plain `Event` has no message to read).
    let on_error = Closure::wrap(Box::new(|event: web_sys::Event| {
        let text = match event.dyn_ref::<web_sys::ErrorEvent>() {
            Some(error) => error_event_text(
                &error.message(),
                &error.filename(),
                error.lineno(),
                error.colno(),
            ),
            None => format!("an {} event without a message", event.type_()),
        };
        report_error(text);
    }) as Box<dyn FnMut(web_sys::Event)>);
    let _ = window.add_event_listener_with_callback("error", on_error.as_ref().unchecked_ref());
    on_error.forget();
    let on_rejection = Closure::wrap(Box::new(|event: web_sys::PromiseRejectionEvent| {
        report_error(format!(
            "unhandled rejection: {}",
            rejection_text(&event.reason())
        ));
    }) as Box<dyn FnMut(web_sys::PromiseRejectionEvent)>);
    let _ = window.add_event_listener_with_callback(
        "unhandledrejection",
        on_rejection.as_ref().unchecked_ref(),
    );
    on_rejection.forget();
    if let Some(document) = window.document() {
        let on_visibility = Closure::wrap(Box::new(|| {
            pause_perf();
            trace_visibility();
            report(Kind::Visibility);
        }) as Box<dyn FnMut()>);
        let _ = document.add_event_listener_with_callback(
            "visibilitychange",
            on_visibility.as_ref().unchecked_ref(),
        );
        on_visibility.forget();
        if let Some(root) = document.document_element() {
            observe_attributes(&root);
        }
    }
    listen_pointers(&window);
    listen_gestures(&window);
    report(Kind::Load);
}

/// Records the system's gestures (#43 PR G, [`trace::sys`]): each of
/// [`sys::RECORDED`] that reaches the window (the bubble phase, so the
/// surface's own listeners already said whether it is prevented), and the
/// visual viewport's zoom at load and at each `resize` of it.
fn listen_gestures(window: &web_sys::Window) {
    let on_gesture = Closure::wrap(Box::new(|event: web_sys::Event| {
        gesture(&event);
    }) as Box<dyn FnMut(web_sys::Event)>);
    for name in sys::RECORDED {
        let _ = window.add_event_listener_with_callback(name, on_gesture.as_ref().unchecked_ref());
    }
    on_gesture.forget();
    let Some(viewport) = window.visual_viewport() else {
        return;
    };
    let on_resize = Closure::wrap(Box::new(zoomed) as Box<dyn FnMut()>);
    let _ = viewport.add_event_listener_with_callback("resize", on_resize.as_ref().unchecked_ref());
    on_resize.forget();
    zoomed();
}

/// A system gesture reached the window: a `sys` event with the page clock,
/// unless [`sys::records`] leaves it out (a capture lost after its
/// pointer's lift: [`perf::Perf`] no longer has it down, the capture-phase
/// `pointerup` / `pointercancel` ran first).
fn gesture(event: &web_sys::Event) {
    let what = event.type_();
    let pointer = event
        .dyn_ref::<web_sys::PointerEvent>()
        .map(web_sys::PointerEvent::pointer_id);
    let held = pointer.is_some_and(|id| {
        PERF.try_with(|perf| perf.borrow().is_down(id))
            .unwrap_or(false)
    });
    if !sys::records(&what, held) {
        return;
    }
    let (on, keys) = control_of(dom::event_element(event).as_ref());
    let prevented = event.default_prevented();
    record(&sys::sys(
        dom::epoch_now(),
        &what,
        &on,
        &keys,
        pointer,
        prevented,
    ));
}

/// What a gesture's record calls `element` ([`sys::kind`]) and the keys of
/// the control it lies in (its root's [`sys::KEYS_ATTR`]; none outside a
/// control or without an element).
fn control_of(element: Option<&web_sys::Element>) -> (String, Vec<String>) {
    let closest = |selector: &str| element.and_then(|e| e.closest(selector).ok().flatten());
    let root = closest(&format!("[{}]", sys::KEYS_ATTR));
    let nearest = closest("[data-testid]");
    let testid = |e: Option<&web_sys::Element>| e.and_then(|e| e.get_attribute("data-testid"));
    let keys = root
        .as_ref()
        .and_then(|r| r.get_attribute(sys::KEYS_ATTR))
        .map(|text| sys::keys_of(&text))
        .unwrap_or_default();
    let on = sys::kind(
        testid(root.as_ref()).as_deref(),
        testid(nearest.as_ref()).as_deref(),
        element.map(web_sys::Element::tag_name).as_deref(),
    );
    (on, keys)
}

/// The visual viewport's scale now: a `zoom` event when
/// [`ZoomWatch::zoom`] says it changed.
fn zoomed() {
    let Some(scale) = web_sys::window()
        .and_then(|w| w.visual_viewport())
        .map(|v| v.scale())
    else {
        return;
    };
    let changed = ZOOM.try_with(|z| z.borrow_mut().zoom(scale));
    if let Ok(Some(scale)) = changed {
        record(&sys::zoom(dom::epoch_now(), scale));
    }
}

/// Counts every pointer that goes down, up or is cancelled anywhere on the
/// page: capture-phase listeners on the window see each one before any
/// control does (and a control that stops its propagation cannot hide it).
fn listen_pointers(window: &web_sys::Window) {
    let on_down = Closure::wrap(Box::new(|event: web_sys::Event| {
        if let Some(pointer) = event.dyn_ref::<web_sys::PointerEvent>() {
            pointer_down(pointer);
        }
    }) as Box<dyn FnMut(web_sys::Event)>);
    let _ = window.add_event_listener_with_callback_and_bool(
        "pointerdown",
        on_down.as_ref().unchecked_ref(),
        true,
    );
    on_down.forget();
    let on_up = Closure::wrap(Box::new(|event: web_sys::Event| {
        if let Some(pointer) = event.dyn_ref::<web_sys::PointerEvent>() {
            pointer_up(pointer);
        }
    }) as Box<dyn FnMut(web_sys::Event)>);
    for name in ["pointerup", "pointercancel"] {
        let _ = window.add_event_listener_with_callback_and_bool(
            name,
            on_up.as_ref().unchecked_ref(),
            true,
        );
    }
    on_up.forget();
}

/// A pointer went down: counted, and a `perf` report when the page never
/// had as many at once.
fn pointer_down(event: &web_sys::PointerEvent) {
    let (id, pointer_type) = (event.pointer_id(), event.pointer_type());
    let primary = event.is_primary();
    let high = PERF.try_with(|perf| perf.borrow_mut().down(id, &pointer_type, primary));
    if high.unwrap_or(false) {
        report(Kind::Perf);
    }
}

/// A pointer went up or was cancelled.
fn pointer_up(event: &web_sys::PointerEvent) {
    let id = event.pointer_id();
    let _ = PERF.try_with(|perf| perf.borrow_mut().up(id));
}

/// The page was hidden or shown: the frame count pauses.
fn pause_perf() {
    let _ = PERF.try_with(|perf| perf.borrow_mut().pause());
}

/// The page was hidden or shown: the flight recorder's event (the next
/// frame's gap is the time hidden, not a stall).
fn trace_visibility() {
    let (t, hidden) = (dom::epoch_now(), dom::hidden());
    let _ = TRACE.try_with(|r| r.borrow_mut().visibility(t, hidden));
}

/// A frame of the animation loop (`raf::tick`), `gap` ms after the one
/// before: counted, recorded when long (#43, the flight recorder), and the
/// periodic `perf` report when it is due. The report's minute is kept on
/// the page clock the reports are stamped with (`dom::now`), not on the
/// frame's own time, which is the frame's start and may lie before the last
/// report's stamp.
pub fn frame(gap: f64) {
    let now = dom::now();
    let t = dom::epoch_now();
    let _ = TRACE.try_with(|r| r.borrow_mut().frame(t, gap));
    let due = PERF.try_with(|perf| perf.borrow_mut().frame(now, gap));
    if due.unwrap_or(false) {
        report(Kind::Perf);
    }
}

/// Reports every change of `data-sw` / `data-wake-lock` on `root` (set by
/// `index.html`'s script, which runs before the app).
fn observe_attributes(root: &web_sys::Element) {
    let on_change = Closure::wrap(
        Box::new(|records: js_sys::Array, _: wasm_bindgen::JsValue| {
            for record in records.iter() {
                let kind = record
                    .dyn_ref::<web_sys::MutationRecord>()
                    .and_then(web_sys::MutationRecord::attribute_name)
                    .and_then(|name| attribute_kind(&name));
                if let Some(kind) = kind {
                    report(kind);
                }
            }
        }) as Box<dyn FnMut(js_sys::Array, wasm_bindgen::JsValue)>,
    );
    // The observed element keeps the observer; the callback lives as long
    // as the page.
    if let Ok(observer) = web_sys::MutationObserver::new(on_change.as_ref().unchecked_ref()) {
        let options = web_sys::MutationObserverInit::new();
        options.set_attributes(true);
        let names = js_sys::Array::of2(&"data-sw".into(), &"data-wake-lock".into());
        options.set_attribute_filter(&names);
        let _ = observer.observe_with_options(root, &options);
    }
    on_change.forget();
}

/// The hub said hello (`LiveStore::on_hello`): `connected`, or `reconnect`.
pub fn connected() {
    let Ok(kind) = DIAG.try_with(|diag| diag.borrow_mut().hello()) else {
        return;
    };
    report(kind);
}

/// The hub socket closed or was dropped (`LiveStore::on_close`).
pub fn disconnected() {
    report(Kind::Disconnected);
}

fn report(kind: Kind) {
    send(kind, None);
}

fn report_error(text: String) {
    send(Kind::Error, Some(text));
}

/// Offers a report of `kind` to the throttle: sent now, or its trailing
/// report armed.
fn send(kind: Kind, error: Option<String>) {
    let error = error.map(|text| truncate_for_display(&text, ERROR_MAX_CHARS));
    let now = dom::now();
    let Ok(verdict) = DIAG.try_with(|diag| diag.borrow_mut().offer(kind, now, error.as_deref()))
    else {
        return;
    };
    match verdict {
        Verdict::Now => post_report(kind, error.as_deref()),
        Verdict::Later(at) => arm_trailing(kind, at - now),
        Verdict::Skip => {}
    }
}

/// Fires `kind`'s trailing report after `wait_ms`.
fn arm_trailing(kind: Kind, wait_ms: f64) {
    set_timeout(
        move || fire_trailing(kind),
        Duration::from_millis(wait_ms.max(0.0) as u64),
    );
}

/// `kind`'s trailing report: sent with the state of now, and armed again
/// while errors wait.
fn fire_trailing(kind: Kind) {
    let now = dom::now();
    let Ok(fired) = DIAG.try_with(|diag| diag.borrow_mut().fire(kind, now)) else {
        return;
    };
    if fired.send {
        post_report(kind, fired.error.as_deref());
    }
    if let Some(at) = fired.again {
        arm_trailing(kind, at - now);
    }
}

/// Posts a report of `kind` with the page's state of now (a `perf` report
/// with its numbers, which then count again from now).
fn post_report(kind: Kind, error: Option<&str>) {
    let Ok(reconnects) = DIAG.try_with(|diag| diag.borrow().reconnects()) else {
        return;
    };
    let mut body = fields(kind, &read_env(), reconnects, error);
    if kind == Kind::Perf {
        let now = dom::now();
        if let Ok(numbers) = PERF.try_with(|perf| perf.borrow_mut().report(now)) {
            numbers.fill(&mut body);
        }
    }
    if let Ok(text) = serde_json::to_string(&body) {
        post(&text);
    }
}

/// What the browser says about the page now.
fn read_env() -> Env {
    let Some(window) = web_sys::window() else {
        return Env::default();
    };
    let navigator = window.navigator();
    let standalone_media = window
        .match_media("(display-mode: standalone)")
        .ok()
        .flatten()
        .is_some_and(|query| query.matches());
    let navigator_standalone = js_sys::Reflect::get(&navigator, &"standalone".into())
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let screen = window.screen().ok().map_or((0.0, 0.0), |s| {
        let size = |v: Result<i32, _>| f64::from(v.unwrap_or(0));
        (size(s.width()), size(s.height()))
    });
    let document = window.document();
    let root = document
        .as_ref()
        .and_then(web_sys::Document::document_element);
    let attr = |name: &str| root.as_ref().and_then(|r| r.get_attribute(name));
    Env {
        standalone_media,
        navigator_standalone,
        ua: navigator.user_agent().unwrap_or_default(),
        host: window.location().host().unwrap_or_default(),
        screen: (screen.0, screen.1, window.device_pixel_ratio()),
        sw: attr("data-sw"),
        wake_lock: attr("data-wake-lock"),
        hidden: document.is_some_and(|d| d.hidden()),
    }
}

/// A rejection's reason as text: an `Error`'s message, a string, or its
/// JSON.
fn rejection_text(reason: &wasm_bindgen::JsValue) -> String {
    if let Some(error) = reason.dyn_ref::<js_sys::Error>() {
        return String::from(error.message());
    }
    if let Some(text) = reason.as_string() {
        return text;
    }
    js_sys::JSON::stringify(reason)
        .ok()
        .and_then(|json| json.as_string())
        .unwrap_or_else(|| "(no reason)".to_string())
}

/// POSTs a report (`keepalive`: it survives a reload that follows). The
/// answer is awaited only so a failed request is no unhandled rejection;
/// nothing depends on it.
fn post(body: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_body(&wasm_bindgen::JsValue::from_str(body));
    // web-sys 0.3.91's RequestInit has no keepalive setter.
    let _ = js_sys::Reflect::set(&init, &"keepalive".into(), &true.into());
    let Ok(headers) = web_sys::Headers::new() else {
        return;
    };
    let _ = headers.set("content-type", "application/json");
    init.set_headers(&headers);
    let Ok(request) = web_sys::Request::new_with_str_and_init(REPORT_URL, &init) else {
        return;
    };
    let answer = wasm_bindgen_futures::JsFuture::from(window.fetch_with_request(&request));
    wasm_bindgen_futures::spawn_local(async move {
        let _ = answer.await;
    });
}

#[cfg(test)]
mod tests;
