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
    let mut t = track(name, parent, kind);
    t.group = true;
    t
}

/// Muted.
fn muted(mut t: Track) -> Track {
    t.mute = true;
    t
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
            track(HAND1_SYNC, Some(0), GROUP),
            muted(group("Speech grp", Some(0), GROUP)),
            track("Mic A", Some(2), GROUP),
            track("Mic B", Some(2), GROUP),
            muted(group("Outer grp", Some(0), GROUP)),
            track("Inner", Some(5), GROUP),
            Track {
                name: STREAM_TRACK.to_string(),
                mute: false,
                group: false,
                parent: None,
                output: out(EXT, STREAM_CH),
                sends: 0,
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
    set.tracks.push(group("Deep grp", Some(5), GROUP));
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
    set.tracks.push(track("Keys", Some(10), GROUP));
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

fn ret(name: &str, kind: &str) -> Return {
    Return {
        name: name.into(),
        mute: false,
        output: out(kind, ""),
    }
}

#[test]
fn a_sends_only_track_feeds_only_the_returns_and_only_with_sends() {
    let mut set = mbc();
    // Sends but no returns: nothing.
    set.tracks[9].sends = 2;
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC]);
    // A return on the main output: the Sends Only track reaches no bus.
    set.returns.push(ret("A-Reverb", "Main"));
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC]);
    // A return routed to the bus: it and the track feeding it are open.
    set.returns.push(ret("B-Delay", BUS));
    let proof = decide(&set).unwrap();
    assert_eq!(proof.open, vec![HAND1_SYNC, "Quiet", "B-Delay"]);
    assert!(!proof.hand1_sync_only);
    // No sends: the track feeds nothing, the return is still open.
    set.tracks[9].sends = 0;
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC, "B-Delay"]);
    // The return muted: nothing through it.
    set.tracks[9].sends = 1;
    set.returns[1].mute = true;
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC]);
}

#[test]
fn two_ways_to_one_return_are_no_loop() {
    // Quiet's sends reach A-Reverb and B-Delay; B-Delay goes to A-Reverb,
    // which goes to the bus: A-Reverb is met twice, on two paths.
    let mut set = mbc();
    set.tracks[9].sends = 1;
    set.returns.push(ret("A-Reverb", "Main"));
    set.returns.push(ret("B-Delay", "A-Reverb"));
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC]);
    set.returns[0].output = out(BUS, "");
    assert_eq!(
        decide(&set).unwrap().open,
        vec![HAND1_SYNC, "Quiet", "A-Reverb", "B-Delay"]
    );
}

#[test]
fn a_group_or_a_track_routed_into_the_bus_from_outside_is_followed() {
    // A group outside the bus, routed to the bus: its unmuted track is open.
    let mut set = mbc();
    set.tracks.push(group("Side grp", None, BUS));
    set.tracks.push(track("Side A", Some(10), GROUP));
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC, "Side A"]);
    set.tracks[10].mute = true;
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC]);
    // A track routed to a track inside the bus.
    let mut set = mbc();
    set.tracks[8].output = out(HAND1_SYNC, "");
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC, "Room"]);
    // To a group inside the bus that is muted: nothing passes.
    set.tracks[8].output = out("Speech grp", "");
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC]);
    // A group shown by its own name instead of Group is followed the same.
    let mut set = mbc();
    set.tracks[1].output = out(BUS, "");
    assert_eq!(decide(&set).unwrap().open, vec![HAND1_SYNC]);
    assert!(decide(&set).unwrap().hand1_sync_only);
}

#[test]
fn a_type_naming_two_tracks_or_group_outside_a_group_is_a_doubt() {
    let mut set = mbc();
    set.tracks.push(track("Twin", None, "Main"));
    set.tracks.push(track("Twin", None, "Main"));
    set.tracks[8].output = out("Twin", "");
    assert_eq!(
        decide(&set),
        Err("Room goes to Twin, which names two tracks".into())
    );
    let mut set = mbc();
    set.tracks[8].output = out(GROUP, "");
    assert_eq!(
        decide(&set),
        Err("Room goes to Group but sits in no group".into())
    );
    // A group index past the list.
    let mut set = mbc();
    set.tracks[8].output = out(GROUP, "");
    set.tracks[8].parent = Some(set.tracks.len());
    assert_eq!(
        decide(&set),
        Err("Room goes to Group but sits in no group".into())
    );
}

#[test]
fn outputs_share_a_channel_when_their_numbers_overlap() {
    let stereo = out(EXT, STREAM_CH);
    assert!(same_output(&stereo, &stereo));
    assert!(same_output(&out(EXT, "3"), &stereo));
    assert!(same_output(&out(EXT, "4"), &stereo));
    assert!(same_output(&out(EXT, "4/5"), &stereo));
    assert!(!same_output(&out(EXT, "5/6"), &stereo));
    assert!(!same_output(&out(EXT, "1"), &stereo));
    assert!(!same_output(&out("Main", STREAM_CH), &stereo));
    // A channel without numbers that differs is a doubt: shared.
    assert!(same_output(&out(EXT, "Final"), &stereo));
    assert!(same_output(&out(EXT, ""), &stereo));
    assert!(same_output(&out(EXT, ""), &out(EXT, "")));
    assert_eq!(channel_numbers("3/4 Final LR"), Some(vec![3, 4]));
    assert_eq!(channel_numbers("12"), Some(vec![12]));
    assert_eq!(channel_numbers("3/x"), None);
    assert_eq!(channel_numbers(""), None);
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
fn a_routing_loop_is_a_doubt() {
    let mut set = mbc();
    // Speech grp and Outer grp each in the other: Mic A loops.
    set.tracks[2].parent = Some(5);
    set.tracks[2].mute = false;
    set.tracks[5].parent = Some(2);
    set.tracks[5].mute = false;
    let why = decide(&set).unwrap_err();
    assert!(why.starts_with("the routing loops through "), "{why}");
    // A track routed to itself names no other track: a doubt.
    let mut set = mbc();
    set.tracks[8].output = out("Room", "");
    assert_eq!(
        decide(&set),
        Err("Room goes to Room, which is no track and no known output".into())
    );
}

#[test]
fn a_type_that_is_no_track_and_no_known_output_is_a_doubt_reached_or_not() {
    // The bus's shown name (Live shows its `#` as a number) names no node.
    let mut set = mbc();
    set.tracks[8].output = out("ToStream 1s sync 1", "");
    assert_eq!(
        decide(&set),
        Err("Room goes to ToStream 1s sync 1, which is no track and no known output".into())
    );
    // Also on a muted track nothing reaches through.
    set.tracks[8].mute = true;
    assert!(decide(&set).is_err());
    // The known ones leave the set.
    for kind in OUT_OF_SET {
        let mut set = mbc();
        set.tracks[8].output = out(kind, "5/6");
        assert!(decide(&set).unwrap().hand1_sync_only, "{kind}");
    }
}

#[test]
fn the_bus_must_be_a_group_and_the_stream_track_must_leave_the_set() {
    let mut set = mbc();
    set.tracks[0].group = false;
    assert_eq!(decide(&set), Err(format!("{BUS} is no group")));
    // The stream track relayed through another track.
    let mut set = mbc();
    set.tracks[7].output = out("Room", "");
    assert_eq!(
        decide(&set),
        Err(format!("{STREAM_TRACK} goes to Room, not out of the set"))
    );
    let mut set = mbc();
    set.tracks[7].output = out(SENDS_ONLY, "");
    assert_eq!(
        decide(&set),
        Err(format!(
            "{STREAM_TRACK} goes to {SENDS_ONLY}, not out of the set"
        ))
    );
    // On the main output it leaves the set; Room shares that output now.
    let mut set = mbc();
    set.tracks[7].output = out("Main", "");
    let proof = decide(&set).unwrap();
    assert_eq!(proof.outputs.others[0].name, "Room");
    assert!(!proof.hand1_sync_only);
    set.tracks[8].output = out(EXT, "5/6");
    assert!(decide(&set).unwrap().hand1_sync_only);
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
