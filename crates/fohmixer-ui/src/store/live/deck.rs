//! The Stream Deck's glue in the store (#52): the hub's `deck`, `deck_keys`
//! and `deck_ack` into the signals and the waiting presses
//! (`store/deck.rs`, tested), the page's `deck_view` (sent again after every
//! hello while the tab is open: the hub forgets a viewer whose socket
//! closed) and `deck_press` (sent at once or not at all: never queued, never
//! sent again after a reconnect, #52 §6). A socket close leaves the keys as
//! they were, dimmed (`online` false), and forgets the waiting presses (the
//! hub's `Detach` releases what this page held).

use fohmixer_proto::client::{ClientMsg, DeckKey};
use leptos::prelude::*;

use super::{FailFn, LiveStore, Socket};
use crate::dom;
use crate::store::deck::DeckInfo;

impl LiveStore {
    /// The hub's `deck`.
    pub(super) fn on_deck(self, info: DeckInfo) {
        let _ = self.deck.try_set(Some(info));
    }

    /// Keys' new states (this page views the tab).
    pub(super) fn on_deck_keys(self, items: Vec<DeckKey>) {
        let _ = self.deck_keys.try_update(|keys| {
            for item in items {
                keys.insert(item.key, item);
            }
        });
    }

    /// A press's answer: a failed down flashes its key; nothing is retried.
    pub(super) fn on_deck_ack(self, seq: u64, ok: bool, error: Option<String>) {
        if let Some(Some(flash)) = self.inner.try_update_value(|i| i.deck.ack(seq, ok)) {
            let why = error.unwrap_or_default();
            dom::log(&format!("Stream Deck press {seq} failed: {why}"));
            flash(why);
        }
    }

    /// The tab opened (`on`) or closed: only a page viewing it gets the keys.
    pub fn deck_view(self, on: bool) {
        let _ = self.inner.try_update_value(|i| i.deck_viewing = on);
        self.send(&ClientMsg::DeckView { on });
    }

    /// After a hello: the deck info starts over (the hub's `deck` sets it
    /// again when it has a Companion), and the open tab tells the hub again.
    pub(super) fn deck_hello(self) {
        // P3: the hub has a `[companion]` only if its `deck` follows this
        // hello; an open page loses the tab when it does not.
        let _ = self.deck.try_set(None);
        if self.inner.try_with_value(|i| i.deck_viewing) == Some(true) {
            self.send(&ClientMsg::DeckView { on: true });
        }
    }

    /// The socket closed: Companion's state unknown until the next hello.
    pub(super) fn deck_closed(self) {
        let _ = self.inner.try_update_value(|i| i.deck.clear());
        let _ = self.deck.try_update(|deck| {
            if let Some(deck) = deck {
                deck.online = false;
            }
        });
    }

    /// Whether a press can go now: the socket open and past its hello.
    pub fn can_send(self) -> bool {
        self.inner
            .try_with_value(|i| i.conn.ready() && i.socket.as_ref().is_some_and(Socket::open))
            .unwrap_or(false)
    }

    /// A Stream Deck press (`down`; an up's measured hold and why): onto the
    /// socket at once with the page's own number, or not at all (`Err` hands
    /// `on_fail` back to flash the key).
    pub fn deck_press(
        self,
        key: u32,
        down: bool,
        hold_ms: Option<f64>,
        why: Option<&str>,
        on_fail: FailFn,
    ) -> Result<u64, FailFn> {
        let Some(seq) = self.inner.try_update_value(|i| i.deck.next_seq()) else {
            return Err(on_fail);
        };
        let msg = ClientMsg::DeckPress {
            key,
            down,
            seq,
            t: dom::epoch_now(),
            hold_ms,
            why: why.map(str::to_string),
        };
        if !self.send(&msg) {
            return Err(on_fail);
        }
        let _ = self
            .inner
            .try_update_value(|i| i.deck.sent(seq, down, on_fail));
        Ok(seq)
    }
}
