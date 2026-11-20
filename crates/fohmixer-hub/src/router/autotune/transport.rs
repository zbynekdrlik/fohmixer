//! The Live set's own events for AUTOTUNE (#104, #140; split from
//! `router/autotune.rs`, #234): its connect walks its targets; its
//! disconnect voids its walks and drops every ref, value and decision
//! first; a track-list change and unsettled markers hold its writes (DRY
//! first through the last known refs) until the release walks and decides
//! again; its transport and a lost transport decide every target (a play
//! start's wait is `engage.rs`, the spreads `spread.rs`).

use super::super::Router;
use crate::rules::autotune::keys::KeySettleAct;
use crate::rules::autotune::{Cause, EngageAct, Step};

impl Router {
    /// `instance` connected: its targets are walked (the script's refs start
    /// over with each connection); each answer decides it.
    pub(in crate::router) fn autotune_connected(&mut self, instance: &str) {
        for key in self.autotune.keys_of(Some(instance), false) {
            self.autotune_walk(&key);
        }
        self.autotune_tell();
    }

    /// `instance` went offline: first its walks in flight are voided, then
    /// every ref, value and decision it found dropped and the rule told;
    /// its next session starts afresh (the script made its DRY writes).
    pub(in crate::router) fn autotune_offline(&mut self, instance: &str) {
        // A stop's DRY still to come: the script made it (#140).
        self.autotune_unspread(instance, None, "offline");
        // A key still to come goes to none of the next session's refs.
        self.autotune_key_unpace(instance, KeySettleAct::Offline);
        self.autotune_engage_cancel(instance, EngageAct::Offline);
        // The walks still to come: the connect walks them all again (#235).
        self.autotune_pace_offline(instance);
        let keys = self.autotune.keys_of(Some(instance), false);
        for key in &keys {
            self.autotune.walks += 1;
            let walk = self.autotune.walks;
            if let Some(target) = self.autotune.targets.get_mut(key) {
                target.walk = walk;
                target.found = None;
                target.problem = None;
                target.decision = None;
                target.writing.clear();
                target.guard.reset();
                target.refusal = None;
                target.sure = true;
                target.key = None;
            }
            self.autotune_unsub(key);
            self.autotune_key_unsub(key);
            self.autotune_key_forget(key);
        }
        // The script's DRY writes died with its connection (#140 PR 3a):
        // an answer still on its way is void, and the connect's walks
        // register afresh.
        self.autotune.safe.remove(instance);
        self.autotune.retiring.remove(instance);
        self.autotune_position_offline(instance);
        self.autotune_hold(instance, "offline");
        self.autotune.rule.on_disconnect(instance);
        self.autotune_tell();
    }

    /// `instance`'s track list changed: its writes are held (DRY first
    /// through the last known refs) and its targets walked again (an index
    /// may name another track now), keeping what they found until the
    /// answer.
    pub(in crate::router) fn autotune_tracks(&mut self, instance: &str) {
        self.autotune_hold(instance, "tracks");
        for key in self.autotune.keys_of(Some(instance), false) {
            // Its key device's values may be another track's until its walk
            // answers (PR 2a review M3), and so may its rack: every change
            // unconfirms it, a second one within the hold too (PR #147
            // re-review R1).
            if let Some(target) = self.autotune.targets.get_mut(&key) {
                target.key_sure = false;
                if target.found.is_some() {
                    target.sure = false;
                }
            }
            self.autotune_walk(&key);
        }
    }

    /// Holds `instance`'s writes (`why`: the cause), recorded once; only
    /// for a Live set with AUTOTUNE targets. A selector the rule had on is
    /// written DRY first, through its last known ref (spec §7: a hold never
    /// leaves a singer on autotune; refs are objects, never an index).
    pub(super) fn autotune_hold(&mut self, instance: &str, why: &str) {
        if self.autotune.keys_of(Some(instance), false).is_empty()
            || self.autotune.held.contains(instance)
        {
            return;
        }
        // A stop's DRY still to come goes now, in the hold's batch (#140: a
        // hold's DRY is immediate).
        self.autotune_unspread(instance, None, "hold");
        let going = self.autotune_judge(instance, Cause::Hold, true);
        self.autotune_select(instance, Cause::Hold, going, false);
        self.autotune.held.insert(instance.to_string());
        // What each target found is of the track its index named before:
        // unconfirmed until its walk answers (PR #147 review I4).
        for key in self.autotune.keys_of(Some(instance), false) {
            let Some(target) = self.autotune.targets.get_mut(&key) else {
                continue;
            };
            if target.found.is_some() {
                target.sure = false;
            }
        }
        self.autotune_step(
            Step::Held {
                instance: instance.to_string(),
                on: true,
                why: why.to_string(),
            },
            None,
        );
    }

    /// Ends `instance`'s hold once its markers are settled (`why`: how),
    /// recorded; its DRY writes are registered as the hold kept its found
    /// targets (#140 PR 3a), its targets are walked again and the set is
    /// decided again (each target unconfirmed since the hold DRY until its
    /// walk answers: PR #147 review I4). Settled with no hold, a
    /// registration a walk's answer left to the markers goes now, and the
    /// set is decided again too (PR #147 review N2: a decision judged held
    /// for unsettled markers is lost otherwise).
    pub(in crate::router) fn autotune_release(&mut self, instance: &str, why: &str) {
        if !self.reverb_settled(instance) {
            return;
        }
        if !self.autotune.held.remove(instance) {
            self.autotune_safe(instance);
            self.autotune_decide(instance, Cause::Release);
            return;
        }
        self.autotune_step(
            Step::Held {
                instance: instance.to_string(),
                on: false,
                why: why.to_string(),
            },
            None,
        );
        self.autotune_safe(instance);
        for key in self.autotune.keys_of(Some(instance), false) {
            self.autotune_walk(&key);
        }
        self.autotune_decide(instance, Cause::Release);
    }

    /// A transport change of `instance` (`prev`: the value before, none for
    /// a session's first): the set's unfound targets walked again, the key
    /// in force judged, every target decided (a stop is DRY at once).
    pub(in crate::router) fn autotune_transport(
        &mut self,
        instance: &str,
        playing: bool,
        prev: Option<bool>,
    ) {
        // Every transport change drops a spread still to come, before it
        // decides: a start a stop's DRY (its selectors are decided afresh,
        // #140), a stop a switch-on's, so no switch-on goes after the
        // stop's DRY (#190).
        let why = if playing { "start" } else { "stop" };
        self.autotune_unspread(instance, None, why);
        if playing {
            // #190: a play start's switch-on waits, with a delay set, and
            // its key writes start `KEY_AFTER_START_MS` after it.
            self.autotune_engage_start(instance, prev);
            self.autotune_key_start(instance, prev);
        } else {
            self.autotune_engage_cancel(instance, EngageAct::Stopped);
        }
        self.autotune.rule.on_playing(instance, playing);
        for key in self.autotune.keys_of(Some(instance), true) {
            self.autotune_walk(&key);
        }
        self.autotune_position_playing(instance, Some(playing));
        self.autotune_decide(instance, Cause::Transport);
        self.autotune_tell();
    }

    /// The rules' transport listener of `instance` failed: AUTOTUNE forgets
    /// its transport, so every target goes DRY (an unknown transport is no
    /// play).
    pub(in crate::router) fn autotune_transport_lost(&mut self, instance: &str) {
        self.autotune_engage_cancel(instance, EngageAct::Stopped);
        self.autotune.rule.transport_lost(instance);
        self.autotune_position_playing(instance, None);
        self.autotune_decide(instance, Cause::Transport);
        self.autotune_tell();
    }
}
