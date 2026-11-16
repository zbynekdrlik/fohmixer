//! Listen on the surface (#210 phase 2; the owner's approved mockup
//! `docs/mockups/listen-v1.html`): the channel detail's 🎧 key beside the
//! channel's chip (a tap at its lift starts listening to that channel,
//! before its fader; a tap again stops), the slim volume beside it while it
//! listens, and the column foot's calm line `listening <name>`. The
//! decisions are `behave::listen`, the socket and the player
//! `crate::listen::Listener`; `listen.css` the look.

use fohmixer_proto::layout::Strip;
use leptos::prelude::*;

use super::{owns_touches, trace_touch};
use crate::behave::listen::{foot_text, inside_box, lift_taps, volume_at};
use crate::listen::Listener;
use crate::store::LiveStore;

/// The detail's 🎧 key and, while it listens, its volume. Mounting it with
/// a strip follows that channel while the page listens (the mockup: picking
/// another channel keeps listening).
#[component]
pub fn ListenKey(strip: Strip) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let listener = use_context::<Listener>();
    if let Some(listener) = listener {
        listener.follow(store, &strip);
    }
    let strip = StoredValue::new(strip);
    let view = listener.map(|l| l.view);
    let lit = move || view.and_then(|v| v.try_with(|v| v.lit)).unwrap_or(false);
    let state = move || {
        view.and_then(|v| v.try_with(|v| v.phase.name()))
            .unwrap_or("off")
    };
    // A tap at its lift: a browser lets a page's audio start only in a
    // user's gesture, and on a touch screen the lift is one (the down is
    // not); only the lift of the finger that went down on the key
    // (`behave::listen::lift_taps`).
    let down = StoredValue::new(None::<i32>);
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = down.try_set_value(Some(ev.pointer_id()));
    };
    let on_cancel = move |_: web_sys::PointerEvent| {
        let _ = down.try_set_value(None);
    };
    let on_up = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let pressed = down.try_update_value(Option::take).flatten();
        let inside = ev
            .current_target()
            .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
            .is_some_and(|el| {
                let r = el.get_bounding_client_rect();
                inside_box(
                    f64::from(ev.client_x()),
                    f64::from(ev.client_y()),
                    (r.left(), r.top(), r.right(), r.bottom()),
                )
            });
        if !lift_taps(pressed, ev.pointer_id(), inside) {
            return;
        }
        let Some(listener) = listener else {
            return;
        };
        trace_touch("tap", &[], ev.pointer_id());
        let _ = strip.try_with_value(|s| listener.tap(store, s));
    };
    let no_keys: Vec<String> = Vec::new();
    view! {
        <button
            type="button"
            class="listen-key"
            use:owns_touches=no_keys
            data-testid="listen-key"
            data-state=state
            aria-pressed=move || lit().to_string()
            aria-label="Listen"
            on:pointerdown=on_down
            on:pointercancel=on_cancel
            on:pointerup=on_up
        >
            "🎧"
        </button>
        <Show when=lit>
            <ListenVolume />
        </Show>
    }
}

/// The slim volume: a finger anywhere on it sets the place under it and
/// drags it; nothing writes Live.
#[component]
fn ListenVolume() -> impl IntoView {
    let listener = expect_context::<Listener>();
    let held = StoredValue::new(None::<i32>);
    let place = move |ev: &web_sys::PointerEvent| {
        let Some(el) = ev
            .current_target()
            .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
        else {
            return;
        };
        let rect = el.get_bounding_client_rect();
        listener.set_volume(volume_at(
            f64::from(ev.client_x()),
            rect.left(),
            rect.width(),
        ));
    };
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        // A new finger takes the volume over: a finger whose end never
        // came (iOS took it, #124) never holds it.
        let _ = held.try_set_value(Some(ev.pointer_id()));
        if let Some(el) = ev
            .current_target()
            .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
        {
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        place(&ev);
    };
    let on_move = move |ev: web_sys::PointerEvent| {
        if held.try_get_value().flatten() == Some(ev.pointer_id()) {
            place(&ev);
        }
    };
    let on_end = move |ev: web_sys::PointerEvent| {
        if held.try_get_value().flatten() == Some(ev.pointer_id()) {
            let _ = held.try_set_value(None);
        }
    };
    let pos = move || listener.volume.try_get().unwrap_or(0.0);
    let fill = move || format!("--v:{:.4};", pos());
    let percent = move || format!("{:.0}", pos() * 100.0);
    let no_keys: Vec<String> = Vec::new();
    view! {
        <div
            class="listen-volume"
            use:owns_touches=no_keys
            data-testid="listen-volume"
            data-value=percent
            style=fill
            on:pointerdown=on_down
            on:pointermove=on_move
            on:pointerup=on_end
            on:pointercancel=on_end
            on:lostpointercapture=on_end
        >
            <span class="listen-volume-track"></span>
        </div>
    }
}

/// The column foot's line while the page listens (or why it stopped).
#[component]
pub fn ListenLine() -> impl IntoView {
    let listener = use_context::<Listener>();
    move || {
        listener
            .and_then(|l| {
                l.view
                    .try_with(|v| foot_text(v.phase, v.label.as_deref()))
                    .flatten()
            })
            .map(|text| {
                view! {
                    <div class="listen-line" data-testid="listen-line">
                        {text}
                    </div>
                }
            })
    }
}
