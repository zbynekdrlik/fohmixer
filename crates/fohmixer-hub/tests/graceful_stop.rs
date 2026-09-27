//! The hub stops gracefully (copied from iemmixer's
//! `iem-server/tests/graceful_stop.rs` @ 22372bc and adapted): the stop
//! closes the listener at once, open requests get up to 5 s, the process
//! exits 0 and the port is free.
//!
//! Unix only: the process tests send SIGTERM/SIGINT. The Windows stop
//! (Ctrl-Break to the hub's console) is exercised on the PC from S3 on.
#![cfg(unix)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::oneshot;

/// In-process: `serve_until` binds, reports the address, answers
/// `/api/version`, and returns `Ok` within 6 s of the stop.
#[tokio::test]
async fn serve_until_answers_then_stops_within_six_seconds() {
    let (ready_tx, ready_rx) = oneshot::channel();
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let task = tokio::spawn(fohmixer_hub::serve_until(
        SocketAddr::from(([127, 0, 0, 1], 0)),
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
    // The listener is closed: a new connection is refused. (Not a re-bind
    // check: the server's side of the connection just closed can still hold
    // the port for a while, in FIN_WAIT or TIME_WAIT, after the listener is
    // gone; the process tests below check the port with the process ended.)
    assert!(
        TcpStream::connect(addr).is_err(),
        "the listener still accepts after serve_until returned"
    );
}

/// A bind failure is an error naming the address, not a hang.
#[tokio::test]
async fn serve_until_reports_a_taken_port() {
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = taken.local_addr().unwrap();
    let (ready_tx, ready_rx) = oneshot::channel();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        fohmixer_hub::serve_until(addr, ready_tx, std::future::pending()),
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

struct Server {
    child: Child,
    port: u16,
    dir: tempfile::TempDir,
}

impl Server {
    /// Everything the hub logged (stdout and stderr).
    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("hub.log")).unwrap_or_default()
    }
}

impl Drop for Server {
    /// A failed test still asks its hub to stop (without waiting).
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = signal(self.child.id(), "TERM");
        }
    }
}

/// A local port nobody listens on (the probe listener is closed again).
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// One `GET /api/version` on its own connection; the status line.
fn version_status(port: u16) -> Option<String> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    s.write_all(b"GET /api/version HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .ok()?;
    let mut text = String::new();
    s.read_to_string(&mut text).ok()?;
    text.lines().next().map(str::to_string)
}

/// The hub binary on a free port, once it answers `/api/version`.
fn start() -> Server {
    let dir = tempfile::tempdir().unwrap();
    let log = std::fs::File::create(dir.path().join("hub.log")).unwrap();
    let port = free_port();
    let child = Command::new(env!("CARGO_BIN_EXE_fohmixer-hub"))
        .env("PORT", port.to_string())
        .env("NO_COLOR", "1")
        .env_remove("RUST_LOG")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn fohmixer-hub");
    let mut server = Server { child, port, dir };
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if version_status(port).as_deref() == Some("HTTP/1.1 200 OK") {
            return server;
        }
        if let Some(status) = server.child.try_wait().unwrap() {
            panic!("the hub ended at start ({status}): {}", server.log());
        }
        assert!(
            Instant::now() < deadline,
            "the hub never answered: {}",
            server.log()
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Sends the signal `name` (TERM or INT) to `pid`: a request to stop, never
/// a force-end.
fn signal(pid: u32, name: &str) -> std::io::Result<ExitStatus> {
    Command::new("kill")
        .args(["-s", name, &pid.to_string()])
        .status()
}

fn request_stop(server: &Server, name: &str) -> Instant {
    let sent = signal(server.child.id(), name).expect("run kill");
    assert!(sent.success(), "signal {name} not sent");
    Instant::now()
}

/// The exit status within `limit` of `since`, or a panic with the log.
fn exit_within(server: &mut Server, since: Instant, limit: Duration) -> ExitStatus {
    loop {
        if let Some(status) = server.child.try_wait().unwrap() {
            return status;
        }
        assert!(
            since.elapsed() < limit,
            "still running {limit:?} after the stop request: {}",
            server.log()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn port_is_free(port: u16) -> bool {
    TcpListener::bind(("0.0.0.0", port)).is_ok()
}

/// A client that never finishes its request (a tablet that lost the network
/// mid-request): it holds the drain for its full 5 s.
fn unfinished_request(port: u16) -> TcpStream {
    let mut stuck = TcpStream::connect(("127.0.0.1", port)).unwrap();
    stuck
        .write_all(b"GET /api/version HTTP/1.1\r\nHost: 127.0.0.1\r\n")
        .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    stuck
}

/// Whether a new connection to `port` is refused within 2 s of `since`.
fn refused_within_2s(port: u16, since: Instant) -> bool {
    loop {
        if TcpStream::connect(("127.0.0.1", port)).is_err() {
            return true;
        }
        if since.elapsed() > Duration::from_secs(2) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn sigterm_stops_the_hub_with_exit_0_and_frees_the_port() {
    let mut server = start();
    let sent = request_stop(&server, "TERM");
    let status = exit_within(&mut server, sent, Duration::from_secs(6));
    let log = server.log();
    assert_eq!(status.code(), Some(0), "{log}");
    assert!(port_is_free(server.port));
    assert!(log.contains("Starting fohmixer-hub v"), "{log}");
    assert!(log.contains("SIGTERM: stopping"), "{log}");
    assert!(log.contains("HTTP server stopped"), "{log}");
    assert!(log.contains("fohmixer-hub stopped"), "{log}");
}

#[test]
fn sigint_stops_it_too() {
    let mut server = start();
    let sent = request_stop(&server, "INT");
    let status = exit_within(&mut server, sent, Duration::from_secs(6));
    let log = server.log();
    assert_eq!(status.code(), Some(0), "{log}");
    assert!(log.contains("SIGINT: stopping"), "{log}");
}

#[test]
fn an_unfinished_request_holds_the_stop_at_most_five_seconds() {
    let mut server = start();
    let stuck = unfinished_request(server.port);
    let sent = request_stop(&server, "TERM");
    // The listener closes at once: no new connection while it drains.
    assert!(
        refused_within_2s(server.port, sent),
        "the listener stayed open: {}",
        server.log()
    );
    // Up to 5 s for the open request, then the process ends on its own.
    let status = exit_within(&mut server, sent, Duration::from_secs(8));
    assert_eq!(status.code(), Some(0), "{}", server.log());
    assert!(port_is_free(server.port));
    drop(stuck);
}

#[test]
fn a_bad_port_exits_1_naming_it() {
    let output = Command::new(env!("CARGO_BIN_EXE_fohmixer-hub"))
        .env("PORT", "not-a-port")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .output()
        .expect("run fohmixer-hub");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("fohmixer-hub: PORT=not-a-port is not a port number"),
        "{stderr}"
    );
}
