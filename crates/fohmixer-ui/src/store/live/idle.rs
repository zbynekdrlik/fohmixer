//! The page's idle glue (#51): carries out `behave::idle`'s `IdleWatch`
//! with the socket, the link's rules and a signal. A `visibilitychange`
//! listener on the document (and the link's tick, should a change be
//! missed) makes a hidden page (a locked iPad, the app in the background)
//! Idle at once and sends `idle {on: true, meter_ms: 0}` from the handler
//! itself, before iOS freezes the page; shown again, the page wakes:
//! `idle {on: false}`, then a ping at once. Each turn sets the watchdog's
//! and the dropout watch's idle rules (`Conn::set_idle`,
//! `DropoutWatch::set_idle`), goes into the flight recorder as an essential
//! `idle` event and writes the store's `idle` signal (`data-idle` on the
//! surface; nothing visible). Each hello's socket has a new outbox at full
//! rate, so an idle page tells it again (`idle_hello`).

use fohmixer_proto::client::ClientMsg;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::LiveStore;
use crate::behave::idle::{self, Sleep, Turn};
use crate::diag;
use crate::dom;

impl LiveStore {
    /// Listens for the page hidden or shown (from `start`; `stop` removes
    /// the listener), and takes the page's state of now.
    pub(super) fn listen_idle(self) {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let on_visibility =
            Closure::wrap(Box::new(move || self.sync_visibility()) as Box<dyn FnMut()>);
        let _ = document.add_event_listener_with_callback(
            "visibilitychange",
            on_visibility.as_ref().unchecked_ref(),
        );
        let _ = self
            .inner
            .try_update_value(|i| i.on_visibility = Some(on_visibility));
        self.sync_visibility();
    }

    /// Removes the `visibilitychange` listener (the store stops).
    pub(super) fn unlisten_idle(self) {
        let listener = self
            .inner
            .try_update_value(|i| i.on_visibility.take())
            .flatten();
        let document = web_sys::window().and_then(|w| w.document());
        if let (Some(listener), Some(document)) = (listener, document) {
            let _ = document.remove_event_listener_with_callback(
                "visibilitychange",
                listener.as_ref().unchecked_ref(),
            );
        }
    }

    /// The page hidden or shown now (its `visibilitychange`, and the link's
    /// tick in case one was missed): the turn this makes, carried out.
    pub(super) fn sync_visibility(self) {
        let (now, hidden) = (dom::epoch_now(), dom::hidden());
        let turn = self
            .inner
            .try_update_value(|i| i.idle.visibility(now, hidden))
            .flatten();
        if let Some(turn) = turn {
            self.turn_idle(turn, now);
        }
    }

    /// Carries out a turn at page time `now`: the link's rules, the hub's
    /// meters (a sleep's `idle`; a wake's `idle` off and a ping at once, so
    /// the radio and the hub hear the page first), the recorder's event and
    /// the signal.
    fn turn_idle(self, turn: Turn, now: f64) {
        let on = matches!(turn, Turn::Sleep(_));
        let clock = dom::now();
        let _ = self.inner.try_update_value(|i| {
            i.conn.set_idle(on, clock);
            i.watch.set_idle(on, now);
        });
        diag::record(&turn.event(now));
        match turn {
            Turn::Sleep(why) => {
                let meter_ms = self.meter_ms(why);
                dom::log(&format!(
                    "the page is idle ({}): its meters at most every {meter_ms} ms (0: none)",
                    why.name()
                ));
                self.send_idle(meter_ms);
            }
            Turn::Wake { why, idle_ms } => {
                dom::log(&format!(
                    "the page woke ({}) after {idle_ms:.0} ms idle",
                    why.name()
                ));
                self.send(&ClientMsg::Idle {
                    on: false,
                    meter_ms: None,
                });
                self.ping_now();
            }
        }
        if self.idle.try_get_untracked() != Some(on) {
            let _ = self.idle.try_set(on);
        }
    }

    /// The meter period a page idle for `why` asks for: none when hidden,
    /// else the layout's `idle_meter_ms` (`behave::idle::meter_ms`).
    fn meter_ms(self, why: Sleep) -> u32 {
        let setting = self
            .layout
            .try_with_untracked(|l| l.as_ref().and_then(|l| l.config.idle_meter_ms))
            .flatten();
        idle::meter_ms(why, setting)
    }

    /// Tells the hub the page is idle, its meters at most every `meter_ms`
    /// (0: none until it wakes).
    fn send_idle(self, meter_ms: u32) {
        self.send(&ClientMsg::Idle {
            on: true,
            meter_ms: Some(meter_ms),
        });
    }

    /// After a hello: an idle page tells the new socket's outbox, which
    /// starts at full rate.
    pub(super) fn idle_hello(self) {
        let why = self.inner.try_with_value(|i| i.idle.sleeping()).flatten();
        if let Some(why) = why {
            self.send_idle(self.meter_ms(why));
        }
    }

    /// A ping now (a wake's), when the socket said hello.
    fn ping_now(self) {
        let ping = self
            .inner
            .try_update_value(|i| i.conn.ready().then(|| i.ping(dom::now(), dom::epoch_now())))
            .flatten();
        if let Some(ping) = ping {
            self.send(&ping);
        }
    }
}
