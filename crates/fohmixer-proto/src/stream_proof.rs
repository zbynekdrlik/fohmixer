//! The proof of the MBC stream mix (#267): which of the `mbc` Live set's
//! tracks reach its stream bus, where the bus goes, and whether only HAND 1
//! sync can be on the stream. camera-box's stream-test guard reads it from
//! the FOH hub's `GET /api/mbc/stream-proof` before each test and relaxes
//! its strict check only on `hand1_sync_only`.
//!
//! The hub walks the set with `get_prop` only (`fohmixer-hub`
//! `stream_proof.rs`) and hands [`decide`] what it read ([`Set`]); this
//! module decides, pure. **Every doubt makes the proof fail, never pass:**
//!
//! - [`Proof::open`]: every unmuted track that is no group and reaches the
//!   bus [`BUS`]: through its chain of groups (the bus somewhere up the
//!   chain, no muted group on the way, the bus included) or routed straight
//!   to it (its output type names the bus). A [`SENDS_ONLY`] track feeds
//!   nothing only with no sends or no returns; with both it stays open (its
//!   audio may come back through a return). The returns count as tracks
//!   routed straight to the bus too.
//! - [`Proof::outputs`]: the bus's output (normally the track
//!   [`STREAM_TRACK`]), that track's output (type and channel, the external
//!   output the stream PC takes), and every other track, return or the main
//!   track routed to the stream track or to that same output ([`Routed`]),
//!   muted or not: the fact is about routing.
//! - [`Proof::hand1_sync_only`]: `open` is exactly [`HAND1_SYNC`], the bus
//!   goes to the stream track and nothing else goes there or to its output.
//!
//! No bus, or two tracks of the bus's or the stream track's name, is an
//! error ([`decide`]'s `Err`): the answer is then `ok: false` with that
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

/// Whether track `at`'s audio reaches the bus `bus` through its groups:
/// the bus up its chain and no muted group on the way (the bus included).
/// The chain is walked at most once per track, so a cycle in a broken read
/// ends there (no bus found).
fn through_groups(tracks: &[Track], at: usize, bus: usize) -> bool {
    let mut parent = tracks[at].parent;
    for _ in 0..tracks.len() {
        let Some(group) = parent.and_then(|p| tracks.get(p).map(|t| (p, t))) else {
            return false;
        };
        if group.1.mute {
            return false;
        }
        if group.0 == bus {
            return true;
        }
        parent = group.1.parent;
    }
    false
}

/// Whether a track with this output feeds anything at all: a
/// [`SENDS_ONLY`] one only with sends and returns.
fn feeds(output: &Output, sends: usize, returns: usize) -> bool {
    output.kind != SENDS_ONLY || (sends > 0 && returns > 0)
}

/// The proof of `set` (see the module's doc), or why there is none.
pub fn decide(set: &Set) -> Result<Proof, String> {
    let bus = the_one(&set.tracks, BUS)?;
    let stream = the_one(&set.tracks, STREAM_TRACK)?;
    let returns = set.returns.len();
    let mut open: Vec<String> = set
        .tracks
        .iter()
        .enumerate()
        .filter(|&(i, t)| {
            i != bus
                && !t.group
                && !t.mute
                && feeds(&t.output, t.sends, returns)
                && (t.output.kind == BUS || through_groups(&set.tracks, i, bus))
        })
        .map(|(_, t)| t.name.clone())
        .collect();
    open.extend(
        set.returns
            .iter()
            .filter(|r| !r.mute && r.output.kind == BUS)
            .map(|r| r.name.clone()),
    );
    let stream_to = set.tracks[stream].output.clone();
    let to_stream = |output: &Output| output.kind == STREAM_TRACK || *output == stream_to;
    let routed = |name: &str, output: &Output| Routed {
        name: name.to_string(),
        kind: output.kind.clone(),
        channel: output.channel.clone(),
    };
    let mut others: Vec<Routed> = set
        .tracks
        .iter()
        .enumerate()
        .filter(|&(i, t)| i != bus && i != stream && to_stream(&t.output))
        .map(|(_, t)| routed(&t.name, &t.output))
        .collect();
    others.extend(
        set.returns
            .iter()
            .filter(|r| to_stream(&r.output))
            .map(|r| routed(&r.name, &r.output)),
    );
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
