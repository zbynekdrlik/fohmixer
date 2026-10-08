//! The router's part of the marker keeper (#68, `router/markers.rs`): its
//! subscriptions, its reads and their answers, and each new set of markers
//! handed to the layout store, whose new composition goes out as a layout
//! change.

use std::sync::Arc;

use fohmixer_proto::markers::Found;
use fohmixer_proto::markers::migrate::MigrationStatus;
use serde_json::Value;

use super::{MARKERS_CLIENT, Router, RouterMsg, cached_value, layout_msg, markers, write_failure};
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
    /// could make endless (`Keeper::values`: one read of an instance at
    /// most).
    pub(super) fn markers_apply(&mut self, actions: Vec<markers::Action>) {
        let cached = self.markers_do(actions);
        let next = self.markers.values(
            cached
                .iter()
                .map(|(key, state)| (key.as_str(), cached_value(state))),
        );
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
                markers::Action::Rename {
                    instance,
                    target,
                    name,
                } => self.markers_rename(&instance, target, name),
            }
        }
        cached
    }

    /// `/api/markers/migration` (#68 PR C): first every instance's lists
    /// read afresh (Live fires no list listener on a track's rename, so the
    /// keeper's names may be old); the answer comes back as
    /// [`RouterMsg::MigrationLists`].
    pub(super) fn markers_migration(
        &self,
        apply: bool,
        reply: tokio::sync::oneshot::Sender<MigrationStatus>,
    ) {
        let reads: Vec<_> = self
            .live
            .iter()
            .map(|(instance, live)| {
                let commands = [markers::TRACKS, markers::RETURNS]
                    .into_iter()
                    .map(|prop| serde_json::json!({"target": "live_set", "name": "get_prop", "args": {"prop": prop}}))
                    .collect();
                (instance.clone(), live.call(commands))
            })
            .collect();
        let tx = self.io.tx.clone();
        tokio::spawn(async move {
            let mut lists = Vec::new();
            for (instance, read) in reads {
                lists.push((instance, read.await.map_err(|e| e.to_string())));
            }
            let _ = tx.send(RouterMsg::MigrationLists {
                apply,
                reply,
                lists,
            });
        });
    }

    /// The migration's rows: the planned tracks of the store's frame as
    /// the keeper finds them with the fresh `lists`; with `apply`, the ready
    /// tracks' plain Tuners are renamed (the rows are what the keeper saw
    /// before), unless the frame has a problem or a read is running.
    pub(super) fn markers_migration_lists(
        &mut self,
        apply: bool,
        lists: &[(String, Result<Vec<Value>, String>)],
    ) -> MigrationStatus {
        let mut actions = Vec::new();
        for (instance, outcome) in lists {
            if let Err(error) = outcome {
                tracing::warn!(instance = %instance, error = %error, "the migration could not read the lists afresh");
            }
            let slots = outcome.as_ref().ok().map(Vec::as_slice);
            actions.extend(self.markers.refresh(instance, slots));
        }
        self.markers_do(actions);
        let Some(migration) = self
            .layout_store
            .as_ref()
            .and_then(|store| store.migration())
        else {
            return MigrationStatus::default();
        };
        let rows = self.markers.migration_rows(&migration.planned);
        let reading = rows.iter().any(|row| self.markers.reading(&row.instance));
        let renames = if apply && migration.problems.is_empty() {
            self.markers.renames(&migration.planned)
        } else {
            Vec::new()
        };
        let renamed = u32::try_from(renames.len()).unwrap_or(u32::MAX);
        self.markers_do(renames);
        MigrationStatus {
            rows,
            problems: migration.problems,
            reading,
            renamed,
        }
    }

    /// Names a Tuner (`set_prop name` on its `$ref`); a failure is logged.
    /// The new name comes back through the Tuner's own `name` watch.
    fn markers_rename(&self, instance: &str, target: Value, name: String) {
        let Some(live) = self.live.get(instance) else {
            return;
        };
        let shown = target.to_string();
        tracing::info!(instance = %instance, target = %shown, "the marker migration names a Tuner (#68)");
        let result = live.call(vec![serde_json::json!({
            "target": target,
            "name": "set_prop",
            "args": {"prop": "name", "value": name},
        })]);
        let tx = self.io.tx.clone();
        let instance = instance.to_string();
        tokio::spawn(async move {
            if let Some(problem) = write_failure(&result.await) {
                // The `$ref` may be gone (the script's registry starts over
                // with each connection): a read gets the Tuners' refs again.
                tracing::warn!(target = %shown, problem = %problem, "a Tuner's rename failed; the Tuners are read again");
                let _ = tx.send(RouterMsg::MarkersRetry { instance });
            }
        });
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

    /// The markers changed: they wait `SETTLE_MS` for the next change (a
    /// set load or a burst of edits is one new layout).
    pub(super) fn markers_found(&mut self, found: Vec<Found>) {
        if self.layout_store.is_none() {
            return;
        }
        self.markers_generation += 1;
        self.markers_pending = Some(found);
        let generation = self.markers_generation;
        let tx = self.io.tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(markers::SETTLE_MS)).await;
            let _ = tx.send(RouterMsg::MarkersSettle { generation });
        });
    }

    /// A change had its quiet time: when it is still the latest, the store
    /// composes the markers with its frame, and a new composition goes out
    /// as a layout change.
    pub(super) fn markers_settled(&mut self, generation: u64) {
        if generation != self.markers_generation {
            return;
        }
        let (Some(store), Some(found)) = (self.layout_store.clone(), self.markers_pending.take())
        else {
            return;
        };
        tracing::info!(found = found.len(), "the Tuner markers changed");
        if let Some(rev) = store.set_markers(found) {
            let served = store.current().1;
            self.handle(layout_msg(rev, served.as_deref()));
        }
    }
}
