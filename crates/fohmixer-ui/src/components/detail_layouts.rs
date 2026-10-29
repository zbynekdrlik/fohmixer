//! The channel detail's fader sectors (#148; spec
//! `docs/superpowers/specs/2026-10-28-sectors-design.md` §4, the owner's
//! approved mockup `docs/mockups/sectors-v1.html`): the LAYOUTS section in
//! `detail-right` (every layout of every sector with a checkbox: a fader
//! shows in every ticked layout, a new one joins at its end; a full layout
//! that does not hold it reads FULL and is disabled) and the bar's ◀ ▶,
//! which step to the pool's strip before and after, so a strip in no layout
//! (a new track) can still be opened and placed. Both show only while the
//! hub's document places faders; an empty document keeps today's detail.
//! The decisions are `binding::sectors` (`detail_ticks`, `TickRow::tap`,
//! `detail_steps`); nothing here writes a Live key.

use fohmixer_proto::layout::Strip;
use fohmixer_proto::sectors::SectorId;
use leptos::prelude::*;

use super::detail::DetailStrip;
use super::{fail_flash, owns_touches};
use crate::binding::sectors::{
    DetailTicks, TickRow, detail_steps, detail_ticks, sector_name, stepped,
};
use crate::store::{LiveStore, on_refusal};

/// The section's ticks for the strip shown: the hub's document as last
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

/// LAYOUTS: a group per sector (LU, LD, RU, RD), a checkbox per layout; the
/// note for a strip with no identity (a marker showing its placeholder).
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
    let section = move || {
        if !shown.try_get().unwrap_or(false) {
            return None;
        }
        let inner = if placed.try_get().unwrap_or(false) {
            let groups = SectorId::ALL
                .into_iter()
                .map(|sector| view! { <TickGroupView ticks=ticks sector=sector /> })
                .collect_view();
            view! {
                <div class="lay-groups">{groups}</div>
                <div class="lay-note">
                    "The fader shows in every ticked layout; a new one joins at its end (ARRANGE moves it)."
                </div>
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
                <h2 class="detail-title">"LAYOUTS"</h2>
                {inner}
            </div>
        })
    };
    view! { {section} }
}

/// One sector's checkboxes under its name; nothing for a sector without
/// layouts. A row is keyed by all it shows, so a changed row is drawn anew
/// and a refusal's flash stays on an unchanged one.
#[component]
fn TickGroupView(ticks: Memo<DetailTicks>, sector: SectorId) -> impl IntoView {
    let rows = Memo::new(move |_| ticks.try_with(|t| t.rows_of(sector)).unwrap_or_default());
    let any = move || rows.try_with(|r| !r.is_empty()).unwrap_or(false);
    let row = move |row: TickRow| view! { <TickButton ticks=ticks row=row /> };
    move || {
        any().then(|| {
            view! {
                <div class="lay-group" data-sector=sector.as_str()>
                    <div class="lay-where">{sector_name(sector)}</div>
                    <div class="lay-row">
                        <For
                            each=move || rows.try_get().unwrap_or_default()
                            key=|r: &TickRow| (r.layout.clone(), r.title.clone(), r.on, r.full)
                            children=row
                        />
                    </div>
                </div>
            }
        })
    }
}

/// A layout's checkbox: a tap (on its `pointerdown`) asks the hub to put
/// the strip in or take it out (`TickRow::tap`); a refusal flashes it
/// (`data-refused`). It owns its touches and writes no Live key.
#[component]
fn TickButton(ticks: Memo<DetailTicks>, row: TickRow) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let failed = RwSignal::new(false);
    let asks = row.tap();
    let layout = StoredValue::new(row.layout.clone());
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let Some(on) = asks else {
            return;
        };
        let Some(id) = ticks.try_with_untracked(|t| t.id().cloned()).flatten() else {
            return;
        };
        let _ = layout.try_with_value(|layout| {
            store.sector_tick(layout, &id, on, Some(on_refusal(fail_flash(failed))))
        });
    };
    let checked = row.on.to_string();
    let mark = if row.on { "✓" } else { "" };
    let full = row.full;
    let full_note = full.then(|| view! { <small>"FULL"</small> });
    let no_keys: Vec<String> = Vec::new();
    view! {
        <button
            type="button"
            class="tick-btn"
            role="checkbox"
            use:owns_touches=no_keys
            data-testid="sector-tick"
            data-layout=row.layout
            aria-checked=checked
            disabled=full
            data-refused=move || failed.try_get().unwrap_or(false).to_string()
            on:pointerdown=on_down
        >
            <span class="box">{mark}</span>
            <span class="tcol">
                <span class="tname">{row.title}</span>
                {full_note}
            </span>
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
