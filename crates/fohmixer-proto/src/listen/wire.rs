//! The Listen device's small messages (#210): its announce (an OSC 1.0
//! datagram on 127.0.0.1, as the LUFS devices', whose OSC readers it
//! shares), the select it asks the hub for over HTTP, and the hello that
//! opens its audio connection (a Jitter message chunk, [`super::jitnet`]).
//!
//! **Announce**, the device to the hub's UDP [`super::DEFAULT_PORT`] once a
//! second: address [`ANNOUNCE_ADDRESS`], type tags [`ANNOUNCE_TAGS`]:
//!
//! | # | type | value |
//! |---|---|---|
//! | 1 | `i` | the wire version, [`WIRE_VERSION`] |
//! | 2 | `i` | the device's listen id: its Max id (`live.path this_device`, per Live process) mixed with a random number drawn at load, so the two Lives on the PC never share one |
//! | 3 | `s` | `v<version.txt>` |
//! | 4–7 | `s` | its first four canonical parents' names, nearest first (`-` past the top) |
//! | 8 | `i` | Live's sample rate (`dspstate~`) |
//! | 9 | `i` | 1 while it streams, else 0 |
//!
//! **Select**, asked by each device every [`POLL_MS`] (Max's `maxurl`):
//! `GET http://127.0.0.1:<hub HTTP port>`[`SELECT_PATH`]`?device=<listen
//! id>`, answered [`select_body`] (`{"on":true}` or `{"on":false}`; the
//! patch asks with `parse_type json` and reads `body` then `on`: Max's dict
//! holds JSON's `true` as 1, which its `sel 1` takes), to loopback peers
//! only and with no token. One UDP select port for every device failed on the PC
//! (#210, P1: the devices that bind one port share one socket, and only one
//! of them hears a datagram), so each device asks for itself. `on` true
//! starts the stream, `on` false stops it at once; an answer without `on`
//! (an error answer, a body that is no JSON) or a failed request is no
//! answer, so a device whose polls go unanswered stops [`SELECT_HOLD_MS`]
//! after its last `on` (a failed request is off within the hold).
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
/// The hub's route a device asks whether it is selected.
pub const SELECT_PATH: &str = "/api/listen/select";
/// The hello's first word (its message's selector).
pub const HELLO_WORD: &str = "fohmixer_listen";
/// The longest datagram taken.
pub const MAX_DATAGRAM: usize = 512;
/// How often a device asks the hub whether it is selected.
pub const POLL_MS: u64 = 250;
/// How long a device streams after the last `on` answer (four polls).
pub const SELECT_HOLD_MS: u64 = 1000;
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

/// The select route's answer, compact JSON (the device's patch parses it
/// into a dictionary and takes `on`).
pub fn select_body(on: bool) -> &'static str {
    if on {
        r#"{"on":true}"#
    } else {
        r#"{"on":false}"#
    }
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
