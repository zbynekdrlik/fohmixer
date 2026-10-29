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
//! (a 2 × 2 grid of the sectors' layout tabs: a tap pages that sector, on
//! this device only), ARRANGE (the reorder mode, `behave::arrange_drag`:
//! while it is on every strip lies under a cover with a ⠿ handle that owns
//! its touches, the faders, MUTE, SEL and the column's other controls are
//! dimmed and inert, every position a sector can show is a cell, the empty
//! ones calm dashed outlines, a strip is dragged within its sector onto an
//! empty cell (its old place left empty) or another strip (the two swap),
//! and a second tap saves; nothing reaches Live; in the mode the LAYOUTS
//! block stays live and renames, adds and removes layouts, mockup v2,
//! `behave::arrange_edit`, saved with the same tap; no undo, the owner's
//! ruling of 29.10.2026), then the arrows of a side
//! that does not fit, the rail and the global controls. No strip draws its
//! instance pill here (it costs 16 px of travel); a layout that mixes Live
//! sets names each strip's set where the group's name sits on the classic
//! page (`binding::sectors::sector_side`), following the shown layout. A
//! layout's empty slot is a blank cell in its place (the slot model,
//! 1.24.0), drawn only in ARRANGE.

use std::collections::BTreeMap;
use std::sync::Arc;

use fohmixer_proto::layout::{Control, Layout, Page};
use fohmixer_proto::sectors::{SectorId, SectorsDoc, StripRef, pool};
use leptos::html;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::arrange::sectors::{SectorArrangement, SectorCell, WindowKey, arrange_sectors};
use crate::arrange::{Arrangement, LineKey, METRICS, Shift, Sides, narrow};
use crate::behave::arrange_drag::{Arrange, ArrangeEnd, Drag, Held, NoteTimer, Tap, arrange_note};
use crate::behave::arrange_edit::{REMOVE_ASK, TabRow, TabTap, field_text, tab_rows};
use crate::binding::sectors::{
    Picks, picked, pool_controls, sector_instances, sector_name, sector_side,
};
use crate::components::{ControlView, down_of, owns_touches};
use crate::dom;
use crate::pages::surface::{
    ColumnFoot, ColumnHead, Head, Nav, ThinStrip, extra_attr, measure_page,
};
use crate::store::ResultFn;

/// How long the refused save's note shows under ARRANGE (ms).
const NOTE_MS: u64 = 4000;

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

/// ARRANGE's mode changed by `change` (written only when it changed:
/// every cell reads it); what `change` returned.
fn change_mode<R>(mode: RwSignal<Arrange>, change: impl FnOnce(&mut Arrange) -> R) -> Option<R> {
    let mut next = mode.try_get_untracked()?;
    let out = change(&mut next);
    if mode.try_with_untracked(|now| *now != next).unwrap_or(false) {
        let _ = mode.try_set(next);
    }
    Some(out)
}

/// The page's one drag in ARRANGE (`behave::arrange_drag::Drag`) and the
/// strip's slot it moves under the finger (an element, so kept locally).
#[derive(Clone, Copy)]
struct Carrier {
    drag: StoredValue<Drag>,
    slot: StoredValue<Option<web_sys::Element>, LocalStorage>,
}

impl Carrier {
    fn new() -> Self {
        Self {
            drag: StoredValue::new(Drag::default()),
            slot: StoredValue::new_local(None),
        }
    }

    /// The carried slot follows the finger by `dx` px (its own `style`:
    /// the slot has no reactive one).
    fn show(self, dx: f64) {
        let _ = self.slot.try_with_value(|slot| {
            if let Some(slot) = slot {
                dom::set_attr(slot, "style", &format!("transform:translateX({dx:.1}px);"));
                dom::set_attr(slot, "data-carried", "true");
            }
        });
    }

    /// The carried slot goes back to its place.
    fn lift(self) {
        if let Some(Some(slot)) = self.slot.try_update_value(Option::take) {
            dom::remove_attr(&slot, "style");
            dom::remove_attr(&slot, "data-carried");
        }
    }

    /// The slot the drag holds now is `slot` (a previous one put back).
    fn hold(self, slot: web_sys::Element) {
        self.lift();
        let _ = self.slot.try_set_value(Some(slot));
    }

    /// No carry is held any more: its slot goes back.
    fn settle(self) {
        if self
            .drag
            .try_with_value(|d| d.held().is_none())
            .unwrap_or(true)
        {
            self.lift();
        }
    }

    /// Every carry ends with nothing moved (the mode ended, a hide).
    fn leave(self) {
        let _ = self.drag.try_update_value(Drag::leave);
        self.lift();
    }
}

/// The cells of `sector` on `slot`'s line (the line's children: one keyed
/// list), each its layout slot (`data-slot`) and its centre (px, the
/// viewport's x).
fn sector_spots(slot: &web_sys::Element, sector: SectorId) -> Vec<(usize, f64)> {
    let Some(line) = slot.parent_element() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut next = line.first_element_child();
    while let Some(el) = next {
        let index = el
            .get_attribute("data-slot")
            .and_then(|n| n.parse::<usize>().ok());
        if let Some(index) = index
            && el.get_attribute("data-sector").as_deref() == Some(sector.as_str())
        {
            let rect = el.get_bounding_client_rect();
            out.push((index, rect.left() + rect.width() / 2.0));
        }
        next = el.next_element_sibling();
    }
    out
}

/// The refused save's note shows for `NOTE_MS`; its timer clears it only
/// while no newer note replaced it (`NoteTimer`).
fn flash_note(
    note: RwSignal<Option<&'static str>>,
    timer: StoredValue<NoteTimer>,
    text: &'static str,
) {
    let Some(mine) = timer.try_update_value(NoteTimer::show) else {
        return;
    };
    let _ = note.try_set(Some(text));
    set_timeout(
        move || {
            if timer.try_with_value(|t| t.expires(mine)).unwrap_or(false) {
                let _ = note.try_set(None);
            }
        },
        std::time::Duration::from_millis(NOTE_MS),
    );
}

/// The default page with sectors: its lines, the column (its head, the
/// LAYOUTS block, ARRANGE and ↶, the arrows of a side that does not fit,
/// the rail and the global controls). Measured as `PageView` is
/// (`surface::measure_page`); `--strip-w` and `data-narrow` as there.
#[component]
pub(crate) fn SectorsView(page: Page, global: Vec<Control>) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let head = expect_context::<Head>();
    let store = nav.store;
    let body_ref = NodeRef::<html::Div>::new();
    let column_ref = NodeRef::<html::Nav>::new();
    let thin_ref = NodeRef::<html::Div>::new();
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
    // anew), and each sector's instance tag.
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
                <ColumnHead />
                <LayoutsBlock shown=shown_doc />
                <div class="sector-actions">
                    <ArrangeButton carrier=carrier refused=refused />
                </div>
                <div class="saved-note" data-testid="arrange-note">{note}</div>
                <div class="shifts">
                    <For
                        each=move || windows.try_get().unwrap_or_default()
                        key=|key: &WindowKey| key.clone()
                        children=shift_view
                    />
                </div>
                <div class="rail" data-testid="rail">
                    <div class="rail-main">{rail}</div>
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
/// only in ARRANGE), else a strip's pool control on its sector's side, with
/// no instance pill and, while the layout its sector shows now mixes Live
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
    // The strip's own Live set as its `shared`: it never draws its
    // instance pill here (1.24.0: 16 px of travel, a bright fill); a mixed
    // layout names the set on its side instead (`sector_side`), a memo of
    // `instances`, so a strip kept through a page follows the layout shown.
    let own = match &control {
        Some(Control::Strip(strip)) => Some(strip.binding.instance.clone()),
        _ => None,
    };
    let own_side = own.clone().unwrap_or_default();
    let shared = Signal::derive(move || own.clone());
    let side_text = Memo::new(move |_| {
        instances
            .try_with(|all| sector_side(all.get(&sector).and_then(Option::as_deref), &own_side))
            .flatten()
    });
    let inner = control.map(|control| {
        view! { <ControlView control=control shared=shared group_name=side_text /> }
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

/// ARRANGE's cover over a strip (`sectors.css` shows it only in the mode):
/// it owns its touches and captures the pointer, so no touch reaches the
/// strip's controls. A finger moved past the slop carries the strip (its
/// slot follows the finger); the lift drops it on the nearest cell of its
/// sector, empty or a strip (`Arrange::dropped_in` over the cells read at
/// the down). A cancel or a lost capture puts it back with nothing moved; a
/// carry whose end never came ends at the next primary down of its type on
/// a cover (#124).
#[component]
fn ArrangeCover(
    sector: SectorId,
    strip: StripRef,
    place: Memo<usize>,
    carrier: Carrier,
) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let store = nav.store;
    let mode = nav.arrange;
    let strip = StoredValue::new(strip);
    let on_down = move |ev: web_sys::PointerEvent| {
        if !mode.try_with_untracked(Arrange::is_on).unwrap_or(false) {
            return;
        }
        ev.prevent_default();
        let Some(el) = dom::current_element(&ev) else {
            return;
        };
        let Some(slot_el) = el.closest(".slot").ok().flatten() else {
            return;
        };
        let (Some(strip), Some(from)) = (strip.try_get_value(), place.try_get_untracked()) else {
            return;
        };
        let held = Held {
            sector,
            strip,
            slot: from,
            spots: sector_spots(&slot_el, sector),
        };
        let x = f64::from(ev.client_x());
        let down = down_of(&ev);
        let took = carrier
            .drag
            .try_update_value(|d| d.down(down, held, x))
            .unwrap_or(false);
        if took {
            carrier.hold(slot_el);
            let _ = el.set_pointer_capture(down.id);
        }
    };
    let on_move = move |ev: web_sys::PointerEvent| {
        let x = f64::from(ev.client_x());
        let offset = carrier
            .drag
            .try_update_value(|d| d.moved(ev.pointer_id(), x))
            .flatten();
        if let Some(dx) = offset {
            carrier.show(dx);
        }
    };
    let on_up = move |ev: web_sys::PointerEvent| {
        let x = f64::from(ev.client_x());
        let drop = carrier
            .drag
            .try_update_value(|d| d.up(ev.pointer_id(), x))
            .flatten();
        carrier.settle();
        let Some((carry, x)) = drop else {
            return;
        };
        // The layout its sector shows now: the edit's (a page in ARRANGE).
        let picks = nav.picks.try_get_untracked().unwrap_or_default();
        let shown = store
            .sectors
            .try_with_untracked(|told| {
                told.shown()
                    .and_then(|doc| mode.try_with_untracked(|m| m.applied(doc)))
            })
            .flatten();
        let Some(layout) = shown
            .as_ref()
            .and_then(|doc| picked(doc, &picks, carry.sector))
            .map(|layout| layout.id.clone())
        else {
            return;
        };
        let _ = change_mode(mode, |m| {
            m.dropped_in(&layout, &carry.spots, carry.slot, &carry.strip, x);
        });
    };
    let on_cancel = move |ev: web_sys::PointerEvent| {
        let _ = carrier.drag.try_update_value(|d| d.cancel(ev.pointer_id()));
        carrier.settle();
    };
    // The cover owns its touches (#43 PR G); it writes no key.
    let no_keys: Vec<String> = Vec::new();
    view! {
        <div
            class="edit-cover"
            use:owns_touches=no_keys
            data-testid="edit-cover"
            on:pointerdown=on_down
            on:pointermove=on_move
            on:pointerup=on_up
            on:pointercancel=on_cancel
            on:lostpointercapture=on_cancel
        >
            <div class="grip">"⠿"<small>"DRAG"</small></div>
        </div>
    }
}

/// A pointer listener of the page (capture phase, removed with the block).
type DownListener = Closure<dyn FnMut(web_sys::PointerEvent)>;

/// Where a down keeps a pending REMOVE?: the asking tab and every ✕.
const ASK_SPOT: &str = ".tab.confirm, [data-testid=\"remove\"]";

/// Where a down keeps a rename going: its field.
const FIELD_SPOT: &str = ".tab-input";

/// The LAYOUTS block: the sectors' cells as the screen has them (`lu ru`
/// over `ld rd`), from `shown` (the hub's document, ARRANGE's edit while
/// the mode is on). In ARRANGE a down anywhere off the asking tab and the
/// ✕ keys drops a pending REMOVE?, and one off the field ends a rename
/// keeping its text (`Edit::outside_down`), before the down's own handler:
/// a capture-phase listener on the window.
#[component]
fn LayoutsBlock(shown: Memo<Option<SectorsDoc>>) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let mode = nav.arrange;
    let listener = Closure::wrap(Box::new(move |ev: web_sys::PointerEvent| {
        let pending = mode
            .try_with_untracked(|m| {
                m.edit_now()
                    .is_some_and(|e| e.confirming.is_some() || e.renaming.is_some())
            })
            .unwrap_or(false);
        if !pending {
            return;
        }
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok());
        let within = |spot: &str| {
            target
                .as_ref()
                .and_then(|t| t.closest(spot).ok().flatten())
                .is_some()
        };
        let (on_ask, on_field) = (within(ASK_SPOT), within(FIELD_SPOT));
        let _ = change_mode(mode, |m| {
            if let Some(edit) = m.edit() {
                edit.outside_down(on_ask, on_field);
            }
        });
        if !on_field {
            blur_field();
        }
    }) as Box<dyn FnMut(web_sys::PointerEvent)>);
    if let Some(window) = web_sys::window() {
        let _ = window.add_event_listener_with_callback_and_bool(
            "pointerdown",
            listener.as_ref().unchecked_ref(),
            true,
        );
    }
    let any_down = StoredValue::new_local(Some::<DownListener>(listener));
    on_cleanup(move || {
        if let (Some(Some(listener)), Some(window)) =
            (any_down.try_update_value(Option::take), web_sys::window())
        {
            let _ = window.remove_event_listener_with_callback_and_bool(
                "pointerdown",
                listener.as_ref().unchecked_ref(),
                true,
            );
        }
    });
    let cells = [SectorId::Lu, SectorId::Ru, SectorId::Ld, SectorId::Rd]
        .into_iter()
        .map(|sector| view! { <LayoutCell sector=sector shown=shown /> })
        .collect_view();
    view! {
        <div class="cap-title" data-testid="sector-layouts-title">"Layouts"</div>
        <div class="sectors" role="group" aria-label="sector layouts" data-testid="sector-layouts">
            {cells}
        </div>
    }
}

/// A rename field that has the focus lets go of it (its rename ended
/// elsewhere: the keyboard goes down).
fn blur_field() {
    let active = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element());
    if let Some(field) = active
        .filter(|el| el.matches(FIELD_SPOT).unwrap_or(false))
        .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = field.blur();
    }
}

/// A sector's cell of the block: a tab per layout of `shown`, the shown one
/// marked; a tap pages the sector to it (on `pointerdown`). In ARRANGE
/// each tab carries ✎ and ✕ (none on the sector's last layout) and the
/// cell a `+` while it holds fewer than four layouts (`TabRow`,
/// `Edit::can_add`); a new layout is shown at once on this device.
#[component]
fn LayoutCell(sector: SectorId, shown: Memo<Option<SectorsDoc>>) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let mode = nav.arrange;
    let rows = Memo::new(move |_| {
        shown
            .try_with(|doc| {
                let doc = doc.as_ref()?;
                nav.picks
                    .try_with(|picks| mode.try_with(|m| tab_rows(doc, picks, sector, m.edit_now())))
                    .flatten()
            })
            .flatten()
            .unwrap_or_default()
    });
    let ids = Memo::new(move |_| {
        rows.try_with(|rows| rows.iter().map(|r| r.id.clone()).collect::<Vec<_>>())
            .unwrap_or_default()
    });
    let can_add = Memo::new(move |_| {
        mode.try_with(|m| m.edit_now().is_some_and(|e| e.can_add(sector)))
            .unwrap_or(false)
    });
    // A left sector's cell under an extra tab (#148 review): inert and
    // dimmed, its layouts hidden under the pane.
    let covered = move || sector.left() && nav.left_covered();
    let on_add = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        if sector.left() && nav.left_covered_now() {
            return;
        }
        let added = change_mode(mode, |m| m.edit().and_then(|e| e.add(sector))).flatten();
        if let Some(id) = added {
            nav.pick(sector, &id);
        }
    };
    let add = move || {
        can_add.try_get().unwrap_or(false).then(|| {
            // It owns its touches (#43 PR G); it writes no key.
            let no_keys: Vec<String> = Vec::new();
            view! {
                <button
                    type="button"
                    class="tab-add"
                    use:owns_touches=no_keys
                    data-testid="add-layout"
                    aria-label="add a layout"
                    on:pointerdown=on_add
                >
                    "+"
                </button>
            }
        })
    };
    let tab = move |id: String| view! { <LayoutTab sector=sector layout=id rows=rows /> };
    view! {
        <div
            class="sec-cell"
            data-sector=sector.as_str()
            title=sector_name(sector)
            aria-disabled=move || covered().to_string()
        >
            <For each=move || ids.try_get().unwrap_or_default() key=|id: &String| id.clone() children=tab />
            {add}
        </div>
    }
}

/// One layout's tab (keyed by its id, so a rename's field keeps its focus
/// while its row changes): the tab (`REMOVE?` while its ✕ asks), its
/// rename field (a real `<input>`: the iPad's keyboard; hidden until ✎,
/// which fills it, focuses it and selects its text in the tap itself) and
/// its ✎ ✕ keys (`sectors.css` shows them only in ARRANGE).
#[component]
fn LayoutTab(sector: SectorId, layout: String, rows: Memo<Vec<TabRow>>) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let mode = nav.arrange;
    let id_attr = layout.clone();
    let tab_attr = layout.clone();
    let id = StoredValue::new(layout);
    let row = Memo::new(move |_| {
        id.try_with_value(|id| {
            rows.try_with(|rows| rows.iter().find(|r| r.id == *id).cloned())
                .flatten()
        })
        .flatten()
    });
    let field = NodeRef::<html::Input>::new();
    let flag = move |pick: fn(&TabRow) -> bool| {
        row.try_with(|r| r.as_ref().is_some_and(pick))
            .unwrap_or(false)
    };
    let asks = move || flag(|r| r.asks);
    let lit = move || flag(|r| r.selected && !r.asks);
    let renaming = move || flag(|r| r.renaming);
    let removable = move || flag(|r| r.removable);
    let text = move || {
        row.try_with(|r| {
            r.as_ref().map(|r| {
                if r.asks {
                    REMOVE_ASK.to_string()
                } else {
                    r.title.clone()
                }
            })
        })
        .flatten()
        .unwrap_or_default()
    };
    // A left sector's layouts under an extra tab take no tap (#148 review).
    let inert = move || sector.left() && nav.left_covered_now();
    let on_tab = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        if inert() {
            return;
        }
        let Some(id) = id.try_get_value() else {
            return;
        };
        let tapped = change_mode(mode, |m| m.edit().map(|e| e.tab_tapped(&id))).flatten();
        if tapped != Some(TabTap::Removed) {
            nav.pick(sector, &id);
        }
    };
    let on_pen = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        if inert() {
            return;
        }
        let Some(id) = id.try_get_value() else {
            return;
        };
        let title = change_mode(mode, |m| m.edit().and_then(|e| e.rename_start(&id))).flatten();
        if let (Some(title), Some(input)) = (title, field.try_get_untracked().flatten()) {
            input.set_value(&title);
            let _ = input.focus();
            input.select();
        }
    };
    let on_x = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        if inert() {
            return;
        }
        let _ = id.try_with_value(|id| {
            change_mode(mode, |m| {
                if let Some(edit) = m.edit() {
                    edit.ask_remove(id);
                }
            })
        });
    };
    let on_input = move || {
        let Some(input) = field.try_get_untracked().flatten() else {
            return;
        };
        let value = input.value();
        let shown = change_mode(mode, |m| m.edit().map(|e| e.typed(&value)))
            .flatten()
            .unwrap_or_else(|| field_text(&value));
        if shown != value {
            input.set_value(&shown);
        }
    };
    let finish = move |keep: bool| {
        let _ = id.try_with_value(|id| {
            change_mode(mode, |m| {
                if let Some(edit) = m.edit() {
                    edit.rename_end_of(id, keep);
                }
            })
        });
    };
    let on_key = move |ev: web_sys::KeyboardEvent| {
        let keep = match ev.key().as_str() {
            "Enter" => true,
            "Escape" => false,
            _ => return,
        };
        ev.prevent_default();
        finish(keep);
        if let Some(input) = field.try_get_untracked().flatten() {
            let _ = input.blur();
        }
    };
    let cross = move || {
        removable().then(|| {
            // It owns its touches (#43 PR G); it writes no key.
            let no_keys: Vec<String> = Vec::new();
            view! {
                <button
                    type="button"
                    class="tool x"
                    use:owns_touches=no_keys
                    data-testid="remove"
                    aria-label="remove the layout"
                    on:pointerdown=on_x
                >
                    "✕"
                </button>
            }
        })
    };
    // The tab and ✎ own their touches (#43 PR G); they write no key.
    let tab_keys: Vec<String> = Vec::new();
    let pen_keys: Vec<String> = Vec::new();
    view! {
        <div
            class="lay-ed"
            data-layout=id_attr
            data-renaming=move || renaming().to_string()
        >
            <button
                type="button"
                class="tab"
                use:owns_touches=tab_keys
                class:selected=lit
                class:confirm=asks
                data-testid="sector-tab"
                data-layout=tab_attr
                aria-pressed=move || lit().to_string()
                on:pointerdown=on_tab
            >
                {text}
            </button>
            <input
                class="tab-input"
                type="text"
                node_ref=field
                maxlength="8"
                autocapitalize="characters"
                autocomplete="off"
                spellcheck="false"
                enterkeyhint="done"
                aria-label="layout name"
                data-testid="rename-input"
                on:input=move |_| on_input()
                on:keydown=on_key
                on:blur=move |_| finish(true)
            />
            <div class="lay-tools">
                <button
                    type="button"
                    class="tool pen"
                    use:owns_touches=pen_keys
                    data-testid="rename"
                    aria-label="rename the layout"
                    on:pointerdown=on_pen
                >
                    "✎"
                </button>
                {cross}
            </div>
        </div>
    }
}

/// ARRANGE (#148): a tap starts the edit mode; the next one sends every
/// sector's layouts as one `SectorEdit` (a rename still in its field kept)
/// and leaves once the hub's new document arrives (a refusal: at once, the
/// hub's document shows again with a short note), or just leaves with
/// nothing changed (`Arrange::tap`). Lit while on; it stays live, and so
/// does the LAYOUTS block, while the rest of the column is inert.
#[component]
fn ArrangeButton(carrier: Carrier, refused: RwSignal<Option<&'static str>>) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let timer = StoredValue::new(NoteTimer::default());
    let store = nav.store;
    let mode = nav.arrange;
    let on = Memo::new(move |_| mode.try_with(Arrange::is_on).unwrap_or(false));
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        // Under an extra tab the left sectors it would arrange are hidden
        // (#148 review): inert and dimmed.
        if nav.left_covered_now() {
            return;
        }
        let Some(doc) = store
            .sectors
            .try_with_untracked(|told| told.shown().cloned())
            .flatten()
        else {
            return;
        };
        let _ = refused.try_set(None);
        let tap = change_mode(mode, |m| m.tap(&doc)).flatten();
        carrier.leave();
        if let Some(Tap::Send(sectors, rev)) = tap {
            let done: ResultFn = Box::new(move |outcome| {
                let note = change_mode(mode, |m| m.heard(&outcome)).flatten();
                if let Some(note) = note {
                    flash_note(refused, timer, note);
                }
            });
            store.sector_edit(sectors, rev, Some(done));
        }
    };
    let lit = move || on.try_get().unwrap_or(false);
    // It owns its touches (#43 PR G); it writes no key.
    let no_keys: Vec<String> = Vec::new();
    view! {
        <div class="seg arrange-btn">
            <button
                type="button"
                class="tab"
                use:owns_touches=no_keys
                class:selected=lit
                data-testid="arrange"
                aria-pressed=move || lit().to_string()
                aria-disabled=move || nav.left_covered().to_string()
                on:pointerdown=on_down
            >
                "ARRANGE"
            </button>
        </div>
    }
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
    fn a_kept_strips_instance_tag_follows_its_sectors_page() {
        // band X stays mounted while lu pages from [band X, band Y] (one
        // set: no tag) to [band X, master A-Reverb #] (mixed: tags).
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
