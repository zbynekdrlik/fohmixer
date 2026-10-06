//! The former MIDI controls (spec F17, F18, D10, X10, X11): toggles and a
//! fader that write their target parameters directly through the LOM and
//! show the targets' real state: all on, all off, or mixed. A toggle's
//! writes Live has not confirmed outline it (the most urgent of its
//! targets', `data-intent`, #43 PR C), and its presses go to the page's
//! flight recorder.

use fohmixer_proto::layout::{ParamTarget, Press};
use leptos::html;
use leptos::prelude::*;
use serde_json::Value;

use super::fader::{self, FaderView, Law};
use super::{
    BtnText, fail_flash, intent_look, key_of, owns_touches, readiness, slot_of, touch_end_name,
    trace_touch,
};
use crate::behave::toggle::{ToggleCtl, ToggleState, Write, aggregate, is_on};
use crate::binding::{SubSpec, param_subs};
use crate::dom;
use crate::store::{LiveStore, Slot};

/// One toggle target: its subscription (none when its path does not
/// parse), its slot and the values it takes for on and off.
#[derive(Clone)]
struct Target {
    spec: Option<SubSpec>,
    slot: RwSignal<Slot>,
    on: Value,
    off: Value,
}

/// The state a toggle shows (reactive) from its targets' slots.
fn toggle_state(targets: &[Target], tracked: bool) -> ToggleState {
    let states: Vec<Option<bool>> = targets
        .iter()
        .map(|t| {
            let read = |s: &Slot| s.value().map(|v| is_on(v, &t.on));
            if tracked {
                t.slot.with(read)
            } else {
                t.slot.try_with_untracked(read).flatten()
            }
        })
        .collect();
    aggregate(&states)
}

/// The `data-state` of a toggle state.
pub fn state_name(state: ToggleState) -> &'static str {
    match state {
        ToggleState::On => "on",
        ToggleState::Off => "off",
        ToggleState::Mixed => "mixed",
        ToggleState::Unknown => "unknown",
    }
}

/// A former MIDI toggle.
#[component]
pub fn ParamToggleView(
    label: String,
    targets: Vec<ParamTarget>,
    press: Press,
    color: Option<String>,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let targets: Vec<Target> = targets
        .iter()
        .zip(param_subs(&targets, false))
        .map(|(t, spec)| Target {
            slot: slot_of(store, spec.as_ref()),
            spec,
            on: t.on.clone().unwrap_or(Value::Null),
            off: t.off.clone().unwrap_or(Value::Null),
        })
        .collect();
    let slots: Vec<RwSignal<Slot>> = targets.iter().map(|t| t.slot).collect();
    let binding = Memo::new(move |_| readiness(&slots));
    let bound = move || binding.get().name();
    let keys: Vec<String> = targets
        .iter()
        .filter_map(|t| t.spec.as_ref())
        .map(key_of)
        .collect();
    let root = NodeRef::<html::Div>::new();
    intent_look(root, store, keys.clone());
    // The root owns its touches (#43 PR G) and names its keys.
    let touch_keys = keys.clone();
    let keys = StoredValue::new(keys);
    let targets = StoredValue::new(targets);
    let ctl = StoredValue::new(ToggleCtl::default());
    let failed = RwSignal::new(false);
    let state = move || {
        targets
            .try_with_value(|all| toggle_state(all, true))
            .unwrap_or(ToggleState::Unknown)
    };
    let write = move |w: Write| {
        let _ = targets.try_with_value(|all| {
            for t in all {
                if let Some(spec) = &t.spec {
                    let value = match w {
                        Write::On => t.on.clone(),
                        Write::Off => t.off.clone(),
                    };
                    // A press or release: a final `set` (#43).
                    store.set(
                        &spec.instance,
                        &spec.target,
                        &spec.prop,
                        value,
                        true,
                        Some(fail_flash(failed)),
                    );
                }
            }
        });
    };
    let on_down = move |ev: web_sys::PointerEvent| {
        let now_state = targets
            .try_with_value(|all| toggle_state(all, false))
            .unwrap_or(ToggleState::Unknown);
        if now_state == ToggleState::Unknown {
            return;
        }
        ev.prevent_default();
        if let Some(el) = dom::current_element(&ev) {
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        let _ = keys.try_with_value(|k| trace_touch("down", k, ev.pointer_id()));
        if let Some(Some(w)) = ctl.try_update_value(|c| c.down(press, now_state, dom::now())) {
            write(w);
        }
    };
    let on_up = move |ev: web_sys::PointerEvent| {
        if let Some(what) = touch_end_name(&ev.type_()) {
            let _ = keys.try_with_value(|k| trace_touch(what, k, ev.pointer_id()));
        }
        if let Some(Some(w)) = ctl.try_update_value(ToggleCtl::up) {
            write(w);
        }
    };
    let disabled = move || {
        if state() == ToggleState::Unknown {
            "true"
        } else {
            "false"
        }
    };
    let label_attr = label.clone();
    let press_name = match press {
        Press::Toggle => "toggle",
        Press::DoubleTapLatch => "double_tap_latch",
        Press::PulseAndDoubleTapLatch => "pulse_and_double_tap_latch",
    };
    // The layout's colour of the toggle: its lit colour.
    let look = color.map(|c| format!("--c:{c};")).unwrap_or_default();
    view! {
        <div
            class="btn param-toggle"
            node_ref=root
            use:owns_touches=touch_keys
            class:failed=move || failed.get()
            data-testid="param-toggle"
            data-label=label_attr
            data-press=press_name
            data-state=move || state_name(state())
            data-binding=bound
            aria-disabled=disabled
            style=look
            on:pointerdown=on_down
            on:pointerup=on_up
            on:pointercancel=on_up
            on:lostpointercapture=on_up
        >
            <BtnText text=label />
        </div>
    }
}

/// A former MIDI fader (spec F18, X10): writes every target by its own
/// range and shows Live's display string of the first.
#[component]
pub fn ParamFaderView(label: String, targets: Vec<ParamTarget>) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let targets: Vec<fader::Target> = param_subs(&targets, true)
        .into_iter()
        .map(|spec| {
            let range = spec.as_ref().map_or_else(
                || RwSignal::new(None),
                |s| store.range(&s.instance, &s.target),
            );
            fader::Target {
                slot: slot_of(store, spec.as_ref()),
                spec,
                law: Law::Linear(range),
            }
        })
        .collect();
    let slot = targets
        .first()
        .map_or_else(|| RwSignal::new(Slot::Pending), |t| t.slot);
    let display = move || slot.with(|s| s.display().unwrap_or_default().to_string());
    let label_attr = label.clone();
    view! {
        <div class="strip param-fader" data-testid="param-fader" data-label=label_attr>
            <span class="param-fader-label">{label}</span>
            <div class="strip-fz param-fz">
                <FaderView targets=targets shaping=false />
            </div>
            <span class="param-fader-display" data-testid="param-display">{display}</span>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn target(value: Option<Value>, on: Value) -> Target {
        let slot = match value {
            Some(value) => Slot::Value {
                value,
                display: None,
                at: 0.0,
            },
            None => Slot::Pending,
        };
        Target {
            spec: None,
            slot: RwSignal::new(slot),
            on,
            off: Value::Null,
        }
    }

    #[test]
    fn a_toggle_shows_its_targets_state() {
        let unmuted = || target(Some(json!(false)), json!(false));
        let muted = || target(Some(json!(true)), json!(false));
        for tracked in [true, false] {
            assert_eq!(
                toggle_state(&[unmuted(), unmuted()], tracked),
                ToggleState::On
            );
            assert_eq!(toggle_state(&[muted(), muted()], tracked), ToggleState::Off);
            assert_eq!(
                toggle_state(&[unmuted(), muted()], tracked),
                ToggleState::Mixed
            );
            assert_eq!(
                toggle_state(&[unmuted(), target(None, json!(false))], tracked),
                ToggleState::Unknown
            );
            assert_eq!(
                toggle_state(&[target(Some(json!(127.0)), json!(127.0))], tracked),
                ToggleState::On
            );
        }
    }

    #[test]
    fn each_toggle_state_has_its_name() {
        assert_eq!(
            [
                ToggleState::On,
                ToggleState::Off,
                ToggleState::Mixed,
                ToggleState::Unknown
            ]
            .map(state_name),
            ["on", "off", "mixed", "unknown"]
        );
    }
}
