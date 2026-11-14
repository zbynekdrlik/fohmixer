//! The channel detail's fader sectors (#148; spec
//! `docs/superpowers/specs/2026-10-28-sectors-design.md` §4): the LAYOUTS
//! section in `detail-right`, only while the hub's document places faders;
//! an empty document keeps today's detail. A strip in no layout (a new
//! track) is opened from the bar's channel selector (#205,
//! `components::channel_select`; ◀ ▶ before it) and placed here.
//!
//! LAYOUTS (#208, the owner's approved mockup `docs/mockups/scenes-v3.html`
//! frame 1; the owner's rule: a settings list shows only what is set, plus
//! one ADD): one compact row per layout that holds the fader (its sector,
//! its title, a ✕ that takes it out), and `+ ADD TO LAYOUT`, which opens a
//! sheet over the detail (`components::sheet`) with the other layouts (a
//! full one disabled, FULL); one tap there puts the fader in, at the
//! layout's end. The ✕ and the picks act at their lift. The decisions are
//! `binding::detail_layouts` (`held`, `picks`) over
//! `binding::sectors::detail_ticks`; nothing here writes a Live key.

use fohmixer_proto::layout::Strip;
use leptos::prelude::*;

use super::sheet::{LiftKey, Sheet, close_later};
use super::{fail_flash, owns_touches};
use crate::binding::detail_layouts::{LayoutPick, LayoutRow, held, picks};
use crate::binding::sectors::{DetailTicks, detail_ticks};
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
    let none = Memo::new(move |_| rows.try_with(Vec::is_empty).unwrap_or(true));
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
            none.try_get()
                .unwrap_or(false)
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
/// (at its lift, `LiftKey`) asks the hub to take the fader out; a refusal
/// flashes the row (`data-refused`).
#[component]
fn LayoutRowView(ticks: Memo<DetailTicks>, row: LayoutRow) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let failed = RwSignal::new(false);
    let layout = StoredValue::new(row.layout.clone());
    let key = LiftKey::default();
    let on_up = move |ev: web_sys::PointerEvent| {
        if key.up(&ev) {
            let _ = layout.try_with_value(|layout| ask_tick(store, ticks, layout, false, failed));
        }
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
                on:pointerdown=move |ev| key.down(&ev)
                on:pointermove=move |ev| key.moved(&ev)
                on:pointerup=on_up
                on:pointercancel=move |ev| key.cancel(&ev)
                on:lostpointercapture=move |ev| key.cancel(&ev)
            >
                "✕"
            </button>
        </div>
    }
}

/// `+ ADD TO LAYOUT` (always, as the approved mockup draws it) and its
/// sheet: a tap (its `pointerdown`: opening changes nothing) opens the
/// sheet; a refusal of a pick flashes the button.
#[component]
fn AddLayout(
    ticks: Memo<DetailTicks>,
    choices: Memo<Vec<LayoutPick>>,
    open: RwSignal<bool>,
) -> impl IntoView {
    let failed = RwSignal::new(false);
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = open.try_set(true);
    };
    let none = Memo::new(move |_| choices.try_with(Vec::is_empty).unwrap_or(false));
    let pick = move |pick: LayoutPick| {
        view! { <PickButton ticks=ticks pick=pick open=open failed=failed /> }
    };
    let sheet = move || {
        open.try_get().unwrap_or(false).then(|| {
            let empty = move || {
                none.try_get().unwrap_or(false).then(|| {
                    view! {
                        <div class="lay-empty" data-testid="lay-sheet-none">
                            "This fader is in every layout."
                        </div>
                    }
                })
            };
            view! {
                <Sheet
                    title="ADD TO LAYOUT"
                    testid="lay-sheet"
                    open=open
                    note="It joins the layout at its end; ARRANGE moves it."
                >
                    <div class="sheet-grid">
                        <For
                            each=move || choices.try_get().unwrap_or_default()
                            key=|p: &LayoutPick| (p.layout.clone(), p.title.clone(), p.sector, p.full)
                            children=pick
                        />
                    </div>
                    {empty}
                </Sheet>
            }
        })
    };
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
}

/// One layout of the sheet: its title and its sector (`· FULL` when the
/// fader would not fit: disabled, asks nothing). Its tap (at its lift)
/// asks the hub to put the fader in and closes the sheet.
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
    let key = LiftKey::default();
    let on_up = move |ev: web_sys::PointerEvent| {
        if key.up(&ev) && asks {
            let _ = layout.try_with_value(|layout| ask_tick(store, ticks, layout, true, failed));
            close_later(open);
        }
    };
    let sub = pick.sub();
    let no_keys: Vec<String> = Vec::new();
    view! {
        <button
            type="button"
            class="sheet-pick"
            use:owns_touches=no_keys
            data-testid="lay-pick"
            data-layout=pick.layout
            data-sector=pick.sector
            data-full=pick.full.to_string()
            disabled=pick.full
            on:pointerdown=move |ev| key.down(&ev)
            on:pointermove=move |ev| key.moved(&ev)
            on:pointerup=on_up
            on:pointercancel=move |ev| key.cancel(&ev)
            on:lostpointercapture=move |ev| key.cancel(&ev)
        >
            <span class="sheet-pick-name">{pick.title}</span>
            <small>{sub}</small>
        </button>
    }
}
