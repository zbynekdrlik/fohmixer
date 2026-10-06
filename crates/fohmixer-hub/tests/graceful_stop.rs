//! The hub binary stops gracefully (copied from iemmixer's
//! `iem-server/tests/graceful_stop.rs` @ 22372bc and adapted): SIGTERM or
//! SIGINT closes the listener at once, open requests get up to 5 s, the
//! process exits 0 and the port is free; with `[companion]` the Stream Deck
//! leaves Companion (`REMOVE-DEVICE`, #52) before the process ends. The
//! in-process `serve_until` tests are in `tests/serve_until.rs`.
//!
//! Unix only: the tests send SIGTERM/SIGINT. The Windows stop (Ctrl-Break to
//! the hub's console) is exercised on the PC from S3 on.
//!
//! One test at a time (`SERIAL` here, the `hub-ports` test group under
//! nextest): the tests check that a stopped hub's port is refused and free,
//! and the kernel readily hands a just-freed port to the next bind to port
//! 0, so a hub another test starts at that moment could take it.
#![cfg(unix)]

use std::io::{BufRead, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

static SERIAL: Mutex<()> = Mutex::new(());

/// This test's turn; a panicked earlier test does not block the rest.
fn serial() -> MutexGuard<'static, ()> {
    SERIAL
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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

/// The port the hub reports in its "HTTP server listening" line
/// (`addr=0.0.0.0:<port>`), once it has logged it.
fn listening_port(log: &str) -> Option<u16> {
    // Whole lines only: the hub may be half-way through writing the last one.
    let line = log
        .split_inclusive('\n')
        .filter(|l| l.ends_with('\n'))
        .find(|l| l.contains("HTTP server listening"))?;
    let addr = line
        .split_whitespace()
        .find_map(|w| w.strip_prefix("addr="))?;
    addr.parse::<SocketAddr>().ok().map(|a| a.port())
}

/// One `GET /api/version` on its own connection; the status line.
fn version_status(port: u16) -> Option<String> {
    status_line(port, "/api/version")
}

/// One `GET <path>` on its own connection; the status line.
fn status_line(port: u16, path: &str) -> Option<String> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    s.write_all(request.as_bytes()).ok()?;
    let mut text = String::new();
    s.read_to_string(&mut text).ok()?;
    text.lines().next().map(str::to_string)
}

/// The hub binary on a port of its own (`PORT=0`: the kernel picks one, the
/// hub logs it), once it answers `/api/version`. No probe-then-release port:
/// parallel tests would be handed the same just-freed port.
fn start() -> Server {
    start_with(&[])
}

/// [`start`] with the environment `envs` on top.
fn start_with(envs: &[(&str, &str)]) -> Server {
    start_toml("instances = []\n", envs)
}

/// [`start_with`] with `toml` as the hub's config file.
fn start_toml(toml: &str, envs: &[(&str, &str)]) -> Server {
    let mut server = spawn_hub(toml, envs);
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if server.port == 0
            && let Some(port) = listening_port(&server.log())
        {
            server.port = port;
        }
        if server.port != 0 && version_status(server.port).as_deref() == Some("HTTP/1.1 200 OK") {
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

/// The hub binary with `PORT=0`, its data folder a temporary one with
/// `toml` as its config file, no inherited `RUST_LOG` and the environment
/// `envs` on top, its output in the server's log; not waited for.
fn spawn_hub(toml: &str, envs: &[(&str, &str)]) -> Server {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("fohmixer-hub.toml"), toml).unwrap();
    let log = std::fs::File::create(dir.path().join("hub.log")).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_fohmixer-hub"));
    command
        .env("PORT", "0")
        .env("FOHMIXER_DATA", dir.path())
        .env("NO_COLOR", "1")
        .env_remove("RUST_LOG");
    for (name, value) in envs {
        command.env(name, value);
    }
    let child = command
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn fohmixer-hub");
    Server {
        child,
        port: 0,
        dir,
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
    let _serial = serial();
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
    let _serial = serial();
    let mut server = start();
    let sent = request_stop(&server, "INT");
    let status = exit_within(&mut server, sent, Duration::from_secs(6));
    let log = server.log();
    assert_eq!(status.code(), Some(0), "{log}");
    assert!(log.contains("SIGINT: stopping"), "{log}");
}

#[test]
fn an_unfinished_request_holds_the_stop_at_most_five_seconds() {
    let _serial = serial();
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
    let took = sent.elapsed();
    assert_eq!(status.code(), Some(0), "{}", server.log());
    // The open request got its drain: the stop did not cut it at once.
    assert!(
        took >= Duration::from_secs(4),
        "stopped after {took:?}: the open request got no drain: {}",
        server.log()
    );
    assert!(port_is_free(server.port));
    drop(stuck);
}

#[test]
fn a_bad_port_exits_1_naming_it() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_fohmixer-hub"))
        .env("PORT", "not-a-port")
        .env("FOHMIXER_DATA", dir.path())
        .env("NO_COLOR", "1")
        .env_remove("RUST_LOG")
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

#[test]
fn a_bad_config_exits_1_naming_it() {
    let _serial = serial();
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("fohmixer-hub.toml"), "http_port = \"x\"\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_fohmixer-hub"))
        .env("FOHMIXER_DATA", dir.path())
        .env("NO_COLOR", "1")
        .env_remove("RUST_LOG")
        .env_remove("PORT")
        .stdin(Stdio::null())
        .output()
        .expect("run fohmixer-hub");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("fohmixer-hub: config "), "{stderr}");
    assert!(stderr.contains("fohmixer-hub.toml"), "{stderr}");
}

#[test]
fn an_unknown_command_prints_the_usage_with_exit_2() {
    let _serial = serial();
    let output = Command::new(env!("CARGO_BIN_EXE_fohmixer-hub"))
        .arg("pin")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .output()
        .expect("run fohmixer-hub");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage: fohmixer-hub"));
}

#[test]
fn rust_log_raises_the_hub_s_own_level() {
    let _serial = serial();
    let mut server = start_with(&[("RUST_LOG", "fohmixer_hub=debug")]);
    // A missing file is logged at debug level only.
    assert_eq!(
        status_line(server.port, "/missing.js").as_deref(),
        Some("HTTP/1.1 404 Not Found")
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while !server.log().contains("static file not found") {
        assert!(
            Instant::now() < deadline,
            "RUST_LOG=fohmixer_hub=debug did not enable the hub's debug lines: {}",
            server.log()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let sent = request_stop(&server, "TERM");
    let status = exit_within(&mut server, sent, Duration::from_secs(6));
    assert_eq!(status.code(), Some(0), "{}", server.log());
}

#[test]
fn a_bad_rust_log_exits_1_naming_it() {
    let _serial = serial();
    let mut server = spawn_hub("instances = []\n", &[("RUST_LOG", "fohmixer_hub=loud")]);
    // It must end by itself; a hub still serving after 10 s fails the test
    // (and is asked to stop by Server's drop).
    let status = exit_within(&mut server, Instant::now(), Duration::from_secs(10));
    let log = server.log();
    assert_eq!(status.code(), Some(1), "{log}");
    assert!(
        log.contains("fohmixer-hub: RUST_LOG=fohmixer_hub=loud is not a valid log filter"),
        "{log}"
    );
}

/// The binary's stop removes the Stream Deck from Companion (#52): the
/// stop waits (bounded) for the Companion task's `REMOVE-DEVICE` before the
/// process ends.
#[test]
fn sigterm_removes_the_stream_deck_from_companion() {
    let _turn = serial();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    // A fake Companion: BEGIN, ADD-DEVICE OK, PONG; every line it got, until
    // the hub closes the connection.
    let fake = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .unwrap();
        stream
            .write_all(b"BEGIN CompanionVersion=\"5.0.7+fake\" ApiVersion=\"1.12.0\" \n")
            .unwrap();
        let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
        let mut got = Vec::new();
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => return got,
                Ok(_) => {}
            }
            let line = line.trim_end().to_string();
            if line.starts_with("ADD-DEVICE") {
                stream
                    .write_all(b"ADD-DEVICE OK DEVICEID=\"fohmixer-1\" \n")
                    .unwrap();
            }
            if let Some(n) = line.strip_prefix("PING ") {
                stream.write_all(format!("PONG {n} \n").as_bytes()).unwrap();
            }
            got.push(line);
        }
    });
    let toml = format!("instances = []\n[companion]\nhost = \"127.0.0.1\"\nport = {port}\n");
    let mut server = start_toml(&toml, &[]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !server
        .log()
        .contains("Stream Deck registered with Companion")
    {
        assert!(
            Instant::now() < deadline,
            "never registered: {}",
            server.log()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let since = request_stop(&server, "TERM");
    let status = exit_within(&mut server, since, Duration::from_secs(10));
    assert!(status.success(), "{status}: {}", server.log());
    let got = fake.join().unwrap();
    assert_eq!(
        got.last().map(String::as_str),
        Some("REMOVE-DEVICE DEVICEID=\"fohmixer-1\""),
        "{got:?}"
    );
}
