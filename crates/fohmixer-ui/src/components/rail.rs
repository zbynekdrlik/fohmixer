//! The rail buttons with an automatic mode (#104, #130, D18; the approved
//! mockup `docs/mockups/reverb-link-v1.html`, screen 2): a tap acts on the
//! finger's lift; a 3 s hold turns the automatic mode on or off (the press
//! is `behave::hold::Hold` with `RAIL_HOLD_MS`: a slide past 10 px, a
//! cancel, a lost capture, a hidden page or a second finger end it with
//! nothing done; a primary down ends a press whose end never came, #124).
//! The look is `behave::rail`'s. Each step goes to the flight recorder.

use std::time::Duration;

use fohmixer_proto::client::HUB_STAGE_AUT;
use fohmixer_proto::layout::Binding;
use leptos::html;
use leptos::prelude::*;
use serde_json::{Value, json};

use super::buttons::toggle_flag;
use super::{
    BtnText, intent_look, key_of, owns_touches, readiness, readiness_now, slot_of, trace_rail,
};
use crate::behave::hold::{Hold, Lift};
use crate::behave::rail::{RAIL_HOLD_MS, RailLook, rail_press, stage_look};
use crate::binding::{mute_sub, transport_sub};
use crate::dom;
use crate::store::{LiveStore, Readiness, Slot};

/// The rule a rail button carries.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleKind {
    /// The merged STAGE button: the stage mics' mute and STAGE AUT.
    Stage(Binding),
}

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
    let check = StoredValue::new(None::<TimeoutHandle>);
    let holding = RwSignal::new(false);
    // The timer goes with the button (the cleanup writes no signal of its
    // own).
    on_cleanup(move || {
        if let Some(Some(handle)) = check.try_update_value(Option::take) {
            handle.clear();
        }
    });
    let aut = move || {
        store
            .hub
            .try_with(|h| h.get(HUB_STAGE_AUT).and_then(Value::as_bool))
            .flatten()
    };
    let look = Memo::new(move |_| {
        stage_look(
            aut(),
            playing.try_with(Slot::flag).flatten(),
            slot.try_with(Slot::flag).flatten(),
            holding.try_get().unwrap_or(false),
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
    // The hold: STAGE AUT the other way, once the hub's flag is known.
    let held = move || {
        let current = store
            .hub
            .try_with_untracked(|h| h.get(HUB_STAGE_AUT).and_then(Value::as_bool))
            .flatten();
        let connected = store.connected.try_get_untracked().unwrap_or(false);
        if let (Some(current), true) = (current, connected) {
            store.set_hub(HUB_STAGE_AUT, json!(!current));
        }
    };
    // The flight recorder hears each step with the stage mics' key.
    let step = move |what: &str, pointer: i32| {
        let _ = keys.try_with_value(|k| trace_rail(what, "stage", k, pointer));
    };
    // The fill follows the press; a press over waits for no check.
    let settle = move || {
        let pressing = hold.try_with_value(Hold::holding).unwrap_or(false);
        if !pressing && let Some(Some(handle)) = check.try_update_value(Option::take) {
            handle.clear();
        }
        let _ = holding.try_set(pressing);
    };
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
        // A primary down ends a press whose end never came (#124).
        if let Some(old) = before.filter(|old| *old != id) {
            step("lost", old);
        }
        step("press", id);
        if let Some(el) = dom::current_element(&ev) {
            let _ = el.set_pointer_capture(id);
        }
        if let Some(Some(handle)) = check.try_update_value(Option::take) {
            handle.clear();
        }
        // One check a little after the hold (a coarse clock never finds the
        // press a hair short of it).
        let timer = set_timeout_with_handle(
            move || {
                let _ = check.try_set_value(None);
                let holds = hold
                    .try_update_value(|h| h.open(dom::now()))
                    .unwrap_or(false);
                settle();
                if holds {
                    step("hold", id);
                    held();
                }
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
            Lift::Open => {
                step("hold", id);
                held();
            }
            Lift::Nothing => {}
        }
    };
    // A cancelled pointer, or one whose capture was lost: nothing done.
    let on_cancel = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        if hold.try_with_value(Hold::pressing).flatten() == Some(id) {
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
    let muted = move || (slot.try_with(Slot::flag).flatten() == Some(true)).to_string();
    view! {
        <div
            class="btn stage-mics rule-btn"
            node_ref=root
            use:owns_touches=touch_keys
            class:on=lit
            class:failed=move || failed.try_get().unwrap_or(false)
            data-testid="stage-mics"
            data-muted=muted
            data-aut=pill
            data-holding=move || holding.try_get().unwrap_or(false).to_string()
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
                <span class="aut-badge" data-testid="aut-badge">
                    "AUT"
                </span>
            </span>
            <span class="btn-sub" data-testid="btn-sub" class:low=low>
                {sub}
            </span>
            <span class="hold" aria-hidden="true"></span>
        </div>
    }
}
