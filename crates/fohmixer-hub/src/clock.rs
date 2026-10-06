//! A page's clock against the hub's (#43, design note §5.1 "Clocks"): the
//! event log maps the page times (`t`) of a client's records to the hub's
//! clock through its pings, by Cristian's method.
//!
//! A ping carries the round trip `rtt` of the latest pong the page got, and
//! the number `rtt_n` of the ping that round trip measured. That ping reached
//! the hub about `rtt / 2` after it left, so the hub's clock is ahead of the
//! page's by `its arrival − (its t + rtt / 2)`: a round trip is paired with
//! the exchange it measured (kept in a short ring of recent pings, each
//! paired once), never with a later ping that may itself have been held up,
//! and a link slower than the ping period still pairs. The estimate with the
//! lowest round trip of the last minute is the least disturbed by the
//! network, and that is the one reported.

use std::collections::VecDeque;

/// How far back a ping counts (hub ms).
pub const WINDOW_MS: f64 = 60_000.0;
/// The most exchanges kept (a client pinging faster than this per minute
/// only shortens its window).
pub const MAX_SAMPLES: usize = 256;
/// The recent pings a round trip can still be paired with (6.4 s at the
/// page's 100 ms).
pub const RING: usize = 64;

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
    /// Recent pings not paired yet: number, page time, arrival (hub ms).
    ring: VecDeque<(u32, f64, f64)>,
    samples: VecDeque<Sample>,
}

impl ClockSync {
    /// Ping `n`, sent at page time `t`, reached the hub at `arrival` (hub
    /// ms) with `rtt`: the round trip of ping `m` as `(m, ms)`. The offset
    /// (hub − page, ms) of the lowest-RTT exchange of the last minute; `None`
    /// while there is none.
    pub fn on_ping(
        &mut self,
        n: u32,
        arrival: f64,
        t: f64,
        rtt: Option<(u32, f64)>,
    ) -> Option<f64> {
        if let Some((m, rtt)) = rtt
            && let Some(i) = self.ring.iter().position(|&(k, _, _)| k == m)
            && let Some((_, t_m, at_m)) = self.ring.remove(i)
        {
            self.samples.push_back(Sample {
                at: at_m,
                rtt,
                offset: at_m - (t_m + rtt / 2.0),
            });
        }
        self.ring.push_back((n, t, arrival));
        if self.ring.len() > RING {
            self.ring.pop_front();
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
            clock.on_ping(1, 10_130.0, 5_100.0, Some((0, 40.0))),
            Some(5_010.0)
        );
        assert!(!clock.is_empty());
        assert_eq!(clock.len(), 1);
        // Ping 2 is held up 300 ms on the way but carries ping 1's 10 ms:
        // the estimate is ping 1's (10 130 − 5105 = 5025), never the late
        // ping's own arrival.
        assert_eq!(
            clock.on_ping(2, 10_500.0, 5_200.0, Some((1, 10.0))),
            Some(5_025.0)
        );
        // Ping 3 carries ping 2's long round trip: kept, but not the best.
        assert_eq!(
            clock.on_ping(3, 10_600.0, 5_300.0, Some((2, 320.0))),
            Some(5_025.0)
        );
        assert_eq!(clock.len(), 3);
        // The same round trip again (no newer pong yet) pairs nothing more.
        assert_eq!(
            clock.on_ping(4, 10_700.0, 5_400.0, Some((2, 320.0))),
            Some(5_025.0)
        );
        assert_eq!(clock.len(), 3);
        // A ping without a round trip still reports the best of the minute.
        assert_eq!(clock.on_ping(5, 10_800.0, 5_500.0, None), Some(5_025.0));
    }

    #[test]
    fn a_link_slower_than_the_ping_period_still_pairs() {
        // A steady 120 ms round trip, a ping every 100 ms: each ping carries
        // the latest pong, two pings back.
        let mut clock = ClockSync::default();
        let offset = 1_000.0;
        let mut got = None;
        for n in 0..10_u32 {
            let t = 100.0 * f64::from(n);
            let arrival = t + offset + 60.0;
            let latest = n.checked_sub(2).map(|m| (m, 120.0));
            got = clock.on_ping(n, arrival, t, latest);
        }
        assert_eq!(got, Some(offset));
        assert_eq!(clock.len(), 8);
    }

    #[test]
    fn a_round_trip_of_an_unknown_ping_pairs_with_nothing() {
        let mut clock = ClockSync::default();
        // The socket's first ping cannot pair (ping 41 was another socket's).
        assert_eq!(clock.on_ping(42, 100.0, 50.0, Some((41, 3.0))), None);
        assert!(clock.is_empty());
        // A ping older than the ring is gone.
        for n in 43..=(43 + RING as u32) {
            clock.on_ping(n, 200.0, 150.0, None);
        }
        assert_eq!(RING, 64);
        assert_eq!(clock.on_ping(200, 300.0, 250.0, Some((43, 2.0))), None);
        assert_eq!(
            clock.on_ping(201, 300.0, 250.0, Some((45, 2.0))),
            Some(200.0 - (150.0 + 1.0))
        );
    }

    #[test]
    fn exchanges_older_than_a_minute_stop_counting() {
        // The page's clock runs with the hub's (a page whose clock stood
        // still while the hub's ran 1 s would be a clock step).
        let mut clock = ClockSync::default();
        clock.on_ping(0, 0.0, 0.0, None);
        clock.on_ping(1, 1_000.0, 960.0, Some((0, 2.0)));
        // Ping 0's exchange (hub 0, offset −1) is the best.
        assert_eq!(
            clock.on_ping(2, 1_100.0, 1_060.0, Some((1, 100.0))),
            Some(-1.0)
        );
        // Exactly a minute after ping 0 reached the hub: it still counts.
        assert_eq!(
            clock.on_ping(3, 60_000.0, 59_860.0, Some((2, 300.0))),
            Some(-1.0)
        );
        // A moment later it is gone; ping 1's (hub 1000, page 960, rtt 100:
        // −10) is the best left.
        assert_eq!(
            clock.on_ping(4, 60_000.5, 59_860.5, Some((3, 300.0))),
            Some(-10.0)
        );
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
            clock.on_ping(n, f64::from(n), 0.0, Some((n - 1, 1_000.0 - f64::from(n))));
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
    fn a_clock_step_is_over_half_a_second_beyond_the_round_trips() {
        assert_eq!(STEP_MS, 500.0);
        assert_eq!(step(None, 9_999.0, 0.0), None, "nothing to step from");
        // Round trips of 0: the bound itself, forward and backward.
        assert_eq!(step(Some((0.0, 0.0)), 500.0, 0.0), None);
        assert_eq!(
            step(Some((0.0, 0.0)), 500.0_f64.next_up(), 0.0),
            Some(500.0_f64.next_up())
        );
        assert_eq!(step(Some((0.0, 0.0)), -500.0, 0.0), None);
        assert_eq!(step(Some((0.0, 0.0)), -500.001, 0.0), Some(-500.001));
        // Each exchange's offset can be off by half its round trip (all of
        // it on one way): 10 and 30 ms allow 20 ms more.
        assert_eq!(step(Some((1_000.0, 10.0)), 1_520.0, 30.0), None);
        assert_eq!(
            step(Some((1_000.0, 10.0)), 1_520.001, 30.0),
            Some(1_520.001 - 1_000.0)
        );
        assert_eq!(step(Some((1_000.0, 10.0)), 480.0, 30.0), None);
        assert_eq!(
            step(Some((1_000.0, 10.0)), 479.999, 30.0),
            Some(479.999 - 1_000.0)
        );
    }

    /// Pings every 100 ms from a page 1000 ms behind the hub, 1 ms on the
    /// way out; from ping 3 on, the hub's clock reads `step_ms` more. Each
    /// ping carries the previous one's round trip (`rtts[n − 1]`). The
    /// estimates after each ping.
    fn stepped(step_ms: f64, rtts: &[f64]) -> (ClockSync, Vec<Option<f64>>) {
        let mut clock = ClockSync::default();
        let mut got = Vec::new();
        for n in 0..=rtts.len() as u32 {
            let t = 100.0 * f64::from(n);
            let stepped = if n >= 3 { step_ms } else { 0.0 };
            let arrival = t + 1_001.0 + stepped;
            let rtt = n.checked_sub(1).map(|m| (m, rtts[m as usize]));
            got.push(clock.on_ping(n, arrival, t, rtt));
        }
        (clock, got)
    }

    #[test]
    fn a_forward_clock_step_starts_the_window_over_at_once() {
        // Ping 3's exchange (the first after the step, 10 ms round trip,
        // offset 3301 − 305) is worse than the old ones, yet the estimate
        // follows it at once, then the better ping 4's.
        let (mut clock, got) = stepped(2_000.0, &[2.0, 2.0, 2.0, 10.0, 2.0]);
        assert_eq!(
            got,
            vec![
                None,
                Some(1_000.0),
                Some(1_000.0),
                Some(1_000.0),
                Some(2_996.0),
                Some(3_000.0)
            ]
        );
        assert_eq!(clock.take_step(), Some(1_996.0), "the step, once");
        assert_eq!(clock.take_step(), None);
        assert_eq!(clock.len(), 2, "the old exchanges are gone");
    }

    #[test]
    fn a_backward_clock_step_starts_the_window_over_too() {
        let (mut clock, got) = stepped(-2_000.0, &[2.0, 2.0, 2.0, 10.0, 2.0]);
        assert_eq!(got[3..], [Some(1_000.0), Some(-1_004.0), Some(-1_000.0)]);
        assert_eq!(clock.take_step(), Some(-2_004.0));
        assert_eq!(clock.len(), 2);
    }

    #[test]
    fn a_held_exchange_is_no_step() {
        // No step: ping 3 waits 1.5 s on its way out (its round trip 1501
        // ms), so its offset reads 749.5 ms off: within half its round trip
        // and the best's (751.5 ms), however far past the 500 ms.
        let mut clock = ClockSync::default();
        clock.on_ping(0, 1_001.0, 0.0, None);
        clock.on_ping(1, 1_101.0, 100.0, Some((0, 2.0)));
        clock.on_ping(2, 1_201.0, 200.0, Some((1, 2.0)));
        clock.on_ping(3, 2_800.0, 300.0, Some((2, 2.0)));
        assert_eq!(
            clock.on_ping(4, 2_802.0, 1_801.0, Some((3, 1_501.0))),
            Some(1_000.0)
        );
        assert_eq!(clock.take_step(), None);
        assert_eq!(clock.len(), 4);
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
