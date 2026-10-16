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
use crate::behave::mute::{GUARD_MS, GuardAction, Linked, MuteGuard, Reach};
use crate::binding::{SubSpec, mute_sub, solo_sub};
use crate::dom;
use crate::store::{LiveStore, Slot};

/// The style of a button lit in Live's colour `value` (the track colour and
/// the text colour that reads on it); nothing until the colour is known.
/// The channel detail's name chip takes it too (#71).
pub(crate) fn colour_style(value: Option<f64>) -> String {
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
        // A marker strip's track by index (#68): Live's track number.
        Anchor::TrackAt { index } | Anchor::ReturnAt { index } => format!("#{}", index + 1),
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

/// A linked mute's partner (#127): another track's mute, its subscription
/// and its slot.
#[derive(Clone)]
pub struct Partner {
    pub spec: SubSpec,
    pub slot: RwSignal<Slot>,
}

/// What a mute's tracks (`slots`: the strip's own first, then its
/// partners') show together and which of them a tap writes (#127: the
/// strip's own and every partner with Live's value), tracked or not.
fn linked_now(slots: &[RwSignal<Slot>], tracked: bool) -> Reach {
    let muted: Vec<Option<bool>> = slots
        .iter()
        .map(|s| {
            if tracked {
                s.try_with(Slot::flag).flatten()
            } else {
                s.try_with_untracked(Slot::flag).flatten()
            }
        })
        .collect();
    Reach::of(&muted)
}

/// The linked mute's mark (#127, the approved mockup paired-mute-v3,
/// variant A): a chain and the number of mutes a tap writes (`slots`: the
/// strip's own and each partner's; a partner without Live's value is not
/// counted), grey while all play, red while all are muted, amber `m/N`
/// while m of them are muted by hand in Live. It takes no touch and no
/// height.
#[component]
pub fn LinkMark(slots: Vec<RwSignal<Slot>>) -> impl IntoView {
    let count = slots.len();
    let slots = StoredValue::new(slots);
    let reach = Memo::new(move |_| {
        slots
            .try_with_value(|s| linked_now(s, true))
            .unwrap_or_else(|| Reach::of(&[]))
    });
    let look = move || reach.try_with(|r| r.state.look()).unwrap_or("unknown");
    let text = move || {
        reach
            .try_with(|r| r.text(count))
            .unwrap_or_else(|| count.to_string())
    };
    view! {
        <span class="link-mark" data-testid="mute-link" data-state=look aria-hidden="true">
            <span class="link-n" data-testid="mute-link-n">
                {text}
            </span>
        </span>
    }
}

/// What a strip's name button shows on its corner line (#127, strip.css
/// `.strip-corner`): a problem's `!` and a return's RET around the linked
/// mute's chain, and the MUTE mark at its right end. The channel detail's
/// MUTE has no corner line: its chain is a pill on the button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Corner {
    /// A marker problem's `!` (#68).
    pub flag: bool,
    /// RET: a return in a group that names its instance (#63).
    pub ret: bool,
}

/// A strip's name button, its mute: lit in the track's Live colour while the
/// track is audible; a guarded strip needs a second tap within 500 ms (the
/// first arms and pulses). A linked mute (#127, `partners`) shows and writes
/// its partners' mutes with its own: lit while any of them plays, dark only
/// when all are muted; a tap mutes all, or unmutes all when all are muted,
/// in one `sets` message. It takes a tap once the strip's own track has
/// Live's value: a partner without one is left out of the tap and of what
/// the button shows, and never disables it (the PA mute always works;
/// `behave::mute::Reach`). `corner`: a strip's corner line, with the chain;
/// none for the channel detail's MUTE, which carries the chain on the
/// button.
#[component]
pub fn MuteView(
    state: RwSignal<Slot>,
    spec: SubSpec,
    guarded: bool,
    label: String,
    color: Option<RwSignal<Slot>>,
    partners: Vec<Partner>,
    corner: Option<Corner>,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let slot = state;
    let linked = !partners.is_empty();
    let all: Vec<RwSignal<Slot>> = std::iter::once(slot)
        .chain(partners.iter().map(|p| p.slot))
        .collect();
    let specs: Vec<SubSpec> = std::iter::once(spec.clone())
        .chain(partners.iter().map(|p| p.spec.clone()))
        .collect();
    let keys: Vec<String> = specs.iter().map(key_of).collect();
    // The strip's corner line, or the detail's MUTE mark and pill.
    let line_slots = all.clone();
    let line = corner.map(move |c| {
        let flag = c
            .flag
            .then(|| view! { <span class="strip-flag" aria-hidden="true">"!"</span> });
        let chain = linked.then(move || view! { <LinkMark slots=line_slots /> });
        let ret = c.ret.then(|| {
            view! { <span class="strip-ret" data-testid="strip-ret" aria-hidden="true">"RET"</span> }
        });
        view! {
            <span class="strip-corner">
                {flag}
                {chain}
                {ret}
                <span class="mute-mark" aria-hidden="true">"MUTE"</span>
            </span>
        }
    });
    let on_button = corner
        .is_none()
        .then(|| view! { <span class="mute-mark" aria-hidden="true">"MUTE"</span> });
    let pill_slots = all.clone();
    let pill = (corner.is_none() && linked).then(move || view! { <LinkMark slots=pill_slots /> });
    let root = NodeRef::<html::Div>::new();
    intent_look(root, store, keys.clone());
    // The root owns its touches (#43 PR G) and names its keys.
    let touch_keys = keys.clone();
    let keys = StoredValue::new(keys);
    let spec = StoredValue::new(spec);
    let specs = StoredValue::new(specs);
    let slots = StoredValue::new(all);
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
            GuardAction::Apply if linked => {
                let _ = armed.try_set(false);
                // What the button shows now (P2), over the tracks with
                // Live's value: all muted unmutes them, else they are muted.
                let reach = slots.try_with_value(|s| linked_now(s, false));
                if let Some(reach) = reach
                    && let Some(muted) = reach.state.tap()
                {
                    let _ = specs.try_with_value(|s| {
                        let targets: Vec<SubSpec> = reach
                            .targets
                            .iter()
                            .filter_map(|&i| s.get(i).cloned())
                            .collect();
                        store.set_together(&targets, &json!(muted), true, move || {
                            fail_flash(failed)
                        });
                    });
                }
            }
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
    let shown = Memo::new(move |_| {
        slots
            .try_with_value(|s| linked_now(s, true).state)
            .unwrap_or(Linked::Unknown)
    });
    let now = move || shown.try_get().unwrap_or(Linked::Unknown);
    // Its own track's readiness alone (#127 review M2).
    let state = Memo::new(move |_| readiness(&[slot]));
    let binding = move || state.try_get().map_or("waiting", |s| s.name());
    let disabled = move || state.try_get().map_or("true", |s| s.disabled());
    let look = move || colour_style(color.and_then(|c| c.try_with(Slot::number).flatten()));
    let length = format!("--n:{};", label_chars(&label));
    view! {
        <div
            class="mute"
            node_ref=root
            use:owns_touches=touch_keys
            class:lit=move || now().lit()
            class:armed=move || armed.try_get().unwrap_or(false)
            class:failed=move || failed.try_get().unwrap_or(false)
            data-testid="mute"
            data-muted=move || now().muted_attr()
            data-binding=binding
            aria-disabled=disabled
            style=look
            on:pointerdown=on_down
        >
            <span class="strip-label" data-testid="strip-label" style=length>{label}</span>
            {on_button}
            {pill}
            {line}
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
    let on = move || slot.try_with(Slot::flag).flatten() == Some(true);
    let state = Memo::new(move |_| readiness(&[slot]));
    let bound = move || state.try_get().map_or("waiting", |s| s.name());
    let disabled = move || state.try_get().map_or("true", |s| s.disabled());
    view! {
        <div
            class="btn solo"
            node_ref=root
            use:owns_touches=touch_keys
            class:on=on
            class:failed=move || failed.try_get().unwrap_or(false)
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
    let muted = move || slot.try_with(Slot::flag).flatten() == Some(true);
    let live = move || slot.try_with(Slot::flag).flatten() == Some(false);
    let state = Memo::new(move |_| readiness(&[slot]));
    let bound = move || state.try_get().map_or("waiting", |s| s.name());
    let disabled = move || state.try_get().map_or("true", |s| s.disabled());
    view! {
        <div
            class="btn stage-mics"
            node_ref=root
            use:owns_touches=touch_keys
            class:on=live
            class:failed=move || failed.try_get().unwrap_or(false)
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
        assert_eq!(anchor_name(&binding(Anchor::TrackAt { index: 0 })), "#1");
        assert_eq!(anchor_name(&binding(Anchor::ReturnAt { index: 11 })), "#12");
        assert_eq!(anchor_name(&binding(Anchor::Master)), "Master");
        assert_eq!(anchor_name(&binding(Anchor::Song)), "Song");
    }

    #[test]
    fn a_linked_mute_reads_every_tracks_mute_tracked_or_not() {
        // #127: the strip's own first, then each partner's.
        let mute = |on: Option<bool>| {
            RwSignal::new(on.map_or(Slot::Pending, |m| Slot::Value {
                value: serde_json::json!(m),
                display: None,
                at: 0.0,
            }))
        };
        for tracked in [true, false] {
            let own = mute(Some(false));
            let partner = mute(Some(true));
            let other = mute(Some(false));
            assert_eq!(
                linked_now(&[own, partner, other], tracked).state,
                Linked::Mixed { muted: 1 }
            );
            // #127 review M2: a partner without a value is left out.
            let waiting = mute(None);
            assert_eq!(
                linked_now(&[own, waiting, partner], tracked),
                Reach {
                    targets: vec![0, 2],
                    state: Linked::Mixed { muted: 1 }
                }
            );
            let _ = own.try_set(Slot::Pending);
            assert_eq!(linked_now(&[own, partner], tracked).state, Linked::Unknown);
            assert_eq!(linked_now(&[partner], tracked).state, Linked::Muted);
            assert_eq!(linked_now(&[other], tracked).state, Linked::Playing);
            assert_eq!(linked_now(&[mute(None)], tracked).state, Linked::Unknown);
        }
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
