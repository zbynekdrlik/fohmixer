//! The meter devices' peaks in the event log (#151), pure: an armed track's
//! output is what Live does not meter, and only the devices' own values can
//! prove from an event's log that its bars showed a real signal. Each
//! `FohMixer Meter` key's loudest L and R are held over a period of
//! [`PERIOD_MS`] that starts at the first value above silence, and given out
//! as one `lufs` `meter_peak` record ([`peak_line`]) at the router's tick.
//! Silence holds nothing, so an idle hub adds no line.
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

/// A key's loudest values of the period, in tenths of dBFS, and whether its
/// device was matched to a strip at the latest of them (only a matched
/// device's values reach the strip's bars).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peak {
    pub left: i16,
    pub right: i16,
    pub matched: bool,
}

/// The period's peaks.
#[derive(Debug, Default)]
pub struct Peaks {
    since: Option<u64>,
    held: BTreeMap<Key, Peak>,
}

impl Peaks {
    /// A reading of `key` at `now`: only a meter device's above silence
    /// counts.
    pub fn note(&mut self, key: &Key, left: i16, right: i16, matched: bool, now: u64) {
        if key.kind != DeviceKind::Meter || !(above_silence(left) || above_silence(right)) {
            return;
        }
        self.since.get_or_insert(now);
        match self.held.get_mut(key) {
            Some(peak) => {
                peak.left = peak.left.max(left);
                peak.right = peak.right.max(right);
                peak.matched = matched;
            }
            None => {
                self.held.insert(
                    key.clone(),
                    Peak {
                        left,
                        right,
                        matched,
                    },
                );
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

/// One period's record: its length and each key's peaks, in dB.
pub fn peak_line(held: &BTreeMap<Key, Peak>, ms: u64) -> Line {
    let meters: Vec<Value> = held
        .iter()
        .map(|(key, peak)| {
            json!({
                "device": key.device,
                "parents": key.parents,
                "matched": peak.matched,
                "left_db": db(peak.left),
                "right_db": db(peak.right),
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
