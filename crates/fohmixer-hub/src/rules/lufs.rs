//! LUFS (#151, spec §4.4, §5.2, §8): which strip a LUFS device's readings
//! belong to, what each loudness point shows, and its problems. Pure: no
//! I/O, the time (router ms) passed in; every input returns [`Act`]s, which
//! the glue (`router/lufs.rs`) carries out.
//!
//! - **The key** ([`Key`], `bound.rs`) is a device at a place: kind, Max id
//!   and its four parents' names. Its track and set are the last two named
//!   parents (`lufs::wire::track_and_set`).
//! - **The candidates** of a key are the served loudness points whose track
//!   has the key's track name in an online Live whose set has the key's set
//!   name ([`candidates`]; the glue names each strip's track: a frame strip
//!   by its anchor's name, a marker strip from the hub's own track list).
//! - **The confirmation:** each candidate Live is asked once
//!   ([`Act::Confirm`], the walk of `confirm.rs`): exactly one Live
//!   answering [`Answer::Ours`] matches the key to it; two (both Lives hold
//!   one at that place) or a Live with two at the place is `two` (the
//!   strips' problem `two LUFS meters`, nothing shown); none is unmatched,
//!   asked again at the key's next reading after [`RECONFIRM_MS`], a
//!   track-list change of a Live, a Live's set name read, a layout or a
//!   disconnect.
//! - **The reading** a matched point shows ([`reading_of`], the main
//!   session's ruling 4 of 27.10): an input point (`+LUFS:<target>`) its
//!   `V`, pre-fader, as trueLevel; an output point (`+LUFS`) its `S` plus
//!   the strip's fader dB ([`post_fader`], exact for a gain: K-weighting is
//!   linear and short-term has no gate), silence when the track is muted,
//!   none until the fader is known.
//! - **Stale:** a LUFS key unheard for [`STALE_MS`] (a meter key
//!   [`METER_STALE_MS`]) is lost at the next tick: one `lost` record, its
//!   points show nothing.
//! - **Problems:** `two LUFS meters`; `no LUFS meter` for a point of an
//!   online Live with no matched device [`NO_METER_GRACE_MS`] after the Live
//!   connected and the layout was served; `no meter device` for an armed
//!   input point with no matched `FohMixer Meter` after the same grace.
//! - **The armed tracks' levels** (§3.4, Task 13): a point whose track holds
//!   exactly one matched, fresh `FohMixer Meter` gets the device's L/R peaks
//!   after its fader, pan and mute (`fohmixer_proto::meter::post_fader`,
//!   [`levels_of`]) as [`Act::Levels`] on every change; when that ends (the
//!   key stale at [`METER_STALE_MS`], unmatched, the Live offline, the point
//!   gone, the fader not known) [`Act::LevelsGone`] gives the strip's meter
//!   keys back to Live's own values.
//! - **Records** (`lufs`): a key found, matched, unmatched (once per key),
//!   two, lost; a point's problem raised (warn) or cleared; a point's state
//!   change at most once per [`STATE_LOG_MS`]. Never one per datagram.

use std::collections::{BTreeMap, BTreeSet};

use fohmixer_proto::client::{LufsDeviceStatus, LufsItem, LufsStripStatus};
use fohmixer_proto::layout::LufsSpec;
use fohmixer_proto::lufs::wire::{DeviceKind, Reading, track_and_set};
use fohmixer_proto::lufs::{LufsProblem, LufsState, SILENT_TENTHS, judge};
use fohmixer_proto::meter;
use fohmixer_proto::volume::value2db;
use serde_json::{Value, json};

#[path = "lufs/bound.rs"]
pub mod bound;
#[path = "lufs/confirm.rs"]
pub mod confirm;

pub use bound::Key;
pub use confirm::Answer;

/// A LUFS key unheard this long is lost.
pub const STALE_MS: u64 = 1000;
/// A meter key unheard this long is lost.
pub const METER_STALE_MS: u64 = 500;
/// `no LUFS meter` waits this long after a Live connected and after a
/// layout was served.
pub const NO_METER_GRACE_MS: u64 = 3000;
/// An unmatched key is asked again at its next reading after this long.
pub const RECONFIRM_MS: u64 = 10_000;
/// A point's state change is recorded at most once in this long.
pub const STATE_LOG_MS: u64 = 3000;

/// How long a key of `kind` lives without a datagram.
pub fn stale_ms(kind: DeviceKind) -> u64 {
    match kind {
        DeviceKind::Lufs => STALE_MS,
        DeviceKind::Meter => METER_STALE_MS,
    }
}

/// Whether a key of `kind` last heard at `heard` is lost at `now`.
pub fn is_stale(kind: DeviceKind, heard: u64, now: u64) -> bool {
    now.saturating_sub(heard) >= stale_ms(kind)
}

/// A kind's word.
pub fn kind_word(kind: DeviceKind) -> &'static str {
    match kind {
        DeviceKind::Lufs => "lufs",
        DeviceKind::Meter => "meter",
    }
}

/// One served loudness point: its strip's instance and track target
/// (canonical), the track's name as the hub knows it now (none: unknown),
/// its spec and its label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LufsStrip {
    pub instance: String,
    pub target: String,
    pub track: Option<String>,
    pub spec: LufsSpec,
    pub label: String,
}

/// A point's track as the hub follows it: Live's volume value (0..1), its
/// pan (−1..+1), mute and (an input point's) arm, each none until heard.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Fader {
    pub volume: Option<f64>,
    pub mute: Option<bool>,
    pub pan: Option<f64>,
    pub arm: Option<bool>,
}

/// An output point's short-term after its fader: `tenths` plus the fader's
/// `db` (rounded to tenths); silence stays silence, and anything at or
/// under [`SILENT_TENTHS`] (the fader at −inf too) is silence.
pub fn post_fader(tenths: i16, db: f64) -> i16 {
    if tenths <= SILENT_TENTHS {
        return SILENT_TENTHS;
    }
    let sum = f64::from(tenths) + (db * 10.0).round();
    if sum.is_nan() || sum <= f64::from(SILENT_TENTHS) {
        SILENT_TENTHS
    } else if sum >= f64::from(i16::MAX) {
        i16::MAX
    } else {
        sum as i16
    }
}

/// The reading a matched point shows (the main session's ruling 4): an
/// input point its `V` (`second`) unchanged; an output point its `S`
/// (`first`) after the fader, silence while muted, none until both the
/// volume and the mute are known.
pub fn reading_of(spec: LufsSpec, first: i16, second: i16, fader: Fader) -> Option<i16> {
    if spec.target.is_some() {
        return Some(second);
    }
    match (fader.mute, fader.volume) {
        (Some(true), _) => Some(SILENT_TENTHS),
        (Some(false), Some(volume)) => Some(post_fader(first, value2db(volume))),
        _ => None,
    }
}

/// A meter device's L/R peaks (tenths of dBFS) as Live's meter values after
/// the track's fader, pan and mute: `(0, 0)` while muted, none until the
/// mute, the volume and the pan are known (the keys stay Live's).
pub fn levels_of(left: i16, right: i16, fader: Fader) -> Option<(f64, f64)> {
    match (fader.mute, fader.volume, fader.pan) {
        (Some(true), _, _) => Some((0.0, 0.0)),
        (Some(false), Some(volume), Some(pan)) => {
            Some(meter::post_fader(left, right, volume, pan, false))
        }
        _ => None,
    }
}

/// The values of a strip's meter keys for `levels` (left, right): each of
/// `props` that is a meter prop with its value, `output_meter_level` the
/// louder side.
pub fn meter_values(levels: (f64, f64), props: &[&str]) -> Vec<(String, f64)> {
    props
        .iter()
        .filter_map(|prop| {
            let value = match *prop {
                "output_meter_left" => levels.0,
                "output_meter_right" => levels.1,
                "output_meter_level" => levels.0.max(levels.1),
                _ => return None,
            };
            Some(((*prop).to_string(), value))
        })
        .collect()
}

/// An input point's meter problem: armed (Live meters an armed track's
/// input), no matched meter device, and the grace over.
pub fn meter_problem(
    input: bool,
    armed: bool,
    has_meter: bool,
    grace_over: bool,
) -> Option<LufsProblem> {
    (input && armed && !has_meter && grace_over).then_some(LufsProblem::NoMeterDevice)
}

/// The strips a key of `kind` with the track and set names `track`, `set`
/// may belong to: the points whose track is `track` in an instance whose
/// set is `set` (`sets`: each online instance's set name, once read).
pub fn candidates<'a>(
    strips: &'a [LufsStrip],
    sets: &BTreeMap<String, String>,
    track: &str,
    set: &str,
) -> Vec<&'a LufsStrip> {
    strips
        .iter()
        .filter(|s| s.track.as_deref() == Some(track))
        .filter(|s| sets.get(&s.instance).map(String::as_str) == Some(set))
        .collect()
}

/// Whether a point's state change is recorded now: it changed, and the
/// last record of the point is [`STATE_LOG_MS`] or longer ago (or none).
pub fn state_due(prev: LufsState, now: LufsState, since_last: Option<u64>) -> bool {
    prev != now && since_last.is_none_or(|ms| ms >= STATE_LOG_MS)
}

/// Whether a point of an online Live with no matched device has its `no
/// LUFS meter` at `now`: the grace has run from the later of the Live's
/// connect and the layout's serving.
pub fn grace_over(online_since: u64, served_at: Option<u64>, now: u64) -> bool {
    now >= online_since.max(served_at.unwrap_or(0)) + NO_METER_GRACE_MS
}

/// A point's problem from its matched devices (`matched`), whether a key
/// at its place is `two`, and whether its grace is over.
pub fn problem_of(matched: usize, two: bool, grace_over: bool) -> Option<LufsProblem> {
    if two || matched >= 2 {
        Some(LufsProblem::TwoLufsMeters)
    } else if matched == 0 && grace_over {
        Some(LufsProblem::NoLufsMeter)
    } else {
        None
    }
}

/// One record and its log line.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub warn: bool,
    pub fields: Value,
    pub text: String,
}

fn key_fields(key: &Key, what: &str) -> Value {
    json!({
        "what": what,
        "kind": kind_word(key.kind),
        "device": key.device,
        "parents": key.parents,
    })
}

/// A key's first datagram.
pub fn found_line(key: &Key, version: &str) -> Line {
    let mut fields = key_fields(key, "found");
    fields["version"] = json!(version);
    Line {
        warn: false,
        fields,
        text: "a LUFS device is heard".to_string(),
    }
}

/// A key matched to `instance`.
pub fn matched_line(key: &Key, instance: &str) -> Line {
    let mut fields = key_fields(key, "matched");
    fields["instance"] = json!(instance);
    Line {
        warn: false,
        fields,
        text: "a LUFS device is matched to its strip".to_string(),
    }
}

/// A key no Live confirms (`asked`: the Lives asked; none: no candidate).
pub fn unmatched_line(key: &Key, asked: &BTreeSet<String>) -> Line {
    let mut fields = key_fields(key, "unmatched");
    fields["asked"] = json!(asked);
    fields["warn"] = json!(true);
    Line {
        warn: true,
        fields,
        text: "a LUFS device matches no loudness point".to_string(),
    }
}

/// A key two Lives (or one Live twice) confirm.
pub fn two_line(key: &Key, instances: &BTreeSet<String>) -> Line {
    let mut fields = key_fields(key, "two");
    fields["instances"] = json!(instances);
    fields["warn"] = json!(true);
    Line {
        warn: true,
        fields,
        text: "a LUFS device's place holds two devices: nothing shown".to_string(),
    }
}

/// A key lost: how long it lived, its last values and its match.
pub fn lost_line(key: &Key, lived_ms: u64, last: (i16, i16), instance: Option<&str>) -> Line {
    let mut fields = key_fields(key, "lost");
    fields["lived_ms"] = json!(lived_ms);
    fields["first"] = json!(last.0);
    fields["second"] = json!(last.1);
    fields["instance"] = json!(instance);
    Line {
        warn: false,
        fields,
        text: "a LUFS device went silent".to_string(),
    }
}

/// A point's problem raised (warn) or cleared.
pub fn problem_line(strip: &LufsStrip, problem: Option<LufsProblem>) -> Line {
    let mut fields = json!({
        "what": "problem",
        "instance": strip.instance,
        "target": strip.target,
        "label": strip.label,
        "problem": problem.map(LufsProblem::text),
    });
    if problem.is_some() {
        fields["warn"] = json!(true);
    }
    Line {
        warn: problem.is_some(),
        fields,
        text: match problem {
            Some(_) => "a loudness point has a problem".to_string(),
            None => "a loudness point's problem cleared".to_string(),
        },
    }
}

/// A point's state change.
pub fn state_line(strip: &LufsStrip, state: LufsState, tenths: Option<i16>) -> Line {
    Line {
        warn: false,
        fields: json!({
            "what": "state",
            "instance": strip.instance,
            "target": strip.target,
            "label": strip.label,
            "state": state.as_str(),
            "tenths": tenths,
        }),
        text: "a loudness point's state changed".to_string(),
    }
}

/// What the rule asks of the glue.
#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    /// Walk `track` (a candidate strip's target) in `instance` for the
    /// key; its answer to [`Lufs::answered`] with `ask`.
    Confirm {
        key: Key,
        instance: String,
        track: String,
        ask: u64,
    },
    /// A point's new reading or problem, to every page.
    Send(LufsItem),
    /// A record and its line.
    Log(Line),
    /// A point's meter keys carry these levels (left, right; Live's 0..1)
    /// in place of Live's own values.
    Levels {
        instance: String,
        target: String,
        levels: (f64, f64),
    },
    /// A point's meter keys go back to Live's own values.
    LevelsGone { instance: String, target: String },
}

/// Where a key stands.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    /// Its walks are out (`ask`): the Lives still to answer, those that
    /// answered ours, and those that answered two.
    Asking {
        ask: u64,
        asked: BTreeSet<String>,
        waiting: BTreeSet<String>,
        ours: BTreeSet<String>,
        two: BTreeSet<String>,
    },
    Matched(String),
    /// Both Lives (or one twice): nothing shown there.
    Two(BTreeSet<String>),
    /// None confirmed (`at`); asked again at its next reading once
    /// `again` or [`RECONFIRM_MS`] passed.
    Unmatched {
        at: u64,
        again: bool,
    },
}

impl Status {
    fn word(&self) -> &'static str {
        match self {
            Status::Asking { .. } => "asking",
            Status::Matched(_) => "matched",
            Status::Two(_) => "two",
            Status::Unmatched { .. } => "unmatched",
        }
    }
}

#[derive(Debug, Clone)]
struct Device {
    version: String,
    first: i16,
    second: i16,
    born: u64,
    heard: u64,
    status: Status,
    told_unmatched: bool,
}

/// What a point last told the pages, and its state log.
#[derive(Debug, Clone, PartialEq)]
struct Shown {
    tenths: Option<i16>,
    problem: Option<LufsProblem>,
    logged: LufsState,
    logged_at: Option<u64>,
}

impl Default for Shown {
    fn default() -> Self {
        Self {
            tenths: None,
            problem: None,
            logged: LufsState::None,
            logged_at: None,
        }
    }
}

/// An online Live: since when, and its set's name once read.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Online {
    since: u64,
    set: Option<String>,
}

/// The rule.
#[derive(Debug, Default)]
pub struct Lufs {
    strips: Vec<LufsStrip>,
    served_at: Option<u64>,
    online: BTreeMap<String, Online>,
    devices: BTreeMap<Key, Device>,
    faders: BTreeMap<(String, String), Fader>,
    shown: BTreeMap<(String, String), Shown>,
    /// The levels each point's meter keys carry now (sent as `Levels`).
    levels: BTreeMap<(String, String), (f64, f64)>,
    asks: u64,
}

impl Lufs {
    pub fn new() -> Self {
        Self::default()
    }

    /// The points served now.
    pub fn strips(&self) -> &[LufsStrip] {
        &self.strips
    }

    /// The served points: a new layout (`layout`, its grace starts at
    /// `now`) or the same points with their tracks named anew (a track
    /// list changed). Unmatched keys are asked again at their next reading.
    pub fn set_strips(&mut self, strips: Vec<LufsStrip>, layout: bool, now: u64) -> Vec<Act> {
        self.strips = strips;
        if layout {
            self.served_at = Some(now);
        }
        let keep: BTreeSet<(String, String)> = self
            .strips
            .iter()
            .map(|s| (s.instance.clone(), s.target.clone()))
            .collect();
        self.shown.retain(|point, _| keep.contains(point));
        self.faders.retain(|point, _| keep.contains(point));
        self.ask_again();
        self.refresh(now)
    }

    /// An instance connected at `now` (its set's name not known yet).
    pub fn connected(&mut self, instance: &str, now: u64) -> Vec<Act> {
        self.online.insert(
            instance.to_string(),
            Online {
                since: now,
                set: None,
            },
        );
        self.refresh(now)
    }

    /// An instance's set's name as read.
    pub fn named(&mut self, instance: &str, set: &str, now: u64) -> Vec<Act> {
        if let Some(online) = self.online.get_mut(instance) {
            online.set = Some(set.to_string());
        }
        self.ask_again();
        self.refresh(now)
    }

    /// An instance went offline: its matches dropped, its points show
    /// nothing.
    pub fn offline(&mut self, instance: &str, now: u64) -> Vec<Act> {
        self.online.remove(instance);
        let mut acts = Vec::new();
        let keys: Vec<Key> = self.devices.keys().cloned().collect();
        for key in keys {
            let gone = match &self.devices[&key].status {
                Status::Matched(i) => i == instance,
                Status::Two(set) => set.contains(instance),
                Status::Asking { waiting, .. } => waiting.contains(instance),
                Status::Unmatched { .. } => false,
            };
            if !gone {
                continue;
            }
            let asking = matches!(self.devices[&key].status, Status::Asking { .. });
            if asking {
                acts.extend(self.answered(
                    &key,
                    instance,
                    self.asks_of(&key),
                    Answer::NotOurs,
                    now,
                ));
            } else if let Some(device) = self.devices.get_mut(&key) {
                device.status = Status::Unmatched {
                    at: now,
                    again: true,
                };
            }
        }
        for point in self.faders.keys().cloned().collect::<Vec<_>>() {
            if point.0 == instance {
                self.faders.insert(point, Fader::default());
            }
        }
        acts.extend(self.refresh(now));
        acts
    }

    fn asks_of(&self, key: &Key) -> u64 {
        match self.devices.get(key).map(|d| &d.status) {
            Some(Status::Asking { ask, .. }) => *ask,
            _ => 0,
        }
    }

    /// A track list changed: unmatched keys are asked again (a track
    /// renamed or inserted may be a key's now).
    pub fn tracks_changed(&mut self) {
        self.ask_again();
    }

    fn ask_again(&mut self) {
        for device in self.devices.values_mut() {
            if let Status::Unmatched { again, .. } = &mut device.status {
                *again = true;
            }
        }
    }

    /// A point's fader (its volume, pan, mute and arm as heard).
    pub fn fader(&mut self, instance: &str, target: &str, fader: Fader, now: u64) -> Vec<Act> {
        self.faders
            .insert((instance.to_string(), target.to_string()), fader);
        self.refresh(now)
    }

    /// One reading taken by the listener.
    pub fn reading(&mut self, reading: &Reading, now: u64) -> Vec<Act> {
        let key = Key::of(reading);
        let mut acts = Vec::new();
        let ask = match self.devices.get_mut(&key) {
            Some(device) => {
                device.version.clone_from(&reading.version);
                device.first = reading.first;
                device.second = reading.second;
                device.heard = now;
                match &device.status {
                    Status::Unmatched { at, again } => {
                        *again || now.saturating_sub(*at) >= RECONFIRM_MS
                    }
                    _ => false,
                }
            }
            None => {
                acts.push(Act::Log(found_line(&key, &reading.version)));
                self.devices.insert(
                    key.clone(),
                    Device {
                        version: reading.version.clone(),
                        first: reading.first,
                        second: reading.second,
                        born: now,
                        heard: now,
                        status: Status::Unmatched {
                            at: now,
                            again: true,
                        },
                        told_unmatched: false,
                    },
                );
                true
            }
        };
        if ask {
            acts.extend(self.ask(&key, now));
        }
        acts.extend(self.refresh(now));
        acts
    }

    /// The online instances' set names (only those read).
    fn sets(&self) -> BTreeMap<String, String> {
        self.online
            .iter()
            .filter_map(|(i, o)| o.set.clone().map(|set| (i.clone(), set)))
            .collect()
    }

    /// Asks every candidate instance of `key` once; none: unmatched.
    fn ask(&mut self, key: &Key, now: u64) -> Vec<Act> {
        let sets = self.sets();
        let Some((track, set)) = track_and_set(&key.parents) else {
            return Vec::new();
        };
        // The first candidate strip of each instance names the track walked.
        let mut tracks: BTreeMap<String, String> = BTreeMap::new();
        for strip in candidates(&self.strips, &sets, track, set) {
            tracks
                .entry(strip.instance.clone())
                .or_insert_with(|| strip.target.clone());
        }
        if tracks.is_empty() {
            // A Live whose set's name is not read yet may be the key's: no
            // record until the names are known (they ask it again).
            if self.online.values().any(|o| o.set.is_none()) {
                if let Some(device) = self.devices.get_mut(key) {
                    device.status = Status::Unmatched {
                        at: now,
                        again: false,
                    };
                }
                return Vec::new();
            }
            return self.unmatched(key, &BTreeSet::new(), now);
        }
        self.asks += 1;
        let ask = self.asks;
        if let Some(device) = self.devices.get_mut(key) {
            device.status = Status::Asking {
                ask,
                asked: tracks.keys().cloned().collect(),
                waiting: tracks.keys().cloned().collect(),
                ours: BTreeSet::new(),
                two: BTreeSet::new(),
            };
        }
        tracks
            .into_iter()
            .map(|(instance, track)| Act::Confirm {
                key: key.clone(),
                instance,
                track,
                ask,
            })
            .collect()
    }

    fn unmatched(&mut self, key: &Key, asked: &BTreeSet<String>, now: u64) -> Vec<Act> {
        let Some(device) = self.devices.get_mut(key) else {
            return Vec::new();
        };
        device.status = Status::Unmatched {
            at: now,
            again: false,
        };
        if device.told_unmatched {
            return Vec::new();
        }
        device.told_unmatched = true;
        vec![Act::Log(unmatched_line(key, asked))]
    }

    /// A confirming walk's answer (`ask` names the walk; an older one's is
    /// void). A Live that failed to answer counts as not ours (asked again
    /// later).
    pub fn answered(
        &mut self,
        key: &Key,
        instance: &str,
        ask: u64,
        answer: Answer,
        now: u64,
    ) -> Vec<Act> {
        let Some(device) = self.devices.get_mut(key) else {
            return Vec::new();
        };
        let Status::Asking {
            ask: current,
            asked,
            waiting,
            ours,
            two,
        } = &mut device.status
        else {
            return Vec::new();
        };
        if *current != ask || !waiting.remove(instance) {
            return Vec::new();
        }
        match answer {
            Answer::Ours => {
                ours.insert(instance.to_string());
            }
            Answer::Two => {
                two.insert(instance.to_string());
            }
            Answer::NotOurs | Answer::Failed(_) => {}
        }
        if !waiting.is_empty() {
            return Vec::new();
        }
        let (asked, ours, two) = (
            std::mem::take(asked),
            std::mem::take(ours),
            std::mem::take(two),
        );
        let mut acts = Vec::new();
        if !two.is_empty() || ours.len() >= 2 {
            let both: BTreeSet<String> = ours.union(&two).cloned().collect();
            acts.push(Act::Log(two_line(key, &both)));
            device.status = Status::Two(both);
        } else if let Some(instance) = ours.into_iter().next() {
            acts.push(Act::Log(matched_line(key, &instance)));
            device.status = Status::Matched(instance);
            device.told_unmatched = false;
        } else {
            acts.extend(self.unmatched(key, &asked, now));
        }
        acts.extend(self.refresh(now));
        acts
    }

    /// The router's tick: stale keys lost, the grace's problems.
    pub fn tick(&mut self, now: u64) -> Vec<Act> {
        let stale: Vec<Key> = self
            .devices
            .iter()
            .filter(|(key, device)| is_stale(key.kind, device.heard, now))
            .map(|(key, _)| key.clone())
            .collect();
        let mut acts = Vec::new();
        for key in stale {
            if let Some(device) = self.devices.remove(&key) {
                let instance = match &device.status {
                    Status::Matched(i) => Some(i.as_str()),
                    _ => None,
                };
                acts.push(Act::Log(lost_line(
                    &key,
                    device.heard.saturating_sub(device.born),
                    (device.first, device.second),
                    instance,
                )));
            }
        }
        acts.extend(self.refresh(now));
        acts
    }

    /// The devices of `kind` matched to `strip` and whether a key of that
    /// kind at its place is `two`.
    fn covering(&self, strip: &LufsStrip, kind: DeviceKind) -> (Vec<&Device>, bool) {
        let mut matched = Vec::new();
        let mut two = false;
        for (key, device) in &self.devices {
            if key.kind != kind
                || track_and_set(&key.parents).map(|(t, _)| t) != strip.track.as_deref()
            {
                continue;
            }
            match &device.status {
                Status::Matched(i) if *i == strip.instance => matched.push(device),
                Status::Two(set) if set.contains(&strip.instance) => two = true,
                _ => {}
            }
        }
        (matched, two)
    }

    /// What a point shows now: its reading, its problem and its meter keys'
    /// levels.
    fn point(
        &self,
        strip: &LufsStrip,
        now: u64,
    ) -> (Option<i16>, Option<LufsProblem>, Option<(f64, f64)>) {
        let (matched, two) = self.covering(strip, DeviceKind::Lufs);
        let grace = self
            .online
            .get(&strip.instance)
            .is_some_and(|o| grace_over(o.since, self.served_at, now));
        let lufs_problem = problem_of(matched.len(), two, grace);
        let fader = self
            .faders
            .get(&(strip.instance.clone(), strip.target.clone()))
            .copied()
            .unwrap_or_default();
        let tenths = match (lufs_problem, matched.as_slice()) {
            (None, [device]) => reading_of(strip.spec, device.first, device.second, fader),
            _ => None,
        };
        let (meters, meters_two) = self.covering(strip, DeviceKind::Meter);
        let levels = match (meters_two, meters.as_slice()) {
            (false, [device]) => levels_of(device.first, device.second, fader),
            _ => None,
        };
        let problem = lufs_problem.or(meter_problem(
            strip.spec.target.is_some(),
            fader.arm == Some(true),
            !meters.is_empty() || meters_two,
            grace,
        ));
        (tenths, problem, levels)
    }

    /// Every point's reading and problem: a change goes to the pages, a
    /// problem's change and a due state change are recorded.
    fn refresh(&mut self, now: u64) -> Vec<Act> {
        let mut acts = Vec::new();
        let strips = self.strips.clone();
        for strip in &strips {
            let (tenths, problem, levels) = self.point(strip, now);
            let point = (strip.instance.clone(), strip.target.clone());
            if self.levels.get(&point) != levels.as_ref() {
                match levels {
                    Some(levels) => {
                        self.levels.insert(point.clone(), levels);
                        acts.push(Act::Levels {
                            instance: strip.instance.clone(),
                            target: strip.target.clone(),
                            levels,
                        });
                    }
                    None => {
                        self.levels.remove(&point);
                        acts.push(Act::LevelsGone {
                            instance: strip.instance.clone(),
                            target: strip.target.clone(),
                        });
                    }
                }
            }
            let shown = self
                .shown
                .entry((strip.instance.clone(), strip.target.clone()))
                .or_default();
            if (shown.tenths, shown.problem) != (tenths, problem) {
                acts.push(Act::Send(LufsItem {
                    instance: strip.instance.clone(),
                    target: strip.target.clone(),
                    tenths,
                    problem,
                }));
            }
            if shown.problem != problem {
                acts.push(Act::Log(problem_line(strip, problem)));
            }
            shown.tenths = tenths;
            shown.problem = problem;
            let state = judge(tenths, strip.spec.target);
            let since = shown.logged_at.map(|at| now.saturating_sub(at));
            if state_due(shown.logged, state, since) {
                acts.push(Act::Log(state_line(strip, state, tenths)));
                shown.logged = state;
                shown.logged_at = Some(now);
            }
        }
        // A point no longer served gives its meter keys back.
        let served: BTreeSet<(String, String)> = strips
            .iter()
            .map(|s| (s.instance.clone(), s.target.clone()))
            .collect();
        let gone: Vec<(String, String)> = self
            .levels
            .keys()
            .filter(|point| !served.contains(*point))
            .cloned()
            .collect();
        for (instance, target) in gone {
            self.levels.remove(&(instance.clone(), target.clone()));
            acts.push(Act::LevelsGone { instance, target });
        }
        acts
    }

    /// Every point's current item (a page's attach).
    pub fn items(&self) -> Vec<LufsItem> {
        self.strips
            .iter()
            .map(|strip| {
                let shown = self
                    .shown
                    .get(&(strip.instance.clone(), strip.target.clone()))
                    .cloned()
                    .unwrap_or_default();
                LufsItem {
                    instance: strip.instance.clone(),
                    target: strip.target.clone(),
                    tenths: shown.tenths,
                    problem: shown.problem,
                }
            })
            .collect()
    }

    /// The followed faders' points: the output points.
    pub fn output_points(&self) -> Vec<(String, String)> {
        self.strips
            .iter()
            .filter(|s| s.spec.target.is_none())
            .map(|s| (s.instance.clone(), s.target.clone()))
            .collect()
    }

    /// `/api/status` `lufs.devices` at `now`.
    pub fn device_status(&self, now: u64) -> Vec<LufsDeviceStatus> {
        self.devices
            .iter()
            .map(|(key, device)| LufsDeviceStatus {
                kind: kind_word(key.kind).to_string(),
                device: key.device,
                version: device.version.clone(),
                parents: key.parents.to_vec(),
                state: device.status.word().to_string(),
                instance: match &device.status {
                    Status::Matched(i) => Some(i.clone()),
                    _ => None,
                },
                age_ms: now.saturating_sub(device.heard),
                first: device.first,
                second: device.second,
            })
            .collect()
    }

    /// `/api/status` `lufs.strips`.
    pub fn strip_status(&self) -> Vec<LufsStripStatus> {
        self.strips
            .iter()
            .map(|strip| {
                let shown = self
                    .shown
                    .get(&(strip.instance.clone(), strip.target.clone()))
                    .cloned()
                    .unwrap_or_default();
                LufsStripStatus {
                    instance: strip.instance.clone(),
                    target: strip.target.clone(),
                    label: strip.label.clone(),
                    track: strip.track.clone(),
                    state: judge(shown.tenths, strip.spec.target),
                    tenths: shown.tenths,
                    problem: shown.problem,
                }
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "lufs/tests.rs"]
mod tests;
