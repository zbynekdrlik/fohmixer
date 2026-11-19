//! The LUFS history (#219, the design on the ticket): pure. Once a second
//! each served loudness point's reading as its strip shows it
//! (`LufsItem.tenths`), the last [`RING_S`] seconds kept in RAM per point,
//! a minute's readings as one line for the day files (`lufs_history.rs`),
//! a line read back, and a window's values bucketed for a page.
//!
//! - A slot is the reading in tenths, [`SILENT_TENTHS`] for silence (and
//!   anything under it), [`HOLE`] for a second with no reading (no device,
//!   stale, Live offline). Both draw as a gap.
//! - A minute line: `{"m": <minute start, UTC ms>, "p": [{"i": <instance>,
//!   "t": <target>, "v": <base64 of 60 i16 LE>, "f": <the same of the
//!   strip's fader>}]}`; a point-minute with no sample above silence is
//!   left out, a minute with none is no line. `f` (#253): the strip's fader
//!   each second in dB tenths (`fohmixer_proto::lufs::fader_tenths`),
//!   [`HOLE`] while unknown, left out when the whole minute is unknown; a
//!   line without it (an older hub's) reads as no fader, and an older hub
//!   reads a newer line (it ignores `f`).
//! - A bucket ([`bucket_value`]): the median of its samples above silence
//!   (the upper middle), silence when half or more are silent, none when it
//!   holds no sample; its fader ([`fader_value`]) the median of its known
//!   fader seconds.

use std::collections::{BTreeMap, VecDeque};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
pub use fohmixer_proto::lufs::HistorySeries as Series;
use fohmixer_proto::lufs::{SILENT_TENTHS, bucket_of, median};
use serde::{Deserialize, Serialize};

/// Seconds kept in RAM per point: the last hour.
pub const RING_S: usize = 3600;
/// A second with no reading.
pub const HOLE: i16 = i16::MIN;
/// Seconds in a minute line.
pub const MINUTE_S: usize = 60;
/// A minute in ms.
pub const MINUTE_MS: i64 = 60_000;
/// The most values a window is bucketed to.
pub const MAX_VALUES: usize = 900;
/// The longest window a page may ask for: 24 h.
pub const MAX_SPAN_MS: i64 = 86_400_000;

/// A loudness point: its strip's instance and track target.
pub type Point = (String, String);

/// The whole second of `ms` (the hub's UTC clock).
pub fn second_of(ms: i64) -> i64 {
    ms - ms.rem_euclid(1000)
}

/// The minute `ms` lies in.
pub fn minute_of(ms: i64) -> i64 {
    ms - ms.rem_euclid(MINUTE_MS)
}

/// A point's slot of a reading: none is a hole, silence and under it
/// [`SILENT_TENTHS`].
pub fn slot_of(tenths: Option<i16>) -> i16 {
    tenths.map_or(HOLE, |t| t.max(SILENT_TENTHS))
}

/// A point's fader slot: its dB in tenths, none a hole.
pub fn fader_slot(tenths: Option<i16>) -> i16 {
    tenths.unwrap_or(HOLE)
}

/// One second of a point (#253): its reading and its strip's fader (dB
/// tenths, `fohmixer_proto::lufs::fader_tenths`), each none when unknown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sample {
    pub point: Point,
    pub tenths: Option<i16>,
    pub fader: Option<i16>,
}

/// Whether a slot holds a sample above silence.
pub fn sounds(slot: i16) -> bool {
    slot > SILENT_TENTHS
}

/// Whether a slot holds a reading (silence included).
pub fn heard(slot: i16) -> bool {
    slot != HOLE
}

/// The slots as base64 of i16 little-endian.
pub fn encode(slots: &[i16]) -> String {
    let bytes: Vec<u8> = slots.iter().flat_map(|s| s.to_le_bytes()).collect();
    STANDARD.encode(bytes)
}

/// A minute's slots from base64; none unless exactly [`MINUTE_S`] of them.
pub fn decode(text: &str) -> Option<Vec<i16>> {
    let bytes = STANDARD.decode(text).ok()?;
    if bytes.len() != MINUTE_S * 2 {
        return None;
    }
    Some(
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| i16::from_le_bytes(*b))
            .collect(),
    )
}

/// One point's minute in a line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PointMinute {
    #[serde(rename = "i")]
    pub instance: String,
    #[serde(rename = "t")]
    pub target: String,
    /// The slots, base64 ([`encode`]).
    #[serde(rename = "v")]
    pub values: String,
    /// The fader's slots, base64 (#253); none when the minute knows none.
    #[serde(rename = "f", default, skip_serializing_if = "Option::is_none")]
    pub faders: Option<String>,
}

/// A minute line of the day files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinuteLine {
    /// The minute's start, UTC ms.
    pub m: i64,
    #[serde(rename = "p")]
    pub points: Vec<PointMinute>,
}

impl MinuteLine {
    /// The line as written (no `\n`).
    pub fn text(&self) -> String {
        serde_json::to_string(self).expect("a minute line serializes")
    }

    /// A line read back; none for one that does not parse.
    pub fn parse(line: &str) -> Option<Self> {
        serde_json::from_str(line).ok()
    }

    /// The slots of `point` in this line; none when it is not there or its
    /// values do not decode.
    pub fn slots(&self, instance: &str, target: &str) -> Option<Vec<i16>> {
        self.points
            .iter()
            .find(|p| p.instance == instance && p.target == target)
            .and_then(|p| decode(&p.values))
    }

    /// The fader's slots of `point` in this line (#253); none when the point
    /// is not there, its minute holds no fader (an older hub's line) or
    /// they do not decode.
    pub fn fader_slots(&self, instance: &str, target: &str) -> Option<Vec<i16>> {
        self.points
            .iter()
            .find(|p| p.instance == instance && p.target == target)
            .and_then(|p| p.faders.as_deref())
            .and_then(decode)
    }
}

/// The first and last second with a reading in a minute starting at `m`.
pub fn span_of(m: i64, slots: &[i16]) -> Option<(i64, i64)> {
    let first = slots.iter().position(|s| heard(*s))?;
    let last = slots.iter().rposition(|s| heard(*s))?;
    Some((m + first as i64 * 1000, m + last as i64 * 1000))
}

/// The readings of the last hour, per point, once a second.
#[derive(Debug, Default)]
pub struct Recorder {
    /// The last second sampled.
    last: Option<i64>,
    /// The first second sampled since the start.
    first: Option<i64>,
    /// Each point's slots (the reading, the fader), the back at `last`.
    rings: BTreeMap<Point, VecDeque<(i16, i16)>>,
}

impl Recorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether `ms` falls in a second not sampled yet.
    pub fn due(&self, ms: i64) -> bool {
        self.last.is_none_or(|last| second_of(ms) > last)
    }

    /// Samples every point's reading and fader at the second of `ms`
    /// (`items`: each served point; a point the hub kept but no longer
    /// serves gets a hole). A second already sampled, or one before it (a
    /// clock stepped back), samples nothing. Gives the line of the minute
    /// that ended, if any.
    pub fn sample(&mut self, ms: i64, items: &[Sample]) -> Option<MinuteLine> {
        let second = second_of(ms);
        if !self.due(ms) {
            return None;
        }
        let ended = self
            .last
            .filter(|last| minute_of(*last) != minute_of(second))
            .and_then(|last| self.line(minute_of(last)));
        let holes = self.last.map_or(0, |last| gap_holes(last, second));
        for (point, ring) in &mut self.rings {
            let slot = items
                .iter()
                .find(|s| &s.point == point)
                .map_or((HOLE, HOLE), slots_of);
            push(ring, holes, slot);
        }
        for sample in items {
            if !self.rings.contains_key(&sample.point) {
                let mut ring = VecDeque::new();
                ring.push_back(slots_of(sample));
                self.rings.insert(sample.point.clone(), ring);
            }
        }
        // A point no longer served whose hour holds nothing is let go.
        self.rings.retain(|point, ring| {
            items.iter().any(|s| &s.point == point) || ring.iter().any(|s| heard(s.0))
        });
        self.last = Some(second);
        self.first.get_or_insert(second);
        ended
    }

    /// The slot of `point` at the second `t` (a hole outside the hour).
    pub fn at(&self, point: &Point, t: i64) -> i16 {
        self.pair_at(point, t).0
    }

    /// The fader's slot of `point` at the second `t` (#253; a hole outside
    /// the hour).
    pub fn fader_at(&self, point: &Point, t: i64) -> i16 {
        self.pair_at(point, t).1
    }

    /// The reading's and the fader's slots of `point` at the second `t`.
    fn pair_at(&self, point: &Point, t: i64) -> (i16, i16) {
        let (Some(last), Some(ring)) = (self.last, self.rings.get(point)) else {
            return (HOLE, HOLE);
        };
        if t > last {
            return (HOLE, HOLE);
        }
        let back = ((last - t) / 1000) as usize;
        ring.len()
            .checked_sub(back + 1)
            .and_then(|i| ring.get(i).copied())
            .unwrap_or((HOLE, HOLE))
    }

    /// The line of the minute starting at `m` from what the hour holds.
    pub fn line(&self, m: i64) -> Option<MinuteLine> {
        let points: Vec<PointMinute> = self
            .rings
            .keys()
            .filter_map(|point| {
                let slots = self.minute(point, m);
                let faders = self.minute_faders(point, m);
                slots.iter().any(|s| sounds(*s)).then(|| PointMinute {
                    instance: point.0.clone(),
                    target: point.1.clone(),
                    values: encode(&slots),
                    faders: faders_text(&faders),
                })
            })
            .collect();
        (!points.is_empty()).then_some(MinuteLine { m, points })
    }

    /// The slots of `point` in the minute starting at `m`.
    pub fn minute(&self, point: &Point, m: i64) -> Vec<i16> {
        (0..MINUTE_S as i64)
            .map(|i| self.at(point, m + i * 1000))
            .collect()
    }

    /// The fader's slots of `point` in the minute starting at `m` (#253).
    pub fn minute_faders(&self, point: &Point, m: i64) -> Vec<i16> {
        (0..MINUTE_S as i64)
            .map(|i| self.fader_at(point, m + i * 1000))
            .collect()
    }

    /// The minute not written yet (the one `last` lies in).
    pub fn current(&self) -> Option<MinuteLine> {
        self.last.and_then(|last| self.line(minute_of(last)))
    }

    /// The first second the hour answers for: before it a window reads the
    /// day files (none before the first sample).
    pub fn covers_from(&self) -> Option<i64> {
        let (first, last) = (self.first?, self.last?);
        Some(first.max(last - (RING_S as i64 - 1) * 1000))
    }

    /// How many points the hour holds.
    pub fn points(&self) -> usize {
        self.rings.len()
    }
}

/// The holes between the second `last` and the later `second`: at most an
/// hour's worth.
pub fn gap_holes(last: i64, second: i64) -> usize {
    let gap = ((second - last) / 1000).saturating_sub(1);
    usize::try_from(gap).unwrap_or(0).min(RING_S)
}

/// A sample's slots: the reading's and the fader's.
pub fn slots_of(sample: &Sample) -> (i16, i16) {
    (slot_of(sample.tenths), fader_slot(sample.fader))
}

/// A minute's fader slots as a line writes them (#253): base64, none when
/// the minute knows no fader.
pub fn faders_text(faders: &[i16]) -> Option<String> {
    faders.iter().any(|f| heard(*f)).then(|| encode(faders))
}

/// Pushes `holes` holes, then `slot`, keeping the last [`RING_S`].
fn push(ring: &mut VecDeque<(i16, i16)>, holes: usize, slot: (i16, i16)) {
    ring.extend(std::iter::repeat_n((HOLE, HOLE), holes));
    ring.push_back(slot);
    let over = ring.len().saturating_sub(RING_S);
    ring.drain(..over);
}

/// Why a window is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowError {
    /// `to` is not after `from`.
    Empty,
    /// Longer than [`MAX_SPAN_MS`].
    TooLong,
    /// Its seconds do not fit the clock's range (PR #223's review: the
    /// arithmetic would overflow).
    OutOfRange,
}

impl WindowError {
    pub fn text(self) -> &'static str {
        match self {
            WindowError::Empty => "the window ends before it starts",
            WindowError::TooLong => "the window is longer than 24 h",
            WindowError::OutOfRange => "the window is out of range",
        }
    }
}

/// A window's seconds: the first whole second at or before `from` and how
/// many seconds up to `to` (not included). Every step is checked: a window
/// whose start, length or end (`start + n * 1000`) leaves the clock's range
/// is [`WindowError::OutOfRange`].
pub fn window(from: i64, to: i64) -> Result<(i64, usize), WindowError> {
    if to <= from {
        return Err(WindowError::Empty);
    }
    let span = to.checked_sub(from).ok_or(WindowError::TooLong)?;
    if span > MAX_SPAN_MS {
        return Err(WindowError::TooLong);
    }
    let start = from
        .checked_sub(from.rem_euclid(1000))
        .ok_or(WindowError::OutOfRange)?;
    let n = (to - start + 999) / 1000;
    start.checked_add(n * 1000).ok_or(WindowError::OutOfRange)?;
    Ok((start, n as usize))
}

/// The seconds per bucket for `n` seconds: at most [`MAX_VALUES`] buckets.
pub fn step_for(n: usize) -> usize {
    n.div_ceil(MAX_VALUES).max(1)
}

/// One bucket's value ([`Series`]): its heard slots by the rule the page's
/// graph shares (`fohmixer_proto::lufs::bucket_of`).
pub fn bucket_value(slots: &[i16]) -> Option<i16> {
    let heard: Vec<i16> = slots.iter().copied().filter(|s| heard(*s)).collect();
    bucket_of(&heard)
}

/// One bucket's fader (#253): the median of its known fader slots, none
/// when it knows none.
pub fn fader_value(faders: &[i16]) -> Option<i16> {
    let known: Vec<i16> = faders.iter().copied().filter(|f| heard(*f)).collect();
    median(&known)
}

/// The slots of a window starting at `from`, one a second, bucketed, and
/// the fader's slots of the same seconds beside them (#253).
pub fn bucket(from: i64, slots: &[i16], faders: &[i16]) -> Series {
    let step = step_for(slots.len());
    Series {
        from,
        step_ms: step as i64 * 1000,
        values: slots.chunks(step).map(bucket_value).collect(),
        fader: faders.chunks(step).map(fader_value).collect(),
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
