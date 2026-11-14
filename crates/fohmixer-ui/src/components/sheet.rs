//! A sheet over the channel detail (#208's ADD TO LAYOUT, #205's channel
//! selector; the owner's approved mockups `docs/mockups/scenes-v3.html`
//! and `channel-select-v1.html`): a shade over the whole detail (a tap
//! closes) and a panel with its title, ✕ and a list. Its keys act at their
//! lift (`LiftKey`, `behave::sheet::key_press`), and a finger dragged
//! over the list scrolls it (`behave::sheet::DragScroll`): the detail takes
//! no native touch scroll. `sheet.css`.

use leptos::prelude::*;

use super::owns_touches;
use crate::behave::hold::{Hold, Lift};
use crate::behave::sheet::{DragScroll, key_acts, key_press};
use crate::dom;

/// A key that acts at its lift (`behave::sheet::key_press`): the down
/// captures the pointer; a slide past 10 px, a cancel or a lost capture
/// ends the press with nothing done; the lift acts.
#[derive(Clone, Copy)]
pub struct LiftKey {
    press: StoredValue<Hold>,
}

impl Default for LiftKey {
    fn default() -> Self {
        Self {
            press: StoredValue::new(key_press()),
        }
    }
}

impl LiftKey {
    /// The key's `pointerdown`.
    pub fn down(self, ev: &web_sys::PointerEvent) {
        ev.prevent_default();
        let id = ev.pointer_id();
        let pressed = self
            .press
            .try_update_value(|h| h.down(id, ev.is_primary(), dom::now()))
            .unwrap_or(false);
        if pressed && let Some(el) = dom::current_element(ev) {
            let _ = el.set_pointer_capture(id);
        }
    }

    /// The key's `pointermove`.
    pub fn moved(self, ev: &web_sys::PointerEvent) {
        let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
        let _ = self
            .press
            .try_update_value(|h| h.moved(ev.pointer_id(), x, y));
    }

    /// The key's `pointerup`: whether the lift acts.
    pub fn up(self, ev: &web_sys::PointerEvent) -> bool {
        let lift = self
            .press
            .try_update_value(|h| h.up(ev.pointer_id(), dom::now()))
            .unwrap_or(Lift::Nothing);
        key_acts(lift)
    }

    /// The key's `pointercancel` and `lostpointercapture`.
    pub fn cancel(self, ev: &web_sys::PointerEvent) {
        let _ = self.press.try_update_value(|h| h.cancel(ev.pointer_id()));
    }
}

/// Closes a sheet from a key's lift, a task later: the lift's own
/// `lostpointercapture` (the capture's implicit release) is dispatched to
/// the key right after its `pointerup`, after the microtasks in which a
/// direct close removes the key and drops its handlers ("closure invoked
/// recursively or after being dropped", `.claude/rules/leptos-view-macro.md`).
pub fn close_later(open: RwSignal<bool>) {
    after_lift(move || {
        let _ = open.try_set(false);
    });
}

/// Runs `then` a task after a key's lift, for the same reason as
/// [`close_later`]: a pick that opens another channel's detail replaces
/// the sheet, the key included.
pub fn after_lift(then: impl FnOnce() + 'static) {
    set_timeout(then, std::time::Duration::ZERO);
}

/// The sheet (`data-testid` `testid`) while `open`: the shade and ✕ close
/// it (`sheet-shade`, `sheet-close`), `note` under the title, `children`
/// the list, which a dragged finger scrolls.
#[component]
pub fn Sheet(
    title: &'static str,
    testid: &'static str,
    open: RwSignal<bool>,
    #[prop(optional)] note: Option<&'static str>,
    #[prop(optional)] wide: bool,
    children: Children,
) -> impl IntoView {
    let close = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = open.try_set(false);
    };
    let drag = StoredValue::new(DragScroll::default());
    let on_move = move |ev: web_sys::PointerEvent| {
        let pressed = ev.buttons() != 0;
        let step = drag
            .try_update_value(|d| d.moved(ev.pointer_id(), f64::from(ev.client_y()), pressed))
            .unwrap_or(0.0);
        if step != 0.0
            && let Some(list) = dom::current_element(&ev)
        {
            list.set_scroll_top(list.scroll_top() + step.round() as i32);
        }
    };
    let on_end = move |ev: web_sys::PointerEvent| {
        let _ = drag.try_update_value(|d| d.end(ev.pointer_id()));
    };
    let note = note.map(|text| view! { <div class="lay-note sheet-note">{text}</div> });
    let shade_keys: Vec<String> = Vec::new();
    let close_keys: Vec<String> = Vec::new();
    view! {
        <div class="sheet" data-testid=testid>
            <div
                class="sheet-shade"
                use:owns_touches=shade_keys
                data-testid="sheet-shade"
                on:pointerdown=close
            ></div>
            <div class="sheet-panel" class:wide=wide role="dialog" aria-label=title>
                <div class="sheet-head">
                    <h3 class="sheet-title">{title}</h3>
                    <button
                        type="button"
                        class="sheet-close"
                        use:owns_touches=close_keys
                        data-testid="sheet-close"
                        aria-label="Close"
                        on:pointerdown=close
                    >
                        "✕"
                    </button>
                </div>
                {note}
                <div
                    class="sheet-body"
                    data-testid="sheet-body"
                    on:pointermove=on_move
                    on:pointerup=on_end
                    on:pointercancel=on_end
                    on:lostpointercapture=on_end
                >
                    {children()}
                </div>
            </div>
        </div>
    }
}
