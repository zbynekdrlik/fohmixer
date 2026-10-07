//! What the surface subscribes (spec §2.5, S4 design note §4–§5; schema 2,
//! #21): each control's Live properties through the general binding form,
//! and the set of the controls on screen: the page's rail and rows with the
//! selected sub-page, and the global controls. Switching a page changes the
//! set, and the store subscribes the difference, so hidden pages hold no
//! Live listeners.

use std::collections::BTreeMap;

use fohmixer_proto::client::hub_key;
use fohmixer_proto::layout::{
    Anchor, Binding, Control, Group, Layout, LayoutConfig, MeterSource, Page, ParamTarget, Row,
    Section, Strip,
};

/// One subscription: an instance, a LOM target, a property and whether
/// Live's display string comes with the value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubSpec {
    pub instance: String,
    pub target: String,
    pub prop: String,
    pub display: bool,
}

impl SubSpec {
    pub fn new(instance: &str, target: String, prop: &str, display: bool) -> Self {
        Self {
            instance: instance.to_string(),
            target,
            prop: prop.to_string(),
            display,
        }
    }

    /// The hub's key of this subscription (the `sub` of its values).
    pub fn key(&self) -> String {
        hub_key(&self.instance, &self.target, &self.prop, self.display)
    }
}

/// The LOM target of `binding` followed by `suffix` (a path relative to it,
/// may be empty); `None` when the binding's path does not parse.
pub fn target_of(binding: &Binding, suffix: &str) -> Option<String> {
    let base = binding.target().ok()?;
    Some(if suffix.is_empty() {
        base
    } else {
        format!("{base} {suffix}")
    })
}

/// `prop` of `binding`'s target followed by `suffix`.
fn spec(binding: &Binding, suffix: &str, prop: &str, display: bool) -> Option<SubSpec> {
    target_of(binding, suffix).map(|t| SubSpec::new(&binding.instance, t, prop, display))
}

/// A strip's subscriptions, by part.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StripSubs {
    /// The volume with its display string (the fader and the dB text).
    pub volume: Option<SubSpec>,
    pub pan: Option<SubSpec>,
    pub mute: Option<SubSpec>,
    /// One meter bar (`level`) or two (`lr`).
    pub meters: Vec<SubSpec>,
    /// The track's colour in Live (the name button, #21): never gates the
    /// strip (I8 gates on its values only).
    pub color: Option<SubSpec>,
}

impl StripSubs {
    /// Every subscription of the strip.
    pub fn all(&self) -> Vec<SubSpec> {
        let mut out: Vec<SubSpec> = [&self.volume, &self.pan, &self.mute]
            .into_iter()
            .flatten()
            .cloned()
            .collect();
        out.extend(self.meters.iter().cloned());
        out.extend(self.color.iter().cloned());
        out
    }
}

/// The meter properties of a meter source (spec X2).
pub fn meter_props(source: MeterSource) -> &'static [&'static str] {
    match source {
        MeterSource::Level => &["output_meter_level"],
        MeterSource::Lr => &["output_meter_left", "output_meter_right"],
    }
}

/// The subscriptions of a strip: every part is drawn (schema 2).
pub fn strip_subs(strip: &Strip, source: MeterSource) -> StripSubs {
    let b = &strip.binding;
    StripSubs {
        volume: spec(b, "mixer_device volume", "value", true),
        pan: spec(b, "mixer_device panning", "value", false),
        mute: spec(b, "", "mute", false),
        meters: meter_props(source)
            .iter()
            .filter_map(|prop| spec(b, "", prop, false))
            .collect(),
        color: spec(b, "", "color", false),
    }
}

/// The subscription of a solo button: its group track's solo.
pub fn solo_sub(binding: &Binding) -> Option<SubSpec> {
    spec(binding, "", "solo", false)
}

/// The subscription of the stage-mic button and TechAlert: the track's
/// mute.
pub fn mute_sub(binding: &Binding) -> Option<SubSpec> {
    spec(binding, "", "mute", false)
}

/// The subscriptions of a former MIDI control's targets, in target order
/// (`None` where a path does not parse); a fader's first target brings
/// Live's display string.
pub fn param_subs(targets: &[ParamTarget], fader: bool) -> Vec<Option<SubSpec>> {
    targets
        .iter()
        .enumerate()
        .map(|(i, t)| spec(&t.binding, "", &t.prop, fader && i == 0))
        .collect()
}

/// The subscriptions of one control: what its component subscribes (both
/// use the functions above, so the two cannot drift apart).
pub fn control_subs(control: &Control, source: MeterSource) -> Vec<SubSpec> {
    match control {
        Control::Strip(strip) => strip_subs(strip, source).all(),
        Control::Solo { binding, .. } => solo_sub(binding).into_iter().collect(),
        Control::Stage { binding, .. } | Control::Alert { binding, .. } => {
            mute_sub(binding).into_iter().collect()
        }
        Control::ParamToggle { targets, .. } => {
            param_subs(targets, false).into_iter().flatten().collect()
        }
        Control::ParamFader { targets, .. } => {
            param_subs(targets, true).into_iter().flatten().collect()
        }
        Control::HubToggle { .. } | Control::Text { .. } => Vec::new(),
    }
}

/// The groups of a row on screen: its groups, and in a pager's place the
/// groups of its sub-page `sub` (index; a missing one shows nothing).
pub fn shown_groups(row: &Row, sub: Option<usize>) -> Vec<&Group> {
    row.sections
        .iter()
        .flat_map(|section| match section {
            Section::Group(group) => vec![group],
            Section::Pager(pager) => sub
                .and_then(|i| pager.pages.get(i))
                .map(|page| page.sections.iter().flat_map(Section::groups).collect())
                .unwrap_or_default(),
        })
        .collect()
}

/// The controls on screen for a selection `path` (from [`selected_path`]:
/// the page, then its pager's sub-page): the page's rail, its rows with the
/// selected sub-page, then the global controls.
pub fn visible_controls<'a>(layout: &'a Layout, path: &[usize]) -> Vec<&'a Control> {
    let mut out: Vec<&Control> = Vec::new();
    if let Some(page) = path.first().and_then(|i| layout.pages.get(*i)) {
        out.extend(page.rail.iter());
        let sub = path.get(1).copied();
        for row in &page.rows {
            for group in shown_groups(row, sub) {
                out.extend(group.controls.iter());
            }
        }
    }
    out.extend(layout.global.iter());
    out
}

/// Every subscription on screen, each key once.
pub fn visible_subs(layout: &Layout, path: &[usize]) -> Vec<SubSpec> {
    let source = layout.config.meter_source.unwrap_or_default();
    let unique: BTreeMap<String, SubSpec> = visible_controls(layout, path)
        .into_iter()
        .flat_map(|c| control_subs(c, source))
        .map(|s| (s.key(), s))
        .collect();
    unique.into_values().collect()
}

/// The solos of a page (its rail and every group, every sub-page): what the
/// SOLO ✕ pill clears.
pub fn page_solos(page: &Page) -> Vec<Binding> {
    page.controls()
        .into_iter()
        .filter_map(|c| match c {
            Control::Solo { binding, .. } => Some(binding.clone()),
            _ => None,
        })
        .collect()
}

/// The index of the page with id `id` among `ids`.
fn index_of<'a>(mut ids: impl Iterator<Item = &'a String>, id: &str) -> Option<usize> {
    ids.position(|p| p == id)
}

/// The page shown and its pager's sub-page: the page remembered for the root
/// (key `""`) and for the page's pager (key: the page's id) while they still
/// exist, else the defaults.
pub fn selected_path(layout: &Layout, remembered: &BTreeMap<String, String>) -> Vec<usize> {
    let ids = || layout.pages.iter().map(|p| &p.id);
    let Some(page) = remembered
        .get("")
        .and_then(|id| index_of(ids(), id))
        .or_else(|| index_of(ids(), &layout.default_page))
        .or_else(|| (!layout.pages.is_empty()).then_some(0))
    else {
        return Vec::new();
    };
    let mut path = vec![page];
    let holder = &layout.pages[page];
    if let Some(pager) = holder.pager().filter(|p| !p.pages.is_empty()) {
        let ids = || pager.pages.iter().map(|p| &p.id);
        let sub = remembered
            .get(&holder.id)
            .and_then(|id| index_of(ids(), id))
            .or_else(|| index_of(ids(), &pager.default_page))
            .unwrap_or(0);
        path.push(sub);
    }
    path
}

/// Remembers page `index` of level `level` (0: the pages; 1: the sub-pages of
/// the pager of the page `path[0]`).
pub fn choose(
    layout: &Layout,
    remembered: &mut BTreeMap<String, String>,
    path: &[usize],
    level: usize,
    index: usize,
) {
    if level == 0 {
        if let Some(page) = layout.pages.get(index) {
            remembered.insert(String::new(), page.id.clone());
        }
        return;
    }
    let Some(holder) = path.first().and_then(|i| layout.pages.get(*i)) else {
        return;
    };
    if let Some(sub) = holder.pager().and_then(|p| p.pages.get(index)) {
        remembered.insert(holder.id.clone(), sub.id.clone());
    }
}

/// The group tracks the page unfolds once loaded (spec F7): each
/// target's instance and LOM target.
pub fn unfold_targets(config: &LayoutConfig) -> Vec<(String, String)> {
    config
        .unfold
        .iter()
        .filter_map(|u| {
            let binding = Binding {
                instance: u.instance.clone(),
                anchor: Anchor::Track {
                    name: u.name.clone(),
                },
                path: None,
            };
            target_of(&binding, "").map(|t| (u.instance.clone(), t))
        })
        .collect()
}

#[cfg(test)]
mod tests;
