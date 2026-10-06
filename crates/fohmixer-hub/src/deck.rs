//! The router's Stream Deck state (#52, spec §5, §8), pure: whether
//! Companion's link is up, Companion's keys as the pages get them, the
//! clients viewing the tab, who holds which key ([`holders`]), when each
//! client was last heard (a holding client silent for 2 s is released), the
//! keys held when the link went (released once it is back: Companion keeps
//! a held key held when its surface goes away), and when a key's change is
//! worth a `deck_key` record (its pressed flag; 10 s after a press on it;
//! once a second while viewed; a `deck_keys` summary every minute). The
//! router (`router/deck.rs`) carries the decisions out. Times are the
//! router's clock (ms).

pub mod holders;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use fohmixer_proto::client::DeckKey;
use serde_json::{Value, json};

pub use holders::{Hold, Holders};

use crate::clock::one_way_delay;
use crate::companion::{Answer, KeyUpdate};
use crate::live::subs::ClientId;

/// A holding client silent this long (ms) is released.
pub const SILENT_MS: f64 = 2000.0;
/// Every change of a key is recorded this long (ms) after a press on it.
pub const PRESS_WINDOW_MS: f64 = 10_000.0;
/// While a client views the tab, a key's changes are recorded this often at
/// most (ms).
pub const KEY_LOG_EVERY_MS: f64 = 1000.0;
/// The `deck_keys` summary comes this often (ms) while the link is up.
pub const SUMMARY_EVERY_MS: f64 = 60_000.0;
/// The router's Stream Deck tick (the silence, the summary).
pub const TICK: Duration = Duration::from_millis(100);
/// The answer to a press on a hub without `[companion]`.
pub const NO_DECK: &str = "no Stream Deck";
/// A cleared key's colour.
pub const BLACK: &str = "#000000";

/// Whether a holding client heard nothing for `since_ms` is silent.
pub fn silent(since_ms: f64) -> bool {
    since_ms >= SILENT_MS
}

/// Whether a key pressed `since_ms` ago is still in its press window.
pub fn in_press_window(since_ms: f64) -> bool {
    since_ms <= PRESS_WINDOW_MS
}

/// Whether a viewed key's record is due, its last one `since_logged_ms` ago.
pub fn key_log_due(since_logged_ms: Option<f64>) -> bool {
    since_logged_ms.is_none_or(|ms| ms >= KEY_LOG_EVERY_MS)
}

/// Whether the summary is due, the last one `since_ms` ago.
pub fn summary_due(since_ms: f64) -> bool {
    since_ms >= SUMMARY_EVERY_MS
}

/// Whether a key's change is a `deck_key` record now: its pressed flag
/// changed; or a press on it came at most 10 s ago; or a client views the
/// tab and the key's last record is at least 1 s old.
pub fn key_record(
    pressed_changed: bool,
    since_press_ms: Option<f64>,
    viewing: bool,
    since_logged_ms: Option<f64>,
) -> bool {
    pressed_changed
        || since_press_ms.is_some_and(in_press_window)
        || (viewing && key_log_due(since_logged_ms))
}

/// FNV-1a (64 bit) of `text` as 16 hex digits: a key image's identity in the
/// event log (the image itself never is).
pub fn img_hash(text: &str) -> String {
    let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    format!("{hash:016x}")
}

/// What became of a page's press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressOutcome {
    /// Written to Companion: Companion's answer acks it.
    Forwarded,
    /// Companion is offline: refused, never sent later.
    Offline,
    /// Companion already holds the key (a down) or still does (an up).
    Held,
    /// An up from a page that does not hold the key.
    NotHeld,
    /// The hub has no `[companion]`.
    NoDeck,
}

impl PressOutcome {
    /// The `reason` of its `deck_press` record (none: forwarded).
    pub fn reason(self) -> Option<&'static str> {
        match self {
            Self::Forwarded => None,
            Self::Offline => Some("offline"),
            Self::Held => Some("held"),
            Self::NotHeld => Some("not held"),
            Self::NoDeck => Some(NO_DECK),
        }
    }

    /// The page's ack of a press not forwarded (`ok`, the error), at once;
    /// none for a forwarded one.
    pub fn ack(self) -> Option<(bool, Option<&'static str>)> {
        match self {
            Self::Forwarded => None,
            Self::Held | Self::NotHeld => Some((true, None)),
            Self::Offline => Some((false, Some("offline"))),
            Self::NoDeck => Some((false, Some(NO_DECK))),
        }
    }
}

/// A key's log state: its last press, its last record, the changes since
/// that record and since the last summary.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct KeyLog {
    pressed_at: Option<f64>,
    logged_at: Option<f64>,
    unlogged: u32,
    summary: u32,
}

/// The keys a stopping hub has to account for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopKeys {
    /// Held now, with Companion reachable: released by the stop.
    pub release: Vec<u32>,
    /// Held when the link went down and never released: lost.
    pub lost: Vec<u32>,
}

/// The Stream Deck as the router knows it.
#[derive(Debug, Default)]
pub struct Deck {
    online: bool,
    keys: BTreeMap<u32, DeckKey>,
    logs: BTreeMap<u32, KeyLog>,
    viewers: BTreeSet<ClientId>,
    holders: Holders,
    stale: BTreeSet<u32>,
    heard: HashMap<ClientId, f64>,
    down_at: BTreeMap<u32, f64>,
    last_press: HashMap<(ClientId, u32), f64>,
    summary_at: f64,
}

impl Deck {
    /// Whether Companion's link is up.
    pub fn online(&self) -> bool {
        self.online
    }

    /// Something came from `client` at `now`.
    pub fn heard(&mut self, client: ClientId, now: f64) {
        self.heard.insert(client, now);
    }

    /// `client` opened (`on`: the whole cache, in key order, for it) or
    /// closed the tab.
    pub fn view(&mut self, client: ClientId, on: bool) -> Option<Vec<DeckKey>> {
        if on {
            self.viewers.insert(client);
            Some(self.keys.values().cloned().collect())
        } else {
            self.viewers.remove(&client);
            None
        }
    }

    /// The clients viewing the tab.
    pub fn viewers(&self) -> Vec<ClientId> {
        self.viewers.iter().copied().collect()
    }

    /// A press of `client`: not forwarded (and not held) while offline.
    pub fn press(&mut self, client: ClientId, key: u32, down: bool) -> PressOutcome {
        if !self.online {
            return PressOutcome::Offline;
        }
        let hold = if down {
            self.holders.down(key, client)
        } else {
            self.holders.up(key, client)
        };
        match hold {
            Hold::Forward => PressOutcome::Forwarded,
            Hold::Held => PressOutcome::Held,
            Hold::NotHeld => PressOutcome::NotHeld,
        }
    }

    /// How many clients hold `key`.
    pub fn holders_of(&self, key: u32) -> usize {
        self.holders.holders(key)
    }

    /// The time since `client`'s previous press of `key` (hub ms); this one
    /// remembered.
    pub fn gap(&mut self, client: ClientId, key: u32, hub_ms: f64) -> Option<f64> {
        self.last_press
            .insert((client, key), hub_ms)
            .map(|before| hub_ms - before)
    }

    /// A press of `key` went to Companion at `now`: a down starts its hold,
    /// an up ends it (the hold's length, when its down went this session).
    pub fn forwarded(&mut self, key: u32, down: bool, now: f64) -> Option<f64> {
        self.logs.entry(key).or_default().pressed_at = Some(now);
        if down {
            self.down_at.insert(key, now);
            None
        } else {
            self.down_at.remove(&key).map(|at| now - at)
        }
    }

    /// `client` is gone: it is forgotten; the keys only it held, to release.
    pub fn detach(&mut self, client: ClientId) -> Vec<u32> {
        self.viewers.remove(&client);
        self.heard.remove(&client);
        self.last_press.retain(|(c, _), _| *c != client);
        self.holders.drop_client(client)
    }

    /// The holding clients silent at `now`, each with the keys only it held
    /// (to release); they hold nothing afterwards.
    pub fn silent_clients(&mut self, now: f64) -> Vec<(ClientId, Vec<u32>)> {
        let quiet: Vec<ClientId> = self
            .holders
            .clients()
            .into_iter()
            .filter(|c| self.heard.get(c).is_none_or(|at| silent(now - at)))
            .collect();
        quiet
            .into_iter()
            .map(|c| (c, self.holders.drop_client(c)))
            .collect()
    }

    /// The link went down: offline; the held keys kept to release once it is
    /// back.
    pub fn link_down(&mut self) {
        self.online = false;
        let held = self.holders.clear();
        self.stale.extend(held);
        self.down_at.clear();
    }

    /// The link is up: online; the keys held when it went down, to release
    /// now.
    pub fn link_up(&mut self) -> Vec<u32> {
        self.online = true;
        std::mem::take(&mut self.stale).into_iter().collect()
    }

    /// The hub stops: the keys held now (to release) and the keys held when
    /// the link went down and never released (lost), each in key order.
    pub fn stop(&mut self) -> StopKeys {
        StopKeys {
            release: self.holders.clear(),
            lost: std::mem::take(&mut self.stale).into_iter().collect(),
        }
    }

    /// A key's new state at `now`: the key as the pages get it, and its
    /// `deck_key` record when one is due.
    pub fn apply(&mut self, update: &KeyUpdate, now: f64) -> (DeckKey, Option<Value>) {
        let cached = self.keys.entry(update.key).or_insert_with(|| DeckKey {
            key: update.key,
            img: None,
            color: None,
            pressed: false,
        });
        let was_pressed = cached.pressed;
        if let Some(img) = &update.img {
            cached.img = Some(img.clone());
        }
        if let Some(color) = &update.color {
            cached.color = Some(color.clone());
        }
        if let Some(pressed) = update.pressed {
            cached.pressed = pressed;
        }
        let key = cached.clone();
        let viewing = !self.viewers.is_empty();
        let log = self.logs.entry(update.key).or_default();
        log.unlogged += 1;
        log.summary += 1;
        let due = key_record(
            key.pressed != was_pressed,
            log.pressed_at.map(|at| now - at),
            viewing,
            log.logged_at.map(|at| now - at),
        );
        let record = due.then(|| {
            let fields = key_fields(&key, log.unlogged);
            log.logged_at = Some(now);
            log.unlogged = 0;
            fields
        });
        (key, record)
    }

    /// `KEYS-CLEAR`: every cached key black and released; them.
    pub fn clear(&mut self) -> Vec<DeckKey> {
        for key in self.keys.values_mut() {
            key.img = None;
            key.color = Some(BLACK.to_string());
            key.pressed = false;
        }
        self.keys.values().cloned().collect()
    }

    /// The `deck_keys` summary when due (every minute while the link is up):
    /// each key's changes since the last one.
    pub fn summary(&mut self, now: f64) -> Option<Value> {
        if !(self.online && summary_due(now - self.summary_at)) {
            return None;
        }
        self.summary_at = now;
        let changes: BTreeMap<String, u32> = self
            .logs
            .iter_mut()
            .filter(|(_, log)| log.summary > 0)
            .map(|(key, log)| (key.to_string(), std::mem::take(&mut log.summary)))
            .collect();
        Some(json!({"changes": changes}))
    }
}

/// A `deck_press` record's facts (spec §8).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PressRecord<'a> {
    pub client: ClientId,
    pub peer: Option<&'a str>,
    pub key: u32,
    pub down: bool,
    pub seq: u64,
    pub t: f64,
    pub hub_ms: f64,
    pub offset_ms: Option<f64>,
    pub gap_ms: Option<f64>,
    pub hold_ms: Option<f64>,
    pub why: Option<&'a str>,
    pub hub_hold_ms: Option<f64>,
    pub outcome: PressOutcome,
    pub holders: usize,
}

/// The `deck_press` fields: the page's press, its delay as a `set`'s, the
/// page's and the hub's hold, what became of it.
pub fn press_fields(p: &PressRecord<'_>) -> Value {
    json!({
        "client": p.client,
        "peer": p.peer,
        "key": p.key,
        "down": p.down,
        "seq": p.seq,
        "t": p.t,
        "hub_ms": p.hub_ms,
        "offset_ms": p.offset_ms,
        "delay_ms": one_way_delay(p.hub_ms, p.t, p.offset_ms),
        "gap_ms": p.gap_ms,
        "hold_ms": p.hold_ms,
        "why": p.why,
        "hub_hold_ms": p.hub_hold_ms,
        "forwarded": p.outcome == PressOutcome::Forwarded,
        "reason": p.outcome.reason(),
        "holders": p.holders,
    })
}

/// The `deck_ok` fields of Companion's answer (a hub-made release has no
/// client and no seq).
pub fn ok_fields(answer: &Answer) -> Value {
    let (client, seq) = answer
        .press
        .from
        .map_or((None, None), |(client, seq)| (Some(client), Some(seq)));
    json!({
        "client": client,
        "seq": seq,
        "key": answer.press.key,
        "down": answer.press.down,
        "ok": answer.ok,
        "error": answer.error,
        "rtt_ms": answer.rtt_ms,
    })
}

/// The `deck_release` fields of a release the hub made itself (`reason`:
/// detach, silent, reconnect, stop).
pub fn release_fields(
    client: Option<ClientId>,
    key: u32,
    reason: &str,
    hub_hold_ms: Option<f64>,
) -> Value {
    json!({"client": client, "key": key, "reason": reason, "hub_hold_ms": hub_hold_ms})
}

/// The `deck_link` fields: `state` up, down or refused.
pub fn link_fields(
    state: &str,
    companion: Option<&str>,
    api: Option<&str>,
    error: Option<&str>,
    down_ms: Option<f64>,
    attempts: Option<u64>,
) -> Value {
    json!({
        "state": state,
        "companion": companion,
        "api": api,
        "error": error,
        "down_ms": down_ms,
        "attempts": attempts,
    })
}

/// The `deck_key` fields: the image as its hash and size only.
pub fn key_fields(key: &DeckKey, changes: u32) -> Value {
    json!({
        "key": key.key,
        "pressed": key.pressed,
        "color": key.color,
        "img_hash": key.img.as_deref().map(img_hash),
        "img_bytes": key.img.as_ref().map(String::len),
        "changes": changes,
    })
}

/// The `deck_view` fields.
pub fn view_fields(client: ClientId, on: bool) -> Value {
    json!({"client": client, "on": on})
}

#[cfg(test)]
mod tests;
