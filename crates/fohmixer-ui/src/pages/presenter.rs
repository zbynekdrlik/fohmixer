//! Presenter's stage tab (#90; the owner's approved
//! `docs/mockups/presenter-stage-v1.html`): over the left half of the page
//! shown, as the other extra tabs (the column stays in the screen's middle
//! and the right half keeps working; on a phone the whole screen but the
//! thin strip). On top a live preview of the stage display: Presenter's own
//! stage page (`/stage?preview=1`, on the hub's own origin: `presenter.rs`
//! in the hub), laid out at 1920 × 1080 and scaled to the pane's width, as
//! Presenter's own operator preview does; the header names the layout on
//! stage. Below, Presenter's layouts as keys in three columns, the one on
//! stage lit; a key's 1 s hold switches the stage at its lift
//! (`components::hold_key`, the rail's press: a stage display is seen by
//! people, so a tap must not switch it). While Presenter can't be reached
//! the preview says so and the keys dim and take no touch; it comes back by
//! itself. `behave::presenter` decides; this file carries it out.
//!
//! The frame is mounted only while the tab shows it and Presenter is up
//! (`behave::presenter::preview`), so its stage page, its socket and its
//! drawing cost the iPad nothing on the mixer, and Presenter coming back
//! mounts a fresh one. The frame takes no touch (`pointer-events: none`,
//! as Presenter's own preview): it only shows. It is the page's own origin
//! (Presenter's stage page through the hub), so it has no `sandbox`: one
//! that allows the scripts and the origin the page needs could be lifted by
//! the frame itself (the browsers warn of exactly that), so it would guard
//! nothing. Presenter is the site's own app; the hub passes only its reads.

use leptos::html;
use leptos::prelude::*;

use crate::behave::presenter::{
    CAPTION, DOWN_TEXT, DOWN_TITLE, FRAME_URL, HEAD, KeyLook, Preview, RELEASE_TEXT, STAGE_H,
    STAGE_W, can_switch, key_look, key_note, key_of, name_of, on_stage, preview, rows, scale,
    switch_to,
};
use crate::components::hold_key::{Flash, HoldKey};
use crate::diag;
use crate::diag::trace::{self, PresenterStep};
use crate::dom;
use crate::store::LiveStore;

/// Whether the page may switch the stage now (`behave::presenter::
/// can_switch`), read tracked.
fn ready(store: LiveStore) -> bool {
    let online = store
        .presenter
        .try_with(|p| p.as_ref().and_then(|p| p.online))
        .flatten();
    let connected = store.connected.try_get().unwrap_or(false);
    let dropping = store.dropouts.try_with(|c| c.active).unwrap_or(false);
    can_switch(online, connected, dropping)
}

/// The pane over the page's left half.
#[component]
pub(crate) fn PresenterView(viewport: RwSignal<(f64, f64)>) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    diag::record(&trace::presenter(dom::epoch_now(), PresenterStep::Open));
    on_cleanup(move || {
        diag::record(&trace::presenter(dom::epoch_now(), PresenterStep::Close));
    });
    let online = Memo::new(move |_| {
        store
            .presenter
            .try_with(|p| p.as_ref().and_then(|p| p.online))
            .flatten()
    });
    let shows = Memo::new(move |_| preview(online.try_get().flatten()));
    let now = move || {
        store
            .presenter
            .try_with(|p| p.as_ref().map(on_stage))
            .flatten()
            .unwrap_or_default()
    };
    // The preview's width: measured once laid out and on every resize.
    let area_ref = NodeRef::<html::Div>::new();
    let width = RwSignal::new(0.0_f64);
    Effect::new(move |_| {
        let _ = viewport.get();
        if let Some(el) = area_ref.get() {
            let _ = width.try_set(f64::from(el.client_width()));
        }
    });
    let stage = move || match shows.try_get().unwrap_or(Preview::Waiting) {
        Preview::Frame => Some(view! { <StageFrame width=width /> }.into_any()),
        Preview::Down => {
            diag::record(&trace::presenter(dom::epoch_now(), PresenterStep::Down));
            Some(
                view! {
                    <div class="pst-down" data-testid="presenter-down">
                        <div class="pst-down-title">{DOWN_TITLE}</div>
                        <div class="pst-down-text">{DOWN_TEXT}</div>
                    </div>
                }
                .into_any(),
            )
        }
        Preview::Waiting => None,
    };
    let codes = Memo::new(move |_| {
        store
            .presenter
            .try_with(|p| {
                p.as_ref()
                    .map(|p| p.layouts.iter().map(|l| l.code.clone()).collect::<Vec<_>>())
            })
            .flatten()
            .unwrap_or_default()
    });
    let grid = move || {
        format!(
            "--pst-rows: {}",
            rows(codes.try_with(Vec::len).unwrap_or(0))
        )
    };
    let shown = move || shows.try_get().unwrap_or(Preview::Waiting).name();
    view! {
        <div class="extra-pane presenter-page" data-testid="presenter">
            <div class="pst-head">
                <b>{HEAD}</b>
                <span class="pst-gap"></span>
                <span class="pst-now" data-testid="presenter-now">{now}</span>
            </div>
            <div
                class="pst-stage"
                node_ref=area_ref
                data-testid="presenter-stage"
                data-preview=shown
            >
                {stage}
            </div>
            <div class="pst-caption">{CAPTION}</div>
            <div class="pst-keys" data-testid="presenter-keys" style=grid>
                <For
                    each=move || codes.try_get().unwrap_or_default()
                    key=|code: &String| code.clone()
                    children=|code| view! { <LayoutKey code=code /> }
                />
            </div>
        </div>
    }
}

/// The preview's frame: Presenter's stage page at 1920 × 1080, scaled to the
/// pane's width; mounted while Presenter is up.
#[component]
fn StageFrame(width: RwSignal<f64>) -> impl IntoView {
    diag::record(&trace::presenter(dom::epoch_now(), PresenterStep::Frame));
    let mounted = StoredValue::new(Some(dom::now()));
    let loaded = move |_: web_sys::Event| {
        if let Some(Some(at)) = mounted.try_update_value(Option::take) {
            let ms = dom::now() - at;
            diag::record(&trace::presenter(
                dom::epoch_now(),
                PresenterStep::Load { ms },
            ));
        }
    };
    let style = move || {
        let factor = scale(width.try_get().unwrap_or(0.0)).unwrap_or(0.0);
        format!("width: {STAGE_W}px; height: {STAGE_H}px; transform: scale({factor})")
    };
    view! {
        <iframe
            class="pst-frame"
            data-testid="presenter-frame"
            title="Stage display"
            src=FRAME_URL
            referrerpolicy="no-referrer"
            allow=""
            tabindex="-1"
            aria-hidden="true"
            style=style
            on:load=loaded
        ></iframe>
    }
}

/// One layout's key: its name and second line, lit when it is on stage,
/// dim when no switch can go; a full press's lift switches the stage.
#[component]
fn LayoutKey(code: String) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let code = StoredValue::new(code);
    let layout = Memo::new(move |_| {
        let code = code.get_value();
        store
            .presenter
            .try_with(|p| {
                p.as_ref().map(|p| {
                    let name = name_of(&p.layouts, &code);
                    let description = p
                        .layouts
                        .iter()
                        .find(|l| l.code == code)
                        .map(|l| l.description.clone())
                        .unwrap_or_default();
                    (name, description, p.current.clone())
                })
            })
            .flatten()
            .unwrap_or_default()
    });
    let can = Memo::new(move |_| ready(store));
    let look = Memo::new(move |_| {
        let current = layout.try_with(|l| l.2.clone()).flatten();
        let code = code.get_value();
        key_look(&code, current.as_deref(), can.try_get().unwrap_or(false))
    });
    let state = Signal::derive(move || look.try_get().unwrap_or(KeyLook::Dim).name());
    // What a full press's lift switches to now (none: refused), read
    // untracked at the full mark and the lift.
    let wanted = move || {
        let online = store
            .presenter
            .try_with_untracked(|p| p.as_ref().and_then(|p| p.online))
            .flatten();
        let connected = store.connected.try_get_untracked().unwrap_or(false);
        let dropping = store
            .dropouts
            .try_with_untracked(|c| c.active)
            .unwrap_or(false);
        switch_to(&code.get_value(), can_switch(online, connected, dropping))
    };
    // The lift's switch: onto the socket at once, or the key flashes; a
    // failed answer flashes it too and is recorded.
    let act = move |code: String, flash: Flash| {
        let sent = code.clone();
        let failed = move |why: String| {
            let step = PresenterStep::Failed {
                code: &code,
                why: &why,
            };
            diag::record(&trace::presenter(dom::epoch_now(), step));
            flash(why);
        };
        if let Err(flash) = store.presenter_switch(&sent, Box::new(failed)) {
            flash("the page could not send the switch".to_string());
        }
    };
    let name = move || layout.try_with(|l| l.0.clone()).unwrap_or_default();
    let note = move || {
        let look = look.try_get().unwrap_or(KeyLook::Dim);
        let description = layout.try_with(|l| l.1.clone()).unwrap_or_default();
        key_note(look, &description).to_string()
    };
    let label = format!("{} (hold)", code.get_value());
    view! {
        <HoldKey
            keys=vec![key_of(&code.get_value())]
            wanted=wanted
            can=can
            act=act
            button="presenter"
            class="pst-key"
            testid="presenter-key"
            label=label
            state=state
            name=code.get_value()
        >
            <i class="pst-fill" aria-hidden="true"></i>
            <b class="pst-name">{name}</b>
            <small class="pst-note">{note}</small>
            <small class="pst-release">{RELEASE_TEXT}</small>
        </HoldKey>
    }
}
