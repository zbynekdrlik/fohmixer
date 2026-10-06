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
    handle.stop();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("a stop ends the task")
        .unwrap();
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
    // A Companion that registers the surface, then neither answers the
    // stop nor closes: the hub's REMOVE-DEVICE and its FIN come at once,
    // the hub keeps reading until the bound, then the task ends.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (events, _seen) = sink();
    let (handle, task) = CompanionHandle::spawn(&deck(port), events);
    let (socket, _) = listener.accept().await.unwrap();
    let (read_half, mut write_half) = socket.into_split();
    let mut lines = BufReader::new(read_half).lines();
    let wait = Duration::from_secs(1);
    write_half
        .write_all(b"BEGIN CompanionVersion=\"5.0.7\" ApiVersion=\"1.12.0\" \n")
        .await
        .unwrap();
    let add = tokio::time::timeout(wait, lines.next_line())
        .await
        .expect("ADD-DEVICE at once")
        .unwrap()
        .unwrap();
    assert!(
        add.starts_with("ADD-DEVICE DEVICEID=\"fohmixer-1\" "),
        "{add}"
    );
    write_half
        .write_all(b"ADD-DEVICE OK DEVICEID=\"fohmixer-1\" \n")
        .await
        .unwrap();
    let deadline = Instant::now() + wait;
    while !handle.snapshot().online {
        assert!(Instant::now() < deadline, "never registered");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let stopped = Instant::now();
    handle.stop();
    let last = tokio::time::timeout(wait, lines.next_line())
        .await
        .expect("REMOVE-DEVICE at once")
        .unwrap();
    assert_eq!(
        last.as_deref(),
        Some("REMOVE-DEVICE DEVICEID=\"fohmixer-1\"")
    );
    // The hub's FIN right after it: its write half shut down, long before
    // the bound.
    let fin = tokio::time::timeout(wait, lines.next_line())
        .await
        .expect("the FIN at once")
        .unwrap();
    assert_eq!(fin, None);
    assert!(stopped.elapsed() < STOP_BOUND, "{:?}", stopped.elapsed());
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("the bound ends the stop")
        .unwrap();
    let took = stopped.elapsed();
    assert!(
        took > STOP_BOUND && took < Duration::from_secs(1),
        "{took:?}"
    );
    drop(write_half);
}
