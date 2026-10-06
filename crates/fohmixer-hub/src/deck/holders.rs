//! Who holds which Stream Deck key (#52, spec §5), pure: a key's set of
//! clients. Companion sees one down when the first client puts a finger on a
//! key and one up when the last lifts it, so two fingers or two tablets on
//! one key never press it twice nor release it early.

use std::collections::{BTreeMap, BTreeSet};

use crate::live::subs::ClientId;

/// What a press does to Companion's key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hold {
    /// Forward it: the first down, or the last up.
    Forward,
    /// Companion already holds the key (a down), or still does (an up).
    Held,
    /// An up from a client that does not hold the key.
    NotHeld,
}

/// Each held key's clients.
#[derive(Debug, Default)]
pub struct Holders {
    keys: BTreeMap<u32, BTreeSet<ClientId>>,
}

impl Holders {
    /// `client` puts a finger on `key`.
    pub fn down(&mut self, key: u32, client: ClientId) -> Hold {
        let set = self.keys.entry(key).or_default();
        let first = set.is_empty();
        set.insert(client);
        if first { Hold::Forward } else { Hold::Held }
    }

    /// `client` lifts its finger from `key`.
    pub fn up(&mut self, key: u32, client: ClientId) -> Hold {
        let Some(set) = self.keys.get_mut(&key) else {
            return Hold::NotHeld;
        };
        if !set.remove(&client) {
            return Hold::NotHeld;
        }
        if set.is_empty() {
            self.keys.remove(&key);
            Hold::Forward
        } else {
            Hold::Held
        }
    }

    /// `client` is gone: it leaves every key; the keys that emptied, to
    /// release, in key order.
    pub fn drop_client(&mut self, client: ClientId) -> Vec<u32> {
        let held: Vec<u32> = self
            .keys
            .iter()
            .filter(|(_, set)| set.contains(&client))
            .map(|(key, _)| *key)
            .collect();
        let mut released = Vec::new();
        for key in held {
            if self.up(key, client) == Hold::Forward {
                released.push(key);
            }
        }
        released
    }

    /// Every hold forgotten; the keys that were held, in key order.
    pub fn clear(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.keys).into_keys().collect()
    }

    /// How many clients hold `key`.
    pub fn holders(&self, key: u32) -> usize {
        self.keys.get(&key).map_or(0, BTreeSet::len)
    }

    /// Every client holding a key.
    pub fn clients(&self) -> BTreeSet<ClientId> {
        self.keys.values().flatten().copied().collect()
    }
}
