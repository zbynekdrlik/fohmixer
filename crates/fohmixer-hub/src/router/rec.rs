//! The router's recording watch (#172): the glue of `rules/rec.rs`. Only the
//! `band` Live is watched ([`REC_INSTANCE`]). It
//!
//! - follows `live_set record_mode` from the connect, whatever the lists do,
//!   and tries a failed subscription again ([`mode_again`]);
//! - takes that Live's track list from the marker keeper's lists, reads each
//!   track's `can_be_armed` (Live has no listener on it, and a group, a
//!   return or the main track has no `arm`: it is never followed there), and
//!   reads again when the list changes, with the connect-time retries
//!   (`retries.rs`);
//! - follows each armable track's `arm` with its own subscriber
//!   [`REC_CLIENT`] (in the `Status` `own` lists), so the watch holds with no
//!   page open; at a list change a track that stays keeps its known `arm`
//!   and its subscription, a subscription whose index names another track
//!   now is made afresh (an index path is bound to its object when it
//!   resolves), and nothing is told or logged until the new list's arms are
//!   heard;
//! - takes the transport from the rules' own (`on_transport`);
//! - tells every page ([`ServerMsg::Rec`]) on a change and at attach, and
//!   logs each change of the transport, the record button or the armed
//!   tracks: one info line and one `rec` record (warn class), so the event
//!   log shows when and why a recording stopped.
//!
//! It never writes anything to Live: the ARM keys and the foot's REC key
//! write as any control does.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use fohmixer_proto::client::{REC_INSTANCE, RecState, ServerMsg, ValueItem, hub_key};
use serde_json::Value;

use super::lufs::{item_name, mute_of, read_again};
use super::markers::TRACKS;
use super::retries::{CONNECT_RETRIES, RETRY_MS};
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

/// Whether a failed `record_mode` subscription is tried again after
/// `failures` in a row: the arm reads' bound.
fn mode_again(failures: u32) -> bool {
    failures <= CONNECT_RETRIES
}

/// The `arm` subscriptions the new list `tracks` ends: each whose index
/// names another track now or none. An index path is bound to its object
/// when it resolves, so a kept key would follow the old track.
fn gone_keys(arms: &BTreeMap<String, u32>, rule: &Rec, tracks: &[(u32, String)]) -> Vec<String> {
    arms.iter()
        .filter(|(_, index)| {
            !tracks
                .iter()
                .any(|(i, name)| i == *index && rule.name_at(*i) == Some(name.as_str()))
        })
        .map(|(key, _)| key.clone())
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
    /// `record_mode`'s failed subscriptions in a row.
    mode_failures: u32,
    /// The new list's tracks whose `arm` is not heard yet.
    waiting: BTreeSet<u32>,
    /// Why a change waits for the new list's arms (the first cause).
    held: Option<&'static str>,
}

impl RecIo {
    /// A new list is on its way: nothing is told until its arms are heard.
    fn settling(&self) -> bool {
        self.reading.is_some() || !self.waiting.is_empty()
    }
}

impl Router {
    /// `instance` connected: the `band` Live's record button is followed.
    pub(super) fn rec_connected(&mut self, instance: &str) {
        if instance == REC_INSTANCE {
            self.rec.mode_failures = 0;
            self.rec_follow_mode();
        }
    }

    /// The marker keeper's lists changed: a new track list of the `band`
    /// Live reads its armable tracks.
    pub(super) fn rec_lists(&mut self) {
        let key = hub_key(REC_INSTANCE, SONG, TRACKS, false);
        let Some(list) = self.reverb.lists.get(&key).cloned() else {
            return;
        };
        if self.rec.list.as_ref() == Some(&list) {
            return;
        }
        self.rec.list = Some(list);
        self.rec.failures = 0;
        self.rec_read();
    }

    /// Follows the record button: the table keeps it through a reconnect.
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
            Err((_, error)) => self.rec_mode_failed(&error),
        }
    }

    /// The record button's subscription failed: tried again while the bound
    /// lasts, the next connect starts it over.
    fn rec_mode_failed(&mut self, error: &str) {
        self.rec.mode_failures += 1;
        if !mode_again(self.rec.mode_failures) {
            tracing::warn!(error = %error, "rec: cannot follow the record button until the next connect");
            return;
        }
        tracing::warn!(error = %error, "rec: cannot follow the record button, trying again");
        let tx = self.io.tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(RETRY_MS)).await;
            let _ = tx.send(RouterMsg::RecModeRetry);
        });
    }

    /// A failed `record_mode` subscription is due again.
    pub(super) fn rec_mode_retry(&mut self) {
        if self.states.get(REC_INSTANCE).is_some_and(|s| s.online) {
            self.rec_follow_mode();
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
            Ok(tracks) => {
                self.rec.reading = None;
                self.rec_follow(tracks);
            }
            Err(why) => {
                self.rec.failures += 1;
                if !read_again(&why, self.rec.failures) {
                    tracing::warn!(error = %why, "rec: the armable tracks could not be read; the last ones stay until the next list change or connect");
                    self.rec.reading = None;
                    self.rec_changed("tracks");
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

    /// Follows the `arm` of `tracks`: a subscription stays only while its
    /// index names the same track (it is bound to that track's object), a
    /// track that stays keeps its `arm`. A change is told once every `arm`
    /// is heard.
    fn rec_follow(&mut self, tracks: Vec<(u32, String)>) {
        tracing::info!(armable = tracks.len(), "rec: the armable tracks are read");
        let wanted: BTreeSet<u32> = tracks.iter().map(|(index, _)| *index).collect();
        let gone = gone_keys(&self.rec.arms, &self.rec.rule, &tracks);
        for key in gone {
            self.rec.arms.remove(&key);
            self.subs.unsubscribe(REC_CLIENT, &key);
        }
        self.rec.rule.set_tracks(tracks);
        self.rec.waiting.clear();
        for index in wanted {
            let target = track_target(index);
            match self
                .subs
                .subscribe(REC_CLIENT, REC_INSTANCE, &target, "arm", false)
            {
                Ok(reply) => {
                    self.rec.arms.insert(reply.key, index);
                    if !self.rec.rule.arm_known(index) {
                        match reply.cached {
                            Some(Cached::Value { value, .. }) => {
                                self.rec.rule.set_arm(index, mute_of(&value));
                            }
                            Some(Cached::Error(_)) => {}
                            None => {
                                self.rec.waiting.insert(index);
                            }
                        }
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
                if let Some(error) = &item.error {
                    self.subs.unsubscribe(REC_CLIENT, &item.sub);
                    self.rec.mode_key = None;
                    self.rec_mode_failed(error);
                } else {
                    self.rec.mode_failures = 0;
                }
            } else if let Some(index) = self.rec.arms.get(&item.sub).copied() {
                self.rec.rule.set_arm(index, heard);
                self.rec.waiting.remove(&index);
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
        self.rec.waiting.clear();
        self.rec.held = None;
        self.rec_unfollow();
        self.rec.rule.clear();
        self.rec_changed("offline");
    }

    /// Logs a change and tells every page.
    fn rec_changed(&mut self, why: &'static str) {
        if self.rec.settling() {
            self.rec.held.get_or_insert(why);
            return;
        }
        let why = self.rec.held.take().unwrap_or(why);
        let now = self.rec.rule.state();
        let armable = self.rec.rule.armable();
        if let Some(fields) = change_fields(&self.rec.told, &now, armable, why) {
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

    use super::super::tests::{attach, offline_router, records};
    use super::*;
    use crate::live::client::LiveError;
    use fohmixer_proto::client::RecTrack;

    fn item(sub: &str, value: Option<Value>, error: Option<&str>) -> ValueItem {
        ValueItem {
            sub: sub.to_string(),
            value,
            display: None,
            error: error.map(str::to_string),
        }
    }

    fn online(router: &mut Router) {
        router.states.get_mut(REC_INSTANCE).expect("band").online = true;
    }

    /// The hub key of track `index`'s `arm`.
    fn arm_key(index: u32) -> String {
        hub_key(REC_INSTANCE, &track_target(index), "arm", false)
    }

    /// Two tracks read and heard, Vox 1 armed.
    fn settled(router: &mut Router) {
        router.rec_follow(vec![(0, "Vox 1".to_string()), (1, "Vox 2".to_string())]);
        router.rec_values(vec![
            item(&arm_key(0), Some(json!(true)), None),
            item(&arm_key(1), Some(json!(false)), None),
        ]);
    }

    #[tokio::test]
    async fn a_list_change_keeps_a_staying_tracks_arm_and_says_nothing_until_it_is_heard() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, rx) = offline_router(dir.path());
        settled(&mut router);
        let page = attach(&mut router, 1);
        let _ = page.take();
        let _ = records(&rx);
        // A list change: its read runs, and a value of the old list counts
        // for the old list's track.
        router.rec.reading = Some(7);
        router.rec_values(vec![item(&arm_key(1), Some(json!(true)), None)]);
        assert_eq!(page.take(), Some(Vec::new()), "no page told while it runs");
        assert!(records(&rx).is_empty());
        // Back: Vox 2 off, then the read: Vox 1 moved to index 1, a new
        // track at 2; Vox 2 went.
        router.rec_values(vec![item(&arm_key(1), Some(json!(false)), None)]);
        router.rec.reading = None;
        router.rec_follow(vec![(1, "Vox 1".to_string()), (2, "New".to_string())]);
        // Index 1 names another track: followed afresh; 0's went, 2's is new.
        assert!(router.rec.arms.contains_key(&arm_key(1)));
        assert!(router.rec.arms.contains_key(&arm_key(2)));
        assert!(!router.rec.arms.contains_key(&arm_key(0)));
        assert!(router.rec.settling(), "the new track's arm is not heard");
        assert!(router.rec.waiting.contains(&2) && router.rec.waiting.len() == 1);
        assert_eq!(page.take(), Some(Vec::new()), "no page told yet");
        assert!(records(&rx).is_empty(), "no false record");
        router.rec_values(vec![item(&arm_key(2), Some(json!(false)), None)]);
        let want = RecState {
            playing: None,
            record_mode: None,
            tracks: vec![
                RecTrack {
                    index: 1,
                    name: "Vox 1".into(),
                    arm: true,
                },
                RecTrack {
                    index: 2,
                    name: "New".into(),
                    arm: false,
                },
            ],
        };
        assert_eq!(page.take(), Some(vec![ServerMsg::Rec(want)]));
        assert!(!router.rec.settling());
        let got = records(&rx);
        // The armed tracks stayed: one track armed, no record of a change.
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn a_subscription_stays_only_while_its_index_names_the_same_track() {
        let mut rule = Rec::default();
        rule.set_tracks(vec![(0, "Vox 1".to_string()), (1, "Vox 2".to_string())]);
        let arms = BTreeMap::from([("a".to_string(), 0), ("b".to_string(), 1)]);
        // Vox 2 stays at 1; index 0 names a new track.
        let tracks = vec![(0, "New".to_string()), (1, "Vox 2".to_string())];
        assert_eq!(gone_keys(&arms, &rule, &tracks), vec!["a".to_string()]);
        // A shifted list: both indexes name others or none.
        let shifted = vec![(0, "Vox 2".to_string())];
        assert_eq!(gone_keys(&arms, &rule, &shifted).len(), 2);
        let same = vec![(0, "Vox 1".to_string()), (1, "Vox 2".to_string())];
        assert!(gone_keys(&arms, &rule, &same).is_empty());
    }

    #[tokio::test]
    async fn a_change_held_for_the_arms_names_its_first_cause() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, rx) = offline_router(dir.path());
        settled(&mut router);
        let _ = records(&rx);
        router.rec.waiting.insert(1);
        router.rec_transport(REC_INSTANCE, Some(true));
        assert!(records(&rx).is_empty());
        router.rec_values(vec![item(&arm_key(1), Some(json!(false)), None)]);
        let got = records(&rx);
        assert_eq!(got[0]["why"], json!("transport"));
        assert_eq!(got[0]["armable"], json!(2));
    }

    #[tokio::test]
    async fn the_record_button_is_followed_from_the_connect_with_no_list() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, _records) = offline_router(dir.path());
        assert_eq!(router.rec.mode_key, None);
        router.rec_connected("master");
        assert_eq!(router.rec.mode_key, None, "another Live");
        router.rec_connected(REC_INSTANCE);
        assert!(router.reverb.lists.is_empty(), "no list");
        assert_eq!(
            router.rec.mode_key.as_deref(),
            Some("band|live_set|record_mode|false")
        );
    }

    #[tokio::test]
    async fn a_failed_record_button_subscription_is_tried_again_within_the_bound() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, _records) = offline_router(dir.path());
        online(&mut router);
        router.rec_connected(REC_INSTANCE);
        let key = router.rec.mode_key.clone().unwrap();
        router.rec_values(vec![item(&key, None, Some("no listener"))]);
        assert_eq!(router.rec.mode_key, None);
        assert_eq!(router.rec.mode_failures, 1);
        // The retry follows again; a value ends the failures.
        router.handle(RouterMsg::RecModeRetry);
        assert_eq!(router.rec.mode_key.as_deref(), Some(key.as_str()));
        router.rec_values(vec![item(&key, Some(json!(true)), None)]);
        assert_eq!(router.rec.mode_failures, 0);
        // An offline Live waits for its connect.
        router.rec_values(vec![item(&key, None, Some("no listener"))]);
        router.states.get_mut(REC_INSTANCE).unwrap().online = false;
        router.handle(RouterMsg::RecModeRetry);
        assert_eq!(router.rec.mode_key, None);
    }

    /// Whether a `RecModeRetry` comes within two delays.
    async fn mode_retry_comes(rx: &mut tokio::sync::mpsc::UnboundedReceiver<RouterMsg>) -> bool {
        tokio::time::sleep(Duration::from_millis(2 * RETRY_MS)).await;
        std::iter::from_fn(|| rx.try_recv().ok()).any(|m| matches!(m, RouterMsg::RecModeRetry))
    }

    #[tokio::test(start_paused = true)]
    async fn a_record_buttons_retry_comes_only_within_the_bound() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, mut rx, _records) = offline_router(dir.path());
        router.rec_mode_failed("no listener");
        assert!(mode_retry_comes(&mut rx).await, "within the bound");
        router.rec.mode_failures = CONNECT_RETRIES;
        router.rec_mode_failed("no listener");
        assert!(!mode_retry_comes(&mut rx).await, "past the bound");
    }

    #[tokio::test]
    async fn every_read_has_a_number_of_its_own() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, _records) = offline_router(dir.path());
        router.rec.list = Some(json!([]));
        router.rec_read();
        assert_eq!(router.rec.reading, Some(1));
        router.rec_read();
        assert_eq!(router.rec.reading, Some(2));
    }

    #[tokio::test]
    async fn a_failed_read_waits_for_its_retry_only_while_live_may_still_answer() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, _records) = offline_router(dir.path());
        router.rec.list = Some(json!([]));
        router.rec_read();
        router.rec_read_done(1, Err(LiveError::Timeout.to_string()));
        assert_eq!(router.rec.reading, Some(1), "a retry comes");
        assert_eq!(router.rec.failures, 1);
        router.rec_read_done(1, Err(LiveError::Timeout.to_string()));
        assert_eq!(router.rec.failures, 2);
        router.rec_read_done(1, Err("refused".to_string()));
        assert_eq!(router.rec.reading, None, "a refusal ends it");
    }

    #[tokio::test]
    async fn no_change_tells_the_pages_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, _records) = offline_router(dir.path());
        settled(&mut router);
        let page = attach(&mut router, 1);
        let _ = page.take();
        router.rec_changed("arm");
        assert_eq!(page.take(), Some(Vec::new()));
    }

    #[tokio::test]
    async fn a_due_retry_reads_only_the_newest_read_of_a_live_online() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, _records) = offline_router(dir.path());
        router.rec.list = Some(json!([]));
        router.rec_read();
        router.rec_retry(1);
        assert_eq!(router.rec.reading, Some(1), "offline: no new read");
        online(&mut router);
        router.rec_retry(7);
        assert_eq!(router.rec.reading, Some(1), "another read: no new read");
        router.rec_retry(1);
        assert_eq!(router.rec.reading, Some(2), "read again");
    }

    #[tokio::test]
    async fn an_offline_live_ends_every_arm_subscription() {
        let dir = tempfile::tempdir().unwrap();
        let (mut router, _rx, _records) = offline_router(dir.path());
        settled(&mut router);
        assert!(router.subs.clients_of(&arm_key(0)).contains(&REC_CLIENT));
        router.rec_offline(REC_INSTANCE);
        assert!(router.rec.arms.is_empty());
        assert!(!router.subs.clients_of(&arm_key(0)).contains(&REC_CLIENT));
    }

    #[test]
    fn the_record_buttons_retries_end_at_the_arm_reads_bound() {
        assert!(mode_again(CONNECT_RETRIES));
        assert!(!mode_again(CONNECT_RETRIES + 1));
    }

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
