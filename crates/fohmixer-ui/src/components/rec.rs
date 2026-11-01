//! The recording watch's keys (#172; `behave::rec`): a strip's ARM key above
//! SEL and the column foot's REC key. Both are one [`HoldKey`], a press as a
//! rail button's (`behave::rail::rail_press`, `Hold::lifted`): full at
//! `RAIL_HOLD_MS`, it acts at its lift; a tap, a slide, a cancel, a lost
//! capture or a hidden page do nothing, and a press whose end never came
//! ends at the next primary down of its type. Each step goes to the flight
//! recorder as a `rail` step (`button` `arm` or `rec`). A press that cannot
//! act (the page not connected, the hub's state not known on this
//! connection) shows no fill and flashes refused at its full mark.

use std::time::Duration;

use fohmixer_proto::client::RecState;
use fohmixer_proto::layout::Strip;
use leptos::prelude::*;
use serde_json::json;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::{down_of, fail_flash, key_of, owns_touches, trace_rail};
use crate::behave::hold::{Hold, Lift};
use crate::behave::rail::{
    Fill, RAIL_HOLD_MS, cancels, fill_of, full_step, hold_step, hold_value, rail_press, restarted,
    stale_press,
};
use crate::behave::rec::{ArmLook, arm_look, foot_look, record_mode, strip_arm};
use crate::binding::{SubSpec, arm_spec, record_spec};
use crate::diag::perf::Pointer;
use crate::dom;
use crate::store::LiveStore;

/// The page-wide `pointerdown` listener, kept to remove it in the cleanup.
type DownListener = Closure<dyn FnMut(web_sys::PointerEvent)>;

/// One key: `spec`, a flag of Live, a full press's lift writes the other
/// way. `current` reads that flag from the hub's state.
#[component]
fn HoldKey<F>(
    spec: SubSpec,
    current: F,
    button: &'static str,
    class: &'static str,
    testid: &'static str,
    label: &'static str,
    #[prop(into)] state: Signal<&'static str>,
    children: Children,
) -> impl IntoView
where
    F: Fn(Option<&RecState>) -> Option<bool> + Copy + Send + Sync + 'static,
{
    let store = expect_context::<LiveStore>();
    let touch_keys = vec![key_of(&spec)];
    let keys = StoredValue::new(touch_keys.clone());
    let spec = StoredValue::new(spec);
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
        fill_of(
            holding.try_get().unwrap_or(false),
            full.try_get().unwrap_or(false),
            store.rec.try_with(|r| current(r.fresh())).flatten(),
            store.connected.try_get().unwrap_or(false),
        )
    });
    // What a hold may write now: the flag the other way, or none (refused).
    let value = move || {
        let flag = store
            .rec
            .try_with_untracked(|r| current(r.fresh()))
            .flatten();
        hold_value(flag, store.connected.try_get_untracked().unwrap_or(false))
    };
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
        let wanted = value();
        step(hold_step(wanted), pointer);
        match wanted {
            Some(on) => {
                let _ = spec.try_with_value(|s| {
                    store.set(
                        &s.instance,
                        &s.target,
                        &s.prop,
                        json!(on),
                        true,
                        Some(fail_flash(failed)),
                    )
                });
            }
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
                    let wanted = value();
                    step(full_step(wanted), id);
                    if wanted.is_none() {
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

/// A strip's ARM key (#172): only on a track that can be armed in the
/// watched Live, a 6 px dot in the free column above SEL, its touch area
/// invisible.
#[component]
pub fn ArmKey(strip: Strip) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let spec = StoredValue::new(arm_spec(&strip.binding));
    let strip = StoredValue::new(strip);
    let look = Memo::new(move |_| {
        store
            .rec
            .try_with(|r| strip.try_with_value(|s| arm_look(r.shown(), s)))
            .flatten()
            .unwrap_or(ArmLook::None)
    });
    let armable = Memo::new(move |_| look.try_get().is_some_and(|l| l != ArmLook::None));
    let state = Signal::derive(move || look.try_get().unwrap_or(ArmLook::None).name());
    let current =
        move |rec: Option<&RecState>| strip.try_with_value(|s| strip_arm(rec, s)).flatten();
    move || {
        if !armable.try_get().unwrap_or(false) {
            return None;
        }
        spec.try_get_value().flatten().map(|spec| {
            view! {
                <HoldKey
                    spec=spec
                    current=current
                    button="arm"
                    class="strip-arm"
                    testid="arm-key"
                    label="Arm (hold)"
                    state=state
                >
                    <i class="rec-dot" aria-hidden="true"></i>
                </HoldKey>
            }
        })
    }
}

/// The column foot's REC key (#172): small dim text with the same dot, in
/// the status line's size.
#[component]
pub fn RecKey() -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let look = Memo::new(move |_| {
        store
            .rec
            .try_with(|r| foot_look(r.shown()))
            .unwrap_or_else(|| foot_look(None))
    });
    let state = Signal::derive(move || look.try_get().unwrap_or_else(|| foot_look(None)).0);
    let word = move || look.try_get().unwrap_or_else(|| foot_look(None)).1;
    view! {
        <HoldKey
            spec=record_spec()
            current=record_mode
            button="rec"
            class="rec-key"
            testid="rec-key"
            label="Record (hold)"
            state=state
        >
            <i class="rec-dot" aria-hidden="true"></i>
            <span class="rec-word">{word}</span>
        </HoldKey>
    }
}
