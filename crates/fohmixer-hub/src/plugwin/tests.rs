use std::sync::Mutex;

use serde_json::json;

use super::sim::{REFUSED, Sim, SimHandle};
use super::*;

/// The events a worker told, in order.
type Heard = Arc<Mutex<Vec<PlugwinEvent>>>;

/// A worker on a simulated backend, run by hand (`command`, `step` at
/// chosen times).
fn worker() -> (Worker, SimHandle, Heard) {
    let (sim, handle) = Sim::new(None);
    let heard: Heard = Arc::new(Mutex::new(Vec::new()));
    let into = Arc::clone(&heard);
    let worker = Worker {
        backend: Box::new(sim),
        events: Arc::new(move |event: PlugwinEvent| into.lock().unwrap().push(event)),
        editors: BTreeMap::new(),
        finding: Vec::new(),
        contact: None,
        clock: Instant::now(),
    };
    (worker, handle, heard)
}

/// Takes the editor Live opens as `session` at `now`; its window.
fn take(worker: &mut Worker, handle: &SimHandle, session: u32, now: f64) -> WindowId {
    let before = handle.windows();
    let (reply, mut answer) = oneshot::channel();
    worker.command(
        Command::Take {
            session,
            before,
            reply,
        },
        now,
    );
    worker.step(now);
    assert_eq!(answer.try_recv().unwrap(), Ok((1349, 809)));
    *handle.windows().last().unwrap()
}

/// The sim's touch records: phase and point.
fn touches(handle: &SimHandle) -> Vec<(String, i64, i64)> {
    handle
        .records()
        .iter()
        .filter(|r| r["op"] == "touch")
        .map(|r| {
            (
                r["phase"].as_str().unwrap().to_string(),
                r["x"].as_i64().unwrap(),
                r["y"].as_i64().unwrap(),
            )
        })
        .collect()
}

fn t(phase: &str, x: i64, y: i64) -> (String, i64, i64) {
    (phase.to_string(), x, y)
}

/// The frames a test sink kept: each one's session and size.
type Kept = Arc<Mutex<Vec<(u32, usize)>>>;

/// A sink that keeps each frame's session and size.
fn sink() -> (FrameSink, Kept) {
    let frames = Arc::new(Mutex::new(Vec::new()));
    let into = Arc::clone(&frames);
    let sink: FrameSink = Arc::new(move |session: u32, jpeg: Bytes| {
        into.lock().unwrap().push((session, jpeg.len()));
    });
    (sink, frames)
}

#[test]
fn the_clocks_turn_at_their_bounds() {
    assert!(!capture_due(40.0_f64.next_down()));
    assert!(capture_due(40.0));
    assert!(!poll_due(50.0_f64.next_down()));
    assert!(poll_due(50.0));
    assert!(!find_over(3000.0_f64.next_down()));
    assert!(find_over(3000.0));
    assert!(!rate_due(60_000.0_f64.next_down()));
    assert!(rate_due(60_000.0));
    assert_eq!(STEP, Duration::from_millis(10));
    assert_eq!(GUARD_TAP, Duration::from_millis(30));
    assert_eq!(QUALITY, 70);
}

#[test]
fn the_inert_spot_sits_in_the_top_bars_empty_middle() {
    assert_eq!(inert_spot(1349), (546, 15));
    assert_eq!(inert_spot(1000), (405, 15));
    assert_eq!(inert_spot(0), (0, 15));
}

#[test]
fn an_open_takes_the_one_new_window_or_fails_once_its_wait_is_over() {
    let (a, b, c) = (WindowId(1), WindowId(2), WindowId(3));
    assert_eq!(pick(&[a], &[a]), Pick::None);
    assert_eq!(pick(&[a], &[a, b]), Pick::One(b));
    assert_eq!(pick(&[a, b], &[b]), Pick::None, "one went away");
    assert_eq!(pick(&[a], &[a, b, c]), Pick::Several);
    assert_eq!(pick(&[], &[c]), Pick::One(c));
    assert_eq!(found(&Pick::One(b), 0.0), Some(Ok(b)));
    assert_eq!(found(&Pick::None, 2999.0), None);
    assert_eq!(found(&Pick::None, 3000.0), Some(Err(NO_WINDOW)));
    assert_eq!(found(&Pick::Several, 2999.0), None);
    assert_eq!(found(&Pick::Several, 3000.0), Some(Err(SEVERAL)));
}

#[test]
fn a_picture_is_encoded_as_a_jpeg() {
    let pixels = sim::picture(5, 16, 8);
    let jpeg = encode(&pixels, QUALITY).unwrap();
    assert_eq!(jpeg[..2], [0xFF_u8, 0xD8]);
    assert_eq!(jpeg[jpeg.len() - 2..], [0xFF_u8, 0xD9]);
    let wide = Pixels {
        width: 70_000,
        height: 1,
        bgra: Vec::new(),
    };
    assert_eq!(
        encode(&wide, QUALITY),
        Err("a picture over 65535 px wide".to_string())
    );
    let high = Pixels {
        width: 1,
        height: 70_000,
        bgra: Vec::new(),
    };
    assert_eq!(
        encode(&high, QUALITY),
        Err("a picture over 65535 px high".to_string())
    );
    let short = Pixels {
        width: 16,
        height: 8,
        bgra: vec![0; 10],
    };
    assert!(encode(&short, QUALITY).is_err(), "too few pixels");
}

#[test]
fn a_minutes_counts_are_means_over_the_grabs_and_the_frames() {
    let counts = Counts {
        grabs: 4,
        sent: 2,
        failed: 1,
        grab_ms: 20.0,
        encode_ms: 30.0,
        bytes: 1000.0,
        width: 10,
        height: 5,
    };
    assert_eq!(
        counts.rate(),
        Rate {
            grabs: 4,
            sent: 2,
            failed: 1,
            grab_ms: 5.0,
            encode_ms: 15.0,
            bytes: 500.0,
            width: 10,
            height: 5,
        }
    );
    assert_eq!(Counts::default().rate(), Rate::default());
}

#[test]
fn a_phase_says_whether_it_is_checked_and_whether_it_ends() {
    let all = [Phase::Down, Phase::Update, Phase::Up, Phase::Cancel];
    assert_eq!(all.map(Phase::name), ["down", "update", "up", "cancel"]);
    assert_eq!(all.map(Phase::checked), [true, true, false, false]);
    assert_eq!(all.map(Phase::ends), [false, false, true, true]);
}

#[test]
fn the_worker_lists_and_takes_the_window_live_opens() {
    let (mut worker, handle, _) = worker();
    let (reply, mut answer) = oneshot::channel();
    worker.command(Command::List(reply), 0.0);
    assert_eq!(answer.try_recv().unwrap(), Ok(Vec::new()));
    let window = take(&mut worker, &handle, 1, 0.0);
    assert_eq!(handle.topmost(window), Some(true));
    assert_eq!(worker.editors[&1].taken.window, window);
    assert_eq!(
        handle.records(),
        vec![json!({"op": "take", "window": window.0})]
    );
}

#[test]
fn an_open_whose_window_never_comes_fails_after_its_wait() {
    let (mut worker, handle, _) = worker();
    handle.auto_open(false);
    let (reply, mut answer) = oneshot::channel();
    worker.command(
        Command::Take {
            session: 1,
            before: Vec::new(),
            reply,
        },
        100.0,
    );
    worker.step(100.0);
    worker.step(149.0);
    worker.step(150.0);
    worker.step(3099.0);
    assert!(answer.try_recv().is_err(), "still waiting");
    worker.step(3100.0);
    assert_eq!(answer.try_recv().unwrap(), Err(NO_WINDOW.to_string()));
    assert!(worker.finding.is_empty());
    // Two new windows at once: refused once the wait is over.
    let (reply, mut answer) = oneshot::channel();
    worker.command(
        Command::Take {
            session: 2,
            before: Vec::new(),
            reply,
        },
        5000.0,
    );
    handle.add_window(0);
    handle.add_window(0);
    worker.step(5000.0);
    assert!(answer.try_recv().is_err());
    worker.step(8000.0);
    assert_eq!(answer.try_recv().unwrap(), Err(SEVERAL.to_string()));
    assert!(worker.editors.is_empty());
}

#[test]
fn a_late_window_is_found_by_a_later_poll() {
    let (mut worker, handle, _) = worker();
    handle.auto_open(false);
    let (reply, mut answer) = oneshot::channel();
    worker.command(
        Command::Take {
            session: 1,
            before: Vec::new(),
            reply,
        },
        0.0,
    );
    worker.step(0.0);
    handle.add_window(0);
    worker.step(49.0);
    assert!(answer.try_recv().is_err(), "the next poll is not due yet");
    worker.step(50.0);
    assert_eq!(answer.try_recv().unwrap(), Ok((1349, 809)));
}

#[test]
fn a_captured_editor_sends_its_pictures_skipping_a_same_one() {
    let (mut worker, handle, heard) = worker();
    handle.still(true);
    take(&mut worker, &handle, 1, 0.0);
    worker.step(10.0);
    assert!(
        heard.lock().unwrap().is_empty(),
        "no capture before its sink"
    );
    let (sink, frames) = sink();
    worker.command(Command::Capture { session: 1, sink }, 20.0);
    worker.command(
        Command::Capture {
            session: 9,
            sink: super::tests::sink().0,
        },
        20.0,
    );
    worker.step(20.0);
    assert_eq!(frames.lock().unwrap().len(), 1);
    assert_eq!(frames.lock().unwrap()[0].0, 1);
    worker.step(59.0);
    worker.step(60.0);
    assert_eq!(frames.lock().unwrap().len(), 1, "the same picture: skipped");
    handle.still(false);
    std::thread::sleep(Duration::from_millis(260));
    worker.step(100.0);
    assert_eq!(frames.lock().unwrap().len(), 2, "a new picture");
    // The minute's counts: three grabs, two frames sent.
    worker.step(60_000.0);
    let events = heard.lock().unwrap().clone();
    let [PlugwinEvent::Rate { session: 1, rate }] = events.as_slice() else {
        panic!("one rate: {events:?}")
    };
    assert_eq!((rate.grabs, rate.sent, rate.failed), (3, 2, 0));
    assert_eq!((rate.width, rate.height), (1349, 809));
    assert!(rate.bytes > 0.0);
    // The window goes away: lost.
    let window = handle.windows()[0];
    handle.remove_window(window);
    worker.step(60_100.0);
    assert_eq!(
        heard.lock().unwrap().last(),
        Some(&PlugwinEvent::Lost { session: 1 })
    );
    assert!(worker.editors.is_empty());
}

#[test]
fn a_contact_goes_to_its_editor_one_at_a_time_and_a_refused_one_ends() {
    let (mut worker, handle, heard) = worker();
    take(&mut worker, &handle, 1, 0.0);
    take(&mut worker, &handle, 2, 0.0);
    let touch = |worker: &mut Worker, session: u32, phase: Phase, at: (i32, i32)| {
        worker.command(Command::Touch { session, phase, at }, 0.0);
    };
    touch(&mut worker, 1, Phase::Down, (10, 20));
    touch(&mut worker, 2, Phase::Down, (1, 1));
    touch(&mut worker, 9, Phase::Down, (1, 1));
    touch(&mut worker, 1, Phase::Update, (11, 21));
    assert_eq!(worker.contact, Some((1, (11, 21))));
    touch(&mut worker, 1, Phase::Up, (11, 21));
    assert_eq!(worker.contact, None);
    touch(&mut worker, 2, Phase::Down, (1, 1));
    touch(&mut worker, 2, Phase::Cancel, (1, 1));
    assert_eq!(
        touches(&handle),
        vec![
            t("down", 10, 20),
            t("update", 11, 21),
            t("up", 11, 21),
            t("down", 1, 1),
            t("cancel", 1, 1),
        ],
        "the second editor's down waited for the first contact's end"
    );
    // A refused down: no contact, said so.
    handle.refuse(true);
    touch(&mut worker, 1, Phase::Down, (5, 5));
    assert_eq!(worker.contact, None);
    assert_eq!(
        heard.lock().unwrap().last(),
        Some(&PlugwinEvent::ContactEnded {
            session: 1,
            why: REFUSED.to_string()
        })
    );
    // A refused update: the contact ends with a cancel at its last point.
    handle.refuse(false);
    touch(&mut worker, 1, Phase::Down, (5, 5));
    handle.refuse(true);
    touch(&mut worker, 1, Phase::Update, (6, 6));
    assert_eq!(worker.contact, None);
    assert_eq!(touches(&handle)[5..], [t("down", 5, 5), t("cancel", 5, 5)]);
    assert_eq!(heard.lock().unwrap().len(), 2);
}

#[test]
fn the_guard_ends_the_contact_stops_the_frames_and_taps_the_inert_spot() {
    let (mut worker, handle, _) = worker();
    handle.still(true);
    take(&mut worker, &handle, 1, 0.0);
    let (sink, frames) = sink();
    worker.command(Command::Capture { session: 1, sink }, 0.0);
    worker.step(0.0);
    worker.command(
        Command::Touch {
            session: 1,
            phase: Phase::Down,
            at: (10, 20),
        },
        0.0,
    );
    let (reply, mut answer) = oneshot::channel();
    worker.command(Command::Guard { session: 1, reply }, 1.0);
    assert_eq!(answer.try_recv().unwrap(), Ok(()));
    assert_eq!(worker.contact, None);
    assert_eq!(
        touches(&handle),
        vec![
            t("down", 10, 20),
            t("up", 10, 20),
            t("down", 546, 15),
            t("up", 546, 15),
        ]
    );
    handle.still(false);
    std::thread::sleep(Duration::from_millis(260));
    worker.step(100.0);
    assert_eq!(frames.lock().unwrap().len(), 1, "no frame after the guard");
    // A guard of no editor, and one whose tap is refused, fail.
    let (reply, mut answer) = oneshot::channel();
    worker.command(Command::Guard { session: 7, reply }, 2.0);
    assert_eq!(answer.try_recv().unwrap(), Err(NO_EDITOR.to_string()));
    handle.refuse(true);
    let (reply, mut answer) = oneshot::channel();
    worker.command(Command::Guard { session: 1, reply }, 3.0);
    assert_eq!(answer.try_recv().unwrap(), Err(REFUSED.to_string()));
}

#[test]
fn a_release_ends_the_contact_and_hands_the_window_back() {
    let (mut worker, handle, _) = worker();
    let window = take(&mut worker, &handle, 1, 0.0);
    worker.command(
        Command::Touch {
            session: 1,
            phase: Phase::Down,
            at: (3, 4),
        },
        0.0,
    );
    let (reply, mut answer) = oneshot::channel();
    worker.command(Command::Release { session: 1, reply }, 1.0);
    assert_eq!(answer.try_recv(), Ok(()));
    assert!(worker.editors.is_empty());
    assert_eq!(worker.contact, None);
    assert_eq!(touches(&handle), vec![t("down", 3, 4), t("up", 3, 4)]);
    assert_eq!(
        handle.records().last(),
        Some(&json!({"op": "release", "window": window.0}))
    );
    // A release of no editor still answers.
    let (reply, mut answer) = oneshot::channel();
    worker.command(Command::Release { session: 1, reply }, 2.0);
    assert_eq!(answer.try_recv(), Ok(()));
    // The stop hands every window back.
    take(&mut worker, &handle, 2, 3.0);
    take(&mut worker, &handle, 3, 3.0);
    worker.command(Command::Stop, 4.0);
    worker.shutdown();
    assert!(worker.editors.is_empty());
    let released = handle
        .records()
        .iter()
        .filter(|r| r["op"] == "release")
        .count();
    assert_eq!(released, 3);
}

/// Waits (bounded) for `check`.
async fn until(what: &str, check: impl Fn() -> bool) {
    for _ in 0..200 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never: {what}");
}

#[tokio::test]
async fn the_worker_thread_serves_its_handle_and_ends_at_the_stop() {
    let (sim, handle) = Sim::new(None);
    let heard: Heard = Arc::new(Mutex::new(Vec::new()));
    let into = Arc::clone(&heard);
    let plugwin = Plugwin::spawn(
        Box::new(sim),
        Arc::new(move |event: PlugwinEvent| into.lock().unwrap().push(event)),
    )
    .unwrap();
    let bounded = Duration::from_secs(5);
    let before = tokio::time::timeout(bounded, plugwin.list())
        .await
        .unwrap()
        .unwrap();
    assert!(before.is_empty());
    let size = tokio::time::timeout(bounded, plugwin.take(1, before))
        .await
        .unwrap();
    assert_eq!(size, Ok((1349, 809)));
    let (sink, frames) = sink();
    plugwin.capture(1, sink);
    until("a frame", || !frames.lock().unwrap().is_empty()).await;
    plugwin.touch(1, Phase::Down, (8, 9));
    until("the touch", || touches(&handle) == vec![t("down", 8, 9)]).await;
    let guarded = tokio::time::timeout(bounded, plugwin.guard(1))
        .await
        .unwrap();
    assert_eq!(guarded, Ok(()));
    tokio::time::timeout(bounded, plugwin.release(1))
        .await
        .unwrap();
    assert!(handle.windows().is_empty(), "Live closed it");
    assert_eq!(
        tokio::time::timeout(bounded, plugwin.guard(1))
            .await
            .unwrap(),
        Err(NO_EDITOR.to_string())
    );
    plugwin.stop();
    let stopped = plugwin.clone();
    until("the worker gone", move || {
        let (reply, _answer) = oneshot::channel();
        stopped.tx.send(Command::List(reply)).is_err()
    })
    .await;
    assert_eq!(plugwin.list().await, Err(STOPPED.to_string()));
    assert_eq!(plugwin.take(2, Vec::new()).await, Err(STOPPED.to_string()));
    assert_eq!(plugwin.guard(2).await, Err(STOPPED.to_string()));
    tokio::time::timeout(bounded, plugwin.release(2))
        .await
        .unwrap();
    assert!(heard.lock().unwrap().is_empty());
}
