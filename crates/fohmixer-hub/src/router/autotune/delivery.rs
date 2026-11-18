//! AUTOTUNE's deliveries (#104, #140; split from `router/autotune.rs`,
//! #249): each of its subscriptions routed by hub key to its own: a
//! target's macro value (here), a key device's Key or Scale and a set's
//! playhead or tempo (`keys.rs`); the pages told after each.

use serde_json::Value;

use super::super::{Router, transport_heard};
use super::keys::Route;
use crate::rules::autotune::switch::still_flying;

impl Router {
    /// A delivery of one of AUTOTUNE's subscriptions (its hub key `sub`): a
    /// target's macro, its key device's Key or Scale, or a set's playhead or
    /// tempo (#140 PR 2), each to its own; anything else does nothing.
    pub(in crate::router) fn autotune_delivery(
        &mut self,
        sub: &str,
        value: Option<Value>,
        error: Option<String>,
    ) {
        match self.autotune.route(sub) {
            Some(Route::Macro(key)) => self.autotune_macro_value(&key, value, error),
            Some(Route::Key(key, part)) => self.autotune_key_value(&key, part, value, error),
            Some(Route::Position(instance, part)) => {
                self.autotune_position_value(&instance, part, value, error);
            }
            None => return,
        }
        self.autotune_tell();
    }

    /// A delivery of a target's macro value: a value (the echo of the
    /// hub's own write on its way, or judged for another writer: #140 PR
    /// 3b), or an error (the macro's path no longer resolves: its value
    /// dropped and the target walked again).
    fn autotune_macro_value(&mut self, key: &str, value: Option<Value>, error: Option<String>) {
        match transport_heard(value, error) {
            Some(Ok(value)) => {
                let Some(target) = self.autotune.targets.get_mut(key) else {
                    return;
                };
                let was = target.value;
                target.value = value.as_f64();
                // The writes on their way before it, then those it did not
                // land (PR #147 review I1).
                let flying: Vec<bool> = target.writing.iter().map(|f| f.on).collect();
                target.writing = still_flying(&target.writing, target.shows());
                self.autotune_heard(key, was, &flying);
            }
            Some(Err(error)) => {
                tracing::warn!(key = %key, error = ?error, "AUTOTUNE lost a rack's macro: it looks again");
                if let Some(target) = self.autotune.targets.get_mut(key) {
                    target.value = None;
                    target.writing.clear();
                }
                self.autotune_walk(key);
            }
            None => {}
        }
    }
}
