//! The song map end to end (#140 PR 1) on SimLive: a set of our own
//! (invented titles) with locators and a `Sections` track; the hub reads
//! its songs, spans, keys and key clips at the connect, again when a
//! locator or a key clip is added in Live, and at every transport change
//! (a rename in Live shows only then); a big set reads in chunks; two
//! tracks named Sections read the first one's clips by its index; and the
//! hub writes nothing to Live for it.
#![cfg(unix)]

mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use support::{Client, Host, TestHub, runtime, serial};

const SECS_5: Duration = Duration::from_secs(5);
const SONGMAP: &str = "/api/songmap?instance=band";

/// The test's own set: two songs ended by SONG END, an ignored locator and
/// a section, and a Sections track (the second) with a section clip and a
/// key clip inside song 1.
fn site(dir: &Path) -> PathBuf {
    let set = json!({"name": "Song Map Site",
        "tracks": [
            {"name": "Click #"},
            {"name": "Sections", "arrangement_clips": [
                {"name": "Verse", "start_time": 8.0, "end_time": 40.0},
                {"name": "Key {G}", "start_time": 40.0, "end_time": 41.0}]}],
        "cue_points": [
            {"name": "001 Test One {C 120 4/4}", "time": 0.0},
            {"name": "* TS", "time": 4.0},
            {"name": "> Ending", "time": 64.0},
            {"name": "SONG END", "time": 96.0},
            {"name": "002 Test Two {F#m 90}", "time": 128.0},
            {"name": "SONG END", "time": 200.0}]});
    let path = dir.join("songmap-site.json");
    std::fs::write(&path, set.to_string()).unwrap();
    path
}

/// A set of `songs` songs, each ended by SONG END.
fn big_site(dir: &Path, songs: u32) -> PathBuf {
    let cues: Vec<Value> = (0..songs)
        .flat_map(|n| {
            let start = f64::from(n) * 10.0;
            [
                json!({"name": format!("{n:03} Song {{C 100}}"), "time": start}),
                json!({"name": "SONG END", "time": start + 5.0}),
            ]
        })
        .collect();
    let set = json!({"name": "Big Site", "tracks": [{"name": "Click #"}], "cue_points": cues});
    let path = dir.join("big-site.json");
    std::fs::write(&path, set.to_string()).unwrap();
    path
}

/// Polls `/api/songmap` until it answers a map `check` takes (`limit` at
/// most).
async fn songmap_until(hub: &TestHub, limit: Duration, check: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + limit;
    loop {
        let (code, body) = hub.get(SONGMAP).await;
        if code == 200 && check(&body) {
            return body;
        }
        assert!(
            Instant::now() < deadline,
            "the song map never matched: {code} {body}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A span's key as `(root, minor)`, none for a keyless span.
fn key(span: &Value) -> Option<(u64, bool)> {
    let key = &span["key"];
    Some((key["root"].as_u64()?, key["minor"].as_bool()?))
}

/// Starts or stops SimLive's transport.
async fn transport(c: &mut Client, verb: &str) {
    let slots = c
        .cmd(
            "band",
            vec![json!({"target": "live_set", "name": verb, "args": []})],
        )
        .await
        .unwrap();
    assert_eq!(slots[0]["ok"], json!(true), "{slots:?}");
}

/// The locators' names and times and the Sections clips' names and starts,
/// read with commands.
async fn set_contents(c: &mut Client) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    for (list, prop) in [
        ("live_set cue_points", "time"),
        ("live_set tracks 1 arrangement_clips", "start_time"),
    ] {
        let (target, attr) = list.rsplit_once(' ').unwrap();
        let items = c.get("band", target, attr).await;
        for (i, item) in items.as_array().unwrap().iter().enumerate() {
            let time = c.get("band", &format!("{list} {i}"), prop).await;
            out.push((
                item["name"].as_str().unwrap().to_string(),
                time.as_f64().unwrap(),
            ));
        }
    }
    out
}

#[test]
fn the_hub_serves_the_sites_song_map() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let host = Host::start_site("band", &site(dir.path()), 0, 0.0);
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let map = songmap_until(&hub, SECS_5, |m| m["spans"].as_array().is_some()).await;
        let spans = map["spans"].as_array().unwrap();
        assert_eq!(spans.len(), 2, "{map}");
        assert_eq!(
            (&spans[0]["number"], &spans[0]["start"], &spans[0]["end"]),
            (&json!(1), &json!(0.0), &json!(96.0))
        );
        assert_eq!(key(&spans[0]), Some((0, false)), "C");
        assert_eq!(
            spans[0]["changes"],
            json!([{"time": 40.0, "key": {"root": 7, "minor": false, "name": "G"}}]),
            "the change to G at its clip's exact start"
        );
        assert_eq!(key(&spans[1]), Some((6, true)), "F# minor");
        assert_eq!(
            (&spans[0]["key"]["name"], &spans[1]["key"]["name"]),
            (&json!("C"), &json!("F#m")),
            "each key named as Auto-Tune spells it"
        );
        assert_eq!(
            (&spans[1]["start"], &spans[1]["end"]),
            (&json!(128.0), &json!(200.0))
        );
        assert_eq!(map["problems"], json!([]));
        assert_eq!(map["sections"], json!("one"));
        assert_eq!(map["stale"], json!(false));
        let status = hub.status_until(SECS_5, |s| !s.songmap.is_empty()).await;
        let band = &status.songmap[0];
        assert_eq!(
            (
                band.instance.as_str(),
                band.songs,
                band.keyed,
                band.changes,
                band.problems
            ),
            ("band", 2, 2, 1, 0)
        );
        // The read is recorded, bulk: no problem.
        let records = hub
            .events_until(SECS_5, |r| r.iter().any(|r| r["ev"] == "songmap"))
            .await;
        let read = records.iter().find(|r| r["ev"] == "songmap").unwrap();
        assert_eq!(
            (
                &read["songs"],
                &read["keyed"],
                &read["changes"],
                &read["problems"]
            ),
            (&json!(2), &json!(2), &json!(1), &json!(0))
        );
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_locator_added_in_live_is_in_the_next_map() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut host = Host::start_site("band", &site(dir.path()), 0, 0.0);
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        songmap_until(&hub, SECS_5, |m| {
            m["spans"].as_array().is_some_and(|s| s.len() == 2)
        })
        .await;
        assert_eq!(host.cue("add 300 003 Test Three {D 100}"), 7);
        assert_eq!(host.cue("add 400 SONG END"), 8);
        let map = songmap_until(&hub, SECS_5, |m| {
            m["spans"].as_array().is_some_and(|s| s.len() == 3) && m["problems"] == json!([])
        })
        .await;
        let third = &map["spans"][2];
        assert_eq!(
            (&third["number"], &third["start"], &third["end"]),
            (&json!(3), &json!(300.0), &json!(400.0))
        );
        assert_eq!(key(third), Some((2, false)), "D");
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_key_clip_added_in_live_is_in_the_next_map() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut host = Host::start_site("band", &site(dir.path()), 0, 0.0);
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        songmap_until(&hub, SECS_5, |m| m["spans"].as_array().is_some()).await;
        // A key change in song 2, on the Sections track (track 1).
        assert_eq!(host.clip("add 1 150.5 151 Key {A}"), 3);
        let map = songmap_until(&hub, SECS_5, |m| {
            m["spans"][1]["changes"]
                .as_array()
                .is_some_and(|c| c.len() == 1)
        })
        .await;
        assert_eq!(
            map["spans"][1]["changes"],
            json!([{"time": 150.5, "key": {"root": 9, "minor": false, "name": "A"}}])
        );
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_rename_is_read_at_the_next_transport_change() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut host = Host::start_site("band", &site(dir.path()), 0, 0.0);
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut c = hub.client().await;
        let first = songmap_until(&hub, SECS_5, |m| m["spans"].as_array().is_some()).await;
        assert_eq!(key(&first["spans"][0]), Some((0, false)));
        // The hub hears its transport before the rename (a session's first
        // value reads nothing).
        hub.status_until(SECS_5, |s| s.instances[0].playing.is_some())
            .await;
        assert_eq!(host.cue("rename 0 001 Test One {Eb 120}"), 6);
        // A rename fires no listener the hub follows: the map stays.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let (_, still) = hub.get(SONGMAP).await;
        assert_eq!(still["read_no"], first["read_no"], "no read since");
        assert_eq!(key(&still["spans"][0]), Some((0, false)));
        transport(&mut c, "start_playing").await;
        let renamed =
            songmap_until(&hub, SECS_5, |m| key(&m["spans"][0]) == Some((3, false))).await;
        assert!(renamed["read_no"].as_u64() > first["read_no"].as_u64());
        transport(&mut c, "stop_playing").await;
        c.close().await;
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_big_set_reads_in_chunks() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let host = Host::start_site("band", &big_site(dir.path(), 450), 0, 0.0);
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let map = songmap_until(&hub, Duration::from_secs(10), |m| {
            m["spans"].as_array().is_some_and(|s| s.len() == 450)
        })
        .await;
        assert_eq!(map["problems"], json!([]));
        assert_eq!(map["sections"], json!("none"));
        assert_eq!(
            (&map["spans"][449]["number"], &map["spans"][449]["end"]),
            (&json!(449), &json!(4495.0))
        );
        let records = hub
            .events_until(SECS_5, |r| {
                r.iter().any(|r| r["ev"] == "songmap" && r["songs"] == 450)
            })
            .await;
        let read = records
            .iter()
            .find(|r| r["ev"] == "songmap" && r["songs"] == 450)
            .unwrap();
        assert_eq!(
            (&read["keyed"], &read["problems"]),
            (&json!(450), &json!(0))
        );
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn the_hub_never_writes_to_live_for_the_map() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut host = Host::start_site("band", &site(dir.path()), 0, 0.0);
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut c = hub.client().await;
        let first = songmap_until(&hub, SECS_5, |m| m["spans"].as_array().is_some()).await;
        let before = set_contents(&mut c).await;
        assert_eq!(
            before,
            [
                ("001 Test One {C 120 4/4}".to_string(), 0.0),
                ("* TS".to_string(), 4.0),
                ("> Ending".to_string(), 64.0),
                ("SONG END".to_string(), 96.0),
                ("002 Test Two {F#m 90}".to_string(), 128.0),
                ("SONG END".to_string(), 200.0),
                ("Verse".to_string(), 8.0),
                ("Key {G}".to_string(), 40.0),
            ]
        );
        // Two more reads (a start and a stop).
        hub.status_until(SECS_5, |s| s.instances[0].playing.is_some())
            .await;
        transport(&mut c, "start_playing").await;
        transport(&mut c, "stop_playing").await;
        songmap_until(&hub, SECS_5, |m| {
            m["read_no"].as_u64() >= first["read_no"].as_u64().map(|n| n + 2)
        })
        .await;
        assert_eq!(
            set_contents(&mut c).await,
            before,
            "nothing in the set moved"
        );
        // The hub listens to the lists, never to a locator or a clip.
        assert_eq!(host.listeners("cue_points", "live_set"), 1);
        assert_eq!(host.listeners("arrangement_clips", "live_set tracks 1"), 1);
        assert_eq!(host.listeners("name", "live_set cue_points 0"), 0);
        assert_eq!(host.listeners("time", "live_set cue_points 0"), 0);
        assert_eq!(
            host.listeners("name", "live_set tracks 1 arrangement_clips 1"),
            0
        );
        // No page wrote, so no write reached the event log.
        let writes: Vec<Value> = hub
            .events()
            .into_iter()
            .filter(|r| matches!(r["ev"].as_str(), Some("set" | "batch" | "applied")))
            .collect();
        assert_eq!(writes, Vec::<Value>::new());
        c.close().await;
        hub.stop().await;
        host.stop();
    });
}

/// A set with two tracks named Sections (#140 review M9 a): the first
/// (track 1) with a key clip in song 1, the second (track 3) with another;
/// AbleSet takes the first, and so does the hub.
fn two_sections_site(dir: &Path) -> PathBuf {
    let set = json!({"name": "Two Sections Site",
        "tracks": [
            {"name": "Click #"},
            {"name": "Sections", "arrangement_clips": [
                {"name": "Key {G}", "start_time": 40.0, "end_time": 41.0}]},
            {"name": "Bass #"},
            {"name": "Sections", "arrangement_clips": [
                {"name": "Key {A}", "start_time": 20.0, "end_time": 21.0}]}],
        "cue_points": [
            {"name": "001 Test One {C 120 4/4}", "time": 0.0},
            {"name": "SONG END", "time": 96.0}]});
    let path = dir.join("two-sections-site.json");
    std::fs::write(&path, set.to_string()).unwrap();
    path
}

#[test]
fn two_sections_tracks_read_the_first_ones_clips_by_its_index() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut host = Host::start_site("band", &two_sections_site(dir.path()), 0, 0.0);
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let mut c = hub.client().await;
        // The script's own answer to the name: ambiguous, never a guess.
        let by_name = json!({"target": "live_set tracks[name=Sections]", "name": "get_prop",
                             "args": {"prop": "arrangement_clips"}});
        let slots = c.cmd("band", vec![by_name]).await.unwrap();
        assert_eq!(slots[0]["ok"], json!(false), "{slots:?}");
        assert!(
            slots[0]["error"]
                .as_str()
                .is_some_and(|e| e.contains("ambiguous")),
            "{slots:?}"
        );
        // So the read took the first one's clips by its index.
        let map = songmap_until(&hub, SECS_5, |m| m["spans"].as_array().is_some()).await;
        assert_eq!(map["sections"], json!({"first_of": 2}));
        assert_eq!(
            map["spans"][0]["changes"],
            json!([{"time": 40.0, "key": {"root": 7, "minor": false, "name": "G"}}]),
            "the first Sections track's key clip, not the second's"
        );
        assert_eq!(map["problems"], json!([]));
        // Its clips are followed at that index, the second's are not.
        assert_eq!(host.listeners("arrangement_clips", "live_set tracks 1"), 1);
        assert_eq!(host.listeners("arrangement_clips", "live_set tracks 3"), 0);
        // A key clip added on the first is in the next map.
        assert_eq!(host.clip("add 1 60 61 Key {D}"), 2);
        let map = songmap_until(&hub, SECS_5, |m| {
            m["spans"][0]["changes"]
                .as_array()
                .is_some_and(|c| c.len() == 2)
        })
        .await;
        assert_eq!(
            map["spans"][0]["changes"][1],
            json!({"time": 60.0, "key": {"root": 2, "minor": false, "name": "D"}})
        );
        assert_eq!(map["sections"], json!({"first_of": 2}));
        c.close().await;
        hub.stop().await;
        host.stop();
    });
}
