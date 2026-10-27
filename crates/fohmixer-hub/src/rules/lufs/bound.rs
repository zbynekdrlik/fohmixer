//! The listener's bound per key (#151, spec §4.4), pure: at most twice a
//! device's nominal rate is taken from one key ([`LUFS_PER_SECOND`],
//! [`METER_PER_SECOND`]), in a window of one second from the key's first
//! datagram in it; at most [`MAX_KEYS`] keys are kept (a new one past that
//! is [`Admit::Full`]). A key heard by nobody for [`FORGET_MS`] frees its
//! room ([`Bound::forget_before`]).
//!
//! A key is one device at one place: its kind, its Max id (per Live
//! process) and its four parents' names. A device moved or a track renamed
//! is a new key; two Lives' devices at the same place with the same id are
//! one key (the hub's confirming walk tells them apart).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use fohmixer_proto::lufs::wire::{DeviceKind, PARENTS, Reading};

/// The keys kept at most.
pub const MAX_KEYS: usize = 64;
/// A LUFS device's datagrams taken per second (it sends 5: its metro 200,
/// the owner's ruling of 27.10).
pub const LUFS_PER_SECOND: u32 = 10;
/// A meter device's datagrams taken per second (it sends 30).
pub const METER_PER_SECOND: u32 = 60;
/// A key's window.
pub const WINDOW: Duration = Duration::from_secs(1);
/// A dropping key is under the bound again after this long without a drop.
pub const QUIET: Duration = Duration::from_secs(10);
/// A key unheard this long is forgotten (its room freed).
pub const FORGET_MS: u64 = 10_000;

/// One device at one place.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    pub kind: DeviceKind,
    pub device: i32,
    pub parents: [String; PARENTS],
}

impl Key {
    /// The key of a reading.
    pub fn of(reading: &Reading) -> Self {
        Self {
            kind: reading.kind,
            device: reading.device,
            parents: reading.parents.clone(),
        }
    }
}

/// What the bound does with a datagram.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    /// Taken.
    Take,
    /// Over its key's rate: dropped.
    Drop,
    /// A new key with no room left: dropped.
    Full,
}

/// A key's change of dropping (logged once each way).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dropping {
    /// It started dropping.
    Started,
    /// It was under the bound for [`QUIET`].
    Ended,
}

/// The datagrams a kind's key may send in a window.
pub fn per_second(kind: DeviceKind) -> u32 {
    match kind {
        DeviceKind::Lufs => LUFS_PER_SECOND,
        DeviceKind::Meter => METER_PER_SECOND,
    }
}

/// The change between a key's dropping before and now, if any.
pub fn drop_warning(was_dropping: bool, dropping: bool) -> Option<Dropping> {
    match (was_dropping, dropping) {
        (false, true) => Some(Dropping::Started),
        (true, false) => Some(Dropping::Ended),
        _ => None,
    }
}

/// Whether a dropping key stops dropping at `now`: its last drop is
/// [`QUIET`] or longer ago.
pub fn quiet_since(last_drop: Instant, now: Instant) -> bool {
    now.saturating_duration_since(last_drop) >= QUIET
}

#[derive(Debug, Clone)]
struct Window {
    start: Instant,
    count: u32,
    heard: Instant,
    dropping: bool,
    last_drop: Instant,
}

/// The keys' windows.
#[derive(Debug, Default)]
pub struct Bound {
    keys: BTreeMap<Key, Window>,
}

impl Bound {
    pub fn new() -> Self {
        Self::default()
    }

    /// The keys kept.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether none is kept.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// One datagram of `key` at `now`: taken or dropped, and the key's
    /// change of dropping.
    pub fn admit(&mut self, key: &Key, now: Instant) -> (Admit, Option<Dropping>) {
        let Some(window) = self.keys.get_mut(key) else {
            if self.keys.len() >= MAX_KEYS {
                return (Admit::Full, None);
            }
            self.keys.insert(
                key.clone(),
                Window {
                    start: now,
                    count: 1,
                    heard: now,
                    dropping: false,
                    last_drop: now,
                },
            );
            return (Admit::Take, None);
        };
        window.heard = now;
        if now.saturating_duration_since(window.start) >= WINDOW {
            window.start = now;
            window.count = 0;
        }
        let was = window.dropping;
        let admit = if window.count >= per_second(key.kind) {
            window.dropping = true;
            window.last_drop = now;
            Admit::Drop
        } else {
            window.count += 1;
            if window.dropping && quiet_since(window.last_drop, now) {
                window.dropping = false;
            }
            Admit::Take
        };
        (admit, drop_warning(was, window.dropping))
    }

    /// Forgets every key last heard before `cutoff`.
    pub fn forget_before(&mut self, cutoff: Instant) {
        self.keys.retain(|_, window| window.heard >= cutoff);
    }
}

#[cfg(test)]
#[path = "bound_tests.rs"]
mod tests;
