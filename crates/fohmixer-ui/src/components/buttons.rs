//! The toggle buttons (spec F12, F14, F15): the strip mute with its
//! protection, the group solos, the stage-mic button and STAGE AUT. Every
//! button shows Live's (or the hub's) value, never a local memory (spec P2),
//! and takes a tap only once that value is here (I8).

use fohmixer_proto::client::HUB_STAGE_AUT;
use fohmixer_proto::layout::{Anchor, Binding, Frame, Style};
use leptos::prelude::*;
use serde_json::{Value, json};

use super::{fail_flash, readiness, slot_of};
use crate::behave::label::strip_label;
use crate::behave::mute::{GUARD_MS, GuardAction, MuteGuard, lit};
use crate::binding::{SubSpec, mute_sub, solo_sub};
use crate::dom;
use crate::stage;
use crate::store::{LiveStore, Slot};

/// The colours of a solo button (TouchOSC).
const SOLO_ON: &str = "#3D61B8";
const SOLO_OFF: &str = "#3C3C3C";

/// The name a binding's anchor shows.
pub fn anchor_name(binding: &Binding) -> String {
    match &binding.anchor {
        Anchor::Track { name } | Anchor::Return { name } => name.clone(),
        Anchor::Master => "Master".to_string(),
        Anchor::Song => "Song".to_string(),
    }
}

/// Writes the inverse of a flag slot's value.
fn toggle_flag(store: LiveStore, spec: &SubSpec, slot: RwSignal<Slot>, failed: RwSignal<bool>) {
    let Some(current) = slot.try_with_untracked(Slot::flag).flatten() else {
        return;
    };
    store.set_prop(
        &spec.instance,
        &spec.target,
        &spec.prop,
        json!(!current),
        Some(fail_flash(failed)),
    );
}

/// A strip's mute: lit while the track is audible; a guarded strip needs a
/// second tap within 500 ms (the first arms and pulses).
#[component]
pub fn MuteView(
    frame: Frame,
    state: RwSignal<Slot>,
    spec: SubSpec,
    guarded: bool,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let slot = state;
    let spec = StoredValue::new(spec);
    let guard = StoredValue::new(MuteGuard::default());
    let armed = RwSignal::new(false);
    let failed = RwSignal::new(false);
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
                let _ = spec.try_with_value(|s| toggle_flag(store, s, slot, failed));
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
    let muted = move || slot.with(Slot::flag);
    let is_lit = move || muted().is_some_and(lit);
    let state = Memo::new(move |_| readiness(&[slot]));
    let binding = move || state.get().name();
    let disabled = move || state.get().disabled();
    let muted_attr = move || match muted() {
        Some(true) => "true",
        Some(false) => "false",
        None => "unknown",
    };
    view! {
        <div
            class="mute"
            class:lit=is_lit
            class:armed=move || armed.get()
            class:failed=move || failed.get()
            data-testid="mute"
            data-muted=muted_attr
            data-binding=binding
            aria-disabled=disabled
            style={stage::box_style(frame)}
            on:pointerdown=on_down
        ></div>
    }
}

/// A group-track solo (spec F14): independent, blue when on.
#[component]
pub fn SoloView(frame: Frame, z: i64, style: Style, binding: Binding) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let label = format!("SOLO {}", strip_label(&anchor_name(&binding)));
    let track = anchor_name(&binding);
    let spec = solo_sub(&binding);
    let slot = slot_of(store, spec.as_ref());
    let spec = StoredValue::new(spec);
    let failed = RwSignal::new(false);
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = spec.try_with_value(|s| {
            if let Some(s) = s {
                toggle_flag(store, s, slot, failed);
            }
        });
    };
    let on = move || slot.with(Slot::flag) == Some(true);
    let css = stage::item_style(frame, z, &style);
    let look = move || {
        let color = if on() { SOLO_ON } else { SOLO_OFF };
        format!("{css}background:{color};")
    };
    let state = Memo::new(move |_| readiness(&[slot]));
    let bound = move || state.get().name();
    let disabled = move || state.get().disabled();
    view! {
        <div
            class="item button solo"
            class:on=on
            class:failed=move || failed.get()
            data-testid="solo"
            data-track=track
            data-on=move || on().to_string()
            data-binding=bound
            aria-disabled=disabled
            style=look
            on:pointerdown=on_down
        >
            {label}
        </div>
    }
}

/// The stage-mic button (spec F15): an inverted mute, lit while the stage
/// mics are muted.
#[component]
pub fn StageMicsView(frame: Frame, z: i64, style: Style, binding: Binding) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let label = style.text.clone().unwrap_or_else(|| "STAGE".to_string());
    let spec = mute_sub(&binding);
    let slot = slot_of(store, spec.as_ref());
    let spec = StoredValue::new(spec);
    let failed = RwSignal::new(false);
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = spec.try_with_value(|s| {
            if let Some(s) = s {
                toggle_flag(store, s, slot, failed);
            }
        });
    };
    let muted = move || slot.with(Slot::flag) == Some(true);
    let state = Memo::new(move |_| readiness(&[slot]));
    let bound = move || state.get().name();
    let disabled = move || state.get().disabled();
    view! {
        <div
            class="item button stage-mics"
            class:on=muted
            class:failed=move || failed.get()
            data-testid="stage-mics"
            data-muted=move || muted().to_string()
            data-binding=bound
            aria-disabled=disabled
            style={stage::item_style(frame, z, &style)}
            on:pointerdown=on_down
        >
            {label}
        </div>
    }
}

/// A hub value's toggle: STAGE AUT (spec F15, X9: the rule runs in the hub).
#[component]
pub fn HubToggleView(
    frame: Frame,
    z: i64,
    style: Style,
    key: String,
    label: String,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let key = StoredValue::new(key);
    let value = move || {
        key.try_with_value(|k| store.hub.with(|hub| hub.get(k).and_then(Value::as_bool)))
            .flatten()
    };
    let on = move || value() == Some(true);
    let ready = move || store.connected.get() && value().is_some();
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let current = key
            .try_with_value(|k| {
                store
                    .hub
                    .try_with_untracked(|hub| hub.get(k).and_then(Value::as_bool))
                    .flatten()
            })
            .flatten();
        let connected = store.connected.try_get_untracked().unwrap_or(false);
        if let (Some(current), true) = (current, connected) {
            let _ = key.try_with_value(|k| store.set_hub(k, json!(!current)));
        }
    };
    let testid = if key.with_value(|k| k == HUB_STAGE_AUT) {
        "stage-aut"
    } else {
        "hub-toggle"
    };
    let disabled = move || if ready() { "false" } else { "true" };
    view! {
        <div
            class="item button hub-toggle"
            class:on=on
            data-testid=testid
            data-on=move || on().to_string()
            aria-disabled=disabled
            style={stage::item_style(frame, z, &style)}
            on:pointerdown=on_down
        >
            {label}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(anchor: Anchor) -> Binding {
        Binding {
            instance: "band".into(),
            anchor,
            path: None,
        }
    }

    #[test]
    fn an_anchor_names_its_track() {
        assert_eq!(
            anchor_name(&binding(Anchor::Track {
                name: "Hand1 #".into()
            })),
            "Hand1 #"
        );
        assert_eq!(
            anchor_name(&binding(Anchor::Return {
                name: "A-Reverb #".into()
            })),
            "A-Reverb #"
        );
        assert_eq!(anchor_name(&binding(Anchor::Master)), "Master");
        assert_eq!(anchor_name(&binding(Anchor::Song)), "Song");
    }

    #[test]
    fn a_button_subscribes_its_anchors_property() {
        let solo = binding(Anchor::Track {
            name: "Stems grp#".into(),
        });
        assert_eq!(
            solo_sub(&solo),
            Some(SubSpec::new(
                "band",
                "live_set tracks[name=Stems grp#]".into(),
                "solo",
                false
            ))
        );
        assert_eq!(mute_sub(&solo).map(|s| s.prop), Some("mute".to_string()));
        let broken = Binding {
            path: Some("devices[name=".into()),
            ..solo
        };
        assert_eq!(mute_sub(&broken), None);
    }
}
