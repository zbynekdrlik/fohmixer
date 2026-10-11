//! The page's inputs of its open Pro-Q 4 editor (#71 PR J), pure: each
//! numbered from 1 per session as it is sent, with its send time; the input
//! the picture drawn last shows (the frames' `shown`); and the one the
//! picture under a landing finger showed (a down's `seen`).
//!
//! From those, on the page's one clock: whether the picture shown is
//! caught up with every input sent ([`Inputs::caught_up`]: a finger on an
//! old picture never acts), and how long a finger's move took to reach the
//! screen ([`Inputs::drawn`]: from the input's send to the draw of the first
//! picture that shows it).

use std::collections::VecDeque;

use fohmixer_proto::eq::{Touch, caught_up};

/// The send times kept (the newest): a frame shows an input a few frames
/// old at most, so these cover any picture that arrives.
pub const INPUTS_KEPT: usize = 64;

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

    /// Whether the picture drawn last shows every input sent.
    pub fn caught_up(&self) -> bool {
        caught_up(self.drawn, self.sent)
    }

    /// A first finger lands now: its down will say the picture drawn last.
    pub fn land(&mut self) {
        self.landed = self.drawn;
    }

    /// A picture of `session` that shows input `shown` was drawn at `now`
    /// (page epoch ms): the time from that input's send, when the picture
    /// is the first to show it and its send time is kept (ms).
    pub fn drawn(&mut self, session: u32, shown: u32, now: f64) -> Option<f64> {
        if session != self.session || shown <= self.drawn {
            return None;
        }
        self.drawn = shown;
        self.times
            .iter()
            .find(|(n, _)| *n == shown)
            .map(|(_, sent)| now - sent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_are_numbered_per_session_and_a_down_says_where_it_landed() {
        let mut inputs = Inputs::default();
        assert!(inputs.caught_up(), "nothing sent");
        assert_eq!(inputs.send(3, Touch::Down, 100.0), (1, Some(0)));
        assert!(!inputs.caught_up());
        assert_eq!(inputs.send(3, Touch::Move, 110.0), (2, None));
        assert_eq!(inputs.send(3, Touch::Up, 120.0), (3, None));
        // A picture showing the down, then one showing the up.
        assert_eq!(inputs.drawn(3, 1, 190.0), Some(90.0));
        assert!(!inputs.caught_up());
        assert_eq!(inputs.drawn(3, 3, 200.0), Some(80.0));
        assert!(inputs.caught_up());
        // A finger lands now: its down says the picture showed input 3.
        inputs.land();
        assert_eq!(inputs.send(3, Touch::Down, 300.0), (4, Some(3)));
        // Another session starts afresh.
        assert_eq!(inputs.send(4, Touch::Down, 400.0), (1, Some(0)));
        assert_eq!(inputs.drawn(3, 4, 410.0), None, "the old session's");
        assert!(!inputs.caught_up());
    }

    #[test]
    fn a_picture_measures_only_the_input_it_is_the_first_to_show() {
        let mut inputs = Inputs::default();
        inputs.send(1, Touch::Down, 0.0);
        inputs.send(1, Touch::Move, 16.0);
        assert_eq!(inputs.drawn(1, 2, 96.0), Some(80.0));
        // The same input again, or an older one: no sample.
        assert_eq!(inputs.drawn(1, 2, 120.0), None);
        assert_eq!(inputs.drawn(1, 1, 130.0), None);
        assert!(inputs.caught_up());
        // A picture that shows no input of the page: none.
        let mut fresh = Inputs::default();
        fresh.send(1, Touch::Down, 0.0);
        assert_eq!(fresh.drawn(1, 0, 50.0), None);
    }

    #[test]
    fn only_the_newest_send_times_are_kept() {
        assert_eq!(INPUTS_KEPT, 64);
        let mut inputs = Inputs::default();
        for i in 0..65 {
            inputs.send(1, Touch::Move, f64::from(i) * 10.0);
        }
        // Input 1 went (65 sent): a picture showing it measures nothing,
        // one showing input 2 does.
        let mut early = inputs.clone();
        assert_eq!(early.drawn(1, 1, 1_000.0), None);
        assert!(!early.caught_up());
        assert_eq!(inputs.drawn(1, 2, 1_000.0), Some(990.0));
        assert_eq!(inputs.drawn(1, 65, 1_000.0), Some(360.0));
        assert!(inputs.caught_up());
    }
}
