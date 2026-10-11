//! One Pro-Q 4 frame in flight per page (#71 PR I): the writer sends a
//! client's next frame only once the page acked the frames sent before it
//! (`eq_frame_ack`), or nothing was heard of them for [`FRAME_ACK_WAIT`].
//!
//! The owner's iPad session (2026-10-11) showed why: the hub wrote frames as
//! fast as Pro-Q's picture changed (about 9 a second at ~64 KB), the iPad's
//! Wi-Fi took fewer, and the frames piled up in the socket's send buffer.
//! The newest-wins slot only acted before the writer, so the picture lagged
//! seconds behind the finger, and every pong, ack and value written after
//! the frames waited behind them (2.2 s stalls every ~3 s). With one frame
//! in flight, a control message waits behind one frame at most, and the
//! frame rate follows what the link takes.
//!
//! The acks carry the session only, so the frames are counted: each ack
//! answers the oldest frame not acked yet (the page acks in order, on one
//! socket). A frame that went because the wait ran out is counted too, so
//! the late acks of a stalled link (it delivers its frames, then all their
//! acks come) never let more frames out than it acked: after a stall the
//! count comes back to 0 and one frame is in flight again. A frame whose
//! ack never comes (the page left that session) keeps the count above 0 for
//! that session: its frames then go one per [`FRAME_ACK_WAIT`], never none,
//! until the session ends ([`Flight::sent`] starts each session afresh).
//!
//! Pure, tested here; the outbox keeps one [`Flight`] and the frame
//! [`Trips`] the editor's `rate` record reports once a minute.

use std::time::{Duration, Instant};

/// How long the writer waits for the page's ack of the frames in flight
/// before it sends the next one anyway (a lost ack never stalls the
/// picture for good). Counted from the last frame sent or ack read,
/// whichever came later: during a stall one frame goes per wait, and once
/// the link delivers again its acks come back first.
pub const FRAME_ACK_WAIT: Duration = Duration::from_millis(1000);

/// Whether the next frame may go at `now`: no frame waits for its ack
/// (`unacked` 0), or nothing was heard of them for [`FRAME_ACK_WAIT`] since
/// `since` (the last frame sent or ack read).
pub fn frame_due(unacked: u32, since: Option<Instant>, now: Instant) -> bool {
    unacked == 0 || since.is_none_or(|since| now.saturating_duration_since(since) >= FRAME_ACK_WAIT)
}

/// The frames of one page's editor on their way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flight {
    /// The session whose frames are counted (0: none yet).
    session: u32,
    /// Its frames sent and not acked yet.
    unacked: u32,
    /// When the last of them went or the last ack came.
    since: Option<Instant>,
    /// When the oldest frame not acked went, if it went alone (no frame
    /// waited for its ack then): its ack measures its round trip.
    timed: Option<Instant>,
}

impl Flight {
    /// Whether a frame of `session` may go at `now`: always the first of a
    /// session, else when [`frame_due`].
    pub fn due(&self, session: u32, now: Instant) -> bool {
        self.session != session || frame_due(self.unacked, self.since, now)
    }

    /// When a frame of `session` waiting may go at the latest (its ack's
    /// wait over); none when it may go now (one of another session, none in
    /// flight, or no time to count from).
    pub fn wait_until(&self, session: u32) -> Option<Instant> {
        if self.session != session || self.unacked == 0 {
            return None;
        }
        self.since.map(|since| since + FRAME_ACK_WAIT)
    }

    /// A frame of `session` went at `now` (a frame of another session starts
    /// the count afresh): whether it went without the acks of the frames
    /// before it (their wait ran out).
    pub fn sent(&mut self, session: u32, now: Instant) -> bool {
        if self.session != session {
            *self = Self {
                session,
                unacked: 0,
                since: None,
                timed: None,
            };
        }
        let waited_out = self.unacked > 0;
        if !waited_out {
            self.timed = Some(now);
        }
        self.unacked = self.unacked.saturating_add(1);
        self.since = Some(now);
        waited_out
    }

    /// The page acked a frame of `session` at `now`: it answers the oldest
    /// frame in flight (an ack of another session, or with none in flight,
    /// is nothing). The round trip of that frame when it went alone.
    pub fn acked(&mut self, session: u32, now: Instant) -> Option<Duration> {
        if self.session != session || self.unacked == 0 {
            return None;
        }
        self.unacked -= 1;
        self.since = Some(now);
        self.timed
            .take()
            .map(|sent| now.saturating_duration_since(sent))
    }

    /// The frames in flight (the tests read the count through it).
    pub fn unacked(&self) -> u32 {
        self.unacked
    }
}

/// The frame round trips and the waits that ran out since the last `rate`
/// record (#71 PR I): what a real session's link did, read back from the
/// event log.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Trips {
    /// Round trips measured (a frame sent alone, then acked).
    pub n: u32,
    /// Their sum and the largest (ms).
    pub total_ms: f64,
    pub max_ms: f64,
    /// Frames that went because the ack's wait ran out.
    pub waited_out: u32,
}

impl Trips {
    /// A frame's round trip of `ms`.
    pub fn trip(&mut self, ms: f64) {
        self.n += 1;
        self.total_ms += ms;
        self.max_ms = self.max_ms.max(ms);
    }

    /// The mean round trip (ms; 0 without one).
    pub fn mean_ms(&self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            self.total_ms / f64::from(self.n)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn the_wait_is_a_second() {
        assert_eq!(FRAME_ACK_WAIT, Duration::from_millis(1000));
    }

    #[test]
    fn a_frame_goes_when_none_is_in_flight_or_once_the_wait_ran_out() {
        let t = Instant::now();
        assert!(frame_due(0, None, t));
        assert!(frame_due(0, Some(t), t), "none in flight: at once");
        assert!(!frame_due(1, Some(t), t));
        assert!(!frame_due(1, Some(t), t + ms(999)));
        assert!(!frame_due(
            2,
            Some(t),
            t + FRAME_ACK_WAIT - Duration::from_nanos(1)
        ));
        assert!(frame_due(1, Some(t), t + ms(1000)), "the wait's end");
        assert!(frame_due(3, Some(t), t + ms(1500)));
        // No time to count from: it goes (a frame never waits for nothing).
        assert!(frame_due(1, None, t));
        // A clock that went back waits on.
        assert!(!frame_due(1, Some(t + ms(10)), t));
    }

    #[test]
    fn a_stalled_link_never_has_more_than_two_frames_in_flight() {
        // PR I review: the wait let one frame out per second of a stall, so
        // after a 3 s stall the pongs waited behind 3–4 frames again.
        let t = Instant::now();
        let mut flight = Flight::default();
        flight.sent(1, t);
        assert!(flight.due(1, t + ms(1000)), "the ack's wait ran out");
        flight.sent(1, t + ms(1000));
        assert_eq!(flight.unacked(), 2);
        // However long the stall, a third waits for an ack, and no wait's
        // end would let it out.
        assert!(!flight.due(1, t + ms(2000)));
        assert!(!flight.due(1, t + ms(60_000)));
        assert_eq!(flight.wait_until(1), None);
    }

    #[test]
    fn a_frame_sent_waits_for_its_ack_and_the_ack_brings_the_next_at_once() {
        let t = Instant::now();
        let mut flight = Flight::default();
        assert!(flight.due(4, t), "the first of a session");
        assert_eq!(flight.wait_until(4), None);
        assert!(!flight.sent(4, t), "sent alone");
        assert_eq!(flight.unacked(), 1);
        assert!(!flight.due(4, t + ms(10)));
        assert_eq!(flight.wait_until(4), Some(t + ms(1000)));
        // Its ack: the round trip, and the next frame goes at once.
        assert_eq!(flight.acked(4, t + ms(80)), Some(ms(80)));
        assert_eq!(flight.unacked(), 0);
        assert!(flight.due(4, t + ms(80)));
        assert_eq!(flight.wait_until(4), None);
        // An ack with nothing in flight, or of another session, is nothing.
        assert_eq!(flight.acked(4, t + ms(90)), None);
        assert_eq!(flight.unacked(), 0);
        assert!(!flight.sent(4, t + ms(100)));
        assert_eq!(flight.acked(3, t + ms(120)), None);
        assert_eq!(flight.unacked(), 1);
        assert!(!flight.due(4, t + ms(120)));
        assert_eq!(flight.acked(4, t + ms(130)), Some(ms(30)));
    }

    #[test]
    fn without_its_ack_the_next_frame_goes_once_the_wait_ran_out_from_the_last_send() {
        let t = Instant::now();
        let mut flight = Flight::default();
        flight.sent(1, t);
        assert!(!flight.due(1, t + ms(999)));
        assert!(flight.due(1, t + ms(1000)), "the wait's end");
        assert!(flight.sent(1, t + ms(1000)), "it went without the ack");
        assert_eq!(flight.unacked(), 2);
        // The next one waits a whole second from that send.
        assert!(!flight.due(1, t + ms(1999)));
        assert_eq!(flight.wait_until(1), Some(t + ms(2000)));
        assert!(flight.due(1, t + ms(2000)));
    }

    #[test]
    fn a_stalled_links_late_acks_bring_one_frame_in_flight_back_never_more() {
        let t = Instant::now();
        let mut flight = Flight::default();
        flight.sent(1, t);
        flight.sent(1, t + ms(1000));
        flight.sent(1, t + ms(2000));
        assert_eq!(flight.unacked(), 3);
        // The link delivers again: the first ack is the first frame's (its
        // round trip, the stall's length); the next frame still waits, a
        // whole wait from that ack, not from the last send.
        assert_eq!(flight.acked(1, t + ms(2500)), Some(ms(2500)));
        assert_eq!(flight.unacked(), 2);
        assert!(!flight.due(1, t + ms(3000)), "counted from the ack");
        assert!(!flight.due(1, t + ms(3499)));
        assert_eq!(flight.wait_until(1), Some(t + ms(3500)));
        assert!(flight.due(1, t + ms(3500)));
        // The frames sent without their acks measure nothing.
        assert_eq!(flight.acked(1, t + ms(2510)), None);
        assert!(!flight.due(1, t + ms(2510)));
        assert_eq!(flight.acked(1, t + ms(2520)), None);
        assert_eq!(flight.unacked(), 0);
        assert!(flight.due(1, t + ms(2520)), "one in flight again");
        // The next frame goes alone and is timed again.
        assert!(!flight.sent(1, t + ms(2520)));
        assert_eq!(flight.acked(1, t + ms(2560)), Some(ms(40)));
    }

    #[test]
    fn a_new_session_starts_the_count_afresh() {
        let t = Instant::now();
        let mut flight = Flight::default();
        flight.sent(1, t);
        flight.sent(1, t + ms(1000));
        assert!(!flight.due(1, t + ms(1100)));
        // Another session's frame goes at once and alone.
        assert!(flight.due(2, t + ms(1100)));
        assert_eq!(flight.wait_until(2), None);
        assert!(!flight.sent(2, t + ms(1100)));
        assert_eq!(flight.unacked(), 1);
        // The old session's acks are nothing now.
        assert_eq!(flight.acked(1, t + ms(1150)), None);
        assert_eq!(flight.unacked(), 1);
        assert_eq!(flight.acked(2, t + ms(1160)), Some(ms(60)));
        assert_eq!(flight.unacked(), 0);
    }

    #[test]
    fn the_trips_keep_their_count_mean_and_largest() {
        let mut trips = Trips::default();
        assert_eq!(trips.mean_ms(), 0.0);
        trips.trip(40.0);
        trips.trip(100.0);
        trips.trip(70.0);
        assert_eq!(
            trips,
            Trips {
                n: 3,
                total_ms: 210.0,
                max_ms: 100.0,
                waited_out: 0,
            }
        );
        assert_eq!(trips.mean_ms(), 70.0);
    }
}
