//! The hub connection's decisions (pure, unit-tested natively): the state
//! the surface keeps between the hub's events, and what each event makes
//! the store do. `LiveStore` (`live.rs`) carries them out with the socket,
//! the timers and the signals.
//!
//! A socket that stays silent is half-open (a Wi-Fi roam, an iPad that
//! slept): once the hub said hello the store pings it every `PING_MS` while
//! the page is visible (every `PING_HIDDEN_MS` while hidden), and a socket
//! that heard nothing for `SILENCE_MS` (no hello, no pong, no value) is
//! dropped and replaced. A socket that is open but never said hello talks
//! to a hub that does not speak this protocol: one bounded reload
//! (`net::on_missing_hello`), else it is replaced too.
//!
//! A ping (#43) carries its number, the page's clock and the round trip of
//! the latest pong on this socket with the number of the ping it measured:
//! the hub pairs the round trip with that ping's own time and arrival, and
//! logs every ping, so its event log resolves a stall of a few hundred ms.

use fohmixer_proto::client::ClientMsg;
use fohmixer_proto::layout::Layout;

use super::{Change, InstanceView, Wanted};
use crate::binding::SubSpec;
use crate::net;

/// The watchdog's period: a ping (after the hello, while the page is
/// visible) and a silence check.
pub const PING_MS: u64 = 100;
/// While the page is hidden it pings this often.
pub const PING_HIDDEN_MS: f64 = 1000.0;
/// A socket that heard nothing from the hub this long is replaced.
pub const SILENCE_MS: f64 = 3000.0;
/// A tick this long after the previous one fired late (a throttled
/// background tab, a blocked main thread): it says nothing about the socket.
const LATE_MS: f64 = 2000.0;

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
}

/// What a socket's close means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Closed {
    /// The store reconnects (it was not stopped).
    pub reconnect: bool,
    /// The socket had said hello: a connection was lost, which the page
    /// reports to the hub (#26, `diag::disconnected`). A socket that never
    /// said hello (an attempt while the hub is down, a reload close) is no
    /// news.
    pub lost: bool,
}

/// What an instance's new state asks of the store.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InstanceChange {
    /// It is offline: its slots wait for Live again (I8).
    pub pending: bool,
    /// It came online, loaded another set or is no longer busy: its
    /// parameter ranges are read again (a range read while Live was away or
    /// stalled failed; another set may have other ranges).
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
    /// The configured groups were unfolded (once per page, spec F7).
    unfolded: bool,
    /// The current socket's number (a new one per socket and per close).
    socket: u64,
    /// When the current socket last heard from the hub (page clock, ms).
    heard: f64,
    /// When the watchdog last ticked (page clock, ms).
    ticked: f64,
    /// The page is hidden (it pings less often).
    hidden: bool,
    /// The next ping's number.
    next_ping: u32,
    /// When the last ping went (page clock, ms).
    pinged: f64,
    /// The last pong on this socket: its ping's number and round trip (ms).
    rtt: Option<(u32, f64)>,
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
        self.ticked = now;
        self.rtt = None;
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
        Hello {
            specs: self.wanted.specs(),
        }
    }

    /// The socket closed (or was dropped): whether the store reconnects
    /// (not after `stop`) and whether a connection was lost. Its watchdog's
    /// next tick is `Done`.
    pub fn closed(&mut self) -> Closed {
        let lost = self.ready;
        self.ready = false;
        self.socket += 1;
        Closed {
            reconnect: !self.stopped,
            lost,
        }
    }

    /// The watchdog tick of socket `socket` at `now`; `open` is whether
    /// that socket is open. A tick that fired late starts a new silence
    /// window (the page, not the socket, was away).
    pub fn tick(&mut self, socket: u64, now: f64, open: bool) -> Tick {
        if self.stopped || socket != self.socket {
            return Tick::Done;
        }
        if now - self.ticked > LATE_MS {
            self.heard = now;
        }
        self.ticked = now;
        if now - self.heard < SILENCE_MS {
            if self.ready && self.ping_due(now) {
                Tick::Ping
            } else {
                Tick::Wait
            }
        } else if open && !self.hello_seen {
            Tick::NoHello
        } else {
            Tick::Silent
        }
    }

    /// Whether a ping is due at `now`: every tick while visible, every
    /// `PING_HIDDEN_MS` while hidden.
    fn ping_due(&self, now: f64) -> bool {
        !self.hidden || now - self.pinged >= PING_HIDDEN_MS
    }

    /// The page was hidden or shown.
    pub fn set_hidden(&mut self, hidden: bool) {
        self.hidden = hidden;
    }

    /// The ping to send at `now` (page clock) and page time `t` (the epoch
    /// clock of `set`): its number, `t`, and the latest pong's round trip on
    /// this socket with the number of the ping it measured.
    pub fn ping(&mut self, now: f64, t: f64) -> ClientMsg {
        let n = self.next_ping;
        self.next_ping = self.next_ping.wrapping_add(1);
        self.pinged = now;
        ClientMsg::Ping {
            n,
            t,
            rtt: self.rtt.map(|(_, rtt)| rtt),
            rtt_n: self.rtt.map(|(of, _)| of),
        }
    }

    /// The number the next ping carries: the pong of that ping proves that
    /// everything sent before it reached the hub (#43, PR C: the flight
    /// recorder's batches).
    pub fn next_ping(&self) -> u32 {
        self.next_ping
    }

    /// The pong of ping `n`, sent at page time `sent`, arrived at page time
    /// `now`: its round trip, kept for the next ping.
    pub fn pong(&mut self, n: u32, sent: f64, now: f64) -> f64 {
        let rtt = now - sent;
        self.rtt = Some((n, rtt));
        rtt
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

    /// Whether a page wants `key` now (a value for any other key is dropped,
    /// #43).
    pub fn wants(&self, key: &str) -> bool {
        self.wanted.contains(key)
    }

    /// Whether the page unfolds the configured groups now (spec F7): once
    /// per page, the first time a layout is on screen (`layout`) while the
    /// socket takes messages, after the hello or the first layout, whichever
    /// comes last.
    pub fn unfold_due(&mut self, layout: bool) -> bool {
        let due = layout && self.ready && !self.unfolded;
        self.unfolded |= due;
        due
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
/// first report). A stalled (busy) Live lets a range read time out, so the
/// ranges are read again once it is idle.
pub fn instance_change(old: Option<&InstanceView>, new: &InstanceView) -> InstanceChange {
    let back = old.is_none_or(|o| !o.online || o.busy || o.set_name != new.set_name);
    InstanceChange {
        pending: !new.online,
        ranges: new.online && back,
    }
}

/// Whether `new` brings its instance back online after `old` (none before
/// its first report): the store sends that instance's open writes again
/// (#43, L4). The hub forgets an instance's pending writes when it goes
/// away, and after a reconnect's hello it reports every instance again,
/// which a lost connection had marked offline. A busy instance getting idle
/// or one that loaded another set is not back: its writes were not dropped.
pub fn back_online(old: Option<&InstanceView>, new: &InstanceView) -> bool {
    new.online && old.is_none_or(|o| !o.online)
}

#[cfg(test)]
mod tests;
