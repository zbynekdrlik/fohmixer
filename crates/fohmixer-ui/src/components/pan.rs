//! The pan control (spec F11): a horizontal relative drag with its own
//! pointer, two releases within 300 ms centre it, grey when centred and
//! cyan otherwise. The frame loop writes the position as `--p` and the bar
//! from the centre as `--lo` / `--w` (the stylesheet draws them; #21). Its
//! writes go through the store's intents like a fader's (#43): a release
//! with nothing unsent tells the store the release time (L4). It shows
//! Live's value after its hold (PR B's decision 7) and, like a fader's cap,
//! its dot is outlined while its write is `unconfirmed` or `not_sent`
//! (`data-intent`, `intent.css`, #43 PR C); each touch's down, up and cancel
//! goes to the page's flight recorder.

use fohmixer_proto::client::set_key;
use leptos::html;
use leptos::prelude::*;
use serde_json::json;

use super::{fail_flash, readiness, trace_touch};
use crate::behave::pan::{self, PanCtl};
use crate::behave::{TouchEnd, touch_end};
use crate::binding::SubSpec;
use crate::dom;
use crate::raf;
use crate::store::intent::State;
use crate::store::{LiveStore, Slot};

/// The dot's width in px (`.pan-dot` in the stylesheet): it travels the
/// bar's width less its own.
const DOT: f64 = 12.0;

/// A pan control showing and writing Live's panning.
#[component]
pub fn PanView(state: RwSignal<Slot>, spec: SubSpec) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let slot = state;
    let keys = vec![set_key(&spec.instance, &spec.target, &spec.prop)];
    let shown_key = keys[0].clone();
    // A pan taken away under a finger gets no pointerup: its write counts as
    // released then (L4).
    {
        let keys = keys.clone();
        on_cleanup(move || store.release(&keys));
    }
    let key = StoredValue::new(keys);
    let spec = StoredValue::new(spec);
    let ctl = StoredValue::new(PanCtl::default());
    let failed = RwSignal::new(false);
    let root = NodeRef::<html::Div>::new();

    let live = move || slot.try_with_untracked(Slot::number).flatten();
    // Each write a `set` through the store's intents (#43); the release's
    // is `final`.
    let send = move |panning: f64, is_final: bool| {
        let _ = spec.try_with_value(|s| {
            store.set(
                &s.instance,
                &s.target,
                &s.prop,
                json!(panning),
                is_final,
                Some(fail_flash(failed)),
            );
        });
    };
    let on_down = move |ev: web_sys::PointerEvent| {
        let Some(at) = live() else {
            return;
        };
        let Some(el) = dom::current_element(&ev) else {
            return;
        };
        ev.prevent_default();
        // The dot's travel, in screen px: the width less the dot's own.
        let travel = (el.get_bounding_client_rect().width() - DOT).max(1.0);
        let id = ev.pointer_id();
        let x = f64::from(ev.client_x());
        let taken = ctl
            .try_update_value(|c| c.down(id, x, travel, dom::now(), at))
            .unwrap_or(false);
        if taken {
            let _ = el.set_pointer_capture(id);
            let _ = key.try_with_value(|k| {
                store.touch(k);
                trace_touch("down", k, id);
            });
        }
    };
    let on_move = move |ev: web_sys::PointerEvent| {
        let (id, x) = (ev.pointer_id(), f64::from(ev.client_x()));
        let _ = ctl.try_update_value(|c| c.moved(id, x));
    };
    // The end of a touch (`behave::touch_end`): the unsent move as a final
    // `set`, or the release time of the write already sent (L4).
    // The flight recorder hears the end of this pan's own touches.
    let ended = move |end: Option<TouchEnd>, what: &str, id: i32| {
        if end.is_some_and(TouchEnd::ends_touch) {
            let _ = key.try_with_value(|k| trace_touch(what, k, id));
        }
        match end {
            Some(TouchEnd::Send(v)) => send(v, true),
            Some(TouchEnd::Released) => {
                let _ = key.try_with_value(|k| store.release(k));
            }
            Some(TouchEnd::NotMine) | None => {}
        }
    };
    let on_up = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        let end = ctl.try_update_value(|c| touch_end(c.drives(id), c.up(id, dom::now())));
        ended(end, "up", id);
    };
    // A cancelled pointer, or one whose capture was lost without an up.
    let on_cancel = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        let end = ctl.try_update_value(|c| touch_end(c.drives(id), c.cancel(id, dom::now())));
        ended(end, "cancel", id);
    };

    raf::animate(root, move |el| {
        let dot = dom::child(&el, ".pan-dot");
        let mut shown: Option<f64> = None;
        let mut look: Option<State> = None;
        Box::new(move |now: f64, _step: f64| {
            // Its write's state (#43, PR C): the outline only, the dot shows
            // Live's value after the hold.
            let intent = store.intent_state(&shown_key);
            if look != Some(intent) {
                dom::set_attr(&el, "data-intent", intent.name());
                look = Some(intent);
            }
            let Some(motion) = ctl.try_update_value(|c| c.frame(now, live())) else {
                return;
            };
            if let Some(v) = motion.send {
                send(v, false);
            }
            let Some(p) = motion.pos else {
                return;
            };
            if shown.is_some_and(|s| (s - p).abs() < 1e-5) {
                return;
            }
            shown = Some(p);
            dom::set_style(&el, "--p", &format!("{p:.5}"));
            dom::set_style(&el, "--lo", &format!("{:.5}", p.min(0.5)));
            dom::set_style(&el, "--w", &format!("{:.5}", (p - 0.5).abs()));
            if let Some(dot) = &dot {
                dom::set_style(dot, "background", pan::color(p));
            }
            dom::set_attr(&el, "data-value", &format!("{:.4}", pan::to_live(p)));
        })
    });

    let state = Memo::new(move |_| readiness(&[slot]));
    let binding = move || state.get().name();
    let disabled = move || state.get().disabled();
    view! {
        <div
            class="pan"
            class:failed=move || failed.get()
            data-testid="pan"
            data-binding=binding
            aria-disabled=disabled
            node_ref=root
            on:pointerdown=on_down
            on:pointermove=on_move
            on:pointerup=on_up
            on:pointercancel=on_cancel
            on:lostpointercapture=on_cancel
        >
            <div class="pan-track"></div>
            <div class="pan-fill"></div>
            <div class="pan-dot"></div>
        </div>
    }
}
