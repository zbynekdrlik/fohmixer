//! A page's clock against the hub's (#43, design note §5.1 "Clocks"): the
//! event log maps the page times (`t`) of a client's records to the hub's
//! clock through its pings, by Cristian's method.
//!
//! A ping carries the page's time `t` and the round trip `rtt` of the
//! previous ping: it reached the hub about `rtt / 2` after it left, so the
//! hub's clock is ahead of the page's by `arrival − (t + rtt / 2)`. The
//! estimate of the ping with the lowest round trip of the last minute is
//! the least disturbed by the network, and that is the one reported.

use std::collections::VecDeque;

/// How far back a ping counts (hub ms).
pub const WINDOW_MS: f64 = 60_000.0;
/// The most pings kept (a client pinging faster than this per minute only
/// shortens its window).
pub const MAX_SAMPLES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Sample {
    /// When the ping reached the hub (hub ms).
    at: f64,
    rtt: f64,
    /// Hub clock minus page clock (ms).
    offset: f64,
}

/// The time a message took from the page to the hub (ms): it left at page
/// time `t` and reached the hub at `hub_ms` (hub UTC ms); `offset` is the
/// page clock's offset (hub − page). `None` before the offset is known.
pub fn one_way_delay(hub_ms: f64, t: f64, offset: Option<f64>) -> Option<f64> {
    offset.map(|o| hub_ms - (t + o))
}

/// One client's pings of the last minute.
#[derive(Debug, Default)]
pub struct ClockSync {
    samples: VecDeque<Sample>,
}

impl ClockSync {
    /// A ping that reached the hub at `arrival` (hub ms), sent at page time
    /// `t`, with the previous ping's round trip `rtt`: the offset (hub −
    /// page, ms) of the lowest-RTT ping of the last minute, this one
    /// included; `None` while no ping of the last minute carried a round
    /// trip.
    pub fn on_ping(&mut self, arrival: f64, t: f64, rtt: Option<f64>) -> Option<f64> {
        if let Some(rtt) = rtt {
            self.samples.push_back(Sample {
                at: arrival,
                rtt,
                offset: arrival - (t + rtt / 2.0),
            });
        }
        self.samples.retain(|s| arrival - s.at <= WINDOW_MS);
        if self.samples.len() > MAX_SAMPLES {
            self.samples.pop_front();
        }
        self.samples
            .iter()
            .min_by(|a, b| a.rtt.total_cmp(&b.rtt))
            .map(|s| s.offset)
    }

    /// How many pings are kept.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offset_is_cristians_estimate_of_the_fastest_ping_of_the_minute() {
        let mut clock = ClockSync::default();
        // The first ping has no round trip yet.
        assert_eq!(clock.on_ping(10_000.0, 4_000.0, None), None);
        assert!(clock.is_empty());
        // Sent at page 5000 with a 40 ms round trip, here at hub 10 030:
        // 10 030 − (5000 + 20) = 5010.
        assert_eq!(clock.on_ping(10_030.0, 5_000.0, Some(40.0)), Some(5_010.0));
        // A slower ping does not replace it.
        assert_eq!(clock.on_ping(11_100.0, 6_000.0, Some(200.0)), Some(5_010.0));
        // A faster one does: 12 012 − (7000 + 5) = 5007.
        assert_eq!(clock.on_ping(12_012.0, 7_000.0, Some(10.0)), Some(5_007.0));
        assert_eq!(clock.len(), 3);
        // A ping without a round trip still reports the best of the minute.
        assert_eq!(clock.on_ping(13_000.0, 8_000.0, None), Some(5_007.0));
    }

    #[test]
    fn a_messages_delay_is_its_arrival_less_its_send_on_the_hub_clock() {
        // Sent at page 5000 with the page 5010 ms behind: 10 010 on the hub
        // clock, here at 10 260: 250 ms on the way.
        assert_eq!(one_way_delay(10_260.0, 5_000.0, Some(5_010.0)), Some(250.0));
        assert_eq!(one_way_delay(10_000.0, 5_000.0, Some(4_990.0)), Some(10.0));
        assert_eq!(one_way_delay(10_000.0, 5_000.0, None), None);
    }

    #[test]
    fn pings_older_than_a_minute_stop_counting() {
        let mut clock = ClockSync::default();
        assert_eq!(clock.on_ping(0.0, 0.0, Some(2.0)), Some(-1.0));
        clock.on_ping(1_000.0, 0.0, Some(100.0));
        // Exactly a minute after the fast one: it still counts.
        assert_eq!(clock.on_ping(60_000.0, 0.0, Some(300.0)), Some(-1.0));
        // A moment later it is gone; the 100 ms one (offset 950) is the
        // fastest left.
        assert_eq!(clock.on_ping(60_000.5, 0.0, Some(300.0)), Some(950.0));
        assert_eq!(clock.len(), 3);
        // Two minutes on, only a new ping counts.
        assert_eq!(clock.on_ping(200_000.0, 0.0, None), None);
        assert!(clock.is_empty());
    }

    #[test]
    fn a_flood_of_pings_keeps_at_most_256() {
        let mut clock = ClockSync::default();
        for i in 0..=MAX_SAMPLES {
            clock.on_ping(i as f64, 0.0, Some(1_000.0 - i as f64));
        }
        assert_eq!(MAX_SAMPLES, 256);
        assert_eq!(clock.len(), 256, "the oldest of 257 went");
        // The first ping (rtt 1000) is gone, the newest (rtt 744) is the fastest.
        assert_eq!(clock.on_ping(300.0, 0.0, None), Some(256.0 - 744.0 / 2.0));
        assert_eq!(WINDOW_MS, 60_000.0);
    }
}
