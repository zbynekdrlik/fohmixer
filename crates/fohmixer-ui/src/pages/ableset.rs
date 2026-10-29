//! AbleSet's tab (#90, design §5; the owner's variant A of
//! `docs/mockups/ableset-tab-v1.html`, placed by #148: the owner's ruling
//! of 29.10.2026): one frame of AbleSet's own page,
//! `https://<AbleSet's name>[:the page's port]/`, over the left half of the
//! page shown (the left sectors, or the rows' left halves; the column stays
//! in the screen's middle, the page's own, and the right half keeps
//! working; on a phone the whole screen but the thin strip), opened
//! through the hub's `/.fohmixer/locked`, which stores AbleSet's lock first
//! (#90, the owner's report of 2026-10-14: every open starts it locked). The
//! frame is mounted only while the tab shows it: on the mixer AbleSet's app, its
//! socket and its drawing cost the iPad nothing. Before it mounts, an image
//! of the hub's `/.fohmixer/probe` on AbleSet's name must load within
//! `PROBE_MS` (an image outside the page, whose handler outlives the tab:
//! its answer can come after the bound or after the tab closed, #90 C2):
//! outside the church AbleSet's name has an Access application
//! of its own, whose login would not show inside a frame, so a failed probe
//! through the tunnel shows the sign-in panel instead (a top-level sign-in
//! that comes back to `#ableset`); on the LAN (the hub's word for this
//! page) a failed probe says why and leaves nothing to tap (#90 PR C review
//! M2). `behave::ableset` decides what shows; this file carries it
//! out. What happens inside the frame (AbleSet's own taps, scrolls and keys)
//! is AbleSet's: the page sees none of it.

use std::time::Duration;

use leptos::prelude::*;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

use crate::behave::ableset::{
    Fail, PROBE_MS, Panel, Probe, SIGNIN_BUTTON, Show, after_link, again, direct_url, frame_url,
    framed_here, panel_text, panel_title, probe_url, probed, probes, returns_to_tab, show,
    signin_url,
};
use crate::components::owns_touches;
use crate::diag;
use crate::diag::trace::{self, AbleSetStep};
use crate::dom;
use crate::store::LiveStore;

/// The frame's sandbox (design §5.3): AbleSet's scripts, its own storage
/// (another origin, so its own), its forms, pop-ups and downloads; never
/// the top-level navigation that would take the mixer away.
const SANDBOX: &str = "allow-scripts allow-same-origin allow-forms allow-popups \
                       allow-popups-to-escape-sandbox allow-modals allow-downloads";

/// The page came back from AbleSet's sign-in (`#ableset`, design §5.4): the
/// hash goes at once (a reload never opens the tab again), and the caller
/// opens the tab once the hub tells of AbleSet's page.
pub fn take_return() -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let location = window.location();
    if !returns_to_tab(&location.hash().unwrap_or_default()) {
        return false;
    }
    let path = location.pathname().unwrap_or_default();
    let search = location.search().unwrap_or_default();
    if let Ok(history) = window.history() {
        let _ =
            history.replace_state_with_url(&JsValue::NULL, "", Some(&format!("{path}{search}")));
    }
    true
}

/// AbleSet's tab: AbleSet's frame or a panel over the page's left half
/// (#148).
#[component]
pub fn AbleSetView() -> impl IntoView {
    let store = expect_context::<LiveStore>();
    // Where this page is: AbleSet frames only in fohmixer's own https page,
    // and its addresses take this page's port.
    let (protocol, hostname, port) = web_sys::window()
        .map(|w| {
            let l = w.location();
            (
                l.protocol().unwrap_or_default(),
                l.hostname().unwrap_or_default(),
                l.port().unwrap_or_default(),
            )
        })
        .unwrap_or_default();
    let at = StoredValue::new((hostname.clone(), port));

    let probe = RwSignal::new(Probe::Idle);
    let probe_n = StoredValue::new(0_u32);
    let probe_started = StoredValue::new(0.0_f64);
    let frame_mounted = StoredValue::new(None::<f64>);
    let online = Memo::new(move |_| {
        store
            .ableset
            .try_with(|a| a.as_ref().and_then(|a| a.online))
            .flatten()
    });
    let here = Memo::new(move |_| {
        store
            .ableset
            .try_with(|a| {
                a.as_ref()
                    .is_some_and(|a| framed_here(&protocol, &hostname, &a.parent))
            })
            .unwrap_or(false)
    });
    // Whether this page's socket came over the LAN (the hub's word).
    let lan = Memo::new(move |_| {
        store
            .ableset
            .try_with(|a| a.as_ref().is_some_and(|a| a.lan))
            .unwrap_or(false)
    });
    let connected = move || store.connected.try_get().unwrap_or(false);
    let shown = Memo::new(move |_| {
        show(
            online.try_get().flatten(),
            here.try_get().unwrap_or(false),
            connected(),
            probe.try_get().unwrap_or(Probe::Idle),
            lan.try_get().unwrap_or(false),
        )
    });

    diag::record(&trace::ableset_view(dom::epoch_now(), true));
    on_cleanup(move || diag::record(&trace::ableset_view(dom::epoch_now(), false)));

    // Probe `n`'s answer (its image's load or error, or the time bound): only
    // the probe on its way takes it, and the recorder hears it with its time.
    let answer = move |n: u32, outcome: Result<(), Fail>| {
        let Some(before) = probe.try_get_untracked() else {
            return;
        };
        let after = probed(before, n, outcome);
        if after != before {
            let _ = probe.try_set(after);
            let ms = dom::now() - probe_started.try_get_value().unwrap_or(0.0);
            diag::record(&trace::ableset(
                dom::epoch_now(),
                AbleSetStep::Probe {
                    ok: outcome.is_ok(),
                    ms,
                },
            ));
        }
    };
    // AbleSet down forgets the probe: its return probes again and mounts a
    // fresh frame.
    Effect::new(move |_| {
        let online = online.try_get().flatten();
        if let Some(before) = probe.try_get_untracked() {
            let after = after_link(online, before);
            if after != before {
                let _ = probe.try_set(after);
            }
        }
    });
    // A probe starts when one is due: an image of its address that is never
    // in the page, and it fails if no answer came within PROBE_MS. The
    // browser loads an image whatever happens to the view, so its one
    // handler (load or error) belongs to the JS GC (`into_js_value`), not
    // to a view: a view's `on:load` closure is dropped with the view, and an
    // answer after the bound, or after the tab closed, then called a dropped
    // closure (#90 C2). `answer` takes only the probe on its way, and finds
    // nothing once the tab is gone.
    Effect::new(move |_| {
        let due = probes(
            online.try_get().flatten(),
            here.try_get().unwrap_or(false),
            connected(),
            probe.try_get().unwrap_or(Probe::Passed),
        );
        if !due {
            return;
        }
        let Some(n) = probe_n.try_update_value(|n| {
            *n += 1;
            *n
        }) else {
            return;
        };
        let _ = probe_started.try_set_value(dom::now());
        let _ = probe.try_set(Probe::Pending(n));
        let port = at
            .try_with_value(|(_, port)| port.clone())
            .unwrap_or_default();
        let src = store
            .ableset
            .try_with_untracked(|a| {
                a.as_ref()
                    .map(|a| probe_url(&a.name, &port, dom::wall_now() as u64))
            })
            .flatten();
        if let (Some(src), Ok(image)) = (src, web_sys::HtmlImageElement::new()) {
            let heard = Closure::<dyn FnMut(web_sys::Event)>::new(move |event: web_sys::Event| {
                let loaded = event.type_() == "load";
                answer(n, if loaded { Ok(()) } else { Err(Fail::Error) });
            })
            .into_js_value();
            image.set_onload(Some(heard.unchecked_ref()));
            image.set_onerror(Some(heard.unchecked_ref()));
            image.set_src(&src);
        }
        set_timeout(
            move || answer(n, Err(Fail::Late)),
            Duration::from_millis(u64::from(PROBE_MS)),
        );
    });
    // Each panel shown goes into the flight recorder once.
    Effect::new(move |before: Option<Show>| {
        let now = shown.try_get().unwrap_or(Show::Panel(Panel::Checking));
        if before != Some(now)
            && let Show::Panel(panel) = now
        {
            diag::record(&trace::ableset(
                dom::epoch_now(),
                AbleSetStep::Panel(panel.name()),
            ));
        }
        now
    });
    // The sign-in panel probes again when the page comes back (the sign-in
    // may have happened in the meantime). `visibilitychange` bubbles from
    // the document to the window.
    let visible = window_event_listener_untyped("visibilitychange", move |_| {
        if dom::hidden() {
            return;
        }
        if let Some(before) = probe.try_get_untracked() {
            let after = again(before);
            if after != before {
                let _ = probe.try_set(after);
            }
        }
    });
    on_cleanup(move || visible.remove());

    // The frame, mounted while `shown` says so: the hub's facts are read
    // untracked, so their news never reloads a mounted frame.
    let content = move || match shown.try_get().unwrap_or(Show::Panel(Panel::Checking)) {
        Show::Frame => {
            let port = at
                .try_with_value(|(_, port)| port.clone())
                .unwrap_or_default();
            let (title, src) = store
                .ableset
                .try_with_untracked(|a| {
                    a.as_ref()
                        .map(|a| (a.title.clone(), frame_url(&a.name, &port)))
                })
                .flatten()
                .unwrap_or_default();
            let _ = frame_mounted.try_set_value(Some(dom::now()));
            let loaded = move |_: web_sys::Event| {
                if let Some(Some(mounted)) = frame_mounted.try_update_value(Option::take) {
                    let ms = dom::now() - mounted;
                    diag::record(&trace::ableset(dom::epoch_now(), AbleSetStep::Load { ms }));
                }
            };
            view! {
                <iframe
                    class="ableset-frame"
                    data-testid="ableset-frame"
                    title=title
                    src=src
                    sandbox=SANDBOX
                    referrerpolicy="no-referrer"
                    allow=""
                    on:load=loaded
                ></iframe>
            }
            .into_any()
        }
        Show::Panel(panel) => {
            let (hostname, port) = at.try_get_value().unwrap_or_default();
            let info = store.ableset.try_get_untracked().flatten();
            let direct = info
                .as_ref()
                .filter(|_| panel == Panel::Elsewhere)
                .and_then(|a| direct_url(&hostname, a.lan_port));
            let signin = info
                .as_ref()
                .map(|a| signin_url(&a.name, &port))
                .unwrap_or_default();
            view! { <AbleSetPanel panel=panel direct=direct signin=signin /> }.into_any()
        }
    };
    let state = move || shown.try_get().map_or("checking", Show::name);
    view! {
        <div class="extra-pane ableset-page" data-testid="ableset" data-state=state>
            <div class="ableset-area">{content}</div>
        </div>
    }
}

/// A panel in the frame's place (the mockup's states, and #90 PR C review
/// M2's LAN failure): its title, its text, and for the sign-in its button
/// (to Access's top-level sign-in), for the emergency address AbleSet's
/// direct address on the PC (in a new tab, without an opener). Both act at
/// `pointerup`, where a finger's touch activates; the LAN failure has none.
#[component]
fn AbleSetPanel(panel: Panel, direct: Option<String>, signin: String) -> impl IntoView {
    let title = panel_title(panel);
    let text = panel_text(panel, direct.is_some());
    let heading = (!title.is_empty()).then(|| view! { <h3>{title}</h3> });
    let button = (panel == Panel::SignIn).then(|| {
        let no_keys: Vec<String> = Vec::new();
        let go = move |_: web_sys::PointerEvent| {
            diag::record(&trace::ableset(dom::epoch_now(), AbleSetStep::SignIn));
            if let Some(window) = web_sys::window() {
                let _ = window.location().assign(&signin);
            }
        };
        view! {
            <button
                type="button"
                class="ableset-btn"
                use:owns_touches=no_keys
                data-testid="ableset-signin"
                on:pointerup=go
            >
                {SIGNIN_BUTTON}
            </button>
        }
    });
    let address = direct.map(|url| {
        let no_keys: Vec<String> = Vec::new();
        let shown = url.clone();
        // Without an opener (#90 PR C review L4): AbleSet's page cannot
        // reach the mixer's tab.
        let open = move |_: web_sys::PointerEvent| {
            if let Some(window) = web_sys::window() {
                let _ = window.open_with_url_and_target_and_features(&url, "_blank", "noopener");
            }
        };
        view! {
            <button
                type="button"
                class="ableset-addr"
                use:owns_touches=no_keys
                data-testid="ableset-direct"
                on:pointerup=open
            >
                {shown}
            </button>
        }
    });
    view! {
        <div class="ableset-panel">
            <div class="ableset-box" data-testid="ableset-panel" data-panel=panel.name()>
                {heading}
                <p class="ableset-text">{text}</p>
                {button}
                {address}
            </div>
        </div>
    }
}
