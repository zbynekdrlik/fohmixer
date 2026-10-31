//! The meter devices' peaks in the event log (#151), pure: an armed track's
//! output is what Live does not meter, and only the devices' own values can
//! prove from an event's log that its bars showed a real signal. Each
//! `FohMixer Meter` key's loudest L and R are held over a period of
//! [`PERIOD_MS`] that starts at the first value above silence, and given out
//! as one `lufs` `meter_peak` record ([`peak_line`]) at the router's tick,
//! beside the loudest levels the hub put on the strip's meter keys for them
//! (after the fader, pan and mute) and whether the strip was muted, so the
//! log proves the bars follow them. Silence holds nothing, so an idle hub
//! adds no line.
//!
//! A value only updates the map: a key is cloned once a period, and the
//! listener's bound keeps the keys to `bound::MAX_KEYS`.

use std::collections::BTreeMap;

use fohmixer_proto::lufs::SILENT_TENTHS;
use fohmixer_proto::lufs::wire::DeviceKind;
use serde_json::{Value, json};

use super::{Key, Line};

/// The period one `meter_peak` record covers.
pub const PERIOD_MS: u64 = 10_000;

/// Whether a meter value is above silence.
pub fn above_silence(tenths: i16) -> bool {
    tenths > SILENT_TENTHS
}

/// Whether a period that started at `since` is over at `now`.
pub fn period_over(since: u64, now: u64) -> bool {
    now.saturating_sub(since) >= PERIOD_MS
}

/// The value of the point (`instance`, `target`) in `map`; a key of two
/// `String`s is found without building one.
pub fn at<'a, V>(
    map: &'a BTreeMap<(String, String), V>,
    instance: &str,
    target: &str,
) -> Option<&'a V> {
    map.iter()
        .find(|((i, t), _)| i == instance && t == target)
        .map(|(_, value)| value)
}

/// Each side's louder of two levels; one known is kept.
pub fn louder(held: Option<(f64, f64)>, new: Option<(f64, f64)>) -> Option<(f64, f64)> {
    match (held, new) {
        (Some(a), Some(b)) => Some((a.0.max(b.0), a.1.max(b.1))),
        (a, b) => a.or(b),
    }
}

/// One reading of a key, or a key's loudest of the period: the device's
/// peaks in tenths of dBFS; whether it was matched to a strip (at the latest
/// reading; only a matched device reaches the bars); the levels the hub sends
/// (left, right, Live's 0..1; none: it sends none, the bars are Live's own);
/// whether the strip was muted (at any reading above silence of the period:
/// with levels above 0 beside it, the mute covered only a part).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub left: i16,
    pub right: i16,
    pub matched: bool,
    pub levels: Option<(f64, f64)>,
    pub muted: bool,
}

/// The period's peaks.
#[derive(Debug, Default)]
pub struct Peaks {
    since: Option<u64>,
    held: BTreeMap<Key, Sample>,
}

impl Peaks {
    /// A reading of `key` at `now`: only a meter device's above silence
    /// counts.
    pub fn note(&mut self, key: &Key, sample: Sample, now: u64) {
        let loud = above_silence(sample.left) || above_silence(sample.right);
        if key.kind != DeviceKind::Meter || !loud {
            return;
        }
        self.since.get_or_insert(now);
        match self.held.get_mut(key) {
            Some(peak) => {
                peak.left = peak.left.max(sample.left);
                peak.right = peak.right.max(sample.right);
                peak.matched = sample.matched;
                peak.levels = louder(peak.levels, sample.levels);
                peak.muted = peak.muted || sample.muted;
            }
            None => {
                self.held.insert(key.clone(), sample);
            }
        }
    }

    /// The period's record once it is over; the next period starts empty.
    pub fn take(&mut self, now: u64) -> Option<Line> {
        let since = self.since.filter(|since| period_over(*since, now))?;
        self.since = None;
        let held = std::mem::take(&mut self.held);
        Some(peak_line(&held, now.saturating_sub(since)))
    }
}

/// Tenths as dB.
fn db(tenths: i16) -> f64 {
    f64::from(tenths) / 10.0
}

/// One period's record: its length and, per key, the device's peaks in dB,
/// the levels the hub sent (`null`: none) and the strip's mute.
pub fn peak_line(held: &BTreeMap<Key, Sample>, ms: u64) -> Line {
    let meters: Vec<Value> = held
        .iter()
        .map(|(key, peak)| {
            json!({
                "device": key.device,
                "parents": key.parents,
                "matched": peak.matched,
                "left_db": db(peak.left),
                "right_db": db(peak.right),
                "level_left": peak.levels.map(|levels| levels.0),
                "level_right": peak.levels.map(|levels| levels.1),
                "muted": peak.muted,
            })
        })
        .collect();
    Line {
        warn: false,
        fields: json!({"what": "meter_peak", "ms": ms, "meters": meters}),
        text: "the meter devices' peaks of the period".to_string(),
    }
}

#[cfg(test)]
#[path = "peaks_tests.rs"]
mod tests;
