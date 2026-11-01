//! The router's recording watch (#172): the glue of `rules/rec.rs`. Only the
//! `band` Live is watched ([`REC_INSTANCE`]). It
//!
//! - takes that Live's track list from the marker keeper's lists, reads each
//!   track's `can_be_armed` (Live has no listener on it, and a group, a
//!   return or the main track has no `arm`: it is never followed there), and
//!   reads again when the list changes, with the connect-time retries
//!   (`retries.rs`);
//! - follows `live_set record_mode` and each armable track's `arm` with its
//!   own subscriber [`REC_CLIENT`] (in the `Status` `own` lists), so the
//!   watch holds with no page open;
//! - takes the transport from the rules' own (`on_transport`);
//! - tells every page ([`ServerMsg::Rec`]) on a change and at attach, and
//!   logs each change of the transport, the record button or the armed
//!   tracks: one info line and one `rec` record (warn class), so the event
//!   log shows when and why a recording stopped.
//!
//! It never writes anything to Live: the ARM keys and the foot's REC key
//! write as any control does.

use std::collections::BTreeMap;
use std::time::Duration;

use fohmixer_proto::client::{REC_INSTANCE, RecState, ServerMsg, ValueItem, hub_key};
use serde_json::Value;

use super::lufs::{item_name, mute_of, read_again};
use super::markers::TRACKS;
use super::retries::RETRY_MS;
use super::{Router, RouterMsg};
use crate::eq::walk::{answered, get};
use crate::live::subs::{Cached, ClientId};
use crate::outbox::Outbox;
use crate::rules::rec::{Rec, armable, change_fields, silent};

/// The watch's own subscriber (`record_mode`, the tracks' `arm`).
pub const REC_CLIENT: ClientId = ClientId::MAX - 6;
/// The target of the record button.
const SONG: &str = "live_set";
/// The record button's property.
const RECORD_MODE: &str = "record_mode";

/// The LOM path of track `index`.
fn track_target(index: u32) -> String {
    format!("{SONG} {TRACKS} {index}")
}

/// The `can_be_armed` read of every item of a track list.
fn reads_of(list: &Value) -> Vec<Value> {
    let tracks = list.as_array().map_or(0, Vec::len);
    (0..tracks as u32)
        .map(|index| get(&track_target(index), "can_be_armed"))
        .collect()
}

/// The names of a track list's items, in its order.
fn names_of(list: &Value) -> Vec<Option<String>> {
    let tracks = list.as_array().map_or(0, Vec::len);
    (0..tracks as u32)
        .map(|index| item_name(list, index))
        .collect()
}

/// The `can_be_armed` answers, one per slot (none: no answer).
fn answers_of(slots: &[Value]) -> Vec<Option<bool>> {
    slots
        .iter()
        .map(|slot| answered(slot).and_then(mute_of))
        .collect()
}

/// The router's recording watch.
#[derive(Debug, Default)]
pub(super) struct RecIo {
    pub rule: Rec,
    /// The state as last told and logged.
    told: RecState,
    /// The `record_mode` subscription's key, once made.
    mode_key: Option<String>,
    /// The `arm` subscriptions: hub key → track index.
    arms: BTreeMap<String, u32>,
    /// The track list the armable tracks were read of.
    list: Option<Value>,
    /// The reads' counter, the newest one's number and its failures in a row.
    reads: u64,
    reading: Option<u64>,
    failures: u32,
    /// The read on its way: a disconnect aborts it.
    task: Option<tokio::task::AbortHandle>,
}

impl Router {
    /// The marker keeper's lists changed: a new track list of the `band`
    /// Live reads its armable tracks.
    pub(super) fn rec_lists(&mut self) {
        let key = hub_key(REC_INSTANCE, SONG, TRACKS, false);
        let Some(list) = self.reverb.lists.get(&key).cloned() else {
            return;
        };
        self.rec_follow_mode();
        if self.rec.list.as_ref() == Some(&list) {
            return;
        }
        self.rec.list = Some(list);
        self.rec.failures = 0;
        self.rec_read();
    }

    /// Follows the record button once: the table keeps it through a
    /// reconnect.
    fn rec_follow_mode(&mut self) {
        if self.rec.mode_key.is_some() {
            return;
        }
        match self
            .subs
            .subscribe(REC_CLIENT, REC_INSTANCE, SONG, RECORD_MODE, false)
        {
            Ok(reply) => {
                self.rec.mode_key = Some(reply.key);
                if let Some(Cached::Value { value, .. }) = reply.cached {
                    self.rec.rule.set_record_mode(mute_of(&value));
                    self.rec_changed(RECORD_MODE);
                }
            }
            Err((_, error)) => {
                tracing::warn!(error = %error, "rec: cannot follow the record button");
            }
        }
    }

    /// Reads `can_be_armed` of every track of the list.
    fn rec_read(&mut self) {
        let (Some(live), Some(list)) = (self.live.get(REC_INSTANCE).cloned(), &self.rec.list)
        else {
            return;
        };
        let commands = reads_of(list);
        self.rec.reads += 1;
        let read = self.rec.reads;
        self.rec.reading = Some(read);
        if let Some(task) = self.rec.task.take() {
            task.abort();
        }
        let tx = self.io.tx.clone();
        let task = tokio::spawn(async move {
            let outcome = live.call(commands).await.map_err(|e| e.to_string());
            let _ = tx.send(RouterMsg::RecRead { read, outcome });
        });
        self.rec.task = Some(task.abort_handle());
    }

    /// A read's answer (only the newest counts): the armable tracks
    /// followed, or the read tried again.
    pub(super) fn rec_read_done(&mut self, read: u64, outcome: Result<Vec<Value>, String>) {
        if self.rec.reading != Some(read) {
            return;
        }
        let names = self.rec.list.as_ref().map(names_of).unwrap_or_default();
        let found = outcome.and_then(|slots| armable(&names, &answers_of(&slots)));
        match found {
            Ok(tracks) => self.rec_follow(tracks),
            Err(why) => {
                self.rec.failures += 1;
                if !read_again(&why, self.rec.failures) {
                    tracing::warn!(error = %why, "rec: the armable tracks could not be read; no ARM key until the next list change or connect");
                    return;
                }
                let tx = self.io.tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(RETRY_MS)).await;
                    let _ = tx.send(RouterMsg::RecRetry { read });
                });
            }
        }
    }

    /// A failed read is due again, while it is still the newest.
    pub(super) fn rec_retry(&mut self, read: u64) {
        let online = self.states.get(REC_INSTANCE).is_some_and(|s| s.online);
        if online && self.rec.reading == Some(read) {
            self.rec_read();
        }
    }

    /// Follows the `arm` of `tracks`, the old ones dropped.
    fn rec_follow(&mut self, tracks: Vec<(u32, String)>) {
        tracing::info!(armable = tracks.len(), "rec: the armable tracks are read");
        self.rec_unfollow();
        let indexes: Vec<u32> = tracks.iter().map(|(index, _)| *index).collect();
        self.rec.rule.set_tracks(tracks);
        for index in indexes {
            let target = track_target(index);
            match self
                .subs
                .subscribe(REC_CLIENT, REC_INSTANCE, &target, "arm", false)
            {
                Ok(reply) => {
                    self.rec.arms.insert(reply.key, index);
                    if let Some(Cached::Value { value, .. }) = reply.cached {
                        self.rec.rule.set_arm(index, mute_of(&value));
                    }
                }
                Err((_, error)) => {
                    tracing::warn!(target = %target, error = %error, "rec: cannot follow a track's arm");
                }
            }
        }
        self.rec_changed("tracks");
    }

    /// Ends every `arm` subscription.
    fn rec_unfollow(&mut self) {
        for key in std::mem::take(&mut self.rec.arms).into_keys() {
            self.subs.unsubscribe(REC_CLIENT, &key);
        }
    }

    /// The deliveries of one flush of the watch's subscriptions, told once.
    pub(super) fn rec_values(&mut self, items: Vec<ValueItem>) {
        let mut why = None;
        for item in items {
            if let Some(error) = &item.error {
                tracing::warn!(key = %item.sub, error = %error, "rec: a subscription failed");
            }
            let heard = item.value.as_ref().and_then(mute_of);
            if self.rec.mode_key.as_deref() == Some(item.sub.as_str()) {
                self.rec.rule.set_record_mode(heard);
                why = Some(RECORD_MODE);
            } else if let Some(index) = self.rec.arms.get(&item.sub).copied() {
                self.rec.rule.set_arm(index, heard);
                why = why.or(Some("arm"));
            }
        }
        if let Some(why) = why {
            self.rec_changed(why);
        }
    }

    /// A transport value of `instance` (none: it failed).
    pub(super) fn rec_transport(&mut self, instance: &str, playing: Option<bool>) {
        if instance == REC_INSTANCE {
            self.rec.rule.set_playing(playing);
            self.rec_changed("transport");
        }
    }

    /// `instance` went offline: for the `band` Live nothing is known until
    /// its next list.
    pub(super) fn rec_offline(&mut self, instance: &str) {
        if instance != REC_INSTANCE {
            return;
        }
        if let Some(task) = self.rec.task.take() {
            task.abort();
        }
        self.rec.reading = None;
        self.rec.list = None;
        self.rec_unfollow();
        self.rec.rule.clear();
        self.rec_changed("offline");
    }

    /// Logs a change and tells every page.
    fn rec_changed(&mut self, why: &str) {
        let now = self.rec.rule.state();
        if let Some(fields) = change_fields(&self.rec.told, &now, why) {
            tracing::info!(
                state = now.watch().name(),
                why,
                is_playing = ?now.playing,
                record_mode = ?now.record_mode,
                armed = now.armed(),
                "rec: the recording watch changed"
            );
            self.io.events.record("rec", fields);
        }
        if now != self.rec.told && !silent(&self.rec.told, &now) {
            for outbox in self.clients.values() {
                outbox.reply(ServerMsg::Rec(now.clone()));
            }
        }
        self.rec.told = now;
    }

    /// A page's attach: the state now.
    pub(super) fn rec_attach(&self, outbox: &Outbox) {
        outbox.reply(ServerMsg::Rec(self.rec.told.clone()));
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn its_client_is_its_own() {
        assert_eq!(REC_CLIENT, ClientId::MAX - 6);
    }

    #[test]
    fn a_list_reads_each_tracks_can_be_armed_and_names_its_items() {
        let list = json!([{"name": "Vox 1"}, {"id": 7}, {"name": "Vox 2"}]);
        assert_eq!(
            reads_of(&list),
            vec![
                json!({"target": "live_set tracks 0", "name": "get_prop", "args": {"prop": "can_be_armed"}}),
                json!({"target": "live_set tracks 1", "name": "get_prop", "args": {"prop": "can_be_armed"}}),
                json!({"target": "live_set tracks 2", "name": "get_prop", "args": {"prop": "can_be_armed"}}),
            ]
        );
        assert_eq!(
            names_of(&list),
            vec![Some("Vox 1".to_string()), None, Some("Vox 2".to_string())]
        );
        assert!(reads_of(&json!("no list")).is_empty());
        assert!(names_of(&json!("no list")).is_empty());
    }

    #[test]
    fn an_answer_is_a_slots_bool_and_a_refused_slot_has_none() {
        let slots = [
            json!({"ok": true, "data": true}),
            json!({"ok": true, "data": false}),
            json!({"ok": false, "error": "no such property"}),
        ];
        assert_eq!(answers_of(&slots), vec![Some(true), Some(false), None]);
    }
}
