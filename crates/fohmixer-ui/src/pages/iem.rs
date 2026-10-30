//! iemmixer's tab (#166; the owner's approved `docs/mockups/iem-tab-v1.html`):
//! one frame of iemmixer's own page over the left half of the page shown, as
//! AbleSet's (the column stays in the screen's middle and the right half keeps
//! working; on a phone the whole screen but the thin strip). `behave::iem`
//! decides; this file carries it out.
//!
//! Unlike AbleSet's frame it lives on behind the other tabs, so listening goes
//! on while the engineer works the faders: the first open mounts it, another
//! tab hides it, and only the ✕ beside the tab ends it (its page's audio with
//! it). So [`IemPane`] is mounted once by `Surface`, outside the layout's
//! `Shell`: a new layout (a Tuner edit in Live) remounts the shell and would
//! reload a frame in it, as it does the deck's and AbleSet's, and moving an
//! iframe in the document reloads it too. The hub's facts are read untracked
//! at the mount, so their news never reloads a mounted frame.
//!
//! Hidden means `visibility: hidden` (`iem.css`), never `display: none` and
//! never off-screen: the frame keeps its box where it was, so its page keeps
//! its size and is neither laid out as nothing nor out of the viewport, the
//! states engines throttle or suspend a frame's timers and drawing in.
//!
//! What happens inside the frame is iemmixer's: another origin, so the page
//! sees none of it (no touch record, no touch-idle wake, as AbleSet's).

use leptos::prelude::*;

use crate::behave::iem::{FRAME_ALLOW, tab_dot};
use crate::components::owns_touches;
use crate::diag;
use crate::diag::trace::{self, IemStep};
use crate::dom;
use crate::pages::ableset::SANDBOX;
use crate::pages::surface::{Extra, Nav};

/// iemmixer's pane, mounted once beside the layout's shell: the frame from
/// the first open until the ✕, shown while its tab is.
#[component]
pub(crate) fn IemPane() -> impl IntoView {
    let nav = expect_context::<Nav>();
    let store = nav.store;
    // One memo: however often the tab is tapped, the frame mounts once.
    let alive = Memo::new(move |_| nav.iem_alive());
    let mounted = StoredValue::new(None::<f64>);
    let frame = move || {
        if !alive.try_get().unwrap_or(false) {
            return None;
        }
        let (title, src) = store
            .iem
            .try_with_untracked(|i| i.as_ref().map(|i| (i.title.clone(), i.url.clone())))
            .flatten()
            .unwrap_or_default();
        let _ = mounted.try_set_value(Some(dom::now()));
        diag::record(&trace::iem(dom::epoch_now(), IemStep::Open));
        let loaded = move |_: web_sys::Event| {
            if let Some(Some(at)) = mounted.try_update_value(Option::take) {
                let ms = dom::now() - at;
                diag::record(&trace::iem(dom::epoch_now(), IemStep::Load { ms }));
            }
        };
        Some(view! {
            <iframe
                class="iem-frame"
                data-testid="iem-frame"
                title=title
                src=src
                sandbox=SANDBOX
                referrerpolicy="no-referrer"
                allow=FRAME_ALLOW
                on:load=loaded
            ></iframe>
        })
    };
    let shown = move || nav.iem_shown().to_string();
    view! {
        <div class="iem-pane" data-testid="iem" data-shown=shown>
            <div class="iem-area">{frame}</div>
        </div>
    }
}

/// iemmixer's tab, under AbleSet's in the column: its title from the hub, a
/// dot while its frame lives behind another tab, and the ✕ beside it while the
/// frame lives.
#[component]
pub(crate) fn IemTab() -> impl IntoView {
    let nav = expect_context::<Nav>();
    let store = nav.store;
    let title = move || {
        store
            .iem
            .try_with(|i| i.as_ref().map(|i| i.title.clone()))
            .flatten()
            .unwrap_or_default()
    };
    let lit = move || nav.iem_shown();
    let alive = move || nav.iem_alive();
    let dot = move || tab_dot(alive(), lit());
    // A tab owns its touches (#43 PR G); it writes no key.
    let no_keys: Vec<String> = Vec::new();
    view! {
        <div class="iem-row" data-testid="iem-row">
            <button
                type="button"
                class="tab extra-tab iem-tab"
                use:owns_touches=no_keys
                class:selected=lit
                data-testid="iem-tab"
                data-selected=move || lit().to_string()
                data-alive=move || alive().to_string()
                on:pointerdown=move |_| nav.show_extra(Extra::Iem)
            >
                <Show when=dot>
                    <i class="iem-dot" data-testid="iem-dot"></i>
                </Show>
                {title}
            </button>
            <Show when=alive>
                <IemClose />
            </Show>
        </div>
    }
}

/// The ✕ beside the tab: ends the frame, and with it iemmixer's audio.
#[component]
fn IemClose() -> impl IntoView {
    let nav = expect_context::<Nav>();
    // It owns its touches (#43 PR G); it writes no key.
    let no_keys: Vec<String> = Vec::new();
    // It acts at the lift of a press that began on it: a finger that slides
    // here ends no audio.
    let pressed = RwSignal::new(false);
    view! {
        <button
            type="button"
            class="iem-close"
            use:owns_touches=no_keys
            data-testid="iem-close"
            aria-label="Close IEM MIXER"
            on:pointerdown=move |_| pressed.set(true)
            on:pointercancel=move |_| pressed.set(false)
            on:pointerleave=move |_| pressed.set(false)
            on:pointerup=move |_| {
                if pressed.get_untracked() {
                    pressed.set(false);
                    nav.close_iem();
                }
            }
        >
            "✕"
        </button>
    }
}
