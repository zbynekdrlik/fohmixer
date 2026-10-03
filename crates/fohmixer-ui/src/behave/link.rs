//! The link's dropouts as the page sees them (#43, owner decision of
//! 2026-10-03; design note §4.4): the page pings the hub every 100 ms while
//! visible, and a **dropout** is one continuous interval in which it hears
//! nothing from the hub for at least [`DROPOUT_MS`] although a pong is due,
//! or the socket is down. Each interval counts once, however long it lasts
//! and whether or not its socket was lost on the way.
//!
//! A silence counts only while the page's own watchdog keeps ticking: a
//! page that was frozen or hidden (its timers late) cannot tell the link's
//! silence from its own.
//!
//! When a dropout ends (a message from the hub, or the next socket's hello)
//! it becomes a report for the hub's event log: when it started (page
//! clock), how long it lasted, whether the socket was lost, and the last
//! round trips before it. The counter on the surface (a number, red while
//! a dropout lasts, reset by a tap) is PR C; this is its measure.
//!
//! All times are the page's clock in ms since the epoch
//! (`performance.timeOrigin + performance.now()`, the clock of `set` and
//! `ping`).

use std::collections::VecDeque;

use fohmixer_proto::client::ClientMsg;
use serde_json::{Value, json};

/// Silence this long with a pong due is a dropout.
pub const DROPOUT_MS: f64 = 300.0;
/// The round trips a report carries (the latest before the dropout).
pub const RTTS_KEPT: usize = 5;
/// Unanswered pings kept (6.4 s at 100 ms: past the 3 s silence that drops
/// the socket); a ping beyond them is not tracked, the oldest stays.
pub const PINGS_KEPT: usize = 64;
/// Finished reports kept while they cannot be sent.
pub const REPORTS_KEPT: usize = 64;

/// One finished dropout.
#[derive(Debug, Clone, PartialEq)]
pub struct Dropout {
    /// When the page last heard the hub (or sent the ping still due).
    pub t: f64,
    /// How long it lasted.
    pub ms: f64,
    /// The socket was lost on the way.
    pub socket_lost: bool,
    /// The last round trips before it, oldest first.
    pub rtts: Vec<f64>,
}

impl Dropout {
    /// Its event for the hub's event log.
    pub fn event(&self) -> Value {
        json!({
            "ev": "dropout",
            "t": self.t,
            "ms": self.ms,
            "socket_lost": self.socket_lost,
            "rtts": self.rtts,
        })
    }
}

/// The `trace` that carries `reports` to the hub; none for no report.
pub fn trace(reports: &[Dropout]) -> Option<ClientMsg> {
    (!reports.is_empty()).then(|| ClientMsg::Trace {
        events: reports.iter().map(Dropout::event).collect(),
    })
}

/// Whether a silence of `gap` ms is a dropout.
pub fn is_dropout(gap: f64) -> bool {
    gap >= DROPOUT_MS
}

/// The dropout watch of one page.
#[derive(Debug, Default)]
pub struct DropoutWatch {
    /// A socket said hello and has not closed since.
    up: bool,
    /// A socket that said hello closed, and no hello came since.
    down: bool,
    /// When the page last heard the hub.
    heard: f64,
    /// When the page's watchdog last ticked.
    ticked: f64,
    /// Pings not answered yet: number, when sent.
    pings: VecDeque<(u32, f64)>,
    /// The latest round trips.
    rtts: VecDeque<f64>,
    /// A dropout in progress: its start, whether its socket was lost.
    open: Option<(f64, bool)>,
    /// Dropouts counted since the page loaded.
    count: u64,
    /// Finished dropouts not sent yet.
    reports: VecDeque<Dropout>,
}

impl DropoutWatch {
    /// Whether the page's own watchdog kept ticking up to `now` (its last
    /// tick is under [`DROPOUT_MS`] old). A page that was frozen or hidden
    /// (its timers late) cannot tell the link's silence from its own, so
    /// only a silence it lived through counts.
    fn ticking(&self, now: f64) -> bool {
        !is_dropout(now - self.ticked)
    }

    /// The start of the current silence while a pong is due: the last
    /// message heard, or the oldest unanswered ping when it is later.
    fn silence_start(&self) -> Option<f64> {
        self.pings.front().map(|&(_, sent)| sent.max(self.heard))
    }

    /// Opens a dropout that started at `start` (counted once).
    fn begin(&mut self, start: f64, socket_lost: bool) {
        if self.open.is_none() {
            self.count += 1;
        }
        let start = self.open.map_or(start, |(s, _)| s);
        self.open = Some((start, socket_lost));
    }

    /// Ends the dropout in progress at `now`, as a report.
    fn finish(&mut self, now: f64) {
        let Some((start, socket_lost)) = self.open.take() else {
            return;
        };
        if self.reports.len() >= REPORTS_KEPT {
            self.reports.pop_front();
        }
        self.reports.push_back(Dropout {
            t: start,
            ms: now - start,
            socket_lost,
            rtts: self.rtts.iter().copied().collect(),
        });
    }

    /// Whether a pong is due and the silence reached a dropout at `now`.
    fn silent(&self, now: f64) -> Option<f64> {
        self.silence_start().filter(|&s| is_dropout(now - s))
    }

    /// A socket said hello at `now`: a dropout in progress (its socket
    /// lost) ends.
    pub fn hello(&mut self, now: f64) {
        self.finish(now);
        self.up = true;
        self.down = false;
        self.heard = now;
        self.pings.clear();
    }

    /// Ping `n` was sent at `now`.
    pub fn ping(&mut self, n: u32, now: f64) {
        if self.up && self.pings.len() < PINGS_KEPT {
            self.pings.push_back((n, now));
        }
    }

    /// A message from the hub arrived at `now`: a silence that reached a
    /// dropout while the page kept ticking ends (it counts even when no tick
    /// saw it).
    pub fn heard(&mut self, now: f64) {
        if !self.up {
            self.heard = now;
            return;
        }
        if self.ticking(now)
            && let Some(start) = self.silent(now)
        {
            self.begin(start, false);
        }
        self.finish(now);
        self.heard = now;
    }

    /// Pong `n` came with round trip `rtt`: it answers every ping up to `n`.
    pub fn pong(&mut self, n: u32, rtt: f64) {
        self.pings.retain(|&(m, _)| m > n);
        if self.rtts.len() >= RTTS_KEPT {
            self.rtts.pop_front();
        }
        self.rtts.push_back(rtt);
    }

    /// The watchdog's tick at `now`: a silence that reached a dropout opens
    /// one (the counter goes up while it lasts). A late tick (the page was
    /// frozen or hidden) starts the silence over, and a socket that is down
    /// while the page ticks is a dropout from this tick.
    pub fn tick(&mut self, now: f64) {
        let late = !self.ticking(now);
        self.ticked = now;
        if late && self.open.is_none() {
            self.heard = now;
        }
        if self.down && !late {
            self.begin(now, true);
        }
        if self.up
            && let Some(start) = self.silent(now)
        {
            self.begin(start, false);
        }
    }

    /// The socket that said hello closed at `now`: a dropout until the next
    /// hello, from the start of the silence that led to it, if any. While the
    /// page is not ticking (frozen, hidden) it starts at the page's next tick.
    pub fn lost(&mut self, now: f64) {
        if !self.up {
            return;
        }
        if self.open.is_some() || self.ticking(now) {
            let start = self.silence_start().unwrap_or(now);
            self.begin(start, true);
        }
        self.up = false;
        self.down = true;
        self.pings.clear();
    }

    /// Whether a dropout lasts now.
    pub fn active(&self) -> bool {
        self.open.is_some()
    }

    /// Dropouts counted since the page loaded.
    pub fn count(&self) -> u64 {
        self.count
    }

    /// The finished dropouts not sent yet, taken.
    pub fn take_reports(&mut self) -> Vec<Dropout> {
        self.reports.drain(..).collect()
    }

    /// Reports that could not be sent go back, ahead of newer ones.
    pub fn requeue(&mut self, reports: Vec<Dropout>) {
        for report in reports.into_iter().rev() {
            if self.reports.len() >= REPORTS_KEPT {
                return;
            }
            self.reports.push_front(report);
        }
    }
}

#[cfg(test)]
#[path = "link/tests.rs"]
mod tests;
