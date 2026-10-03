//! The hub connection and Live's state for the surface (S4 design note
//! §4): the `LiveStore` context owns the client WebSocket (reconnecting in
//! under 2 s, never giving up), the handshake, the subscriptions (each key
//! once, only the pages on screen), one signal per subscription, the
//! instances' states and the hub values.
//!
//! I8: every subscription starts `Pending` and goes back to `Pending` when
//! its instance goes offline, REFRESH ALL resubscribes, a page switch
//! subscribes or unsubscribes it while the hub is connected, or a hello
//! finds no page wanting it; a value for a key no page wants is dropped
//! (one the hub sent before it read the unsub); a control accepts input
//! only while its slots hold Live's values. A lost hub connection keeps
//! each wanted slot's value, marked `Stale` (#43, L2): the control still
//! shows it and takes touches, and the next value from the hub makes it
//! fresh. I5: a binding that does not resolve is an `Error` slot; every
//! control bound to one is shown red (`data-binding="unresolved"`) and
//! disabled (`Readiness`).
//!
//! The pure parts are unit-tested natively: here `Slot`, `Readiness`,
//! `Wanted`, `Badge`, `slot_failure`, `range_from` and `next_range`, in `conn` the
//! connection's decisions (reconnect, hello, the watchdog and its pings, the
//! layout and instance changes), in `intent` the controls' writes waiting
//! for their ack (#43). `LiveStore` (`live`) is the browser glue that
//! carries them out.

mod conn;
pub mod intent;
mod live;

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::binding::SubSpec;

pub use live::LiveStore;

/// What the surface knows of one subscription.
#[derive(Debug, Clone, PartialEq)]
pub enum Slot {
    /// No value from Live yet: the control is disabled (I8).
    Pending,
    /// Live's value and display string, received at `at` (page clock, ms).
    Value {
        value: Value,
        display: Option<String>,
        at: f64,
    },
    /// Live's last value from a hub connection that was lost (#43, §4.2):
    /// shown and touchable (L2), but not fresh (a meter or the status light
    /// does not take it).
    Stale {
        value: Value,
        display: Option<String>,
        at: f64,
    },
    /// The binding does not resolve (a missing or ambiguous name): red and
    /// disabled (I5).
    Error(String),
}

impl Slot {
    /// The slot a `subbed` or `values` item leaves: an error, a value, or
    /// `None` when it carries neither (the hub waits for Live).
    pub fn from_item(
        value: Option<Value>,
        display: Option<String>,
        error: Option<String>,
        at: f64,
    ) -> Option<Self> {
        match (error, value) {
            (Some(error), _) => Some(Self::Error(error)),
            (None, Some(value)) => Some(Self::Value { value, display, at }),
            (None, None) => None,
        }
    }

    /// The slot after the hub connection was lost: a value is kept, marked
    /// stale; anything else stays as it is.
    pub fn into_stale(self) -> Self {
        match self {
            Self::Value { value, display, at } => Self::Stale { value, display, at },
            other => other,
        }
    }

    /// The slot after a page switch (or a reconnect) changed whether its key
    /// is subscribed (#43). With the hub `connected` it is `Pending`: a key
    /// subscribed now waits for its fresh value (I8), and one no longer
    /// subscribed has nothing to keep it current, so it never comes back
    /// later with an old value. During an outage a known value is kept,
    /// stale (L2): a page left and re-entered in the same outage still takes
    /// touches.
    pub fn rewanted(self, connected: bool) -> Self {
        if connected {
            Self::Pending
        } else {
            self.into_stale()
        }
    }

    /// Whether Live's value is here, fresh or stale (the control may take
    /// input, L2).
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Value { .. } | Self::Stale { .. })
    }

    /// Whether Live's value is here and fresh (from the current connection).
    pub fn is_fresh(&self) -> bool {
        matches!(self, Self::Value { .. })
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }

    pub fn value(&self) -> Option<&Value> {
        match self {
            Self::Value { value, .. } | Self::Stale { value, .. } => Some(value),
            _ => None,
        }
    }

    /// Live's value as a number.
    pub fn number(&self) -> Option<f64> {
        self.value().and_then(Value::as_f64)
    }

    /// Live's value as a number, only while it is fresh (a meter never
    /// freezes at a stale level).
    pub fn fresh_number(&self) -> Option<f64> {
        match self {
            Self::Value { value, .. } => value.as_f64(),
            _ => None,
        }
    }

    /// Live's value as a flag (`mute`, `solo`).
    pub fn flag(&self) -> Option<bool> {
        self.value().and_then(Value::as_bool)
    }

    /// Live's display string.
    pub fn display(&self) -> Option<&str> {
        match self {
            Self::Value { display, .. } | Self::Stale { display, .. } => display.as_deref(),
            _ => None,
        }
    }

    /// When the value arrived.
    pub fn at(&self) -> Option<f64> {
        match self {
            Self::Value { at, .. } | Self::Stale { at, .. } => Some(*at),
            _ => None,
        }
    }
}

/// Whether a control's bindings let it take input (I5, I8). Ordered: a
/// control is as far from ready as its worst binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Readiness {
    /// Every slot holds Live's value.
    Ready,
    /// A slot waits for Live's value: disabled.
    Waiting,
    /// A binding does not resolve: red and disabled.
    Unresolved,
}

impl Readiness {
    /// One slot's readiness.
    pub fn of_slot(slot: &Slot) -> Self {
        match slot {
            Slot::Value { .. } | Slot::Stale { .. } => Self::Ready,
            Slot::Pending => Self::Waiting,
            Slot::Error(_) => Self::Unresolved,
        }
    }

    /// A control's readiness from its bindings' (the worst one counts; an
    /// unresolved binding outranks a missing value).
    pub fn all(each: impl IntoIterator<Item = Self>) -> Self {
        each.into_iter().max().unwrap_or(Self::Ready)
    }

    /// The control's `data-binding`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Waiting => "waiting",
            Self::Unresolved => "unresolved",
        }
    }

    /// The control's `aria-disabled`.
    pub fn disabled(self) -> &'static str {
        match self {
            Self::Ready => "false",
            Self::Waiting | Self::Unresolved => "true",
        }
    }
}

/// What a new wanted set changes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Change {
    /// Keys to unsubscribe.
    pub removed: Vec<String>,
    /// Subscriptions to make.
    pub added: Vec<SubSpec>,
}

/// The subscriptions the surface wants: each key once.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Wanted {
    specs: BTreeMap<String, SubSpec>,
}

impl Wanted {
    /// Replaces the wanted set with `specs`: what to unsubscribe and
    /// subscribe (both in key order; duplicates count once).
    pub fn replace(&mut self, specs: Vec<SubSpec>) -> Change {
        let new: BTreeMap<String, SubSpec> = specs.into_iter().map(|s| (s.key(), s)).collect();
        let removed = self
            .specs
            .keys()
            .filter(|k| !new.contains_key(*k))
            .cloned()
            .collect();
        let added = new
            .iter()
            .filter(|(k, _)| !self.specs.contains_key(*k))
            .map(|(_, s)| s.clone())
            .collect();
        self.specs = new;
        Change { removed, added }
    }

    /// Every wanted subscription, in key order.
    pub fn specs(&self) -> Vec<SubSpec> {
        self.specs.values().cloned().collect()
    }

    /// Whether `key` is wanted (#43: a value for a key no page wants is not
    /// kept: nothing would keep it current).
    pub fn contains(&self, key: &str) -> bool {
        self.specs.contains_key(key)
    }

    /// The wanted keys of `instance` (every key for `None`).
    pub fn keys_of(&self, instance: Option<&str>) -> Vec<String> {
        self.specs
            .iter()
            .filter(|(_, s)| instance.is_none_or(|i| s.instance == i))
            .map(|(k, _)| k.clone())
            .collect()
    }

    pub fn len(&self) -> usize {
        self.specs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }
}

/// One instance as the hub reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstanceView {
    pub online: bool,
    pub busy: bool,
    pub set_name: String,
}

/// A connection badge's state (spec §2.5 robustness).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Badge {
    Online,
    Busy,
    Offline,
}

impl Badge {
    pub fn of(view: &InstanceView) -> Self {
        match (view.online, view.busy) {
            (false, _) => Self::Offline,
            (true, true) => Self::Busy,
            (true, false) => Self::Online,
        }
    }

    /// The badge's `data-state` and CSS class.
    pub fn name(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Busy => "busy",
            Self::Offline => "offline",
        }
    }
}

/// Why a one-command `cmd` did not do its work, if it did not: the
/// script's error slot or the hub's refusal (spec I6: never retried).
pub fn slot_failure(outcome: &Result<Vec<Value>, String>) -> Option<String> {
    match outcome {
        Ok(slots) => match slots.first() {
            Some(slot) if slot.get("ok") == Some(&json!(true)) => None,
            Some(slot) => Some(
                slot.get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("failed")
                    .to_string(),
            ),
            None => Some("no result".to_string()),
        },
        Err(e) => Some(e.clone()),
    }
}

/// A parameter's `min` and `max` from the result of a
/// `get_prop min` + `get_prop max` batch; `None` unless both are numbers
/// and the range is not empty.
pub fn range_from(outcome: &Result<Vec<Value>, String>) -> Option<(f64, f64)> {
    let slots = outcome.as_ref().ok()?;
    let number = |i: usize| -> Option<f64> {
        let slot = slots.get(i)?;
        if slot.get("ok") == Some(&json!(true)) {
            slot.get("data").and_then(Value::as_f64)
        } else {
            None
        }
    };
    let (min, max) = (number(0)?, number(1)?);
    (max > min).then_some((min, max))
}

/// A parameter's range after a read: the new one, or, when the read got no
/// answer (the hub or a stalled Live did not reply), the one before. A read
/// Live answered without a range (the parameter is gone) clears it.
pub fn next_range(
    before: Option<(f64, f64)>,
    outcome: &Result<Vec<Value>, String>,
) -> Option<(f64, f64)> {
    match outcome {
        Ok(_) => range_from(outcome),
        Err(_) => before,
    }
}

/// The write key (`instance|target|prop`, #43) of a subscription's key
/// (`instance|target|prop|display`): the protocol's own inverse of its key.
pub use fohmixer_proto::client::write_key_of as write_key;

/// The key under which the engineer's token is stored.
pub const TOKEN_KEY: &str = "fohmixer_token";

/// What hears a command's result slots (or why there are none).
pub type ResultFn = Box<dyn FnOnce(Result<Vec<Value>, String>)>;

#[cfg(test)]
mod tests;
