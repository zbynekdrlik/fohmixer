//! A mixer strip (spec F2–F5, F8–F13): the fader, pan, mute, meter, status
//! pill, Live's dB text, the label and the instance label, each where the
//! imported layout put it.

use fohmixer_proto::layout::{Frame, Strip, Style};
use leptos::prelude::*;

use super::Settings;
use super::buttons::{MuteView, anchor_name};
use super::fader::{FaderView, Law};
use super::meter::{MeterView, StatusView};
use super::pan::PanView;
use crate::behave::label::strip_label;
use crate::binding::strip_subs;
use crate::stage;
use crate::store::{LiveStore, Slot};

/// One strip.
#[component]
pub fn StripView(
    frame: Frame,
    z: i64,
    style: Style,
    strip: Strip,
    settings: Settings,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let subs = strip_subs(&strip, settings.meter_source);
    let volume = subs.volume.as_ref().map(|s| store.slot(s));
    let pan = subs.pan.as_ref().map(|s| store.slot(s));
    let mute = subs.mute.as_ref().map(|s| store.slot(s));
    let meters: Vec<RwSignal<Slot>> = subs.meters.iter().map(|s| store.slot(s)).collect();
    let all: Vec<RwSignal<Slot>> = [volume, pan, mute]
        .into_iter()
        .flatten()
        .chain(meters.iter().copied())
        .collect();
    let activity: Vec<RwSignal<Slot>> = volume.into_iter().chain(meters.iter().copied()).collect();
    let name = anchor_name(&strip.binding);
    let instance = strip.binding.instance.clone();
    let c = strip.children.clone();
    let at = move |f: Frame| stage::relative(f, frame);

    let fader = c
        .fader
        .zip(volume)
        .zip(subs.volume.clone())
        .map(|((f, slot), spec)| {
            view! {
                <FaderView
                    frame={at(f)}
                    state=slot
                    targets={vec![(spec, Law::Volume)]}
                    shaping={settings.shaping}
                />
            }
        });
    let pan_view = c
        .pan
        .zip(pan)
        .zip(subs.pan.clone())
        .map(|((f, slot), spec)| view! { <PanView frame={at(f)} state=slot spec=spec /> });
    let mute_view = c
        .mute
        .zip(mute)
        .zip(subs.mute.clone())
        .map(|((f, slot), spec)| {
            view! { <MuteView frame={at(f)} state=slot spec=spec guarded={strip.mute_guard} /> }
        });
    let meter = c
        .meter
        .map(|f| view! { <MeterView frame={at(f)} levels={meters.clone()} /> });
    let status = c.status.map(
        |f| view! { <StatusView frame={at(f)} slots={all.clone()} activity={activity.clone()} /> },
    );
    let db = c.db.map(|f| {
        let text = move || {
            volume
                .map(|v| v.with(|s| s.display().unwrap_or_default().to_string()))
                .unwrap_or_default()
        };
        view! {
            <div class="strip-db" data-testid="db" style={stage::box_style(at(f))}>
                {text}
            </div>
        }
    });
    let label = c.label.map(|f| {
        view! {
            <div class="strip-label" data-testid="strip-label" style={stage::box_style(at(f))}>
                {strip_label(&name)}
            </div>
        }
    });
    let instance_label = c.instance_label.map(|f| {
        view! {
            <div class="strip-instance" data-testid="strip-instance" style={stage::box_style(at(f))}>
                {instance.clone()}
            </div>
        }
    });
    let kind = format!("{:?}", strip.strip_kind).to_lowercase();
    view! {
        <div
            class="item strip"
            data-testid="strip"
            data-track={name.clone()}
            data-instance={strip.binding.instance.clone()}
            data-kind=kind
            style={stage::item_style(frame, z, &style)}
        >
            {fader}
            {pan_view}
            {meter}
            {mute_view}
            {status}
            {db}
            {label}
            {instance_label}
        </div>
    }
}
