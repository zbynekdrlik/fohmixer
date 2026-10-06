//! A Stream Deck key's presses on the page (#52, spec §6), pure: each held
//! key's pointers, when its first finger came and whether its down went to
//! the hub. A key is down at its first finger and up at its last lift, so
//! Companion's long-press and duration actions see the real hold; a down
//! that cannot go now flashes the key and is never sent later, and its up is
//! not sent either. Leaving the tab or the page going hidden lifts every
//! held key; a closed socket forgets them (the hub releases them). The
//! component (`pages/deck.rs`) carries the actions out.

use std::collections::{BTreeMap, BTreeSet};

/// Why an up came.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// `pointerup`.
    Up,
    /// `pointercancel` (the system took the touch).
    Cancel,
    /// `lostpointercapture` without a `pointerup` before it.
    Lost,
    /// The page went hidden (`visibilitychange`).
    Hidden,
    /// The tab was left (the page's cleanup).
    Tab,
}

impl Why {
    /// Its name on the wire and in the flight recorder.
    pub fn name(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Cancel => "cancel",
            Self::Lost => "lost",
            Self::Hidden => "hidden",
            Self::Tab => "tab",
        }
    }

    /// The why of a pointer event that ends a touch.
    pub fn of_event(event_type: &str) -> Option<Self> {
        match event_type {
            "pointerup" => Some(Self::Up),
            "pointercancel" => Some(Self::Cancel),
            "lostpointercapture" => Some(Self::Lost),
            _ => None,
        }
    }
}

/// What the page does about a pointer event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    /// Send the press: a down, or an up with the page's hold and its why.
    Send {
        key: u32,
        down: bool,
        hold_ms: Option<f64>,
        why: Option<Why>,
    },
    /// Flash the key red: its down cannot go now (never later either).
    Flash {
        key: u32,
    },
    Nothing,
}

/// One held key.
#[derive(Debug, Clone, PartialEq)]
struct Hold {
    pointers: BTreeSet<i32>,
    /// The first finger's page time.
    at: f64,
    /// Its down went to the hub.
    sent: bool,
}

/// The page's held keys.
#[derive(Debug, Default)]
pub struct Presses {
    held: BTreeMap<u32, Hold>,
}

impl Presses {
    /// A finger (`pointer`) on `key` at page time `t`; `connected`: a press
    /// can go now (the socket ready, Companion online).
    pub fn down(&mut self, key: u32, pointer: i32, t: f64, connected: bool) -> Action {
        if let Some(hold) = self.held.get_mut(&key) {
            hold.pointers.insert(pointer);
            return Action::Nothing;
        }
        self.held.insert(
            key,
            Hold {
                pointers: BTreeSet::from([pointer]),
                at: t,
                sent: connected,
            },
        );
        if connected {
            Action::Send {
                key,
                down: true,
                hold_ms: None,
                why: None,
            }
        } else {
            Action::Flash { key }
        }
    }

    /// The down of `key` did not go after all (the socket refused it): its
    /// up is not sent either.
    pub fn unsent(&mut self, key: u32) {
        if let Some(hold) = self.held.get_mut(&key) {
            hold.sent = false;
        }
    }

    /// A finger (`pointer`) leaves `key` at page time `t`: the up of a key
    /// whose down went, when it was the last finger.
    pub fn up(&mut self, key: u32, pointer: i32, t: f64, why: Why) -> Action {
        let last = match self.held.get_mut(&key) {
            Some(hold) => hold.pointers.remove(&pointer) && hold.pointers.is_empty(),
            None => false,
        };
        if !last {
            return Action::Nothing;
        }
        match self.held.remove(&key) {
            Some(hold) if hold.sent => Action::Send {
                key,
                down: false,
                hold_ms: Some(t - hold.at),
                why: Some(why),
            },
            _ => Action::Nothing,
        }
    }

    /// The page leaves every key (`why`: hidden, tab): an up for each key
    /// whose down went, in key order.
    pub fn leave_all(&mut self, t: f64, why: Why) -> Vec<Action> {
        std::mem::take(&mut self.held)
            .into_iter()
            .filter(|(_, hold)| hold.sent)
            .map(|(key, hold)| Action::Send {
                key,
                down: false,
                hold_ms: Some(t - hold.at),
                why: Some(why),
            })
            .collect()
    }

    /// The socket closed: every hold forgotten, no up sent.
    pub fn clear(&mut self) {
        self.held.clear();
    }

    /// Whether a finger holds `key` (the local outline).
    pub fn is_held(&self, key: u32) -> bool {
        self.held.contains_key(&key)
    }

    /// The keys fingers hold.
    pub fn held_keys(&self) -> BTreeSet<u32> {
        self.held.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn down_sent(key: u32) -> Action {
        Action::Send {
            key,
            down: true,
            hold_ms: None,
            why: None,
        }
    }

    fn up_sent(key: u32, hold_ms: f64, why: Why) -> Action {
        Action::Send {
            key,
            down: false,
            hold_ms: Some(hold_ms),
            why: Some(why),
        }
    }

    #[test]
    fn a_key_is_down_at_the_touch_and_up_at_the_release_with_its_hold() {
        let mut p = Presses::default();
        assert_eq!(p.down(3, 21, 1000.0, true), down_sent(3));
        assert!(p.is_held(3));
        assert_eq!(p.up(3, 21, 1150.5, Why::Up), up_sent(3, 150.5, Why::Up));
        assert!(!p.is_held(3));
        // The lost capture after the lift ends nothing new.
        assert_eq!(p.up(3, 21, 1151.0, Why::Lost), Action::Nothing);
    }

    #[test]
    fn two_fingers_on_one_key_send_one_down_and_one_up() {
        let mut p = Presses::default();
        assert_eq!(p.down(4, 1, 0.0, true), down_sent(4));
        assert_eq!(p.down(4, 2, 50.0, true), Action::Nothing);
        assert_eq!(
            p.up(4, 1, 100.0, Why::Up),
            Action::Nothing,
            "a finger still holds it"
        );
        assert!(p.is_held(4));
        assert_eq!(
            p.up(4, 2, 300.0, Why::Cancel),
            up_sent(4, 300.0, Why::Cancel)
        );
        // A pointer that never touched the key.
        assert_eq!(p.down(5, 1, 0.0, true), down_sent(5));
        assert_eq!(p.up(5, 9, 10.0, Why::Up), Action::Nothing);
        assert!(p.is_held(5));
        // A key nobody holds.
        assert_eq!(p.up(6, 1, 10.0, Why::Up), Action::Nothing);
    }

    #[test]
    fn a_down_that_cannot_go_flashes_and_its_up_is_never_sent() {
        let mut p = Presses::default();
        assert_eq!(p.down(7, 1, 0.0, false), Action::Flash { key: 7 });
        assert!(p.is_held(7), "the outline shows the finger");
        assert_eq!(p.up(7, 1, 200.0, Why::Up), Action::Nothing);
        // A down the socket refused after all.
        assert_eq!(p.down(8, 1, 0.0, true), down_sent(8));
        p.unsent(8);
        assert_eq!(p.up(8, 1, 200.0, Why::Up), Action::Nothing);
        p.unsent(9);
        assert!(!p.is_held(9), "unsent of a key nobody holds holds nothing");
    }

    #[test]
    fn leaving_sends_an_up_for_every_sent_key_in_key_order() {
        let mut p = Presses::default();
        p.down(9, 1, 100.0, true);
        p.down(2, 2, 200.0, true);
        p.down(2, 3, 250.0, true);
        p.down(5, 4, 300.0, false);
        assert_eq!(p.held_keys(), BTreeSet::from([2, 5, 9]));
        assert_eq!(
            p.leave_all(1000.0, Why::Tab),
            vec![up_sent(2, 800.0, Why::Tab), up_sent(9, 900.0, Why::Tab)]
        );
        assert_eq!(p.held_keys(), BTreeSet::new());
        assert_eq!(
            p.up(2, 2, 1100.0, Why::Up),
            Action::Nothing,
            "left: nothing more"
        );
        assert_eq!(p.leave_all(2000.0, Why::Hidden), vec![]);
    }

    #[test]
    fn a_closed_socket_forgets_every_hold_without_an_up() {
        let mut p = Presses::default();
        p.down(1, 1, 0.0, true);
        p.clear();
        assert!(!p.is_held(1));
        assert_eq!(
            p.up(1, 1, 50.0, Why::Up),
            Action::Nothing,
            "the hub releases it"
        );
    }

    #[test]
    fn the_why_of_an_up() {
        assert_eq!(Why::of_event("pointerup"), Some(Why::Up));
        assert_eq!(Why::of_event("pointercancel"), Some(Why::Cancel));
        assert_eq!(Why::of_event("lostpointercapture"), Some(Why::Lost));
        assert_eq!(Why::of_event("pointermove"), None);
        assert_eq!(
            [Why::Up, Why::Cancel, Why::Lost, Why::Hidden, Why::Tab].map(Why::name),
            ["up", "cancel", "lost", "hidden", "tab"]
        );
    }
}
