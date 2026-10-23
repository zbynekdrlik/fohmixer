//! The router's song map (#140 PR 1): it reads every Live set's locators
//! and its `Sections` track's clips into a song map
//! (`fohmixer_proto::songmap`), keeps the newest one per set, serves it at
//! `GET /api/songmap`, sums it up in `/api/status` and records each read
//! (`songmap`). It writes nothing to Live: every command is a `get_prop`,
//! and its subscriptions are its own ([`SONGMAP_CLIENT`]).
//!
//! **When it reads** (each a trigger through [`next_read`]): a connect (the
//! first values of its subscriptions after it: `live_set cue_points` and
//! `live_set tracks`, so a read never misses a change made between its
//! answer and the listeners'), a change of the locators' list, of the track
//! list (a `Sections` track added, moved or deleted), of the Sections
//! track's clips (followed by its index once a read found it), and every
//! transport change of that set, so a stop or a start picks up renames and
//! moves made in Live (a locator's name and time and a clip's name are not
//! followed one by one: 928 + 3508 listeners would be too many). A trigger
//! while a read runs reads once more after it.
//!
//! **How it reads** (one task per read, every loop over a list it holds):
//! one call for the locators, the tracks and the clips of
//! `tracks[name=Sections]` (the script refuses an ambiguous name: with two
//! such tracks the first one's clips are read by its index in a second
//! call), then the needed locators' `time` and the key clips' `start_time`
//! through each object's `$ref` (an object moved or deleted meanwhile is
//! never paired with another one's time) in calls of at most `TIME_CHUNK`
//! commands, one after another. A failed
//! call ends the read with its error and keeps the old map, stale, and the
//! read is tried once more after [`RETRY_MS`]; a disconnect voids the read
//! in flight (its answer's number is no longer the set's; its task is
//! aborted, so it asks Live for nothing more) and keeps the last map, stale
//! until the next read.

use std::collections::{BTreeMap, BTreeSet};

use fohmixer_proto::client::{ProblemReport, SongMapReport, SongMapStatus, SpanReport, ValueItem};
use fohmixer_proto::songmap::{
    ClipName, Cue, Problem, ProblemKind, SectionsState, SongMap, TIME_CHUNK, build, classify,
    clip_needs_time, counts, needs_time,
};
use serde_json::{Value, json};

use super::{Router, RouterMsg};
use crate::eq::walk::{answered, get, items, refused};
use crate::live::client::LiveHandle;
use crate::live::subs::ClientId;

/// The song map's subscriber (clients count up from 1; the unfold and
/// marker keepers, REVERB and AUTOTUNE are `MAX` … `MAX − 3`).
pub const SONGMAP_CLIENT: ClientId = ClientId::MAX - 4;
/// The track AbleSet reads the sections from, by its name.
pub const SECTIONS: &str = "live_set tracks[name=Sections]";
/// That name.
pub const SECTIONS_NAME: &str = "Sections";
/// The set's locators.
pub const CUE_POINTS: &str = "cue_points";
/// A track's clips in the arrangement.
pub const CLIPS: &str = "arrangement_clips";
/// The problem texts a `songmap` record carries at most.
pub const MAX_TEXTS: usize = 20;
/// A failed read (no disconnect) is tried once more after this, in ms (the
/// unfold and marker keepers' `RETRY_MS`; #140 review M3).
pub const RETRY_MS: u64 = 2000;

/// One read's map.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Read {
    pub map: SongMap,
    pub sections: SectionsState,
    /// When it was read (hub UTC ms).
    pub at_ms: u64,
    pub read_no: u64,
}

/// What a read found in Live, for `build`.
#[derive(Debug, Clone, PartialEq)]
pub struct Listing {
    pub cues: Vec<Cue>,
    pub clips: Vec<ClipName>,
    pub sections: SectionsState,
    /// The first `Sections` track's path (index form), none without one.
    pub track: Option<String>,
}

/// One object of a list a read found: its path (index form, for the words
/// of a failed read), its name, and its `$ref` with its class, through
/// which its time is read (#140 review M2: a locator moved past another
/// during a read is never paired with the other one's time, and a deleted
/// one answers a stale reference, which fails the read).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Object {
    pub path: String,
    pub name: String,
    pub id: String,
    pub class: String,
}

/// What the first call of a read found: the locators and the Sections
/// tracks, each with its path, name and `$ref`.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Listed {
    pub cues: Vec<Object>,
    pub sections: SectionsState,
    pub track: Option<String>,
    /// The Sections clips when the name binding answered (one such track);
    /// none: read them at `track`, if any.
    pub clips: Option<Vec<Object>>,
}

/// What a trigger does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Trigger {
    /// A read now (none runs).
    Read,
    /// Once more after the one running.
    Again,
}

/// What follows a read's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum After {
    /// A read now: a trigger came while it ran.
    Read,
    /// The one retry of a failed read, after [`RETRY_MS`].
    Retry,
}

/// The router's song map.
#[derive(Debug, Default)]
pub(super) struct SongMapIo {
    /// The newest map of each set.
    pub maps: BTreeMap<String, Read>,
    /// The read in flight of each set, by its number.
    pub reading: BTreeMap<String, u64>,
    /// The task of each set's read in flight: a disconnect aborts it, so
    /// its remaining commands never reach a set loaded meanwhile (#140
    /// review M4).
    pub tasks: BTreeMap<String, tokio::task::JoinHandle<()>>,
    /// The sets that read again after the read in flight.
    pub dirty: BTreeSet<String>,
    /// The sets whose map is not current: from before a disconnect, or its
    /// last read failed (#140 review M3).
    pub stale: BTreeSet<String>,
    /// Each set's retry waiting for [`RETRY_MS`], by the failed read's
    /// number (a read started, a disconnect or a later failure voids it).
    pub retry: BTreeMap<String, u64>,
    /// The sets whose read in flight is a retry: its failure is not tried
    /// again.
    pub retrying: BTreeSet<String>,
    /// The reads' own counter.
    pub reads: u64,
    /// Each subscription (hub key) and its set.
    pub subs: BTreeMap<String, String>,
    /// Each set's Sections clips subscription (hub key) and its path.
    pub clips: BTreeMap<String, (String, String)>,
    /// The clips subscriptions a read just made, with the clips' names it
    /// read: a first value naming the same clips is that read's own.
    pub fresh: BTreeMap<String, Vec<String>>,
    /// The problems of each set's last read since its connect: a read's
    /// record is warn class only when its own differ (#140 review M8).
    pub last_problems: BTreeMap<String, Vec<Problem>>,
}

impl SongMapIo {
    /// Each set's map summed up, for `/api/status`.
    pub fn status(&self) -> Vec<SongMapStatus> {
        self.maps
            .iter()
            .map(|(instance, read)| status_of(instance, read, self.stale.contains(instance)))
            .collect()
    }
}

/// A set's map in `/api/status`.
pub(super) fn status_of(instance: &str, read: &Read, stale: bool) -> SongMapStatus {
    let n = counts(&read.map);
    SongMapStatus {
        instance: instance.to_string(),
        songs: n.songs,
        keyed: n.keyed,
        changes: n.changes,
        problems: n.problems,
        sections: read.sections,
        at_ms: read.at_ms,
        stale,
    }
}

/// Why `GET /api/songmap` has no map to answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Missing {
    /// No Live set of that name.
    Unknown,
    /// The set's map was not read yet.
    NotRead,
}

/// `GET /api/songmap`'s answer for a set: unknown, not read yet, or its
/// newest map with every span and every problem in words.
pub(super) fn report_of(
    instance: &str,
    known: bool,
    read: Option<&Read>,
    stale: bool,
) -> Result<SongMapReport, Missing> {
    if !known {
        return Err(Missing::Unknown);
    }
    let read = read.ok_or(Missing::NotRead)?;
    Ok(SongMapReport {
        instance: instance.to_string(),
        read_no: read.read_no,
        at_ms: read.at_ms,
        stale,
        sections: read.sections,
        counts: counts(&read.map),
        spans: read.map.spans.iter().map(SpanReport::from).collect(),
        problems: read
            .map
            .problems
            .iter()
            .map(|p| ProblemReport {
                kind: p.kind,
                song: p.song,
                time: p.time,
                text: p.text(),
            })
            .collect(),
    })
}

/// Whether a list value names exactly `names`, in order (a listener's
/// value encodes each object with its name).
pub(super) fn same_names(value: Option<&Value>, names: &[String]) -> bool {
    value.and_then(Value::as_array).is_some_and(|list| {
        list.len() == names.len()
            && list
                .iter()
                .zip(names)
                .all(|(item, name)| item.get("name").and_then(Value::as_str) == Some(name.as_str()))
    })
}

/// What follows a read's answer (#140 review M3): a read now when a trigger
/// came while it ran (`dirty`), else its one retry when it failed and was
/// no retry itself (`retried`), else nothing.
pub(super) fn after_read(failed: bool, retried: bool, dirty: bool) -> Option<After> {
    match (dirty, failed && !retried) {
        (true, _) => Some(After::Read),
        (false, true) => Some(After::Retry),
        (false, false) => None,
    }
}

/// What a trigger does with a read running (`reading`) and one more already
/// asked for (`dirty`): a read now, once more after it, or nothing.
pub(super) fn next_read(reading: bool, dirty: bool) -> Option<Trigger> {
    match (reading, dirty) {
        (false, _) => Some(Trigger::Read),
        (true, false) => Some(Trigger::Again),
        (true, true) => None,
    }
}

/// Whether a transport value after `prev` reads the map again: a change
/// does, a session's first value (none before it) does not, since the
/// connect's own read covers it.
pub(super) fn transport_reads(prev: Option<bool>) -> bool {
    prev.is_some()
}

/// Which Sections track a read takes, by how many tracks carry the name.
pub(super) fn sections_state(tracks_named_sections: usize) -> SectionsState {
    match tracks_named_sections {
        0 => SectionsState::None,
        1 => SectionsState::One,
        n => SectionsState::FirstOf(n),
    }
}

/// A read's first call: the locators, the Sections track's clips by its
/// name, and the tracks (whose names count the Sections tracks).
pub(super) fn first_commands() -> Vec<Value> {
    vec![
        get("live_set", CUE_POINTS),
        get(SECTIONS, CLIPS),
        get("live_set", "tracks"),
    ]
}

/// The objects of a list answer.
fn objects(data: &Value) -> Vec<Object> {
    items(data)
        .into_iter()
        .map(|i| Object {
            path: i.path.to_string(),
            name: i.name.to_string(),
            id: i.id.to_string(),
            class: i.class.to_string(),
        })
        .collect()
}

/// A `get_prop` of `prop` on `object` through its `$ref` and class: the
/// same object wherever it moved (the script refuses the ref of a deleted
/// object, or of another class, as `StaleRef`).
pub(super) fn by_ref(object: &Object, prop: &str) -> Value {
    json!({
        "target": {"$ref": object.id, "class": object.class},
        "name": "get_prop",
        "args": {"prop": prop},
    })
}

/// A slot's list, or why it has none (`what` names it).
fn list_of(slot: Option<&Value>, what: &str) -> Result<Vec<Object>, String> {
    match slot.and_then(answered) {
        Some(data) => Ok(objects(data)),
        None => Err(format!(
            "{what}: {}",
            refused(slot).unwrap_or_else(|| "no list".to_string())
        )),
    }
}

/// The first call's answer: the locators (required), the tracks named
/// `Sections` (the tracks required), and the clips when the name binding
/// answered.
pub(super) fn listed(slots: &[Value]) -> Result<Listed, String> {
    let cues = list_of(slots.first(), CUE_POINTS)?;
    let tracks = list_of(slots.get(2), "tracks")?;
    let named: Vec<&Object> = tracks
        .iter()
        .filter(|track| track.name == SECTIONS_NAME)
        .collect();
    Ok(Listed {
        cues,
        sections: sections_state(named.len()),
        track: named.first().map(|track| track.path.clone()),
        clips: slots.get(1).and_then(answered).map(objects),
    })
}

/// The second call's answer, the clips of the first track named `Sections`
/// read by its index (`track`): its clips, followed at that path; or, when
/// they cannot be read (a group track of that name, #140 review M9), none,
/// `SectionsState::Unreadable` and nothing to follow, so the spans still
/// come from the locators.
pub(super) fn clips_by_index(
    slot: Option<&Value>,
    sections: SectionsState,
    track: String,
) -> (Vec<Object>, SectionsState, Option<String>) {
    match slot.and_then(answered) {
        Some(data) => (objects(data), sections, Some(track)),
        None => (Vec::new(), SectionsState::Unreadable, None),
    }
}

/// A read's map: the parser's, with a problem when the Sections track's
/// clips could not be read (#140 review M9).
pub(super) fn map_of(listing: &Listing) -> SongMap {
    let mut map = build(&listing.cues, &listing.clips);
    if listing.sections == SectionsState::Unreadable {
        map.problems.push(Problem {
            kind: ProblemKind::SectionsUnreadable,
            song: None,
            time: None,
            clip: None,
        });
    }
    map
}

/// The `get_prop prop` of each object `wanted` takes (by its name), through
/// its `$ref`, in calls of at most `TIME_CHUNK` commands.
fn chunked(objects: &[Object], wanted: impl Fn(&str) -> bool, prop: &str) -> Vec<Vec<Value>> {
    let commands: Vec<Value> = objects
        .iter()
        .filter(|object| wanted(&object.name))
        .map(|object| by_ref(object, prop))
        .collect();
    commands.chunks(TIME_CHUNK).map(<[Value]>::to_vec).collect()
}

/// The `time` reads of the locators that need one (neither ignored nor a
/// section), in calls of at most `TIME_CHUNK`.
pub(super) fn time_commands(cues: &[Object]) -> Vec<Vec<Value>> {
    chunked(cues, |name| needs_time(&classify(name)), "time")
}

/// The `start_time` reads of the clips whose name holds `{`, in calls of at
/// most `TIME_CHUNK`.
pub(super) fn clip_commands(clips: &[Object]) -> Vec<Vec<Value>> {
    chunked(clips, clip_needs_time, "start_time")
}

/// Each object's name with its time from `slots` (in order, one per object
/// `wanted` takes; none for the others), or the first that did not read.
fn timed(
    objects: &[Object],
    wanted: impl Fn(&str) -> bool,
    slots: &[Value],
    prop: &str,
) -> Result<Vec<(String, Option<f64>)>, String> {
    let mut slots = slots.iter();
    objects
        .iter()
        .map(|object| {
            if !wanted(&object.name) {
                return Ok((object.name.clone(), None));
            }
            let slot = slots.next();
            match slot.and_then(answered).and_then(Value::as_f64) {
                Some(time) => Ok((object.name.clone(), Some(time))),
                None => Err(format!(
                    "{} {prop}: {}",
                    object.path,
                    refused(slot).unwrap_or_else(|| "not a number".to_string())
                )),
            }
        })
        .collect()
}

/// The locators with their times read.
pub(super) fn cues_of(cues: &[Object], slots: &[Value]) -> Result<Vec<Cue>, String> {
    let timed = timed(cues, |name| needs_time(&classify(name)), slots, "time")?;
    Ok(timed
        .into_iter()
        .map(|(name, time)| Cue { name, time })
        .collect())
}

/// The clips with their starts read.
pub(super) fn clips_of(clips: &[Object], slots: &[Value]) -> Result<Vec<ClipName>, String> {
    let timed = timed(clips, clip_needs_time, slots, "start_time")?;
    Ok(timed
        .into_iter()
        .map(|(name, start)| ClipName { name, start })
        .collect())
}

/// Whether a read's problems differ from the set's last read since its
/// connect (`last`; none: the first read, which differs when it has any):
/// only then is its record warn class, so a known problem left as it was
/// stays bulk at every transport change (#140 review M8).
pub(super) fn problems_changed(last: Option<&[Problem]>, now: &[Problem]) -> bool {
    last.map_or(!now.is_empty(), |last| last != now)
}

/// The `songmap` record of a read: its counts, which Sections track, how
/// long it took (`ms`), its first problems in words, and whether they
/// changed since the set's last read (`problems_changed`, its class).
pub(super) fn songmap_fields(
    instance: &str,
    read: &Read,
    ms: u64,
    problems_changed: bool,
) -> Value {
    let n = counts(&read.map);
    let texts: Vec<String> = read
        .map
        .problems
        .iter()
        .take(MAX_TEXTS)
        .map(|p| p.text())
        .collect();
    json!({
        "instance": instance,
        "read_no": read.read_no,
        "songs": n.songs,
        "keyed": n.keyed,
        "changes": n.changes,
        "problems": n.problems,
        "sections": read.sections,
        "ms": ms,
        "problem_texts": texts,
        "problems_changed": problems_changed,
    })
}

/// Whether a read's record is warn class, judged on the record as it is
/// written (`events::stamp` gives it the `ev` that `events::is_warn` keys
/// on): the read's log line takes that level (#140 review M1).
pub(super) fn songmap_warn(fields: &Value) -> bool {
    crate::events::is_warn(&crate::events::stamp("songmap", 0, fields.clone()))
}

/// The words of a read's log line: a failed read's, a read whose problems
/// changed, or a plain one.
pub(super) fn read_words(fields: &Value) -> &'static str {
    if fields.get("error").is_some() {
        "the song map could not be read"
    } else if fields.get("problems_changed") == Some(&Value::Bool(true)) {
        "the song map was read, its problems changed"
    } else {
        "the song map was read"
    }
}

/// The `songmap` record of a read that failed: whether the set's kept map
/// is stale now (`stale`; none kept: false) and whether the read is tried
/// again after [`RETRY_MS`] (`retry`).
pub(super) fn songmap_error_fields(
    instance: &str,
    read_no: u64,
    error: &str,
    ms: u64,
    stale: bool,
    retry: bool,
) -> Value {
    json!({"instance": instance, "read_no": read_no, "ms": ms, "error": error,
           "stale": stale, "retry": retry})
}

/// One read call.
async fn call(live: &LiveHandle, commands: Vec<Value>) -> Result<Vec<Value>, String> {
    live.call(commands).await.map_err(|e| e.to_string())
}

/// Reads a set's locators and Sections clips (see the module's doc).
async fn read_songmap(live: LiveHandle) -> Result<Listing, String> {
    let first = call(&live, first_commands()).await?;
    let listed = listed(&first)?;
    let (clips, sections, track) = match (listed.clips, listed.track) {
        (Some(clips), track) => (clips, listed.sections, track),
        (None, Some(track)) => {
            let slots = call(&live, vec![get(&track, CLIPS)]).await?;
            clips_by_index(slots.first(), listed.sections, track)
        }
        (None, None) => (Vec::new(), listed.sections, None),
    };
    let mut times = Vec::new();
    for chunk in time_commands(&listed.cues) {
        times.extend(call(&live, chunk).await?);
    }
    let mut starts = Vec::new();
    for chunk in clip_commands(&clips) {
        starts.extend(call(&live, chunk).await?);
    }
    Ok(Listing {
        cues: cues_of(&listed.cues, &times)?,
        clips: clips_of(&clips, &starts)?,
        sections,
        track,
    })
}

impl Router {
    /// `instance` connected: its lists are followed (a subscription the
    /// table already holds, from an earlier connect, stays as it is), and
    /// their first values read the map.
    pub(super) fn songmap_connected(&mut self, instance: &str) {
        for (target, prop) in [("live_set", CUE_POINTS), ("live_set", "tracks")] {
            self.songmap_follow(instance, target, prop);
        }
    }

    /// Subscribes `target`'s `prop` for the song map: its hub key and
    /// whether the table already held a value of it (then no first value
    /// comes), if it is one.
    fn songmap_follow(
        &mut self,
        instance: &str,
        target: &str,
        prop: &str,
    ) -> Option<(String, bool)> {
        match self
            .subs
            .subscribe(SONGMAP_CLIENT, instance, target, prop, false)
        {
            Ok(reply) => {
                self.songmap
                    .subs
                    .insert(reply.key.clone(), instance.to_string());
                Some((reply.key, reply.cached.is_some()))
            }
            Err((_, error)) => {
                tracing::warn!(instance = %instance, target = %target, prop = %prop, error = %error, "the song map cannot follow a list");
                None
            }
        }
    }

    /// Follows the clips of the Sections track a read found (by its index
    /// path), or none; a first value naming the clips the read saw
    /// (`names`) is the read's own (none comes when the table already held
    /// one: a page follows the same clips).
    fn songmap_follow_clips(&mut self, instance: &str, track: Option<&str>, names: Vec<String>) {
        let wanted = track.map(|t| format!("{t} {CLIPS}"));
        let current = self
            .songmap
            .clips
            .get(instance)
            .map(|(_, path)| path.clone());
        if wanted == current {
            return;
        }
        if let Some((key, _)) = self.songmap.clips.remove(instance) {
            self.subs.unsubscribe(SONGMAP_CLIENT, &key);
            self.songmap.subs.remove(&key);
            self.songmap.fresh.remove(&key);
        }
        let Some(track) = track else {
            return;
        };
        if let Some((key, cached)) = self.songmap_follow(instance, track, CLIPS) {
            if !cached {
                self.songmap.fresh.insert(key.clone(), names);
            }
            self.songmap
                .clips
                .insert(instance.to_string(), (key, format!("{track} {CLIPS}")));
        }
    }

    /// The deliveries of the song map's subscriptions: each a trigger of
    /// its set, once per set (the first value of a clips subscription a read
    /// just made is that read's own when it names the clips the read saw: a
    /// clip added meanwhile still reads).
    pub(super) fn songmap_delivery(&mut self, items: Vec<ValueItem>) {
        let mut sets = BTreeSet::new();
        for item in items {
            let Some(instance) = self.songmap.subs.get(&item.sub).cloned() else {
                continue;
            };
            let read = self.songmap.fresh.remove(&item.sub);
            if read.is_some_and(|names| same_names(item.value.as_ref(), &names)) {
                continue;
            }
            sets.insert(instance);
        }
        for instance in sets {
            self.songmap_trigger(&instance);
        }
    }

    /// A transport change of `instance` (`prev`: the transport before it)
    /// reads its map again; a session's first value does not (its connect
    /// reads).
    pub(super) fn songmap_transport(&mut self, instance: &str, prev: Option<bool>) {
        if transport_reads(prev) {
            self.songmap_trigger(instance);
        }
    }

    /// A trigger: a read now, once more after the running one, or nothing.
    pub(super) fn songmap_trigger(&mut self, instance: &str) {
        let reading = self.songmap.reading.contains_key(instance);
        let dirty = self.songmap.dirty.contains(instance);
        match next_read(reading, dirty) {
            Some(Trigger::Read) => self.songmap_read_start(instance, false),
            Some(Trigger::Again) => {
                self.songmap.dirty.insert(instance.to_string());
            }
            None => {}
        }
    }

    /// Starts a read of `instance` while it is online (`retry`: the one
    /// retry of a failed read, whose own failure is not tried again; a
    /// retry still waiting is void then); its answer comes back as
    /// [`RouterMsg::SongMapRead`].
    fn songmap_read_start(&mut self, instance: &str, retry: bool) {
        let online = self.states.get(instance).is_some_and(|s| s.online);
        let (true, Some(live)) = (online, self.live.get(instance)) else {
            return;
        };
        let live = live.clone();
        self.songmap.retry.remove(instance);
        if retry {
            self.songmap.retrying.insert(instance.to_string());
        }
        self.songmap.reads += 1;
        let read_no = self.songmap.reads;
        self.songmap.reading.insert(instance.to_string(), read_no);
        let key = instance.to_string();
        let instance = instance.to_string();
        let tx = self.io.tx.clone();
        let task = tokio::spawn(async move {
            let started = std::time::Instant::now();
            let result = read_songmap(live).await;
            let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            let _ = tx.send(RouterMsg::SongMapRead {
                instance,
                read_no,
                result,
                ms,
            });
        });
        self.songmap.tasks.insert(key, task);
    }

    /// A read's answer: a void one (a disconnect, an older read) does
    /// nothing; a map replaces the set's last one, an error keeps it, stale
    /// (#140 review M3); both are recorded, then a trigger that came
    /// meanwhile reads again, or a failed read that was no retry is tried
    /// once more after [`RETRY_MS`].
    pub(super) fn songmap_read(
        &mut self,
        instance: &str,
        read_no: u64,
        result: Result<Listing, String>,
        ms: u64,
    ) {
        if self.songmap.reading.get(instance) != Some(&read_no) {
            return;
        }
        self.songmap.reading.remove(instance);
        self.songmap.tasks.remove(instance);
        let retried = self.songmap.retrying.remove(instance);
        let dirty = self.songmap.dirty.remove(instance);
        let after = after_read(result.is_err(), retried, dirty);
        let fields = match result {
            Ok(listing) => {
                let read = Read {
                    map: map_of(&listing),
                    sections: listing.sections,
                    at_ms: u64::try_from(crate::events::now_ms()).unwrap_or(0),
                    read_no,
                };
                let last = self.songmap.last_problems.get(instance);
                let changed = problems_changed(last.map(Vec::as_slice), &read.map.problems);
                let fields = songmap_fields(instance, &read, ms, changed);
                self.songmap
                    .last_problems
                    .insert(instance.to_string(), read.map.problems.clone());
                self.songmap.maps.insert(instance.to_string(), read);
                self.songmap.stale.remove(instance);
                let names = listing.clips.iter().map(|c| c.name.clone()).collect();
                self.songmap_follow_clips(instance, listing.track.as_deref(), names);
                fields
            }
            Err(error) => {
                // The kept map is no longer current (#140 review M3).
                let stale = self.songmap.maps.contains_key(instance);
                if stale {
                    self.songmap.stale.insert(instance.to_string());
                }
                let retry = after == Some(After::Retry);
                songmap_error_fields(instance, read_no, &error, ms, stale, retry)
            }
        };
        let words = read_words(&fields);
        if songmap_warn(&fields) {
            tracing::warn!(instance = %instance, fields = %fields, "{words}");
        } else {
            tracing::info!(instance = %instance, fields = %fields, "{words}");
        }
        self.io.events.record("songmap", fields);
        match after {
            Some(After::Read) => self.songmap_read_start(instance, false),
            Some(After::Retry) => self.songmap_retry_later(instance, read_no),
            None => {}
        }
    }

    /// Asks for the one retry of `instance`'s failed read `read_no` after
    /// [`RETRY_MS`] ([`RouterMsg::SongMapRetry`]).
    fn songmap_retry_later(&mut self, instance: &str, read_no: u64) {
        self.songmap.retry.insert(instance.to_string(), read_no);
        let tx = self.io.tx.clone();
        let instance = instance.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(RETRY_MS)).await;
            let _ = tx.send(RouterMsg::SongMapRetry { instance, read_no });
        });
    }

    /// A failed read's retry is due: it reads, unless it is void (a read
    /// started since, a disconnect, or another failed read's retry).
    pub(super) fn songmap_retry(&mut self, instance: &str, read_no: u64) {
        if self.songmap.retry.get(instance) != Some(&read_no) {
            return;
        }
        self.songmap.retry.remove(instance);
        self.songmap_read_start(instance, true);
    }

    /// `instance` went offline: its read in flight is void and its task
    /// aborted (#140 review M4), nothing more is asked of it, and its map is
    /// stale until the next read.
    pub(super) fn songmap_offline(&mut self, instance: &str) {
        self.songmap.reading.remove(instance);
        if let Some(task) = self.songmap.tasks.remove(instance) {
            task.abort();
        }
        self.songmap.dirty.remove(instance);
        self.songmap.retry.remove(instance);
        self.songmap.retrying.remove(instance);
        // The first read after the next connect is judged as a first one.
        self.songmap.last_problems.remove(instance);
        if let Some((key, _)) = self.songmap.clips.get(instance) {
            self.songmap.fresh.remove(key);
        }
        if self.songmap.maps.contains_key(instance) {
            self.songmap.stale.insert(instance.to_string());
        }
    }

    /// `GET /api/songmap`'s answer for `instance`.
    pub(super) fn songmap_report(&self, instance: &str) -> Result<SongMapReport, Missing> {
        report_of(
            instance,
            self.live.contains_key(instance),
            self.songmap.maps.get(instance),
            self.songmap.stale.contains(instance),
        )
    }
}

#[cfg(test)]
#[path = "songmap/tests.rs"]
mod tests;
