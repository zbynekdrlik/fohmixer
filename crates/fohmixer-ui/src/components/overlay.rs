//! The global control (spec F16; the rail's footer on every page, #21):
//! TechAlert, the mute of its track with the full-screen blink while it is
//! unmuted.

use fohmixer_proto::layout::Binding;
use leptos::html;
use leptos::prelude::*;

use super::buttons::toggle_flag;
use super::{BtnText, key_of, owns_touches, readiness, slot_of};
use crate::behave::mute::{GUARD_MS, GuardAction, MuteGuard};
use crate::behave::timing::blink_on;
use crate::binding::mute_sub;
use crate::dom;
use crate::raf;
use crate::store::{LiveStore, Slot};

/// TechAlert (spec F16): a button lit while the TechAlert track is unmuted
/// (a tap mutes or unmutes it), and the red wash over the whole surface
/// that shows every other `period_ms` meanwhile; the wash never takes a
/// touch. An unresolved binding shows red (I5).
#[component]
pub fn AlertView(
    binding: Binding,
    period_ms: u32,
    label: Option<String>,
    guarded: bool,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let spec = mute_sub(&binding);
    let slot = slot_of(store, spec.as_ref());
    // The button owns its touches (#43 PR G) and names its key.
    let touch_keys: Vec<String> = spec.iter().map(key_of).collect();
    let spec = StoredValue::new(spec);
    let failed = RwSignal::new(false);
    let guard = StoredValue::new(MuteGuard::default());
    let armed = RwSignal::new(false);
    let root = NodeRef::<html::Div>::new();
    let period = f64::from(period_ms);
    raf::animate(root, move |el| {
        let mut shown: Option<bool> = None;
        let mut was_active: Option<bool> = None;
        Box::new(move |now: f64, _step: f64| {
            let active = slot.try_with_untracked(Slot::flag).flatten() == Some(false);
            if was_active != Some(active) {
                dom::set_attr(&el, "data-active", if active { "true" } else { "false" });
                was_active = Some(active);
            }
            let on = blink_on(active, now, period);
            if shown != Some(on) {
                dom::set_style(&el, "visibility", if on { "visible" } else { "hidden" });
                dom::set_attr(&el, "data-visible", if on { "true" } else { "false" });
                shown = Some(on);
            }
        })
    });
    // A guarded TechAlert needs a second tap within 500 ms (the first arms
    // and pulses), as a guarded strip's mute.
    let on_down = move |ev: web_sys::PointerEvent| {
        if !slot.try_with_untracked(Slot::is_ready).unwrap_or(false) {
            return;
        }
        ev.prevent_default();
        let action = guard
            .try_update_value(|g| g.on_tap(guarded, dom::now()))
            .unwrap_or(GuardAction::Arm);
        match action {
            GuardAction::Apply => {
                let _ = armed.try_set(false);
                let _ = spec.try_with_value(|s| {
                    if let Some(s) = s {
                        toggle_flag(store, s, slot, failed);
                    }
                });
            }
            GuardAction::Arm => {
                let _ = armed.try_set(true);
                set_timeout(
                    move || {
                        let still = guard
                            .try_with_value(|g| g.armed(dom::now()))
                            .unwrap_or(false);
                        if !still {
                            let _ = armed.try_set(false);
                        }
                    },
                    std::time::Duration::from_millis(GUARD_MS as u64 + 20),
                );
            }
        }
    };
    let active = move || slot.try_with(Slot::flag).flatten() == Some(false);
    let state = Memo::new(move |_| readiness(&[slot]));
    let bound = move || state.try_get().map_or("waiting", |s| s.name());
    let disabled = move || state.try_get().map_or("true", |s| s.disabled());
    let text = label.unwrap_or_else(|| "TechAlert".to_string());
    view! {
        <div
            class="btn alert-toggle"
            use:owns_touches=touch_keys
            class:on=active
            class:armed=move || armed.try_get().unwrap_or(false)
            class:failed=move || failed.try_get().unwrap_or(false)
            data-testid="alert-toggle"
            data-on=move || active().to_string()
            data-binding=bound
            aria-disabled=disabled
            on:pointerdown=on_down
        >
            <BtnText text=text />
        </div>
        <div
            class="alert-wash"
            data-testid="alert"
            data-visible="false"
            data-active="false"
            data-binding=bound
            style="visibility:hidden;"
            node_ref=root
        ></div>
    }
}
