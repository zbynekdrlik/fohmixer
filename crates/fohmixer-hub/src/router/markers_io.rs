//! The router's part of the marker keeper (#68, `router/markers.rs`): its
//! subscriptions, its reads and their answers, and each new set of markers
//! handed to the layout store, whose new composition goes out as a layout
//! change.

use std::sync::Arc;

use fohmixer_proto::markers::Found;
use serde_json::Value;

use super::{MARKERS_CLIENT, Router, RouterMsg, cached_value, layout_msg, markers};
use crate::layout::LayoutStore;
use crate::live::subs::Cached;

impl Router {
    /// Follows the instances' Tuner markers and serves them through `store`.
    pub fn with_layout(mut self, store: Arc<LayoutStore>) -> Self {
        self.layout_store = Some(store);
        let names: Vec<String> = self.live.keys().cloned().collect();
        let actions = self.markers.set_instances(names);
        self.markers_apply(actions);
        self
    }

    /// Carries out the marker keeper's actions: its subscriptions first,
    /// then the values they already held. Those ask only for reads and the
    /// markers (`Keeper::value`), so two steps do it, with no loop a mutant
    /// could make endless.
    pub(super) fn markers_apply(&mut self, actions: Vec<markers::Action>) {
        let cached = self.markers_do(actions);
        let mut next = Vec::new();
        for (key, state) in cached {
            next.extend(self.markers.value(&key, cached_value(&state)));
        }
        self.markers_do(next);
    }

    /// Carries out actions; the subscriptions' cached states, by key.
    fn markers_do(&mut self, actions: Vec<markers::Action>) -> Vec<(String, Cached)> {
        let mut cached: Vec<(String, Cached)> = Vec::new();
        for action in actions {
            match action {
                markers::Action::Sub(watch) => {
                    let (instance, target, prop) = watch.target();
                    let instance = instance.to_string();
                    match self
                        .subs
                        .subscribe(MARKERS_CLIENT, &instance, &target, prop, false)
                    {
                        Ok(reply) => {
                            self.markers.subscribed(watch, reply.key.clone());
                            if let Some(state) = reply.cached {
                                cached.push((reply.key, state));
                            }
                        }
                        Err((_, error)) => {
                            tracing::warn!(instance = %instance, target = %target, error = %error, "the hub cannot follow the Tuner markers there");
                        }
                    }
                }
                markers::Action::Unsub { key } => {
                    self.subs.unsubscribe(MARKERS_CLIENT, &key);
                }
                markers::Action::Read {
                    instance,
                    seq,
                    step,
                    commands,
                } => self.markers_read(instance, seq, step, commands),
                markers::Action::Retry { instance } => self.markers_retry(instance),
                markers::Action::Found(found) => self.markers_found(found),
            }
        }
        cached
    }

    /// Reads a batch of the markers; the answer comes back as
    /// [`RouterMsg::MarkersRead`].
    fn markers_read(&self, instance: String, seq: u64, step: markers::Step, commands: Vec<Value>) {
        let Some(live) = self.live.get(&instance) else {
            return;
        };
        let result = live.call(commands);
        let tx = self.io.tx.clone();
        tokio::spawn(async move {
            let outcome = result.await.map_err(|e| e.to_string());
            let _ = tx.send(RouterMsg::MarkersRead {
                instance,
                seq,
                step,
                outcome,
            });
        });
    }

    /// Asks the keeper to read `instance` again after `RETRY_MS`.
    fn markers_retry(&self, instance: String) {
        let tx = self.io.tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(markers::RETRY_MS)).await;
            let _ = tx.send(RouterMsg::MarkersRetry { instance });
        });
    }

    /// The markers changed: the store composes them with its frame, and a
    /// new composition goes out as a layout change.
    pub(super) fn markers_found(&mut self, found: Vec<Found>) {
        let Some(store) = self.layout_store.clone() else {
            return;
        };
        tracing::info!(found = found.len(), "the Tuner markers changed");
        if let Some(rev) = store.set_markers(found) {
            let served = store.current().1;
            self.handle(layout_msg(rev, served.as_deref()));
        }
    }
}
