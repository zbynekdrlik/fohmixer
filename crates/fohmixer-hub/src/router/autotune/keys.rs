//! The router's half of the autotune keys (#140 PR 2a; spec §5, §6), read
//! only: it follows each found target's key device (its Key and Scale
//! `value` at the paths the find gave, AUTOTUNE's own subscriptions) and
//! each Live set's playhead (`live_set` `current_song_time` and `tempo`, a
//! set with targets only), judges the key in force on the set's newest song
//! map (`rules::autotune::keys::in_force`, a read in flight keeping the old
//! map until it answers) and records each change (`key`), each key device
//! found (`key_device`) and each one missing (`key_missing`, when news).
//! It also refuses a page's AUTOTUNE AUT on and tap until #140 PR 3
//! (`refused`): nothing here writes to Live. The writes are PR 2b's.

use std::collections::BTreeSet;

use fohmixer_proto::client::{KeyTargetStatus, ServerMsg};
use fohmixer_proto::songmap::{Key, SongMap};
use serde_json::Value;

use super::super::{Router, transport_heard};
use super::{AUTOTUNE_CLIENT, AutotuneIo, missing_news};
use crate::live::subs::{Cached, ClientId};
use crate::rules::autotune::find::KeyFound;
use crate::rules::autotune::keys::{KeyFollow, in_force, shows};
use crate::rules::autotune::{REFUSED_WHY, Step};

/// A Live set's playhead in beats.
pub const CURRENT_SONG_TIME: &str = "current_song_time";
/// A Live set's tempo in bpm.
pub const TEMPO: &str = "tempo";
/// A parameter's value.
pub const VALUE: &str = "value";

/// One Live set's playhead as AUTOTUNE follows it: the position, the
/// transport and the tempo as last heard (none before, and after a
/// disconnect), its subscriptions, and its key in force.
#[derive(Debug, Clone, Default, PartialEq)]
pub(in crate::router) struct Position {
    pub time: Option<f64>,
    pub playing: Option<bool>,
    pub tempo: Option<f64>,
    pub time_sub: Option<String>,
    pub tempo_sub: Option<String>,
    pub follow: KeyFollow,
}

/// Which value of a key device a subscription follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KeyPart {
    Key,
    Scale,
}

/// Which value of a Live set's playhead a subscription follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PositionPart {
    Time,
    Tempo,
}

/// What one of AUTOTUNE's subscriptions follows: a target's macro or its
/// key device's value (by the target's key), or a set's playhead (by the
/// set).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Route {
    Macro(String),
    Key(String, KeyPart),
    Position(String, PositionPart),
}

impl AutotuneIo {
    /// What the subscription `sub` (a hub key) follows, if it is one of
    /// AUTOTUNE's.
    pub(super) fn route(&self, sub: &str) -> Option<Route> {
        let is = |held: &Option<String>| held.as_deref() == Some(sub);
        let target = self.targets.iter().find_map(|(key, t)| {
            if is(&t.sub) {
                Some(Route::Macro(key.clone()))
            } else if is(&t.key_sub) {
                Some(Route::Key(key.clone(), KeyPart::Key))
            } else if is(&t.scale_sub) {
                Some(Route::Key(key.clone(), KeyPart::Scale))
            } else {
                None
            }
        });
        target.or_else(|| {
            self.positions.iter().find_map(|(instance, p)| {
                if is(&p.time_sub) {
                    Some(Route::Position(instance.clone(), PositionPart::Time))
                } else if is(&p.tempo_sub) {
                    Some(Route::Position(instance.clone(), PositionPart::Tempo))
                } else {
                    None
                }
            })
        })
    }
}

/// A number a delivery brings (`heard`, as `transport_heard` reads it: the
/// error first): its value (none when it is no number), none after an
/// error, the one held before (`was`) when it brought neither.
pub(super) fn heard_number(heard: Option<&Result<Value, String>>, was: Option<f64>) -> Option<f64> {
    match heard {
        Some(Ok(value)) => value.as_f64(),
        Some(Err(_)) => None,
        None => was,
    }
}

/// The song and key in force at a set's position on its song map: none
/// without a map or a position heard; one tick ahead only while it is
/// known to play.
pub(super) fn key_now(map: Option<&SongMap>, position: &Position) -> Option<(u16, Key)> {
    let (Some(map), Some(time)) = (map, position.time) else {
        return None;
    };
    in_force(map, time, position.playing == Some(true), position.tempo)
}

/// A target's key device in `/api/status`: the device, its path, its two
/// values once both were heard and the key they show; or why there is none.
pub(super) fn key_status(
    key: &Result<KeyFound, String>,
    values: (Option<f64>, Option<f64>),
) -> KeyTargetStatus {
    match key {
        Ok(found) => {
            let both = values.0.zip(values.1);
            KeyTargetStatus {
                device: Some(found.params.device.name().to_string()),
                path: Some(found.device.clone()),
                values: both.map(|(key, scale)| [key, scale]),
                shows: both
                    .and_then(|(key, scale)| shows(key, scale))
                    .map(|key| key.name()),
                problem: None,
            }
        }
        Err(problem) => KeyTargetStatus {
            device: None,
            path: None,
            values: None,
            shows: None,
            problem: Some(problem.clone()),
        },
    }
}

impl Router {
    /// A layout's Live sets with targets (`sets`): each one's playhead is
    /// followed, a set no longer among them let go.
    pub(super) fn autotune_positions(&mut self, sets: &BTreeSet<String>) {
        let gone: Vec<String> = self
            .autotune
            .positions
            .keys()
            .filter(|set| !sets.contains(*set))
            .cloned()
            .collect();
        for set in gone {
            self.autotune_position_let_go(&set);
        }
        for set in sets {
            self.autotune_position_follow(set);
        }
    }

    /// Follows `instance`'s playhead and tempo, unless it does already; its
    /// transport as last heard, and the key in force at once when the table
    /// already holds the position (a page follows it).
    pub(super) fn autotune_position_follow(&mut self, instance: &str) {
        if self.autotune.positions.contains_key(instance) {
            return;
        }
        let (time_sub, time) = self.autotune_number_sub(instance, "live_set", CURRENT_SONG_TIME);
        let (tempo_sub, tempo) = self.autotune_number_sub(instance, "live_set", TEMPO);
        let position = Position {
            time,
            playing: self.transport_now(instance),
            tempo,
            time_sub,
            tempo_sub,
            follow: KeyFollow::new(),
        };
        self.autotune
            .positions
            .insert(instance.to_string(), position);
        self.autotune_position(instance);
    }

    /// Lets go of a set's playhead (a layout without its targets).
    fn autotune_position_let_go(&mut self, instance: &str) {
        let Some(position) = self.autotune.positions.remove(instance) else {
            return;
        };
        for sub in [position.time_sub, position.tempo_sub]
            .into_iter()
            .flatten()
        {
            self.subs.unsubscribe(AUTOTUNE_CLIENT, &sub);
        }
    }

    /// Subscribes `target`'s `prop` for AUTOTUNE: its hub key and the
    /// table's value as a number when it holds one.
    fn autotune_number_sub(
        &mut self,
        instance: &str,
        target: &str,
        prop: &str,
    ) -> (Option<String>, Option<f64>) {
        match self
            .subs
            .subscribe(AUTOTUNE_CLIENT, instance, target, prop, false)
        {
            Ok(reply) => {
                let value = match &reply.cached {
                    Some(Cached::Value { value, .. }) => value.as_f64(),
                    _ => None,
                };
                (Some(reply.key), value)
            }
            Err((_, error)) => {
                tracing::warn!(instance = %instance, target = %target, prop = %prop, error = %error, "AUTOTUNE cannot follow a value");
                (None, None)
            }
        }
    }

    /// A delivery of a set's playhead or tempo: kept (none after an error),
    /// and the key in force judged again.
    pub(super) fn autotune_position_value(
        &mut self,
        instance: &str,
        part: PositionPart,
        value: Option<Value>,
        error: Option<String>,
    ) {
        let heard = transport_heard(value, error);
        if let Some(Err(error)) = &heard {
            tracing::warn!(instance = %instance, error = ?error, "AUTOTUNE cannot follow the song position");
        }
        let Some(position) = self.autotune.positions.get_mut(instance) else {
            return;
        };
        match part {
            PositionPart::Time => position.time = heard_number(heard.as_ref(), position.time),
            PositionPart::Tempo => position.tempo = heard_number(heard.as_ref(), position.tempo),
        }
        self.autotune_position(instance);
    }

    /// A set's transport as AUTOTUNE hears it (none: the listener failed):
    /// the key in force judged again (the lookahead is a playing set's).
    pub(super) fn autotune_position_playing(&mut self, instance: &str, playing: Option<bool>) {
        let Some(position) = self.autotune.positions.get_mut(instance) else {
            return;
        };
        position.playing = playing;
        self.autotune_position(instance);
    }

    /// The key in force of `instance` at its position on its newest song
    /// map (stale or not: a read in flight keeps the old map until it
    /// answers, so the key never flickers to none): a change is one `key`
    /// record.
    pub(super) fn autotune_position(&mut self, instance: &str) {
        let map = self.songmap.maps.get(instance).map(|read| &read.map);
        let Some(position) = self.autotune.positions.get_mut(instance) else {
            return;
        };
        let now = key_now(map, position);
        let Some(why) = position.follow.on_position(now) else {
            return;
        };
        self.autotune_step(
            Step::Key {
                instance: instance.to_string(),
                song: now.map(|(song, _)| song),
                key: now.map(|(_, key)| key.name()),
                why,
            },
            None,
        );
    }

    /// A new song map of `instance` was stored (`songmap_read`): the key in
    /// force judged on it.
    pub(in crate::router) fn autotune_map_read(&mut self, instance: &str) {
        self.autotune_position(instance);
    }

    /// `instance` went offline: its playhead's values end and its key in
    /// force is forgotten (the first position after the connect is a song
    /// start); the subscriptions stay (the table resolves them again).
    pub(super) fn autotune_position_offline(&mut self, instance: &str) {
        if let Some(position) = self.autotune.positions.get_mut(instance) {
            position.time = None;
            position.playing = None;
            position.tempo = None;
            position.follow.reset();
        }
    }

    /// A target's rack found (`autotune_target_found`): its key device's Key
    /// and Scale followed afresh when it is another than before (another
    /// path or `$ref`, or none followed: a disconnect, a lost value),
    /// recorded `key_device`; or why there is none, recorded `key_missing`
    /// when that is news.
    pub(super) fn autotune_key_found(&mut self, key: &str) {
        let Some(target) = self.autotune.targets.get(key) else {
            return;
        };
        let Some(found) = target.found.as_ref() else {
            return;
        };
        let now = found.key.clone().map_err(|missing| missing.to_string());
        let before = target.key.clone();
        let (instance, track) = (target.instance.clone(), target.track.clone());
        match now {
            Ok(device) => {
                if before.as_ref() == Some(&Ok(device.clone())) {
                    return;
                }
                self.autotune_key_unsub(key);
                let (key_sub, key_value) =
                    self.autotune_number_sub(&instance, &device.key.path, VALUE);
                let (scale_sub, scale_value) =
                    self.autotune_number_sub(&instance, &device.scale.path, VALUE);
                let step = Step::KeyDevice {
                    instance,
                    target: track,
                    device: device.params.device,
                    path: device.device.clone(),
                };
                if let Some(target) = self.autotune.targets.get_mut(key) {
                    target.key = Some(Ok(device));
                    target.key_sub = key_sub;
                    target.scale_sub = scale_sub;
                    target.values = (key_value, scale_value);
                }
                self.autotune_step(step, None);
            }
            Err(problem) => {
                self.autotune_key_unsub(key);
                let was = before.as_ref().and_then(|k| k.as_ref().err());
                let news = missing_news(was.map(String::as_str), &problem);
                if let Some(target) = self.autotune.targets.get_mut(key) {
                    target.key = Some(Err(problem.clone()));
                }
                if news {
                    self.autotune_step(
                        Step::KeyMissing {
                            instance,
                            target: track,
                            problem,
                        },
                        None,
                    );
                }
            }
        }
    }

    /// Ends a target's key device subscriptions (their values with them).
    pub(super) fn autotune_key_unsub(&mut self, key: &str) {
        let Some(target) = self.autotune.targets.get_mut(key) else {
            return;
        };
        target.values = (None, None);
        let subs = [target.key_sub.take(), target.scale_sub.take()];
        for sub in subs.into_iter().flatten() {
            self.subs.unsubscribe(AUTOTUNE_CLIENT, &sub);
        }
    }

    /// A delivery of a target's Key or Scale value: kept; an error (its path
    /// no longer resolves) drops it and the device, and walks the target
    /// again, so its answer follows the device afresh.
    pub(super) fn autotune_key_value(
        &mut self,
        key: &str,
        part: KeyPart,
        value: Option<Value>,
        error: Option<String>,
    ) {
        let heard = transport_heard(value, error);
        let Some(target) = self.autotune.targets.get_mut(key) else {
            return;
        };
        match part {
            KeyPart::Key => target.values.0 = heard_number(heard.as_ref(), target.values.0),
            KeyPart::Scale => target.values.1 = heard_number(heard.as_ref(), target.values.1),
        }
        if let Some(Err(error)) = heard {
            tracing::warn!(key = %key, error = ?error, "AUTOTUNE lost a key device's value: it looks again");
            target.key = None;
            self.autotune_walk(key);
        }
    }

    /// A page's AUTOTUNE AUT on or tap (`what`), refused until #140 PR 3:
    /// recorded, and that page told the state again (its button shows what
    /// the hub holds); nothing is written and nothing saved.
    pub(super) fn autotune_refused(&self, client: ClientId, what: &str) {
        self.autotune_step(
            Step::Refused {
                what: what.to_string(),
                why: REFUSED_WHY.to_string(),
            },
            Some(client),
        );
        if let Some(outbox) = self.clients.get(&client) {
            outbox.reply(ServerMsg::Autotune(self.autotune.state()));
        }
    }
}

#[cfg(test)]
#[path = "keys_tests.rs"]
mod tests;
