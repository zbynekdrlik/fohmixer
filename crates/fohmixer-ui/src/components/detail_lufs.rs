//! The channel detail's middle-box tabs and its LUFS history (#219, the
//! design on the ticket; the owner's approved mockup
//! `docs/mockups/lufs-history-v2.html`): `EQ` (the Pro-Q 4 cards) and, on a
//! `+LUFS` strip, `LUFS`, each filling the whole box. The LUFS tab draws the
//! point's short-term reading over a window (`10 MIN`, `1 H`, `3 H`, a drag
//! back in time, a past day), read from the hub's `/api/lufs/history` and
//! `/api/lufs/days`, its live edge from the page's own readings once a
//! second. Every decision is `behave::lufs_history`; this is the glue.

use std::time::Duration;

use fohmixer_proto::layout::Strip;
use fohmixer_proto::lufs::{HistoryDay, HistorySeries, caption};
use leptos::html;
use leptos::prelude::*;

use super::owns_touches;
use crate::behave::lufs::{look, point_key};
use crate::behave::lufs_history::{
    BOX, COLUMNS, DEFAULT_SPAN_MS, Gesture, HOLD_MS, MINUTE_MS, Press, SECOND_MS, Tab, View,
    WINDOWS, band, column_at, columns, cursor_text, day_label, hh_mm, keep_tail, local_day, runs,
    samples, scale, series_end, shown_tab, tab_row, target_text, tick_label, time_ticks, x_of,
    y_of,
};
use crate::dom;
use crate::store::LiveStore;

/// The middle box's tab and the history's window, kept per page (a context
/// of the surface, beside `DetailStrip`): the next detail opens on them.
#[derive(Clone, Copy)]
pub struct DetailMiddle {
    pub tab: RwSignal<Tab>,
    pub span: RwSignal<i64>,
}

impl DetailMiddle {
    /// `EQ` and `10 MIN`, as a page starts.
    pub fn new() -> Self {
        DetailMiddle {
            tab: RwSignal::new(Tab::Eq),
            span: RwSignal::new(DEFAULT_SPAN_MS),
        }
    }
}

impl Default for DetailMiddle {
    fn default() -> Self {
        Self::new()
    }
}

/// The local clock's offset from UTC at `t` (ms; local minus UTC).
fn offset_at(t: i64) -> i64 {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(t as f64));
    -(date.get_timezone_offset() as i64) * MINUTE_MS
}

/// The wall clock, whole ms.
fn wall_ms() -> i64 {
    dom::wall_now() as i64
}

/// The detail's middle box: the tab row (only with two tabs) and the panes;
/// the EQ pane stays mounted (its cards keep their pictures), the LUFS pane
/// only while shown.
#[component]
pub fn DetailMiddleBox(strip: Strip, children: Children) -> impl IntoView {
    let middle = use_context::<DetailMiddle>().unwrap_or_default();
    let has_lufs = point_key(&strip).is_some();
    let shown = Memo::new(move |_| shown_tab(middle.tab.try_get().unwrap_or(Tab::Eq), has_lufs));
    let tab_name = move || shown.try_get().unwrap_or(Tab::Eq).name();
    let eq_hidden = move || shown.try_get().unwrap_or(Tab::Eq) != Tab::Eq;
    let row = tab_row(has_lufs).then(|| view! { <MiddleTabs shown=shown /> });
    let lufs_strip = StoredValue::new(strip);
    let lufs_pane = move || {
        let strip = lufs_strip.try_get_value()?;
        (shown.try_get() == Some(Tab::Lufs)).then(|| {
            view! {
                <div class="detail-pane lh-pane" data-pane="lufs" data-testid="detail-pane-lufs">
                    <DetailLufs strip=strip />
                </div>
            }
        })
    };
    view! {
        <section
            class="detail-box detail-middle"
            data-testid="detail-middle"
            data-tabs=has_lufs.to_string()
            data-tab=tab_name
        >
            {row}
            <div class="detail-pane eq-pane" data-pane="eq" data-testid="detail-pane-eq" hidden=eq_hidden>
                {children()}
            </div>
            {lufs_pane}
        </section>
    }
}

/// `EQ | LUFS`: a tap shows its pane and keeps it for the page.
#[component]
fn MiddleTabs(shown: Memo<Tab>) -> impl IntoView {
    let middle = use_context::<DetailMiddle>().unwrap_or_default();
    let tabs = [Tab::Eq, Tab::Lufs].map(|tab| {
        let keys: Vec<String> = Vec::new();
        let on = move || (shown.try_get() == Some(tab)).to_string();
        let down = move |ev: web_sys::PointerEvent| {
            ev.prevent_default();
            dom::log(&format!("detail middle: tab {}", tab.name()));
            let _ = middle.tab.try_set(tab);
        };
        view! {
            <button
                type="button"
                class="mtab"
                data-testid="detail-tab"
                data-tab=tab.name()
                aria-pressed=on
                use:owns_touches=keys
                on:pointerdown=down
            >
                {tab.label()}
            </button>
        }
    });
    view! { <div class="mtabs" data-testid="detail-tabs">{tabs.into_iter().collect_view()}</div> }
}

/// A finger on the graph: its press, the view it began on, the plot's left
/// edge and width (px) at the down, its pointer.
#[derive(Clone, Copy)]
struct Held {
    press: Press,
    start: View,
    left: f64,
    width: f64,
    pointer: i32,
}

/// The LUFS history of a `+LUFS` strip's point (mockup v2's LUFS tab).
#[component]
pub fn DetailLufs(strip: Strip) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let middle = use_context::<DetailMiddle>().unwrap_or_default();
    let spec = strip.lufs;
    let target = spec.and_then(|s| s.target);
    let Some((instance, path)) = point_key(&strip) else {
        return ().into_any();
    };
    let point = StoredValue::new((instance, path));

    let now = RwSignal::new(wall_ms());
    let shown = RwSignal::new(View::live(
        middle.span.try_get_untracked().unwrap_or(DEFAULT_SPAN_MS),
    ));
    let series = RwSignal::new(HistorySeries::default());
    let tail = RwSignal::new(Vec::<(i64, Option<i16>)>::new());
    let days = RwSignal::new(Vec::<HistoryDay>::new());
    let cursor = RwSignal::new(None::<i64>);
    let asked = StoredValue::new(0u64);
    let held = StoredValue::new(None::<Held>);
    let hold_timer = StoredValue::new(None::<TimeoutHandle>);
    let ticker = StoredValue::new(None::<IntervalHandle>);
    let plot = NodeRef::<html::Div>::new();

    // The hub's series for the window shown; an older answer than the last
    // ask is dropped.
    let fetch = move || {
        let Some(v) = shown.try_get_untracked() else {
            return;
        };
        let (t0, t1) = v.window(wall_ms());
        let Some(n) = asked.try_update_value(|a| {
            *a += 1;
            *a
        }) else {
            return;
        };
        let Some((instance, path)) = point.try_get_value() else {
            return;
        };
        dom::log(&format!(
            "lufs history: ask {instance} {path} {t0}..{t1} (live {})",
            v.is_live()
        ));
        leptos::task::spawn_local(async move {
            let Some(got) = store.lufs_history(&instance, &path, t0, t1).await else {
                return;
            };
            if asked.try_get_value() != Some(n) {
                return;
            }
            let end = series_end(&got);
            let _ = tail.try_update(|t| keep_tail(t, end));
            let _ = series.try_set(got);
        });
    };
    let fetch_days = move || {
        let Some((instance, path)) = point.try_get_value() else {
            return;
        };
        leptos::task::spawn_local(async move {
            if let Some(got) = store.lufs_days(&instance, &path).await {
                let _ = days.try_set(got);
            }
        });
    };
    fetch();
    fetch_days();

    // Once a second: the clock, the live edge from the page's own reading,
    // and every minute the hub's series and days again.
    let ticks = StoredValue::new(0u32);
    let tick = move || {
        let t = wall_ms();
        let _ = now.try_set(t);
        let Some(v) = shown.try_get_untracked() else {
            return;
        };
        if v.is_live() {
            let reading = point
                .try_with_value(|(instance, path)| {
                    store
                        .lufs
                        .try_with_untracked(|l| l.point(instance, path).tenths)
                })
                .flatten()
                .flatten();
            let second = t - t.rem_euclid(SECOND_MS);
            let (t0, _) = v.window(t);
            let _ = tail.try_update(|tl| {
                tl.push((second, reading));
                keep_tail(tl, t0);
            });
        }
        let n = ticks.try_update_value(|n| {
            *n += 1;
            *n
        });
        let minute = n.is_some_and(|n| n % 60 == 0);
        if minute && v.is_live() {
            fetch();
        }
        // The days once a minute; every 5 s while none is known yet (a
        // point whose first minute had no reading at the open).
        let none_yet = days.try_with_untracked(Vec::is_empty).unwrap_or(false);
        if minute || (none_yet && n.is_some_and(|n| n % 5 == 0)) {
            fetch_days();
        }
    };
    if let Ok(handle) = set_interval_with_handle(tick, Duration::from_millis(SECOND_MS as u64)) {
        let _ = ticker.try_set_value(Some(handle));
    }
    on_cleanup(move || {
        if let Some(Some(handle)) = ticker.try_update_value(Option::take) {
            handle.clear();
        }
        if let Some(Some(handle)) = hold_timer.try_update_value(Option::take) {
            handle.clear();
        }
    });

    // What is drawn.
    let window = Memo::new(move |_| {
        let t = now.try_get().unwrap_or(0);
        shown
            .try_get()
            .unwrap_or(View::live(DEFAULT_SPAN_MS))
            .window(t)
    });
    let cols = Memo::new(move |_| {
        let (t0, t1) = window.try_get().unwrap_or((0, 1));
        let mut all = series.try_with(samples).unwrap_or_default();
        all.extend(tail.try_get().unwrap_or_default());
        columns(&all, t0, t1)
    });
    let graph_scale = Memo::new(move |_| scale(target, &cols.try_get().unwrap_or_default()));
    let line = Memo::new(move |_| {
        let s = graph_scale.try_get().unwrap_or_else(|| scale(target, &[]));
        runs(&cols.try_get().unwrap_or_default(), &s, target)
    });

    // The head: the caption, the reading as the strip shows it, the target
    // or the cursor.
    let reading = Memo::new(move |_| {
        let p = point
            .try_with_value(|(instance, path)| store.lufs.try_with(|l| l.point(instance, path)))
            .flatten()
            .unwrap_or_default();
        look(p.tenths, p.problem, spec)
    });
    let [cap_top, cap_kind] = spec.map_or(["LUFS", ""], |s| caption(s.kind()));
    let now_text = move || {
        reading
            .try_with(|l| l.text.clone())
            .flatten()
            .unwrap_or_default()
    };
    let now_state = move || reading.try_with(|l| l.state.as_str()).unwrap_or("none");
    let target_line = target_text(target).unwrap_or_default();
    let cursor_line = move || {
        let t = cursor.try_get().flatten()?;
        let (t0, t1) = window.try_get()?;
        let v = cols.try_with(|c| c.get(column_at(t, t0, t1)).copied().flatten())?;
        Some(cursor_text(t, v, offset_at(t)))
    };

    // The window chips and NOW ›.
    let chips = WINDOWS.map(|(label, span)| {
        let keys: Vec<String> = Vec::new();
        let on = move || {
            shown
                .try_with(|v| v.chip_on(span))
                .unwrap_or(false)
                .to_string()
        };
        let down = move |ev: web_sys::PointerEvent| {
            ev.prevent_default();
            let t = wall_ms();
            let _ = shown.try_update(|v| *v = v.pick_span(span, t));
            let _ = middle.span.try_set(span);
            dom::log(&format!("lufs history: window {label}"));
            fetch();
        };
        view! {
            <button
                type="button"
                class="lh-chip"
                data-testid="lufs-window"
                data-span=label
                aria-pressed=on
                use:owns_touches=keys
                on:pointerdown=down
            >
                {label}
            </button>
        }
    });
    let live = move || shown.try_with(View::is_live).unwrap_or(true);
    let now_keys: Vec<String> = Vec::new();
    let to_now = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = shown.try_update(|v| *v = v.to_now());
        dom::log("lufs history: back to now");
        fetch();
    };

    // The days that hold this point.
    let day_chips = move || {
        let t = wall_ms();
        let today = local_day(t, offset_at(t));
        days.try_get()
            .unwrap_or_default()
            .into_iter()
            .map(|d| {
                let this = local_day(d.first_ms, offset_at(d.first_ms));
                let label = day_label(this, today);
                let is_today = this == today;
                let keys: Vec<String> = Vec::new();
                let on = move || shown.try_with(|v| v.day_on(d)).unwrap_or(false).to_string();
                let down = move |ev: web_sys::PointerEvent| {
                    ev.prevent_default();
                    let _ = shown.try_update(|v| *v = v.pick_day(d, is_today));
                    dom::log(&format!(
                        "lufs history: day {}..{} (today {is_today})",
                        d.first_ms, d.last_ms
                    ));
                    fetch();
                };
                view! {
                    <button
                        type="button"
                        class="lh-chip lh-day"
                        data-testid="lufs-day"
                        data-today=is_today.to_string()
                        aria-pressed=on
                        use:owns_touches=keys
                        on:pointerdown=down
                    >
                        {label}
                    </button>
                }
            })
            .collect_view()
    };

    // The scale's labels and lines (percent of the plot).
    let y_ticks = move || {
        let s = graph_scale.try_get()?;
        let band_lines = band(&s).map(|(top, bottom)| {
            view! {
                <div class="lh-band" style=format!("top:{:.3}%", top / 10.0)></div>
                <div class="lh-band" style=format!("top:{:.3}%", bottom / 10.0)></div>
            }
        });
        let lines = s
            .ticks
            .iter()
            .map(|v| {
                let main = s.target == Some(*v);
                let top = format!("top:{:.3}%", y_of(*v, &s) / 10.0);
                view! { <div class="lh-hline" data-main=main.to_string() style=top></div> }
            })
            .collect_view();
        Some(view! { {lines} {band_lines} })
    };
    let y_labels = move || {
        let s = graph_scale.try_get()?;
        Some(
            s.ticks
                .iter()
                .map(|v| {
                    let main = s.target == Some(*v);
                    let top = format!("top:{:.3}%", y_of(*v, &s) / 10.0);
                    view! {
                        <span data-main=main.to_string() style=top>
                            {tick_label(*v)}
                        </span>
                    }
                })
                .collect_view(),
        )
    };
    let x_parts = move || {
        let (t0, t1) = window.try_get()?;
        let offset = offset_at(t1);
        Some(
            time_ticks(t0, t1, offset)
                .into_iter()
                .map(|t| {
                    (
                        format!("left:{:.3}%", x_of(t, t0, t1) / 10.0),
                        hh_mm(t, offset),
                    )
                })
                .collect::<Vec<_>>(),
        )
    };
    let x_lines = move || {
        x_parts().map(|parts| {
            parts
                .into_iter()
                .map(|(left, _)| view! { <div class="lh-vline" style=left></div> })
                .collect_view()
        })
    };
    let x_labels = move || {
        x_parts().map(|parts| {
            parts
                .into_iter()
                .map(|(left, text)| view! { <span style=left>{text}</span> })
                .collect_view()
        })
    };
    let polylines = move || {
        line.try_get()
            .unwrap_or_default()
            .into_iter()
            .map(|r| {
                view! {
                    <polyline
                        class="lh-line"
                        data-testid="lufs-line"
                        data-green=r.green.to_string()
                        points=r.points
                    ></polyline>
                }
            })
            .collect_view()
    };
    let drawn = move || line.try_with(Vec::len).unwrap_or(0);
    let empty = move || drawn() > 0;
    let cursor_mark = move || {
        let t = cursor.try_get().flatten()?;
        let (t0, t1) = window.try_get()?;
        let left = format!("left:{:.3}%", x_of(t.clamp(t0, t1), t0, t1) / 10.0);
        Some(view! { <div class="lh-cursor-line" data-testid="lufs-cursor" style=left></div> })
    };
    let view_box = format!("0 0 {BOX} {BOX}");
    let columns_n = COLUMNS.to_string();

    // A finger on the graph: a drag pans back in time (the hub asked at its
    // end), a hold shows the cursor.
    let time_at = move |x: f64, h: &Held| {
        let (t0, t1) = window.try_get_untracked().unwrap_or((0, 1));
        crate::behave::lufs_history::cursor_time((x - h.left) / h.width.max(1.0), t0, t1)
    };
    let end_press = move |why: &str| {
        if let Some(Some(handle)) = hold_timer.try_update_value(Option::take) {
            handle.clear();
        }
        let Some(Some(h)) = held.try_update_value(Option::take) else {
            return;
        };
        let _ = cursor.try_set(None);
        dom::log(&format!(
            "lufs history: press {why} (dragged {}, held {})",
            h.press.dragged(),
            h.press.holding()
        ));
        if h.press.dragged() {
            fetch();
        }
    };
    let graph_keys: Vec<String> = Vec::new();
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        if held.try_with_value(Option::is_some).unwrap_or(true) {
            return;
        }
        let Some(area) = plot.get_untracked() else {
            return;
        };
        let rect = area.get_bounding_client_rect();
        let x = f64::from(ev.client_x());
        let Some(start) = shown.try_get_untracked() else {
            return;
        };
        let h = Held {
            press: Press::down(x, dom::now()),
            start,
            left: rect.left(),
            width: rect.width(),
            pointer: ev.pointer_id(),
        };
        let _ = held.try_set_value(Some(h));
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
                    let t = time_at(h.press.x0(), &h);
                    let _ = cursor.try_set(Some(t));
                    dom::log("lufs history: hold, cursor shown");
                }
            },
            Duration::from_millis(HOLD_MS as u64 + 20),
        );
        let _ = hold_timer.try_set_value(timer.ok());
    };
    let on_move = move |ev: web_sys::PointerEvent| {
        let x = f64::from(ev.client_x());
        let Some(Some(gesture)) = held.try_update_value(|h| {
            h.as_mut()
                .filter(|h| h.pointer == ev.pointer_id())
                .map(|h| h.press.moved(x))
        }) else {
            return;
        };
        let Some(Some(h)) = held.try_get_value() else {
            return;
        };
        match gesture {
            Gesture::Still => {}
            Gesture::Pan(dx) => {
                let _ = shown.try_set(View::panned(h.start, dx, h.width, wall_ms()));
            }
            Gesture::Cursor(x) => {
                let _ = cursor.try_set(Some(time_at(x, &h)));
            }
        }
    };
    let ours = move |ev: &web_sys::PointerEvent| {
        held.try_with_value(|h| h.is_some_and(|h| h.pointer == ev.pointer_id()))
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
    let span_label = move || {
        shown
            .try_with(|v| match (v.day, v.is_live()) {
                (Some(_), _) => "day",
                (None, true) => "live",
                (None, false) => "back",
            })
            .unwrap_or("live")
    };

    view! {
        <div class="lh" data-testid="lufs-history" data-state=span_label>
            <div class="lh-head">
                <div class="lh-cap" data-testid="lufs-history-cap">
                    <span>{cap_top}</span>
                    <span>{cap_kind}</span>
                </div>
                <div class="lh-now" data-testid="lufs-history-now" data-l=now_state>
                    {now_text}
                </div>
                <div class="lh-target" data-testid="lufs-history-target">
                    {target_line}
                </div>
                <div class="lh-cursor" data-testid="lufs-history-cursor">
                    {cursor_line}
                </div>
                <div class="lh-chips">
                    <button
                        type="button"
                        class="lh-chip back"
                        data-testid="lufs-now-chip"
                        hidden=live
                        use:owns_touches=now_keys
                        on:pointerdown=to_now
                    >
                        "NOW ›"
                    </button>
                    {chips.into_iter().collect_view()}
                </div>
            </div>
            <div
                class="lh-graph"
                data-testid="lufs-graph"
                data-columns=columns_n
                data-runs=move || drawn().to_string()
                use:owns_touches=graph_keys
                on:pointerdown=on_down
                on:pointermove=on_move
                on:pointerup=on_up
                on:pointercancel=on_cancel
                on:lostpointercapture=on_lost
            >
                <div class="lh-y" aria-hidden="true">{y_labels}</div>
                <div class="lh-plot" node_ref=plot>
                    {y_ticks}
                    {x_lines}
                    <svg class="lh-svg" viewBox=view_box preserveAspectRatio="none" aria-hidden="true">
                        {polylines}
                    </svg>
                    {cursor_mark}
                    <div class="lh-empty" data-testid="lufs-history-empty" hidden=empty>
                        "NO LUFS IN THIS WINDOW"
                    </div>
                </div>
                <div class="lh-x" aria-hidden="true">{x_labels}</div>
            </div>
            <div class="lh-days" data-testid="lufs-days">{day_chips}</div>
        </div>
    }
    .into_any()
}
