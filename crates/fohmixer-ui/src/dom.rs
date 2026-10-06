//! Thin browser helpers (web_sys): the page and wall clocks, style and attribute
//! writes for the animation loop, the console log, local storage, and the
//! one listener that prevents an event's default action (#43 PR G). Local
//! storage can throw (a private window, blocked site data), so every access
//! is a `Result` that is dropped: a failed read is "nothing stored".

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

thread_local! {
    /// The one listener that prevents an event's default action (#43 PR G),
    /// shared by every element that prevents one: no closure per element,
    /// and adding it again for the same event (a directive run again) is a
    /// no-op.
    static PREVENT: Closure<dyn FnMut(web_sys::Event)> = Closure::wrap(
        Box::new(|event: web_sys::Event| event.prevent_default()) as Box<dyn FnMut(web_sys::Event)>,
    );
}

/// Prevents the default action of each of `names` on `target` and inside it
/// (#43 PR G), with an active listener (`passive: false`): a passive one
/// cannot, and Chromium makes a `touchstart` listener on the window passive
/// by default, so it goes on the element itself.
pub fn prevent_on(target: &web_sys::EventTarget, names: &[&str]) {
    let options = web_sys::AddEventListenerOptions::new();
    options.set_passive(false);
    let _ = PREVENT.try_with(|listener| {
        for name in names {
            let _ = target.add_event_listener_with_callback_and_add_event_listener_options(
                name,
                listener.as_ref().unchecked_ref(),
                &options,
            );
        }
    });
}

/// The element an event came from: its target, or the target's parent
/// element when that is a text node (a selection can start in one).
pub fn event_element(event: &web_sys::Event) -> Option<web_sys::Element> {
    match event.target()?.dyn_into::<web_sys::Element>() {
        Ok(element) => Some(element),
        Err(other) => other.dyn_into::<web_sys::Node>().ok()?.parent_element(),
    }
}

/// The page clock in ms (`performance.now()`, the clock of the
/// `requestAnimationFrame` timestamps and of the events).
pub fn now() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or_else(js_sys::Date::now, |p| p.now())
}

/// The page clock on the epoch (`performance.timeOrigin +
/// performance.now()`, ms): the time a `set` or a `ping` carries (#43), so
/// the hub can map it onto its own clock.
pub fn epoch_now() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or_else(js_sys::Date::now, |p| p.time_origin() + p.now())
}

/// When a browser event happened on the page clock of [`epoch_now`] (ms
/// since the epoch): its `timeStamp` (ms since the page's time origin) plus
/// that origin (#43 PR D: a pointer move's own time, not its handler's).
pub fn event_epoch(event: &web_sys::Event) -> f64 {
    let origin = web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.time_origin());
    origin + event.time_stamp()
}

/// Whether the page is hidden (`document.hidden`).
pub fn hidden() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .is_some_and(|d| d.hidden())
}

/// The wall clock in ms (`Date.now()`): unlike the page clock it goes on
/// across page loads, so a time kept in local storage (the last handshake
/// reload) is compared on it.
pub fn wall_now() -> f64 {
    js_sys::Date::now()
}

/// An informational console line (the E2E console guard fails on warnings
/// and errors, so only real failures use those).
pub fn log(message: &str) {
    web_sys::console::log_1(&message.into());
}

/// A real failure, on the console.
pub fn error(message: &str) {
    web_sys::console::error_1(&message.into());
}

/// `element.style[prop] = value`.
pub fn set_style(element: &web_sys::HtmlElement, prop: &str, value: &str) {
    let _ = element.style().set_property(prop, value);
}

/// `element.setAttribute(name, value)`.
pub fn set_attr(element: &web_sys::Element, name: &str, value: &str) {
    let _ = element.set_attribute(name, value);
}

/// Removes an attribute (failures ignored, like `set_attr`).
pub fn remove_attr(element: &web_sys::Element, name: &str) {
    let _ = element.remove_attribute(name);
}

/// The viewport's size in CSS px.
pub fn viewport() -> (f64, f64) {
    let Some(window) = web_sys::window() else {
        return (0.0, 0.0);
    };
    let size = |v: Result<wasm_bindgen::JsValue, _>| v.ok().and_then(|v| v.as_f64()).unwrap_or(0.0);
    (size(window.inner_width()), size(window.inner_height()))
}

/// The element an event was registered on, as an HTML element.
pub fn current_element(event: &web_sys::Event) -> Option<web_sys::HtmlElement> {
    event.current_target()?.dyn_into().ok()
}

/// The first descendant of `root` matching `selector`, as an HTML element.
pub fn child(root: &web_sys::Element, selector: &str) -> Option<web_sys::HtmlElement> {
    root.query_selector(selector).ok()??.dyn_into().ok()
}

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// A local-storage value, if one is stored and readable.
pub fn storage_get(key: &str) -> Option<String> {
    storage()?.get_item(key).ok()?
}

/// Stores a local-storage value (a failure is only logged).
pub fn storage_set(key: &str, value: &str) {
    if storage().is_none_or(|s| s.set_item(key, value).is_err()) {
        log(&format!("local storage: cannot store {key}"));
    }
}

/// Removes a local-storage value.
pub fn storage_remove(key: &str) {
    if let Some(s) = storage() {
        let _ = s.remove_item(key);
    }
}
