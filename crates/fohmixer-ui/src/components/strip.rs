//! A mixer strip (spec F2–F5, F8–F13; the redesign, #21; held in the hands,
//! #63): at the top the name button, the mute, lit in the track's Live
//! colour while the track is audible and only 30 px high (the fader gets
//! the height), with the status light and Live's dB readout on the line
//! under it (a hand holding the tablet covers the bottom of the screen, so
//! nothing to read sits there); then the instance tag when the group's
//! strips differ and the fader zone (the dB scale, the meter, the fader).
//! The pan left the strip for the channel detail (#71, D17: at the foot
//! the iPad's home bar waits for a swipe, and pans are rarely used), so the
//! strip neither draws nor subscribes it (`StripSubs::shown`): a hold on
//! the ☰ at the end of the readout line opens the detail.

use std::time::Duration;

use fohmixer_proto::layout::{Strip, StripKind};
use leptos::prelude::*;

use super::buttons::{MuteView, anchor_name};
use super::detail::DetailStrip;
use super::fader::{FaderView, Law, Target};
use super::meter::{MeterView, StatusView};
use super::{Settings, owns_touches, trace_touch};
use crate::behave::db_text::db_text;
use crate::behave::fader::{UNITY, VolumeLaw};
use crate::behave::hold::{HINT_MS, HOLD_MS, Hold, Lift};
use crate::behave::label::{mark_look, shown_label};
use crate::behave::scale::{tick_label, tick_pos, ticks};
use crate::binding::strip_subs;
use crate::dom;
use crate::store::{LiveStore, Slot};

/// The dB scale beside a fader: the labels of the fader's volume law at the
/// fader positions of their levels.
#[component]
pub fn ScaleView(law: VolumeLaw) -> impl IntoView {
    let labels = ticks(law)
        .iter()
        .map(|db| {
            let top = format!("top:{:.3}%;", (1.0 - tick_pos(law, *db)) * 100.0);
            let unity = *db == 0.0;
            view! {
                <span class="tick" class:unity=unity data-db=db.to_string() style=top>
                    {tick_label(*db)}
                </span>
            }
        })
        .collect_view();
    view! { <div class="scale" data-testid="scale" aria-hidden="true">{labels}</div> }
}

/// Live's dB readout in TouchOSC's form (one decimal, no unit, white exactly
/// at 0 dB).
#[component]
pub fn DbView(state: RwSignal<Slot>) -> impl IntoView {
    let text = Memo::new(move |_| state.try_with(|s| s.display().and_then(db_text)).flatten());
    let shown = move || text.try_get().flatten().map(|t| t.text).unwrap_or_default();
    let unity = move || text.try_get().flatten().is_some_and(|t| t.unity);
    view! {
        <span class="db" class:unity=unity data-testid="db">{shown}</span>
    }
}

/// The ☰ at the end of a strip's readout line (#71, F27): a hold of
/// `HOLD_MS` opens the strip's channel detail (`behave::hold`; the button
/// fills meanwhile, `data-holding`, a CSS transition), a tap shows the hint
/// for `HINT_MS` (`data-hint`: the stylesheet's text over the name button,
/// no layout change), a finger that slides further than `SLIDE_PX` ends
/// the press with neither (a fader grab that lands on ☰). It owns its
/// touches and writes no key; its down, up and cancel go to the flight
/// recorder; a conflicted strip's is inert like the rest of it
/// (`states.css`).
#[component]
fn StripMenu(strip: Strip) -> impl IntoView {
    let detail = use_context::<DetailStrip>();
    let strip = StoredValue::new(strip);
    let hold = StoredValue::new(Hold::default());
    let check = StoredValue::new(None::<TimeoutHandle>);
    let hint_off = StoredValue::new(None::<TimeoutHandle>);
    let holding = RwSignal::new(false);
    let hint = RwSignal::new(false);
    // The timers go with the strip (the cleanup writes no signal of its own).
    on_cleanup(move || {
        for timer in [check, hint_off] {
            if let Some(Some(handle)) = timer.try_update_value(Option::take) {
                handle.clear();
            }
        }
    });
    // The fill shows the press (`Hold::holding`); a press over waits for no
    // check.
    let settle = move || {
        let pressing = hold.try_with_value(Hold::holding).unwrap_or(false);
        if !pressing && let Some(Some(handle)) = check.try_update_value(Option::take) {
            handle.clear();
        }
        let _ = holding.try_set(pressing);
    };
    let open = move || {
        if let (Some(DetailStrip(shown)), Some(held)) = (detail, strip.try_get_value()) {
            let _ = shown.try_set(Some(held));
        }
    };
    let show_hint = move || {
        let _ = hint.try_set(true);
        if let Some(Some(handle)) = hint_off.try_update_value(Option::take) {
            handle.clear();
        }
        let timer = set_timeout_with_handle(
            move || {
                let _ = hint_off.try_set_value(None);
                let _ = hint.try_set(false);
            },
            Duration::from_millis(HINT_MS as u64),
        );
        let _ = hint_off.try_set_value(timer.ok());
    };
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let id = ev.pointer_id();
        let primary = ev.is_primary();
        let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
        let pressed = hold
            .try_update_value(|h| h.down(id, primary, x, y, dom::now()))
            .unwrap_or(false);
        if !pressed {
            return;
        }
        trace_touch("down", &[], id);
        if let Some(el) = dom::current_element(&ev) {
            let _ = el.set_pointer_capture(id);
        }
        if let Some(Some(handle)) = check.try_update_value(Option::take) {
            handle.clear();
        }
        // One check a little after the hold (as the mute guard's), so a
        // coarse clock never finds the press a hair short of it.
        let timer = set_timeout_with_handle(
            move || {
                let _ = check.try_set_value(None);
                let opens = hold
                    .try_update_value(|h| h.open(dom::now()))
                    .unwrap_or(false);
                settle();
                if opens {
                    open();
                }
            },
            Duration::from_millis(HOLD_MS as u64 + 20),
        );
        let _ = check.try_set_value(timer.ok());
        settle();
    };
    // A finger that slides off the press ends it (`Hold::moved`): the
    // recorder hears it as a cancel.
    let on_move = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        let (x, y) = (f64::from(ev.client_x()), f64::from(ev.client_y()));
        let ended = hold
            .try_update_value(|h| h.moved(id, x, y))
            .unwrap_or(false);
        if ended {
            trace_touch("cancel", &[], id);
            settle();
        }
    };
    let on_up = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        let mine = hold.try_with_value(|h| h.drives(id)).unwrap_or(false);
        let lift = hold
            .try_update_value(|h| h.up(id, dom::now()))
            .unwrap_or(Lift::Nothing);
        if mine {
            trace_touch("up", &[], id);
        }
        settle();
        match lift {
            Lift::Open => open(),
            Lift::Tap => show_hint(),
            Lift::Nothing => {}
        }
    };
    // A cancelled pointer, or one whose capture was lost: no tap.
    let on_cancel = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        let mine = hold.try_with_value(|h| h.drives(id)).unwrap_or(false);
        let _ = hold.try_update_value(|h| h.cancel(id));
        if mine {
            trace_touch("cancel", &[], id);
        }
        settle();
    };
    // It owns its touches (#43 PR G); it writes no key.
    let no_keys: Vec<String> = Vec::new();
    view! {
        <button
            type="button"
            class="strip-menu"
            use:owns_touches=no_keys
            data-testid="strip-menu"
            aria-label="Detail kanála (podrž)"
            data-holding=move || holding.try_get().unwrap_or(false).to_string()
            data-hint=move || hint.try_get().unwrap_or(false).to_string()
            on:pointerdown=on_down
            on:pointermove=on_move
            on:pointerup=on_up
            on:pointercancel=on_cancel
            on:lostpointercapture=on_cancel
        >
            <span class="strip-menu-icon" aria-hidden="true">"☰"</span>
        </button>
    }
}

/// One strip; `shared` is the Live instance every strip of its group shares
/// (#63: the group's title names it, so the strip does not).
#[component]
pub fn StripView(strip: Strip, settings: Settings, shared: Option<String>) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    // The strip its ☰ opens the detail of (#71).
    let menu_strip = strip.clone();
    let subs = strip_subs(&strip, settings.meter_source);
    let volume = subs.volume.as_ref().map(|s| store.slot(s));
    let mute = subs.mute.as_ref().map(|s| store.slot(s));
    let color = subs.color.as_ref().map(|s| store.slot(s));
    let meters: Vec<RwSignal<Slot>> = subs.meters.iter().map(|s| store.slot(s)).collect();
    // The status light: every value the strip shows (the colour never
    // gates it), and activity from the volume and the meters.
    let all: Vec<RwSignal<Slot>> = [volume, mute]
        .into_iter()
        .flatten()
        .chain(meters.iter().copied())
        .collect();
    let activity: Vec<RwSignal<Slot>> = volume.into_iter().chain(meters.iter().copied()).collect();
    let name = anchor_name(&strip.binding);
    let label = shown_label(strip.label.as_deref(), &name);
    let label_attr = label.clone();
    let instance = strip.binding.instance.clone();

    let fader = volume.zip(subs.volume.clone()).map(|(slot, spec)| {
        let targets = vec![Target {
            spec: Some(spec),
            slot,
            law: Law::Volume(settings.law),
        }];
        view! { <FaderView targets=targets shaping={settings.shaping} /> }
    });
    let mute_view = mute.zip(subs.mute.clone()).map(|(slot, spec)| {
        view! {
            <MuteView
                state=slot
                spec=spec
                guarded={strip.mute_guard}
                label=label
                color=color
            />
        }
    });
    let db = volume.map(|slot| view! { <DbView state=slot /> });
    let ret = strip.strip_kind == StripKind::Return;
    // The instance tag only where the group does not name it (#63).
    let tagged = shared.as_deref() != Some(instance.as_str());
    let tag = tagged.then(|| {
        let text = if ret {
            format!("{instance} · ret")
        } else {
            instance.clone()
        };
        let tag_instance = instance.clone();
        view! {
            <span class="strip-tag" data-testid="strip-instance" data-instance=tag_instance>
                {text}
            </span>
        }
    });
    // A return in a named group says so on its name button.
    let ret_mark = (ret && !tagged).then(|| {
        view! { <span class="strip-ret" data-testid="strip-ret" aria-hidden="true">"RET"</span> }
    });
    // A marker problem (#68): a conflict disables the strip, any other
    // problem marks it; the word replaces Live's dB on the readout line.
    let look = mark_look(strip.mark);
    let mark = look.map(|(mark, _)| mark);
    let mark_word = look.map(|(_, word)| {
        view! { <span class="strip-mark" data-testid="strip-mark">{word}</span> }
    });
    let flag = (mark == Some("problem")).then(|| {
        view! { <span class="strip-flag" aria-hidden="true">"!"</span> }
    });
    let kind = format!("{:?}", strip.strip_kind).to_lowercase();
    // Each attribute its own copy (the macro may move a value into a child
    // before an attribute reads it).
    let strip_instance = instance;
    let wide = strip.wide;
    let law = settings.law;
    // The unity line at 0 dB on the fader's law (`strip.css` `.fader::after`).
    let unity = format!("--u:{:.4};", law.to_pos(UNITY));
    view! {
        <div
            class="strip"
            class:wide=wide
            data-testid="strip"
            data-track=name
            data-label=label_attr
            data-instance=strip_instance
            data-kind=kind
            data-mark=mark
        >
            <div class="strip-head">
                {mute_view}
                <div class="strip-info">
                    <div class="strip-readout" data-testid="strip-readout">
                        <StatusView slots=all activity=activity />
                        {db}
                        {mark_word}
                    </div>
                    <StripMenu strip=menu_strip />
                </div>
                {ret_mark}
                {flag}
            </div>
            {tag}
            <div class="strip-fz" style=unity>
                <ScaleView law=law />
                <MeterView levels={meters.clone()} law=law />
                {fader}
            </div>
        </div>
    }
}
