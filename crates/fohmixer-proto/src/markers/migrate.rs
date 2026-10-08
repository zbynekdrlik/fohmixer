//! The migration of a frame whose strips are bound by track name to Tuner
//! markers (#68 PR C): every group holding such strips becomes a `tags`
//! group of the same title, place and other controls, and each strip's track
//! gets the marker that puts the same strip there again. The hub names the
//! owner's plain Tuners with these markers; the CLI writes the frame.
//!
//! What a marker cannot carry is a problem, never dropped silently: the hub
//! renames nothing and the CLI writes nothing while the frame has one.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{MAX_GROUP_NAME, MAX_PLACE, TrackKind};
use crate::layout::{Anchor, Control, Group, Layout, Section, Strip, StripKind, strip_label};

/// A track's planned marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Planned {
    pub instance: String,
    pub kind: TrackKind,
    /// The track's name (the frame binds the strip by it).
    pub track: String,
    /// The Tuner name the track's marker gets.
    pub marker: String,
}

/// A planned track as the hub finds it in Live (`/api/markers/migration`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationRow {
    pub instance: String,
    pub kind: TrackKind,
    pub track: String,
    pub marker: String,
    /// The tracks of that kind bearing the name: 1 found, 0 missing, more
    /// ambiguous (the duplicate names the markers end).
    pub matches: u32,
    /// The track's index when exactly one bears the name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    /// Its Tuners whose name is no marker.
    pub plain: u32,
    /// Its marker Tuners.
    pub markers: u32,
    /// One of its Tuners bears the planned marker.
    pub done: bool,
}

impl MigrationRow {
    /// Whether the hub names its Tuner: one track bears the name, it holds
    /// one plain Tuner and no marker yet.
    pub fn ready(&self) -> bool {
        self.matches == 1 && self.plain == 1 && self.markers == 0
    }
}

/// `/api/markers/migration`'s answer: every planned track, the frame's
/// problems (no rename while there is one), whether a read of the tracks
/// was still running (no rename then either), and for a `POST` how many
/// Tuners it asked Live to rename.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationStatus {
    pub rows: Vec<MigrationRow>,
    #[serde(default)]
    pub problems: Vec<String>,
    #[serde(default)]
    pub reading: bool,
    #[serde(default)]
    pub renamed: u32,
}

/// The converted frame, the markers that fill it, and what they could not
/// carry over.
#[derive(Debug, Clone, PartialEq)]
pub struct Migration {
    pub frame: Layout,
    pub planned: Vec<Planned>,
    /// Each a sentence naming the track or the group: the surface would
    /// change if the migration went on.
    pub problems: Vec<String>,
}

/// A planned track before its label is chosen.
struct Track {
    instance: String,
    kind: TrackKind,
    name: String,
    groups: Vec<(String, usize)>,
    /// Each of its strips' pin and mute guard.
    pins: Vec<bool>,
    guards: Vec<bool>,
}

/// The frame with every group of strips bound by name turned into a `tags`
/// group, and the marker of each of those strips' tracks. A group with a
/// strip bound otherwise (by index, the master) stays as it is.
pub fn migration(frame: &Layout) -> Migration {
    let mut out = frame.clone();
    let mut tracks: Vec<Track> = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    // The frame's own tag groups keep their names.
    let mut used: BTreeSet<String> = groups(frame)
        .iter()
        .filter_map(|g| g.tags.clone())
        .collect();
    for page in &mut out.pages {
        for row in &mut page.rows {
            for section in &mut row.sections {
                match section {
                    Section::Group(group) => {
                        migrate_group(group, &mut used, &mut tracks, &mut problems);
                    }
                    Section::Pager(pager) => {
                        for sub in &mut pager.pages {
                            for inner in &mut sub.sections {
                                if let Section::Group(group) = inner {
                                    migrate_group(group, &mut used, &mut tracks, &mut problems);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    for track in &tracks {
        if track.pins.iter().any(|p| *p != track.pins[0]) {
            problems.push(format!(
                "{:?} ({}): pinned in one group and not in another (a marker pins it in all)",
                track.name, track.instance
            ));
        }
        if track.guards.iter().any(|g| *g != track.guards[0]) {
            problems.push(format!(
                "{:?} ({}): mute-guarded in one group and not in another (a marker guards it in all)",
                track.name, track.instance
            ));
        }
    }
    let labels = labels(&tracks);
    let planned = tracks
        .into_iter()
        .zip(labels)
        .map(|(track, label)| Planned {
            marker: marker_name(
                &label,
                &track.groups,
                track.pins.contains(&true),
                track.guards.contains(&true),
            ),
            instance: track.instance,
            kind: track.kind,
            track: track.name,
        })
        .collect();
    Migration {
        frame: out,
        planned,
        problems,
    }
}

/// Every group of the layout, its pagers' sub-pages' too.
fn groups(layout: &Layout) -> Vec<&Group> {
    layout
        .pages
        .iter()
        .flat_map(|p| &p.rows)
        .flat_map(|r| &r.sections)
        .flat_map(Section::groups)
        .collect()
}

/// The strip and its track's instance, kind and name, when it is bound by
/// name; none otherwise.
fn by_name(control: &Control) -> Option<(&Strip, TrackKind, &str)> {
    let Control::Strip(strip) = control else {
        return None;
    };
    let (kind, name) = match &strip.binding.anchor {
        Anchor::Track { name } => (TrackKind::Track, name),
        Anchor::Return { name } => (TrackKind::Return, name),
        _ => return None,
    };
    Some((strip.as_ref(), kind, name.as_str()))
}

/// Turns `group` into a tags group when every strip in it is bound by name
/// (and it holds one at least), recording its strips' tracks and what a
/// marker cannot carry.
fn migrate_group(
    group: &mut Group,
    used: &mut BTreeSet<String>,
    tracks: &mut Vec<Track>,
    problems: &mut Vec<String>,
) {
    let is_strip = |c: &Control| matches!(c, Control::Strip(_));
    let strips: Vec<&Control> = group.controls.iter().filter(|c| is_strip(c)).collect();
    if group.tags.is_some() || strips.is_empty() || strips.iter().any(|c| by_name(c).is_none()) {
        return;
    }
    let base = group
        .title
        .as_deref()
        .map(tag_name)
        .filter(|t| !t.is_empty())
        .or_else(|| group.id.as_deref().map(tag_name).filter(|t| !t.is_empty()))
        .unwrap_or_else(|| "GROUP".to_string());
    let tag = unique(&base, used);
    // The marker strips come first, then the group's other controls.
    let before_last = group
        .controls
        .iter()
        .rposition(is_strip)
        .map_or(&[][..], |last| &group.controls[..last]);
    if before_last.iter().any(|c| !is_strip(c)) {
        problems.push(format!(
            "group {tag}: a control before or between its strips would move after them"
        ));
    }
    let mut seen: Vec<(&str, TrackKind, &str)> = Vec::new();
    for (i, (strip, kind, name)) in strips.iter().copied().filter_map(by_name).enumerate() {
        let instance = strip.binding.instance.as_str();
        if seen.contains(&(instance, kind, name)) {
            problems.push(format!(
                "{name:?} ({instance}): twice in group {tag} (a marker shows it once)"
            ));
            continue;
        }
        seen.push((instance, kind, name));
        // A marker's strip is never wide, labels itself, binds no path and
        // takes its kind from its track's.
        let kind_of_track = match kind {
            TrackKind::Track => StripKind::Standard,
            TrackKind::Return => StripKind::Return,
        };
        if strip.wide
            || strip.label.is_some()
            || strip.binding.path.is_some()
            || strip.strip_kind != kind_of_track
        {
            problems.push(format!(
                "{name:?} ({instance}): its width, label, path or kind would be lost"
            ));
        }
        let at = tracks
            .iter()
            .position(|t| t.instance == instance && t.kind == kind && t.name == name);
        let track = match at {
            Some(at) => &mut tracks[at],
            None => {
                tracks.push(Track {
                    instance: instance.to_string(),
                    kind,
                    name: name.to_string(),
                    groups: Vec::new(),
                    pins: Vec::new(),
                    guards: Vec::new(),
                });
                tracks.last_mut().expect("just pushed")
            }
        };
        track.groups.push((tag.clone(), i + 1));
        track.pins.push(strip.pinned);
        track.guards.push(strip.mute_guard);
    }
    group.controls.retain(|c| !is_strip(c));
    group.tags = Some(tag);
}

/// A group name from a title or an id: its letters uppercased with their
/// accents dropped, digits, `-` and `_` kept, a run of spaces (or anything
/// else) one `_`, none at either end, at most [`MAX_GROUP_NAME`] characters.
pub fn tag_name(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().flat_map(char::to_uppercase) {
        let kept = plain_letter(c).or_else(|| {
            (c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-' || c == '_').then_some(c)
        });
        match kept {
            Some(k) => out.push(k),
            None if !out.ends_with('_') => out.push('_'),
            None => {}
        }
    }
    let trimmed: String = out.trim_matches('_').chars().take(MAX_GROUP_NAME).collect();
    trimmed.trim_end_matches('_').to_string()
}

/// The plain capital of a Slovak or Czech accented capital.
fn plain_letter(c: char) -> Option<char> {
    let plain = match c {
        'Á' | 'Ä' => 'A',
        'Č' => 'C',
        'Ď' => 'D',
        'É' | 'Ě' => 'E',
        'Í' => 'I',
        'Ĺ' | 'Ľ' => 'L',
        'Ň' => 'N',
        'Ó' | 'Ô' | 'Ö' => 'O',
        'Ŕ' | 'Ř' => 'R',
        'Š' => 'S',
        'Ť' => 'T',
        'Ú' | 'Ů' | 'Ü' => 'U',
        'Ý' => 'Y',
        'Ž' => 'Z',
        _ => return None,
    };
    Some(plain)
}

/// `base`, or `base_2`, `base_3` … when taken (cut to fit the name's length).
/// Of `used.len() + 1` candidates one is free, so the search is bounded.
fn unique(base: &str, used: &mut BTreeSet<String>) -> String {
    let name = (1..=used.len() + 1)
        .map(|n| {
            if n == 1 {
                base.to_string()
            } else {
                let suffix = format!("_{n}");
                let keep = MAX_GROUP_NAME - suffix.len();
                format!("{}{suffix}", &base[..base.len().min(keep)])
            }
        })
        .find(|name| !used.contains(name))
        .expect("one of the candidates is free");
    used.insert(name.clone());
    name
}

/// A track's name without its return prefix (`X-`), its `#` marks and a
/// `"` (a label holds none).
pub fn full_label(name: &str) -> String {
    let name = name.replace('"', "");
    let mut chars = name.trim_start().chars();
    let prefixed = matches!(
        (chars.next(), chars.next()),
        (Some(letter), Some('-')) if letter.is_alphabetic()
    );
    let rest = if prefixed {
        chars.as_str()
    } else {
        name.trim_start()
    };
    let words: Vec<&str> = rest.split_whitespace().filter(|w| *w != "#").collect();
    words.join(" ").trim_end_matches('#').trim_end().to_string()
}

/// Each track's label: what its strip shows now ([`strip_label`]); where two
/// would be equal (both strips would be a KONFLIKT), the track's name
/// without its marks ([`full_label`]); where still equal, a number after it.
fn labels(tracks: &[Track]) -> Vec<String> {
    let first: Vec<String> = tracks
        .iter()
        .map(|t| strip_label(&t.name).replace('"', ""))
        .collect();
    let second: Vec<String> = tracks
        .iter()
        .zip(&first)
        .map(|(t, label)| {
            if first.iter().filter(|l| *l == label).count() > 1 {
                full_label(&t.name)
            } else {
                label.clone()
            }
        })
        .collect();
    let mut out: Vec<String> = Vec::new();
    for label in &second {
        let label = if label.is_empty() {
            "?"
        } else {
            label.as_str()
        };
        // Each label already given rules out one candidate, and so does
        // each other track's own label: of `out + second` candidates
        // (`second` holds this track's own) one is free.
        let name = (1..=out.len() + second.len())
            .map(|n| {
                if n == 1 {
                    label.to_string()
                } else {
                    format!("{label} {n}")
                }
            })
            .find(|name| !out.contains(name) && (name == label || !second.contains(name)))
            .expect("one of the candidates is free");
        out.push(name);
    }
    out
}

/// The Tuner name: the quoted label, a `+G:` tag per group with its place
/// (none past [`MAX_PLACE`]), then `+PIN` and `+MG` when set.
pub fn marker_name(label: &str, groups: &[(String, usize)], pin: bool, mute_guard: bool) -> String {
    let mut out = format!("\"{label}\"");
    for (group, place) in groups {
        if u32::try_from(*place).is_ok_and(|p| p <= MAX_PLACE) {
            out.push_str(&format!(" +G:{group}:{place}"));
        } else {
            out.push_str(&format!(" +G:{group}"));
        }
    }
    if pin {
        out.push_str(" +PIN");
    }
    if mute_guard {
        out.push_str(" +MG");
    }
    out
}

#[cfg(test)]
mod tests;
