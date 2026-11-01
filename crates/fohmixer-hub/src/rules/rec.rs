//! The recording watch (#172): what the hub knows of the `band` Live's
//! recording, as pure state. Recording happens in that Live only, so the
//! other Live sets are not watched; a track list's `can_be_armed` reads say
//! which tracks can record (a group, a return and the main track cannot).
//! Every change is logged (`router/rec.rs`), so a later read of the event
//! log shows when and why a recording stopped: the page's own view of the
//! state is `RecState::watch`.

use std::collections::BTreeMap;

use fohmixer_proto::client::{RecState, RecTrack};
use serde_json::{Value, json};

/// The recording watch's inputs as heard.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rec {
    playing: Option<bool>,
    record_mode: Option<bool>,
    /// The armable tracks by index: name and `arm` (none until heard).
    tracks: BTreeMap<u32, (String, Option<bool>)>,
}

impl Rec {
    pub fn set_playing(&mut self, playing: Option<bool>) {
        self.playing = playing;
    }

    pub fn set_record_mode(&mut self, record_mode: Option<bool>) {
        self.record_mode = record_mode;
    }

    /// The armable tracks now: every `arm` starts unheard.
    pub fn set_tracks(&mut self, tracks: Vec<(u32, String)>) {
        self.tracks = tracks
            .into_iter()
            .map(|(index, name)| (index, (name, None)))
            .collect();
    }

    /// Track `index`'s `arm` (none: not known any more).
    pub fn set_arm(&mut self, index: u32, arm: Option<bool>) {
        if let Some(track) = self.tracks.get_mut(&index) {
            track.1 = arm;
        }
    }

    /// The Live set is gone: nothing is known.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The state the pages are told: the tracks whose `arm` was heard.
    pub fn state(&self) -> RecState {
        RecState {
            playing: self.playing,
            record_mode: self.record_mode,
            tracks: self
                .tracks
                .iter()
                .filter_map(|(index, (name, arm))| {
                    arm.map(|arm| RecTrack {
                        index: *index,
                        name: name.clone(),
                        arm,
                    })
                })
                .collect(),
        }
    }
}

/// The armable tracks of a track list (`names`, one per item in the list's
/// order) by its `can_be_armed` reads (`reads`, likewise), or why not: a
/// read without an answer fails the whole list.
pub fn armable(
    names: &[Option<String>],
    reads: &[Option<bool>],
) -> Result<Vec<(u32, String)>, String> {
    if names.len() != reads.len() {
        return Err(format!(
            "{} answers for {} tracks",
            reads.len(),
            names.len()
        ));
    }
    let mut out = Vec::new();
    for (index, (name, read)) in names.iter().zip(reads).enumerate() {
        let Some(can) = read else {
            return Err(format!("track {index}: no can_be_armed answer"));
        };
        if *can {
            let name = name.clone().unwrap_or_default();
            out.push((index as u32, name));
        }
    }
    Ok(out)
}

/// The `rec` record of a change from `before` to `now` (`why`: what
/// changed): none while the transport, the record button and the armed
/// tracks all stay (they make the watch).
pub fn change_fields(before: &RecState, now: &RecState, why: &str) -> Option<Value> {
    let same = before.playing == now.playing
        && before.record_mode == now.record_mode
        && armed_names(before) == armed_names(now);
    (!same).then(|| {
        json!({
            "state": now.watch().name(),
            "prev": before.watch().name(),
            "why": why,
            "is_playing": now.playing,
            "record_mode": now.record_mode,
            "armed": now.armed(),
            "armed_tracks": armed_names(now),
            "armable": now.tracks.len(),
        })
    })
}

/// The names of the armed tracks, in Live's order.
fn armed_names(state: &RecState) -> Vec<&str> {
    state
        .tracks
        .iter()
        .filter(|t| t.arm)
        .map(|t| t.name.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> Rec {
        let mut rec = Rec::default();
        rec.set_tracks(vec![(1, "Vox 1".into()), (4, "Vox 2".into())]);
        rec
    }

    #[test]
    fn a_track_shows_once_its_arm_is_heard() {
        let mut rec = rec();
        assert!(rec.state().tracks.is_empty(), "no arm heard yet");
        rec.set_arm(4, Some(true));
        rec.set_arm(9, Some(true));
        let state = rec.state();
        assert_eq!(
            state.tracks,
            vec![RecTrack {
                index: 4,
                name: "Vox 2".into(),
                arm: true,
            }]
        );
        rec.set_arm(4, None);
        assert!(rec.state().tracks.is_empty(), "known no more");
        // A new list starts every arm unheard.
        rec.set_arm(1, Some(false));
        rec.set_tracks(vec![(1, "Vox 1".into())]);
        assert!(rec.state().tracks.is_empty());
    }

    #[test]
    fn the_state_carries_the_transport_and_the_record_button_and_clear_forgets_all() {
        let mut rec = rec();
        rec.set_playing(Some(true));
        rec.set_record_mode(Some(false));
        rec.set_arm(1, Some(true));
        let state = rec.state();
        assert_eq!(state.playing, Some(true));
        assert_eq!(state.record_mode, Some(false));
        assert_eq!(state.armed(), 1);
        rec.clear();
        assert_eq!(rec, Rec::default());
    }

    #[test]
    fn only_tracks_that_can_be_armed_are_armable_and_a_missing_answer_fails_the_list() {
        let names = vec![Some("Vox 1".to_string()), Some("Group".to_string()), None];
        assert_eq!(
            armable(&names, &[Some(true), Some(false), Some(true)]),
            Ok(vec![(0, "Vox 1".to_string()), (2, String::new())])
        );
        assert_eq!(
            armable(&names, &[Some(true), None, Some(true)]),
            Err("track 1: no can_be_armed answer".to_string())
        );
        assert_eq!(
            armable(&names, &[Some(true)]),
            Err("1 answers for 3 tracks".to_string())
        );
    }

    fn state(playing: bool, record: bool, armed: &[&str]) -> RecState {
        RecState {
            playing: Some(playing),
            record_mode: Some(record),
            tracks: armed
                .iter()
                .enumerate()
                .map(|(i, name)| RecTrack {
                    index: i as u32,
                    name: (*name).to_string(),
                    arm: true,
                })
                .collect(),
        }
    }

    #[test]
    fn a_stop_of_a_recording_is_one_record_with_its_inputs_and_why() {
        let before = state(true, true, &["Vox 1", "Vox 2"]);
        let now = state(false, true, &["Vox 1", "Vox 2"]);
        assert_eq!(
            change_fields(&before, &now, "transport"),
            Some(json!({
                "state": "ready",
                "prev": "recording",
                "why": "transport",
                "is_playing": false,
                "record_mode": true,
                "armed": 2,
                "armed_tracks": ["Vox 1", "Vox 2"],
                "armable": 2,
            }))
        );
    }

    #[test]
    fn each_input_makes_a_record_and_nothing_else_does() {
        let base = state(false, false, &["Vox 1"]);
        assert_eq!(change_fields(&base, &base.clone(), "arm"), None);
        // Armed tracks swapped: same count, same watch.
        assert!(change_fields(&base, &state(false, false, &["Vox 2"]), "arm").is_some());
        // The record button while stopped: the watch stays Ready.
        assert!(change_fields(&base, &state(false, true, &["Vox 1"]), "record_mode").is_some());
        // The transport with nothing armed: the watch stays Off.
        let idle = state(false, false, &[]);
        assert!(change_fields(&idle, &state(true, false, &[]), "transport").is_some());
        // Not armed, but one more armable track: no record.
        let mut more = base.clone();
        more.tracks.push(RecTrack {
            index: 7,
            name: "Vox 3".into(),
            arm: false,
        });
        assert_eq!(change_fields(&base, &more, "tracks"), None);
        // An arm that turns a playing set into a recording.
        assert!(
            change_fields(
                &state(true, true, &[]),
                &state(true, true, &["Vox 1"]),
                "arm"
            )
            .is_some()
        );
    }
}
