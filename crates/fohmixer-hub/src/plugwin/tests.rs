use std::cell::Cell;
use std::sync::Mutex;

use serde_json::json;

use super::sim::{REFUSED, Sim, SimHandle};
use super::*;

/// The events a worker told, in order.
type Heard = Arc<Mutex<Vec<PlugwinEvent>>>;

thread_local! {
    /// The clock of a worker run by hand on the test's thread (ms): it
    /// stands where the test set it ([`Worker::step_at`],
    /// [`Worker::command_at`]); a slow backend's grab moves it on ([`Slow`]).
    static NOW: Cell<f64> = const { Cell::new(0.0) };
}

impl Worker {
    /// One step at `now` on the test's clock.
    fn step_at(&mut self, now: f64) {
        NOW.with(|clock| clock.set(now));
        self.step();
    }

    /// One command at `now` on the test's clock.
    fn command_at(&mut self, command: Command, now: f64) {
        NOW.with(|clock| clock.set(now));
        self.command(command);
    }
}

/// A worker on a simulated backend, run by hand (`command_at`, `step_at`
/// at chosen times).
fn worker() -> (Worker, SimHandle, Heard) {
    let (sim, handle) = Sim::new(None);
    let (worker, heard) = worker_on(Box::new(sim));
    (worker, handle, heard)
}

/// A worker on `backend`, run by hand on the test's clock.
fn worker_on(backend: Box<dyn Backend>) -> (Worker, Heard) {
    let heard: Heard = Arc::new(Mutex::new(Vec::new()));
    let into = Arc::clone(&heard);
    let worker = Worker {
        backend,
        events: Arc::new(move |event: PlugwinEvent| into.lock().unwrap().push(event)),
        editors: BTreeMap::new(),
        finding: Vec::new(),
        contact: None,
        encoder: Encoder::spawn().unwrap(),
        clock: Box::new(|| NOW.with(Cell::get)),
    };
    (worker, heard)
}

/// Waits (bounded, 5 s) on the test's own thread for `check`.
fn wait_for(what: &str, check: impl Fn() -> bool) {
    for _ in 0..500 {
        if check() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("never: {what}");
}

/// Takes the editor Live opens as `session` at `now`; its window.
fn take(worker: &mut Worker, handle: &SimHandle, session: u32, now: f64) -> WindowId {
    let before = handle.windows();
    let (reply, mut answer) = oneshot::channel();
    worker.command_at(
        Command::Take {
            session,
            before,
            reply,
        },
        now,
    );
    worker.step_at(now);
    assert_eq!(answer.try_recv().unwrap(), Ok((1349, 809)));
    *handle.windows().last().unwrap()
}

/// A phase of contact `contact` on `session` at `now`.
fn touch_at(
    worker: &mut Worker,
    session: u32,
    contact: u32,
    phase: Phase,
    at: (i32, i32),
    now: f64,
) {
    let command = Command::Touch {
        session,
        contact,
        phase,
        at,
    };
    worker.command_at(command, now);
}

/// The worker's contact: its session, its number and its point.
fn held(worker: &Worker) -> Option<(u32, u32, (i32, i32))> {
    worker
        .contact
        .map(|held| (held.session, held.contact, held.at))
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
    assert_eq!(found(&Pick::One(b), true, 0.0), Some(Ok(b)));
    assert_eq!(found(&Pick::None, false, 2999.0), None);
    assert_eq!(found(&Pick::None, false, 3000.0), Some(Err(NO_WINDOW)));
    assert_eq!(found(&Pick::Several, false, 2999.0), None);
    assert_eq!(found(&Pick::Several, false, 3000.0), Some(Err(SEVERAL)));
    // One new window whose picture is not there yet: awaited, refused only
    // once the wait is over.
    assert_eq!(found(&Pick::One(b), false, 2999.0), None);
    assert_eq!(
        found(&Pick::One(b), false, 3000.0),
        Some(Err(reason::NO_PICTURE))
    );
    assert_eq!(found(&Pick::One(b), true, 3000.0), Some(Ok(b)));
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
        gap_ms: 62.5,
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
            gap_ms: 62.5,
        }
    );
    assert_eq!(Counts::default().rate(), Rate::default());
}

#[test]
fn a_minutes_counts_add_up_the_grabs_and_the_encoder_s_frames() {
    let mut counts = Counts::default();
    counts.grabbed(2.5, true);
    counts.grabbed(4.0, false);
    counts.encoded(3.0, Some(1000));
    counts.encoded(5.5, Some(3000));
    counts.encoded(9.0, None);
    // The largest gap between a contact's injections.
    counts.injected(30.0);
    counts.injected(80.0);
    counts.injected(10.0);
    assert_eq!(
        counts,
        Counts {
            grabs: 2,
            sent: 2,
            failed: 2,
            grab_ms: 6.5,
            encode_ms: 8.5,
            bytes: 4000.0,
            width: 0,
            height: 0,
            gap_ms: 80.0,
        }
    );
    assert_eq!(millis(Duration::from_millis(1500)), 1500.0);
    assert_eq!(millis(Duration::from_micros(2500)), 2.5);
    assert_eq!(millis(Duration::ZERO), 0.0);
}

/// A picture for the encoder, `width` × 8 px.
fn job(session: u32, width: u32, sink: FrameSink) -> Job {
    Job {
        session,
        pixels: Arc::new(sim::picture(1, width, 8)),
        sink,
    }
}

#[test]
fn the_encoder_waits_while_no_picture_waits_and_until_the_stop() {
    let none = |waiting: bool, stopped: bool| Handoff {
        waiting: waiting.then(|| job(1, 16, sink().0)),
        done: Vec::new(),
        stopped,
    };
    assert!(idle(&mut none(false, false)));
    assert!(!idle(&mut none(true, false)));
    assert!(!idle(&mut none(false, true)));
    assert!(!idle(&mut none(true, true)));
}

#[test]
fn the_newest_picture_wins_and_a_forgotten_one_is_dropped() {
    // The hand-off alone, no thread: what the encoder would take next.
    let mut encoder = Encoder {
        shared: Arc::new(Shared::default()),
        thread: None,
    };
    let waiting = |encoder: &Encoder| {
        encoder
            .shared
            .lock()
            .waiting
            .as_ref()
            .map(|job| (job.session, job.pixels.width))
    };
    encoder.put(1, job(1, 16, sink().0).pixels, sink().0);
    encoder.put(2, job(2, 24, sink().0).pixels, sink().0);
    assert_eq!(waiting(&encoder), Some((2, 24)), "the newest wins");
    encoder.forget(1);
    assert_eq!(waiting(&encoder), Some((2, 24)), "another session's");
    let next = encoder.shared.wait_job().expect("a picture waits");
    assert_eq!((next.session, next.pixels.width), (2, 24));
    assert_eq!(waiting(&encoder), None, "taken");
    encoder.put(3, job(3, 16, sink().0).pixels, sink().0);
    encoder.forget(3);
    assert_eq!(waiting(&encoder), None, "forgotten");
    // What it made is handed over once.
    let made = Encoded {
        session: 2,
        ms: 1.5,
        bytes: Some(10),
    };
    encoder.shared.lock().done.push(made);
    assert_eq!(encoder.done(), vec![made]);
    assert_eq!(encoder.done(), Vec::new());
    // Stopped: a waiting picture is dropped, and there is no next one.
    encoder.put(4, job(4, 16, sink().0).pixels, sink().0);
    encoder.stop();
    assert_eq!(waiting(&encoder), None);
    assert!(encoder.shared.wait_job().is_none());
}

#[test]
fn the_encoders_thread_sends_each_jpeg_to_its_sink_and_ends_at_the_stop() {
    let mut encoder = Encoder::spawn().unwrap();
    let (sink, frames) = sink();
    encoder.put(5, job(5, 16, sink.clone()).pixels, sink.clone());
    wait_for("a frame", || frames.lock().unwrap().len() == 1);
    let got = frames.lock().unwrap().clone();
    let [(session, size)] = got.as_slice() else {
        panic!("one frame: {got:?}")
    };
    let (session, size) = (*session, *size);
    assert_eq!(session, 5);
    let done = encoder.done();
    assert_eq!(done.len(), 1, "reported before the sink got it");
    assert_eq!((done[0].session, done[0].bytes), (5, Some(size)));
    assert!(done[0].ms >= 0.0);
    // A picture that cannot be encoded is reported, and no frame sent.
    let broken = Arc::new(Pixels {
        width: 16,
        height: 8,
        bgra: vec![0; 10],
    });
    encoder.put(6, broken, sink);
    wait_for("its report", || {
        encoder.shared.lock().done.iter().any(|e| e.session == 6)
    });
    assert_eq!(encoder.done()[0].bytes, None);
    assert_eq!(frames.lock().unwrap().len(), 1);
    encoder.stop();
    assert!(encoder.thread.is_none(), "joined");
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
    worker.command_at(Command::List(reply), 0.0);
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
    worker.command_at(
        Command::Take {
            session: 1,
            before: Vec::new(),
            reply,
        },
        100.0,
    );
    worker.step_at(100.0);
    worker.step_at(149.0);
    worker.step_at(150.0);
    worker.step_at(3099.0);
    assert!(answer.try_recv().is_err(), "still waiting");
    worker.step_at(3100.0);
    assert_eq!(answer.try_recv().unwrap(), Err(NO_WINDOW.to_string()));
    assert!(worker.finding.is_empty());
    // Two new windows at once: refused once the wait is over.
    let (reply, mut answer) = oneshot::channel();
    worker.command_at(
        Command::Take {
            session: 2,
            before: Vec::new(),
            reply,
        },
        5000.0,
    );
    handle.add_window(0);
    handle.add_window(0);
    worker.step_at(5000.0);
    assert!(answer.try_recv().is_err());
    worker.step_at(8000.0);
    assert_eq!(answer.try_recv().unwrap(), Err(SEVERAL.to_string()));
    assert!(worker.editors.is_empty());
}

#[test]
fn a_window_whose_pro_q_picture_comes_late_is_awaited_until_the_wait_is_over() {
    // Live shows its editor's window before Pro-Q attaches its picture
    // (`FF_UIWindow`): the open waits for it.
    let (mut worker, handle, _) = worker();
    handle.child_late(true);
    let (reply, mut answer) = oneshot::channel();
    let take = Command::Take {
        session: 1,
        before: Vec::new(),
        reply,
    };
    worker.command_at(take, 0.0);
    worker.step_at(0.0);
    assert!(answer.try_recv().is_err(), "its picture is not there yet");
    worker.step_at(50.0);
    assert!(answer.try_recv().is_err());
    handle.child_late(false);
    worker.step_at(100.0);
    assert_eq!(answer.try_recv().unwrap(), Ok((1349, 809)));
    // One whose picture never comes fails once the wait is over.
    handle.child_late(true);
    let (reply, mut answer) = oneshot::channel();
    let take = Command::Take {
        session: 2,
        before: handle.windows(),
        reply,
    };
    worker.command_at(take, 1000.0);
    worker.step_at(1000.0);
    worker.step_at(3999.0);
    assert!(answer.try_recv().is_err(), "still waiting");
    worker.step_at(4000.0);
    assert_eq!(
        answer.try_recv().unwrap(),
        Err(reason::NO_PICTURE.to_string())
    );
    assert_eq!(worker.editors.keys().copied().collect::<Vec<u32>>(), [1]);
}

#[test]
fn a_late_window_is_found_by_a_later_poll() {
    let (mut worker, handle, _) = worker();
    handle.auto_open(false);
    let (reply, mut answer) = oneshot::channel();
    worker.command_at(
        Command::Take {
            session: 1,
            before: Vec::new(),
            reply,
        },
        0.0,
    );
    worker.step_at(0.0);
    handle.add_window(0);
    worker.step_at(49.0);
    assert!(answer.try_recv().is_err(), "the next poll is not due yet");
    worker.step_at(50.0);
    assert_eq!(answer.try_recv().unwrap(), Ok((1349, 809)));
}

#[test]
fn an_open_late_in_the_workers_life_counts_its_own_poll_and_wait() {
    // Asked for 10 s into the worker's clock: its polls and its wait count
    // from there, not from the worker's start.
    let (mut worker, handle, _) = worker();
    handle.auto_open(false);
    let (reply, mut answer) = oneshot::channel();
    worker.command_at(
        Command::Take {
            session: 1,
            before: Vec::new(),
            reply,
        },
        10_000.0,
    );
    worker.step_at(10_000.0);
    assert!(answer.try_recv().is_err(), "its wait has just begun");
    handle.add_window(0);
    worker.step_at(10_049.0);
    assert!(answer.try_recv().is_err(), "the next poll is not due yet");
    worker.step_at(10_050.0);
    assert_eq!(answer.try_recv().unwrap(), Ok((1349, 809)));
}

#[test]
fn a_minutes_rate_counts_from_the_editors_take() {
    let (mut worker, handle, heard) = worker();
    take(&mut worker, &handle, 1, 100_000.0);
    worker.step_at(100_010.0);
    assert!(
        heard.lock().unwrap().is_empty(),
        "no rate right after the take"
    );
    worker.step_at(160_000.0);
    let events = heard.lock().unwrap().clone();
    assert!(
        matches!(events.as_slice(), [PlugwinEvent::Rate { session: 1, .. }]),
        "one rate a minute after the take: {events:?}"
    );
}

#[test]
fn a_captured_editor_sends_its_pictures_skipping_a_same_one() {
    let (mut worker, handle, heard) = worker();
    handle.still(true);
    take(&mut worker, &handle, 1, 0.0);
    worker.step_at(10.0);
    assert!(
        heard.lock().unwrap().is_empty(),
        "no capture before its sink"
    );
    let (sink, frames) = sink();
    worker.command_at(Command::Capture { session: 1, sink }, 20.0);
    worker.command_at(
        Command::Capture {
            session: 9,
            sink: super::tests::sink().0,
        },
        20.0,
    );
    worker.step_at(20.0);
    // The encoder's thread makes the frame.
    wait_for("the first frame", || frames.lock().unwrap().len() == 1);
    assert_eq!(frames.lock().unwrap()[0].0, 1);
    worker.step_at(59.0);
    // Still: the sim's counter moves on (one step is 250 ms), its picture
    // does not.
    std::thread::sleep(Duration::from_millis(260));
    worker.step_at(60.0);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(frames.lock().unwrap().len(), 1, "the same picture: skipped");
    handle.still(false);
    std::thread::sleep(Duration::from_millis(260));
    worker.step_at(100.0);
    wait_for("a new picture", || frames.lock().unwrap().len() == 2);
    // Moving: the picture changes with the time.
    std::thread::sleep(Duration::from_millis(260));
    worker.step_at(140.0);
    wait_for("the picture after it", || frames.lock().unwrap().len() == 3);
    // The minute's counts: four grabs, three frames sent.
    worker.step_at(60_000.0);
    let events = heard.lock().unwrap().clone();
    let [PlugwinEvent::Rate { session: 1, rate }] = events.as_slice() else {
        panic!("one rate: {events:?}")
    };
    assert_eq!((rate.grabs, rate.sent, rate.failed), (4, 3, 0));
    assert_eq!((rate.width, rate.height), (1349, 809));
    assert!(rate.bytes > 0.0);
    // The window goes away: lost.
    let window = handle.windows()[0];
    handle.remove_window(window);
    worker.step_at(60_100.0);
    assert_eq!(
        heard.lock().unwrap().last(),
        Some(&PlugwinEvent::Lost { session: 1 })
    );
    assert!(worker.editors.is_empty());
}

#[test]
fn a_resting_contact_is_injected_again_50_ms_after_its_last_injection() {
    let (mut worker, handle, heard) = worker();
    take(&mut worker, &handle, 1, 0.0);
    touch_at(&mut worker, 1, 1, Phase::Down, (10, 20), 1000.0);
    worker.step_at(1049.0);
    assert_eq!(touches(&handle), vec![t("down", 10, 20)]);
    worker.step_at(1050.0);
    assert_eq!(touches(&handle)[1..], [t("update", 10, 20)]);
    worker.step_at(1099.0);
    assert_eq!(touches(&handle).len(), 2, "counted from the last injection");
    // A move is an injection: the next keep-alive counts from it.
    touch_at(&mut worker, 1, 1, Phase::Update, (12, 20), 1120.0);
    worker.step_at(1169.0);
    assert_eq!(touches(&handle).len(), 3);
    worker.step_at(1170.0);
    assert_eq!(touches(&handle)[3..], [t("update", 12, 20)]);
    // Ended: nothing more.
    touch_at(&mut worker, 1, 1, Phase::Up, (12, 20), 1180.0);
    worker.step_at(1300.0);
    assert_eq!(touches(&handle)[4..], [t("up", 12, 20)]);
    // A keep-alive the point refuses (another window over it) ends the
    // contact with a cancel at its last point, and says so.
    touch_at(&mut worker, 1, 2, Phase::Down, (5, 5), 2000.0);
    handle.refuse(true);
    worker.step_at(2050.0);
    assert_eq!(worker.contact, None);
    assert_eq!(touches(&handle)[5..], [t("down", 5, 5), t("cancel", 5, 5)]);
    assert_eq!(
        heard.lock().unwrap().last(),
        Some(&PlugwinEvent::ContactEnded {
            session: 1,
            contact: 2,
            why: REFUSED.to_string()
        })
    );
}

#[test]
fn a_minute_holds_the_largest_gap_between_a_contacts_injections() {
    let (mut worker, handle, heard) = worker();
    take(&mut worker, &handle, 1, 0.0);
    assert!(!keepalive_due(50.0_f64.next_down()));
    assert!(keepalive_due(50.0));
    touch_at(&mut worker, 1, 1, Phase::Down, (10, 20), 100.0);
    touch_at(&mut worker, 1, 1, Phase::Update, (11, 20), 130.0);
    worker.step_at(180.0);
    touch_at(&mut worker, 1, 1, Phase::Update, (12, 20), 260.0);
    touch_at(&mut worker, 1, 1, Phase::Up, (12, 20), 270.0);
    assert_eq!(
        touches(&handle),
        vec![
            t("down", 10, 20),
            t("update", 11, 20),
            t("update", 11, 20),
            t("update", 12, 20),
            t("up", 12, 20),
        ]
    );
    worker.step_at(60_000.0);
    let events = heard.lock().unwrap().clone();
    let [PlugwinEvent::Rate { session: 1, rate }] = events.as_slice() else {
        panic!("one rate: {events:?}")
    };
    assert_eq!(rate.gap_ms, 80.0, "130 to 180 to 260 to 270");
}

#[test]
fn the_keep_alive_is_looked_at_first_and_after_each_grab_on_a_fresh_clock() {
    // Two editors captured, each grab taking 35 ms of the worker's clock.
    let (sim, handle) = Sim::new(None);
    handle.still(true);
    let slow = Slow {
        sim,
        hold: Duration::ZERO,
        grab_ms: 35.0,
    };
    let (mut worker, _) = worker_on(Box::new(slow));
    take(&mut worker, &handle, 1, 0.0);
    take(&mut worker, &handle, 2, 0.0);
    for session in [1, 2] {
        let capture = Command::Capture {
            session,
            sink: sink().0,
        };
        worker.command_at(capture, 0.0);
    }
    touch_at(&mut worker, 1, 1, Phase::Down, (10, 20), 1000.0);
    // At 1040 nothing is due; the first grab ends at 1075, when the
    // contact is 75 ms old: it goes then, not after the second grab.
    worker.step_at(1040.0);
    assert_eq!(
        touches(&handle),
        vec![t("down", 10, 20), t("update", 10, 20)]
    );
    let sent = |worker: &Worker| worker.contact.map(|held| held.sent);
    assert_eq!(sent(&worker), Some(1075.0), "stamped when it went");
    assert_eq!(worker.editors[&1].counts.gap_ms, 75.0);
    // At 1130 it is due before any grab (55 ms): it goes first, then again
    // after the second grab (1200).
    worker.step_at(1130.0);
    assert_eq!(touches(&handle).len(), 4);
    assert_eq!(sent(&worker), Some(1200.0));
    assert_eq!(worker.editors[&1].counts.gap_ms, 75.0, "55 and 70 since");
}

#[test]
fn a_contact_goes_to_its_editor_one_at_a_time_and_a_refused_one_ends() {
    let (mut worker, handle, heard) = worker();
    take(&mut worker, &handle, 1, 0.0);
    take(&mut worker, &handle, 2, 0.0);
    let touch = |worker: &mut Worker, session: u32, contact: u32, phase: Phase, at: (i32, i32)| {
        touch_at(worker, session, contact, phase, at, 0.0);
    };
    touch(&mut worker, 1, 1, Phase::Down, (10, 20));
    touch(&mut worker, 2, 2, Phase::Down, (1, 1));
    touch(&mut worker, 9, 3, Phase::Down, (1, 1));
    touch(&mut worker, 1, 1, Phase::Update, (11, 21));
    assert_eq!(held(&worker), Some((1, 1, (11, 21))));
    touch(&mut worker, 1, 1, Phase::Up, (11, 21));
    assert_eq!(worker.contact, None);
    touch(&mut worker, 2, 2, Phase::Down, (1, 1));
    touch(&mut worker, 2, 2, Phase::Cancel, (1, 1));
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
    // A phase of no contact down, of another number or of another session
    // is dropped.
    touch(&mut worker, 2, 2, Phase::Update, (3, 3));
    touch(&mut worker, 1, 4, Phase::Down, (7, 7));
    touch(&mut worker, 1, 3, Phase::Update, (8, 8));
    touch(&mut worker, 2, 4, Phase::Up, (7, 7));
    assert_eq!(held(&worker), Some((1, 4, (7, 7))));
    touch(&mut worker, 1, 4, Phase::Up, (7, 7));
    assert_eq!(touches(&handle)[5..], [t("down", 7, 7), t("up", 7, 7)]);
    // A refused down: no contact, said so.
    handle.refuse(true);
    touch(&mut worker, 1, 5, Phase::Down, (5, 5));
    assert_eq!(worker.contact, None);
    let ended = |contact: u32| PlugwinEvent::ContactEnded {
        session: 1,
        contact,
        why: REFUSED.to_string(),
    };
    assert_eq!(heard.lock().unwrap().last(), Some(&ended(5)));
    // A refused update: the contact ends with a cancel at its last point.
    handle.refuse(false);
    touch(&mut worker, 1, 6, Phase::Down, (5, 5));
    handle.refuse(true);
    touch(&mut worker, 1, 6, Phase::Update, (6, 6));
    assert_eq!(worker.contact, None);
    assert_eq!(touches(&handle)[7..], [t("down", 5, 5), t("cancel", 5, 5)]);
    assert_eq!(heard.lock().unwrap().as_slice(), [ended(5), ended(6)]);
}

#[test]
fn a_down_goes_when_no_contact_is_down_and_the_rest_only_to_the_one_down() {
    let all = [Phase::Down, Phase::Update, Phase::Up, Phase::Cancel];
    assert_eq!(
        all.map(|p| accepts(None, 1, 1, p)),
        [true, false, false, false]
    );
    assert_eq!(
        all.map(|p| accepts(Some((1, 1)), 1, 1, p)),
        [false, true, true, true]
    );
    assert_eq!(all.map(|p| accepts(Some((1, 1)), 1, 2, p)), [false; 4]);
    assert_eq!(all.map(|p| accepts(Some((1, 1)), 2, 1, p)), [false; 4]);
}

#[test]
fn the_guard_ends_the_contact_stops_the_frames_and_taps_the_inert_spot() {
    let (mut worker, handle, _) = worker();
    handle.still(true);
    take(&mut worker, &handle, 1, 0.0);
    let (sink, frames) = sink();
    worker.command_at(Command::Capture { session: 1, sink }, 0.0);
    worker.step_at(0.0);
    wait_for("the first frame", || frames.lock().unwrap().len() == 1);
    touch_at(&mut worker, 1, 1, Phase::Down, (10, 20), 0.0);
    let (reply, mut answer) = oneshot::channel();
    worker.command_at(Command::Guard { session: 1, reply }, 1.0);
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
    worker.step_at(100.0);
    assert_eq!(frames.lock().unwrap().len(), 1, "no frame after the guard");
    // A guard of no editor, and one whose tap is refused, fail.
    let (reply, mut answer) = oneshot::channel();
    worker.command_at(Command::Guard { session: 7, reply }, 2.0);
    assert_eq!(answer.try_recv().unwrap(), Err(NO_EDITOR.to_string()));
    handle.refuse(true);
    let (reply, mut answer) = oneshot::channel();
    worker.command_at(Command::Guard { session: 1, reply }, 3.0);
    assert_eq!(answer.try_recv().unwrap(), Err(REFUSED.to_string()));
}

#[test]
fn the_guard_first_cancels_another_sessions_contact() {
    // Two engineers: one drags on the first editor while the other's
    // editor closes. The PC injects one contact: the guard's tap would
    // fail, or its up would end the other's drag.
    let (mut worker, handle, heard) = worker();
    let first = take(&mut worker, &handle, 1, 0.0);
    let second = take(&mut worker, &handle, 2, 0.0);
    touch_at(&mut worker, 1, 7, Phase::Down, (10, 20), 0.0);
    let (reply, mut answer) = oneshot::channel();
    worker.command_at(Command::Guard { session: 2, reply }, 1.0);
    assert_eq!(answer.try_recv().unwrap(), Ok(()));
    assert_eq!(worker.contact, None, "the other engineer's contact ended");
    let on = |r: &serde_json::Value| {
        (
            r["window"].as_u64().unwrap(),
            r["phase"].as_str().unwrap().to_string(),
            r["x"].as_i64().unwrap(),
            r["y"].as_i64().unwrap(),
        )
    };
    let touched: Vec<_> = handle
        .records()
        .iter()
        .filter(|r| r["op"] == "touch")
        .map(on)
        .collect();
    let at = |window: WindowId, phase: &str, x: i64, y: i64| (window.0, phase.to_string(), x, y);
    assert_eq!(
        touched,
        vec![
            at(first, "down", 10, 20),
            at(first, "cancel", 10, 20),
            at(second, "down", 546, 15),
            at(second, "up", 546, 15),
        ]
    );
    assert_eq!(
        heard.lock().unwrap().as_slice(),
        [PlugwinEvent::ContactEnded {
            session: 1,
            contact: 7,
            why: "the close guard of another editor".to_string()
        }]
    );
}

#[test]
fn a_guard_of_an_editor_the_worker_no_longer_holds_ends_no_contact() {
    // A close racing its lost window: its guard fails, and the other
    // engineer's drag goes on untouched.
    let (mut worker, handle, heard) = worker();
    take(&mut worker, &handle, 1, 0.0);
    touch_at(&mut worker, 1, 3, Phase::Down, (10, 20), 0.0);
    let (reply, mut answer) = oneshot::channel();
    worker.command_at(Command::Guard { session: 2, reply }, 1.0);
    assert_eq!(answer.try_recv().unwrap(), Err(NO_EDITOR.to_string()));
    assert_eq!(held(&worker), Some((1, 3, (10, 20))));
    assert_eq!(touches(&handle), vec![t("down", 10, 20)]);
    assert!(heard.lock().unwrap().is_empty());
}

#[test]
fn a_window_lost_under_a_contact_ends_it_with_a_cancel_at_its_last_point() {
    let (mut worker, handle, heard) = worker();
    let window = take(&mut worker, &handle, 1, 0.0);
    take(&mut worker, &handle, 2, 0.0);
    touch_at(&mut worker, 1, 1, Phase::Down, (10, 20), 0.0);
    touch_at(&mut worker, 1, 1, Phase::Update, (12, 24), 0.0);
    // The other editor's window going away leaves this contact alone.
    let other = handle.windows()[1];
    handle.remove_window(other);
    worker.step_at(10.0);
    assert_eq!(held(&worker), Some((1, 1, (12, 24))));
    assert_eq!(touches(&handle).len(), 2);
    handle.remove_window(window);
    worker.step_at(20.0);
    assert_eq!(worker.contact, None);
    assert_eq!(
        touches(&handle),
        vec![t("down", 10, 20), t("update", 12, 24), t("cancel", 12, 24)]
    );
    assert_eq!(
        heard.lock().unwrap().as_slice(),
        [
            PlugwinEvent::Lost { session: 2 },
            PlugwinEvent::Lost { session: 1 }
        ]
    );
}

#[test]
fn a_release_ends_the_contact_and_hands_the_window_back() {
    let (mut worker, handle, _) = worker();
    let window = take(&mut worker, &handle, 1, 0.0);
    touch_at(&mut worker, 1, 1, Phase::Down, (3, 4), 0.0);
    let (reply, mut answer) = oneshot::channel();
    let release = |session: u32, closed: bool, reply| Command::Release {
        session,
        closed,
        reply,
    };
    worker.command_at(release(1, true, reply), 1.0);
    assert_eq!(answer.try_recv(), Ok(()));
    assert!(worker.editors.is_empty());
    assert_eq!(worker.contact, None);
    assert_eq!(touches(&handle), vec![t("down", 3, 4), t("up", 3, 4)]);
    assert_eq!(
        handle.records().last(),
        Some(&json!({"op": "release", "window": window.0}))
    );
    assert!(handle.windows().is_empty(), "Live closed it");
    // A release of no editor still answers.
    let (reply, mut answer) = oneshot::channel();
    worker.command_at(release(1, true, reply), 2.0);
    assert_eq!(answer.try_recv(), Ok(()));
    // Handed back while Live keeps it open: its window stays.
    let open = take(&mut worker, &handle, 4, 2.5);
    let (reply, mut answer) = oneshot::channel();
    worker.command_at(release(4, false, reply), 2.5);
    assert_eq!(answer.try_recv(), Ok(()));
    assert_eq!(handle.windows(), vec![open]);
    assert_eq!(handle.topmost(open), Some(false), "z-order put back");
    // The stop hands every window back; the editors stay open in Live.
    take(&mut worker, &handle, 2, 3.0);
    take(&mut worker, &handle, 3, 3.0);
    worker.command_at(Command::Stop, 4.0);
    worker.shutdown();
    assert!(worker.editors.is_empty());
    let released = handle
        .records()
        .iter()
        .filter(|r| r["op"] == "release")
        .count();
    assert_eq!(released, 4);
    assert_eq!(handle.windows().len(), 3, "still open in Live");
}

/// A simulated backend whose release takes `hold` (a slow hand-back) and
/// whose grab moves the test's clock on by `grab_ms` (a grab's time on the
/// PC, for a worker run by hand).
struct Slow {
    sim: Sim,
    hold: Duration,
    grab_ms: f64,
}

impl Backend for Slow {
    fn editors(&mut self) -> Result<Vec<WindowId>, String> {
        self.sim.editors()
    }
    fn windows_of(&mut self, pid: u32) -> Result<Vec<WindowId>, String> {
        self.sim.windows_of(pid)
    }
    fn live_opened(&mut self) {
        self.sim.live_opened();
    }
    fn ready(&mut self, window: WindowId) -> bool {
        self.sim.ready(window)
    }
    fn take(&mut self, window: WindowId) -> Result<Taken, String> {
        self.sim.take(window)
    }
    fn alive(&mut self, taken: &Taken) -> bool {
        self.sim.alive(taken)
    }
    fn grab(&mut self, taken: &Taken) -> Result<Pixels, String> {
        NOW.with(|clock| clock.set(clock.get() + self.grab_ms));
        self.sim.grab(taken)
    }
    fn touch(&mut self, taken: &Taken, phase: Phase, at: (i32, i32)) -> Result<(), String> {
        self.sim.touch(taken, phase, at)
    }
    fn release(&mut self, taken: &Taken) {
        std::thread::sleep(self.hold);
        self.sim.release(taken);
    }
    fn close_window(&mut self, taken: &Taken) {
        self.sim.close_window(taken);
    }
}

/// A worker thread on a sim whose release takes `hold`, holding a window.
async fn slow_worker(hold: Duration) -> (Plugwin, SimHandle) {
    let (sim, handle) = Sim::new(None);
    let backend = Slow {
        sim,
        hold,
        grab_ms: 0.0,
    };
    let plugwin = Plugwin::spawn(Box::new(backend), Arc::new(|_: PlugwinEvent| {})).unwrap();
    assert_eq!(plugwin.take(1, Vec::new()).await, Ok((1349, 809)));
    (plugwin, handle)
}

#[tokio::test]
async fn the_stop_waits_for_the_worker_to_hand_its_windows_back() {
    assert_eq!((STOP_POLLS, STOP_POLL), (100, Duration::from_millis(10)));
    assert_eq!(STOP_WAIT, STOP_POLL * STOP_POLLS);
    let (plugwin, handle) = slow_worker(Duration::from_millis(300)).await;
    plugwin.stop();
    let started = Instant::now();
    assert!(plugwin.stopped().await, "ended within the wait");
    assert!(
        started.elapsed() >= Duration::from_millis(250),
        "waited for the release: {:?}",
        started.elapsed()
    );
    assert!(handle.records().iter().any(|r| r["op"] == "release"));
    assert!(plugwin.stopped().await, "a later look answers at once");
    // A worker that does not end within the wait: false, bounded.
    let (plugwin, _handle) = slow_worker(STOP_WAIT + Duration::from_millis(700)).await;
    plugwin.stop();
    let started = Instant::now();
    assert!(!plugwin.stopped().await);
    let waited = started.elapsed();
    assert!(
        (STOP_WAIT..Duration::from_secs(3)).contains(&waited),
        "{waited:?}"
    );
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
async fn a_resting_contact_goes_again_in_time_while_two_editors_grab_slowly() {
    // Two engineers, each with an editor captured, and every grab taking
    // 35 ms (the PC's BitBlt of a 4.3 MB picture): the keep-alive must not
    // wait for the step's grabs, or Windows cancels the resting contact
    // (no frame for 100 ms).
    let (sim, handle) = Sim::new(None);
    handle.still(true);
    handle.grab_delay(Duration::from_millis(35));
    let plugwin = Plugwin::spawn(Box::new(sim), Arc::new(|_: PlugwinEvent| {})).unwrap();
    let bounded = Duration::from_secs(5);
    for session in [1, 2] {
        let before = handle.windows();
        let size = tokio::time::timeout(bounded, plugwin.take(session, before))
            .await
            .unwrap();
        assert_eq!(size, Ok((1349, 809)));
        plugwin.capture(session, sink().0);
    }
    plugwin.touch(1, 1, Phase::Down, (8, 9));
    tokio::time::sleep(Duration::from_millis(1000)).await;
    plugwin.touch(1, 1, Phase::Up, (8, 9));
    until("the up", || touches(&handle).last() == Some(&t("up", 8, 9))).await;
    plugwin.stop();
    let records = handle.records();
    let touched: Vec<&serde_json::Value> = records.iter().filter(|r| r["op"] == "touch").collect();
    let resends = touched.iter().filter(|r| r["phase"] == "update").count();
    assert!(resends >= 8, "{touched:?}");
    assert!(
        !touched.iter().any(|r| r["phase"] == "cancel"),
        "{touched:?}"
    );
    let times: Vec<f64> = touched
        .iter()
        .map(|r| r["t"].as_f64().expect("a touch's time"))
        .collect();
    let largest = times
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .fold(0.0, f64::max);
    assert!(largest < 100.0, "the largest gap {largest} ms: {times:?}");
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
    plugwin.touch(1, 1, Phase::Down, (8, 9));
    // The down (a keep-alive may follow it 50 ms later).
    until("the touch", || {
        touches(&handle).first() == Some(&t("down", 8, 9))
    })
    .await;
    let guarded = tokio::time::timeout(bounded, plugwin.guard(1))
        .await
        .unwrap();
    assert_eq!(guarded, Ok(()));
    tokio::time::timeout(bounded, plugwin.release(1, true))
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
    tokio::time::timeout(bounded, plugwin.release(2, false))
        .await
        .unwrap();
    assert!(heard.lock().unwrap().is_empty());
}
