//! Claps from the surface (#220): what the pages are told, what
//! `/api/status` shows, and the `FohMixer Claps` device's small messages.
//!
//! The device (`live-device/FohMixer-Claps/`, a Max for Live MIDI effect
//! first on the Claps track) plays the pattern itself on Live's transport
//! grid: whole beats, from the next bar, silent while Live stops. The hub
//! only decides what it plays (`fohmixer-hub` `rules/claps.rs`) and tells
//! it over loopback UDP (OSC 1.0, as the LUFS and Listen devices):
//!
//! **To the device**, UDP `127.0.0.1:<[claps] device_port>`
//! ([`DEVICE_PORT`]), at every change and again every [`SEND_EVERY_MS`]
//! (a reloaded device gets the state back; the device acts only on a
//! change):
//!
//! | address | tags | values |
//! |---|---|---|
//! | [`NOTES_ADDRESS`] | `,i…` | the pitches each clap plays, 1 to [`MAX_NOTES`] |
//! | [`PATTERN_ADDRESS`] | `,iiii` | beat 1 to 4 of the bar: 1 claps, 0 not |
//! | [`SILENT_ADDRESS`] | `,i` | 1 while an active clip of the track covers the playhead |
//! | [`ON_ADDRESS`] | `,i` | 1: play the pattern from the next bar on; 0: stop |
//! | [`POLL_ADDRESS`] | `,` | answer with the hello (#264), every [`SEND_EVERY_MS`], with or without a Claps track |
//!
//! **To the hub**, UDP `127.0.0.1:<[claps] port>` ([`HUB_PORT`]), as the
//! answer to each poll (#264: the device runs no clock of its own, its
//! `udpreceive @defer 1` answers in Live's main thread): [`HELLO_ADDRESS`],
//! [`HELLO_TAGS`]: the wire version ([`WIRE_VERSION`]), `v<version.txt>`
//! and 1 while it claps.
//!
//! Pure: no I/O, slices only.

use serde::{Deserialize, Serialize};

use crate::lufs::wire::{WireError, osc_i32, osc_string, push_string};

/// The hub's UDP port for the device's hello.
pub const HUB_PORT: u16 = 39113;
/// The device's UDP port for the hub's words.
pub const DEVICE_PORT: u16 = 39114;
/// The wire version of the hello; a new field is a new version.
pub const WIRE_VERSION: i32 = 1;
/// The hello's address.
pub const HELLO_ADDRESS: &str = "/fohmixer/claps";
/// The hello's type tags.
pub const HELLO_TAGS: &str = ",isi";
/// The pattern's address.
pub const PATTERN_ADDRESS: &str = "/fohmixer/claps/pattern";
/// The switch's address.
pub const ON_ADDRESS: &str = "/fohmixer/claps/on";
/// The clip silence's address.
pub const SILENT_ADDRESS: &str = "/fohmixer/claps/silent";
/// The pitches' address.
pub const NOTES_ADDRESS: &str = "/fohmixer/claps/notes";
/// The poll's address (#264): no argument; the device answers its hello.
pub const POLL_ADDRESS: &str = "/fohmixer/claps/poll";
/// The longest datagram taken.
pub const MAX_DATAGRAM: usize = 256;
/// How often the hub sends the device its whole state and its poll.
pub const SEND_EVERY_MS: u64 = 1000;
/// A device unheard this long is shown gone.
pub const FORGET_MS: u64 = 3000;
/// The beats of a bar (4/4 only).
pub const BEATS: usize = 4;
/// The most pitches a clap plays.
pub const MAX_NOTES: usize = 8;
/// The pitch a clap plays without `[claps] notes` (the lowest the Claps
/// track's clips use; the PC's own is set by hand at the placement).
pub const DEFAULT_NOTE: u8 = 50;

/// The beats of a bar that clap, beat 1 first.
pub type Pattern = [bool; BEATS];

/// What the Claps strip shows (`ServerMsg::Claps`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClapsPhase {
    /// Nothing claps (nothing lit).
    #[default]
    Off,
    /// The engineer taps; a bar with no tap ends it.
    Tapping,
    /// The pattern is set: Live claps from the next bar.
    NextBar,
    /// Live claps the pattern.
    On,
    /// On, but an active clip of the track covers the playhead: the clip
    /// claps, the device is silent (never twice).
    Clip,
    /// On, but Live stops: nothing claps until it plays.
    Waits,
    /// Live's time signature is not 4/4: nothing claps.
    Signature,
}

/// The Claps strip's state, told to every page at attach and on every
/// change: the phase, the pattern (the taps while tapping), and Live's
/// playhead as the hub last heard it (`beat`, in beats, with the `tempo`;
/// none while Live stops or before it was heard) so the page moves its
/// "now" ring between messages; a jump or a tempo change is told again.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClapsState {
    pub state: ClapsPhase,
    pub pattern: Pattern,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tempo: Option<f64>,
}

/// The device as the hub last heard it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClapsDevice {
    pub version: String,
    /// Whether it claps (its own word).
    pub running: bool,
    /// How long ago it was heard (ms).
    pub age_ms: u64,
}

/// What the device is told (`/api/status`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClapsWords {
    pub pattern: Pattern,
    pub on: bool,
    pub silent: bool,
    pub notes: Vec<u8>,
}

/// Claps in `/api/status`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ClapsStatus {
    /// The Claps track (`<instance> <label>`); none without a `+CLAPS`.
    #[serde(default)]
    pub track: Option<String>,
    #[serde(default)]
    pub state: ClapsPhase,
    /// What the device is told now.
    #[serde(default)]
    pub words: ClapsWords,
    /// The device, once heard.
    #[serde(default)]
    pub device: Option<ClapsDevice>,
    /// The taps taken and refused since the start.
    #[serde(default)]
    pub taps: u64,
    #[serde(default)]
    pub refused: u64,
    /// The track's arrangement clips as last read, and the active ones.
    #[serde(default)]
    pub clips: usize,
    #[serde(default)]
    pub active_clips: usize,
    /// Why the clips are not known (the last read's error).
    #[serde(default)]
    pub clips_error: Option<String>,
    /// Live's time signature as heard (`4/4`).
    #[serde(default)]
    pub signature: Option<String>,
    /// The datagrams sent to the device (its words; the polls apart).
    #[serde(default)]
    pub sent: u64,
    /// The polls sent to the device (#264).
    #[serde(default)]
    pub polls: u64,
}

/// One word to the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Word {
    Notes(Vec<u8>),
    Pattern(Pattern),
    Silent(bool),
    On(bool),
    /// The poll (#264): the device answers with its hello.
    Poll,
}

/// The device's hello.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    pub version: String,
    pub running: bool,
}

/// The datagram of a word.
pub fn encode(word: &Word) -> Vec<u8> {
    let (address, values): (&str, Vec<i32>) = match word {
        Word::Notes(notes) => (NOTES_ADDRESS, notes.iter().map(|&n| i32::from(n)).collect()),
        Word::Pattern(pattern) => (
            PATTERN_ADDRESS,
            pattern.iter().map(|&b| i32::from(b)).collect(),
        ),
        Word::Silent(silent) => (SILENT_ADDRESS, vec![i32::from(*silent)]),
        Word::On(on) => (ON_ADDRESS, vec![i32::from(*on)]),
        Word::Poll => (POLL_ADDRESS, Vec::new()),
    };
    let mut out = Vec::with_capacity(64);
    push_string(&mut out, address);
    push_string(&mut out, &format!(",{}", "i".repeat(values.len())));
    for value in values {
        out.extend_from_slice(&value.to_be_bytes());
    }
    out
}

/// The ints after a datagram's `,i…` tags.
fn ints(rest: &[u8], tags: &str) -> Result<Vec<i32>, WireError> {
    let count = tags
        .strip_prefix(',')
        .filter(|t| t.bytes().all(|b| b == b'i'))
        .ok_or(WireError::Tags)?
        .len();
    let mut values = Vec::with_capacity(count);
    let mut rest = rest;
    for _ in 0..count {
        let (value, after) = osc_i32(rest)?;
        values.push(value);
        rest = after;
    }
    if !rest.is_empty() {
        return Err(WireError::Trailing);
    }
    Ok(values)
}

/// A 0 or 1.
fn flag(value: i32) -> Result<bool, WireError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(WireError::Range),
    }
}

/// Reads a word (the tests' devices).
pub fn parse(datagram: &[u8]) -> Result<Word, WireError> {
    if datagram.len() > MAX_DATAGRAM {
        return Err(WireError::TooLong);
    }
    let (address, rest) = osc_string(datagram)?;
    let (tags, rest) = osc_string(rest)?;
    let values = ints(rest, tags)?;
    match (address, values.as_slice()) {
        (NOTES_ADDRESS, notes) if (1..=MAX_NOTES).contains(&notes.len()) => notes
            .iter()
            .map(|&n| {
                u8::try_from(n)
                    .ok()
                    .filter(|n| *n < 128)
                    .ok_or(WireError::Range)
            })
            .collect::<Result<Vec<u8>, WireError>>()
            .map(Word::Notes),
        (PATTERN_ADDRESS, &[a, b, c, d]) => {
            Ok(Word::Pattern([flag(a)?, flag(b)?, flag(c)?, flag(d)?]))
        }
        (SILENT_ADDRESS, &[v]) => Ok(Word::Silent(flag(v)?)),
        (ON_ADDRESS, &[v]) => Ok(Word::On(flag(v)?)),
        (POLL_ADDRESS, &[]) => Ok(Word::Poll),
        (NOTES_ADDRESS | PATTERN_ADDRESS | SILENT_ADDRESS | ON_ADDRESS | POLL_ADDRESS, _) => {
            Err(WireError::Tags)
        }
        _ => Err(WireError::Address),
    }
}

/// The datagram of a hello (the tests' devices).
pub fn encode_hello(hello: &Hello) -> Vec<u8> {
    let mut out = Vec::with_capacity(48);
    push_string(&mut out, HELLO_ADDRESS);
    push_string(&mut out, HELLO_TAGS);
    out.extend_from_slice(&WIRE_VERSION.to_be_bytes());
    push_string(&mut out, &hello.version);
    out.extend_from_slice(&i32::from(hello.running).to_be_bytes());
    out
}

/// Reads a hello.
pub fn parse_hello(datagram: &[u8]) -> Result<Hello, WireError> {
    if datagram.len() > MAX_DATAGRAM {
        return Err(WireError::TooLong);
    }
    let (address, rest) = osc_string(datagram)?;
    if address != HELLO_ADDRESS {
        return Err(WireError::Address);
    }
    let (tags, rest) = osc_string(rest)?;
    if tags != HELLO_TAGS {
        return Err(WireError::Tags);
    }
    let (wire, rest) = osc_i32(rest)?;
    if wire != WIRE_VERSION {
        return Err(WireError::Version);
    }
    let (version, rest) = osc_string(rest)?;
    let (running, rest) = osc_i32(rest)?;
    if !rest.is_empty() {
        return Err(WireError::Trailing);
    }
    Ok(Hello {
        version: version.to_string(),
        running: flag(running)?,
    })
}

/// The words that carry a whole state, in the order the device takes them
/// (the pattern before the switch that starts it).
pub fn words_of(words: &ClapsWords) -> Vec<Word> {
    vec![
        Word::Notes(words.notes.clone()),
        Word::Pattern(words.pattern),
        Word::Silent(words.silent),
        Word::On(words.on),
    ]
}

#[cfg(test)]
mod tests;
