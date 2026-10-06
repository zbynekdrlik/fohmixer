//! The Stream Deck tab (#52, spec §2, §6): Companion's keys as one grid of
//! square keys as large as the area allows, the layout's global controls on
//! the rail (as on the Conf page). A key is down at the touch and up at the
//! release (`behave::deck::Presses` decides; this file carries it out): a
//! down that cannot go now flashes red and is never sent later; a finger on
//! a key shows a local outline at once (Companion's pressed look needs a
//! round trip). A down goes only while the link is not dropping out
//! (`behave::deck::can_press`), and every press carries its pointer event's
//! own time, so the hub refuses one that waited (`late`). The page tells the
//! hub it views the tab (only a viewer gets the keys' images) and lifts
//! every held key when the tab is left (`tab`) or the page goes hidden
//! (`hidden`); a closed socket forgets the holds (the hub releases them); a
//! primary pointer's down first lifts the holds whose end the browser never
//! delivered. Every key owns its touches (`use:owns_touches`): no loupe,
//! callout or drag on a hold.

use std::collections::BTreeSet;

use fohmixer_proto::layout::Control;
use leptos::html;
use leptos::prelude::*;

use crate::behave::deck::{Action, Presses, Why, can_press};
use crate::components::{ControlView, fail_flash, owns_touches};
use crate::diag::{self, trace};
use crate::dom;
use crate::store::LiveStore;
use crate::store::deck::key_side;

/// The gap between two keys (px): `.deck-grid`'s `gap` in deck.css.
pub const KEY_GAP: f64 = 8.0;
/// The grid area's padding on each side (px): `.deck-area` in deck.css.
pub const AREA_PAD: f64 = 10.0;

/// The grid's CSS variables: its columns and rows (`shape`; one key until
/// the hub described the deck) and the side of a square key in `area`
/// (`.deck-area`'s client size, its padding included).
fn grid_vars(shape: Option<(u32, u32)>, area: (f64, f64)) -> String {
    let (columns, rows) = shape.unwrap_or((1, 1));
    let (width, height) = area;
    let side = key_side(
        width - 2.0 * AREA_PAD,
        height - 2.0 * AREA_PAD,
        columns,
        rows,
        KEY_GAP,
    );
    format!("--cols:{columns};--rows:{rows};--key:{side:.0}px;")
}

/// Carries a press decision made at page time `t` out: onto the socket and
/// into the flight recorder; a down that could not go flashes `failed`
/// (when given) and its up is not sent either (`Presses::unsent`, at once).
fn carry(
    store: LiveStore,
    presses: StoredValue<Presses>,
    action: Action,
    failed: Option<RwSignal<bool>>,
    t: f64,
) {
    match action {
        Action::Nothing => {}
        Action::Flash { key } => {
            if let Some(failed) = failed {
                fail_flash(failed)("not connected".to_string());
            }
            diag::record(&trace::deck(t, key, true, None, None, false, None));
        }
        Action::Send {
            key,
            down,
            hold_ms,
            why,
        } => {
            let why = why.map(Why::name);
            let on_fail: Box<dyn FnOnce(String)> = match failed {
                Some(failed) => fail_flash(failed),
                None => Box::new(|_: String| {}),
            };
            match store.deck_press(key, down, t, hold_ms, why, on_fail) {
                Ok(seq) => diag::record(&trace::deck(t, key, down, hold_ms, why, true, Some(seq))),
                Err(flash) => {
                    if down {
                        let _ = presses.try_update_value(|p| p.unsent(key));
                        flash("not connected".to_string());
                    }
                    diag::record(&trace::deck(t, key, down, hold_ms, why, false, None));
                }
            }
        }
    }
}

/// The held keys' outline follows the state machine, written only on a
/// change: a `try_set` notifies even with the same value, and every key's
/// outline reads `held`.
fn show_held(presses: StoredValue<Presses>, held: RwSignal<BTreeSet<u32>>) {
    let Some(keys) = presses.try_with_value(Presses::held_keys) else {
        return;
    };
    if held.try_with_untracked(|shown| *shown != keys) == Some(true) {
        let _ = held.try_set(keys);
    }
}

/// Every held key up (`why`: the tab left, the page hidden): each up goes
/// out through `LiveStore::deck_press` like a lift's. The outline is left
/// alone: the tab's cleanup calls this while the keys' view still exists,
/// and a write of `held` there would wake their outline effects after the
/// page's owner disposed the signal (a panic, checkpoint C of #52).
fn lift_keys(store: LiveStore, presses: StoredValue<Presses>, why: Why) {
    let t = dom::epoch_now();
    let actions = presses
        .try_update_value(|p| p.leave_all(t, why))
        .unwrap_or_default();
    for action in actions {
        carry(store, presses, action, None, t);
    }
}

/// Every held key up while the page stays (`why`: the page hidden), and
/// the outline then.
fn leave_keys(
    store: LiveStore,
    presses: StoredValue<Presses>,
    held: RwSignal<BTreeSet<u32>>,
    why: Why,
) {
    lift_keys(store, presses, why);
    show_held(presses, held);
}

/// The Stream Deck tab's page: the rail with the layout's global controls,
/// then the key grid.
#[component]
pub fn DeckView(global: Vec<Control>, viewport: RwSignal<(f64, f64)>) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let presses = StoredValue::new(Presses::default());
    let held = RwSignal::new(BTreeSet::<u32>::new());
    let area_ref = NodeRef::<html::Div>::new();
    let area = RwSignal::new((0.0_f64, 0.0_f64));
    store.deck_view(true);
    diag::record(&trace::deck_view(dom::epoch_now(), true));
    // `visibilitychange` bubbles from the document to the window.
    let hidden = window_event_listener_untyped("visibilitychange", move |_| {
        if dom::hidden() {
            leave_keys(store, presses, held, Why::Hidden);
        }
    });
    // The page's own signals stay untouched here (`lift_keys`, no outline):
    // its keys' render effects outlive this cleanup by a microtask.
    on_cleanup(move || {
        hidden.remove();
        lift_keys(store, presses, Why::Tab);
        store.deck_view(false);
        diag::record(&trace::deck_view(dom::epoch_now(), false));
    });
    // A closed socket forgets every hold: the hub releases them (Detach).
    Effect::new(move |_| {
        if !store.connected.get() {
            let _ = presses.try_update_value(Presses::clear);
            show_held(presses, held);
        }
    });
    // The grid's room: measured once laid out and on every resize.
    Effect::new(move |_| {
        let _ = viewport.get();
        if let Some(el) = area_ref.get() {
            let _ = area.try_set((f64::from(el.client_width()), f64::from(el.client_height())));
        }
    });
    let shape = Memo::new(move |_| store.deck.with(|d| d.as_ref().map(|d| (d.columns, d.rows))));
    let grid_style = move || grid_vars(shape.get(), area.get());
    let keys = move || {
        let (columns, rows) = shape.get().unwrap_or((0, 0));
        (0..columns * rows)
            .map(|index| view! { <DeckKeyView index=index presses=presses held=held /> })
            .collect_view()
    };
    let global = global
        .into_iter()
        .map(|control| view! { <ControlView control=control /> })
        .collect_view();
    view! {
        <div class="body deck-page" data-testid="deck">
            <nav class="rail" data-testid="rail">
                <div class="rail-main"></div>
                <div class="rail-foot">{global}</div>
            </nav>
            <div class="deck-area" node_ref=area_ref>
                <div class="deck-grid" data-testid="deck-grid" style=grid_style>
                    {keys}
                </div>
            </div>
        </div>
    }
}

/// One key: Companion's image (or colour), Companion's pressed look, the
/// local outline while a finger holds it, the red flash, dimmed offline.
/// (`index` is the key's number: a prop named `key` could read as Leptos'
/// own keyed-list attribute.)
#[component]
fn DeckKeyView(
    index: u32,
    presses: StoredValue<Presses>,
    held: RwSignal<BTreeSet<u32>>,
) -> impl IntoView {
    let key = index;
    let store = expect_context::<LiveStore>();
    let failed = RwSignal::new(false);
    let look = Memo::new(move |_| store.deck_keys.with(|keys| keys.get(&key).cloned()));
    let online = Memo::new(move |_| store.deck.with(|d| d.as_ref().is_some_and(|d| d.online)));
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        if let Some(el) = dom::current_element(&ev) {
            let _ = el.set_pointer_capture(ev.pointer_id());
        }
        // The pointer event's own time: a down a frozen page delayed is
        // late at the hub.
        let t = dom::event_epoch(&ev);
        // A primary pointer's down: the holds whose end never came go up
        // first (`lost`), so a missed end never leaves a key stuck.
        let missed = presses
            .try_update_value(|p| p.missed_ups(ev.is_primary(), t))
            .unwrap_or_default();
        for action in missed {
            carry(store, presses, action, None, t);
        }
        // A press can go now: the socket ready, Companion online, the link
        // not dropping out (a press would wait in a stalled socket).
        let connected = can_press(
            store.can_send(),
            online.try_get_untracked().unwrap_or(false),
            store.dropping_out(),
        );
        let action = presses.try_update_value(|p| p.down(key, ev.pointer_id(), t, connected));
        if let Some(action) = action {
            carry(store, presses, action, Some(failed), t);
        }
        show_held(presses, held);
    };
    let on_end = move |ev: web_sys::PointerEvent| {
        let Some(why) = Why::of_event(&ev.type_()) else {
            return;
        };
        let t = dom::event_epoch(&ev);
        let action = presses.try_update_value(|p| p.up(key, ev.pointer_id(), t, why));
        if let Some(action) = action {
            carry(store, presses, action, Some(failed), t);
        }
        show_held(presses, held);
    };
    // Read in place (`with`): only the image needs its data URL as a string.
    let pressed = move || look.with(|k| k.as_ref().is_some_and(|k| k.pressed));
    let is_held = move || held.with(|h| h.contains(&key));
    let offline = move || !online.get();
    let is_failed = move || failed.get();
    let colour = move || {
        look.with(|k| {
            k.as_ref()
                .and_then(|k| k.color.as_deref())
                .map(|c| format!("background-color:{c};"))
                .unwrap_or_default()
        })
    };
    let image = move || {
        look.with(|k| k.as_ref().and_then(|k| k.img.clone()))
            .map(|src| view! { <img class="deck-img" src=src alt="" draggable="false" /> })
    };
    // A key owns its touches (#43 PR G); it writes no Live key.
    let no_keys: Vec<String> = Vec::new();
    let key_text = key.to_string();
    view! {
        <button
            type="button"
            class="deck-key"
            use:owns_touches=no_keys
            class:pressed=pressed
            class:held=is_held
            class:failed=is_failed
            class:offline=offline
            data-testid="deck-key"
            data-key=key_text
            data-pressed=move || pressed().to_string()
            data-held=move || is_held().to_string()
            data-failed=move || is_failed().to_string()
            style=colour
            on:pointerdown=on_down
            on:pointerup=on_end
            on:pointercancel=on_end
            on:lostpointercapture=on_end
        >
            {image}
        </button>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_fills_the_area_less_its_padding() {
        // The width decides: 1104 − 2 × 10 = 1084 px for 8 keys and 7 gaps
        // (the width's `−` as `+`: 133; as `/`: 0; its `×` as `+`: 129; as
        // `/`: 130).
        assert_eq!(
            grid_vars(Some((8, 4)), (1104.0, 778.0)),
            "--cols:8;--rows:4;--key:128px;"
        );
        // The height decides: 220 − 2 × 10 = 200 px for 2 keys and 1 gap
        // (the height's `−` as `+`: 116; as `/`: 1; its `×` as `+`: 100; as
        // `/`: 105).
        assert_eq!(
            grid_vars(Some((4, 2)), (1020.0, 220.0)),
            "--cols:4;--rows:2;--key:96px;"
        );
        // No deck described yet: one key in the area.
        assert_eq!(
            grid_vars(None, (120.0, 140.0)),
            "--cols:1;--rows:1;--key:100px;"
        );
        // Not laid out yet: no key, never a negative size.
        assert_eq!(
            grid_vars(Some((8, 4)), (0.0, 0.0)),
            "--cols:8;--rows:4;--key:0px;"
        );
    }
}
