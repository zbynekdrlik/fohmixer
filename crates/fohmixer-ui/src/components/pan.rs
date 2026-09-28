//! The pan control (spec F11): a horizontal relative drag with its own
//! pointer, two releases within 300 ms centre it, grey when centred and
//! cyan otherwise. The dot moves only in the frame loop.

use fohmixer_proto::layout::Frame;
use leptos::html;
use leptos::prelude::*;
use serde_json::json;

use super::{fail_flash, readiness};
use crate::behave::pan::{self, PanCtl};
use crate::binding::SubSpec;
use crate::dom;
use crate::raf;
use crate::stage;
use crate::store::{LiveStore, Slot};

/// The dot's width in canvas px.
const DOT: f64 = 28.0;

/// A pan control showing and writing Live's panning.
#[component]
pub fn PanView(frame: Frame, state: RwSignal<Slot>, spec: SubSpec) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let slot = state;
    let spec = StoredValue::new(spec);
    let ctl = StoredValue::new(PanCtl::default());
    let failed = RwSignal::new(false);
    let root = NodeRef::<html::Div>::new();

    let live = move || slot.try_with_untracked(Slot::number).flatten();
    let send = move |panning: f64| {
        let _ = spec.try_with_value(|s| {
            store.set_prop(
                &s.instance,
                &s.target,
                &s.prop,
                json!(panning),
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
        let width = el.get_bounding_client_rect().width();
        let travel = width * (frame.w - DOT).max(1.0) / frame.w.max(1.0);
        let id = ev.pointer_id();
        let x = f64::from(ev.client_x());
        let taken = ctl
            .try_update_value(|c| c.down(id, x, travel, dom::now(), at))
            .unwrap_or(false);
        if taken {
            let _ = el.set_pointer_capture(id);
        }
    };
    let on_move = move |ev: web_sys::PointerEvent| {
        let (id, x) = (ev.pointer_id(), f64::from(ev.client_x()));
        let _ = ctl.try_update_value(|c| c.moved(id, x));
    };
    let on_up = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        if let Some(Some(v)) = ctl.try_update_value(|c| c.up(id, dom::now())) {
            send(v);
        }
    };
    // A cancelled pointer, or one whose capture was lost without an up.
    let on_cancel = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        if let Some(Some(v)) = ctl.try_update_value(|c| c.cancel(id, dom::now())) {
            send(v);
        }
    };

    raf::animate(root, move |el| {
        let dot = dom::child(&el, ".pan-dot");
        let travel = (frame.w - DOT).max(0.0);
        let mut shown: Option<f64> = None;
        Box::new(move |now: f64, _step: f64| {
            let Some(motion) = ctl.try_update_value(|c| c.frame(now, live())) else {
                return;
            };
            if let Some(v) = motion.send {
                send(v);
            }
            let Some(p) = motion.pos else {
                return;
            };
            if shown.is_some_and(|s| (s - p).abs() < 1e-5) {
                return;
            }
            shown = Some(p);
            if let Some(dot) = &dot {
                dom::set_style(dot, "transform", &format!("translateX({}px)", p * travel));
                dom::set_style(dot, "background", pan::color(p));
            }
            dom::set_attr(&el, "data-value", &format!("{:.4}", pan::to_live(p)));
        })
    });

    let binding = move || readiness(&[slot]).name();
    let disabled = move || readiness(&[slot]).disabled();
    view! {
        <div
            class="pan"
            class:failed=move || failed.get()
            data-testid="pan"
            data-binding=binding
            aria-disabled=disabled
            style={stage::box_style(frame)}
            node_ref=root
            on:pointerdown=on_down
            on:pointermove=on_move
            on:pointerup=on_up
            on:pointercancel=on_cancel
            on:lostpointercapture=on_cancel
        >
            <div class="pan-track"></div>
            <div class="pan-dot"></div>
        </div>
    }
}
