//! The router's half of the Stream Deck (#52, spec §5): it sends each
//! client the `deck` message (on attach and on Companion's link changes),
//! the keys only to the clients viewing the tab (through their outboxes,
//! after everything else), forwards the pages' presses through the holder
//! set to the Companion task, acks each press to its page (at once, or with
//! Companion's answer), and makes the releases no page can: a closed socket
//! (`detach`), a holding page silent for 2 s (`silent`), the keys held when
//! Companion was lost (`reconnect`, released after the next `ADD-DEVICE OK`:
//! Companion keeps a held key held when its surface goes away) and the stop
//! (`stop`, before the task's `REMOVE-DEVICE`). A key held when Companion
//! was lost and still not released at the stop (the hub stops before the
//! link is back) cannot be released: it is logged as `lost`, warn class, and
//! nothing is sent for it. Every hop is an event-log record (`deck_link`,
//! `deck_press`, `deck_ok`, `deck_release`, `deck_key`, `deck_keys`,
//! `deck_view`). The decisions are `crate::deck`'s.

use fohmixer_proto::client::{DeckKey, ServerMsg};

use super::Router;
use crate::companion::{CompanionEvent, CompanionHandle, Press};
use crate::config::CompanionCfg;
use crate::deck::{
    Deck, PressOutcome, PressRecord, StopKeys, link_fields, ok_fields, press_fields,
    release_fields, view_fields,
};
use crate::live::subs::ClientId;
use crate::outbox::Outbox;

/// A page's key press as the router takes it.
pub(super) struct PressMsg {
    pub client: ClientId,
    pub key: u32,
    pub down: bool,
    pub seq: u64,
    pub t: f64,
    pub hold_ms: Option<f64>,
    pub why: Option<String>,
    /// When it reached the hub (hub UTC ms).
    pub hub_ms: f64,
    /// The page clock's offset as of the socket's last ping.
    pub offset_ms: Option<f64>,
}

/// The Stream Deck's part of the router.
pub(super) struct DeckIo {
    cfg: CompanionCfg,
    companion: CompanionHandle,
    state: Deck,
}

impl Router {
    /// The router of a hub with `[companion]` (#52).
    #[must_use]
    pub fn with_deck(mut self, cfg: CompanionCfg, companion: CompanionHandle) -> Self {
        self.deck = Some(DeckIo {
            cfg,
            companion,
            state: Deck::default(),
        });
        self
    }

    /// The `deck` message of the Stream Deck now.
    fn deck_msg(io: &DeckIo) -> ServerMsg {
        ServerMsg::Deck {
            online: io.state.online(),
            columns: io.cfg.columns,
            rows: io.cfg.rows,
            title: io.cfg.title.clone(),
        }
    }

    /// A new client: the Stream Deck's state (none without `[companion]`:
    /// the page shows no tab), and it counts as heard.
    pub(super) fn deck_attach(&mut self, client: ClientId, outbox: &Outbox) {
        let now = self.now_ms();
        if let Some(io) = self.deck.as_mut() {
            io.state.heard(client, now);
            outbox.reply(Self::deck_msg(io));
        }
    }

    /// Something came from `client`.
    pub(super) fn deck_heard(&mut self, client: ClientId) {
        let now = self.now_ms();
        if let Some(io) = self.deck.as_mut() {
            io.state.heard(client, now);
        }
    }

    /// `client` opened the tab (the whole cache goes to it) or closed it.
    pub(super) fn deck_view(&mut self, client: ClientId, on: bool) {
        let Some(io) = self.deck.as_mut() else {
            return;
        };
        let keys = io.state.view(client, on);
        if let Some(outbox) = self.clients.get(&client) {
            match keys {
                Some(keys) => {
                    for key in keys {
                        outbox.deck_key(key);
                    }
                }
                None => outbox.forget_deck(),
            }
        }
        self.io.events.record("deck_view", view_fields(client, on));
    }

    /// A page's press: through the holder set to Companion, recorded, acked
    /// at once unless forwarded (then Companion's answer acks it).
    pub(super) fn deck_press(&mut self, p: PressMsg) {
        let now = self.now_ms();
        let (outcome, gap_ms, hub_hold_ms, holders) = match self.deck.as_mut() {
            None => (PressOutcome::NoDeck, None, None, 0),
            Some(io) => {
                let gap_ms = io.state.gap(p.client, p.key, p.hub_ms);
                let outcome = io.state.press(p.client, p.key, p.down);
                let mut hub_hold_ms = None;
                if outcome == PressOutcome::Forwarded {
                    hub_hold_ms = io.state.forwarded(p.key, p.down, now);
                    io.companion.press(Press {
                        key: p.key,
                        down: p.down,
                        from: Some((p.client, p.seq)),
                    });
                }
                (outcome, gap_ms, hub_hold_ms, io.state.holders_of(p.key))
            }
        };
        let record = PressRecord {
            client: p.client,
            peer: self.peers.get(&p.client).map(String::as_str),
            key: p.key,
            down: p.down,
            seq: p.seq,
            t: p.t,
            hub_ms: p.hub_ms,
            offset_ms: p.offset_ms,
            gap_ms,
            hold_ms: p.hold_ms,
            why: p.why.as_deref(),
            hub_hold_ms,
            outcome,
            holders,
        };
        self.io.events.record("deck_press", press_fields(&record));
        if let Some((ok, error)) = outcome.ack() {
            self.deck_ack(p.client, p.seq, ok, error.map(str::to_string), None);
        }
    }

    /// A `deck_ack` to its page (gone pages get nothing).
    fn deck_ack(
        &self,
        client: ClientId,
        seq: u64,
        ok: bool,
        error: Option<String>,
        rtt_ms: Option<f64>,
    ) {
        if let Some(outbox) = self.clients.get(&client) {
            outbox.reply(ServerMsg::DeckAck {
                seq,
                ok,
                error,
                rtt_ms,
            });
        }
    }

    /// A release the hub makes itself (`reason`: detach, silent, reconnect,
    /// stop), for `client` (none: the hub's own).
    fn deck_release(&mut self, client: Option<ClientId>, key: u32, reason: &str) {
        let now = self.now_ms();
        let Some(io) = self.deck.as_mut() else {
            return;
        };
        let hub_hold_ms = io.state.forwarded(key, false, now);
        io.companion.press(Press {
            key,
            down: false,
            from: None,
        });
        self.io.events.record(
            "deck_release",
            release_fields(client, key, reason, hub_hold_ms),
        );
        tracing::info!(client = ?client, key, reason, "the hub released a Stream Deck key");
    }

    /// A client is gone: the keys only it held are released.
    pub(super) fn deck_detach(&mut self, client: ClientId) {
        let Some(io) = self.deck.as_mut() else {
            return;
        };
        for key in io.state.detach(client) {
            self.deck_release(Some(client), key, "detach");
        }
    }

    /// The Stream Deck's tick: holding pages silent for 2 s released; the
    /// minute's summary.
    pub(super) fn deck_tick(&mut self) {
        let now = self.now_ms();
        let Some(io) = self.deck.as_mut() else {
            return;
        };
        let silent = io.state.silent_clients(now);
        let summary = io.state.summary(now);
        for (client, keys) in silent {
            for key in keys {
                self.deck_release(Some(client), key, "silent");
            }
        }
        if let Some(fields) = summary {
            self.io.events.record("deck_keys", fields);
        }
    }

    /// The Companion task's event.
    pub(super) fn deck_event(&mut self, event: CompanionEvent) {
        let now = self.now_ms();
        let Some(io) = self.deck.as_mut() else {
            return;
        };
        match event {
            CompanionEvent::Up {
                companion,
                api,
                attempts,
                down_ms,
            } => {
                let stale = io.state.link_up();
                self.io.events.record(
                    "deck_link",
                    link_fields(
                        "up",
                        Some(companion.as_str()),
                        Some(api.as_str()),
                        None,
                        down_ms,
                        Some(attempts),
                    ),
                );
                self.deck_broadcast();
                for key in stale {
                    self.deck_release(None, key, "reconnect");
                }
            }
            CompanionEvent::Down { error } => {
                io.state.link_down();
                self.io.events.record(
                    "deck_link",
                    link_fields("down", None, None, Some(error.as_str()), None, None),
                );
                self.deck_broadcast();
            }
            CompanionEvent::Failed {
                error,
                refused,
                companion,
                api,
                attempts,
            } => {
                let state = if refused { "refused" } else { "down" };
                self.io.events.record(
                    "deck_link",
                    link_fields(
                        state,
                        companion.as_deref(),
                        api.as_deref(),
                        Some(error.as_str()),
                        None,
                        Some(attempts),
                    ),
                );
            }
            CompanionEvent::Key(update) => {
                let (key, record) = io.state.apply(&update, now);
                if let Some(fields) = record {
                    self.io.events.record("deck_key", fields);
                }
                self.deck_fan_out(&[key]);
            }
            CompanionEvent::Clear => {
                let keys = io.state.clear();
                self.deck_fan_out(&keys);
            }
            CompanionEvent::Answered(answer) => {
                self.io.events.record("deck_ok", ok_fields(&answer));
                if let Some((client, seq)) = answer.press.from {
                    self.deck_ack(client, seq, answer.ok, answer.error, answer.rtt_ms);
                }
            }
        }
    }

    /// Keys to the clients viewing the tab.
    fn deck_fan_out(&self, keys: &[DeckKey]) {
        let Some(io) = &self.deck else {
            return;
        };
        for client in io.state.viewers() {
            if let Some(outbox) = self.clients.get(&client) {
                for key in keys {
                    outbox.deck_key(key.clone());
                }
            }
        }
    }

    /// The `deck` message to every client (the link went up or down).
    fn deck_broadcast(&self) {
        let Some(io) = &self.deck else {
            return;
        };
        let msg = Self::deck_msg(io);
        for outbox in self.clients.values() {
            outbox.reply(msg.clone());
        }
    }

    /// The hub stops: every held key released, then the task told to stop
    /// (its `REMOVE-DEVICE` goes after the releases: one channel, in order).
    /// A key held when Companion was lost, with the link still down, cannot
    /// be released: it is logged `lost` and nothing is sent for it.
    pub(super) fn deck_stop(&mut self) {
        let Some(io) = self.deck.as_mut() else {
            return;
        };
        let StopKeys { release, lost } = io.state.stop();
        for key in release {
            self.deck_release(None, key, "stop");
        }
        for key in lost {
            self.io
                .events
                .record("deck_release", release_fields(None, key, "lost", None));
            tracing::warn!(
                key,
                "a Stream Deck key held when Companion was lost stays held there: the hub stops before the link is back"
            );
        }
        if let Some(io) = &self.deck {
            io.companion.stop();
        }
    }
}
