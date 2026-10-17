//! The rail buttons with an automatic mode (#104, #130, D18; the approved
//! mockup `docs/mockups/reverb-link-v1.html`, screen 2; the owner's ruling
//! on #104 of 2026-10-17): a lift before `RAIL_HOLD_MS` is the tap; at
//! `RAIL_HOLD_MS` the press is full (the fill stays full, the line says
//! `pusť — zapne AUT` / `pusť — vypne AUT`, nothing is sent) and the lift
//! then turns the automatic mode on or off. The press is
//! `behave::hold::Hold::lifted`: a slide past 10 px, a cancel, a lost
//! capture or a hidden page end it with nothing done, before or after the
//! full mark, so a lost end never acts; a second finger is no press, the
//! first goes on. A press whose end never came ends at the next primary
//! down of its type anywhere on the page (#124, review I1: a capture-phase
//! window listener). A press that cannot act (the page not connected, the
//! hub's flag not known on this connection) shows no fill, and at its full
//! mark it flashes refused and is dead (its lift does nothing); a full
//! press's lift that cannot act is refused the same way. The decisions are
//! `behave::rail`'s; each step goes to the flight recorder.

use std::time::Duration;

use fohmixer_proto::client::HUB_STAGE_AUT;
use fohmixer_proto::layout::Binding;
use leptos::html;
use leptos::prelude::*;
use serde_json::json;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::buttons::toggle_flag;
use super::{
    BtnText, down_of, fail_flash, intent_look, key_of, owns_touches, readiness, readiness_now,
    slot_of, trace_rail,
};
use crate::behave::hold::{Hold, Lift};
use crate::behave::rail::{
    Aut, Fill, RAIL_HOLD_MS, RailLook, SETTLE_MS, cancels, fill_of, full_step, hold_step,
    hold_value, rail_press, restarted, settling, stage_look, stale_press, transport_changed,
};
use crate::binding::{mute_sub, transport_sub};
use crate::diag::perf::Pointer;
use crate::dom;
use crate::store::{LiveStore, Readiness, Slot};

/// The rule a rail button carries.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleKind {
    /// The merged STAGE button: the stage mics' mute and STAGE AUT.
    Stage(Binding),
}

/// A page-wide `pointerdown` listener of a rail button (capture phase),
/// kept to remove it in the cleanup.
type DownListener = Closure<dyn FnMut(web_sys::PointerEvent)>;

/// A rail button with an automatic mode: the merged STAGE button (#104)
/// shows the stage mics' mute (lit while they are live, as `StageMicsView`)
/// and the hub's STAGE AUT flag with what the rule does now (its Live's
/// transport, `binding::transport_sub`).
#[component]
pub fn RuleButton(kind: RuleKind, label: String) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let RuleKind::Stage(binding) = kind;
    let spec = mute_sub(&binding);
    let slot = slot_of(store, spec.as_ref());
    let playing = store.slot(&transport_sub(&binding.instance));
    let keys: Vec<String> = spec.iter().map(key_of).collect();
    let root = NodeRef::<html::Div>::new();
    intent_look(root, store, keys.clone());
    // The root owns its touches (#43 PR G) and names its keys.
    let touch_keys = keys.clone();
    let keys = StoredValue::new(keys);
    let spec = StoredValue::new(spec);
    let failed = RwSignal::new(false);
    let hold = StoredValue::new(rail_press());
    // The press's pointer type (#124: only a primary down of its type ends
    // a press whose end never came).
    let press_kind = StoredValue::new(None::<Pointer>);
    let check = StoredValue::new(None::<TimeoutHandle>);
    // The press's state as shown: filling (before its hold), full (past it).
    let holding = RwSignal::new(false);
    let full = RwSignal::new(false);
    // The transport's last change not yet settled (the page clock), its
    // value before, and the one look check after it (review M3).
    let changed_at = RwSignal::new(None::<f64>);
    let transport_before = StoredValue::new(None::<bool>);
    let settle_check = StoredValue::new(None::<TimeoutHandle>);
    let any_down = StoredValue::new_local(None::<DownListener>);
    // The timers and the page's listener go with the button (the cleanup
    // writes no signal of its own).
    on_cleanup(move || {
        for timer in [check, settle_check] {
            if let Some(Some(handle)) = timer.try_update_value(Option::take) {
                handle.clear();
            }
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
    // Each change of the transport: when it came, and one look check once
    // it has settled.
    Effect::new(move |_| {
        let now_playing = playing.try_with(Slot::flag).flatten();
        let before = transport_before
            .try_update_value(|b| std::mem::replace(b, now_playing))
            .flatten();
        if !transport_changed(before, now_playing) {
            return;
        }
        let _ = changed_at.try_set(Some(dom::now()));
        if let Some(Some(handle)) = settle_check.try_update_value(Option::take) {
            handle.clear();
        }
        let timer = set_timeout_with_handle(
            move || {
                let _ = settle_check.try_set_value(None);
                let _ = changed_at.try_set(None);
            },
            Duration::from_millis(SETTLE_MS as u64 + 20),
        );
        let _ = settle_check.try_set_value(timer.ok());
    });
    // STAGE AUT as shown (kept stale through an outage, #104 M4) and as a
    // hold may act on it (this connection's).
    let aut = move || store.hub.try_with(|h| h.flag(HUB_STAGE_AUT)).flatten();
    let fresh_aut = move || {
        store
            .hub
            .try_with(|h| h.fresh_flag(HUB_STAGE_AUT))
            .flatten()
    };
    let connected = move || store.connected.try_get().unwrap_or(false);
    // The fill and the hold's lines: only for a press that can act.
    let fill = Memo::new(move |_| {
        fill_of(
            holding.try_get().unwrap_or(false),
            full.try_get().unwrap_or(false),
            fresh_aut(),
            connected(),
        )
    });
    let look = Memo::new(move |_| {
        stage_look(
            aut(),
            playing.try_with(Slot::flag).flatten(),
            slot.try_with(Slot::flag).flatten(),
            fill.try_get().unwrap_or(Fill::None),
            settling(changed_at.try_get().flatten(), dom::now()),
        )
    });
    // The tap: the stage mics by hand, as STAGE did (I8: Live's value here).
    let tap = move || {
        if readiness_now(&[slot]) != Readiness::Ready {
            return;
        }
        let _ = spec.try_with_value(|s| {
            if let Some(s) = s {
                toggle_flag(store, s, slot, failed);
            }
        });
    };
    // The flight recorder hears each step with the stage mics' key.
    let step = move |what: &str, pointer: i32| {
        let _ = keys.try_with_value(|k| trace_rail(what, "stage", k, pointer));
    };
    // What a hold may send now: STAGE AUT the other way, or none (refused).
    let can_send = move || {
        let aut = store
            .hub
            .try_with_untracked(|h| h.fresh_flag(HUB_STAGE_AUT))
            .flatten();
        let connected = store.connected.try_get_untracked().unwrap_or(false);
        hold_value(aut, connected)
    };
    let refuse = move || fail_flash(failed)("STAGE AUT cannot be switched now".to_string());
    // The lift of a full press: STAGE AUT the other way when it can act,
    // else refused.
    let held = move |pointer: i32| {
        let value = can_send();
        step(hold_step(value), pointer);
        match value {
            Some(on) => store.set_hub(HUB_STAGE_AUT, json!(on)),
            None => refuse(),
        }
    };
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
    // A press whose end never came ends at the next primary down anywhere
    // on the page, before any control's handler sees it (#124, review I1).
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
    // A hidden page ends a press with nothing done. `visibilitychange`
    // bubbles from the document to the window.
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
        // A down of another type took over a press still down (N2).
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
        // One check a little after the hold (a coarse clock never finds the
        // press a hair short of it): the press is full, nothing is sent; one
        // that cannot act is refused and dead.
        let timer = set_timeout_with_handle(
            move || {
                let _ = check.try_set_value(None);
                let fills = hold
                    .try_update_value(|h| h.open(dom::now()))
                    .unwrap_or(false);
                if fills {
                    let value = can_send();
                    step(full_step(value), id);
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
    // A finger that slides off the press ends it (`Hold::moved`).
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
            Lift::Tap => {
                step("tap", id);
                tap();
            }
            Lift::Open => held(id),
            Lift::Nothing => {}
        }
    };
    // A cancelled pointer, or one whose capture was lost: nothing done.
    let on_cancel = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        if cancels(hold.try_with_value(Hold::pressing).flatten(), id) {
            step("cancel", id);
        }
        let _ = hold.try_update_value(|h| h.cancel(id));
        settle();
    };
    let state = Memo::new(move |_| readiness(&[slot]));
    let bound = move || state.try_get().map_or("waiting", |s| s.name());
    let disabled = move || state.try_get().map_or("true", |s| s.disabled());
    let lit = move || look.try_with(|l| l.lit).unwrap_or(false);
    let pill = move || look.try_with(|l| l.aut.name()).unwrap_or("off");
    let low = move || look.try_with(|l| l.low).unwrap_or(false);
    let sub = move || look.try_with(|l: &RailLook| l.sub.clone()).flatten();
    // The pill is in the page only while the rule is on: no hidden text in
    // a rail button (its words are checked to stay inside it).
    let pill_on = Memo::new(move |_| look.try_with(|l| l.aut != Aut::Off).unwrap_or(false));
    let badge = move || {
        pill_on.try_get().unwrap_or(false).then(|| {
            view! {
                <span class="aut-badge" data-testid="aut-badge">
                    "AUT"
                </span>
            }
        })
    };
    let muted = move || (slot.try_with(Slot::flag).flatten() == Some(true)).to_string();
    let filling = move || fill.try_get().is_some_and(Fill::holding).to_string();
    let is_full = move || fill.try_get().is_some_and(Fill::full).to_string();
    // The fill's time is the hold's (rule.css reads it).
    let hold_time = format!("--hold-ms:{RAIL_HOLD_MS}ms;");
    view! {
        <div
            class="btn stage-mics rule-btn"
            node_ref=root
            use:owns_touches=touch_keys
            class:on=lit
            class:failed=move || failed.try_get().unwrap_or(false)
            style=hold_time
            data-testid="stage-mics"
            data-muted=muted
            data-aut=pill
            data-holding=filling
            data-full=is_full
            data-binding=bound
            aria-disabled=disabled
            on:pointerdown=on_down
            on:pointermove=on_move
            on:pointerup=on_up
            on:pointercancel=on_cancel
            on:lostpointercapture=on_cancel
        >
            <span class="btn-row">
                <BtnText text=label />
                {badge}
            </span>
            <span class="btn-sub" data-testid="btn-sub" class:low=low>
                {sub}
            </span>
            <span class="hold" aria-hidden="true"></span>
        </div>
    }
}
