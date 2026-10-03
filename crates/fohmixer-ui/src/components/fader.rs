//! The fader (spec F8–F10, I4, F18): Pointer Events with this fader's own
//! pointer (several faders move at once), relative movement, the touch
//! shaping, the double-tap glide, sends coalesced to one per animation frame
//! and the final value on release, each a `set` through the store's intents
//! (#43; the release's is `final`). The frame loop writes the position as
//! `--p` (the stylesheet places the cap and the fill from it). The travel
//! is the track's whole height, 1:1 with the finger (TouchOSC's
//! `responseFactor` 100; #21, parity audit #2).
//!
//! Its write's intent (#43, PR B): while the store holds the first target's
//! write open the cap shows it (L3); the frame loop writes the state as
//! `data-intent` and, while it is `unconfirmed` or `not_sent`, Live's value
//! as the ghost (`--g`, `data-ghost`): the stylesheet outlines the cap amber
//! or red and draws the ghost line, no text (§4.3). A release with nothing
//! unsent and a glide's end tell the store the release time (L4); a touch
//! drops a `not_sent` write.

use fohmixer_proto::client::set_key;
use leptos::html;
use leptos::prelude::*;
use serde_json::json;

use super::{fail_flash, readiness, readiness_now};
use crate::behave::fader::{self as curve, FaderCtl, UNITY};
use crate::behave::{TouchEnd, touch_end};
use crate::binding::SubSpec;
use crate::dom;
use crate::raf;
use crate::store::intent::State;
use crate::store::{LiveStore, Readiness, Slot};

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

    /// The same, untracked (in an event handler).
    fn can_map(self) -> bool {
        match self {
            Self::Volume => true,
            Self::Linear(range) => range.try_with_untracked(Option::is_some).unwrap_or(false),
        }
    }
}

/// A fader's readiness: its slots', and waiting while a law has no range
/// yet (I8: every target must be able to take the value).
fn fader_readiness(slots: Readiness, mapped: bool) -> Readiness {
    match slots {
        Readiness::Ready if !mapped => Readiness::Waiting,
        other => other,
    }
}

/// One target a fader writes: its subscription (none when its path does
/// not parse), its slot and its law.
#[derive(Debug, Clone)]
pub struct Target {
    pub spec: Option<SubSpec>,
    pub slot: RwSignal<Slot>,
    pub law: Law,
}

/// A vertical fader showing its first target (by that target's law) and
/// writing every target, each by its own law. It takes a touch only while
/// every target has Live's value and a law that can map it (I8), and is
/// red while any target's binding does not resolve (I5).
#[component]
pub fn FaderView(targets: Vec<Target>, shaping: bool) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let (slot, law) = targets.first().map_or_else(
        || {
            (
                RwSignal::new(Slot::Error("a fader without a target".into())),
                Law::Volume,
            )
        },
        |t| (t.slot, t.law),
    );
    let mut slots: Vec<RwSignal<Slot>> = targets.iter().map(|t| t.slot).collect();
    if slots.is_empty() {
        slots.push(slot);
    }
    let laws: Vec<Law> = targets.iter().map(|t| t.law).collect();
    // The write keys of the targets (#43); the first is the one shown.
    let keys: Vec<String> = targets
        .iter()
        .filter_map(|t| t.spec.as_ref())
        .map(|s| set_key(&s.instance, &s.target, &s.prop))
        .collect();
    let shown_key = targets
        .first()
        .and_then(|t| t.spec.as_ref())
        .map(|s| set_key(&s.instance, &s.target, &s.prop));
    let keys = StoredValue::new(keys);
    let targets = StoredValue::new(targets);
    let ctl = StoredValue::new(FaderCtl::new(shaping, law.glide_to()));
    let failed = RwSignal::new(false);
    let root = NodeRef::<html::Div>::new();

    let state = {
        let (slots, laws) = (slots.clone(), laws.clone());
        Memo::new(move |_| fader_readiness(readiness(&slots), laws.iter().all(|l| l.ready())))
    };
    let takes_input = move || {
        fader_readiness(readiness_now(&slots), laws.iter().all(|l| l.can_map())) == Readiness::Ready
    };
    let live = move || {
        slot.try_with_untracked(Slot::number)
            .flatten()
            .and_then(|v| law.pos(v))
    };
    let send = move |p: f64, is_final: bool| {
        let _ = targets.try_with_value(|all| {
            for t in all {
                if let (Some(spec), Some(value)) = (&t.spec, t.law.value(p)) {
                    store.set(
                        &spec.instance,
                        &spec.target,
                        &spec.prop,
                        json!(value),
                        is_final,
                        Some(fail_flash(failed)),
                    );
                }
            }
        });
    };

    let on_down = move |ev: web_sys::PointerEvent| {
        if !takes_input() {
            return;
        }
        let Some(at) = live() else {
            return;
        };
        let Some(el) = dom::current_element(&ev) else {
            return;
        };
        ev.prevent_default();
        let travel = el.get_bounding_client_rect().height().max(1.0);
        let id = ev.pointer_id();
        let y = f64::from(ev.client_y());
        let taken = ctl
            .try_update_value(|c| c.down(id, y, travel, dom::now(), at))
            .unwrap_or(false);
        if taken {
            let _ = el.set_pointer_capture(id);
            let _ = keys.try_with_value(|k| store.touch(k));
        }
    };
    let on_move = move |ev: web_sys::PointerEvent| {
        let (id, y) = (ev.pointer_id(), f64::from(ev.client_y()));
        let _ = ctl.try_update_value(|c| c.moved(id, y, dom::now()));
    };
    // The end of a touch (`behave::touch_end`): the unsent move as a final
    // `set`, or the release time of the write already sent (L4).
    let ended = move |end: Option<TouchEnd>| match end {
        Some(TouchEnd::Send(p)) => send(p, true),
        Some(TouchEnd::Released) => {
            let _ = keys.try_with_value(|k| store.release(k));
        }
        Some(TouchEnd::NotMine) | None => {}
    };
    let on_up = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        ended(ctl.try_update_value(|c| touch_end(c.drives(id), c.up(id, dom::now()))));
    };
    // A cancelled pointer, or one whose capture was lost without an up:
    // the touch ends without a tap.
    let on_cancel = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        ended(ctl.try_update_value(|c| touch_end(c.drives(id), c.cancel(id, dom::now()))));
    };

    raf::animate(root, move |el| {
        let mut shown: Option<f64> = None;
        let mut look: Option<State> = None;
        let mut ghost: Option<f64> = None;
        Box::new(move |now: f64, _step: f64| {
            let intent = shown_key
                .as_deref()
                .map_or(State::Confirmed, |k| store.intent_state(k));
            let at = live();
            let Some((motion, glide_ended)) = ctl.try_update_value(|c| {
                c.intent(intent.is_open(), now);
                (c.frame(now, at), c.take_ended())
            }) else {
                return;
            };
            if let Some(p) = motion.send {
                send(p, false);
            }
            if glide_ended {
                let _ = keys.try_with_value(|k| store.release(k));
            }
            if look != Some(intent) {
                dom::set_attr(&el, "data-intent", intent.name());
                look = Some(intent);
            }
            let g = at.filter(|_| intent.shows_ghost());
            if g != ghost {
                if let Some(g) = g {
                    dom::set_style(&el, "--g", &format!("{g:.5}"));
                    if let Some(v) = law.value(g) {
                        dom::set_attr(&el, "data-ghost", &format!("{v:.4}"));
                    }
                }
                ghost = g;
            }
            let Some(p) = motion.pos else {
                return;
            };
            if shown.is_some_and(|s| (s - p).abs() < 1e-5) {
                return;
            }
            shown = Some(p);
            dom::set_style(&el, "--p", &format!("{p:.5}"));
            if let Some(v) = law.value(p) {
                dom::set_attr(&el, "data-value", &format!("{v:.4}"));
            }
        })
    });

    let binding = move || state.get().name();
    let disabled = move || state.get().disabled();
    view! {
        <div
            class="fader"
            class:failed=move || failed.get()
            data-testid="fader"
            data-binding=binding
            aria-disabled=disabled
            node_ref=root
            on:pointerdown=on_down
            on:pointermove=on_move
            on:pointerup=on_up
            on:pointercancel=on_cancel
            on:lostpointercapture=on_cancel
        >
            <div class="fader-groove"></div>
            <div class="fader-fill"></div>
            <div class="fader-ghost"></div>
            <div class="fader-rail">
                <div class="fader-cap"></div>
            </div>
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
        assert!(law.can_map());
    }

    #[test]
    fn the_linear_law_waits_for_the_parameters_range() {
        let range = RwSignal::new(None);
        let law = Law::Linear(range);
        assert_eq!(law.pos(0.5), None);
        assert_eq!(law.value(0.5), None);
        assert!(!law.ready());
        assert!(!law.can_map());
        assert_eq!(law.glide_to(), None, "no double tap on a parameter fader");
        let _ = range.try_set(Some((-15.0, 15.0)));
        assert!(law.ready());
        assert!(law.can_map());
        assert_eq!(law.pos(0.0), Some(0.5));
        assert_eq!(law.value(0.25), Some(-7.5));
    }

    #[test]
    fn a_fader_waits_while_a_law_has_no_range_and_an_unresolved_target_stays_red() {
        assert_eq!(fader_readiness(Readiness::Ready, true), Readiness::Ready);
        assert_eq!(fader_readiness(Readiness::Ready, false), Readiness::Waiting);
        assert_eq!(
            fader_readiness(Readiness::Waiting, true),
            Readiness::Waiting
        );
        assert_eq!(
            fader_readiness(Readiness::Unresolved, false),
            Readiness::Unresolved
        );
        assert_eq!(
            fader_readiness(Readiness::Unresolved, true),
            Readiness::Unresolved
        );
    }
}
