//! The Claps strip's left column (#220; the owner's approved mockup
//! `docs/mockups/claps-v2.html`): above SEL, where other strips show LUFS,
//! so the fader keeps its whole travel. Top down: the calm state line, the
//! four beat dots (2 × 2, each its own target), and TAP, a
//! `components::hold_key` [`HoldKey`]: a tap acts on its lift with the
//! down's own page time (the hub snaps it to Live's beat), a 1 s hold stops
//! everything. Every decision is the hub's (its `claps` message,
//! `LiveStore.claps`) or `behave::claps`'s; the frame loop only moves the
//! "now" ring and the clap's flash between the hub's messages.

use fohmixer_proto::claps::{BEATS, ClapsState};
use fohmixer_proto::layout::Strip;
use leptos::html;
use leptos::prelude::*;

use super::hold_key::{Flash, HoldKey, Tap};
use super::owns_touches;
use crate::behave::claps::{dot_tap, hit, phase_name, ring, state_lit, state_text, stop_value};
use crate::dom;
use crate::raf;
use crate::store::LiveStore;

/// The column, on the strip whose Tuner carries `+CLAPS` only, once the hub
/// told the page its claps.
#[component]
pub fn ClapsColumn(strip: Strip) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let told = Memo::new(move |_| store.claps.try_get().flatten().map(|(state, _)| state));
    let shown = move || strip.claps && told.try_with(Option::is_some).unwrap_or(false);
    move || shown().then(|| view! { <ClapsParts told=told /> })
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
    let pattern = move || {
        told.try_with(|t| t.as_ref().map(|s| s.pattern))
            .flatten()
            .unwrap_or([false; BEATS])
    };
    let line = move || state_text(phase());
    let lit = move || state_lit(phase()).to_string();
    let state = Signal::derive(move || phase_name(phase()));
    let root = NodeRef::<html::Div>::new();
    // The ring and the flash, between the hub's messages: written on a
    // change only.
    raf::animate(root, move |el| {
        let dots: Vec<Option<web_sys::Element>> = (0..BEATS)
            .map(|i| {
                el.query_selector(&format!(".claps-dot[data-beat=\"{i}\"]"))
                    .ok()
                    .flatten()
            })
            .collect();
        let mut drawn: (Option<usize>, bool) = (None, false);
        Box::new(move |_now: f64, _step: f64| {
            let now = dom::epoch_now();
            let look = store
                .claps
                .try_with_untracked(|c| {
                    c.as_ref()
                        .map(|(state, at)| (ring(state, *at, now), hit(state, *at, now)))
                })
                .flatten()
                .unwrap_or((None, false));
            if look == drawn {
                return;
            }
            for (i, dot) in dots.iter().enumerate() {
                if let Some(dot) = dot {
                    let on = look.0 == Some(i);
                    dom::set_attr(dot, "data-now", if on { "true" } else { "false" });
                    dom::set_attr(dot, "data-hit", if on && look.1 { "true" } else { "false" });
                }
            }
            drawn = look;
        })
    });
    let dots = (0..BEATS)
        .map(|i| {
            let set = move || pattern()[i].to_string();
            let no_keys: Vec<String> = Vec::new();
            let on_down = move |ev: web_sys::PointerEvent| {
                ev.prevent_default();
                let on = dot_tap(&pattern(), i);
                // `i` is under BEATS (4): it fits a u8.
                let _ = store.clap_beat(i as u8, on);
            };
            view! {
                <button
                    type="button"
                    class="claps-dot"
                    data-testid="claps-dot"
                    data-beat=i
                    data-set=set
                    data-now="false"
                    data-hit="false"
                    aria-label=format!("Beat {}", i + 1)
                    use:owns_touches=no_keys
                    on:pointerdown=on_down
                >
                    {i + 1}
                </button>
            }
        })
        .collect_view();
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
            <div class="claps-dots">{dots}</div>
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
