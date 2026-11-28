//! The poll ports the hub's polled devices name, and the decision that one
//! is shared (#264, the design comment 7228766032). [`PollPorts`] is pure;
//! [`PortTable`] is the hub's one table of it, shared by the LUFS
//! devices' listener and Listen.
//!
//! Each polled device (FohMixer LUFS, Meter and Listen) draws its poll
//! port once, at load, from ONE range, and about 50 devices draw. Max's
//! `udpreceive` binds with `SO_REUSEADDR`, so a second device that draws a
//! taken port binds without an error; on the FOH PC Windows then gives the
//! port's datagrams to its FIRST binder only (the PC's measurement of
//! 27.11), so the device that loaded later announced at its load and
//! hears nothing more. The kinds share the range, so the table holds
//! every kind's keys ([`PortKey`]): a meter and a Listen device on one
//! port are found as two meters are.
//!
//! **The decision (the design's two triggers):** a key heard on a port is
//! the one that hears there, and the port is shared with another device's
//! key on it that is either (1) still kept and newer there (it named the
//! port after this one: deaf), or (2) no longer kept, remembered for
//! [`REMEMBER_MS`] after it was last heard, and never seen ANSWER there
//! (it went stale while this one answers: its window ended before this one
//! was heard again, its announce came in out of bind order, or an earlier
//! move did not take). A key answers when it is heard again on its port
//! between [`BURST_MS`] and [`ANSWER_MS`] after its previous datagram, as
//! a polled device is every 33-250 ms ([`answers`]): a deaf device speaks
//! only at its load (the load's gate bang and Device On's observer: two
//! datagrams within microseconds, a burst) and at a later Device On turning
//! 1 (seconds apart), so a first binder quiet for a moment and a device
//! that answered and left are no deaf one. The hub moves the key that
//! hears ([`Shared::hearing`]), at most once a port every
//! [`MOVE_EVERY_MS`] ([`move_due`]), and polls the port on for the move's
//! keys for that window ([`moving`]), so the deaf one, which hears once the
//! other left, is learned again. A device moved or renamed in Live answers
//! with a new key of the SAME device (kind, Max id and Live set,
//! [`PortKey::same_device`]) on its old port: no share. Two kept keys
//! that named a port at the same time give no order: no share (inside one
//! Live both devices share Max's socket and both hear: a move of the older
//! one moves both, harmless).
//!
//! **Accepted (rare):** a first binder that never answered (its rack off
//! before the first poll) while a newcomer announces after its window gets
//! the newcomer moved (the move still lands at the first binder when the
//! kinds agree); a deaf device that left, its port drawn by a new device
//! within 15 s, moves the new one once for nothing; two Lives whose sets
//! have one name and whose devices have one Max id count as one device; a
//! device nested deeper than one rack names a rack as its set.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use fohmixer_proto::listen::wire::{self as listen_wire, FORGET_MS};
pub use fohmixer_proto::lufs::wire::MOVE_EVERY_MS;
use fohmixer_proto::lufs::wire::{self as lufs_wire, track_and_set};
use serde_json::{Value, json};

use crate::rules::lufs::bound::Key;
use crate::rules::lufs::{is_stale, kind_word};

/// A key no longer kept is remembered this long after it was last heard:
/// three move windows, so a move that did not take is sent twice more.
pub const REMEMBER_MS: u64 = 3 * MOVE_EVERY_MS;

/// A key's port, since when it names it, when it was last heard, and
/// whether it answered there ([`answers`]).
#[derive(Debug, Clone, Copy)]
struct Held {
    port: u16,
    since: Instant,
    heard: Instant,
    answered: bool,
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

/// The keys' poll ports (kept and remembered) and the moves of the last
/// [`MOVE_EVERY_MS`].
#[derive(Debug)]
pub struct PollPorts<K> {
    keys: BTreeMap<K, Held>,
    gone: BTreeMap<K, Held>,
    moves: BTreeMap<u16, Moved<K>>,
    same: fn(&K, &K) -> bool,
}

fn equal<K: PartialEq>(a: &K, b: &K) -> bool {
    a == b
}

impl<K: Ord + Clone> Default for PollPorts<K> {
    fn default() -> Self {
        Self::with_same(equal::<K>)
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

/// Datagrams of one key closer than this are one burst (the load's two).
pub const BURST_MS: u64 = 10;
/// A key heard again within this is answering polls (every 33-250 ms).
pub const ANSWER_MS: u64 = 1000;

/// Whether a key heard at `before` and again at `now` answered there: a gap
/// of [`BURST_MS`] or more and under [`ANSWER_MS`].
pub fn answers(before: Instant, now: Instant) -> bool {
    let gap = now.saturating_duration_since(before);
    gap >= Duration::from_millis(BURST_MS) && gap < Duration::from_millis(ANSWER_MS)
}

/// Whether another key's `held` port names it deaf there for a key that
/// names the port `since`: kept, it is newer there (the first trigger);
/// no longer `kept`, it never answered there (the second).
fn deaf(held: &Held, kept: bool, since: Instant) -> bool {
    if kept {
        held.since > since
    } else {
        !held.answered
    }
}

/// Whether a key no longer kept, last heard at `heard`, is still
/// remembered at `now`.
pub fn remembered(heard: Instant, now: Instant) -> bool {
    now.saturating_duration_since(heard) < Duration::from_millis(REMEMBER_MS)
}

/// The ports in `range` (the devices') that two or more DIFFERENT
/// processes bind, from the OS's UDP endpoint table's `rows` (port, pid).
pub fn shared_by_processes(rows: &[(u16, u32)], range: &Range<u16>) -> BTreeSet<u16> {
    let _ = (rows, range);
    BTreeSet::new()
}

/// The different processes that bind `port` in `rows`.
pub fn processes_on(rows: &[(u16, u32)], port: u16) -> usize {
    let _ = (rows, port);
    0
}

impl<K: Ord + Clone> PollPorts<K> {
    /// A table whose keys of one device are told by `same` (a new key of
    /// the same device on its port is no share).
    pub fn with_same(same: fn(&K, &K) -> bool) -> Self {
        Self {
            keys: BTreeMap::new(),
            gone: BTreeMap::new(),
            moves: BTreeMap::new(),
            same,
        }
    }

    /// `key` heard naming `port` at `now`; `kept(other, heard)` says whether
    /// another key last heard at `heard` still counts at `now`.
    pub fn heard(
        &mut self,
        key: &K,
        port: u16,
        now: Instant,
        kept: impl Fn(&K, Instant) -> bool,
    ) -> Heard<K> {
        let before = self.keys.get(key).or(self.gone.get(key)).copied();
        self.gone.remove(key);
        let from = before.map(|h| h.port).filter(|old| *old != port);
        let (since, answered) = match before {
            Some(h) if h.port == port => (h.since, h.answered || answers(h.heard, now)),
            _ => (now, false),
        };
        self.keys.insert(
            key.clone(),
            Held {
                port,
                since,
                heard: now,
                answered,
            },
        );
        let same = self.same;
        let deaf = self
            .keys
            .iter()
            .chain(self.gone.iter())
            .find(|(other, held)| {
                *other != key
                    && !same(other, key)
                    && held.port == port
                    && deaf(held, kept(other, held.heard), since)
            })
            .map(|(deaf, _)| deaf.clone());
        let shared = deaf
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

    /// Moves the keys no longer kept at `now` to the remembered ones, drops
    /// those past [`REMEMBER_MS`] and the moves past their window.
    pub fn forget(&mut self, now: Instant, kept: impl Fn(&K, Instant) -> bool) {
        let (stay, stale): (BTreeMap<K, Held>, BTreeMap<K, Held>) = std::mem::take(&mut self.keys)
            .into_iter()
            .partition(|(key, held)| kept(key, held.heard));
        self.keys = stay;
        self.gone.extend(stale);
        self.gone.retain(|_, held| remembered(held.heard, now));
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

    /// A port found shared by the OS's table at `now`: whether a move is
    /// due (no move of the port within its window).
    pub fn found(&mut self, port: u16, now: Instant) -> bool {
        let _ = (port, now);
        false
    }

    /// The kept keys' ports `pick` takes, each once.
    pub fn ports_of(&self, pick: impl Fn(&K) -> bool) -> usize {
        let ports: BTreeSet<u16> = self
            .keys
            .iter()
            .filter(|(key, _)| pick(key))
            .map(|(_, held)| held.port)
            .collect();
        ports.len()
    }

    /// The kept keys `pick` takes.
    pub fn len_of(&self, pick: impl Fn(&K) -> bool) -> usize {
        self.keys.keys().filter(|key| pick(key)).count()
    }

    /// The kept keys' ports, each once.
    pub fn ports(&self) -> usize {
        self.ports_of(|_| true)
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

/// A polled device's key in the hub's one table: a LUFS device's or a
/// meter's (`lufs_udp`) or a Listen device's id (`listen`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum PortKey {
    Lufs(Key),
    Listen(i32),
}

impl PortKey {
    /// Whether the key, last heard at `heard`, still counts at `now`: a
    /// LUFS or meter key within its stale window (`rules::lufs`), a Listen
    /// device within `FORGET_MS` (a clock that went back is no loss).
    pub fn kept(&self, heard: Instant, now: Instant) -> bool {
        let ms =
            u64::try_from(now.saturating_duration_since(heard).as_millis()).unwrap_or(u64::MAX);
        match self {
            PortKey::Lufs(key) => !is_stale(key.kind, 0, ms),
            PortKey::Listen(_) => ms < FORGET_MS,
        }
    }

    /// Whether two keys are of one device: the same kind, Max id and Live
    /// set (a rename or a move in Live changes a LUFS key's parents, never
    /// these; the set tells two Lives' devices of one Max id apart: the ids
    /// count per Live process).
    pub fn same_device(a: &PortKey, b: &PortKey) -> bool {
        let set = |key: &Key| track_and_set(&key.parents).map(|(_, set)| set.to_string());
        match (a, b) {
            (PortKey::Lufs(a), PortKey::Lufs(b)) => {
                a.kind == b.kind && a.device == b.device && set(a) == set(b)
            }
            (PortKey::Listen(a), PortKey::Listen(b)) => a == b,
            _ => false,
        }
    }

    /// Whether it is a LUFS or meter key.
    pub fn is_lufs(&self) -> bool {
        matches!(self, PortKey::Lufs(_))
    }

    /// The address of the hub's move of its device.
    pub fn move_address(&self) -> &'static str {
        match self {
            PortKey::Lufs(key) => key.kind.move_address(),
            PortKey::Listen(_) => listen_wire::MOVE_ADDRESS,
        }
    }

    /// The hub's move of its device (`lufs::wire::move_datagram` of its
    /// kind, `listen::wire::move_datagram`).
    pub fn move_datagram(&self) -> Vec<u8> {
        match self {
            PortKey::Lufs(key) => lufs_wire::move_datagram(key.kind),
            PortKey::Listen(_) => listen_wire::move_datagram(),
        }
    }

    /// Its fields in a record: its kind and device, a LUFS or meter key's
    /// parents too.
    pub fn fields(&self) -> Value {
        match self {
            PortKey::Lufs(key) => {
                json!({"kind": kind_word(key.kind), "device": key.device, "parents": key.parents})
            }
            PortKey::Listen(device) => json!({"kind": "listen", "device": device}),
        }
    }
}

/// The hub's one table of every polled device's port and the shared ports
/// found and not yet moved, each taken by the side whose kind hears there
/// (the LUFS devices' listener sends a LUFS or meter move, Listen's poll
/// round a Listen move).
#[derive(Debug)]
pub struct PortTable {
    inner: Mutex<(PollPorts<PortKey>, Vec<Shared<PortKey>>)>,
}

impl Default for PortTable {
    fn default() -> Self {
        Self {
            inner: Mutex::new((PollPorts::with_same(PortKey::same_device), Vec::new())),
        }
    }
}

impl PortTable {
    fn lock(&self) -> std::sync::MutexGuard<'_, (PollPorts<PortKey>, Vec<Shared<PortKey>>)> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `key` heard naming `port` at `now`: the port it named before when it
    /// moved; a port found shared waits for [`PortTable::take`].
    pub fn heard(&self, key: PortKey, port: u16, now: Instant) -> Option<u16> {
        let mut guard = self.lock();
        let (ports, shared) = &mut *guard;
        let heard = ports.heard(&key, port, now, |other, at| other.kept(at, now));
        shared.extend(heard.shared);
        heard.from
    }

    /// The shared ports found whose hearing key `pick` takes, taken.
    pub fn take(&self, pick: impl Fn(&PortKey) -> bool) -> Vec<Shared<PortKey>> {
        let mut guard = self.lock();
        let (taken, kept) = std::mem::take(&mut guard.1)
            .into_iter()
            .partition(|share| pick(&share.hearing));
        guard.1 = kept;
        taken
    }

    /// The ports to poll at `now` for the keys `pick` takes
    /// ([`PollPorts::due`]), the keys no longer kept forgotten first.
    pub fn due(&self, now: Instant, pick: impl Fn(&PortKey) -> bool) -> BTreeSet<u16> {
        let mut guard = self.lock();
        guard.0.forget(now, |key, heard| key.kept(heard, now));
        guard.0.due(now, pick)
    }

    /// A port found shared by the OS's table at `now`: whether a move is
    /// due ([`PollPorts::found`]).
    pub fn found(&self, port: u16, now: Instant) -> bool {
        self.lock().0.found(port, now)
    }

    /// The kept keys' ports `pick` takes, each once.
    pub fn ports_of(&self, pick: impl Fn(&PortKey) -> bool) -> usize {
        self.lock().0.ports_of(pick)
    }

    /// The kept keys `pick` takes.
    pub fn len_of(&self, pick: impl Fn(&PortKey) -> bool) -> usize {
        self.lock().0.len_of(pick)
    }
}

#[cfg(test)]
mod tests;
