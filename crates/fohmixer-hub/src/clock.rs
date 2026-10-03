//! A page's clock against the hub's (#43, design note §5.1 "Clocks"): the
//! event log maps the page times (`t`) of a client's records to the hub's
//! clock through its pings, by Cristian's method.
//!
//! Ping n carries the round trip `rtt` of ping n − 1 (the page sends none
//! when that pong had not come back). Ping n − 1 reached the hub about
//! `rtt / 2` after it left, so the hub's clock is ahead of the page's by
//! `its arrival − (its t + rtt / 2)`: the round trip is paired with the
//! exchange it measured, never with a later ping that may itself have been
//! held up. The estimate with the lowest round trip of the last minute is
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

/// One socket's pings of the last minute.
#[derive(Debug, Default)]
pub struct ClockSync {
    /// The last ping: its number, page time and arrival (hub ms).
    last: Option<(u32, f64, f64)>,
    samples: VecDeque<Sample>,
}

impl ClockSync {
    /// Ping `n`, sent at page time `t`, reached the hub at `arrival` (hub
    /// ms) with `rtt`, the round trip of ping `n − 1`: the offset (hub −
    /// page, ms) of the lowest-RTT exchange of the last minute; `None`
    /// while there is none.
    pub fn on_ping(&mut self, n: u32, arrival: f64, t: f64, rtt: Option<f64>) -> Option<f64> {
        if let Some(rtt) = rtt
            && let Some((before, t_before, at_before)) = self.last
            && before == n.wrapping_sub(1)
        {
            self.samples.push_back(Sample {
                at: at_before,
                rtt,
                offset: at_before - (t_before + rtt / 2.0),
            });
        }
        self.last = Some((n, t, arrival));
        self.samples.retain(|s| arrival - s.at <= WINDOW_MS);
        if self.samples.len() > MAX_SAMPLES {
            self.samples.pop_front();
        }
        self.samples
            .iter()
            .min_by(|a, b| a.rtt.total_cmp(&b.rtt))
            .map(|s| s.offset)
    }

    /// How many exchanges are kept.
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
    fn a_round_trip_is_paired_with_the_ping_it_measured() {
        let mut clock = ClockSync::default();
        // Ping 0 has no round trip yet.
        assert_eq!(clock.on_ping(0, 10_030.0, 5_000.0, None), None);
        assert!(clock.is_empty());
        // Ping 1 carries ping 0's 40 ms: ping 0 left at page 5000 and came
        // at hub 10 030, 20 ms on the way: 10 030 − (5000 + 20) = 5010.
        assert_eq!(
            clock.on_ping(1, 10_130.0, 5_100.0, Some(40.0)),
            Some(5_010.0)
        );
        assert!(!clock.is_empty());
        assert_eq!(clock.len(), 1);
        // Ping 2 is held up 300 ms on the way but carries ping 1's 10 ms:
        // the estimate is ping 1's (10 130 − 5105 = 5025), never the late
        // ping's own arrival.
        assert_eq!(
            clock.on_ping(2, 10_500.0, 5_200.0, Some(10.0)),
            Some(5_025.0)
        );
        // Ping 3 carries ping 2's long round trip: kept, but not the best.
        assert_eq!(
            clock.on_ping(3, 10_600.0, 5_300.0, Some(320.0)),
            Some(5_025.0)
        );
        assert_eq!(clock.len(), 3);
        // A round trip of an earlier ping than n − 1 is not paired.
        assert_eq!(
            clock.on_ping(5, 10_800.0, 5_500.0, Some(5.0)),
            Some(5_025.0)
        );
        assert_eq!(clock.len(), 3);
        // A ping without a round trip still reports the best of the minute.
        assert_eq!(clock.on_ping(6, 10_900.0, 5_600.0, None), Some(5_025.0));
    }

    #[test]
    fn the_first_ping_of_a_socket_pairs_with_nothing() {
        let mut clock = ClockSync::default();
        assert_eq!(clock.on_ping(41, 100.0, 50.0, Some(3.0)), None);
        assert!(clock.is_empty());
        // The numbers wrap.
        let mut wrap = ClockSync::default();
        wrap.on_ping(u32::MAX, 100.0, 50.0, None);
        assert_eq!(wrap.on_ping(0, 200.0, 150.0, Some(2.0)), Some(49.0));
    }

    #[test]
    fn exchanges_older_than_a_minute_stop_counting() {
        let mut clock = ClockSync::default();
        clock.on_ping(0, 0.0, 0.0, None);
        clock.on_ping(1, 1_000.0, 0.0, Some(2.0));
        // Ping 0's exchange (hub 0, offset −1) is the best.
        assert_eq!(clock.on_ping(2, 1_100.0, 0.0, Some(100.0)), Some(-1.0));
        // Exactly a minute after ping 0 reached the hub: it still counts.
        assert_eq!(clock.on_ping(3, 60_000.0, 0.0, Some(300.0)), Some(-1.0));
        // A moment later it is gone; ping 1's (hub 1000, rtt 100: 950) is
        // the best left.
        assert_eq!(clock.on_ping(4, 60_000.5, 0.0, Some(300.0)), Some(950.0));
        assert_eq!(clock.len(), 3);
        // Two minutes on, only a new exchange counts.
        assert_eq!(clock.on_ping(9, 200_000.0, 0.0, None), None);
        assert!(clock.is_empty());
    }

    #[test]
    fn a_flood_of_pings_keeps_at_most_256() {
        let mut clock = ClockSync::default();
        clock.on_ping(0, 0.0, 0.0, None);
        for n in 1..=257_u32 {
            clock.on_ping(n, f64::from(n), 0.0, Some(1_000.0 - f64::from(n)));
        }
        assert_eq!(MAX_SAMPLES, 256);
        assert_eq!(clock.len(), 256, "the oldest of 257 went");
        // The newest exchange (ping 256: hub 256, rtt 743) is the fastest.
        assert_eq!(
            clock.on_ping(400, 300.0, 0.0, None),
            Some(256.0 - 743.0 / 2.0)
        );
        assert_eq!(WINDOW_MS, 60_000.0);
    }

    #[test]
    fn a_messages_delay_is_its_arrival_less_its_send_on_the_hub_clock() {
        // Sent at page 5000 with the page 5010 ms behind: 10 010 on the hub
        // clock, here at 10 260: 250 ms on the way.
        assert_eq!(one_way_delay(10_260.0, 5_000.0, Some(5_010.0)), Some(250.0));
        assert_eq!(one_way_delay(10_000.0, 5_000.0, Some(4_990.0)), Some(10.0));
        assert_eq!(one_way_delay(10_000.0, 5_000.0, None), None);
    }
}
