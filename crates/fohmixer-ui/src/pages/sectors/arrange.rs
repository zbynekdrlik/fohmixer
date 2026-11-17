//! ARRANGE's drag on the sectors page (#148; #233: onto any visible
//! sector's cell), the glue of `behave::arrange_drag`: the page's one carry
//! and the elements it moves and marks (`Carrier`), every visible sector's
//! cells read from the DOM at the down (`page_spots`), the layouts a drop's
//! two sectors show (`shown_layouts`) and the cover over each strip
//! (`ArrangeCover`). Every decision is the tested `Arrange`, `Drag`,
//! `drop_spot` and `landing`.

use std::sync::Arc;

use fohmixer_proto::layout::Layout;
use fohmixer_proto::sectors::{SectorId, StripRef, pool};
use leptos::prelude::*;

use crate::behave::arrange_drag::{Arrange, Drag, Held, Spot, drop_spot};
use crate::binding::sectors::picked;
use crate::components::{down_of, owns_touches};
use crate::dom;
use crate::pages::surface::{Head, Nav};

/// ARRANGE's mode changed by `change` (written only when it changed:
/// every cell reads it); what `change` returned.
pub(super) fn change_mode<R>(
    mode: RwSignal<Arrange>,
    change: impl FnOnce(&mut Arrange) -> R,
) -> Option<R> {
    let mut next = mode.try_get_untracked()?;
    let out = change(&mut next);
    if mode.try_with_untracked(|now| *now != next).unwrap_or(false) {
        let _ = mode.try_set(next);
    }
    Some(out)
}

/// The page's one drag in ARRANGE (`behave::arrange_drag::Drag`), the
/// strip's slot it moves under the finger, and the cell it is over now with
/// that cell's element while it carries a mark (elements, so kept locally).
#[derive(Clone, Copy)]
pub(super) struct Carrier {
    drag: StoredValue<Drag>,
    slot: StoredValue<Option<web_sys::Element>, LocalStorage>,
    over: StoredValue<Option<Spot>>,
    target: StoredValue<Option<web_sys::Element>, LocalStorage>,
}

impl Carrier {
    pub(super) fn new() -> Self {
        Self {
            drag: StoredValue::new(Drag::default()),
            slot: StoredValue::new_local(None),
            over: StoredValue::new(None),
            target: StoredValue::new_local(None),
        }
    }

    /// The carried slot follows the finger by `dx`, `dy` px (its own
    /// `style`: the slot has no reactive one).
    fn show(self, dx: f64, dy: f64) {
        let _ = self.slot.try_with_value(|slot| {
            if let Some(slot) = slot {
                dom::set_attr(
                    slot,
                    "style",
                    &format!("transform:translate({dx:.1}px,{dy:.1}px);"),
                );
                dom::set_attr(slot, "data-carried", "true");
            }
        });
    }

    /// Whether the finger is still over the cell `spot` it was over.
    fn still_over(self, spot: Option<Spot>) -> bool {
        self.over
            .try_with_value(|over| *over == spot)
            .unwrap_or(false)
    }

    /// The finger is over `spot` now; its cell's mark (`data-drop`: `yes`,
    /// or `no` for one that takes no drop; none over the strip's own slot)
    /// replaces the previous one.
    fn aim(self, spot: Option<Spot>, cell: Option<(web_sys::Element, bool)>) {
        self.unaim();
        let _ = self.over.try_set_value(spot);
        if let Some((cell, takes)) = cell {
            dom::set_attr(&cell, "data-drop", if takes { "yes" } else { "no" });
            let _ = self.target.try_set_value(Some(cell));
        }
    }

    /// No cell is aimed at any more.
    fn unaim(self) {
        let _ = self.over.try_set_value(None);
        if let Some(Some(cell)) = self.target.try_update_value(Option::take) {
            dom::remove_attr(&cell, "data-drop");
        }
    }

    /// The carried slot goes back to its place.
    fn lift(self) {
        self.unaim();
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
    pub(super) fn leave(self) {
        let _ = self.drag.try_update_value(Drag::leave);
        self.lift();
    }
}

/// Every visible sector's cells on the page `slot` lies in (#233: the
/// lines' children, each line one keyed list), each its sector
/// (`data-sector`), its layout slot (`data-slot`), its centre's x and its
/// box's top and bottom (px, the viewport's). A cell with no box (out of
/// a side's window) is none; with `left_covered` (an extra tab over the
/// left half) the left sectors' cells are none.
fn page_spots(slot: &web_sys::Element, left_covered: bool) -> Vec<Spot> {
    let Some(body) = slot.closest(".body").ok().flatten() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut line = body.first_element_child();
    while let Some(row) = line {
        if row.matches(".line").unwrap_or(false) {
            let mut next = row.first_element_child();
            while let Some(el) = next {
                if let Some(spot) = spot_of(&el)
                    && !(left_covered && spot.sector.left())
                {
                    out.push(spot);
                }
                next = el.next_element_sibling();
            }
        }
        line = row.next_element_sibling();
    }
    out
}

/// A cell's spot (see [`page_spots`]); none for an element that is no
/// sector's cell or has no box.
fn spot_of(el: &web_sys::Element) -> Option<Spot> {
    let slot = el.get_attribute("data-slot")?.parse::<usize>().ok()?;
    let id = el.get_attribute("data-sector")?;
    let sector = SectorId::ALL.into_iter().find(|s| s.as_str() == id)?;
    let rect = el.get_bounding_client_rect();
    (rect.width() > 0.0).then(|| Spot {
        sector,
        slot,
        x: rect.left() + rect.width() / 2.0,
        top: rect.top(),
        bottom: rect.bottom(),
    })
}

/// The cell element of `spot` on the page `slot` lies in.
fn spot_cell(slot: &web_sys::Element, spot: Spot) -> Option<web_sys::Element> {
    let body = slot.closest(".body").ok().flatten()?;
    let selector = format!(
        ".line > [data-sector=\"{}\"][data-slot=\"{}\"]",
        spot.sector.as_str(),
        spot.slot
    );
    body.query_selector(&selector).ok().flatten()
}

/// The layouts the sectors `from` and `into` show now (the edit's in
/// ARRANGE, `picked` over this device's picks).
fn shown_layouts(nav: Nav, from: SectorId, into: SectorId) -> Option<(String, String)> {
    let picks = nav.picks.try_get_untracked().unwrap_or_default();
    let mode = nav.arrange;
    nav.store
        .sectors
        .try_with_untracked(|told| {
            let doc = told.shown()?;
            mode.try_with_untracked(|m| {
                let shown = m.applied(doc);
                let from = picked(&shown, &picks, from)?.id.clone();
                let into = picked(&shown, &picks, into)?.id.clone();
                Some((from, into))
            })
            .flatten()
        })
        .flatten()
}

/// The carried strip (its sector, slot and strip) and the cell the finger
/// at (`x`, `y`) is over (`drop_spot` over the cells read at the down).
fn carried_over(
    carrier: Carrier,
    x: f64,
    y: f64,
) -> Option<(SectorId, usize, StripRef, Option<Spot>)> {
    carrier
        .drag
        .try_with_value(|d| {
            d.held()
                .map(|c| (c.sector, c.slot, c.strip.clone(), drop_spot(&c.spots, x, y)))
        })
        .flatten()
}

/// ARRANGE's cover over a strip (`sectors.css` shows it only in the mode):
/// it owns its touches and captures the pointer, so no touch reaches the
/// strip's controls. A finger moved past the slop carries the strip (its
/// slot follows the finger); the cell it is over is marked (`data-drop`,
/// `Arrange::aim`: `no` where it takes no drop, none over its own slot),
/// worked out again only when that cell changes; the lift drops it on the
/// nearest cell of every visible sector (#233: another sector's too), an
/// empty one or a strip (`Arrange::dropped_in` over the cells read at the
/// down: `drop_spot`, `landing`). A cancel or a lost capture puts it back
/// with nothing moved; a carry whose end never came ends at the next
/// primary down of its type on a cover (#124). A drop is judged with the
/// served layout's pool (#236: a layout it would grow past FULL takes no
/// drop, as the hub's save would refuse it).
#[component]
pub(super) fn ArrangeCover(
    sector: SectorId,
    strip: StripRef,
    place: Memo<usize>,
    carrier: Carrier,
) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let layout: StoredValue<Arc<Layout>> = StoredValue::new(expect_context::<Head>().layout);
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
            spots: page_spots(&slot_el, nav.left_covered_now()),
        };
        let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
        let down = down_of(&ev);
        let took = carrier
            .drag
            .try_update_value(|d| d.down(down, held, x, y))
            .unwrap_or(false);
        if took {
            carrier.hold(slot_el);
            let _ = el.set_pointer_capture(down.id);
        }
    };
    let on_move = move |ev: web_sys::PointerEvent| {
        let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
        let offset = carrier
            .drag
            .try_update_value(|d| d.moved(ev.pointer_id(), x, y))
            .flatten();
        let Some((dx, dy)) = offset else {
            return;
        };
        carrier.show(dx, dy);
        let Some((own, at, held, spot)) = carried_over(carrier, x, y) else {
            return;
        };
        if carrier.still_over(spot) {
            return;
        }
        let cell = spot.and_then(|spot| {
            let (from, into) = shown_layouts(nav, own, spot.sector)?;
            let takes = layout
                .try_with_value(|layout| {
                    let pool = pool(layout);
                    mode.try_with_untracked(|m| m.aim(&from, at, &into, spot.slot, &held, &pool))
                })
                .flatten()
                .flatten()?;
            let slot_el = carrier.slot.try_with_value(Clone::clone).flatten()?;
            Some((spot_cell(&slot_el, spot)?, takes))
        });
        carrier.aim(spot, cell);
    };
    let on_up = move |ev: web_sys::PointerEvent| {
        let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
        let drop = carrier
            .drag
            .try_update_value(|d| d.up(ev.pointer_id(), x, y))
            .flatten();
        carrier.settle();
        let Some(carry) = drop else {
            return;
        };
        let Some(spot) = drop_spot(&carry.spots, x, y) else {
            return;
        };
        let Some((from, into)) = shown_layouts(nav, carry.sector, spot.sector) else {
            return;
        };
        let _ = layout.try_with_value(|layout| {
            let pool = pool(layout);
            change_mode(mode, |m| {
                m.dropped_in(&from, carry.slot, &into, spot.slot, &carry.strip, &pool)
            })
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
