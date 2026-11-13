//! A Live that is down, said in words (#178): the store's glue. The
//! decision is `behave::live_down::live_word` (not running / no set / on);
//! this keeps each instance's word in `LiveStore.live` and, while a Live has
//! no set or is not running, its wanted slots' errors quiet (`Slot::NoSet`:
//! they wait, never red). A Live is judged again only when its word can change: its report,
//! a slot of it changing class (never every value), the wanted set, a hello
//! and a close.

use std::collections::BTreeMap;

use leptos::prelude::*;

use super::LiveStore;
use crate::behave::live_down::{LiveWord, Seen, judges_set, live_word};
use crate::dom;
use crate::store::Slot;

impl LiveStore {
    /// The word of `instance` now (its errors arrive quiet while it has
    /// no set).
    pub(super) fn word_of(self, instance: &str) -> Option<LiveWord> {
        self.live
            .try_with_untracked(|all| all.get(instance).copied())
            .flatten()
    }

    /// The instance of a wanted key.
    pub(super) fn instance_of(self, key: &str) -> Option<String> {
        self.inner
            .try_with_value(|i| i.conn.instance_of(key))
            .flatten()
    }

    /// The wanted slots of `instance` whose target tells whether its set
    /// is there (`judges_set`).
    fn judging_slots(self, instance: &str) -> Vec<ArcRwSignal<Slot>> {
        self.inner
            .try_with_value(|i| {
                i.conn
                    .keys_of(Some(instance))
                    .into_iter()
                    .filter(|k| i.conn.target_of(k).is_some_and(|t| judges_set(&t)))
                    .filter_map(|k| i.slots.get(&k).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every Live judged again (the wanted set, a hello, a close).
    pub(super) fn judge_all(self) {
        let names: Vec<String> = self
            .instances
            .try_with_untracked(|all| all.keys().cloned().collect())
            .unwrap_or_default();
        for name in &names {
            self.judge(name);
        }
    }

    /// One Live judged again: its word, and its errors quiet while it has
    /// no set or is not running (red again once it is on, #178). During an outage (no word) the
    /// slots stay as they are.
    pub(super) fn judge(self, instance: &str) {
        let connected = self.connected.try_get_untracked().unwrap_or(false);
        let (online, script_missing) = self
            .instances
            .try_with_untracked(|all| {
                all.get(instance)
                    .map(|v| (Some(v.online), v.script_missing))
            })
            .flatten()
            .unwrap_or((None, false));
        let signals = self.slots_of(Some(instance));
        let seen = Seen::of(
            self.judging_slots(instance)
                .iter()
                .filter_map(|signal| signal.try_with_untracked(Slot::class)),
        );
        let word = live_word(connected, online, script_missing, seen);
        if let Some(word) = word {
            let quiet = word.quiets();
            for signal in signals {
                let turns = signal
                    .try_with_untracked(|slot| slot.clone().quieted(quiet) != *slot)
                    .unwrap_or(false);
                if turns {
                    let _ = signal.try_update(|slot| {
                        *slot = std::mem::replace(slot, Slot::Pending).quieted(quiet);
                    });
                }
            }
        }
        let old = self
            .live
            .try_with_untracked(|all| all.get(instance).copied())
            .flatten();
        if old == word {
            return;
        }
        dom::log(&format!(
            "Live {instance}: {}",
            word.map_or("unknown", LiveWord::word)
        ));
        let _ = self
            .live
            .try_update(|all: &mut BTreeMap<String, LiveWord>| {
                match word {
                    Some(word) => all.insert(instance.to_string(), word),
                    None => all.remove(instance),
                };
            });
    }
}
