//! The hub connection's decisions (pure, unit-tested natively): the state
//! the surface keeps between the hub's events, and what each event makes
//! the store do. `LiveStore` (`live.rs`) carries them out with the socket,
//! the timers and the signals.
//!
//! A socket that stays silent is half-open (a Wi-Fi roam, an iPad that
//! slept): once the hub said hello the store pings it every `PING_MS`, and
//! a socket that heard nothing for `SILENCE_MS` (no hello, no pong, no
//! value) is dropped and replaced. A socket that is open but never said
//! hello talks to a hub that does not speak this protocol: one bounded
//! reload (`net::on_missing_hello`), else it is replaced too.

use fohmixer_proto::layout::Layout;

use super::{Change, InstanceView, Wanted};
use crate::binding::SubSpec;
use crate::net;

/// The watchdog's period: a ping (after the hello) and a silence check.
pub const PING_MS: u64 = 1000;
/// A socket that heard nothing from the hub this long is replaced.
pub const SILENCE_MS: f64 = 3000.0;

/// What the watchdog does on one of its ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tick {
    /// The socket it watched is gone (replaced or closed, or the store
    /// stopped): the watchdog ends.
    Done,
    /// Nothing to do yet (no hello to ping after).
    Wait,
    /// Ping the hub.
    Ping,
    /// Open without a hello: the hub does not speak this protocol.
    NoHello,
    /// Silent too long: drop the socket and reconnect.
    Silent,
}

/// What a hello starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    /// Every wanted subscription, sent again.
    pub specs: Vec<SubSpec>,
    /// The page's first hello: the automatic refresh follows (spec F6).
    pub auto_refresh: bool,
}

/// What an instance's new state asks of the store.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InstanceChange {
    /// It is offline: its slots wait for Live again (I8).
    pub pending: bool,
    /// It came online or loaded another set: its parameter ranges are read
    /// again (a range read while Live was away failed; another set may
    /// have other ranges).
    pub ranges: bool,
}

/// The connection's state.
#[derive(Debug, Default)]
pub struct Conn {
    wanted: Wanted,
    /// The current socket said hello: messages may be sent.
    ready: bool,
    hello_seen: bool,
    /// Reconnect attempts since the last hello.
    attempt: u32,
    /// The last command id.
    last_id: u64,
    /// The layout revision on screen.
    rev: u64,
    stopped: bool,
    /// The automatic refresh ran (once per page).
    refreshed: bool,
    /// The current socket's number (a new one per socket and per close).
    socket: u64,
    /// When the current socket last heard from the hub (page clock, ms).
    heard: f64,
}

impl Conn {
    /// The surface unmounts: nothing reconnects any more.
    pub fn stop(&mut self) {
        self.stopped = true;
        self.ready = false;
    }

    pub fn stopped(&self) -> bool {
        self.stopped
    }

    /// Whether the socket takes messages (it said hello).
    pub fn ready(&self) -> bool {
        self.ready
    }

    /// The wait before the next reconnect attempt (the schedule starts
    /// again at a hello).
    pub fn retry_delay(&mut self) -> f64 {
        let wait = net::reconnect_delay(self.attempt);
        self.attempt = self.attempt.saturating_add(1);
        wait
    }

    /// A new socket was opened at `now`: its number (its watchdog's key).
    pub fn opened(&mut self, now: f64) -> u64 {
        self.socket += 1;
        self.ready = false;
        self.hello_seen = false;
        self.heard = now;
        self.socket
    }

    /// Any message from the hub, at `now`.
    pub fn heard(&mut self, now: f64) {
        self.heard = now;
    }

    /// The hub's hello (a protocol this page speaks), at `now`.
    pub fn hello(&mut self, now: f64) -> Hello {
        self.ready = true;
        self.hello_seen = true;
        self.attempt = 0;
        self.heard = now;
        let auto_refresh = !self.refreshed;
        self.refreshed = true;
        Hello {
            specs: self.wanted.specs(),
            auto_refresh,
        }
    }

    /// The socket closed (or was dropped): whether the store reconnects
    /// (not after `stop`). Its watchdog's next tick is `Done`.
    pub fn closed(&mut self) -> bool {
        self.ready = false;
        self.socket += 1;
        !self.stopped
    }

    /// The watchdog tick of socket `socket` at `now`; `open` is whether
    /// that socket is open.
    pub fn tick(&self, socket: u64, now: f64, open: bool) -> Tick {
        if self.stopped || socket != self.socket {
            Tick::Done
        } else if now - self.heard < SILENCE_MS {
            if self.ready { Tick::Ping } else { Tick::Wait }
        } else if open && !self.hello_seen {
            Tick::NoHello
        } else {
            Tick::Silent
        }
    }

    /// A new wanted set: what to unsubscribe and subscribe, and whether the
    /// socket takes it now (otherwise the next hello sends the whole set).
    pub fn want(&mut self, specs: Vec<SubSpec>) -> (Change, bool) {
        (self.wanted.replace(specs), self.ready)
    }

    /// How many subscriptions are wanted.
    pub fn wanted_len(&self) -> usize {
        self.wanted.len()
    }

    /// The wanted keys of `instance` (every key for `None`).
    pub fn keys_of(&self, instance: Option<&str>) -> Vec<String> {
        self.wanted.keys_of(instance)
    }

    /// REFRESH ALL: every wanted subscription, when the socket takes
    /// messages.
    pub fn refresh(&self) -> Option<Vec<SubSpec>> {
        self.ready.then(|| self.wanted.specs())
    }

    /// A new command id.
    pub fn next_id(&mut self) -> String {
        self.last_id += 1;
        format!("c{}", self.last_id)
    }

    /// Whether layout revision `rev` is the one on screen.
    pub fn shows(&self, rev: u64) -> bool {
        self.rev == rev
    }

    /// Layout revision `rev` is on screen.
    pub fn showing(&mut self, rev: u64) {
        self.rev = rev;
    }
}

/// Whether a served layout replaces the one on screen: only a different
/// one does. Every reconnect serves the layout again (a hub restart even
/// starts its revisions over); replacing an equal layout would rebuild
/// every control under the engineer's fingers (I4).
pub fn replaces(current: Option<&Layout>, served: &Layout) -> bool {
    current != Some(served)
}

/// What an instance's new state `new` asks, after `old` (none before its
/// first report).
pub fn instance_change(old: Option<&InstanceView>, new: &InstanceView) -> InstanceChange {
    let back = old.is_none_or(|o| !o.online || o.set_name != new.set_name);
    InstanceChange {
        pending: !new.online,
        ranges: new.online && back,
    }
}

#[cfg(test)]
mod tests;
