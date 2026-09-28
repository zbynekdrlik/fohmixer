//! What the surface subscribes (spec §2.5, S4 design note §4–§5): each
//! item's Live properties through the general binding form, and the set of
//! the pages on screen. Only the visible pages and the overlay are
//! subscribed; switching a page changes the set, and the store subscribes
//! the difference, so hidden pages hold no Live listeners.

use std::collections::BTreeMap;

use fohmixer_proto::client::hub_key;
use fohmixer_proto::layout::{
    Anchor, Binding, Item, ItemKind, Layout, LayoutConfig, MeterSource, Page, Strip,
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

/// The subscriptions of a strip's present parts.
pub fn strip_subs(strip: &Strip, source: MeterSource) -> StripSubs {
    let c = &strip.children;
    let b = &strip.binding;
    let volume = (c.fader.is_some() || c.db.is_some())
        .then(|| spec(b, "mixer_device volume", "value", true))
        .flatten();
    let pan = c
        .pan
        .and_then(|_| spec(b, "mixer_device panning", "value", false));
    let mute = c.mute.and_then(|_| spec(b, "", "mute", false));
    let meters = if c.meter.is_some() {
        meter_props(source)
            .iter()
            .filter_map(|prop| spec(b, "", prop, false))
            .collect()
    } else {
        Vec::new()
    };
    StripSubs {
        volume,
        pan,
        mute,
        meters,
    }
}

/// The subscriptions of one item.
pub fn item_subs(item: &Item, source: MeterSource) -> Vec<SubSpec> {
    match &item.kind {
        ItemKind::Strip(strip) => strip_subs(strip, source).all(),
        ItemKind::Solo { binding } => spec(binding, "", "solo", false).into_iter().collect(),
        ItemKind::Stage { binding, .. } | ItemKind::Alert { binding, .. } => {
            spec(binding, "", "mute", false).into_iter().collect()
        }
        ItemKind::ParamToggle { targets, .. } => targets
            .iter()
            .filter_map(|t| spec(&t.binding, "", &t.prop, false))
            .collect(),
        ItemKind::ParamFader { targets, .. } => targets
            .iter()
            .enumerate()
            .filter_map(|(i, t)| spec(&t.binding, "", &t.prop, i == 0))
            .collect(),
        ItemKind::Area { .. }
        | ItemKind::HubToggle { .. }
        | ItemKind::Refresh { .. }
        | ItemKind::Label { .. } => Vec::new(),
    }
}

/// The subscriptions of a page's own items (not of its pager's pages).
pub fn page_subs(page: &Page, source: MeterSource) -> Vec<SubSpec> {
    page.items
        .iter()
        .flat_map(|item| item_subs(item, source))
        .collect()
}

/// The pages on screen: `path[0]` of the root pages, then `path[1]` of that
/// page's pager, and so on (a path from [`selected_path`]).
pub fn visible_pages<'a>(layout: &'a Layout, path: &[usize]) -> Vec<&'a Page> {
    let mut out = Vec::new();
    let mut level = layout.pages.as_slice();
    for index in path {
        let Some(page) = level.get(*index) else {
            break;
        };
        out.push(page);
        level = page
            .pager
            .as_ref()
            .map(|p| p.pages.as_slice())
            .unwrap_or_default();
    }
    out
}

/// Every subscription on screen: the overlay's and the visible pages',
/// each key once.
pub fn visible_subs(layout: &Layout, path: &[usize]) -> Vec<SubSpec> {
    let source = layout.config.meter_source.unwrap_or_default();
    let overlay = layout.overlay.iter().flat_map(|i| item_subs(i, source));
    let pages = visible_pages(layout, path)
        .into_iter()
        .flat_map(|p| page_subs(p, source));
    let unique: BTreeMap<String, SubSpec> = overlay.chain(pages).map(|s| (s.key(), s)).collect();
    unique.into_values().collect()
}

/// How deep pagers may nest (a bound for the walk, far above any layout).
const MAX_DEPTH: usize = 8;

/// The page shown on each level: the page remembered for its pager (keyed
/// by the id of the page holding the pager, `""` for the root pages) while
/// it still exists, else the level's default page.
pub fn selected_path(layout: &Layout, remembered: &BTreeMap<String, String>) -> Vec<usize> {
    let mut path = Vec::new();
    let mut level = layout.pages.as_slice();
    let mut default = layout.tabbar.default_page;
    let mut parent = String::new();
    for _ in 0..MAX_DEPTH {
        if level.is_empty() {
            break;
        }
        let wanted = remembered
            .get(&parent)
            .and_then(|id| level.iter().position(|p| p.id == *id));
        let index = wanted.unwrap_or(default).min(level.len() - 1);
        path.push(index);
        let page = &level[index];
        let Some(pager) = &page.pager else {
            break;
        };
        parent = page.id.clone();
        level = pager.pages.as_slice();
        default = pager.tabbar.default_page;
    }
    path
}

/// Remembers page `index` of level `level` (on the pages of `path`) as the
/// choice of its pager.
pub fn choose(
    layout: &Layout,
    remembered: &mut BTreeMap<String, String>,
    path: &[usize],
    level: usize,
    index: usize,
) {
    let (parent, pages) = if level == 0 {
        (String::new(), layout.pages.as_slice())
    } else {
        let shown = visible_pages(layout, path);
        let Some(holder) = shown.get(level - 1) else {
            return;
        };
        let Some(pager) = &holder.pager else {
            return;
        };
        (holder.id.clone(), pager.pages.as_slice())
    };
    if let Some(page) = pages.get(index) {
        remembered.insert(parent, page.id.clone());
    }
}

/// The group tracks REFRESH ALL unfolds (spec F7): each target's instance
/// and LOM target.
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
