use std::time::{Duration, Instant};

use fohmixer_proto::claps::{ClapsPhase, Word, parse};
use fohmixer_proto::client::hub_key;
use serde_json::json;

use super::super::tests::{attach, offline_router, records};
use super::*;

/// The Claps track of the tests: band track 4.
fn track() -> ClapsTrack {
    ClapsTrack {
        instance: "band".into(),
        index: 4,
        track: "live_set tracks 4".into(),
        label: "CLAPS".into(),
    }
}

/// The hub key of the rule's subscription of `target`'s `prop`.
fn key(target: &str, prop: &str) -> String {
    hub_key("band", target, prop, false)
}

fn item(sub: &str, value: Value) -> ValueItem {
    ValueItem {
        sub: sub.to_string(),
        value: Some(value),
        display: None,
        error: None,
    }
}

/// Live's playhead `pos` heard at hub time `at_ms`.
fn heard(router: &mut Router, pos: f64, at_ms: f64) {
    router.claps_delivery(vec![item(&key("live_set", SONG_TIME), json!(pos))], at_ms);
}

/// Live's transport as the rules hear it.
fn transport(router: &mut Router, playing: bool) {
    router.transport.get_mut("band").expect("band").playing = Some(playing);
    router.claps_transport("band", Some(playing));
}

/// A device socket the router sends to, and the router told so.
fn device(router: Router) -> (Router, std::net::UdpSocket) {
    let socket = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let port = socket.local_addr().unwrap().port();
    let router = router.with_claps(ClapsSetup {
        notes: vec![52, 57],
        device_port: port,
    });
    (router, socket)
}

/// The words the device got, in order (until a read times out).
fn words(socket: &std::net::UdpSocket, n: usize) -> Vec<Word> {
    let mut buf = [0u8; 512];
    (0..n)
        .map(|_| {
            let len = socket.recv(&mut buf).expect("a word");
            parse(&buf[..len]).expect("a word that reads")
        })
        .collect()
}

/// Live playing in 4/4 at 120 bpm, its Claps track followed.
fn ready(router: &mut Router) {
    router.claps_layout(Some(track()));
    router.claps_delivery(
        vec![
            item(&key("live_set", TEMPO), json!(120.0)),
            item(&key("live_set", NUMERATOR), json!(4)),
            item(&key("live_set", DENOMINATOR), json!(4)),
        ],
        0.0,
    );
    transport(router, true);
}

/// The steps of the `claps` records waiting.
fn steps(rx: &std::sync::mpsc::Receiver<Value>) -> Vec<Value> {
    records(rx)
        .into_iter()
        .filter(|r| r["ev"] == "claps")
        .collect()
}

/// The `claps` messages a page got.
fn told(page: &Outbox) -> Vec<ClapsState> {
    page.take()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| match m {
            ServerMsg::Claps(state) => Some(state),
            _ => None,
        })
        .collect()
}

#[test]
fn a_tap_is_the_page_time_through_the_offset_never_after_its_arrival() {
    assert_eq!(tap_ms(1000.0, 1300.0, Some(250.0)), 1250.0);
    assert_eq!(tap_ms(1000.0, 1300.0, Some(400.0)), 1300.0, "never late");
    assert_eq!(tap_ms(1000.0, 1300.0, None), 1300.0);
}

#[test]
fn every_clip_is_read_by_its_ref_three_props_each_in_chunks() {
    let clip = |i: usize| Object {
        path: format!("live_set tracks 4 arrangement_clips {i}"),
        name: String::new(),
        id: format!("{}", 100 + i),
        class: "Clip".into(),
    };
    let clips: Vec<Object> = (0..70).map(clip).collect();
    let calls = clip_commands(&clips);
    assert_eq!(
        calls.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![200, 10],
        "210 reads in calls of at most 200"
    );
    assert_eq!(
        calls[0][..3].to_vec(),
        vec![
            by_ref(&clips[0], "start_time"),
            by_ref(&clips[0], "end_time"),
            by_ref(&clips[0], "muted"),
        ]
    );
    assert!(clip_commands(&[]).is_empty());
}

#[test]
fn the_clips_read_back_and_a_slot_that_did_not_names_its_clip() {
    let clips = vec![
        Object {
            path: "c0".into(),
            name: String::new(),
            id: "1".into(),
            class: "Clip".into(),
        },
        Object {
            path: "c1".into(),
            name: String::new(),
            id: "2".into(),
            class: "Clip".into(),
        },
    ];
    let ok = |v: Value| json!({"ok": true, "data": v});
    let slots = vec![
        ok(json!(8.0)),
        ok(json!(16.0)),
        ok(json!(false)),
        ok(json!(32.0)),
        ok(json!(40.0)),
        ok(json!(1)),
    ];
    assert_eq!(
        clips_of(&clips, &slots),
        Ok(vec![
            Clip {
                start: 8.0,
                end: 16.0,
                muted: false
            },
            Clip {
                start: 32.0,
                end: 40.0,
                muted: true
            },
        ])
    );
    let mut bad = slots.clone();
    bad[4] = json!({"ok": false, "error": "gone"});
    let error = clips_of(&clips, &bad).unwrap_err();
    assert!(error.starts_with("c1 end_time: "), "{error}");
    let mut text = slots.clone();
    text[2] = ok(json!("no"));
    assert!(
        clips_of(&clips, &text)
            .unwrap_err()
            .starts_with("c0 muted: ")
    );
    let mut start = slots.clone();
    start[0] = ok(json!("x"));
    assert!(
        clips_of(&clips, &start)
            .unwrap_err()
            .starts_with("c0 start_time: ")
    );
    assert_eq!(
        clips_of(&clips, &slots[..5]),
        Err("5 answers for 2 clips".to_string())
    );
    assert_eq!(clips_of(&[], &[]), Ok(Vec::new()));
}

#[test]
fn every_step_has_its_words_and_only_trouble_warns() {
    for (step, words) in [
        ("tap", "claps: a tap"),
        ("refused", "claps: a tap or a dot refused"),
        ("beat", "claps: a beat dot switched"),
        ("stop", "claps: stopped from a page"),
        ("pattern", "claps: the device's pattern"),
        ("on", "claps: the device switched"),
        ("track", "claps: the Claps track"),
        ("clips", "claps: the Claps track's clips read"),
        ("device", "claps: the device"),
        ("listen", "claps: listening for the device"),
        ("other", "claps"),
    ] {
        assert_eq!(step_words(&json!({"step": step})), words);
    }
    assert!(step_words(&json!({"step": "silent"})).contains("is silent"));
    assert!(step_warn(&json!({"step": "refused"})));
    assert!(step_warn(&json!({"step": "clips", "error": "offline"})));
    assert!(step_warn(&json!({"step": "device", "gone": true})));
    assert!(!step_warn(&json!({"step": "clips", "error": null})));
    assert!(!step_warn(&json!({"step": "tap"})));
    assert!(!step_warn(&json!({"step": "device", "gone": false})));
}

#[tokio::test]
async fn a_claps_track_is_followed_and_a_layout_without_it_lets_go() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    let page = attach(&mut router, 1);
    assert_eq!(told(&page), vec![ClapsState::default()], "at attach");
    let before = router.subs.subscriptions("band");
    router.claps_layout(Some(track()));
    assert_eq!(router.subs.subscriptions("band"), before + 5);
    assert_eq!(
        router.claps.subs.get(&key("live_set tracks 4", CLIPS)),
        Some(&Part::Clips)
    );
    let got = steps(&rx);
    assert_eq!(got[0]["step"], "track");
    assert_eq!(got[0]["to"], "band CLAPS");
    assert_eq!(got[0]["track"], "live_set tracks 4");
    // The same track again: nothing.
    router.claps_layout(Some(track()));
    assert!(steps(&rx).is_empty());
    router.claps_layout(None);
    assert_eq!(router.subs.subscriptions("band"), before);
    assert!(router.claps.subs.is_empty());
    assert_eq!(steps(&rx)[0]["from"], "band CLAPS");
}

#[test]
fn a_track_is_known_by_its_set_and_label_not_its_index() {
    let at = |index: u32, label: &str, instance: &str| ClapsTrack {
        instance: instance.into(),
        index,
        track: format!("live_set tracks {index}"),
        label: label.into(),
    };
    let claps = at(4, "CLAPS", "band");
    assert_eq!(track_change(None, None), TrackChange::Same);
    assert_eq!(track_change(Some(&claps), Some(&claps)), TrackChange::Same);
    assert_eq!(
        track_change(Some(&claps), Some(&at(5, "CLAPS", "band"))),
        TrackChange::Moved
    );
    for other in [at(4, "OTHER", "band"), at(4, "CLAPS", "master")] {
        assert_eq!(
            track_change(Some(&claps), Some(&other)),
            TrackChange::Afresh,
            "{other:?}"
        );
    }
    assert_eq!(track_change(Some(&claps), None), TrackChange::Afresh);
    assert_eq!(track_change(None, Some(&claps)), TrackChange::Afresh);
}

#[tokio::test]
async fn a_layout_without_the_claps_track_switches_the_device_off() {
    let dir = tempfile::tempdir().unwrap();
    let (router, _own, rx) = offline_router(dir.path());
    let (mut router, socket) = device(router);
    ready(&mut router);
    router.claps_beat(1, 1, true);
    let sent = words(&socket, 8);
    assert_eq!(sent[7], Word::On(true));
    let _ = steps(&rx);
    router.claps_layout(None);
    assert_eq!(
        words(&socket, 4),
        vec![
            Word::Notes(vec![52, 57]),
            Word::Pattern([false; 4]),
            Word::Silent(false),
            Word::On(false),
        ],
        "told while it is still the track"
    );
    let got = steps(&rx);
    assert!(
        got.iter().any(|r| r["step"] == "on" && r["on"] == false),
        "{got:?}"
    );
}

#[tokio::test]
async fn a_moved_claps_track_keeps_what_claps_and_follows_its_clips_anew() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    ready(&mut router);
    router.claps_beat(1, 1, true);
    router.claps.clips = vec![Clip {
        start: 0.0,
        end: 4.0,
        muted: false,
    }];
    router.claps.reading = Some(7);
    let before = router.subs.subscriptions("band");
    let _ = steps(&rx);
    let moved = ClapsTrack {
        instance: "band".into(),
        index: 6,
        track: "live_set tracks 6".into(),
        label: "CLAPS".into(),
    };
    router.claps_layout(Some(moved.clone()));
    assert_eq!(router.claps.track.as_ref(), Some(&moved));
    assert_eq!(
        router.claps.rule.pattern(),
        [false, true, false, false],
        "still clapping"
    );
    assert!(router.claps.words().on);
    assert_eq!(
        router.claps.clips.len(),
        1,
        "the same track's clips stay until the new read answers"
    );
    assert_eq!(router.claps.reading, None, "the old place's read is void");
    assert_eq!(router.subs.subscriptions("band"), before);
    assert_eq!(
        router.claps.subs.get(&key("live_set tracks 6", CLIPS)),
        Some(&Part::Clips)
    );
    assert!(
        !router
            .claps
            .subs
            .contains_key(&key("live_set tracks 4", CLIPS))
    );
    let got = steps(&rx);
    assert_eq!(got[0]["step"], "track");
    assert_eq!(got[0]["moved"], true);
    assert_eq!(got[0]["to"], "live_set tracks 6");
}

#[tokio::test]
async fn taps_choose_the_beats_and_live_claps_them_from_the_next_bar() {
    let dir = tempfile::tempdir().unwrap();
    let (router, _own, rx) = offline_router(dir.path());
    let (mut router, socket) = device(router);
    let page = attach(&mut router, 1);
    ready(&mut router);
    let _ = told(&page);
    let _ = steps(&rx);
    // Nothing claps yet: the device is told so once.
    assert_eq!(
        words(&socket, 4),
        vec![
            Word::Notes(vec![52, 57]),
            Word::Pattern([false; 4]),
            Word::Silent(false),
            Word::On(false),
        ]
    );
    heard(&mut router, 8.0, 10_000.0);
    // A tap 0.55 s later at 120 bpm: 9.1, beat 2 of the bar; the page's
    // clock is 200 ms behind the hub's.
    router.claps_tap(1, 10_350.0, 10_560.0, Some(200.0));
    let got = steps(&rx);
    assert_eq!(got[0]["step"], "tap");
    assert_eq!(got[0]["beat"], 2);
    assert!(
        (got[0]["pos"].as_f64().unwrap() - 9.1).abs() < 1e-9,
        "{got:?}"
    );
    assert_eq!(got[0]["late_ms"], 10.0);
    assert_eq!(got[0]["client"], 1);
    assert_eq!(got[0]["peer"], "10.0.0.1");
    assert_eq!(told(&page).last().unwrap().state, ClapsPhase::Tapping);
    heard(&mut router, 10.9, 11_450.0);
    router.claps_tap(1, 11_500.0, 11_500.0, None);
    assert_eq!(steps(&rx)[0]["beat"], 4);
    assert_eq!(router.claps.rule.shown(), [false, true, false, true]);
    // A bar with no tap: the pattern goes to the device, on.
    heard(&mut router, 15.0, 13_500.0);
    let got = steps(&rx);
    assert_eq!(got[0]["step"], "pattern");
    assert_eq!(got[0]["pattern"], json!([false, true, false, true]));
    assert_eq!(got[1]["step"], "on");
    assert_eq!(got[1]["on"], true);
    assert_eq!(
        words(&socket, 4),
        vec![
            Word::Notes(vec![52, 57]),
            Word::Pattern([false, true, false, true]),
            Word::Silent(false),
            Word::On(true),
        ]
    );
    let shown = told(&page);
    assert_eq!(shown.last().unwrap().state, ClapsPhase::NextBar);
    assert_eq!(shown.last().unwrap().pattern, [false, true, false, true]);
    heard(&mut router, 16.0, 14_000.0);
    assert_eq!(told(&page).last().unwrap().state, ClapsPhase::On);
    assert_eq!(router.claps.taps, 2);
    // Nothing new for the device: nothing sent until the tick.
    let status = router.claps_status(Instant::now());
    assert_eq!(status.state, ClapsPhase::On);
    assert_eq!(status.track.as_deref(), Some("band CLAPS"));
    assert_eq!(status.signature.as_deref(), Some("4/4"));
    assert_eq!(status.sent, 8);
    router.claps_tick(Instant::now());
    assert_eq!(words(&socket, 4)[3], Word::On(true));
    assert_eq!(router.claps_status(Instant::now()).sent, 12);
}

#[tokio::test]
async fn a_tap_is_refused_without_a_track_while_live_stops_and_out_of_four_four() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    let page = attach(&mut router, 1);
    let _ = told(&page);
    router.claps_tap(1, 0.0, 0.0, None);
    let got = steps(&rx);
    assert_eq!(got[0]["step"], "refused");
    assert_eq!(got[0]["why"], "no +CLAPS track");
    assert_eq!(told(&page).len(), 1, "the tapping page is told what holds");
    ready(&mut router);
    transport(&mut router, false);
    let _ = steps(&rx);
    router.claps_tap(1, 0.0, 0.0, None);
    assert_eq!(steps(&rx)[0]["why"], "Live does not play");
    transport(&mut router, true);
    router.claps_tap(1, 0.0, 0.0, None);
    assert_eq!(steps(&rx)[0]["why"], "no playhead heard");
    // A playhead heard more than 2 s before the tap is stale.
    heard(&mut router, 8.0, 1000.0);
    router.claps_tap(1, 3001.0, 3001.0, None);
    assert_eq!(steps(&rx)[0]["why"], "no playhead heard");
    router.claps_delivery(vec![item(&key("live_set", NUMERATOR), json!(3))], 0.0);
    router.claps_tap(1, 1000.0, 1000.0, None);
    assert_eq!(steps(&rx)[0]["why"], "not 4/4");
    assert_eq!(told(&page).last().unwrap().state, ClapsPhase::Signature);
    assert_eq!(router.claps.refused, 5);
    assert_eq!(router.claps.taps, 0);
}

#[tokio::test]
async fn a_dot_and_a_stop_from_a_page() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    let page = attach(&mut router, 1);
    router.claps_beat(1, 0, true);
    assert_eq!(steps(&rx)[0]["why"], "no +CLAPS track");
    ready(&mut router);
    let _ = steps(&rx);
    let _ = told(&page);
    router.claps_beat(1, 4, true);
    assert_eq!(steps(&rx)[0]["why"], "no such beat");
    router.claps_beat(1, 2, true);
    let got = steps(&rx);
    assert_eq!(got[0]["step"], "beat");
    assert_eq!(got[0]["beat"], 3);
    assert_eq!(got[1]["step"], "pattern");
    assert_eq!(got[2]["step"], "on");
    assert_eq!(router.claps.words().pattern, [false, false, true, false]);
    // The same dot again changes nothing and records no beat.
    router.claps_beat(1, 2, true);
    assert!(steps(&rx).is_empty());
    let _ = told(&page);
    router.claps_stop(1);
    let got = steps(&rx);
    assert_eq!(got[0]["step"], "stop");
    assert_eq!(got[0]["changed"], true);
    assert_eq!(got[1]["step"], "pattern");
    assert_eq!(got[2]["on"], false);
    assert_eq!(told(&page).last().unwrap().state, ClapsPhase::Off);
}

#[tokio::test]
async fn an_active_clip_over_the_playhead_silences_the_device() {
    let dir = tempfile::tempdir().unwrap();
    let (router, _own, rx) = offline_router(dir.path());
    let (mut router, socket) = device(router);
    let page = attach(&mut router, 1);
    ready(&mut router);
    router.claps_beat(1, 0, true);
    let _ = words(&socket, 8);
    router.claps.reading = Some(3);
    router.claps_clips(
        3,
        Ok(vec![Clip {
            start: 16.0,
            end: 24.0,
            muted: false,
        }]),
        12,
    );
    let got = steps(&rx);
    let clips = got.iter().find(|r| r["step"] == "clips").expect("clips");
    assert_eq!(clips["clips"], 1);
    assert_eq!(clips["active"], 1);
    assert_eq!(clips["ms"], 12);
    heard(&mut router, 10.0, 0.0);
    heard(&mut router, 12.0, 1_000.0);
    assert_eq!(router.claps.rule.mode(), crate::rules::claps::Mode::On);
    heard(&mut router, 15.0, 2_500.0);
    let _ = told(&page);
    let _ = steps(&rx);
    // Half a beat ahead: 15.49 does not count yet, 15.5 does.
    heard(&mut router, 15.49, 2_745.0);
    assert!(steps(&rx).is_empty(), "not silent yet");
    heard(&mut router, 15.5, 2_750.0);
    let got = steps(&rx);
    assert_eq!(got[0]["step"], "silent");
    assert_eq!(got[0]["silent"], true);
    assert_eq!(words(&socket, 4)[2], Word::Silent(true));
    assert_eq!(told(&page).last().unwrap().state, ClapsPhase::Clip);
    heard(&mut router, 23.49, 6_745.0);
    assert!(
        steps(&rx).is_empty(),
        "the clip's last beat is still the clip's"
    );
    heard(&mut router, 23.5, 6_750.0);
    assert_eq!(steps(&rx)[0]["silent"], false);
    assert_eq!(told(&page).last().unwrap().state, ClapsPhase::On);
}

#[tokio::test]
async fn a_read_is_void_unless_it_is_the_one_on_its_way_and_a_failure_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    ready(&mut router);
    let _ = steps(&rx);
    router.claps.reading = Some(2);
    router.claps_clips(1, Ok(Vec::new()), 5);
    assert_eq!(router.claps.reading, Some(2));
    assert!(steps(&rx).is_empty());
    router.claps_clips(2, Err("offline".into()), 5);
    let got = steps(&rx);
    assert_eq!(got[0]["step"], "clips");
    assert_eq!(got[0]["error"], "offline");
    assert_eq!(router.claps.clips_error.as_deref(), Some("offline"));
    assert_eq!(router.claps.reading, None);
    // The same clips again record nothing; after a failure they do.
    router.claps.reading = Some(4);
    router.claps_clips(4, Ok(Vec::new()), 5);
    assert_eq!(steps(&rx)[0]["clips"], 0);
    router.claps.reading = Some(5);
    router.claps_clips(5, Ok(Vec::new()), 5);
    assert!(steps(&rx).is_empty());
    assert_eq!(router.claps.clips_error, None);
}

#[tokio::test]
async fn a_read_asked_while_one_runs_comes_after_it() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, _rx) = offline_router(dir.path());
    router.claps_layout(Some(track()));
    assert_eq!(router.claps.reading, None, "no list heard yet");
    router.claps_delivery(vec![item(&key("live_set tracks 4", CLIPS), json!([]))], 0.0);
    assert_eq!(router.claps.reading, Some(1));
    transport(&mut router, true);
    assert!(router.claps.dirty);
    router.claps_clips(1, Ok(Vec::new()), 1);
    assert_eq!(router.claps.reading, Some(2), "the owed one");
    assert!(!router.claps.dirty);
    router.claps_offline("band");
    assert_eq!(router.claps.reading, None);
    router.claps_clips(2, Ok(Vec::new()), 1);
    assert_eq!(router.claps.reading, None, "void after the disconnect");
}

#[tokio::test]
async fn a_jump_or_a_tempo_change_tells_the_pages_the_playhead_again() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, _rx) = offline_router(dir.path());
    let page = attach(&mut router, 1);
    ready(&mut router);
    heard(&mut router, 8.0, 1000.0);
    let shown = told(&page);
    assert_eq!(shown.last().unwrap().beat, Some(8.0));
    assert_eq!(shown.last().unwrap().tempo, Some(120.0));
    // On the ring: nothing told.
    heard(&mut router, 9.0, 1500.0);
    heard(&mut router, 9.2, 1600.0);
    assert!(told(&page).is_empty());
    // A jump.
    heard(&mut router, 40.0, 1700.0);
    assert_eq!(told(&page).last().unwrap().beat, Some(40.0));
    router.claps_delivery(vec![item(&key("live_set", TEMPO), json!(124.0))], 1800.0);
    assert_eq!(told(&page).last().unwrap().tempo, Some(124.0));
    // A stop: no playhead.
    transport(&mut router, false);
    assert_eq!(told(&page).last().unwrap().beat, None);
}

#[tokio::test]
async fn the_devices_hello_is_recorded_when_new_and_its_silence_once() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    let start = Instant::now();
    let hello = Hello {
        version: "v1.52.0".into(),
        running: false,
    };
    router.claps_hello(hello.clone(), start);
    let got = steps(&rx);
    assert_eq!(got[0]["step"], "device");
    assert_eq!(got[0]["version"], "v1.52.0");
    router.claps_hello(hello.clone(), start + Duration::from_millis(1000));
    assert!(steps(&rx).is_empty(), "the same device");
    let status = router.claps_status(start + Duration::from_millis(1500));
    assert_eq!(
        status.device,
        Some(ClapsDevice {
            version: "v1.52.0".into(),
            running: false,
            age_ms: 500
        })
    );
    router.claps_tick(start + Duration::from_millis(3999));
    assert!(steps(&rx).is_empty(), "heard 2999 ms ago");
    router.claps_tick(start + Duration::from_millis(4000));
    let got = steps(&rx);
    assert_eq!(got[0]["gone"], true);
    assert!(router.claps_status(start).device.is_none());
    router.claps_tick(start + Duration::from_millis(9000));
    assert!(steps(&rx).is_empty(), "gone once");
    router.claps_listen("127.0.0.1:39113", None);
    assert_eq!(steps(&rx)[0]["addr"], "127.0.0.1:39113");
}

#[tokio::test]
async fn nothing_goes_to_the_device_without_a_claps_track() {
    let dir = tempfile::tempdir().unwrap();
    let (router, _own, _rx) = offline_router(dir.path());
    let (mut router, socket) = device(router);
    socket
        .set_read_timeout(Some(Duration::from_millis(200)))
        .unwrap();
    router.claps_tick(Instant::now());
    let mut buf = [0u8; 64];
    assert!(socket.recv(&mut buf).is_err());
    assert_eq!(router.claps_status(Instant::now()).sent, 0);
}

#[tokio::test]
async fn a_router_message_reaches_the_claps() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, rx) = offline_router(dir.path());
    ready(&mut router);
    let _ = steps(&rx);
    router.handle(RouterMsg::ClapBeat {
        client: 1,
        beat: 1,
        on: true,
    });
    assert_eq!(steps(&rx)[0]["step"], "beat");
    router.handle(RouterMsg::ClapStop { client: 1 });
    assert_eq!(steps(&rx)[0]["step"], "stop");
    router.handle(RouterMsg::ClapTap {
        client: 1,
        t: 0.0,
        hub_ms: 0.0,
        offset_ms: None,
    });
    assert_eq!(steps(&rx)[0]["step"], "refused");
    router.handle(RouterMsg::ClapsHello {
        hello: Hello {
            version: "v1".into(),
            running: true,
        },
    });
    assert_eq!(steps(&rx)[0]["step"], "device");
    router.handle(RouterMsg::ClapsListen {
        addr: "a".into(),
        error: Some("taken".into()),
    });
    assert_eq!(steps(&rx)[0]["error"], "taken");
    router.claps.reading = Some(9);
    router.handle(RouterMsg::ClapsClips {
        read: 9,
        outcome: Err("x".into()),
        ms: 1,
    });
    assert_eq!(steps(&rx)[0]["error"], "x");
    router.handle(RouterMsg::ClapsTick);
    let status = super::super::tests::status(&mut router);
    assert_eq!(status.claps.track.as_deref(), Some("band CLAPS"));
}

#[tokio::test]
async fn a_moved_claps_track_stays_silent_over_its_clip_until_the_new_read() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, _rx) = offline_router(dir.path());
    ready(&mut router);
    router.claps_beat(1, 1, true);
    router.claps.clips = vec![Clip {
        start: 0.0,
        end: 8.0,
        muted: false,
    }];
    router.claps_delivery(vec![item(&key("live_set", SONG_TIME), json!(1.0))], 0.0);
    assert!(router.claps.silent, "the clip covers the playhead");
    router.claps_layout(Some(ClapsTrack {
        instance: "band".into(),
        index: 6,
        track: "live_set tracks 6".into(),
        label: "CLAPS".into(),
    }));
    assert!(
        router.claps.silent,
        "the same clips at the new index: the device stays silent"
    );
}

#[tokio::test]
async fn clips_the_rule_cannot_follow_keep_the_device_silent() {
    let dir = tempfile::tempdir().unwrap();
    let (mut router, _own, _rx) = offline_router(dir.path());
    router.claps_layout(Some(ClapsTrack {
        instance: "band".into(),
        index: 4,
        track: "live_set tracks ]".into(),
        label: "CLAPS".into(),
    }));
    assert!(
        router.claps.silent,
        "clips not followed are unknown: never clap over them"
    );
}
