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
    /// How many tracks can be armed, heard or not.
    armable: usize,
}

impl Rec {
    pub fn set_playing(&mut self, playing: Option<bool>) {
        self.playing = playing;
    }

    pub fn set_record_mode(&mut self, record_mode: Option<bool>) {
        self.record_mode = record_mode;
    }

    /// The armable tracks now. A track that stays keeps its `arm`: one of
    /// that name at that index, else the only one of that name (a moved
    /// track). Any other starts unheard.
    pub fn set_tracks(&mut self, tracks: Vec<(u32, String)>) {
        self.armable = tracks.len();
        let old = std::mem::take(&mut self.tracks);
        self.tracks = tracks
            .into_iter()
            .map(|(index, name)| {
                let arm = kept_arm(&old, index, &name);
                (index, (name, arm))
            })
            .collect();
    }

    /// How many tracks can be armed, heard or not.
    pub fn armable(&self) -> usize {
        self.armable
    }

    /// Whether track `index`'s `arm` is known.
    pub fn arm_known(&self, index: u32) -> bool {
        self.tracks.get(&index).is_some_and(|t| t.1.is_some())
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

/// The `arm` a track of the new list keeps from the old one.
fn kept_arm(old: &BTreeMap<u32, (String, Option<bool>)>, index: u32, name: &str) -> Option<bool> {
    if let Some((same, arm)) = old.get(&index)
        && same == name
    {
        return *arm;
    }
    let mut named = old.values().filter(|(same, _)| same == name);
    match (named.next(), named.next()) {
        (Some((_, arm)), None) => *arm,
        _ => None,
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

/// Whether the watch has heard nothing of the set yet but the transport,
/// which the `transport` record tells: nothing to say to a page or the log.
pub fn silent(before: &RecState, now: &RecState) -> bool {
    let blank = |s: &RecState| s.record_mode.is_none() && s.tracks.is_empty();
    blank(before) && blank(now)
}

/// The `rec` record of a change from `before` to `now` (`why`: what
/// changed): none while the transport, the record button and the armed
/// tracks all stay (they make the watch), or while the watch is [`silent`].
pub fn change_fields(
    before: &RecState,
    now: &RecState,
    armable: usize,
    why: &str,
) -> Option<Value> {
    let same = silent(before, now)
        || before.playing == now.playing
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
            "armable": armable,
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
        assert!(!rec.arm_known(4));
        rec.set_arm(1, Some(false));
        assert!(rec.arm_known(1));
    }

    #[test]
    fn a_track_that_stays_keeps_its_arm_through_a_new_list() {
        let mut rec = rec();
        rec.set_arm(1, Some(true));
        rec.set_arm(4, Some(false));
        // Vox 1 moved to index 0, Vox 2 stayed, a new track and a new name
        // at Vox 1's old index start unheard.
        rec.set_tracks(vec![
            (0, "Vox 1".into()),
            (1, "New".into()),
            (4, "Vox 2".into()),
        ]);
        assert_eq!(rec.armable(), 3);
        assert_eq!(rec.state().armed(), 1);
        assert!(rec.arm_known(0) && rec.arm_known(4));
        assert!(!rec.arm_known(1));
        // Two of one name: only the one at its own index is known to stay.
        let mut twin = Rec::default();
        twin.set_tracks(vec![(1, "Keys".into()), (5, "Keys".into())]);
        twin.set_arm(1, Some(true));
        twin.set_arm(5, Some(false));
        twin.set_tracks(vec![(1, "Keys".into()), (3, "Keys".into())]);
        assert!(twin.arm_known(1));
        assert!(!twin.arm_known(3));
    }

    #[test]
    fn the_armable_count_holds_tracks_whose_arm_is_not_heard_yet() {
        let mut rec = rec();
        rec.set_arm(1, Some(true));
        assert_eq!(rec.armable(), 2);
        assert_eq!(rec.state().tracks.len(), 1);
        let fields = change_fields(&RecState::default(), &rec.state(), rec.armable(), "tracks");
        assert_eq!(fields.unwrap()["armable"], json!(2));
        rec.clear();
        assert_eq!(rec.armable(), 0);
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
            change_fields(&before, &now, 2, "transport"),
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
    fn the_transport_alone_says_nothing() {
        let unheard = RecState::default();
        let playing = RecState {
            playing: Some(true),
            record_mode: None,
            tracks: Vec::new(),
        };
        assert!(silent(&unheard, &playing));
        assert_eq!(change_fields(&unheard, &playing, 0, "transport"), None);
        // A set that was heard and went is news; so is its first word.
        let heard = state(false, false, &[]);
        assert!(!silent(&heard, &unheard));
        assert!(!silent(&unheard, &heard));
        assert!(change_fields(&heard, &unheard, 0, "offline").is_some());
        let tracks = RecState {
            playing: None,
            record_mode: None,
            tracks: state(false, false, &["Vox 1"]).tracks,
        };
        assert!(!silent(&unheard, &tracks));
    }

    #[test]
    fn each_input_makes_a_record_and_nothing_else_does() {
        let base = state(false, false, &["Vox 1"]);
        assert_eq!(change_fields(&base, &base.clone(), 1, "arm"), None);
        // Armed tracks swapped: same count, same watch.
        assert!(change_fields(&base, &state(false, false, &["Vox 2"]), 1, "arm").is_some());
        // The record button while stopped: the watch stays Ready.
        assert!(change_fields(&base, &state(false, true, &["Vox 1"]), 1, "record_mode").is_some());
        // The transport with nothing armed: the watch stays Off.
        let idle = state(false, false, &[]);
        assert!(change_fields(&idle, &state(true, false, &[]), 0, "transport").is_some());
        // Not armed, but one more armable track: no record.
        let mut more = base.clone();
        more.tracks.push(RecTrack {
            index: 7,
            name: "Vox 3".into(),
            arm: false,
        });
        assert_eq!(change_fields(&base, &more, 2, "tracks"), None);
        // An arm that turns a playing set into a recording.
        assert!(
            change_fields(
                &state(true, true, &[]),
                &state(true, true, &["Vox 1"]),
                1,
                "arm"
            )
            .is_some()
        );
    }
}
