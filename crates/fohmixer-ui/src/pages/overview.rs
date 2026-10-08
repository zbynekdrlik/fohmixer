//! A phone's overview bar (#63): a row wider than a phone's screen scrolls
//! inside itself, but nearly every part of a strip owns its touches (the
//! name button, the pan, the fader over its whole zone), so a finger had
//! almost nowhere to swipe it. The bar above the row draws the row in
//! miniature (a block per strip, in its name button's colour, dark while
//! muted) with the part on screen outlined; a touch or a drag on it moves the
//! row there. It shows only on a phone (`phone.css`), and only while the
//! shown row does not fit. Its geometry is `flow::overview_window` and
//! `flow::overview_scroll`; this file is the browser glue.

use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::owns_touches;
use crate::dom;
use crate::flow::{overview_scroll, overview_window};
use crate::raf;

/// How often the miniature follows the strips (their places and colours),
/// in ms: a strip's colour or mute changes rarely, its place only with the
/// layout.
const REDRAW_MS: f64 = 500.0;

/// The phone layout's media query: equal to `phone.css`'s, so the frame
/// loop reads nothing on a tablet.
const PHONE: &str = "(max-height: 520px), (max-width: 700px)";

/// How often the bar reads the shown row's scroll position while no finger
/// drags it, in ms (a read is a layout read; a drag reads every frame).
const READ_MS: f64 = 100.0;

/// The row number `index` of `rows` (its rows are its children, in order).
fn row_at(rows: &web_sys::Element, index: usize) -> Option<web_sys::HtmlElement> {
    let mut row = rows.first_element_child();
    for _ in 0..index {
        row = row?.next_element_sibling();
    }
    row?.dyn_into().ok()
}

/// Whether `el` carries the class `name`.
fn has_class(el: &web_sys::Element, name: &str) -> bool {
    el.class_name().split_whitespace().any(|c| c == name)
}

/// The strips under `el`, in order.
fn collect_strips(el: &web_sys::Element, out: &mut Vec<web_sys::Element>) {
    let mut next = el.first_element_child();
    while let Some(child) = next {
        if child.get_attribute("data-testid").as_deref() == Some("strip") {
            out.push(child.clone());
        } else {
            collect_strips(&child, out);
        }
        next = child.next_element_sibling();
    }
}

/// A strip's colour in the miniature: its name button's while the track is
/// audible (the accent until Live's colour came), dark while muted.
fn strip_colour(strip: &web_sys::Element) -> String {
    let Some(mute) = dom::child(strip, ".mute") else {
        return "var(--faint)".to_string();
    };
    if !has_class(&mute, "lit") {
        return "#2a2a3c".to_string();
    }
    let colour = mute.style().get_property_value("--tc").unwrap_or_default();
    if colour.is_empty() {
        "var(--accent)".to_string()
    } else {
        colour
    }
}

/// Draws the miniature of `row` (`width` px in all) into `target`: one block
/// per strip at its place, the blocks reused from the last drawing.
fn draw_strips(target: &web_sys::HtmlElement, row: &web_sys::HtmlElement, width: f64) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let origin = row.get_bounding_client_rect().left() - f64::from(row.scroll_left());
    let mut strips = Vec::new();
    collect_strips(row, &mut strips);
    let mut reuse = target.first_element_child();
    for strip in &strips {
        let rect = strip.get_bounding_client_rect();
        let style = format!(
            "left:{:.3}%;width:{:.3}%;background:{};",
            (rect.left() - origin) / width * 100.0,
            rect.width() / width * 100.0,
            strip_colour(strip)
        );
        let block = match reuse.take() {
            Some(block) => {
                reuse = block.next_element_sibling();
                block
            }
            None => {
                let Ok(block) = document.create_element("div") else {
                    return;
                };
                block.set_class_name("overview-strip");
                let _ = target.append_child(&block);
                block
            }
        };
        if block.get_attribute("style").as_deref() != Some(style.as_str()) {
            dom::set_attr(&block, "style", &style);
        }
    }
    // Blocks of strips no longer there.
    while let Some(block) = reuse {
        reuse = block.next_element_sibling();
        block.remove();
    }
}

/// The overview bar of the rows in `rows` (the page's `.rows`), for the row
/// `shown` (a phone's screen; none while the rail shows) and the pager's
/// sub-page `sub` (a switch redraws the miniature at once).
#[component]
pub fn OverviewView(
    rows: NodeRef<html::Div>,
    shown: Memo<Option<usize>>,
    sub: Memo<Option<usize>>,
) -> impl IntoView {
    let root = NodeRef::<html::Div>::new();
    // The pointer that drags the bar (one at a time: a second finger is
    // ignored while the first is down).
    let held = StoredValue::new(None::<i32>);
    let shown_row = move || {
        let index = shown.try_get_untracked().flatten()?;
        let rows_el = rows.try_get_untracked().flatten()?;
        row_at(&rows_el, index)
    };
    // Moves the shown row so its visible part centres where `ev` is.
    let jump = move |ev: &web_sys::PointerEvent| {
        let (Some(bar), Some(row)) = (dom::current_element(ev), shown_row()) else {
            return;
        };
        let rect = bar.get_bounding_client_rect();
        if rect.width() <= 0.0 {
            return;
        }
        let at = (f64::from(ev.client_x()) - rect.left()) / rect.width();
        let left = overview_scroll(
            at,
            f64::from(row.scroll_width()),
            f64::from(row.client_width()),
        );
        row.set_scroll_left(left.round() as i32);
    };
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        if held.get_value().is_some() {
            return;
        }
        if let Some(bar) = dom::current_element(&ev) {
            let _ = bar.set_pointer_capture(ev.pointer_id());
        }
        held.set_value(Some(ev.pointer_id()));
        jump(&ev);
    };
    let on_move = move |ev: web_sys::PointerEvent| {
        if held.get_value() == Some(ev.pointer_id()) {
            jump(&ev);
        }
    };
    let on_end = move |ev: web_sys::PointerEvent| {
        if held.get_value() == Some(ev.pointer_id()) {
            held.set_value(None);
        }
    };
    raf::animate(root, move |el| {
        let window = dom::child(&el, ".overview-window");
        let strips = dom::child(&el, ".overview-strips");
        let phone = web_sys::window().and_then(|w| w.match_media(PHONE).ok().flatten());
        let mut needed: Option<bool> = None;
        let mut drawn: Option<(f64, f64)> = None;
        let mut drawn_for: Option<(usize, Option<usize>)> = None;
        let mut next_read = f64::NEG_INFINITY;
        let mut next_redraw = f64::NEG_INFINITY;
        Box::new(move |now: f64, _step: f64| {
            // A tablet has no bar: nothing is read there.
            if !phone.as_ref().is_some_and(web_sys::MediaQueryList::matches) {
                return;
            }
            let dragging = held.try_get_value().flatten().is_some();
            if now < next_read && !dragging {
                return;
            }
            next_read = now + READ_MS;
            // Another row or sub-page: the miniature is drawn again at once.
            let key = shown
                .try_get_untracked()
                .flatten()
                .map(|index| (index, sub.try_get_untracked().flatten()));
            if drawn_for != key {
                drawn_for = key;
                next_redraw = f64::NEG_INFINITY;
            }
            // Only a row that scrolls (its own decision, `flow::overflows`)
            // and has a scroll range needs the bar.
            let row = shown_row().filter(|row| has_class(row, "scrolls"));
            let geometry = row.as_ref().and_then(|row| {
                let width = f64::from(row.scroll_width());
                overview_window(
                    f64::from(row.scroll_left()),
                    width,
                    f64::from(row.client_width()),
                )
                .map(|part| (part, width))
            });
            let is_needed = geometry.is_some();
            if needed != Some(is_needed) {
                dom::set_attr(&el, "data-needed", if is_needed { "true" } else { "false" });
                needed = Some(is_needed);
                drawn = None;
                next_redraw = f64::NEG_INFINITY;
            }
            let (Some(((left, part), width)), Some(row)) = (geometry, row) else {
                return;
            };
            if drawn != Some((left, part)) {
                if let Some(window) = &window {
                    dom::set_style(window, "left", &format!("{:.3}%", left * 100.0));
                    dom::set_style(window, "width", &format!("{:.3}%", part * 100.0));
                }
                drawn = Some((left, part));
            }
            if now >= next_redraw {
                next_redraw = now + REDRAW_MS;
                if let Some(strips) = &strips {
                    draw_strips(strips, &row, width);
                }
            }
        })
    });
    // A tap target: it owns its touches (#43 PR G) and writes no key.
    let no_keys: Vec<String> = Vec::new();
    view! {
        <div
            class="overview"
            data-testid="overview"
            data-needed="false"
            node_ref=root
            use:owns_touches=no_keys
            on:pointerdown=on_down
            on:pointermove=on_move
            on:pointerup=on_end
            on:pointercancel=on_end
            on:lostpointercapture=on_end
        >
            <div class="overview-strips"></div>
            <div class="overview-window"></div>
        </div>
    }
}
