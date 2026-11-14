//! The scene automation's rule (#207; the owner's rulings of 2026-11-14):
//! a scene that comes (`POST /api/scene`) moves every fader with a rule for
//! it. Pure: the router (`router/scenes.rs`) resolves each rule's fader in
//! the served layout ([`fader_targets`]), hands the rule Live's values as
//! the hub knows them, and carries out the [`Act`]s: each a `set` of the
//! rule's own client through the setter, as REVERB's (#130).
//!
//! - **The move:** a volume glides from where it is (a glide of the rule
//!   still running on that key: its value now; else Live's) to the rule's
//!   dB, linear in dB over the rule's time (REVERB's [`Glide`], each step
//!   [`STEP_MS`], the last one exactly the target); a time of 0, or a
//!   volume the hub has not heard, writes the target at once. A mute or an
//!   unmute is written once at the start, to the strip's track and the
//!   tracks its linked mute writes (#127), unless the strip's own mute is
//!   there already.
//! - **What moved:** a fader whose volume is not at the target already
//!   (within [`SAME_VOLUME`]) or whose mute changes (or is not known):
//!   the pages' `moved`, which flashes the edge.
//! - **The hand wins** (the owner, 2026-11-14): a page's write of a key
//!   under a glide stops it at once ([`Scenes::hand`]). A new scene's glide
//!   of the same key replaces the running one; an instance going offline
//!   ends its glides.

use fohmixer_proto::client::set_key;
use fohmixer_proto::layout::Control;
use fohmixer_proto::scenes::SceneRule;
use fohmixer_proto::sectors::StripRef;
use fohmixer_proto::volume::{FLOOR_DB, value2db, volume_of};
use serde::Serialize;
use serde_json::{Value, json};

pub use super::reverb::{Glide, SAME_VOLUME, STEP_MS, same_volume};

/// The Live objects a fader's rule writes: its volume (none for a fader
/// without one) and the tracks of its mute (its own first), each with its
/// Live set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaderTargets {
    pub volume: Option<(String, String)>,
    pub mutes: Vec<(String, String)>,
}

/// A control's [`FaderTargets`]: a strip's volume and its track's mute with
/// its linked mutes' tracks; none for any other control (a parameter fader
/// has no volume or mute of its own).
pub fn fader_targets(control: &Control) -> Option<FaderTargets> {
    let Control::Strip(strip) = control else {
        return None;
    };
    let own = strip.binding.target().ok()?;
    let instance = strip.binding.instance.clone();
    let mut mutes = vec![(instance.clone(), own.clone())];
    mutes.extend(
        strip
            .mute_links
            .iter()
            .filter_map(|b| b.target().ok().map(|t| (b.instance.clone(), t))),
    );
    Some(FaderTargets {
        volume: Some((instance, format!("{own} mixer_device volume"))),
        mutes,
    })
}

/// One fader of a scene, as the router hands it: its identity, its rule,
/// what it writes and Live's values of its volume and its own mute (none:
/// not heard).
#[derive(Debug, Clone, PartialEq)]
pub struct SceneFader {
    pub strip: StripRef,
    pub rule: SceneRule,
    pub targets: FaderTargets,
    pub volume_now: Option<f64>,
    pub mute_now: Option<bool>,
}

/// Why a glide stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Why {
    /// It arrived.
    Done,
    /// A page wrote its key: the hand wins.
    Hand,
    /// A new scene glides the same key.
    Replaced,
    /// Its instance went offline.
    Offline,
}

impl Why {
    pub fn as_str(self) -> &'static str {
        match self {
            Why::Done => "done",
            Why::Hand => "hand",
            Why::Replaced => "replaced",
            Why::Offline => "offline",
        }
    }
}

/// A step of the rule: a glide's start and stop (the `scene_glide`
/// records).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "what", rename_all = "lowercase")]
pub enum Step {
    Start {
        instance: String,
        label: String,
        scene: String,
        from_db: f64,
        to_db: f64,
        ms: u32,
    },
    Stop {
        instance: String,
        label: String,
        scene: String,
        why: Why,
    },
}

/// What the router carries out.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// A `set` of `prop` of `target` in `instance` (its write key `key`).
    Write {
        instance: String,
        target: String,
        prop: &'static str,
        key: String,
        value: Value,
    },
    Step(Step),
}

/// A volume's dB as the records show it: two decimals (the law's
/// bisection is 1e-4 wide), the law's floor for −∞.
pub fn shown_db(volume: f64) -> f64 {
    (value2db(volume).max(FLOOR_DB) * 100.0).round() / 100.0
}

/// Whether a fader moves: its volume is not at `to` (or not known), or its
/// mute changes (or is not known).
pub fn moves(
    volume_now: Option<f64>,
    to: Option<f64>,
    mute_now: Option<bool>,
    mute: Option<bool>,
) -> bool {
    let volume = to.is_some_and(|to| volume_now.is_none_or(|now| !same_volume(now, to)));
    let muted = mute.is_some_and(|m| mute_now != Some(m));
    volume || muted
}

/// One glide running.
#[derive(Debug, Clone, PartialEq)]
struct Running {
    instance: String,
    target: String,
    label: String,
    scene: String,
    glide: Glide,
}

/// The rule's state: the glides running, by write key.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scenes {
    glides: std::collections::BTreeMap<String, Running>,
    writes: u64,
}

impl Scenes {
    /// How many glides run.
    pub fn glides(&self) -> usize {
        self.glides.len()
    }

    /// Every glide step written since the start.
    pub fn writes(&self) -> u64 {
        self.writes
    }

    /// `scene` came at `now` with these faders: its writes and steps, and
    /// how many faders it moves.
    pub fn scene(&mut self, scene: &str, faders: Vec<SceneFader>, now: f64) -> (Vec<Act>, u32) {
        let mut acts = Vec::new();
        let mut moved = 0;
        for fader in faders {
            let to = fader.rule.db.map(volume_of);
            let mute = fader.rule.mute.map(|m| m.value());
            let volume = fader.targets.volume.clone().zip(to);
            // A glide of the key still running: where it is now.
            let running = volume.as_ref().and_then(|((instance, target), _)| {
                self.glides
                    .get(&set_key(instance, target, "value"))
                    .map(|r| r.glide.at(now).0)
            });
            let from = running.or(fader.volume_now);
            let moving = moves(from, to, fader.mute_now, mute);
            // A running glide of the key is replaced even when it is at
            // the new target already (it would go on to its own).
            if !moving && running.is_none() {
                continue;
            }
            moved += u32::from(moving);
            if let Some(m) = mute.filter(|m| fader.mute_now != Some(*m)) {
                for (instance, target) in &fader.targets.mutes {
                    acts.push(Act::Write {
                        instance: instance.clone(),
                        target: target.clone(),
                        prop: "mute",
                        key: set_key(instance, target, "mute"),
                        value: json!(m),
                    });
                }
            }
            if let Some(((instance, target), to)) = volume {
                self.glide(
                    &mut acts,
                    scene,
                    &fader.strip.label,
                    (instance, target),
                    (from, to),
                    fader.rule.time_ms,
                    now,
                );
            }
        }
        (acts, moved)
    }

    /// A volume's glide from `from` (none: not heard) to `to`: a running
    /// glide of the key is replaced; none to make when it is there; at once
    /// with no time or no start.
    #[allow(clippy::too_many_arguments)]
    fn glide(
        &mut self,
        acts: &mut Vec<Act>,
        scene: &str,
        label: &str,
        (instance, target): (String, String),
        (from, to): (Option<f64>, f64),
        ms: u32,
        now: f64,
    ) {
        let key = set_key(&instance, &target, "value");
        let replaced = self.glides.remove(&key);
        if let Some(old) = &replaced {
            acts.push(stop(old, Why::Replaced));
        }
        // There already: nothing, but a replaced glide's end is written
        // exactly.
        let there = from.is_some_and(|from| same_volume(from, to));
        if there && replaced.is_none() {
            return;
        }
        let Some(from) = from.filter(|_| ms > 0 && !there) else {
            self.writes += 1;
            acts.push(Act::Write {
                instance,
                target,
                prop: "value",
                key,
                value: json!(to),
            });
            return;
        };
        acts.push(Act::Step(Step::Start {
            instance: instance.clone(),
            label: label.to_string(),
            scene: scene.to_string(),
            from_db: shown_db(from),
            to_db: shown_db(to),
            ms,
        }));
        self.glides.insert(
            key,
            Running {
                instance,
                target,
                label: label.to_string(),
                scene: scene.to_string(),
                glide: Glide::new(from, to, now, f64::from(ms)),
            },
        );
    }

    /// A page wrote `key`: its glide stops there.
    pub fn hand(&mut self, key: &str) -> Vec<Act> {
        self.glides
            .remove(key)
            .map(|r| vec![stop(&r, Why::Hand)])
            .unwrap_or_default()
    }

    /// `instance` went offline: its glides end.
    pub fn offline(&mut self, instance: &str) -> Vec<Act> {
        let gone: Vec<String> = self
            .glides
            .iter()
            .filter(|(_, r)| r.instance == instance)
            .map(|(k, _)| k.clone())
            .collect();
        gone.iter()
            .filter_map(|k| self.glides.remove(k))
            .map(|r| stop(&r, Why::Offline))
            .collect()
    }

    /// The time is `now`: every glide's step (its last one exactly the
    /// target, then its stop `done`).
    pub fn tick(&mut self, now: f64) -> Vec<Act> {
        let mut acts = Vec::new();
        let mut done = Vec::new();
        for (key, running) in &self.glides {
            let (value, last) = running.glide.at(now);
            acts.push(Act::Write {
                instance: running.instance.clone(),
                target: running.target.clone(),
                prop: "value",
                key: key.clone(),
                value: json!(value),
            });
            if last {
                done.push(key.clone());
            }
        }
        self.writes += acts.len() as u64;
        for key in done {
            if let Some(r) = self.glides.remove(&key) {
                acts.push(stop(&r, Why::Done));
            }
        }
        acts
    }

    /// When the rule next needs the time: the next step while a glide runs.
    pub fn next_tick(&self, now: f64) -> Option<f64> {
        (!self.glides.is_empty()).then_some(now + STEP_MS)
    }
}

/// The stop of `running` for `why`.
fn stop(running: &Running, why: Why) -> Act {
    Act::Step(Step::Stop {
        instance: running.instance.clone(),
        label: running.label.clone(),
        scene: running.scene.clone(),
        why,
    })
}

/// A step's log line.
pub fn step_line(step: &Step) -> String {
    match step {
        Step::Start {
            instance,
            label,
            scene,
            from_db,
            to_db,
            ms,
        } => format!(
            "scene {scene:?} glides {instance}|{label} from {from_db:.1} dB to {to_db:.1} dB in {ms} ms"
        ),
        Step::Stop {
            instance,
            label,
            scene,
            why,
        } => format!(
            "scene {scene:?} stopped the glide of {instance}|{label}: {}",
            why.as_str()
        ),
    }
}

#[cfg(test)]
#[path = "scenes/tests.rs"]
mod tests;
