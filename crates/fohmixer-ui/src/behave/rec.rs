//! The recording watch on the surface (#172; the approved mockup
//! `docs/mockups/rec-watch-v3.html` with the owner's later correction,
//! smaller and calmer): what a strip's ARM key and the column foot's REC key
//! show, and what their 1 s hold (`behave::rail::rail_press`) acts on. The
//! hub's `rec` message says it all: the page follows no `arm` of its own, so
//! a strip without an armable track (a group, a return, a strip of another
//! Live) has no key and no listener.

use fohmixer_proto::client::{REC_INSTANCE, RecState, RecTrack, RecWatch};
use fohmixer_proto::layout::{Anchor, Strip, StripKind};

/// What a strip's ARM key shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmLook {
    /// No key: the strip's track cannot be armed.
    None,
    /// A faint ring.
    Off,
    /// A steady muted dot.
    Armed,
    /// The dot's slow pulse: armed, the record button on, Live playing.
    Recording,
}

impl ArmLook {
    /// Its `data-state`.
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Off => "off",
            Self::Armed => "armed",
            Self::Recording => "recording",
        }
    }
}

/// The armable track `strip` is bound to, as the hub told it: a standard
/// strip of the watched Live, by the index or the name its anchor names.
pub fn strip_track<'a>(state: &'a RecState, strip: &Strip) -> Option<&'a RecTrack> {
    if strip.strip_kind != StripKind::Standard || strip.binding.instance != REC_INSTANCE {
        return None;
    }
    state.tracks.iter().find(|t| match &strip.binding.anchor {
        Anchor::TrackAt { index } => t.index == *index,
        Anchor::Track { name } => t.name == *name,
        _ => false,
    })
}

/// What `strip`'s ARM key shows by the hub's state.
pub fn arm_look(state: Option<&RecState>, strip: &Strip) -> ArmLook {
    let Some(state) = state else {
        return ArmLook::None;
    };
    match strip_track(state, strip) {
        None => ArmLook::None,
        Some(track) if !track.arm => ArmLook::Off,
        Some(_) if state.watch() == RecWatch::Recording => ArmLook::Recording,
        Some(_) => ArmLook::Armed,
    }
}

/// `strip`'s track's `arm` a hold turns the other way (none: it cannot).
pub fn strip_arm(state: Option<&RecState>, strip: &Strip) -> Option<bool> {
    strip_track(state?, strip).map(|t| t.arm)
}

/// The record button a hold of the foot's key turns the other way.
pub fn record_mode(state: Option<&RecState>) -> Option<bool> {
    state?.record_mode
}

/// What the foot's key shows: its `data-state` and its word.
pub fn foot_look(state: Option<&RecState>) -> (&'static str, &'static str) {
    let watch = state.map_or(RecWatch::Off, RecState::watch);
    let word = match watch {
        RecWatch::NotRec => "NOT REC",
        RecWatch::NoArm => "NO ARM",
        RecWatch::Off | RecWatch::Ready | RecWatch::Recording => "REC",
    };
    (watch.name(), word)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    fn strip(instance: &str, anchor: Value, kind: &str) -> Strip {
        serde_json::from_value(json!({
            "binding": {"instance": instance, "anchor": anchor},
            "strip_kind": kind,
        }))
        .unwrap()
    }

    fn at(index: u32) -> Strip {
        strip(
            "band",
            json!({"kind": "track_at", "index": index}),
            "standard",
        )
    }

    fn state(playing: bool, record: bool, arm: bool) -> RecState {
        RecState {
            playing: Some(playing),
            record_mode: Some(record),
            tracks: vec![
                RecTrack {
                    index: 1,
                    name: "Vox 1".into(),
                    arm,
                },
                RecTrack {
                    index: 3,
                    name: "Vox 2".into(),
                    arm: false,
                },
            ],
        }
    }

    #[test]
    fn a_strip_finds_its_track_by_its_index_or_its_name_only_in_the_watched_live() {
        let told = state(false, false, true);
        assert_eq!(strip_track(&told, &at(3)).map(|t| t.index), Some(3));
        assert_eq!(strip_track(&told, &at(2)), None, "no armable track there");
        let named = strip(
            "band",
            json!({"kind": "track", "name": "Vox 1"}),
            "standard",
        );
        assert_eq!(strip_track(&told, &named).map(|t| t.index), Some(1));
        let other = strip(
            "master",
            json!({"kind": "track_at", "index": 1}),
            "standard",
        );
        assert_eq!(strip_track(&told, &other), None, "another Live");
        let back = strip("band", json!({"kind": "return_at", "index": 1}), "return");
        assert_eq!(strip_track(&told, &back), None, "a return");
        let same_index = strip("band", json!({"kind": "track_at", "index": 1}), "return");
        assert_eq!(strip_track(&told, &same_index), None, "a return's kind");
        let main = strip("band", json!({"kind": "master"}), "standard");
        assert_eq!(strip_track(&told, &main), None, "the main track");
    }

    #[test]
    fn a_strip_shows_no_key_a_ring_a_dot_or_the_pulse() {
        assert_eq!(arm_look(None, &at(1)), ArmLook::None);
        let off = state(true, true, false);
        assert_eq!(arm_look(Some(&off), &at(2)), ArmLook::None);
        assert_eq!(arm_look(Some(&off), &at(1)), ArmLook::Off);
        let stopped = state(false, true, true);
        assert_eq!(arm_look(Some(&stopped), &at(1)), ArmLook::Armed);
        let no_button = state(true, false, true);
        assert_eq!(arm_look(Some(&no_button), &at(1)), ArmLook::Armed);
        let recording = state(true, true, true);
        assert_eq!(arm_look(Some(&recording), &at(1)), ArmLook::Recording);
        // Not armed while the others record: never the pulse.
        assert_eq!(arm_look(Some(&recording), &at(3)), ArmLook::Off);
        let names = [
            ArmLook::None,
            ArmLook::Off,
            ArmLook::Armed,
            ArmLook::Recording,
        ]
        .map(ArmLook::name);
        assert_eq!(names, ["none", "off", "armed", "recording"]);
    }

    #[test]
    fn a_hold_turns_what_the_hub_told() {
        let told = state(false, true, true);
        assert_eq!(strip_arm(Some(&told), &at(1)), Some(true));
        assert_eq!(strip_arm(Some(&told), &at(3)), Some(false));
        assert_eq!(strip_arm(Some(&told), &at(2)), None);
        assert_eq!(strip_arm(None, &at(1)), None);
        assert_eq!(record_mode(Some(&told)), Some(true));
        assert_eq!(record_mode(Some(&RecState::default())), None);
        assert_eq!(record_mode(None), None);
    }

    #[test]
    fn the_foot_says_rec_not_rec_or_no_arm() {
        assert_eq!(foot_look(None), ("off", "REC"));
        assert_eq!(foot_look(Some(&RecState::default())), ("off", "REC"));
        assert_eq!(
            foot_look(Some(&state(false, false, true))),
            ("ready", "REC")
        );
        assert_eq!(
            foot_look(Some(&state(true, true, true))),
            ("recording", "REC")
        );
        assert_eq!(
            foot_look(Some(&state(true, false, true))),
            ("not_rec", "NOT REC")
        );
        assert_eq!(
            foot_look(Some(&state(true, true, false))),
            ("no_arm", "NO ARM")
        );
    }
}
