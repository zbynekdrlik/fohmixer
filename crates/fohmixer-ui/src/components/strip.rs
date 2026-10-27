//! A mixer strip (spec F2–F5, F8–F13; the redesign, #21; held in the hands,
//! #63; the owner's mockup v8, #151): at the top the name button, the mute,
//! as the strip's top edge (37 px, calm: the track's Live colour only as a
//! 3 px edge), the name first and one marks line under it, with the status
//! light and Live's dB readout on the line under it (a hand holding the
//! tablet covers the bottom of the screen, so nothing to read sits there);
//! then the instance tag when the group's strips differ and the fader
//! zone: the info column (the LUFS slot, the group's name on the first
//! strip of its run, #127), the fader with the stereo meter in its track,
//! the dB scale on its right, and SEL at the bottom left, whose hold opens
//! the channel detail. The pan left the strip for the channel detail (#71,
//! D17: at the foot the iPad's home bar waits for a swipe, and pans are
//! rarely used), so the strip neither draws nor subscribes it
//! (`StripSubs::shown`).

use std::time::Duration;

use fohmixer_proto::layout::Strip;
use leptos::prelude::*;

use super::buttons::{Corner, MuteView, Partner, anchor_name};
use super::detail::DetailStrip;
use super::fader::{FaderView, Law, Target};
use super::fader_link::link_parts;
use super::meter::StatusView;
use super::{Settings, key_of, owns_touches, trace_detail};
use crate::arrange::GroupLabel;
use crate::behave::db_text::db_text;
use crate::behave::fader::{UNITY, VolumeLaw};
use crate::behave::hold::{HINT_MS, HOLD_MS, Hold, Lift};
use crate::behave::label::{mark_look, shown_label};
use crate::behave::lufs::{LufsLook, kind_name, look as lufs_look_of, point_key};
use crate::behave::reverb::held_low;
use crate::behave::scale::{tick_label, tick_pos, ticks};
use crate::binding::{detail_keys, strip_subs};
use crate::dom;
use crate::store::{LiveStore, Slot};

/// The dB scale beside a fader: the labels of the fader's volume law at the
/// fader positions of their levels. It takes no touch: the fader over the
/// scale gets it. A strip's sits right of its fader's track (#151,
/// `strip.css`), the detail's left of its meter.
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

/// A strip's info column (#151, mockups v8 and v9): left of the cap's
/// widest reach, the zone's height: the LUFS caption (`LUFS` over `IN` or
/// `OUT`, two 9 px lines) and the reading (`behave::lufs::look`: quiet
/// text, tinted only out of range; silence, no reading, a problem or a
/// strip without `+LUFS` show nothing, their rows kept), and on the first
/// strip of a group's run `group_name`, the group's name read bottom to
/// top, in grey (#127; one colour per strip, the track's own). It takes no
/// touch: the fader under it gets it.
#[component]
fn InfoColumn(
    group_name: Signal<Option<GroupLabel>>,
    #[prop(into)] lufs: Signal<LufsLook>,
    kind: Option<&'static str>,
) -> impl IntoView {
    let side = move || {
        group_name
            .try_get()
            .flatten()
            .map(|GroupLabel { text, instance }| {
                view! {
                    <span class="group-side" data-testid="group-side" data-instance=instance>
                        {text}
                    </span>
                }
            })
    };
    let state = move || lufs.try_with(|l| l.state.as_str()).unwrap_or("none");
    let text = move || {
        lufs.try_with(|l| l.text.clone())
            .flatten()
            .unwrap_or_default()
    };
    let line = move |i: usize| {
        move || {
            lufs.try_with(|l| l.caption.map(|c| c[i]))
                .flatten()
                .unwrap_or_default()
        }
    };
    let captioned = move || {
        lufs.try_with(|l| l.caption.is_some())
            .unwrap_or(false)
            .to_string()
    };
    let cap_kind = kind;
    view! {
        <div class="strip-col" data-testid="strip-col" aria-hidden="true">
            <div class="lufs-cap" data-testid="lufs-cap" data-kind=cap_kind data-shown=captioned>
                <span>{line(0)}</span>
                <span>{line(1)}</span>
            </div>
            <div class="lufs" data-testid="lufs" data-kind=kind data-l=state>{text}</div>
            {side}
        </div>
    }
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

/// SEL at a strip's bottom left (#151, mockup v8; ☰ on the readout line
/// before, #71, F27): a hold of `HOLD_MS` opens the strip's channel detail
/// (`behave::hold`; the key fills meanwhile, `data-holding`, a CSS
/// transition), a tap shows the hint for `HINT_MS` (`data-hint`: the
/// stylesheet's text above the key, no layout change), a finger that
/// slides further than `SLIDE_PX` ends the press with neither (a fader grab
/// that lands on SEL). It owns its touches and writes no key; each step
/// (press, slid, tap, open) goes to the flight recorder with the strip's
/// keys (`binding::detail_keys`); a conflicted strip's is inert like the
/// rest of it (`states.css`).
#[component]
fn DetailKey(strip: Strip) -> impl IntoView {
    let detail = use_context::<DetailStrip>();
    let keys = StoredValue::new(detail_keys(&strip));
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
    // The flight recorder hears each step with the strip's keys.
    let step = move |what: &str, pointer: i32| {
        let _ = keys.try_with_value(|k| trace_detail(what, None, k, Some(pointer)));
    };
    // `why`: `check` (the hold's check) or `lift` (a lift after the hold).
    let open = move |why: &str, pointer: i32| {
        if let (Some(shown), Some(held)) = (detail, strip.try_get_value()) {
            shown.open(held, why, pointer);
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
        let pressed = hold
            .try_update_value(|h| h.down(id, primary, dom::now()))
            .unwrap_or(false);
        if !pressed {
            return;
        }
        step("press", id);
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
                    open("check", id);
                }
            },
            Duration::from_millis(HOLD_MS as u64 + 20),
        );
        let _ = check.try_set_value(timer.ok());
        settle();
    };
    // A finger that slides off the press ends it (`Hold::moved`).
    let on_move = move |ev: web_sys::PointerEvent| {
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
        let id = ev.pointer_id();
        let lift = hold
            .try_update_value(|h| h.up(id, dom::now()))
            .unwrap_or(Lift::Nothing);
        settle();
        match lift {
            Lift::Open => open("lift", id),
            Lift::Tap => {
                step("tap", id);
                show_hint();
            }
            Lift::Nothing => {}
        }
    };
    // A cancelled pointer, or one whose capture was lost: no tap.
    let on_cancel = move |ev: web_sys::PointerEvent| {
        let id = ev.pointer_id();
        let _ = hold.try_update_value(|h| h.cancel(id));
        settle();
    };
    // It owns its touches (#43 PR G); it writes no key.
    let no_keys: Vec<String> = Vec::new();
    view! {
        <button
            type="button"
            class="strip-sel"
            use:owns_touches=no_keys
            data-testid="detail-key"
            aria-label="Channel detail (hold)"
            data-holding=move || holding.try_get().unwrap_or(false).to_string()
            data-hint=move || hint.try_get().unwrap_or(false).to_string()
            on:pointerdown=on_down
            on:pointermove=on_move
            on:pointerup=on_up
            on:pointercancel=on_cancel
            on:lostpointercapture=on_cancel
        >
            <span class="strip-sel-key" aria-hidden="true">
                <span class="strip-sel-word">"SEL"</span>
            </span>
        </button>
    }
}

/// One strip; `shared` is the Live instance every strip of its group shares
/// (#63: the group names it once, since #127 by its name on the side of
/// its run's first strip, so the strip does not); `group_name` that name
/// when the strip starts a run (#127, `InfoColumn`).
#[component]
pub fn StripView(
    strip: Strip,
    settings: Settings,
    shared: Option<String>,
    #[prop(optional, into)] group_name: Signal<Option<GroupLabel>>,
) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    // The strip its SEL opens the detail of (#71, #151).
    let key_strip = strip.clone();
    let subs = strip_subs(&strip, settings.meter_source);
    // The fader link (#130, D18): a leader's partners and switch, a
    // partner's leaders, the corner's mark.
    let parts = link_parts(store, &strip, &subs, settings.law);
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

    let link_on = parts.link_on;
    let flink = parts.flink;
    let link_partners = parts.partners;
    // A reverb strip (#130, D18): REVERB's marks on its fader, and its
    // readout in the rule's colour while the rule holds it low.
    let reverb = strip.reverb;
    let volume_key = StoredValue::new(subs.volume.as_ref().map(key_of).unwrap_or_default());
    let low = move || {
        reverb
            && store
                .reverb
                .try_with(|r| volume_key.try_with_value(|k| held_low(r.shown(), k)))
                .flatten()
                .unwrap_or(false)
    };
    let readout_aut = move || if low() { "low" } else { "" };
    let fader = volume.zip(subs.volume.clone()).map(|(slot, spec)| {
        let targets = vec![Target {
            spec: Some(spec),
            slot,
            law: Law::Volume(settings.law),
        }];
        // The stereo pair in the fader's track (#151).
        let track_meters = meters.clone();
        view! {
            <FaderView
                targets=targets
                shaping={settings.shaping}
                partners=link_partners
                link_on=link_on
                reverb=reverb
                meters=track_meters
            />
        }
    });
    // The instance tag only where the group does not name it (#63).
    let tagged = shared.as_deref() != Some(instance.as_str());
    // A marker problem (#68): a conflict disables the strip, any other
    // problem marks it; the word replaces Live's dB on the readout line.
    let look = mark_look(strip.mark);
    let mark = look.map(|(mark, _)| mark);
    // The linked mute (#127): each partner's mute, read and written with
    // the strip's own. The name button's marks line (#151) holds a
    // problem's `!`, the chain, the fader link's mark and `MUTED`; no RET.
    let partners: Vec<Partner> = subs
        .linked
        .iter()
        .map(|spec| Partner {
            spec: spec.clone(),
            slot: store.slot(spec),
        })
        .collect();
    let corner: Option<Corner> = Some(Corner {
        flag: mark == Some("problem"),
    });
    // The loudness reading (#151): the hub's point for this strip's track,
    // read only on a `+LUFS` strip; a problem is the marks line's `!`.
    let lufs_spec = strip.lufs;
    let lufs_kind = kind_name(lufs_spec);
    let lufs_key = StoredValue::new(point_key(&strip));
    let lufs_look = Memo::new(move |_| {
        let point = lufs_key
            .try_with_value(|key| {
                key.as_ref().and_then(|(instance, target)| {
                    store.lufs.try_with(|l| l.point(instance, target))
                })
            })
            .flatten()
            .unwrap_or_default();
        lufs_look_of(point.tenths, point.problem, lufs_spec)
    });
    let lufs_flag =
        Signal::derive(move || lufs_look.try_with(|l| l.problem.is_some()).unwrap_or(false));
    let lufs_problem = move || lufs_look.try_with(|l| l.problem).flatten();
    let mute_view = mute.zip(subs.mute.clone()).map(|(slot, spec)| {
        view! {
            <MuteView
                state=slot
                spec=spec
                guarded={strip.mute_guard}
                label=label
                color=color
                partners=partners
                corner=corner
                flink=flink
                lufs_flag=lufs_flag
            />
        }
    });
    let db = volume.map(|slot| view! { <DbView state=slot /> });
    // The instance alone, a return's too (#151: no RET on a strip).
    let tag = tagged.then(|| {
        let text = instance.clone();
        let tag_instance = instance.clone();
        view! {
            <span class="strip-tag" data-testid="strip-instance" data-instance=tag_instance>
                {text}
            </span>
        }
    });
    let mark_word = look.map(|(_, word)| {
        view! { <span class="strip-mark" data-testid="strip-mark">{word}</span> }
    });
    let kind = format!("{:?}", strip.strip_kind).to_lowercase();
    // Each attribute its own copy (the macro may move a value into a child
    // before an attribute reads it).
    let strip_instance = instance;
    let wide = strip.wide;
    let law = settings.law;
    // The unity mark at 0 dB on the fader's law (`strip.css` `.fader::after`).
    let unity = format!("--u:{:.4};", law.to_pos(UNITY));
    // The fader link's mark (#130): `link.css` gives a leader's cap line
    // the accent.
    let flink_attr = move || flink.try_get().unwrap_or_default().name();
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
            data-flink=flink_attr
            data-lufs=lufs_kind
            data-lufs-problem=lufs_problem
        >
            <div class="strip-head">
                {mute_view}
                <div class="strip-info">
                    <div class="strip-readout" data-testid="strip-readout" data-aut=readout_aut>
                        <StatusView slots=all activity=activity />
                        {db}
                        {mark_word}
                    </div>
                </div>
            </div>
            {tag}
            <div class="strip-zone" style=unity>
                <InfoColumn group_name=group_name lufs=lufs_look kind=lufs_kind />
                <ScaleView law=law />
                {fader}
                <DetailKey strip=key_strip />
            </div>
        </div>
    }
}
