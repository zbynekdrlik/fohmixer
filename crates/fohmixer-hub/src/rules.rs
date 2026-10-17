//! STAGE AUT (spec §2.4, F15, X9): a rule the hub runs on Live's values
//! (REVERB's, #130, is `rules/reverb.rs`). While the flag is on, the
//! stage-mic track follows the transport: playing → muted, stopped → live.
//! The flag is a hub value shared by every client and kept in
//! `<data>/hub-state.json`, so the rule keeps working while every iPad
//! sleeps.
//!
//! The rule writes once per change of `is_playing` (never twice for one
//! change, and never fighting a hand-set mute in between), once when the
//! flag is turned on, and once after the instance reconnects (a set load):
//! the rule then applies to the fresh state.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[path = "rules/links.rs"]
pub mod links;
#[path = "rules/reverb.rs"]
pub mod reverb;

/// The file of the persisted hub values.
pub const HUB_STATE_FILE: &str = "hub-state.json";

/// The mute the rule wants for this transport state; `None` while the flag
/// is off.
pub fn stage_rule(flag: bool, is_playing: bool) -> Option<bool> {
    flag.then_some(is_playing)
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
    /// REVERB (#130, D18): `reverb_aut` and, from PR 4, `reverb_low`.
    #[serde(flatten)]
    pub reverb: reverb::Saved,
    /// The fader links' switches a page set (#130, D18).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fader_links: Vec<links::SavedLink>,
}

impl HubState {
    /// `<data>/hub-state.json`; missing is the defaults. A file that does not
    /// parse is logged and read as the defaults (the rules off) — never a
    /// reason to leave the mixer down — and is not overwritten until the
    /// flag changes.
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
        };
        state.save(dir.path()).unwrap();
        assert_eq!(HubState::load(dir.path()), state);
        // An older file (STAGE AUT only) reads with REVERB's defaults, and
        // the defaults write no key of REVERB's.
        std::fs::write(state_path(dir.path()), r#"{"stage_aut": true}"#).unwrap();
        assert_eq!(HubState::load(dir.path()).reverb, reverb::Saved::default());
        assert_eq!(HubState::load(dir.path()).fader_links, vec![]);
        HubState {
            stage_aut: false,
            reverb: reverb::Saved::default(),
            fader_links: Vec::new(),
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
        HubState {
            stage_aut: true,
            reverb: reverb::Saved::default(),
            fader_links: Vec::new(),
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
            }
            .save(dir.path())
            .is_err()
        );
    }
}
