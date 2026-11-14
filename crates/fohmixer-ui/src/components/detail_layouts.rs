//! The channel detail's fader sectors (#148; spec
//! `docs/superpowers/specs/2026-10-28-sectors-design.md` §4): the LAYOUTS
//! section in `detail-right` and the bar's ◀ ▶, which step to the pool's
//! strip before and after, so a strip in no layout (a new track) can still
//! be opened and placed. Both show only while the hub's document places
//! faders; an empty document keeps today's detail.
//!
//! LAYOUTS (#208, the owner's approved mockup `docs/mockups/scenes-v3.html`
//! frame 1; the owner's rule: a settings list shows only what is set, plus
//! one ADD): one compact row per layout that holds the fader (its sector,
//! its title, a ✕ that takes it out), and `+ ADD TO LAYOUT`, which opens a
//! sheet over the detail with the other layouts (a full one disabled,
//! FULL); one tap there puts the fader in, at the layout's end. The
//! decisions are `binding::detail_layouts` (`held`, `picks`) over
//! `binding::sectors::detail_ticks`, and `detail_steps` for ◀ ▶; nothing
//! here writes a Live key.

use fohmixer_proto::layout::Strip;
use leptos::prelude::*;

use super::detail::DetailStrip;
use super::{fail_flash, owns_touches};
use crate::binding::detail_layouts::{LayoutPick, LayoutRow, held, picks};
use crate::binding::sectors::{DetailTicks, detail_steps, detail_ticks, stepped};
use crate::store::{LiveStore, on_refusal};

/// The section's layouts for the strip shown: the hub's document as last
/// sent (kept through an outage) and the served layout's pool.
fn ticks_memo(store: LiveStore, strip: StoredValue<Strip>) -> Memo<DetailTicks> {
    Memo::new(move |_| {
        let layout = store.layout.try_get().flatten();
        let Some(layout) = layout else {
            return DetailTicks::Off;
        };
        store
            .sectors
            .try_with(|told| strip.try_with_value(|s| detail_ticks(told.shown(), &layout, s)))
            .flatten()
            .unwrap_or(DetailTicks::Off)
    })
}

/// Asks the hub to put the shown strip in `layout` (`on`) or take it out;
/// a refusal flashes `failed`.
fn ask_tick(
    store: LiveStore,
    ticks: Memo<DetailTicks>,
    layout: &str,
    on: bool,
    failed: RwSignal<bool>,
) {
    let Some(id) = ticks.try_with_untracked(|t| t.id().cloned()).flatten() else {
        return;
    };
    store.sector_tick(layout, &id, on, Some(on_refusal(fail_flash(failed))));
}

/// LAYOUTS: the head with the count, the fader's layouts (or the empty
/// line), ADD and its sheet; the note for a strip with no identity (a
/// marker showing its placeholder).
#[component]
pub fn DetailLayouts(strip: Strip) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let strip = StoredValue::new(strip);
    let ticks = ticks_memo(store, strip);
    let shown = Memo::new(move |_| {
        ticks
            .try_with(|t| !matches!(t, DetailTicks::Off))
            .unwrap_or(false)
    });
    let placed = Memo::new(move |_| {
        ticks
            .try_with(|t| matches!(t, DetailTicks::Rows { .. }))
            .unwrap_or(false)
    });
    let rows = Memo::new(move |_| ticks.try_with(held).unwrap_or_default());
    let choices = Memo::new(move |_| ticks.try_with(picks).unwrap_or_default());
    let open = RwSignal::new(false);
    // Nothing left to pick (another page filled the rest): the sheet goes.
    Effect::new(move |_| {
        let none = choices.try_with(Vec::is_empty).unwrap_or(true);
        if none && open.try_get_untracked().unwrap_or(false) {
            let _ = open.try_set(false);
        }
    });
    let count = move || rows.try_with(Vec::len).unwrap_or(0).to_string();
    let section = move || {
        if !shown.try_get().unwrap_or(false) {
            return None;
        }
        let inner = if placed.try_get().unwrap_or(false) {
            view! {
                <LayoutList ticks=ticks rows=rows />
                <AddLayout ticks=ticks choices=choices open=open />
            }
            .into_any()
        } else {
            view! {
                <div class="lay-note" data-testid="detail-layouts-none">
                    "This fader has no name yet, so it cannot be placed in a layout."
                </div>
            }
            .into_any()
        };
        Some(view! {
            <div class="lay-sec" data-testid="detail-layouts">
                <div class="col-head">
                    <h2 class="detail-title">"LAYOUTS"</h2>
                    <span class="col-now" data-testid="lay-count">
                        {count}
                    </span>
                </div>
                {inner}
            </div>
        })
    };
    view! { {section} }
}

/// The fader's layouts, one row each, or the empty line. A row is keyed by
/// all it shows, so a changed row is drawn anew and a refusal's flash stays
/// on an unchanged one.
#[component]
fn LayoutList(ticks: Memo<DetailTicks>, rows: Memo<Vec<LayoutRow>>) -> impl IntoView {
    let none = move || rows.try_with(Vec::is_empty).unwrap_or(true);
    let row = move |row: LayoutRow| view! { <LayoutRowView ticks=ticks row=row /> };
    view! {
        <div class="lay-rows">
            <For
                each=move || rows.try_get().unwrap_or_default()
                key=|r: &LayoutRow| (r.layout.clone(), r.title.clone(), r.sector)
                children=row
            />
        </div>
        {move || {
            none()
                .then(|| {
                    view! {
                        <div class="lay-empty" data-testid="lay-empty">
                            "This fader is in no layout."
                        </div>
                    }
                })
        }}
    }
}

/// A layout that holds the fader: its sector, its title and ✕, whose tap
/// (its `pointerdown`) asks the hub to take the fader out; a refusal
/// flashes the row (`data-refused`).
#[component]
fn LayoutRowView(ticks: Memo<DetailTicks>, row: LayoutRow) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let failed = RwSignal::new(false);
    let layout = StoredValue::new(row.layout.clone());
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = layout.try_with_value(|layout| ask_tick(store, ticks, layout, false, failed));
    };
    let label = format!("Take this fader out of {}", row.title);
    let no_keys: Vec<String> = Vec::new();
    view! {
        <div
            class="lay-row"
            data-testid="lay-row"
            data-layout=row.layout
            data-sector=row.sector
            data-refused=move || failed.try_get().unwrap_or(false).to_string()
        >
            <span class="lay-where">{row.sector}</span>
            <span class="lay-what">{row.title}</span>
            <button
                type="button"
                class="lay-remove"
                use:owns_touches=no_keys
                data-testid="lay-remove"
                aria-label=label
                on:pointerdown=on_down
            >
                "✕"
            </button>
        </div>
    }
}

/// `+ ADD TO LAYOUT` (only while a layout is left to pick) and its sheet:
/// a tap opens the sheet; a refusal of a pick flashes the button.
#[component]
fn AddLayout(
    ticks: Memo<DetailTicks>,
    choices: Memo<Vec<LayoutPick>>,
    open: RwSignal<bool>,
) -> impl IntoView {
    let failed = RwSignal::new(false);
    let any = move || choices.try_with(|c| !c.is_empty()).unwrap_or(false);
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = open.try_set(true);
    };
    let sheet = move || {
        (open.try_get().unwrap_or(false) && any())
            .then(|| view! { <LayoutSheet ticks=ticks choices=choices open=open failed=failed /> })
    };
    move || {
        any().then(|| {
            let no_keys: Vec<String> = Vec::new();
            view! {
                <button
                    type="button"
                    class="lay-add"
                    use:owns_touches=no_keys
                    data-testid="lay-add"
                    data-refused=move || failed.try_get().unwrap_or(false).to_string()
                    on:pointerdown=on_down
                >
                    "+ ADD TO LAYOUT"
                </button>
                {sheet}
            }
        })
    }
}

/// The sheet over the detail: a shade (a tap closes), the title with ✕,
/// the note, and a pick per layout the fader is not in. A pick's tap asks
/// the hub to put the fader in and closes the sheet; a FULL pick is
/// disabled.
#[component]
fn LayoutSheet(
    ticks: Memo<DetailTicks>,
    choices: Memo<Vec<LayoutPick>>,
    open: RwSignal<bool>,
    failed: RwSignal<bool>,
) -> impl IntoView {
    let close = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = open.try_set(false);
    };
    let pick = move |pick: LayoutPick| {
        view! { <PickButton ticks=ticks pick=pick open=open failed=failed /> }
    };
    let shade_keys: Vec<String> = Vec::new();
    let close_keys: Vec<String> = Vec::new();
    view! {
        <div class="lay-sheet" data-testid="lay-sheet">
            <div
                class="lay-shade"
                use:owns_touches=shade_keys
                data-testid="lay-shade"
                on:pointerdown=close
            ></div>
            <div class="lay-panel" role="dialog" aria-label="Add to layout">
                <div class="lay-sheet-head">
                    <h3 class="lay-sheet-title">"ADD TO LAYOUT"</h3>
                    <button
                        type="button"
                        class="lay-sheet-close"
                        use:owns_touches=close_keys
                        data-testid="lay-sheet-close"
                        aria-label="Close"
                        on:pointerdown=close
                    >
                        "✕"
                    </button>
                </div>
                <div class="lay-note">"It joins the layout at its end; ARRANGE moves it."</div>
                <div class="lay-picks">
                    <For
                        each=move || choices.try_get().unwrap_or_default()
                        key=|p: &LayoutPick| (p.layout.clone(), p.title.clone(), p.sector, p.full)
                        children=pick
                    />
                </div>
            </div>
        </div>
    }
}

/// One layout of the sheet: its title and its sector (`· FULL` when the
/// fader would not fit: disabled, asks nothing).
#[component]
fn PickButton(
    ticks: Memo<DetailTicks>,
    pick: LayoutPick,
    open: RwSignal<bool>,
    failed: RwSignal<bool>,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let asks = pick.asks();
    let layout = StoredValue::new(pick.layout.clone());
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        if !asks {
            return;
        }
        let _ = layout.try_with_value(|layout| ask_tick(store, ticks, layout, true, failed));
        let _ = open.try_set(false);
    };
    let sub = pick.sub();
    let no_keys: Vec<String> = Vec::new();
    view! {
        <button
            type="button"
            class="lay-pick"
            use:owns_touches=no_keys
            data-testid="lay-pick"
            data-layout=pick.layout
            data-sector=pick.sector
            data-full=pick.full.to_string()
            disabled=pick.full
            on:pointerdown=on_down
        >
            <span class="lay-pick-name">{pick.title}</span>
            <small>{sub}</small>
        </button>
    }
}

/// The bar's ◀ ▶: the pool's strip before and after this one (the
/// parameter faders skipped, no wrap; `.end` at either end). A step opens
/// that strip's detail (`DetailStrip::step` with the tested
/// `binding::sectors::stepped`, recorded as an open with `why: "step"`).
#[component]
pub fn DetailStep(strip: Strip) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let shown = use_context::<DetailStrip>();
    let strip = StoredValue::new(strip);
    let steps = Memo::new(move |_| {
        let layout = store.layout.try_get().flatten()?;
        store
            .sectors
            .try_with(|told| strip.try_with_value(|s| detail_steps(told.shown(), &layout, s)))
            .flatten()
            .flatten()
    });
    let go = move |ev: web_sys::PointerEvent, forward: bool| {
        ev.prevent_default();
        let next = steps
            .try_with_untracked(|s| stepped(s.as_ref(), forward))
            .flatten();
        if let (Some(next), Some(detail)) = (next, shown) {
            detail.step(next, ev.pointer_id());
        }
    };
    let at_start = move || {
        steps
            .try_with(|s| s.as_ref().is_none_or(|s| s.back.is_none()))
            .unwrap_or(true)
    };
    let at_end = move || {
        steps
            .try_with(|s| s.as_ref().is_none_or(|s| s.next.is_none()))
            .unwrap_or(true)
    };
    let live = move || steps.try_with(Option::is_some).unwrap_or(false);
    move || {
        live().then(|| {
            let back_keys: Vec<String> = Vec::new();
            let next_keys: Vec<String> = Vec::new();
            view! {
                <div class="detail-steps" data-testid="detail-steps">
                    <button
                        type="button"
                        class="detail-step"
                        use:owns_touches=back_keys
                        class:end=at_start
                        data-testid="detail-prev"
                        aria-label="The previous fader"
                        on:pointerdown=move |ev| go(ev, false)
                    >
                        "◀"
                    </button>
                    <button
                        type="button"
                        class="detail-step"
                        use:owns_touches=next_keys
                        class:end=at_end
                        data-testid="detail-next"
                        aria-label="The next fader"
                        on:pointerdown=move |ev| go(ev, true)
                    >
                        "▶"
                    </button>
                </div>
            }
        })
    }
}
