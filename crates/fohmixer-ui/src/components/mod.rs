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
