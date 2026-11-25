use super::*;

/// An output of `kind` and `channel`.
fn out(kind: &str, channel: &str) -> Output {
    Output {
        kind: kind.to_string(),
        channel: channel.to_string(),
    }
}

/// A track named `name` in group `parent`, routed to `kind`.
fn track(name: &str, parent: Option<usize>, kind: &str) -> Track {
    Track {
        name: name.to_string(),
        mute: false,
        group: false,
        parent,
        output: out(kind, ""),
        sends: 0,
    }
}

/// A group track.
fn group(name: &str, parent: Option<usize>, kind: &str) -> Track {
    Track {
        group: true,
        ..track(name, parent, kind)
    }
}

/// Muted.
fn muted(t: Track) -> Track {
    Track { mute: true, ..t }
}

const EXT: &str = "Ext. Out";
const STREAM_CH: &str = "3/4 Final LR";

/// The shape of the MBC set as the probe read it (invented track names
/// besides the constants): the bus (0) holds HAND 1 sync and a muted
/// group (2) with two tracks, a nested group (5) with one; the bus goes to
/// the stream track (7), which goes to the external output 3/4; a track
/// on the main output and an unmuted Sends Only track inside the bus (9)
/// with no returns.
fn mbc() -> Set {
    Set {
        tracks: vec![
            group(BUS, None, STREAM_TRACK),
            track(HAND1_SYNC, Some(0), "Group"),
            muted(group("Speech grp", Some(0), "Group")),
            track("Mic A", Some(2), "Group"),
            track("Mic B", Some(2), "Group"),
            muted(group("Outer grp", Some(0), "Group")),
            track("Inner", Some(5), "Group"),
            Track {
                output: out(EXT, STREAM_CH),
                ..track(STREAM_TRACK, None, "")
            },
            track("Room", None, "Main"),
            track("Quiet", Some(0), SENDS_ONLY),
        ],
        returns: Vec::new(),
        main: out(EXT, "1/2"),
    }
}

#[test]
fn the_mbc_set_carries_only_hand1_sync() {
    let proof = decide(&mbc()).unwrap();
    assert_eq!(proof.open, vec![HAND1_SYNC.to_string()]);
    assert!(proof.hand1_sync_only);
    assert_eq!(
        proof.outputs,
        Outputs {
            bus: BUS.into(),
            bus_to: STREAM_TRACK.into(),
            stream: STREAM_TRACK.into(),
            stream_to: out(EXT, STREAM_CH),
            others: Vec::new(),
        }
    );
    assert!(proof.outputs.hold());
}

#[test]
fn an_unmuted_group_opens_its_tracks_and_a_muted_one_closes_them() {
    let mut set = mbc();
    set.tracks[2].mute = false;
    let proof = decide(&set).unwrap();
    assert_eq!(proof.open, vec![HAND1_SYNC, "Mic A", "Mic B"]);
    assert!(!proof.hand1_sync_only);
    // A muted track inside an open group stays closed; a group is never
    // listed itself.
    set.tracks[3].mute = true;
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC, "Mic B"]);
}

#[test]
fn a_nested_group_reaches_the_bus_only_when_no_group_on_the_way_is_muted() {
    let mut set = mbc();
    // Inner (6) sits in Outer grp (5), muted: closed.
    assert!(!decide(&set).unwrap().open.contains(&"Inner".to_string()));
    set.tracks[5].mute = false;
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC, "Inner"]);
    // One level deeper: Inner in a group inside Outer grp.
    set.tracks.push(group("Deep grp", Some(5), "Group"));
    set.tracks[6].parent = Some(10);
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC, "Inner"]);
    set.tracks[10].mute = true;
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC]);
    // A muted bus closes everything (nothing on the stream: no proof).
    set.tracks[0].mute = true;
    let proof = decide(&set).unwrap();
    assert!(proof.open.is_empty());
    assert!(!proof.hand1_sync_only);
}

#[test]
fn a_track_in_another_group_or_none_does_not_reach_the_bus() {
    let mut set = mbc();
    set.tracks.push(group("Band grp", None, "Main"));
    set.tracks.push(track("Keys", Some(10), "Group"));
    let proof = decide(&set).unwrap();
    assert_eq!(proof.open, vec![HAND1_SYNC]);
    assert!(proof.hand1_sync_only);
}

#[test]
fn a_track_routed_straight_to_the_bus_is_open() {
    let mut set = mbc();
    set.tracks[8].output = out(BUS, "");
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC, "Room"]);
    set.tracks[8].mute = true;
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC]);
    // A return routed to the bus too.
    set.returns.push(Return {
        name: "A-Reverb".into(),
        mute: false,
        output: out(BUS, ""),
    });
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC, "A-Reverb"]);
    set.returns[0].mute = true;
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC]);
}

#[test]
fn a_sends_only_track_feeds_nothing_without_sends_or_returns() {
    let mut set = mbc();
    // Sends but no returns: nothing.
    set.tracks[9].sends = 2;
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC]);
    // Returns but no sends: nothing.
    set.tracks[9].sends = 0;
    set.returns.push(Return {
        name: "A-Reverb".into(),
        mute: false,
        output: out("Main", ""),
    });
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC]);
    // Both: open (its audio may come back through a return).
    set.tracks[9].sends = 1;
    let proof = decide(&set).unwrap();
    assert_eq!(proof.open, vec![HAND1_SYNC, "Quiet"]);
    assert!(!proof.hand1_sync_only);
    // Any other output type with both sends and returns stays open too.
    assert!(feeds(&out("Group", ""), 0, 0));
    assert!(!feeds(&out(SENDS_ONLY, ""), 1, 0));
    assert!(!feeds(&out(SENDS_ONLY, ""), 0, 1));
    assert!(feeds(&out(SENDS_ONLY, ""), 1, 1));
}

#[test]
fn a_second_track_routed_to_the_stream_track_or_its_output_breaks_the_proof() {
    let mut set = mbc();
    set.tracks[8].output = out(STREAM_TRACK, "");
    let proof = decide(&set).unwrap();
    assert_eq!(proof.open, vec![HAND1_SYNC]);
    assert_eq!(
        proof.outputs.others,
        vec![Routed {
            name: "Room".into(),
            kind: STREAM_TRACK.into(),
            channel: String::new()
        }]
    );
    assert!(!proof.hand1_sync_only);
    // Muted, it still counts: the fact is the routing.
    set.tracks[8].mute = true;
    assert!(!decide(&set).unwrap().hand1_sync_only);
    // The same external output as the stream track's.
    let mut set = mbc();
    set.tracks[8].output = out(EXT, STREAM_CH);
    let proof = decide(&set).unwrap();
    assert_eq!(proof.outputs.others[0].name, "Room");
    assert_eq!(proof.outputs.others[0].channel, STREAM_CH);
    assert!(!proof.hand1_sync_only);
    // Another channel of that output is no stream feed.
    set.tracks[8].output = out(EXT, "5/6");
    assert!(decide(&set).unwrap().hand1_sync_only);
}

#[test]
fn a_return_or_the_main_track_on_the_stream_output_breaks_the_proof() {
    let mut set = mbc();
    set.returns.push(Return {
        name: "B-Delay".into(),
        mute: true,
        output: out(STREAM_TRACK, ""),
    });
    let proof = decide(&set).unwrap();
    assert_eq!(proof.outputs.others.len(), 1);
    assert_eq!(proof.outputs.others[0].name, "B-Delay");
    assert!(!proof.hand1_sync_only);
    let mut set = mbc();
    set.main = out(EXT, STREAM_CH);
    let proof = decide(&set).unwrap();
    assert_eq!(
        proof.outputs.others,
        vec![Routed {
            name: "Main".into(),
            kind: EXT.into(),
            channel: STREAM_CH.into()
        }]
    );
    assert!(!proof.hand1_sync_only);
}

#[test]
fn the_bus_elsewhere_or_another_track_open_breaks_the_proof() {
    let mut set = mbc();
    set.tracks[0].output = out(EXT, STREAM_CH);
    let proof = decide(&set).unwrap();
    assert_eq!(proof.outputs.bus_to, EXT);
    // The bus itself is never one of the others.
    assert!(proof.outputs.others.is_empty());
    assert!(!proof.outputs.hold());
    assert!(!proof.hand1_sync_only);
    // HAND 1 sync muted: nothing open, no proof.
    let mut set = mbc();
    set.tracks[1].mute = true;
    let proof = decide(&set).unwrap();
    assert!(proof.open.is_empty());
    assert!(!proof.hand1_sync_only);
}

#[test]
fn a_missing_or_doubled_bus_or_stream_track_is_no_answer() {
    let mut set = mbc();
    set.tracks[0].name = "Other grp".into();
    assert_eq!(decide(&set), Err(format!("no track is named {BUS}")));
    let mut set = mbc();
    set.tracks[8].name = BUS.into();
    assert_eq!(decide(&set), Err(format!("two tracks are named {BUS}")));
    let mut set = mbc();
    set.tracks[7].name = "Other".into();
    assert_eq!(
        decide(&set),
        Err(format!("no track is named {STREAM_TRACK}"))
    );
    let mut set = mbc();
    set.tracks[8].name = STREAM_TRACK.into();
    assert_eq!(
        decide(&set),
        Err(format!("two tracks are named {STREAM_TRACK}"))
    );
}

#[test]
fn a_broken_chain_of_groups_reaches_nothing() {
    let mut set = mbc();
    // A cycle that never meets the bus.
    set.tracks[2].parent = Some(5);
    set.tracks[2].mute = false;
    set.tracks[5].parent = Some(2);
    set.tracks[5].mute = false;
    // A parent index past the list.
    set.tracks[8].parent = Some(99);
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC]);
}

#[test]
fn an_answer_carries_the_proof_or_the_reason() {
    let proof = decide(&mbc()).unwrap();
    let ok = Answer::of(Ok(proof.clone()), 1_700);
    assert_eq!(
        serde_json::to_value(&ok).unwrap(),
        serde_json::json!({"ok": true, "ts": 1_700, "age_ms": 0, "hand1_sync_only": true,
            "open": [HAND1_SYNC],
            "outputs": {"bus": BUS, "bus_to": STREAM_TRACK, "stream": STREAM_TRACK,
                        "stream_to": {"type": EXT, "channel": STREAM_CH}, "others": []}})
    );
    let failed = Answer::of(Err("mbc is offline".into()), 9);
    assert_eq!(
        serde_json::to_value(&failed).unwrap(),
        serde_json::json!({"ok": false, "ts": 9, "age_ms": 0, "hand1_sync_only": false,
            "reason": "mbc is offline"})
    );
    // An open set other than HAND 1 sync answers ok with false.
    let mut set = mbc();
    set.tracks[2].mute = false;
    let answer = Answer::of(decide(&set), 5);
    assert!(answer.ok);
    assert!(!answer.hand1_sync_only);
}
