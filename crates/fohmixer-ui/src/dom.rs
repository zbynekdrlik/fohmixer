//! Thin browser helpers (web_sys): the page and wall clocks, style and attribute
//! writes for the animation loop, the console log and local storage. Local
//! storage can throw (a private window, blocked site data), so every access
//! is a `Result` that is dropped: a failed read is "nothing stored".

use wasm_bindgen::JsCast;

/// The page clock in ms (`performance.now()`, the clock of the
/// `requestAnimationFrame` timestamps and of the events).
pub fn now() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or_else(js_sys::Date::now, |p| p.now())
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

/// Fits `el`'s text into `el`: sets `base_px` (or, for `None`, the
/// stylesheet's size, an earlier fit dropped) as its font size, measures the
/// laid-out text against the element's box (both in page px, so the stage's
/// scale cancels out) and sets the size [`crate::stage::fitted_font`] gives.
pub fn fit_text(el: &web_sys::HtmlElement, base_px: Option<f64>) {
    let base = match base_px {
        Some(px) => px,
        None => {
            let _ = el.style().remove_property("font-size");
            let Some(px) = computed_px(el, "font-size") else {
                return;
            };
            px
        }
    };
    set_style(el, "font-size", &format!("{base}px"));
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(range) = document.create_range() else {
        return;
    };
    if range.select_node_contents(el).is_err() {
        return;
    }
    let text = range.get_bounding_client_rect();
    let room = el.get_bounding_client_rect();
    let size = crate::stage::fitted_font(
        base,
        (text.width(), text.height()),
        (room.width(), room.height()),
    );
    set_style(el, "font-size", &format!("{size}px"));
}

/// A computed CSS length of `el` in px (`getComputedStyle`).
fn computed_px(el: &web_sys::HtmlElement, prop: &str) -> Option<f64> {
    let style = web_sys::window()?.get_computed_style(el).ok()??;
    crate::stage::px_value(&style.get_property_value(prop).ok()?)
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

/// Calls `f` once the document's fonts have loaded (`document.fonts.ready`,
/// #21); never where the browser has no font loading API.
pub fn on_fonts_ready(f: impl FnOnce() + 'static) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(fonts) = js_sys::Reflect::get(&document, &"fonts".into()) else {
        return;
    };
    let Ok(ready) = js_sys::Reflect::get(&fonts, &"ready".into()) else {
        return;
    };
    let Ok(promise) = ready.dyn_into::<js_sys::Promise>() else {
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        if wasm_bindgen_futures::JsFuture::from(promise).await.is_ok() {
            f();
        }
    });
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
