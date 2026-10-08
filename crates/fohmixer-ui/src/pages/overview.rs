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

use crate::components::owns_touches;
use crate::dom;
use crate::flow::{overview_scroll, overview_window};
use crate::raf;

/// How often the miniature follows the strips (their places and colours),
/// in ms: a strip's colour or mute changes rarely, its place only with the
/// layout.
const REDRAW_MS: f64 = 500.0;

/// The row a phone shows among `rows`.
fn shown_row(rows: &web_sys::Element) -> Option<web_sys::HtmlElement> {
    dom::child(rows, r#".row[data-shown="true"]"#)
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
    if !mute.class_name().split_whitespace().any(|c| c == "lit") {
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

/// The overview bar of the rows in `rows` (the page's `.rows`).
#[component]
pub fn OverviewView(rows: NodeRef<html::Div>) -> impl IntoView {
    let root = NodeRef::<html::Div>::new();
    // The pointer that drags the bar.
    let held = StoredValue::new(None::<i32>);
    // Moves the shown row so its visible part centres where `ev` is.
    let jump = move |ev: &web_sys::PointerEvent| {
        let Some(bar) = dom::current_element(ev) else {
            return;
        };
        let Some(rows_el) = rows.try_get_untracked().flatten() else {
            return;
        };
        let Some(row) = shown_row(&rows_el) else {
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
        let mut needed: Option<bool> = None;
        let mut drawn: Option<(f64, f64)> = None;
        let mut next_redraw = f64::NEG_INFINITY;
        Box::new(move |now: f64, _step: f64| {
            // The tablet has no bar (the stylesheet shows it on a phone only).
            if el.client_width() == 0 {
                return;
            }
            let row = rows
                .try_get_untracked()
                .flatten()
                .and_then(|rows_el| shown_row(&rows_el));
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
