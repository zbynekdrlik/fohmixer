//! The channel detail (spec F27, D17; #71 PR D; the approved mockup
//! `docs/mockups/channel-detail-v2.html`): a hold on a strip's ☰ opens that
//! channel over the whole screen, focused work on it alone. A bar (back to
//! the mix, the name chip in the track's colour, its group and instance),
//! then on the left (the left thumb) the mute, Live's dB and a tall fader
//! with its scale and meter, on the right (the right thumb) the pan with
//! Live's display string and STRED; the middle stays free (PR E puts the
//! channel's Pro-Q 4 there). Its parts are the strip's components, its
//! subscriptions `binding::detail_subs` (the surface's wanted set has them
//! through `binding::wanted_subs`), its look `detail.css`.

use std::time::Duration;

use fohmixer_proto::layout::{Strip, StripKind};
use leptos::prelude::*;
use serde_json::json;

use super::buttons::{MuteView, anchor_name, colour_style};
use super::fader::{FaderView, Law, Target};
use super::meter::{MeterView, StatusView};
use super::pan::PanView;
use super::strip::{DbView, ScaleView};
use super::{
    Settings, fail_flash, key_of, owns_touches, readiness, readiness_now, trace_detail, trace_touch,
};
use crate::behave::fader::UNITY;
use crate::behave::hold::guard_left;
use crate::behave::label::shown_label;
use crate::binding::{CLOSE_EXIT, SubSpec, detail_keys, detail_subs};
use crate::dom;
use crate::store::{LiveStore, Readiness, Slot};

/// The channel detail's strip, as held (`Nav.detail` on the surface), and
/// when a hold opened it: a context, so a hold on a strip's ☰ opens it, its
/// exit and a new layout close it. Each open and close goes to the flight
/// recorder with the strip's keys.
#[derive(Clone, Copy)]
pub struct DetailStrip {
    /// The strip as held.
    pub strip: RwSignal<Option<Strip>>,
    /// When a hold opened it (the page clock, `dom::now`): the opening
    /// guard counts from it (`behave::hold::guard_left`).
    pub opened_at: StoredValue<f64>,
}

impl DetailStrip {
    /// A hold opened `strip`'s detail (`why`: `check`, the hold's check;
    /// `lift`, a lift after the hold) by pointer `pointer`.
    pub fn open(self, strip: Strip, why: &str, pointer: i32) {
        trace_detail("open", Some(why), &detail_keys(&strip), Some(pointer));
        let _ = self.opened_at.try_set_value(dom::now());
        let _ = self.strip.try_set(Some(strip));
    }

    /// The detail closes (`why`: `exit`, `layout`, `conflict`; `pointer`
    /// the exit's finger).
    pub fn close(self, why: &str, pointer: Option<i32>) {
        let keys = self
            .strip
            .try_with_untracked(|held| held.as_ref().map(detail_keys))
            .flatten()
            .unwrap_or_default();
        trace_detail("close", Some(why), &keys, pointer);
        let _ = self.strip.try_set(None);
    }
}

/// The bar's line beside the name chip: the group's title and the Live
/// instance (a return's with "ret", as a strip's tag writes it).
pub fn bar_text(title: Option<&str>, instance: &str, ret: bool) -> String {
    let instance = if ret {
        format!("{instance} · ret")
    } else {
        instance.to_string()
    };
    match title {
        Some(title) => format!("{title} · {instance}"),
        None => instance,
    }
}

/// STRED (F11, #71): a tap writes the pan's centre, panning 0.0, as a final
/// `set` (#43); it owns its touches with the pan's key, and waits for Live's
/// value like the pan (I8). The pan's dot shows the write's state.
#[component]
fn CentreView(state: RwSignal<Slot>, spec: SubSpec) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let slot = state;
    let keys = vec![key_of(&spec)];
    let touch_keys = keys.clone();
    let keys = StoredValue::new(keys);
    let spec = StoredValue::new(spec);
    let failed = RwSignal::new(false);
    let on_down = move |ev: web_sys::PointerEvent| {
        if readiness_now(&[slot]) != Readiness::Ready {
            return;
        }
        ev.prevent_default();
        let _ = keys.try_with_value(|k| trace_touch("tap", k, ev.pointer_id()));
        let _ = spec.try_with_value(|s| {
            store.set(
                &s.instance,
                &s.target,
                &s.prop,
                json!(0.0),
                true,
                Some(fail_flash(failed)),
            )
        });
    };
    let state = Memo::new(move |_| readiness(&[slot]));
    let binding = move || state.try_get().map_or("waiting", |s| s.name());
    let disabled = move || state.try_get().map_or("true", |s| s.disabled());
    view! {
        <button
            type="button"
            class="detail-centre"
            use:owns_touches=touch_keys
            class:failed=move || failed.try_get().unwrap_or(false)
            data-testid="detail-centre"
            data-binding=binding
            aria-disabled=disabled
            on:pointerdown=on_down
        >
            "STRED"
        </button>
    }
}

/// One channel over the whole screen; `group` is its group's title
/// (`binding::group_title`).
#[component]
pub fn DetailView(strip: Strip, group: Option<String>) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let settings = expect_context::<Settings>();
    let shown = use_context::<DetailStrip>();
    let subs = detail_subs(&strip, settings.meter_source);
    let volume = subs.volume.as_ref().map(|s| store.slot(s));
    let pan = subs.pan.as_ref().map(|s| store.slot(s));
    let mute = subs.mute.as_ref().map(|s| store.slot(s));
    let color = subs.color.as_ref().map(|s| store.slot(s));
    let meters: Vec<RwSignal<Slot>> = subs.meters.iter().map(|s| store.slot(s)).collect();
    // The status light: every value the detail shows (the pan too; the
    // colour never gates it), and activity from the volume and the meters.
    let all: Vec<RwSignal<Slot>> = [volume, pan, mute]
        .into_iter()
        .flatten()
        .chain(meters.iter().copied())
        .collect();
    let activity: Vec<RwSignal<Slot>> = volume.into_iter().chain(meters.iter().copied()).collect();
    let name = anchor_name(&strip.binding);
    let label = shown_label(strip.label.as_deref(), &name);
    let label_attr = label.clone();
    let instance = strip.binding.instance.clone();
    let ret = strip.strip_kind == StripKind::Return;
    let sub = bar_text(group.as_deref(), &instance, ret);
    let kind = format!("{:?}", strip.strip_kind).to_lowercase();
    let law = settings.law;

    // Right after a hold opens it, its parts take no touch for what is left
    // of OPEN_GUARD_MS (`data-guard`, `detail.css`; `guard_left`: a new
    // layout's remount guards no more): a second tap at ☰'s spot would land
    // on its MUTE. The detail itself still takes them, so nothing passes
    // through to the strips below. The timer goes with the detail.
    let opened_at = shown
        .and_then(|d| d.opened_at.try_get_value())
        .unwrap_or(f64::NEG_INFINITY);
    let left = guard_left(opened_at, dom::now());
    let guard = RwSignal::new(left > 0.0);
    let guard_off = StoredValue::new(None::<TimeoutHandle>);
    if left > 0.0 {
        let timer = set_timeout_with_handle(
            move || {
                let _ = guard_off.try_set_value(None);
                let _ = guard.try_set(false);
            },
            Duration::from_millis(left.ceil() as u64),
        );
        let _ = guard_off.try_set_value(timer.ok());
    }
    on_cleanup(move || {
        if let Some(Some(handle)) = guard_off.try_update_value(Option::take) {
            handle.clear();
        }
    });
    // Back to the mix: the detail closes (its exit owns its touches and
    // writes no key; the close goes to the flight recorder).
    let close = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        if let Some(detail) = shown {
            detail.close(CLOSE_EXIT, Some(ev.pointer_id()));
        }
    };
    // Its two labels: a phone upright shows the short one (`detail.css`).
    let exit_keys: Vec<String> = Vec::new();
    // The name chip in the track's colour, as the strip's name button.
    let chip = move || colour_style(color.and_then(|c| c.try_with(Slot::number).flatten()));
    // The mute with the strip's guard, its look the detail's (dark while
    // the channel sounds, red while muted: `detail.css`).
    let mute_label = "MUTE".to_string();
    let no_colour: Option<RwSignal<Slot>> = None;
    let mute_view = mute.zip(subs.mute.clone()).map(|(slot, spec)| {
        view! {
            <MuteView
                state=slot
                spec=spec
                guarded={strip.mute_guard}
                label=mute_label
                color=no_colour
            />
        }
    });
    let db = volume.map(|slot| view! { <DbView state=slot /> });
    let fader = volume.zip(subs.volume.clone()).map(|(slot, spec)| {
        let targets = vec![Target {
            spec: Some(spec),
            slot,
            law: Law::Volume(law),
        }];
        view! { <FaderView targets=targets shaping={settings.shaping} /> }
    });
    // The unity line at 0 dB on the fader's law (`strip.css` `.fader::after`).
    let unity = format!("--u:{:.4};", law.to_pos(UNITY));
    let pan_view = pan
        .zip(subs.pan.clone())
        .map(|(slot, spec)| view! { <PanView state=slot spec=spec /> });
    // Live's display string of the pan (subscribed with it).
    let pan_text = move || {
        pan.and_then(|s| {
            s.try_with(|slot| slot.display().map(str::to_string))
                .flatten()
        })
        .unwrap_or_default()
    };
    let centre = pan
        .zip(subs.pan.clone())
        .map(|(slot, spec)| view! { <CentreView state=slot spec=spec /> });
    view! {
        <div
            class="detail"
            data-testid="detail"
            data-track=name
            data-label=label_attr
            data-instance=instance
            data-kind=kind
            data-guard=move || guard.try_get().unwrap_or(false).to_string()
        >
            <div class="detail-bar">
                <button
                    type="button"
                    class="detail-exit"
                    use:owns_touches=exit_keys
                    data-testid="detail-exit"
                    on:pointerdown=close
                >
                    <span class="detail-exit-long">"← SPÄŤ NA MIX"</span>
                    <span class="detail-exit-short">"← MIX"</span>
                </button>
                <div class="detail-name">
                    <span class="detail-chip" data-testid="detail-name" style=chip>
                        {label}
                    </span>
                    <span class="detail-sub" data-testid="detail-sub">
                        {sub}
                    </span>
                </div>
            </div>
            <div class="detail-body">
                <section class="detail-box detail-left" data-testid="detail-left">
                    {mute_view}
                    <div class="detail-readout" data-testid="detail-readout">
                        <StatusView slots=all activity=activity />
                        {db}
                    </div>
                    <div class="detail-fz" style=unity>
                        <ScaleView law=law />
                        <MeterView levels=meters law=law />
                        {fader}
                    </div>
                </section>
                <div class="detail-middle" data-testid="detail-middle"></div>
                <section class="detail-box detail-right" data-testid="detail-right">
                    <h2 class="detail-title">"PANORÁMA"</h2>
                    {pan_view}
                    <div class="detail-pan-row">
                        <span>"L 50"</span>
                        <span class="detail-pan-value" data-testid="pan-display">
                            {pan_text}
                        </span>
                        <span>"R 50"</span>
                    </div>
                    {centre}
                </section>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_names_the_group_and_the_instance() {
        assert_eq!(bar_text(Some("HANDS"), "master", false), "HANDS · master");
        assert_eq!(bar_text(None, "band", false), "band");
        assert_eq!(bar_text(Some("FX"), "master", true), "FX · master · ret");
        assert_eq!(bar_text(None, "band", true), "band · ret");
    }
}
