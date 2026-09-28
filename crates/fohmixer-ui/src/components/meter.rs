//! The strip meter (spec F13, X2) and the status pill (spec F5, I5): both
//! move only in the frame loop, reading their slots untracked, so Live's
//! meter pushes never re-render the view.

use fohmixer_proto::layout::Frame;
use leptos::html;
use leptos::prelude::*;

use crate::behave::css;
use crate::behave::meter::{MeterBar, level_to_pos};
use crate::behave::status::status_color;
use crate::dom;
use crate::raf;
use crate::stage;
use crate::store::Slot;

/// Registers a frame function once `root` is in the page, and removes it
/// with the component.
fn animate<F>(root: NodeRef<html::Div>, make: F)
where
    F: FnOnce(web_sys::HtmlDivElement) -> raf::FrameFn + 'static,
{
    let frame_id = StoredValue::new(None::<usize>);
    root.on_load(move |el: web_sys::HtmlDivElement| {
        frame_id.set_value(Some(raf::register(make(el))));
    });
    on_cleanup(move || {
        if let Some(Some(id)) = frame_id.try_get_value() {
            raf::unregister(id);
        }
    });
}

/// A meter: one bar per level slot (one for `level`, two for `lr`).
#[component]
pub fn MeterView(frame: Frame, levels: Vec<RwSignal<Slot>>) -> impl IntoView {
    let root = NodeRef::<html::Div>::new();
    let count = levels.len();
    animate(root, move |el| {
        let fills: Vec<Option<web_sys::HtmlElement>> = (0..count)
            .map(|i| dom::child(&el, &format!(".meter-bar:nth-child({}) .meter-fill", i + 1)))
            .collect();
        let mut bars: Vec<MeterBar> = vec![MeterBar::default(); count];
        let mut drawn: Vec<(f64, String)> = vec![(-1.0, String::new()); count];
        Box::new(move |_now: f64, step: f64| {
            for (i, slot) in levels.iter().enumerate() {
                let level = slot
                    .try_with_untracked(Slot::number)
                    .flatten()
                    .unwrap_or(0.0);
                bars[i].target(level_to_pos(level));
                let (pos, color) = bars[i].step(step);
                let color = css(color);
                if (pos - drawn[i].0).abs() < 1e-4 && color == drawn[i].1 {
                    continue;
                }
                if let Some(fill) = &fills[i] {
                    dom::set_style(fill, "transform", &format!("scaleY({pos})"));
                    dom::set_style(fill, "background", &color);
                }
                if i == 0 {
                    dom::set_attr(&el, "data-level", &format!("{pos:.4}"));
                }
                drawn[i] = (pos, color);
            }
        })
    });
    let bars = (0..count)
        .map(|_| {
            view! {
                <div class="meter-bar">
                    <div class="meter-fill"></div>
                </div>
            }
        })
        .collect_view();
    view! {
        <div class="meter" data-testid="meter" data-level="0" style={stage::box_style(frame)} node_ref=root>
            {bars}
        </div>
    }
}

/// The status pill: red while the strip is not bound (a binding does not
/// resolve, or Live's values are not here), yellow after a value, fading to
/// green.
#[component]
pub fn StatusView(
    frame: Frame,
    slots: Vec<RwSignal<Slot>>,
    activity: Vec<RwSignal<Slot>>,
) -> impl IntoView {
    let root = NodeRef::<html::Div>::new();
    animate(root, move |el| {
        let mut drawn = (String::new(), None::<bool>);
        Box::new(move |now: f64, _step: f64| {
            let bound = slots
                .iter()
                .all(|s| s.try_with_untracked(Slot::is_ready).unwrap_or(false));
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
            style={stage::box_style(frame)}
            node_ref=root
        ></div>
    }
}
