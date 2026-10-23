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
//! in calls of at most `TIME_CHUNK` commands, one after another. A failed
//! call ends the read with its error and keeps the old map; a disconnect
//! voids the read in flight (its answer's number is no longer the set's)
//! and keeps the last map, stale until the next read.

use std::collections::{BTreeMap, BTreeSet};

use fohmixer_proto::client::{ProblemReport, SongMapReport, SongMapStatus, ValueItem};
use fohmixer_proto::songmap::{
    ClipName, Cue, SectionsState, SongMap, TIME_CHUNK, build, classify, clip_needs_time, counts,
    needs_time,
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

/// What the first call of a read found: the locators and the Sections
/// tracks, with their paths and names.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Listed {
    pub cues: Vec<(String, String)>,
    pub sections: SectionsState,
    pub track: Option<String>,
    /// The Sections clips when the name binding answered (one such track);
    /// none: read them at `track`, if any.
    pub clips: Option<Vec<(String, String)>>,
}

/// What a trigger does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Trigger {
    /// A read now (none runs).
    Read,
    /// Once more after the one running.
    Again,
}

/// The router's song map.
#[derive(Debug, Default)]
pub(super) struct SongMapIo {
    /// The newest map of each set.
    pub maps: BTreeMap<String, Read>,
    /// The read in flight of each set, by its number.
    pub reading: BTreeMap<String, u64>,
    /// The sets that read again after the read in flight.
    pub dirty: BTreeSet<String>,
    /// The sets whose map is from before a disconnect.
    pub stale: BTreeSet<String>,
    /// The reads' own counter.
    pub reads: u64,
    /// Each subscription (hub key) and its set.
    pub subs: BTreeMap<String, String>,
    /// Each set's Sections clips subscription (hub key) and its path.
    pub clips: BTreeMap<String, (String, String)>,
    /// The clips subscriptions a read just made, with the clips' names it
    /// read: a first value naming the same clips is that read's own.
    pub fresh: BTreeMap<String, Vec<String>>,
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
        spans: read.map.spans.clone(),
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

/// The `(path, name)` of each object of a list answer.
fn pairs(data: &Value) -> Vec<(String, String)> {
    items(data)
        .into_iter()
        .map(|i| (i.path.to_string(), i.name.to_string()))
        .collect()
}

/// A slot's list, or why it has none (`what` names it).
fn list_of(slot: Option<&Value>, what: &str) -> Result<Vec<(String, String)>, String> {
    match slot.and_then(answered) {
        Some(data) => Ok(pairs(data)),
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
    let named: Vec<&(String, String)> = tracks
        .iter()
        .filter(|(_, name)| name == SECTIONS_NAME)
        .collect();
    Ok(Listed {
        cues,
        sections: sections_state(named.len()),
        track: named.first().map(|(path, _)| path.clone()),
        clips: slots.get(1).and_then(answered).map(pairs),
    })
}

/// The `get_prop prop` of each pair `wanted` takes, in calls of at most
/// `TIME_CHUNK` commands.
fn chunked(
    pairs: &[(String, String)],
    wanted: impl Fn(&str) -> bool,
    prop: &str,
) -> Vec<Vec<Value>> {
    let commands: Vec<Value> = pairs
        .iter()
        .filter(|(_, name)| wanted(name))
        .map(|(path, _)| get(path, prop))
        .collect();
    commands.chunks(TIME_CHUNK).map(<[Value]>::to_vec).collect()
}

/// The `time` reads of the locators that need one (neither ignored nor a
/// section), in calls of at most `TIME_CHUNK`.
pub(super) fn time_commands(cues: &[(String, String)]) -> Vec<Vec<Value>> {
    chunked(cues, |name| needs_time(&classify(name)), "time")
}

/// The `start_time` reads of the clips whose name holds `{`, in calls of at
/// most `TIME_CHUNK`.
pub(super) fn clip_commands(clips: &[(String, String)]) -> Vec<Vec<Value>> {
    chunked(clips, clip_needs_time, "start_time")
}

/// Each pair's name with its time from `slots` (in order, one per pair
/// `wanted` takes; none for the others), or the first that did not read.
fn timed(
    pairs: &[(String, String)],
    wanted: impl Fn(&str) -> bool,
    slots: &[Value],
    prop: &str,
) -> Result<Vec<(String, Option<f64>)>, String> {
    let mut slots = slots.iter();
    pairs
        .iter()
        .map(|(path, name)| {
            if !wanted(name) {
                return Ok((name.clone(), None));
            }
            let slot = slots.next();
            match slot.and_then(answered).and_then(Value::as_f64) {
                Some(time) => Ok((name.clone(), Some(time))),
                None => Err(format!(
                    "{path} {prop}: {}",
                    refused(slot).unwrap_or_else(|| "not a number".to_string())
                )),
            }
        })
        .collect()
}

/// The locators with their times read.
pub(super) fn cues_of(cues: &[(String, String)], slots: &[Value]) -> Result<Vec<Cue>, String> {
    let timed = timed(cues, |name| needs_time(&classify(name)), slots, "time")?;
    Ok(timed
        .into_iter()
        .map(|(name, time)| Cue { name, time })
        .collect())
}

/// The clips with their starts read.
pub(super) fn clips_of(
    clips: &[(String, String)],
    slots: &[Value],
) -> Result<Vec<ClipName>, String> {
    let timed = timed(clips, clip_needs_time, slots, "start_time")?;
    Ok(timed
        .into_iter()
        .map(|(name, start)| ClipName { name, start })
        .collect())
}

/// The `songmap` record of a read: its counts, which Sections track, how
/// long it took (`ms`) and its first problems in words.
pub(super) fn songmap_fields(instance: &str, read: &Read, ms: u64) -> Value {
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
    })
}

/// Whether a read's record is warn class, judged on the record as it is
/// written (`events::stamp` gives it the `ev` that `events::is_warn` keys
/// on): the read's log line takes that level (#140 review M1).
pub(super) fn songmap_warn(fields: &Value) -> bool {
    crate::events::is_warn(&crate::events::stamp("songmap", 0, fields.clone()))
}

/// The `songmap` record of a read that failed.
pub(super) fn songmap_error_fields(instance: &str, read_no: u64, error: &str, ms: u64) -> Value {
    json!({"instance": instance, "read_no": read_no, "ms": ms, "error": error})
}

/// One read call.
async fn call(live: &LiveHandle, commands: Vec<Value>) -> Result<Vec<Value>, String> {
    live.call(commands).await.map_err(|e| e.to_string())
}

/// Reads a set's locators and Sections clips (see the module's doc).
async fn read_songmap(live: LiveHandle) -> Result<Listing, String> {
    let first = call(&live, first_commands()).await?;
    let listed = listed(&first)?;
    let clips = match (listed.clips, &listed.track) {
        (Some(clips), _) => clips,
        (None, Some(track)) => {
            let slots = call(&live, vec![get(track, CLIPS)]).await?;
            list_of(slots.first(), CLIPS)?
        }
        (None, None) => Vec::new(),
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
        sections: listed.sections,
        track: listed.track,
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
            Some(Trigger::Read) => self.songmap_read_start(instance),
            Some(Trigger::Again) => {
                self.songmap.dirty.insert(instance.to_string());
            }
            None => {}
        }
    }

    /// Starts a read of `instance` while it is online; its answer comes
    /// back as [`RouterMsg::SongMapRead`].
    fn songmap_read_start(&mut self, instance: &str) {
        let online = self.states.get(instance).is_some_and(|s| s.online);
        let (true, Some(live)) = (online, self.live.get(instance)) else {
            return;
        };
        let live = live.clone();
        self.songmap.reads += 1;
        let read_no = self.songmap.reads;
        self.songmap.reading.insert(instance.to_string(), read_no);
        let instance = instance.to_string();
        let tx = self.io.tx.clone();
        tokio::spawn(async move {
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
    }

    /// A read's answer: a void one (a disconnect, an older read) does
    /// nothing; a map replaces the set's last one, an error keeps it; both
    /// are recorded, and a trigger that came meanwhile reads again.
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
        let fields = match result {
            Ok(listing) => {
                let read = Read {
                    map: build(&listing.cues, &listing.clips),
                    sections: listing.sections,
                    at_ms: u64::try_from(crate::events::now_ms()).unwrap_or(0),
                    read_no,
                };
                let fields = songmap_fields(instance, &read, ms);
                self.songmap.maps.insert(instance.to_string(), read);
                self.songmap.stale.remove(instance);
                let names = listing.clips.iter().map(|c| c.name.clone()).collect();
                self.songmap_follow_clips(instance, listing.track.as_deref(), names);
                fields
            }
            Err(error) => songmap_error_fields(instance, read_no, &error, ms),
        };
        if songmap_warn(&fields) {
            tracing::warn!(instance = %instance, fields = %fields, "the song map was read with problems");
        } else {
            tracing::info!(instance = %instance, fields = %fields, "the song map was read");
        }
        self.io.events.record("songmap", fields);
        if self.songmap.dirty.remove(instance) {
            self.songmap_read_start(instance);
        }
    }

    /// `instance` went offline: its read in flight is void, nothing more is
    /// asked of it, and its map is stale until the next read.
    pub(super) fn songmap_offline(&mut self, instance: &str) {
        self.songmap.reading.remove(instance);
        self.songmap.dirty.remove(instance);
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
