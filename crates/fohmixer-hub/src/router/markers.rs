//! Finds the Tuner markers of each instance (#68, spec D16): a Tuner in a
//! track's (or return track's) device chain whose name carries the strip's
//! label and tags. The layout store composes them with its frame file.
//!
//! A pure state machine the router drives with its own subscriber
//! (`MARKERS_CLIENT`) and its own reads, as the unfold keeper (#58):
//!
//! - **Lists:** it listens to each instance's `live_set` `tracks` and
//!   `return_tracks`. A list value that differs from the last one starts a
//!   read of the instance.
//! - **A read** asks every track's `devices` (one batch), then the
//!   `class_name` of every device of Live's own (`class` [`NATIVE`]: a rack,
//!   a plug-in or a Max device is never a Tuner) in a second batch. The
//!   Tuners found replace the instance's; the keeper then listens to every
//!   track's `devices` (a Tuner added or removed) and every Tuner's `name`
//!   (a rename fires only that listener). A `devices` watch it holds stays
//!   (the subscription table binds a track by index to the object there
//!   when the list of tracks changes); the names are subscribed afresh
//!   after each read (a device inserted before a Tuner moves it).
//! - **One read per flush:** the values of one flush ([`Keeper::values`])
//!   ask for at most one read of an instance: a list change gives every
//!   track's `devices` a new value at once.
//! - **Values:** a `devices` value that differs from what the read saw
//!   starts a read; a Tuner's `name` value updates that Tuner. The values a
//!   fresh subscription brings equal what the read saw, so they start
//!   nothing (no endless read).
//! - **Found:** each track whose Tuners include a marker (a quote or a `+`
//!   tag, [`is_marker`]) is one [`Found`]: its first marker's name and how
//!   many markers it holds. A change of the whole set (every instance) is
//!   handed to the store.
//! - **A newer read voids an older answer**; a failed read is tried again
//!   after [`RETRY_MS`], at most [`MAX_RETRIES`] times in a row.

use std::collections::BTreeMap;

use fohmixer_proto::markers::migrate::{MigrationRow, Planned};
use fohmixer_proto::markers::{Found, TUNER_CLASS, TrackKind, is_marker};
use serde_json::{Value, json};

/// `live_set`'s list of tracks.
pub const TRACKS: &str = "tracks";
/// `live_set`'s list of return tracks.
pub const RETURNS: &str = "return_tracks";
/// A track's list of devices.
pub const DEVICES: &str = "devices";
/// A device's name.
pub const NAME: &str = "name";
/// The property that tells a Tuner from Live's other devices.
pub const CLASS_NAME: &str = "class_name";
/// The script's class of Live's own devices (a Tuner among them).
pub const NATIVE: &str = "Device";
/// The wait before a failed read is tried again.
pub const RETRY_MS: u64 = 2000;
/// Failed reads of an instance in a row that are tried again.
pub const MAX_RETRIES: u32 = 3;
/// The quiet time after a change of the markers before they are served: a
/// set load or a burst of edits is one new layout, not several.
pub const SETTLE_MS: u64 = 300;

/// A track (or return track) of an instance: its kind and index.
pub type TrackAt = (TrackKind, u32);

/// The LOM path of the track (or return track) at `index`.
pub fn track_path(kind: TrackKind, index: u32) -> String {
    match kind {
        TrackKind::Track => format!("live_set tracks {index}"),
        TrackKind::Return => format!("live_set return_tracks {index}"),
    }
}

/// The LOM path of device `device` of the track at `index`.
pub fn device_path(kind: TrackKind, index: u32, device: u32) -> String {
    format!("{} devices {device}", track_path(kind, index))
}

/// What a keeper subscription listens to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Watch {
    /// `live_set` `tracks`.
    Tracks(String),
    /// `live_set` `return_tracks`.
    Returns(String),
    /// A track's `devices`.
    Devices {
        instance: String,
        kind: TrackKind,
        index: u32,
    },
    /// A Tuner's `name`.
    Name {
        instance: String,
        kind: TrackKind,
        index: u32,
        device: u32,
    },
}

impl Watch {
    /// Its instance, LOM target and property.
    pub fn target(&self) -> (&str, String, &'static str) {
        match self {
            Self::Tracks(instance) => (instance.as_str(), "live_set".to_string(), TRACKS),
            Self::Returns(instance) => (instance.as_str(), "live_set".to_string(), RETURNS),
            Self::Devices {
                instance,
                kind,
                index,
            } => (instance.as_str(), track_path(*kind, *index), DEVICES),
            Self::Name {
                instance,
                kind,
                index,
                device,
            } => (instance.as_str(), device_path(*kind, *index, *device), NAME),
        }
    }

    fn instance(&self) -> &str {
        match self {
            Self::Tracks(instance)
            | Self::Returns(instance)
            | Self::Devices { instance, .. }
            | Self::Name { instance, .. } => instance.as_str(),
        }
    }

    /// A watch of one track or device (a read keeps the `devices` it still
    /// wants and subscribes the Tuners' names afresh).
    fn per_track(&self) -> bool {
        matches!(self, Self::Devices { .. } | Self::Name { .. })
    }
}

/// The two batches of a read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Every track's `devices`.
    Devices,
    /// The `class_name` of each device of Live's own.
    Classes,
}

/// What the keeper asks the router to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Subscribe to the watch, then tell `subscribed`.
    Sub(Watch),
    /// End the subscription of this hub key.
    Unsub { key: String },
    /// Send these commands to the instance and hand the answer to
    /// `read_done` with `seq` and `step`.
    Read {
        instance: String,
        seq: u64,
        step: Step,
        commands: Vec<Value>,
    },
    /// Ask `retry` for the instance after `RETRY_MS`.
    Retry { instance: String },
    /// The markers found changed: every instance's, sorted.
    Found(Vec<Found>),
    /// The migration (#68 PR C): `set_prop name` of the Tuner `device` of
    /// the track at `index` to `name`. The new name comes back through the
    /// Tuner's own `name` watch.
    Rename {
        instance: String,
        kind: TrackKind,
        index: u32,
        device: u32,
        name: String,
    },
}

/// The actions with only the last read of each instance. A newer read
/// voids an older one's answer (`Keeper::read_done`), so sending the older
/// is waste: a change of the list of tracks binds every track's `devices`
/// to its new object, and each new value asks for a read in one flush.
fn latest_reads(actions: Vec<Action>) -> Vec<Action> {
    let last: BTreeMap<String, usize> = actions
        .iter()
        .enumerate()
        .filter_map(|(i, a)| match a {
            Action::Read { instance, .. } => Some((instance.clone(), i)),
            _ => None,
        })
        .collect();
    actions
        .into_iter()
        .enumerate()
        .filter(|(i, a)| match a {
            Action::Read { instance, .. } => last.get(instance) == Some(i),
            _ => true,
        })
        .map(|(_, a)| a)
        .collect()
}

/// A device that may be a Tuner: where it is and its name.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Candidate {
    at: TrackAt,
    device: u32,
    name: String,
}

/// A read in flight.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pending {
    seq: u64,
    /// The tracks and return tracks it read (`Step::Devices`).
    counts: (u32, u32),
    /// The devices the first batch read, by track.
    devices: BTreeMap<TrackAt, Value>,
    /// The devices whose class the second batch reads.
    candidates: Vec<Candidate>,
}

/// One instance's state.
#[derive(Debug, Default)]
struct Instance {
    /// The last `tracks` and `return_tracks` values.
    tracks: Option<Value>,
    returns: Option<Value>,
    /// Each track's devices as the last read saw them.
    devices: BTreeMap<TrackAt, Value>,
    /// The Tuners: where, and their names.
    tuners: BTreeMap<(TrackAt, u32), String>,
    pending: Option<Pending>,
    failures: u32,
}

/// The names of a list value's items (the script encodes a track as an
/// object with its `name`); none for an item without one.
fn item_names(list: Option<&Value>) -> Vec<Option<&str>> {
    list.and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| item.get("name").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default()
}

/// The length of a list value (0: no list).
fn len_of(value: Option<&Value>) -> u32 {
    value
        .and_then(Value::as_array)
        .map_or(0, |list| u32::try_from(list.len()).unwrap_or(u32::MAX))
}

/// A slot's data, when it answered.
fn answered(slot: Option<&Value>) -> Option<&Value> {
    slot.filter(|s| s.get("ok") == Some(&Value::Bool(true)))
        .and_then(|s| s.get("data"))
}

/// The tracks a read covers, in command order.
fn tracks_of(counts: (u32, u32)) -> Vec<TrackAt> {
    (0..counts.0)
        .map(|i| (TrackKind::Track, i))
        .chain((0..counts.1).map(|i| (TrackKind::Return, i)))
        .collect()
}

/// The first batch: every track's `devices`.
pub fn devices_commands(counts: (u32, u32)) -> Vec<Value> {
    tracks_of(counts)
        .into_iter()
        .map(|(kind, index)| {
            json!({"target": track_path(kind, index), "name": "get_prop", "args": {"prop": DEVICES}})
        })
        .collect()
}

/// What the first batch found: each track's devices, and the devices of
/// Live's own (the candidates whose class the second batch reads).
fn read_devices(counts: (u32, u32), slots: &[Value]) -> (BTreeMap<TrackAt, Value>, Vec<Candidate>) {
    let mut devices = BTreeMap::new();
    let mut candidates = Vec::new();
    for (at, slot) in tracks_of(counts).into_iter().zip(slots) {
        let Some(list) = answered(Some(slot)) else {
            continue;
        };
        for (device, item) in list.as_array().into_iter().flatten().enumerate() {
            if item.get("class").and_then(Value::as_str) == Some(NATIVE) {
                candidates.push(Candidate {
                    at,
                    device: u32::try_from(device).unwrap_or(u32::MAX),
                    name: item
                        .get(NAME)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                });
            }
        }
        devices.insert(at, list.clone());
    }
    (devices, candidates)
}

/// The second batch: each candidate's `class_name`.
fn classes_commands(candidates: &[Candidate]) -> Vec<Value> {
    candidates
        .iter()
        .map(|c| {
            json!({"target": device_path(c.at.0, c.at.1, c.device), "name": "get_prop", "args": {"prop": CLASS_NAME}})
        })
        .collect()
}

/// The Tuners among the candidates, by the second batch's answers.
fn read_tuners(candidates: &[Candidate], slots: &[Value]) -> BTreeMap<(TrackAt, u32), String> {
    candidates
        .iter()
        .zip(slots)
        .filter(|(_, slot)| answered(Some(*slot)).and_then(Value::as_str) == Some(TUNER_CLASS))
        .map(|(c, _)| ((c.at, c.device), c.name.clone()))
        .collect()
}

/// The markers of an instance's Tuners: per track, its first marker's name
/// and how many markers it holds.
pub fn found_of(instance: &str, tuners: &BTreeMap<(TrackAt, u32), String>) -> Vec<Found> {
    let mut found: Vec<Found> = Vec::new();
    for (((kind, index), _), name) in tuners {
        if !is_marker(name) {
            continue;
        }
        let same = found
            .last()
            .is_some_and(|last| last.kind == *kind && last.index == *index);
        if !same {
            found.push(Found {
                instance: instance.to_string(),
                kind: *kind,
                index: *index,
                name: name.clone(),
                tuners: 0,
            });
        }
        if let Some(last) = found.last_mut() {
            last.tuners += 1;
        }
    }
    found
}

/// The keeper.
#[derive(Debug, Default)]
pub struct Keeper {
    instances: BTreeMap<String, Instance>,
    /// The keeper's subscriptions, with their hub keys.
    subs: BTreeMap<Watch, String>,
    /// The same, by hub key.
    by_key: BTreeMap<String, Watch>,
    /// The last read's number (one counter across the instances).
    last_read: u64,
    /// The markers last handed out.
    found: Vec<Found>,
}

impl Keeper {
    /// The instances to follow: their lists are subscribed.
    pub fn set_instances(&mut self, names: impl IntoIterator<Item = String>) -> Vec<Action> {
        let mut actions = Vec::new();
        for name in names {
            self.instances.entry(name.clone()).or_default();
            actions.push(Action::Sub(Watch::Tracks(name.clone())));
            actions.push(Action::Sub(Watch::Returns(name)));
        }
        actions
    }

    /// The router subscribed `watch` under the hub key `key`.
    pub fn subscribed(&mut self, watch: Watch, key: String) {
        self.by_key.insert(key.clone(), watch.clone());
        self.subs.insert(watch, key);
    }

    /// A value of a keeper subscription (none: an error, no news).
    pub fn value(&mut self, key: &str, value: Option<&Value>) -> Vec<Action> {
        let (Some(watch), Some(value)) = (self.by_key.get(key).cloned(), value) else {
            return Vec::new();
        };
        let Some(state) = self.instances.get_mut(watch.instance()) else {
            return Vec::new();
        };
        let changed = match &watch {
            Watch::Tracks(_) => {
                let changed = state.tracks.as_ref() != Some(value);
                state.tracks = Some(value.clone());
                changed
            }
            Watch::Returns(_) => {
                let changed = state.returns.as_ref() != Some(value);
                state.returns = Some(value.clone());
                changed
            }
            Watch::Devices { kind, index, .. } => {
                state.devices.get(&(*kind, *index)) != Some(value)
            }
            Watch::Name {
                kind,
                index,
                device,
                ..
            } => {
                let name = value.as_str().unwrap_or_default();
                if let Some(known) = state.tuners.get_mut(&((*kind, *index), *device))
                    && known.as_str() != name
                {
                    *known = name.to_string();
                    return self.report().into_iter().collect();
                }
                false
            }
        };
        if changed {
            vec![self.read(watch.instance())]
        } else {
            Vec::new()
        }
    }

    /// The values of one flush (the router's deliveries, or the values
    /// fresh subscriptions already held): each through [`Keeper::value`],
    /// then only the last read of each instance (`latest_reads`).
    pub fn values<'a>(
        &mut self,
        items: impl IntoIterator<Item = (&'a str, Option<&'a Value>)>,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        for (key, value) in items {
            actions.extend(self.value(key, value));
        }
        latest_reads(actions)
    }

    /// A failed read's second chance (`Action::Retry`).
    pub fn retry(&mut self, instance: &str) -> Vec<Action> {
        if self.instances.contains_key(instance) {
            vec![self.read(instance)]
        } else {
            Vec::new()
        }
    }

    /// The answer to a batch of read `seq`: the second batch's commands, or
    /// the Tuners found (fresh subscriptions, and the markers when they
    /// changed). An error is tried again up to `MAX_RETRIES` times in a row.
    pub fn read_done(
        &mut self,
        instance: &str,
        seq: u64,
        step: Step,
        outcome: &Result<Vec<Value>, String>,
    ) -> Vec<Action> {
        let Some(state) = self.instances.get_mut(instance) else {
            return Vec::new();
        };
        let Some(pending) = state.pending.as_mut().filter(|p| p.seq == seq) else {
            return Vec::new();
        };
        let Ok(slots) = outcome else {
            state.pending = None;
            state.failures += 1;
            return if state.failures <= MAX_RETRIES {
                vec![Action::Retry {
                    instance: instance.to_string(),
                }]
            } else {
                Vec::new()
            };
        };
        let tuners = match step {
            Step::Devices => {
                let (devices, candidates) = read_devices(pending.counts, slots);
                pending.devices = devices;
                if !candidates.is_empty() {
                    let commands = classes_commands(&candidates);
                    pending.candidates = candidates;
                    return vec![Action::Read {
                        instance: instance.to_string(),
                        seq,
                        step: Step::Classes,
                        commands,
                    }];
                }
                BTreeMap::new()
            }
            Step::Classes => read_tuners(&pending.candidates, slots),
        };
        let Some(done) = state.pending.take() else {
            return Vec::new();
        };
        // Only a whole read resets the count: a second batch that keeps
        // failing is tried again `MAX_RETRIES` times too.
        state.failures = 0;
        state.devices = done.devices;
        state.tuners = tuners;
        let mut actions = self.resubscribe(instance, done.counts);
        actions.extend(self.report());
        actions
    }

    /// Each planned track (#68 PR C) as the latest reads found it.
    pub fn migration_rows(&self, planned: &[Planned]) -> Vec<MigrationRow> {
        planned.iter().map(|p| self.locate(p).0).collect()
    }

    /// The renames of the migration: each planned track that is ready
    /// (`MigrationRow::ready`) gets its one plain Tuner named its marker,
    /// unless a read of its instance is running (a device inserted before
    /// the Tuner may have moved it).
    pub fn renames(&self, planned: &[Planned]) -> Vec<Action> {
        planned
            .iter()
            .filter(|p| !self.reading(&p.instance))
            .filter_map(|p| {
                let (row, device) = self.locate(p);
                Some(Action::Rename {
                    index: row.index?,
                    device: device?,
                    instance: row.instance,
                    kind: row.kind,
                    name: row.marker,
                })
            })
            .collect()
    }

    /// Whether a read of `instance` is running.
    pub fn reading(&self, instance: &str) -> bool {
        self.instances
            .get(instance)
            .is_some_and(|state| state.pending.is_some())
    }

    /// The instance's lists read afresh (the migration's names: Live fires
    /// no list listener on a track's rename): `slots` answer `get_prop
    /// tracks` and `get_prop return_tracks`. A list that changed is kept and
    /// starts a read, as its listener's value would.
    pub fn refresh(&mut self, instance: &str, slots: &[Value]) -> Vec<Action> {
        let tracks = answered(slots.first()).cloned();
        let returns = answered(slots.get(1)).cloned();
        let Some(state) = self.instances.get_mut(instance) else {
            return Vec::new();
        };
        let mut changed = false;
        for (fresh, kept) in [(tracks, &mut state.tracks), (returns, &mut state.returns)] {
            if fresh.is_some() && fresh != *kept {
                *kept = fresh;
                changed = true;
            }
        }
        if changed {
            vec![self.read(instance)]
        } else {
            Vec::new()
        }
    }

    /// A planned track's row, and the device of its one plain Tuner when
    /// the row is ready.
    fn locate(&self, planned: &Planned) -> (MigrationRow, Option<u32>) {
        let state = self.instances.get(&planned.instance);
        let list = state.and_then(|s| match planned.kind {
            TrackKind::Track => s.tracks.as_ref(),
            TrackKind::Return => s.returns.as_ref(),
        });
        let indexes: Vec<u32> = item_names(list)
            .into_iter()
            .enumerate()
            .filter(|(_, name)| *name == Some(planned.track.as_str()))
            .filter_map(|(i, _)| u32::try_from(i).ok())
            .collect();
        let index = match indexes.as_slice() {
            [one] => Some(*one),
            _ => None,
        };
        let tuners: Vec<(u32, &String)> = match (state, index) {
            (Some(state), Some(index)) => state
                .tuners
                .iter()
                .filter(|(((kind, at), _), _)| *kind == planned.kind && *at == index)
                .map(|((_, device), name)| (*device, name))
                .collect(),
            _ => Vec::new(),
        };
        let plain: Vec<u32> = tuners
            .iter()
            .filter(|(_, name)| !is_marker(name))
            .map(|(device, _)| *device)
            .collect();
        let markers = tuners.iter().filter(|(_, name)| is_marker(name)).count();
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        let row = MigrationRow {
            instance: planned.instance.clone(),
            kind: planned.kind,
            track: planned.track.clone(),
            marker: planned.marker.clone(),
            matches: count(indexes.len()),
            index,
            plain: count(plain.len()),
            markers: count(markers),
            done: tuners.iter().any(|(_, name)| **name == planned.marker),
        };
        let device = plain.first().copied().filter(|_| row.ready());
        (row, device)
    }

    /// The markers found now (for the tests).
    #[cfg(test)]
    pub fn found(&self) -> &[Found] {
        &self.found
    }

    fn read(&mut self, instance: &str) -> Action {
        self.last_read += 1;
        let seq = self.last_read;
        let state = self.instances.entry(instance.to_string()).or_default();
        let counts = (
            len_of(state.tracks.as_ref()),
            len_of(state.returns.as_ref()),
        );
        state.pending = Some(Pending {
            seq,
            counts,
            devices: BTreeMap::new(),
            candidates: Vec::new(),
        });
        Action::Read {
            instance: instance.to_string(),
            seq,
            step: Step::Devices,
            commands: devices_commands(counts),
        }
    }

    /// The instance's per-track watches after a read: every track's
    /// `devices` (one already held stays: the subscription table binds it to
    /// the object at its index again when the list of tracks changes), and
    /// every Tuner's `name` afresh (a device inserted before a Tuner moves
    /// it, and only a new subscription names the device there now).
    fn resubscribe(&mut self, instance: &str, counts: (u32, u32)) -> Vec<Action> {
        let devices: Vec<Watch> = tracks_of(counts)
            .into_iter()
            .map(|(kind, index)| Watch::Devices {
                instance: instance.to_string(),
                kind,
                index,
            })
            .collect();
        let mut actions = Vec::new();
        let gone: Vec<Watch> = self
            .subs
            .keys()
            .filter(|w| w.per_track() && w.instance() == instance && !devices.contains(w))
            .cloned()
            .collect();
        for watch in gone {
            if let Some(key) = self.subs.remove(&watch) {
                self.by_key.remove(&key);
                actions.push(Action::Unsub { key });
            }
        }
        for watch in devices {
            if !self.subs.contains_key(&watch) {
                actions.push(Action::Sub(watch));
            }
        }
        if let Some(state) = self.instances.get(instance) {
            for &((kind, index), device) in state.tuners.keys() {
                actions.push(Action::Sub(Watch::Name {
                    instance: instance.to_string(),
                    kind,
                    index,
                    device,
                }));
            }
        }
        actions
    }

    /// `Action::Found` when the markers of every instance changed.
    fn report(&mut self) -> Option<Action> {
        let found: Vec<Found> = self
            .instances
            .iter()
            .flat_map(|(name, state)| found_of(name, &state.tuners))
            .collect();
        if found == self.found {
            return None;
        }
        self.found = found.clone();
        Some(Action::Found(found))
    }
}

#[cfg(test)]
mod tests;
