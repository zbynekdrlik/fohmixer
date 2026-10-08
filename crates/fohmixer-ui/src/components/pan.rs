//! The pan control (spec F11): a horizontal relative drag with its own
//! pointer, two releases within 300 ms glide it to the centre (#63), grey
//! when centred and cyan otherwise. The frame loop writes the position as
//! `--p` and the bar from the centre as `--lo` / `--w` (the stylesheet
//! draws them; #21). Its
//! writes go through the store's intents like a fader's (#43): a release
//! with nothing unsent tells the store the release time (L4). It shows
//! Live's value after its hold (PR B's decision 7) and, like a fader's cap,
//! its dot is outlined while its write is `unconfirmed` or `not_sent`
//! (`data-intent`, `intent.css`, #43 PR C); each touch's down (with where
//! it started, `PanCtl::press`), up and cancel goes to the page's flight
//! recorder, and each frame that sends from the finger with the pointer moves
//! it carried (`diag::trace::moves::Trail`, PR D).

use fohmixer_proto::client::set_key;
use leptos::html;
use leptos::prelude::*;
use serde_json::json;

use super::{fail_flash, owns_touches, readiness, trace_move, trace_start, trace_touch};
use crate::behave::pan::{self, PanCtl};
use crate::behave::{TouchEnd, touch_end};
use crate::binding::SubSpec;
use crate::diag::trace::moves::{Axis, Press, Trail};
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
    // The root owns its touches (#43 PR G) and names its key.
    let touch_keys = keys.clone();
    let key = StoredValue::new(keys);
    let shown_key = StoredValue::new(shown_key);
    let spec = StoredValue::new(spec);
    let ctl = StoredValue::new(PanCtl::default());
    // The finger's moves between frames, for the flight recorder (#43 PR D).
    let trail = StoredValue::new(Trail::default());
    let failed = RwSignal::new(false);
    let root = NodeRef::<html::Div>::new();

    let live = move || slot.try_with_untracked(Slot::number).flatten();
    // Each write a `set` through the store's intents (#43); the release's
    // is `final`. The finger's moves since the last send go to the recorder
    // with it (the move record holds positions: Live's panning is 2p − 1).
    let send = move |panning: f64, is_final: bool| {
        let seq = spec
            .try_with_value(|s| {
                store.set(
                    &s.instance,
                    &s.target,
                    &s.prop,
                    json!(panning),
                    is_final,
                    Some(fail_flash(failed)),
                )
            })
            .flatten();
        let _ = shown_key.try_with_value(|k| trace_move(trail, k, pan::to_pos(panning), seq));
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
            .try_update_value(|c| c.press(id, x, travel, dom::now(), at))
            .flatten();
        if let Some(start) = taken {
            let _ = el.set_pointer_capture(id);
            let press = Press {
                at: dom::event_epoch(&ev),
                c: x,
                travel,
            };
            let _ = trail.try_update_value(|t| t.start(id, Axis::Right, press, start.from));
            let _ = key.try_with_value(|k| {
                store.touch(k);
                trace_start(k, id, press, start, None);
            });
        }
    };
    let on_move = move |ev: web_sys::PointerEvent| {
        let (id, x) = (ev.pointer_id(), f64::from(ev.client_x()));
        // A move this pan took, with where it left the pan (the touch's
        // first one only anchors it: the recorder's `a`, #43 PR F).
        let moved = ctl
            .try_update_value(|c| c.moved(id, x).then(|| c.pos()))
            .flatten();
        if let Some(pos) = moved {
            let at = dom::event_epoch(&ev);
            let _ = trail.try_update_value(|t| t.moved(id, at, x, pos));
        }
    };
    // The end of a touch (`behave::touch_end`): the unsent move as a final
    // `set`, or the release time of the write already sent (L4).
    // The flight recorder hears the end of this pan's own touches.
    let ended = move |end: Option<TouchEnd>, what: &str, id: i32| {
        match end {
            Some(TouchEnd::Send(v)) => send(v, true),
            Some(TouchEnd::Released) => {
                let _ = key.try_with_value(|k| store.release(k));
            }
            Some(TouchEnd::NotMine) | None => {}
        }
        if end.is_some_and(TouchEnd::ends_touch) {
            let _ = key.try_with_value(|k| trace_touch(what, k, id));
            let _ = trail.try_update_value(|t| t.end(id));
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
            let intent = shown_key
                .try_with_value(|k| store.intent_state(k))
                .unwrap_or(State::Confirmed);
            if look != Some(intent) {
                dom::set_attr(&el, "data-intent", intent.name());
                look = Some(intent);
            }
            let Some((motion, glide_ended)) =
                ctl.try_update_value(|c| (c.frame(now, live()), c.take_ended()))
            else {
                return;
            };
            if let Some(v) = motion.send {
                send(v, false);
            }
            // A glide's end is a release (L4), as a fader's.
            if glide_ended {
                let _ = key.try_with_value(|k| store.release(k));
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
    let binding = move || state.try_get().map_or("waiting", |s| s.name());
    let disabled = move || state.try_get().map_or("true", |s| s.disabled());
    view! {
        <div
            class="pan"
            class:failed=move || failed.try_get().unwrap_or(false)
            data-testid="pan"
            data-binding=binding
            aria-disabled=disabled
            node_ref=root
            use:owns_touches=touch_keys
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
