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
//! dimmed and inert, a strip is dragged within its sector, and a second tap
//! saves; nothing reaches Live) and ↶ (the hub's undo of the last change,
//! dim while the hub holds no version before it), then the arrows of a side
//! that does not fit, the rail and the global controls. The group's name on
//! a strip's side is not drawn here; its instance tag follows the shown
//! layout.

use std::collections::BTreeMap;
use std::sync::Arc;

use fohmixer_proto::layout::{Control, Layout, Page};
use fohmixer_proto::sectors::{SectorId, SectorsDoc, pool};
use leptos::html;
use leptos::prelude::*;

use crate::arrange::sectors::{SectorArrangement, SectorCell, WindowKey, arrange_sectors};
use crate::arrange::{Arrangement, LineKey, METRICS, Shift, Sides, narrow};
use crate::behave::arrange_drag::{
    Arrange, ArrangeEnd, Drag, NoteTimer, Tap, arrange_note, sector_refs,
};
use crate::binding::sectors::{Picks, picked, sector_name, shown_instance};
use crate::components::{ControlView, down_of, fail_flash, owns_touches};
use crate::dom;
use crate::pages::surface::{ColumnHead, Head, Nav, ThinStrip, measure_page};
use crate::store::{LiveStore, ResultFn, on_refusal};

/// How long the refused save's note shows under ARRANGE (ms).
const NOTE_MS: u64 = 4000;

/// The sectors page's arrangement, a memo of what it depends on: the
/// measured page (none before it is laid out), the hub's document (none
/// while there is none), this device's picks and the sides' windows. Every
/// read tolerates a disposed value (`try_*`): a layout change disposes the
/// page while its keyed lists' effects may still run once.
pub(crate) fn sector_memo(
    layout: Arc<Layout>,
    measured: Signal<(Sides, f64)>,
    doc: Signal<Option<SectorsDoc>>,
    picks: Signal<Picks>,
    offsets: Signal<BTreeMap<WindowKey, usize>>,
) -> Memo<Option<SectorArrangement>> {
    Memo::new(move |_| {
        let (sides, height) = measured.try_get()?;
        if !sides.measured() {
            return None;
        }
        let pool = pool(&layout);
        doc.try_with(|doc| {
            let doc = doc.as_ref()?;
            picks
                .try_with(|picks| {
                    offsets.try_with(|offsets| {
                        let offset = |key: &WindowKey| offsets.get(key).copied().unwrap_or(0);
                        arrange_sectors(doc, &pool, picks, sides, height, offset, &METRICS)
                    })
                })
                .flatten()
        })
        .flatten()
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

/// The centres (px, the viewport's x) of `sector`'s slots on `slot`'s
/// line, in their order (the line's children: one keyed list).
fn sector_centres(slot: &web_sys::Element, sector: SectorId) -> Vec<f64> {
    let Some(line) = slot.parent_element() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut next = line.first_element_child();
    while let Some(el) = next {
        if el.get_attribute("data-sector").as_deref() == Some(sector.as_str()) {
            let rect = el.get_bounding_client_rect();
            out.push(rect.left() + rect.width() / 2.0);
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
            .try_with(|told| told.shown().map(|told| told.doc.clone()))
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
    let arranged = sector_memo(
        head.layout.clone(),
        measured.into(),
        shown_doc.into(),
        nav.picks.into(),
        offsets.into(),
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
        view! { <SectorLineView arranged=arranged line=line carrier=carrier /> }
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
                <LayoutsBlock />
                <div class="sector-actions">
                    <ArrangeButton carrier=carrier refused=refused />
                    <UndoButton />
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
        view! { <SectorCellView cell=cell line=line arranged=arranged carrier=carrier /> }
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

/// One strip of a sector: its pool control on its sector's side, with the
/// instance tag of the layout its sector shows when it was mounted and no
/// group name; ARRANGE's cover over it (shown only in the mode).
#[component]
fn SectorCellView(
    cell: SectorCell,
    line: usize,
    arranged: Memo<Option<SectorArrangement>>,
    carrier: Carrier,
) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let head = expect_context::<Head>();
    let pool = pool(&head.layout);
    let control = pool
        .iter()
        .find(|p| p.id == cell.strip)
        .map(|p| p.control.clone());
    let picks = nav.picks.try_get_untracked().unwrap_or_default();
    let shared = nav
        .store
        .sectors
        .try_with_untracked(|told| {
            told.shown()
                .and_then(|told| shown_instance(&told.doc, &picks, cell.sector, &pool))
        })
        .flatten();
    let side = if cell.sector.left() { "left" } else { "right" };
    let inner = control.map(|control| view! { <ControlView control=control shared=shared /> });
    let sector = cell.sector;
    view! {
        <div class="slot" data-sector=sector.as_str() data-side=side>
            {inner}
            <ArrangeCover cell=cell line=line arranged=arranged carrier=carrier />
        </div>
    }
}

/// ARRANGE's cover over a strip (`sectors.css` shows it only in the mode):
/// it owns its touches and captures the pointer, so no touch reaches the
/// strip's controls. A finger moved past the slop carries the strip (its
/// slot follows the finger); the lift drops it among its sector's strips
/// (`Arrange::dropped_in` over the centres read at the down). A cancel or a
/// lost capture puts it back with nothing moved; a carry whose end never
/// came ends at the next primary down of its type on a cover (#124).
#[component]
fn ArrangeCover(
    cell: SectorCell,
    line: usize,
    arranged: Memo<Option<SectorArrangement>>,
    carrier: Carrier,
) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let store = nav.store;
    let mode = nav.arrange;
    let cell = StoredValue::new(cell);
    let on_down = move |ev: web_sys::PointerEvent| {
        if !mode.try_with_untracked(Arrange::is_on).unwrap_or(false) {
            return;
        }
        ev.prevent_default();
        let Some(el) = dom::current_element(&ev) else {
            return;
        };
        let Some(slot) = el.closest(".slot").ok().flatten() else {
            return;
        };
        let Some((sector, strip)) = cell.try_with_value(|c| (c.sector, c.strip.clone())) else {
            return;
        };
        let centres = sector_centres(&slot, sector);
        let x = f64::from(ev.client_x());
        let down = down_of(&ev);
        let took = carrier
            .drag
            .try_update_value(|d| d.down(down, sector, strip, x, centres))
            .unwrap_or(false);
        if took {
            carrier.hold(slot);
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
        let shown = arranged
            .try_with_untracked(|a| a.as_ref().map(|a| sector_refs(a, line, carry.sector)))
            .flatten()
            .unwrap_or_default();
        let picks = nav.picks.try_get_untracked().unwrap_or_default();
        let _ = store.sectors.try_with_untracked(|told| {
            let Some(layout) = told
                .shown()
                .and_then(|told| picked(&told.doc, &picks, carry.sector))
            else {
                return;
            };
            change_mode(mode, |m| {
                m.dropped_in(layout, &shown, &carry.centres, &carry.strip, x);
            });
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

/// The LAYOUTS block: the sectors' cells as the screen has them (`lu ru`
/// over `ld rd`).
#[component]
fn LayoutsBlock() -> impl IntoView {
    let cells = [SectorId::Lu, SectorId::Ru, SectorId::Ld, SectorId::Rd]
        .into_iter()
        .map(|sector| view! { <LayoutCell sector=sector /> })
        .collect_view();
    view! {
        <div class="cap-title" data-testid="sector-layouts-title">"Layouts"</div>
        <div class="sectors" role="group" aria-label="sector layouts" data-testid="sector-layouts">
            {cells}
        </div>
    }
}

/// A sector's cell of the block: a tab per layout, the shown one marked; a
/// tap pages the sector to it (on `pointerdown`).
#[component]
fn LayoutCell(sector: SectorId) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let store = nav.store;
    let tabs = Memo::new(move |_| {
        store
            .sectors
            .try_with(|told| {
                told.shown()
                    .map(|told| {
                        told.doc
                            .layouts(sector)
                            .iter()
                            .map(|l| (l.id.clone(), l.title.clone()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    });
    let selected = Memo::new(move |_| {
        store
            .sectors
            .try_with(|told| {
                let told = told.shown()?;
                nav.picks
                    .try_with(|picks| picked(&told.doc, picks, sector).map(|l| l.id.clone()))
                    .flatten()
            })
            .flatten()
    });
    let tab = move |(id, title): (String, String)| {
        let lit = {
            let id = id.clone();
            move || selected.try_get().flatten().as_deref() == Some(id.as_str())
        };
        let pressed = lit.clone();
        let tap = id.clone();
        // A tab owns its touches (#43 PR G); it writes no key.
        let no_keys: Vec<String> = Vec::new();
        view! {
            <button
                type="button"
                class="tab"
                use:owns_touches=no_keys
                class:selected=lit
                data-testid="sector-tab"
                data-layout=id
                aria-pressed=move || pressed().to_string()
                on:pointerdown=move |_| nav.pick(sector, &tap)
            >
                {title}
            </button>
        }
    };
    view! {
        <div class="sec-cell" data-sector=sector.as_str() title=sector_name(sector)>
            <For
                each=move || tabs.try_get().unwrap_or_default()
                key=|tab: &(String, String)| tab.clone()
                children=tab
            />
        </div>
    }
}

/// ARRANGE (#148): a tap starts the reorder mode; the next one sends the
/// changed layouts' orders as one `SectorOrders` and leaves once the hub's
/// new document arrives (a refusal: at once, the hub's order shows again
/// with a short note), or
/// just leaves with nothing changed (`Arrange::tap`). Lit while on; it
/// stays live while the rest of the column is inert.
#[component]
fn ArrangeButton(carrier: Carrier, refused: RwSignal<Option<&'static str>>) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let timer = StoredValue::new(NoteTimer::default());
    let store = nav.store;
    let mode = nav.arrange;
    let on = Memo::new(move |_| mode.try_with(Arrange::is_on).unwrap_or(false));
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let Some(doc) = store
            .sectors
            .try_with_untracked(|told| told.shown().map(|told| told.doc.clone()))
            .flatten()
        else {
            return;
        };
        let _ = refused.try_set(None);
        let tap = change_mode(mode, |m| m.tap(&doc)).flatten();
        carrier.leave();
        if let Some(Tap::Send(orders, rev)) = tap {
            let done: ResultFn = Box::new(move |outcome| {
                let note = change_mode(mode, |m| m.heard(&outcome)).flatten();
                if let Some(note) = note {
                    flash_note(refused, timer, note);
                }
            });
            store.sector_orders(orders, rev, Some(done));
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
                on:pointerdown=on_down
            >
                "ARRANGE"
            </button>
        </div>
    }
}

/// ↶: the hub's undo of the last change (a tick or an ARRANGE save; a
/// second ↶ redoes it). Dim (`aria-disabled`) while the hub holds no
/// version before it; a tap sends the undo only while the hub's document is
/// fresh and says it holds one, and a refusal flashes the button.
#[component]
fn UndoButton() -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let undo = Memo::new(move |_| {
        store
            .sectors
            .try_with(|told| told.shown().is_some_and(|told| told.undo))
            .unwrap_or(false)
    });
    let failed = RwSignal::new(false);
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let can = store
            .sectors
            .try_with_untracked(|told| told.fresh().is_some_and(|told| told.undo))
            .unwrap_or(false);
        if can {
            store.sector_undo(Some(on_refusal(fail_flash(failed))));
        }
    };
    // It owns its touches (#43 PR G); it writes no Live key.
    let no_keys: Vec<String> = Vec::new();
    view! {
        <button
            type="button"
            class="undo-btn"
            use:owns_touches=no_keys
            class:failed=move || failed.try_get().unwrap_or(false)
            data-testid="sector-undo"
            aria-label="Undo the last layout change"
            aria-disabled=move || (!undo.try_get().unwrap_or(false)).to_string()
            on:pointerdown=on_down
        >
            "↶"
        </button>
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
        <div class="shift" data-testid="shift" data-sectors=sectors>
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
                .map(|label| StripRef {
                    instance: "band".to_string(),
                    label: label.to_string(),
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
        let arranged = sector_memo(
            Arc::new(layout),
            measured.into(),
            shown.into(),
            picks.into(),
            offsets.into(),
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
        // No document: nothing arranged.
        let _ = shown.try_set(None);
        assert_eq!(arranged.get_untracked(), None);
    }
}
