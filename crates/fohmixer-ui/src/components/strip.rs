//! A mixer strip (spec F2–F5, F8–F13; the redesign, #21): the pan bar, the
//! instance tag, the fader zone (the dB scale, the meter, the fader), the
//! status light with Live's dB readout, and the name button that is the
//! mute, lit in the track's Live colour while the track is audible.

use fohmixer_proto::layout::{Strip, StripKind};
use leptos::prelude::*;

use super::Settings;
use super::buttons::{MuteView, anchor_name};
use super::fader::{FaderView, Law, Target};
use super::meter::{MeterView, StatusView};
use super::pan::PanView;
use crate::behave::db_text::db_text;
use crate::behave::label::strip_label;
use crate::behave::scale::{TICKS, tick_label, tick_pos};
use crate::binding::strip_subs;
use crate::store::{LiveStore, Slot};

/// The dB scale beside a fader: TouchOSC's labels at the fader positions of
/// their levels.
#[component]
pub fn ScaleView() -> impl IntoView {
    let ticks = TICKS
        .iter()
        .map(|db| {
            let top = format!("top:{:.3}%;", (1.0 - tick_pos(*db)) * 100.0);
            let unity = *db == 0.0;
            view! {
                <span class="tick" class:unity=unity data-db=db.to_string() style=top>
                    {tick_label(*db)}
                </span>
            }
        })
        .collect_view();
    view! { <div class="scale" data-testid="scale" aria-hidden="true">{ticks}</div> }
}

/// Live's dB readout in TouchOSC's form (one decimal, no unit, white exactly
/// at 0 dB).
#[component]
pub fn DbView(state: RwSignal<Slot>) -> impl IntoView {
    let text = Memo::new(move |_| state.with(|s| s.display().and_then(db_text)));
    let shown = move || text.get().map(|t| t.text).unwrap_or_default();
    let unity = move || text.get().is_some_and(|t| t.unity);
    view! {
        <span class="db" class:unity=unity data-testid="db">{shown}</span>
    }
}

/// One strip.
#[component]
pub fn StripView(strip: Strip, settings: Settings) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let subs = strip_subs(&strip, settings.meter_source);
    let volume = subs.volume.as_ref().map(|s| store.slot(s));
    let pan = subs.pan.as_ref().map(|s| store.slot(s));
    let mute = subs.mute.as_ref().map(|s| store.slot(s));
    let color = subs.color.as_ref().map(|s| store.slot(s));
    let meters: Vec<RwSignal<Slot>> = subs.meters.iter().map(|s| store.slot(s)).collect();
    // The status light: every value the strip shows (the colour never
    // gates it), and activity from the volume and the meters.
    let all: Vec<RwSignal<Slot>> = [volume, pan, mute]
        .into_iter()
        .flatten()
        .chain(meters.iter().copied())
        .collect();
    let activity: Vec<RwSignal<Slot>> = volume.into_iter().chain(meters.iter().copied()).collect();
    let name = anchor_name(&strip.binding);
    let instance = strip.binding.instance.clone();

    let fader = volume.zip(subs.volume.clone()).map(|(slot, spec)| {
        let targets = vec![Target {
            spec: Some(spec),
            slot,
            law: Law::Volume,
        }];
        view! { <FaderView targets=targets shaping={settings.shaping} /> }
    });
    let pan_view = pan
        .zip(subs.pan.clone())
        .map(|(slot, spec)| view! { <PanView state=slot spec=spec /> });
    let mute_view = mute.zip(subs.mute.clone()).map(|(slot, spec)| {
        view! {
            <MuteView
                state=slot
                spec=spec
                guarded={strip.mute_guard}
                label={strip_label(&name)}
                color=color
            />
        }
    });
    let db = volume.map(|slot| view! { <DbView state=slot /> });
    let tag = match strip.strip_kind {
        StripKind::Return => format!("{instance} · ret"),
        StripKind::Standard => instance.clone(),
    };
    let kind = format!("{:?}", strip.strip_kind).to_lowercase();
    // Each attribute its own copy (the macro may move a value into a child
    // before an attribute reads it).
    let strip_instance = instance.clone();
    let tag_instance = instance;
    let wide = strip.wide;
    view! {
        <div
            class="strip"
            class:wide=wide
            data-testid="strip"
            data-track=name
            data-instance=strip_instance
            data-kind=kind
        >
            {pan_view}
            <span class="strip-tag" data-testid="strip-instance" data-instance=tag_instance>
                {tag}
            </span>
            <div class="strip-fz">
                <ScaleView />
                <MeterView levels={meters.clone()} />
                {fader}
            </div>
            <div class="strip-dbrow">
                <StatusView slots=all activity=activity />
                {db}
            </div>
            {mute_view}
        </div>
    }
}
