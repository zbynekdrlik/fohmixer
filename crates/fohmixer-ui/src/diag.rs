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
//! most one report per kind per [`REPORT_GAP_MS`], an error message only
//! once) and the hellos (the first is `connected`, the later ones
//! `reconnect`).
//!
//! The browser glue ([`install`], [`connected`], [`disconnected`] and the
//! helpers under them) reads the browser, asks [`Diag`] and posts to
//! `/api/client-report` with `fetch` `keepalive` (a report sent while the
//! page reloads still arrives); it decides nothing.

use std::cell::RefCell;
use std::collections::VecDeque;

use fohmixer_proto::client::ReportFields;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::dom;
use crate::lifecycle::truncate_for_display;

/// At most one report of a kind per this interval (page clock, ms).
pub const REPORT_GAP_MS: f64 = 5000.0;
/// The longest error text a report carries, in characters.
pub const ERROR_MAX_CHARS: usize = 300;
/// How many error messages are remembered as sent (older ones may be sent
/// again).
pub const ERRORS_REMEMBERED: usize = 32;
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
}

/// How many kinds there are (the throttle's slots).
const KINDS: usize = 8;

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
        }
    }
}

/// Whether a report last sent at `last` (page clock) lets another go at
/// `now`: none sent yet, [`REPORT_GAP_MS`] passed, or a clock that went
/// backwards.
fn gap_passed(now: f64, last: Option<f64>) -> bool {
    last.is_none_or(|t| now - t >= REPORT_GAP_MS || now < t)
}

/// The page's report state: the throttle and the hellos.
#[derive(Debug, Default)]
pub struct Diag {
    /// When each kind was last sent (page clock, ms), by [`Kind::slot`].
    last: [Option<f64>; KINDS],
    /// The error messages sent, the oldest first.
    errors: VecDeque<String>,
    /// Hub hellos since the page loaded.
    hellos: u32,
}

impl Diag {
    /// Whether a report of `kind` (an error: with its `error` text) goes out
    /// at `now`; one that goes is recorded. A kind goes at most once per
    /// [`REPORT_GAP_MS`]; an error message already sent never again.
    pub fn allow(&mut self, kind: Kind, now: f64, error: Option<&str>) -> bool {
        if !gap_passed(now, self.last[kind.slot()]) {
            return false;
        }
        if let Some(message) = error {
            if self.errors.iter().any(|sent| sent == message) {
                return false;
            }
            self.errors.push_back(message.to_string());
            if self.errors.len() > ERRORS_REMEMBERED {
                self.errors.pop_front();
            }
        }
        self.last[kind.slot()] = Some(now);
        true
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
    }
}

// ---------------------------------------------------------------------------
// Browser glue
// ---------------------------------------------------------------------------

thread_local! {
    /// The page's one report state (WASM runs on one thread).
    static DIAG: RefCell<Diag> = RefCell::new(Diag::default());
}

/// Listens for errors, unhandled rejections, visibility changes and the
/// service worker's and wake lock's attributes, then reports the load.
/// Called once, from `main`; the listeners live as long as the page.
pub fn install() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let on_error = Closure::wrap(Box::new(|event: web_sys::ErrorEvent| {
        report_error(error_event_text(
            &event.message(),
            &event.filename(),
            event.lineno(),
            event.colno(),
        ));
    }) as Box<dyn FnMut(web_sys::ErrorEvent)>);
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
        let on_visibility =
            Closure::wrap(Box::new(|| report(Kind::Visibility)) as Box<dyn FnMut()>);
        let _ = document.add_event_listener_with_callback(
            "visibilitychange",
            on_visibility.as_ref().unchecked_ref(),
        );
        on_visibility.forget();
        if let Some(root) = document.document_element() {
            observe_attributes(&root);
        }
    }
    report(Kind::Load);
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

/// Sends a report of `kind` when the throttle lets it go.
fn send(kind: Kind, error: Option<String>) {
    let error = error.map(|text| truncate_for_display(&text, ERROR_MAX_CHARS));
    let Ok(Some(reconnects)) = DIAG.try_with(|diag| {
        let mut diag = diag.borrow_mut();
        if diag.allow(kind, dom::now(), error.as_deref()) {
            Some(diag.reconnects())
        } else {
            None
        }
    }) else {
        return;
    };
    let body = fields(kind, &read_env(), reconnects, error.as_deref());
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
