//! Keeps the groups of the surface's strip tracks unfolded (#58): Live
//! sends no meter of a track inside a folded group (verified on the PC: a
//! folded group's tracks kept one frozen `output_meter_level`, and their
//! listeners pushed nothing), so a folded group would freeze its strips'
//! meters on every tablet.
//!
//! A pure state machine the router drives with its own subscriber
//! (`UNFOLD_CLIENT`) and its own reads. Live lets no one listen to a
//! track's `group_track`, `fold_state` or `is_visible` ("not observable",
//! read on the PC), so:
//!
//! - **When:** the keeper listens to each instance's `live_set` `tracks`
//!   (a track added, moved or deleted) and `visible_tracks` (a group folded
//!   or unfolded); both values are delivered again when Live connects or
//!   loads a set. Every value, a layout change and a retry start a read. A
//!   newer read voids an older answer.
//! - **A read** asks, for every strip track `T` of the instance, the `name`
//!   and the `fold_state` of `tracks[name=T] group_track`, of `… group_track
//!   group_track`, and so on up to [`MAX_DEPTH`] (one batch, one round trip;
//!   a step past the top answers an error).
//! - **Unfolded:** a group read folded (Live answers `true`, SimLive 1) gets
//!   `set_prop fold_state false` through that same path, so a group named
//!   like another still unfolds the right one. The unfold changes
//!   `visible_tracks`, whose read then finds it open. The groups held (in
//!   `/api/status`) are the names the latest read found.
//! - **A failed read** (a timeout during a busy connect, Live away) is
//!   tried again after [`RETRY_MS`], at most [`MAX_RETRIES`] times in a
//!   row; the next list change or connect reads anyway.
//!
//! The layout names no group: the strips are enough (the owner's decision on
//! #58 replaced TouchOSC's `unfold_<instance>` list).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

/// A track of an instance: `(instance, LOM target)` for a strip's track
/// (`Layout::strip_tracks`: by name, or a marker strip's by index, #68),
/// `(instance, group name)` for a group held.
pub type Track = (String, String);

/// The property a group's fold is read from and written to.
pub const FOLD: &str = "fold_state";
/// The list whose changes (tracks added, moved, deleted) start a read.
pub const TRACKS: &str = "tracks";
/// The list whose changes (a group folded or unfolded) start a read.
pub const VISIBLE: &str = "visible_tracks";
/// How far up a chain of groups a read looks.
pub const MAX_DEPTH: usize = 6;
/// The wait before a failed read is tried again.
pub const RETRY_MS: u64 = 2000;
/// Failed reads of an instance in a row that are tried again.
pub const MAX_RETRIES: u32 = 3;

/// What a keeper subscription listens to: an instance's list.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Watch {
    /// `live_set` `tracks`.
    Tracks(String),
    /// `live_set` `visible_tracks`.
    Visible(String),
}

impl Watch {
    /// Its instance, LOM target and property.
    pub fn target(&self) -> (&str, String, &'static str) {
        match self {
            Self::Tracks(instance) => (instance.as_str(), "live_set".to_string(), TRACKS),
            Self::Visible(instance) => (instance.as_str(), "live_set".to_string(), VISIBLE),
        }
    }

    fn instance(&self) -> &str {
        match self {
            Self::Tracks(instance) | Self::Visible(instance) => instance.as_str(),
        }
    }
}

/// What the keeper asks the router to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Subscribe to the watch, then tell `subscribed`.
    Sub(Watch),
    /// End the subscription of this hub key.
    Unsub { key: String },
    /// Send these commands to the instance and hand the answer to
    /// `read_done` with `seq`.
    Read {
        instance: String,
        seq: u64,
        commands: Vec<Value>,
    },
    /// Unfold the group `group` of `instance`: `set_prop fold_state false`
    /// on `target`, the path it was read through.
    Unfold {
        instance: String,
        target: String,
        group: String,
    },
    /// Ask `retry` for the instance after `RETRY_MS`.
    Retry { instance: String },
}

/// The LOM target of a track by name (the layout's own form; the tests'
/// strips, `Layout::strip_tracks` gives the hub's).
#[cfg(test)]
pub fn target(name: &str) -> String {
    format!(
        "live_set tracks[name={}]",
        fohmixer_proto::path::escape_name(name)
    )
}

/// The path of the group `depth` steps up from the track at `track` (its
/// LOM target).
pub fn group_path(track: &str, depth: usize) -> String {
    format!("{track}{}", " group_track".repeat(depth))
}

/// Whether a `fold_state` value says folded (Live answers `true`/`false`,
/// SimLive 1/0).
pub fn folded(value: &Value) -> bool {
    value.as_i64() == Some(1) || value.as_bool() == Some(true)
}

/// A read of the groups the `tracks` sit in: for each track, for each step
/// up to `MAX_DEPTH`, the group's `name`, then its `fold_state`.
pub fn read_commands(tracks: &[String]) -> Vec<Value> {
    tracks
        .iter()
        .flat_map(|track| (1..=MAX_DEPTH).map(move |depth| group_path(track, depth)))
        .flat_map(|path| {
            [
                json!({"target": path, "name": "get_prop", "args": {"prop": "name"}}),
                json!({"target": path, "name": "get_prop", "args": {"prop": FOLD}}),
            ]
        })
        .collect()
}

/// A slot's data, when it answered.
fn answered(slot: Option<&Value>) -> Option<&Value> {
    slot.filter(|s| s.get("ok") == Some(&Value::Bool(true)))
        .and_then(|s| s.get("data"))
}

/// What a read of `tracks` found: each group's name, and the path to each
/// folded one (the first path that reached it).
pub fn read_groups(
    tracks: &[String],
    slots: &[Value],
) -> (BTreeSet<String>, BTreeMap<String, String>) {
    let mut names = BTreeSet::new();
    let mut folded_at = BTreeMap::new();
    let paths = tracks
        .iter()
        .flat_map(|track| (1..=MAX_DEPTH).map(move |depth| group_path(track, depth)));
    for (path, pair) in paths.zip(slots.chunks(2)) {
        let Some(name) = answered(pair.first()).and_then(Value::as_str) else {
            continue;
        };
        names.insert(name.to_string());
        if answered(pair.get(1)).is_some_and(folded) {
            folded_at.entry(name.to_string()).or_insert(path);
        }
    }
    (names, folded_at)
}

/// Tracks by instance: each instance's names, in order.
pub fn by_instance(tracks: Vec<Track>) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (instance, name) in tracks {
        out.entry(instance).or_default().push(name);
    }
    out
}

/// The keeper.
#[derive(Debug, Default)]
pub struct Keeper {
    /// The layout's strip tracks, per instance.
    strips: BTreeMap<String, Vec<String>>,
    /// The groups held: those the latest read of each instance found.
    held: BTreeSet<Track>,
    /// The keeper's subscriptions, with their hub keys.
    subs: BTreeMap<Watch, String>,
    /// The same, by hub key.
    by_key: BTreeMap<String, Watch>,
    /// The latest read of each instance (an older answer is void).
    reads: BTreeMap<String, u64>,
    /// The last read's number (one counter across the instances).
    last_read: u64,
    /// Failed reads in a row, per instance.
    failures: BTreeMap<String, u32>,
}

impl Keeper {
    /// The layout's strip tracks: what to follow now. Each instance with
    /// strips is read at once.
    pub fn set_strips(&mut self, strips: impl IntoIterator<Item = Track>) -> Vec<Action> {
        self.strips = by_instance(strips.into_iter().collect());
        self.held
            .retain(|(instance, _)| self.strips.contains_key(instance));
        // A read in flight for an instance without strips is void.
        self.reads
            .retain(|instance, _| self.strips.contains_key(instance));
        self.failures.clear();
        let mut actions = self.sync();
        let instances: Vec<String> = self.strips.keys().cloned().collect();
        for instance in instances {
            actions.push(self.read(&instance));
        }
        actions
    }

    /// The router subscribed `watch` under the hub key `key`.
    pub fn subscribed(&mut self, watch: Watch, key: String) {
        self.by_key.insert(key.clone(), watch.clone());
        self.subs.insert(watch, key);
    }

    /// A value of a keeper subscription: a list changed (or Live connected),
    /// so the instance is read again. A failing list is no news.
    pub fn value(&mut self, key: &str, value: Option<&Value>) -> Vec<Action> {
        match (self.by_key.get(key).cloned(), value) {
            (Some(watch), Some(_)) => {
                let instance = watch.instance().to_string();
                self.failures.remove(&instance);
                vec![self.read(&instance)]
            }
            _ => Vec::new(),
        }
    }

    /// A failed read's second chance (`Action::Retry`).
    pub fn retry(&mut self, instance: &str) -> Vec<Action> {
        if self.strips.contains_key(instance) {
            vec![self.read(instance)]
        } else {
            Vec::new()
        }
    }

    /// The answer to read `seq` of `instance`: the groups held, and an
    /// unfold for each folded one. An error changes nothing, and the read
    /// is tried again up to `MAX_RETRIES` times in a row.
    pub fn read_done(
        &mut self,
        instance: &str,
        seq: u64,
        outcome: &Result<Vec<Value>, String>,
    ) -> Vec<Action> {
        if self.reads.get(instance) != Some(&seq) {
            return Vec::new();
        }
        let Ok(slots) = outcome else {
            let failures = self.failures.entry(instance.to_string()).or_default();
            *failures += 1;
            return if *failures <= MAX_RETRIES {
                vec![Action::Retry {
                    instance: instance.to_string(),
                }]
            } else {
                Vec::new()
            };
        };
        self.failures.remove(instance);
        let tracks = self.strips.get(instance).cloned().unwrap_or_default();
        let (names, folded_at) = read_groups(&tracks, slots);
        self.held.retain(|(i, _)| i != instance);
        self.held
            .extend(names.into_iter().map(|name| (instance.to_string(), name)));
        folded_at
            .into_iter()
            .map(|(group, target)| Action::Unfold {
                instance: instance.to_string(),
                target,
                group,
            })
            .collect()
    }

    /// The groups held now (for the status and tests).
    pub fn held(&self) -> Vec<Track> {
        self.held.iter().cloned().collect()
    }

    fn read(&mut self, instance: &str) -> Action {
        self.last_read += 1;
        self.reads.insert(instance.to_string(), self.last_read);
        Action::Read {
            instance: instance.to_string(),
            seq: self.last_read,
            commands: read_commands(
                self.strips
                    .get(instance)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
            ),
        }
    }

    /// Subscribes to the lists of the instances with strips and lets go of
    /// the rest.
    fn sync(&mut self) -> Vec<Action> {
        let wanted: BTreeSet<Watch> = self
            .strips
            .keys()
            .flat_map(|i| [Watch::Tracks(i.clone()), Watch::Visible(i.clone())])
            .collect();
        let mut actions = Vec::new();
        let gone: Vec<Watch> = self
            .subs
            .keys()
            .filter(|w| !wanted.contains(*w))
            .cloned()
            .collect();
        for watch in gone {
            if let Some(key) = self.subs.remove(&watch) {
                self.by_key.remove(&key);
                actions.push(Action::Unsub { key });
            }
        }
        for watch in wanted {
            if !self.subs.contains_key(&watch) {
                actions.push(Action::Sub(watch));
            }
        }
        actions
    }
}

#[cfg(test)]
mod tests;
