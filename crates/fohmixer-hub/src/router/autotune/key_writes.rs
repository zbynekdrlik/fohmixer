//! The router's writes of the autotune keys (#140 PR 2b; spec §5, §6 and
//! §7's hold): the key in force goes to every found key device of a Live
//! set (the Auto-Tune Artist's Key and Scale, AudioGridder's slots 0 and 1)
//! through their parameters' `$ref`s with class `DeviceParameter`, never
//! by an index path, and to Live's bar (`live_set` `root_note` and
//! `scale_name`), in one `live.call` batch; every write is read back with
//! a second call (a rewrite of an unchanged value fires no listener).
//!
//! - **When:** a song start and a key change write every target of the
//!   set and the bar (`song`, `change`); a target found anew, or owed the
//!   key since a disconnect, a held write or another device, is written
//!   alone (`found`) once its Key and Scale values are heard, so the
//!   quarter-step rule (`keys::write_value`) sees what Live holds; a write
//!   not confirmed is written once more at once (`retry`, once per key).
//!   A keyless song and out of every song write nothing.
//! - **Held** as the macros are (`may_write`, `autotune_hold`): while the
//!   set's markers are not settled each target and the bar owe the key,
//!   and the hold's release walks the targets again, whose answers write it
//!   (`found`). A write needs the connect's song map (PR 2a review M1).
//! - **Read back:** a refused slot (a Max map holds the parameter: Live's
//!   `Value cannot be set, the parameter is disabled`) is `refused`, a
//!   stale `$ref` also walks its target again; the others are read through
//!   their `$ref`s and judged (`keys::judge`). Each target's outcome is one
//!   `readback` record, Live's bar one `bar` record (it gates nothing), a
//!   key change written while the set plays one `late` record.
//! - **Another writer:** a value heard between writes that no longer shows
//!   the key the hub confirmed there is one `another_writer` record per key
//!   and is never written against; never while the set is held or before
//!   the target's walk confirmed its device again (PR 2a review M3).
//!
//! Nothing here writes a macro or a chain selector.

use std::time::{Duration, Instant};

use fohmixer_proto::songmap::Key;
use serde_json::{Value, json};

use super::super::{Router, RouterMsg};
use super::keys::LateNote;
use super::{PARAMETER_CLASS, may_write};
use crate::eq::close::STALE_REF;
use crate::eq::walk::{answered, get, refused};
use crate::rules::autotune::Step;
use crate::rules::autotune::find::KeyFound;
use crate::rules::autotune::keys::{
    KeyState, KeyTarget, KeyWhy, Readback, bar_shows, bar_state, change_time, judge, key_values,
    late_ms, lookahead_beats, shows,
};

/// Live's bar's key: an index on Live's scale, C = 0 to B = 11 (spec A.5).
pub const BAR_ROOT: &str = "root_note";
/// Live's bar's scale: "Major" or "Minor".
pub const BAR_SCALE: &str = "scale_name";

/// A parameter's `set_prop value` through its `$ref`.
fn param_set(id: &str, value: f64) -> Value {
    json!({
        "target": {"$ref": id, "class": PARAMETER_CLASS},
        "name": "set_prop",
        "args": {"prop": "value", "value": value},
    })
}

/// A parameter's `get_prop value` through its `$ref`.
fn param_get(id: &str) -> Value {
    json!({
        "target": {"$ref": id, "class": PARAMETER_CLASS},
        "name": "get_prop",
        "args": {"prop": "value"},
    })
}

/// One write's call: each key device's Key and Scale at its values, then,
/// with `bar`, Live's bar's `root_note` and `scale_name`: one batch,
/// applied in one Live tick.
pub(super) fn key_write_commands(
    devices: &[(&KeyFound, (f64, f64))],
    bar: Option<Key>,
) -> Vec<Value> {
    let mut commands: Vec<Value> = devices
        .iter()
        .flat_map(|(found, (key, scale))| {
            [
                param_set(&found.key.id, *key),
                param_set(&found.scale.id, *scale),
            ]
        })
        .collect();
    if let Some(bar) = bar {
        commands.push(json!({"target": "live_set", "name": "set_prop",
                             "args": {"prop": BAR_ROOT, "value": bar.root}}));
        commands.push(json!({"target": "live_set", "name": "set_prop",
                             "args": {"prop": BAR_SCALE, "value": bar.live_scale_name()}}));
    }
    commands
}

/// The read-back's call: each key device's Key and Scale value, then, with
/// `bar`, Live's bar.
pub(super) fn key_read_commands(devices: &[&KeyFound], bar: bool) -> Vec<Value> {
    let mut commands: Vec<Value> = devices
        .iter()
        .flat_map(|found| [param_get(&found.key.id), param_get(&found.scale.id)])
        .collect();
    if bar {
        commands.push(get("live_set", BAR_ROOT));
        commands.push(get("live_set", BAR_SCALE));
    }
    commands
}

/// Whether a slot's error is the script's stale `$ref` (the object went).
fn stale(slot: Option<&Value>) -> bool {
    slot.and_then(|s| s.get("errorType"))
        .and_then(Value::as_str)
        == Some(STALE_REF)
}

/// How one key device's write answered (its two slots from `at`): none
/// when both went through; else Live's words (the call's error when it
/// got no answer at all) and whether its `$ref` is stale.
pub(super) fn write_problem(
    outcome: &Result<Vec<Value>, String>,
    at: usize,
) -> Option<(String, bool)> {
    let slots = match outcome {
        Ok(slots) => slots,
        Err(why) => return Some((why.clone(), false)),
    };
    [at, at + 1].into_iter().find_map(|i| {
        let slot = slots.get(i);
        refused(slot).map(|why| (why, stale(slot)))
    })
}

/// One key device's Key and Scale read back (its two slots from `at`), or
/// why not.
pub(super) fn read_values(
    outcome: &Result<Vec<Value>, String>,
    at: usize,
) -> Result<(f64, f64), String> {
    let slots = outcome.as_ref().map_err(Clone::clone)?;
    let number = |i: usize| -> Result<f64, String> {
        let slot = slots.get(i);
        match slot.and_then(answered) {
            Some(data) => data.as_f64().ok_or_else(|| "not a number".to_string()),
            None => Err(refused(slot).unwrap_or_default()),
        }
    };
    Ok((number(at)?, number(at + 1)?))
}

/// The key Live's bar shows in a read-back (its two slots from `at`).
pub(super) fn bar_read(outcome: &Result<Vec<Value>, String>, at: usize) -> Option<Key> {
    let slots = outcome.as_ref().ok()?;
    let data = |i: usize| slots.get(i).and_then(answered);
    bar_shows(
        data(at).and_then(Value::as_u64),
        data(at + 1).and_then(Value::as_str),
    )
}

/// A round trip in ms.
pub(super) fn elapsed_ms(elapsed: Duration) -> f64 {
    elapsed.as_secs_f64() * 1000.0
}

/// The key a target found gets: the key in force, unless its device's
/// `$ref`s (`refs`) are the ones it was last written to (`wrote`), and only
/// once both its values were heard (`heard`).
pub(super) fn found_key_write(
    wrote: Option<&(String, String)>,
    refs: &(String, String),
    heard: bool,
    in_force: Option<Key>,
) -> Option<Key> {
    in_force.filter(|_| heard && wrote != Some(refs))
}

/// Whether a value heard is judged for another writer (PR 2a review M3):
/// only while the set is not held and the target's walk confirmed its
/// device, and only once per key (`already`).
pub(super) fn judged_for_another(sure: bool, held: bool, already: bool) -> bool {
    sure && !held && !already
}

/// The lateness to measure for a key change decided at the playhead `time`
/// (spec §6): from its change's start in song `song` (one lookahead on, at
/// `tempo`), only while the set plays with a tempo heard (a stopped jump
/// past a key clip is no crossing) and the song has a change in force.
pub(super) fn late_note(
    map: &fohmixer_proto::songmap::SongMap,
    song: u16,
    time: f64,
    playing: bool,
    tempo: Option<f64>,
) -> Option<LateNote> {
    let tempo = tempo.filter(|_| playing)?;
    let change = change_time(map, song, time + lookahead_beats(Some(tempo)))?;
    Some(LateNote {
        song,
        at: time,
        change,
        tempo,
    })
}

/// One key device a write goes to: its target's key, the device, its
/// target's label and the values written.
struct Going {
    key: String,
    device: KeyFound,
    label: String,
    values: (f64, f64),
}

impl Router {
    /// Writes `key` to the found key devices of `keys` (targets of `set`,
    /// in that order) whose values were heard, and to Live's bar for the
    /// set's own write (`why` song or change) or when a held one owes it:
    /// in one call through their `$ref`s, recorded first (`key_write`);
    /// the answer comes back as [`RouterMsg::AutotuneKeysWritten`]. Held
    /// while the set's markers are not settled: each target and the bar
    /// then owe the key. A target without its values heard owes it too.
    /// Nothing to write: no record, no call.
    pub(super) fn autotune_key_write(&mut self, set: &str, keys: &[String], key: Key, why: KeyWhy) {
        let own = matches!(why, KeyWhy::Song | KeyWhy::Change);
        let holding = self.autotune.held.contains(set);
        if !may_write(holding, self.reverb_settled(set)) {
            for k in keys {
                if let Some(target) = self.autotune.targets.get_mut(k) {
                    target.key_wrote = None;
                }
            }
            if let Some(position) = self.autotune.positions.get_mut(set) {
                position.bar_owed |= own;
            }
            self.autotune_hold(set, "markers");
            return;
        }
        let mut found: Vec<Going> = Vec::new();
        for k in keys {
            let Some(target) = self.autotune.targets.get_mut(k) else {
                continue;
            };
            let Some(Ok(device)) = &target.key else {
                continue;
            };
            let device = device.clone();
            let (Some(key_value), Some(scale_value)) = target.values else {
                // Owed until both its values are heard.
                target.key_wrote = None;
                continue;
            };
            found.push(Going {
                key: k.clone(),
                device,
                label: target.spec.label.clone(),
                values: key_values(key, (Some(key_value), Some(scale_value))),
            });
        }
        let bar = self
            .autotune
            .positions
            .get(set)
            .is_some_and(|p| own || p.bar_owed);
        if found.is_empty() && !bar {
            return;
        }
        let Some(live) = self.live.get(set).cloned() else {
            return;
        };
        self.autotune.key_writes += 1;
        let write = self.autotune.key_writes;
        for going in &found {
            if let Some(target) = self.autotune.targets.get_mut(&going.key) {
                target.keyt.want(key);
                target.keyt.sent();
                target.key_write = write;
                target.key_wrote =
                    Some((going.device.key.id.clone(), going.device.scale.id.clone()));
            }
        }
        let note = self
            .autotune_late_note(set)
            .filter(|_| why == KeyWhy::Change);
        if bar && let Some(position) = self.autotune.positions.get_mut(set) {
            position.bar_writing = Some((write, key));
            position.bar_owed = false;
            position.late = note;
        }
        self.autotune_step(
            Step::KeyWrite {
                instance: set.to_string(),
                key: key.name(),
                why,
                targets: found.iter().map(|going| going.label.clone()).collect(),
                bar,
            },
            None,
        );
        let keys: Vec<String> = found.iter().map(|going| going.key.clone()).collect();
        let commands = {
            let devices: Vec<(&KeyFound, (f64, f64))> = found
                .iter()
                .map(|going| (&going.device, going.values))
                .collect();
            key_write_commands(&devices, bar.then_some(key))
        };
        let started = Instant::now();
        let result = live.call(commands);
        let (instance, tx) = (set.to_string(), self.io.tx.clone());
        tokio::spawn(async move {
            let outcome = result.await.map_err(|e| e.to_string());
            let rtt_ms = elapsed_ms(started.elapsed());
            let _ = tx.send(RouterMsg::AutotuneKeysWritten {
                instance,
                write,
                keys,
                outcome,
                rtt_ms,
            });
        });
    }

    /// What `set`'s key change now is measured from: its song's change in
    /// force at the playhead (see [`late_note`]).
    fn autotune_late_note(&self, set: &str) -> Option<LateNote> {
        let position = self.autotune.positions.get(set)?;
        let song = position.follow.now().0?;
        let map = &self.songmap.maps.get(set)?.map;
        late_note(
            map,
            song,
            position.time?,
            position.playing == Some(true),
            position.tempo,
        )
    }

    /// The key Live's bar is being written with by write `write`, if that
    /// is the bar's newest write.
    fn autotune_bar_of(&self, set: &str, write: u64) -> Option<Key> {
        let (newest, key) = self.autotune.positions.get(set)?.bar_writing?;
        (newest == write).then_some(key)
    }

    /// The answer to key write `write` of `instance`'s targets `keys` (its
    /// slots in that order, then Live's bar's): the change's lateness
    /// (`late`), each target whose newest write it is refused (`readback`,
    /// a stale `$ref` walked again) or read back, the read-back's call for
    /// the others and the bar, and one retry of the refused ones.
    pub(in crate::router) fn autotune_keys_written(
        &mut self,
        instance: &str,
        write: u64,
        keys: &[String],
        outcome: &Result<Vec<Value>, String>,
        rtt_ms: f64,
    ) {
        let bar = self.autotune_bar_of(instance, write);
        let note = match bar {
            Some(_) => self
                .autotune
                .positions
                .get_mut(instance)
                .and_then(|position| position.late.take()),
            None => None,
        };
        if let Some(note) = note {
            self.autotune_step(
                Step::Late {
                    instance: instance.to_string(),
                    song: note.song,
                    ms: late_ms(note.at, note.change, note.tempo, rtt_ms),
                },
                None,
            );
        }
        let mut read: Vec<(String, KeyFound)> = Vec::new();
        let mut retry: Vec<String> = Vec::new();
        let mut again = None;
        for (i, key) in keys.iter().enumerate() {
            let Some(target) = self.autotune.targets.get(key) else {
                continue;
            };
            if target.key_write != write {
                continue;
            }
            match write_problem(outcome, 2 * i) {
                None => {
                    if let Some(Ok(device)) = &target.key {
                        read.push((key.clone(), device.clone()));
                    }
                }
                Some((why, gone)) => {
                    let wanted = self.autotune_key_outcome(key, Readback::Refused(why));
                    if gone {
                        // The device went: its walk finds it anew (found).
                        self.autotune_walk(key);
                    } else if wanted.is_some() {
                        retry.push(key.clone());
                        again = wanted;
                    }
                }
            }
        }
        self.autotune_key_read(instance, write, read, bar.is_some());
        if let Some(key) = again {
            self.autotune_key_write(instance, &retry, key, KeyWhy::Retry);
        }
    }

    /// Reads back write `write`'s targets `devices` (and, with `bar`, Live's
    /// bar) through their `$ref`s; the answer comes back as
    /// [`RouterMsg::AutotuneKeysRead`]. Nothing to read: no call.
    fn autotune_key_read(
        &mut self,
        instance: &str,
        write: u64,
        devices: Vec<(String, KeyFound)>,
        bar: bool,
    ) {
        if devices.is_empty() && !bar {
            return;
        }
        let Some(live) = self.live.get(instance).cloned() else {
            return;
        };
        let commands = {
            let found: Vec<&KeyFound> = devices.iter().map(|(_, device)| device).collect();
            key_read_commands(&found, bar)
        };
        let result = live.call(commands);
        let keys: Vec<String> = devices.into_iter().map(|(key, _)| key).collect();
        let (instance, tx) = (instance.to_string(), self.io.tx.clone());
        tokio::spawn(async move {
            let outcome = result.await.map_err(|e| e.to_string());
            let _ = tx.send(RouterMsg::AutotuneKeysRead {
                instance,
                write,
                keys,
                outcome,
            });
        });
    }

    /// The read-back of write `write` of `instance`'s targets `keys` (their
    /// slots in that order, then Live's bar's): each target whose newest
    /// write it is takes the values read and is judged (`readback`), Live's
    /// bar too (`bar`), and the ones not confirmed are retried once.
    pub(in crate::router) fn autotune_keys_read(
        &mut self,
        instance: &str,
        write: u64,
        keys: &[String],
        outcome: &Result<Vec<Value>, String>,
    ) {
        let mut retry: Vec<String> = Vec::new();
        let mut again = None;
        for (i, key) in keys.iter().enumerate() {
            let Some(target) = self.autotune.targets.get_mut(key) else {
                continue;
            };
            let Some(want) = target.keyt.wanted().filter(|_| target.key_write == write) else {
                continue;
            };
            let judged = match read_values(outcome, 2 * i) {
                Ok(values) => {
                    target.values = (Some(values.0), Some(values.1));
                    judge(want, values)
                }
                Err(why) => Readback::Refused(why),
            };
            if let Some(wanted) = self.autotune_key_outcome(key, judged) {
                retry.push(key.clone());
                again = Some(wanted);
            }
        }
        if let Some(want) = self.autotune_bar_of(instance, write) {
            let seen = bar_read(outcome, 2 * keys.len());
            if let Some(position) = self.autotune.positions.get_mut(instance) {
                position.bar = seen.map(|key| key.name());
                position.bar_writing = None;
            }
            self.autotune_step(
                Step::Bar {
                    instance: instance.to_string(),
                    key: want.name(),
                    state: bar_state(want, seen),
                    shows: seen.map(|key| key.name()),
                },
                None,
            );
        }
        if let Some(key) = again {
            self.autotune_key_write(instance, &retry, key, KeyWhy::Retry);
        }
    }

    /// A target's write came to `outcome`: recorded (`readback`); the key
    /// to write again when it is to be retried.
    fn autotune_key_outcome(&mut self, key: &str, outcome: Readback) -> Option<Key> {
        let target = self.autotune.targets.get_mut(key)?;
        let want = target.keyt.wanted()?;
        let (seen, problem) = match &outcome {
            Readback::Confirmed => (Some(want), None),
            Readback::Shows(seen) => (*seen, None),
            Readback::Refused(why) => (None, Some(why.clone())),
        };
        let again = target.keyt.written(outcome);
        let step = Step::Readback {
            instance: target.instance.clone(),
            target: target.track.clone(),
            key: want.name(),
            state: target.keyt.state(),
            shows: seen.map(|key| key.name()),
            problem,
        };
        self.autotune_step(step, None);
        again.map(|_| want)
    }

    /// A target found (`autotune_key_found`) or one of its values heard: the
    /// key in force written to it alone (`found`) when it is owed (another
    /// device, none written since a disconnect or a held write) and both
    /// its values are heard.
    pub(super) fn autotune_key_owed(&mut self, key: &str) {
        let Some(target) = self.autotune.targets.get(key) else {
            return;
        };
        let Some(Ok(device)) = &target.key else {
            return;
        };
        let refs = (device.key.id.clone(), device.scale.id.clone());
        let heard = target.values.0.is_some() && target.values.1.is_some();
        let in_force = self
            .autotune
            .positions
            .get(&target.instance)
            .and_then(|p| p.follow.now().1);
        let instance = target.instance.clone();
        let owed = found_key_write(target.key_wrote.as_ref(), &refs, heard, in_force);
        if let Some(want) = owed {
            self.autotune_key_write(&instance, &[key.to_string()], want, KeyWhy::Found);
        }
    }

    /// A value of a target's key device heard: another writer's when it no
    /// longer shows the key the hub confirmed there (`another_writer`, once
    /// per key, never written against; see [`judged_for_another`]).
    pub(super) fn autotune_another(&mut self, key: &str) {
        let Some(target) = self.autotune.targets.get_mut(key) else {
            return;
        };
        let (Some(key_value), Some(scale_value)) = target.values else {
            return;
        };
        let held = self.autotune.held.contains(&target.instance);
        let already = target.keyt.state() == KeyState::AnotherWriter;
        if !judged_for_another(target.key_sure, held, already)
            || !target.keyt.heard((key_value, scale_value))
        {
            return;
        }
        target.keyt.another();
        let step = Step::AnotherWriter {
            instance: target.instance.clone(),
            target: target.track.clone(),
            shows: shows(key_value, scale_value).map(|key| key.name()),
        };
        self.autotune_step(step, None);
    }

    /// A target's key writes forgotten (a disconnect, another device, its
    /// rack or device lost): nothing wanted, an answer on its way void, the
    /// key owed, the device not confirmed.
    pub(super) fn autotune_key_forget(&mut self, key: &str) {
        if let Some(target) = self.autotune.targets.get_mut(key) {
            target.keyt = KeyTarget::new();
            target.key_write = 0;
            target.key_wrote = None;
            target.key_sure = false;
        }
    }
}

#[cfg(test)]
#[path = "key_writes_tests.rs"]
mod tests;
