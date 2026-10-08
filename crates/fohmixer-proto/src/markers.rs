//! Tuner markers (#68, spec D16): what is on the surface comes from a Tuner
//! in each track's device chain, renamed with the strip's label and its
//! tags, AbleSet style: `"Vox 1" +G:VOCALS:2 +G:TALKSHOW:1 +PIN +MG`.
//!
//! - [`parse`] reads one Tuner's name: the quoted label, the groups with
//!   their places, `+PIN`, `+MG`, and every problem as a value (never an
//!   error: a problem is shown on the strip, I9).
//! - [`compose`] turns the frame (the layout file: pages, rows, the rail)
//!   and the markers the hub found into the served layout: a frame group
//!   with `tags` shows that tag group's strips, every other tag group
//!   becomes a view (a page flagged `view`), equal labels are conflicts.
//!
//! Pure, shared by the hub, `layout check` and the tests.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::layout::{
    Anchor, Binding, Control, Group, Layout, LayoutError, Page, Row, Section, Strip, StripKind,
    StripMark,
};

/// The `class_name` of Live's Tuner (only a Tuner can be a marker).
pub const TUNER_CLASS: &str = "Tuner";
/// The longest group name a `+G:` tag takes.
pub const MAX_GROUP_NAME: usize = 32;
/// The largest place a `+G:NAME:N` tag takes.
pub const MAX_PLACE: u32 = 999;
/// A view page's id: this prefix and its group's name.
pub const VIEW_PREFIX: &str = "view-";

/// A group a marker puts its strip in, and its place there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupTag {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<u32>,
}

/// What one Tuner's name says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marker {
    /// The quoted label; none when it is missing or empty (a problem).
    pub label: Option<String>,
    /// The `+G:` tags, in their order, each name once.
    pub groups: Vec<GroupTag>,
    /// `+PIN`: always on screen (D15).
    pub pin: bool,
    /// `+MG`: the mute guard (F12).
    pub mute_guard: bool,
    pub problems: Vec<TagProblem>,
}

/// A problem of a marker or of its place among the others.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum TagProblem {
    /// The name does not start with a quoted label.
    NoLabel,
    /// The label is `""`.
    EmptyLabel,
    /// The label's closing quote is missing.
    UnclosedLabel,
    /// A `+` tag fohmixer does not know (AbleSet's are uppercase too).
    UnknownTag { tag: String },
    /// A `+G:` tag whose name or place is not valid.
    BadGroup { tag: String },
    /// The same group named twice (the first counts).
    DuplicateGroup { name: String },
    /// Text outside the label that is no tag.
    StrayText { text: String },
    /// No `+G:` tag: the strip shows on no page.
    NoGroup,
    /// Another marker has the same label: both strips are disabled.
    Conflict { label: String },
    /// The track holds more than one marker (the first counts).
    DoubleTuner,
    /// Another strip of the group has the same place.
    SamePlace { group: String, place: u32 },
}

/// Whether a Tuner's name makes it a marker: a quote or a `+` token.
pub fn is_marker(name: &str) -> bool {
    name.contains('"') || name.split_whitespace().any(|t| t.starts_with('+'))
}

/// Whether `name` is a valid group name: `A-Z`, `0-9`, `-`, `_`, 1 to
/// [`MAX_GROUP_NAME`] characters.
pub fn valid_group_name(name: &str) -> bool {
    (1..=MAX_GROUP_NAME).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// A group's title as shown: its name with `_` as a space (AbleSet's rule).
pub fn group_title(name: &str) -> String {
    name.replace('_', " ")
}

/// A `+G:` tag's argument: `NAME` or `NAME:N`.
fn group_tag(arg: &str) -> Option<GroupTag> {
    let mut parts = arg.split(':');
    let name = parts.next()?;
    let place = match parts.next() {
        None => None,
        Some(text) => {
            let place: u32 = text.parse().ok()?;
            if !(1..=MAX_PLACE).contains(&place) {
                return None;
            }
            Some(place)
        }
    };
    if parts.next().is_some() || !valid_group_name(name) {
        return None;
    }
    Some(GroupTag {
        name: name.to_string(),
        place,
    })
}

/// Reads one Tuner's name.
pub fn parse(name: &str) -> Marker {
    let mut marker = Marker::default();
    let text = name.trim();
    let rest = match text.strip_prefix('"') {
        Some(after) => match after.split_once('"') {
            Some((label, rest)) => {
                let label = label.trim();
                if label.is_empty() {
                    marker.problems.push(TagProblem::EmptyLabel);
                } else {
                    marker.label = Some(label.to_string());
                }
                rest
            }
            None => {
                marker.problems.push(TagProblem::UnclosedLabel);
                after
            }
        },
        None => {
            marker.problems.push(TagProblem::NoLabel);
            text
        }
    };
    for token in rest.split_whitespace() {
        let Some(tag) = token.strip_prefix('+') else {
            marker.problems.push(TagProblem::StrayText {
                text: token.to_string(),
            });
            continue;
        };
        match tag {
            "PIN" => marker.pin = true,
            "MG" => marker.mute_guard = true,
            _ => match tag.strip_prefix("G:") {
                Some(arg) => match group_tag(arg) {
                    Some(group) if marker.groups.iter().any(|g| g.name == group.name) => {
                        marker
                            .problems
                            .push(TagProblem::DuplicateGroup { name: group.name });
                    }
                    Some(group) => marker.groups.push(group),
                    None => marker.problems.push(TagProblem::BadGroup {
                        tag: token.to_string(),
                    }),
                },
                None => marker.problems.push(TagProblem::UnknownTag {
                    tag: token.to_string(),
                }),
            },
        }
    }
    if marker.groups.is_empty() {
        marker.problems.push(TagProblem::NoGroup);
    }
    marker
}

/// What a frame (the layout file) may not hold (#68): a view page or an
/// id starting with [`VIEW_PREFIX`] (the markers make those), or a `tags`
/// group with controls of its own (the composition replaces them).
pub fn frame_problems(frame: &Layout) -> Vec<LayoutError> {
    let mut out = Vec::new();
    let mut id = |at: &str, id: &str| {
        if id.starts_with(VIEW_PREFIX) {
            out.push(LayoutError {
                at: at.to_string(),
                message: format!("{id:?} starts with {VIEW_PREFIX:?}, the Tuner markers' views"),
            });
        }
    };
    for (i, page) in frame.pages.iter().enumerate() {
        let at = format!("pages[{i}]");
        id(&at, page.id.as_str());
        for (r, row) in page.rows.iter().enumerate() {
            for (s, section) in row.sections.iter().enumerate() {
                let section_at = format!("{at}.rows[{r}].sections[{s}]");
                if let Section::Pager(pager) = section {
                    id(&section_at, pager.id.as_str());
                    for sub in &pager.pages {
                        id(&section_at, sub.id.as_str());
                    }
                }
                for group in section.groups() {
                    if let Some(group_id) = &group.id {
                        id(&section_at, group_id.as_str());
                    }
                }
            }
        }
    }
    for (i, page) in frame.pages.iter().enumerate() {
        let at = format!("pages[{i}]");
        if page.view {
            out.push(LayoutError {
                at: at.clone(),
                message: "a view page: the Tuner markers make those".to_string(),
            });
        }
        let groups = page
            .rows
            .iter()
            .flat_map(|r| &r.sections)
            .flat_map(Section::groups);
        for group in groups {
            if group.tags.is_some() && !group.controls.is_empty() {
                out.push(LayoutError {
                    at: at.clone(),
                    message: format!(
                        "the tags group {:?} holds controls of its own (the markers' strips replace them)",
                        group.id.as_deref().unwrap_or("")
                    ),
                });
            }
        }
    }
    out
}

/// Whether a marker's track is a track or a return track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Track,
    Return,
}

/// A marker the hub found: where, and its Tuner's name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Found {
    pub instance: String,
    pub kind: TrackKind,
    /// The track's index in `live_set tracks` (or `return_tracks`).
    pub index: u32,
    /// The marker Tuner's name (the track's first marker).
    pub name: String,
    /// How many marker Tuners the track holds (more than one: a problem).
    pub tuners: u32,
}

impl Found {
    /// Where it is, in the order strips without a place are shown.
    fn key(&self) -> (&str, TrackKind, u32) {
        (self.instance.as_str(), self.kind, self.index)
    }
}

/// A marker's problems, for `/api/status` (I9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkerReport {
    pub instance: String,
    pub kind: TrackKind,
    pub index: u32,
    /// The Tuner's name as Live shows it.
    pub name: String,
    pub problems: Vec<TagProblem>,
}

/// The served layout and every marker problem.
#[derive(Debug, Clone, PartialEq)]
pub struct Composed {
    pub layout: Layout,
    pub problems: Vec<MarkerReport>,
}

/// One marker read: where, what it says, and every problem it has.
struct Entry<'a> {
    found: &'a Found,
    marker: Marker,
    problems: Vec<TagProblem>,
}

impl Entry<'_> {
    /// The strip it shows: bound to its track by index, its label (or the
    /// track's number when the label is missing), its mark.
    fn strip(&self) -> Strip {
        let anchor = match self.found.kind {
            TrackKind::Track => Anchor::TrackAt {
                index: self.found.index,
            },
            TrackKind::Return => Anchor::ReturnAt {
                index: self.found.index,
            },
        };
        let conflict = self
            .problems
            .iter()
            .any(|p| matches!(p, TagProblem::Conflict { .. }));
        let mark = if conflict {
            Some(StripMark::Conflict)
        } else if self.problems.is_empty() {
            None
        } else {
            Some(StripMark::Problem)
        };
        Strip {
            binding: Binding {
                instance: self.found.instance.clone(),
                anchor,
                path: None,
            },
            strip_kind: match self.found.kind {
                TrackKind::Track => StripKind::Standard,
                TrackKind::Return => StripKind::Return,
            },
            wide: false,
            mute_guard: self.marker.mute_guard,
            pinned: self.marker.pin,
            label: Some(
                self.marker
                    .label
                    .clone()
                    .unwrap_or_else(|| format!("#{}", self.found.index + 1)),
            ),
            mark,
        }
    }
}

/// Every tag group's members, by group name: the entries' indices in the
/// group's order (by place, then without a place in Live's order).
fn members(entries: &[Entry<'_>]) -> BTreeMap<String, Vec<usize>> {
    let mut groups: BTreeMap<String, Vec<(Option<u32>, usize)>> = BTreeMap::new();
    for (i, entry) in entries.iter().enumerate() {
        for tag in &entry.marker.groups {
            groups
                .entry(tag.name.clone())
                .or_default()
                .push((tag.place, i));
        }
    }
    groups
        .into_iter()
        .map(|(name, mut list)| {
            // A place first (smallest first), then none; equal: Live's order
            // (the entries are in it).
            list.sort_by_key(|&(place, i)| (place.is_none(), place, i));
            (name, list.into_iter().map(|(_, i)| i).collect())
        })
        .collect()
}

/// The tag group names a frame group shows (`Group.tags`), in document
/// order.
fn frame_tags(frame: &Layout) -> BTreeSet<String> {
    frame
        .pages
        .iter()
        .flat_map(|p| &p.rows)
        .flat_map(|r| &r.sections)
        .flat_map(Section::groups)
        .filter_map(|g| g.tags.clone())
        .collect()
}

/// Fills every frame group that names a tag group.
fn fill(sections: &mut [Section], strips: &BTreeMap<String, Vec<Strip>>) {
    for section in sections {
        match section {
            Section::Group(group) => {
                if let Some(name) = &group.tags {
                    group.controls = strips
                        .get(name)
                        .map(|list| {
                            list.iter()
                                .cloned()
                                .map(|s| Control::Strip(Box::new(s)))
                                .collect()
                        })
                        .unwrap_or_default();
                }
            }
            Section::Pager(pager) => {
                for sub in &mut pager.pages {
                    fill(&mut sub.sections, strips);
                }
            }
        }
    }
}

/// The frame and the markers as the served layout, and every problem.
pub fn compose(frame: &Layout, found: &[Found]) -> Composed {
    let mut sorted: Vec<&Found> = found.iter().collect();
    sorted.sort_by(|a, b| a.key().cmp(&b.key()));
    let mut entries: Vec<Entry<'_>> = sorted
        .into_iter()
        .map(|f| {
            let marker = parse(&f.name);
            let mut problems = marker.problems.clone();
            if f.tuners > 1 {
                problems.push(TagProblem::DoubleTuner);
            }
            Entry {
                found: f,
                marker,
                problems,
            }
        })
        .collect();
    // An equal label on two or more markers: a conflict on each.
    let mut labels: BTreeMap<String, usize> = BTreeMap::new();
    for entry in &entries {
        if let Some(label) = &entry.marker.label {
            *labels.entry(label.clone()).or_default() += 1;
        }
    }
    for entry in &mut entries {
        if let Some(label) = &entry.marker.label
            && labels.get(label).copied().unwrap_or(0) > 1
        {
            entry.problems.push(TagProblem::Conflict {
                label: label.clone(),
            });
        }
    }
    let groups = members(&entries);
    // Two strips on one place of a group: both marked.
    for (name, list) in &groups {
        let places: Vec<(usize, u32)> = list
            .iter()
            .filter_map(|&i| {
                entries[i]
                    .marker
                    .groups
                    .iter()
                    .find(|g| &g.name == name)
                    .and_then(|g| g.place)
                    .map(|place| (i, place))
            })
            .collect();
        for &(i, place) in &places {
            if places.iter().filter(|&&(_, p)| p == place).count() > 1 {
                entries[i].problems.push(TagProblem::SamePlace {
                    group: name.clone(),
                    place,
                });
            }
        }
    }
    let strips: Vec<Strip> = entries.iter().map(Entry::strip).collect();
    let by_group: BTreeMap<String, Vec<Strip>> = groups
        .iter()
        .map(|(name, list)| {
            (
                name.clone(),
                list.iter().map(|&i| strips[i].clone()).collect(),
            )
        })
        .collect();
    let mut layout = frame.clone();
    for page in &mut layout.pages {
        for row in &mut page.rows {
            fill(&mut row.sections, &by_group);
        }
    }
    // Every tag group the frame does not show is a view, in the order of
    // its first marker.
    let shown = frame_tags(frame);
    let mut views: Vec<(usize, &String)> = groups
        .iter()
        .filter(|(name, _)| !shown.contains(*name))
        .map(|(name, list)| (list.iter().copied().min().unwrap_or(0), name))
        .collect();
    views.sort();
    for (_, name) in views {
        layout
            .pages
            .push(view_page(name, &groups, &entries, &strips));
    }
    let problems = entries
        .iter()
        .filter(|e| !e.problems.is_empty())
        .map(|e| MarkerReport {
            instance: e.found.instance.clone(),
            kind: e.found.kind,
            index: e.found.index,
            name: e.found.name.clone(),
            problems: e.problems.clone(),
        })
        .collect();
    Composed { layout, problems }
}

/// A view: its group's strips, then every pinned strip not in it, under
/// the title of each one's first group (in the order of the markers).
fn view_page(
    name: &str,
    groups: &BTreeMap<String, Vec<usize>>,
    entries: &[Entry<'_>],
    strips: &[Strip],
) -> Page {
    let own = groups.get(name).cloned().unwrap_or_default();
    let mut sections = vec![Section::Group(Group {
        id: Some(format!("{VIEW_PREFIX}{name}-strips")),
        title: Some(group_title(name)),
        color: None,
        tags: None,
        controls: own
            .iter()
            .map(|&i| Control::Strip(Box::new(strips[i].clone())))
            .collect(),
    })];
    let mut pins: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let Some(first) = entry.marker.groups.first() else {
            continue;
        };
        if !entry.marker.pin || own.contains(&i) {
            continue;
        }
        match pins.iter_mut().find(|(g, _)| g == &first.name) {
            Some((_, list)) => list.push(i),
            None => pins.push((first.name.clone(), vec![i])),
        }
    }
    for (group, list) in pins {
        sections.push(Section::Group(Group {
            id: Some(format!("{VIEW_PREFIX}{name}-pins-{group}")),
            title: Some(group_title(&group)),
            color: None,
            tags: None,
            controls: list
                .iter()
                .map(|&i| Control::Strip(Box::new(strips[i].clone())))
                .collect(),
        }));
    }
    Page {
        id: format!("{VIEW_PREFIX}{name}"),
        title: group_title(name),
        view: true,
        rail: Vec::new(),
        rows: vec![Row {
            sections,
            weight: 1.0,
        }],
    }
}

#[cfg(test)]
mod tests;
