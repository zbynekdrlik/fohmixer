//! The page's idle glue (#51): carries out `behave::idle`'s `IdleWatch`
//! with the socket, the link's rules and a signal. A `visibilitychange`
//! listener on the document (and the link's tick, should a change be
//! missed) makes a hidden page (a locked iPad, the app in the background)
//! Idle at once and sends `idle {on: true, meter_ms: 0}` from the handler
//! itself, before iOS freezes the page; shown again, the page wakes:
//! `idle {on: false}`, then a ping at once.
//!
//! Touch-idle (PR 2b): the link's tick makes a page left untouched for the
//! layout's `idle_after_s` Idle (`tick_idle`; absent or 0: never, the
//! default), unless a finger is down, the Pro-Q 4 screen is open or an
//! input on its editor is not shown yet (`behave::idle::holds`). A
//! capture-phase, passive `pointerdown` listener on the window (`touched`)
//! wakes it before any control sees the touch: the control's own handler
//! then runs on a page already awake.
//!
//! Each turn sets the watchdog's and the dropout watch's idle rules
//! (`Conn::set_idle`, `DropoutWatch::set_idle`), goes into the flight
//! recorder as an essential `idle` event and writes the store's `idle`
//! signal (`data-idle` on the surface; nothing visible). Each hello's
//! socket has a new outbox at full rate, so an idle page tells it again
//! (`idle_hello`).

use fohmixer_proto::client::ClientMsg;
use leptos::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use super::LiveStore;
use crate::behave::idle::{self, Sleep, Turn};
use crate::diag;
use crate::dom;

impl LiveStore {
    /// Listens for the page hidden or shown and for every pointer down
    /// (from `start`; `stop` removes the listeners), and takes the page's
    /// state of now.
    pub(super) fn listen_idle(self) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let on_touch = Closure::wrap(Box::new(move || self.touched()) as Box<dyn FnMut()>);
        let options = web_sys::AddEventListenerOptions::new();
        options.set_capture(true);
        options.set_passive(true);
        let _ = window.add_event_listener_with_callback_and_add_event_listener_options(
            "pointerdown",
            on_touch.as_ref().unchecked_ref(),
            &options,
        );
        let _ = self.inner.try_update_value(|i| i.on_touch = Some(on_touch));
        let Some(document) = window.document() else {
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

    /// Removes the `visibilitychange` and `pointerdown` listeners (the store
    /// stops).
    pub(super) fn unlisten_idle(self) {
        let (on_visibility, on_touch) = self
            .inner
            .try_update_value(|i| (i.on_visibility.take(), i.on_touch.take()))
            .unwrap_or_default();
        let window = web_sys::window();
        if let (Some(listener), Some(window)) = (on_touch, &window) {
            let _ = window.remove_event_listener_with_callback_and_bool(
                "pointerdown",
                listener.as_ref().unchecked_ref(),
                true,
            );
        }
        let document = window.and_then(|w| w.document());
        if let (Some(listener), Some(document)) = (on_visibility, document) {
            let _ = document.remove_event_listener_with_callback(
                "visibilitychange",
                listener.as_ref().unchecked_ref(),
            );
        }
    }

    /// A pointer went down anywhere on the page (the window's capture-phase
    /// listener, before any control's handler): the countdown starts over,
    /// and a page idle for no touch wakes at once.
    fn touched(self) {
        let now = dom::epoch_now();
        let turn = self
            .inner
            .try_update_value(|i| i.idle.touched(now))
            .flatten();
        if let Some(turn) = turn {
            self.turn_idle(turn, now);
        }
    }

    /// Touch-idle's tick (from the link's tick): the layout's
    /// `idle_after_s`, read untracked so a new layout counts from the next
    /// tick, and whether something holds the page Active (a finger down,
    /// the open Pro-Q 4 screen, an input on its editor not shown yet).
    pub(super) fn tick_idle(self) {
        let setting = self
            .layout
            .try_with_untracked(|l| l.as_ref().and_then(|l| l.config.idle_after_s))
            .flatten();
        let eq_open = self
            .inner
            .try_with_value(|i| i.eq_frames.is_some())
            .unwrap_or(false);
        let held = idle::holds(diag::finger_down(), eq_open, self.eq_caught_up());
        let now = dom::epoch_now();
        let turn = self
            .inner
            .try_update_value(|i| i.idle.tick(now, idle::idle_after_ms(setting), held))
            .flatten();
        if let Some(turn) = turn {
            self.turn_idle(turn, now);
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
