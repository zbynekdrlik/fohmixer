//! The LUFS history's graph (#219; its look since #253, the owner's
//! approved mockup `docs/mockups/lufs-history-v3.html`): the glue of
//! `behave::lufs_history`'s graph decisions, split from `detail_lufs.rs`.
//! `LufsGraph` holds the press (a drag pans back in time, a hold shows the
//! cursor) and the layout; `GraphScales` the labels (LUFS left, the
//! fader's dB right), `GraphGrid` the faint lines, `GraphLines` the SVG
//! (the band's clip, the curve and its green copy, the fader's line) and
//! the live end's dot. Every decision is a pure function; nothing here
//! decides.

use std::time::Duration;

use leptos::html;
use leptos::prelude::*;

use super::super::{down_of, owns_touches};
use super::{Lh, offset_at, wall_ms};
use crate::behave::lufs_history::{
    Answers, BOX, Gesture, HOLD_MS, OnDown, Press, Scale, View, band, column_x, cursor_time,
    fader_label, fader_lines, green, lines, live_end, on_down, pct, scale, tick_label, tick_marks,
    x_frac, x_of, y_of,
};
use crate::dom;

/// The band's `clipPath` id (one detail shows at a time).
const BAND_CLIP: &str = "lh-band-clip";

/// A finger on the graph: its press, the view it began on, and the plot's
/// left edge and width (px) at the down.
#[derive(Clone, Copy)]
struct Held {
    press: Press,
    start: View,
    left: f64,
    width: f64,
}

/// The graph: the scale's labels and lines, the time ticks, the line and
/// the fader's, the cursor; a drag pans back in time, a hold shows the
/// cursor.
#[component]
pub(super) fn LufsGraph(lh: Lh) -> impl IntoView {
    let held = StoredValue::new(None::<Held>);
    let hold_timer = StoredValue::new(None::<TimeoutHandle>);
    let plot = NodeRef::<html::Div>::new();
    on_cleanup(move || {
        if let Some(Some(handle)) = hold_timer.try_update_value(Option::take) {
            handle.clear();
        }
    });

    let values = Memo::new(move |_| {
        lh.cols
            .try_with(|c| c.iter().map(|c| c.v).collect::<Vec<_>>())
            .unwrap_or_default()
    });
    let graph_scale = Memo::new(move |_| scale(lh.target, &values.try_get().unwrap_or_default()));
    let line = Memo::new(move |_| {
        let s = graph_scale
            .try_get()
            .unwrap_or_else(|| scale(lh.target, &[]));
        lines(&values.try_get().unwrap_or_default(), &s)
    });
    let x_parts = Memo::new(move |_| {
        let (t0, t1) = lh.window.try_get().unwrap_or((0, 1));
        tick_marks(t0, t1, offset_at)
            .into_iter()
            .map(|(t, text)| (format!("left:{}", pct(x_of(t, t0, t1))), text))
            .collect::<Vec<_>>()
    });
    let x_labels = move || {
        x_parts
            .try_get()
            .unwrap_or_default()
            .into_iter()
            .map(|(left, text)| view! { <span style=left>{text}</span> })
            .collect_view()
    };
    let drawn = move || line.try_with(Vec::len).unwrap_or(0);
    let drawn_any = move || drawn() > 0;
    let answered = move || !lh.answers.try_with(Answers::failed).unwrap_or(false);
    let cursor_mark = move || {
        let t = lh.cursor.try_get().flatten()?;
        let (t0, t1) = lh.window.try_get()?;
        let left = format!("left:{}", pct(x_of(t.clamp(t0, t1), t0, t1)));
        Some(view! { <div class="lh-cursor-line" data-testid="lufs-cursor" style=left></div> })
    };

    // A finger on the graph.
    let time_at = move |x: f64, h: &Held| {
        let (t0, t1) = lh.window.try_get_untracked().unwrap_or((0, 1));
        cursor_time(x_frac(x, h.left, h.width), t0, t1)
    };
    let end_press = move |why: &str| {
        if let Some(Some(handle)) = hold_timer.try_update_value(Option::take) {
            handle.clear();
        }
        let Some(Some(h)) = held.try_update_value(Option::take) else {
            return;
        };
        let _ = lh.cursor.try_set(None);
        dom::log(&format!(
            "lufs history: press {why} (dragged {}, held {})",
            h.press.dragged(),
            h.press.holding()
        ));
        if h.press.dragged()
            && let Some(v) = lh.shown.try_get_untracked()
        {
            lh.show(v, "drag");
        }
    };
    let graph_keys: Vec<String> = Vec::new();
    let on_down_ev = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let down = down_of(&ev);
        let act = held
            .try_with_value(|h| on_down(h.as_ref().map(|h| &h.press), down))
            .unwrap_or(OnDown::Ignore);
        match act {
            OnDown::Ignore => return,
            OnDown::Restart => end_press("lost"),
            OnDown::Start => {}
        }
        let (Some(area), Some(start)) = (plot.get_untracked(), lh.shown.try_get_untracked()) else {
            return;
        };
        let rect = area.get_bounding_client_rect();
        let x = f64::from(ev.client_x());
        let _ = held.try_set_value(Some(Held {
            press: Press::down(x, dom::now(), down),
            start,
            left: rect.left(),
            width: rect.width(),
        }));
        if let Some(el) = dom::current_element(&ev) {
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        let timer = set_timeout_with_handle(
            move || {
                let _ = hold_timer.try_set_value(None);
                let holds = held
                    .try_update_value(|h| h.as_mut().is_some_and(|h| h.press.check(dom::now())))
                    .unwrap_or(false);
                if holds && let Some(Some(h)) = held.try_get_value() {
                    let _ = lh.cursor.try_set(Some(time_at(h.press.x0(), &h)));
                    dom::log("lufs history: hold, cursor shown");
                }
            },
            Duration::from_millis(HOLD_MS as u64 + 20),
        );
        let _ = hold_timer.try_set_value(timer.ok());
    };
    let on_move = move |ev: web_sys::PointerEvent| {
        let x = f64::from(ev.client_x());
        let Some(Some((gesture, h))) = held.try_update_value(|h| {
            let h = h.as_mut().filter(|h| h.press.is(ev.pointer_id()))?;
            Some((h.press.moved(x), *h))
        }) else {
            return;
        };
        match gesture {
            Gesture::Still => {}
            Gesture::Pan(dx) => {
                let _ = lh
                    .shown
                    .try_set(View::panned(h.start, dx, h.width, wall_ms()));
            }
            Gesture::Cursor(x) => {
                let _ = lh.cursor.try_set(Some(time_at(x, &h)));
            }
        }
    };
    let ours = move |ev: &web_sys::PointerEvent| {
        held.try_with_value(|h| h.is_some_and(|h| h.press.is(ev.pointer_id())))
            .unwrap_or(false)
    };
    let on_up = move |ev: web_sys::PointerEvent| {
        if ours(&ev) {
            end_press("up");
        }
    };
    let on_cancel = move |ev: web_sys::PointerEvent| {
        if ours(&ev) {
            end_press("cancel");
        }
    };
    let on_lost = move |ev: web_sys::PointerEvent| {
        if ours(&ev) {
            end_press("lost");
        }
    };

    view! {
        <div
            class="lh-graph"
            data-testid="lufs-graph"
            data-runs=move || drawn().to_string()
            use:owns_touches=graph_keys
            on:pointerdown=on_down_ev
            on:pointermove=on_move
            on:pointerup=on_up
            on:pointercancel=on_cancel
            on:lostpointercapture=on_lost
        >
            <GraphScales graph_scale=graph_scale />
            <div class="lh-plot" node_ref=plot>
                <GraphGrid graph_scale=graph_scale x_parts=x_parts />
                <GraphLines lh=lh values=values graph_scale=graph_scale line=line />
                {cursor_mark}
                <div class="lh-empty" data-testid="lufs-history-empty" hidden=drawn_any>
                    "NO LUFS IN THIS WINDOW"
                </div>
                <div class="lh-failed" data-testid="lufs-history-failed" hidden=answered>
                    "NO ANSWER FROM THE HUB"
                </div>
            </div>
            <div class="lh-x" aria-hidden="true">
                {x_labels}
            </div>
            <div class="lh-axis" data-side="left" data-testid="lufs-axis" aria-hidden="true">
                "LUFS"
            </div>
            <div class="lh-axis" data-side="right" data-testid="lufs-fader-axis" aria-hidden="true">
                "FADER dB"
            </div>
        </div>
    }
}

/// The scale's labels: LUFS every 3 LU at the left (the target's
/// brighter), the fader's dB at the same ticks at the right (#253).
#[component]
fn GraphScales(graph_scale: Memo<Scale>) -> impl IntoView {
    let y_labels = move || {
        let s = graph_scale.try_get()?;
        Some(
            s.ticks
                .iter()
                .map(|v| {
                    let main = s.target == Some(*v);
                    let top = format!("top:{}", pct(y_of(*v, &s)));
                    view! {
                        <span data-testid="lufs-y" data-main=main.to_string() style=top>
                            {tick_label(*v)}
                        </span>
                    }
                })
                .collect_view(),
        )
    };
    let fader_labels = move || {
        let s = graph_scale.try_get()?;
        Some(
            s.ticks
                .iter()
                .map(|v| {
                    let top = format!("top:{}", pct(y_of(*v, &s)));
                    view! {
                        <span data-testid="lufs-fader-y" style=top>
                            {fader_label(*v, &s)}
                        </span>
                    }
                })
                .collect_view(),
        )
    };
    view! {
        <div class="lh-y" aria-hidden="true">
            {y_labels}
        </div>
        <div class="lh-yr" aria-hidden="true">
            {fader_labels}
        </div>
    }
}

/// The faint lines: one per scale tick (the target's brighter), the band's
/// two dashed ones, and one per time tick.
#[component]
fn GraphGrid(graph_scale: Memo<Scale>, x_parts: Memo<Vec<(String, String)>>) -> impl IntoView {
    let y_lines = move || {
        let s = graph_scale.try_get()?;
        let band_lines = band(&s).map(|(top, bottom)| {
            view! {
                <div class="lh-band" style=format!("top:{}", pct(top))></div>
                <div class="lh-band" style=format!("top:{}", pct(bottom))></div>
            }
        });
        let lines = s
            .ticks
            .iter()
            .map(|v| {
                let main = s.target == Some(*v);
                let top = format!("top:{}", pct(y_of(*v, &s)));
                view! { <div class="lh-hline" data-main=main.to_string() style=top></div> }
            })
            .collect_view();
        Some(view! { {lines} {band_lines} })
    };
    let x_lines = move || {
        x_parts
            .try_get()
            .unwrap_or_default()
            .into_iter()
            .map(|(left, _)| view! { <div class="lh-vline" style=left></div> })
            .collect_view()
    };
    view! { {y_lines} {x_lines} }
}

/// The SVG: the band's clip, each run's curve and (with a band) its green
/// copy clipped to it, the fader's line; then the live end's dot.
#[component]
fn GraphLines(
    lh: Lh,
    values: Memo<Vec<Option<i16>>>,
    graph_scale: Memo<Scale>,
    line: Memo<Vec<String>>,
) -> impl IntoView {
    let band_now = Memo::new(move |_| graph_scale.try_with(band).flatten());
    let fader = Memo::new(move |_| {
        let s = graph_scale
            .try_get()
            .unwrap_or_else(|| scale(lh.target, &[]));
        fader_lines(&lh.cols.try_get().unwrap_or_default(), &s)
    });
    let clip = move || {
        let (top, bottom) = band_now.try_get().flatten()?;
        let height = format!("{:.1}", bottom - top);
        Some(view! {
            <defs>
                <clipPath id=BAND_CLIP>
                    <rect x="0" y=format!("{top:.1}") width=BOX.to_string() height=height></rect>
                </clipPath>
            </defs>
        })
    };
    let paths = move || {
        let banded = band_now.try_get().flatten().is_some();
        line.try_get()
            .unwrap_or_default()
            .into_iter()
            .map(|d| {
                let green_copy = banded.then(|| {
                    let d = d.clone();
                    view! {
                        <path
                            class="lh-line"
                            data-testid="lufs-line-green"
                            data-green="true"
                            clip-path=format!("url(#{BAND_CLIP})")
                            d=d
                        ></path>
                    }
                });
                view! {
                    <path class="lh-line" data-testid="lufs-line" data-green="false" d=d></path>
                    {green_copy}
                }
            })
            .collect_view()
    };
    let fader_paths = move || {
        fader
            .try_get()
            .unwrap_or_default()
            .into_iter()
            .map(|points| {
                view! { <polyline class="lh-fader-line" data-testid="lufs-fader-line" points=points></polyline> }
            })
            .collect_view()
    };
    let dot = move || {
        let live = lh.shown.try_with(View::is_live).unwrap_or(false);
        let vals = values.try_get()?;
        let (i, v) = live_end(live, &vals)?;
        let s = graph_scale.try_get()?;
        let place = format!(
            "left:{};top:{}",
            pct(column_x(i, vals.len())),
            pct(y_of(i32::from(v), &s))
        );
        let inside = green(v, lh.target).to_string();
        Some(view! { <div class="lh-dot" data-testid="lufs-dot" data-in=inside style=place></div> })
    };
    let view_box = format!("0 0 {BOX} {BOX}");
    view! {
        <svg class="lh-svg" viewBox=view_box preserveAspectRatio="none" aria-hidden="true">
            {clip}
            {paths}
            {fader_paths}
        </svg>
        {dot}
    }
}
