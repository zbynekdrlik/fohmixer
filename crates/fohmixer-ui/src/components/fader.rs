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
//! drops a `not_sent` write. Each touch's down, up and cancel goes to the
//! page's flight recorder (#43, PR C): the down with where the touch started
//! (`FaderCtl::press`), and each frame that sends from the finger with the
//! pointer moves it carried (`diag::trace::moves::Trail`, PR D). A primary
//! down of the held pointer's type ends a touch whose end the browser never
//! delivered (#124) as a cancel, recorded `lost` before the new finger's
//! down; a hidden page ends it too.
//!
//! The backing-tracks fader (#58, `vca`): its cap stands at the loudest
//! target, a touch keeps every target's volume at its start and each move
//! writes them by `behave::vca::Drag` (the loudest at the finger, every
//! other one moved by the same dB), and the cap shows the loudest open
//! write. Its double tap (the owner's decision of 2026-10-13) glides every
//! target to 0 dB by `behave::vca::Home`: the release that starts the glide
//! takes what every target shows, stretches the cap's glide from the
//! loudest over the furthest track's way, and each glide frame writes every
//! target that far on its way.
//!
//! A reverb fader (#130, D18) carries REVERB's marks (`ReverbMarks`): while
//! the hub's rule holds it low, the lowered part of its groove, its return
//! level and `AUTO −n`; once the engineer took it, `MAN`.

use fohmixer_proto::client::set_key;
use leptos::html;
use leptos::prelude::*;
use serde_json::json;

use super::{
    down_of, fail_flash, owns_touches, readiness, readiness_now, trace_move, trace_start,
    trace_touch,
};
use crate::behave::fader::{self as curve, FaderCtl, UNITY, VolumeLaw};
use crate::behave::fader_link::{link_drag, link_volumes};
use crate::behave::reverb::{Mark, aut_text, mark_of};
use crate::behave::vca::{self, Drag, Home};
use crate::behave::{Taken, TouchEnd, touch_end};
use crate::binding::SubSpec;
use crate::diag::trace::moves::{Axis, Press, Trail};
use crate::dom;
use crate::raf;
use crate::store::intent::{State, most_urgent};
use crate::store::{LiveStore, Readiness, Slot};

/// How a fader maps its position to Live's value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Law {
    /// A track volume on its volume law (#63: Live's own by default), double
    /// tap to 0 dB.
    Volume(VolumeLaw),
    /// A parameter's range, linear (`cc_linear`), read from Live.
    Linear(RwSignal<Option<(f64, f64)>>),
}

impl Law {
    /// The position of Live's value, when the law can say.
    fn pos(self, value: f64) -> Option<f64> {
        match self {
            Self::Volume(law) => Some(law.to_pos(value)),
            Self::Linear(range) => range
                .try_get_untracked()
                .flatten()
                .map(|r| curve::linear_pos(value, r)),
        }
    }

    /// Live's value at position `p`.
    fn value(self, p: f64) -> Option<f64> {
        match self {
            Self::Volume(law) => Some(law.to_live(p)),
            Self::Linear(range) => range
                .try_get_untracked()
                .flatten()
                .map(|r| curve::linear_value(p, r)),
        }
    }

    /// Where a double tap glides.
    fn glide_to(self) -> Option<f64> {
        match self {
            Self::Volume(law) => Some(law.to_pos(UNITY)),
            Self::Linear(_) => None,
        }
    }

    /// Whether a touch's first pointer move only anchors the drag (#43 PR
    /// F): a strip's volume fader's does, and so does the backing-tracks
    /// fader's (#58: its targets are volumes); the linear parameter faders
    /// keep TouchOSC's first move (the owner's call on #43).
    fn anchors(self) -> bool {
        match self {
            Self::Volume(_) => true,
            Self::Linear(_) => false,
        }
    }

    /// The volume law the touch shaping measures its dB steps on: a volume
    /// fader's own; TouchOSC's for a parameter fader (as the Lua shaped it).
    fn shaping_law(self) -> VolumeLaw {
        match self {
            Self::Volume(law) => law,
            Self::Linear(_) => VolumeLaw::TouchOsc,
        }
    }

    /// A volume fader's volume law (#58: the backing-tracks fader's double
    /// tap moves its tracks on it); none for a parameter fader.
    fn volume_law(self) -> Option<VolumeLaw> {
        match self {
            Self::Volume(law) => Some(law),
            Self::Linear(_) => None,
        }
    }

    /// The volume law a touch's record names (#63): a volume fader's; none
    /// for a parameter fader (its positions are its range's).
    fn traced(self) -> Option<VolumeLaw> {
        self.volume_law()
    }

    /// Whether the law can map (reactive: a parameter's range arrived).
    fn ready(self) -> bool {
        match self {
            Self::Volume(_) => true,
            Self::Linear(range) => range.try_with(Option::is_some).unwrap_or(false),
        }
    }

    /// The same, untracked (in an event handler).
    fn can_map(self) -> bool {
        match self {
            Self::Volume(_) => true,
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

/// The volumes `targets` show (#58): each target's open write while the
/// store holds one, else Live's value (`None` before Live's).
pub fn shown_volumes(store: LiveStore, targets: &[Target]) -> Vec<Option<f64>> {
    targets
        .iter()
        .map(|t| {
            let open = t.spec.as_ref().and_then(|s| {
                store
                    .intent_view(&set_key(&s.instance, &s.target, &s.prop))
                    .1
            });
            open.or_else(|| t.slot.try_with_untracked(Slot::number).flatten())
        })
        .collect()
}

/// A backing-tracks fader's writes as its cap sees them (#58): the most
/// urgent state of its targets' writes (`data-intent`) and the loudest open
/// write's volume (the cap shows it while one is open, §4.2).
fn vca_intent(views: &[(State, Option<f64>)]) -> (State, Option<f64>) {
    let state = most_urgent(views.iter().map(|(state, _)| *state));
    let open: Vec<Option<f64>> = views.iter().map(|(_, value)| *value).collect();
    (state, vca::loudest(&open))
}

/// The REVERB rule's marks on a reverb fader (#130, D18; the approved
/// mockup reverb-link-v2, screen 1, states 3 and 4): while the rule holds
/// it low, the lowered part of its groove, a dashed return mark at its song
/// level (`--r`) and `AUTO −n` (the live difference, `aut_text`); once the
/// engineer took it, `MAN`. Each is hidden unless `.shown`. No touch, no
/// height: they sit in the fader and read its `--p` and `--x`.
/// `volume_key` is the fader's volume's write key (the hub's
/// `ReverbFader.key`), `state` its slot, `law` the strip's volume law.
#[component]
pub fn ReverbMarks(volume_key: String, state: RwSignal<Slot>, law: VolumeLaw) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let volume_key = StoredValue::new(volume_key);
    let mark = Memo::new(move |_| {
        store
            .reverb
            .try_with(|r| volume_key.try_with_value(|k| mark_of(r.shown(), k)))
            .flatten()
            .unwrap_or(Mark::None)
    });
    let back = move || match mark.try_get() {
        Some(Mark::Aut { back }) => Some(back),
        _ => None,
    };
    let at = move || {
        back()
            .map(|b| format!("--r:{:.5};", law.to_pos(b)))
            .unwrap_or_default()
    };
    let shown = move || back().is_some();
    let hand = move || mark.try_get() == Some(Mark::Hand);
    let text = move || {
        back()
            .zip(state.try_with(Slot::number).flatten())
            .map(|(b, live)| aut_text(b, live))
            .unwrap_or_default()
    };
    view! {
        <div class="aut-span" data-testid="aut-span" class:shown=shown style=at></div>
        <div class="aut-ret" data-testid="aut-ret" class:shown=shown style=at></div>
        <div class="fz-mark aut" data-testid="aut-mark" class:shown=shown style=at>
            <span class="fz-pill">{text}</span>
        </div>
        <div class="fz-mark hand" data-testid="hand-mark" class:shown=hand>
            <span class="fz-pill">"MAN"</span>
        </div>
    }
}

/// A vertical fader showing its first target (by that target's law) and
/// writing every target, each by its own law. It takes a touch only while
/// every target has Live's value and a law that can map it (I8), and is
/// red while any target's binding does not resolve (I5). With `vca` (the
/// backing-tracks fader, #58) it shows the loudest target, a touch moves
/// every target by the same dB from where the touch started, and a double
/// tap glides every target to 0 dB. A fader link's leader (#130, D18:
/// `partners`, while `link_on`) writes its partners with its own by the
/// same dB, in one `sets`. A reverb fader (`reverb`, #130) shows REVERB's
/// marks (`ReverbMarks`; never the backing-tracks fader).
#[component]
pub fn FaderView(
    targets: Vec<Target>,
    shaping: bool,
    #[prop(optional)] vca: bool,
    /// A fader link's partners (#130, D18): the volumes a leader's touch
    /// writes with its own by the same dB while `link_on`; empty for any
    /// other fader.
    #[prop(optional)]
    partners: Vec<Target>,
    /// The leader's link is on (its hub switch, and partners to move).
    #[prop(optional, into)]
    link_on: Signal<bool>,
    /// A reverb fader (#130, D18: its strip's `+REVERB`): REVERB's marks.
    #[prop(optional)]
    reverb: bool,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let (slot, law) = targets.first().map_or_else(
        || {
            (
                RwSignal::new(Slot::Error("a fader without a target".into())),
                Law::Volume(VolumeLaw::default()),
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
    // The root owns its touches (#43 PR G) and names its keys.
    let touch_keys = keys.clone();
    let keys = StoredValue::new(keys);
    let shown_key = StoredValue::new(shown_key);
    let targets = StoredValue::new(targets);
    let partners = StoredValue::new(partners);
    // A leader's linked touch (#130): the drag around its own start and the
    // partners it moves; none while unlinked.
    let link = StoredValue::new(None::<(Drag, Vec<usize>)>);
    // The keys this touch writes (its own; a linked one's partners' too).
    let touched = StoredValue::new(touch_keys.clone());
    // A fader taken away under a finger (a page switch) gets no pointerup:
    // the keys its last touch wrote count as released then, never held
    // forever (L4). A partner's key is released only when a linked touch
    // wrote it (the cleanup runs before the owner's stored values go).
    on_cleanup(move || {
        let _ = touched.try_with_value(|k| store.release(k));
    });
    // A backing-tracks fader's touch: every target's volume at its start;
    // and its double tap's glide: from every target's volume at the glide's
    // start to 0 dB.
    let drag = StoredValue::new(None::<Drag>);
    let home = StoredValue::new(None::<Home>);
    let ctl = StoredValue::new(
        FaderCtl::new(shaping, law.glide_to())
            .anchoring(law.anchors())
            .law(law.shaping_law()),
    );
    // The finger's moves between frames, for the flight recorder (#43 PR D).
    let trail = StoredValue::new(Trail::default());
    let failed = RwSignal::new(false);
    let root = NodeRef::<html::Div>::new();

    let state = {
        let (slots, laws) = (slots.clone(), laws.clone());
        Memo::new(move |_| fader_readiness(readiness(&slots), laws.iter().all(|l| l.ready())))
    };
    let takes_input = move || {
        fader_readiness(readiness_now(&slots), laws.iter().all(|l| l.can_map())) == Readiness::Ready
    };
    // Live's value as the cap's position: the first target's, or the
    // loudest target's on a backing-tracks fader.
    let live = move || {
        let value = if vca {
            targets
                .try_with_value(|all| {
                    let values: Vec<Option<f64>> = all
                        .iter()
                        .map(|t| t.slot.try_with_untracked(Slot::number).flatten())
                        .collect();
                    vca::loudest(&values)
                })
                .flatten()
        } else {
            slot.try_with_untracked(Slot::number).flatten()
        };
        value.and_then(|v| law.pos(v))
    };
    // Every target's write of position `p`: the first target's sequence
    // number (the flight recorder's move record names it). A
    // backing-tracks fader's values come from its touch's start (#58) or,
    // on a glide's frame (`glide`: how far it has come), from its double
    // tap's `Home`; any other's from each target's own law. One target is
    // one `set`; several go as ONE `sets` (#130: Live moves them in the
    // same batch).
    let send = move |p: f64, is_final: bool, glide: Option<f64>| -> Option<u64> {
        // A leader's linked touch (#130): its own value and each partner's
        // by the same dB, in ONE `sets` (one batch in Live).
        if !vca && let Some(Some((moved, idx))) = link.try_with_value(Clone::clone) {
            let own = targets
                .try_with_value(|all| {
                    all.first()
                        .and_then(|t| Some((t.spec.clone()?, t.law.value(p)?)))
                })
                .flatten();
            if let Some((spec, value)) = own {
                let mut items = vec![(spec, json!(value))];
                let _ = partners.try_with_value(|all| {
                    for (i, volume) in link_volumes(&moved, &idx, value) {
                        if let Some(spec) = all.get(i).and_then(|t| t.spec.clone()) {
                            items.push((spec, json!(volume)));
                        }
                    }
                });
                return store.set_each(&items, is_final, move || fail_flash(failed));
            }
        }
        let values: Vec<Option<f64>> = if vca {
            let volumes = match glide {
                Some(at) => home
                    .try_with_value(|h| h.as_ref().map(|h| h.volumes(at)))
                    .flatten(),
                None => drag
                    .try_with_value(|d| {
                        d.as_ref()
                            .zip(law.value(p))
                            .map(|(d, finger)| d.volumes(finger))
                    })
                    .flatten(),
            };
            volumes.unwrap_or_default().into_iter().map(Some).collect()
        } else {
            targets
                .try_with_value(|all| {
                    all.iter()
                        .map(|t| t.law.value(p))
                        .collect::<Vec<Option<f64>>>()
                })
                .unwrap_or_default()
        };
        targets
            .try_with_value(|all| {
                let items: Vec<(SubSpec, serde_json::Value)> = all
                    .iter()
                    .zip(values)
                    .filter_map(|(t, value)| Some((t.spec.clone()?, json!(value?))))
                    .collect();
                match items.as_slice() {
                    [] => None,
                    [(spec, value)] => store.set(
                        &spec.instance,
                        &spec.target,
                        &spec.prop,
                        value.clone(),
                        is_final,
                        Some(fail_flash(failed)),
                    ),
                    // The backing-tracks fader (#58, #130): every track in
                    // one batch.
                    _ => store.set_each(&items, is_final, move || fail_flash(failed)),
                }
            })
            .flatten()
    };
    // A send from the finger: its moves since the last one, to the recorder.
    let sent_moves = move |p: f64, seq: Option<u64>| {
        let _ = shown_key.try_with_value(|k| {
            if let Some(key) = k {
                trace_move(trail, key, p, seq);
            }
        });
    };

    // The end of a touch (`behave::touch_end`): the unsent move as a final
    // `set`, or the release time of the write already sent (L4).
    // The flight recorder hears the end of this fader's own touches.
    let ended = move |end: Option<TouchEnd>, what: &str, id: i32| {
        match end {
            Some(TouchEnd::Send(p)) => sent_moves(p, send(p, true, None)),
            Some(TouchEnd::Released) => {
                let _ = touched.try_with_value(|k| store.release(k));
            }
            Some(TouchEnd::NotMine) | None => {}
        }
        if end.is_some_and(TouchEnd::ends_touch) {
            let _ = keys.try_with_value(|k| trace_touch(what, k, id));
            let _ = trail.try_update_value(|t| t.end(id));
        }
    };

    // A hidden page ends a touch the browser has not ended (#124): a hidden
    // Home Screen app keeps no touch going, and this works whatever
    // `isPrimary` the next finger brings. `visibilitychange` bubbles to
    // the window.
    let hidden = window_event_listener_untyped("visibilitychange", move |_| {
        if !dom::hidden() {
            return;
        }
        if let Some(lost) = ctl.try_update_value(|c| c.hidden(dom::now())).flatten() {
            ended(Some(lost.end), "lost", lost.pointer);
        }
    });
    on_cleanup(move || hidden.remove());

    let on_down = move |ev: web_sys::PointerEvent| {
        if !takes_input() {
            return;
        }
        let Some(at) = live() else {
            return;
        };
        // A backing-tracks fader's touch starts from what every target
        // shows (#58); it takes none while a target shows nothing.
        let begun = if vca {
            let shown = targets
                .try_with_value(|all| shown_volumes(store, all))
                .unwrap_or_default();
            let Some(begun) = shown
                .into_iter()
                .collect::<Option<Vec<f64>>>()
                .and_then(Drag::new)
            else {
                return;
            };
            Some(begun)
        } else {
            None
        };
        let Some(el) = dom::current_element(&ev) else {
            return;
        };
        ev.prevent_default();
        let travel = el.get_bounding_client_rect().height().max(1.0);
        let down = down_of(&ev);
        let id = down.id;
        let y = f64::from(ev.client_y());
        let taken = ctl
            .try_update_value(|c| c.press(down, y, travel, dom::now(), at))
            .flatten();
        if let Some(Taken { start, lost }) = taken {
            // A touch whose end the browser never delivered, ended by this
            // primary down (#124): its end first, recorded as `lost`.
            if let Some(lost) = lost {
                ended(Some(lost.end), "lost", lost.pointer);
            }
            let _ = drag.try_set_value(begun);
            // A leader's touch with its link on (#130): its own shown volume
            // and each partner's at the down; none while unlinked.
            let linking = !vca && link_on.try_get_untracked().unwrap_or(false);
            let begun_link = if linking {
                let own = targets
                    .try_with_value(|all| shown_volumes(store, &all[..all.len().min(1)]))
                    .unwrap_or_default();
                let theirs = partners
                    .try_with_value(|p| shown_volumes(store, p))
                    .unwrap_or_default();
                link_drag(own.first().copied().flatten(), &theirs)
            } else {
                None
            };
            let mut writes = keys.try_with_value(Clone::clone).unwrap_or_default();
            if let Some((_, idx)) = &begun_link {
                let _ = partners.try_with_value(|p| {
                    writes.extend(
                        idx.iter()
                            .filter_map(|&i| p.get(i).and_then(|t| t.spec.as_ref()))
                            .map(|s| set_key(&s.instance, &s.target, &s.prop)),
                    );
                });
            }
            let _ = link.try_set_value(begun_link);
            store.touch(&writes);
            let _ = touched.try_set_value(writes);
            let _ = el.set_pointer_capture(id);
            let press = Press {
                at: dom::event_epoch(&ev),
                c: y,
                travel,
            };
            let _ = trail.try_update_value(|t| t.start(id, Axis::Up, press, start.from));
            let _ = keys.try_with_value(|k| trace_start(k, id, press, start, law.traced()));
        }
    };
    let on_move = move |ev: web_sys::PointerEvent| {
        let (id, y) = (ev.pointer_id(), f64::from(ev.client_y()));
        // A move this fader took, with where it left the fader (the touch's
        // first one only anchors it: the recorder's `a`, #43 PR F).
        let moved = ctl
            .try_update_value(|c| c.moved(id, y, dom::now()).then(|| c.pos()))
            .flatten();
        if let Some(pos) = moved {
            let at = dom::event_epoch(&ev);
            let _ = trail.try_update_value(|t| t.moved(id, at, y, pos));
        }
    };
    let on_up = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        let end = ctl.try_update_value(|c| touch_end(c.drives(id), c.up(id, dom::now())));
        ended(end, "up", id);
        // A backing-tracks fader's double tap (#58): the glide this touch's
        // release started (a press stops any glide, so one running now is
        // this release's) takes every target from what it shows now (the
        // tap's last write included) to 0 dB, the cap from the loudest over
        // the furthest track's way; with a target that shows nothing there
        // is nothing to glide from, and the glide is dropped (both do
        // nothing without a glide). Another pointer's up touches no glide.
        if vca && end.is_some_and(TouchEnd::ends_touch) {
            let begun = law.volume_law().and_then(|l| {
                targets
                    .try_with_value(|all| Home::new(l, &shown_volumes(store, all)))
                    .flatten()
            });
            let _ = ctl.try_update_value(|c| match &begun {
                Some(h) => c.stretch_glide(h.cap_from(), h.span()),
                None => c.drop_glide(),
            });
            let _ = home.try_set_value(begun);
        }
    };
    // A cancelled pointer, or one whose capture was lost without an up:
    // the touch ends without a tap.
    let on_cancel = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        let end = ctl.try_update_value(|c| touch_end(c.drives(id), c.cancel(id, dom::now())));
        ended(end, "cancel", id);
    };

    raf::animate(root, move |el| {
        let mut shown: Option<f64> = None;
        let mut look: Option<State> = None;
        let mut ghost: Option<f64> = None;
        let mut held: Option<bool> = None;
        Box::new(move |now: f64, _step: f64| {
            // The shown key's write, or a backing-tracks fader's loudest.
            let view = if vca {
                keys.try_with_value(|k| {
                    let views: Vec<(State, Option<f64>)> =
                        k.iter().map(|k| store.intent_view(k)).collect();
                    vca_intent(&views)
                })
            } else {
                shown_key
                    .try_with_value(|k| k.as_deref().map(|k| store.intent_view(k)))
                    .flatten()
            };
            let (intent, written) = view.unwrap_or((State::Confirmed, None));
            // The open write's position: the cap shows it (§4.2).
            let written = written
                .filter(|_| intent.is_open())
                .and_then(|v| law.pos(v));
            let at = live();
            let Some((motion, glide_ended, is_held, glide)) = ctl.try_update_value(|c| {
                c.intent(intent.is_open(), now);
                if let Some(p) = written {
                    c.write_at(p);
                }
                // How far a glide has come at this frame (its send's).
                let glide = c.glide_progress(now);
                let motion = c.frame(now, at);
                (motion, c.take_ended(), c.held(), glide)
            }) else {
                return;
            };
            // A finger or a glide drives it: the strip lights (#63,
            // `strip.css`), above the finger.
            if held != Some(is_held) {
                dom::set_attr(&el, "data-held", if is_held { "true" } else { "false" });
                held = Some(is_held);
            }
            if let Some(p) = motion.send {
                sent_moves(p, send(p, false, glide));
            }
            if glide_ended {
                let _ = touched.try_with_value(|k| store.release(k));
            }
            if look != Some(intent) {
                dom::set_attr(&el, "data-intent", intent.name());
                look = Some(intent);
            }
            let g = at.filter(|_| intent.shows_ghost());
            if g != ghost {
                match g.and_then(|g| law.value(g).map(|v| (g, v))) {
                    Some((g, v)) => {
                        dom::set_style(&el, "--g", &format!("{g:.5}"));
                        dom::set_attr(&el, "data-ghost", &format!("{v:.4}"));
                    }
                    None => dom::remove_attr(&el, "data-ghost"),
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

    let binding = move || state.try_get().map_or("waiting", |s| s.name());
    let disabled = move || state.try_get().map_or("true", |s| s.disabled());
    // REVERB's marks on a reverb strip's volume fader (#130): its first
    // target's key, slot and volume law.
    let marks = (reverb && !vca)
        .then(|| shown_key.try_get_value().flatten().zip(law.volume_law()))
        .flatten()
        .map(|(volume_key, volume_law)| {
            view! { <ReverbMarks volume_key=volume_key state=slot law=volume_law /> }
        });
    view! {
        <div
            class="fader"
            class:failed=move || failed.try_get().unwrap_or(false)
            data-testid="fader"
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
            <div class="fader-groove"></div>
            <div class="fader-fill"></div>
            <div class="fader-ghost"></div>
            {marks}
            <div class="fader-rail">
                <div class="fader-cap" data-testid="fader-cap"></div>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_volume_law_is_the_fader_curve_with_a_glide_to_0_db() {
        let law = Law::Volume(VolumeLaw::TouchOsc);
        assert_eq!(law.pos(0.85), Some(curve::to_pos(0.85)));
        assert_eq!(law.value(0.5), Some(curve::to_live(0.5)));
        assert_eq!(law.glide_to(), Some(curve::to_pos(UNITY)));
        assert_eq!(law.shaping_law(), VolumeLaw::TouchOsc);
        assert!(
            law.anchors(),
            "a touch's first move only anchors (#43 PR F)"
        );
        assert!(law.ready());
        assert!(law.can_map());
        // Live's own law (#63): the position is Live's volume.
        let live = Law::Volume(VolumeLaw::Live);
        assert_eq!(live.pos(0.6), Some(0.6));
        assert_eq!(live.value(0.3), Some(0.3));
        assert_eq!(live.glide_to(), Some(UNITY));
        assert_eq!(live.shaping_law(), VolumeLaw::Live);
        assert_eq!(live.traced(), Some(VolumeLaw::Live));
        assert_eq!(law.traced(), Some(VolumeLaw::TouchOsc));
        // The backing-tracks fader's double tap moves its tracks on it (#58).
        assert_eq!(live.volume_law(), Some(VolumeLaw::Live));
        assert_eq!(law.volume_law(), Some(VolumeLaw::TouchOsc));
        assert!(live.anchors() && live.ready() && live.can_map());
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
        assert_eq!(
            law.shaping_law(),
            VolumeLaw::TouchOsc,
            "shaped as the Lua did"
        );
        assert_eq!(law.traced(), None, "its positions are its range's");
        assert_eq!(
            law.volume_law(),
            None,
            "a parameter's range is no volume law"
        );
        assert!(!law.anchors(), "TouchOSC's first move (#43 PR F)");
        let _ = range.try_set(Some((-15.0, 15.0)));
        assert!(law.ready());
        assert!(law.can_map());
        assert_eq!(law.pos(0.0), Some(0.5));
        assert_eq!(law.value(0.25), Some(-7.5));
    }

    #[test]
    fn a_backing_tracks_faders_cap_shows_its_loudest_open_write_and_most_urgent_state() {
        assert_eq!(
            vca_intent(&[
                (State::Confirmed, None),
                (State::Sending, Some(0.4)),
                (State::Unconfirmed, Some(0.6)),
            ]),
            (State::Unconfirmed, Some(0.6))
        );
        assert_eq!(
            vca_intent(&[(State::NotSent, Some(0.3)), (State::Sending, Some(0.2))]),
            (State::NotSent, Some(0.3))
        );
        assert_eq!(
            vca_intent(&[(State::Confirmed, None), (State::Confirmed, None)]),
            (State::Confirmed, None)
        );
        assert_eq!(vca_intent(&[]), (State::Confirmed, None));
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
