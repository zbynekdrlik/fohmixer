//! The pongs' round trips, one summary a second (#43 PR D): the flight
//! recorder no longer keeps an event per pong (10 a second while the page is
//! visible: 204 812 of them at the service of 2026-10-04, most of its
//! volume). The hub logs every ping that reaches it with the page's latest
//! round trip, so the page keeps only `{"ev": "rtt", "t", "n", "min", "med",
//! "max"}` per second of pongs: how many came and how long they took.

use serde_json::{Value, json};

use super::moves::round1;

/// A summary covers the pongs of this long (ms) from its first.
pub const RTT_WINDOW_MS: f64 = 1000.0;

/// Whether a window that began at `start` is over at `now`.
pub fn window_over(start: f64, now: f64) -> bool {
    now - start >= RTT_WINDOW_MS
}

/// The summary of round trips `rtts` (sorted, not empty) of the window from
/// `start`: their count, least, median (the upper middle of an even count)
/// and most, to 0.1 ms.
pub fn summary(start: f64, rtts: &[f64]) -> Value {
    let at = |i: usize| rtts.get(i).copied().map(round1);
    json!({
        "ev": "rtt",
        "t": start,
        "n": rtts.len(),
        "min": at(0),
        "med": at(rtts.len() / 2),
        "max": at(rtts.len().saturating_sub(1)),
    })
}

/// The pongs of the current second.
#[derive(Debug, Default)]
pub struct RttWindow {
    /// When its first pong came (page ms); none while it is empty.
    start: Option<f64>,
    rtts: Vec<f64>,
}

impl RttWindow {
    /// A pong at `t`, `rtt` ms after its ping: the summary of the window it
    /// closes, if one was over.
    pub fn add(&mut self, t: f64, rtt: f64) -> Option<Value> {
        let closed = self.close(t);
        self.start.get_or_insert(t);
        self.rtts.push(rtt);
        closed
    }

    /// The window's summary when it is over at `now` (it starts again empty);
    /// none while it lasts or is empty.
    pub fn close(&mut self, now: f64) -> Option<Value> {
        let start = self.start.filter(|&start| window_over(start, now))?;
        self.start = None;
        let mut rtts = std::mem::take(&mut self.rtts);
        rtts.sort_by(f64::total_cmp);
        Some(summary(start, &rtts))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_is_one_second_from_its_first_pong() {
        assert_eq!(RTT_WINDOW_MS, 1000.0);
        assert!(!window_over(5_000.0, 5_999.0));
        assert!(!window_over(5_000.0, 6_000.0_f64.next_down()));
        assert!(window_over(5_000.0, 6_000.0));
    }

    #[test]
    fn a_summary_is_the_count_and_the_least_middle_and_most_round_trip() {
        assert_eq!(
            summary(7.0, &[2.04, 3.0, 4.0, 9.96]),
            json!({"ev": "rtt", "t": 7.0, "n": 4, "min": 2.0, "med": 4.0, "max": 10.0})
        );
        assert_eq!(
            summary(8.0, &[5.0, 6.0, 7.0]),
            json!({"ev": "rtt", "t": 8.0, "n": 3, "min": 5.0, "med": 6.0, "max": 7.0})
        );
        assert_eq!(
            summary(9.0, &[12.25]),
            json!({"ev": "rtt", "t": 9.0, "n": 1, "min": 12.3, "med": 12.3, "max": 12.3})
        );
    }

    #[test]
    fn the_pongs_of_each_second_make_one_summary() {
        let mut w = RttWindow::default();
        assert_eq!(w.close(1_000.0), None, "empty");
        for (i, rtt) in [30.0, 10.0, 20.0, 50.0, 40.0].into_iter().enumerate() {
            assert_eq!(w.add(1_000.0 + 100.0 * i as f64, rtt), None);
        }
        assert_eq!(w.close(1_999.0), None, "the second lasts");
        assert_eq!(
            w.add(2_000.0, 15.0),
            Some(json!({"ev": "rtt", "t": 1_000.0, "n": 5, "min": 10.0, "med": 30.0, "max": 50.0})),
            "the pong at 2 s closes the first second and opens the next"
        );
        assert_eq!(w.close(2_999.0), None);
        assert_eq!(
            w.close(3_000.0),
            Some(json!({"ev": "rtt", "t": 2_000.0, "n": 1, "min": 15.0, "med": 15.0, "max": 15.0})),
            "a silence closes it at the tick"
        );
        assert_eq!(w.close(9_000.0), None, "nothing after it");
        assert_eq!(w.add(9_500.0, 7.0), None, "a new window");
        assert_eq!(w.close(10_499.0), None);
    }
}
