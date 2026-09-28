//! The subscription table (S3 design note §4), a pure state machine: the
//! router feeds it client requests and the script's frames, and sends the
//! requests it asks for.
//!
//! - One entry per hub key (`instance|target|prop|display`), shared by every
//!   subscribed client: the first subscriber makes the hub send
//!   `add_listener`, the last one leaving makes it send `remove_listener`.
//!   The latest value is cached and handed at once to a later subscriber.
//! - Several hub keys can land on one Live listener (`tracks 0` and
//!   `tracks[name=…]` are the same object): Live keys are reference-counted,
//!   and `remove_listener` goes out only when no entry holds the key.
//! - A name binding (`tracks[name=Hand1 #] …`) is guarded: the hub also
//!   listens to the list each name selects from and to the selected object's
//!   `name`. A change there re-resolves every name binding of the instance
//!   by its original path, so a renamed track gives its subscribers an error
//!   (spec I5), never a stale value, and a rename back gives the value again.
//! - On `connect` (a Live start or set load) every entry is resolved again
//!   by its original path: ids of the old session mean nothing.
//! - A `remove_listener` is sent only while no `add_listener` of the same
//!   instance is in flight: an `add_listener` sent earlier could return the
//!   very key being removed, and the script runs requests in order.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use fohmixer_proto::client::{ValueItem, hub_key};
use fohmixer_proto::path::LomPath;
use serde_json::{Value, json};

use super::LiveValue;

/// A client connection (an internal subscriber too).
pub type ClientId = u64;

/// Commands per request to a script: results stay small (S2 notes a large
/// result holds Live's interpreter for ~10 ms per MB).
pub const BATCH_MAX: usize = 64;

/// What the hub knows about a subscription.
#[derive(Debug, Clone, PartialEq)]
pub enum Cached {
    /// Live's value, and its display string when the key asked for one.
    Value {
        value: Value,
        display: Option<String>,
    },
    /// Why there is no value (the binding does not resolve, the object went).
    Error(String),
}

impl Cached {
    /// The client item of `sub` in this state.
    pub fn item(&self, sub: &str) -> ValueItem {
        match self {
            Self::Value { value, display } => ValueItem::value(sub, value.clone(), display.clone()),
            Self::Error(error) => ValueItem::error(sub, error),
        }
    }
}

/// A request for one instance's script: the envelope's uuid and commands.
#[derive(Debug, Clone, PartialEq)]
pub struct Outgoing {
    pub instance: String,
    pub uuid: String,
    pub commands: Vec<Value>,
}

/// The answer to `subscribe`: the hub key and the cached state, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct SubReply {
    pub key: String,
    pub cached: Option<Cached>,
}

#[derive(Debug, Clone)]
enum Tag {
    Resolve { key: String, seq: u64 },
    Release { live: String },
}

#[derive(Debug)]
struct Batch {
    instance: String,
    generation: u64,
    tags: Vec<Tag>,
}

#[derive(Debug)]
struct Entry {
    instance: String,
    target: String,
    prop: String,
    display: bool,
    /// The target selects by name somewhere (it is re-resolved when a guard
    /// fires).
    named: bool,
    clients: BTreeSet<ClientId>,
    /// Entries this one guards (it listens to a name or a list they depend
    /// on).
    dependents: BTreeSet<String>,
    /// The guard entries of this one.
    guards: Vec<String>,
    /// The Live listener key, while resolved.
    live: Option<String>,
    cached: Option<Cached>,
    /// The latest `add_listener` sent for this entry.
    seq: u64,
}

#[derive(Debug, Default)]
struct Instance {
    online: bool,
    /// Bumped on every `connect`: answers of an older session are ignored.
    generation: u64,
    /// Entries to resolve at the next drain.
    dirty: BTreeSet<String>,
    /// Live keys no entry holds any more, to remove at a drain.
    release: BTreeSet<String>,
}

/// The table.
#[derive(Debug, Default)]
pub struct Subs {
    entries: BTreeMap<String, Entry>,
    by_live: HashMap<(String, String), BTreeSet<String>>,
    instances: BTreeMap<String, Instance>,
    inflight: HashMap<String, Batch>,
    next_uuid: u64,
    deliveries: Vec<(ClientId, ValueItem)>,
}

/// A result slot of `add_listener`: the Live key and the state it gives.
fn parse_slot(slot: &Value) -> Result<(String, Cached), String> {
    if slot.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(slot
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("no result")
            .to_string());
    }
    let null = Value::Null;
    let data = slot.get("data").unwrap_or(&null);
    let key = data
        .get("key")
        .and_then(Value::as_str)
        .ok_or("add_listener answered without a key")?;
    Ok((
        key.to_string(),
        Cached::Value {
            value: data.get("value").cloned().unwrap_or(Value::Null),
            display: data
                .get("display")
                .and_then(Value::as_str)
                .map(str::to_string),
        },
    ))
}

/// The guards of a path: for every name step, the list it selects from
/// (the owner's list attribute) and the selected object's `name`.
fn guard_targets(path: &LomPath) -> Vec<(String, String)> {
    let mut guards = Vec::new();
    for (k, step) in path.steps.iter().enumerate() {
        if step.name.is_some() {
            guards.push((path.prefix(k).text(), step.attr.clone()));
            guards.push((path.prefix(k + 1).text(), "name".to_string()));
        }
    }
    guards
}

impl Subs {
    /// A table for these instances (all offline).
    pub fn new<I: IntoIterator<Item = String>>(instances: I) -> Self {
        Self {
            instances: instances
                .into_iter()
                .map(|name| (name, Instance::default()))
                .collect(),
            ..Self::default()
        }
    }

    /// Subscribes `client`. `Err((key, message))`: the request cannot be a
    /// subscription (unknown instance, a bad prop or a target that does not
    /// parse); nothing is stored.
    pub fn subscribe(
        &mut self,
        client: ClientId,
        instance: &str,
        target: &str,
        prop: &str,
        display: bool,
    ) -> Result<SubReply, (String, String)> {
        let key = hub_key(instance, target, prop, display);
        if !self.instances.contains_key(instance) {
            return Err((key, format!("unknown instance {instance:?}")));
        }
        if prop.is_empty() || prop.starts_with('_') {
            return Err((key, format!("bad prop {prop:?}")));
        }
        let path = LomPath::parse(target).map_err(|e| (key.clone(), e.to_string()))?;
        self.ensure(&key, instance, &path, prop, display);
        let entry = self.entries.get_mut(&key).expect("ensured above");
        let first = entry.clients.is_empty();
        entry.clients.insert(client);
        let cached = entry.cached.clone();
        if first {
            self.add_guards(&key, instance, &path);
        }
        Ok(SubReply { key, cached })
    }

    /// Ends `client`'s subscription `key`; false when it had none.
    pub fn unsubscribe(&mut self, client: ClientId, key: &str) -> bool {
        let Some(entry) = self.entries.get_mut(key) else {
            return false;
        };
        if !entry.clients.remove(&client) {
            return false;
        }
        self.maybe_drop(key);
        true
    }

    /// Ends every subscription of `client` (its connection closed).
    pub fn drop_client(&mut self, client: ClientId) {
        let keys: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, e)| e.clients.contains(&client))
            .map(|(k, _)| k.clone())
            .collect();
        for key in keys {
            self.unsubscribe(client, &key);
        }
    }

    /// The script of `instance` said `connect`: resolve everything again.
    pub fn connected(&mut self, instance: &str) {
        let keys: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, e)| e.instance == instance)
            .map(|(k, _)| k.clone())
            .collect();
        let Some(inst) = self.instances.get_mut(instance) else {
            return;
        };
        inst.online = true;
        inst.generation += 1;
        inst.release.clear();
        inst.dirty.extend(keys);
    }

    /// The connection to `instance` ended: every listener and value of it is
    /// gone with it.
    pub fn disconnected(&mut self, instance: &str) {
        let Some(inst) = self.instances.get_mut(instance) else {
            return;
        };
        inst.online = false;
        inst.release.clear();
        for entry in self.entries.values_mut() {
            if entry.instance == instance {
                entry.live = None;
                entry.cached = None;
            }
        }
        self.by_live.retain(|(i, _), _| i != instance);
        self.inflight.retain(|_, b| b.instance != instance);
    }

    /// Whether `instance` is connected.
    pub fn is_online(&self, instance: &str) -> bool {
        self.instances.get(instance).is_some_and(|i| i.online)
    }

    /// A `values` push of `instance`.
    pub fn on_values(&mut self, instance: &str, items: &[LiveValue]) {
        if !self.is_online(instance) {
            return;
        }
        let mut guard_fired = false;
        for item in items {
            let id = (instance.to_string(), item.key.clone());
            let Some(keys) = self.by_live.get(&id).cloned() else {
                continue;
            };
            if item.error.is_some() {
                // The script dropped the listener itself: nothing to remove.
                self.by_live.remove(&id);
            }
            for key in keys {
                let Some(entry) = self.entries.get_mut(&key) else {
                    continue;
                };
                guard_fired |= !entry.dependents.is_empty();
                let cached = match &item.error {
                    Some(error) => {
                        entry.live = None;
                        Cached::Error(error.clone())
                    }
                    None => Cached::Value {
                        value: item.value.clone(),
                        display: item.display.clone().filter(|_| entry.display),
                    },
                };
                self.set_cached(&key, cached);
            }
        }
        if guard_fired {
            self.resolve_named(instance);
        }
    }

    /// The result of a request this table asked for; false when `uuid` is
    /// not one of its requests.
    pub fn on_result(&mut self, instance: &str, uuid: &str, slots: &[Value]) -> bool {
        let Some(batch) = self.inflight.remove(uuid) else {
            return false;
        };
        let generation = self.instances.get(instance).map(|i| i.generation);
        if batch.instance != instance || generation != Some(batch.generation) {
            return true;
        }
        let missing = json!({"ok": false, "error": "no result"});
        for (i, tag) in batch.tags.iter().enumerate() {
            let slot = slots.get(i).unwrap_or(&missing);
            match tag {
                Tag::Resolve { key, seq } => self.resolved(instance, key, *seq, slot),
                Tag::Release { live } => {
                    if slot.get("ok").and_then(Value::as_bool) != Some(true) {
                        tracing::debug!(instance = %instance, live = %live, slot = %slot, "remove_listener failed");
                    }
                }
            }
        }
        true
    }

    /// The requests to send now, in order, per instance: first the removals
    /// (only while no `add_listener` is in flight), then the resolutions.
    pub fn drain_outgoing(&mut self) -> Vec<Outgoing> {
        let mut out = Vec::new();
        let names: Vec<String> = self.instances.keys().cloned().collect();
        for name in names {
            let resolving = self.inflight.values().any(|b| {
                b.instance == name && b.tags.iter().any(|t| matches!(t, Tag::Resolve { .. }))
            });
            let inst = self.instances.get_mut(&name).expect("listed above");
            if !inst.online {
                continue;
            }
            let mut commands: Vec<(Value, Tag)> = Vec::new();
            if !resolving {
                for live in std::mem::take(&mut inst.release) {
                    let (id, prop) = live.rsplit_once('.').unwrap_or((live.as_str(), ""));
                    commands.push((
                        json!({"target": {"$ref": id}, "name": "remove_listener", "args": {"prop": prop}}),
                        Tag::Release { live: live.clone() },
                    ));
                }
            }
            for key in std::mem::take(&mut inst.dirty) {
                let Some(entry) = self.entries.get_mut(&key) else {
                    continue;
                };
                entry.seq += 1;
                let mut args = json!({"prop": entry.prop});
                if entry.display {
                    args["display"] = json!(true);
                }
                commands.push((
                    json!({"target": entry.target, "name": "add_listener", "args": args}),
                    Tag::Resolve {
                        key,
                        seq: entry.seq,
                    },
                ));
            }
            let generation = inst.generation;
            let mut commands = commands.into_iter().peekable();
            while commands.peek().is_some() {
                let (chunk, tags): (Vec<Value>, Vec<Tag>) =
                    commands.by_ref().take(BATCH_MAX).unzip();
                self.next_uuid += 1;
                let uuid = format!("s{}", self.next_uuid);
                self.inflight.insert(
                    uuid.clone(),
                    Batch {
                        instance: name.clone(),
                        generation,
                        tags,
                    },
                );
                out.push(Outgoing {
                    instance: name.clone(),
                    uuid,
                    commands: chunk,
                });
            }
        }
        out
    }

    /// The client items produced since the last call.
    pub fn take_deliveries(&mut self) -> Vec<(ClientId, ValueItem)> {
        std::mem::take(&mut self.deliveries)
    }

    /// The cached state of a hub key.
    pub fn cached(&self, key: &str) -> Option<&Cached> {
        self.entries.get(key).and_then(|e| e.cached.as_ref())
    }

    /// Client subscriptions (hub keys with a subscriber) of `instance`.
    pub fn subscriptions(&self, instance: &str) -> usize {
        self.entries
            .values()
            .filter(|e| e.instance == instance && !e.clients.is_empty())
            .count()
    }

    /// Live listeners held on `instance` (distinct Live keys).
    pub fn listeners(&self, instance: &str) -> usize {
        self.by_live.keys().filter(|(i, _)| i == instance).count()
    }

    // --- internals ---

    /// Creates the entry `key` if it does not exist (to be resolved).
    fn ensure(&mut self, key: &str, instance: &str, path: &LomPath, prop: &str, display: bool) {
        if self.entries.contains_key(key) {
            return;
        }
        self.entries.insert(
            key.to_string(),
            Entry {
                instance: instance.to_string(),
                target: path.text(),
                prop: prop.to_string(),
                display,
                named: path.steps.iter().any(|s| s.name.is_some()),
                clients: BTreeSet::new(),
                dependents: BTreeSet::new(),
                guards: Vec::new(),
                live: None,
                cached: None,
                seq: 0,
            },
        );
        if let Some(inst) = self.instances.get_mut(instance) {
            inst.dirty.insert(key.to_string());
        }
    }

    /// Guards `key` (a client subscription) against renames and list
    /// changes along its name steps.
    fn add_guards(&mut self, key: &str, instance: &str, path: &LomPath) {
        let mut guards: Vec<String> = Vec::new();
        for (target, prop) in guard_targets(path) {
            let guard = hub_key(instance, &target, &prop, false);
            if guard == key || guards.contains(&guard) {
                continue;
            }
            let guard_path = LomPath::parse(&target).expect("a prefix of a parsed path parses");
            self.ensure(&guard, instance, &guard_path, &prop, false);
            if let Some(entry) = self.entries.get_mut(&guard) {
                entry.dependents.insert(key.to_string());
            }
            guards.push(guard);
        }
        if let Some(entry) = self.entries.get_mut(key) {
            entry.guards = guards;
        }
    }

    /// Removes `key` when nothing needs it any more; an entry that lost its
    /// last client but still guards others drops its own guards.
    fn maybe_drop(&mut self, key: &str) {
        let Some(entry) = self.entries.get_mut(key) else {
            return;
        };
        if !entry.clients.is_empty() {
            return;
        }
        let guards = std::mem::take(&mut entry.guards);
        let keep = !entry.dependents.is_empty();
        if !keep {
            let entry = self.entries.remove(key).expect("present above");
            if let Some(inst) = self.instances.get_mut(&entry.instance) {
                inst.dirty.remove(key);
            }
            if let Some(live) = entry.live {
                self.unlink(&entry.instance, &live, key);
            }
        }
        for guard in guards {
            if let Some(g) = self.entries.get_mut(&guard) {
                g.dependents.remove(key);
            }
            self.maybe_drop(&guard);
        }
    }

    fn link(&mut self, instance: &str, live: &str, key: &str) {
        self.by_live
            .entry((instance.to_string(), live.to_string()))
            .or_default()
            .insert(key.to_string());
        if let Some(inst) = self.instances.get_mut(instance) {
            inst.release.remove(live);
        }
    }

    fn unlink(&mut self, instance: &str, live: &str, key: &str) {
        let id = (instance.to_string(), live.to_string());
        let Some(keys) = self.by_live.get_mut(&id) else {
            return;
        };
        keys.remove(key);
        if keys.is_empty() {
            self.by_live.remove(&id);
            self.schedule_release(instance, live);
        }
    }

    fn schedule_release(&mut self, instance: &str, live: &str) {
        if let Some(inst) = self.instances.get_mut(instance)
            && inst.online
        {
            inst.release.insert(live.to_string());
        }
    }

    /// Stores a state and hands it to the entry's clients when it changed.
    fn set_cached(&mut self, key: &str, cached: Cached) {
        let Some(entry) = self.entries.get_mut(key) else {
            return;
        };
        if entry.cached.as_ref() == Some(&cached) {
            return;
        }
        for client in &entry.clients {
            self.deliveries.push((*client, cached.item(key)));
        }
        entry.cached = Some(cached);
    }

    /// Re-resolves every name binding (and guard) of `instance`.
    fn resolve_named(&mut self, instance: &str) {
        let keys: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, e)| e.instance == instance && (e.named || !e.dependents.is_empty()))
            .map(|(k, _)| k.clone())
            .collect();
        if let Some(inst) = self.instances.get_mut(instance) {
            inst.dirty.extend(keys);
        }
    }

    /// The answer to the `add_listener` number `seq` of `key`.
    fn resolved(&mut self, instance: &str, key: &str, seq: u64, slot: &Value) {
        let outcome = parse_slot(slot);
        let current = self.entries.get(key).filter(|e| e.seq == seq);
        let Some(entry) = current else {
            // A newer request is in flight, or nobody wants the key now:
            // the listener stays only if another entry holds it.
            if let Ok((live, _)) = outcome
                && !self
                    .by_live
                    .contains_key(&(instance.to_string(), live.clone()))
            {
                self.schedule_release(instance, &live);
            }
            return;
        };
        let wants_display = entry.display;
        let guard_only = entry.clients.is_empty();
        let old = entry.live.clone();
        match outcome {
            Ok((live, cached)) => {
                if old.as_deref() != Some(live.as_str()) {
                    if let Some(old) = &old {
                        self.unlink(instance, old, key);
                    }
                    self.link(instance, &live, key);
                    if let Some(entry) = self.entries.get_mut(key) {
                        entry.live = Some(live);
                    }
                }
                let cached = match cached {
                    Cached::Value { value, display } => Cached::Value {
                        value,
                        display: display.filter(|_| wants_display),
                    },
                    error @ Cached::Error(_) => error,
                };
                self.set_cached(key, cached);
            }
            Err(error) => {
                if guard_only {
                    // A guard keeps its object: a rename back fires it again.
                    return;
                }
                if let Some(old) = &old {
                    self.unlink(instance, old, key);
                }
                if let Some(entry) = self.entries.get_mut(key) {
                    entry.live = None;
                }
                self.set_cached(key, Cached::Error(error));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOLUME: &str = "live_set tracks[name=Hand1 #] mixer_device volume";
    const VOLUME_KEY: &str = "band|live_set tracks[name=Hand1 #] mixer_device volume|value|true";
    const NAME_GUARD: &str = "band|live_set tracks[name=Hand1 #]|name|false";
    const LIST_GUARD: &str = "band|live_set|tracks|false";

    fn table() -> Subs {
        Subs::new(["band".to_string(), "master".to_string()])
    }

    fn online() -> Subs {
        let mut subs = table();
        subs.connected("band");
        subs.connected("master");
        subs
    }

    fn ok(key: &str, value: Value) -> Value {
        json!({"ok": true, "data": {"key": key, "value": value}})
    }

    fn ok_display(key: &str, value: Value, display: &str) -> Value {
        json!({"ok": true, "data": {"key": key, "value": value, "display": display}})
    }

    fn fail(error: &str) -> Value {
        json!({"ok": false, "error": error, "errorType": "PathError"})
    }

    fn push(key: &str, value: Value) -> LiveValue {
        LiveValue {
            key: key.to_string(),
            value,
            display: None,
            error: None,
        }
    }

    fn gone(key: &str) -> LiveValue {
        LiveValue {
            key: key.to_string(),
            value: Value::Null,
            display: None,
            error: Some("gone".to_string()),
        }
    }

    /// The (target, name, prop) of every command sent.
    fn commands(out: &[Outgoing]) -> Vec<(String, String, String)> {
        out.iter()
            .flat_map(|o| o.commands.iter())
            .map(|c| {
                let target = match &c["target"] {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                (
                    target,
                    c["name"].as_str().unwrap().to_string(),
                    c["args"]["prop"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    /// Answers every add_listener with a key derived from its target and
    /// prop, the given value for the volume and names for the guards.
    fn answer_all(subs: &mut Subs, out: &[Outgoing]) {
        for o in out {
            let slots: Vec<Value> = o
                .commands
                .iter()
                .map(|c| {
                    let target = c["target"].as_str().unwrap_or("");
                    let prop = c["args"]["prop"].as_str().unwrap();
                    match (target, prop) {
                        (VOLUME, "value") => ok_display("live_10.value", json!(0.85), "0.0 dB"),
                        ("live_set tracks[name=Hand1 #]", "name") => {
                            ok("live_11.name", json!("Hand1 #"))
                        }
                        ("live_set", "tracks") => ok("live_1.tracks", json!([])),
                        ("live_set", "is_playing") => ok("live_1.is_playing", json!(false)),
                        _ => json!({"ok": true, "data": null}),
                    }
                })
                .collect();
            subs.on_result(&o.instance, &o.uuid, &slots);
        }
    }

    fn sub_volume(subs: &mut Subs, client: ClientId) -> SubReply {
        subs.subscribe(client, "band", VOLUME, "value", true)
            .unwrap()
    }

    #[test]
    fn the_first_subscriber_resolves_the_path_and_its_guards() {
        let mut subs = online();
        let reply = sub_volume(&mut subs, 1);
        assert_eq!(reply.key, VOLUME_KEY);
        assert_eq!(reply.cached, None);
        let out = subs.drain_outgoing();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].instance, "band");
        let mut sent = commands(&out);
        sent.sort();
        assert_eq!(
            sent,
            vec![
                ("live_set".into(), "add_listener".into(), "tracks".into()),
                (
                    "live_set tracks[name=Hand1 #]".into(),
                    "add_listener".into(),
                    "name".into()
                ),
                (VOLUME.into(), "add_listener".into(), "value".into()),
            ]
        );
        let volume = out[0]
            .commands
            .iter()
            .find(|c| c["target"] == VOLUME)
            .unwrap();
        assert_eq!(volume["args"], json!({"prop": "value", "display": true}));
        let guard = out[0]
            .commands
            .iter()
            .find(|c| c["target"] == "live_set")
            .unwrap();
        assert_eq!(
            guard["args"],
            json!({"prop": "tracks"}),
            "no display for guards"
        );
        assert!(subs.drain_outgoing().is_empty(), "sent once");
        answer_all(&mut subs, &out);
        assert_eq!(
            subs.take_deliveries(),
            vec![(
                1,
                ValueItem::value(VOLUME_KEY, json!(0.85), Some("0.0 dB".into()))
            )]
        );
        assert_eq!(subs.subscriptions("band"), 1);
        assert_eq!(subs.listeners("band"), 3);
        assert_eq!(subs.listeners("master"), 0);
        assert_eq!(
            subs.cached(VOLUME_KEY),
            Some(&Cached::Value {
                value: json!(0.85),
                display: Some("0.0 dB".into())
            })
        );
    }

    #[test]
    fn a_second_subscriber_shares_the_listener_and_gets_the_cached_value() {
        let mut subs = online();
        sub_volume(&mut subs, 1);
        let out = subs.drain_outgoing();
        answer_all(&mut subs, &out);
        subs.take_deliveries();
        let reply = sub_volume(&mut subs, 2);
        assert_eq!(
            reply.cached,
            Some(Cached::Value {
                value: json!(0.85),
                display: Some("0.0 dB".into())
            })
        );
        assert!(subs.drain_outgoing().is_empty(), "no second add_listener");
        assert_eq!(subs.subscriptions("band"), 1);
        subs.on_values("band", &[push("live_10.value", json!(0.5))]);
        let mut got = subs.take_deliveries();
        got.sort_by_key(|(c, _)| *c);
        assert_eq!(
            got,
            vec![
                (1, ValueItem::value(VOLUME_KEY, json!(0.5), None)),
                (2, ValueItem::value(VOLUME_KEY, json!(0.5), None)),
            ]
        );
        // The same value again is no news.
        subs.on_values("band", &[push("live_10.value", json!(0.5))]);
        assert!(subs.take_deliveries().is_empty());
    }

    #[test]
    fn the_last_unsubscribe_removes_the_listeners() {
        let mut subs = online();
        sub_volume(&mut subs, 1);
        sub_volume(&mut subs, 2);
        let out = subs.drain_outgoing();
        answer_all(&mut subs, &out);
        assert!(subs.unsubscribe(1, VOLUME_KEY));
        assert!(!subs.unsubscribe(1, VOLUME_KEY), "already gone");
        assert!(!subs.unsubscribe(1, "band|nothing|x|false"));
        assert!(subs.drain_outgoing().is_empty(), "client 2 still listens");
        assert!(subs.unsubscribe(2, VOLUME_KEY));
        let out = subs.drain_outgoing();
        let mut sent = commands(&out);
        sent.sort();
        assert_eq!(
            sent,
            vec![
                (
                    "{\"$ref\":\"live_1\"}".into(),
                    "remove_listener".into(),
                    "tracks".into()
                ),
                (
                    "{\"$ref\":\"live_10\"}".into(),
                    "remove_listener".into(),
                    "value".into()
                ),
                (
                    "{\"$ref\":\"live_11\"}".into(),
                    "remove_listener".into(),
                    "name".into()
                ),
            ]
        );
        assert_eq!(subs.subscriptions("band"), 0);
        assert_eq!(subs.listeners("band"), 0);
        assert!(subs.cached(VOLUME_KEY).is_none());
        let ok_all: Vec<Value> = out[0]
            .commands
            .iter()
            .map(|_| json!({"ok": true, "data": null}))
            .collect();
        assert!(subs.on_result("band", &out[0].uuid, &ok_all));
        assert!(
            !subs.on_result("band", &out[0].uuid, &ok_all),
            "answered once"
        );
        assert!(subs.drain_outgoing().is_empty());
    }

    #[test]
    fn a_dropped_client_leaves_every_subscription() {
        let mut subs = online();
        sub_volume(&mut subs, 7);
        subs.subscribe(7, "band", "live_set", "is_playing", false)
            .unwrap();
        subs.subscribe(8, "band", "live_set", "is_playing", false)
            .unwrap();
        let out = subs.drain_outgoing();
        answer_all(&mut subs, &out);
        subs.drop_client(7);
        assert_eq!(subs.subscriptions("band"), 1, "client 8's is_playing stays");
        let out = subs.drain_outgoing();
        let mut sent: Vec<String> = commands(&out)
            .iter()
            .map(|(_, n, p)| format!("{n} {p}"))
            .collect();
        sent.sort();
        assert_eq!(
            sent,
            vec![
                "remove_listener name",
                "remove_listener tracks",
                "remove_listener value"
            ]
        );
    }

    #[test]
    fn two_paths_to_one_object_share_its_live_key() {
        let mut subs = online();
        let by_name = subs.subscribe(
            1,
            "band",
            "live_set tracks[name=Hand1 #] mute",
            "mute",
            false,
        );
        assert!(by_name.is_ok());
        let by_name = subs
            .subscribe(1, "band", "live_set tracks[name=Hand1 #]", "mute", false)
            .unwrap();
        let by_index = subs
            .subscribe(2, "band", "live_set tracks 0", "mute", false)
            .unwrap();
        subs.unsubscribe(1, "band|live_set tracks[name=Hand1 #] mute|mute|false");
        let out = subs.drain_outgoing();
        let slots: Vec<Value> = out[0]
            .commands
            .iter()
            .map(|c| {
                match (
                    c["target"].as_str().unwrap(),
                    c["args"]["prop"].as_str().unwrap(),
                ) {
                    ("live_set tracks[name=Hand1 #]", "mute") | ("live_set tracks 0", "mute") => {
                        ok("live_5.mute", json!(false))
                    }
                    ("live_set tracks[name=Hand1 #]", "name") => {
                        ok("live_5.name", json!("Hand1 #"))
                    }
                    _ => ok("live_1.tracks", json!([])),
                }
            })
            .collect();
        subs.on_result("band", &out[0].uuid, &slots);
        assert_eq!(subs.listeners("band"), 3);
        subs.on_values("band", &[push("live_5.mute", json!(true))]);
        let mut got = subs.take_deliveries();
        got.sort_by_key(|(c, _)| *c);
        assert_eq!(
            got.iter()
                .filter(|(_, i)| i.value == Some(json!(true)))
                .count(),
            2
        );
        // One of the two leaves: the shared listener stays.
        subs.unsubscribe(2, &by_index.key);
        assert!(
            commands(&subs.drain_outgoing()).is_empty(),
            "live_5.mute is still held by the name path"
        );
        subs.unsubscribe(1, &by_name.key);
        let removed = commands(&subs.drain_outgoing());
        assert_eq!(removed.len(), 3, "{removed:?}");
    }

    #[test]
    fn a_rename_turns_the_binding_into_an_error_and_a_rename_back_heals_it() {
        let mut subs = online();
        sub_volume(&mut subs, 1);
        let out = subs.drain_outgoing();
        answer_all(&mut subs, &out);
        subs.take_deliveries();
        // The track is renamed: its name guard fires.
        subs.on_values("band", &[push("live_11.name", json!("Hand9 #"))]);
        assert!(subs.take_deliveries().is_empty(), "guards deliver nothing");
        let out = subs.drain_outgoing();
        let mut sent = commands(&out);
        sent.sort();
        assert_eq!(sent.len(), 3, "the binding and both guards: {sent:?}");
        let slots: Vec<Value> = out[0]
            .commands
            .iter()
            .map(|c| match c["args"]["prop"].as_str().unwrap() {
                "tracks" => ok("live_1.tracks", json!([])),
                _ => fail("not found: tracks[name=Hand1 #]"),
            })
            .collect();
        subs.on_result("band", &out[0].uuid, &slots);
        assert_eq!(
            subs.take_deliveries(),
            vec![(
                1,
                ValueItem::error(VOLUME_KEY, "not found: tracks[name=Hand1 #]")
            )]
        );
        // The stale listener is removed; the name guard keeps its object.
        assert_eq!(
            commands(&subs.drain_outgoing()),
            vec![(
                "{\"$ref\":\"live_10\"}".into(),
                "remove_listener".into(),
                "value".into()
            )]
        );
        assert_eq!(subs.listeners("band"), 2);
        // A later push of the removed key is nobody's.
        subs.on_values("band", &[push("live_10.value", json!(0.1))]);
        assert!(subs.take_deliveries().is_empty());
        // Renamed back: the guard fires again and the binding resolves.
        subs.on_values("band", &[push("live_11.name", json!("Hand1 #"))]);
        let out = subs.drain_outgoing();
        answer_all(&mut subs, &out);
        assert_eq!(
            subs.take_deliveries(),
            vec![(
                1,
                ValueItem::value(VOLUME_KEY, json!(0.85), Some("0.0 dB".into()))
            )]
        );
    }

    #[test]
    fn removals_wait_for_resolutions_in_flight() {
        let mut subs = online();
        let a = subs
            .subscribe(1, "band", "live_set tracks 0", "mute", false)
            .unwrap();
        let out = subs.drain_outgoing();
        subs.on_result("band", &out[0].uuid, &[ok("live_5.mute", json!(false))]);
        // B resolves to the same key while A leaves: its request is out first.
        subs.subscribe(2, "band", "live_set tracks 0", "solo", false)
            .unwrap();
        let resolving = subs.drain_outgoing();
        subs.unsubscribe(1, &a.key);
        assert!(
            subs.drain_outgoing().is_empty(),
            "no remove_listener while an add_listener is in flight"
        );
        subs.on_result(
            "band",
            &resolving[0].uuid,
            &[ok("live_5.solo", json!(false))],
        );
        assert_eq!(
            commands(&subs.drain_outgoing()),
            vec![(
                "{\"$ref\":\"live_5\"}".into(),
                "remove_listener".into(),
                "mute".into()
            )]
        );
    }

    #[test]
    fn a_removal_is_cancelled_when_the_key_is_wanted_again() {
        let mut subs = online();
        let a = subs
            .subscribe(1, "band", "live_set tracks 0", "mute", false)
            .unwrap();
        let out = subs.drain_outgoing();
        subs.on_result("band", &out[0].uuid, &[ok("live_5.mute", json!(false))]);
        subs.subscribe(2, "band", "live_set tracks[name=Hand1 #]", "mute", false)
            .unwrap();
        let resolving = subs.drain_outgoing();
        subs.unsubscribe(1, &a.key);
        let slots: Vec<Value> = resolving[0]
            .commands
            .iter()
            .map(|c| match c["args"]["prop"].as_str().unwrap() {
                "mute" => ok("live_5.mute", json!(false)),
                "name" => ok("live_5.name", json!("Hand1 #")),
                _ => ok("live_1.tracks", json!([])),
            })
            .collect();
        subs.on_result("band", &resolving[0].uuid, &slots);
        assert!(
            commands(&subs.drain_outgoing()).is_empty(),
            "live_5.mute is held again"
        );
    }

    #[test]
    fn a_stale_answer_is_ignored_and_an_unwanted_key_released() {
        let mut subs = online();
        let reply = subs
            .subscribe(1, "band", "live_set tracks 0", "mute", false)
            .unwrap();
        let first = subs.drain_outgoing();
        // Re-resolved (a reconnect) before the first answer arrived.
        subs.disconnected("band");
        subs.connected("band");
        let second = subs.drain_outgoing();
        assert!(
            !subs.on_result("band", &first[0].uuid, &[ok("live_9.mute", json!(true))]),
            "a disconnect forgot the old session's requests"
        );
        assert!(subs.take_deliveries().is_empty());
        subs.on_result("band", &second[0].uuid, &[ok("live_5.mute", json!(false))]);
        assert_eq!(
            subs.take_deliveries(),
            vec![(1, ValueItem::value(&reply.key, json!(false), None))]
        );
        // An answer for an entry that is gone releases its key.
        subs.subscribe(2, "band", "live_set tracks 1", "mute", false)
            .unwrap();
        let out = subs.drain_outgoing();
        subs.unsubscribe(2, "band|live_set tracks 1|mute|false");
        subs.on_result("band", &out[0].uuid, &[ok("live_6.mute", json!(false))]);
        assert_eq!(
            commands(&subs.drain_outgoing()),
            vec![(
                "{\"$ref\":\"live_6\"}".into(),
                "remove_listener".into(),
                "mute".into()
            )]
        );
        // A superseded request (a newer one is in flight) does not win.
        subs.on_values("band", &[]);
        let key = reply.key.clone();
        subs.connected("band");
        let older = subs.drain_outgoing();
        subs.connected("band");
        let newer = subs.drain_outgoing();
        subs.on_result("band", &newer[0].uuid, &[ok("live_5.mute", json!(true))]);
        subs.on_result("band", &older[0].uuid, &[ok("live_5.mute", json!(false))]);
        assert_eq!(
            subs.cached(&key),
            Some(&Cached::Value {
                value: json!(true),
                display: None
            })
        );
    }

    #[test]
    fn a_superseded_answer_of_the_same_session_keeps_the_newer_state() {
        let mut subs = online();
        let reply = subs
            .subscribe(1, "band", "live_set tracks[name=Keys]", "mute", false)
            .unwrap();
        let out = subs.drain_outgoing();
        let answer = |c: &Value| match c["args"]["prop"].as_str().unwrap() {
            "mute" => ok("live_5.mute", json!(false)),
            "name" => ok("live_5.name", json!("Keys")),
            _ => ok("live_1.tracks", json!([])),
        };
        let slots: Vec<Value> = out[0].commands.iter().map(answer).collect();
        subs.on_result("band", &out[0].uuid, &slots);
        // Two list changes: two re-resolutions in flight.
        subs.on_values("band", &[push("live_1.tracks", json!([1]))]);
        let older = subs.drain_outgoing();
        subs.on_values("band", &[push("live_1.tracks", json!([1, 2]))]);
        let newer = subs.drain_outgoing();
        assert!(!older.is_empty() && !newer.is_empty());
        let newer_slots: Vec<Value> = newer[0].commands.iter().map(answer).collect();
        subs.on_result("band", &newer[0].uuid, &newer_slots);
        let older_slots: Vec<Value> = older[0]
            .commands
            .iter()
            .map(|c| match c["args"]["prop"].as_str().unwrap() {
                "mute" => ok("live_8.mute", json!(true)),
                _ => answer(c),
            })
            .collect();
        subs.on_result("band", &older[0].uuid, &older_slots);
        assert_eq!(
            subs.cached(&reply.key),
            Some(&Cached::Value {
                value: json!(false),
                display: None
            })
        );
        assert_eq!(
            commands(&subs.drain_outgoing()),
            vec![(
                "{\"$ref\":\"live_8\"}".into(),
                "remove_listener".into(),
                "mute".into()
            )],
            "the superseded answer's key is nobody's"
        );
    }

    #[test]
    fn offline_subscriptions_resolve_on_connect_and_a_disconnect_clears_values() {
        let mut subs = table();
        let reply = subs
            .subscribe(1, "band", "live_set", "is_playing", false)
            .unwrap();
        assert!(subs.drain_outgoing().is_empty(), "offline: nothing sent");
        assert!(!subs.is_online("band"));
        subs.connected("band");
        assert!(subs.is_online("band"));
        let out = subs.drain_outgoing();
        assert_eq!(
            commands(&out),
            vec![(
                "live_set".into(),
                "add_listener".into(),
                "is_playing".into()
            )]
        );
        answer_all(&mut subs, &out);
        assert_eq!(
            subs.take_deliveries(),
            vec![(1, ValueItem::value(&reply.key, json!(false), None))]
        );
        subs.disconnected("band");
        assert!(!subs.is_online("band"));
        assert!(
            subs.cached(&reply.key).is_none(),
            "no stale value while offline"
        );
        assert_eq!(subs.listeners("band"), 0);
        subs.on_values("band", &[push("live_1.is_playing", json!(true))]);
        assert!(
            subs.take_deliveries().is_empty(),
            "offline pushes are ignored"
        );
        // A new subscriber while offline gets no value.
        assert_eq!(
            subs.subscribe(2, "band", "live_set", "is_playing", false)
                .unwrap()
                .cached,
            None
        );
        subs.connected("band");
        let out = subs.drain_outgoing();
        assert_eq!(out.len(), 1);
        answer_all(&mut subs, &out);
        let mut got = subs.take_deliveries();
        got.sort_by_key(|(c, _)| *c);
        assert_eq!(got.len(), 2, "both get the fresh value");
        // Unknown instances are no-ops.
        subs.connected("nowhere");
        subs.disconnected("nowhere");
        assert!(!subs.is_online("nowhere"));
    }

    #[test]
    fn a_disconnect_drops_the_instances_requests_in_flight() {
        let mut subs = online();
        subs.subscribe(1, "band", "live_set", "is_playing", false)
            .unwrap();
        subs.subscribe(1, "master", "live_set", "is_playing", false)
            .unwrap();
        let out = subs.drain_outgoing();
        assert_eq!(out.len(), 2);
        subs.disconnected("band");
        let band = out.iter().find(|o| o.instance == "band").unwrap();
        let master = out.iter().find(|o| o.instance == "master").unwrap();
        assert!(!subs.on_result("band", &band.uuid, &[ok("live_1.is_playing", json!(true))]));
        assert!(subs.on_result(
            "master",
            &master.uuid,
            &[ok("live_1.is_playing", json!(true))]
        ));
        assert_eq!(subs.take_deliveries().len(), 1);
        // A uuid answered by the wrong instance is dropped.
        subs.connected("band");
        let again = subs.drain_outgoing();
        assert!(subs.on_result(
            "master",
            &again[0].uuid,
            &[ok("live_1.is_playing", json!(true))]
        ));
        assert!(subs.take_deliveries().is_empty());
    }

    #[test]
    fn a_gone_object_is_an_error_without_a_removal() {
        let mut subs = online();
        let reply = subs
            .subscribe(1, "band", "live_set tracks 0", "mute", false)
            .unwrap();
        let out = subs.drain_outgoing();
        subs.on_result("band", &out[0].uuid, &[ok("live_5.mute", json!(false))]);
        subs.take_deliveries();
        subs.on_values("band", &[gone("live_5.mute")]);
        assert_eq!(
            subs.take_deliveries(),
            vec![(1, ValueItem::error(&reply.key, "gone"))]
        );
        assert_eq!(subs.listeners("band"), 0);
        assert!(
            subs.drain_outgoing().is_empty(),
            "the script dropped it itself"
        );
        // A gone guard re-resolves the name bindings.
        sub_volume(&mut subs, 2);
        let out = subs.drain_outgoing();
        answer_all(&mut subs, &out);
        subs.on_values("band", &[gone("live_11.name")]);
        let sent = commands(&subs.drain_outgoing());
        assert!(sent.iter().any(|(t, _, _)| t == VOLUME), "{sent:?}");
    }

    #[test]
    fn a_failed_first_resolution_is_an_error_for_the_subscribers() {
        let mut subs = online();
        let reply = subs
            .subscribe(1, "band", "live_set tracks 99", "mute", false)
            .unwrap();
        let out = subs.drain_outgoing();
        subs.on_result("band", &out[0].uuid, &[fail("not found: tracks 99")]);
        assert_eq!(
            subs.take_deliveries(),
            vec![(1, ValueItem::error(&reply.key, "not found: tracks 99"))]
        );
        let again = subs
            .subscribe(2, "band", "live_set tracks 99", "mute", false)
            .unwrap();
        assert_eq!(
            again.cached,
            Some(Cached::Error("not found: tracks 99".into()))
        );
        // A missing slot and a slot without a key are errors too.
        let reply = subs
            .subscribe(3, "band", "live_set tracks 1", "mute", false)
            .unwrap();
        let out = subs.drain_outgoing();
        subs.on_result("band", &out[0].uuid, &[]);
        assert_eq!(
            subs.take_deliveries(),
            vec![(3, ValueItem::error(&reply.key, "no result"))]
        );
        subs.on_values("band", &[]);
        subs.connected("band");
        let out = subs.drain_outgoing();
        let slots: Vec<Value> = out[0]
            .commands
            .iter()
            .map(|_| json!({"ok": true, "data": {"value": 1}}))
            .collect();
        subs.on_result("band", &out[0].uuid, &slots);
        let got = subs.take_deliveries();
        assert!(
            got.iter()
                .any(|(_, i)| i.error.as_deref() == Some("add_listener answered without a key")),
            "{got:?}"
        );
        assert_eq!(
            subs.cached(&reply.key),
            Some(&Cached::Error("add_listener answered without a key".into()))
        );
    }

    #[test]
    fn a_display_string_goes_only_to_keys_that_asked_for_one() {
        let mut subs = online();
        let plain = subs
            .subscribe(
                1,
                "band",
                "live_set tracks 0 mixer_device volume",
                "value",
                false,
            )
            .unwrap();
        let out = subs.drain_outgoing();
        assert_eq!(out[0].commands[0]["args"], json!({"prop": "value"}));
        subs.on_result(
            "band",
            &out[0].uuid,
            &[ok_display("live_10.value", json!(0.85), "0.0 dB")],
        );
        assert_eq!(
            subs.take_deliveries(),
            vec![(1, ValueItem::value(&plain.key, json!(0.85), None))]
        );
        subs.on_values(
            "band",
            &[LiveValue {
                key: "live_10.value".into(),
                value: json!(0.5),
                display: Some("-6.0 dB".into()),
                error: None,
            }],
        );
        assert_eq!(
            subs.take_deliveries(),
            vec![(1, ValueItem::value(&plain.key, json!(0.5), None))]
        );
    }

    #[test]
    fn bad_requests_are_refused_with_their_key() {
        let mut subs = online();
        assert_eq!(
            subs.subscribe(1, "drums", "live_set", "is_playing", false),
            Err((
                "drums|live_set|is_playing|false".into(),
                "unknown instance \"drums\"".into()
            ))
        );
        assert_eq!(
            subs.subscribe(1, "band", "live_set", "", false)
                .unwrap_err()
                .1,
            "bad prop \"\""
        );
        assert_eq!(
            subs.subscribe(1, "band", "live_set", "_x", false)
                .unwrap_err()
                .1,
            "bad prop \"_x\""
        );
        assert_eq!(
            subs.subscribe(1, "band", "live_set tracks[name=x", "mute", false),
            Err((
                "band|live_set tracks[name=x|mute|false".into(),
                "syntax: unterminated [name=".into()
            ))
        );
        assert_eq!(subs.subscriptions("band"), 0);
        assert!(subs.drain_outgoing().is_empty());
    }

    #[test]
    fn requests_are_split_into_batches() {
        let mut subs = online();
        for i in 0..(BATCH_MAX + 6) {
            subs.subscribe(1, "band", &format!("live_set tracks {i}"), "mute", false)
                .unwrap();
        }
        let out = subs.drain_outgoing();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].commands.len(), BATCH_MAX);
        assert_eq!(out[1].commands.len(), 6);
        assert_ne!(out[0].uuid, out[1].uuid);
    }

    #[test]
    fn a_guard_that_gets_a_client_and_loses_it_stays_a_guard() {
        let mut subs = online();
        sub_volume(&mut subs, 1);
        // The client also listens to the track name itself (the guard key).
        let name = subs
            .subscribe(2, "band", "live_set tracks[name=Hand1 #]", "name", false)
            .unwrap();
        assert_eq!(name.key, NAME_GUARD);
        let out = subs.drain_outgoing();
        answer_all(&mut subs, &out);
        assert_eq!(subs.subscriptions("band"), 2);
        subs.take_deliveries();
        subs.unsubscribe(2, NAME_GUARD);
        assert!(
            commands(&subs.drain_outgoing()).is_empty(),
            "still the volume's guard"
        );
        assert_eq!(subs.subscriptions("band"), 1);
        // Renamed: the guard (now guard-only) fails to resolve and keeps its object.
        subs.on_values("band", &[push("live_11.name", json!("Other"))]);
        let out = subs.drain_outgoing();
        let slots: Vec<Value> = out[0]
            .commands
            .iter()
            .map(|c| match c["args"]["prop"].as_str().unwrap() {
                "tracks" => ok("live_1.tracks", json!([])),
                _ => fail("not found"),
            })
            .collect();
        subs.on_result("band", &out[0].uuid, &slots);
        assert_eq!(
            subs.listeners("band"),
            2,
            "the name guard and the list guard"
        );
        assert!(subs.cached(LIST_GUARD).is_some());
        subs.unsubscribe(1, VOLUME_KEY);
        let removed = commands(&subs.drain_outgoing());
        assert_eq!(
            removed.len(),
            3,
            "the failed binding's old key and both guards: {removed:?}"
        );
        assert!(subs.cached(NAME_GUARD).is_none());
        assert_eq!(subs.listeners("band"), 0);
    }

    #[test]
    fn guard_targets_cover_every_name_step() {
        let path = LomPath::parse("live_set tracks[name=A] devices[name=B] parameters 1").unwrap();
        assert_eq!(
            guard_targets(&path),
            vec![
                ("live_set".to_string(), "tracks".to_string()),
                ("live_set tracks[name=A]".to_string(), "name".to_string()),
                ("live_set tracks[name=A]".to_string(), "devices".to_string()),
                (
                    "live_set tracks[name=A] devices[name=B]".to_string(),
                    "name".to_string()
                ),
            ]
        );
        assert!(guard_targets(&LomPath::parse("live_set tracks 0 mute").unwrap()).is_empty());
    }

    #[test]
    fn cached_states_become_client_items() {
        assert_eq!(
            Cached::Value {
                value: json!(1),
                display: Some("x".into())
            }
            .item("k"),
            ValueItem::value("k", json!(1), Some("x".into()))
        );
        assert_eq!(
            Cached::Error("e".into()).item("k"),
            ValueItem::error("k", "e")
        );
    }

    #[test]
    fn slots_parse() {
        assert_eq!(
            parse_slot(&ok_display("live_1.value", json!(0.5), "x")),
            Ok((
                "live_1.value".into(),
                Cached::Value {
                    value: json!(0.5),
                    display: Some("x".into())
                }
            ))
        );
        assert_eq!(
            parse_slot(&json!({"ok": true, "data": {"key": "k"}})),
            Ok((
                "k".into(),
                Cached::Value {
                    value: Value::Null,
                    display: None
                }
            ))
        );
        assert_eq!(parse_slot(&fail("nope")), Err("nope".into()));
        assert_eq!(parse_slot(&json!({"ok": false})), Err("no result".into()));
        assert_eq!(parse_slot(&json!({})), Err("no result".into()));
        assert_eq!(
            parse_slot(&json!({"ok": true})),
            Err("add_listener answered without a key".into())
        );
    }
}
