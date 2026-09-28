//! The fader (spec F8–F10, I4, F18): Pointer Events with this fader's own
//! pointer (several faders move at once), relative movement, the touch
//! shaping, the double-tap glide, sends coalesced to one per animation frame
//! and the final value on release. The cap moves only in the frame loop.

use fohmixer_proto::layout::Frame;
use leptos::html;
use leptos::prelude::*;
use serde_json::json;

use super::fail_flash;
use crate::behave::fader::{self as curve, FaderCtl, UNITY};
use crate::binding::SubSpec;
use crate::dom;
use crate::raf;
use crate::stage;
use crate::store::{LiveStore, Slot};

/// The cap's height in canvas px; the cap travels the rest of the frame.
pub const CAP: f64 = 36.0;

/// How a fader maps its position to Live's value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Law {
    /// A track volume: `v = p^0.515`, double tap to 0 dB.
    Volume,
    /// A parameter's range, linear (`cc_linear`), read from Live.
    Linear(RwSignal<Option<(f64, f64)>>),
}

impl Law {
    /// The position of Live's value, when the law can say.
    fn pos(self, value: f64) -> Option<f64> {
        match self {
            Self::Volume => Some(curve::to_pos(value)),
            Self::Linear(range) => range
                .try_get_untracked()
                .flatten()
                .map(|r| curve::linear_pos(value, r)),
        }
    }

    /// Live's value at position `p`.
    fn value(self, p: f64) -> Option<f64> {
        match self {
            Self::Volume => Some(curve::to_live(p)),
            Self::Linear(range) => range
                .try_get_untracked()
                .flatten()
                .map(|r| curve::linear_value(p, r)),
        }
    }

    /// Where a double tap glides.
    fn glide_to(self) -> Option<f64> {
        match self {
            Self::Volume => Some(curve::to_pos(UNITY)),
            Self::Linear(_) => None,
        }
    }

    /// Whether the law can map (reactive: a parameter's range arrived).
    fn ready(self) -> bool {
        match self {
            Self::Volume => true,
            Self::Linear(range) => range.with(Option::is_some),
        }
    }
}

/// A vertical fader showing `slot` (by the first target's law) and
/// writing every target, each by its own law.
#[component]
pub fn FaderView(
    frame: Frame,
    slot: RwSignal<Slot>,
    targets: Vec<(SubSpec, Law)>,
    shaping: bool,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let law = targets.first().map_or(Law::Volume, |(_, law)| *law);
    let targets = StoredValue::new(targets);
    let ctl = StoredValue::new(FaderCtl::new(shaping, law.glide_to()));
    let failed = RwSignal::new(false);
    let frame_id = StoredValue::new(None::<usize>);
    let root = NodeRef::<html::Div>::new();

    let ready = move || slot.with(Slot::is_ready) && law.ready();
    let live = move || {
        slot.try_with_untracked(Slot::number)
            .flatten()
            .and_then(|v| law.pos(v))
    };
    let send = move |p: f64| {
        let _ = targets.try_with_value(|all| {
            for (t, target_law) in all {
                if let Some(value) = target_law.value(p) {
                    store.set_prop(
                        &t.instance,
                        &t.target,
                        &t.prop,
                        json!(value),
                        Some(fail_flash(failed)),
                    );
                }
            }
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
        let height = el.get_bounding_client_rect().height();
        let travel = height * (frame.h - CAP).max(1.0) / frame.h.max(1.0);
        let id = ev.pointer_id();
        let y = f64::from(ev.client_y());
        let taken = ctl
            .try_update_value(|c| c.down(id, y, travel, dom::now(), at))
            .unwrap_or(false);
        if taken {
            let _ = el.set_pointer_capture(id);
        }
    };
    let on_move = move |ev: web_sys::PointerEvent| {
        let (id, y) = (ev.pointer_id(), f64::from(ev.client_y()));
        let _ = ctl.try_update_value(|c| c.moved(id, y, dom::now()));
    };
    let on_up = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        if let Some(Some(p)) = ctl.try_update_value(|c| c.up(id, dom::now())) {
            send(p);
        }
    };
    let on_cancel = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        if let Some(Some(p)) = ctl.try_update_value(|c| c.cancel(id, dom::now())) {
            send(p);
        }
    };

    root.on_load(move |el: web_sys::HtmlDivElement| {
        let cap = dom::child(&el, ".fader-cap");
        let fill = dom::child(&el, ".fader-fill");
        let travel = (frame.h - CAP).max(0.0);
        let mut shown: Option<f64> = None;
        let id = raf::register(Box::new(move |now: f64, _step: f64| {
            let Some(motion) = ctl.try_update_value(|c| c.frame(now, live())) else {
                return;
            };
            if let Some(p) = motion.send {
                send(p);
            }
            let Some(p) = motion.pos else {
                return;
            };
            if shown.is_some_and(|s| (s - p).abs() < 1e-5) {
                return;
            }
            shown = Some(p);
            if let Some(cap) = &cap {
                dom::set_style(cap, "transform", &format!("translateY({}px)", -p * travel));
            }
            if let Some(fill) = &fill {
                dom::set_style(fill, "transform", &format!("scaleY({p})"));
            }
            if let Some(v) = law.value(p) {
                dom::set_attr(&el, "data-value", &format!("{v:.4}"));
            }
        }));
        frame_id.set_value(Some(id));
    });
    on_cleanup(move || {
        if let Some(Some(id)) = frame_id.try_get_value() {
            raf::unregister(id);
        }
    });

    let disabled = move || if ready() { "false" } else { "true" };
    view! {
        <div
            class="fader"
            class:failed=move || failed.get()
            data-testid="fader"
            aria-disabled=disabled
            style={stage::box_style(frame)}
            node_ref=root
            on:pointerdown=on_down
            on:pointermove=on_move
            on:pointerup=on_up
            on:pointercancel=on_cancel
        >
            <div class="fader-groove"></div>
            <div class="fader-fill"></div>
            <div class="fader-cap"></div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_volume_law_is_the_fader_curve_with_a_glide_to_0_db() {
        let law = Law::Volume;
        assert_eq!(law.pos(0.85), Some(curve::to_pos(0.85)));
        assert_eq!(law.value(0.5), Some(curve::to_live(0.5)));
        assert_eq!(law.glide_to(), Some(curve::to_pos(UNITY)));
        assert!(law.ready());
    }

    #[test]
    fn the_linear_law_waits_for_the_parameters_range() {
        let range = RwSignal::new(None);
        let law = Law::Linear(range);
        assert_eq!(law.pos(0.5), None);
        assert_eq!(law.value(0.5), None);
        assert!(!law.ready());
        assert_eq!(law.glide_to(), None, "no double tap on a parameter fader");
        let _ = range.try_set(Some((-15.0, 15.0)));
        assert!(law.ready());
        assert_eq!(law.pos(0.0), Some(0.5));
        assert_eq!(law.value(0.25), Some(-7.5));
    }
}
