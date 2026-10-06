use std::time::Duration;

use tokio::io::AsyncReadExt;

use super::*;
use crate::companion::STOP_BOUND;

fn deck(port: u16) -> CompanionCfg {
    CompanionCfg {
        host: "127.0.0.1".into(),
        port,
        columns: 8,
        rows: 4,
        bitmap_px: 72,
        title: "Stream Deck".into(),
    }
}

/// An event sink and what it got.
fn sink() -> (Events, Arc<Mutex<Vec<CompanionEvent>>>) {
    let seen = Arc::new(Mutex::new(Vec::<CompanionEvent>::new()));
    let into = Arc::clone(&seen);
    let events: Events = Arc::new(move |e: CompanionEvent| into.lock().unwrap().push(e));
    (events, seen)
}

/// Waits up to 1 s for `event`.
async fn answered_at_once(seen: &Mutex<Vec<CompanionEvent>>, event: &CompanionEvent) {
    let started = Instant::now();
    while !seen.lock().unwrap().contains(event) {
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "answered at once, never queued"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Every answer `press` got so far.
fn answers_of(seen: &Mutex<Vec<CompanionEvent>>, press: Press) -> Vec<Answer> {
    seen.lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            CompanionEvent::Answered(answer) if answer.press == press => Some(answer.clone()),
            _ => None,
        })
        .collect()
}

/// Companion's side of the hub's connection on `listener`, past the
/// handshake (`BEGIN`, the hub's `ADD-DEVICE`, `ADD-DEVICE OK`), once the
/// handle reports the session.
async fn registered(
    listener: &tokio::net::TcpListener,
    handle: &CompanionHandle,
) -> BufReader<TcpStream> {
    let (socket, _) = listener.accept().await.unwrap();
    let mut companion = BufReader::new(socket);
    companion
        .get_mut()
        .write_all(b"BEGIN CompanionVersion=\"5.0.7\" ApiVersion=\"1.12.0\" \n")
        .await
        .unwrap();
    let add = next_line(&mut companion).await.expect("ADD-DEVICE");
    assert!(
        add.starts_with("ADD-DEVICE DEVICEID=\"fohmixer-1\" "),
        "{add}"
    );
    companion
        .get_mut()
        .write_all(b"ADD-DEVICE OK DEVICEID=\"fohmixer-1\" \n")
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(1);
    while !handle.snapshot().online {
        assert!(Instant::now() < deadline, "never registered");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    companion
}

/// The hub's next line within 1 s, its line end removed; `None` at its FIN.
async fn next_line(companion: &mut BufReader<TcpStream>) -> Option<String> {
    let mut line = String::new();
    let read = tokio::time::timeout(Duration::from_secs(1), companion.read_line(&mut line))
        .await
        .expect("a line, or the FIN, within 1 s")
        .unwrap();
    (read > 0).then(|| line.trim_end_matches('\n').to_string())
}

#[tokio::test]
async fn a_press_before_add_device_ok_is_answered_offline_and_never_written() {
    // A Companion that accepts and says nothing: the session waits for BEGIN.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (events, seen) = sink();
    let (handle, task) = CompanionHandle::spawn(&deck(port), events);
    let (mut socket, _) = listener.accept().await.unwrap();
    let press = Press {
        key: 2,
        down: true,
        from: Some((4, 1)),
    };
    handle.press(press);
    answered_at_once(&seen, &CompanionEvent::Answered(Answer::offline(press))).await;
    // Nothing reached Companion.
    let mut buf = [0_u8; 64];
    let read = tokio::time::timeout(Duration::from_millis(500), socket.read(&mut buf)).await;
    assert!(read.is_err(), "no line went out: {read:?}");
    handle.stop();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("a stop ends the task")
        .unwrap();
}

#[tokio::test]
async fn a_press_without_a_session_is_answered_offline_at_once() {
    // A port nothing listens on: the task stays without a session.
    let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let (events, seen) = sink();
    let (handle, task) = CompanionHandle::spawn(&deck(port), events);
    let press = Press {
        key: 1,
        down: true,
        from: Some((3, 9)),
    };
    handle.press(press);
    answered_at_once(&seen, &CompanionEvent::Answered(Answer::offline(press))).await;
    // The first failure of the outage is an event; the status counts it.
    let deadline = Instant::now() + Duration::from_secs(3);
    while handle.snapshot().connect_failures == 0 {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(seen.lock().unwrap().iter().any(|e| matches!(
        e,
        CompanionEvent::Failed {
            refused: false,
            attempts: 1,
            ..
        }
    )));
    assert!(!handle.status().online);
    // A press behind the stop: offline at once; a press after the task
    // ended: offline from the handle. Each press gets one answer.
    handle.stop();
    let behind = Press {
        key: 2,
        down: true,
        from: Some((3, 10)),
    };
    handle.press(behind);
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("a stop ends the task")
        .unwrap();
    let after = Press {
        key: 3,
        down: false,
        from: Some((3, 11)),
    };
    handle.press(after);
    for press in [press, behind, after] {
        assert_eq!(
            answers_of(&seen, press),
            vec![Answer::offline(press)],
            "{press:?}"
        );
    }
}

#[tokio::test]
async fn the_reader_passes_a_line_and_the_close_on_then_ends() {
    // Companion writes one line and closes: the reader hands on the line,
    // then the close, and ends (its sender gone), reading nothing more.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let hub = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut companion, _) = listener.accept().await.unwrap();
    companion.write_all(b"PONG 1 \n").await.unwrap();
    drop(companion);
    let (read_half, _write_half) = hub.into_split();
    let (lines, mut inbound) = mpsc::channel(INBOUND_QUEUE);
    let reader = tokio::spawn(read_lines(BufReader::new(read_half), lines));
    for want in [Some(Read::Line("PONG 1 ".into())), Some(Read::Closed), None] {
        let got = tokio::time::timeout(Duration::from_secs(1), inbound.recv())
            .await
            .expect("the reader hands on at once");
        assert_eq!(got, want);
    }
    tokio::time::timeout(Duration::from_secs(1), reader)
        .await
        .expect("the reader ended")
        .unwrap();
}

#[tokio::test]
async fn a_companion_that_never_answers_the_stop_holds_it_500_ms_at_most() {
    // A Companion that registers the surface, then answers nothing and
    // never closes: the hub's last line and its FIN come at once, the hub
    // keeps reading until the bound, and the press Companion never
    // answered is answered offline then, once; a press behind the stop is
    // answered offline at once, before the wait.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (events, seen) = sink();
    let (handle, task) = CompanionHandle::spawn(&deck(port), events);
    let mut companion = registered(&listener, &handle).await;
    let pending = Press {
        key: 7,
        down: true,
        from: Some((5, 1)),
    };
    handle.press(pending);
    assert_eq!(
        next_line(&mut companion).await.as_deref(),
        Some("KEY-PRESS DEVICEID=\"fohmixer-1\" KEY=7 PRESSED=1")
    );
    let stopped = Instant::now();
    handle.stop();
    // Behind the stop (both queued before the task runs: one thread).
    let behind = Press {
        key: 8,
        down: true,
        from: Some((5, 2)),
    };
    handle.press(behind);
    assert_eq!(
        next_line(&mut companion).await.as_deref(),
        Some("REMOVE-DEVICE DEVICEID=\"fohmixer-1\"")
    );
    // The hub's FIN right after it: its write half shut down, long before
    // the bound.
    assert_eq!(next_line(&mut companion).await, None);
    assert!(stopped.elapsed() < STOP_BOUND, "{:?}", stopped.elapsed());
    // By order, not by a clock: the press behind the stop was answered
    // before REMOVE-DEVICE went out, while the waiting one has no answer
    // until the wait is over.
    assert_eq!(answers_of(&seen, behind), vec![Answer::offline(behind)]);
    assert!(answers_of(&seen, pending).is_empty(), "no answer yet");
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("the bound ends the stop")
        .unwrap();
    let took = stopped.elapsed();
    assert!(
        took > STOP_BOUND && took < Duration::from_secs(1),
        "{took:?}"
    );
    assert_eq!(answers_of(&seen, pending), vec![Answer::offline(pending)]);
    assert_eq!(answers_of(&seen, behind), vec![Answer::offline(behind)]);
    drop(companion);
}

#[tokio::test]
async fn a_reset_ends_the_link_with_the_reads_error() {
    // Companion registers the surface, then resets the connection (an
    // abortive close): the link is lost with the read's own error.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (events, seen) = sink();
    let (handle, task) = CompanionHandle::spawn(&deck(port), events);
    let socket = registered(&listener, &handle).await.into_inner();
    socket.set_zero_linger().unwrap();
    drop(socket);
    let deadline = Instant::now() + Duration::from_secs(2);
    let error = loop {
        let down = seen.lock().unwrap().iter().find_map(|e| match e {
            CompanionEvent::Down { error } => Some(error.clone()),
            _ => None,
        });
        if let Some(error) = down {
            break error;
        }
        assert!(Instant::now() < deadline, "the link was never lost");
        tokio::time::sleep(Duration::from_millis(5)).await;
    };
    let cause = error
        .strip_prefix("reading from Companion failed: ")
        .unwrap_or_else(|| panic!("not the read's error: {error}"));
    assert!(!cause.is_empty(), "{error}");
    assert_eq!(handle.snapshot().last_error, Some(error.clone()));
    handle.stop();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("a stop ends the task")
        .unwrap();
}

#[tokio::test]
async fn the_writers_end_is_told_with_its_error() {
    assert_eq!(writer_outcome(Ok(Ok(()))), Ok(()));
    assert_eq!(
        writer_outcome(Ok(Err(std::io::Error::other("test reset")))),
        Err("test reset".to_string())
    );
    let task = tokio::spawn(std::future::pending::<std::io::Result<()>>());
    task.abort();
    let cancelled = writer_outcome(task.await);
    assert!(
        cancelled
            .as_ref()
            .is_err_and(|error| error.contains("cancelled")),
        "{cancelled:?}"
    );
    // A session that loses its writer names the failed write's error.
    assert_eq!(
        write_end(&Err("test reset".into())),
        "a write to Companion failed: test reset"
    );
    assert_eq!(write_end(&Ok(())), "the writer to Companion ended");
    // The stop's note: everything written, a failed write, still writing.
    assert_eq!(
        writer_note(Some(&Ok(()))),
        "every line written, then the FIN"
    );
    assert_eq!(
        writer_note(Some(&Err("test reset".into()))),
        "a write failed: test reset"
    );
    assert_eq!(writer_note(None), "still writing when the wait ended");
}
