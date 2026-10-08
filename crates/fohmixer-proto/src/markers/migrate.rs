//! The migration of a frame whose strips are bound by track name to Tuner
//! markers (#68 PR C): every group holding such strips becomes a `tags`
//! group of the same title, place and other controls, and each strip's track
//! gets the marker that puts the same strip there again. The hub names the
//! owner's plain Tuners with these markers; the CLI writes the frame.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{MAX_GROUP_NAME, TrackKind};
use crate::layout::{Anchor, Control, Group, Layout, Section, strip_label};

/// The longest place a migrated group gives (a group of more strips keeps
/// the rest in Live's order: no place).
const MAX_MIGRATED_PLACE: usize = 999;

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

/// `/api/markers/migration`'s answer: every planned track; for a `POST`,
/// how many Tuners it asked Live to rename.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationStatus {
    pub rows: Vec<MigrationRow>,
    #[serde(default)]
    pub renamed: u32,
}

/// The converted frame and the markers that fill it.
#[derive(Debug, Clone, PartialEq)]
pub struct Migration {
    pub frame: Layout,
    pub planned: Vec<Planned>,
}

/// A planned track before its label is chosen.
struct Track {
    instance: String,
    kind: TrackKind,
    name: String,
    groups: Vec<(String, usize)>,
    pin: bool,
    mute_guard: bool,
}

/// The frame with every group of strips bound by name turned into a `tags`
/// group, and the marker of each of those strips' tracks. A group with a
/// strip bound otherwise (by index, the master) stays as it is.
pub fn migration(frame: &Layout) -> Migration {
    let mut out = frame.clone();
    let mut tracks: Vec<Track> = Vec::new();
    let mut used: BTreeSet<String> = BTreeSet::new();
    for page in &mut out.pages {
        for row in &mut page.rows {
            for section in &mut row.sections {
                match section {
                    Section::Group(group) => migrate_group(group, &mut used, &mut tracks),
                    Section::Pager(pager) => {
                        for sub in &mut pager.pages {
                            for inner in &mut sub.sections {
                                if let Section::Group(group) = inner {
                                    migrate_group(group, &mut used, &mut tracks);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    let labels = labels(&tracks);
    let planned = tracks
        .into_iter()
        .zip(labels)
        .map(|(track, label)| Planned {
            marker: marker_name(&label, &track.groups, track.pin, track.mute_guard),
            instance: track.instance,
            kind: track.kind,
            track: track.name,
        })
        .collect();
    Migration {
        frame: out,
        planned,
    }
}

/// The instance, kind and name of a strip bound by name; none otherwise.
fn by_name(control: &Control) -> Option<(&str, TrackKind, &str, bool, bool)> {
    let Control::Strip(strip) = control else {
        return None;
    };
    let (kind, name) = match &strip.binding.anchor {
        Anchor::Track { name } => (TrackKind::Track, name),
        Anchor::Return { name } => (TrackKind::Return, name),
        _ => return None,
    };
    Some((
        strip.binding.instance.as_str(),
        kind,
        name.as_str(),
        strip.pinned,
        strip.mute_guard,
    ))
}

/// Turns `group` into a tags group when every strip in it is bound by name
/// (and it holds one at least), recording its strips' tracks.
fn migrate_group(group: &mut Group, used: &mut BTreeSet<String>, tracks: &mut Vec<Track>) {
    let strips: Vec<&Control> = group
        .controls
        .iter()
        .filter(|c| matches!(c, Control::Strip(_)))
        .collect();
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
    for (i, (instance, kind, name, pin, mute_guard)) in
        strips.iter().copied().filter_map(by_name).enumerate()
    {
        let place = i + 1;
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
                    pin: false,
                    mute_guard: false,
                });
                tracks.last_mut().expect("just pushed")
            }
        };
        if !track.groups.iter().any(|(g, _)| *g == tag) {
            track.groups.push((tag.clone(), place));
        }
        track.pin |= pin;
        track.mute_guard |= mute_guard;
    }
    group.controls.retain(|c| !matches!(c, Control::Strip(_)));
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
    words
        .join(" ")
        .trim_end_matches('#')
        .trim_end()
        .replace('"', "")
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
        // Each label taken or another track's rules out one candidate.
        let name = (1..=out.len() + second.len() + 1)
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

/// The Tuner name: the quoted label, a `+G:` tag per group with its place,
/// then `+PIN` and `+MG` when set.
pub fn marker_name(label: &str, groups: &[(String, usize)], pin: bool, mute_guard: bool) -> String {
    let mut out = format!("\"{label}\"");
    for (group, place) in groups {
        if *place <= MAX_MIGRATED_PLACE {
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
