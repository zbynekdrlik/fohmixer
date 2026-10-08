//! The strip meter (spec F13, X2) and the status light (spec F5, I5): both
//! move only in the frame loop, reading their slots untracked, so Live's
//! meter pushes never re-render the view. A bar is the zone gradient of the
//! scale (green, yellow from −12 dB, red from −3 dB, as TouchOSC's colours)
//! under a cover the loop shrinks to the level, with a peak line held 1.5 s;
//! the clip light over the bars lights at 0 dB and stays lit until tapped
//! (#21).

use leptos::html;
use leptos::prelude::*;

use super::owns_touches;
use crate::behave::css;
use crate::behave::fader::VolumeLaw;
use crate::behave::meter::{MeterBar, level_to_pos};
use crate::behave::peak::{Peak, clips};
use crate::behave::scale::zone_style;
use crate::behave::status::status_color;
use crate::dom;
use crate::raf;
use crate::store::Slot;

/// A meter: one bar per level slot (one for `level`, two for `lr`), on the
/// fader's volume law (#63: the calibration places a level on TouchOSC's
/// scale; the bar shows it where the fader reads the same dB).
#[component]
pub fn MeterView(levels: Vec<RwSignal<Slot>>, law: VolumeLaw) -> impl IntoView {
    let root = NodeRef::<html::Div>::new();
    let count = levels.len();
    let clip = RwSignal::new(false);
    raf::animate(root, move |el| {
        let part = |i: usize, name: &str| {
            dom::child(&el, &format!(".meter-bar:nth-child({}) .{name}", i + 1))
        };
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
                if clips(level) && clip.try_get_untracked() == Some(false) {
                    let _ = clip.try_set(true);
                }
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
                if i == 0 {
                    dom::set_attr(&el, "data-level", &format!("{pos:.4}"));
                    dom::set_attr(&el, "data-peak", &format!("{peak:.4}"));
                }
                drawn[i] = (pos, peak);
            }
        })
    });
    let bars = (0..count)
        .map(|_| {
            view! {
                <div class="meter-bar">
                    <div class="meter-zones"></div>
                    <div class="meter-cover"></div>
                    <div class="meter-peak"></div>
                </div>
            }
        })
        .collect_view();
    let reset = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = clip.try_set(false);
    };
    // The clip light is a tap target: it owns its touches (#43 PR G).
    let no_keys: Vec<String> = Vec::new();
    view! {
        <div
            class="meter"
            data-testid="meter"
            data-level="0"
            data-peak="0"
            style={zone_style(law)}
            node_ref=root
        >
            <div
                class="meter-clip"
                class:on=move || clip.try_get().unwrap_or(false)
                data-testid="clip"
                data-on=move || clip.try_get().unwrap_or(false).to_string()
                use:owns_touches=no_keys
                on:pointerdown=reset
            ></div>
            <div class="meter-bars">{bars}</div>
        </div>
    }
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
