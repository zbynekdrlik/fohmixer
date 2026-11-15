//! The router's half of the scene automation (#207): the rules' document
//! (`scenes.json`) at every page's attach, the pages' rules, the file's
//! hand edits (polled with the layout), and a scene's moves: its faders
//! resolved in the served layout, the rule (`rules::scenes`) handed Live's
//! values as the hub knows them, each glide step a `set` of
//! [`SCENES_CLIENT`] through the setter (as REVERB's, #130), each step a
//! `scene_glide` record and a log line, a wake per step.
//!
//! The rule's own subscriber [`SCENES_CLIENT`] follows the volume and the
//! mute of every fader that has a rule, so Live's value now is at hand when
//! a scene comes (`Subs::live_value`); its subscriptions are the hub's own
//! (`/api/status` counts the pages').
//!
//! A hub with neither an automation token nor `[scenes]` (the scene read
//! from Companion) takes no scene: its pages are sent no `scenes` document
//! (they show no SCENES section) and a page's rule is refused.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use fohmixer_proto::client::{SceneSourceStatus, ScenesStatus, ServerMsg, set_key};
use fohmixer_proto::layout::Layout;
use fohmixer_proto::roles;
use fohmixer_proto::scenes::{self as model, SceneRule};
use fohmixer_proto::sectors::{self as sectors_model, StripRef};
use serde_json::{Value, json};

use super::reverb::{rearm, wake_in};
use super::writes::SetMsg;
use super::{Router, RouterMsg};
use crate::live::subs::ClientId;
use crate::outbox::Outbox;
use crate::rules::scenes::{Act, FaderTargets, SceneFader, Scenes, fader_targets, step_line};
use crate::scenes::{Polled, SceneStore};

/// The scene rule's subscriber and writer (the other rules count down from
/// `MAX`; `REC_CLIENT` is `MAX − 6`).
pub const SCENES_CLIENT: ClientId = ClientId::MAX - 7;
/// The peer its writes name in the event log.
pub const SCENES_PEER: &str = "rule:scenes";
/// The words a page's rule is refused with on a hub without the token.
pub const OFF: &str = "scenes are off: no automation token and no [scenes]";

/// The last scene that came: its name, when (UTC ms) and what it moved.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct LastScene {
    pub name: String,
    pub at_ms: f64,
    pub moved: u32,
}

/// The router's scenes.
pub(super) struct ScenesIo {
    pub store: SceneStore,
    /// The hub has an automation token or reads Companion: it takes scenes.
    pub on: bool,
    pub rule: Scenes,
    /// Each followed key (write key) → its hub key.
    pub watched: BTreeMap<String, String>,
    /// The rule's write counter (its own `seq`).
    pub seq: u64,
    /// The latest armed wake's number, and when it comes (router ms).
    pub generation: u64,
    pub armed: Option<f64>,
    pub last: Option<LastScene>,
    /// The ration of the "not learned" warn line (a trigger in a loop past
    /// `MAX_KNOWN` names never floods the log).
    pub full_lines: crate::ration::Ration,
}

impl ScenesIo {
    pub fn open(data_dir: &std::path::Path) -> Self {
        Self {
            store: SceneStore::open(data_dir),
            on: false,
            rule: Scenes::default(),
            watched: BTreeMap::new(),
            seq: 0,
            generation: 0,
            armed: None,
            last: None,
            full_lines: crate::ration::Ration::default(),
        }
    }
}

/// A `scene_rule` record's fields: `what` (`set`, `file`), the page and
/// its peer (none for a hand edit), then `extra`'s.
pub(super) fn rule_fields(
    what: &str,
    client: Option<ClientId>,
    peer: Option<&str>,
    extra: Value,
) -> Value {
    let mut fields = json!({"what": what, "client": client, "peer": peer});
    if let (Some(map), Value::Object(more)) = (fields.as_object_mut(), extra) {
        map.extend(more);
    }
    fields
}

/// The info line of a page's rule.
pub(super) fn rule_line(strip: &StripRef, scene: &str, rule: Option<&SceneRule>) -> String {
    let what = match rule {
        None => "removed".to_string(),
        Some(r) => {
            let db = r.db.map_or("-".to_string(), |db| format!("{db:.1} dB"));
            let mute = r.mute.map_or("-", |m| m.word());
            format!("{db}, {mute}, {} ms", r.time_ms)
        }
    };
    format!(
        "a scene rule changed: {}|{} in scene {scene:?}: {what}",
        strip.instance, strip.label
    )
}

/// The info line of a scene that came.
pub(super) fn scene_line(scene: &str, source: &str, peer: &str, moved: u32) -> String {
    format!("scene {scene:?} came from {peer} ({source}): {moved} faders move")
}

/// The warn line of a scene name past what the document keeps.
pub(super) fn full_line(scene: &str) -> String {
    format!(
        "scene {scene:?} runs, but scenes.json keeps {} names already: it is not learned",
        model::MAX_KNOWN
    )
}

/// The most "not learned" warn lines per `ration::GAP`.
pub(super) const FULL_LINES: u32 = 4;

/// Whether `instance` is online as the router knows it.
pub(super) fn online(states: &BTreeMap<String, super::InstanceState>, instance: &str) -> bool {
    states.get(instance).is_some_and(|s| s.online)
}

/// The warn line of a write of `scenes.json` that failed.
pub(super) fn write_problem(error: &std::io::Error) -> String {
    format!("cannot write the scene rules (scenes.json): {error}")
}

/// Whether a Live `mute` value is on (a bool, or a number other than 0).
pub(super) fn mute_on(value: &Value) -> Option<bool> {
    value.as_bool().or_else(|| value.as_f64().map(|n| n != 0.0))
}

/// The keys a fader's targets are followed by: its volume's `value` and its
/// own track's `mute`.
pub(super) fn followed(targets: &FaderTargets) -> Vec<(String, String, &'static str)> {
    let volume = targets
        .volume
        .iter()
        .map(|(i, t)| (i.clone(), t.clone(), "value"));
    let mute = targets
        .mutes
        .first()
        .into_iter()
        .map(|(i, t)| (i.clone(), t.clone(), "mute"));
    volume.chain(mute).collect()
}

impl Router {
    /// The hub takes scenes (it has an automation token or `[scenes]`).
    pub fn with_scenes(mut self, on: bool) -> Self {
        self.scenes.on = on;
        self
    }

    /// The served layout.
    fn scenes_layout(&self) -> Option<Arc<Layout>> {
        self.layout_store
            .as_ref()
            .and_then(|store| store.current().1)
    }

    /// Every fader with a rule that the served layout carries, with its
    /// targets.
    fn scenes_targets(&self) -> Vec<(StripRef, FaderTargets)> {
        let Some(layout) = self.scenes_layout() else {
            return Vec::new();
        };
        let pool = sectors_model::pool(&layout);
        self.scenes
            .store
            .doc()
            .strips()
            .into_iter()
            .filter_map(|strip| {
                let pooled = pool.iter().find(|p| p.id == strip)?;
                fader_targets(pooled.control).map(|t| (strip, t))
            })
            .collect()
    }

    /// Follows the volume and mute of every fader with a rule (a changed
    /// document or layout), and lets go of the rest.
    pub(super) fn scenes_watch(&mut self) {
        if !self.scenes.on {
            return;
        }
        let wanted: BTreeMap<String, (String, String, &'static str)> = self
            .scenes_targets()
            .iter()
            .flat_map(|(_, t)| followed(t))
            .map(|(i, t, p)| (set_key(&i, &t, p), (i, t, p)))
            .collect();
        let gone: Vec<String> = self
            .scenes
            .watched
            .keys()
            .filter(|k| !wanted.contains_key(*k))
            .cloned()
            .collect();
        for key in gone {
            if let Some(sub) = self.scenes.watched.remove(&key) {
                self.subs.unsubscribe(SCENES_CLIENT, &sub);
            }
        }
        for (key, (instance, target, prop)) in wanted {
            if self.scenes.watched.contains_key(&key) {
                continue;
            }
            match self
                .subs
                .subscribe(SCENES_CLIENT, &instance, &target, prop, false)
            {
                Ok(reply) => {
                    self.scenes.watched.insert(key, reply.key);
                }
                Err((_, error)) => {
                    tracing::warn!(instance = %instance, target = %target, error = %error, "the scene rules cannot follow a fader");
                }
            }
        }
    }

    /// The document as the pages get it.
    fn scenes_message(&self) -> ServerMsg {
        ServerMsg::Scenes {
            doc: self.scenes.store.doc().clone(),
        }
    }

    /// The last scene as the pages get it (`live`: it came just now).
    fn scene_message(last: &LastScene, live: bool) -> ServerMsg {
        ServerMsg::Scene {
            name: last.name.clone(),
            at_ms: last.at_ms,
            moved: last.moved,
            live,
        }
    }

    /// A page attached: the document and the last scene (a hub that takes
    /// scenes only).
    pub(super) fn scenes_attach(&self, outbox: &Outbox) {
        if !self.scenes.on {
            return;
        }
        outbox.reply(self.scenes_message());
        if let Some(last) = &self.scenes.last {
            outbox.reply(Self::scene_message(last, false));
        }
    }

    /// `msg` to every page.
    fn scenes_tell(&self, msg: &ServerMsg) {
        for outbox in self.clients.values() {
            outbox.reply(msg.clone());
        }
    }

    /// An answer to the asker alone.
    fn scenes_answer(&self, client: ClientId, msg: ServerMsg) {
        if let Some(outbox) = self.clients.get(&client) {
            outbox.reply(msg);
        }
    }

    /// The asker's `error` with `message`.
    fn scenes_refuse(&self, client: ClientId, id: String, message: String) {
        self.scenes_answer(
            client,
            ServerMsg::Error {
                id: Some(id),
                message,
            },
        );
    }

    /// The asker's `result`.
    fn scenes_taken(&self, client: ClientId, id: String) {
        self.scenes_answer(
            client,
            ServerMsg::Result {
                id,
                data: Vec::new(),
            },
        );
    }

    /// A page's rule for `strip` and `scene` (none: removed).
    pub(super) fn scenes_set(
        &mut self,
        client: ClientId,
        id: String,
        strip: &StripRef,
        scene: &str,
        rule: Option<SceneRule>,
    ) {
        if !self.scenes.on {
            self.scenes_refuse(client, id, OFF.to_string());
            return;
        }
        if !roles::reaches_strip(self.role_of(client), strip) {
            tracing::info!(client, "a scene rule is refused: role");
            self.scenes_refuse(client, id, crate::ws::role::REFUSED.to_string());
            return;
        }
        let served = self.scenes_layout();
        let pool = served
            .as_deref()
            .map(sectors_model::pool)
            .unwrap_or_default();
        match model::set_rule(self.scenes.store.doc(), strip, scene, rule, &pool) {
            Err(refusal) => self.scenes_refuse(client, id, refusal.text().to_string()),
            Ok(None) => self.scenes_taken(client, id),
            Ok(Some(next)) => {
                let kept = next.rule(strip, scene);
                match self.scenes.store.commit(next) {
                    Ok(()) => {
                        let peer = self.peers.get(&client).map(String::as_str);
                        self.io.events.record(
                            "scene_rule",
                            rule_fields(
                                "set",
                                Some(client),
                                peer,
                                json!({"strip": strip, "scene": scene, "rule": kept}),
                            ),
                        );
                        tracing::info!(client, "{}", rule_line(strip, scene, kept.as_ref()));
                        self.scenes_taken(client, id);
                        self.scenes_tell(&self.scenes_message());
                        self.scenes_watch();
                    }
                    Err(e) => {
                        tracing::warn!("{}", write_problem(&e));
                        self.scenes_refuse(client, id, e.to_string());
                    }
                }
            }
        }
    }

    /// The file read again (the layout poll's tick), as the colours'.
    pub(super) fn scenes_poll(&mut self) {
        match self.scenes.store.poll() {
            Some(Polled::Taken) => {
                self.io
                    .events
                    .record("scene_rule", rule_fields("file", None, None, json!({})));
                tracing::info!("scenes.json was edited by hand: its rules are served");
                // The last scene is one the document knows: a hand edit that
                // drops its name forgets it (a later page shows no line).
                if self
                    .scenes
                    .last
                    .as_ref()
                    .is_some_and(|l| !self.scenes.store.doc().knows(&l.name))
                {
                    self.scenes.last = None;
                }
                if self.scenes.on {
                    self.scenes_tell(&self.scenes_message());
                }
                self.scenes_watch();
            }
            Some(Polled::Refused(why)) => {
                tracing::warn!("scenes.json is refused, the last good document stays served: {why}")
            }
            Some(Polled::Unreadable(why)) => tracing::warn!(
                "cannot read scenes.json, the document stays served and is read again at the next poll: {why}"
            ),
            Some(Polled::Unwritable(why)) => tracing::warn!(
                "scenes.json was removed and cannot be written back, the document stays served and is written again at the next poll: {why}"
            ),
            Some(Polled::Restored) => {
                tracing::warn!(
                    "scenes.json was removed; it is written back from the document served"
                );
                self.io.events.record(
                    "scene_rule",
                    rule_fields("file", None, None, json!({"why": "deleted"})),
                );
            }
            None => {}
        }
    }

    /// A scene came (`POST /api/scene`, the name already checked): its name
    /// learned, its faders moved, the pages told. How many faders it moves.
    pub(super) fn scene_came(&mut self, name: &str, source: &str, peer: &str) -> u32 {
        if let Some(next) = model::heard(self.scenes.store.doc(), name) {
            match self.scenes.store.commit(next) {
                Ok(()) => self.scenes_tell(&self.scenes_message()),
                Err(e) => tracing::warn!("{}", write_problem(&e)),
            }
        } else if model::is_full(self.scenes.store.doc(), name)
            && let Some(skipped) = self
                .scenes
                .full_lines
                .take(std::time::Instant::now(), FULL_LINES)
        {
            tracing::warn!(skipped, "{}", full_line(name));
        }
        let targets: BTreeMap<StripRef, FaderTargets> = self.scenes_targets().into_iter().collect();
        let faders: Vec<SceneFader> = self
            .scenes
            .store
            .doc()
            .rules_of(name)
            .into_iter()
            // A Live set that is offline is left out: nothing reaches it,
            // so nothing of it counts as moved.
            .filter(|(strip, _)| online(&self.states, &strip.instance))
            .filter_map(|(strip, rule)| {
                let targets = targets.get(&strip)?.clone();
                let volume_now = targets.volume.as_ref().and_then(|(i, t)| {
                    self.subs
                        .live_value(&set_key(i, t, "value"))
                        .and_then(Value::as_f64)
                });
                let mute_now = targets.mutes.first().and_then(|(i, t)| {
                    self.subs
                        .live_value(&set_key(i, t, "mute"))
                        .and_then(mute_on)
                });
                Some(SceneFader {
                    strip,
                    rule,
                    targets,
                    volume_now,
                    mute_now,
                })
            })
            .collect();
        let now = self.now_ms();
        let (acts, moved) = self.scenes.rule.scene(name, faders, now);
        self.io.events.record(
            "scene",
            json!({"scene": name, "source": source, "peer": peer, "moved": moved}),
        );
        tracing::info!("{}", scene_line(name, source, peer, moved));
        self.scenes_apply(acts);
        let last = LastScene {
            name: name.to_string(),
            at_ms: crate::live::wall_ms().unwrap_or(0.0),
            moved,
        };
        self.scenes_tell(&Self::scene_message(&last, true));
        self.scenes.last = Some(last);
        moved
    }

    /// A page's write of `key` (`take_set`; never the rule's own): its
    /// glide stops.
    pub(super) fn scenes_hand(&mut self, key: &str) {
        let acts = self.scenes.rule.hand(key);
        self.scenes_apply(acts);
    }

    /// An instance went offline: its glides end.
    pub(super) fn scenes_offline(&mut self, instance: &str) {
        let acts = self.scenes.rule.offline(instance);
        self.scenes_apply(acts);
    }

    /// The armed wake came (a stale one does nothing).
    pub(super) fn scenes_wake(&mut self, generation: u64) {
        if generation != self.scenes.generation {
            return;
        }
        self.scenes.armed = None;
        let now = self.now_ms();
        let acts = self.scenes.rule.tick(now);
        self.scenes_apply(acts);
    }

    /// Carries out the rule's acts, then arms the next wake.
    fn scenes_apply(&mut self, acts: Vec<Act>) {
        let mut instances: BTreeSet<String> = BTreeSet::new();
        for act in acts {
            match act {
                Act::Write {
                    instance,
                    target,
                    prop,
                    value,
                    ..
                } => {
                    self.scenes.seq += 1;
                    let t = crate::live::wall_ms().unwrap_or(0.0);
                    self.take_set(SetMsg {
                        client: SCENES_CLIENT,
                        instance: instance.clone(),
                        target,
                        prop: prop.to_string(),
                        value,
                        seq: self.scenes.seq,
                        t,
                        is_final: true,
                        hub_ms: t,
                        offset_ms: Some(0.0),
                    });
                    instances.insert(instance);
                }
                Act::Step(step) => {
                    tracing::info!("{}", step_line(&step));
                    let fields = serde_json::to_value(&step).unwrap_or(Value::Null);
                    self.io.events.record("scene_glide", fields);
                }
            }
        }
        for instance in &instances {
            self.write_next(instance);
        }
        self.scenes_arm();
    }

    /// Arms one wake for the next glide step.
    fn scenes_arm(&mut self) {
        let now = self.now_ms();
        let Some(at) = self.scenes.rule.next_tick(now) else {
            return;
        };
        if !rearm(self.scenes.armed, at) {
            return;
        }
        self.scenes.generation += 1;
        self.scenes.armed = Some(at);
        let generation = self.scenes.generation;
        let tx = self.io.tx.clone();
        let wait = wake_in(at, now);
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            let _ = tx.send(RouterMsg::ScenesWake { generation });
        });
    }

    /// `/api/status` `scenes`.
    pub(super) fn scenes_status(&self) -> ScenesStatus {
        let served = self.scenes_layout();
        let pool = served
            .as_deref()
            .map(sectors_model::pool)
            .unwrap_or_default();
        let doc = self.scenes.store.doc();
        ScenesStatus {
            on: self.scenes.on,
            strips: doc.count(),
            unknown: model::unknown(doc, &pool),
            error: self.scenes.store.error().map(str::to_string),
            known: doc.known.len(),
            last: self.scenes.last.as_ref().map(|l| l.name.clone()),
            glides: self.scenes.rule.glides(),
            writes: self.scenes.rule.writes(),
            // The poll's own state (`scene_source`), filled by `Hub::status`.
            companion: SceneSourceStatus::default(),
        }
    }
}

#[cfg(test)]
#[path = "scenes/tests.rs"]
mod tests;
