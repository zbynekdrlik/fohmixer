//! The meters (spec F13, X2) and the status light (spec F5, I5): all move
//! only in the frame loop, reading their slots untracked, so Live's meter
//! pushes never re-render the view. A bar is the zone gradient of the
//! scale (green, yellow from −12 dB, red from −3 dB, as TouchOSC's colours)
//! under a cover the loop shrinks to the level, with a peak line held 1.5 s.
//! A strip's meter is the stereo pair in its fader's track (#151,
//! `TrackMeter`); the channel detail keeps `MeterView` beside its fader.
//! No clip light (#151: Live mixes in 32-bit float, a track cannot clip
//! inside it).

use leptos::html;
use leptos::prelude::*;

use crate::behave::css;
use crate::behave::fader::VolumeLaw;
use crate::behave::meter::{MeterBar, bar_sources, level_to_pos};
use crate::behave::peak::Peak;
use crate::behave::scale::zone_style;
use crate::behave::status::status_color;
use crate::dom;
use crate::raf;
use crate::store::Slot;

/// Moves the bars under `root` (its `.meter-bar` children in order, at any
/// depth) in the frame loop: bar `i` reads `levels[i]` (one slot may feed
/// two bars), on the fader's volume law. Each bar carries its own
/// `data-level` / `data-peak`; `root` carries the first bar's.
fn animate_bars(root: NodeRef<html::Div>, levels: Vec<RwSignal<Slot>>, law: VolumeLaw) {
    let count = levels.len();
    raf::animate(root, move |el| {
        let bar = |i: usize| dom::child(&el, &format!(".meter-bar:nth-child({})", i + 1));
        let part = |i: usize, name: &str| {
            dom::child(&el, &format!(".meter-bar:nth-child({}) .{name}", i + 1))
        };
        let boxes: Vec<Option<web_sys::HtmlElement>> = (0..count).map(bar).collect();
        let covers: Vec<Option<web_sys::HtmlElement>> =
            (0..count).map(|i| part(i, "meter-cover")).collect();
        let lines: Vec<Option<web_sys::HtmlElement>> =
            (0..count).map(|i| part(i, "meter-peak")).collect();
        let mut bars: Vec<MeterBar> = vec![MeterBar::default(); count];
        let mut peaks: Vec<Peak> = vec![Peak::default(); count];
        let mut drawn: Vec<(f64, f64)> = vec![(-1.0, -1.0); count];
        Box::new(move |now: f64, step: f64| {
            for (i, slot) in levels.iter().enumerate() {
                // A stale level (the hub connection was lost) is no level:
                // the meter falls instead of freezing (#43).
                let level = slot
                    .try_with_untracked(Slot::fresh_number)
                    .flatten()
                    .unwrap_or(0.0);
                bars[i].target(level_to_pos(level));
                let (shown, _color) = bars[i].step(step);
                let pos = law.from_touchosc(shown);
                let peak = peaks[i].step(pos, now);
                let (was, was_peak) = drawn[i];
                if (pos - was).abs() < 1e-4 && (peak - was_peak).abs() < 1e-4 {
                    continue;
                }
                if let Some(cover) = &covers[i] {
                    dom::set_style(cover, "transform", &format!("scaleY({})", 1.0 - pos));
                }
                if let Some(line) = &lines[i] {
                    dom::set_style(line, "--pk", &format!("{peak:.4}"));
                    dom::set_style(line, "opacity", if peak > 0.001 { "0.75" } else { "0" });
                }
                if let Some(b) = &boxes[i] {
                    dom::set_attr(b, "data-level", &format!("{pos:.4}"));
                    dom::set_attr(b, "data-peak", &format!("{peak:.4}"));
                }
                if i == 0 {
                    dom::set_attr(&el, "data-level", &format!("{pos:.4}"));
                    dom::set_attr(&el, "data-peak", &format!("{peak:.4}"));
                }
                drawn[i] = (pos, peak);
            }
        })
    });
}

/// One meter bar's parts.
fn bar_view() -> impl IntoView {
    view! {
        <div class="meter-bar" data-testid="meter-bar" data-level="0" data-peak="0">
            <div class="meter-zones"></div>
            <div class="meter-cover"></div>
            <div class="meter-peak"></div>
        </div>
    }
}

/// A meter beside a fader (the channel detail's): one bar per level slot
/// (one for `level`, two for `lr`), on the fader's volume law (#63: the
/// calibration places a level on TouchOSC's scale; the bar shows it where
/// the fader reads the same dB).
#[component]
pub fn MeterView(levels: Vec<RwSignal<Slot>>, law: VolumeLaw) -> impl IntoView {
    let root = NodeRef::<html::Div>::new();
    let count = levels.len();
    animate_bars(root, levels, law);
    let bars = (0..count).map(|_| bar_view()).collect_view();
    view! {
        <div
            class="meter"
            data-testid="meter"
            data-level="0"
            data-peak="0"
            style={zone_style(law)}
            node_ref=root
        >
            <div class="meter-bars">{bars}</div>
        </div>
    }
}

/// A strip's meter in its fader's track (#151, mockup v8): two bars, L and
/// R, under the cap; with the `level` source both read the one value
/// (`behave::meter::bar_sources`), with `lr` one channel each. No level
/// slot, no track meter.
#[component]
pub fn TrackMeter(levels: Vec<RwSignal<Slot>>, law: VolumeLaw) -> impl IntoView {
    let fed: Option<Vec<RwSignal<Slot>>> =
        bar_sources(levels.len()).map(|sources| sources.iter().map(|&i| levels[i]).collect());
    fed.map(|fed| {
        let root = NodeRef::<html::Div>::new();
        animate_bars(root, fed, law);
        view! {
            <div
                class="fader-track"
                data-testid="meter"
                data-level="0"
                data-peak="0"
                style={zone_style(law)}
                node_ref=root
            >
                {bar_view()}
                {bar_view()}
            </div>
        }
    })
}

/// The status light: red while the strip is not bound (a binding does not
/// resolve, or Live's values are not here or only stale ones are, #43),
/// yellow after a value, fading to green.
#[component]
pub fn StatusView(slots: Vec<RwSignal<Slot>>, activity: Vec<RwSignal<Slot>>) -> impl IntoView {
    let root = NodeRef::<html::Div>::new();
    raf::animate(root, move |el| {
        let mut drawn = (String::new(), None::<bool>);
        Box::new(move |now: f64, _step: f64| {
            let bound = slots
                .iter()
                .all(|s| s.try_with_untracked(Slot::is_fresh).unwrap_or(false));
            let last = activity
                .iter()
                .filter_map(|s| s.try_with_untracked(Slot::at).flatten())
                .fold(f64::NEG_INFINITY, f64::max);
            let color = css(status_color(bound, now - last));
            if drawn.0 != color {
                dom::set_style(&el, "background", &color);
                drawn.0 = color;
            }
            if drawn.1 != Some(bound) {
                let state = if bound { "bound" } else { "unbound" };
                dom::set_attr(&el, "data-state", state);
                drawn.1 = Some(bound);
            }
        })
    });
    view! {
        <div
            class="status"
            data-testid="status"
            data-state="unbound"
            node_ref=root
        ></div>
    }
}
