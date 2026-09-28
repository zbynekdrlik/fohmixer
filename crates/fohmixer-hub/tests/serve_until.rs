//! In-process tests of `fohmixer_hub::serve_until`: it binds, reports the
//! bound address, answers, and returns within its drain bound after the stop;
//! a taken port is an error, not a hang.
//!
//! One test at a time, and never at the same time as
//! `tests/graceful_stop.rs` (`SERIAL` here; `cargo test` runs one test binary
//! at a time; the `hub-ports` test group under nextest): the kernel readily
//! hands a just-freed port to the next bind to port 0, so another test's
//! listener could take this one's port and answer the "refused" check.

use std::future::Future;
use std::net::{SocketAddr, TcpListener};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use fohmixer_hub::config::Config;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;

static SERIAL: Mutex<()> = Mutex::new(());

/// This test's turn; a panicked earlier test does not block the rest.
fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// A config on `dir` without Live instances (nothing to connect to here).
fn config(dir: &std::path::Path) -> Config {
    let mut config = Config::defaults(dir);
    config.instances.clear();
    config
}

/// `f` on a fresh current-thread runtime (the `SERIAL` guard stays outside
/// the async code).
fn block_on<F: Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a tokio runtime")
        .block_on(f)
}

/// In-process: `serve_until` binds, reports the address, answers
/// `/api/version`, and returns `Ok` within 6 s of the stop.
#[test]
fn serve_until_answers_then_stops_within_six_seconds() {
    let _serial = serial();
    block_on(answers_then_stops());
}

async fn answers_then_stops() {
    let dir = tempfile::tempdir().unwrap();
    let (ready_tx, ready_rx) = oneshot::channel();
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let task = tokio::spawn(fohmixer_hub::serve_until(
        SocketAddr::from(([127, 0, 0, 1], 0)),
        config(dir.path()),
        ready_tx,
        async move {
            let _ = stop_rx.await;
        },
    ));
    let addr = tokio::time::timeout(Duration::from_secs(5), ready_rx)
        .await
        .expect("ready within 5 s")
        .expect("serve_until reports its address");
    assert_ne!(addr.port(), 0, "the bound port, not the requested 0");

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    stream
        .write_all(b"GET /api/version HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_string(&mut text))
        .await
        .expect("an answer within 5 s")
        .unwrap();
    assert!(text.starts_with("HTTP/1.1 200 OK"), "{text}");
    assert!(
        text.contains(&format!("\"version\":\"{}\"", fohmixer_proto::VERSION)),
        "{text}"
    );
    drop(stream);

    let stopped = Instant::now();
    stop_tx.send(()).unwrap();
    let result = tokio::time::timeout(Duration::from_secs(6), task)
        .await
        .expect("serve_until returns within 6 s of the stop")
        .expect("the serve task did not panic");
    result.expect("serve_until returns Ok");
    assert!(stopped.elapsed() < Duration::from_secs(6));
    // The listener is closed: a new connection is refused.
    assert!(
        std::net::TcpStream::connect(addr).is_err(),
        "the listener still accepts after serve_until returned"
    );
}

/// A bind failure is an error naming the address, not a hang.
#[test]
fn serve_until_reports_a_taken_port() {
    let _serial = serial();
    block_on(reports_a_taken_port());
}

async fn reports_a_taken_port() {
    let dir = tempfile::tempdir().unwrap();
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = taken.local_addr().unwrap();
    let (ready_tx, ready_rx) = oneshot::channel();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        fohmixer_hub::serve_until(addr, config(dir.path()), ready_tx, std::future::pending()),
    )
    .await
    .expect("a bind failure returns at once");
    let error = result.expect_err("the port is taken");
    assert!(
        format!("{error:#}").starts_with(&format!("binding {addr}")),
        "{error:#}"
    );
    assert!(
        ready_rx.await.is_err(),
        "no ready signal without a listener"
    );
}
