//! The rail buttons with an automatic mode (#104, #130, D18, D19; the
//! approved mockups `docs/mockups/reverb-link-v1.html`, screen 2,
//! `docs/mockups/reverb-link-v2.html`, screen 3, and
//! `docs/mockups/autotune-v2.html` (#140 PR 3c); the owner's rulings on
//! #104 of 2026-10-17): the merged STAGE button, REVERB and AUTOTUNE. A
//! lift before `RAIL_HOLD_MS` (1 s, not the mockup's 3 s) is the tap
//! (STAGE: the stage mics by hand; REVERB: the reverb faders 10 dB down or
//! back, asked of the hub; AUTOTUNE: the singers' panel opened or closed,
//! nothing sent, `components::autotune_panel`); at 1 s the press is full (the fill stays full, the line says
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
//! press's lift that cannot act is refused the same way, and so is a REVERB
//! tap or an AUTOTUNE tap that would open the panel (the hub's state of the
//! rule not known on this connection, or the page not connected). The
//! panel closes at a tap on AUTOTUNE again or a down anywhere outside it
//! (the button's page listener). The decisions are
//! `behave::rail`'s, `behave::reverb`'s and `behave::autotune`'s; each step
//! goes to the flight recorder.

use std::time::Duration;

use fohmixer_proto::client::{HUB_REVERB_LOW, HUB_STAGE_AUT};
use fohmixer_proto::layout::Binding;
use leptos::html;
use leptos::prelude::*;
use serde_json::json;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::autotune_panel::AutotunePanel;
use super::buttons::toggle_flag;
use super::{
    BtnText, down_of, fail_flash, intent_look, key_of, owns_touches, readiness, readiness_now,
    slot_of, trace_rail,
};
use crate::behave::autotune::{
    Anchor, autotune_look, autotune_ready, dismisses, has_problem, state_name, swallows_up,
    tap_opens_panel,
};
use crate::behave::hold::{Hold, Lift};
use crate::behave::rail::{
    AUT_BADGE, Aut, Fill, RAIL_HOLD_MS, RailLook, Rule, SETTLE_MS, cancels, fill_of, full_step,
    hold_step, hold_value, rail_press, ready_hold, restarted, settling, stage_look, stale_press,
    transport_changed,
};
use crate::behave::reverb::{reverb_look, reverb_ready, tap_step, tap_value};
use crate::binding::{mute_sub, transport_sub};
use crate::diag::perf::Pointer;
use crate::dom;
use crate::store::{LiveStore, Readiness, Slot};

/// The rule a rail button carries.
#[derive(Debug, Clone, PartialEq)]
pub enum RuleKind {
    /// The merged STAGE button: the stage mics' mute and STAGE AUT.
    Stage(Binding),
    /// REVERB (#130): the reverb faders lowered or back, and REVERB AUT;
    /// the hub moves the faders, the button writes no key of its own.
    Reverb,
    /// AUTOTUNE (#104, D19; #140 PR 3c): AUTOTUNE AUT and the singers'
    /// panel; the hub writes the racks, the button no key.
    Autotune,
}

/// A page-wide pointer listener of a rail button (capture phase: its
/// `pointerdown`, and AUTOTUNE's `pointerup` / `pointercancel`), kept to
/// remove it in the cleanup.
type DownListener = Closure<dyn FnMut(web_sys::PointerEvent)>;

/// The window events whose capture-phase listener takes the up of a down
/// that closed AUTOTUNE's panel (PR #150 re-review n2).
const UP_EVENTS: [&str; 2] = ["pointerup", "pointercancel"];

/// A rail button with an automatic mode: the merged STAGE button (#104)
/// shows the stage mics' mute (lit while they are live, as `StageMicsView`)
/// and the hub's STAGE AUT flag with what the rule does now (its Live's
/// transport, `binding::transport_sub`); REVERB (#130) shows the hub's
/// REVERB state (lit at the song level, dark while lowered) and what
/// REVERB AUT does now (`behave::reverb::reverb_look`); AUTOTUNE (#140 PR
/// 3c, mockup v2) shows the hub's AUTOTUNE state (lit while every singer
/// tunes, mixed, dark), AUT, the state and the key
/// (`behave::autotune::autotune_look`), and opens the singers' panel.
#[component]
pub fn RuleButton(kind: RuleKind, label: String) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let rule = match &kind {
        RuleKind::Stage(_) => Rule::Stage,
        RuleKind::Reverb => Rule::Reverb,
        RuleKind::Autotune => Rule::Autotune,
    };
    // STAGE writes the stage mics' mute and reads its Live's transport;
    // REVERB and AUTOTUNE write nothing themselves (the hub does).
    let stage = match kind {
        RuleKind::Stage(binding) => Some(binding),
        RuleKind::Reverb | RuleKind::Autotune => None,
    };
    let is_stage = stage.is_some();
    let button = rule.button();
    let aut_key = rule.aut_key();
    let spec = stage.as_ref().and_then(mute_sub);
    let slot = stage.as_ref().map(|_| slot_of(store, spec.as_ref()));
    let playing = stage
        .as_ref()
        .map(|b| store.slot(&transport_sub(&b.instance)));
    let keys: Vec<String> = spec.iter().map(key_of).collect();
    let root = NodeRef::<html::Div>::new();
    // A toggle's open writes outlined (#43 PR C): STAGE's mute; REVERB
    // writes none.
    if is_stage {
        intent_look(root, store, keys.clone());
    }
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
    // STAGE: the transport's last change not yet settled (the page clock),
    // its value before, and the one look check after it (review M3).
    let changed_at = RwSignal::new(None::<f64>);
    let transport_before = StoredValue::new(None::<bool>);
    let settle_check = StoredValue::new(None::<TimeoutHandle>);
    let any_down = StoredValue::new_local(None::<DownListener>);
    let any_up = StoredValue::new_local(None::<DownListener>);
    // The pointer whose down closed AUTOTUNE's panel: its up is taken too
    // (PR #150 re-review n2: AbleSet's sign-in acts on its up alone).
    let dismissed = StoredValue::new(None::<i32>);
    // AUTOTUNE's panel: open with where it opens from (the column's right
    // edge, the button's top and bottom on the screen), or closed.
    let open = RwSignal::new(None::<Anchor>);
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
        if let (Some(Some(listener)), Some(window)) =
            (any_up.try_update_value(Option::take), web_sys::window())
        {
            for name in UP_EVENTS {
                let _ = window.remove_event_listener_with_callback_and_bool(
                    name,
                    listener.as_ref().unchecked_ref(),
                    true,
                );
            }
        }
    });
    // The transport STAGE reads (its Live's); REVERB and AUTOTUNE none
    // (their states say it).
    let transport = move || match (rule, playing) {
        (Rule::Stage, Some(playing)) => playing.try_with(Slot::flag).flatten(),
        _ => None,
    };
    // Each change of that transport: when it came, and one look check once
    // it has settled.
    if rule == Rule::Stage {
        Effect::new(move |_| {
            let now_playing = transport();
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
    }
    // STAGE AUT as shown (kept stale through an outage, #104 M4; REVERB's
    // and AUTOTUNE's are in the hub's last state of the rule) and the
    // automatic mode as a hold may act on it (this connection's).
    let aut = move || store.hub.try_with(|h| h.flag(HUB_STAGE_AUT)).flatten();
    let fresh_aut = move || match rule {
        Rule::Stage => store
            .hub
            .try_with(|h| h.fresh_flag(HUB_STAGE_AUT))
            .flatten(),
        Rule::Reverb => store
            .reverb
            .try_with(|r| r.fresh().map(|s| s.aut))
            .flatten(),
        Rule::Autotune => store
            .autotune
            .try_with(|a| a.fresh().map(|s| s.aut))
            .flatten(),
    };
    let connected = move || store.connected.try_get().unwrap_or(false);
    // STAGE: its mics' readiness; REVERB and AUTOTUNE: waiting before the
    // hub's state, red (I5) without a reverb fader or a found target.
    let state = Memo::new(move |_| {
        let ready = match (rule, slot) {
            (_, Some(slot)) => return readiness(&[slot]),
            (Rule::Autotune, None) => store
                .autotune
                .try_with(|a| autotune_ready(a.shown()))
                .flatten(),
            (_, None) => store.reverb.try_with(|r| reverb_ready(r.shown())).flatten(),
        };
        match ready {
            None => Readiness::Waiting,
            Some(false) => Readiness::Unresolved,
            Some(true) => Readiness::Ready,
        }
    });
    // REVERB and AUTOTUNE hold only while ready: a red or waiting button's
    // hold does nothing (PR #150 review M2, the mockup's cell 13); STAGE's
    // flag needs no mics.
    let ready = move |state: Option<Readiness>| is_stage || state == Some(Readiness::Ready);
    // The fill and the hold's lines: only for a press that can act.
    let fill = Memo::new(move |_| {
        fill_of(
            holding.try_get().unwrap_or(false),
            full.try_get().unwrap_or(false),
            ready_hold(fresh_aut(), ready(state.try_get())),
            connected(),
        )
    });
    let look = Memo::new(move |_| {
        let fill = fill.try_get().unwrap_or(Fill::None);
        let unsettled = settling(changed_at.try_get().flatten(), dom::now());
        match (rule, slot, playing) {
            (Rule::Stage, Some(slot), Some(playing)) => stage_look(
                aut(),
                playing.try_with(Slot::flag).flatten(),
                slot.try_with(Slot::flag).flatten(),
                fill,
                unsettled,
            ),
            (Rule::Autotune, _, _) => store
                .autotune
                .try_with(|a| autotune_look(a.shown(), fill))
                .unwrap_or_else(|| autotune_look(None, fill)),
            _ => store
                .reverb
                .try_with(|r| reverb_look(r.shown(), fill))
                .unwrap_or_else(|| reverb_look(None, fill)),
        }
    });
    // The flight recorder hears each step with the keys the button writes
    // (none for REVERB and AUTOTUNE: the hub writes).
    let step = move |what: &str, pointer: i32| {
        let _ = keys.try_with_value(|k| trace_rail(what, button, k, pointer));
    };
    let refuse = move || fail_flash(failed)(rule.refusal().to_string());
    // The tap. STAGE: the stage mics by hand, as STAGE did (I8: Live's
    // value here). REVERB: the faders down or back as the button shows them
    // (`reverb_low`), only while connected with the hub's state of this
    // connection, else refused. AUTOTUNE: the singers' panel closed when
    // open, else opened beside the column on the same condition (sends
    // nothing).
    // Where the panel opens from now: the column's right edge and the
    // button's top and bottom on the screen (at the tap, and again at every
    // resize: PR #150 review N3).
    let anchor_now = move || {
        root.try_get_untracked().flatten().map(|el| {
            let button = el.get_bounding_client_rect();
            let column = el
                .closest(".column")
                .ok()
                .flatten()
                .map_or(button.right(), |c| c.get_bounding_client_rect().right());
            Anchor {
                column_right: column,
                button_top: button.top(),
                button_bottom: button.bottom(),
            }
        })
    };
    let tap = move |pointer: i32| {
        // An open panel closes at a tap on AUTOTUNE in every state of the
        // button (PR #150 review N1).
        if rule == Rule::Autotune && open.try_get_untracked().flatten().is_some() {
            step("tap", pointer);
            let _ = open.try_set(None);
            return;
        }
        if let Some(slot) = slot {
            step("tap", pointer);
            if readiness_now(&[slot]) != Readiness::Ready {
                return;
            }
            let _ = spec.try_with_value(|s| {
                if let Some(s) = s {
                    toggle_flag(store, s, slot, failed);
                }
            });
            return;
        }
        if state.try_get_untracked() != Some(Readiness::Ready) {
            step("tap", pointer);
            return;
        }
        let online = store.connected.try_get_untracked().unwrap_or(false);
        if rule == Rule::Autotune {
            let opens = store
                .autotune
                .try_with_untracked(|a| tap_opens_panel(a.fresh(), online))
                .unwrap_or(false);
            if !opens {
                step("refused", pointer);
                refuse();
                return;
            }
            step("tap", pointer);
            let anchor = anchor_now();
            let _ = open.try_set(anchor);
            return;
        }
        let value = store
            .reverb
            .try_with_untracked(|r| tap_value(r.fresh(), online))
            .flatten();
        step(tap_step(value), pointer);
        match value {
            Some(value) => store.set_hub(HUB_REVERB_LOW, json!(value)),
            None => refuse(),
        }
    };
    // The panel closes when this connection no longer has the hub's state
    // (a lost socket: it shows no stale singer, Review Focus 5).
    if rule == Rule::Autotune {
        Effect::new(move |_| {
            let fresh = store
                .autotune
                .try_with(|a| a.fresh().is_some())
                .unwrap_or(false);
            if !fresh && open.try_get_untracked().flatten().is_some() {
                let _ = open.try_set(None);
                // Recorded as the outside tap's close (PR #150 review N4).
                step("close", 0);
            }
        });
    }
    // What a hold may send now: the automatic mode the other way, or none
    // (refused).
    let can_send = move || {
        let aut = match rule {
            Rule::Stage => store
                .hub
                .try_with_untracked(|h| h.fresh_flag(HUB_STAGE_AUT))
                .flatten(),
            Rule::Reverb => store
                .reverb
                .try_with_untracked(|r| r.fresh().map(|s| s.aut))
                .flatten(),
            Rule::Autotune => store
                .autotune
                .try_with_untracked(|a| a.fresh().map(|s| s.aut))
                .flatten(),
        };
        let connected = store.connected.try_get_untracked().unwrap_or(false);
        hold_value(ready_hold(aut, ready(state.try_get_untracked())), connected)
    };
    // The lift of a full press: the automatic mode the other way when it
    // can act, else refused.
    let held = move |pointer: i32| {
        let value = can_send();
        step(hold_step(value), pointer);
        match value {
            Some(on) => store.set_hub(aut_key, json!(on)),
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
        // A down outside AUTOTUNE and its panel closes the panel, and is
        // taken for that alone. A new down of the pointer whose up was to
        // be taken ends that wait (its up never came).
        if swallows_up(dismissed.try_get_value().flatten(), ev.pointer_id()) {
            let _ = dismissed.try_set_value(None);
        }
        let is_open = open.try_get_untracked().flatten().is_some();
        if is_open {
            let target = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
            let on_button = root
                .try_get_untracked()
                .flatten()
                .zip(target.as_ref())
                .is_some_and(|(el, t)| {
                    let node: &web_sys::Node = t;
                    el.contains(Some(node))
                });
            let on_panel = target
                .as_ref()
                .and_then(|t| t.closest(".tpanel").ok().flatten())
                .is_some();
            // It closes the panel and reaches nothing under it (PR #150
            // review I1: a strip's mute acts on its down).
            if dismisses(is_open, on_button, on_panel) {
                ev.prevent_default();
                ev.stop_propagation();
                let _ = dismissed.try_set_value(Some(ev.pointer_id()));
                let _ = open.try_set(None);
                step("close", ev.pointer_id());
            }
        }
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
    // The up (or cancel) of the down that closed the panel reaches nothing
    // either: a control that acts on its up alone (AbleSet's sign-in and
    // direct address) never acts on a tap that closed the panel (PR #150
    // re-review n2). Only AUTOTUNE has a panel.
    if rule == Rule::Autotune {
        let listener = Closure::wrap(Box::new(move |ev: web_sys::PointerEvent| {
            if swallows_up(dismissed.try_get_value().flatten(), ev.pointer_id()) {
                ev.prevent_default();
                ev.stop_propagation();
                let _ = dismissed.try_set_value(None);
            }
        }) as Box<dyn FnMut(web_sys::PointerEvent)>);
        if let Some(window) = web_sys::window() {
            for name in UP_EVENTS {
                let _ = window.add_event_listener_with_callback_and_bool(
                    name,
                    listener.as_ref().unchecked_ref(),
                    true,
                );
            }
        }
        let _ = any_up.try_set_value(Some(listener));
    }
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
    // A turned or resized screen places the open panel again (PR #150
    // review N3).
    let resized = window_event_listener_untyped("resize", move |_| {
        if open.try_get_untracked().flatten().is_some() {
            let _ = open.try_set(anchor_now());
        }
    });
    on_cleanup(move || resized.remove());
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
            Lift::Tap => tap(id),
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
                    {AUT_BADGE}
                </span>
            }
        })
    };
    // STAGE's mics muted; REVERB's faders lowered; AUTOTUNE's targets on,
    // off, mixed or waiting, mixed, and a target without its rack (each
    // only on its kind).
    let muted =
        move || slot.map(|slot| (slot.try_with(Slot::flag).flatten() == Some(true)).to_string());
    let lowered = move || (rule == Rule::Reverb).then(|| low().to_string());
    let is_autotune = rule == Rule::Autotune;
    let tune_state = move || {
        is_autotune.then(|| {
            store
                .autotune
                .try_with(|a| state_name(a.shown()))
                .unwrap_or("waiting")
                .to_string()
        })
    };
    let mixed =
        move || is_autotune.then(|| look.try_with(|l| l.mixed).unwrap_or(false).to_string());
    let problem = Memo::new(move |_| {
        is_autotune
            && store
                .autotune
                .try_with(|a| has_problem(a.shown()))
                .unwrap_or(false)
    });
    let problem_attr = move || is_autotune.then(|| problem.try_get().unwrap_or(false).to_string());
    // The yellow `!` corner is in the page only while a target has no rack
    // (no hidden text in a rail button).
    let flag = move || {
        problem.try_get().unwrap_or(false).then(|| {
            view! {
                <span class="rule-flag" data-testid="rule-flag" aria-hidden="true">
                    "!"
                </span>
            }
        })
    };
    let open_attr = move || is_autotune.then(|| open.try_get().flatten().is_some().to_string());
    // The panel, beside the column while open, only on this connection's
    // state.
    let shown_at = Memo::new(move |_| {
        let fresh = store
            .autotune
            .try_with(|a| a.fresh().is_some())
            .unwrap_or(false);
        open.try_get().flatten().filter(|_| fresh)
    });
    let panel = move || {
        shown_at
            .try_get()
            .flatten()
            .map(|anchor| view! { <AutotunePanel anchor=anchor /> })
    };
    let filling = move || fill.try_get().is_some_and(Fill::holding).to_string();
    let is_full = move || fill.try_get().is_some_and(Fill::full).to_string();
    // The fill's time is the hold's (rule.css reads it).
    let hold_time = format!("--hold-ms:{RAIL_HOLD_MS}ms;");
    let class = rule.class();
    let testid = rule.testid();
    view! {
        <div
            class=class
            node_ref=root
            use:owns_touches=touch_keys
            class:on=lit
            class:failed=move || failed.try_get().unwrap_or(false)
            style=hold_time
            data-testid=testid
            data-muted=muted
            data-low=lowered
            data-state=tune_state
            data-mixed=mixed
            data-problem=problem_attr
            data-open=open_attr
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
            {flag}
        </div>
        {panel}
    }
}
