//! The toggle buttons (spec F12, F14, F15): the strip mute with its
//! protection, the group solos, the stage-mic button and STAGE AUT. Every
//! button shows Live's (or the hub's) value, never a local memory (spec P2),
//! and takes a tap only once that value is here (I8). A write that Live has
//! not confirmed outlines the button (`data-intent`, #43 PR C:
//! `components::intent_look`), and each tap goes to the page's flight
//! recorder.

use fohmixer_proto::client::HUB_STAGE_AUT;
use fohmixer_proto::layout::{Anchor, Binding};
use leptos::html;
use leptos::prelude::*;
use serde_json::{Value, json};

use super::{
    BtnText, fail_flash, intent_look, key_of, owns_touches, readiness, slot_of, trace_touch,
};
use crate::behave::colour::{css_color, text_on};
use crate::behave::label::{label_chars, strip_label};
use crate::behave::mute::{GUARD_MS, GuardAction, MuteGuard, lit};
use crate::binding::{SubSpec, mute_sub, solo_sub};
use crate::dom;
use crate::store::{LiveStore, Slot};

/// The style of a button lit in Live's colour `value` (the track colour and
/// the text colour that reads on it); nothing until the colour is known.
fn colour_style(value: Option<f64>) -> String {
    value
        .and_then(|v| css_color(v).map(|css| format!("--tc:{css};--tt:{};", text_on(v))))
        .unwrap_or_default()
}

/// A solo button's text: `SOLO` and its label, or the first word of its
/// group track's name.
fn solo_text(label: Option<&str>, binding: &Binding) -> String {
    let name = label.map_or_else(|| strip_label(&anchor_name(binding)), str::to_string);
    format!("SOLO {name}")
}

/// The name a binding's anchor shows.
pub fn anchor_name(binding: &Binding) -> String {
    match &binding.anchor {
        Anchor::Track { name } | Anchor::Return { name } => name.clone(),
        Anchor::Master => "Master".to_string(),
        Anchor::Song => "Song".to_string(),
    }
}

/// Writes the inverse of a flag slot's value (a tap: a final `set`, #43).
/// A tap inverts what the button shows, Live's value (P2), also while a
/// write is still on its way: nothing on the button shows that write, so a
/// second tap means "it did not take", not "undo".
pub(super) fn toggle_flag(
    store: LiveStore,
    spec: &SubSpec,
    slot: RwSignal<Slot>,
    failed: RwSignal<bool>,
) {
    let Some(current) = slot.try_with_untracked(Slot::flag).flatten() else {
        return;
    };
    store.set(
        &spec.instance,
        &spec.target,
        &spec.prop,
        json!(!current),
        true,
        Some(fail_flash(failed)),
    );
}

/// A strip's name button, its mute: lit in the track's Live colour while the
/// track is audible; a guarded strip needs a second tap within 500 ms (the
/// first arms and pulses).
#[component]
pub fn MuteView(
    state: RwSignal<Slot>,
    spec: SubSpec,
    guarded: bool,
    label: String,
    color: Option<RwSignal<Slot>>,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let slot = state;
    let keys = vec![key_of(&spec)];
    let root = NodeRef::<html::Div>::new();
    intent_look(root, store, keys.clone());
    // The root owns its touches (#43 PR G) and names its keys.
    let touch_keys = keys.clone();
    let keys = StoredValue::new(keys);
    let spec = StoredValue::new(spec);
    let guard = StoredValue::new(MuteGuard::default());
    let armed = RwSignal::new(false);
    let failed = RwSignal::new(false);
    let on_down = move |ev: web_sys::PointerEvent| {
        if !slot.try_with_untracked(Slot::is_ready).unwrap_or(false) {
            return;
        }
        ev.prevent_default();
        let _ = keys.try_with_value(|k| trace_touch("tap", k, ev.pointer_id()));
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
    let look = move || colour_style(color.and_then(|c| c.with(Slot::number)));
    let length = format!("--n:{};", label_chars(&label));
    view! {
        <div
            class="mute"
            node_ref=root
            use:owns_touches=touch_keys
            class:lit=is_lit
            class:armed=move || armed.get()
            class:failed=move || failed.get()
            data-testid="mute"
            data-muted=muted_attr
            data-binding=binding
            aria-disabled=disabled
            style=look
            on:pointerdown=on_down
        >
            <span class="strip-label" data-testid="strip-label" style=length>{label}</span>
            <span class="mute-mark" aria-hidden="true">"MUTE"</span>
        </div>
    }
}

/// A group-track solo (spec F14): independent, lit when on.
#[component]
pub fn SoloView(binding: Binding, label: Option<String>) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let label = solo_text(label.as_deref(), &binding);
    let track = anchor_name(&binding);
    let spec = solo_sub(&binding);
    let slot = slot_of(store, spec.as_ref());
    let keys: Vec<String> = spec.iter().map(key_of).collect();
    let root = NodeRef::<html::Div>::new();
    intent_look(root, store, keys.clone());
    // The root owns its touches (#43 PR G) and names its keys.
    let touch_keys = keys.clone();
    let keys = StoredValue::new(keys);
    let spec = StoredValue::new(spec);
    let failed = RwSignal::new(false);
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = keys.try_with_value(|k| trace_touch("tap", k, ev.pointer_id()));
        let _ = spec.try_with_value(|s| {
            if let Some(s) = s {
                toggle_flag(store, s, slot, failed);
            }
        });
    };
    let on = move || slot.with(Slot::flag) == Some(true);
    let state = Memo::new(move |_| readiness(&[slot]));
    let bound = move || state.get().name();
    let disabled = move || state.get().disabled();
    view! {
        <div
            class="btn solo"
            node_ref=root
            use:owns_touches=touch_keys
            class:on=on
            class:failed=move || failed.get()
            data-testid="solo"
            data-track=track
            data-on=move || on().to_string()
            data-binding=bound
            aria-disabled=disabled
            on:pointerdown=on_down
        >
            <BtnText text=label />
        </div>
    }
}

/// The stage-mic button (spec F15): the mute of the stage-mic track, lit
/// while Live reports the mics live (`mute` false), like TouchOSC's button
/// (mute false -> x 1) and every strip's mute (F12, lit when audible). An
/// unknown or unmapped state stays dark: a lit STAGE means "the stage is open"
/// (#9, parity audit #21).
#[component]
pub fn StageMicsView(binding: Binding, label: Option<String>) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let label = label.unwrap_or_else(|| "STAGE".to_string());
    let spec = mute_sub(&binding);
    let slot = slot_of(store, spec.as_ref());
    let keys: Vec<String> = spec.iter().map(key_of).collect();
    let root = NodeRef::<html::Div>::new();
    intent_look(root, store, keys.clone());
    // The root owns its touches (#43 PR G) and names its keys.
    let touch_keys = keys.clone();
    let keys = StoredValue::new(keys);
    let spec = StoredValue::new(spec);
    let failed = RwSignal::new(false);
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = keys.try_with_value(|k| trace_touch("tap", k, ev.pointer_id()));
        let _ = spec.try_with_value(|s| {
            if let Some(s) = s {
                toggle_flag(store, s, slot, failed);
            }
        });
    };
    let muted = move || slot.with(Slot::flag) == Some(true);
    let live = move || slot.with(Slot::flag) == Some(false);
    let state = Memo::new(move |_| readiness(&[slot]));
    let bound = move || state.get().name();
    let disabled = move || state.get().disabled();
    view! {
        <div
            class="btn stage-mics"
            node_ref=root
            use:owns_touches=touch_keys
            class:on=live
            class:failed=move || failed.get()
            data-testid="stage-mics"
            data-muted=move || muted().to_string()
            data-binding=bound
            aria-disabled=disabled
            on:pointerdown=on_down
        >
            <BtnText text=label />
        </div>
    }
}

/// A hub value's toggle: STAGE AUT (spec F15, X9: the rule runs in the hub).
#[component]
pub fn HubToggleView(key: String, label: String) -> impl IntoView {
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
    // It owns its touches (#43 PR G); a hub value has no write key.
    let no_keys: Vec<String> = Vec::new();
    view! {
        <div
            class="btn hub-toggle"
            use:owns_touches=no_keys
            class:on=on
            data-testid=testid
            data-on=move || on().to_string()
            aria-disabled=disabled
            on:pointerdown=on_down
        >
            <BtnText text=label />
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
    fn a_solo_says_solo_and_its_label_or_its_groups_first_word() {
        let group = binding(Anchor::Track {
            name: "Stems grp#".into(),
        });
        assert_eq!(solo_text(None, &group), "SOLO Stems");
        assert_eq!(solo_text(Some("Podklady"), &group), "SOLO Podklady");
    }

    #[test]
    fn a_name_button_takes_the_tracks_colour_once_known() {
        assert_eq!(colour_style(None), "");
        assert_eq!(colour_style(Some(12.5)), "");
        assert_eq!(
            colour_style(Some(f64::from(0xF5C451u32))),
            "--tc:#F5C451;--tt:#10101a;"
        );
        assert_eq!(
            colour_style(Some(f64::from(0x1E3A8Au32))),
            "--tc:#1E3A8A;--tt:#f4f4fa;"
        );
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
