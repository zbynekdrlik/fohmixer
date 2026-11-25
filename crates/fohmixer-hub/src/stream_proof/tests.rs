use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use fohmixer_proto::stream_proof::{BUS, HAND1_SYNC, STREAM_TRACK};
use serde_json::{Value, json};

use super::*;
use crate::live::client::{Script, scripted};

/// One track of the fake Live: name, mute, group, the group it is in,
/// output type and channel, sends.
#[derive(Debug, Clone)]
struct Ft {
    name: String,
    mute: bool,
    foldable: bool,
    group: Option<usize>,
    out: (String, String),
    sends: usize,
}

fn ft(name: &str, group: Option<usize>, out: &str) -> Ft {
    Ft {
        name: name.into(),
        mute: false,
        foldable: false,
        group,
        out: (out.into(), String::new()),
        sends: 0,
    }
}

/// A fake Live set answering the walk's `get_prop`s.
#[derive(Debug, Clone)]
struct Fake {
    tracks: Vec<Ft>,
    returns: Vec<Ft>,
    main: (String, String),
    /// A target whose reads fail.
    fail: Option<String>,
    /// Track names read after the list (a rename meanwhile).
    renamed: Option<(usize, String)>,
    /// A `group_track` `$ref` the list never gave.
    stray_group: bool,
    /// One slot answered as given: target, prop, the slot.
    slot_as: Option<(String, String, Value)>,
    /// The track-name reads after the facts' request answer this (a move
    /// between step 2 and step 3).
    late_name: Option<String>,
}

/// The MBC set's shape (invented names besides the constants).
fn mbc() -> Fake {
    let mut talk = ft("Talk grp", Some(0), "Group");
    talk.foldable = true;
    talk.mute = true;
    let mut bus = ft(BUS, None, STREAM_TRACK);
    bus.foldable = true;
    let mut stream = ft(STREAM_TRACK, None, "Ext. Out");
    stream.out.1 = "3/4 Final LR".into();
    Fake {
        tracks: vec![
            bus,
            ft(HAND1_SYNC, Some(0), "Group"),
            talk,
            ft("Talk A", Some(2), "Group"),
            ft("Quiet", Some(0), "Sends Only"),
            stream,
            ft("Room", None, "Main"),
        ],
        returns: Vec::new(),
        main: ("Ext. Out".into(), "1/2".into()),
        fail: None,
        renamed: None,
        stray_group: false,
        slot_as: None,
        late_name: None,
    }
}

fn ok(data: Value) -> Value {
    json!({"ok": true, "data": data})
}

impl Fake {
    fn object(kind: &str, i: usize, t: &Ft) -> Value {
        json!({"$ref": format!("{kind}{i}"), "path": format!("live_set {kind} {i}"),
               "class": "Track", "name": t.name})
    }

    /// The slot of one command.
    fn slot(&self, command: &Value) -> Value {
        let target = command["target"].as_str().unwrap();
        let prop = command["args"]["prop"].as_str().unwrap();
        if self.fail.as_deref() == Some(target) {
            return json!({"ok": false, "error": "not found"});
        }
        if let Some((t, p, slot)) = &self.slot_as
            && t == target
            && p == prop
        {
            return slot.clone();
        }
        if target == "live_set" {
            let (kind, list) = match prop {
                "tracks" => ("tracks", &self.tracks),
                _ => ("return_tracks", &self.returns),
            };
            let objects: Vec<Value> = list
                .iter()
                .enumerate()
                .map(|(i, t)| Self::object(kind, i, t))
                .collect();
            return ok(json!(objects));
        }
        let words: Vec<&str> = target.split(' ').collect();
        let (track, rest) = match words.as_slice() {
            ["live_set", "master_track", rest @ ..] => (None, rest),
            ["live_set", kind, i, rest @ ..] => {
                let i: usize = i.parse().unwrap();
                let list = if *kind == "tracks" {
                    &self.tracks
                } else {
                    &self.returns
                };
                (Some((*kind, i, &list[i])), rest)
            }
            _ => return json!({"ok": false, "error": "bad target"}),
        };
        let out = track.map_or(&self.main, |(_, _, t)| &t.out);
        match (track, rest, prop) {
            (_, ["output_routing_type"], "display_name") => ok(json!(out.0)),
            (_, ["output_routing_channel"], "display_name") => ok(json!(out.1)),
            (Some((_, i, t)), [], "name") => match &self.renamed {
                Some((at, name)) if *at == i => ok(json!(name)),
                _ => ok(json!(t.name)),
            },
            (Some((_, _, t)), [], "mute") => ok(json!(t.mute)),
            (Some((_, _, t)), [], "is_foldable") => ok(json!(t.foldable)),
            (Some((_, _, t)), [], "group_track") => match t.group {
                None => ok(Value::Null),
                Some(_) if self.stray_group => ok(json!({"$ref": "elsewhere"})),
                Some(g) => ok(Self::object("tracks", g, &self.tracks[g])),
            },
            (Some((_, _, t)), ["mixer_device"], "sends") => ok(json!(vec![0.0; t.sends])),
            _ => json!({"ok": false, "error": "no such property"}),
        }
    }
}

/// How the fake answers a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Answer,
    /// Every call dropped unanswered.
    Drop,
    /// The second call's answer one slot short.
    Short,
}

/// Answers every call of `script` from `fake` as `mode` says, counting the
/// calls and their commands.
fn drive(
    mut script: Script,
    mut fake: Fake,
    mode: Mode,
) -> (Arc<AtomicUsize>, Arc<std::sync::Mutex<Vec<usize>>>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (c, s) = (Arc::clone(&calls), Arc::clone(&sizes));
    tokio::spawn(async move {
        loop {
            match script.try_call() {
                Some(asked) => {
                    let n = c.fetch_add(1, Ordering::SeqCst);
                    s.lock().unwrap().push(asked.commands.len());
                    if mode == Mode::Drop {
                        drop(asked);
                        continue;
                    }
                    let mut slots: Vec<Value> =
                        asked.commands.iter().map(|cmd| fake.slot(cmd)).collect();
                    if mode == Mode::Short && n == 1 {
                        slots.pop();
                    }
                    if n == 1
                        && let Some(name) = fake.late_name.take()
                    {
                        fake.renamed = Some((4, name));
                    }
                    asked.answer(slots);
                }
                None => tokio::time::sleep(Duration::from_millis(1)).await,
            }
        }
    });
    (calls, sizes)
}

/// A scripted online `mbc` answered by `fake`.
fn live(
    fake: Fake,
) -> (
    LiveHandle,
    Arc<AtomicUsize>,
    Arc<std::sync::Mutex<Vec<usize>>>,
) {
    let (handle, script) = scripted(MBC_INSTANCE);
    handle.set_online(true);
    let (calls, sizes) = drive(script, fake, Mode::Answer);
    (handle, calls, sizes)
}

#[tokio::test]
async fn a_walk_reads_the_set_and_the_proof_holds() {
    let (handle, calls, sizes) = live(mbc());
    let set = walk(&handle).await.unwrap();
    assert_eq!(set.tracks.len(), 7);
    assert_eq!(set.tracks[3].parent, Some(2));
    assert_eq!(set.tracks[2].parent, Some(0));
    assert!(set.tracks[2].group && set.tracks[2].mute);
    assert_eq!(set.tracks[5].output.channel, "3/4 Final LR");
    assert_eq!(set.main.channel, "1/2");
    // Three requests: the lists, the facts (6 per track and the main
    // track's 2), the one Sends Only track's name and sends.
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(*sizes.lock().unwrap(), vec![2, 7 * 6 + 2, 2]);
    let proof = decide(&set).unwrap();
    assert_eq!(proof.open, vec![HAND1_SYNC]);
    assert!(proof.hand1_sync_only);
}

#[tokio::test]
async fn returns_and_sends_are_read_and_a_sends_only_track_reaches_the_bus_through_a_return() {
    let mut fake = mbc();
    fake.tracks[4].sends = 1;
    fake.returns.push(ft("A-Reverb", None, BUS));
    let mut delay = ft("B-Delay", None, "Ext. Out");
    delay.mute = true;
    delay.out.1 = "5/6".into();
    fake.returns.push(delay);
    let (handle, _, sizes) = live(fake);
    let set = walk(&handle).await.unwrap();
    assert_eq!(set.returns.len(), 2);
    assert_eq!(set.returns[0].name, "A-Reverb");
    assert_eq!(set.returns[0].output.kind, BUS);
    assert!(!set.returns[0].mute);
    assert_eq!(set.returns[1].name, "B-Delay");
    assert!(set.returns[1].mute);
    assert_eq!(set.returns[1].output.channel, "5/6");
    assert_eq!(set.main.channel, "1/2");
    assert_eq!(set.tracks[4].sends, 1);
    assert_eq!(*sizes.lock().unwrap(), vec![2, 7 * 6 + 2 * 4 + 2, 2]);
    assert_eq!(
        decide(&set).unwrap().open,
        vec![HAND1_SYNC, "Quiet", "A-Reverb"]
    );
}

#[tokio::test]
async fn a_large_set_is_read_in_requests_of_at_most_a_chunk() {
    let mut fake = mbc();
    for i in 0..40 {
        fake.tracks.push(ft(&format!("Extra {i}"), None, "Main"));
    }
    let (handle, _, sizes) = live(fake);
    let set = walk(&handle).await.unwrap();
    assert_eq!(set.tracks.len(), 47);
    assert_eq!(
        *sizes.lock().unwrap(),
        vec![2, CHUNK, 47 * 6 + 2 - CHUNK, 2]
    );
    assert!(decide(&set).unwrap().hand1_sync_only);
}

#[tokio::test]
async fn a_set_with_no_sends_only_track_asks_no_third_request() {
    let mut fake = mbc();
    fake.tracks[4].out.0 = "Group".into();
    let (handle, _, sizes) = live(fake);
    walk(&handle).await.unwrap();
    assert_eq!(*sizes.lock().unwrap(), vec![2, 7 * 6 + 2]);
}

#[tokio::test]
async fn a_failed_read_fails_the_walk_with_its_words() {
    let mut fake = mbc();
    fake.fail = Some("live_set tracks 5 output_routing_channel".into());
    let (handle, _, _) = live(fake);
    assert_eq!(
        walk(&handle).await,
        Err("live_set tracks 5 output_routing_channel: not found".into())
    );
    let mut fake = mbc();
    fake.fail = Some("live_set".into());
    let (handle, _, _) = live(fake);
    assert_eq!(
        walk(&handle).await,
        Err("live_set tracks: not found".into())
    );
    let mut fake = mbc();
    fake.fail = Some("live_set tracks 4 mixer_device".into());
    let (handle, _, _) = live(fake);
    assert_eq!(
        walk(&handle).await,
        Err("live_set tracks 4 mixer_device sends: not found".into())
    );
    let mut fake = mbc();
    fake.fail = Some("live_set master_track output_routing_type".into());
    let (handle, _, _) = live(fake);
    assert_eq!(
        walk(&handle).await,
        Err("live_set master_track output_routing_type: not found".into())
    );
}

#[tokio::test]
async fn a_track_renamed_or_a_stray_group_during_the_walk_fails_it() {
    let mut fake = mbc();
    fake.renamed = Some((1, "Moved".into()));
    let (handle, _, _) = live(fake);
    assert_eq!(
        walk(&handle).await,
        Err(format!(
            "the tracks changed during the read: live_set tracks 1 is now \"Moved\", was \"{HAND1_SYNC}\""
        ))
    );
    let mut fake = mbc();
    fake.stray_group = true;
    let (handle, _, _) = live(fake);
    assert_eq!(
        walk(&handle).await,
        Err("live_set tracks 1 group_track: a group that is not in the track list".into())
    );
}

#[test]
fn a_slot_of_the_wrong_kind_is_named() {
    let slots = vec![ok(json!(3)), ok(json!("x")), json!({"ok": true})];
    assert_eq!(
        text(&slots, 0, "p name"),
        Err("p name: not a text: 3".into())
    );
    assert_eq!(
        flag(&slots, 1, "p mute"),
        Err("p mute: not a bool: \"x\"".into())
    );
    assert_eq!(text(&slots, 1, "p name"), Ok("x".into()));
    assert_eq!(data(&slots, 2, "p"), Err("p: no answer".into()));
    assert_eq!(data(&slots, 3, "p"), Err("p: no result".into()));
    assert_eq!(flag(&[ok(json!(true))], 0, "p"), Ok(true));
}

#[tokio::test]
async fn live_not_answering_fails_the_walk() {
    let (handle, script) = scripted(MBC_INSTANCE);
    handle.set_online(true);
    let (calls, _) = drive(script, mbc(), Mode::Drop);
    assert_eq!(
        walk(&handle).await,
        Err("Live did not answer: instance offline".into())
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn an_answer_is_young_only_inside_its_window() {
    let at = Instant::now();
    let window = Duration::from_secs(30);
    assert_eq!(age_within(at, at, window), Some(0));
    assert_eq!(
        age_within(at, at + Duration::from_millis(1500), window),
        Some(1500)
    );
    assert_eq!(age_within(at, at + window, window), None);
    assert_eq!(
        age_within(at, at + window - Duration::from_millis(1), window),
        Some(29_999)
    );
    // A clock read before `at` is age 0.
    assert_eq!(age_within(at + window, at, window), Some(0));
    assert_eq!(WINDOW, Duration::from_secs(30));
    assert_eq!(CHUNK, 200);
}

#[test]
fn a_record_names_the_proof_or_the_error() {
    let ok = Answer::of(
        Ok(fohmixer_proto::stream_proof::Proof {
            open: vec![HAND1_SYNC.into()],
            hand1_sync_only: true,
            outputs: fohmixer_proto::stream_proof::Outputs {
                bus: BUS.into(),
                bus_to: STREAM_TRACK.into(),
                stream: STREAM_TRACK.into(),
                stream_to: Output::default(),
                others: Vec::new(),
            },
        }),
        1,
    );
    assert_eq!(
        record_fields(&ok, 12),
        json!({"ok": true, "ms": 12, "open": [HAND1_SYNC], "hand1_sync_only": true})
    );
    let failed = Answer::of(Err("why".into()), 1);
    assert_eq!(
        record_fields(&failed, 3),
        json!({"ok": false, "ms": 3, "error": "why"})
    );
}

#[tokio::test]
async fn an_answer_is_served_again_inside_the_window_and_walked_again_after_it() {
    let (handle, calls, _) = live(mbc());
    let (events, records) = EventLog::channel(16);
    let proofs = StreamProofs::default();
    let first = proofs.answer(Some(&handle), &events).await;
    assert!(first.ok && first.hand1_sync_only, "{first:?}");
    assert_eq!(first.age_ms, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    tokio::time::sleep(Duration::from_millis(20)).await;
    let again = proofs.answer(Some(&handle), &events).await;
    assert_eq!(calls.load(Ordering::SeqCst), 3, "no second walk");
    assert_eq!(again.ts, first.ts);
    assert!(again.age_ms >= 20, "{again:?}");
    assert_eq!(again.open, first.open);
    let record = records.try_recv().unwrap();
    assert_eq!(record["ev"], "stream_proof");
    assert_eq!(record["ok"], true);
    assert!(records.try_recv().is_err(), "one record per walk");
    // A window of 0: every request walks.
    let proofs = StreamProofs::with_window(Duration::ZERO);
    proofs.answer(Some(&handle), &events).await;
    proofs.answer(Some(&handle), &events).await;
    assert_eq!(calls.load(Ordering::SeqCst), 9);
}

#[tokio::test]
async fn requests_together_make_one_walk() {
    let (handle, calls, _) = live(mbc());
    let events = EventLog::off();
    let proofs = StreamProofs::default();
    let (a, b, c) = tokio::join!(
        proofs.answer(Some(&handle), &events),
        proofs.answer(Some(&handle), &events),
        proofs.answer(Some(&handle), &events),
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "one walk of three requests"
    );
    assert_eq!((a.ts, a.ts), (b.ts, c.ts));
    assert!(a.ok && b.ok && c.ok);
}

#[tokio::test]
async fn a_failed_walk_is_answered_and_cached_too() {
    let mut fake = mbc();
    fake.tracks[0].name = "Renamed bus".into();
    let (handle, calls, _) = live(fake);
    let (events, records) = EventLog::channel(16);
    let proofs = StreamProofs::default();
    let answer = proofs.answer(Some(&handle), &events).await;
    assert!(!answer.ok);
    assert!(!answer.hand1_sync_only);
    assert_eq!(
        answer.reason.as_deref(),
        Some(&*format!("no track is named {BUS}"))
    );
    assert_eq!(
        records.try_recv().unwrap()["error"],
        format!("no track is named {BUS}")
    );
    let calls_after = calls.load(Ordering::SeqCst);
    proofs.answer(Some(&handle), &events).await;
    assert_eq!(calls.load(Ordering::SeqCst), calls_after);
}

#[tokio::test]
async fn no_mbc_or_an_offline_mbc_is_answered_at_once_and_not_cached() {
    let events = EventLog::off();
    let proofs = StreamProofs::default();
    let none = proofs.answer(None, &events).await;
    assert_eq!(none.reason.as_deref(), Some("this hub has no mbc Live set"));
    assert!(!none.ok && !none.hand1_sync_only);
    let (handle, calls, _) = live(mbc());
    handle.set_online(false);
    let offline = proofs.answer(Some(&handle), &events).await;
    assert_eq!(
        offline.reason.as_deref(),
        Some("the mbc Live set is offline")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    // Online again: walked at once (nothing was cached).
    handle.set_online(true);
    let walked = proofs.answer(Some(&handle), &events).await;
    assert!(walked.ok, "{walked:?}");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn an_answer_that_is_no_list_or_short_or_without_refs_fails_the_walk() {
    let mut fake = mbc();
    fake.slot_as = Some(("live_set".into(), "return_tracks".into(), ok(json!("none"))));
    let (handle, _, _) = live(fake);
    assert_eq!(
        walk(&handle).await,
        Err("live_set return_tracks: not a list: \"none\"".into())
    );
    let mut fake = mbc();
    fake.slot_as = Some((
        "live_set".into(),
        "return_tracks".into(),
        ok(json!([{"$ref": "q0", "class": "Track", "name": "A"}])),
    ));
    let (handle, _, _) = live(fake);
    assert_eq!(
        walk(&handle).await,
        Err("live_set return_tracks: an object without a path or a $ref".into())
    );
    let mut fake = mbc();
    fake.slot_as = Some((
        "live_set".into(),
        "return_tracks".into(),
        ok(json!([{"path": "live_set return_tracks 0", "class": "Track", "name": "A"}])),
    ));
    let (handle, _, _) = live(fake);
    assert_eq!(
        walk(&handle).await,
        Err("live_set return_tracks: an object without a path or a $ref".into())
    );
    let mut fake = mbc();
    fake.slot_as = Some((
        "live_set tracks 4 mixer_device".into(),
        "sends".into(),
        ok(json!(2)),
    ));
    let (handle, _, _) = live(fake);
    assert_eq!(
        walk(&handle).await,
        Err("live_set tracks 4 mixer_device sends: not a list: 2".into())
    );
    let (handle, script) = scripted(MBC_INSTANCE);
    handle.set_online(true);
    drive(script, mbc(), Mode::Short);
    assert_eq!(
        walk(&handle).await,
        Err(format!(
            "Live answered {} of {} reads",
            7 * 6 + 1,
            7 * 6 + 2
        ))
    );
}

#[tokio::test]
async fn a_group_without_a_ref_or_a_move_before_the_sends_fails_the_walk() {
    let mut fake = mbc();
    fake.slot_as = Some((
        "live_set tracks 1".into(),
        "group_track".into(),
        ok(json!({"path": "live_set tracks 0", "class": "Track"})),
    ));
    let (handle, _, _) = live(fake);
    assert_eq!(
        walk(&handle).await,
        Err("live_set tracks 1 group_track: a group that is not in the track list".into())
    );
    let mut fake = mbc();
    fake.late_name = Some("Elsewhere".into());
    let (handle, _, _) = live(fake);
    assert_eq!(
        walk(&handle).await,
        Err("the tracks changed during the read: live_set tracks 4 is now \"Elsewhere\", was \"Quiet\"".into())
    );
}
