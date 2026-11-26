//! The Listen device's small messages (#210): its announce (an OSC 1.0
//! datagram on 127.0.0.1, as the LUFS devices', whose OSC readers it
//! shares), the hub's polls (#264) and the hello that opens its audio
//! connection (a Jitter message chunk, [`super::jitnet`]).
//!
//! **Announce**, the device to the hub's UDP [`super::DEFAULT_PORT`], as
//! the answer to each of the hub's polls, at its load and when its Device
//! On turns 1: address [`ANNOUNCE_ADDRESS`], type tags [`ANNOUNCE_TAGS`]:
//!
//! | # | type | value |
//! |---|---|---|
//! | 1 | `i` | the wire version, [`WIRE_VERSION`] |
//! | 2 | `i` | the device's listen id: its Max id (`live.path this_device`, per Live process) mixed with a random number drawn at load, so the two Lives on the PC never share one |
//! | 3 | `s` | `v<version.txt>` |
//! | 4–7 | `s` | its first four canonical parents' names, nearest first (`-` past the top) |
//! | 8 | `i` | Live's sample rate (`dspstate~`) |
//! | 9 | `i` | 1 while it streams, else 0 |
//! | 10 | `i` | the port it takes the hub's polls on, drawn at load from the LUFS devices' range (`lufs::wire::poll_port`) |
//!
//! **The polls (#264, the lane's design in comment 7184226996 on #264):**
//! the device holds no clock (a Max for Live clock runs in Live's audio
//! thread and posts from there). The hub sends each device it heard
//! [`poll_datagram`] (`/fohmixer/listen/poll ,i <on>`) every [`POLL_MS`] on
//! its port: the select pushed, 1 only for the device of the track
//! listened to; the device turns on (its hello, then its frame gate opens)
//! or off at once, and answers with its announce. While a device is on the
//! hub sends it [`frame_datagram`] (`/fohmixer/listen/frame`) every
//! [`FRAME_POLL_MS`]: each bangs its `jit.catch~ @mode 1`, which hands out
//! every whole frame caught since the bang before, so a late poll only
//! puts more frames in one matrix. The patch's `udpreceive @defer 1` takes
//! both in Max's main thread. A device unheard for [`FORGET_MS`] is
//! forgotten, polled no more; its parents are read again every
//! [`REREAD_POLLS`]th poll. Version 1 (#210: an announce each second, the
//! select asked over HTTP) is refused.
//!
//! **Hello**, the first message chunk on the device's TCP connection, sent
//! again with each announce while it is on:
//! `fohmixer_listen <wire version> <device id> v<version> <sample rate>`.
//!
//! Pure: no I/O, slices only.

use crate::lufs::wire::{
    NO_PARENT, PARENTS, WireError, osc_i32, osc_string, poll_port, push_string, track_and_set,
};

use super::jitnet::Atom;

/// The wire version of the announce and the hello; a new field is a new
/// version (2: the poll port, #264).
pub const WIRE_VERSION: i32 = 2;
/// The announce's address.
pub const ANNOUNCE_ADDRESS: &str = "/fohmixer/listen";
/// The announce's type tags.
pub const ANNOUNCE_TAGS: &str = ",iisssssiii";
/// The hub's poll of a device: its select (#264).
pub const POLL_ADDRESS: &str = "/fohmixer/listen/poll";
/// The hub's frame poll of the device it selected (#264).
pub const FRAME_ADDRESS: &str = "/fohmixer/listen/frame";
/// The hello's first word (its message's selector).
pub const HELLO_WORD: &str = "fohmixer_listen";
/// The longest datagram taken.
pub const MAX_DATAGRAM: usize = 512;
/// How often the hub polls each device it heard (4 a second).
pub const POLL_MS: u64 = 250;
/// How often the hub sends the selected device its frame poll: one 20 ms
/// frame a poll on average.
pub const FRAME_POLL_MS: u64 = 20;
/// The device reads its parents again every this many polls (10 s).
pub const REREAD_POLLS: u64 = 40;
/// A device unheard this long is forgotten.
pub const FORGET_MS: u64 = 3000;

/// One announce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announce {
    pub device: i32,
    pub version: String,
    pub parents: [String; PARENTS],
    pub sample_rate: i32,
    pub sending: bool,
    /// The port it takes the hub's polls on.
    pub port: u16,
}

impl Announce {
    /// Its track's and set's names ([`track_and_set`]).
    pub fn track_and_set(&self) -> Option<(&str, &str)> {
        track_and_set(&self.parents)
    }
}

/// The hello of an audio connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    pub device: i32,
    pub version: String,
    pub sample_rate: i32,
}

/// Reads an announce.
pub fn parse_announce(datagram: &[u8]) -> Result<Announce, WireError> {
    if datagram.len() > MAX_DATAGRAM {
        return Err(WireError::TooLong);
    }
    let (address, rest) = osc_string(datagram)?;
    if address != ANNOUNCE_ADDRESS {
        return Err(WireError::Address);
    }
    let (tags, rest) = osc_string(rest)?;
    if tags != ANNOUNCE_TAGS {
        return Err(WireError::Tags);
    }
    let (wire, rest) = osc_i32(rest)?;
    if wire != WIRE_VERSION {
        return Err(WireError::Version);
    }
    let (device, rest) = osc_i32(rest)?;
    let (version, rest) = osc_string(rest)?;
    let (p0, rest) = osc_string(rest)?;
    let (p1, rest) = osc_string(rest)?;
    let (p2, rest) = osc_string(rest)?;
    let (p3, rest) = osc_string(rest)?;
    let (sample_rate, rest) = osc_i32(rest)?;
    let (sending, rest) = osc_i32(rest)?;
    let (port, rest) = osc_i32(rest)?;
    if !rest.is_empty() {
        return Err(WireError::Trailing);
    }
    if sample_rate < 0 || !(0..=1).contains(&sending) {
        return Err(WireError::Range);
    }
    let announce = Announce {
        device,
        version: version.to_string(),
        parents: [p0, p1, p2, p3].map(str::to_string),
        sample_rate,
        sending: sending == 1,
        port: poll_port(port)?,
    };
    if announce.track_and_set().is_none() {
        return Err(WireError::Identity);
    }
    Ok(announce)
}

/// The datagram of an announce (the tests' sender; SimLive pins the same
/// bytes).
pub fn encode_announce(a: &Announce) -> Vec<u8> {
    let mut out = Vec::with_capacity(MAX_DATAGRAM);
    push_string(&mut out, ANNOUNCE_ADDRESS);
    push_string(&mut out, ANNOUNCE_TAGS);
    out.extend_from_slice(&WIRE_VERSION.to_be_bytes());
    out.extend_from_slice(&a.device.to_be_bytes());
    push_string(&mut out, &a.version);
    for parent in &a.parents {
        push_string(&mut out, parent);
    }
    out.extend_from_slice(&a.sample_rate.to_be_bytes());
    out.extend_from_slice(&i32::from(a.sending).to_be_bytes());
    out.extend_from_slice(&i32::from(a.port).to_be_bytes());
    out
}

/// The hub's poll of a device (#264): [`POLL_ADDRESS`] with one int, 1
/// when it is the device selected, else 0 (the patch's `route` hands it
/// the int, `sel 1` reads it).
pub fn poll_datagram(on: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    push_string(&mut out, POLL_ADDRESS);
    push_string(&mut out, ",i");
    out.extend_from_slice(&i32::from(on).to_be_bytes());
    out
}

/// The hub's frame poll (#264): [`FRAME_ADDRESS`] with no argument (the
/// patch's `route` hands it a bang into the frame gate).
pub fn frame_datagram() -> Vec<u8> {
    let mut out = Vec::with_capacity(32);
    push_string(&mut out, FRAME_ADDRESS);
    push_string(&mut out, ",");
    out
}

/// A whole number from a Max atom: a long, or a float with no fraction
/// (Max's `dspstate~` gives the sample rate as a float).
fn whole(atom: &Atom) -> Option<i32> {
    match atom {
        Atom::Long(n) => Some(*n),
        Atom::Float(f) if f.fract() == 0.0 && f.abs() < 1e9 => {
            // A float with no fraction under 1e9 fits an i32.
            Some(*f as i32)
        }
        _ => None,
    }
}

/// Reads a hello from a message chunk's atoms; none for any other message.
pub fn parse_hello(atoms: &[Atom]) -> Option<Hello> {
    let [Atom::Sym(word), wire, device, Atom::Sym(version), rate] = atoms else {
        return None;
    };
    if word != HELLO_WORD || whole(wire)? != WIRE_VERSION {
        return None;
    }
    Some(Hello {
        device: whole(device)?,
        version: version.clone(),
        sample_rate: whole(rate)?,
    })
}

/// The hello's atoms (the tests' devices).
pub fn hello_atoms(h: &Hello) -> Vec<Atom> {
    vec![
        Atom::Sym(HELLO_WORD.to_string()),
        Atom::Long(WIRE_VERSION),
        Atom::Long(h.device),
        Atom::Sym(h.version.clone()),
        Atom::Long(h.sample_rate),
    ]
}

/// The parents of a device on a track (`[track, set, -, -]`), in a rack's
/// chain (`[chain, rack, track, set]`): the tests' helper.
pub fn parents_in(chain: Option<(&str, &str)>, track: &str, set: &str) -> [String; PARENTS] {
    match chain {
        Some((chain, rack)) => [chain, rack, track, set].map(str::to_string),
        None => [track, set, NO_PARENT, NO_PARENT].map(str::to_string),
    }
}

#[cfg(test)]
mod tests;
