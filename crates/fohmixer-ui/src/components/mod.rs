//! The surface's controls (spec §2.5; schema 2, #21): one generic component
//! per layout control kind, each bound by the general binding form; the
//! stylesheet places them. Their behaviour lives in `crate::behave` (pure,
//! tested); these components wire pointer events, the store and the
//! animation loop to it. Moving parts are written by the shared frame loop
//! only.

pub mod buttons;
pub mod fader;
pub mod meter;
pub mod overlay;
pub mod pan;
pub mod params;
pub mod strip;

use fohmixer_proto::client::set_key;
use fohmixer_proto::layout::{Control, MeterSource};
use leptos::html;
use leptos::prelude::*;

use crate::behave::Start;
use crate::behave::label::longest_word_chars;
use crate::binding::SubSpec;
use crate::diag::trace::moves::{self, Press, Trail};
use crate::dom;
use crate::raf;
use crate::store::intent::{State, most_urgent};
use crate::store::{LiveStore, Readiness, Slot};

use buttons::{HubToggleView, SoloView, StageMicsView};
use overlay::{AlertView, RefreshView};
use params::{ParamFaderView, ParamToggleView};
use strip::StripView;

/// The layout-wide control settings (a context of the stage).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Fader touch shaping and its post-release delay (spec X3).
    pub shaping: bool,
    /// The strip meters' source (spec X2).
    pub meter_source: MeterSource,
}

/// How long a failed write shows on its control.
pub const FAIL_FLASH_MS: u64 = 400;

/// A failure callback that flashes `failed` red (spec I6: a failed slot is
/// shown, never retried).
pub fn fail_flash(failed: RwSignal<bool>) -> Box<dyn FnOnce(String)> {
    Box::new(move |_why: String| {
        let _ = failed.try_set(true);
        set_timeout(
            move || {
                let _ = failed.try_set(false);
            },
            std::time::Duration::from_millis(FAIL_FLASH_MS),
        );
    })
}

/// The key a control's writes to `spec` go under (`instance|target|prop`,
/// #43).
pub fn key_of(spec: &SubSpec) -> String {
    set_key(&spec.instance, &spec.target, &spec.prop)
}

/// Records a touch (`down`, `up`, `cancel`, or a toggle's `tap`) on the
/// control writing `keys`, by pointer `pointer`, in the page's flight
/// recorder (#43, PR C).
pub fn trace_touch(what: &str, keys: &[String], pointer: i32) {
    crate::diag::record(&crate::diag::trace::touch(
        dom::epoch_now(),
        what,
        keys,
        pointer,
    ));
}

/// Records a fader's or pan's taken down with where its touch started (#43
/// PR D, `diag::trace::moves::touch_start`).
pub fn trace_start(keys: &[String], pointer: i32, press: Press, start: Start) {
    crate::diag::record(&moves::touch_start(
        dom::epoch_now(),
        keys,
        pointer,
        press,
        start,
    ));
}

/// Records the frame that sent position `p` (the set `seq`) of the control
/// writing `key`: the finger's moves since the last one (#43 PR D,
/// `Trail::take`; nothing without a move), a touch's first ones as
/// essential.
pub fn trace_move(trail: StoredValue<Trail>, key: &str, p: f64, seq: Option<u64>) {
    let t = dom::epoch_now();
    let Some((record, essential)) = trail
        .try_update_value(|tr| tr.take(t, key, p, seq))
        .flatten()
    else {
        return;
    };
    if essential {
        crate::diag::record_essential(&record);
    } else {
        crate::diag::record(&record);
    }
}

/// The flight recorder's name of the event that ends a touch: `up` for a
/// `pointerup`, `cancel` for a `pointercancel`; none for the
/// `lostpointercapture` that follows either (it ends nothing new).
pub fn touch_end_name(event_type: &str) -> Option<&'static str> {
    match event_type {
        "pointerup" => Some("up"),
        "pointercancel" => Some("cancel"),
        _ => None,
    }
}

/// The look of a pan's or a toggle's open writes (#43, PR C): every frame
/// `root` gets `data-intent`, the most urgent state of its writes to `keys`
/// (written only when it changes); `intent.css` outlines an `unconfirmed`
/// one amber and a `not_sent` one red, no text. The control still shows
/// Live's value (P2; PR B's decision 7).
pub fn intent_look(root: NodeRef<html::Div>, store: LiveStore, keys: Vec<String>) {
    raf::animate(root, move |el| {
        let mut shown: Option<State> = None;
        Box::new(move |_now: f64, _step: f64| {
            let state = most_urgent(keys.iter().map(|k| store.intent_state(k)));
            if shown != Some(state) {
                dom::set_attr(&el, "data-intent", state.name());
                shown = Some(state);
            }
        })
    });
}

/// Why a control has no subscription: its binding's path does not parse.
const UNPARSED: &str = "the binding's path does not parse";

/// The slot of a control's subscription; a binding without one (its path
/// does not parse) is unresolved from the start (I5).
pub fn slot_of(store: LiveStore, spec: Option<&SubSpec>) -> RwSignal<Slot> {
    spec.map_or_else(
        || RwSignal::new(Slot::Error(UNPARSED.to_string())),
        |s| store.slot(s),
    )
}

/// The readiness of a control bound to `slots`, tracked (for the view; a
/// component keeps it in one `Memo` for its attributes).
pub fn readiness(slots: &[RwSignal<Slot>]) -> Readiness {
    Readiness::all(
        slots
            .iter()
            .map(|s| s.try_with(Readiness::of_slot).unwrap_or(Readiness::Waiting)),
    )
}

/// The same, untracked (in an event handler).
pub fn readiness_now(slots: &[RwSignal<Slot>]) -> Readiness {
    Readiness::all(slots.iter().map(|s| {
        s.try_with_untracked(Readiness::of_slot)
            .unwrap_or(Readiness::Waiting)
    }))
}

/// One layout control.
#[component]
pub fn ControlView(control: Control) -> impl IntoView {
    let settings = expect_context::<Settings>();
    match control {
        Control::Strip(strip) => {
            view! { <StripView strip={*strip} settings=settings /> }.into_any()
        }
        Control::Solo { binding, label } => {
            view! { <SoloView binding=binding label=label /> }.into_any()
        }
        Control::Stage { binding, label, .. } => {
            view! { <StageMicsView binding=binding label=label /> }.into_any()
        }
        Control::HubToggle { key, label } => {
            view! { <HubToggleView key=key label=label /> }.into_any()
        }
        Control::ParamToggle {
            label,
            targets,
            press,
            color,
        } => view! {
            <ParamToggleView label=label targets=targets press=press color=color />
        }
        .into_any(),
        Control::ParamFader { label, targets } => {
            view! { <ParamFaderView label=label targets=targets /> }.into_any()
        }
        Control::Alert {
            binding,
            period_ms,
            label,
            mute_guard,
        } => view! {
            <AlertView binding=binding period_ms=period_ms label=label guarded=mute_guard />
        }
        .into_any(),
        Control::Refresh { label } => view! { <RefreshView label=label /> }.into_any(),
        Control::Text { text } => {
            view! { <div class="text" data-testid="label">{text}</div> }.into_any()
        }
    }
}

/// A button's text: in the rail its font follows its longest word (`--n`,
/// the stylesheet's `.rail .btn-text`), so a word is never broken (#21).
#[component]
pub fn BtnText(text: String) -> impl IntoView {
    let length = format!("--n:{};", longest_word_chars(&text));
    view! { <span class="btn-text" style=length>{text}</span> }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_controls_writes_go_under_its_subscriptions_key_without_the_display() {
        let spec = SubSpec::new(
            "band",
            "live_set tracks[name=Vox 1] mixer_device volume".into(),
            "value",
            true,
        );
        assert_eq!(
            key_of(&spec),
            "band|live_set tracks[name=Vox 1] mixer_device volume|value"
        );
    }

    #[test]
    fn a_pointer_up_or_cancel_ends_a_touch_a_lost_capture_adds_nothing() {
        assert_eq!(touch_end_name("pointerup"), Some("up"));
        assert_eq!(touch_end_name("pointercancel"), Some("cancel"));
        assert_eq!(touch_end_name("lostpointercapture"), None);
        assert_eq!(touch_end_name("pointerdown"), None);
    }

    #[test]
    fn a_controls_readiness_follows_its_slots_tracked_or_not() {
        let value = || Slot::Value {
            value: json!(true),
            display: None,
            at: 0.0,
        };
        let a = RwSignal::new(value());
        let b = RwSignal::new(Slot::Pending);
        for read in [readiness, readiness_now] {
            let _ = b.try_set(Slot::Pending);
            assert_eq!(read(&[a, b]), Readiness::Waiting);
            let _ = b.try_set(value());
            assert_eq!(read(&[a, b]), Readiness::Ready);
            let _ = b.try_set(Slot::Error("gone".into()));
            assert_eq!(read(&[a, b]), Readiness::Unresolved);
        }
    }
}
