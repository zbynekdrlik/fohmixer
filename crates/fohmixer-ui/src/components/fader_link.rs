//! The fader link's glue (#130, D18): what a strip and its channel detail
//! need of it (the leader's partners and switch, the partner's leaders, the
//! corner mark), and the detail's switch and note.

use fohmixer_proto::client::fader_link_key;
use fohmixer_proto::layout::{LinkId, Strip};
use leptos::prelude::*;
use serde_json::json;

use super::fader::{Law, Target};
use super::{BtnText, fail_flash, owns_touches, trace_touch};
use crate::behave::fader::VolumeLaw;
use crate::behave::fader_link::{
    Flink, flink_of, follow_state, follow_text, follows_on, link_text, linked,
};
use crate::behave::rail::hold_value;
use crate::binding::StripSubs;
use crate::store::LiveStore;

/// A partner's leader: its switch's hub key, its label, always on (`fixed`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Follow {
    pub key: String,
    pub leader: String,
    pub fixed: bool,
}

/// What a strip's fader, corner and detail need of its fader links.
pub struct LinkParts {
    /// A leader's partners' volumes (empty for any other strip).
    pub partners: Vec<Target>,
    /// The leader's link is on and it has partners.
    pub link_on: Signal<bool>,
    /// The corner's mark.
    pub flink: Signal<Flink>,
    /// A leader's switch's hub key.
    pub key: Option<String>,
    /// A leader's partners' labels (its switch's text).
    pub labels: Vec<String>,
    /// A partner's leaders.
    pub follows: Vec<Follow>,
}

/// The fader link parts of `strip` (its subscriptions `subs`, the fader law `law`).
pub fn link_parts(store: LiveStore, strip: &Strip, subs: &StripSubs, law: VolumeLaw) -> LinkParts {
    let instance = strip.binding.instance.clone();
    let key_of = |id: &str| {
        fader_link_key(&LinkId {
            instance: instance.clone(),
            id: id.to_string(),
        })
    };
    let partners: Vec<Target> = subs
        .fader_partners
        .iter()
        .map(|spec| Target {
            spec: Some(spec.clone()),
            slot: store.slot(spec),
            law: Law::Volume(law),
        })
        .collect();
    let count = partners.len();
    let key = strip.fader_link.as_ref().map(|l| key_of(&l.id));
    let labels = strip
        .fader_link
        .iter()
        .flat_map(|l| &l.partners)
        .map(|p| p.label.clone())
        .collect();
    let follows: Vec<Follow> = strip
        .fader_follows
        .iter()
        .map(|f| Follow {
            key: key_of(&f.id),
            leader: f.leader.clone(),
            fixed: f.fixed,
        })
        .collect();
    // Memos (review M6): another hub value's change (STAGE AUT, another
    // link's switch) re-runs them but reaches no mark or fader.
    let flag = move |k: &str| store.hub.try_with(|h| h.flag(k)).flatten();
    let lead_key = key.clone();
    let link_on = Memo::new(move |_| linked(lead_key.as_deref().and_then(flag), count));
    let follow_keys = follows.clone();
    let follow_on = Memo::new(move |_| {
        let now: Vec<(Option<bool>, bool)> = follow_keys
            .iter()
            .map(|f| (flag(&f.key), f.fixed))
            .collect();
        follows_on(&now)
    });
    let flink = Memo::new(move |_| {
        flink_of(
            link_on.try_get().unwrap_or(false),
            follow_on.try_get().unwrap_or(false),
        )
    });
    LinkParts {
        partners,
        link_on: link_on.into(),
        flink: flink.into(),
        key,
        labels,
        follows,
    }
}

/// A leader's switch in its channel detail (#130, D18): `LINKED TO <the
/// partners' labels>`, lit while on. A tap asks the hub for the other value
/// once its value is fresh on this connection and the page is connected
/// (the rail's rule, `behave::rail::hold_value`), else it flashes refused.
/// It owns its touches; its tap goes to the flight recorder with the
/// leader's and its partners' volume keys.
#[component]
pub fn FaderLinkView(link_key: String, labels: Vec<String>, keys: Vec<String>) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    let text = link_text(&labels);
    let key = StoredValue::new(link_key);
    let lit = Memo::new(move |_| {
        key.try_with_value(|k| store.hub.try_with(|h| h.flag(k)).flatten())
            .flatten()
            == Some(true)
    });
    let on = move || lit.try_get().unwrap_or(false);
    let ready = move || store.connected.try_get().unwrap_or(false);
    let failed = RwSignal::new(false);
    let touch_keys = keys.clone();
    let keys = StoredValue::new(keys);
    let on_down = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        let _ = keys.try_with_value(|k| trace_touch("tap", k, ev.pointer_id()));
        let fresh = key
            .try_with_value(|k| store.hub.try_with_untracked(|h| h.fresh_flag(k)).flatten())
            .flatten();
        let connected = store.connected.try_get_untracked().unwrap_or(false);
        match (hold_value(fresh, connected), key.try_get_value()) {
            (Some(value), Some(k)) => store.set_hub(&k, json!(value)),
            _ => fail_flash(failed)("refused".to_string()),
        }
    };
    view! {
        <div
            class="btn fader-link"
            use:owns_touches=touch_keys
            class:on=on
            class:failed=move || failed.try_get().unwrap_or(false)
            data-testid="fader-link"
            data-on=move || on().to_string()
            aria-disabled=move || (!ready()).to_string()
            on:pointerdown=on_down
        >
            <BtnText text=text />
        </div>
    }
}

/// A partner's note in its channel detail (#130): each leader that moves
/// it (`FOLLOWS <leader>`) and whether it does now; no switch, no touch.
#[component]
pub fn FaderFollowView(follows: Vec<Follow>) -> impl IntoView {
    let store = expect_context::<LiveStore>();
    follows
        .into_iter()
        .map(|f| {
            let who = follow_text(&f.leader);
            let key = StoredValue::new(f.key);
            let value = Memo::new(move |_| {
                key.try_with_value(|k| store.hub.try_with(|h| h.flag(k)).flatten())
                    .flatten()
            });
            let flag = move || value.try_get().flatten();
            let (leader, fixed) = (f.leader, f.fixed);
            let state = move || follow_state(flag(), fixed, &leader);
            let on = move || (fixed || flag() == Some(true)).to_string();
            view! {
                <div class="fader-follow" data-testid="fader-follow" data-on=on>
                    <span class="fader-follow-who">{who}</span>
                    <span class="fader-follow-state">{state}</span>
                </div>
            }
        })
        .collect_view()
}
