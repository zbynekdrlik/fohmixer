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
//!   (spec I5), never a stale value — not even one pushed in the same frame
//!   as the rename — and a rename back gives the value again. A guard is an
//!   entry of its own (its key ends in `|guard`), never shared with a
//!   client's subscription, so a client may subscribe to exactly what a
//!   guard watches (a track's name) without the guard losing its object.
//! - A new subscriber of a name binding, or of a binding in error, resolves
//!   it again (a page switch rebinds this way, even when another client
//!   holds the binding): a renamed-back or newly ambiguous name is found
//!   then.
//! - While a binding a list guard guards is in error, the hub also watches
//!   the `name` of every item of that list (`tracks 0`, `tracks 1`, …; the
//!   list guard's value is the list, so its length is known): a name that
//!   was missing when its binding was made has no object for its name guard
//!   to listen to, and renaming another track to it changes no list. Any
//!   item's rename then resolves the name bindings again, and the binding
//!   heals by itself (#58: no REFRESH ALL). The watches follow the list's
//!   length and go once none of the list's bindings is in error. New
//!   watches send the list's bindings in error again after them: a rename
//!   that lands before a watch is heard is found by that resolution. A
//!   watch is resolved again only when a list changed (its index then holds
//!   another object), not on every rename.
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
    Resolve {
        key: String,
        seq: u64,
    },
    /// A `remove_listener` (its answer changes nothing: a failed removal
    /// means the object is gone, and its listener with it).
    Release,
}

/// A request in flight. A disconnect forgets its instance's requests, so an
/// answer from an older session never arrives here.
#[derive(Debug)]
struct Batch {
    instance: String,
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
    /// A guard (it has dependents, never clients).
    guard: bool,
    /// A list guard (it listens to the list a name step selects from).
    list: bool,
    /// An item name watch of a list guard (#58).
    watch: bool,
    clients: BTreeSet<ClientId>,
    /// Entries this one guards (it listens to a name or a list they depend
    /// on).
    dependents: BTreeSet<String>,
    /// The guard entries of this one.
    guards: Vec<String>,
    /// A list guard's item name watches, by index (#58).
    watches: Vec<String>,
    /// The Live listener key, while resolved.
    live: Option<String>,
    cached: Option<Cached>,
    /// The latest `add_listener` sent for this entry (numbered across the
    /// table: an entry dropped and made again never takes an older answer).
    seq: u64,
}

#[derive(Debug, Default)]
struct Instance {
    online: bool,
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
    next_seq: u64,
    deliveries: Vec<(ClientId, ValueItem)>,
    /// List guards whose item name watches may have to change (#58).
    watch_dirty: BTreeSet<String>,
}

/// The key of the guard entry watching `prop` of `target`: never a client's
/// key (those end in `|true` or `|false`).
fn guard_key(instance: &str, target: &str, prop: &str) -> String {
    format!("{}|guard", hub_key(instance, target, prop, false))
}

/// Whether a guard's firing resolves `entry` again: a name binding or a
/// guard; an item name watch on an index path (#58) only when a list
/// changed (`lists`: its index may hold another object now). A rename alone
/// leaves every index where it was.
fn re_resolves(entry: &Entry, lists: bool) -> bool {
    entry.named || (entry.guard && (lists || !entry.watch))
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
        let created = self.ensure(&key, instance, &path, prop, display, false);
        let entry = self.entries.get_mut(&key).expect("ensured above");
        entry.clients.insert(client);
        let cached = entry.cached.clone();
        let refresh = entry.named || matches!(entry.cached, Some(Cached::Error(_)));
        if created {
            self.add_guards(&key, instance, &path);
        } else if refresh && let Some(inst) = self.instances.get_mut(instance) {
            inst.dirty.insert(key.clone());
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
        for (key, entry) in self.entries.iter_mut() {
            if entry.instance == instance {
                entry.live = None;
                entry.cached = None;
                if entry.list {
                    self.watch_dirty.insert(key.clone());
                }
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
        // A guard in the frame: the name bindings are resolved again, and
        // their values of this frame may be the renamed object's.
        let fired = |test: fn(&Entry) -> bool| {
            items.iter().any(|item| {
                self.by_live
                    .get(&(instance.to_string(), item.key.clone()))
                    .is_some_and(|keys| keys.iter().any(|k| self.entries.get(k).is_some_and(test)))
            })
        };
        let guard_fired = fired(|e| e.guard);
        let list_fired = fired(|e| e.list);
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
                if item.error.is_some() {
                    entry.live = None;
                }
                if guard_fired && entry.named && !entry.guard {
                    continue;
                }
                let cached = match &item.error {
                    Some(error) => Cached::Error(error.clone()),
                    None => Cached::Value {
                        value: item.value.clone(),
                        display: item.display.clone().filter(|_| entry.display),
                    },
                };
                self.set_cached(&key, cached);
            }
        }
        if guard_fired {
            self.resolve_named(instance, list_fired);
        }
    }

    /// The result of a request this table asked for; false when `uuid` is
    /// not one of its requests.
    pub fn on_result(&mut self, instance: &str, uuid: &str, slots: &[Value]) -> bool {
        let Some(batch) = self.inflight.remove(uuid) else {
            return false;
        };
        if batch.instance != instance {
            return true;
        }
        let missing = json!({"ok": false, "error": "no result"});
        for (i, tag) in batch.tags.iter().enumerate() {
            let slot = slots.get(i).unwrap_or(&missing);
            match tag {
                Tag::Resolve { key, seq } => self.resolved(instance, key, *seq, slot),
                Tag::Release => {}
            }
        }
        true
    }

    /// The requests to send now, in order, per instance: first the removals
    /// (only while no `add_listener` is in flight), then the resolutions.
    pub fn drain_outgoing(&mut self) -> Vec<Outgoing> {
        self.sync_watches();
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
                        Tag::Release,
                    ));
                }
            }
            for key in std::mem::take(&mut inst.dirty) {
                let Some(entry) = self.entries.get_mut(&key) else {
                    continue;
                };
                self.next_seq += 1;
                entry.seq = self.next_seq;
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

    /// Live's value of a write key (`instance|target|prop`, #43 PR D) as
    /// the hub knows it: the cached value it pushes to pages, of the
    /// subscription with the display string (a strip's) or else without;
    /// none while no subscription holds a value (none subscribed, not
    /// resolved yet, or in error).
    pub fn live_value(&self, write_key: &str) -> Option<&Value> {
        [true, false].into_iter().find_map(|display| {
            match self.cached(&format!("{write_key}|{display}")) {
                Some(Cached::Value { value, .. }) => Some(value),
                _ => None,
            }
        })
    }

    /// Client subscriptions (hub keys with a subscriber) of `instance`.
    pub fn subscriptions(&self, instance: &str) -> usize {
        self.entries
            .values()
            .filter(|e| e.instance == instance && !e.clients.is_empty())
            .count()
    }

    /// Client subscriptions of `instance` held by a client besides the `own`
    /// ones (the router's unfold keeper, #58, and marker keeper, #68: their
    /// subscriptions are the hub's own).
    pub fn subscriptions_besides(&self, instance: &str, own: &[ClientId]) -> usize {
        self.entries
            .values()
            .filter(|e| e.instance == instance && e.clients.iter().any(|c| !own.contains(c)))
            .count()
    }

    /// Live listeners held on `instance` (distinct Live keys).
    pub fn listeners(&self, instance: &str) -> usize {
        self.by_live.keys().filter(|(i, _)| i == instance).count()
    }

    // --- internals ---

    /// Creates the entry `key` if it does not exist (to be resolved); true
    /// when it did not.
    fn ensure(
        &mut self,
        key: &str,
        instance: &str,
        path: &LomPath,
        prop: &str,
        display: bool,
        guard: bool,
    ) -> bool {
        if self.entries.contains_key(key) {
            return false;
        }
        self.entries.insert(
            key.to_string(),
            Entry {
                instance: instance.to_string(),
                target: path.text(),
                prop: prop.to_string(),
                display,
                named: path.steps.iter().any(|s| s.name.is_some()),
                guard,
                list: false,
                watch: false,
                clients: BTreeSet::new(),
                dependents: BTreeSet::new(),
                guards: Vec::new(),
                watches: Vec::new(),
                live: None,
                cached: None,
                seq: 0,
            },
        );
        if let Some(inst) = self.instances.get_mut(instance) {
            inst.dirty.insert(key.to_string());
        }
        true
    }

    /// Guards `key` (a client subscription) against renames and list
    /// changes along its name steps.
    fn add_guards(&mut self, key: &str, instance: &str, path: &LomPath) {
        let mut guards: Vec<String> = Vec::new();
        for (target, prop) in guard_targets(path) {
            let guard = guard_key(instance, &target, &prop);
            let guard_path = LomPath::parse(&target).expect("a prefix of a parsed path parses");
            self.ensure(&guard, instance, &guard_path, &prop, false, true);
            if let Some(entry) = self.entries.get_mut(&guard) {
                entry.dependents.insert(key.to_string());
                // The list guard listens to a list attribute (`tracks`), the
                // name guard to `name`.
                entry.list = prop != "name";
                if entry.list {
                    self.watch_dirty.insert(guard.clone());
                }
            }
            guards.push(guard);
        }
        if let Some(entry) = self.entries.get_mut(key) {
            entry.guards = guards;
        }
    }

    /// Removes `key` when nothing needs it any more (no client, guarding
    /// nobody), and then its guards that guard nobody else.
    fn maybe_drop(&mut self, key: &str) {
        let Some(entry) = self.entries.get(key) else {
            return;
        };
        if !entry.clients.is_empty() || !entry.dependents.is_empty() {
            return;
        }
        let entry = self.entries.remove(key).expect("present above");
        if let Some(inst) = self.instances.get_mut(&entry.instance) {
            inst.dirty.remove(key);
        }
        if let Some(live) = &entry.live {
            self.unlink(&entry.instance, live, key);
        }
        self.watch_dirty.remove(key);
        for guard in entry.guards.into_iter().chain(entry.watches) {
            if let Some(g) = self.entries.get_mut(&guard) {
                g.dependents.remove(key);
                if g.list {
                    self.watch_dirty.insert(guard.clone());
                }
            }
            self.maybe_drop(&guard);
        }
    }

    /// Brings the item name watches of every list guard that may need it in
    /// line (#58): one per item of the list while any entry the list guard
    /// guards is in error, none otherwise (also while the list has no
    /// value: offline, or not resolved yet).
    fn sync_watches(&mut self) {
        for list_key in std::mem::take(&mut self.watch_dirty) {
            let Some(list) = self.entries.get(&list_key).filter(|e| e.list) else {
                continue;
            };
            // The list's bindings in error and their guards (a name guard
            // made while its name was missing holds no object yet).
            let again: Vec<String> = list
                .dependents
                .iter()
                .filter_map(|d| self.entries.get(d).map(|e| (d, e)))
                .filter(|(_, e)| matches!(e.cached, Some(Cached::Error(_))))
                .flat_map(|(d, e)| std::iter::once(d.clone()).chain(e.guards.iter().cloned()))
                .collect();
            let in_error = !again.is_empty();
            let wanted = match &list.cached {
                Some(Cached::Value {
                    value: Value::Array(items),
                    ..
                }) if in_error => items.len(),
                _ => 0,
            };
            let have = list.watches.len();
            let instance = list.instance.clone();
            let owner = format!("{} {}", list.target, list.prop);
            // With new watches, sent after them (an index target sorts
            // before a name step's `[`): a rename that lands before a watch
            // is heard is found by this resolution.
            let mut again = Some(again);
            for index in have..wanted {
                if let Some(keys) = again.take()
                    && let Some(inst) = self.instances.get_mut(&instance)
                {
                    inst.dirty.extend(keys);
                }
                let target = format!("{owner} {index}");
                let path =
                    LomPath::parse(&target).expect("a list guard's target and an index parse");
                let watch = guard_key(&instance, &target, "name");
                self.ensure(&watch, &instance, &path, "name", false, true);
                if let Some(entry) = self.entries.get_mut(&watch) {
                    entry.dependents.insert(list_key.clone());
                    entry.watch = true;
                }
                if let Some(list) = self.entries.get_mut(&list_key) {
                    list.watches.push(watch);
                }
            }
            for _ in wanted..have {
                let Some(watch) = self
                    .entries
                    .get_mut(&list_key)
                    .and_then(|list| list.watches.pop())
                else {
                    break;
                };
                if let Some(entry) = self.entries.get_mut(&watch) {
                    entry.dependents.remove(&list_key);
                }
                self.maybe_drop(&watch);
            }
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
        let was_error = matches!(entry.cached, Some(Cached::Error(_)));
        let is_error = matches!(cached, Cached::Error(_));
        entry.cached = Some(cached);
        if entry.list {
            self.watch_dirty.insert(key.to_string());
        } else if was_error != is_error {
            // Its list guards may start or stop watching their items.
            self.watch_dirty.extend(entry.guards.iter().cloned());
        }
    }

    /// Re-resolves every name binding (and guard) of `instance`; an item
    /// name watch on an index path only when a list changed (`lists`).
    fn resolve_named(&mut self, instance: &str, lists: bool) {
        let keys: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, e)| e.instance == instance && re_resolves(e, lists))
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
        let guard = entry.guard;
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
                if guard {
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
mod tests;
