//! The router's claps from the surface (#220): the glue of `rules/claps.rs`.
//!
//! - The Claps track is the served layout's `+CLAPS` strip
//!   (`Layout::claps_track`, carried by `RouterMsg::Layout`). Its Live set's
//!   playhead, tempo and time signature (`live_set` `current_song_time`,
//!   `tempo`, `signature_numerator`, `signature_denominator`) and the
//!   track's `arrangement_clips` are followed by the rule's own subscriber
//!   [`CLAPS_CLIENT`] (in the `Status` `own` lists); the transport is the
//!   rules' own (`on_transport`). A layout whose Claps track is the same
//!   track at another index ([`track_change`]) keeps what claps and
//!   follows its clips at the new place; any other change stops it (the
//!   device told `on 0` first).
//! - The clips are read (each one's `start_time`, `end_time` and `muted`,
//!   through its `$ref`, in calls of at most `TIME_CHUNK`) at every value
//!   of the list (its first one after a connect too) and at every transport
//!   change: Live has no list change for a clip switched on or off, so that
//!   shows at the next start or stop, as the song map's names do. A read
//!   that fails is recorded; the next trigger reads again.
//! - A page's `clap_tap` is mapped to the hub's clock by its socket's offset
//!   ([`tap_ms`]), then to the song position ([`position_at`]); its
//!   `clap_beat` and `clap_stop` go to the rule as they are.
//! - The device (`FohMixer Claps`) is told the rule's words over UDP at
//!   every change and again every `SEND_EVERY_MS` (`RouterMsg::ClapsTick`),
//!   while there is a Claps track; each tick also polls it, with or without
//!   a Claps track (#264: the device runs no clock, it answers the poll with
//!   its hello, `RouterMsg::ClapsHello`, `claps_udp.rs`).
//! - Every page gets `ServerMsg::Claps` at attach and on a change of the
//!   phase, the pattern, the tempo or a playhead off the pages' ring
//!   ([`ring_off`]).
//! - Every step is one `claps` record (warn class) and one log line.

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::time::Instant;

use fohmixer_proto::claps::{
    ClapsDevice, ClapsState, ClapsStatus, ClapsWords, FORGET_MS, Hello, Word, encode, words_of,
};
use fohmixer_proto::client::{ServerMsg, ValueItem};
use fohmixer_proto::layout::ClapsTrack;
use fohmixer_proto::songmap::TIME_CHUNK;
use serde_json::{Value, json};

use super::songmap::{Object, by_ref, objects};
use super::{Router, RouterMsg, transport_heard};
use crate::eq::walk::{answered, get, refused};
use crate::live::subs::{Cached, ClientId};
use crate::outbox::Outbox;
use crate::rules::claps::{
    Claps, Clip, Refusal, SILENCE_AHEAD_BEATS, active, covers, four_four, position_at, ring_off,
    signature_text,
};

/// The rule's own subscriber (the playhead, the tempo, the signature, the
/// track's clips).
pub const CLAPS_CLIENT: ClientId = ClientId::MAX - 8;
/// A Live set's playhead in beats.
pub const SONG_TIME: &str = "current_song_time";
/// A Live set's tempo in bpm.
pub const TEMPO: &str = "tempo";
/// A Live set's time signature.
pub const NUMERATOR: &str = "signature_numerator";
pub const DENOMINATOR: &str = "signature_denominator";
/// A track's arrangement clips.
pub const CLIPS: &str = "arrangement_clips";
/// The props a clip is read of, in order.
pub const CLIP_PROPS: [&str; 3] = ["start_time", "end_time", "muted"];

/// What one of the rule's subscriptions follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Part {
    Time,
    Tempo,
    Numerator,
    Denominator,
    Clips,
}

/// The `[claps]` words the router needs: the pitches and where the device
/// listens (0: nothing is sent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClapsSetup {
    pub notes: Vec<u8>,
    pub device_port: u16,
}

/// The hub time of a tap (UTC ms): its page time `t` through the socket's
/// clock offset, never later than its arrival `hub_ms`; the arrival without
/// an offset yet.
pub fn tap_ms(t: f64, hub_ms: f64, offset_ms: Option<f64>) -> f64 {
    offset_ms.map_or(hub_ms, |offset| (t + offset).min(hub_ms))
}

/// What a new layout's Claps track is to the one followed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackChange {
    /// The same track at the same place: nothing to do.
    Same,
    /// The same track (its Live set and label) at another index: what
    /// claps goes on, its clips are followed at the new place.
    Moved,
    /// Another track, a new one or none: the rule starts afresh.
    Afresh,
}

/// How `wanted` follows `current` (a track is known by its Live set and
/// label, as REVERB knows a fader: an index moves with every track
/// inserted above it).
pub fn track_change(current: Option<&ClapsTrack>, wanted: Option<&ClapsTrack>) -> TrackChange {
    match (current, wanted) {
        (None, None) => TrackChange::Same,
        (Some(a), Some(b)) if a == b => TrackChange::Same,
        (Some(a), Some(b)) if a.instance == b.instance && a.label == b.label => TrackChange::Moved,
        _ => TrackChange::Afresh,
    }
}

/// The reads of every clip's start, end and switch, through its `$ref`, in
/// calls of at most `TIME_CHUNK` commands.
pub(super) fn clip_commands(clips: &[Object]) -> Vec<Vec<Value>> {
    let commands: Vec<Value> = clips
        .iter()
        .flat_map(|clip| CLIP_PROPS.iter().map(move |prop| by_ref(clip, prop)))
        .collect();
    commands.chunks(TIME_CHUNK).map(<[Value]>::to_vec).collect()
}

/// A clip's switch: Live answers a bool, the script's codec may give 0/1.
fn switch_of(value: &Value) -> Option<bool> {
    value.as_bool().or_else(|| value.as_f64().map(|n| n != 0.0))
}

/// The clips read (three slots each, in [`CLIP_PROPS`]' order), or the
/// first slot that did not read.
pub(super) fn clips_of(clips: &[Object], slots: &[Value]) -> Result<Vec<Clip>, String> {
    if slots.len() != clips.len() * CLIP_PROPS.len() {
        return Err(format!("{} answers for {} clips", slots.len(), clips.len()));
    }
    clips
        .iter()
        .zip(slots.chunks(CLIP_PROPS.len()))
        .map(|(clip, three)| {
            let bad = |i: usize| {
                format!(
                    "{} {}: {}",
                    clip.path,
                    CLIP_PROPS[i],
                    refused(three.get(i)).unwrap_or_else(|| "no value".to_string())
                )
            };
            let start = answered(&three[0])
                .and_then(Value::as_f64)
                .ok_or_else(|| bad(0))?;
            let end = answered(&three[1])
                .and_then(Value::as_f64)
                .ok_or_else(|| bad(1))?;
            let muted = answered(&three[2])
                .and_then(switch_of)
                .ok_or_else(|| bad(2))?;
            Ok(Clip { start, end, muted })
        })
        .collect()
}

/// One read call.
async fn call(
    live: &crate::live::client::LiveHandle,
    commands: Vec<Value>,
) -> Result<Vec<Value>, String> {
    live.call(commands).await.map_err(|e| e.to_string())
}

/// Reads a track's clips: their list, then each one's start, end and
/// switch.
async fn read_clips(
    live: crate::live::client::LiveHandle,
    track: String,
) -> Result<Vec<Clip>, String> {
    let first = call(&live, vec![get(&track, CLIPS)]).await?;
    let slot = first.first();
    let list = slot.and_then(answered).ok_or_else(|| {
        format!(
            "{track} {CLIPS}: {}",
            refused(slot).unwrap_or_else(|| "no list".to_string())
        )
    })?;
    let clips = objects(list);
    let mut slots = Vec::new();
    for chunk in clip_commands(&clips) {
        slots.extend(call(&live, chunk).await?);
    }
    clips_of(&clips, &slots)
}

/// A record's step in its log line's words.
pub fn step_words(fields: &Value) -> &'static str {
    match fields.get("step").and_then(Value::as_str) {
        Some("tap") => "claps: a tap",
        Some("refused") => "claps: a tap or a dot refused",
        Some("beat") => "claps: a beat dot switched",
        Some("stop") => "claps: stopped from a page",
        Some("pattern") => "claps: the device's pattern",
        Some("on") => "claps: the device switched",
        Some("silent") => "claps: a clip covers the playhead (the device is silent) or no more",
        Some("track") => "claps: the Claps track",
        Some("clips") => "claps: the Claps track's clips read",
        Some("device") => "claps: the device",
        Some("listen") => "claps: listening for the device",
        _ => "claps",
    }
}

/// Whether a record's line is a warning: a refusal, a failure, a device
/// gone.
pub fn step_warn(fields: &Value) -> bool {
    fields.get("error").is_some_and(|e| !e.is_null())
        || matches!(fields.get("step").and_then(Value::as_str), Some("refused"))
        || fields.get("gone") == Some(&json!(true))
}

/// The router's claps.
#[derive(Debug, Default)]
pub(super) struct ClapsIo {
    pub rule: Claps,
    /// The Claps track of the served layout.
    pub track: Option<ClapsTrack>,
    /// The rule's subscriptions: hub key → what it follows.
    pub subs: BTreeMap<String, Part>,
    /// The playhead as last heard and when (hub UTC ms).
    pub time: Option<f64>,
    pub heard_ms: f64,
    pub tempo: Option<f64>,
    pub numerator: Option<f64>,
    pub denominator: Option<f64>,
    /// The track's clips as last read, and why the last read failed.
    pub clips: Vec<Clip>,
    pub clips_error: Option<String>,
    /// The Claps track's clips could not be followed: unknown clips keep
    /// the device silent (never clap over a clip nobody can see).
    pub unfollowed: bool,
    /// The reads' counter, the one on its way, and whether another is owed
    /// after it.
    pub reads: u64,
    pub reading: Option<u64>,
    pub dirty: bool,
    /// Whether an active clip covers the playhead.
    pub silent: bool,
    pub notes: Vec<u8>,
    /// The state the pages were told, and its playhead (beats, hub ms,
    /// tempo).
    pub told: Option<ClapsState>,
    pub told_ring: Option<(f64, f64, f64)>,
    /// The words last sent to the device.
    pub sent: Option<ClapsWords>,
    pub sender: Option<(UdpSocket, SocketAddr)>,
    pub datagrams: u64,
    /// The polls sent (#264).
    pub polls: u64,
    /// The device's last hello and when.
    pub device: Option<(Hello, Instant)>,
    pub taps: u64,
    pub refused: u64,
}

impl ClapsIo {
    /// The words the device gets now.
    pub fn words(&self) -> ClapsWords {
        self.rule.words(self.silent, &self.notes)
    }

    /// The state the pages get now: Live's playhead only while it plays.
    pub fn state(&self, playing: bool) -> ClapsState {
        ClapsState {
            state: self.rule.phase(self.silent),
            pattern: self.rule.shown(),
            beat: self.time.filter(|_| playing),
            tempo: self.tempo,
        }
    }
}

impl Router {
    /// The rule's pitches and the device's port (`[claps]`): with a port, a
    /// socket of its own on 127.0.0.1 sends to it.
    pub fn with_claps(mut self, setup: ClapsSetup) -> Self {
        self.claps.notes = setup.notes;
        if setup.device_port != 0 {
            let to = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, setup.device_port));
            match UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)) {
                Ok(socket) => {
                    if let Err(e) = socket.set_nonblocking(true) {
                        tracing::warn!(error = %e, "claps: the device's socket stays blocking");
                    }
                    self.claps.sender = Some((socket, to));
                }
                Err(e) => {
                    tracing::error!(error = %e, "claps: no socket for the device; nothing claps from the surface");
                }
            }
        }
        self
    }

    /// One `claps` record and its log line.
    fn claps_record(&self, mut fields: Value, client: Option<ClientId>) {
        if let Some(client) = client {
            fields["client"] = json!(client);
            fields["peer"] = json!(self.peers.get(&client));
        }
        if step_warn(&fields) {
            tracing::warn!(fields = %fields, "{}", step_words(&fields));
        } else {
            tracing::info!(fields = %fields, "{}", step_words(&fields));
        }
        self.io.events.record("claps", fields);
    }

    /// A new layout's Claps track: the same one keeps everything; another
    /// (or none) stops what claps, lets go of the old subscriptions and
    /// follows the new track's set.
    pub(super) fn claps_layout(&mut self, track: Option<ClapsTrack>) {
        match track_change(self.claps.track.as_ref(), track.as_ref()) {
            TrackChange::Same => return,
            TrackChange::Moved => {
                if let Some(track) = track {
                    self.claps_moved(track);
                }
                return;
            }
            TrackChange::Afresh => {}
        }
        let from = self
            .claps
            .track
            .as_ref()
            .map(|t| format!("{} {}", t.instance, t.label));
        let to = track
            .as_ref()
            .map(|t| format!("{} {}", t.instance, t.label));
        self.claps_record(json!({"step": "track", "from": from, "to": to, "track": track.as_ref().map(|t| &t.track)}), None);
        for key in std::mem::take(&mut self.claps.subs).into_keys() {
            self.subs.unsubscribe(CLAPS_CLIENT, &key);
        }
        self.claps.rule.stop();
        // The old track's device is told to stop while it is still the
        // track (`claps_send` sends nothing without one).
        self.claps_send(false);
        let same_set =
            matches!((&self.claps.track, &track), (Some(a), Some(b)) if a.instance == b.instance);
        if !same_set {
            self.claps.time = None;
            self.claps.tempo = None;
            self.claps.numerator = None;
            self.claps.denominator = None;
            self.claps.rule.offline();
        }
        self.claps.clips.clear();
        self.claps.clips_error = None;
        self.claps.unfollowed = false;
        self.claps.reading = None;
        self.claps.dirty = false;
        self.claps.track = track;
        let Some(track) = self.claps.track.clone() else {
            self.claps_changed(None);
            return;
        };
        let playing = self.transport_now(&track.instance);
        self.claps.rule.transport(playing);
        let wanted = [
            ("live_set", SONG_TIME, Part::Time),
            ("live_set", TEMPO, Part::Tempo),
            ("live_set", NUMERATOR, Part::Numerator),
            ("live_set", DENOMINATOR, Part::Denominator),
            (track.track.as_str(), CLIPS, Part::Clips),
        ];
        let mut cached = Vec::new();
        for (target, prop, part) in wanted {
            match self
                .subs
                .subscribe(CLAPS_CLIENT, &track.instance, target, prop, false)
            {
                Ok(reply) => {
                    self.claps.subs.insert(reply.key.clone(), part);
                    if let Some(Cached::Value { value, .. }) = reply.cached {
                        cached.push((part, value));
                    }
                }
                Err((_, error)) => {
                    tracing::warn!(instance = %track.instance, target = %target, prop = %prop, error = %error, "claps: cannot follow a value");
                    self.claps.unfollowed |= part == Part::Clips;
                }
            }
        }
        let now = crate::live::wall_ms().unwrap_or(0.0);
        for (part, value) in cached {
            self.claps_part(part, Some(value), now);
        }
        self.claps_changed(None);
    }

    /// The same Claps track at another index (a track inserted or deleted
    /// above it): what claps goes on; its clips are followed afresh, the ones read kept until the new read answers
    /// (an index path is bound to its object when it resolves).
    fn claps_moved(&mut self, track: ClapsTrack) {
        self.claps_record(
            json!({"step": "track", "moved": true,
                   "from": self.claps.track.as_ref().map(|t| &t.track), "to": track.track}),
            None,
        );
        let clips: Vec<String> = self
            .claps
            .subs
            .iter()
            .filter(|(_, part)| **part == Part::Clips)
            .map(|(key, _)| key.clone())
            .collect();
        for key in clips {
            self.claps.subs.remove(&key);
            self.subs.unsubscribe(CLAPS_CLIENT, &key);
        }
        self.claps.reading = None;
        self.claps.dirty = false;
        let follow = self
            .subs
            .subscribe(CLAPS_CLIENT, &track.instance, &track.track, CLIPS, false);
        self.claps.track = Some(track);
        match follow {
            Ok(reply) => {
                self.claps.unfollowed = false;
                self.claps.subs.insert(reply.key, Part::Clips);
                if matches!(reply.cached, Some(Cached::Value { .. })) {
                    self.claps_read();
                }
            }
            Err((_, error)) => {
                tracing::warn!(error = %error, "claps: cannot follow the moved track's clips");
                self.claps.unfollowed = true;
            }
        }
        self.claps_changed(None);
    }

    /// The rule's deliveries of one flush, at hub time `now_ms`.
    pub(super) fn claps_delivery(&mut self, items: Vec<ValueItem>, now_ms: f64) {
        if items.is_empty() {
            return;
        }
        for item in items {
            let Some(part) = self.claps.subs.get(&item.sub).copied() else {
                continue;
            };
            match transport_heard(item.value, item.error) {
                Some(Ok(value)) => self.claps_part(part, Some(value), now_ms),
                Some(Err(error)) => {
                    tracing::warn!(key = %item.sub, error = ?error, "claps: a followed value failed");
                    self.claps_part(part, None, now_ms);
                }
                None => {}
            }
        }
        self.claps_changed(None);
    }

    /// One value heard (none: its listener failed).
    fn claps_part(&mut self, part: Part, value: Option<Value>, now_ms: f64) {
        let number = value.as_ref().and_then(Value::as_f64);
        match part {
            Part::Time => {
                self.claps.time = number;
                self.claps.heard_ms = now_ms;
                if let Some(pos) = number
                    && self.claps_playing()
                {
                    self.claps.rule.position(pos);
                }
            }
            Part::Tempo => self.claps.tempo = number,
            Part::Numerator => self.claps.numerator = number,
            Part::Denominator => self.claps.denominator = number,
            Part::Clips => {
                if value.is_some() {
                    self.claps_read();
                }
            }
        }
        if matches!(part, Part::Numerator | Part::Denominator) {
            self.claps
                .rule
                .signature(four_four(self.claps.numerator, self.claps.denominator));
        }
    }

    /// Whether the Claps track's Live plays.
    fn claps_playing(&self) -> bool {
        self.claps
            .track
            .as_ref()
            .is_some_and(|t| self.transport_now(&t.instance) == Some(true))
    }

    /// A transport change of `instance` (none: its listener failed): the
    /// rule follows its Claps track's set, and the clips are read again.
    pub(super) fn claps_transport(&mut self, instance: &str, playing: Option<bool>) {
        if self
            .claps
            .track
            .as_ref()
            .is_none_or(|t| t.instance != instance)
        {
            return;
        }
        self.claps.rule.transport(playing);
        if playing.is_some() {
            self.claps_read();
        }
        self.claps_changed(None);
    }

    /// `instance` went offline: its values end, a read on its way is void.
    pub(super) fn claps_offline(&mut self, instance: &str) {
        if self
            .claps
            .track
            .as_ref()
            .is_none_or(|t| t.instance != instance)
        {
            return;
        }
        self.claps.time = None;
        self.claps.tempo = None;
        self.claps.numerator = None;
        self.claps.denominator = None;
        self.claps.reading = None;
        self.claps.dirty = false;
        self.claps.rule.offline();
        self.claps_changed(None);
    }

    /// Reads the Claps track's clips (once more after the read on its way).
    fn claps_read(&mut self) {
        let Some(track) = self.claps.track.clone() else {
            return;
        };
        if self.claps.reading.is_some() {
            self.claps.dirty = true;
            return;
        }
        let Some(live) = self.live.get(&track.instance).cloned() else {
            return;
        };
        self.claps.reads += 1;
        let read = self.claps.reads;
        self.claps.reading = Some(read);
        self.claps.dirty = false;
        let tx = self.io.tx.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            let outcome = read_clips(live, track.track).await;
            let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            let _ = tx.send(RouterMsg::ClapsClips { read, outcome, ms });
        });
    }

    /// A read of the clips ended (void unless it is the one on its way).
    pub(super) fn claps_clips(&mut self, read: u64, outcome: Result<Vec<Clip>, String>, ms: u64) {
        if self.claps.reading != Some(read) {
            return;
        }
        self.claps.reading = None;
        match outcome {
            Ok(clips) => {
                let changed = clips != self.claps.clips || self.claps.clips_error.is_some();
                self.claps.clips = clips;
                self.claps.clips_error = None;
                if changed {
                    self.claps_record(
                        json!({"step": "clips", "clips": self.claps.clips.len(),
                               "active": active(&self.claps.clips), "ms": ms}),
                        None,
                    );
                }
            }
            Err(error) => {
                self.claps_record(json!({"step": "clips", "error": error, "ms": ms}), None);
                self.claps.clips_error = Some(error);
            }
        }
        if self.claps.dirty {
            self.claps_read();
        }
        self.claps_changed(None);
    }

    /// A page's tap at page time `t` (`hub_ms` its arrival, `offset_ms`
    /// its socket's clock offset).
    pub(super) fn claps_tap(
        &mut self,
        client: ClientId,
        t: f64,
        hub_ms: f64,
        offset_ms: Option<f64>,
    ) {
        let at = tap_ms(t, hub_ms, offset_ms);
        let outcome = if self.claps.track.is_none() {
            Err(Refusal::NoTrack)
        } else {
            let pos = self
                .claps
                .time
                .and_then(|heard| position_at(heard, self.claps.heard_ms, self.claps.tempo, at));
            self.claps.rule.tap(pos).map(|beat| (beat, pos))
        };
        match outcome {
            Ok((beat, pos)) => {
                self.claps.taps += 1;
                self.claps_record(
                    json!({"step": "tap", "beat": beat + 1, "pos": pos, "late_ms": hub_ms - at}),
                    Some(client),
                );
            }
            Err(why) => {
                self.claps.refused += 1;
                self.claps_record(
                    json!({"step": "refused", "what": "tap", "why": why.words()}),
                    Some(client),
                );
            }
        }
        self.claps_changed(Some(client));
    }

    /// A page's beat dot.
    pub(super) fn claps_beat(&mut self, client: ClientId, beat: u8, on: bool) {
        if self.claps.track.is_none() || usize::from(beat) >= fohmixer_proto::claps::BEATS {
            self.claps.refused += 1;
            let why = if self.claps.track.is_none() {
                Refusal::NoTrack.words()
            } else {
                "no such beat"
            };
            self.claps_record(
                json!({"step": "refused", "what": "beat", "beat": beat, "why": why}),
                Some(client),
            );
        } else if self.claps.rule.toggle(usize::from(beat), on) {
            self.claps_record(
                json!({"step": "beat", "beat": beat + 1, "on": on}),
                Some(client),
            );
        }
        self.claps_changed(Some(client));
    }

    /// A page's stop (TAP held 1 s).
    pub(super) fn claps_stop(&mut self, client: ClientId) {
        let changed = self.claps.rule.stop();
        self.claps_record(json!({"step": "stop", "changed": changed}), Some(client));
        self.claps_changed(Some(client));
    }

    /// After anything changed: the clip silence judged at the playhead
    /// (`SILENCE_AHEAD_BEATS` ahead while Live plays), the device told what changed, the
    /// pages what changed (`asker` is told even when nothing did: its page
    /// shows what the hub holds).
    fn claps_changed(&mut self, asker: Option<ClientId>) {
        let playing = self.claps_playing();
        let ahead = if playing { SILENCE_AHEAD_BEATS } else { 0.0 };
        let silent = self.claps.unfollowed
            || self
                .claps
                .time
                .is_some_and(|pos| covers(&self.claps.clips, pos + ahead));
        if silent != self.claps.silent && self.claps.track.is_some() {
            self.claps_record(json!({"step": "silent", "silent": silent}), None);
        }
        self.claps.silent = silent;
        self.claps_send(false);
        let state = self.claps.state(playing);
        let ring = state
            .beat
            .zip(state.tempo)
            .is_some_and(|(beat, _)| ring_off(self.claps.told_ring, beat, self.claps.heard_ms));
        let news = self.claps.told.as_ref().is_none_or(|told| {
            told.state != state.state
                || told.pattern != state.pattern
                || told.tempo != state.tempo
                || told.beat.is_some() != state.beat.is_some()
        });
        if news || ring {
            self.claps.told_ring = state
                .beat
                .zip(state.tempo)
                .map(|(beat, tempo)| (beat, self.claps.heard_ms, tempo));
            for outbox in self.clients.values() {
                outbox.reply(ServerMsg::Claps(state.clone()));
            }
            self.claps.told = Some(state);
        } else if let Some(outbox) = asker.and_then(|c| self.clients.get(&c)) {
            outbox.reply(ServerMsg::Claps(state));
        }
    }

    /// Sends the device the rule's words: a change is recorded and sent at
    /// once; `again` (the tick) sends them unchanged. Nothing without a
    /// Claps track or a socket.
    fn claps_send(&mut self, again: bool) {
        if self.claps.track.is_none() {
            return;
        }
        let words = self.claps.words();
        let before = self.claps.sent.clone();
        let changed = before.as_ref() != Some(&words);
        if changed {
            let was = before.unwrap_or_default();
            if was.pattern != words.pattern {
                self.claps_record(json!({"step": "pattern", "pattern": words.pattern}), None);
            }
            if was.on != words.on {
                self.claps_record(json!({"step": "on", "on": words.on}), None);
            }
        }
        if !(changed || again) {
            return;
        }
        self.claps.sent = Some(words.clone());
        let Some((socket, to)) = &self.claps.sender else {
            return;
        };
        for word in words_of(&words) {
            match socket.send_to(&encode(&word), to) {
                Ok(_) => self.claps.datagrams += 1,
                Err(e) => tracing::debug!(error = %e, "claps: a datagram to the device failed"),
            }
        }
    }

    /// The tick: the device gets its words again (a reloaded one takes the
    /// state back), then the poll its hello answers (#264), and a device
    /// unheard for `FORGET_MS` is said gone once.
    pub(super) fn claps_tick(&mut self, now: Instant) {
        self.claps_send(true);
        self.claps_poll();
        let gone = self.claps.device.as_ref().is_some_and(|(_, at)| {
            now.saturating_duration_since(*at).as_millis() >= u128::from(FORGET_MS)
        });
        if gone && let Some((hello, _)) = self.claps.device.take() {
            self.claps_record(
                json!({"step": "device", "gone": true, "version": hello.version}),
                None,
            );
        }
    }

    /// The poll (#264): sent with or without a Claps track, so a loaded
    /// device shows in `/api/status` before its track is tagged; nothing
    /// without a socket.
    fn claps_poll(&mut self) {
        let Some((socket, to)) = &self.claps.sender else {
            return;
        };
        match socket.send_to(&encode(&Word::Poll), to) {
            Ok(_) => self.claps.polls += 1,
            Err(e) => tracing::debug!(error = %e, "claps: a poll to the device failed"),
        }
    }

    /// The device's hello: recorded when it is new (first, another
    /// version, or after it was gone).
    pub(super) fn claps_hello(&mut self, hello: Hello, now: Instant) {
        let news = self
            .claps
            .device
            .as_ref()
            .is_none_or(|(was, _)| was.version != hello.version);
        if news {
            self.claps_record(
                json!({"step": "device", "version": hello.version, "running": hello.running}),
                None,
            );
        }
        self.claps.device = Some((hello, now));
    }

    /// The hello listener bound, or why not.
    pub(super) fn claps_listen(&self, addr: &str, error: Option<String>) {
        self.claps_record(
            json!({"step": "listen", "addr": addr, "error": error}),
            None,
        );
    }

    /// A new page is told the claps' state.
    pub(super) fn claps_attach(&self, outbox: &Outbox) {
        outbox.reply(ServerMsg::Claps(self.claps.state(self.claps_playing())));
    }

    /// Claps in `/api/status`.
    pub(super) fn claps_status(&self, now: Instant) -> ClapsStatus {
        ClapsStatus {
            track: self
                .claps
                .track
                .as_ref()
                .map(|t| format!("{} {}", t.instance, t.label)),
            state: self.claps.rule.phase(self.claps.silent),
            words: self.claps.words(),
            device: self.claps.device.as_ref().map(|(hello, at)| ClapsDevice {
                version: hello.version.clone(),
                running: hello.running,
                age_ms: u64::try_from(now.saturating_duration_since(*at).as_millis())
                    .unwrap_or(u64::MAX),
            }),
            taps: self.claps.taps,
            refused: self.claps.refused,
            clips: self.claps.clips.len(),
            active_clips: active(&self.claps.clips),
            clips_error: self.claps.clips_error.clone(),
            signature: signature_text(self.claps.numerator, self.claps.denominator),
            sent: self.claps.datagrams,
            polls: self.claps.polls,
        }
    }
}

#[cfg(test)]
#[path = "claps/tests.rs"]
mod tests;
