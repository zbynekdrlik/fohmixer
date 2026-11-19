//! The channel detail's middle-box tabs and its LUFS history (#219, the
//! design on the ticket; the owner's approved mockup
//! `docs/mockups/lufs-history-v2.html`, its graph since #253 mockup v3):
//! `EQ` (the Pro-Q 4 cards) and, on a `+LUFS` strip, `LUFS`, each filling
//! the whole box. The LUFS tab draws the point's short-term reading over a
//! window (`10 MIN`, `1 H`, `3 H`, a drag back in time, a past day) as a
//! calm curve, and the strip's fader beside it on its own dB scale, read
//! from the hub's `/api/lufs/history` and `/api/lufs/days`, its live edge
//! from the page's own reading and fader once a second. Every decision is
//! `behave::lufs_history`; this is the glue: the card (`DetailLufs`) and
//! its head, graph and days, sharing one `Lh`.

use std::time::Duration;

use fohmixer_proto::layout::{LufsSpec, Strip};
use fohmixer_proto::lufs::{HistoryDay, caption};
use leptos::prelude::*;

use super::owns_touches;
use crate::behave::lufs::{look, point_key};
use crate::behave::lufs_history::{
    Answers, Column, DEFAULT_SPAN_MS, MINUTE_MS, SECOND_MS, Sample, Tab, View, WINDOWS, column_at,
    cursor_text, day_chip_at, drawn_columns, edge_reading, fader_text, keep_tail, push_reading,
    refresh, retry_series, samples, second_of, series_end, shown_tab, tab_row, tabs, target_text,
};
use crate::binding::strip_subs;
use crate::dom;
use crate::store::{LiveStore, Slot};

mod graph;
use fohmixer_proto::layout::MeterSource;
use fohmixer_proto::lufs::fader_tenths;
use graph::LufsGraph;

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
            <div
                class="detail-pane eq-pane"
                data-pane="eq"
                data-testid="detail-pane-eq"
                hidden=eq_hidden
            >
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
    let keys = tabs(true).into_iter().map(|tab| {
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
    view! { <div class="mtabs" data-testid="detail-tabs">{keys.collect_view()}</div> }
}

/// The card's shared state: the point, the strip's volume, the view, the
/// clock, the hub's series and days, the live edge's samples, the cursor,
/// and what is drawn from them.
#[derive(Clone, Copy)]
struct Lh {
    store: LiveStore,
    point: StoredValue<(String, String)>,
    /// The strip's volume as the detail follows it (#253: the head's fader
    /// and the live edge's).
    volume: Option<RwSignal<Slot>>,
    target: Option<i8>,
    kept_span: RwSignal<i64>,
    shown: RwSignal<View>,
    now: RwSignal<i64>,
    answers: RwSignal<Answers>,
    tail: RwSignal<Vec<Sample>>,
    days: RwSignal<Vec<HistoryDay>>,
    cursor: RwSignal<Option<i64>>,
    window: Memo<(i64, i64)>,
    cols: Memo<Vec<Column>>,
}

impl Lh {
    /// The hub's series for the window shown; an answer older than the
    /// newest ask is dropped, a failed one keeps the last answer drawn
    /// (`Answers`).
    fn fetch(self) {
        let Some(v) = self.shown.try_get_untracked() else {
            return;
        };
        let (t0, t1) = v.window(wall_ms());
        let Some(n) = self.answers.try_update(Answers::ask) else {
            return;
        };
        let Some((instance, path)) = self.point.try_get_value() else {
            return;
        };
        dom::log(&format!(
            "lufs history: ask {instance} {path} {t0}..{t1} ({})",
            v.state()
        ));
        leptos::task::spawn_local(async move {
            let got = self.store.lufs_history(&instance, &path, t0, t1).await;
            let end = got.as_ref().map(series_end);
            let taken = self
                .answers
                .try_update(|a| a.answer(n, got))
                .unwrap_or(false);
            if taken && let Some(end) = end {
                let _ = self.tail.try_update(|t| keep_tail(t, end));
            }
        });
    }

    /// The days that hold the point.
    fn fetch_days(self) {
        let Some((instance, path)) = self.point.try_get_value() else {
            return;
        };
        leptos::task::spawn_local(async move {
            if let Some(got) = self.store.lufs_days(&instance, &path).await {
                let _ = self.days.try_set(got);
            }
        });
    }

    /// The strip's fader as the head shows it, dB tenths: Live's volume as
    /// last heard, kept through an outage like the strip's own (none while
    /// it is not known).
    fn fader_now(self) -> Option<i16> {
        fader_tenths(self.volume?.try_with(Slot::number).flatten())
    }

    /// The strip's fader the live edge records, dB tenths: only a fresh
    /// value (a stale one through an outage is no second's fader).
    fn edge_fader(self) -> Option<i16> {
        fader_tenths(
            self.volume?
                .try_with_untracked(Slot::fresh_number)
                .flatten(),
        )
    }

    /// One second: the clock, the live edge's reading and fader, and the
    /// hub asked again when `refresh` says so (`n` counts the ticks from 1).
    fn tick(self, n: u32) {
        let t = wall_ms();
        let _ = self.now.try_set(t);
        let Some(v) = self.shown.try_get_untracked() else {
            return;
        };
        if v.is_live() {
            let reading = self
                .point
                .try_with_value(|(instance, path)| {
                    self.store.lufs.try_with_untracked(|l| {
                        let p = l.point(instance, path);
                        edge_reading(p.tenths, p.problem.is_some())
                    })
                })
                .flatten()
                .flatten();
            let from = v.window(t).0;
            let fader = self.edge_fader();
            let _ = self
                .tail
                .try_update(|tl| push_reading(tl, second_of(t), reading, fader, from));
        }
        let known = self
            .days
            .try_with_untracked(|d| !d.is_empty())
            .unwrap_or(true);
        let again = refresh(n, v.is_live(), known);
        let failed = self
            .answers
            .try_with_untracked(Answers::failed)
            .unwrap_or(false);
        if again.series || retry_series(n, failed) {
            self.fetch();
        }
        if again.days {
            self.fetch_days();
        }
    }

    /// A new view (a chip, a day, NOW, a drag's end): shown, then asked.
    fn show(self, view: View, why: &str) {
        dom::log(&format!("lufs history: {why} ({})", view.state()));
        let _ = self.shown.try_set(view);
        self.fetch();
    }
}

/// The LUFS history of a `+LUFS` strip's point (mockup v2's LUFS tab).
#[component]
pub fn DetailLufs(strip: Strip) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let middle = use_context::<DetailMiddle>().unwrap_or_default();
    let spec = strip.lufs;
    let Some(key) = point_key(&strip) else {
        return ().into_any();
    };
    let target = spec.and_then(|s| s.target);
    let shown = RwSignal::new(View::live(
        middle.span.try_get_untracked().unwrap_or(DEFAULT_SPAN_MS),
    ));
    let now = RwSignal::new(wall_ms());
    let answers = RwSignal::new(Answers::default());
    let tail = RwSignal::new(Vec::<Sample>::new());
    let window = Memo::new(move |_| {
        let t = now.try_get().unwrap_or(0);
        shown
            .try_get()
            .unwrap_or(View::live(DEFAULT_SPAN_MS))
            .window(t)
    });
    let cols = Memo::new(move |_| {
        let (t0, t1) = window.try_get().unwrap_or((0, 1));
        let mut all = answers
            .try_with(|a| a.drawn().map(samples))
            .flatten()
            .unwrap_or_default();
        all.extend(tail.try_get().unwrap_or_default());
        drawn_columns(&all, t0, t1)
    });
    let volume = strip_subs(&strip, MeterSource::Level)
        .volume
        .map(|spec| store.slot(&spec));
    let lh = Lh {
        store,
        point: StoredValue::new(key),
        volume,
        target,
        kept_span: middle.span,
        shown,
        now,
        answers,
        tail,
        days: RwSignal::new(Vec::new()),
        cursor: RwSignal::new(None),
        window,
        cols,
    };
    lh.fetch();
    lh.fetch_days();
    let ticks = StoredValue::new(0u32);
    let ticker = StoredValue::new(None::<IntervalHandle>);
    let tick = move || {
        if let Some(n) = ticks.try_update_value(|n| {
            *n += 1;
            *n
        }) {
            lh.tick(n);
        }
    };
    if let Ok(handle) = set_interval_with_handle(tick, Duration::from_millis(SECOND_MS as u64)) {
        let _ = ticker.try_set_value(Some(handle));
    }
    on_cleanup(move || {
        if let Some(Some(handle)) = ticker.try_update_value(Option::take) {
            handle.clear();
        }
    });
    let state = move || shown.try_with(View::state).unwrap_or("live");
    let failed = move || {
        answers
            .try_with(Answers::failed)
            .unwrap_or(false)
            .to_string()
    };
    view! {
        <div class="lh" data-testid="lufs-history" data-state=state data-failed=failed>
            <LufsHead lh=lh spec=spec />
            <LufsGraph lh=lh />
            <LufsDays lh=lh />
        </div>
    }
    .into_any()
}

/// The head: the caption, the reading as the strip shows it, the target or
/// the cursor's time and value, NOW › and the window chips.
#[component]
fn LufsHead(lh: Lh, spec: Option<LufsSpec>) -> impl IntoView {
    let reading = Memo::new(move |_| {
        let p = lh
            .point
            .try_with_value(|(instance, path)| lh.store.lufs.try_with(|l| l.point(instance, path)))
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
    let target_line = target_text(lh.target).unwrap_or_default();
    let fader_line = move || fader_text(lh.fader_now()).unwrap_or_default();
    let cursor_line = move || {
        let t = lh.cursor.try_get().flatten()?;
        let (t0, t1) = lh.window.try_get()?;
        let col = lh
            .cols
            .try_with(|c| c.get(column_at(t, t0, t1)).copied().unwrap_or_default())?;
        Some(cursor_text(t, col, offset_at(t)))
    };
    let chips = WINDOWS.map(|(label, span)| {
        let keys: Vec<String> = Vec::new();
        let on = move || {
            lh.shown
                .try_with(|v| v.chip_on(span))
                .unwrap_or(false)
                .to_string()
        };
        let down = move |ev: web_sys::PointerEvent| {
            ev.prevent_default();
            let Some(v) = lh.shown.try_get_untracked() else {
                return;
            };
            let _ = lh.kept_span.try_set(span);
            lh.show(v.pick_span(span, wall_ms()), label);
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
    let live = move || lh.shown.try_with(View::is_live).unwrap_or(true);
    let now_keys: Vec<String> = Vec::new();
    let to_now = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        if let Some(v) = lh.shown.try_get_untracked() {
            lh.show(v.to_now(), "NOW");
        }
    };
    view! {
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
            <div class="lh-fader" data-testid="lufs-history-fader">
                {fader_line}
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
    }
}

/// The days that hold this point (newest first; only those, the
/// show-only-used rule): a past day whole, today the live edge, each named
/// by the UTC day its file holds (`day_chip_at`).
#[component]
fn LufsDays(lh: Lh) -> impl IntoView {
    let chips = move || {
        // Today from the clock of the last days read: the chips are drawn
        // again with each read (once a minute), not each second.
        let t = lh.now.try_get_untracked().unwrap_or_else(wall_ms);
        lh.days
            .try_get()
            .unwrap_or_default()
            .into_iter()
            .map(|d| {
                let (label, is_today) = day_chip_at(d, t);
                let keys: Vec<String> = Vec::new();
                let on = move || {
                    lh.shown
                        .try_with(|v| v.day_on(d))
                        .unwrap_or(false)
                        .to_string()
                };
                let down = move |ev: web_sys::PointerEvent| {
                    ev.prevent_default();
                    if let Some(v) = lh.shown.try_get_untracked() {
                        lh.show(v.pick_day(d, is_today), "day");
                    }
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
    view! {
        <div class="lh-days" data-testid="lufs-days">
            {chips}
        </div>
    }
}
