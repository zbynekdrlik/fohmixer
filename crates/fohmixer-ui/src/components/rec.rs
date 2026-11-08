//! The recording watch's keys (#172; `behave::rec`): a strip's ARM key above
//! SEL and the column foot's REC key. Both are one `components::hold_key`
//! [`HoldKey`], a press as a rail button's (`behave::rail::rail_press`,
//! `Hold::lifted`): full at `RAIL_HOLD_MS`, it acts at its lift; a tap, a
//! slide, a cancel, a lost capture or a hidden page do nothing, and a press
//! whose end never came ends at the next primary down of its type. Each step
//! goes to the flight recorder as a `rail` step (`button` `arm` or `rec`). A
//! press that cannot act (the page not connected, the hub's state not known
//! on this connection) shows no fill and flashes refused at its full mark.

use fohmixer_proto::client::RecState;
use fohmixer_proto::layout::Strip;
use leptos::prelude::*;
use serde_json::json;

use super::hold_key::{Flash, HoldKey};
use super::key_of;
use crate::behave::rail::hold_value;
use crate::behave::rec::{ArmLook, arm_look, foot_look, record_mode, strip_arm};
use crate::binding::{SubSpec, arm_spec, record_spec};
use crate::store::LiveStore;

/// One flag key: `spec`, a flag of Live, a full press's lift writes the
/// other way. `current` reads that flag from the hub's state; the press may
/// act only while the page is connected and the flag is known
/// (`hold_value`).
#[component]
fn FlagKey<F>(
    spec: SubSpec,
    current: F,
    button: &'static str,
    class: &'static str,
    testid: &'static str,
    label: &'static str,
    #[prop(into)] state: Signal<&'static str>,
    children: Children,
) -> impl IntoView
where
    F: Fn(Option<&RecState>) -> Option<bool> + Copy + Send + Sync + 'static,
{
    let store = expect_context::<LiveStore>();
    let keys = vec![key_of(&spec)];
    let spec = StoredValue::new(spec);
    let can = Signal::derive(move || {
        let flag = store.rec.try_with(|r| current(r.fresh())).flatten();
        hold_value(flag, store.connected.try_get().unwrap_or(false)).is_some()
    });
    // What a hold may write now: the flag the other way, or none (refused).
    let wanted = move || {
        let flag = store
            .rec
            .try_with_untracked(|r| current(r.fresh()))
            .flatten();
        hold_value(flag, store.connected.try_get_untracked().unwrap_or(false))
    };
    let act = move |on: bool, flash: Flash| {
        let _ = spec.try_with_value(|s| {
            store.set(
                &s.instance,
                &s.target,
                &s.prop,
                json!(on),
                true,
                Some(flash),
            )
        });
    };
    view! {
        <HoldKey
            keys=keys
            wanted=wanted
            can=can
            act=act
            button=button
            class=class
            testid=testid
            label=label
            state=state
        >
            {children()}
        </HoldKey>
    }
}

/// A strip's ARM key (#172): only on a track that can be armed in the
/// watched Live, a 6 px dot in the free column above SEL, its touch area
/// invisible.
#[component]
pub fn ArmKey(strip: Strip) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let spec = StoredValue::new(arm_spec(&strip.binding));
    let strip = StoredValue::new(strip);
    let look = Memo::new(move |_| {
        store
            .rec
            .try_with(|r| strip.try_with_value(|s| arm_look(r.shown(), s)))
            .flatten()
            .unwrap_or(ArmLook::None)
    });
    let armable = Memo::new(move |_| look.try_get().is_some_and(|l| l != ArmLook::None));
    let state = Signal::derive(move || look.try_get().unwrap_or(ArmLook::None).name());
    let current =
        move |rec: Option<&RecState>| strip.try_with_value(|s| strip_arm(rec, s)).flatten();
    move || {
        if !armable.try_get().unwrap_or(false) {
            return None;
        }
        spec.try_get_value().flatten().map(|spec| {
            view! {
                <FlagKey
                    spec=spec
                    current=current
                    button="arm"
                    class="strip-arm"
                    testid="arm-key"
                    label="Arm (hold)"
                    state=state
                >
                    <i class="rec-dot" aria-hidden="true"></i>
                </FlagKey>
            }
        })
    }
}

/// The column foot's REC key (#172): small dim text with the same dot, in
/// the status line's size.
#[component]
pub fn RecKey() -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let look = Memo::new(move |_| {
        store
            .rec
            .try_with(|r| foot_look(r.shown()))
            .unwrap_or_else(|| foot_look(None))
    });
    let state = Signal::derive(move || look.try_get().unwrap_or_else(|| foot_look(None)).0);
    let word = move || look.try_get().unwrap_or_else(|| foot_look(None)).1;
    view! {
        <FlagKey
            spec=record_spec()
            current=record_mode
            button="rec"
            class="rec-key"
            testid="rec-key"
            label="Record (hold)"
            state=state
        >
            <i class="rec-dot" aria-hidden="true"></i>
            <span class="rec-word">{word}</span>
        </FlagKey>
    }
}
