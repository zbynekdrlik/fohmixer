//! The page's inputs of its open Pro-Q 4 editor (#71 PR J), pure: each
//! numbered from 1 per session as it is sent, with its send time; the input
//! the picture drawn last shows (the frames' `shown`); and the one the
//! picture under a landing finger showed (a down's `seen`).
//!
//! From those, on the page's one clock: whether the picture shown is
//! caught up with every input sent ([`Inputs::caught_up`]: a finger on an
//! old picture never acts), and how long each input took to reach the
//! screen ([`Inputs::drawn`]: from its send to the draw of the first
//! picture that shows it).
//!
//! Every call names the open editor's session: another session's state is
//! none (the PR J review: a screen left with an input not shown yet, or a
//! hub restarted whose session numbers start again, must never hold the
//! next screen's fingers back), and the screen's open starts them afresh
//! ([`Inputs::default`]).

use std::collections::VecDeque;

use fohmixer_proto::eq::{Touch, caught_up};

/// The send times kept (the newest): about 17 s of moves at 60 a second,
/// so a stall's slowest inputs are still measured.
pub const INPUTS_KEPT: usize = 1024;

/// The page's inputs of one open session.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Inputs {
    /// The session they are of (0: none yet).
    session: u32,
    /// The last input's number.
    sent: u32,
    /// The newest inputs' numbers and send times (page epoch ms).
    times: VecDeque<(u32, f64)>,
    /// The input the picture drawn last shows.
    drawn: u32,
    /// The input the picture under the last landing finger showed.
    landed: u32,
}

/// What a drawn picture measured (PR J): the latency of each input it is
/// the first to show whose send time was kept, in order, and how many it
/// shows whose send time was not.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Drawn {
    pub samples: Vec<f64>,
    pub lost: u32,
}

impl Inputs {
    /// Another session than these inputs' starts them afresh.
    fn of(&mut self, session: u32) {
        if self.session != session {
            *self = Self {
                session,
                sent: 0,
                times: VecDeque::new(),
                drawn: 0,
                landed: 0,
            };
        }
    }

    /// `touch` sent on `session` at `now` (page epoch ms): its number, and
    /// on a down the input the picture it landed on showed.
    pub fn send(&mut self, session: u32, touch: Touch, now: f64) -> (u32, Option<u32>) {
        self.of(session);
        self.sent += 1;
        if self.times.len() >= INPUTS_KEPT {
            self.times.pop_front();
        }
        self.times.push_back((self.sent, now));
        (self.sent, (touch == Touch::Down).then_some(self.landed))
    }

    /// Whether the picture drawn last on `session` shows every input sent
    /// on it (none sent: it does).
    pub fn caught_up(&self, session: u32) -> bool {
        self.session != session || caught_up(self.drawn, self.sent)
    }

    /// A first finger lands on `session`'s picture now: its down will say
    /// the picture drawn last.
    pub fn land(&mut self, session: u32) {
        self.of(session);
        self.landed = self.drawn;
    }

    /// A picture of `session` that shows input `shown` was drawn at `now`
    /// (page epoch ms): the latency of each input it is the first to show.
    pub fn drawn(&mut self, session: u32, shown: u32, now: f64) -> Drawn {
        self.of(session);
        let mut drawn = Drawn::default();
        if shown <= self.drawn {
            return drawn;
        }
        let first = self.drawn;
        self.drawn = shown;
        for (n, sent) in &self.times {
            if *n > first && *n <= shown {
                drawn.samples.push(now - sent);
            }
        }
        // The inputs it shows whose send time is gone (never more than it
        // shows: a picture of inputs this page never sent measures none).
        let measured = u32::try_from(drawn.samples.len()).unwrap_or(u32::MAX);
        let newly = shown.min(self.sent).saturating_sub(first);
        drawn.lost = newly.saturating_sub(measured);
        drawn
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(drawn: Drawn) -> Vec<f64> {
        drawn.samples
    }

    #[test]
    fn inputs_are_numbered_per_session_and_a_down_says_where_it_landed() {
        let mut inputs = Inputs::default();
        assert!(inputs.caught_up(3), "nothing sent");
        assert_eq!(inputs.send(3, Touch::Down, 100.0), (1, Some(0)));
        assert!(!inputs.caught_up(3));
        assert_eq!(inputs.send(3, Touch::Move, 110.0), (2, None));
        assert_eq!(inputs.send(3, Touch::Up, 120.0), (3, None));
        // A picture showing the down, then one showing the move and the up.
        assert_eq!(samples(inputs.drawn(3, 1, 190.0)), vec![90.0]);
        assert!(!inputs.caught_up(3));
        assert_eq!(
            inputs.drawn(3, 3, 200.0),
            Drawn {
                samples: vec![90.0, 80.0],
                lost: 0
            }
        );
        assert!(inputs.caught_up(3));
        // A finger lands now: its down says the picture showed input 3.
        inputs.land(3);
        assert_eq!(inputs.send(3, Touch::Down, 300.0), (4, Some(3)));
    }

    #[test]
    fn another_session_never_holds_a_finger_back() {
        // The PR J review: a screen left with its up not shown yet held every
        // later screen's fingers back until a reload.
        let mut inputs = Inputs::default();
        inputs.send(3, Touch::Down, 0.0);
        inputs.send(3, Touch::Up, 10.0);
        assert!(!inputs.caught_up(3));
        // The next screen's session: nothing sent on it yet.
        assert!(inputs.caught_up(4));
        // Its first picture starts it afresh; a finger lands on it.
        assert_eq!(inputs.drawn(4, 0, 50.0), Drawn::default());
        inputs.land(4);
        assert!(inputs.caught_up(4));
        assert_eq!(inputs.send(4, Touch::Down, 60.0), (1, Some(0)));
        // A late picture of the old session starts that one afresh too,
        // measuring nothing of the new one's inputs.
        assert_eq!(inputs.drawn(3, 2, 70.0), Drawn::default());
        assert!(inputs.caught_up(4), "none sent on it now");
        // A landing on a session never seen before: seen 0.
        let mut fresh = Inputs::default();
        fresh.land(7);
        assert_eq!(fresh.send(7, Touch::Down, 0.0), (1, Some(0)));
    }

    #[test]
    fn a_picture_measures_every_input_it_is_the_first_to_show() {
        let mut inputs = Inputs::default();
        inputs.send(1, Touch::Down, 0.0);
        inputs.send(1, Touch::Move, 16.0);
        inputs.send(1, Touch::Move, 32.0);
        assert_eq!(samples(inputs.drawn(1, 2, 96.0)), vec![96.0, 80.0]);
        // The same input again, or an older one: no sample.
        assert_eq!(inputs.drawn(1, 2, 120.0), Drawn::default());
        assert_eq!(inputs.drawn(1, 1, 130.0), Drawn::default());
        assert!(!inputs.caught_up(1));
        assert_eq!(samples(inputs.drawn(1, 3, 140.0)), vec![108.0]);
        assert!(inputs.caught_up(1));
        // A picture that shows no input of the page: none.
        let mut fresh = Inputs::default();
        fresh.send(1, Touch::Down, 0.0);
        assert_eq!(fresh.drawn(1, 0, 50.0), Drawn::default());
        // One that says more than the page sent measures what it sent.
        assert_eq!(
            fresh.drawn(1, 5, 60.0),
            Drawn {
                samples: vec![60.0],
                lost: 0
            }
        );
    }

    #[test]
    fn the_newest_send_times_are_kept_and_the_others_counted_lost() {
        assert_eq!(INPUTS_KEPT, 1024);
        let mut inputs = Inputs::default();
        for i in 0..1026 {
            inputs.send(1, Touch::Move, f64::from(i));
        }
        // Inputs 1 and 2 went: a picture showing up to 3 measures input 3
        // only and counts two lost.
        assert_eq!(
            inputs.drawn(1, 3, 2_000.0),
            Drawn {
                samples: vec![1_998.0],
                lost: 2
            }
        );
        assert!(!inputs.caught_up(1));
        assert_eq!(inputs.drawn(1, 1026, 2_000.0).samples.len(), 1023);
        assert!(inputs.caught_up(1));
    }
}
