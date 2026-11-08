//! One key acted on by a 1 s hold (#172 ARM and REC, #90 Presenter's stage
//! layouts): a press as a rail button's (`behave::rail::rail_press`,
//! `Hold::lifted`): full at `RAIL_HOLD_MS`, it acts at its lift; a tap, a
//! slide, a cancel, a lost capture or a hidden page do nothing, and a press
//! whose end never came ends at the next primary down of its type anywhere
//! on the page. Each step goes to the flight recorder as a `rail` step with
//! the key's `button` and keys. A press that cannot act (`can` false: the
//! fill shows only for a key that can act; `wanted` none at the full mark or
//! the lift) shows no fill and flashes refused at its full mark.

use std::time::Duration;

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::{down_of, fail_flash, owns_touches, trace_rail};
use crate::behave::hold::{Hold, Lift};
use crate::behave::rail::{
    Fill, RAIL_HOLD_MS, cancels, full_step, hold_step, press_fill, rail_press, restarted,
    stale_press,
};
use crate::diag::perf::Pointer;
use crate::dom;

/// The page-wide `pointerdown` listener, kept to remove it in the cleanup.
type DownListener = Closure<dyn FnMut(web_sys::PointerEvent)>;

/// The failure flash a key's action hands its write.
pub type Flash = Box<dyn FnOnce(String)>;

/// One key: `wanted` is what a full press's lift acts with now (none: it
/// is refused), read at the full mark and at the lift; `can` whether a press
/// shows its fill; `act` the lift's action, handed the key's failure flash.
/// `name` goes to the root's `data-name`.
#[component]
pub fn HoldKey<V, W, A>(
    keys: Vec<String>,
    wanted: W,
    #[prop(into)] can: Signal<bool>,
    act: A,
    button: &'static str,
    class: &'static str,
    testid: &'static str,
    #[prop(into)] label: String,
    #[prop(into)] state: Signal<&'static str>,
    #[prop(optional)] name: Option<String>,
    children: Children,
) -> impl IntoView
where
    V: 'static,
    W: Fn() -> Option<V> + Copy + Send + Sync + 'static,
    A: Fn(V, Flash) + Copy + Send + Sync + 'static,
{
    let touch_keys = keys.clone();
    let keys = StoredValue::new(keys);
    let hold = StoredValue::new(rail_press());
    let press_kind = StoredValue::new(None::<Pointer>);
    let check = StoredValue::new(None::<TimeoutHandle>);
    let any_down = StoredValue::new_local(None::<DownListener>);
    let holding = RwSignal::new(false);
    let full = RwSignal::new(false);
    let failed = RwSignal::new(false);
    on_cleanup(move || {
        if let Some(Some(handle)) = check.try_update_value(Option::take) {
            handle.clear();
        }
        if let (Some(Some(listener)), Some(window)) =
            (any_down.try_update_value(Option::take), web_sys::window())
        {
            let _ = window.remove_event_listener_with_callback_and_bool(
                "pointerdown",
                listener.as_ref().unchecked_ref(),
                true,
            );
        }
    });
    let fill = Memo::new(move |_| {
        press_fill(
            holding.try_get().unwrap_or(false),
            full.try_get().unwrap_or(false),
            can.try_get().unwrap_or(false),
        )
    });
    let step = move |what: &str, pointer: i32| {
        let _ = keys.try_with_value(|k| trace_rail(what, button, k, pointer));
    };
    let refuse = move || fail_flash(failed)(format!("{button} cannot be switched now"));
    // The press's state follows it; a press no longer filling waits for no
    // check.
    let settle = move || {
        let filling = hold.try_with_value(Hold::holding).unwrap_or(false);
        let is_full = hold.try_with_value(Hold::full).unwrap_or(false);
        if !filling && let Some(Some(handle)) = check.try_update_value(Option::take) {
            handle.clear();
        }
        let _ = holding.try_set(filling);
        let _ = full.try_set(is_full);
    };
    let held = move |pointer: i32| {
        let value = wanted();
        step(hold_step(value.as_ref().map(|_| true)), pointer);
        match value {
            Some(value) => act(value, fail_flash(failed)),
            None => refuse(),
        }
    };
    // A press whose end never came ends at the next primary down anywhere
    // on the page, before any control's handler sees it (#124).
    let listener = Closure::wrap(Box::new(move |ev: web_sys::PointerEvent| {
        let pressing = hold.try_with_value(Hold::pressing).flatten();
        let kind = press_kind.try_get_value().flatten();
        if let Some(old) = stale_press(pressing, kind, down_of(&ev)) {
            let _ = hold.try_update_value(Hold::leave);
            step("lost", old);
            settle();
        }
    }) as Box<dyn FnMut(web_sys::PointerEvent)>);
    if let Some(window) = web_sys::window() {
        let _ = window.add_event_listener_with_callback_and_bool(
            "pointerdown",
            listener.as_ref().unchecked_ref(),
            true,
        );
    }
    let _ = any_down.try_set_value(Some(listener));
    // A hidden page ends a press with nothing done.
    let hidden = window_event_listener_untyped("visibilitychange", move |_| {
        if dom::hidden()
            && let Some(Some(pointer)) = hold.try_update_value(Hold::leave)
        {
            step("lost", pointer);
            settle();
        }
    });
    on_cleanup(move || hidden.remove());
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let id = ev.pointer_id();
        let primary = ev.is_primary();
        let before = hold.try_with_value(Hold::pressing).flatten();
        let pressed = hold
            .try_update_value(|h| h.down(id, primary, dom::now()))
            .unwrap_or(false);
        if !pressed {
            return;
        }
        if let Some(old) = restarted(before, id) {
            step("lost", old);
        }
        let _ = press_kind.try_set_value(Pointer::parse(&ev.pointer_type()));
        step("press", id);
        if let Some(el) = dom::current_element(&ev) {
            let _ = el.set_pointer_capture(id);
        }
        if let Some(Some(handle)) = check.try_update_value(Option::take) {
            handle.clear();
        }
        // One check a little after the hold: the press is full, nothing is
        // sent; one that cannot act is refused and dead.
        let timer = set_timeout_with_handle(
            move || {
                let _ = check.try_set_value(None);
                let fills = hold
                    .try_update_value(|h| h.open(dom::now()))
                    .unwrap_or(false);
                if fills {
                    let value = wanted();
                    step(full_step(value.as_ref().map(|_| true)), id);
                    if value.is_none() {
                        let _ = hold.try_update_value(Hold::refuse);
                        refuse();
                    }
                }
                settle();
            },
            Duration::from_millis(RAIL_HOLD_MS as u64 + 20),
        );
        let _ = check.try_set_value(timer.ok());
        settle();
    };
    let on_move = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
        let ended = hold
            .try_update_value(|h| h.moved(id, x, y))
            .unwrap_or(false);
        if ended {
            step("slid", id);
            settle();
        }
    };
    let on_up = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        let lift = hold
            .try_update_value(|h| h.up(id, dom::now()))
            .unwrap_or(Lift::Nothing);
        settle();
        match lift {
            // A tap does nothing.
            Lift::Tap => step("tap", id),
            Lift::Open => held(id),
            Lift::Nothing => {}
        }
    };
    let on_cancel = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        if cancels(hold.try_with_value(Hold::pressing).flatten(), id) {
            step("cancel", id);
        }
        let _ = hold.try_update_value(|h| h.cancel(id));
        settle();
    };
    let filling = move || fill.try_get().is_some_and(Fill::holding).to_string();
    let is_full = move || fill.try_get().is_some_and(Fill::full).to_string();
    let shown = move || state.try_get().unwrap_or("off");
    view! {
        <button
            type="button"
            class=class
            use:owns_touches=touch_keys
            class:failed=move || failed.try_get().unwrap_or(false)
            data-testid=testid
            data-name=name
            aria-label=label
            data-state=shown
            data-holding=filling
            data-full=is_full
            on:pointerdown=on_down
            on:pointermove=on_move
            on:pointerup=on_up
            on:pointercancel=on_cancel
            on:lostpointercapture=on_cancel
        >
            {children()}
        </button>
    }
}
