//! The Claps strip's left column (#220; the owner's approved mockup
//! `docs/mockups/claps-v2.html`, its beat dots removed by #260's v3): above
//! SEL, where other strips show LUFS, so the fader keeps its whole travel.
//! Top down: the calm state line and TAP, a `components::hold_key`
//! [`HoldKey`]: a tap acts on its lift with the down's own page time (the
//! hub snaps it to Live's beat), a 1 s hold stops everything, and TAP
//! flashes at each clap Live plays from the tapped pattern. Every decision
//! is the hub's (its `claps` message, `LiveStore.claps`) or
//! `behave::claps`'s; the frame loop only draws the clap's flash between
//! the hub's messages.

use fohmixer_proto::claps::ClapsState;
use fohmixer_proto::layout::Strip;
use leptos::html;
use leptos::prelude::*;

use super::hold_key::{Flash, HoldKey, Tap};
use crate::behave::claps::{hit, phase_name, state_lit, state_text, stop_value};
use crate::dom;
use crate::raf;
use crate::store::LiveStore;

/// The column, on the strip whose Tuner carries `+CLAPS` only, once the hub
/// told the page its claps.
#[component]
pub fn ClapsColumn(strip: Strip) -> impl IntoView {
    // Every other strip builds nothing (#220 review).
    if !strip.claps {
        return None;
    }
    let store = expect_context::<LiveStore>();
    let told = Memo::new(move |_| store.claps.try_get().flatten().map(|(state, _)| state));
    // Mounted once the hub told one, and only then: a memo of the bool, so
    // a new state never builds the column again (a press on TAP goes on).
    let shown = Memo::new(move |_| told.try_with(Option::is_some).unwrap_or(false));
    Some(move || {
        shown
            .try_get()
            .unwrap_or(false)
            .then(|| view! { <ClapsParts told=told /> })
    })
}

/// The column's parts over the hub's state `told`.
#[component]
fn ClapsParts(told: Memo<Option<ClapsState>>) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let phase = move || {
        told.try_with(|t| t.as_ref().map(|s| s.state))
            .flatten()
            .unwrap_or_default()
    };
    let line = move || state_text(phase());
    let lit = move || state_lit(phase()).to_string();
    let state = Signal::derive(move || phase_name(phase()));
    let root = NodeRef::<html::Div>::new();
    // The clap's flash on TAP, between the hub's messages: written on a
    // change only.
    raf::animate(root, move |el| {
        let tap = el.query_selector(".claps-tap").ok().flatten();
        // Nothing drawn yet: the first frame writes it (a column built again
        // over the same element never keeps an old flash).
        let mut drawn: Option<bool> = None;
        Box::new(move |_now: f64, _step: f64| {
            let now = dom::epoch_now();
            let lit = store
                .claps
                .try_with_untracked(|c| c.as_ref().map(|(state, at)| hit(state, *at, now)))
                .flatten()
                .unwrap_or(false);
            if drawn == Some(lit) {
                return;
            }
            if let Some(tap) = &tap {
                dom::set_attr(tap, "data-hit", if lit { "true" } else { "false" });
            }
            drawn = Some(lit);
        })
    });
    let can = Signal::derive(move || {
        stop_value(
            told.try_with(Option::is_some).unwrap_or(false),
            store.connected.try_get().unwrap_or(false),
        )
        .is_some()
    });
    let wanted = move || {
        stop_value(
            told.try_with_untracked(Option::is_some).unwrap_or(false),
            store.connected.try_get_untracked().unwrap_or(false),
        )
    };
    let act = move |(): (), flash: Flash| {
        if !store.clap_stop() {
            flash("the hub is not connected".to_string());
        }
    };
    let tap: Tap = std::sync::Arc::new(move |t: f64| {
        let _ = store.clap_tap(t);
    });
    view! {
        <div class="claps-col" data-testid="claps-col" data-state=move || state.try_get() node_ref=root>
            <div class="claps-state" data-testid="claps-state" data-lit=lit>{line}</div>
            <HoldKey
                keys=Vec::new()
                wanted=wanted
                can=can
                act=act
                button="claps"
                class="claps-tap"
                testid="claps-tap"
                label="Tap along; hold 1 s to stop"
                state=state
                tap=tap
            >
                <span class="claps-tap-word">"TAP"</span>
            </HoldKey>
        </div>
    }
}
