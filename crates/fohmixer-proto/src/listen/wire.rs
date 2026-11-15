//! The Listen device's small messages (#210): its announce and the hub's
//! select (OSC 1.0 datagrams on 127.0.0.1, as the LUFS devices', whose OSC
//! readers they share), and the hello that opens its audio connection (a
//! Jitter message chunk, [`super::jitnet`]).
//!
//! **Announce**, the device to the hub's UDP [`super::DEFAULT_PORT`] once a
//! second: address [`ANNOUNCE_ADDRESS`], type tags [`ANNOUNCE_TAGS`]:
//!
//! | # | type | value |
//! |---|---|---|
//! | 1 | `i` | the wire version, [`WIRE_VERSION`] |
//! | 2 | `i` | the device's Max id (`live.path this_device`) |
//! | 3 | `s` | `v<version.txt>` |
//! | 4–7 | `s` | its first four canonical parents' names, nearest first (`-` past the top) |
//! | 8 | `i` | Live's sample rate (`dspstate~`) |
//! | 9 | `i` | 1 while it streams, else 0 |
//!
//! **Select**, the hub to every device's UDP [`super::SELECT_PORT`], every
//! [`SELECT_EVERY_MS`] while a page listens: address [`SELECT_ADDRESS`],
//! `,ii`, the device id to stream (0: none) and the hub's turn (a number
//! that changes with each new track; the device does not read it, the log
//! does). A device streams while the newest select names its id, and stops
//! [`SELECT_HOLD_MS`] after the last one that did.
//!
//! **Hello**, the first message chunk on the device's TCP connection:
//! `fohmixer_listen <wire version> <device id> v<version> <sample rate>`.
//!
//! Pure: no I/O, slices only.

use crate::lufs::wire::{
    NO_PARENT, PARENTS, WireError, osc_i32, osc_string, push_string, track_and_set,
};

use super::jitnet::Atom;

/// The wire version of all three; a new field is a new version.
pub const WIRE_VERSION: i32 = 1;
/// The announce's address.
pub const ANNOUNCE_ADDRESS: &str = "/fohmixer/listen";
/// The announce's type tags.
pub const ANNOUNCE_TAGS: &str = ",iisssssii";
/// The select's address.
pub const SELECT_ADDRESS: &str = "/fohmixer/listen/select";
/// The select's type tags.
pub const SELECT_TAGS: &str = ",ii";
/// The hello's first word (its message's selector).
pub const HELLO_WORD: &str = "fohmixer_listen";
/// The longest datagram taken.
pub const MAX_DATAGRAM: usize = 512;
/// How often the hub selects the device while a page listens.
pub const SELECT_EVERY_MS: u64 = 500;
/// How long a device streams after the last select that named it.
pub const SELECT_HOLD_MS: u64 = 1500;
/// The device's announce period.
pub const ANNOUNCE_EVERY_MS: u64 = 1000;
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
}

impl Announce {
    /// Its track's and set's names ([`track_and_set`]).
    pub fn track_and_set(&self) -> Option<(&str, &str)> {
        track_and_set(&self.parents)
    }
}

/// One select.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Select {
    /// The device to stream; 0: none.
    pub device: i32,
    pub turn: i32,
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
    out
}

/// The datagram of a select.
pub fn encode_select(s: Select) -> Vec<u8> {
    let mut out = Vec::with_capacity(48);
    push_string(&mut out, SELECT_ADDRESS);
    push_string(&mut out, SELECT_TAGS);
    out.extend_from_slice(&s.device.to_be_bytes());
    out.extend_from_slice(&s.turn.to_be_bytes());
    out
}

/// Reads a select (the tests' devices).
pub fn parse_select(datagram: &[u8]) -> Result<Select, WireError> {
    if datagram.len() > MAX_DATAGRAM {
        return Err(WireError::TooLong);
    }
    let (address, rest) = osc_string(datagram)?;
    if address != SELECT_ADDRESS {
        return Err(WireError::Address);
    }
    let (tags, rest) = osc_string(rest)?;
    if tags != SELECT_TAGS {
        return Err(WireError::Tags);
    }
    let (device, rest) = osc_i32(rest)?;
    let (turn, rest) = osc_i32(rest)?;
    if !rest.is_empty() {
        return Err(WireError::Trailing);
    }
    Ok(Select { device, turn })
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
