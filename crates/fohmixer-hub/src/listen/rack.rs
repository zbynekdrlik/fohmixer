//! The `Listen` rack's Device On (#238): the hub switches a track's rack on
//! only while a page holds the listen of that track, so an idle `FohMixer
//! Listen` device costs Live's audio thread nothing (dasio measured the idle
//! devices making band Live's callback run long and lose blocks).
//!
//! The rack is a `RackDevice` named exactly [`RACK_NAME`] among the
//! `devices` of the track found by its name, the name the page listens to:
//! a track, a return track or the main track of that name (one read of the
//! three, [`read_commands`]; a page listens to a return's or the main's
//! strip too); its Device On is `parameters 0`, written with
//! the script's generic `set_prop value`, as the hub's other LOM writes. One
//! task ([`run`]) carries the switches out one after another, in the order
//! the lease made them ([`super::Listen`]): a take's new rack on, then the
//! old one off. Nothing walks the tracks at the hub's start (#235: a walk
//! costs audio blocks); the sets are saved with every rack off.
//!
//! The decisions are pure ([`start_switches`], [`leave_switch`],
//! [`rack_of`], [`written`]); a failed write is one `listen` record and a
//! warn line, and changes nothing else.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use fohmixer_proto::path::escape_name;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use super::{Start, Target};
use crate::eq::walk::{DEVICES, RACK, answered, get, items, refused};
use crate::events::EventLog;

/// The rack's name: the owner's placement (`listen.md`, the PC checks).
pub const RACK_NAME: &str = "Listen";
/// Why a track has no rack to switch.
pub const NO_RACK: &str = "no Listen rack";
/// Why a track's racks are not switched: which one is the device's?
pub const TWO_RACKS: &str = "two Listen racks";
/// Why a switch had no Live to ask.
pub const UNKNOWN_INSTANCE: &str = "unknown instance";

/// One switch of a track's rack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Switch {
    pub instance: String,
    pub track: String,
    pub on: bool,
}

impl Switch {
    fn of(target: &Target, on: bool) -> Self {
        Self {
            instance: target.instance.clone(),
            track: target.track.clone(),
            on,
        }
    }
}

/// The switches of a start of `to` that found `from` the target: the
/// track's rack on at a new target; none for one more listener; at a take
/// the new rack on FIRST, then the old one off (never the old one off when
/// it is the same track of the same Live: a set loaded meanwhile).
pub fn start_switches(start: Start, from: Option<&Target>, to: &Target) -> Vec<Switch> {
    match (start, from) {
        (Start::Join, _) => Vec::new(),
        (Start::Take, Some(from)) if from.instance != to.instance || from.track != to.track => {
            vec![Switch::of(to, true), Switch::of(from, false)]
        }
        _ => vec![Switch::of(to, true)],
    }
}

/// The switch of a lease's leave, the target's listeners counted after it:
/// the rack off once the last one left.
pub fn leave_switch(target: &Target) -> Option<Switch> {
    (target.listeners == 0).then(|| Switch::of(target, false))
}

/// What a switch did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Device On written.
    Done,
    /// No rack to switch (the track or its rack not there): its words.
    Missing(String),
    /// Live did not take it (offline, no answer, a refused write): its words.
    Failed(String),
}

impl Outcome {
    /// The record's `result`.
    pub fn result(&self) -> &'static str {
        match self {
            Outcome::Done => "done",
            Outcome::Missing(_) => "missing",
            Outcome::Failed(_) => "failed",
        }
    }

    /// Why it is not done.
    pub fn problem(&self) -> Option<&str> {
        match self {
            Outcome::Done => None,
            Outcome::Missing(why) | Outcome::Failed(why) => Some(why),
        }
    }
}

/// The read of the track's devices.
pub fn read_commands(track: &str) -> Vec<Value> {
    let name = escape_name(track);
    vec![
        get(&format!("live_set tracks[name={name}]"), DEVICES),
        get(&format!("live_set return_tracks[name={name}]"), DEVICES),
        get(MAIN, "name"),
        get(MAIN, DEVICES),
    ]
}

/// The main track's path.
const MAIN: &str = "live_set master_track";
/// The script's words for a name no track of a list carries.
const NOT_FOUND: &str = "not found";

/// The rack's path (index form) from the read's answer of `track`
/// ([`read_commands`]: the tracks', the returns' and, when it carries the
/// name, the main track's devices), or why none: none or two `RackDevice`s
/// named [`RACK_NAME`] among them, or the script's words when a list could
/// not be read for another reason than no track of that name (two tracks
/// of the name are `ambiguous`; which one listens?).
pub fn rack_of(track: &str, slots: &[Value]) -> Result<String, Outcome> {
    let main = slots.get(2).and_then(answered).and_then(Value::as_str) == Some(track);
    let mut lists = vec![slots.first(), slots.get(1)];
    if main {
        lists.push(slots.get(3));
    }
    let mut racks = Vec::new();
    let mut doubt = None;
    for slot in lists {
        match refused(slot) {
            None => racks.extend(
                items(slot.and_then(answered).unwrap_or(&Value::Null))
                    .into_iter()
                    .filter(|item| item.class == RACK && item.name == RACK_NAME)
                    .map(|item| item.path.to_string()),
            ),
            Some(why) if !why.starts_with(NOT_FOUND) => {
                doubt.get_or_insert(why);
            }
            Some(_) => {}
        }
    }
    match racks.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(Outcome::Missing(
            doubt.unwrap_or_else(|| NO_RACK.to_string()),
        )),
        _ => Err(Outcome::Missing(TWO_RACKS.to_string())),
    }
}

/// The write of the rack's Device On.
pub fn write_commands(rack: &str, on: bool) -> Vec<Value> {
    vec![json!({
        "target": format!("{rack} parameters 0"),
        "name": "set_prop",
        "args": {"prop": "value", "value": if on { 1.0 } else { 0.0 }},
    })]
}

/// What the write's answer says.
pub fn written(slots: &[Value]) -> Outcome {
    match refused(slots.first()) {
        None => Outcome::Done,
        Some(why) => Outcome::Failed(why),
    }
}

/// The `listen` record of a switch.
pub fn rack_fields(switch: &Switch, outcome: &Outcome) -> Value {
    json!({
        "what": "rack",
        "instance": switch.instance,
        "track": switch.track,
        "on": switch.on,
        "result": outcome.result(),
        "problem": outcome.problem(),
    })
}

/// One batch of commands to an instance's script, answered with its slots
/// or the call's error in words (the hub's [`crate::live::LiveHandle::call`]).
pub type Lom = Arc<
    dyn Fn(&str, Vec<Value>) -> Pin<Box<dyn Future<Output = Result<Vec<Value>, String>> + Send>>
        + Send
        + Sync,
>;

/// A switch for [`run`], with whoever waits for its outcome.
pub struct Job {
    pub switch: Switch,
    pub done: Option<oneshot::Sender<Outcome>>,
}

/// The switches' queue.
pub type Racks = mpsc::UnboundedSender<Job>;

/// One switch: the rack found, then its Device On written.
pub async fn switch(lom: &Lom, switch: &Switch) -> Outcome {
    let read = match lom(&switch.instance, read_commands(&switch.track)).await {
        Ok(slots) => slots,
        Err(why) => return Outcome::Failed(why),
    };
    let rack = match rack_of(&switch.track, &read) {
        Ok(rack) => rack,
        Err(outcome) => return outcome,
    };
    match lom(&switch.instance, write_commands(&rack, switch.on)).await {
        Ok(slots) => written(&slots),
        Err(why) => Outcome::Failed(why),
    }
}

/// Carries the switches out one after another until the queue closes: each
/// one `listen` `rack` record and one log line (warn unless done).
pub async fn run(mut jobs: mpsc::UnboundedReceiver<Job>, lom: Lom, events: EventLog) {
    while let Some(job) = jobs.recv().await {
        let outcome = switch(&lom, &job.switch).await;
        let Switch {
            instance,
            track,
            on,
        } = &job.switch;
        match outcome.problem() {
            None => {
                tracing::info!(instance = ?instance, track = ?track, on, "listen: a Listen rack switched");
            }
            Some(problem) => {
                tracing::warn!(instance = ?instance, track = ?track, on, result = outcome.result(), problem = ?problem, "listen: a Listen rack was not switched");
            }
        }
        events.record("listen", rack_fields(&job.switch, &outcome));
        if let Some(done) = job.done {
            // The page may have gone meanwhile.
            let _ = done.send(outcome);
        }
    }
}

#[cfg(test)]
mod tests;
