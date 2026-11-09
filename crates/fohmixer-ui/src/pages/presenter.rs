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
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::behave::presenter::{
    CAPTION, DOWN_TEXT, DOWN_TITLE, FRAME_URL, HEAD, KeyLook, MINI_HEAD, MiniFit, Place, Preview,
    RAIL_GAP, RELEASE_TEXT, RailRoom, STAGE_H, STAGE_W, can_switch, key_look, key_note, key_of,
    mini_fit, mini_fit_noted, mini_note, name_of, on_stage, phone, preview, rows, scale, switch_to,
    track_count,
};
use crate::components::hold_key::{Flash, HoldKey};
use crate::components::owns_touches;
use crate::diag;
use crate::diag::trace::{self, PresenterStep};
use crate::dom;
use crate::pages::surface::{Extra, Nav};
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
        Preview::Frame => Some(view! { <StageFrame width=width place=Place::Pane /> }.into_any()),
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

/// A stage frame (#90): Presenter's stage page at 1920 × 1080, scaled to
/// `width`; mounted while Presenter is up, in the PRESENTER pane or the
/// column's miniature (`place`: its class, test id and recorder steps).
#[component]
pub(crate) fn StageFrame(#[prop(into)] width: Signal<f64>, place: Place) -> impl IntoView {
    let (mount, load): (PresenterStep, fn(f64) -> PresenterStep<'static>) = match place {
        Place::Pane => (PresenterStep::Frame, |ms| PresenterStep::Load { ms }),
        Place::Column => (PresenterStep::MiniFrame, |ms| PresenterStep::MiniLoad {
            ms,
        }),
    };
    diag::record(&trace::presenter(dom::epoch_now(), mount));
    let mounted = StoredValue::new(Some(dom::now()));
    let loaded = move |_: web_sys::Event| {
        if let Some(Some(at)) = mounted.try_update_value(Option::take) {
            let ms = dom::now() - at;
            diag::record(&trace::presenter(dom::epoch_now(), load(ms)));
        }
    };
    let style = move || {
        let factor = scale(width.try_get().unwrap_or(0.0)).unwrap_or(0.0);
        format!("width: {STAGE_W}px; height: {STAGE_H}px; transform: scale({factor})")
    };
    view! {
        <iframe
            class=place.class()
            data-testid=place.testid()
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

/// The rail's free room watcher: a `ResizeObserver` and its callback, kept
/// while the miniature's place lives.
type RailObserver = (web_sys::ResizeObserver, Closure<dyn FnMut()>);

/// The rail as it is laid out (`behave::presenter::RailRoom`): its width,
/// the buttons' room without the miniature (the rail less its foot and the
/// gap) and `.rail-main`'s resolved rows. The boxes are read unrounded
/// (`getBoundingClientRect`): a whole-px `clientHeight` rounded up could fit
/// a miniature half a px too tall.
fn measure_rail(rail: &web_sys::HtmlDivElement) -> Option<RailRoom> {
    let main = rail.query_selector(".rail-main").ok()??;
    let foot = rail.query_selector(".rail-foot").ok()??;
    let (box_, foot) = (
        rail.get_bounding_client_rect(),
        foot.get_bounding_client_rect(),
    );
    Some(RailRoom {
        width: box_.width(),
        room: box_.height() - foot.height() - RAIL_GAP,
        rows: track_count(&dom::computed(&main, "grid-template-rows").unwrap_or_default()),
    })
}

/// The column's stage miniature (1.37.0; way A of the owner's approved
/// `docs/mockups/presenter-stage-v2.html`): the same stage frame as the
/// PRESENTER pane, in the rail above TECHALERT (`rail`: the `.rail` it lies
/// in, measured on every resize), as wide as the rail or, since 1.38.2,
/// shrunk to the room the rail's rows leave at their full 44 px, never under
/// its least picture (40 px high, unless the rail is too narrow for it):
/// below that the rows give the room up, the faders never
/// (`behave::presenter::mini_fit`). A dim `STAGE` caption with the layout on
/// stage, `offline` in red and the picture dimmed while Presenter is down. A
/// tap opens the PRESENTER tab; while it is open, on a phone or with no
/// `[presenter]` it is not there at all. Each
/// change of the decision is recorded (`mini_fit`: the viewport, the rail's
/// room and rows, the picture or why it hides). The frame takes no touch;
/// the box owns its touches.
#[component]
pub(crate) fn StageMini(rail: NodeRef<html::Div>) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let store = nav.store;
    let measured = RwSignal::new(None::<RailRoom>);
    let observer = StoredValue::new_local(None::<RailObserver>);
    Effect::new(move |_| {
        let Some(el) = rail.get() else {
            return;
        };
        let observed = el.clone();
        let callback = Closure::wrap(Box::new(move || {
            let now = measure_rail(&observed);
            if measured.try_get_untracked() != Some(now) {
                let _ = measured.try_set(now);
            }
        }) as Box<dyn FnMut()>);
        if let Ok(watch) = web_sys::ResizeObserver::new(callback.as_ref().unchecked_ref()) {
            // The rail, and its foot: a foot that grows (TECHALERT's words, a
            // late font) leaves the rail's size as it was.
            watch.observe(&el);
            if let Ok(Some(foot)) = el.query_selector(".rail-foot") {
                watch.observe(&foot);
            }
            if let Some(Some((old, _))) =
                observer.try_update_value(|o| o.replace((watch, callback)))
            {
                old.disconnect();
            }
        }
    });
    on_cleanup(move || {
        if let Some(Some((watch, _))) = observer.try_update_value(Option::take) {
            watch.disconnect();
        }
    });
    let fit = Memo::new(move |_| {
        let configured = store.presenter.try_with(Option::is_some).unwrap_or(false);
        let open = nav.shown_extra() == Extra::Presenter;
        let (w, h) = nav.viewport.try_get().unwrap_or_default();
        mini_fit(configured, open, phone(w, h), measured.try_get().flatten())
    });
    Effect::new(move |_| {
        let Some(now) = fit.try_get() else {
            return;
        };
        if mini_fit_noted(now) {
            let (vw, vh) = nav.viewport.try_get_untracked().unwrap_or_default();
            let rail = measured.try_get_untracked().flatten();
            diag::record(&trace::presenter(
                dom::epoch_now(),
                PresenterStep::MiniFit {
                    vw,
                    vh,
                    rail,
                    fit: now,
                },
            ));
        }
    });
    let picture = Memo::new(move |_| fit.try_get().and_then(MiniFit::picture));
    // The box is mounted by whether it shows, never by its size: a new size
    // (the column settling, a turn of the screen) restyles the box it has.
    let shown = Memo::new(move |_| picture.try_with(Option::is_some).unwrap_or(false));
    move || {
        shown
            .try_get()
            .unwrap_or(false)
            .then(|| view! { <MiniBox picture=picture /> })
    }
}

/// The miniature's box: the caption and the picture; a tap opens PRESENTER.
#[component]
fn MiniBox(picture: Memo<Option<(f64, f64)>>) -> impl IntoView {
    let nav = expect_context::<Nav>();
    let store = nav.store;
    let size_of = move || picture.try_get().flatten().unwrap_or((0.0, 0.0));
    let width = Signal::derive(move || size_of().0);
    let note = Memo::new(move |_| {
        store
            .presenter
            .try_with(|p| p.as_ref().map(mini_note))
            .flatten()
            .unwrap_or_default()
    });
    let shows = Memo::new(move |_| {
        preview(
            store
                .presenter
                .try_with(|p| p.as_ref().and_then(|p| p.online))
                .flatten(),
        )
    });
    let picture = move || {
        (shows.try_get() == Some(Preview::Frame))
            .then(|| view! { <StageFrame width=width place=Place::Column /> })
    };
    let size = move || {
        let (w, h) = size_of();
        format!("width: {w}px; height: {h}px")
    };
    let tap = move |_: web_sys::PointerEvent| {
        diag::record(&trace::presenter(dom::epoch_now(), PresenterStep::MiniTap));
        nav.show_extra(Extra::Presenter);
    };
    // The box owns its touches (#43 PR G); it writes no key.
    let no_keys: Vec<String> = Vec::new();
    view! {
        <div
            class="stage-mini"
            role="button"
            aria-label="Open PRESENTER"
            use:owns_touches=no_keys
            data-testid="stage-mini"
            data-offline=move || note.try_with(|n| n.1).unwrap_or(false).to_string()
            on:pointerdown=tap
        >
            <div class="mini-cap">
                <span>{MINI_HEAD}</span>
                <em data-testid="mini-note">{move || note.try_with(|n| n.0.clone()).unwrap_or_default()}</em>
            </div>
            <div
                class="mini-stage"
                data-testid="mini-stage"
                data-preview=move || shows.try_get().unwrap_or(Preview::Waiting).name()
                style=size
            >
                {picture}
            </div>
        </div>
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
