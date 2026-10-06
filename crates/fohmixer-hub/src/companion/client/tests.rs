use std::time::Duration;

use tokio::io::AsyncReadExt;

use super::*;

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
