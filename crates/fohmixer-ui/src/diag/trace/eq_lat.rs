//! The Pro-Q 4 screen's picture latency in the flight recorder (#71 PR J):
//! one `eq_lat` event per [`LAT_WINDOW_MS`] of drawing, essential, with the
//! frames drawn (`f`), every latency sample in that window (`ms`: from an
//! input's send to the draw of the first picture that shows it, on the
//! page's clock, `store::eq::inputs::Inputs::drawn`), and the longest
//! decode (`dec`). Every sample is kept, so the median and the p95 of a
//! session come exactly from the event log (acceptance point 1: median
//! ≤ 100 ms, p95 ≤ 200 ms while dragging a band at 3×).
//!
//! Pure, tested natively; `components/eq.rs` records them through
//! `diag::record`.

use serde_json::{Value, json};

use super::moves::round1;

/// One `eq_lat` event per this much drawing (ms of page time from the
/// window's first frame).
pub const LAT_WINDOW_MS: f64 = 1000.0;

/// Whether a window that began at `since` spans a whole [`LAT_WINDOW_MS`]
/// at `t` (page ms).
fn full(since: f64, t: f64) -> bool {
    t - since >= LAT_WINDOW_MS
}

/// The frames drawn since the last `eq_lat` event.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct EqLat {
    /// The window's first frame (page epoch ms).
    since: Option<f64>,
    /// Frames drawn in it.
    frames: u32,
    /// Their latency samples (ms, to 0.1).
    ms: Vec<f64>,
    /// The longest decode (ms).
    decode_ms: f64,
}

impl EqLat {
    /// A frame drawn at `t` (page epoch ms) after `decode_ms` of decode,
    /// with its latency `sample` when it is the first to show an input:
    /// the window's event once it spans [`LAT_WINDOW_MS`].
    pub fn drawn(&mut self, t: f64, decode_ms: f64, sample: Option<f64>) -> Option<Value> {
        let since = *self.since.get_or_insert(t);
        self.frames += 1;
        self.decode_ms = self.decode_ms.max(decode_ms);
        self.ms.extend(sample.map(round1));
        full(since, t).then(|| self.take(since, t))
    }

    /// The screen goes at `t`: the window's event, if a frame was drawn in
    /// it.
    pub fn leave(&mut self, t: f64) -> Option<Value> {
        let since = self.since?;
        Some(self.take(since, t))
    }

    /// The window's event, the window started afresh.
    fn take(&mut self, since: f64, t: f64) -> Value {
        let event = json!({
            "ev": "eq_lat",
            "t": since,
            "span": round1(t - since),
            "f": self.frames,
            "ms": self.ms,
            "dec": round1(self.decode_ms),
        });
        *self = Self::default();
        event
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_of_frames_is_one_event_with_every_sample() {
        assert_eq!(LAT_WINDOW_MS, 1000.0);
        assert!(!full(100.0, 1100.0_f64.next_down()));
        assert!(full(100.0, 1100.0));
        let mut lat = EqLat::default();
        assert_eq!(lat.leave(50.0), None, "nothing drawn");
        assert_eq!(lat.drawn(100.0, 8.04, Some(83.26)), None);
        assert_eq!(lat.drawn(140.0, 12.5, None), None);
        assert_eq!(lat.drawn(600.0, 3.0, Some(120.0)), None);
        assert_eq!(
            lat.drawn(1100.0, 9.0, Some(95.0)),
            Some(json!({"ev": "eq_lat", "t": 100.0, "span": 1000.0, "f": 4,
                        "ms": [83.3, 120.0, 95.0], "dec": 12.5}))
        );
        // The next window starts with the next frame.
        assert_eq!(lat.drawn(1150.0, 4.0, None), None);
        assert_eq!(
            lat.leave(1400.0),
            Some(json!({"ev": "eq_lat", "t": 1150.0, "span": 250.0, "f": 1,
                        "ms": [], "dec": 4.0}))
        );
        assert_eq!(lat.leave(1500.0), None, "left already");
    }
}
