//! The proof of the MBC stream mix (#267): which of the `mbc` Live set's
//! tracks reach its stream bus, where the bus goes, and whether only HAND 1
//! sync can be on the stream. camera-box's stream-test guard reads it from
//! the FOH hub's `GET /api/mbc/stream-proof` before each test and relaxes
//! its strict check only on `hand1_sync_only`.
//!
//! The hub walks the set with `get_prop` only (`fohmixer-hub`
//! `stream_proof.rs`) and hands [`decide`] what it read ([`Set`]); this
//! module decides, pure. **Every doubt it can see makes the proof fail,
//! never pass**; what the walk does not read (the stream track's own
//! input, a cue output) it cannot see (#267).
//!
//! **The routing graph.** Every track and return is a node; its output type
//! ([`Output::kind`]) is its edge: [`GROUP`] goes to the group it sits in,
//! a type that names one track or return goes to that one (a group or a
//! track routed to another track), [`SENDS_ONLY`] goes to every return when
//! the track has sends and the set has returns (else nowhere), and the
//! types of [`OUT_OF_SET`] (`Main`, `Master`, `Ext. Out`) leave the set. A
//! muted node passes nothing on. A type naming two tracks or none and no
//! known output (Live shows a `#` in a name as a number, so a route to a
//! track by its shown name would be one), [`GROUP`] on a track in no group,
//! a loop, a bus that is no group, or a stream track whose own output does
//! not leave the set, is an error: every node's output is checked, reached
//! or not.
//!
//! - [`Proof::open`]: every unmuted track that is no group, and every
//!   unmuted return, from which a path of unmuted nodes reaches the bus
//!   [`BUS`] (the bus itself unmuted).
//! - [`Proof::outputs`]: the bus's output (normally the track
//!   [`STREAM_TRACK`]), that track's output (type and channel, the external
//!   output the stream PC takes), and every other track, group, return or
//!   the main track routed to the stream track or to an output sharing a
//!   channel with its output ([`same_output`]: `3` and `3/4` share one; a
//!   channel without numbers shares when it differs at all) ([`Routed`]),
//!   muted or not: the fact is about routing.
//! - [`Proof::hand1_sync_only`]: `open` is exactly [`HAND1_SYNC`], the bus
//!   goes to the stream track and nothing else goes there or to its output.
//!
//! No bus, or two tracks of the bus's or the stream track's name, is an
//! error too ([`decide`]'s `Err`): the answer is then `ok: false` with that
//! reason, which camera-box treats as strict. The names are the set's own
//! (config-like constants, recorded on #267: one set, one reader).

use serde::{Deserialize, Serialize};

/// The stream bus: the group whose sum is the stream mix.
pub const BUS: &str = "ToStream 1s sync #";
/// The track the bus goes to, whose output is the stream PC's input.
pub const STREAM_TRACK: &str = "OM STREAM";
/// The one track that may be open for `hand1_sync_only`.
pub const HAND1_SYNC: &str = "Hand1sync #";
/// The output type of a track that feeds its sends only.
pub const SENDS_ONLY: &str = "Sends Only";
/// The output type of a track that goes to the group it sits in.
pub const GROUP: &str = "Group";
/// The output types that leave the set: the main output (Live 12's name
/// and the older one) and the external outputs. Any other type that names
/// no track or return is a doubt.
pub const OUT_OF_SET: &[&str] = &["Main", "Master", "Ext. Out"];

/// A track's output routing as Live shows it: the type's and the channel's
/// `display_name`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Output {
    #[serde(rename = "type")]
    pub kind: String,
    pub channel: String,
}

/// One track as the walk read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub name: String,
    pub mute: bool,
    /// A group track (`is_foldable`).
    pub group: bool,
    /// The index (in [`Set::tracks`]) of the group it sits in, if any.
    pub parent: Option<usize>,
    pub output: Output,
    /// Its sends (read for a [`SENDS_ONLY`] track only, else 0).
    pub sends: usize,
}

/// A return track as the walk read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Return {
    pub name: String,
    pub mute: bool,
    pub output: Output,
}

/// What the walk read of the set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Set {
    /// Every track in Live's order (groups and their tracks).
    pub tracks: Vec<Track>,
    pub returns: Vec<Return>,
    /// The main track's output.
    pub main: Output,
}

/// A track routed to the stream track or to its output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Routed {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub channel: String,
}

/// Where the stream mix goes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outputs {
    /// The bus's name ([`BUS`]).
    pub bus: String,
    /// The bus's output type (normally [`STREAM_TRACK`]).
    pub bus_to: String,
    /// The stream track's name ([`STREAM_TRACK`]).
    pub stream: String,
    /// The stream track's output.
    pub stream_to: Output,
    /// Every other track, return or the main track routed to the stream
    /// track or to the same output (none for the proof).
    pub others: Vec<Routed>,
}

impl Outputs {
    /// Whether the bus goes to the stream track and nothing else goes there
    /// or to its output.
    pub fn hold(&self) -> bool {
        self.bus_to == self.stream && self.others.is_empty()
    }
}

/// The decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proof {
    pub open: Vec<String>,
    pub hand1_sync_only: bool,
    pub outputs: Outputs,
}

/// The answer of `GET /api/mbc/stream-proof`: `ok` with the proof, or
/// `ok: false` with its `reason` (no `open`, no `outputs`,
/// `hand1_sync_only` false). `ts` is when the walk ended (hub UTC ms),
/// `age_ms` how old the answer is when it is served (a cached one).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    pub ok: bool,
    pub ts: u64,
    pub age_ms: u64,
    pub hand1_sync_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Outputs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Answer {
    /// The answer of a walk at `ts`: its proof or why there is none.
    pub fn of(result: Result<Proof, String>, ts: u64) -> Self {
        match result {
            Ok(proof) => Self {
                ok: true,
                ts,
                age_ms: 0,
                hand1_sync_only: proof.hand1_sync_only,
                open: Some(proof.open),
                outputs: Some(proof.outputs),
                reason: None,
            },
            Err(reason) => Self {
                ok: false,
                ts,
                age_ms: 0,
                hand1_sync_only: false,
                open: None,
                outputs: None,
                reason: Some(reason),
            },
        }
    }
}

/// The one track named `name`, or why not.
fn the_one(tracks: &[Track], name: &str) -> Result<usize, String> {
    let mut found = tracks
        .iter()
        .enumerate()
        .filter(|(_, t)| t.name == name)
        .map(|(i, _)| i);
    match (found.next(), found.next()) {
        (Some(i), None) => Ok(i),
        (None, _) => Err(format!("no track is named {name}")),
        (Some(_), Some(_)) => Err(format!("two tracks are named {name}")),
    }
}

/// A node of the routing graph: track `i` is `i`, return `j` is
/// `tracks.len() + j`.
struct Graph<'a> {
    set: &'a Set,
}

impl Graph<'_> {
    fn len(&self) -> usize {
        self.set.tracks.len() + self.set.returns.len()
    }

    fn name(&self, node: usize) -> &str {
        self.set.tracks.get(node).map_or_else(
            || &self.set.returns[node - self.set.tracks.len()].name,
            |t| &t.name,
        )
    }

    fn mute(&self, node: usize) -> bool {
        self.set.tracks.get(node).map_or_else(
            || self.set.returns[node - self.set.tracks.len()].mute,
            |t| t.mute,
        )
    }

    fn output(&self, node: usize) -> &Output {
        self.set.tracks.get(node).map_or_else(
            || &self.set.returns[node - self.set.tracks.len()].output,
            |t| &t.output,
        )
    }

    /// Where `node`'s audio goes (see the module's doc), or why it is in
    /// doubt.
    fn edges(&self, node: usize) -> Result<Vec<usize>, String> {
        let kind = self.output(node).kind.as_str();
        let track = self.set.tracks.get(node);
        if kind == GROUP {
            return match track.and_then(|t| t.parent) {
                Some(p) if p < self.set.tracks.len() => Ok(vec![p]),
                _ => Err(format!(
                    "{} goes to {GROUP} but sits in no group",
                    self.name(node)
                )),
            };
        }
        if kind == SENDS_ONLY {
            let sends = track.map_or(0, |t| t.sends);
            return Ok(if sends > 0 {
                (self.set.tracks.len()..self.len()).collect()
            } else {
                Vec::new()
            });
        }
        let named: Vec<usize> = (0..self.len())
            .filter(|&n| n != node && self.name(n) == kind)
            .collect();
        if named.len() > 1 {
            return Err(format!(
                "{} goes to {kind}, which names two tracks",
                self.name(node)
            ));
        }
        if named.is_empty() && !OUT_OF_SET.contains(&kind) {
            return Err(format!(
                "{} goes to {kind}, which is no track and no known output",
                self.name(node)
            ));
        }
        Ok(named)
    }

    /// Whether a path of unmuted nodes leads from `node` to `bus` (the bus
    /// unmuted); `path` holds the nodes before it, and a node met again on
    /// it is a loop.
    fn reaches(&self, node: usize, bus: usize, path: &mut Vec<usize>) -> Result<bool, String> {
        if path.contains(&node) {
            return Err(format!("the routing loops through {}", self.name(node)));
        }
        path.push(node);
        for next in self.edges(node)? {
            if self.mute(next) {
                continue;
            }
            if next == bus || self.reaches(next, bus, path)? {
                return Ok(true);
            }
        }
        path.pop();
        Ok(false)
    }
}

/// The numbers of an output channel (`3/4 Final LR` is 3 and 4), none when
/// its first word holds anything but numbers and `/`.
fn channel_numbers(channel: &str) -> Option<Vec<u32>> {
    let first = channel.split_whitespace().next()?;
    first.split('/').map(|n| n.parse().ok()).collect()
}

/// Whether `a` and `b` are the same output or share a channel of it: the
/// same type and the same channel, or channels whose numbers overlap, or
/// (a doubt) channels that differ without both having numbers.
pub fn same_output(a: &Output, b: &Output) -> bool {
    if a.kind != b.kind {
        return false;
    }
    if a.channel == b.channel {
        return true;
    }
    match (channel_numbers(&a.channel), channel_numbers(&b.channel)) {
        (Some(x), Some(y)) => x.iter().any(|n| y.contains(n)),
        _ => true,
    }
}

/// The proof of `set` (see the module's doc), or why there is none.
pub fn decide(set: &Set) -> Result<Proof, String> {
    let bus = the_one(&set.tracks, BUS)?;
    let stream = the_one(&set.tracks, STREAM_TRACK)?;
    if !set.tracks[bus].group {
        return Err(format!("{BUS} is no group"));
    }
    let graph = Graph { set };
    // Every node's output is read whole first: a doubt anywhere fails.
    for node in 0..graph.len() {
        graph.edges(node)?;
    }
    let stream_kind = &set.tracks[stream].output.kind;
    if !graph.edges(stream)?.is_empty() || stream_kind == SENDS_ONLY {
        return Err(format!(
            "{STREAM_TRACK} goes to {stream_kind}, not out of the set"
        ));
    }
    let bus_open = !set.tracks[bus].mute;
    let mut open = Vec::new();
    for node in 0..graph.len() {
        let group = set.tracks.get(node).is_some_and(|t| t.group);
        if node == bus || group || graph.mute(node) {
            continue;
        }
        if bus_open && graph.reaches(node, bus, &mut Vec::new())? {
            open.push(graph.name(node).to_string());
        }
    }
    let stream_to = set.tracks[stream].output.clone();
    let to_stream =
        |output: &Output| output.kind == STREAM_TRACK || same_output(output, &stream_to);
    let routed = |name: &str, output: &Output| Routed {
        name: name.to_string(),
        kind: output.kind.clone(),
        channel: output.channel.clone(),
    };
    let mut others: Vec<Routed> = (0..graph.len())
        .filter(|&n| n != bus && n != stream && to_stream(graph.output(n)))
        .map(|n| routed(graph.name(n), graph.output(n)))
        .collect();
    if to_stream(&set.main) {
        others.push(routed("Main", &set.main));
    }
    let outputs = Outputs {
        bus: BUS.to_string(),
        bus_to: set.tracks[bus].output.kind.clone(),
        stream: STREAM_TRACK.to_string(),
        stream_to,
        others,
    };
    let hand1_sync_only = open == [HAND1_SYNC] && outputs.hold();
    Ok(Proof {
        open,
        hand1_sync_only,
        outputs,
    })
}

#[cfg(test)]
mod tests;
