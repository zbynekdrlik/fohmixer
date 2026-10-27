//! The LUFS devices' datagram (#151, spec §4.4): one OSC 1.0 message per
//! reading, sent by Max's `udpsend` to the hub on `127.0.0.1`.
//!
//! Address [`LUFS_ADDRESS`] or [`METER_ADDRESS`], type tags [`TYPE_TAGS`]:
//!
//! | # | type | value |
//! |---|---|---|
//! | 1 | `i` | the wire version, [`WIRE_VERSION`] |
//! | 2 | `i` | the device's Max id (`live.path this_device`, per Live process) |
//! | 3 | `s` | the device's version, `v<version.txt>` |
//! | 4–7 | `s` | the names of the device's first four canonical parents, nearest first ([`NO_PARENT`] past the top) |
//! | 8 | `i` | `/lufs`: `S` in tenths of LU (−700…0); `/meter`: the L peak in tenths of dBFS (−700…120) |
//! | 9 | `i` | `/lufs`: `V` in tenths (−700…0); `/meter`: the R peak |
//!
//! Max's `live.path` gives no absolute path of a device (its `getpath`
//! echoes the path it was given: #151's probe of 27.10), so the device names
//! its parents instead: on a track `[track, set, -, -]`, in a rack's chain
//! `[chain, rack, track, set]`. The last named parent is the set (which
//! tells the band's Live from the master's), the one before it the track
//! ([`track_and_set`]); the hub confirms the device with its own read.
//!
//! Pure: no I/O. The parser reads with slices only (`split_at`,
//! `iter().position`), never a hand-kept index.

/// The hub's default port, fixed in the devices' patches
/// (`scripts/check_lufs_device.py` keeps them equal).
pub const DEFAULT_PORT: u16 = 39110;
/// The wire version the devices send; a new field is a new version.
pub const WIRE_VERSION: i32 = 1;
/// The longest datagram the hub takes.
pub const MAX_DATAGRAM: usize = 512;
/// The LUFS device's address.
pub const LUFS_ADDRESS: &str = "/fohmixer/lufs";
/// The meter device's address.
pub const METER_ADDRESS: &str = "/fohmixer/meter";
/// The one shape of the message.
pub const TYPE_TAGS: &str = ",iisssssii";
/// A parent the device could not name (past the set).
pub const NO_PARENT: &str = "-";
/// The parents a datagram names.
pub const PARENTS: usize = 4;
/// The lowest value either device sends (−70).
pub const FLOOR_TENTHS: i16 = -700;
/// The highest LUFS reading (0 LUFS).
pub const LUFS_TOP_TENTHS: i16 = 0;
/// The highest meter reading (+12 dBFS).
pub const METER_TOP_TENTHS: i16 = 120;

/// One `/fohmixer/lufs` datagram: device 7, `v1.19.0`, in the chain
/// `LUFS IN` of the rack `CORE Effects` on `Vox 1` in the set `FOH Band`,
/// `S` −25.3 and `V` −24.8. SimLive's encoder pins the same bytes.
pub const GOLDEN_HEX: &str = "2f666f686d697865722f6c75667300002c6969737373737369690000\
00000001000000077631\
2e31392e30004c55465320494e00434f5245204566666563747300000000566f782031000000464f482042\
616e6400000000ffffff03ffffff08";

/// Which device sent a datagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DeviceKind {
    /// `FohMixer LUFS`: `S` and `V`.
    Lufs,
    /// `FohMixer Meter`: the L and R peaks.
    Meter,
}

impl DeviceKind {
    /// The address it sends to.
    pub fn address(self) -> &'static str {
        match self {
            DeviceKind::Lufs => LUFS_ADDRESS,
            DeviceKind::Meter => METER_ADDRESS,
        }
    }

    /// The highest value it sends, in tenths.
    pub fn top(self) -> i16 {
        match self {
            DeviceKind::Lufs => LUFS_TOP_TENTHS,
            DeviceKind::Meter => METER_TOP_TENTHS,
        }
    }
}

/// One reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub kind: DeviceKind,
    /// The device's Max id (per Live process).
    pub device: i32,
    /// The device's version, `v<version>`.
    pub version: String,
    /// Its first four canonical parents' names, nearest first.
    pub parents: [String; PARENTS],
    /// `S` or the L peak, in tenths.
    pub first: i16,
    /// `V` or the R peak, in tenths.
    pub second: i16,
}

impl Reading {
    /// Its track's and its set's names ([`track_and_set`]).
    pub fn track_and_set(&self) -> Option<(&str, &str)> {
        track_and_set(&self.parents)
    }
}

/// Why a datagram was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WireError {
    /// Over [`MAX_DATAGRAM`] bytes.
    TooLong,
    /// It ends inside a field, or a string's padding is not NULs to four.
    Truncated,
    /// Bytes after the last field.
    Trailing,
    /// Another address.
    Address,
    /// Other type tags.
    Tags,
    /// Another wire version.
    Version,
    /// A string that is not UTF-8.
    Utf8,
    /// A value out of its device's range.
    Range,
    /// Fewer than two named parents: no track and set.
    Identity,
}

impl WireError {
    /// Its word for the hub's counters.
    pub fn reason(self) -> &'static str {
        match self {
            WireError::TooLong => "too_long",
            WireError::Truncated => "truncated",
            WireError::Trailing => "trailing",
            WireError::Address => "address",
            WireError::Tags => "tags",
            WireError::Version => "version",
            WireError::Utf8 => "utf8",
            WireError::Range => "range",
            WireError::Identity => "identity",
        }
    }
}

/// The track's and the set's names among a device's parents (nearest
/// first): the last of the named ones (those before the first
/// [`NO_PARENT`]) is the set, the one before it the track. None with fewer
/// than two named. A device nested deeper than one rack names no set among
/// four; the hub then finds no Live set of that name and matches nothing.
pub fn track_and_set(parents: &[String]) -> Option<(&str, &str)> {
    let named = parents
        .iter()
        .take_while(|p| p.as_str() != NO_PARENT)
        .count();
    match parents.get(..named)? {
        [.., track, set] => Some((track.as_str(), set.as_str())),
        _ => None,
    }
}

/// An OSC string at the start of `buf`: its text and the rest after its
/// padding (the NUL and up to three more, to a multiple of four).
fn osc_string(buf: &[u8]) -> Result<(&str, &[u8]), WireError> {
    let len = buf
        .iter()
        .position(|&b| b == 0)
        .ok_or(WireError::Truncated)?;
    let padded = (len + 4) & !3;
    if padded > buf.len() {
        return Err(WireError::Truncated);
    }
    let (field, rest) = buf.split_at(padded);
    let (text, padding) = field.split_at(len);
    if padding.iter().any(|&b| b != 0) {
        return Err(WireError::Truncated);
    }
    let text = std::str::from_utf8(text).map_err(|_| WireError::Utf8)?;
    Ok((text, rest))
}

/// A big-endian OSC int at the start of `buf`, and the rest.
fn osc_i32(buf: &[u8]) -> Result<(i32, &[u8]), WireError> {
    let (head, rest) = buf.split_at_checked(4).ok_or(WireError::Truncated)?;
    let bytes = <[u8; 4]>::try_from(head).map_err(|_| WireError::Truncated)?;
    Ok((i32::from_be_bytes(bytes), rest))
}

/// A reading's value, in its device's range.
fn value(kind: DeviceKind, raw: i32) -> Result<i16, WireError> {
    i16::try_from(raw)
        .ok()
        .filter(|v| (FLOOR_TENTHS..=kind.top()).contains(v))
        .ok_or(WireError::Range)
}

/// Reads one datagram.
pub fn parse(datagram: &[u8]) -> Result<Reading, WireError> {
    if datagram.len() > MAX_DATAGRAM {
        return Err(WireError::TooLong);
    }
    let (address, rest) = osc_string(datagram)?;
    let kind = match address {
        LUFS_ADDRESS => DeviceKind::Lufs,
        METER_ADDRESS => DeviceKind::Meter,
        _ => return Err(WireError::Address),
    };
    let (tags, rest) = osc_string(rest)?;
    if tags != TYPE_TAGS {
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
    let (first, rest) = osc_i32(rest)?;
    let (second, rest) = osc_i32(rest)?;
    if !rest.is_empty() {
        return Err(WireError::Trailing);
    }
    let reading = Reading {
        kind,
        device,
        version: version.to_string(),
        parents: [p0, p1, p2, p3].map(str::to_string),
        first: value(kind, first)?,
        second: value(kind, second)?,
    };
    if reading.track_and_set().is_none() {
        return Err(WireError::Identity);
    }
    Ok(reading)
}

/// Appends an OSC string: its bytes, a NUL and the padding to four.
fn push_string(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
    let padded = (text.len() + 4) & !3;
    out.resize(out.len() + padded - text.len(), 0);
}

/// The datagram of a reading (the tests' and the hub tests' sender).
pub fn encode(r: &Reading) -> Vec<u8> {
    let mut out = Vec::with_capacity(MAX_DATAGRAM);
    push_string(&mut out, r.kind.address());
    push_string(&mut out, TYPE_TAGS);
    out.extend_from_slice(&WIRE_VERSION.to_be_bytes());
    out.extend_from_slice(&r.device.to_be_bytes());
    push_string(&mut out, &r.version);
    for parent in &r.parents {
        push_string(&mut out, parent);
    }
    out.extend_from_slice(&i32::from(r.first).to_be_bytes());
    out.extend_from_slice(&i32::from(r.second).to_be_bytes());
    out
}

#[cfg(test)]
mod tests;
