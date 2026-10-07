//! Keeps the groups of the surface's strip tracks unfolded (#58): Live
//! sends no meter of a track inside a folded group (verified on the PC: a
//! folded group's tracks kept one frozen `output_meter_level`, and their
//! listeners pushed nothing), so a folded group would freeze its strips'
//! meters on every tablet.
//!
//! A pure state machine the router drives with its own subscriber
//! (`UNFOLD_CLIENT`) and its own reads:
//!
//! - **Which groups:** a read asks, for every strip track of an instance,
//!   the `name` of its `group_track`, of that one's `group_track`, and so on
//!   up to [`MAX_DEPTH`] (one batch, one round trip; a step past the top
//!   answers an error). `group_track` cannot be listened to (Live and
//!   SimLive), so the keeper reads again whenever the instance's track list
//!   changes: it listens to `live_set` `tracks`, whose value is delivered
//!   again on every change and when Live connects or loads a set. A newer
//!   read voids an older answer.
//! - **Unfolded:** it listens to each held group's `fold_state`; a folded
//!   one gets `set_prop fold_state false` at once, when it is first heard
//!   and whenever someone folds it. A group no strip track sits in any more
//!   is let go.
//!
//! The layout names no group: the strips are enough (the owner's decision on
//! #58 replaced TouchOSC's `unfold_<instance>` list).

use std::collections::{BTreeMap, BTreeSet};

use fohmixer_proto::path::escape_name;
use serde_json::{Value, json};

/// A track of an instance, by name: `(instance, name)`.
pub type Track = (String, String);

/// The property a group's fold is read from and written to.
pub const FOLD: &str = "fold_state";
/// The list whose changes make the keeper read the groups again.
pub const TRACKS: &str = "tracks";
/// How far up a chain of groups a read looks.
pub const MAX_DEPTH: usize = 6;

/// What a keeper subscription reads.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Watch {
    /// An instance's track list (`live_set` `tracks`).
    Tracks(String),
    /// A held group's `fold_state`.
    Fold(Track),
}

impl Watch {
    /// Its instance, LOM target and property.
    pub fn target(&self) -> (&str, String, &'static str) {
        match self {
            Self::Tracks(instance) => (instance.as_str(), "live_set".to_string(), TRACKS),
            Self::Fold((instance, name)) => (instance.as_str(), target(name), FOLD),
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
    /// Unfold the group: `set_prop fold_state false` on `target(name)`.
    Unfold(Track),
}

/// The LOM target of a track by name (the layout's own form, so a page's
/// subscription to the same track shares its Live listener).
pub fn target(name: &str) -> String {
    format!("live_set tracks[name={}]", escape_name(name))
}

/// Whether a `fold_state` value says folded (Live answers 0 or 1).
pub fn folded(value: &Value) -> bool {
    value.as_i64() == Some(1) || value.as_bool() == Some(true)
}

/// A read of the groups the `tracks` sit in: for each, the `name` of its
/// group, of that group's group, … up to `MAX_DEPTH`, in that order.
pub fn read_commands(tracks: &[String]) -> Vec<Value> {
    tracks
        .iter()
        .flat_map(|name| {
            (1..=MAX_DEPTH).map(move |depth| {
                let up = " group_track".repeat(depth);
                json!({"target": format!("{}{up}", target(name)), "name": "get_prop", "args": {"prop": "name"}})
            })
        })
        .collect()
}

/// The group names a read's answer gives (each slot that answered a name).
pub fn read_groups(slots: &[Value]) -> BTreeSet<String> {
    slots
        .iter()
        .filter(|slot| slot.get("ok") == Some(&Value::Bool(true)))
        .filter_map(|slot| slot.get("data").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
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
}

impl Keeper {
    /// The layout's strip tracks: what to follow now. Each instance with
    /// strips is read at once.
    pub fn set_strips(&mut self, strips: impl IntoIterator<Item = Track>) -> Vec<Action> {
        self.strips = by_instance(strips.into_iter().collect());
        self.held
            .retain(|(instance, _)| self.strips.contains_key(instance));
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

    /// A value of a keeper subscription (`None`: an error, nothing there).
    pub fn value(&mut self, key: &str, value: Option<&Value>) -> Vec<Action> {
        match self.by_key.get(key).cloned() {
            Some(Watch::Tracks(instance)) => vec![self.read(&instance)],
            Some(Watch::Fold(track)) if value.is_some_and(folded) => vec![Action::Unfold(track)],
            _ => Vec::new(),
        }
    }

    /// The answer to read `seq` of `instance` (an error: nothing changes;
    /// the next list change or connect reads again).
    pub fn read_done(
        &mut self,
        instance: &str,
        seq: u64,
        outcome: &Result<Vec<Value>, String>,
    ) -> Vec<Action> {
        let (Some(&latest), Ok(slots)) = (self.reads.get(instance), outcome) else {
            return Vec::new();
        };
        if latest != seq {
            return Vec::new();
        }
        self.held.retain(|(i, _)| i != instance);
        self.held.extend(
            read_groups(slots)
                .into_iter()
                .map(|name| (instance.to_string(), name)),
        );
        self.sync()
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

    /// Subscribes to what the strips and the held groups need now and lets
    /// go of the rest.
    fn sync(&mut self) -> Vec<Action> {
        let wanted: BTreeSet<Watch> = self
            .strips
            .keys()
            .map(|instance| Watch::Tracks(instance.clone()))
            .chain(self.held.iter().cloned().map(Watch::Fold))
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
