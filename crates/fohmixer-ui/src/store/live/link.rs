//! The link's glue (#43): the dropout watch's own tick, the dropout counter
//! on the surface and the page's flight recorder's uploads. The decisions
//! are `behave::link` (`DropoutWatch`: what a dropout is, the counter and
//! its reset) and `diag::trace` (`Recorder`: what a batch holds, when it
//! goes, when it is proved logged); this carries them out with the socket,
//! a timer and a signal.
//!
//! A batch goes only from the tick, never from a message's handler: a
//! hello, a dropout or a reset asks for one as soon as the recorder's cap
//! lets it (#43 PR D), so the writes and subscriptions that a hello or an
//! instance back sets off are on the socket before it; and no batch goes in
//! a tick after a set went (`Recorder::set_went`).

use std::time::Duration;

use leptos::prelude::*;

use super::LiveStore;
use crate::diag::{self, trace::Recorder};
use crate::dom;
use crate::store::conn;

impl LiveStore {
    /// The dropout watch's own tick (#43), every `PING_MS` until the store
    /// stops, whatever the socket does: it keeps ticking while the page
    /// reconnects, so a lost socket counts from the page's next on-time tick.
    /// Each tick also shows the counter, tells the flight recorder the
    /// writes that turned `unconfirmed` or `not_sent` (#43 PR E) and sends
    /// its next batch when one is due.
    pub(super) fn tick_link(self) {
        set_timeout(
            move || {
                if self.stopped() {
                    return;
                }
                let _ = self
                    .inner
                    .try_update_value(|i| i.watch.tick(dom::epoch_now()));
                self.show_counter();
                self.note_intents();
                self.upload();
                self.tick_link();
            },
            Duration::from_millis(conn::PING_MS),
        );
    }

    /// The dropouts that ended go into the flight recorder, which sends them
    /// to the hub's event log at the next tick (#43, §4.4).
    pub(super) fn collect_dropouts(self) {
        let reports = self
            .inner
            .try_update_value(|i| i.watch.take_reports())
            .unwrap_or_default();
        if reports.is_empty() {
            return;
        }
        for report in &reports {
            dom::log(&format!(
                "the hub link dropped out for {:.0} ms{}",
                report.ms,
                if report.socket_lost {
                    " (the socket was lost)"
                } else {
                    ""
                }
            ));
            diag::record(&report.event());
        }
        let _ = diag::with_trace(Recorder::soon);
    }

    /// Sends the flight recorder's next batch when one is due (§5.2): on a
    /// socket that said hello and holds at most 1 KB unsent, with no set
    /// since the previous tick, within the recorder's cap; the pong of the
    /// next ping proves it logged. A batch the socket did not take goes
    /// again.
    fn upload(self) {
        let now = dom::epoch_now();
        let batch = self
            .inner
            .try_with_value(|i| {
                let (ready, buffered) = match &i.socket {
                    Some(socket) if socket.open() => (i.conn.ready(), socket.buffered()),
                    _ => (false, 0),
                };
                let proof = i.conn.next_ping();
                diag::with_trace(|r| r.upload(now, ready, buffered, proof)).flatten()
            })
            .flatten();
        let Some(text) = batch else {
            return;
        };
        if !self.send_text(&text) {
            dom::log("a flight recorder batch was not sent: it goes again");
            let _ = diag::with_trace(Recorder::requeue);
        }
    }

    /// The counter was tapped (#43, §4.4): it shows 0, and the reset goes to
    /// the hub's event log at the next tick.
    pub fn reset_dropouts(self) {
        let Some(event) = self
            .inner
            .try_update_value(|i| i.watch.reset(dom::epoch_now()))
        else {
            return;
        };
        dom::log(&format!(
            "the dropout counter was reset from {}",
            event["count"]
        ));
        diag::record(&event);
        let _ = diag::with_trace(Recorder::soon);
        self.show_counter();
    }

    /// The counter's signal follows the dropout watch (set only on a change,
    /// so the number is not drawn again every tick).
    pub(super) fn show_counter(self) {
        let Some(counter) = self.inner.try_with_value(|i| i.watch.counter()) else {
            return;
        };
        if self.dropouts.try_get_untracked() != Some(counter) {
            let _ = self.dropouts.try_set(counter);
        }
    }
}
