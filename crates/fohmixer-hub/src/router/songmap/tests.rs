use fohmixer_proto::client::{ValueItem, hub_key};
use fohmixer_proto::songmap::{ClipName, Counts, Cue, Key, ProblemKind, SectionsState};
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedReceiver;

use super::super::tests::{next_of, offline_router, records, status};
use super::super::{Router, RouterMsg};
use super::{
    CLIPS, CUE_POINTS, Listed, Listing, MAX_TEXTS, Missing, Read, SECTIONS, SONGMAP_CLIENT,
    Trigger, clip_commands, clips_of, cues_of, first_commands, listed, next_read, report_of,
    same_names, sections_state, songmap_error_fields, songmap_fields, songmap_warn, status_of,
    time_commands, transport_reads,
};
use crate::live::client::LiveEvent;

const TRANSPORT: &str = "band|live_set|is_playing|false";

fn cue_key() -> String {
    hub_key("band", "live_set", CUE_POINTS, false)
}

fn tracks_key() -> String {
    hub_key("band", "live_set", "tracks", false)
}

fn clips_key(track: &str) -> String {
    hub_key("band", track, CLIPS, false)
}

/// `(path, name)` pairs of a list, its paths `<list> <i>`.
fn listed_pairs(list: &str, names: &[&str]) -> Vec<(String, String)> {
    names
        .iter()
        .enumerate()
        .map(|(i, n)| (format!("{list} {i}"), (*n).to_string()))
        .collect()
}

/// A list answer as the script encodes it.
fn objects(list: &str, class: &str, names: &[&str]) -> Value {
    Value::from(
        names
            .iter()
            .enumerate()
            .map(|(i, n)| json!({"$ref": format!("live_{i}"), "path": format!("{list} {i}"), "class": class, "name": n}))
            .collect::<Vec<Value>>(),
    )
}

fn ok(data: Value) -> Value {
    json!({"ok": true, "data": data})
}

fn refused(error: &str) -> Value {
    json!({"ok": false, "error": error, "errorType": "PathError"})
}

/// A set read: two songs, the second keyless, a key clip in the first.
fn listing() -> Listing {
    listing_at(Some("live_set tracks 1"), SectionsState::One)
}

/// The same set with its Sections track at `track`.
fn listing_at(track: Option<&str>, sections: SectionsState) -> Listing {
    let cue = |name: &str, time: Option<f64>| Cue {
        name: name.into(),
        time,
    };
    Listing {
        cues: vec![
            cue("001 Test One {C 120 4/4}", Some(0.0)),
            cue("> Verse", None),
            cue("SONG END", Some(100.0)),
            cue("002 Test Two", Some(120.0)),
            cue("SONG END", Some(200.0)),
        ],
        clips: vec![
            ClipName {
                name: "Verse".into(),
                start: None,
            },
            ClipName {
                name: "Key {G}".into(),
                start: Some(50.0),
            },
        ],
        sections,
        track: track.map(str::to_string),
    }
}

/// The band connected (the offline router's instance stays offline in its
/// task, so a read's call answers `instance offline` at once).
fn connected(router: &mut Router) {
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Connected(crate::live::ConnectInfo {
            instance: "band".into(),
            set_name: "set".into(),
            script_version: "0".into(),
            live_version: "12".into(),
        }),
    });
}

/// Live answers every listener of the band waiting: the transport, the
/// locators and the tracks (no Sections track), anything else empty.
fn answer_listeners(router: &mut Router) {
    for o in router.subs.drain_outgoing() {
        let slots: Vec<Value> = o
            .commands
            .iter()
            .map(|c| match c["args"]["prop"].as_str().unwrap_or("") {
                "is_playing" => ok(json!({"key": "live_1.is_playing", "value": false})),
                "cue_points" => ok(json!({"key": "live_2.cue_points", "value": []})),
                "tracks" => ok(json!({"key": "live_3.tracks", "value": []})),
                _ => ok(Value::Null),
            })
            .collect();
        router.subs.on_result(&o.instance, &o.uuid, &slots);
    }
}

/// The `songmap` records waiting, without `ev` and `ts`.
fn songmap_records(rx: &std::sync::mpsc::Receiver<Value>) -> Vec<Value> {
    records(rx)
        .into_iter()
        .filter(|r| r["ev"] == "songmap")
        .map(|mut r| {
            let fields = r.as_object_mut().unwrap();
            fields.remove("ev");
            fields.remove("ts");
            r
        })
        .collect()
}

/// The next read's answer the router's task sends.
async fn read_sent(rx: &mut UnboundedReceiver<RouterMsg>) -> (u64, Result<Listing, String>) {
    let msg = next_of(rx, "a song map read", |m| {
        matches!(m, RouterMsg::SongMapRead { .. })
    })
    .await;
    let RouterMsg::SongMapRead {
        read_no, result, ..
    } = msg
    else {
        unreachable!()
    };
    (read_no, result)
}

fn answer(router: &mut Router, read_no: u64, result: Result<Listing, String>) {
    router.handle(RouterMsg::SongMapRead {
        instance: "band".into(),
        read_no,
        result,
        ms: 7,
    });
}

fn report(
    router: &mut Router,
    instance: &str,
) -> Result<fohmixer_proto::client::SongMapReport, Missing> {
    let (reply, mut answer) = tokio::sync::oneshot::channel();
    router.handle(RouterMsg::SongMap {
        instance: instance.into(),
        reply,
    });
    answer.try_recv().expect("answered at once")
}

#[tokio::test]
async fn a_connect_reads_the_map_once() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, mut rx, events) = offline_router(dir.path());
    connected(&mut router);
    assert_eq!(router.songmap.reads, 0, "the lists' first values read");
    answer_listeners(&mut router);
    router.flush();
    // The locators' and the tracks' first values and the transport's came
    // in one flush: one read.
    assert_eq!(router.songmap.reads, 1);
    assert_eq!(router.songmap.reading.get("band"), Some(&1));
    assert!(router.songmap.dirty.is_empty());
    let (read_no, result) = read_sent(&mut rx).await;
    assert_eq!(
        (read_no, result),
        (1, Err("instance offline".to_string())),
        "its first call went to Live"
    );
    // Live's answer: the map, summed up in the status and recorded.
    answer(&mut router, 1, Ok(listing()));
    assert_eq!(router.songmap.reads, 1, "nothing asked for another read");
    let now = status(&mut router).songmap;
    assert_eq!(now.len(), 1);
    assert_eq!(
        (
            now[0].instance.as_str(),
            now[0].songs,
            now[0].keyed,
            now[0].changes,
            now[0].problems,
            now[0].sections,
            now[0].stale
        ),
        ("band", 2, 1, 1, 0, SectionsState::One, false)
    );
    assert!(now[0].at_ms > 1_767_225_600_000, "the hub's UTC clock");
    let got = songmap_records(&events);
    assert_eq!(
        got,
        vec![
            json!({"instance": "band", "read_no": 1, "songs": 2, "keyed": 1, "changes": 1,
                    "problems": 0, "sections": "one", "ms": 7, "problem_texts": []})
        ]
    );
    // The read found the Sections track: its clips are followed by index.
    assert_eq!(
        router.songmap.clips.get("band").map(|(k, _)| k.clone()),
        Some(clips_key("live_set tracks 1"))
    );
}

#[tokio::test]
async fn the_lists_are_followed_once_and_are_the_hubs_own() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, _events) = offline_router(dir.path());
    assert_eq!(router.subs.subscriptions("band"), 1, "the transport only");
    connected(&mut router);
    assert_eq!(router.subs.subscriptions("band"), 3);
    assert_eq!(
        router.songmap.subs.keys().cloned().collect::<Vec<_>>(),
        vec![cue_key(), tracks_key()]
    );
    // A reconnect keeps them (the table resolves them again).
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    connected(&mut router);
    assert_eq!(router.subs.subscriptions("band"), 3);
    let now = status(&mut router);
    assert_eq!(now.subscriptions["band"], 0, "the hub's own");
    assert_eq!(now.listeners["band"], 0, "the hub's own");
}

#[test]
fn only_needed_cue_times_are_read() {
    let names = [
        "001 A {C 100}",
        "> Intro",
        "* TS",
        "SONG END",
        "002 B {D 100}",
        "> Verse",
        "> Chorus",
        "*MS",
        "() x",
        "SONG END",
        "[x] clyphx",
        "003 C",
        "> Ending",
        "* end",
        "SONG END",
    ];
    let cues = listed_pairs("live_set cue_points", &names);
    let calls = time_commands(&cues);
    assert_eq!(calls.len(), 1);
    let paths: Vec<&str> = calls[0]
        .iter()
        .map(|c| c["target"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        [
            "live_set cue_points 0",
            "live_set cue_points 3",
            "live_set cue_points 4",
            "live_set cue_points 9",
            "live_set cue_points 11",
            "live_set cue_points 14"
        ]
    );
    assert!(
        calls[0]
            .iter()
            .all(|c| c["name"] == "get_prop" && c["args"] == json!({"prop": "time"}))
    );
}

#[test]
fn times_are_read_in_chunks_of_200() {
    let names: Vec<String> = (0..450).map(|i| format!("{:03} Song", i % 1000)).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let calls = time_commands(&listed_pairs("live_set cue_points", &refs));
    let sizes: Vec<usize> = calls.iter().map(Vec::len).collect();
    assert_eq!(sizes, [200, 200, 50]);
    assert_eq!(calls[2][49]["target"], "live_set cue_points 449");
    assert!(time_commands(&[]).is_empty(), "no locators, no call");
}

#[test]
fn only_clips_with_a_brace_get_their_start_read() {
    let clips = listed_pairs(
        "live_set tracks 1 arrangement_clips",
        &["Verse", "Key {G}", "Chorus", "Bridge {soft}"],
    );
    let calls = clip_commands(&clips);
    assert_eq!(
        calls,
        vec![vec![
            json!({"target": "live_set tracks 1 arrangement_clips 1", "name": "get_prop",
                   "args": {"prop": "start_time"}}),
            json!({"target": "live_set tracks 1 arrangement_clips 3", "name": "get_prop",
                   "args": {"prop": "start_time"}}),
        ]]
    );
    let many: Vec<(String, String)> = (0..201)
        .map(|i| (format!("c {i}"), "Key {A}".to_string()))
        .collect();
    let sizes: Vec<usize> = clip_commands(&many).iter().map(Vec::len).collect();
    assert_eq!(sizes, [200, 1]);
}

#[test]
fn the_first_call_reads_the_locators_the_sections_clips_and_the_tracks() {
    assert_eq!(SECTIONS, "live_set tracks[name=Sections]");
    assert_eq!(
        first_commands(),
        vec![
            json!({"target": "live_set", "name": "get_prop", "args": {"prop": "cue_points"}}),
            json!({"target": "live_set tracks[name=Sections]", "name": "get_prop",
                   "args": {"prop": "arrangement_clips"}}),
            json!({"target": "live_set", "name": "get_prop", "args": {"prop": "tracks"}}),
        ]
    );
    assert_eq!(SONGMAP_CLIENT, u64::MAX - 4);
}

#[test]
fn the_first_answer_lists_the_locators_and_the_sections_track() {
    let slots = [
        ok(objects(
            "live_set cue_points",
            "CuePoint",
            &["001 A {C}", "SONG END"],
        )),
        ok(objects(
            "live_set tracks 1 arrangement_clips",
            "Clip",
            &["Key {G}"],
        )),
        ok(objects(
            "live_set tracks",
            "Track",
            &["Click", "Sections", "Bass"],
        )),
    ];
    assert_eq!(
        listed(&slots),
        Ok(Listed {
            cues: listed_pairs("live_set cue_points", &["001 A {C}", "SONG END"]),
            sections: SectionsState::One,
            track: Some("live_set tracks 1".into()),
            clips: Some(listed_pairs(
                "live_set tracks 1 arrangement_clips",
                &["Key {G}"]
            )),
        })
    );
}

#[test]
fn a_set_without_sections_has_spans_and_no_changes() {
    let slots = [
        ok(objects(
            "live_set cue_points",
            "CuePoint",
            &["001 A {C}", "SONG END"],
        )),
        refused("not found: tracks[name=Sections]"),
        ok(objects(
            "live_set tracks",
            "Track",
            &["Click", "Sections 2"],
        )),
    ];
    let first = listed(&slots).unwrap();
    assert_eq!(
        (first.sections, first.track.clone(), first.clips.clone()),
        (SectionsState::None, None, None)
    );
    // Its map: the song's span, no change, no problem.
    let cues = cues_of(&first.cues, &[ok(json!(0.0)), ok(json!(16.0))]).unwrap();
    let map = fohmixer_proto::songmap::build(&cues, &[]);
    assert_eq!(map.spans.len(), 1);
    assert!(map.spans[0].changes.is_empty());
    assert!(map.problems.is_empty());
}

#[test]
fn the_report_names_a_second_sections_track() {
    let slots = [
        ok(objects("live_set cue_points", "CuePoint", &[])),
        refused("ambiguous: tracks[name=Sections]"),
        ok(objects(
            "live_set tracks",
            "Track",
            &["Click", "Sections", "Bass", "Sections"],
        )),
    ];
    let first = listed(&slots).unwrap();
    assert_eq!(first.sections, SectionsState::FirstOf(2));
    assert_eq!(
        first.track.as_deref(),
        Some("live_set tracks 1"),
        "the first one's clips are read by its index"
    );
    assert_eq!(first.clips, None);
    assert_eq!(
        serde_json::to_value(first.sections).unwrap(),
        json!({"first_of": 2})
    );
}

#[test]
fn a_first_answer_without_the_locators_or_the_tracks_fails_the_read() {
    let tracks = ok(objects("live_set tracks", "Track", &[]));
    assert_eq!(
        listed(&[refused("no such property"), ok(json!([])), tracks.clone()]),
        Err("cue_points: no such property".to_string())
    );
    assert_eq!(
        listed(&[ok(json!([])), ok(json!([])), refused("gone")]),
        Err("tracks: gone".to_string())
    );
    assert_eq!(
        listed(&[ok(json!([])), ok(json!([]))]),
        Err("tracks: no result".to_string())
    );
    assert_eq!(listed(&[]), Err("cue_points: no result".to_string()));
}

#[test]
fn sections_state_counts_the_tracks_named_sections() {
    assert_eq!(sections_state(0), SectionsState::None);
    assert_eq!(sections_state(1), SectionsState::One);
    assert_eq!(sections_state(2), SectionsState::FirstOf(2));
    assert_eq!(sections_state(5), SectionsState::FirstOf(5));
}

#[test]
fn the_times_read_go_to_the_cues_that_needed_them() {
    let cues = listed_pairs(
        "live_set cue_points",
        &["001 A {C}", "> Verse", "SONG END", "* TS"],
    );
    assert_eq!(
        cues_of(&cues, &[ok(json!(0.0)), ok(json!(64.5))]),
        Ok(vec![
            Cue {
                name: "001 A {C}".into(),
                time: Some(0.0)
            },
            Cue {
                name: "> Verse".into(),
                time: None
            },
            Cue {
                name: "SONG END".into(),
                time: Some(64.5)
            },
            Cue {
                name: "* TS".into(),
                time: None
            },
        ])
    );
    assert_eq!(
        cues_of(&cues, &[ok(json!(0.0)), refused("not found: cue_points 2")]),
        Err("live_set cue_points 2 time: not found: cue_points 2".to_string())
    );
    assert_eq!(
        cues_of(&cues, &[ok(json!(0.0))]),
        Err("live_set cue_points 2 time: no result".to_string())
    );
    assert_eq!(
        cues_of(&cues, &[ok(json!(0.0)), ok(json!("x"))]),
        Err("live_set cue_points 2 time: not a number".to_string())
    );
    let clips = listed_pairs("live_set tracks 1 arrangement_clips", &["Verse", "Key {G}"]);
    assert_eq!(
        clips_of(&clips, &[ok(json!(22897.0))]),
        Ok(vec![
            ClipName {
                name: "Verse".into(),
                start: None
            },
            ClipName {
                name: "Key {G}".into(),
                start: Some(22897.0)
            },
        ])
    );
    assert_eq!(
        clips_of(&clips, &[]),
        Err("live_set tracks 1 arrangement_clips 1 start_time: no result".to_string())
    );
}

#[test]
fn a_trigger_starts_a_read_or_marks_one_more_once() {
    assert_eq!(next_read(false, false), Some(Trigger::Read));
    assert_eq!(next_read(false, true), Some(Trigger::Read));
    assert_eq!(next_read(true, false), Some(Trigger::Again));
    assert_eq!(next_read(true, true), None);
    assert!(transport_reads(Some(false)));
    assert!(transport_reads(Some(true)));
    assert!(!transport_reads(None));
}

/// A router, its own messages, its records and its data folder.
type Online = (
    Router,
    UnboundedReceiver<RouterMsg>,
    std::sync::mpsc::Receiver<Value>,
    tempfile::TempDir,
);

/// A router whose band is connected and whose lists are followed (their
/// first values not delivered yet).
fn online() -> Online {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, rx, events) = offline_router(dir.path());
    connected(&mut router);
    let _ = router.subs.drain_outgoing();
    (router, rx, events, dir)
}

#[tokio::test]
async fn a_trigger_during_a_read_reads_again_once_after_it() {
    let (mut router, _rx, _events, _dir) = online();
    router.songmap_delivery(vec![ValueItem::value(&cue_key(), json!([]), None)]);
    assert_eq!(router.songmap.reads, 1);
    // Two list changes while it runs: one more read, after it.
    router.songmap_delivery(vec![ValueItem::value(&cue_key(), json!([1]), None)]);
    router.songmap_delivery(vec![ValueItem::value(&tracks_key(), json!([2]), None)]);
    assert_eq!(router.songmap.reads, 1);
    assert!(router.songmap.dirty.contains("band"));
    answer(&mut router, 1, Ok(listing()));
    assert_eq!(router.songmap.reads, 2, "the one more read");
    assert_eq!(router.songmap.reading.get("band"), Some(&2));
    assert!(router.songmap.dirty.is_empty());
    answer(&mut router, 2, Ok(listing()));
    assert_eq!(router.songmap.reads, 2, "and no other");
    assert!(router.songmap.reading.is_empty());
    // A failed read keeps the map and also reads once more when asked.
    router.songmap_trigger("band");
    router.songmap_trigger("band");
    answer(&mut router, 3, Err("Live did not answer".into()));
    assert_eq!(router.songmap.reads, 4);
    assert_eq!(router.songmap.maps["band"].read_no, 2);
}

#[tokio::test]
async fn deliveries_of_one_flush_read_once_and_others_read_nothing() {
    let (mut router, _rx, _events, _dir) = online();
    router.songmap_delivery(vec![
        ValueItem::value(&cue_key(), json!([]), None),
        ValueItem::value(&tracks_key(), json!([]), None),
        ValueItem::value("band|live_set|tempo|false", json!(120.0), None),
    ]);
    assert_eq!(router.songmap.reads, 1);
    assert!(router.songmap.dirty.is_empty(), "one trigger per set");
    // A key the song map does not hold is no trigger.
    router.songmap.reading.clear();
    router.songmap_delivery(vec![ValueItem::value(
        "band|live_set|tempo|false",
        json!(121.0),
        None,
    )]);
    assert_eq!(router.songmap.reads, 1);
    // An error of a list (the listener gone) is a trigger too.
    router.songmap_delivery(vec![ValueItem::error(&cue_key(), "gone")]);
    assert_eq!(router.songmap.reads, 2);
}

#[tokio::test]
async fn a_read_waits_for_its_set_to_be_online() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _rx, _events) = offline_router(dir.path());
    router.songmap_trigger("band");
    assert_eq!(router.songmap.reads, 0);
    assert!(router.songmap.reading.is_empty());
    // An unknown set never reads.
    router.states.get_mut("band").unwrap().online = true;
    router.songmap_trigger("master");
    assert_eq!(router.songmap.reads, 0);
    router.songmap_trigger("band");
    assert_eq!(router.songmap.reads, 1);
}

#[tokio::test]
async fn the_sections_clips_are_followed_by_the_index_a_read_found() {
    let (mut router, _rx, _events, _dir) = online();
    router.songmap_trigger("band");
    answer(&mut router, 1, Ok(listing()));
    let first = clips_key("live_set tracks 1");
    assert_eq!(
        router.songmap.clips.get("band"),
        Some(&(
            first.clone(),
            "live_set tracks 1 arrangement_clips".to_string()
        ))
    );
    assert!(router.songmap.subs.contains_key(&first));
    assert_eq!(router.subs.subscriptions("band"), 4);
    // Its first value names the clips the read saw: the read's own, no read.
    assert_eq!(
        router.songmap.fresh.get(&first),
        Some(&vec!["Verse".to_string(), "Key {G}".to_string()])
    );
    router.songmap_delivery(vec![ValueItem::value(
        &first,
        json!([{"name": "Verse"}, {"name": "Key {G}"}]),
        None,
    )]);
    assert_eq!(router.songmap.reads, 1);
    assert!(router.songmap.fresh.is_empty());
    // A later one is a change: a read.
    router.songmap_delivery(vec![ValueItem::value(&first, json!([1]), None)]);
    assert_eq!(router.songmap.reads, 2);
    // The track moved: the clips are followed at its new index.
    let moved = listing_at(Some("live_set tracks 3"), SectionsState::One);
    answer(&mut router, 2, Ok(moved));
    let second = clips_key("live_set tracks 3");
    assert_eq!(
        router.songmap.clips.get("band").map(|(k, _)| k.clone()),
        Some(second.clone())
    );
    assert!(!router.songmap.subs.contains_key(&first));
    assert_eq!(router.subs.subscriptions("band"), 4, "the old one let go");
    // An error as its first value is a trigger (and it is fresh no more).
    router.songmap_delivery(vec![ValueItem::error(&second, "not found")]);
    assert_eq!(router.songmap.reads, 3);
    router.songmap_delivery(vec![ValueItem::value(&second, json!([]), None)]);
    assert!(router.songmap.dirty.contains("band"), "a change: once more");
    // A set without the track any more: none followed.
    let none = listing_at(None, SectionsState::None);
    answer(&mut router, 3, Ok(none));
    assert_eq!(router.songmap.clips.get("band"), None);
    assert!(!router.songmap.subs.contains_key(&second));
    assert_eq!(router.subs.subscriptions("band"), 3);
}

#[test]
fn a_first_value_is_the_reads_own_only_when_it_names_the_same_clips() {
    let names = vec!["Verse".to_string(), "Key {G}".to_string()];
    let value = |names: &[&str]| -> Value {
        names
            .iter()
            .map(|n| json!({"$ref": "live_1", "class": "Clip", "name": n}))
            .collect()
    };
    assert!(same_names(Some(&value(&["Verse", "Key {G}"])), &names));
    assert!(!same_names(
        Some(&value(&["Verse", "Key {G}", "Key {A}"])),
        &names
    ));
    assert!(!same_names(Some(&value(&["Verse"])), &names));
    assert!(!same_names(Some(&value(&["Key {G}", "Verse"])), &names));
    assert!(!same_names(Some(&value(&["Verse", "Key {A}"])), &names));
    assert!(!same_names(Some(&json!([{"name": "Verse"}, {}])), &names));
    assert!(!same_names(None, &names));
    assert!(!same_names(Some(&json!("Verse")), &names));
    assert!(same_names(Some(&json!([])), &[]));
}

#[tokio::test]
async fn a_clip_added_before_the_clips_were_followed_still_reads() {
    let (mut router, _rx, _events, _dir) = online();
    router.songmap_trigger("band");
    answer(&mut router, 1, Ok(listing()));
    let key = clips_key("live_set tracks 1");
    // The listener's first value already holds a clip the read did not see.
    router.songmap_delivery(vec![ValueItem::value(
        &key,
        json!([{"name": "Verse"}, {"name": "Key {G}"}, {"name": "Key {A}"}]),
        None,
    )]);
    assert_eq!(router.songmap.reads, 2);
    assert!(router.songmap.fresh.is_empty());
}

#[tokio::test]
async fn clips_a_page_already_follows_give_no_first_value() {
    let (mut router, _rx, _events, _dir) = online();
    // A page follows the Sections clips; Live answers its listener.
    router.handle(RouterMsg::Sub {
        client: 1,
        instance: "band".into(),
        target: "live_set tracks 1".into(),
        prop: CLIPS.into(),
        display: false,
    });
    for o in router.subs.drain_outgoing() {
        let slots: Vec<Value> = o
            .commands
            .iter()
            .map(|c| match c["args"]["prop"].as_str().unwrap_or("") {
                CLIPS => ok(json!({"key": "live_9.arrangement_clips", "value": []})),
                prop => ok(json!({"key": format!("live_8.{prop}"), "value": []})),
            })
            .collect();
        router.subs.on_result(&o.instance, &o.uuid, &slots);
    }
    let _ = router.subs.take_deliveries();
    router.songmap_trigger("band");
    answer(&mut router, 1, Ok(listing()));
    let key = clips_key("live_set tracks 1");
    assert_eq!(
        router.songmap.clips.get("band").map(|(k, _)| k.clone()),
        Some(key.clone())
    );
    assert!(router.songmap.fresh.is_empty(), "its value is the table's");
    router.songmap_delivery(vec![ValueItem::value(&key, json!([1]), None)]);
    assert_eq!(router.songmap.reads, 2, "the next value is a change");
}

#[tokio::test]
async fn a_disconnect_voids_the_read_in_flight_and_keeps_the_last_map_stale() {
    let (mut router, _rx, events, _dir) = online();
    router.songmap_trigger("band");
    answer(&mut router, 1, Ok(listing()));
    let _ = songmap_records(&events);
    router.songmap_trigger("band");
    router.songmap_trigger("band");
    assert_eq!(router.songmap.reading.get("band"), Some(&2));
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    assert!(router.songmap.reading.is_empty());
    assert!(router.songmap.dirty.is_empty(), "nothing more is asked");
    // The read's answer after it is void: the last map stays, stale.
    let other = Listing {
        cues: Vec::new(),
        clips: Vec::new(),
        sections: SectionsState::None,
        track: None,
    };
    answer(&mut router, 2, Ok(other));
    assert_eq!(songmap_records(&events), Vec::<Value>::new());
    assert_eq!(router.songmap.reads, 2);
    let now = status(&mut router).songmap;
    assert_eq!((now[0].songs, now[0].stale), (2, true));
    let stale = report(&mut router, "band").unwrap();
    assert_eq!((stale.read_no, stale.stale), (1, true));
    // The clips' first value after the reconnect reads.
    let first = clips_key("live_set tracks 1");
    assert!(!router.songmap.fresh.contains_key(&first));
    // The next read replaces it, fresh.
    connected(&mut router);
    router.songmap_trigger("band");
    answer(&mut router, 3, Ok(listing()));
    let now = status(&mut router).songmap;
    assert!(!now[0].stale);
    assert_eq!(report(&mut router, "band").unwrap().read_no, 3);
}

#[tokio::test]
async fn a_disconnect_before_a_first_read_marks_nothing_stale() {
    let (mut router, _rx, _events, _dir) = online();
    router.handle(RouterMsg::Live {
        instance: "band".into(),
        event: LiveEvent::Disconnected,
    });
    assert!(router.songmap.stale.is_empty());
    assert_eq!(report(&mut router, "band"), Err(Missing::NotRead));
}

#[tokio::test]
async fn a_failed_read_keeps_the_map_and_is_recorded_warn() {
    let (mut router, _rx, events, _dir) = online();
    router.songmap_trigger("band");
    answer(&mut router, 1, Ok(listing()));
    let _ = songmap_records(&events);
    router.songmap_trigger("band");
    answer(&mut router, 2, Err("Live did not answer".into()));
    let got = songmap_records(&events);
    assert_eq!(
        got,
        vec![json!({"instance": "band", "read_no": 2, "ms": 7, "error": "Live did not answer"})]
    );
    assert_eq!(report(&mut router, "band").unwrap().read_no, 1);
    assert!(crate::events::is_warn(
        &json!({"ev": "songmap", "error": "Live did not answer"})
    ));
}

/// A set read whose map has one problem: song 1 is not ended.
fn listing_with_a_problem() -> Listing {
    Listing {
        cues: vec![Cue {
            name: "001 Test One {C 120}".into(),
            time: Some(0.0),
        }],
        clips: Vec::new(),
        sections: SectionsState::None,
        track: None,
    }
}

#[tokio::test]
async fn a_read_with_problems_is_logged_at_warn() {
    // Review M1: the read's log line takes the class its record has as it
    // is written (with its `ev`), never the class of its bare fields.
    let (mut router, _rx, events, _dir) = online();
    router.songmap_trigger("band");
    let log = crate::ws::tests::logged(|| answer(&mut router, 1, Ok(listing_with_a_problem())));
    let got = songmap_records(&events);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["problems"], 1);
    assert!(log.contains(" WARN "), "{log}");
    assert!(log.contains("the song map"), "{log}");
    assert!(!log.contains(" INFO "), "{log}");
}

#[test]
fn a_reads_class_is_its_records_as_written() {
    let read = |listing: Listing| Read {
        map: fohmixer_proto::songmap::build(&listing.cues, &listing.clips),
        sections: listing.sections,
        at_ms: 1,
        read_no: 1,
    };
    assert!(songmap_warn(&songmap_fields(
        "band",
        &read(listing_with_a_problem()),
        3
    )));
    assert!(!songmap_warn(&songmap_fields("band", &read(listing()), 3)));
    assert!(songmap_warn(&songmap_error_fields(
        "band",
        2,
        "Live did not answer",
        3
    )));
}

#[tokio::test]
async fn a_transport_change_reads_again() {
    let (mut router, _rx, _events, _dir) = online();
    // A session's first transport value does not read (its connect does).
    router.on_transport(TRANSPORT, &json!(false));
    assert_eq!(router.songmap.reads, 0);
    router.on_transport(TRANSPORT, &json!(true));
    assert_eq!(router.songmap.reads, 1);
    answer(&mut router, 1, Ok(listing()));
    // The same value again is no change; a stop is.
    router.on_transport(TRANSPORT, &json!(true));
    assert_eq!(router.songmap.reads, 1);
    router.on_transport(TRANSPORT, &json!(false));
    assert_eq!(router.songmap.reads, 2);
}

#[test]
fn the_report_answers_an_unknown_set_a_set_not_read_and_a_map() {
    let read = Read {
        map: fohmixer_proto::songmap::build(&listing().cues, &listing().clips),
        sections: SectionsState::FirstOf(2),
        at_ms: 1_790_000_000_000,
        read_no: 4,
    };
    assert_eq!(
        report_of("x", false, Some(&read), false),
        Err(Missing::Unknown)
    );
    assert_eq!(report_of("band", true, None, false), Err(Missing::NotRead));
    let got = report_of("band", true, Some(&read), true).unwrap();
    assert_eq!(
        serde_json::to_value(&got).unwrap(),
        json!({
            "instance": "band", "read_no": 4, "at_ms": 1_790_000_000_000_u64, "stale": true,
            "sections": {"first_of": 2},
            "counts": {"songs": 2, "keyed": 1, "changes": 1, "problems": 0},
            "spans": [
                {"number": 1, "start": 0.0, "end": 100.0,
                 "key": {"root": 0, "minor": false, "name": "C"},
                 "changes": [{"time": 50.0, "key": {"root": 7, "minor": false, "name": "G"}}]},
                {"number": 2, "start": 120.0, "end": 200.0, "key": null, "changes": []}
            ],
            "problems": []
        })
    );
    // Problems carry their words.
    let keyless = Listing {
        cues: listing().cues,
        clips: vec![ClipName {
            name: "Key {D}".into(),
            start: Some(150.0),
        }],
        sections: SectionsState::One,
        track: None,
    };
    let read = Read {
        map: fohmixer_proto::songmap::build(&keyless.cues, &keyless.clips),
        sections: SectionsState::One,
        at_ms: 1,
        read_no: 5,
    };
    let got = report_of("band", true, Some(&read), false).unwrap();
    assert_eq!(
        serde_json::to_value(&got.problems).unwrap(),
        json!([{"kind": "clip_in_keyless_song", "song": 2, "time": 150.0,
                "text": "song 002: the key clip \"Key {D}\" lies in a song without a key (beat 150)"}])
    );
    assert!(!got.stale);
}

#[test]
fn the_record_and_the_status_sum_a_read_up() {
    let cue = |name: &str, time: f64| Cue {
        name: name.into(),
        time: Some(time),
    };
    // 25 songs, each not ended: 25 problems, 20 texts in the record.
    let cues: Vec<Cue> = (1..=25)
        .map(|n| cue(&format!("{n:03} S {{C}}"), f64::from(n)))
        .collect();
    let read = Read {
        map: fohmixer_proto::songmap::build(&cues, &[]),
        sections: SectionsState::None,
        at_ms: 1_790_000_000_000,
        read_no: 9,
    };
    let fields = songmap_fields("band", &read, 31);
    assert_eq!(fields["problems"], 25);
    assert_eq!(fields["problem_texts"].as_array().unwrap().len(), MAX_TEXTS);
    assert_eq!(MAX_TEXTS, 20);
    assert_eq!(
        fields["problem_texts"][0],
        "song 001: not ended by SONG END, STOP or AUTOSTOP (beat 1)"
    );
    assert_eq!(
        fields["problem_texts"][19],
        "song 020: not ended by SONG END, STOP or AUTOSTOP (beat 20)"
    );
    let mut rest = fields.clone();
    rest.as_object_mut().unwrap().remove("problem_texts");
    assert_eq!(
        rest,
        json!({"instance": "band", "read_no": 9, "songs": 25, "keyed": 0, "changes": 0,
               "problems": 25, "sections": "none", "ms": 31})
    );
    assert!(crate::events::is_warn(&{
        let mut r = fields.clone();
        r["ev"] = json!("songmap");
        r
    }));
    assert_eq!(
        songmap_error_fields("band", 3, "offline", 2),
        json!({"instance": "band", "read_no": 3, "ms": 2, "error": "offline"})
    );
    let s = status_of("band", &read, true);
    assert_eq!(
        (
            s.instance.as_str(),
            s.songs,
            s.keyed,
            s.changes,
            s.problems,
            s.sections,
            s.at_ms,
            s.stale
        ),
        (
            "band",
            25,
            0,
            0,
            25,
            SectionsState::None,
            1_790_000_000_000,
            true
        )
    );
    assert_eq!(
        fohmixer_proto::songmap::counts(&read.map),
        Counts {
            songs: 25,
            keyed: 0,
            changes: 0,
            problems: 25
        }
    );
}

#[test]
fn the_key_names_of_the_report_are_autotunes() {
    // The map's keys are Key values; a page or the CLI names them.
    let map = fohmixer_proto::songmap::build(&listing().cues, &listing().clips);
    let key: Key = map.spans[0].changes[0].key;
    assert_eq!(key.name(), "G");
    assert_eq!(
        map.problems
            .iter()
            .map(|p| p.kind)
            .collect::<Vec<ProblemKind>>(),
        vec![]
    );
}
