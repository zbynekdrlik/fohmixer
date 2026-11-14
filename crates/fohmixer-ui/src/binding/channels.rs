//! The channel detail's channel selector (#205, the owner's approved mockup
//! `docs/mockups/channel-select-v1.html`; it replaced the detail's ◀ ▶):
//! the name chip opens a sheet with every fader of the pool, grouped by
//! Live set, a fader no layout holds marked NOT PLACED and the one shown
//! outlined; one tap opens that fader's detail. Pure: the list, and what a
//! pick opens; `components::channel_select` draws it.

use fohmixer_proto::layout::{Layout, Strip};
use fohmixer_proto::sectors::{SectorId, SectorsDoc, StripRef, pool};

use super::sectors::{detail_ref, sectors_live};
use crate::behave::label::shown_label;

/// One fader of the selector.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    pub id: StripRef,
    pub strip: Strip,
    /// Its name as its strip shows it.
    pub label: String,
    /// Some layout of the document holds it.
    pub placed: bool,
    /// It is the fader the detail shows.
    pub current: bool,
}

/// One Live set's faders, under its name (`BAND`, `MASTER`, `MBC`).
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelGroup {
    pub instance: String,
    pub name: String,
    pub channels: Vec<Channel>,
}

impl ChannelGroup {
    /// What the sheet keys a group by: its Live set and each fader's
    /// identity, mark and outline, so a change in another group keeps this
    /// one's picks (and a press under a finger on one).
    pub fn key(&self) -> String {
        let channels: Vec<String> = self
            .channels
            .iter()
            .map(|c| format!("{}:{}:{}", c.id.label, c.placed, c.current))
            .collect();
        format!("{}|{}", self.instance, channels.join("|"))
    }
}

/// Whether some layout of `doc` holds `id`.
fn placed(doc: &SectorsDoc, id: &StripRef) -> bool {
    SectorId::ALL
        .iter()
        .any(|&sector| doc.layouts(sector).iter().any(|layout| layout.holds(id)))
}

/// The selector's list for the detail of `current`: every strip of the
/// served layout's pool, in its order (the parameter faders left out, as
/// ◀ ▶ left them), grouped by Live set in the order the pool meets them.
/// None while the document places no fader (the detail as before).
pub fn channel_groups(
    doc: Option<&SectorsDoc>,
    layout: &Layout,
    current: &Strip,
) -> Vec<ChannelGroup> {
    let Some(doc) = doc.filter(|doc| sectors_live(Some(doc))) else {
        return Vec::new();
    };
    let now = detail_ref(current);
    let mut groups: Vec<ChannelGroup> = Vec::new();
    for pooled in pool(layout) {
        let Some(strip) = pooled.strip() else {
            continue;
        };
        let channel = Channel {
            label: shown_label(strip.label.as_deref(), &pooled.id.label),
            placed: placed(doc, &pooled.id),
            current: now.as_ref() == Some(&pooled.id),
            strip: strip.clone(),
            id: pooled.id.clone(),
        };
        match groups
            .iter_mut()
            .find(|g| g.instance == channel.id.instance)
        {
            Some(group) => group.channels.push(channel),
            None => groups.push(ChannelGroup {
                instance: channel.id.instance.clone(),
                name: channel.id.instance.to_uppercase(),
                channels: vec![channel],
            }),
        }
    }
    groups
}

/// The `why` of a detail opened from the selector (the flight recorder's
/// `open`; ◀ ▶ recorded `step`).
pub const PICK_WHY: &str = "pick";

/// What a pick opens: the strip, the open's `why` ([`PICK_WHY`]) and the
/// keys the recorder names it by.
#[derive(Debug, Clone, PartialEq)]
pub struct Picked {
    pub strip: Strip,
    pub why: &'static str,
    pub keys: Vec<String>,
}

/// A pick of `strip` in the selector.
pub fn picked(strip: &Strip) -> Picked {
    Picked {
        strip: strip.clone(),
        why: PICK_WHY,
        keys: super::detail_keys(strip),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::sectors::SectorLayout;
    use serde_json::{Value, json};

    fn strip(instance: &str, name: &str) -> Value {
        json!({"kind": "strip",
               "binding": {"instance": instance, "anchor": {"kind": "track", "name": name}},
               "strip_kind": "standard"})
    }

    fn strip_of(value: Value) -> Strip {
        let control: fohmixer_proto::layout::Control =
            serde_json::from_value(value).expect("the control parses");
        let fohmixer_proto::layout::Control::Strip(strip) = control else {
            panic!("not a strip");
        };
        *strip
    }

    /// The pool: band Vox 1 #, master Hand1 #, band Keys 1, a linked
    /// parameter fader (no strip), a marker strip labelled by its Tuner,
    /// mbc CG.
    fn layout() -> Layout {
        let mut marker = strip("band", "x");
        marker["binding"]["anchor"] = json!({"kind": "track_at", "index": 4});
        marker["label"] = json!("Rev ST");
        let pod = json!({"kind": "param_fader", "label": "Podklady",
                         "link": {"instance": "band", "id": "POD"}});
        serde_json::from_value(json!({
            "schema": 2,
            "default_page": "foh",
            "pages": [
                {"id": "foh", "title": "FOH",
                 "rows": [{"sections": [{"kind": "group", "id": "g", "controls": [
                     strip("band", "Vox 1 #"), strip("master", "Hand1 #"), strip("band", "Keys 1"),
                     pod, marker, strip("mbc", "CG")
                 ]}]}]}
            ]
        }))
        .expect("the layout parses")
    }

    fn sref(instance: &str, label: &str) -> StripRef {
        StripRef {
            instance: instance.to_string(),
            label: label.to_string(),
        }
    }

    /// lu holds Vox 1 # and Rev ST; rd's second layout holds CG.
    fn doc() -> SectorsDoc {
        let sl = |id: &str, strips: Vec<StripRef>| SectorLayout {
            id: id.to_string(),
            title: id.to_uppercase(),
            strips: strips.into_iter().map(Some).collect(),
        };
        SectorsDoc {
            rev: 1,
            sectors: [
                (
                    SectorId::Lu,
                    vec![sl(
                        "lu-1",
                        vec![sref("band", "Vox 1 #"), sref("band", "Rev ST")],
                    )],
                ),
                (
                    SectorId::Rd,
                    vec![sl("rd-1", vec![]), sl("rd-2", vec![sref("mbc", "CG")])],
                ),
            ]
            .into_iter()
            .collect(),
        }
    }

    /// Each group as `NAME: label[ placed][ current], …`.
    fn shape(groups: &[ChannelGroup]) -> Vec<String> {
        groups
            .iter()
            .map(|g| {
                let channels: Vec<String> = g
                    .channels
                    .iter()
                    .map(|c| {
                        let placed = if c.placed { " placed" } else { "" };
                        let current = if c.current { " current" } else { "" };
                        format!("{}{placed}{current}", c.label)
                    })
                    .collect();
                format!("{}: {}", g.name, channels.join(", "))
            })
            .collect()
    }

    #[test]
    fn every_pool_strip_grouped_by_live_set_placed_and_current_marked() {
        let layout = layout();
        let doc = doc();
        let keys = strip_of(strip("band", "Keys 1"));
        let groups = channel_groups(Some(&doc), &layout, &keys);
        assert_eq!(
            shape(&groups),
            vec![
                "BAND: Vox placed, Keys current, Rev ST placed",
                "MASTER: Hand1",
                "MBC: CG placed",
            ]
        );
        assert_eq!(groups[0].instance, "band");
        assert_eq!(groups[1].key(), "master|Hand1 #:false:false");
        assert_eq!(
            groups[0].key(),
            "band|Vox 1 #:true:false|Keys 1:false:true|Rev ST:true:false"
        );
        assert_eq!(groups[0].channels[1].id, sref("band", "Keys 1"));
        assert_eq!(groups[0].channels[1].strip, keys);
    }

    #[test]
    fn no_list_while_the_document_places_no_fader() {
        let layout = layout();
        let keys = strip_of(strip("band", "Keys 1"));
        assert_eq!(channel_groups(None, &layout, &keys), Vec::new());
        assert_eq!(
            channel_groups(Some(&SectorsDoc::empty()), &layout, &keys),
            Vec::new()
        );
    }

    #[test]
    fn a_strip_with_no_identity_is_no_ones_current() {
        let layout = layout();
        let mut marker = strip("band", "x");
        marker["binding"]["anchor"] = json!({"kind": "track_at", "index": 2});
        marker["label"] = json!("#3");
        let groups = channel_groups(Some(&doc()), &layout, &strip_of(marker));
        assert!(groups.iter().flat_map(|g| &g.channels).all(|c| !c.current));
        assert_eq!(groups.iter().map(|g| g.channels.len()).sum::<usize>(), 5);
    }

    #[test]
    fn a_pick_opens_its_strip_recorded_as_pick() {
        let hand = strip_of(strip("master", "Hand1 #"));
        let picked = picked(&hand);
        assert_eq!(picked.strip, hand);
        assert_eq!(picked.why, "pick");
        assert_eq!(PICK_WHY, "pick");
        assert_eq!(picked.keys, super::super::detail_keys(&hand));
        assert!(!picked.keys.is_empty());
    }
}
