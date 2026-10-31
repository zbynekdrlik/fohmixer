//! STAGE AUT (spec §2.4, F15, X9): a rule the hub runs on Live's values
//! (REVERB's, #130, is `rules/reverb.rs`; AUTOTUNE's, #104, is
//! `rules/autotune.rs`). While the flag is on, the
//! stage-mic track follows the transport: playing → muted, stopped → live.
//! The flag is a hub value shared by every client and kept in
//! `<data>/hub-state.json`, so the rule keeps working while every iPad
//! sleeps.
//!
//! The rule writes once per change of `is_playing` (never twice for one
//! change, and never fighting a hand-set mute in between), once when the
//! flag is turned on, and once after the instance reconnects (a set load):
//! the rule then applies to the fresh state. A `+CTRL` track that only moves
//! in its Live set is the same track: the rule keeps its state, no write
//! (#104, [`stage_change`]), unless its last write went out while the track
//! places were unsure and so may have hit the old index
//! ([`StageAut::on_moved`]).

use std::io;
use std::path::{Path, PathBuf};

use fohmixer_proto::layout::{Binding, CtrlId};
use serde::{Deserialize, Serialize};

#[path = "rules/autotune.rs"]
pub mod autotune;
#[path = "rules/links.rs"]
pub mod links;
#[path = "rules/lufs.rs"]
pub mod lufs;
#[path = "rules/reverb.rs"]
pub mod reverb;

/// The file of the persisted hub values.
pub const HUB_STATE_FILE: &str = "hub-state.json";

/// The mute the rule wants for this transport state; `None` while the flag
/// is off.
pub fn stage_rule(flag: bool, is_playing: bool) -> Option<bool> {
    flag.then_some(is_playing)
}

/// Where STAGE AUT acts: the stage-mic track's mute (its instance's
/// transport drives it, `Router::transport`), and the `+CTRL` id of the
/// frame's control when it names one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageTarget {
    pub instance: String,
    pub target: String,
    pub ctrl: Option<CtrlId>,
}

impl StageTarget {
    /// The served binding's target; none when it has none (no Tuner yet).
    pub fn of(binding: &Binding, ctrl: Option<CtrlId>) -> Option<Self> {
        Some(Self {
            instance: binding.instance.clone(),
            target: binding.target().ok()?,
            ctrl,
        })
    }
}

/// What a new layout's STAGE AUT target means for the rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageChange {
    /// The same target: nothing to do.
    Same,
    /// The same `+CTRL` control in the same Live set at another index: the
    /// target changes, the rule keeps its state and writes nothing but
    /// [`StageAut::on_moved`]. A track inserted or deleted above it, or the
    /// tag moved to another track in one composition: [`stage_change`]
    /// cannot tell them apart. In the second case, during a song, the new
    /// track stays open until the next transport change and the old one
    /// stays muted; accepted, nobody moves this tag during a song.
    Moved,
    /// Another instance or control, a new binding or a lost one: the rule
    /// starts afresh.
    Afresh,
}

/// How `wanted` follows `current` (#104). A frame's track by name is never
/// `Moved`: its target changes only when the frame or the name does.
pub fn stage_change(current: Option<&StageTarget>, wanted: Option<&StageTarget>) -> StageChange {
    let (Some(now), Some(next)) = (current, wanted) else {
        return if current.is_none() && wanted.is_none() {
            StageChange::Same
        } else {
            StageChange::Afresh
        };
    };
    if now.instance != next.instance {
        StageChange::Afresh
    } else if now.target == next.target {
        StageChange::Same
    } else if now.ctrl.is_some() && now.ctrl == next.ctrl {
        StageChange::Moved
    } else {
        StageChange::Afresh
    }
}

/// The rule's state between events.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StageAut {
    on: bool,
    /// The latest `is_playing` of this Live session.
    playing: Option<bool>,
    /// The mute this rule last wrote in this Live session.
    written: Option<bool>,
    /// Writes made since the hub started.
    writes: u64,
    /// The last write went out with the track places unsure (a track-list
    /// read or a composition under way, a reconnect): it may have hit the
    /// old index.
    unsure: bool,
}

impl StageAut {
    /// The rule with the persisted flag.
    pub fn new(on: bool) -> Self {
        Self {
            on,
            ..Self::default()
        }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    pub fn writes(&self) -> u64 {
        self.writes
    }

    /// A new `is_playing` value: the mute to write, if any.
    pub fn on_playing(&mut self, playing: bool) -> Option<bool> {
        self.playing = Some(playing);
        self.decide()
    }

    /// The flag changed: turning it on applies the rule at once.
    pub fn set_flag(&mut self, on: bool) -> Option<bool> {
        if on && !self.on {
            self.written = None;
        }
        self.on = on;
        self.decide()
    }

    /// The instance went away: its next session starts fresh.
    pub fn on_disconnect(&mut self) {
        self.playing = None;
        self.written = None;
        self.unsure = false;
    }

    /// A write goes out, with the track places `unsure` or known.
    pub fn note_write(&mut self, unsure: bool) {
        self.unsure = unsure;
    }

    /// The track places are known and nothing moved: the last write hit
    /// its track.
    pub fn places_known(&mut self) {
        self.unsure = false;
    }

    /// The target moved: nothing to write, so a mute set by hand stays,
    /// unless the last write may have hit the old index. Then that mute,
    /// for the moved track. A read that gave up keeps the note (the places
    /// stayed unknown): a later move rewriting the rule's mute is accepted.
    pub fn on_moved(&mut self) -> Option<bool> {
        if !std::mem::take(&mut self.unsure) || !self.on {
            return None;
        }
        let mute = self.written?;
        self.writes += 1;
        Some(mute)
    }

    fn decide(&mut self) -> Option<bool> {
        let mute = stage_rule(self.on, self.playing?)?;
        if self.written == Some(mute) {
            return None;
        }
        self.written = Some(mute);
        self.writes += 1;
        Some(mute)
    }
}

/// The persisted hub values. The router keeps them whole and saves them
/// whole (#130: a save of one value keeps the others).
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct HubState {
    #[serde(default)]
    pub stage_aut: bool,
    /// REVERB (#130, D18): `reverb_aut` and `reverb_low` (what the rule
    /// holds lowered, by fader identity).
    #[serde(flatten)]
    pub reverb: reverb::Saved,
    /// The fader links' switches a page set (#130, D18).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fader_links: Vec<links::SavedLink>,
    /// AUTOTUNE AUT (#104, D19); written only while on, kept across a
    /// restart since #140 PR 3b.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub autotune_aut: bool,
    /// The singers off by hand (#140 PR 3b, spec §2.2), sorted; written
    /// only while one is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub autotune_off: Vec<String>,
}

impl HubState {
    /// `<data>/hub-state.json`; missing is the defaults. A file that does not
    /// parse is logged and read as the defaults (the rules off) — never a
    /// reason to leave the mixer down — and is not overwritten until a hub
    /// value changes (STAGE AUT, a fader link's switch, REVERB AUT or what
    /// REVERB holds lowered, AUTOTUNE AUT, a singer's manual off; a layout
    /// alone saves nothing; #130, #104, #140).
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join(HUB_STATE_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                tracing::error!(path = %path.display(), error = %e, "unreadable hub state: the hub's rules start off");
                Self::default()
            }),
            Err(e) => {
                if let Some(problem) = read_problem(&e) {
                    tracing::error!(path = %path.display(), error = %problem, "unreadable hub state: the hub's rules start off");
                }
                Self::default()
            }
        }
    }

    /// Writes the file atomically (a temporary file, then a rename).
    pub fn save(&self, data_dir: &Path) -> io::Result<()> {
        let path = data_dir.join(HUB_STATE_FILE);
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        crate::atomic_write(&path, &json)
    }
}

/// Why reading the hub state failed, when it is a problem: a missing file is
/// just the defaults (a first start).
fn read_problem(e: &io::Error) -> Option<String> {
    (e.kind() != io::ErrorKind::NotFound).then(|| e.to_string())
}

/// Where the hub values live.
pub fn state_path(data_dir: &Path) -> PathBuf {
    data_dir.join(HUB_STATE_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::layout::Anchor;

    #[test]
    fn only_a_missing_state_file_is_no_problem() {
        assert_eq!(
            read_problem(&io::Error::from(io::ErrorKind::NotFound)),
            None
        );
        assert_eq!(
            read_problem(&io::Error::other("disk on fire")),
            Some("disk on fire".to_string())
        );
    }

    #[test]
    fn the_rule_mutes_while_playing_and_only_with_the_flag() {
        assert_eq!(stage_rule(true, true), Some(true));
        assert_eq!(stage_rule(true, false), Some(false));
        assert_eq!(stage_rule(false, true), None);
        assert_eq!(stage_rule(false, false), None);
    }

    #[test]
    fn a_move_writes_only_what_may_have_hit_the_old_index() {
        let mut rule = StageAut::new(true);
        assert_eq!(rule.on_playing(true), Some(true));
        assert_eq!(rule.on_moved(), None, "a sure write: a hand's mute stays");
        rule.note_write(true);
        assert_eq!(rule.on_moved(), Some(true));
        assert_eq!(rule.writes(), 2);
        assert_eq!(rule.on_moved(), None, "once");
        rule.note_write(true);
        rule.places_known();
        assert_eq!(rule.on_moved(), None, "it hit its track");
        rule.note_write(true);
        rule.note_write(false);
        assert_eq!(rule.on_moved(), None, "the last write counts");
        rule.note_write(true);
        rule.on_disconnect();
        assert_eq!(rule.on_moved(), None, "a new session");
        assert_eq!(rule.on_playing(false), Some(false));
        rule.note_write(true);
        assert_eq!(rule.set_flag(false), None);
        assert_eq!(rule.on_moved(), None, "AUT off");
        assert_eq!(rule.writes(), 3);
        let mut rule = StageAut::new(true);
        rule.note_write(true);
        assert_eq!(rule.on_moved(), None, "nothing written yet");
        assert_eq!(rule.writes(), 0);
    }

    #[test]
    fn a_stage_target_that_only_changes_index_has_moved() {
        let at = |instance: &str, index: u32, ctrl: Option<CtrlId>| StageTarget {
            instance: instance.into(),
            target: format!("live_set tracks {index}"),
            ctrl,
        };
        let stage = Some(CtrlId::Stage);
        let change = |a: &StageTarget, b: &StageTarget| stage_change(Some(a), Some(b));
        assert_eq!(stage_change(None, None), StageChange::Same);
        let filled = at("band", 3, stage);
        assert_eq!(stage_change(None, Some(&filled)), StageChange::Afresh);
        assert_eq!(stage_change(Some(&filled), None), StageChange::Afresh);
        assert_eq!(change(&filled, &filled), StageChange::Same);
        assert_eq!(
            change(&filled, &at("band", 3, None)),
            StageChange::Same,
            "the same track"
        );
        assert_eq!(change(&filled, &at("band", 2, stage)), StageChange::Moved);
        assert_eq!(
            change(&filled, &at("master", 2, stage)),
            StageChange::Afresh
        );
        assert_eq!(
            change(&filled, &at("master", 3, stage)),
            StageChange::Afresh
        );
        let other = Some(CtrlId::Talkback);
        assert_eq!(change(&filled, &at("band", 2, other)), StageChange::Afresh);
        assert_eq!(change(&filled, &at("band", 2, None)), StageChange::Afresh);
        // A frame's track by name is not a moved control.
        let by_name = at("band", 2, None);
        assert_eq!(change(&by_name, &at("band", 3, stage)), StageChange::Afresh);
        assert_eq!(change(&by_name, &at("band", 3, None)), StageChange::Afresh);
    }

    #[test]
    fn a_stage_target_needs_a_filled_binding() {
        let binding = |anchor: Anchor| Binding {
            instance: "band".into(),
            anchor,
            path: None,
        };
        assert_eq!(
            StageTarget::of(&binding(Anchor::TrackAt { index: 4 }), Some(CtrlId::Stage)),
            Some(StageTarget {
                instance: "band".into(),
                target: "live_set tracks 4".into(),
                ctrl: Some(CtrlId::Stage),
            })
        );
        let id = CtrlId::Stage;
        assert_eq!(
            StageTarget::of(&binding(Anchor::Ctrl { id }), Some(id)),
            None
        );
    }

    #[test]
    fn it_writes_once_per_transport_change() {
        let mut rule = StageAut::new(true);
        assert!(rule.is_on());
        assert_eq!(
            rule.on_playing(false),
            Some(false),
            "the first state applies"
        );
        assert_eq!(rule.on_playing(false), None, "no change, no write");
        assert_eq!(rule.on_playing(true), Some(true));
        assert_eq!(rule.on_playing(true), None);
        assert_eq!(rule.on_playing(false), Some(false));
        assert_eq!(rule.writes(), 3);
    }

    #[test]
    fn with_the_flag_off_it_never_writes() {
        let mut rule = StageAut::new(false);
        assert_eq!(rule.on_playing(true), None);
        assert_eq!(rule.on_playing(false), None);
        assert_eq!(rule.writes(), 0);
        assert!(!rule.is_on());
    }

    #[test]
    fn turning_the_flag_on_applies_the_rule_at_once() {
        let mut rule = StageAut::new(false);
        assert_eq!(rule.set_flag(true), None, "no transport state yet");
        assert_eq!(rule.on_playing(true), Some(true));
        assert_eq!(rule.set_flag(false), None);
        assert_eq!(
            rule.on_playing(false),
            None,
            "off: the transport is not followed"
        );
        assert_eq!(
            rule.set_flag(true),
            Some(false),
            "on again: applied at once"
        );
        assert_eq!(rule.set_flag(true), None, "already on");
        assert_eq!(rule.writes(), 2);
    }

    #[test]
    fn a_reconnect_applies_the_rule_once_to_the_fresh_state() {
        let mut rule = StageAut::new(true);
        assert_eq!(rule.on_playing(true), Some(true));
        rule.on_disconnect();
        assert_eq!(rule.set_flag(true), None, "no state while disconnected");
        assert_eq!(
            rule.on_playing(true),
            Some(true),
            "one write after reconnect"
        );
        assert_eq!(rule.on_playing(true), None);
        assert_eq!(rule.writes(), 2);
    }

    #[test]
    fn the_state_file_round_trips_and_a_bad_one_starts_off() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(HubState::load(dir.path()), HubState::default());
        let state = HubState {
            stage_aut: true,
            reverb: reverb::Saved {
                aut: true,
                low: Some(reverb::SavedLow {
                    by: fohmixer_proto::client::Low::Stop,
                    faders: vec![reverb::SavedFader {
                        instance: "band".into(),
                        label: "Rev ST".into(),
                        back: 0.85,
                        left: Some(0.6),
                    }],
                }),
            },
            fader_links: vec![links::SavedLink {
                instance: "band".into(),
                id: "REV".into(),
                on: false,
            }],
            autotune_aut: true,
            autotune_off: vec!["#Vox 7".into(), "VOX1".into()],
        };
        state.save(dir.path()).unwrap();
        assert_eq!(HubState::load(dir.path()), state);
        // AUT and the singers off by hand are kept across a restart (#140
        // PR 3b).
        let saved: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(state_path(dir.path())).unwrap())
                .unwrap();
        assert_eq!(saved["autotune_aut"], serde_json::json!(true));
        assert_eq!(saved["autotune_off"], serde_json::json!(["#Vox 7", "VOX1"]));
        // An older file (STAGE AUT only) reads with REVERB's defaults, and
        // the defaults write no key of REVERB's.
        std::fs::write(state_path(dir.path()), r#"{"stage_aut": true}"#).unwrap();
        assert_eq!(HubState::load(dir.path()).reverb, reverb::Saved::default());
        assert_eq!(HubState::load(dir.path()).fader_links, vec![]);
        assert!(!HubState::load(dir.path()).autotune_aut, "AUTOTUNE AUT off");
        assert!(HubState::load(dir.path()).autotune_off.is_empty());
        HubState {
            stage_aut: false,
            reverb: reverb::Saved::default(),
            fader_links: Vec::new(),
            autotune_aut: false,
            autotune_off: Vec::new(),
        }
        .save(dir.path())
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(state_path(dir.path()))
                .unwrap()
                .parse::<serde_json::Value>()
                .unwrap(),
            serde_json::json!({"stage_aut": false})
        );
        // AUTOTUNE AUT on is written beside STAGE AUT (#104).
        HubState {
            stage_aut: false,
            reverb: reverb::Saved::default(),
            fader_links: Vec::new(),
            autotune_aut: true,
            autotune_off: Vec::new(),
        }
        .save(dir.path())
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(state_path(dir.path()))
                .unwrap()
                .parse::<serde_json::Value>()
                .unwrap(),
            serde_json::json!({"stage_aut": false, "autotune_aut": true})
        );
        assert!(HubState::load(dir.path()).autotune_aut);
        HubState {
            stage_aut: true,
            reverb: reverb::Saved::default(),
            fader_links: Vec::new(),
            autotune_aut: false,
            autotune_off: Vec::new(),
        }
        .save(dir.path())
        .unwrap();
        assert_eq!(state_path(dir.path()), dir.path().join(HUB_STATE_FILE));
        assert!(HubState::load(dir.path()).stage_aut);
        std::fs::write(state_path(dir.path()), "{not json").unwrap();
        assert!(!HubState::load(dir.path()).stage_aut);
        assert_eq!(
            std::fs::read_to_string(state_path(dir.path())).unwrap(),
            "{not json",
            "a bad file is left for inspection"
        );
        std::fs::remove_file(state_path(dir.path())).unwrap();
        std::fs::create_dir(state_path(dir.path())).unwrap();
        assert!(!HubState::load(dir.path()).stage_aut, "unreadable: off");
        assert!(
            HubState {
                stage_aut: true,
                reverb: reverb::Saved::default(),
                fader_links: Vec::new(),
                autotune_aut: false,
                autotune_off: Vec::new(),
            }
            .save(dir.path())
            .is_err()
        );
    }
}
