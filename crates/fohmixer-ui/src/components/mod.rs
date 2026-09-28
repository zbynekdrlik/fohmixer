//! The surface's controls (spec §2.5): one generic component per layout
//! item kind, each bound by the general binding form and placed from its
//! canvas frame. Their behaviour lives in `crate::behave` (pure, tested);
//! these components wire pointer events, the store and the animation loop
//! to it. Moving parts are written by the shared frame loop only.

pub mod area;
pub mod buttons;
pub mod fader;
pub mod meter;
pub mod overlay;
pub mod pan;
pub mod params;
pub mod strip;

use fohmixer_proto::layout::{Item, ItemKind, MeterSource};
use leptos::prelude::*;

use crate::binding::SubSpec;
use crate::store::{LiveStore, Readiness, Slot};

use area::{AreaView, LabelView};
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

/// The readiness of a control bound to `slots`, tracked (for the view).
pub fn readiness(slots: &[RwSignal<Slot>]) -> Readiness {
    let current: Vec<Slot> = slots
        .iter()
        .map(|s| s.try_get().unwrap_or(Slot::Pending))
        .collect();
    Readiness::of(&current)
}

/// The same, untracked (in an event handler).
pub fn readiness_now(slots: &[RwSignal<Slot>]) -> Readiness {
    let current: Vec<Slot> = slots
        .iter()
        .map(|s| s.try_get_untracked().unwrap_or(Slot::Pending))
        .collect();
    Readiness::of(&current)
}

/// One placed layout item.
#[component]
pub fn ItemView(item: Item) -> impl IntoView {
    let settings = expect_context::<Settings>();
    let frame = item.frame;
    let z = item.z;
    let style = item.style;
    match item.kind {
        ItemKind::Area { title } => {
            view! { <AreaView frame=frame z=z style=style title=title /> }.into_any()
        }
        ItemKind::Label { text } => {
            view! { <LabelView frame=frame z=z style=style text=text /> }.into_any()
        }
        ItemKind::Strip(strip) => view! {
            <StripView frame=frame z=z style=style strip={*strip} settings=settings />
        }
        .into_any(),
        ItemKind::Solo { binding } => {
            view! { <SoloView frame=frame z=z style=style binding=binding /> }.into_any()
        }
        ItemKind::Stage { binding, .. } => {
            view! { <StageMicsView frame=frame z=z style=style binding=binding /> }.into_any()
        }
        ItemKind::HubToggle { key, label } => view! {
            <HubToggleView frame=frame z=z style=style key=key label=label />
        }
        .into_any(),
        ItemKind::ParamToggle {
            label,
            targets,
            press,
        } => view! {
            <ParamToggleView frame=frame z=z style=style label=label targets=targets press=press />
        }
        .into_any(),
        ItemKind::ParamFader { label, targets } => view! {
            <ParamFaderView frame=frame z=z style=style label=label targets=targets />
        }
        .into_any(),
        ItemKind::Alert { binding, period_ms } => view! {
            <AlertView frame=frame z=z style=style binding=binding period_ms=period_ms />
        }
        .into_any(),
        ItemKind::Refresh { label } => {
            view! { <RefreshView frame=frame z=z style=style label=label /> }.into_any()
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
