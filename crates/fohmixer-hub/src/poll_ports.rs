//! The poll ports the hub's polled devices name, and the decision that one
//! is shared (#264, the design comment 7228766032). Pure: no I/O.
//!
//! Each polled device (the LUFS devices' and the meters' keys in
//! `lufs_udp`, the Listen devices' ids in `listen`) draws its poll port
//! once, at load, from one range, and about 50 devices draw. Max's
//! `udpreceive` binds with `SO_REUSEADDR`, so a second device that draws a
//! taken port binds without an error; on the FOH PC Windows then gives the
//! port's datagrams to its FIRST binder only (the PC's measurement of
//! 27.11), so the device that loaded later announced once, at its load,
//! and hears nothing more.
//!
//! **The decision:** a port is shared when a key that named it EARLIER (the
//! first binder) is heard after a newer key named it, while that newer key
//! still counts. The older one hears, the newer one is deaf: the hub moves
//! the older one ([`Shared::hearing`]), at most once a port every
//! [`MOVE_EVERY_MS`] ([`move_due`]), and polls the port on for the move's
//! keys for that window ([`moving`]), so the deaf one, which hears once the
//! other left, is learned again even after its own window ended. A device
//! moved or renamed in Live answers with a new key on its old port and its
//! old key is never heard again: no share. Two keys that named a port at
//! the same time give no order: no share (inside one Live both devices
//! share Max's socket and both hear: harmless).

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

pub use fohmixer_proto::lufs::wire::MOVE_EVERY_MS;

/// A key's port, since when it names it, and when it was last heard.
#[derive(Debug, Clone, Copy)]
struct Held {
    port: u16,
    since: Instant,
    heard: Instant,
}

/// A move sent: when, and the two keys of its port.
#[derive(Debug, Clone)]
struct Moved<K> {
    at: Instant,
    keys: [K; 2],
}

/// A port two keys name: the key that still answers on it (to move) and
/// the one that stays, deaf until the other leaves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shared<K> {
    pub port: u16,
    pub hearing: K,
    pub deaf: K,
}

/// What one heard key tells: the port it named before (it moved), and a
/// port found shared (a move is due).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heard<K> {
    pub from: Option<u16>,
    pub shared: Option<Shared<K>>,
}

/// The keys' poll ports and the moves of the last [`MOVE_EVERY_MS`].
#[derive(Debug)]
pub struct PollPorts<K> {
    keys: BTreeMap<K, Held>,
    moves: BTreeMap<u16, Moved<K>>,
}

impl<K> Default for PollPorts<K> {
    fn default() -> Self {
        Self {
            keys: BTreeMap::new(),
            moves: BTreeMap::new(),
        }
    }
}

/// The move window.
fn window() -> Duration {
    Duration::from_millis(MOVE_EVERY_MS)
}

/// Whether a port last moved at `last` (none: never) may be moved at `now`.
pub fn move_due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|at| {
        now.checked_duration_since(at)
            .is_some_and(|d| d >= window())
    })
}

/// Whether a move sent at `at` still holds its port polled at `now`.
pub fn moving(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) < window()
}

impl<K: Ord + Clone> PollPorts<K> {
    /// `key` heard naming `port` at `now`; `kept(other, heard)` says whether
    /// another key last heard at `heard` still counts at `now`.
    pub fn heard(
        &mut self,
        key: &K,
        port: u16,
        now: Instant,
        kept: impl Fn(&K, Instant) -> bool,
    ) -> Heard<K> {
        let before = self.keys.get(key).copied();
        let from = before.map(|h| h.port).filter(|old| *old != port);
        let since = match before {
            Some(h) if h.port == port => h.since,
            _ => now,
        };
        self.keys.insert(
            key.clone(),
            Held {
                port,
                since,
                heard: now,
            },
        );
        let newer = self.keys.iter().find(|(other, held)| {
            *other != key && held.port == port && held.since > since && kept(other, held.heard)
        });
        let shared = newer
            .map(|(deaf, _)| deaf.clone())
            .filter(|_| move_due(self.moves.get(&port).map(|m| m.at), now))
            .map(|deaf| {
                self.moves.insert(
                    port,
                    Moved {
                        at: now,
                        keys: [key.clone(), deaf.clone()],
                    },
                );
                Shared {
                    port,
                    hearing: key.clone(),
                    deaf,
                }
            });
        Heard { from, shared }
    }

    /// Drops the keys no longer kept at `now` and the moves past their
    /// window.
    pub fn forget(&mut self, now: Instant, kept: impl Fn(&K, Instant) -> bool) {
        self.keys.retain(|key, held| kept(key, held.heard));
        self.moves.retain(|_, moved| moving(moved.at, now));
    }

    /// The ports to poll at `now` for the keys `pick` takes, each once: the
    /// kept keys' ports and the ports moved within the window whose keys it
    /// takes.
    pub fn due(&self, now: Instant, pick: impl Fn(&K) -> bool) -> BTreeSet<u16> {
        let kept = self
            .keys
            .iter()
            .filter(|(key, _)| pick(key))
            .map(|(_, held)| held.port);
        let moved = self
            .moves
            .iter()
            .filter(|(_, moved)| moving(moved.at, now) && moved.keys.iter().any(&pick))
            .map(|(port, _)| *port);
        kept.chain(moved).collect()
    }

    /// The kept keys' ports, each once.
    pub fn ports(&self) -> usize {
        let ports: BTreeSet<u16> = self.keys.values().map(|held| held.port).collect();
        ports.len()
    }

    /// The keys kept.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// No key kept.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

#[cfg(test)]
mod tests;
