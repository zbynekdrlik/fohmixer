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
        guarding: Vec::new(),
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

/// The frames a test sink kept: each one's session, size and the input it
/// shows (PR J).
type Kept = Arc<Mutex<Vec<(u32, usize, u32)>>>;

/// A sink that keeps each frame's session, size and shown input.
fn sink() -> (FrameSink, Kept) {
    let frames = Arc::new(Mutex::new(Vec::new()));
    let into = Arc::clone(&frames);
    let sink: FrameSink = Arc::new(move |frame: Frame| {
        into.lock()
            .unwrap()
            .push((frame.session, frame.jpeg.len(), frame.shown));
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
    assert!(!refresh_due(1000.0_f64.next_down()));
    assert!(refresh_due(1000.0));
    assert_eq!(STEP, Duration::from_millis(10));
    assert_eq!(GUARD_TAP, Duration::from_millis(30));
    assert_eq!(QUALITY, 60);
}

#[test]
fn an_editor_is_quiet_again_100_ms_after_its_last_contact_ended() {
    assert_eq!(QUIET_MS, 100.0);
    assert!(!quiet(100.0_f64.next_down()));
    assert!(quiet(100.0));
    // No contact ended since the take (`ended` is minus infinity): quiet.
    assert!(quiet(0.0 - f64::NEG_INFINITY));
    // A time that is no number, or one before it ended: not.
    assert!(!quiet(f64::NAN));
    assert!(!quiet(f64::NEG_INFINITY));
}

#[test]
fn a_release_posts_the_takes_rectangle_only_when_changed_open_and_quiet() {
    assert_eq!(repost(true, false, QUIET_MS), Repost::Take);
    assert_eq!(
        repost(true, false, f64::INFINITY),
        Repost::Take,
        "no contact ended since the take"
    );
    assert_eq!(repost(true, false, QUIET_MS.next_down()), Repost::NotQuiet);
    assert_eq!(repost(true, false, 0.0), Repost::NotQuiet, "its own up");
    assert_eq!(
        repost(false, false, QUIET_MS),
        Repost::Nothing,
        "never changed"
    );
    assert_eq!(repost(false, false, 0.0), Repost::Nothing);
    assert_eq!(
        repost(true, true, QUIET_MS),
        Repost::Nothing,
        "Live closed it"
    );
    assert_eq!(repost(true, true, 0.0), Repost::Nothing);
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
fn a_taken_window_answers_once_on_top_or_fails_once_its_wait_is_over() {
    assert_eq!(placed(true, 0.0), Some(Ok(())));
    assert_eq!(placed(true, 3000.0), Some(Ok(())), "on top wins at the end");
    assert_eq!(placed(false, 0.0), None);
    assert_eq!(placed(false, 3000.0_f64.next_down()), None);
    assert_eq!(placed(false, 3000.0), Some(Err(NOT_ON_TOP)));
    assert_eq!(NOT_ON_TOP, reason::NOT_ON_TOP);
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
fn a_whole_sim_picture_at_the_frames_quality_is_under_24_kb_and_smaller_than_at_70() {
    // #71 PR I: the frames' bytes, measured. The sim's picture (Pro-Q 4's
    // 1349 x 809, a dark field and a bar) is far flatter than Pro-Q's, whose
    // real frames ran ~64 KB at q70 on the owner's iPad session; a q60 frame
    // of a real Pro-Q screenshot measured ~15 % smaller than at q70.
    let pixels = sim::picture(5, sim::SIM_WIDTH, sim::SIM_HEIGHT);
    let frame = encode(&pixels, QUALITY).unwrap();
    let at_70 = encode(&pixels, 70).unwrap();
    assert!(frame.len() <= 24 * 1024, "{} bytes", frame.len());
    assert!(
        frame.len() < at_70.len(),
        "{} bytes, {} at q70",
        frame.len(),
        at_70.len()
    );
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
        shown: 0,
        crop: None,
        size: (width, 8),
        pixels: Arc::new(sim::picture(1, width, 8)),
        sink,
    }
}

#[test]
fn the_encoder_waits_while_no_picture_waits_and_until_the_stop() {
    let none = |waiting: bool, stopped: bool| Handoff {
        waiting: waiting.then(|| job(1, 16, sink().0)),
        done: Vec::new(),
        captured: BTreeSet::new(),
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
    encoder.put(job(1, 16, sink().0));
    encoder.put(job(2, 24, sink().0));
    assert_eq!(waiting(&encoder), Some((2, 24)), "the newest wins");
    encoder.forget(1);
    assert_eq!(waiting(&encoder), Some((2, 24)), "another session's");
    let next = encoder.shared.wait_job().expect("a picture waits");
    assert_eq!((next.session, next.pixels.width), (2, 24));
    assert_eq!(waiting(&encoder), None, "taken");
    encoder.put(job(3, 16, sink().0));
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
    encoder.put(job(4, 16, sink().0));
    encoder.stop();
    assert_eq!(waiting(&encoder), None);
    assert!(encoder.shared.wait_job().is_none());
}

#[test]
fn the_encoders_thread_sends_each_jpeg_to_its_sink_and_ends_at_the_stop() {
    let mut encoder = Encoder::spawn().unwrap();
    let (sink, frames) = sink();
    encoder.capture(5);
    encoder.capture(6);
    encoder.put(job(5, 16, sink.clone()));
    wait_for("a frame", || frames.lock().unwrap().len() == 1);
    let got = frames.lock().unwrap().clone();
    let [(session, size, shown)] = got.as_slice() else {
        panic!("one frame: {got:?}")
    };
    let (session, size) = (*session, *size);
    assert_eq!(*shown, 0, "the picture's input, as put");
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
    encoder.put(Job {
        session: 6,
        shown: 0,
        crop: None,
        size: (16, 8),
        pixels: broken,
        sink,
    });
    wait_for("its report", || {
        encoder.shared.lock().done.iter().any(|e| e.session == 6)
    });
    assert_eq!(encoder.done()[0].bytes, None);
    assert_eq!(frames.lock().unwrap().len(), 1);
    encoder.stop();
    assert!(encoder.thread.is_none(), "joined");
}

#[test]
fn a_dropped_encoder_stops_its_thread() {
    let encoder = Encoder::spawn().unwrap();
    let shared = Arc::clone(&encoder.shared);
    drop(encoder);
    assert!(shared.lock().stopped, "the drop stopped the encoder");
    // The thread ended: only this test's handle is left on the hand-off.
    assert_eq!(Arc::strong_count(&shared), 1);
}

#[test]
fn a_frame_encoded_while_its_session_was_forgotten_never_reaches_its_sink() {
    // The guard, a release or a lost window ran while the encoder made the
    // frame (it had taken the picture already): no frame, nothing counted.
    let encoder = Encoder {
        shared: Arc::new(Shared::default()),
        thread: None,
    };
    let (sink, frames) = sink();
    encoder.capture(3);
    encoder.put(job(3, 16, sink.clone()));
    let taken = encoder.shared.wait_job().expect("a picture waits");
    encoder.forget(3);
    deliver(&encoder.shared, taken);
    assert!(frames.lock().unwrap().is_empty());
    assert_eq!(encoder.done(), Vec::new());
    // A session still captured gets its frame, reported first.
    encoder.capture(4);
    encoder.put(job(4, 16, sink.clone()));
    let taken = encoder.shared.wait_job().expect("a picture waits");
    deliver(&encoder.shared, taken);
    assert_eq!(frames.lock().unwrap().len(), 1);
    assert_eq!(encoder.done().len(), 1);
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
fn a_take_answers_once_its_window_is_on_top() {
    // The take's z-order change is posted: Live's thread, busy around an
    // editor's open, lands it later. Until then another editor may cover
    // the window, so a grab would show its pixels and a touch there would be
    // refused: the take answers only once its window is on top.
    let (mut worker, handle, _) = worker();
    handle.topmost_late(true);
    let (reply, mut answer) = oneshot::channel();
    let take = Command::Take {
        session: 1,
        before: Vec::new(),
        reply,
    };
    worker.command_at(take, 0.0);
    worker.step_at(0.0);
    let window = handle.windows()[0];
    assert_eq!(
        handle.records(),
        vec![json!({"op": "take", "window": window.0})],
        "taken, its change posted"
    );
    assert!(answer.try_recv().is_err(), "not on top yet");
    assert!(worker.editors.is_empty(), "nothing grabbed or touched yet");
    worker.step_at(2999.0);
    assert!(answer.try_recv().is_err());
    // Landed: the take answers at the next look, even at the wait's end.
    handle.topmost_late(false);
    worker.step_at(3000.0);
    assert_eq!(answer.try_recv().unwrap(), Ok((1349, 809)));
    assert_eq!(worker.editors[&1].taken.window, window);
    // One that never comes on top is handed back once the wait (counted
    // from its take) is over, and its take fails.
    handle.topmost_late(true);
    let (reply, mut answer) = oneshot::channel();
    let take = Command::Take {
        session: 2,
        before: handle.windows(),
        reply,
    };
    worker.command_at(take, 5000.0);
    worker.step_at(5000.0);
    let second = *handle.windows().last().unwrap();
    worker.step_at(7999.0);
    assert!(answer.try_recv().is_err(), "still waiting");
    worker.step_at(8000.0);
    assert_eq!(
        answer.try_recv().unwrap(),
        Err(reason::NOT_ON_TOP.to_string())
    );
    assert_eq!(
        handle.records().last(),
        Some(&json!({"op": "release", "window": second.0}))
    );
    assert_eq!(handle.topmost(second), Some(false), "its z-order as it was");
    assert_eq!(worker.editors.keys().copied().collect::<Vec<u32>>(), [1]);
    // The stop hands back a window still waiting for its place on top.
    let (reply, _answer) = oneshot::channel();
    let take = Command::Take {
        session: 3,
        before: handle.windows(),
        reply,
    };
    worker.command_at(take, 9000.0);
    worker.step_at(9000.0);
    let third = *handle.windows().last().unwrap();
    assert!(!worker.editors.contains_key(&3));
    worker.shutdown();
    let released: Vec<u64> = handle
        .records()
        .iter()
        .filter(|r| r["op"] == "release")
        .map(|r| r["window"].as_u64().unwrap())
        .collect();
    assert_eq!(released, [second.0, third.0, window.0]);
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
fn an_editor_let_go_reports_the_counts_of_its_last_minute() {
    // The PR I review: an editor released before its minute was over left
    // no `rate` of that minute at all.
    let (mut worker, handle, heard) = worker();
    take(&mut worker, &handle, 1, 0.0);
    let (sink, frames) = sink();
    worker.command_at(Command::Capture { session: 1, sink }, 10.0);
    worker.step_at(10.0);
    wait_for("its frame", || frames.lock().unwrap().len() == 1);
    // The worker counts what the encoder made at its next step.
    worker.step_at(20.0);
    let (reply, _released) = oneshot::channel();
    worker.command_at(
        Command::Release {
            session: 1,
            closed: false,
            reply,
        },
        30_000.0,
    );
    let events = heard.lock().unwrap().clone();
    let [PlugwinEvent::Rate { session: 1, rate }] = events.as_slice() else {
        panic!("its last rate: {events:?}")
    };
    assert_eq!((rate.grabs, rate.sent, rate.failed), (1, 1, 0));
    // One that grabbed nothing since its last minute reports nothing.
    let (mut quiet, quiet_handle, quiet_heard) = self::worker();
    take(&mut quiet, &quiet_handle, 1, 0.0);
    let (reply, _released) = oneshot::channel();
    quiet.command_at(
        Command::Release {
            session: 1,
            closed: false,
            reply,
        },
        30_000.0,
    );
    assert!(quiet_heard.lock().unwrap().is_empty());
}

#[test]
fn a_picture_shows_the_last_input_handled_a_paint_before_its_grab() {
    // PR J: Pro-Q gets PAINT_MS (40 ms) to paint an input.
    assert!(!painted(100.0, 140.0_f64.next_down()));
    assert!(painted(100.0, 140.0));
    let mut shown = Shown::default();
    assert_eq!(shown.at_grab(0.0), 0, "none yet");
    shown.handled(1, 100.0);
    shown.handled(2, 110.0);
    assert_eq!(shown.at_grab(139.0), 0);
    assert_eq!(shown.at_grab(140.0), 1);
    assert_eq!(shown.at_grab(149.9), 1);
    assert_eq!(shown.at_grab(150.0), 2);
    assert_eq!(shown.at_grab(500.0), 2, "it stays");
    // Past SHOWN_KEPT waiting, the newest waiting gives way: nothing counts
    // as painted before its time (the PR J review).
    assert_eq!(SHOWN_KEPT, 64);
    let mut many = Shown::default();
    for n in 1..=64 {
        many.handled(n, 1000.0 + f64::from(n));
    }
    many.handled(65, 1100.0);
    assert_eq!(many.at_grab(1040.0), 0, "none painted yet");
    assert_eq!(many.at_grab(1041.0), 1);
    assert_eq!(many.at_grab(1104.0), 63, "64 gave way to 65");
    assert_eq!(many.at_grab(1139.0), 63);
    assert_eq!(many.at_grab(1140.0), 65);
}

#[test]
fn an_equal_picture_goes_again_once_it_shows_a_newer_input() {
    // PR J: a tap on an inert spot changes no pixel, but the page must
    // learn that its picture caught up.
    let (mut worker, handle, _) = worker();
    handle.still(true);
    take(&mut worker, &handle, 1, 0.0);
    let (sink, frames) = sink();
    worker.command_at(Command::Capture { session: 1, sink }, 0.0);
    worker.step_at(0.0);
    wait_for("the first frame", || frames.lock().unwrap().len() == 1);
    worker.command_at(Command::Handled { session: 1, n: 1 }, 50.0);
    // Grabbed 10 ms after: not painted yet, the same picture is skipped.
    worker.step_at(60.0);
    assert_eq!(worker.editors[&1].counts.grabs, 2);
    assert_eq!(worker.editors[&1].last_shown, 0);
    // Grabbed 50 ms after: it shows input 1 and goes again.
    worker.step_at(100.0);
    assert_eq!(worker.editors[&1].last_shown, 1);
    wait_for("the picture again", || frames.lock().unwrap().len() == 2);
    assert_eq!(frames.lock().unwrap()[1].2, 1);
    // An input of another session is nothing.
    worker.command_at(Command::Handled { session: 9, n: 5 }, 110.0);
    worker.step_at(150.0);
    assert_eq!(worker.editors[&1].last_shown, 1);
    // The same picture showing the same input goes again a second after
    // the last one went (the PR J re-review: a frame the page could not
    // decode must not leave it behind for good).
    worker.step_at(1099.0);
    assert_eq!(worker.editors[&1].put, 100.0, "not yet");
    worker.step_at(1140.0);
    assert_eq!(worker.editors[&1].put, 1140.0);
    wait_for("the same picture again", || {
        frames.lock().unwrap().len() == 3
    });
}

#[test]
fn a_picture_whose_encode_failed_is_no_last_picture() {
    // PR J review: an equal picture showing a newer input was never sent
    // again when its encode failed.
    let (mut worker, handle, _) = worker();
    handle.still(true);
    take(&mut worker, &handle, 1, 0.0);
    let (sink, frames) = sink();
    worker.command_at(Command::Capture { session: 1, sink }, 0.0);
    worker.step_at(0.0);
    wait_for("the first frame", || frames.lock().unwrap().len() == 1);
    worker.step_at(1.0);
    assert!(worker.editors[&1].last.is_some(), "a frame went");
    // The encoder reports a failure of that session: its picture is
    // forgotten, so the next grab of the same pixels goes again.
    worker.encoder.shared.lock().done.push(Encoded {
        session: 1,
        ms: 3.0,
        bytes: None,
    });
    worker.step_at(2.0);
    assert!(worker.editors[&1].last.is_none());
    assert_eq!(worker.editors[&1].counts.failed, 1);
    worker.step_at(40.0);
    wait_for("the same picture again", || {
        frames.lock().unwrap().len() == 2
    });
}

#[test]
fn a_crop_is_the_part_within_the_picture_and_none_for_the_whole() {
    // PR L.
    let crop = |x, y, w, h| Crop { x, y, w, h };
    assert_eq!(
        crop_within(crop(10, 20, 30, 40), (100, 80)),
        Some(crop(10, 20, 30, 40))
    );
    // Clamped to the picture.
    assert_eq!(
        crop_within(crop(90, 60, 30, 40), (100, 80)),
        Some(crop(90, 60, 10, 20))
    );
    assert_eq!(
        crop_within(crop(0, 0, 100, 79), (100, 80)),
        Some(crop(0, 0, 100, 79))
    );
    // The whole picture, or nothing: none.
    assert_eq!(crop_within(crop(0, 0, 100, 80), (100, 80)), None);
    assert_eq!(crop_within(crop(0, 0, 500, 500), (100, 80)), None);
    assert_eq!(crop_within(crop(100, 10, 30, 40), (100, 80)), None);
    assert_eq!(crop_within(crop(10, 80, 30, 40), (100, 80)), None);
    assert_eq!(crop_within(crop(10, 10, 0, 40), (100, 80)), None);
    assert_eq!(crop_within(crop(10, 10, 30, 0), (100, 80)), None);
    assert_eq!(crop_within(crop(0, 0, 4, 4), (0, 0)), None);
}

#[test]
fn a_crop_takes_its_rows_and_columns_of_the_picture() {
    // A 4 × 3 picture whose pixel (x, y) is [x, y, 7, 255].
    let bgra = (0..3_u8)
        .flat_map(|y| (0..4_u8).flat_map(move |x| [x, y, 7, 255]))
        .collect();
    let picture = Pixels {
        width: 4,
        height: 3,
        bgra,
    };
    let part = cropped(
        &picture,
        Crop {
            x: 1,
            y: 1,
            w: 2,
            h: 2,
        },
    );
    assert_eq!((part.width, part.height), (2, 2));
    assert_eq!(
        part.bgra,
        vec![1, 1, 7, 255, 2, 1, 7, 255, 1, 2, 7, 255, 2, 2, 7, 255]
    );
}

#[test]
fn a_frame_says_where_its_part_lies_and_whether_it_is_whole() {
    let part = Frame {
        session: 4,
        shown: 9,
        crop: Some(Crop {
            x: 320,
            y: 160,
            w: 640,
            h: 400,
        }),
        size: (1349, 809),
        jpeg: Bytes::from_static(b"jpeg"),
    };
    assert_eq!(
        part.head(),
        FrameHead {
            session: 4,
            seq: 0,
            shown: 9,
            x: 320,
            y: 160,
            width: 1349,
            height: 809,
        }
    );
    assert!(!part.whole());
    let whole = Frame {
        session: 4,
        shown: 9,
        crop: None,
        size: (1349, 809),
        jpeg: Bytes::from_static(b"jpeg"),
    };
    assert_eq!((whole.head().x, whole.head().y), (0, 0));
    assert!(whole.whole());
}

/// A view of the part `crop` of the sim's picture (1349 × 809).
fn part(crop: Crop) -> Option<Part> {
    Some(Part {
        crop,
        picture: (1349, 809),
    })
}

#[test]
fn a_view_crops_the_frames_until_the_whole_picture_again() {
    // PR L: the page shows a part at 2× and more; its frames carry it.
    let (mut worker, handle, _) = worker();
    take(&mut worker, &handle, 1, 0.0);
    let (sink, frames) = sink();
    worker.command_at(Command::Capture { session: 1, sink }, 0.0);
    worker.step_at(0.0);
    wait_for("the whole picture", || frames.lock().unwrap().len() == 1);
    let crop = Crop {
        x: 100,
        y: 50,
        w: 300,
        h: 200,
    };
    worker.command_at(
        Command::View {
            session: 1,
            part: part(crop),
        },
        10.0,
    );
    assert_eq!(worker.editors[&1].part, part(crop));
    std::thread::sleep(Duration::from_millis(260));
    worker.step_at(40.0);
    assert_eq!(
        worker.editors[&1]
            .last
            .as_ref()
            .map(|p| (p.width, p.height)),
        Some((300, 200)),
        "the part only"
    );
    wait_for("the part", || frames.lock().unwrap().len() == 2);
    // A view of another session is nothing; none is the whole picture.
    worker.command_at(
        Command::View {
            session: 9,
            part: None,
        },
        50.0,
    );
    assert_eq!(worker.editors[&1].part, part(crop));
    worker.command_at(
        Command::View {
            session: 1,
            part: None,
        },
        60.0,
    );
    std::thread::sleep(Duration::from_millis(260));
    worker.step_at(80.0);
    assert_eq!(
        worker.editors[&1]
            .last
            .as_ref()
            .map(|p| (p.width, p.height)),
        Some((1349, 809))
    );
}

#[test]
fn a_part_cuts_only_a_picture_of_its_size() {
    // The PR L re-review: a part is of the picture's size the page drew.
    // One of another size goes whole until the page's part for it lands,
    // and one back at that size is cut again (the page may never have seen
    // the size between, and still thinks its part goes).
    let (mut worker, handle, _) = worker();
    let window = take(&mut worker, &handle, 1, 0.0);
    let (sink, frames) = sink();
    worker.command_at(Command::Capture { session: 1, sink }, 0.0);
    worker.step_at(0.0);
    wait_for("the whole picture", || frames.lock().unwrap().len() == 1);
    let crop = Crop {
        x: 100,
        y: 50,
        w: 300,
        h: 200,
    };
    // A part of another picture's size: the whole picture.
    worker.command_at(
        Command::View {
            session: 1,
            part: Some(Part {
                crop,
                picture: (1000, 700),
            }),
        },
        10.0,
    );
    worker.step_at(40.0);
    assert_eq!(worker.editors[&1].last_crop, None, "not of this size");
    worker.command_at(
        Command::View {
            session: 1,
            part: part(crop),
        },
        50.0,
    );
    worker.step_at(80.0);
    assert_eq!(worker.editors[&1].last_crop, Some(crop));
    // The window takes another size on the PC: the next frame is whole.
    handle.resize_window(window, (1000, 700));
    worker.step_at(120.0);
    assert_eq!(worker.editors[&1].last_crop, None);
    // Back at the part's size: cut again, with no new view.
    handle.resize_window(window, (1349, 809));
    worker.step_at(160.0);
    assert_eq!(worker.editors[&1].last_crop, Some(crop));
    assert_eq!(worker.editors[&1].part, part(crop));
}

#[test]
fn a_new_part_of_equal_pixels_still_goes() {
    // PR L review: a pan over a flat region gave equal pixels, and the page
    // never learnt the part's new corner.
    let (mut worker, handle, _) = worker();
    handle.still(true);
    take(&mut worker, &handle, 1, 0.0);
    let (sink, frames) = sink();
    worker.command_at(Command::Capture { session: 1, sink }, 0.0);
    worker.step_at(0.0);
    wait_for("the whole picture", || frames.lock().unwrap().len() == 1);
    // The sim's bar: a bright column; two parts of the dark field beside it.
    let whole = worker.editors[&1].last.clone().expect("a picture");
    let bar = (0..whole.width as usize)
        .find(|x| whole.bgra[(20 * whole.width as usize + x) * 4] == 0xF0)
        .expect("the bar") as u32;
    let x = if bar > 400 { 100 } else { 600 };
    let square = |x: u32| Crop {
        x,
        y: 100,
        w: 64,
        h: 64,
    };
    worker.command_at(
        Command::View {
            session: 1,
            part: part(square(x)),
        },
        10.0,
    );
    worker.step_at(40.0);
    wait_for("the first part", || frames.lock().unwrap().len() == 2);
    worker.command_at(
        Command::View {
            session: 1,
            part: part(square(x + 64)),
        },
        50.0,
    );
    worker.step_at(80.0);
    assert_eq!(worker.editors[&1].last_crop, Some(square(x + 64)));
    wait_for("the second part, of equal pixels", || {
        frames.lock().unwrap().len() == 3
    });
}

#[test]
fn a_cropped_editors_guard_sends_its_whole_picture_first() {
    // PR L review: the card's last picture is a whole one, and the edits
    // at 2× and more were only ever in parts.
    let (mut worker, handle, _) = worker();
    take(&mut worker, &handle, 1, 0.0);
    let got = Arc::new(Mutex::new(Vec::<Frame>::new()));
    let into = Arc::clone(&got);
    // The PR L re-review: when the guard's own frame goes (on the worker's
    // thread; the encoder's calls its sink holding the hand-off), no
    // picture of its session can reach the sink any more, or an older one
    // would land over the card's.
    let shared = Arc::clone(&worker.encoder.shared);
    let captured = Arc::new(Mutex::new(Vec::<bool>::new()));
    let at_guard = Arc::clone(&captured);
    let sink: FrameSink = Arc::new(move |frame: Frame| {
        if std::thread::current().name() != Some("plugwin-jpeg") {
            let still = shared.lock().captured.contains(&frame.session);
            at_guard.lock().unwrap().push(still);
        }
        into.lock().unwrap().push(frame);
    });
    worker.command_at(Command::Capture { session: 1, sink }, 0.0);
    worker.command_at(
        Command::View {
            session: 1,
            part: part(Crop {
                x: 100,
                y: 50,
                w: 300,
                h: 200,
            }),
        },
        0.0,
    );
    worker.step_at(0.0);
    wait_for("a part", || got.lock().unwrap().len() == 1);
    assert!(!got.lock().unwrap()[0].whole());
    // The page went back to the whole picture, with no grab since: the
    // last frame was still a part, so the card's picture is old.
    worker.command_at(
        Command::View {
            session: 1,
            part: None,
        },
        5.0,
    );
    let _answer = guard_at(&mut worker, 1, 10.0);
    let frames = got.lock().unwrap().clone();
    assert_eq!(frames.len(), 2, "its whole picture at the guard");
    assert!(frames[1].whole());
    assert_eq!(frames[1].size, (1349, 809));
    assert_eq!(*captured.lock().unwrap(), [false], "forgotten first");
    // Counted as a grab and a frame sent (its last rate holds it).
    assert_eq!(
        (
            worker.editors[&1].counts.grabs,
            worker.editors[&1].counts.sent
        ),
        (2, 1),
        "the encoder's frame is counted at a step"
    );
    // An editor that was whole sends nothing more at its guard.
    let (mut worker, handle, _) = self::worker();
    take(&mut worker, &handle, 1, 0.0);
    let got = Arc::new(Mutex::new(Vec::<Frame>::new()));
    let into = Arc::clone(&got);
    let sink: FrameSink = Arc::new(move |frame: Frame| into.lock().unwrap().push(frame));
    worker.command_at(Command::Capture { session: 1, sink }, 0.0);
    worker.step_at(0.0);
    wait_for("a whole frame", || got.lock().unwrap().len() == 1);
    let _answer = guard_at(&mut worker, 1, 10.0);
    assert_eq!(got.lock().unwrap().len(), 1);
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
    let first = worker.editors[&1].last.clone().expect("its first picture");
    worker.step_at(60.0);
    // The hand-off is synchronous: the grab happened, and nothing was
    // handed to the encoder (a frame it encoded would come later than any
    // look at the sink).
    assert_eq!(worker.editors[&1].counts.grabs, 2);
    assert!(
        worker.encoder.shared.lock().waiting.is_none(),
        "the same picture: skipped"
    );
    let kept = worker.editors[&1].last.as_ref().expect("a picture");
    assert!(Arc::ptr_eq(&first, kept), "the first picture still kept");
    assert_eq!(frames.lock().unwrap().len(), 1);
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
    // The window goes away: lost, after the counts of its last minute
    // (the grab at 60 000 ms, after the minute's rate).
    let window = handle.windows()[0];
    handle.remove_window(window);
    worker.step_at(60_100.0);
    let events = heard.lock().unwrap().clone();
    let [
        _,
        PlugwinEvent::Rate { session: 1, rate },
        PlugwinEvent::Lost { session: 1 },
    ] = events.as_slice()
    else {
        panic!("its last rate, then lost: {events:?}")
    };
    assert_eq!(rate.grabs, 1);
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
    // Another session's down while this contact is down: dropped, and the
    // router hears that contact ended (`busy`), so its state never holds a
    // contact the worker never took.
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
    let busy = PlugwinEvent::ContactEnded {
        session: 2,
        contact: 2,
        why: "busy".to_string(),
    };
    assert_eq!(
        heard.lock().unwrap().as_slice(),
        [busy, ended(5), ended(6)],
        "a dropped phase of an ended contact says nothing"
    );
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
fn a_down_waits_out_its_editors_resize_and_the_rest_go() {
    let all = [Phase::Down, Phase::Update, Phase::Up, Phase::Cancel];
    assert_eq!(all.map(|p| waits_out(p, true)), [true, false, false, false]);
    assert_eq!(all.map(|p| waits_out(p, false)), [false; 4]);
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
    assert_eq!(worker.contact, None, "ended at once");
    // 100 ms after its up, its placement, a pixel aside, then the known
    // rectangle, each seen to land at a step: then the tap.
    worker.step_at(101.0);
    worker.step_at(111.0);
    assert!(answer.try_recv().is_err(), "read back at the next look");
    worker.step_at(121.0);
    assert_eq!(answer.try_recv().unwrap(), Ok(()));
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
    worker.step_at(200.0);
    // The hand-off is synchronous: no sink, no grab since the guard,
    // nothing handed to the encoder.
    assert!(worker.editors[&1].sink.is_none());
    assert_eq!(
        worker.editors[&1].counts.grabs, 1,
        "no grab after the guard"
    );
    assert!(worker.encoder.shared.lock().waiting.is_none());
    assert_eq!(frames.lock().unwrap().len(), 1, "no frame after the guard");
    // A guard of no editor, and one whose tap is refused, fail.
    let (reply, mut answer) = oneshot::channel();
    worker.command_at(Command::Guard { session: 7, reply }, 200.0);
    assert_eq!(answer.try_recv().unwrap(), Err(NO_EDITOR.to_string()));
    handle.refuse(true);
    let (reply, mut answer) = oneshot::channel();
    worker.command_at(Command::Guard { session: 1, reply }, 300.0);
    worker.step_at(310.0);
    worker.step_at(320.0);
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
    assert_eq!(worker.contact, None, "the other engineer's contact ended");
    worker.step_at(10.0);
    worker.step_at(20.0);
    assert_eq!(answer.try_recv().unwrap(), Ok(()));
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
    fn on_top(&mut self, taken: &Taken) -> bool {
        self.sim.on_top(taken)
    }
    fn alive(&mut self, taken: &Taken) -> bool {
        self.sim.alive(taken)
    }
    fn work(&mut self, taken: &Taken) -> Result<Rect, String> {
        self.sim.work(taken)
    }
    fn rect(&mut self, taken: &Taken) -> Result<Rect, String> {
        self.sim.rect(taken)
    }
    fn resize(&mut self, taken: &Taken, rect: Rect) -> Result<(), String> {
        self.sim.resize(taken, rect)
    }
    fn client(&mut self, taken: &Taken) -> Result<(u32, u32), String> {
        self.sim.client(taken)
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
    let started = Instant::now();
    plugwin.stop();
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
    let started = Instant::now();
    plugwin.stop();
    assert!(!plugwin.stopped().await);
    let waited = started.elapsed();
    assert!(
        (STOP_WAIT..Duration::from_secs(3)).contains(&waited),
        "{waited:?}"
    );
}

#[tokio::test]
async fn a_second_look_at_the_stop_waits_for_the_first_ones_answer() {
    let (plugwin, handle) = slow_worker(Duration::from_millis(300)).await;
    plugwin.stop();
    let second = async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let ended = plugwin.stopped().await;
        // Its answer comes only once the worker handed its window back.
        let released = handle.records().iter().any(|r| r["op"] == "release");
        (ended, released)
    };
    let (first, (second, released)) = tokio::join!(plugwin.stopped(), second);
    assert!(first, "the first look: ended within the wait");
    assert!(second);
    assert!(released, "the second look answered before the worker ended");
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
    // Two editors captured (the worker takes several; the hub holds one at
    // a time since the round-3 ruling, so two is the worst case), every
    // grab taking 25 ms (the PC's BitBlt of a 4.3 MB picture): the
    // keep-alive must not wait for the step's grabs, or Windows cancels the
    // resting contact (no frame for 100 ms). Waiting for both grabs makes
    // gaps of 50 + 2 × 25 ms and more on nearly every keep-alive; checked
    // between them, the worst is about 75 ms.
    let (sim, handle) = Sim::new(None);
    handle.still(true);
    handle.grab_delay(Duration::from_millis(25));
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
    // Wall-clock times on a thread the runner shares with every other test
    // (the coverage build, the mutation shards): it may be held up once,
    // as the integration test allows (`tests/eq.rs`). The strict 100 ms
    // bound is the explicit-clock test's
    // (`the_keep_alive_is_looked_at_first_and_after_each_grab_on_a_fresh_clock`).
    let gaps: Vec<f64> = times.windows(2).map(|pair| pair[1] - pair[0]).collect();
    let late = gaps.iter().filter(|gap| **gap >= 100.0).count();
    let largest = gaps.iter().copied().fold(0.0, f64::max);
    assert!(
        late <= 1 && largest < 150.0,
        "{late} gaps of 100 ms or more, the largest {largest} ms: {gaps:?}"
    );
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
    // Its editor's last part of a minute, at its release (the PR I review),
    // and nothing else.
    let heard = heard.lock().unwrap().clone();
    assert!(
        matches!(heard.as_slice(), [PlugwinEvent::Rate { session: 1, .. }]),
        "{heard:?}"
    );
}

/// A page's picture area (PR G).
fn area(w: f64, h: f64) -> Area {
    Area { w, h }
}

/// An upright phone's area: the picture 667 × 1361 in the sim's room.
const UPRIGHT: Area = Area { w: 392.0, h: 800.0 };

/// The sim's resize records: the size asked.
fn resizes(handle: &SimHandle) -> Vec<(i64, i64)> {
    handle
        .records()
        .iter()
        .filter(|r| r["op"] == "resize")
        .map(|r| (r["w"].as_i64().unwrap(), r["h"].as_i64().unwrap()))
        .collect()
}

/// A resize of `session` for `area` at `now`: its answer.
fn resize_at(
    worker: &mut Worker,
    session: u32,
    area: Area,
    now: f64,
) -> oneshot::Receiver<Option<Resized>> {
    let (reply, answer) = oneshot::channel();
    worker.command_at(
        Command::Resize {
            session,
            area,
            reply,
        },
        now,
    );
    answer
}

/// The guard of `session` at `now`: its answer.
fn guard_at(worker: &mut Worker, session: u32, now: f64) -> oneshot::Receiver<Result<(), String>> {
    let (reply, answer) = oneshot::channel();
    worker.command_at(Command::Guard { session, reply }, now);
    answer
}

/// The taken window as the sim takes it: Live's frame around its picture.
fn taken_at(width: u32, height: u32, rect: Rect) -> Taken {
    Taken {
        window: WindowId(1),
        picture: WindowId(2),
        pid: 3,
        was_topmost: false,
        width,
        height,
        rect,
    }
}

#[test]
fn a_size_asked_adds_the_frame_and_the_room_is_the_work_area_less_it() {
    // Live's window: 1365 × 848 around a 1349 × 809 picture.
    let rect = Rect {
        left: 600,
        top: 296,
        width: 1365,
        height: 848,
    };
    let taken = taken_at(1349, 809, rect);
    assert_eq!(window_size(&taken, (760, 1271)), (776, 1310));
    assert_eq!(window_size(&taken, (1349, 809)), (1365, 848));
    // The work area less the frame, wherever the window stands (a resize
    // moves it in when it does not fit there).
    let work = Rect {
        left: 0,
        top: 0,
        width: 2560,
        height: 1400,
    };
    assert_eq!(room(work, &taken), (2544, 1361));
    let right = Rect {
        left: 2560,
        top: 10,
        width: 1920,
        height: 1040,
    };
    assert_eq!(room(right, &taken), (1904, 1001));
    // A frame larger than the area: no room.
    let tiny = Rect {
        left: 0,
        top: 0,
        width: 10,
        height: 30,
    };
    assert_eq!(room(tiny, &taken), (0, 0));
}

/// A rectangle.
fn rect(left: i32, top: i32, width: i32, height: i32) -> Rect {
    Rect {
        left,
        top,
        width,
        height,
    }
}

#[test]
fn a_window_stays_where_it_fits_and_moves_into_the_work_area_where_not() {
    // #74 review: a window low on the screen grows past the taskbar no more.
    let work = rect(0, 0, 2560, 1400);
    let low = rect(600, 600, 1365, 848);
    // Where it stands when it fits there.
    assert_eq!(inside(work, low, (1365, 800)), rect(600, 600, 1365, 800));
    assert_eq!(inside(work, low, (1960, 800)), rect(600, 600, 1960, 800));
    // An upright picture's window is too high there: up just enough.
    assert_eq!(inside(work, low, (683, 1400)), rect(600, 0, 683, 1400));
    assert_eq!(inside(work, low, (683, 1000)), rect(600, 400, 683, 1000));
    // Too wide there: left just enough.
    assert_eq!(inside(work, low, (2000, 800)), rect(560, 600, 2000, 800));
    // Past the near edges (a window partly off the screen): in.
    let off = rect(-50, -20, 1365, 848);
    assert_eq!(inside(work, off, (1365, 848)), rect(0, 0, 1365, 848));
    // Larger than the work area: at its near edges.
    assert_eq!(inside(work, low, (3000, 1500)), rect(0, 0, 3000, 1500));
    // On a second screen to the right, its own edges.
    let right = rect(2560, 10, 1920, 1040);
    let there = rect(3000, 500, 1365, 848);
    assert_eq!(inside(right, there, (683, 1000)), rect(3000, 50, 683, 1000));
    assert_eq!(
        inside(right, there, (1800, 600)),
        rect(2680, 450, 1800, 600)
    );
    // Exactly at the far edges still fits.
    assert_eq!(
        inside(work, rect(1195, 552, 1, 1), (1365, 848)),
        rect(1195, 552, 1365, 848)
    );
}

#[test]
fn a_resize_lands_within_2_px_or_settles_unlanded_after_1_s() {
    assert_eq!((RESIZE_MS, RESIZE_SLACK), (1000.0, 2));
    assert_eq!(MIN_PICTURE, (600, 400));
    assert!(lands((667, 1361), (667, 1361)));
    assert!(lands((669, 1359), (667, 1361)));
    assert!(lands((665, 1363), (667, 1361)));
    assert!(!lands((670, 1361), (667, 1361)));
    assert!(!lands((667, 1358), (667, 1361)));
    assert!(!lands((664, 1364), (667, 1361)));
    assert!(!resize_over(1000.0_f64.next_down()));
    assert!(resize_over(1000.0));
    const FROM: (u32, u32) = (1349, 809);
    assert_eq!(settled((667, 1361), (667, 1361), FROM, 0.0), Some(true));
    assert_eq!(settled((667, 1361), (667, 1361), FROM, 5000.0), Some(true));
    assert_eq!(settled((1349, 809), (667, 1361), FROM, 999.0), None);
    assert_eq!(settled((1349, 809), (667, 1361), FROM, 1000.0), Some(false));
    // Over its wait, a picture that left its size landed at the size the
    // editor took (the PC's Live: 760 x 1361 asked, 760 x 1275 came).
    assert_eq!(settled((760, 1275), (760, 1361), FROM, 999.0), None);
    assert_eq!(settled((760, 1275), (760, 1361), FROM, 1000.0), Some(true));
    // Within the slack of where it was: it never left.
    assert_eq!(settled((1351, 807), (760, 1361), FROM, 1000.0), Some(false));
    assert_eq!(settled((1352, 809), (760, 1361), FROM, 1000.0), Some(true));
}

#[test]
fn the_guard_waits_for_a_settling_resize_then_places_its_editor_in_two_steps_before_its_tap() {
    use GuardStep::{Aside, Fail, Known, Tap, Wait};
    assert_eq!(KNOWN_SIZE, (1349, 809));
    assert_eq!(inert_spot(KNOWN_SIZE.0), (546, 15));
    let known = rect(0, 0, 1365, 848);
    let first = Placement::Aside(rect(1, 0, 1365, 848));
    let second = Placement::Known(known);
    let (there, at_size) = (Some(known), Some(KNOWN_SIZE));
    // A resize of the editor still settles: its way back goes first.
    assert_eq!(guard_step(true, None, there, at_size), Wait);
    assert_eq!(
        guard_step(true, Some((second, 5000.0)), there, at_size),
        Wait
    );
    // Nothing posted yet: the first placement, whatever the window reads
    // (at the known rectangle and size too: a rectangle posted before may
    // still wait).
    assert_eq!(guard_step(false, None, there, at_size), Aside);
    assert_eq!(guard_step(false, None, None, None), Aside);
    // The first seen to land, at any size: the second.
    let aside_read = Some(first.rect());
    assert_eq!(
        guard_step(false, Some((first, 10.0)), aside_read, Some((760, 1271))),
        Known
    );
    // Not seen (the known rectangle read proves nothing then): waited for,
    // and a failure once its wait is over.
    assert_eq!(
        guard_step(false, Some((first, 999.0)), there, at_size),
        Wait
    );
    assert_eq!(
        guard_step(false, Some((first, 1000.0)), there, at_size),
        Fail(first)
    );
    // The second seen with the picture at exactly the known size: the tap.
    assert_eq!(guard_step(false, Some((second, 10.0)), there, at_size), Tap);
    assert_eq!(
        guard_step(false, Some((second, 5000.0)), there, at_size),
        Tap
    );
    // There at another size, or the size elsewhere: not landed.
    let off_size = Some((1349, 808));
    assert_eq!(
        guard_step(false, Some((second, 999.0)), there, off_size),
        Wait
    );
    assert_eq!(
        guard_step(false, Some((second, 999.0)), aside_read, at_size),
        Wait
    );
    assert_eq!(
        guard_step(false, Some((second, 1000.0)), there, off_size),
        Fail(second)
    );
    assert_eq!(
        guard_step(false, Some((second, 1000.0)), None, None),
        Fail(second)
    );
}

#[test]
fn a_placement_lands_once_the_window_reads_it_the_known_one_at_the_known_size() {
    let work = rect(0, 0, 2560, 1400);
    let known = rect(900, 552, 1365, 848);
    // A pixel to the right, unless the window stands exactly there: then a
    // pixel to the left.
    let right = rect(901, 552, 1365, 848);
    assert_eq!(aside(known, rect(900, 700, 1365, 848), work), right);
    assert_eq!(aside(known, known, work), right);
    assert_eq!(aside(known, right, work), rect(899, 552, 1365, 848));
    assert_eq!(
        aside(known, rect(901, 552, 683, 1400), work),
        right,
        "there at another size"
    );
    // Inward (round 3): against the work area's right edge, a pixel to the
    // left, unless the window stands exactly there: then to the right.
    let edge = rect(1195, 552, 1365, 848);
    let left = rect(1194, 552, 1365, 848);
    assert_eq!(aside(edge, rect(2000, 1000, 776, 1310), work), left);
    assert_eq!(aside(edge, edge, work), left);
    assert_eq!(aside(edge, left, work), rect(1196, 552, 1365, 848));
    // A pixel short of that edge, the right still fits.
    assert_eq!(
        aside(rect(1194, 552, 1365, 848), known, work),
        rect(1195, 552, 1365, 848)
    );
    // A second screen's own right edge.
    let second = rect(2560, 10, 1920, 1040);
    assert_eq!(
        aside(rect(3115, 50, 1365, 848), known, second),
        rect(3114, 50, 1365, 848)
    );
    assert_eq!(
        aside(rect(3114, 50, 1365, 848), known, second),
        rect(3115, 50, 1365, 848)
    );
    let first = Placement::Aside(right);
    assert_eq!(first.rect(), right);
    assert!(first.landed(Some(right), Some((760, 1271))), "at any size");
    assert!(first.landed(Some(right), None));
    assert!(!first.landed(Some(known), Some(KNOWN_SIZE)));
    assert!(!first.landed(None, Some(KNOWN_SIZE)), "unread");
    let second = Placement::Known(known);
    assert_eq!(second.rect(), known);
    assert!(second.landed(Some(known), Some(KNOWN_SIZE)));
    assert!(!second.landed(Some(known), Some((1349, 808))));
    assert!(!second.landed(Some(known), Some((1348, 809))));
    assert!(!second.landed(Some(known), None));
    assert!(!second.landed(Some(right), Some(KNOWN_SIZE)));
    assert!(!second.landed(None, Some(KNOWN_SIZE)));
}

#[test]
fn a_take_at_another_size_than_the_known_one_is_told() {
    assert_eq!(other_size((1349, 809)), None);
    assert_eq!(other_size((760, 1271)), Some((760, 1271)));
    assert_eq!(other_size((1349, 808)), Some((1349, 808)));
    assert_eq!(other_size((1348, 809)), Some((1348, 809)));
}

#[test]
fn a_guard_that_cannot_tap_says_what_the_window_read_and_what_it_posted() {
    let first = Placement::Aside(rect(301, 50, 1365, 848));
    assert_eq!(
        not_landed(first, Some(rect(300, 50, 776, 1310)), Some((760, 1271))),
        format!(
            "{NOT_BACK}: its first placement did not land (the window is 776x1310 at 300,50, 1365x848 at 301,50 posted)"
        )
    );
    assert_eq!(
        not_landed(first, None, None),
        format!(
            "{NOT_BACK}: its first placement did not land (the window is unread, 1365x848 at 301,50 posted)"
        )
    );
    let second = Placement::Known(rect(300, 50, 1365, 848));
    assert_eq!(
        not_landed(second, Some(rect(301, 50, 1365, 848)), Some((1349, 808))),
        format!(
            "{NOT_BACK}: its placement did not land (the window is 1365x848 at 301,50 with a 1349x808 picture, 1365x848 at 300,50 posted for 1349x809)"
        )
    );
    assert_eq!(
        not_landed(second, None, None),
        format!(
            "{NOT_BACK}: its placement did not land (the window is unread with a 0x0 picture, 1365x848 at 300,50 posted for 1349x809)"
        )
    );
}

#[test]
fn the_known_size_goes_at_the_takes_place_moved_in_when_it_does_not_fit() {
    let work = rect(0, 0, 2560, 1400);
    // Taken at 760 × 1271 in the corner: the known size there.
    let corner = taken_at(760, 1271, rect(0, 0, 776, 1310));
    assert_eq!(known_rect(work, &corner), rect(0, 0, 1365, 848));
    // Taken low and right: moved in.
    let low = taken_at(760, 1271, rect(2000, 1000, 776, 1310));
    assert_eq!(known_rect(work, &low), rect(1195, 552, 1365, 848));
}

#[test]
fn a_grab_of_another_size_is_news_only_with_no_resize_on_its_way() {
    assert!(regrown(false, (800, 600), (1349, 809)));
    assert!(!regrown(false, (1349, 809), (1349, 809)));
    assert!(!regrown(true, (800, 600), (1349, 809)));
    assert!(!regrown(true, (1349, 809), (1349, 809)));
}

#[test]
fn a_resize_posts_the_areas_size_and_answers_once_it_lands() {
    let (mut worker, handle, heard) = worker();
    let window = take(&mut worker, &handle, 1, 0.0);
    let mut answer = resize_at(&mut worker, 1, UPRIGHT, 10.0);
    assert_eq!(resizes(&handle), [(667, 1361)], "posted at once");
    assert!(answer.try_recv().is_err(), "looked at in the step");
    assert!(worker.editors[&1].changed);
    worker.step_at(30.0);
    assert_eq!(
        answer.try_recv().unwrap(),
        Some(Resized {
            asked: (667, 1361),
            client: (667, 1361),
            ok: true,
            ms: 20.0,
            reverted: false,
        })
    );
    assert_eq!(worker.editors[&1].size, (667, 1361));
    assert_eq!(handle.size(window), Some((667, 1361)));
    // A side the minimum holds up: 30:1 is 2544 × 400.
    let mut answer = resize_at(&mut worker, 1, area(3000.0, 100.0), 40.0);
    worker.step_at(40.0);
    let wide = answer.try_recv().unwrap().expect("a resize");
    assert_eq!(
        (wide.asked, wide.client, wide.ok),
        ((2544, 400), (2544, 400), true)
    );
    // Nothing asked: no editor, an area with no shape.
    let mut answer = resize_at(&mut worker, 9, UPRIGHT, 50.0);
    assert_eq!(answer.try_recv().unwrap(), None);
    let mut answer = resize_at(&mut worker, 1, area(0.0, 800.0), 50.0);
    assert_eq!(answer.try_recv().unwrap(), None);
    assert_eq!(resizes(&handle).len(), 2);
    // Its frames follow the size, and no event says it again.
    let (sink, _frames) = sink();
    worker.command_at(Command::Capture { session: 1, sink }, 60.0);
    worker.step_at(60.0);
    assert_eq!(worker.editors[&1].counts.width, 2544);
    let heard = heard.lock().unwrap().clone();
    assert!(heard.is_empty(), "{heard:?}");
}

#[test]
fn a_resize_moves_a_window_into_the_work_area_and_its_release_puts_it_back() {
    // #74 review: a window low on the screen, upright, crossed the taskbar.
    let (mut worker, handle, _) = worker();
    handle.auto_open(false);
    let window = handle.add_window(0);
    handle.move_window(window, (900, 700));
    let (reply, mut answer) = oneshot::channel();
    let take = Command::Take {
        session: 1,
        before: Vec::new(),
        reply,
    };
    worker.command_at(take, 0.0);
    worker.step_at(0.0);
    assert_eq!(answer.try_recv().unwrap(), Ok((1349, 809)));
    let mut resized = resize_at(&mut worker, 1, UPRIGHT, 10.0);
    worker.step_at(20.0);
    let landed = resized.try_recv().unwrap().expect("a resize");
    assert_eq!((landed.client, landed.ok), ((667, 1361), true));
    // Upright it does not fit 700 px down: up to the top, its x kept.
    assert_eq!(handle.rect(window), Some(rect(900, 0, 683, 1400)));
    // Handed back while Live keeps it open: its own place and size again.
    let (reply, _released) = oneshot::channel();
    worker.command_at(
        Command::Release {
            session: 1,
            closed: false,
            reply,
        },
        30.0,
    );
    assert_eq!(handle.rect(window), Some(rect(900, 700, 1365, 848)));
}

#[test]
fn a_resize_waits_for_the_finger_on_its_editor_to_lift_the_newest_only() {
    // The review of PR #74 (M4): a page's new area resized the editor under
    // a resting or dragging finger (iPad Split View, a phone turned with a
    // finger held), and Pro-Q laid itself out under the injected contact.
    let (mut worker, handle, _) = worker();
    take(&mut worker, &handle, 1, 0.0);
    touch_at(&mut worker, 1, 1, Phase::Down, (300, 200), 0.0);
    let mut first = resize_at(&mut worker, 1, area(800.0, 320.0), 10.0);
    let mut newest = resize_at(&mut worker, 1, UPRIGHT, 20.0);
    worker.step_at(30.0);
    assert_eq!(
        resizes(&handle),
        Vec::<(i64, i64)>::new(),
        "nothing under the finger"
    );
    assert_eq!(first.try_recv().unwrap(), None, "the newest replaced it");
    assert!(newest.try_recv().is_err(), "waiting");
    touch_at(&mut worker, 1, 1, Phase::Up, (300, 200), 40.0);
    worker.step_at(50.0);
    assert_eq!(
        resizes(&handle),
        Vec::<(i64, i64)>::new(),
        "not within 100 ms of the up"
    );
    worker.step_at(140.0);
    assert_eq!(
        resizes(&handle),
        [(667, 1361)],
        "the newest, 100 ms after the up"
    );
    worker.step_at(150.0);
    let landed = newest.try_recv().unwrap().expect("a resize");
    assert_eq!((landed.client, landed.ok), ((667, 1361), true));
    // Another session's finger is no reason to wait.
    take(&mut worker, &handle, 2, 160.0);
    touch_at(&mut worker, 2, 2, Phase::Down, (10, 10), 160.0);
    resize_at(&mut worker, 1, area(800.0, 320.0), 170.0);
    assert_eq!(resizes(&handle), [(667, 1361), (1652, 661)], "at once");
}

#[test]
fn a_guard_drops_a_resize_that_waits_for_a_finger() {
    let (mut worker, handle, _) = worker();
    take(&mut worker, &handle, 1, 0.0);
    touch_at(&mut worker, 1, 1, Phase::Down, (300, 200), 0.0);
    let mut waiting = resize_at(&mut worker, 1, UPRIGHT, 10.0);
    let mut guarded = guard_at(&mut worker, 1, 20.0);
    assert_eq!(waiting.try_recv().unwrap(), None, "dropped");
    worker.step_at(30.0);
    assert_eq!(
        resizes(&handle),
        Vec::<(i64, i64)>::new(),
        "not within 100 ms of the guard's up"
    );
    for now in [120.0, 130.0, 140.0] {
        worker.step_at(now);
    }
    assert_eq!(guarded.try_recv().unwrap(), Ok(()), "at its own size");
    assert_eq!(
        resizes(&handle),
        [(1349, 809), (1349, 809)],
        "the guard's own placements only"
    );
}

#[test]
fn a_guard_right_after_a_contact_places_nothing_until_100_ms_after_it() {
    // The review of PR #74, round 3 (F1): a page gone mid-drag closes its
    // editor, and the guard posted its first placement microseconds after
    // the up it injects itself. Live's thread takes posted messages before
    // injected input, so the editor could move and Pro-Q lay itself out
    // before that up landed, at the old screen point.
    let (mut worker, handle, _) = worker();
    take(&mut worker, &handle, 1, 0.0);
    touch_at(&mut worker, 1, 1, Phase::Down, (300, 200), 0.0);
    touch_at(&mut worker, 1, 1, Phase::Update, (320, 210), 10.0);
    let mut guarded = guard_at(&mut worker, 1, 20.0);
    assert_eq!(
        touches(&handle),
        [
            t("down", 300, 200),
            t("update", 320, 210),
            t("up", 320, 210)
        ]
    );
    worker.step_at(119.9);
    assert_eq!(
        resizes(&handle),
        Vec::<(i64, i64)>::new(),
        "nothing within 100 ms of its up"
    );
    assert!(guarded.try_recv().is_err());
    worker.step_at(120.0);
    assert_eq!(resizes(&handle), [(1349, 809)], "its first placement");
    worker.step_at(130.0);
    assert_eq!(resizes(&handle), [(1349, 809), (1349, 809)]);
    assert!(guarded.try_recv().is_err(), "read back at the next look");
    worker.step_at(140.0);
    assert_eq!(guarded.try_recv().unwrap(), Ok(()));
    assert_eq!(
        touches(&handle)[3..],
        [t("down", 546, 15), t("up", 546, 15)]
    );
}

#[test]
fn no_down_reaches_an_editor_while_its_resize_is_on_its_way() {
    // The review of PR #74, round 2 (N3): Live lands a posted size before
    // an injected down, so a down while a resize was on its way hit the
    // new layout at the old picture's point (another button of the top
    // bar). Round 3 (F1): a resize that waited for a finger goes only
    // 100 ms after its up, so a double tap's second down now comes before
    // it is posted, and the resize waits for that contact too.
    let (mut worker, handle, heard) = worker();
    let window = take(&mut worker, &handle, 1, 0.0);
    handle.resize_late(true);
    touch_at(&mut worker, 1, 1, Phase::Down, (300, 200), 0.0);
    let mut resized = resize_at(&mut worker, 1, UPRIGHT, 10.0);
    touch_at(&mut worker, 1, 1, Phase::Up, (300, 200), 20.0);
    worker.step_at(20.0);
    touch_at(&mut worker, 1, 2, Phase::Down, (300, 200), 60.0);
    assert_eq!(held(&worker), Some((1, 2, (300, 200))), "the second tap");
    touch_at(&mut worker, 1, 2, Phase::Up, (300, 200), 100.0);
    worker.step_at(199.9);
    assert_eq!(
        resizes(&handle),
        Vec::<(i64, i64)>::new(),
        "not within 100 ms of the second up"
    );
    worker.step_at(200.0);
    assert_eq!(resizes(&handle), [(667, 1361)], "on its way");
    // A down while it is on its way: refused, and the router hears its
    // contact ended (`busy`); its up is dropped.
    touch_at(&mut worker, 1, 3, Phase::Down, (300, 200), 210.0);
    assert_eq!(worker.contact, None);
    assert_eq!(
        heard.lock().unwrap().last(),
        Some(&PlugwinEvent::ContactEnded {
            session: 1,
            contact: 3,
            why: BUSY.to_string()
        })
    );
    touch_at(&mut worker, 1, 3, Phase::Up, (300, 200), 250.0);
    assert_eq!(
        touches(&handle),
        [
            t("down", 300, 200),
            t("up", 300, 200),
            t("down", 300, 200),
            t("up", 300, 200)
        ]
    );
    // Landed: the next down goes, in the new layout.
    assert!(handle.land_one(window));
    worker.step_at(260.0);
    let landed = resized.try_recv().unwrap().expect("a resize");
    assert_eq!((landed.client, landed.ok), ((667, 1361), true));
    touch_at(&mut worker, 1, 4, Phase::Down, (300, 900), 270.0);
    assert_eq!(held(&worker), Some((1, 4, (300, 900))));
    touch_at(&mut worker, 1, 4, Phase::Up, (300, 900), 280.0);
    // Another editor's resize on its way refuses nothing here.
    take(&mut worker, &handle, 2, 290.0);
    resize_at(&mut worker, 2, UPRIGHT, 290.0);
    touch_at(&mut worker, 1, 5, Phase::Down, (10, 10), 300.0);
    assert_eq!(held(&worker), Some((1, 5, (10, 10))));
}

#[test]
fn a_resize_that_waited_for_a_finger_goes_100_ms_after_its_up() {
    // The review of PR #74, round 3 (F1): a resize that waited for a finger
    // was posted in the step right after its injected up. Live's thread
    // takes posted messages before injected input, so Pro-Q could lay
    // itself out at the new size before the up landed, which then ended a
    // band's drag at the old picture's point.
    let (mut worker, handle, _) = worker();
    take(&mut worker, &handle, 1, 0.0);
    touch_at(&mut worker, 1, 1, Phase::Down, (300, 200), 0.0);
    let mut resized = resize_at(&mut worker, 1, UPRIGHT, 10.0);
    touch_at(&mut worker, 1, 1, Phase::Up, (300, 200), 20.0);
    worker.step_at(20.0);
    worker.step_at(119.9);
    assert_eq!(
        resizes(&handle),
        Vec::<(i64, i64)>::new(),
        "none within 100 ms of the up"
    );
    assert!(resized.try_recv().is_err(), "waiting");
    worker.step_at(120.0);
    assert_eq!(resizes(&handle), [(667, 1361)], "100 ms after the up");
    let landed = resized.try_recv().unwrap().expect("a resize");
    assert_eq!((landed.client, landed.ok), ((667, 1361), true));
}

#[test]
fn every_end_of_a_contact_holds_its_editors_resizes_for_100_ms() {
    // The review of PR #74, round 3 (F1): each way a contact ends holds the
    // resizes of its editor for 100 ms, a resize asked in that time too
    // (Live's thread lands a posted rectangle before injected input).
    let (mut worker, handle, _) = worker();
    let first = take(&mut worker, &handle, 1, 0.0);
    take(&mut worker, &handle, 2, 0.0);
    let posted = |handle: &SimHandle| -> Vec<(i64, i64)> {
        handle
            .records()
            .iter()
            .filter(|r| r["op"] == "resize" && r["window"] == first.0)
            .map(|r| (r["w"].as_i64().unwrap(), r["h"].as_i64().unwrap()))
            .collect()
    };
    // A cancel (the page's pinch), then a new area at once.
    touch_at(&mut worker, 1, 1, Phase::Down, (300, 200), 0.0);
    touch_at(&mut worker, 1, 1, Phase::Cancel, (300, 200), 10.0);
    let mut upright = resize_at(&mut worker, 1, UPRIGHT, 20.0);
    worker.step_at(109.9);
    assert_eq!(
        posted(&handle),
        Vec::<(i64, i64)>::new(),
        "none within 100 ms of the cancel"
    );
    assert!(upright.try_recv().is_err(), "waiting");
    worker.step_at(110.0);
    assert_eq!(posted(&handle), [(667, 1361)]);
    assert!(upright.try_recv().unwrap().is_some_and(|r| r.ok));
    // A refused update: its contact ends with a cancel at its last point.
    touch_at(&mut worker, 1, 2, Phase::Down, (300, 900), 200.0);
    handle.refuse(true);
    touch_at(&mut worker, 1, 2, Phase::Update, (310, 900), 210.0);
    handle.refuse(false);
    assert_eq!(worker.contact, None);
    let mut wide = resize_at(&mut worker, 1, area(800.0, 320.0), 220.0);
    worker.step_at(309.9);
    assert_eq!(posted(&handle).len(), 1, "none within 100 ms of the cancel");
    worker.step_at(310.0);
    assert_eq!(posted(&handle), [(667, 1361), (1652, 661)]);
    assert!(wide.try_recv().unwrap().is_some_and(|r| r.ok));
    // Another editor's guard cancels this editor's contact; that guard's
    // own editor had no contact, so it places it at once.
    touch_at(&mut worker, 1, 3, Phase::Down, (10, 10), 400.0);
    let mut guarded = guard_at(&mut worker, 2, 410.0);
    assert_eq!(worker.contact, None);
    let mut again = resize_at(&mut worker, 1, UPRIGHT, 420.0);
    worker.step_at(509.9);
    assert_eq!(posted(&handle).len(), 2, "none within 100 ms of the cancel");
    worker.step_at(510.0);
    assert_eq!(posted(&handle), [(667, 1361), (1652, 661), (667, 1361)]);
    assert!(again.try_recv().unwrap().is_some_and(|r| r.ok));
    assert_eq!(guarded.try_recv().unwrap(), Ok(()));
    assert_eq!(
        touches(&handle),
        [
            t("down", 300, 200),
            t("cancel", 300, 200),
            t("down", 300, 900),
            t("cancel", 300, 900),
            t("down", 10, 10),
            t("cancel", 10, 10),
            t("down", 546, 15),
            t("up", 546, 15),
        ]
    );
}

#[test]
fn a_resize_that_does_not_land_settles_after_its_wait_and_a_newer_one_replaces_it() {
    let (mut worker, handle, heard) = worker();
    let window = take(&mut worker, &handle, 1, 0.0);
    handle.resize_late(true);
    let mut first = resize_at(&mut worker, 1, UPRIGHT, 100.0);
    worker.step_at(100.0);
    // A newer one replaces it: its caller hears nothing.
    let mut answer = resize_at(&mut worker, 1, area(800.0, 320.0), 200.0);
    assert!(first.try_recv().is_err());
    worker.step_at(1199.0);
    assert!(answer.try_recv().is_err(), "still waiting");
    worker.step_at(1200.0);
    assert_eq!(
        answer.try_recv().unwrap(),
        Some(Resized {
            asked: (1652, 661),
            client: (1349, 809),
            ok: false,
            ms: 1000.0,
            reverted: true,
        })
    );
    assert!(worker.editors[&1].resizing.is_none());
    // Its rectangle before goes back, after them (#74 review).
    assert_eq!(resizes(&handle), [(667, 1361), (1652, 661), (1349, 809)]);
    // They land late, one at a time: a grab of another size says so (its
    // points are of that size).
    assert!(handle.land_one(window));
    assert!(handle.land_one(window));
    assert_eq!(handle.size(window), Some((1652, 661)));
    let (sink, _frames) = sink();
    worker.command_at(Command::Capture { session: 1, sink }, 1300.0);
    worker.step_at(1300.0);
    let sized = |width: u32, height: u32| PlugwinEvent::Sized {
        session: 1,
        width,
        height,
    };
    assert_eq!(heard.lock().unwrap().as_slice(), [sized(1652, 661)]);
    assert_eq!(worker.editors[&1].size, (1652, 661));
    worker.step_at(1340.0);
    assert_eq!(heard.lock().unwrap().len(), 1, "said once");
    // The way back lands: said too.
    assert!(handle.land_one(window));
    worker.step_at(1380.0);
    assert_eq!(
        heard.lock().unwrap().as_slice(),
        [sized(1652, 661), sized(1349, 809)]
    );
}

#[test]
fn a_window_resized_on_the_pc_is_reported_and_put_back_by_the_guard() {
    let (mut worker, handle, heard) = worker();
    let window = take(&mut worker, &handle, 1, 0.0);
    let (sink, _frames) = sink();
    worker.command_at(Command::Capture { session: 1, sink }, 0.0);
    worker.step_at(0.0);
    assert!(heard.lock().unwrap().is_empty());
    handle.resize_window(window, (1000, 700));
    worker.step_at(40.0);
    assert_eq!(
        heard.lock().unwrap().as_slice(),
        [PlugwinEvent::Sized {
            session: 1,
            width: 1000,
            height: 700
        }]
    );
    let mut answer = guard_at(&mut worker, 1, 50.0);
    worker.step_at(50.0);
    worker.step_at(60.0);
    assert_eq!(answer.try_recv().unwrap(), Ok(()));
    assert_eq!(resizes(&handle), [(1349, 809), (1349, 809)]);
    assert_eq!(touches(&handle), [t("down", 546, 15), t("up", 546, 15)]);
}

#[test]
fn the_guard_puts_the_editors_own_size_back_before_its_tap() {
    let (mut worker, handle, _) = worker();
    let window = take(&mut worker, &handle, 1, 0.0);
    resize_at(&mut worker, 1, UPRIGHT, 0.0);
    worker.step_at(0.0);
    touch_at(&mut worker, 1, 1, Phase::Down, (300, 900), 5.0);
    let mut answer = guard_at(&mut worker, 1, 10.0);
    // The contact ends at once; the placements wait 100 ms after its up,
    // and the tap for the editor's own size.
    assert_eq!(touches(&handle), [t("down", 300, 900), t("up", 300, 900)]);
    assert!(answer.try_recv().is_err());
    assert_eq!(
        resizes(&handle),
        [(667, 1361)],
        "not within 100 ms of the up"
    );
    worker.step_at(110.0);
    assert_eq!(
        resizes(&handle),
        [(667, 1361), (1349, 809)],
        "a pixel aside first"
    );
    worker.step_at(120.0);
    assert_eq!(
        resizes(&handle),
        [(667, 1361), (1349, 809), (1349, 809)],
        "seen there: the known rectangle"
    );
    assert!(answer.try_recv().is_err(), "read back at the next look");
    worker.step_at(130.0);
    assert_eq!(answer.try_recv().unwrap(), Ok(()));
    assert_eq!(handle.size(window), Some((1349, 809)));
    // The tap at the inert spot of the editor's own width, after the
    // restore.
    let records = handle.records();
    let ops: Vec<String> = records
        .iter()
        .map(|r| r["op"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        ops,
        [
            "take", "resize", "touch", "touch", "resize", "resize", "touch", "touch"
        ]
    );
    assert_eq!(
        touches(&handle)[2..],
        [t("down", 546, 15), t("up", 546, 15)]
    );
    assert!(!worker.editors[&1].changed, "its own size again");
    assert_eq!(worker.editors[&1].size, (1349, 809));
    // At its own size again: a guard still places it (its own placement
    // proves nothing else waits), and the release posts no size.
    let mut answer = guard_at(&mut worker, 1, 140.0);
    assert!(answer.try_recv().is_err(), "placed first");
    worker.step_at(150.0);
    worker.step_at(160.0);
    assert_eq!(answer.try_recv().unwrap(), Ok(()));
    let (reply, _released) = oneshot::channel();
    worker.command_at(
        Command::Release {
            session: 1,
            closed: false,
            reply,
        },
        170.0,
    );
    assert_eq!(resizes(&handle).len(), 5);
}

#[test]
fn no_resize_reaches_an_editor_once_its_guard_began() {
    // The review of PR #74 (I2): an editor at its own size is tapped at
    // once, so its guard never waits; a resize that reached the worker
    // after that tap posted the page's shape during the guard's wait, and
    // Live closed Pro-Q at it.
    let (mut worker, handle, _) = worker();
    take(&mut worker, &handle, 1, 0.0);
    let mut guarded = guard_at(&mut worker, 1, 10.0);
    let mut answer = resize_at(&mut worker, 1, UPRIGHT, 20.0);
    assert_eq!(answer.try_recv().unwrap(), None, "refused");
    worker.step_at(30.0);
    worker.step_at(40.0);
    assert_eq!(guarded.try_recv().unwrap(), Ok(()));
    worker.step_at(1100.0);
    let own = [(1349, 809), (1349, 809)];
    assert_eq!(resizes(&handle), own, "the guard's own placements only");
    // Nor later: the guard's mark stays with the editor.
    let mut again = resize_at(&mut worker, 1, area(800.0, 320.0), 1200.0);
    assert_eq!(again.try_recv().unwrap(), None);
    assert_eq!(resizes(&handle), own);
}

/// Takes, as `session` at `now`, a window Live opened at `size` (its last
/// size, or one given on the PC) at `at`; its window.
fn take_sized(
    worker: &mut Worker,
    handle: &SimHandle,
    session: u32,
    size: (u32, u32),
    at: (i32, i32),
) -> WindowId {
    handle.auto_open(false);
    let window = handle.add_window(0);
    handle.resize_window(window, size);
    handle.move_window(window, at);
    let before: Vec<WindowId> = handle
        .windows()
        .into_iter()
        .filter(|w| *w != window)
        .collect();
    let (reply, mut answer) = oneshot::channel();
    let take = Command::Take {
        session,
        before,
        reply,
    };
    worker.command_at(take, 0.0);
    worker.step_at(0.0);
    assert_eq!(answer.try_recv().unwrap(), Ok(size));
    window
}

#[test]
fn the_guard_taps_only_at_the_size_its_inert_spot_is_known_at() {
    // The review of PR #74 (I1): the inert spot (546, 15) was verified at
    // 1349 × 809 only. An editor taken at another size (Live or Pro-Q kept
    // its last one, or it was resized on the PC first) was put back to
    // that size and tapped at 0.405 of its width: next to Undo at 760 px.
    let (mut worker, handle, _) = worker();
    let window = take_sized(&mut worker, &handle, 1, (760, 1271), (0, 0));
    let mut guarded = guard_at(&mut worker, 1, 10.0);
    assert!(guarded.try_recv().is_err(), "placed first: no tap yet");
    assert_eq!(touches(&handle), Vec::new());
    worker.step_at(10.0);
    assert_eq!(
        resizes(&handle),
        [(1349, 809), (1349, 809)],
        "the known size posted, a pixel aside first"
    );
    worker.step_at(20.0);
    assert_eq!(guarded.try_recv().unwrap(), Ok(()));
    assert_eq!(touches(&handle), [t("down", 546, 15), t("up", 546, 15)]);
    assert_eq!(handle.size(window), Some((1349, 809)));
    // Live closes it at that size: the release posts nothing (its window
    // is gone).
    let (reply, _released) = oneshot::channel();
    let release = Command::Release {
        session: 1,
        closed: true,
        reply,
    };
    worker.command_at(release, 30.0);
    assert_eq!(resizes(&handle).len(), 2, "nothing more posted");
    assert!(handle.windows().is_empty(), "Live closed it");
}

#[test]
fn a_guards_first_placement_at_the_work_areas_right_edge_goes_a_pixel_inward() {
    // The review of PR #74, round 3: a window taken low and right gets its
    // known rectangle moved in against the work area's right edge, and its
    // first placement went a pixel to the right, past that edge.
    let (mut worker, handle, _) = worker();
    let window = take_sized(&mut worker, &handle, 1, (760, 1271), (2000, 1000));
    let mut guarded = guard_at(&mut worker, 1, 10.0);
    let lefts = |handle: &SimHandle| -> Vec<(i64, i64)> {
        handle
            .records()
            .iter()
            .filter(|r| r["op"] == "resize")
            .map(|r| (r["left"].as_i64().unwrap(), r["top"].as_i64().unwrap()))
            .collect()
    };
    assert_eq!(lefts(&handle), [(1194, 552)], "a pixel to the left");
    worker.step_at(20.0);
    worker.step_at(30.0);
    assert_eq!(guarded.try_recv().unwrap(), Ok(()));
    assert_eq!(lefts(&handle), [(1194, 552), (1195, 552)]);
    assert_eq!(handle.rect(window), Some(rect(1195, 552, 1365, 848)));
    assert_eq!(touches(&handle), [t("down", 546, 15), t("up", 546, 15)]);
}

#[test]
fn a_guard_reads_the_editors_size_even_when_no_change_was_seen() {
    // The review of PR #74 (M5): a window resized on the PC within the last
    // capture period (no grab saw it) was tapped at once, at the spot of
    // another layout.
    let (mut worker, handle, _) = worker();
    let window = take(&mut worker, &handle, 1, 0.0);
    handle.resize_window(window, (1000, 700));
    let mut guarded = guard_at(&mut worker, 1, 5.0);
    assert!(guarded.try_recv().is_err(), "read, not trusted");
    worker.step_at(10.0);
    worker.step_at(20.0);
    assert_eq!(guarded.try_recv().unwrap(), Ok(()));
    assert_eq!(resizes(&handle), [(1349, 809), (1349, 809)]);
    assert_eq!(touches(&handle), [t("down", 546, 15), t("up", 546, 15)]);
}

#[test]
fn a_guard_whose_known_size_never_comes_taps_nothing_and_the_window_goes_back() {
    let (mut worker, handle, _) = worker();
    let window = take_sized(&mut worker, &handle, 1, (760, 1271), (300, 50));
    handle.resize_refused(true);
    let mut guarded = guard_at(&mut worker, 1, 10.0);
    worker.step_at(10.0);
    worker.step_at(1009.0);
    assert!(guarded.try_recv().is_err(), "still waiting");
    worker.step_at(1010.0);
    assert_eq!(
        guarded.try_recv().unwrap(),
        Err(format!(
            "{NOT_BACK}: its first placement did not land (the window is 776x1310 at 300,50, 1365x848 at 301,50 posted)"
        ))
    );
    assert_eq!(touches(&handle), Vec::new(), "no tap at an unknown spot");
    // Handed back while Live keeps it open: its own place and size again.
    handle.resize_refused(false);
    let (reply, _released) = oneshot::channel();
    worker.command_at(
        Command::Release {
            session: 1,
            closed: false,
            reply,
        },
        1100.0,
    );
    assert_eq!(handle.rect(window), Some(rect(300, 50, 776, 1310)));
}

#[test]
fn a_resize_that_never_lands_goes_back_and_the_guard_taps_nothing_while_nothing_lands() {
    // The review of PR #74 (M6): a resize that settled unlanded left the
    // window as it was then (Live's frame resized and Pro-Q not: a clipped
    // picture for the session); its rectangle before is posted back.
    // (M1, round 2): both may still wait in Live's thread while the picture
    // reads its size before, so a read of the size cannot tell "back" from
    // "nothing landed yet": the guard waits to see its own placement land
    // behind them, and taps nothing while nothing lands.
    let (mut worker, handle, _) = worker();
    take(&mut worker, &handle, 1, 0.0);
    handle.resize_late(true);
    let mut resized = resize_at(&mut worker, 1, UPRIGHT, 0.0);
    worker.step_at(1000.0);
    let settled = resized.try_recv().unwrap().expect("a resize");
    assert_eq!((settled.ok, settled.client), (false, (1349, 809)));
    assert_eq!(
        resizes(&handle),
        [(667, 1361), (1349, 809)],
        "its rectangle before posted back"
    );
    let mut guarded = guard_at(&mut worker, 1, 1100.0);
    assert!(
        guarded.try_recv().is_err(),
        "the known size read, placed first"
    );
    worker.step_at(1110.0);
    worker.step_at(2099.0);
    assert!(guarded.try_recv().is_err(), "still waiting");
    worker.step_at(2100.0);
    assert_eq!(
        guarded.try_recv().unwrap(),
        Err(format!(
            "{NOT_BACK}: its first placement did not land (the window is 1365x848 at 0,0, 1365x848 at 1,0 posted)"
        ))
    );
    assert_eq!(touches(&handle), Vec::new(), "no tap");
}

#[test]
fn the_guard_waits_for_a_resize_on_its_way_then_puts_the_size_back() {
    let (mut worker, handle, _) = worker();
    take(&mut worker, &handle, 1, 0.0);
    handle.resize_late(true);
    let mut resized = resize_at(&mut worker, 1, UPRIGHT, 0.0);
    let mut answer = guard_at(&mut worker, 1, 10.0);
    // A resize while the guard runs is not asked.
    let mut later = resize_at(&mut worker, 1, area(800.0, 320.0), 20.0);
    assert_eq!(later.try_recv().unwrap(), None);
    worker.step_at(500.0);
    assert_eq!(resizes(&handle), [(667, 1361)], "the guard's size waits");
    worker.step_at(1000.0);
    // Settled unlanded: its way back posted, then the guard's known size,
    // in the same step.
    assert_eq!(resized.try_recv().unwrap().map(|r| r.ok), Some(false));
    assert_eq!(resizes(&handle), [(667, 1361), (1349, 809), (1349, 809)]);
    assert!(answer.try_recv().is_err());
    // The window's thread lands them in order: the guard's placement, a
    // pixel aside, last. Once it is seen, the known rectangle; once that
    // reads back with the known size, the tap.
    let window = handle.windows()[0];
    assert!(handle.land_one(window));
    worker.step_at(1010.0);
    assert!(answer.try_recv().is_err(), "the upright picture first");
    handle.resize_late(false);
    worker.step_at(1020.0);
    assert!(answer.try_recv().is_err(), "its placement seen");
    worker.step_at(1030.0);
    assert_eq!(answer.try_recv().unwrap(), Ok(()));
    assert_eq!(touches(&handle), [t("down", 546, 15), t("up", 546, 15)]);
}

/// Lands the rectangles posted to `window` one at a time, with a step
/// 10 ms after each from `now` on (8 rounds), and checks after every step
/// that no tap came while a rectangle posted before it still waited: the
/// PC's window thread takes its posted messages before injected input, so
/// the tap would land in the layout of the last of them.
fn land_in_turn(worker: &mut Worker, handle: &SimHandle, window: WindowId, now: f64) {
    for round in 1..=8 {
        handle.land_one(window);
        worker.step_at(now + 10.0 * f64::from(round));
        if !touches(handle).is_empty() {
            assert_eq!(
                handle.waiting(window),
                0,
                "tapped while a posted rectangle waited: {:?}",
                resizes(handle)
            );
        }
    }
}

#[test]
fn the_guard_taps_only_once_every_rectangle_posted_before_its_own_landed() {
    // The review of PR #74, round 2 (N1): the guard trusted the picture's
    // size alone. A window at 1349 × 809 that crossed the work area (Live
    // reopened it there) got an upright resize while Live's thread was
    // busy: it settled unlanded and its rectangle before went back. The
    // guard's known rectangle (moved into the work area) waited behind
    // both; once the upright one landed (the window moved: no doubt) and
    // the one before it, the picture read 1349 × 809 and the guard tapped
    // with its own still waiting. Live lands that before the injected
    // down, which then met the EQ display 148 px below the inert spot.
    let (mut worker, handle, _) = worker();
    let window = take_sized(&mut worker, &handle, 1, KNOWN_SIZE, (900, 700));
    handle.resize_late(true);
    resize_at(&mut worker, 1, UPRIGHT, 0.0);
    worker.step_at(1000.0);
    assert_eq!(
        resizes(&handle),
        [(667, 1361), (1349, 809)],
        "upright, then its way back"
    );
    let mut guarded = guard_at(&mut worker, 1, 1010.0);
    worker.step_at(1010.0);
    land_in_turn(&mut worker, &handle, window, 1010.0);
    assert_eq!(guarded.try_recv().unwrap(), Ok(()));
    assert_eq!(touches(&handle), [t("down", 546, 15), t("up", 546, 15)]);
    assert_eq!(
        handle.rect(window),
        Some(rect(900, 552, 1365, 848)),
        "the known size, moved into the work area"
    );
}

#[test]
fn a_resize_that_reads_its_size_at_once_proves_nothing_to_the_guard() {
    // The review of PR #74, round 2 (N1): a resize that asks the size the
    // picture already reads lands at its first look. A newer area (Pro-Q's
    // own shape) replaced an upright resize still on its way, so the
    // upright one still waited in Live's thread while the picture read
    // 1349 × 809: the guard tapped at once, before it landed.
    let (mut worker, handle, _) = worker();
    let window = take(&mut worker, &handle, 1, 0.0);
    handle.resize_late(true);
    resize_at(&mut worker, 1, UPRIGHT, 10.0);
    let mut own = resize_at(&mut worker, 1, area(1349.0, 809.0), 20.0);
    worker.step_at(20.0);
    let landed = own.try_recv().unwrap().expect("a resize");
    assert_eq!(
        (landed.client, landed.ok),
        ((1349, 809), true),
        "read at once"
    );
    assert_eq!(handle.waiting(window), 2, "both still on their way");
    let mut guarded = guard_at(&mut worker, 1, 30.0);
    worker.step_at(30.0);
    land_in_turn(&mut worker, &handle, window, 30.0);
    assert_eq!(guarded.try_recv().unwrap(), Ok(()));
    assert_eq!(touches(&handle), [t("down", 546, 15), t("up", 546, 15)]);
    assert_eq!(handle.size(window), Some(KNOWN_SIZE));
}

#[test]
fn a_guard_taps_once_its_own_placement_lands_though_nothing_was_seen_to_move() {
    // The review of PR #74, round 2 (N2): the window's thread landed the
    // resize and its way back between two of the worker's looks, so
    // nothing it read moved: the doubt never cleared and the close was
    // left open in Live. The guard's own placement, a pixel aside first,
    // is seen to land, and with it everything posted before it.
    let (mut worker, handle, _) = worker();
    take(&mut worker, &handle, 1, 0.0);
    handle.resize_late(true);
    resize_at(&mut worker, 1, UPRIGHT, 0.0);
    worker.step_at(1000.0);
    handle.resize_late(false);
    worker.step_at(1010.0);
    let mut guarded = guard_at(&mut worker, 1, 1020.0);
    worker.step_at(1030.0);
    worker.step_at(1040.0);
    assert_eq!(guarded.try_recv().unwrap(), Ok(()));
    assert_eq!(touches(&handle), [t("down", 546, 15), t("up", 546, 15)]);
}

#[test]
fn a_guard_taps_once_its_own_placement_lands_though_live_keeps_the_editors_size() {
    // The review of PR #74, round 2 (N2): Live's window may keep its size
    // (whether it passes a resize on is the PC check's). An upright resize
    // then never lands, its way back changes nothing, and the guard's
    // known rectangle is where the window stands: nothing ever moved, the
    // doubt never cleared, and every close was left open in Live. The
    // guard's placement moves the window a pixel first: that lands, seen.
    let (mut worker, handle, _) = worker();
    let window = take(&mut worker, &handle, 1, 0.0);
    handle.size_kept(true);
    let mut resized = resize_at(&mut worker, 1, UPRIGHT, 0.0);
    worker.step_at(1000.0);
    let settled = resized.try_recv().unwrap().expect("a resize");
    assert_eq!(
        (settled.ok, settled.client, settled.reverted),
        (false, (1349, 809), true)
    );
    let mut guarded = guard_at(&mut worker, 1, 1010.0);
    for now in [1020.0, 1030.0, 1040.0] {
        worker.step_at(now);
    }
    assert_eq!(guarded.try_recv().unwrap(), Ok(()));
    assert_eq!(touches(&handle), [t("down", 546, 15), t("up", 546, 15)]);
    assert_eq!(handle.rect(window), Some(rect(0, 0, 1365, 848)));
}

#[test]
fn a_guard_whose_size_never_comes_back_taps_nothing_and_the_release_posts_it_again() {
    let (mut worker, handle, _) = worker();
    let window = take(&mut worker, &handle, 1, 0.0);
    resize_at(&mut worker, 1, UPRIGHT, 0.0);
    worker.step_at(0.0);
    handle.resize_refused(true);
    let mut answer = guard_at(&mut worker, 1, 100.0);
    worker.step_at(100.0);
    worker.step_at(1099.0);
    assert!(answer.try_recv().is_err(), "still waiting");
    worker.step_at(1100.0);
    assert_eq!(
        answer.try_recv().unwrap(),
        Err(format!(
            "{NOT_BACK}: its first placement did not land (the window is 683x1400 at 0,0, 1365x848 at 1,0 posted)"
        ))
    );
    assert_eq!(touches(&handle), Vec::new(), "no tap at an unknown spot");
    assert!(worker.guarding.is_empty());
    // The release (the router leaves the editor open) posts it again.
    handle.resize_refused(false);
    let (reply, _released) = oneshot::channel();
    worker.command_at(
        Command::Release {
            session: 1,
            closed: false,
            reply,
        },
        1200.0,
    );
    assert_eq!(resizes(&handle), [(667, 1361), (1349, 809), (1349, 809)]);
    assert_eq!(
        handle.records().last(),
        Some(&json!({"op": "release", "window": window.0})),
        "the size before the z-order"
    );
    assert_eq!(handle.size(window), Some((1349, 809)));
}

#[test]
fn a_guard_whose_second_placement_never_lands_taps_nothing() {
    // The first placement landed, so nothing posted before it waits; the
    // known rectangle itself never does (Live's thread takes no rectangle
    // from then on): its own wait from its own post, then no tap.
    let (mut worker, handle, _) = worker();
    take_sized(&mut worker, &handle, 1, (760, 1271), (0, 0));
    let mut guarded = guard_at(&mut worker, 1, 10.0);
    handle.resize_refused(true);
    worker.step_at(20.0);
    assert_eq!(resizes(&handle), [(1349, 809), (1349, 809)], "both posted");
    worker.step_at(1019.0);
    assert!(
        guarded.try_recv().is_err(),
        "its wait runs from its own post"
    );
    worker.step_at(1020.0);
    assert_eq!(
        guarded.try_recv().unwrap(),
        Err(format!(
            "{NOT_BACK}: its placement did not land (the window is 1365x848 at 1,0 with a 1349x809 picture, 1365x848 at 0,0 posted for 1349x809)"
        ))
    );
    assert_eq!(touches(&handle), Vec::new(), "no tap");
}

#[test]
fn a_guard_of_an_editor_live_keeps_at_another_size_taps_nothing() {
    // Live's window may keep its size (the PC check's): one at another size
    // never reads the known rectangle a pixel aside, whose size differs, so
    // the guard taps nothing at a spot of an unknown layout (the editor is
    // left open in Live).
    let (mut worker, handle, _) = worker();
    take_sized(&mut worker, &handle, 1, (760, 1271), (0, 0));
    handle.size_kept(true);
    let mut guarded = guard_at(&mut worker, 1, 10.0);
    worker.step_at(20.0);
    worker.step_at(1009.0);
    assert!(guarded.try_recv().is_err(), "still waiting");
    worker.step_at(1010.0);
    assert_eq!(
        guarded.try_recv().unwrap(),
        Err(format!(
            "{NOT_BACK}: its first placement did not land (the window is 776x1310 at 1,0, 1365x848 at 1,0 posted)"
        ))
    );
    assert_eq!(touches(&handle), Vec::new(), "no tap");
}

#[test]
fn a_guard_whose_window_goes_while_it_waits_fails() {
    let (mut worker, handle, _) = worker();
    let window = take(&mut worker, &handle, 1, 0.0);
    handle.resize_late(true);
    resize_at(&mut worker, 1, UPRIGHT, 0.0);
    let mut answer = guard_at(&mut worker, 1, 10.0);
    worker.step_at(10.0);
    handle.remove_window(window);
    // The step that finds it gone (the captures) comes after the guards'.
    worker.step_at(20.0);
    assert!(answer.try_recv().is_err());
    worker.step_at(30.0);
    assert_eq!(answer.try_recv().unwrap(), Err(NO_EDITOR.to_string()));
    assert!(worker.guarding.is_empty());
}

#[test]
fn the_stop_posts_a_changed_editors_own_size_before_handing_it_back() {
    let (mut worker, handle, _) = worker();
    take(&mut worker, &handle, 1, 0.0);
    take(&mut worker, &handle, 2, 0.0);
    resize_at(&mut worker, 2, UPRIGHT, 0.0);
    worker.step_at(0.0);
    let _waiting = guard_at(&mut worker, 2, 10.0);
    worker.shutdown();
    assert!(worker.guarding.is_empty());
    let ops: Vec<(String, i64)> = handle
        .records()
        .iter()
        .skip(2)
        .map(|r| {
            (
                r["op"].as_str().unwrap().to_string(),
                r["w"].as_i64().unwrap_or(0),
            )
        })
        .collect();
    assert_eq!(
        ops,
        [
            ("resize".to_string(), 667),
            ("resize".to_string(), 1349),
            ("release".to_string(), 0),
            ("resize".to_string(), 1349),
            ("release".to_string(), 0),
        ]
    );
}

#[test]
fn a_release_within_100_ms_of_a_contacts_end_leaves_the_editor_where_it_is() {
    // The final review of PR #74 (its first note): the release ended a
    // contact and posted the take's rectangle at once. Live's thread takes
    // posted messages before injected input, so the editor could move
    // before that up landed (a stop, or a failed guard, with a finger
    // down). Within 100 ms of a contact's end the release posts nothing:
    // the editor stays where it is (this path leaves it open in Live).
    let (mut worker, handle, _) = worker();
    let release = |worker: &mut Worker, session: u32, now: f64| {
        let (reply, mut answer) = oneshot::channel();
        let command = Command::Release {
            session,
            closed: false,
            reply,
        };
        worker.command_at(command, now);
        assert_eq!(answer.try_recv(), Ok(()));
    };
    // A finger down at the release: its own up, and nothing posted.
    let first = take(&mut worker, &handle, 1, 0.0);
    resize_at(&mut worker, 1, UPRIGHT, 0.0);
    worker.step_at(0.0);
    assert_eq!(resizes(&handle), [(667, 1361)]);
    touch_at(&mut worker, 1, 1, Phase::Down, (300, 200), 10.0);
    release(&mut worker, 1, 20.0);
    assert_eq!(touches(&handle), [t("down", 300, 200), t("up", 300, 200)]);
    assert_eq!(
        resizes(&handle),
        [(667, 1361)],
        "nothing right after its own up"
    );
    assert_eq!(
        handle.records().last(),
        Some(&json!({"op": "release", "window": first.0}))
    );
    assert_eq!(handle.size(first), Some((667, 1361)), "where it is");
    // An up 99.9 ms before the release: nothing either.
    let second = take(&mut worker, &handle, 2, 100.0);
    resize_at(&mut worker, 2, UPRIGHT, 100.0);
    worker.step_at(100.0);
    touch_at(&mut worker, 2, 2, Phase::Down, (300, 200), 110.0);
    touch_at(&mut worker, 2, 2, Phase::Up, (300, 200), 120.0);
    release(&mut worker, 2, 219.9);
    assert_eq!(
        resizes(&handle),
        [(667, 1361), (667, 1361)],
        "not within 100 ms of the up"
    );
    assert_eq!(handle.size(second), Some((667, 1361)));
    // 100 ms after it: the take's rectangle again.
    let third = take(&mut worker, &handle, 3, 300.0);
    resize_at(&mut worker, 3, UPRIGHT, 300.0);
    worker.step_at(300.0);
    touch_at(&mut worker, 3, 3, Phase::Down, (300, 200), 310.0);
    touch_at(&mut worker, 3, 3, Phase::Up, (300, 200), 320.0);
    release(&mut worker, 3, 420.0);
    assert_eq!(
        resizes(&handle),
        [(667, 1361), (667, 1361), (667, 1361), (1349, 809)]
    );
    assert_eq!(handle.size(third), Some((1349, 809)), "its own size again");
}

#[tokio::test]
async fn the_handle_resizes_through_the_worker_thread() {
    let (sim, handle) = Sim::new(None);
    handle.still(true);
    let plugwin = Plugwin::spawn(Box::new(sim), Arc::new(|_: PlugwinEvent| {})).unwrap();
    let bounded = Duration::from_secs(5);
    let size = tokio::time::timeout(bounded, plugwin.take(1, Vec::new()))
        .await
        .unwrap();
    assert_eq!(size, Ok((1349, 809)));
    let resized = tokio::time::timeout(bounded, plugwin.resize(1, UPRIGHT))
        .await
        .unwrap()
        .expect("a resize");
    assert_eq!(
        (resized.asked, resized.client, resized.ok),
        ((667, 1361), (667, 1361), true)
    );
    assert!(resized.ms < RESIZE_MS, "{resized:?}");
    assert_eq!(plugwin.resize(2, UPRIGHT).await, None, "no such editor");
    // The guard puts the size back before its tap.
    assert_eq!(
        tokio::time::timeout(bounded, plugwin.guard(1))
            .await
            .unwrap(),
        Ok(())
    );
    assert_eq!(resizes(&handle), [(667, 1361), (1349, 809), (1349, 809)]);
    assert_eq!(touches(&handle), [t("down", 546, 15), t("up", 546, 15)]);
    plugwin.stop();
    assert!(plugwin.stopped().await);
    assert_eq!(plugwin.resize(1, UPRIGHT).await, None, "stopped");
}

#[tokio::test]
async fn the_handles_commands_reach_the_worker_in_call_order_whatever_is_awaited_first() {
    // The review of PR #74, round 2 (N4): the router's order test runs on
    // the current-thread runtime, which polls its tasks in spawn order, so
    // the old `async fn`s (sent at their first poll) passed it too. Here the
    // guard's future is made first and the resize's second, and the resize's
    // is awaited first: sent at the call, the guard reaches the worker first
    // and the resize is refused; sent at the first poll, the resize went
    // first and the upright size was posted before the guard.
    let (sim, handle) = Sim::new(None);
    handle.still(true);
    let plugwin = Plugwin::spawn(Box::new(sim), Arc::new(|_: PlugwinEvent| {})).unwrap();
    let bounded = Duration::from_secs(5);
    let size = tokio::time::timeout(bounded, plugwin.take(1, Vec::new()))
        .await
        .unwrap();
    assert_eq!(size, Ok((1349, 809)));
    let guarded = plugwin.guard(1);
    let resized = plugwin.resize(1, UPRIGHT);
    assert_eq!(
        tokio::time::timeout(bounded, resized).await.unwrap(),
        None,
        "refused: its guard began first"
    );
    assert_eq!(
        tokio::time::timeout(bounded, guarded).await.unwrap(),
        Ok(())
    );
    let posted = resizes(&handle);
    assert!(
        posted.iter().all(|size| *size == (1349, 809)),
        "only the guard's own size: {posted:?}"
    );
    assert_eq!(touches(&handle), [t("down", 546, 15), t("up", 546, 15)]);
    plugwin.stop();
    assert!(plugwin.stopped().await);
}
