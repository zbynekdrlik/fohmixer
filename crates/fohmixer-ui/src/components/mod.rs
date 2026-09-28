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

use fohmixer_proto::layout::{Control, MeterSource};
use leptos::prelude::*;

use crate::binding::SubSpec;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
