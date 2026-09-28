//! A mixer strip (spec F2–F5, F8–F13): the fader, pan, mute, meter, status
//! pill, Live's dB text, the label and the instance label, each where the
//! imported layout put it.

use fohmixer_proto::layout::{Frame, Strip, Style};
use leptos::html;
use leptos::prelude::*;

use super::Settings;
use super::buttons::{MuteView, anchor_name};
use super::fader::{FaderView, Law, Target};
use super::meter::{MeterView, StatusView};
use super::pan::PanView;
use crate::behave::label::strip_label;
use crate::binding::strip_subs;
use crate::dom;
use crate::stage;
use crate::store::{LiveStore, Slot};

/// A text of the strip at its stylesheet size (`.strip-db`, `.strip-label`,
/// `.strip-instance`), shrunk to fit its box (#9: a narrow strip's 39 × 25
/// instance label showed "BANC" for BAND): an effect writes `text` and then
/// fits it, so the fitting always measures the text shown.
#[component]
fn FittedText(
    css_class: &'static str,
    testid: &'static str,
    frame: Frame,
    text: Signal<String>,
) -> impl IntoView {
    let node = NodeRef::<html::Div>::new();
    Effect::new(move |_| {
        let shown = text.get();
        if let Some(el) = node.get() {
            el.set_text_content(Some(&shown));
            dom::fit_text(&el, None);
        }
    });
    view! {
        <div class=css_class data-testid=testid node_ref=node style={stage::box_style(frame)}></div>
    }
}

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
            let targets = vec![Target {
                spec: Some(spec),
                slot,
                law: Law::Volume,
            }];
            view! { <FaderView frame={at(f)} targets=targets shaping={settings.shaping} /> }
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
    // Live's display string exactly as it came (spec P2), fitted, never cut.
    let db = c.db.map(|f| {
        let text = move || {
            volume
                .map(|v| v.with(|s| s.display().unwrap_or_default().to_string()))
                .unwrap_or_default()
        };
        view! {
            <FittedText
                css_class="strip-db"
                testid="db"
                frame={at(f)}
                text={Signal::derive(text)}
            />
        }
    });
    let label = c.label.map(|f| {
        let text = strip_label(&name);
        view! {
            <FittedText
                css_class="strip-label"
                testid="strip-label"
                frame={at(f)}
                text={Signal::stored(text)}
            />
        }
    });
    let instance_label = c.instance_label.map(|f| {
        let text = instance.clone();
        view! {
            <FittedText
                css_class="strip-instance"
                testid="strip-instance"
                frame={at(f)}
                text={Signal::stored(text)}
            />
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
