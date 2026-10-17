//! The fader link's glue (#130, D18): what a strip and its channel detail
//! need of it (the leader's partners and switch, the partner's leaders, the
//! corner mark), and the detail's switch and note.

use fohmixer_proto::client::fader_link_key;
use fohmixer_proto::layout::{LinkId, Strip};
use leptos::prelude::*;

use super::fader::{Law, Target};
use crate::behave::fader::VolumeLaw;
use crate::behave::fader_link::{Flink, flink_of, follows_on, linked};
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
    let flag = move |k: &str| store.hub.try_with(|h| h.flag(k)).flatten();
    let lead_key = key.clone();
    let link_on = Signal::derive(move || linked(lead_key.as_deref().and_then(flag), count));
    let follow_keys = follows.clone();
    let follow_on = Signal::derive(move || {
        let now: Vec<(Option<bool>, bool)> = follow_keys
            .iter()
            .map(|f| (flag(&f.key), f.fixed))
            .collect();
        follows_on(&now)
    });
    let flink = Signal::derive(move || {
        flink_of(
            link_on.try_get().unwrap_or(false),
            follow_on.try_get().unwrap_or(false),
        )
    });
    LinkParts {
        partners,
        link_on,
        flink,
        key,
        labels,
        follows,
    }
}
