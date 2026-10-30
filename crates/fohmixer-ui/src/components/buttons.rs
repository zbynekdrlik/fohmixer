//! The toggle buttons (spec F12, F14, F15): the strip mute with its
//! protection (a guarded mute acts by a 1 s hold with a fill, #162), the
//! group solos and the stage-mic button without STAGE AUT
//! (with it, the stage button is `components::rail::RuleButton`, D18).
//! Every button shows Live's value, never a local memory (spec P2),
//! and takes a tap only once that value is here (I8). A write that Live has
//! not confirmed outlines the button (`data-intent`, #43 PR C:
//! `components::intent_look`), and each tap goes to the page's flight
//! recorder.

use std::time::Duration;

use fohmixer_proto::colors::ColorKey;
use fohmixer_proto::layout::{Anchor, Binding};
use leptos::html;
use leptos::prelude::*;
use serde_json::json;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::{
    BtnText, down_of, fail_flash, intent_look, key_of, owns_touches, readiness, slot_of,
    trace_rail, trace_touch,
};
use crate::behave::colour::colour_style;
use crate::behave::fader_link::Flink;
use crate::behave::hold::{HINT_MS, Hold, Lift};
use crate::behave::label::{label_chars, strip_label};
use crate::behave::marks::marks_look;
use crate::behave::mute::{Linked, Reach, chain_compact, hold_line, mute_fill, mute_press};
use crate::behave::rail::{
    Fill, RAIL_HOLD_MS, cancels, full_step, hold_step, restarted, stale_press,
};
use crate::binding::{SubSpec, mute_sub, solo_sub};
use crate::diag::perf::Pointer;
use crate::dom;
use crate::store::{LiveStore, Slot};

/// A solo button's text: `SOLO` and its label, or the first word of its
/// group track's name.
fn solo_text(label: Option<&str>, binding: &Binding) -> String {
    let name = label.map_or_else(|| strip_label(&anchor_name(binding)), str::to_string);
    format!("SOLO {name}")
}

/// The name a binding's anchor shows.
pub fn anchor_name(binding: &Binding) -> String {
    match &binding.anchor {
        Anchor::Track { name } | Anchor::Return { name } => name.clone(),
        // A marker strip's track by index (#68): Live's track number.
        Anchor::TrackAt { index } | Anchor::ReturnAt { index } => format!("#{}", index + 1),
        Anchor::Master => "Master".to_string(),
        Anchor::Song => "Song".to_string(),
        // A control waiting for its Tuner's `+CTRL` (#104).
        Anchor::Ctrl { id } => id.name().to_string(),
    }
}

/// Writes the inverse of a flag slot's value (a tap: a final `set`, #43).
/// A tap inverts what the button shows, Live's value (P2), also while a
/// write is still on its way: nothing on the button shows that write, so a
/// second tap means "it did not take", not "undo".
pub(super) fn toggle_flag(
    store: LiveStore,
    spec: &SubSpec,
    slot: RwSignal<Slot>,
    failed: RwSignal<bool>,
) {
    let Some(current) = slot.try_with_untracked(Slot::flag).flatten() else {
        return;
    };
    store.set(
        &spec.instance,
        &spec.target,
        &spec.prop,
        json!(!current),
        true,
        Some(fail_flash(failed)),
    );
}

/// A guarded mute's page-wide `pointerdown` listener (capture phase, #124:
/// a press whose end never came ends at the next primary down of its
/// type), kept to remove it in the cleanup.
type DownListener = Closure<dyn FnMut(web_sys::PointerEvent)>;

/// The flight recorder's `button` of a guarded mute's `rail` steps (#162).
const MUTE_BUTTON: &str = "mute";

/// Why a guarded mute's hold did nothing (the failure flash's words).
const MUTE_REFUSAL: &str = "the mute cannot be switched now";

/// A linked mute's partner (#127): another track's mute, its subscription
/// and its slot.
#[derive(Clone)]
pub struct Partner {
    pub spec: SubSpec,
    pub slot: RwSignal<Slot>,
}

/// What a mute's tracks (`slots`: the strip's own first, then its
/// partners') show together and which of them a tap writes (#127: the
/// strip's own and every partner with Live's value), tracked or not.
fn linked_now(slots: &[RwSignal<Slot>], tracked: bool) -> Reach {
    let muted: Vec<Option<bool>> = slots
        .iter()
        .map(|s| {
            if tracked {
                s.try_with(Slot::flag).flatten()
            } else {
                s.try_with_untracked(Slot::flag).flatten()
            }
        })
        .collect();
    Reach::of(&muted)
}

/// The linked mute's mark (#127, the approved mockup paired-mute-v3,
/// variant A): a chain and the number of mutes a tap writes (`slots`: the
/// strip's own and each partner's; a partner without Live's value is not
/// counted), grey while all play, red while all are muted, amber `m/N`
/// while m of them are muted by hand in Live. It takes no touch and no
/// height. `bare`: the chain without its count (#151: a muted strip's
/// marks line, where `MUTED` says it).
#[component]
pub fn LinkMark(
    slots: Vec<RwSignal<Slot>>,
    #[prop(optional, into)] bare: Signal<bool>,
) -> impl IntoView {
    let count = slots.len();
    let slots = StoredValue::new(slots);
    let reach = Memo::new(move |_| {
        slots
            .try_with_value(|s| linked_now(s, true))
            .unwrap_or_else(|| Reach::of(&[]))
    });
    let look = move || reach.try_with(|r| r.state.look()).unwrap_or("unknown");
    let text = move || {
        reach
            .try_with(|r| r.text(count))
            .unwrap_or_else(|| count.to_string())
    };
    // A long count is drawn compact, so it fits the strip's marks line
    // beside a problem's `!` (#127 re-review N1).
    let compact = move || chain_compact(&text());
    let is_bare = move || bare.try_get().unwrap_or(false);
    view! {
        <span
            class="link-mark"
            class:compact=compact
            class:bare=is_bare
            data-testid="mute-link"
            data-state=look
            aria-hidden="true"
        >
            <span class="link-n" data-testid="mute-link-n">
                {text}
            </span>
        </span>
    }
}

/// What a strip's name button shows on its marks line besides the chain,
/// the fader link and `MUTED` (#151, mockup v8; strip.css `.strip-marks`):
/// a problem's `!`. The channel detail's MUTE has no marks line: its chain
/// is a pill on the button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Corner {
    /// A marker problem's `!` (#68).
    pub flag: bool,
}

/// A strip's name button, its mute (#151, mockup v8: calm, the strip's top
/// edge, the name first and the marks line under it; the strip's colour
/// chosen on the tablet, #164, only as its 3 px edge, `color`; the detail's
/// MUTE has none). A guarded
/// strip's mute acts by a hold (#162, `behave::mute::mute_press`, as the
/// rail buttons' `Hold::lifted(RAIL_HOLD_MS)`): the fill grows across the
/// button for 1 s, the line then says what the lift does (on a strip in the
/// marks line's place, on the detail's MUTE in the label's) and the lift
/// writes; a lift before full shows the `hold` hint for `HINT_MS` and
/// writes nothing, and a slide past 10 px, a cancel, a lost capture, a
/// hidden page or the next primary down of its type elsewhere (#124) end the
/// press with nothing written. Each step goes to the flight recorder as a
/// `rail` step of the button `mute`. An unguarded mute acts on its down,
/// one tap. A linked mute (#127, `partners`) shows and writes
/// its partners' mutes with its own: lit while any of them plays, dark only
/// when all are muted; a tap mutes all, or unmutes all when all are muted,
/// in one `sets` message. It takes a tap once the strip's own track has
/// Live's value: a partner without one is left out of the tap and of what
/// the button shows, and never disables it (the PA mute always works;
/// `behave::mute::Reach`). `corner`: a strip's marks line, with the chain;
/// none for the channel detail's MUTE, which carries the chain on the
/// button.
#[component]
pub fn MuteView(
    state: RwSignal<Slot>,
    spec: SubSpec,
    guarded: bool,
    label: String,
    color: Option<Signal<Option<ColorKey>>>,
    partners: Vec<Partner>,
    corner: Option<Corner>,
    /// The fader link's mark on the marks line (#130): a leader's or a
    /// partner's; none on the detail's MUTE.
    #[prop(optional, into)]
    flink: Signal<Flink>,
    /// A loudness point's problem (#151, `no LUFS meter`): the marks
    /// line's `!` too, as a marker problem's.
    #[prop(optional, into)]
    lufs_flag: Signal<bool>,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let slot = state;
    let linked = !partners.is_empty();
    let all: Vec<RwSignal<Slot>> = std::iter::once(slot)
        .chain(partners.iter().map(|p| p.slot))
        .collect();
    let specs: Vec<SubSpec> = std::iter::once(spec.clone())
        .chain(partners.iter().map(|p| p.spec.clone()))
        .collect();
    let keys: Vec<String> = specs.iter().map(key_of).collect();
    // What the button shows, every track's mute together.
    let shown_slots = all.clone();
    let shown = Memo::new(move |_| linked_now(&shown_slots, true).state);
    // The strip's marks line (#151, mockup v8), under the name: a problem's
    // `!`, the chain, the fader link's mark and `MUTED` at its right end;
    // what gives way when it fills is `behave::marks::marks_look`, from
    // what the strip carries (nothing is measured). The detail's MUTE has
    // its mark and pill on the button instead.
    let line_slots = all.clone();
    let line = corner.map(move |c| {
        let flagged = move || c.flag || lufs_flag.try_get().unwrap_or(false);
        let looks = Memo::new(move |_| {
            let fader = flink.try_get().unwrap_or_default().shown();
            let muted = shown.try_get() == Some(Linked::Muted);
            marks_look(flagged(), linked, fader, muted)
        });
        let chain_bare = Signal::derive(move || looks.try_get().is_some_and(|l| l.chain_bare));
        let flink_bare = move || looks.try_get().is_some_and(|l| l.flink_bare);
        let flink_yield = move || looks.try_get().is_some_and(|l| l.flink_yield);
        let crowd = move || looks.try_get().is_some_and(|l| l.crowd);
        let flag = move || {
            flagged().then(|| {
                view! { <span class="strip-flag" data-testid="strip-flag" aria-hidden="true">"!"</span> }
            })
        };
        let chain = linked.then(move || view! { <LinkMark slots=line_slots bare=chain_bare /> });
        let fader_mark = move || {
            let role = flink.try_get().unwrap_or_default();
            role.shown().then(|| {
                let name = role.name();
                let follow = role.follows();
                // One class list (`yield` is a Rust keyword, so no
                // `class:yield`).
                let classes = move || {
                    let mut list = String::from("link-mark flink");
                    for (on, class) in [
                        (follow, " follow"),
                        (flink_bare(), " bare"),
                        (flink_yield(), " yield"),
                    ] {
                        if on {
                            list.push_str(class);
                        }
                    }
                    list
                };
                view! {
                    <span
                        class=classes
                        data-testid="fader-link-mark"
                        data-role=name
                        aria-hidden="true"
                    >
                        <span class="link-n">"dB"</span>
                    </span>
                }
            })
        };
        view! {
            <span class="strip-marks" class:crowd=crowd data-testid="strip-marks">
                {flag}
                {chain}
                {fader_mark}
                <span class="mute-mark" data-testid="mute-mark" aria-hidden="true">"MUTED"</span>
            </span>
        }
    });
    let on_button = corner
        .is_none()
        .then(|| view! { <span class="mute-mark" aria-hidden="true">"MUTE"</span> });
    let pill_slots = all.clone();
    let pill = (corner.is_none() && linked).then(move || view! { <LinkMark slots=pill_slots /> });
    let root = NodeRef::<html::Div>::new();
    intent_look(root, store, keys.clone());
    // The root owns its touches (#43 PR G) and names its keys.
    let touch_keys = keys.clone();
    let keys = StoredValue::new(keys);
    let spec = StoredValue::new(spec);
    let specs = StoredValue::new(specs);
    let slots = StoredValue::new(all);
    let failed = RwSignal::new(false);
    // The write a tap or a full hold's lift makes, from what the button
    // shows now (P2).
    let write = move || {
        if linked {
            // Over the tracks with Live's value: all muted unmutes them,
            // else they are muted.
            let reach = slots.try_with_value(|s| linked_now(s, false));
            if let Some(reach) = reach
                && let Some(muted) = reach.state.tap()
            {
                let _ = specs.try_with_value(|s| {
                    let targets: Vec<SubSpec> = reach
                        .targets
                        .iter()
                        .filter_map(|&i| s.get(i).cloned())
                        .collect();
                    store.set_together(&targets, &json!(muted), true, move || fail_flash(failed));
                });
            }
        } else {
            let _ = spec.try_with_value(|s| toggle_flag(store, s, slot, failed));
        }
    };
    // A guarded mute's press (#162): its pointer's type (#124), its one
    // check, the hint's timer, and its state as shown.
    let hold = StoredValue::new(mute_press());
    let press_kind = StoredValue::new(None::<Pointer>);
    let check = StoredValue::new(None::<TimeoutHandle>);
    let hint_timer = StoredValue::new(None::<TimeoutHandle>);
    let holding = RwSignal::new(false);
    let full = RwSignal::new(false);
    let hint = RwSignal::new(false);
    let any_down = StoredValue::new_local(None::<DownListener>);
    // The timers and the page's listener go with the button (the cleanup
    // writes no signal of its own).
    on_cleanup(move || {
        for timer in [check, hint_timer] {
            if let Some(Some(handle)) = timer.try_update_value(Option::take) {
                handle.clear();
            }
        }
        if let (Some(Some(listener)), Some(window)) =
            (any_down.try_update_value(Option::take), web_sys::window())
        {
            let _ = window.remove_event_listener_with_callback_and_bool(
                "pointerdown",
                listener.as_ref().unchecked_ref(),
                true,
            );
        }
    });
    // The mute a full hold's lift writes now (none: the strip's own track
    // has no value, the hold is refused).
    let to_now = move || {
        slots
            .try_with_value(|s| linked_now(s, false).state.tap())
            .flatten()
    };
    let fill = Memo::new(move |_| {
        mute_fill(
            holding.try_get().unwrap_or(false),
            full.try_get().unwrap_or(false),
            shown.try_get().and_then(Linked::tap),
        )
    });
    let said = Memo::new(move |_| {
        hold_line(
            fill.try_get().unwrap_or(Fill::None),
            shown.try_get().and_then(Linked::tap),
            hint.try_get().unwrap_or(false),
        )
    });
    let step = move |what: &str, pointer: i32| {
        let _ = keys.try_with_value(|k| trace_rail(what, MUTE_BUTTON, k, pointer));
    };
    let refuse = move || fail_flash(failed)(MUTE_REFUSAL.to_string());
    // The press's state follows it; a press no longer filling waits for no
    // check.
    let settle = move || {
        let filling = hold.try_with_value(Hold::holding).unwrap_or(false);
        let is_full = hold.try_with_value(Hold::full).unwrap_or(false);
        if !filling && let Some(Some(handle)) = check.try_update_value(Option::take) {
            handle.clear();
        }
        let _ = holding.try_set(filling);
        let _ = full.try_set(is_full);
    };
    let hide_hint = move || {
        if let Some(Some(handle)) = hint_timer.try_update_value(Option::take) {
            handle.clear();
        }
        let _ = hint.try_set(false);
    };
    // A lift before full: the hint, for HINT_MS.
    let show_hint = move || {
        hide_hint();
        let _ = hint.try_set(true);
        let timer = set_timeout_with_handle(
            move || {
                let _ = hint_timer.try_set_value(None);
                let _ = hint.try_set(false);
            },
            Duration::from_millis(HINT_MS as u64),
        );
        let _ = hint_timer.try_set_value(timer.ok());
    };
    // The lift of a full press: the write when it can act, else refused.
    let held = move |pointer: i32| {
        let value = to_now();
        step(hold_step(value), pointer);
        match value {
            Some(_) => write(),
            None => refuse(),
        }
    };
    if guarded {
        // A press whose end never came ends at the next primary down
        // anywhere on the page, before any control's handler sees it
        // (#124).
        let listener = Closure::wrap(Box::new(move |ev: web_sys::PointerEvent| {
            let pressing = hold.try_with_value(Hold::pressing).flatten();
            let kind = press_kind.try_get_value().flatten();
            if let Some(old) = stale_press(pressing, kind, down_of(&ev)) {
                let _ = hold.try_update_value(Hold::leave);
                step("lost", old);
                settle();
            }
        }) as Box<dyn FnMut(web_sys::PointerEvent)>);
        if let Some(window) = web_sys::window() {
            let _ = window.add_event_listener_with_callback_and_bool(
                "pointerdown",
                listener.as_ref().unchecked_ref(),
                true,
            );
        }
        let _ = any_down.try_set_value(Some(listener));
        // A hidden page ends a press with nothing done.
        let hidden = window_event_listener_untyped("visibilitychange", move |_| {
            if dom::hidden()
                && let Some(Some(pointer)) = hold.try_update_value(Hold::leave)
            {
                step("lost", pointer);
                settle();
            }
        });
        on_cleanup(move || hidden.remove());
    }
    let on_down = move |ev: web_sys::PointerEvent| {
        if !slot.try_with_untracked(Slot::is_ready).unwrap_or(false) {
            return;
        }
        ev.prevent_default();
        if !guarded {
            let _ = keys.try_with_value(|k| trace_touch("tap", k, ev.pointer_id()));
            write();
            return;
        }
        let id = ev.pointer_id();
        let before = hold.try_with_value(Hold::pressing).flatten();
        let pressed = hold
            .try_update_value(|h| h.down(id, ev.is_primary(), dom::now()))
            .unwrap_or(false);
        if !pressed {
            return;
        }
        // A down of another type took over a press still down.
        if let Some(old) = restarted(before, id) {
            step("lost", old);
        }
        let _ = press_kind.try_set_value(Pointer::parse(&ev.pointer_type()));
        step("press", id);
        hide_hint();
        if let Some(el) = dom::current_element(&ev) {
            let _ = el.set_pointer_capture(id);
        }
        if let Some(Some(handle)) = check.try_update_value(Option::take) {
            handle.clear();
        }
        // One check a little after the hold: the press is full, nothing is
        // written; one that cannot act is refused and dead.
        let timer = set_timeout_with_handle(
            move || {
                let _ = check.try_set_value(None);
                let fills = hold
                    .try_update_value(|h| h.open(dom::now()))
                    .unwrap_or(false);
                if fills {
                    let value = to_now();
                    step(full_step(value), id);
                    if value.is_none() {
                        let _ = hold.try_update_value(Hold::refuse);
                        refuse();
                    }
                }
                settle();
            },
            Duration::from_millis(RAIL_HOLD_MS as u64 + 20),
        );
        let _ = check.try_set_value(timer.ok());
        settle();
    };
    // A finger that slides off the press ends it (`Hold::moved`).
    let on_move = move |ev: web_sys::PointerEvent| {
        if !guarded {
            return;
        }
        let id = ev.pointer_id();
        let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
        let ended = hold
            .try_update_value(|h| h.moved(id, x, y))
            .unwrap_or(false);
        if ended {
            step("slid", id);
            settle();
        }
    };
    let on_up = move |ev: web_sys::PointerEvent| {
        if !guarded {
            return;
        }
        let id = ev.pointer_id();
        let lift = hold
            .try_update_value(|h| h.up(id, dom::now()))
            .unwrap_or(Lift::Nothing);
        settle();
        match lift {
            Lift::Tap => {
                step("hint", id);
                show_hint();
            }
            Lift::Open => held(id),
            Lift::Nothing => {}
        }
    };
    // A cancelled pointer, or one whose capture was lost: nothing done.
    let on_cancel = move |ev: web_sys::PointerEvent| {
        if !guarded {
            return;
        }
        let id = ev.pointer_id();
        if cancels(hold.try_with_value(Hold::pressing).flatten(), id) {
            step("cancel", id);
        }
        let _ = hold.try_update_value(|h| h.cancel(id));
        settle();
    };
    let now = move || shown.try_get().unwrap_or(Linked::Unknown);
    // Its own track's readiness alone (#127 review M2).
    let state = Memo::new(move |_| readiness(&[slot]));
    let binding = move || state.try_get().map_or("waiting", |s| s.name());
    let disabled = move || state.try_get().map_or("true", |s| s.disabled());
    // A guarded mute's fill takes the hold's time (strip.css reads it).
    let hold_time = if guarded {
        format!("--hold-ms:{RAIL_HOLD_MS}ms;")
    } else {
        String::new()
    };
    let look = move || {
        let colour = color
            .map(|c| colour_style(c.try_get().flatten()))
            .unwrap_or_default();
        format!("{colour}{hold_time}")
    };
    let length = format!("--n:{};", label_chars(&label));
    let filling = move || guarded.then(|| fill.try_get().is_some_and(Fill::holding).to_string());
    let is_full = move || guarded.then(|| fill.try_get().is_some_and(Fill::full).to_string());
    let says = move || said.try_get().flatten().is_some().then_some("true");
    // The hold's line is in the page only while it says something (no
    // hidden text in a button).
    let said_view = move || {
        said.try_get().flatten().map(|text| {
            view! {
                <span class="mute-hold" data-testid="mute-hold" aria-hidden="true">
                    {text}
                </span>
            }
        })
    };
    let fill_layer = guarded.then(|| view! { <span class="hold" aria-hidden="true"></span> });
    view! {
        <div
            class="mute"
            node_ref=root
            use:owns_touches=touch_keys
            class:lit=move || now().lit()
            class:failed=move || failed.try_get().unwrap_or(false)
            data-testid="mute"
            data-muted=move || now().muted_attr()
            data-guarded=guarded.to_string()
            data-holding=filling
            data-full=is_full
            data-says=says
            data-binding=binding
            aria-disabled=disabled
            style=look
            on:pointerdown=on_down
            on:pointermove=on_move
            on:pointerup=on_up
            on:pointercancel=on_cancel
            on:lostpointercapture=on_cancel
        >
            <span class="strip-label" data-testid="strip-label" style=length>{label}</span>
            {on_button}
            {pill}
            {said_view}
            {line}
            {fill_layer}
        </div>
    }
}

/// A group-track solo (spec F14): independent, lit when on.
#[component]
pub fn SoloView(binding: Binding, label: Option<String>) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let label = solo_text(label.as_deref(), &binding);
    let track = anchor_name(&binding);
    let spec = solo_sub(&binding);
    let slot = slot_of(store, spec.as_ref());
    let keys: Vec<String> = spec.iter().map(key_of).collect();
    let root = NodeRef::<html::Div>::new();
    intent_look(root, store, keys.clone());
    // The root owns its touches (#43 PR G) and names its keys.
    let touch_keys = keys.clone();
    let keys = StoredValue::new(keys);
    let spec = StoredValue::new(spec);
    let failed = RwSignal::new(false);
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = keys.try_with_value(|k| trace_touch("tap", k, ev.pointer_id()));
        let _ = spec.try_with_value(|s| {
            if let Some(s) = s {
                toggle_flag(store, s, slot, failed);
            }
        });
    };
    let on = move || slot.try_with(Slot::flag).flatten() == Some(true);
    let state = Memo::new(move |_| readiness(&[slot]));
    let bound = move || state.try_get().map_or("waiting", |s| s.name());
    let disabled = move || state.try_get().map_or("true", |s| s.disabled());
    view! {
        <div
            class="btn solo"
            node_ref=root
            use:owns_touches=touch_keys
            class:on=on
            class:failed=move || failed.try_get().unwrap_or(false)
            data-testid="solo"
            data-track=track
            data-on=move || on().to_string()
            data-binding=bound
            aria-disabled=disabled
            on:pointerdown=on_down
        >
            <BtnText text=label />
        </div>
    }
}

/// The stage-mic button (spec F15): the mute of the stage-mic track, lit
/// while Live reports the mics live (`mute` false), like TouchOSC's button
/// (mute false -> x 1) and every strip's mute (F12, lit when audible). An
/// unknown or unmapped state stays dark: a lit STAGE means "the stage is open"
/// (#9, parity audit #21).
#[component]
pub fn StageMicsView(binding: Binding, label: Option<String>) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let label = label.unwrap_or_else(|| "STAGE".to_string());
    let spec = mute_sub(&binding);
    let slot = slot_of(store, spec.as_ref());
    let keys: Vec<String> = spec.iter().map(key_of).collect();
    let root = NodeRef::<html::Div>::new();
    intent_look(root, store, keys.clone());
    // The root owns its touches (#43 PR G) and names its keys.
    let touch_keys = keys.clone();
    let keys = StoredValue::new(keys);
    let spec = StoredValue::new(spec);
    let failed = RwSignal::new(false);
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = keys.try_with_value(|k| trace_touch("tap", k, ev.pointer_id()));
        let _ = spec.try_with_value(|s| {
            if let Some(s) = s {
                toggle_flag(store, s, slot, failed);
            }
        });
    };
    let muted = move || slot.try_with(Slot::flag).flatten() == Some(true);
    let live = move || slot.try_with(Slot::flag).flatten() == Some(false);
    let state = Memo::new(move |_| readiness(&[slot]));
    let bound = move || state.try_get().map_or("waiting", |s| s.name());
    let disabled = move || state.try_get().map_or("true", |s| s.disabled());
    view! {
        <div
            class="btn stage-mics"
            node_ref=root
            use:owns_touches=touch_keys
            class:on=live
            class:failed=move || failed.try_get().unwrap_or(false)
            data-testid="stage-mics"
            data-muted=move || muted().to_string()
            data-binding=bound
            aria-disabled=disabled
            on:pointerdown=on_down
        >
            <BtnText text=label />
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::layout::CtrlId;

    fn binding(anchor: Anchor) -> Binding {
        Binding {
            instance: "band".into(),
            anchor,
            path: None,
        }
    }

    #[test]
    fn an_anchor_names_its_track() {
        assert_eq!(
            anchor_name(&binding(Anchor::Track {
                name: "Hand1 #".into()
            })),
            "Hand1 #"
        );
        assert_eq!(
            anchor_name(&binding(Anchor::Return {
                name: "A-Reverb #".into()
            })),
            "A-Reverb #"
        );
        assert_eq!(anchor_name(&binding(Anchor::TrackAt { index: 0 })), "#1");
        assert_eq!(anchor_name(&binding(Anchor::ReturnAt { index: 11 })), "#12");
        assert_eq!(anchor_name(&binding(Anchor::Master)), "Master");
        assert_eq!(anchor_name(&binding(Anchor::Song)), "Song");
        assert_eq!(
            anchor_name(&binding(Anchor::Ctrl {
                id: CtrlId::Talkback
            })),
            "talkback"
        );
    }

    #[test]
    fn a_linked_mute_reads_every_tracks_mute_tracked_or_not() {
        // #127: the strip's own first, then each partner's.
        let mute = |on: Option<bool>| {
            RwSignal::new(on.map_or(Slot::Pending, |m| Slot::Value {
                value: serde_json::json!(m),
                display: None,
                at: 0.0,
            }))
        };
        for tracked in [true, false] {
            let own = mute(Some(false));
            let partner = mute(Some(true));
            let other = mute(Some(false));
            assert_eq!(
                linked_now(&[own, partner, other], tracked).state,
                Linked::Mixed { muted: 1 }
            );
            // #127 review M2: a partner without a value is left out.
            let waiting = mute(None);
            assert_eq!(
                linked_now(&[own, waiting, partner], tracked),
                Reach {
                    targets: vec![0, 2],
                    state: Linked::Mixed { muted: 1 }
                }
            );
            let _ = own.try_set(Slot::Pending);
            assert_eq!(linked_now(&[own, partner], tracked).state, Linked::Unknown);
            assert_eq!(linked_now(&[partner], tracked).state, Linked::Muted);
            assert_eq!(linked_now(&[other], tracked).state, Linked::Playing);
            assert_eq!(linked_now(&[mute(None)], tracked).state, Linked::Unknown);
        }
    }

    #[test]
    fn a_solo_says_solo_and_its_label_or_its_groups_first_word() {
        let group = binding(Anchor::Track {
            name: "Stems grp#".into(),
        });
        assert_eq!(solo_text(None, &group), "SOLO Stems");
        assert_eq!(solo_text(Some("Podklady"), &group), "SOLO Podklady");
    }

    #[test]
    fn a_control_waiting_for_its_tuner_subscribes_nothing() {
        // No target: `slot_of` makes it unresolved, so it shows red (#104).
        let waiting = binding(Anchor::Ctrl { id: CtrlId::Stage });
        assert_eq!(crate::binding::mute_sub(&waiting), None);
    }

    #[test]
    fn a_button_subscribes_its_anchors_property() {
        let solo = binding(Anchor::Track {
            name: "Stems grp#".into(),
        });
        assert_eq!(
            solo_sub(&solo),
            Some(SubSpec::new(
                "band",
                "live_set tracks[name=Stems grp#]".into(),
                "solo",
                false
            ))
        );
        assert_eq!(mute_sub(&solo).map(|s| s.prop), Some("mute".to_string()));
        let broken = Binding {
            path: Some("devices[name=".into()),
            ..solo
        };
        assert_eq!(mute_sub(&broken), None);
    }
}
