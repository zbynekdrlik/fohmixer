//! The walk behind `GET /api/mbc/stream-proof` (#267): one read of the
//! `mbc` Live set through its Live client (`get_prop` only: no write, no
//! subscription), decided by `fohmixer_proto::stream_proof::decide`, at
//! most once per [`WINDOW`] and one at a time ([`StreamProofs::answer`]).
//!
//! **The walk** ([`walk`]), three steps, each through
//! [`LiveHandle::call`] in requests of at most [`CHUNK`] commands, one
//! after another:
//!
//! 1. `live_set tracks` and `live_set return_tracks` (each object's path in
//!    index form, its name and its `$ref`);
//! 2. per track `name`, `mute`, `is_foldable`, `group_track` and the
//!    `display_name` of `<track> output_routing_type` /
//!    `output_routing_channel`; per return its `name`, `mute` and output;
//!    the main track's output; and, when the list named exactly one track
//!    [`STREAM_TRACK`], that track's `<track> input_routing_type`
//!    `display_name` (its name is read in the same request, so the input is
//!    that track's; none read when the list named it never or twice, which
//!    the decision fails on anyway). A track whose name is not the one the list
//!    gave was moved meanwhile: the walk fails (never a fact paired with
//!    another track). `group_track` answers the group as an object whose
//!    `$ref` is the listed group's (the script's id is Live's pointer), so
//!    the chain is followed by `$ref`, never by name;
//! 3. the `name` (checked again) and `<track> mixer_device sends` (its
//!    count) of each `Sends Only` track.
//!
//! Any slot that fails fails the walk with the script's words
//! (`<path> <prop>: <error>`), and so do a call (offline, a timeout), an
//! answer with fewer or more slots than reads, a list that is no list or
//! holds an object without a path or `$ref`, and a sends answer that is no
//! list.
//!
//! **The answer** ([`StreamProofs::answer`]): the walks take turns on one
//! lock, and an answer younger than [`WINDOW`] is served again with its
//! `age_ms` (a request that waited for a walk gets that walk's answer); a
//! hub without `mbc` or with `mbc` offline is answered at once and nothing
//! is cached (no read was made); a cached answer is served for the rest of
//! its window even when `mbc` went offline meanwhile (its `ts` and `age_ms`
//! say how old it is). Every walk is one `stream_proof` record (`ok`, `ms`,
//! `open`, `hand1_sync_only`, or `error`, which makes it warn class) and
//! one info line (warn when it failed).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use fohmixer_proto::roles::MBC_INSTANCE;
use fohmixer_proto::stream_proof::{
    Answer, Output, Return, SENDS_ONLY, STREAM_TRACK, Set, Track, decide,
};
use serde_json::{Value, json};

use crate::eq::walk::{answered, get, items, refused};
use crate::events::EventLog;
use crate::live::client::LiveHandle;
use crate::ration::Ration;

/// An answer is served again for this long.
pub const WINDOW: Duration = Duration::from_secs(30);
/// The commands of one request at most.
pub const CHUNK: usize = 200;
/// The reads of one track in step 2.
const TRACK_READS: usize = 6;
/// The reads of one return in step 2.
const RETURN_READS: usize = 4;
/// The main track.
const MAIN: &str = "live_set master_track";

/// A listed object: its path (index form), its name and its `$ref`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Listed {
    path: String,
    name: String,
    id: String,
}

/// Step 1's commands.
fn list_commands() -> Vec<Value> {
    vec![get("live_set", "tracks"), get("live_set", "return_tracks")]
}

/// A slot's data, or why it has none (`what`: its path and prop).
fn data<'a>(slots: &'a [Value], at: usize, what: &str) -> Result<&'a Value, String> {
    let slot = slots.get(at);
    match refused(slot) {
        Some(why) => Err(format!("{what}: {why}")),
        None => Ok(slot.and_then(answered).unwrap_or(&Value::Null)),
    }
}

/// A list answer's objects: a list whose every object has a path and a
/// `$ref`, or why not.
fn listed(slots: &[Value], at: usize, what: &str) -> Result<Vec<Listed>, String> {
    let value = data(slots, at, what)?;
    let Some(all) = value.as_array() else {
        return Err(format!("{what}: not a list: {value}"));
    };
    let objects: Vec<Listed> = items(value)
        .into_iter()
        .filter(|item| !item.id.is_empty())
        .map(|item| Listed {
            path: item.path.to_string(),
            name: item.name.to_string(),
            id: item.id.to_string(),
        })
        .collect();
    if objects.len() != all.len() {
        return Err(format!("{what}: an object without a path or a $ref"));
    }
    Ok(objects)
}

/// The `get_prop`s of an object's output routing.
fn output_commands(path: &str) -> [Value; 2] {
    [
        get(&format!("{path} output_routing_type"), "display_name"),
        get(&format!("{path} output_routing_channel"), "display_name"),
    ]
}

/// The index of the one listed track named [`STREAM_TRACK`] (none: no
/// such track, or two).
fn stream_listed(tracks: &[Listed]) -> Option<usize> {
    let mut found = tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.name == STREAM_TRACK)
        .map(|(i, _)| i);
    match (found.next(), found.next()) {
        (Some(i), None) => Some(i),
        _ => None,
    }
}

/// The `get_prop` of a track's input routing type.
fn input_command(path: &str) -> Value {
    get(&format!("{path} input_routing_type"), "display_name")
}

/// Step 2's commands: every track's, every return's, the main track's, the
/// stream track's input.
fn fact_commands(tracks: &[Listed], returns: &[Listed]) -> Vec<Value> {
    let mut commands = Vec::new();
    for t in tracks {
        commands.extend([
            get(&t.path, "name"),
            get(&t.path, "mute"),
            get(&t.path, "is_foldable"),
            get(&t.path, "group_track"),
        ]);
        commands.extend(output_commands(&t.path));
    }
    for r in returns {
        commands.extend([get(&r.path, "name"), get(&r.path, "mute")]);
        commands.extend(output_commands(&r.path));
    }
    commands.extend(output_commands(MAIN));
    if let Some(i) = stream_listed(tracks) {
        commands.push(input_command(&tracks[i].path));
    }
    commands
}

/// A slot's text.
fn text(slots: &[Value], at: usize, what: &str) -> Result<String, String> {
    match data(slots, at, what)? {
        Value::String(s) => Ok(s.clone()),
        other => Err(format!("{what}: not a text: {other}")),
    }
}

/// A slot's bool.
fn flag(slots: &[Value], at: usize, what: &str) -> Result<bool, String> {
    match data(slots, at, what)? {
        Value::Bool(b) => Ok(*b),
        other => Err(format!("{what}: not a bool: {other}")),
    }
}

/// The output at `at` (type, then channel) of `path`.
fn output(slots: &[Value], at: usize, path: &str) -> Result<Output, String> {
    Ok(Output {
        kind: text(slots, at, &format!("{path} output_routing_type"))?,
        channel: text(slots, at + 1, &format!("{path} output_routing_channel"))?,
    })
}

/// The name at `at` of `listed`, which must be the one the list gave.
fn same_name(slots: &[Value], at: usize, listed: &Listed) -> Result<String, String> {
    let name = text(slots, at, &format!("{} name", listed.path))?;
    if name != listed.name {
        return Err(format!(
            "the tracks changed during the read: {} is now {name:?}, was {:?}",
            listed.path, listed.name
        ));
    }
    Ok(name)
}

/// The group a `group_track` answer names, by its `$ref` among the listed
/// tracks (none: no group).
fn parent(value: &Value, tracks: &[Listed], what: &str) -> Result<Option<usize>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let id = value.get("$ref").and_then(Value::as_str).unwrap_or("");
    tracks
        .iter()
        .position(|t| !id.is_empty() && t.id == id)
        .map(Some)
        .ok_or_else(|| format!("{what}: a group that is not in the track list"))
}

/// Step 2's answer as the set's facts (every track's sends 0).
fn facts(slots: &[Value], tracks: &[Listed], returns: &[Listed]) -> Result<Set, String> {
    let mut set = Set {
        tracks: Vec::new(),
        returns: Vec::new(),
        main: Output::default(),
        stream_input: None,
    };
    for (i, t) in tracks.iter().enumerate() {
        let at = i * TRACK_READS;
        let what = |prop: &str| format!("{} {prop}", t.path);
        set.tracks.push(Track {
            name: same_name(slots, at, t)?,
            mute: flag(slots, at + 1, &what("mute"))?,
            group: flag(slots, at + 2, &what("is_foldable"))?,
            parent: parent(
                data(slots, at + 3, &what("group_track"))?,
                tracks,
                &what("group_track"),
            )?,
            output: output(slots, at + 4, &t.path)?,
            sends: 0,
        });
    }
    let base = tracks.len() * TRACK_READS;
    for (i, r) in returns.iter().enumerate() {
        let at = base + i * RETURN_READS;
        set.returns.push(Return {
            name: same_name(slots, at, r)?,
            mute: flag(slots, at + 1, &format!("{} mute", r.path))?,
            output: output(slots, at + 2, &r.path)?,
        });
    }
    let main_at = base + returns.len() * RETURN_READS;
    set.main = output(slots, main_at, MAIN)?;
    if let Some(i) = stream_listed(tracks) {
        let what = format!("{} input_routing_type", tracks[i].path);
        set.stream_input = Some(text(slots, main_at + 2, &what)?);
    }
    Ok(set)
}

/// The tracks of step 3 (each `Sends Only` one) and their commands: its
/// name (a move meanwhile fails the walk) and its sends.
fn sends_commands(set: &Set, tracks: &[Listed]) -> (Vec<usize>, Vec<Value>) {
    let which: Vec<usize> = set
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.output.kind == SENDS_ONLY)
        .map(|(i, _)| i)
        .collect();
    let commands = which
        .iter()
        .flat_map(|&i| {
            [
                get(&tracks[i].path, "name"),
                get(&format!("{} mixer_device", tracks[i].path), "sends"),
            ]
        })
        .collect();
    (which, commands)
}

/// A sends answer's count, or why it is none.
fn sends_count(slots: &[Value], at: usize, what: &str) -> Result<usize, String> {
    match data(slots, at, what)? {
        Value::Array(sends) => Ok(sends.len()),
        other => Err(format!("{what}: not a list: {other}")),
    }
}

/// Every command through `live`, at most [`CHUNK`] per request, in order.
async fn call(live: &LiveHandle, commands: Vec<Value>) -> Result<Vec<Value>, String> {
    let mut slots = Vec::with_capacity(commands.len());
    for chunk in commands.chunks(CHUNK) {
        let answer = live
            .call(chunk.to_vec())
            .await
            .map_err(|e| format!("Live did not answer: {e}"))?;
        if answer.len() != chunk.len() {
            return Err(format!(
                "Live answered {} of {} reads",
                answer.len(),
                chunk.len()
            ));
        }
        slots.extend(answer);
    }
    Ok(slots)
}

/// Reads the set's facts (see the module's doc).
pub async fn walk(live: &LiveHandle) -> Result<Set, String> {
    let first = call(live, list_commands()).await?;
    let tracks = listed(&first, 0, "live_set tracks")?;
    let returns = listed(&first, 1, "live_set return_tracks")?;
    let slots = call(live, fact_commands(&tracks, &returns)).await?;
    let mut set = facts(&slots, &tracks, &returns)?;
    let (which, commands) = sends_commands(&set, &tracks);
    let slots = call(live, commands).await?;
    for (n, &i) in which.iter().enumerate() {
        same_name(&slots, 2 * n, &tracks[i])?;
        let what = format!("{} mixer_device sends", tracks[i].path);
        set.tracks[i].sends = sends_count(&slots, 2 * n + 1, &what)?;
    }
    Ok(set)
}

/// A cached answer and when its walk ended.
#[derive(Debug, Clone)]
struct Cached {
    at: Instant,
    answer: Answer,
}

/// How old an answer of `at` is at `now`, while it is younger than
/// `window` (none: walk again).
pub fn age_within(at: Instant, now: Instant, window: Duration) -> Option<u64> {
    let age = now.saturating_duration_since(at);
    (age < window).then(|| u64::try_from(age.as_millis()).unwrap_or(u64::MAX))
}

/// The `mbc` instance to walk, or why there is no walk to make.
pub fn to_walk(live: Option<&LiveHandle>) -> Result<&LiveHandle, &'static str> {
    match live {
        None => Err("this hub has no mbc Live set"),
        Some(live) if !live.snapshot().online => Err("the mbc Live set is offline"),
        Some(live) => Ok(live),
    }
}

/// The fields of a walk's record.
pub fn record_fields(answer: &Answer, ms: u64) -> Value {
    match &answer.reason {
        Some(reason) => json!({"ok": false, "ms": ms, "error": reason}),
        None => json!({"ok": true, "ms": ms, "open": answer.open,
                       "hand1_sync_only": answer.hand1_sync_only}),
    }
}

/// The hub's stream proofs: the last answer and the refusals' ration.
#[derive(Debug)]
pub struct StreamProofs {
    last: tokio::sync::Mutex<Option<Cached>>,
    window: Duration,
    /// The ration of the route's refusal lines and records.
    pub refusals: Mutex<Ration>,
}

impl Default for StreamProofs {
    fn default() -> Self {
        Self::with_window(WINDOW)
    }
}

impl StreamProofs {
    /// Proofs served again for `window` (the hub's: [`WINDOW`]).
    pub fn with_window(window: Duration) -> Self {
        Self {
            last: tokio::sync::Mutex::new(None),
            window,
            refusals: Mutex::new(Ration::default()),
        }
    }

    /// The answer for a request (see the module's doc): `live` is the
    /// hub's `mbc` instance.
    pub async fn answer(&self, live: Option<&LiveHandle>, events: &EventLog) -> Answer {
        let mut last = self.last.lock().await;
        if let Some(cached) = last.as_ref()
            && let Some(age_ms) = age_within(cached.at, Instant::now(), self.window)
        {
            let mut answer = cached.answer.clone();
            answer.age_ms = age_ms;
            return answer;
        }
        let now_ms = || u64::try_from(crate::events::now_ms()).unwrap_or(0);
        let live = match to_walk(live) {
            Ok(live) => live,
            Err(why) => {
                tracing::info!(instance = MBC_INSTANCE, "stream proof: no walk: {why}");
                return Answer::of(Err(why.to_string()), now_ms());
            }
        };
        let started = Instant::now();
        let result = walk(live).await.and_then(|set| decide(&set));
        let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let answer = Answer::of(result, now_ms());
        match &answer.reason {
            Some(reason) => tracing::warn!(ms, "stream proof: no proof: {reason}"),
            None => tracing::info!(
                ms,
                open = ?answer.open,
                hand1_sync_only = answer.hand1_sync_only,
                "stream proof: walked"
            ),
        }
        events.record("stream_proof", record_fields(&answer, ms));
        *last = Some(Cached {
            at: Instant::now(),
            answer: answer.clone(),
        });
        answer
    }
}

#[cfg(test)]
mod tests;
