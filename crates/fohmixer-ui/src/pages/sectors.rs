//! The FOH page's four fader sectors (#148; spec
//! `docs/superpowers/specs/2026-10-28-sectors-design.md` §4, the owner's
//! approved mockup `docs/mockups/sectors-v1.html`): while the hub's
//! document holds a layout, the default page shows the sectors in place of
//! its rows: two lines (up, down), each cut by the column, the left
//! sector's strips on its left and the right sector's on its right
//! (`arrange::sectors`, pure). Each line renders as one keyed list of its
//! cells (`SectorCell::key`), so paging one sector remounts only that
//! sector's strips that change; the strip width depends on the document and
//! the screen only, never on the picks. The column holds the LAYOUTS block
//! (a 2 × 2 grid of the sectors' layout tabs, no caption since #212: a tap
//! pages that sector, on this device only), ARRANGE (since #212 a small
//! `⇄` key at the right of the page tab; the reorder mode, `behave::arrange_drag`:
//! while it is on every strip lies under a cover with a ⠿ handle that owns
//! its touches, the faders, MUTE, SEL and the column's other controls are
//! dimmed and inert, every position a sector can show is a cell, the empty
//! ones calm dashed outlines, a strip is dragged onto an empty cell (its
//! old place left empty) or another strip (the two swap) of its own sector
//! or of any other visible one (#233: a move between the layouts shown),
//! and a second tap saves; nothing reaches Live; in the mode the LAYOUTS
//! block stays live and renames, adds and removes layouts, mockup v2,
//! `behave::arrange_edit`, saved with the same tap; no undo, the owner's
//! ruling of 29.10.2026), then the arrows of a side
//! that does not fit, the rail and the global controls. A strip says its
//! Live set in its info column (#211); a layout that mixes Live
//! sets names each strip's set where the group's name sits on the classic
//! page (`binding::sectors::sector_side`), following the shown layout. A
//! layout's empty slot is a blank cell in its place (the slot model,
//! 1.24.0), drawn only in ARRANGE. The column's own parts (the LAYOUTS
//! block, ARRANGE's key) are `sectors/column.rs` (#213).

use std::collections::BTreeMap;
use std::sync::Arc;

use fohmixer_proto::layout::{Control, Layout, Page};
use fohmixer_proto::sectors::{SectorId, SectorsDoc, StripRef, pool};
use leptos::html;
use leptos::prelude::*;

use crate::arrange::sectors::{SectorArrangement, SectorCell, WindowKey, arrange_sectors};
use crate::arrange::{Arrangement, LineKey, METRICS, Shift, Sides, narrow};
use crate::behave::arrange_drag::{Arrange, ArrangeEnd, arrange_note};
use crate::binding::sectors::{Picks, pool_controls, sector_instances, sector_side};
use crate::components::stream_lufs::stream_readouts;
use crate::components::{ControlView, owns_touches};
use crate::dom;
use crate::pages::presenter::StageMini;
use crate::pages::surface::{
    ColumnFoot, ColumnHead, Head, Nav, ThinStrip, extra_attr, measure_page,
};

mod arrange;
mod column;

use arrange::{ArrangeCover, Carrier, change_mode};
use column::{ArrangeButton, LayoutsBlock};

/// The sectors page's arrangement, a memo of what it depends on: the
/// measured page (none before it is laid out), the hub's document (none
/// while there is none), this device's picks, the sides' windows and
/// whether ARRANGE is on (its further positions as empty cells). Every
/// read tolerates a disposed value (`try_*`): a layout change disposes the
/// page while its keyed lists' effects may still run once.
pub(crate) fn sector_memo(
    layout: Arc<Layout>,
    measured: Signal<(Sides, f64)>,
    doc: Signal<Option<SectorsDoc>>,
    picks: Signal<Picks>,
    offsets: Signal<BTreeMap<WindowKey, usize>>,
    arranging: Signal<bool>,
) -> Memo<Option<SectorArrangement>> {
    Memo::new(move |_| {
        let (sides, height) = measured.try_get()?;
        if !sides.measured() {
            return None;
        }
        let arranging = arranging.try_get().unwrap_or(false);
        let pool = pool(&layout);
        doc.try_with(|doc| {
            let doc = doc.as_ref()?;
            picks
                .try_with(|picks| {
                    offsets.try_with(|offsets| {
                        let offset = |key: &WindowKey| offsets.get(key).copied().unwrap_or(0);
                        arrange_sectors(
                            doc, &pool, picks, sides, height, offset, arranging, &METRICS,
                        )
                    })
                })
                .flatten()
        })
        .flatten()
    })
}

/// Each sector's shared Live set (`binding::sectors::sector_instances`):
/// the set its shown layout's strips share, none when they differ (then
/// each strip names its own set on its side, `sector_side`). A memo of the
/// hub's document and this device's picks, so a strip that stays mounted
/// while its sector pages follows the new layout; the pool is built once
/// per change, never per strip.
pub(crate) fn instances_memo(
    layout: Arc<Layout>,
    doc: Signal<Option<SectorsDoc>>,
    picks: Signal<Picks>,
) -> Memo<BTreeMap<SectorId, Option<String>>> {
    Memo::new(move |_| {
        let pool = pool(&layout);
        doc.try_with(|doc| {
            doc.as_ref()
                .and_then(|doc| picks.try_with(|picks| sector_instances(doc, picks, &pool)))
        })
        .flatten()
        .unwrap_or_default()
    })
}

/// A side's window moved a step (`forward` or back), kept by its key.
fn step_sector_window(
    offsets: RwSignal<BTreeMap<WindowKey, usize>>,
    key: &WindowKey,
    shift: Option<Shift>,
    forward: bool,
) {
    let Some(now) = shift else {
        return;
    };
    let next = now.moved(forward);
    let _ = offsets.try_update(|all| {
        all.insert(key.clone(), next);
    });
}

/// The default page with sectors: its lines, the column (its head with the
/// ARRANGE key beside the page tab, #212; the LAYOUTS block, the arrows of a
/// side that does not fit, the rail and the global controls). Measured as `PageView` is
/// (`surface::measure_page`); `--strip-w` and `data-narrow` as there.
#[component]
pub(crate) fn SectorsView(page: Page, global: Vec<Control>) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let head = expect_context::<Head>();
    let store = nav.store;
    let body_ref = NodeRef::<html::Div>::new();
    let column_ref = NodeRef::<html::Nav>::new();
    let thin_ref = NodeRef::<html::Div>::new();
    let rail_ref = NodeRef::<html::Div>::new();
    let measured = measure_page(nav.viewport, body_ref, column_ref, thin_ref);
    let doc = Memo::new(move |_| {
        store
            .sectors
            .try_with(|told| told.shown().cloned())
            .flatten()
    });
    // ARRANGE (#148): the page shows the edited orders; a new document from
    // the hub ends the mode unsaved (`Arrange::doc_arrived`, also the first
    // look: a mode left from before a remount), and so does a hide.
    let mode = nav.arrange;
    let carrier = Carrier::new();
    let shown_doc = Memo::new(move |_| {
        doc.try_get()
            .flatten()
            .map(|doc| mode.try_with(|m| m.applied(&doc)).unwrap_or(doc))
    });
    let rev = Memo::new(move |_| doc.try_with(|d| d.as_ref().map(|d| d.rev)).flatten());
    // The pool's controls, once for the page (a layout change mounts it
    // anew), and each sector's shared Live set.
    let controls = StoredValue::new(pool_controls(&head.layout));
    let instances = instances_memo(head.layout.clone(), doc.into(), nav.picks.into());
    Effect::new(move |_| {
        let Some(rev) = rev.try_get().flatten() else {
            return;
        };
        let off = change_mode(mode, |m| {
            m.doc_arrived(rev);
            !m.is_on()
        });
        if off.unwrap_or(true) {
            carrier.leave();
        }
    });
    let hidden = window_event_listener_untyped("visibilitychange", move |_| {
        if !dom::hidden() {
            return;
        }
        carrier.leave();
        let _ = change_mode(mode, |m| m.ended_by(ArrangeEnd::Hidden));
    });
    on_cleanup(move || hidden.remove());
    let refused = RwSignal::new(None::<&'static str>);
    let note = move || {
        let on = mode.try_with(Arrange::is_on).unwrap_or(false);
        arrange_note(on, refused.try_get().flatten())
    };
    // Each side's window, kept by its sectors (a phone, #148 call 14).
    let offsets = RwSignal::new(BTreeMap::<WindowKey, usize>::new());
    let arranging_now = Memo::new(move |_| mode.try_with(Arrange::is_on).unwrap_or(false));
    let arranged = sector_memo(
        head.layout.clone(),
        measured.into(),
        shown_doc.into(),
        nav.picks.into(),
        offsets.into(),
        arranging_now.into(),
    );
    let count = Memo::new(move |_| {
        arranged
            .try_with(|a| a.as_ref().map_or(0, |a| a.lines.len()))
            .unwrap_or(0)
    });
    let windows = Memo::new(move |_| {
        arranged
            .try_with(|a| {
                a.as_ref()
                    .map(SectorArrangement::windows)
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    });
    // The lines share the height; the strip width every line shares,
    // floored (a side never overflows by rounding).
    let floored = move || {
        arranged
            .try_with(|a| a.as_ref().map(|a| (a.width * 100.0).floor() / 100.0))
            .flatten()
    };
    let body_style = move || {
        let lines = count.try_get().unwrap_or(0).max(1);
        floored()
            .map(|width| {
                format!("grid-template-rows:repeat({lines},minmax(0,1fr));--strip-w:{width:.2}px;")
            })
            .unwrap_or_default()
    };
    let narrow_flags = move || floored().map(|width| narrow(width, &METRICS));
    let narrow_attr = move || narrow_flags().is_some_and(|n| n.normal).to_string();
    let narrow_wide_attr = move || narrow_flags().is_some_and(|n| n.wide).to_string();
    let rail = page
        .rail
        .into_iter()
        .map(|control| view! { <ControlView control=control /> })
        .collect_view();
    let global = global
        .into_iter()
        .map(|control| view! { <ControlView control=control /> })
        .collect_view();
    let line_view = move |line: usize| {
        view! {
            <SectorLineView
                arranged=arranged
                line=line
                carrier=carrier
                controls=controls
                instances=instances
            />
        }
    };
    let shift_view = move |key: WindowKey| {
        view! { <SectorShiftView arranged=arranged offsets=offsets key=key /> }
    };
    // The phone's thin strip keeps its ☰; its ◀ ▶ stay hidden here (the
    // column holds each side's arrows; the phone design follows on #148).
    let no_lines = Memo::new(move |_| None::<Arrangement>);
    let no_offsets = RwSignal::new(BTreeMap::<LineKey, usize>::new());
    // The phone's column over the faders (#63), as on every page.
    let close = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        nav.open_column(false);
    };
    let close_keys: Vec<String> = Vec::new();
    let open = move || nav.column_open.try_get().unwrap_or(false);
    let shade = move || {
        open().then(|| {
            let shade_keys: Vec<String> = Vec::new();
            view! {
                <div
                    class="column-shade"
                    use:owns_touches=shade_keys
                    data-testid="column-shade"
                    on:pointerdown=close
                ></div>
            }
        })
    };
    let arranging = move || mode.try_with(Arrange::is_on).unwrap_or(false).to_string();
    let stream = stream_readouts(&head.layout, shown_doc, nav.picks);
    // ARRANGE's key beside the page tab (#212, the owner's approved
    // `docs/mockups/column-calm-v2.html`).
    let arrange_key = (
        page.id.clone(),
        ViewFnOnce::from(move || view! { <ArrangeButton carrier=carrier refused=refused /> }),
    );
    view! {
        <div
            class="body"
            data-testid="page"
            data-page=page.id
            data-sectors="true"
            data-arrange=arranging
            data-column-open=move || open().to_string()
            data-extra=extra_attr(nav)
            node_ref=body_ref
            style=body_style
            data-narrow=narrow_attr
            data-narrow-wide=narrow_wide_attr
        >
            <For each=move || 0..count.try_get().unwrap_or(0) key=|line| *line children=line_view />
            <ThinStrip arranged=no_lines offsets=no_offsets node=thin_ref />
            {shade}
            <nav class="column" data-testid="column" node_ref=column_ref>
                <button
                    type="button"
                    class="column-close"
                    use:owns_touches=close_keys
                    data-testid="column-close"
                    on:pointerdown=close
                >
                    "✕ CLOSE"
                </button>
                <ColumnHead beside=arrange_key />
                <LayoutsBlock shown=shown_doc />
                <div class="saved-note" data-testid="arrange-note">{note}</div>
                {stream}
                <div class="shifts">
                    <For
                        each=move || windows.try_get().unwrap_or_default()
                        key=|key: &WindowKey| key.clone()
                        children=shift_view
                    />
                </div>
                <div class="rail" data-testid="rail" node_ref=rail_ref>
                    <div class="rail-main">{rail}</div>
                    <StageMini rail=rail_ref />
                    <div class="rail-foot">{global}</div>
                </div>
                <ColumnFoot />
            </nav>
        </div>
    }
}

/// One line: its cells in one keyed list (`SectorCell::key`), the left
/// sector's then the right one's, each on its side of the column
/// (`page.css`: `.split` takes the rest of the left half and the column).
#[component]
fn SectorLineView(
    arranged: Memo<Option<SectorArrangement>>,
    line: usize,
    carrier: Carrier,
    controls: StoredValue<BTreeMap<StripRef, Control>>,
    instances: Memo<BTreeMap<SectorId, Option<String>>>,
) -> impl IntoView {
    let list = Memo::new(move |_| {
        arranged
            .try_with(|a| {
                a.as_ref()
                    .and_then(|a| a.lines.get(line))
                    .map(|l| {
                        l.left
                            .cells
                            .iter()
                            .chain(l.right.cells.iter())
                            .cloned()
                            .collect::<Vec<SectorCell>>()
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    });
    // The line's row, and its left side's span (`--l-units` strips and
    // `--l-cells` cells).
    let place = move || {
        let (units, cells) = arranged
            .try_with(|a| {
                a.as_ref().and_then(|a| a.lines.get(line)).map(|l| {
                    (
                        l.left.cells.iter().map(|c| c.units).sum::<f64>(),
                        l.left.cells.len(),
                    )
                })
            })
            .flatten()
            .unwrap_or((0.0, 0));
        format!("grid-row:{};--l-units:{units};--l-cells:{cells};", line + 1)
    };
    let cell = move |cell: SectorCell| {
        view! {
            <SectorCellView
                cell=cell
                line=line
                arranged=arranged
                carrier=carrier
                controls=controls
                instances=instances
            />
        }
    };
    view! {
        <div
            class="line"
            data-testid="line"
            data-line=line.to_string()
            data-captions="false"
            style=place
        >
            <div class="split" aria-hidden="true"></div>
            <For each=move || list.try_get().unwrap_or_default() key=SectorCell::key children=cell />
        </div>
    }
}

/// One cell of a sector: an empty slot's blank (`sectors.css` outlines it
/// only in ARRANGE), else a strip's pool control on its sector's side,
/// with, while the layout its sector shows now mixes Live
/// sets, its own set as the side text (a memo of `instances`: a strip kept
/// through a page follows the new layout); ARRANGE's cover over it (shown
/// only in the mode). `data-slot` is its layout slot now (a kept strip's
/// element moves between slots).
#[component]
fn SectorCellView(
    cell: SectorCell,
    line: usize,
    arranged: Memo<Option<SectorArrangement>>,
    carrier: Carrier,
    controls: StoredValue<BTreeMap<StripRef, Control>>,
    instances: Memo<BTreeMap<SectorId, Option<String>>>,
) -> impl IntoView {
    let sector = cell.sector;
    let side = if sector.left() { "left" } else { "right" };
    let Some(strip) = cell.strip.clone() else {
        return view! {
            <div
                class="slot sector-gap"
                data-sector=sector.as_str()
                data-side=side
                data-slot=cell.slot.to_string()
                data-testid="sector-gap"
            ></div>
        }
        .into_any();
    };
    let key = cell.key();
    let first_slot = cell.slot;
    let slot_now = Memo::new(move |_| {
        arranged
            .try_with(|a| {
                a.as_ref()
                    .and_then(|a| a.cell_of(line, &key))
                    .map(|c| c.slot)
            })
            .flatten()
            .unwrap_or(first_slot)
    });
    let control = controls
        .try_with_value(|all| all.get(&strip).cloned())
        .flatten();
    // A mixed layout names the strip's set on its side (`sector_side`), a
    // memo of `instances`, so a strip kept through a page follows the
    // layout shown; otherwise the strip's own set mark says it (#211).
    let own_side = match &control {
        Some(Control::Strip(strip)) => strip.binding.instance.clone(),
        _ => String::new(),
    };
    let side_text = Memo::new(move |_| {
        instances
            .try_with(|all| sector_side(all.get(&sector).and_then(Option::as_deref), &own_side))
            .flatten()
    });
    let inner = control.map(|control| {
        view! { <ControlView control=control group_name=side_text /> }
    });
    let slot_attr = move || slot_now.try_get().unwrap_or(first_slot).to_string();
    view! {
        <div class="slot" data-sector=sector.as_str() data-side=side data-slot=slot_attr>
            {inner}
            <ArrangeCover sector=sector strip=strip place=slot_now carrier=carrier />
        </div>
    }
    .into_any()
}

/// A side's arrows in the column (a side that does not fit): ◀ and ▶ move
/// its window a step, and between them where it stands.
#[component]
fn SectorShiftView(
    arranged: Memo<Option<SectorArrangement>>,
    offsets: RwSignal<BTreeMap<WindowKey, usize>>,
    key: WindowKey,
) -> impl IntoView {
    let nav = expect_context::<Nav>();
    // A left side's arrows under an extra tab move hidden strips (#148
    // review): inert and dimmed.
    let left = key.sectors.iter().all(|s| s.left());
    let covered = move || left && nav.left_covered();
    let sectors = key
        .sectors
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let key = StoredValue::new(key);
    let shift = Memo::new(move |_| {
        key.try_with_value(|key| {
            arranged
                .try_with(|a| a.as_ref().and_then(|a| a.shift_of(key)))
                .flatten()
        })
        .flatten()
    });
    let step = move |forward: bool| {
        if left && nav.left_covered_now() {
            return;
        }
        let _ = key.try_with_value(|key| {
            step_sector_window(offsets, key, shift.try_get_untracked().flatten(), forward);
        });
    };
    let on_back = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        step(false);
    };
    let on_next = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        step(true);
    };
    let now = move || shift.try_get().flatten();
    let at_start = move || now().is_none_or(|s| s.offset == 0);
    let at_end = move || now().is_none_or(|s| s.offset >= s.last);
    let label = move || now().map(|s| s.label()).unwrap_or_default();
    // The arrows own their touches (#43 PR G); they write no key.
    let back_keys: Vec<String> = Vec::new();
    let next_keys: Vec<String> = Vec::new();
    view! {
        <div
            class="shift"
            data-testid="shift"
            data-sectors=sectors
            aria-disabled=move || covered().to_string()
        >
            <button
                type="button"
                class="btn shift-btn"
                use:owns_touches=back_keys
                class:end=at_start
                data-testid="shift-back"
                on:pointerdown=on_back
            >
                "◀"
            </button>
            <span class="shift-where" data-testid="shift-where">{label}</span>
            <button
                type="button"
                class="btn shift-btn"
                use:owns_touches=next_keys
                class:end=at_end
                data-testid="shift-next"
                on:pointerdown=on_next
            >
                "▶"
            </button>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::sectors::{SectorLayout, StripRef};
    use serde_json::{Value, json};

    fn strip(name: &str) -> Value {
        json!({"kind": "strip", "binding": {"instance": "band", "anchor": {"kind": "track", "name": name}},
               "strip_kind": "standard"})
    }

    fn sl(id: &str, strips: &[&str]) -> SectorLayout {
        SectorLayout {
            id: id.to_string(),
            title: id.to_uppercase(),
            strips: strips
                .iter()
                .map(|label| {
                    Some(StripRef {
                        instance: "band".to_string(),
                        label: label.to_string(),
                    })
                })
                .collect(),
        }
    }

    fn keys(side: &crate::arrange::sectors::SectorSide) -> Vec<String> {
        side.cells.iter().map(SectorCell::key).collect()
    }

    #[test]
    fn paging_one_sector_changes_only_its_cells_and_not_the_width() {
        let names = ["S1", "S2", "S3", "S4", "S5", "S6", "S7", "S8"];
        let controls: Vec<Value> = names.iter().map(|n| strip(n)).collect();
        let layout: Layout = serde_json::from_value(json!({
            "schema": 2, "default_page": "foh",
            "pages": [{"id": "foh", "title": "FOH",
                       "rows": [{"sections": [{"kind": "group", "id": "g", "controls": controls}]}]}]
        }))
        .expect("a layout");
        let doc = SectorsDoc {
            rev: 1,
            sectors: [
                (SectorId::Lu, vec![sl("lu-1", &["S1", "S2"])]),
                (SectorId::Ld, vec![sl("ld-1", &["S3"])]),
                (
                    SectorId::Ru,
                    vec![sl("ru-1", &["S4", "S5"]), sl("ru-2", &["S6", "S7", "S8"])],
                ),
                (SectorId::Rd, vec![sl("rd-1", &["S1"])]),
            ]
            .into_iter()
            .collect(),
        };
        let owner = Owner::new();
        owner.set();
        let measured = RwSignal::new((Sides::both(0.0), 822.0));
        let shown = RwSignal::new(Some(doc));
        let picks = RwSignal::new(Picks::new());
        let offsets = RwSignal::new(BTreeMap::<WindowKey, usize>::new());
        let arranging = RwSignal::new(false);
        let arranged = sector_memo(
            Arc::new(layout),
            measured.into(),
            shown.into(),
            picks.into(),
            offsets.into(),
            arranging.into(),
        );
        // Not laid out yet: nothing arranged.
        assert_eq!(arranged.get_untracked(), None);
        let _ = measured.try_set((Sides::both(519.0), 822.0));
        let before = arranged.get_untracked().expect("arranged");
        assert_eq!(before.lines.len(), 2);
        assert_eq!(
            keys(&before.lines[0].right),
            vec!["ru:band|S4", "ru:band|S5"]
        );
        // ru pages to ru-2: only ru's cells change.
        let _ = picks.try_update(|p| {
            p.insert(SectorId::Ru, "ru-2".to_string());
        });
        let after = arranged.get_untracked().expect("arranged");
        assert_eq!(after.width, before.width);
        assert_eq!(
            keys(&after.lines[0].right),
            vec!["ru:band|S6", "ru:band|S7", "ru:band|S8"]
        );
        assert_eq!(after.lines[0].left, before.lines[0].left);
        assert_eq!(after.lines[1], before.lines[1]);
        // ARRANGE on: the further positions are empty cells, the width
        // stays; off again: gone.
        let _ = arranging.try_set(true);
        let on = arranged.get_untracked().expect("arranged");
        assert_eq!(on.width, before.width);
        assert_eq!(
            keys(&on.lines[0].right),
            vec!["ru:band|S6", "ru:band|S7", "ru:band|S8", "ru:#3"]
        );
        let _ = arranging.try_set(false);
        assert_eq!(arranged.get_untracked(), Some(after));
        // No document: nothing arranged.
        let _ = shown.try_set(None);
        assert_eq!(arranged.get_untracked(), None);
    }

    fn strip_of(instance: &str, name: &str) -> Value {
        json!({"kind": "strip", "binding": {"instance": instance, "anchor": {"kind": "track", "name": name}},
               "strip_kind": "standard"})
    }

    #[test]
    fn a_kept_strips_side_set_follows_its_sectors_page() {
        // band X stays mounted while lu pages from [band X, band Y] (one
        // set: names nothing) to [band X, master A-Reverb #] (mixed: names
        // each set).
        let layout: Layout = serde_json::from_value(json!({
            "schema": 2, "default_page": "foh",
            "pages": [{"id": "foh", "title": "FOH",
                       "rows": [{"sections": [{"kind": "group", "id": "g", "controls": [
                           strip_of("band", "X"), strip_of("band", "Y"),
                           strip_of("master", "A-Reverb #"), strip_of("band", "A-Reverb #")]}]}]}]
        }))
        .expect("a layout");
        let at = |instance: &str, label: &str| StripRef {
            instance: instance.to_string(),
            label: label.to_string(),
        };
        let doc = SectorsDoc {
            rev: 1,
            sectors: [(
                SectorId::Lu,
                vec![
                    SectorLayout {
                        id: "lu-1".to_string(),
                        title: "BAND".to_string(),
                        strips: vec![Some(at("band", "X")), Some(at("band", "Y"))],
                    },
                    SectorLayout {
                        id: "lu-2".to_string(),
                        title: "MIX".to_string(),
                        strips: vec![Some(at("band", "X")), Some(at("master", "A-Reverb #"))],
                    },
                ],
            )]
            .into_iter()
            .collect(),
        };
        let owner = Owner::new();
        owner.set();
        let shown = RwSignal::new(Some(doc));
        let picks = RwSignal::new(Picks::new());
        let instances = instances_memo(Arc::new(layout.clone()), shown.into(), picks.into());
        let lu = move || {
            instances
                .get_untracked()
                .get(&SectorId::Lu)
                .cloned()
                .flatten()
        };
        assert_eq!(lu().as_deref(), Some("band"));
        let _ = picks.try_update(|p| {
            p.insert(SectorId::Lu, "lu-2".to_string());
        });
        assert_eq!(
            lu(),
            None,
            "a mixed layout tags every strip, the kept one too"
        );
        let _ = picks.try_update(|p| {
            p.insert(SectorId::Lu, "lu-1".to_string());
        });
        assert_eq!(lu().as_deref(), Some("band"));
        assert_eq!(instances.get_untracked().get(&SectorId::Ru), Some(&None));
        // The pool's controls, built once for the page, by identity.
        let controls = pool_controls(&layout);
        assert_eq!(controls.len(), 4);
        assert!(matches!(
            controls.get(&at("master", "A-Reverb #")),
            Some(Control::Strip(strip)) if strip.binding.instance == "master"
        ));
        assert_eq!(controls.get(&at("band", "Gone")), None);
        let _ = shown.try_set(None);
        assert!(instances.get_untracked().values().all(Option::is_none));
    }
}
