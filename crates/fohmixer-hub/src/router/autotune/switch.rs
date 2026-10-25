//! The router's decisions of the autotune selectors (#140 PR 3b, spec
//! §2.2, §7): each target's [`TrackInput`] from what the hub holds (AUT,
//! its set's transport and key in force, its singer, its problems, the
//! guard, the set's registration of its DRY writes, its key's read-back),
//! [`decide`], and one batch per set of the selectors whose decision
//! differs from what Live shows, through their `$ref`s (never an index
//! path) with each target's own on or off value.
//!
//! - **One write per change:** a selector Live already shows as decided,
//!   or whose write is on its way, is not written again (`needs_write`);
//!   judged against where Live will be once the writes on their way land,
//!   so a stop right after a switch-on writes DRY at once, and every write's
//!   answer decides the set again (PR #147 review I1). A DRY decision reads
//!   the off value (N4: DRY wins), never the pages' on-window.
//! - **Unconfirmed targets** (PR #147 review I4: since a hold, until their
//!   walk answers) are DRY and written nothing.
//! - **A write Live refuses** gets one retry, then that value to that
//!   `$ref` stops and the target shows the problem until a change (I5).
//! - **Keys first at a switch-on:** a track turns on only with its key
//!   confirmed for the key in force, so the read-back decides it (`key`).
//!   A track the rule already selected stays on through a key change until
//!   its read-back disagrees.
//! - **DRY at once** at a stop, AUT off, a singer off by hand, a problem,
//!   a registration of the DRY writes not kept (PR #146 review I2), and
//!   before a hold through the last known refs (`autotune_hold`).
//! - **Another writer** (`autotune_heard`): a move of a selector the hub
//!   did not write, away from its decision, while the set is not held. To
//!   the chain while the hub wants DRY: DRY again once, then the guard
//!   stops the hub until the next song start (`SelectorGuard`); to DRY
//!   while the hub wants the chain: left so. Either is recorded
//!   (`selector_writer`, warn) and decides the track `another_writer`.
//! - **Records:** one `select` per switch of a decision (on, or DRY from
//!   on) with its why and cause, one `write` per batch; `/api/status`
//!   shows every target's last decision.

use fohmixer_proto::client::{SingerState, TargetState, TrackState};
use fohmixer_proto::layout::singer_of;
use fohmixer_proto::songmap::Key;
use serde_json::Value;

use super::super::{Router, RouterMsg};
use super::keys::Position;
use super::safe::Safe;
use super::{AutotuneIo, Target, may_write, state_of, write_commands};
use crate::rules::autotune::keys::{KeyState, shows as key_shows};
use crate::rules::autotune::switch::{
    Flying, GuardAct, KeyCheck, TrackInput, another_move, decide, key_check, needs_write,
    refused_decision,
};
use crate::rules::autotune::{Cause, Step, WriterAct, shows};

/// What a selector write sends: the target's key, its macro's `$ref` and
/// label, the decision written and its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Going {
    pub key: String,
    pub id: String,
    pub label: String,
    pub on: bool,
    pub value: u8,
}

/// Whether `list` holds the DRY write of `id` with `off`.
fn covers(list: &[(String, u8)], id: &str, off: u8) -> bool {
    list.iter()
        .any(|(listed, value)| listed == id && *value == off)
}

/// Whether the fall back is armed for this selector (`id`) and its off
/// value (PR #146 review I2, refined by the PR #147 ruling on I2): both
/// the list the script last kept and the newest one sent hold it, so
/// whichever the script holds now (it swaps lists whole, only on its OK)
/// has its DRY write. A registration that only adds a target takes no
/// selector DRY; one that drops a selector's write does.
pub(super) fn armed(safe: Option<&Safe>, id: &str, off: u8) -> bool {
    safe.is_some_and(|safe| {
        safe.kept
            .as_deref()
            .is_some_and(|kept| covers(kept, id, off))
            && covers(&safe.list, id, off)
    })
}

/// A singer's off by hand as a page sends it: `{"id": "<singer>", "off":
/// bool}`, nothing more.
pub(super) fn singer_value(value: &Value) -> Option<(String, bool)> {
    let fields = value.as_object().filter(|fields| fields.len() == 2)?;
    let id = fields.get("id")?.as_str()?;
    let off = fields.get("off")?.as_bool()?;
    Some((id.to_string(), off))
}

/// Whether a key is being checked (the surface's `checking`): AUT on, Live
/// playing in a keyed song, and a target's key still on its way.
pub(super) fn checking(
    aut: bool,
    playing: Option<bool>,
    key: Option<Key>,
    mut checks: impl Iterator<Item = KeyCheck>,
) -> bool {
    aut && playing == Some(true) && key.is_some() && checks.any(|check| check == KeyCheck::Pending)
}

/// A singer's track on the surface: none without one, else its selector on
/// the chain (`tune`) or not.
pub(super) fn track_state(state: Option<TargetState>) -> TrackState {
    match state {
        None => TrackState::None,
        Some(TargetState::On) => TrackState::Tune,
        Some(_) => TrackState::Dry,
    }
}

/// A target's part of its singer: the PA track (`+AUTOTUNE:0`, on at 0) or
/// the stream track.
pub(super) fn part(on: u8) -> &'static str {
    if on == 0 { "PA" } else { "stream" }
}

/// A singer's label: its first track's label that is a marker's own (not
/// `#<index + 1>`), else its first track's.
pub(super) fn singer_label(labels: &[&str]) -> String {
    labels
        .iter()
        .find(|label| !label.starts_with('#'))
        .or_else(|| labels.first())
        .map_or_else(String::new, |label| (*label).to_string())
}

/// Why a track's key is wrong, in words (`stream Key reads G, wrote F`):
/// Live refused it, or its values show another key.
pub(super) fn key_why(part: &str, state: KeyState, want: Option<Key>, seen: Option<Key>) -> String {
    let name = |key: Option<Key>| key.map_or_else(|| "no key".to_string(), |key| key.name());
    match state {
        KeyState::Refused => format!("{part} Key refused, wrote {}", name(want)),
        _ => format!("{part} Key reads {}, wrote {}", name(seen), name(want)),
    }
}

/// Why a target names no singer (PR #147 review N6): it has no manual
/// off, so no autotune for it.
pub const NO_SINGER: &str = "its Tuner names no singer: no +ID, no +MUTE, no label of its own";

/// Whether a target names no singer: no `+ID`, no `+MUTE` and a placeholder
/// label (two `+MUTE`s are their own problem).
fn names_no_singer(target: &Target) -> bool {
    !target.spec.two_singers && singer_of(&target.spec).is_none()
}

/// The words of Live's refusal of a selector's write once the hub stopped
/// writing it (PR #147 review I5).
fn refusal_words(target: &Target) -> Option<String> {
    let refusal = target.refusal.as_ref()?;
    let id = &target.found.as_ref()?.id;
    refusal
        .stops(id, refusal.on)
        .then(|| format!("Live refuses its selector: {}", refusal.why))
}

/// A target problem's words for the surface: its rack's or walk's (a
/// missing rack too: PR #147 review N3), its Tuner naming two singers or
/// none, its key device's, Live refusing its selector, another writer on
/// it.
pub(super) fn trouble(target: &Target) -> Option<String> {
    if let Some(problem) = &target.problem {
        return Some(problem.clone());
    }
    if target.spec.two_singers {
        return Some("its Tuner names two singers".to_string());
    }
    if names_no_singer(target) {
        return Some(NO_SINGER.to_string());
    }
    if let Some(Err(missing)) = &target.key {
        return Some(missing.clone());
    }
    if let Some(words) = refusal_words(target) {
        return Some(words);
    }
    target
        .guard
        .another()
        .then(|| "another writer moves its selector".to_string())
}

/// Whether a target has a problem its decision reads (spec §7): its walks
/// given up, its Tuner naming two singers or none (PR #147 review N6), no
/// key device.
pub(super) fn has_problem(target: &Target) -> bool {
    target.problem.is_some()
        || target.spec.two_singers
        || names_no_singer(target)
        || matches!(target.key, Some(Err(_)))
}

/// Whether a target counts for the pages' `checking` (PR #147 review N3):
/// found, with a key device, and no problem; a missing one is a problem,
/// never a key being checked.
pub(super) fn checked(target: &Target) -> bool {
    target.found.is_some() && matches!(target.key, Some(Ok(_))) && !has_problem(target)
}

impl AutotuneIo {
    /// A target's key check against its set's key in force.
    pub(super) fn check_of(&self, target: &Target) -> KeyCheck {
        let in_force = self
            .positions
            .get(&target.instance)
            .and_then(|p| p.follow.now().1);
        key_check(
            matches!(target.key, Some(Ok(_))),
            target.keyt.wanted(),
            target.keyt.state(),
            in_force,
        )
    }

    /// Every singer in Live's order of their first track: its tracks, its
    /// off by hand, and whether its tracks' keys and targets are fine. A
    /// target that names no singer is one of its own, with no id (PR #147
    /// review N6: no manual off).
    pub(super) fn singers(&self) -> Vec<SingerState> {
        let mut groups: Vec<(Option<String>, Vec<&Target>)> = Vec::new();
        for target in self.ordered() {
            let id = singer_of(&target.spec);
            let at = id.as_ref().and_then(|id| {
                groups
                    .iter()
                    .position(|(seen, _)| seen.as_ref() == Some(id))
            });
            match at {
                Some(at) => groups[at].1.push(target),
                None => groups.push((id, vec![target])),
            }
        }
        groups
            .into_iter()
            .map(|(id, mine)| {
                let labels: Vec<&str> = mine.iter().map(|t| t.spec.label.as_str()).collect();
                let of = |pa: bool| {
                    track_state(
                        mine.iter()
                            .find(|t| (t.spec.on == 0) == pa)
                            .map(|t| state_of(t)),
                    )
                };
                let why = mine.iter().find_map(|t| self.why_of(t));
                SingerState {
                    label: singer_label(&labels),
                    stream: of(false),
                    pa: of(true),
                    off: id.as_deref().is_some_and(|id| self.rule.is_off(id)),
                    key_ok: why.is_none(),
                    why,
                    id: id.unwrap_or_default(),
                }
            })
            .collect()
    }

    /// Why a target is not fine: its problem (a missing rack too: PR #147
    /// review N3), else a found one's key's.
    fn why_of(&self, target: &Target) -> Option<String> {
        let part = part(target.spec.on);
        if let Some(trouble) = trouble(target) {
            return Some(format!("{part}: {trouble}"));
        }
        target.found.as_ref()?;
        let seen = match target.values {
            (Some(key), Some(scale)) => key_shows(key, scale),
            _ => None,
        };
        (self.check_of(target) == KeyCheck::Bad)
            .then(|| key_why(part, target.keyt.state(), target.keyt.wanted(), seen))
    }

    /// A found target's input to its decision (`held`: its set's writes
    /// held, or its markers not settled; an unconfirmed target judges as
    /// held: PR #147 review I4).
    fn input(&self, target: &Target, held: bool) -> Option<TrackInput> {
        let found = target.found.as_ref()?;
        let in_force = self
            .positions
            .get(&target.instance)
            .map_or((None, None), |p: &Position| p.follow.now());
        Some(TrackInput {
            aut: self.rule.is_on(),
            held: held || !target.sure,
            playing: self.rule.playing(&target.instance),
            in_force,
            singer_off: singer_of(&target.spec).is_some_and(|id| self.rule.is_off(&id)),
            problem: has_problem(target),
            another_writer: target.guard.another(),
            armed: armed(self.safe.get(&target.instance), &found.id, target.spec.off),
            key: self.check_of(target),
            selected: target.decision.is_some_and(|d| d.on),
        })
    }
}

impl Router {
    /// Decides every found target of `set` (`cause`: what changed) and
    /// writes the selectors that need it in one batch. While the set's
    /// writes are held nothing goes (the release decides again); a write
    /// that meets its markers unsettled holds the set, DRY first.
    pub(in crate::router) fn autotune_decide(&mut self, set: &str, cause: Cause) {
        let holding = self.autotune.held.contains(set);
        let settled = self.reverb_settled(set);
        let going = self.autotune_judge(set, cause, !may_write(holding, settled));
        if going.is_empty() || holding {
            return;
        }
        if !settled {
            self.autotune_hold(set, "markers");
            return;
        }
        self.autotune_select(set, cause, going, false);
    }

    /// Each found target of `set`'s decision (`held`: as while its writes
    /// are held), kept; one `select` record per switch; the selectors to
    /// write (none for a target not confirmed since a hold: PR #147 review
    /// I4). A refusal Live gave its selector ends with a new decision or
    /// `$ref`; while it stops the write, a refused chain is DRY with a
    /// problem and a refused DRY is not written again (PR #147 review I5).
    pub(super) fn autotune_judge(&mut self, set: &str, cause: Cause, held: bool) -> Vec<Going> {
        let mut going = Vec::new();
        for key in self.autotune.keys_in_order(set) {
            let Some(input) = self
                .autotune
                .targets
                .get(&key)
                .and_then(|t| self.autotune.input(t, held))
            else {
                continue;
            };
            let rule = decide(&input);
            let Some(target) = self.autotune.targets.get_mut(&key) else {
                continue;
            };
            let Some(id) = target.found.as_ref().map(|f| f.id.clone()) else {
                continue;
            };
            if target
                .refusal
                .as_ref()
                .is_some_and(|refusal| !refusal.holds(&id, rule.on))
            {
                target.refusal = None;
            }
            let decision = refused_decision(rule, target.refusal.as_ref(), &id);
            let was = target.decision.replace(decision);
            let stopped = target.guard.given_up()
                || target
                    .refusal
                    .as_ref()
                    .is_some_and(|refusal| refusal.stops(&id, decision.on));
            // Never before its first value is heard (that value decides),
            // nor while it is not confirmed since a hold.
            let write = target.sure
                && target.value.is_some()
                && needs_write(decision.on, target.shows(), target.writing_now(), stopped);
            let switched = was.is_some_and(|w| w.on) != decision.on;
            let (instance, track, label) = (
                target.instance.clone(),
                target.track.clone(),
                target.spec.label.clone(),
            );
            let value = super::macro_value(&target.spec, decision.on);
            if switched {
                self.autotune_step(
                    Step::Select {
                        instance,
                        target: track,
                        on: decision.on,
                        why: decision.why,
                        cause,
                    },
                    None,
                );
            }
            if write {
                going.push(Going {
                    key,
                    id,
                    label,
                    on: decision.on,
                    value,
                });
            }
        }
        going
    }

    /// Writes `going`'s selectors of `set` in one batch through their
    /// `$ref`s, recorded first (`write`); each write (numbered, its own
    /// counter) is on its way until Live shows it or its answer says it
    /// failed; the answer comes back as [`RouterMsg::AutotuneWritten`].
    /// `retire`: the DRY writes of targets let go, whose set's registration
    /// waits for this answer (PR #147 review I3). Nothing to write: no
    /// record, no call.
    pub(super) fn autotune_select(
        &mut self,
        set: &str,
        cause: Cause,
        going: Vec<Going>,
        retire: bool,
    ) {
        if going.is_empty() {
            return;
        }
        let Some(live) = self.live.get(set).cloned() else {
            return;
        };
        self.autotune.selects += 1;
        let write = self.autotune.selects;
        for g in &going {
            if let Some(target) = self.autotune.targets.get_mut(&g.key) {
                target.writing.push(Flying {
                    write,
                    on: g.on,
                    answered: false,
                });
            }
        }
        if retire {
            self.autotune
                .retiring
                .entry(set.to_string())
                .or_default()
                .insert(write);
        }
        let labels = |on: bool| -> Vec<String> {
            going
                .iter()
                .filter(|g| g.on == on)
                .map(|g| g.label.clone())
                .collect()
        };
        self.autotune_step(
            Step::Write {
                instance: set.to_string(),
                cause,
                on: labels(true),
                off: labels(false),
            },
            None,
        );
        self.autotune.rule.wrote();
        let pairs: Vec<(&str, u8)> = going.iter().map(|g| (g.id.as_str(), g.value)).collect();
        let result = live.call(write_commands(&pairs));
        let selectors: Vec<(String, String, bool)> = going
            .iter()
            .map(|g| (g.key.clone(), g.id.clone(), g.on))
            .collect();
        let (instance, tx) = (set.to_string(), self.io.tx.clone());
        tokio::spawn(async move {
            let outcome = result.await.map_err(|e| e.to_string());
            let _ = tx.send(RouterMsg::AutotuneWritten {
                instance,
                write,
                selectors,
                outcome,
            });
        });
    }

    /// A target's selector value heard (`was` the one before, `flying` the
    /// values of the hub's writes that were on its way): its first value
    /// decides its set; a move another writer made goes to the guard.
    /// Nothing while the set is held or unsettled, or the target is not
    /// confirmed since a hold: its values may be another track's by the
    /// index path (the release, or its walk, decides again).
    pub(super) fn autotune_heard(&mut self, key: &str, was: Option<f64>, flying: &[bool]) {
        let Some(target) = self.autotune.targets.get(key) else {
            return;
        };
        let instance = target.instance.clone();
        let holding = self.autotune.held.contains(&instance);
        if !may_write(holding, self.reverb_settled(&instance)) || !target.sure {
            return;
        }
        let Some(decision) = target.decision else {
            return;
        };
        let Some(was) = was else {
            // Its first value: decided with what Live shows.
            self.autotune_decide(&instance, Cause::Found);
            return;
        };
        let before = shows(Some(was), target.spec.on, target.spec.off);
        let track = target.track.clone();
        match another_move(before, target.shows(), decision.on, flying) {
            Some(true) => self.autotune_moved_on(key),
            Some(false) => {
                if let Some(target) = self.autotune.targets.get_mut(key) {
                    target.guard.left();
                }
                self.autotune_step(
                    Step::SelectorWriter {
                        instance: instance.clone(),
                        target: track,
                        act: WriterAct::LeftDry,
                    },
                    None,
                );
                self.autotune_decide(&instance, Cause::Guard);
            }
            // No other writer's move: the set is decided again with what
            // Live shows now, so a selector a write's value left off its
            // decision is written (DRY wins, PR #147 review I1).
            None => self.autotune_decide(&instance, Cause::Found),
        }
    }

    /// Another writer moved a target's selector off DRY while the hub wants
    /// DRY: DRY again once, or, a second time within 10 s, the hub stops
    /// writing it (its decision `another_writer`).
    fn autotune_moved_on(&mut self, key: &str) {
        let now_ms = self.now_ms();
        let Some(target) = self.autotune.targets.get_mut(key) else {
            return;
        };
        let act = target.guard.moved(now_ms);
        let (instance, track) = (target.instance.clone(), target.track.clone());
        let dry = target.found.as_ref().map(|found| Going {
            key: key.to_string(),
            id: found.id.clone(),
            label: target.spec.label.clone(),
            on: false,
            value: target.spec.off,
        });
        let (act, going) = match act {
            GuardAct::WriteDryOnce => (WriterAct::DryAgain, dry.into_iter().collect()),
            GuardAct::GiveUp => (WriterAct::GaveUp, Vec::new()),
        };
        self.autotune_step(
            Step::SelectorWriter {
                instance: instance.clone(),
                target: track,
                act,
            },
            None,
        );
        self.autotune_select(&instance, Cause::Guard, going, false);
        self.autotune_decide(&instance, Cause::Guard);
    }
}

#[cfg(test)]
#[path = "switch_tests.rs"]
mod tests;
