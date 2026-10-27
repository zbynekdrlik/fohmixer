//! The router's LUFS (#151, spec §5.2): the glue of the pure rule
//! `rules/lufs.rs`. It
//!
//! - hands the listener's readings ([`RouterMsg::Lufs`]) and its tick
//!   ([`RouterMsg::LufsTick`], every [`LUFS_TICK`]) to the rule;
//! - reads each Live's set name and its master track's name at the connect
//!   (`get_prop name` of `live_set` and `live_set master_track`: the main
//!   session's ruling 2), with the connect-time retries (`retries.rs`);
//! - names each loudness point's track: a frame strip by its anchor's name,
//!   a marker strip from the marker keeper's track lists (the items' `name`),
//!   the master from the read ([`track_name`]); a list's value names them
//!   again;
//! - follows each output point's fader (`mixer_device volume` `value` and
//!   `mute`) with its own subscriber [`LEVELS_CLIENT`] (in the `Status`
//!   `own` lists), for the rule's post-fader reading;
//! - runs the rule's confirming walks (`rules/lufs/confirm.rs`) through the
//!   instance's script, one task each ([`RouterMsg::LufsConfirmed`]);
//! - sends the points' items to every page (`Outbox::lufs`, paced with the
//!   meters) and at attach; records each `lufs` line with one log line.
//!
//! It never writes anything to Live.

use std::collections::BTreeMap;
use std::time::Duration;

use fohmixer_proto::client::{LufsStatus, hub_key};
use fohmixer_proto::layout::{Anchor, LufsPoint};
use fohmixer_proto::lufs::wire::Reading;
use serde_json::Value;
use tokio::sync::mpsc;

use super::markers::{RETURNS, TRACKS};
use super::retries::{CONNECT_RETRIES, RETRY_MS, transient};
use super::{Router, RouterMsg};
use crate::eq::walk::{answered, get};
use crate::live::client::LiveHandle;
use crate::live::subs::{Cached, ClientId};
use crate::outbox::Outbox;
use crate::rules::lufs::confirm::{self, Confirm, Step};
use crate::rules::lufs::{Act, Answer, Fader, Key, Lufs, LufsStrip};

/// The rule's own subscriber (the output points' faders; #151 Task 13 adds
/// the armed tracks' levels).
pub const LEVELS_CLIENT: ClientId = ClientId::MAX - 5;
/// The rule's tick.
pub const LUFS_TICK: Duration = Duration::from_millis(100);
/// A walk that ran out of reads.
pub const TOO_DEEP: &str = "the confirming walk ran out of reads";

/// The router's tick for LUFS: a [`RouterMsg::LufsTick`] every `period`;
/// it ends with the router's channel.
pub(crate) async fn lufs_ticks(router: mpsc::UnboundedSender<RouterMsg>, period: Duration) {
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if router.send(RouterMsg::LufsTick).is_err() {
            return;
        }
    }
}

/// Which part of an output point's fader a subscription follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Part {
    Volume,
    Mute,
}

/// One subscription an output point's fader needs.
struct Follow {
    instance: String,
    point: String,
    part: Part,
    target: String,
    prop: &'static str,
}

/// The name of item `index` of a list value as the script encodes it.
pub(super) fn item_name(list: &Value, index: u32) -> Option<String> {
    let index = usize::try_from(index).ok()?;
    list.as_array()?
        .get(index)?
        .get("name")?
        .as_str()
        .map(str::to_string)
}

/// A point's track name as the hub knows it: a frame strip's anchor's
/// name, a marker strip's item of the instance's list (`lists`, by hub
/// key: the marker keeper's), the master's name as read (`masters`); none
/// when unknown.
pub(super) fn track_name(
    lists: &BTreeMap<String, Value>,
    masters: &BTreeMap<String, String>,
    instance: &str,
    anchor: &Anchor,
) -> Option<String> {
    let list = |prop: &str, index: u32| {
        lists
            .get(&hub_key(instance, "live_set", prop, false))
            .and_then(|list| item_name(list, index))
    };
    match anchor {
        Anchor::Track { name } | Anchor::Return { name } => Some(name.clone()),
        Anchor::TrackAt { index } => list(TRACKS, *index),
        Anchor::ReturnAt { index } => list(RETURNS, *index),
        Anchor::Master => masters.get(instance).cloned(),
        Anchor::Song => None,
    }
}

/// A mute value as Live answers it (a bool; a number counts by not 0).
pub(super) fn mute_of(value: &Value) -> Option<bool> {
    value.as_bool().or_else(|| value.as_f64().map(|v| v != 0.0))
}

/// The names read's answer: the set's name and the master track's, or why
/// not.
pub(super) fn names_of(slots: &[Value]) -> Result<(String, String), String> {
    let name = |i: usize| {
        slots
            .get(i)
            .and_then(answered)
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    match (name(0), name(1)) {
        (Some(set), Some(master)) => Ok((set, master)),
        _ => Err(format!(
            "the set's name did not read: {}",
            Value::from(slots.to_vec())
        )),
    }
}

/// Whether a failed names read is tried again: Live did not answer or was
/// offline, and fewer than [`CONNECT_RETRIES`] failed in a row.
pub(super) fn read_again(why: &str, failures: u32) -> bool {
    transient(why) && failures <= CONNECT_RETRIES
}

/// The router's LUFS.
#[derive(Debug, Default)]
pub(super) struct LufsIo {
    pub rule: Lufs,
    /// The served layout's points.
    pub points: Vec<LufsPoint>,
    /// The output points' fader subscriptions: hub key → point and part.
    pub watched: BTreeMap<String, (String, String, Part)>,
    /// The output points' faders as heard.
    pub faders: BTreeMap<(String, String), Fader>,
    /// Each instance's master track's name, as read.
    pub masters: BTreeMap<String, String>,
    /// The names reads' counter, each instance's newest and its failures.
    pub reads: u64,
    pub reading: BTreeMap<String, u64>,
    pub failures: BTreeMap<String, u32>,
}

/// A confirming walk of `track` in one Live (bounded by its reads).
async fn confirm_walk(live: LiveHandle, key: &Key, track: &str) -> Answer {
    let (mut walk, mut commands) = Confirm::start(key.kind, &key.parents, track);
    for _ in 0..confirm::MAX_READS {
        let slots = match live.call(commands).await {
            Ok(slots) => slots,
            Err(e) => return Answer::Failed(e.to_string()),
        };
        match walk.answer(&slots) {
            Step::Read(next) => commands = next,
            Step::Done(answer) => return answer,
        }
    }
    Answer::Failed(TOO_DEEP.to_string())
}

impl Router {
    fn lufs_now(&self) -> u64 {
        self.now_ms() as u64
    }

    /// The points with their tracks named as the hub knows them now.
    fn lufs_strips(&self) -> Vec<LufsStrip> {
        self.lufs
            .points
            .iter()
            .map(|p| LufsStrip {
                instance: p.instance.clone(),
                target: p.target.clone(),
                track: track_name(
                    &self.reverb.lists,
                    &self.lufs.masters,
                    &p.instance,
                    &p.anchor,
                ),
                spec: p.spec,
                label: p.label.clone(),
            })
            .collect()
    }

    /// A new layout's points: their output points' faders followed, the
    /// rule told.
    pub(super) fn lufs_layout(&mut self, points: Vec<LufsPoint>) {
        self.lufs.points = points;
        let mut wanted: BTreeMap<String, Follow> = BTreeMap::new();
        for p in self.lufs.points.iter().filter(|p| p.spec.target.is_none()) {
            let volume = format!("{} mixer_device volume", p.target);
            for (part, target, prop) in [
                (Part::Volume, volume, "value"),
                (Part::Mute, p.target.clone(), "mute"),
            ] {
                let key = hub_key(&p.instance, &target, prop, false);
                wanted.insert(
                    key,
                    Follow {
                        instance: p.instance.clone(),
                        point: p.target.clone(),
                        part,
                        target,
                        prop,
                    },
                );
            }
        }
        let gone: Vec<String> = self
            .lufs
            .watched
            .keys()
            .filter(|k| !wanted.contains_key(*k))
            .cloned()
            .collect();
        for key in gone {
            self.lufs.watched.remove(&key);
            self.subs.unsubscribe(LEVELS_CLIENT, &key);
        }
        for (key, follow) in wanted {
            if self.lufs.watched.contains_key(&key) {
                continue;
            }
            match self.subs.subscribe(
                LEVELS_CLIENT,
                &follow.instance,
                &follow.target,
                follow.prop,
                false,
            ) {
                Ok(reply) => {
                    self.lufs
                        .watched
                        .insert(reply.key, (follow.instance, follow.point, follow.part));
                }
                Err((_, error)) => {
                    tracing::warn!(instance = %follow.instance, target = %follow.target, error = %error, "lufs: cannot follow an output point's fader");
                }
            }
        }
        let now = self.lufs_now();
        let strips = self.lufs_strips();
        let acts = self.lufs.rule.set_strips(strips, true, now);
        self.lufs_apply(acts);
        // The faders the table already holds (a page follows them too).
        let held: Vec<(String, Value)> = self
            .lufs
            .watched
            .keys()
            .filter_map(|key| match self.subs.cached(key) {
                Some(Cached::Value { value, .. }) => Some((key.clone(), value.clone())),
                _ => None,
            })
            .collect();
        for (key, value) in held {
            self.lufs_value(&key, Some(value));
        }
    }

    /// A track list's value: the points' tracks named again (only a change
    /// tells the rule).
    pub(super) fn lufs_lists(&mut self) {
        let strips = self.lufs_strips();
        if strips == self.lufs.rule.strips() {
            return;
        }
        self.lufs.rule.tracks_changed();
        let now = self.lufs_now();
        let acts = self.lufs.rule.set_strips(strips, false, now);
        self.lufs_apply(acts);
    }

    /// A value of an output point's fader subscription.
    pub(super) fn lufs_value(&mut self, sub: &str, value: Option<Value>) {
        let Some((instance, point, part)) = self.lufs.watched.get(sub).cloned() else {
            return;
        };
        let fader = self
            .lufs
            .faders
            .entry((instance.clone(), point.clone()))
            .or_default();
        match part {
            Part::Volume => fader.volume = value.as_ref().and_then(Value::as_f64),
            Part::Mute => fader.mute = value.as_ref().and_then(mute_of),
        }
        let fader = *fader;
        let now = self.lufs_now();
        let acts = self.lufs.rule.fader(&instance, &point, fader, now);
        self.lufs_apply(acts);
    }

    /// One reading the listener took.
    pub(super) fn lufs_reading(&mut self, reading: &Reading) {
        let now = self.lufs_now();
        let acts = self.lufs.rule.reading(reading, now);
        self.lufs_apply(acts);
    }

    /// The rule's tick.
    pub(super) fn lufs_tick(&mut self) {
        let now = self.lufs_now();
        let acts = self.lufs.rule.tick(now);
        self.lufs_apply(acts);
    }

    /// A walk's answer.
    pub(super) fn lufs_confirmed(&mut self, key: &Key, instance: &str, ask: u64, answer: Answer) {
        if let Answer::Failed(why) = &answer {
            tracing::warn!(instance = %instance, device = key.device, error = %why, "lufs: a confirming walk failed (asked again later)");
        }
        let now = self.lufs_now();
        let acts = self.lufs.rule.answered(key, instance, ask, answer, now);
        self.lufs_apply(acts);
    }

    /// An instance connected: the rule told, its names read.
    pub(super) fn lufs_connected(&mut self, instance: &str) {
        let now = self.lufs_now();
        let acts = self.lufs.rule.connected(instance, now);
        self.lufs_apply(acts);
        self.lufs.failures.insert(instance.to_string(), 0);
        self.lufs_read_names(instance);
    }

    /// Reads the set's and the master track's names of `instance`.
    fn lufs_read_names(&mut self, instance: &str) {
        let Some(live) = self.live.get(instance).cloned() else {
            return;
        };
        self.lufs.reads += 1;
        let read = self.lufs.reads;
        self.lufs.reading.insert(instance.to_string(), read);
        let tx = self.io.tx.clone();
        let instance = instance.to_string();
        tokio::spawn(async move {
            let commands = vec![
                get("live_set", "name"),
                get("live_set master_track", "name"),
            ];
            let outcome = match live.call(commands).await {
                Ok(slots) => names_of(&slots),
                Err(e) => Err(e.to_string()),
            };
            let _ = tx.send(RouterMsg::LufsNamed {
                instance,
                read,
                outcome,
            });
        });
    }

    /// The names read's answer (void unless the instance's newest).
    pub(super) fn lufs_named(
        &mut self,
        instance: &str,
        read: u64,
        outcome: Result<(String, String), String>,
    ) {
        if self.lufs.reading.get(instance) != Some(&read) {
            return;
        }
        match outcome {
            Ok((set, master)) => {
                tracing::info!(instance = %instance, "lufs: the set's name is read");
                self.lufs.masters.insert(instance.to_string(), master);
                let now = self.lufs_now();
                let acts = self.lufs.rule.named(instance, &set, now);
                self.lufs_apply(acts);
                self.lufs_lists();
            }
            Err(why) => {
                let failures = self.lufs.failures.entry(instance.to_string()).or_insert(0);
                *failures += 1;
                if !read_again(&why, *failures) {
                    tracing::warn!(instance = %instance, error = %why, "lufs: the set's name could not be read; no LUFS device matches in it until it reconnects");
                    return;
                }
                let tx = self.io.tx.clone();
                let instance = instance.to_string();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(RETRY_MS)).await;
                    let _ = tx.send(RouterMsg::LufsNamesRetry { instance, read });
                });
            }
        }
    }

    /// A failed names read is due again (void once another read started).
    pub(super) fn lufs_names_retry(&mut self, instance: &str, read: u64) {
        let online = self.states.get(instance).is_some_and(|s| s.online);
        if online && self.lufs.reading.get(instance) == Some(&read) {
            self.lufs_read_names(instance);
        }
    }

    /// An instance went offline: its matches and names dropped.
    pub(super) fn lufs_offline(&mut self, instance: &str) {
        self.lufs.reading.remove(instance);
        self.lufs.masters.remove(instance);
        self.lufs.faders.retain(|(i, _), _| i != instance);
        let now = self.lufs_now();
        let acts = self.lufs.rule.offline(instance, now);
        self.lufs_apply(acts);
    }

    /// A page's attach: every point's current item.
    pub(super) fn lufs_attach(&self, outbox: &Outbox) {
        let items = self.lufs.rule.items();
        if !items.is_empty() {
            outbox.lufs(items);
        }
    }

    /// `/api/status` `lufs`'s router part (the listener's is the hub's).
    pub(super) fn lufs_status(&self) -> LufsStatus {
        LufsStatus {
            listener: Default::default(),
            devices: self.lufs.rule.device_status(self.lufs_now()),
            strips: self.lufs.rule.strip_status(),
        }
    }

    /// Carries out the rule's acts.
    pub(super) fn lufs_apply(&mut self, acts: Vec<Act>) {
        let mut items = Vec::new();
        for act in acts {
            match act {
                Act::Confirm {
                    key,
                    instance,
                    track,
                    ask,
                } => self.lufs_confirm(key, instance, track, ask),
                Act::Send(item) => items.push(item),
                Act::Log(line) => {
                    if line.warn {
                        tracing::warn!(record = %line.fields, "lufs: {}", line.text);
                    } else {
                        tracing::info!(record = %line.fields, "lufs: {}", line.text);
                    }
                    self.io.events.record("lufs", line.fields);
                }
            }
        }
        if !items.is_empty() {
            for outbox in self.clients.values() {
                outbox.lufs(items.clone());
            }
        }
    }

    /// Starts a confirming walk; its answer comes back as
    /// [`RouterMsg::LufsConfirmed`].
    fn lufs_confirm(&mut self, key: Key, instance: String, track: String, ask: u64) {
        let Some(live) = self.live.get(&instance).cloned() else {
            let now = self.lufs_now();
            let answer = Answer::Failed(format!("unknown instance {instance:?}"));
            let acts = self.lufs.rule.answered(&key, &instance, ask, answer, now);
            self.lufs_apply(acts);
            return;
        };
        let tx = self.io.tx.clone();
        tokio::spawn(async move {
            let answer = confirm_walk(live, &key, &track).await;
            let _ = tx.send(RouterMsg::LufsConfirmed {
                key,
                instance,
                ask,
                answer,
            });
        });
    }
}

#[cfg(test)]
#[path = "lufs/tests.rs"]
mod tests;
