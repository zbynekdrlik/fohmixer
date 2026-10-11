//! At most one Pro-Q 4 frame in flight per page, two after a lost ack
//! (#71 PR I): the writer sends a client's next frame only once the page
//! acked the frames sent before it (`eq_frame_ack`), or, with one frame in
//! flight, once nothing was heard of it for [`FRAME_ACK_WAIT`].
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
//! The frames of a session are numbered from 1 (`seq`, in the frame's
//! header), and an ack names the newest frame the page got: it acks every
//! frame up to it, so a later ack covers one that never came, and the
//! count of frames in flight (sent less acked) is always exact. The wait
//! lets a second frame go when the first one's ack does not come (a frame
//! the page dropped, a lost ack), never a third: during a stall of the
//! downlink, two frames at most wait in the socket, however long it lasts
//! (the PR I review: one frame a second went into a stalled socket, and
//! after a 3 s stall the pongs waited behind 3–4 frames again). A page
//! that never acks gets two frames, then none until it acks.
//!
//! Pure, tested here; the outbox keeps one [`Flight`] and the frame
//! [`Trips`] the editor's `rate` record reports.

use std::time::{Duration, Instant};

/// How long the writer waits for the page's ack of the one frame in flight
/// before it sends the next one anyway (a lost ack never stalls the
/// picture). Counted from the last frame sent or ack read, whichever came
/// later.
pub const FRAME_ACK_WAIT: Duration = Duration::from_millis(1000);

/// Whether the next frame may go at `now` with `in_flight` frames not acked
/// yet: none in flight; one, once nothing was heard of it for
/// [`FRAME_ACK_WAIT`] since `since` (the last frame sent or ack read); never
/// with two.
pub fn frame_due(in_flight: u32, since: Option<Instant>, now: Instant) -> bool {
    match in_flight {
        0 => true,
        1 => since.is_none_or(|since| now.saturating_duration_since(since) >= FRAME_ACK_WAIT),
        _ => false,
    }
}

/// The frames of one page's editor on their way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Flight {
    /// The session whose frames are counted (0: none yet).
    session: u32,
    /// The number of its last frame sent (0: none yet).
    sent: u32,
    /// The newest frame the page acked (never above `sent`).
    acked: u32,
    /// When the last frame went or the last ack came.
    since: Option<Instant>,
    /// The frame that went with none in flight, and when: its ack measures
    /// its round trip.
    timed: Option<(u32, Instant)>,
}

impl Flight {
    /// The frames sent and not acked yet.
    pub fn unacked(&self) -> u32 {
        self.sent - self.acked
    }

    /// Whether a frame of `session` may go at `now`: always the first of a
    /// session, else when [`frame_due`].
    pub fn due(&self, session: u32, now: Instant) -> bool {
        self.session != session || frame_due(self.unacked(), self.since, now)
    }

    /// When a frame of `session` waiting may go at the latest: the wait's
    /// end of the one frame in flight. None when it may go now (one of
    /// another session, none in flight) and when only an ack lets it go
    /// (two in flight).
    pub fn wait_until(&self, session: u32) -> Option<Instant> {
        if self.session != session || self.unacked() != 1 {
            return None;
        }
        self.since.map(|since| since + FRAME_ACK_WAIT)
    }

    /// A frame of `session` goes at `now` (a frame of another session starts
    /// the count afresh): its number, and whether it went without the ack of
    /// the one before (its wait ran out).
    pub fn sent(&mut self, session: u32, now: Instant) -> (u32, bool) {
        if self.session != session {
            *self = Self {
                session,
                sent: 0,
                acked: 0,
                since: None,
                timed: None,
            };
        }
        let waited_out = self.unacked() > 0;
        self.sent = self.sent.saturating_add(1);
        if !waited_out {
            self.timed = Some((self.sent, now));
        }
        self.since = Some(now);
        (self.sent, waited_out)
    }

    /// The page acked the frames of `session` up to `seq` at `now`: an ack
    /// of another session, of a frame not sent, or of one acked already is
    /// nothing. The round trip of the frame that went alone, once an ack
    /// reaches it.
    pub fn acked(&mut self, session: u32, seq: u32, now: Instant) -> Option<Duration> {
        if self.session != session || seq <= self.acked || seq > self.sent {
            return None;
        }
        self.acked = seq;
        self.since = Some(now);
        let (_, went) = self.timed.filter(|(timed, _)| *timed <= seq)?;
        self.timed = None;
        Some(now.saturating_duration_since(went))
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

    /// Whether nothing was measured and no wait ran out.
    pub fn is_empty(&self) -> bool {
        self.n == 0 && self.waited_out == 0
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
    fn a_frame_goes_when_none_is_in_flight_or_once_the_wait_of_one_ran_out() {
        let t = Instant::now();
        assert!(frame_due(0, None, t));
        assert!(frame_due(0, Some(t), t), "none in flight: at once");
        assert!(!frame_due(1, Some(t), t));
        assert!(!frame_due(
            1,
            Some(t),
            t + FRAME_ACK_WAIT - Duration::from_nanos(1)
        ));
        assert!(frame_due(1, Some(t), t + ms(1000)), "the wait's end");
        assert!(frame_due(1, Some(t), t + ms(1500)));
        // No time to count from: it goes (a frame never waits for nothing).
        assert!(frame_due(1, None, t));
        // A clock that went back waits on.
        assert!(!frame_due(1, Some(t + ms(10)), t));
        // Two or more in flight: only an ack lets the next one go.
        assert!(!frame_due(2, Some(t), t + ms(1000)));
        assert!(!frame_due(2, None, t));
        assert!(!frame_due(3, Some(t), t + ms(60_000)));
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
        assert_eq!(flight.sent(4, t), (1, false), "the first, sent alone");
        assert_eq!(flight.unacked(), 1);
        assert!(!flight.due(4, t + ms(10)));
        assert_eq!(flight.wait_until(4), Some(t + ms(1000)));
        // Its ack: the round trip, and the next frame goes at once.
        assert_eq!(flight.acked(4, 1, t + ms(80)), Some(ms(80)));
        assert_eq!(flight.unacked(), 0);
        assert!(flight.due(4, t + ms(80)));
        assert_eq!(flight.wait_until(4), None);
        // An ack of a frame acked already, or of another session, is
        // nothing.
        assert_eq!(flight.acked(4, 1, t + ms(90)), None);
        assert_eq!(flight.unacked(), 0);
        assert_eq!(flight.sent(4, t + ms(100)), (2, false));
        // A late repeat of an ack changes nothing: the wait still counts
        // from the send.
        assert_eq!(flight.acked(4, 1, t + ms(110)), None);
        assert_eq!(flight.wait_until(4), Some(t + ms(1100)));
        assert_eq!(flight.acked(3, 2, t + ms(120)), None);
        assert_eq!(flight.unacked(), 1);
        assert!(!flight.due(4, t + ms(120)));
        // An ack of a frame not sent yet is nothing either.
        assert_eq!(flight.acked(4, 3, t + ms(125)), None);
        assert_eq!(flight.unacked(), 1);
        assert_eq!(flight.acked(4, 2, t + ms(130)), Some(ms(30)));
        assert_eq!(flight.unacked(), 0);
    }

    #[test]
    fn without_its_ack_a_second_frame_goes_once_the_wait_ran_out_and_an_ack_of_it_acks_both() {
        let t = Instant::now();
        let mut flight = Flight::default();
        flight.sent(1, t);
        assert!(!flight.due(1, t + ms(999)));
        assert_eq!(flight.wait_until(1), Some(t + ms(1000)));
        assert!(flight.due(1, t + ms(1000)), "the wait's end");
        assert_eq!(
            flight.sent(1, t + ms(1000)),
            (2, true),
            "it went without the ack"
        );
        assert_eq!(flight.unacked(), 2);
        // The page never got the first one (or its ack was lost): the
        // second one's ack acks both, and the first one's round trip is
        // measured to it.
        assert_eq!(flight.acked(1, 2, t + ms(1040)), Some(ms(1040)));
        assert_eq!(flight.unacked(), 0);
        assert!(flight.due(1, t + ms(1040)), "one in flight again");
        // The late ack of the first one is nothing now.
        assert_eq!(flight.acked(1, 1, t + ms(1050)), None);
        assert_eq!(flight.unacked(), 0);
    }

    #[test]
    fn a_stalled_links_late_acks_bring_one_frame_in_flight_back() {
        let t = Instant::now();
        let mut flight = Flight::default();
        flight.sent(1, t);
        flight.sent(1, t + ms(1000));
        assert!(!flight.due(1, t + ms(2500)));
        // The link delivers again: the first ack is the first frame's (its
        // round trip, the stall's length); the second is still in flight,
        // so the next frame waits a whole wait from that ack.
        assert_eq!(flight.acked(1, 1, t + ms(2500)), Some(ms(2500)));
        assert_eq!(flight.unacked(), 1);
        assert!(!flight.due(1, t + ms(3499)), "counted from the ack");
        assert_eq!(flight.wait_until(1), Some(t + ms(3500)));
        assert!(flight.due(1, t + ms(3500)));
        // The second frame went without the ack before it: its ack measures
        // nothing, and one frame is in flight again.
        assert_eq!(flight.acked(1, 2, t + ms(2510)), None);
        assert_eq!(flight.unacked(), 0);
        assert!(flight.due(1, t + ms(2510)));
        // The next frame goes alone and is timed again.
        assert_eq!(flight.sent(1, t + ms(2520)), (3, false));
        assert_eq!(flight.acked(1, 3, t + ms(2560)), Some(ms(40)));
    }

    #[test]
    fn a_new_session_starts_the_count_afresh() {
        let t = Instant::now();
        let mut flight = Flight::default();
        flight.sent(1, t);
        flight.sent(1, t + ms(1000));
        assert!(!flight.due(1, t + ms(1100)));
        // Another session's frame goes at once and alone, numbered from 1.
        assert!(flight.due(2, t + ms(1100)));
        assert_eq!(flight.wait_until(2), None);
        assert_eq!(flight.wait_until(1), None);
        assert_eq!(flight.sent(2, t + ms(1100)), (1, false));
        assert_eq!(flight.unacked(), 1);
        // The old session's acks are nothing now.
        assert_eq!(flight.acked(1, 1, t + ms(1150)), None);
        assert_eq!(flight.unacked(), 1);
        assert_eq!(flight.acked(2, 1, t + ms(1160)), Some(ms(60)));
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
        assert!(!trips.is_empty());
        assert!(Trips::default().is_empty());
        let waited = Trips {
            n: 0,
            total_ms: 0.0,
            max_ms: 0.0,
            waited_out: 1,
        };
        assert!(!waited.is_empty(), "a wait that ran out");
    }
}
