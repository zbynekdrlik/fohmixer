//! The Listen devices' audio connections (#210): a TCP listener on
//! `127.0.0.1:<[listen] port>` (no config widens the address), at most
//! [`MAX_CONNECTIONS`] at once, each read by a task of its own through the
//! pure `jitnet::Reader`. A connection's first message must be the device's
//! hello; its matrices are answered with a latency chunk (never waiting on
//! the device: a full socket drops the answer, counted) and, while the hub
//! selects that device ([`Listen::accepts`]) at 48 kHz, cut into 20 ms
//! frames, encoded and sent to the listening sockets. Anything refused is
//! counted by reason (`/api/status` `listen.refused`); a chunk that breaks
//! the stream's framing ends the connection.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use bytes::Bytes;
use fohmixer_proto::listen::SAMPLE_RATE;
use fohmixer_proto::listen::frames::Framer;
use fohmixer_proto::listen::jitnet::{Chunk, LATENCY_LEN, Matrix, Reader, latency};
use fohmixer_proto::listen::wire::{Hello, parse_hello};
use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use super::Listen;
use super::encoder::Encoder;

/// The most audio connections at once. One device streams at a time, but
/// `jit.net.send` may connect at load and stay connected while idle (probe
/// P2 on #210 tells): 64 covers every listenable track of both Lives.
pub const MAX_CONNECTIONS: usize = 64;
/// A chunk begun and not finished for this long ends its connection (a
/// stuck sender, or one dripping a byte at a time, never holds a slot and
/// a buffer for good); judged on every pass, from the chunk's beginning.
pub const STALL: Duration = Duration::from_secs(5);
/// How often a quiet connection is looked at for a stall.
pub const STALL_CHECK: Duration = Duration::from_secs(1);
/// One socket read.
pub const READ: usize = 64 * 1024;

/// What a matrix does on a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Take {
    /// Framed, encoded, sent.
    Frames,
    /// Refused for the reason named.
    Refused(&'static str),
}

/// The decision for one matrix: from a device that said hello, at 48 kHz,
/// selected now.
pub fn take(hello: Option<&Hello>, selected: impl FnOnce(i32) -> bool) -> Take {
    let Some(hello) = hello else {
        return Take::Refused("no_hello");
    };
    if u32::try_from(hello.sample_rate) != Ok(SAMPLE_RATE) {
        return Take::Refused("sample_rate");
    }
    if !selected(hello.device) {
        return Take::Refused("not_selected");
    }
    Take::Frames
}

/// Whether a connection holding `buffered` bytes of an unfinished chunk,
/// which began at `since`, is stalled at `now`.
pub fn stalled(buffered: usize, since: Instant, now: Instant) -> bool {
    buffered > 0 && now.saturating_duration_since(since) >= STALL
}

/// What became of a latency answer's write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    /// Written whole.
    Sent,
    /// The socket's buffer was full: nothing written, counted.
    Dropped,
    /// Part of it written, or the socket failed: the device would read a
    /// broken stream, so the connection ends.
    Broken,
}

/// The outcome of a non-blocking write of one latency answer.
pub fn answer_of(result: &std::io::Result<usize>) -> Answer {
    match result {
        Ok(n) if *n == LATENCY_LEN => Answer::Sent,
        Ok(_) => Answer::Broken,
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Answer::Dropped,
        Err(_) => Answer::Broken,
    }
}

/// Whether a hello is news on its connection (the device sends it again
/// each second while it streams: only a change is recorded).
pub fn hello_news(old: Option<&Hello>, new: &Hello) -> bool {
    old != Some(new)
}

/// Whether one more connection fits.
pub fn admits(open: usize) -> bool {
    open < MAX_CONNECTIONS
}

/// Starts the accept loop on a bound listener (inside a tokio runtime).
pub fn spawn(
    listener: std::net::TcpListener,
    listen: Arc<Listen>,
) -> std::io::Result<JoinHandle<()>> {
    listener.set_nonblocking(true)?;
    let listener = TcpListener::from_std(listener)?;
    Ok(tokio::spawn(accept(listener, listen)))
}

async fn accept(listener: TcpListener, listen: Arc<Listen>) {
    loop {
        let (socket, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                tracing::debug!(error = %e, "listen: an accept failed");
                listen.counters.refuse("accept");
                tokio::time::sleep(super::udp::ERROR_PAUSE).await;
                continue;
            }
        };
        let open = listen.counters.connections.load(Ordering::Relaxed);
        if !admits(open) {
            listen.counters.refuse("connections");
            continue;
        }
        listen.counters.connections.fetch_add(1, Ordering::Relaxed);
        let listen = Arc::clone(&listen);
        tokio::spawn(async move {
            let why = connection(socket, &listen).await;
            listen.counters.connections.fetch_sub(1, Ordering::Relaxed);
            tracing::info!(peer = %peer, why, "listen: a device's connection ended");
        });
    }
}

/// One connection's state.
struct Conn {
    reader: Reader,
    hello: Option<Hello>,
    framer: Framer,
    /// Made at the first frame; a failure is kept, so it is logged once
    /// per connection, never per matrix.
    encoder: Option<Result<Encoder, ()>>,
    opened: Instant,
    /// When the unfinished chunk (the reader's buffered bytes) began: its
    /// first bytes' read, or the read that finished the chunk before it.
    began: Instant,
    /// Whether its last matrix was taken (a change is one record).
    taking: bool,
}

/// Reads a connection until it ends; the words of why.
async fn connection(mut socket: TcpStream, listen: &Listen) -> &'static str {
    let mut conn = Conn {
        reader: Reader::default(),
        hello: None,
        framer: Framer::default(),
        encoder: None,
        opened: Instant::now(),
        began: Instant::now(),
        taking: false,
    };
    let mut buf = vec![0u8; READ];
    loop {
        // Only an unfinished chunk needs the stall check's timer; an idle
        // connection (no bytes waiting) just waits.
        let unfinished = conn.reader.buffered() > 0;
        let read = if unfinished {
            tokio::time::timeout(STALL_CHECK, socket.read(&mut buf))
                .await
                .ok()
        } else {
            Some(socket.read(&mut buf).await)
        };
        let n = match read {
            None => 0,
            Some(Ok(0)) => return "closed",
            Some(Ok(n)) => n,
            Some(Err(_)) => return "read error",
        };
        let now = Instant::now();
        if !unfinished {
            conn.began = now;
        }
        conn.reader.push(&buf[..n]);
        for _ in 0..=n {
            match conn.reader.next_chunk() {
                Ok(Some(Chunk::Message(atoms))) => {
                    conn.began = now;
                    match parse_hello(&atoms) {
                        Some(hello) => on_hello(&mut conn, hello, listen),
                        None => listen.counters.refuse("message"),
                    }
                }
                Ok(Some(Chunk::Matrix(m))) => {
                    conn.began = now;
                    let before = conn.opened.elapsed().as_secs_f64() * 1000.0;
                    on_matrix(&mut conn, &m, listen);
                    let after = conn.opened.elapsed().as_secs_f64() * 1000.0;
                    // Never wait on the device: a full socket drops it.
                    let answer = latency(m.order, m.time, before, after);
                    match answer_of(&socket.try_write(&answer)) {
                        Answer::Sent => {}
                        Answer::Dropped => listen.counters.refuse("latency_dropped"),
                        Answer::Broken => {
                            listen.counters.refuse("latency_broken");
                            return "latency_broken";
                        }
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    listen.counters.refuse(e.reason());
                    return e.reason();
                }
            }
        }
        // Every pass: a chunk dripping a byte at a time stalls too.
        if stalled(conn.reader.buffered(), conn.began, Instant::now()) {
            listen.counters.refuse("stalled");
            return "stalled";
        }
    }
}

fn on_hello(conn: &mut Conn, hello: Hello, listen: &Listen) {
    if !hello_news(conn.hello.as_ref(), &hello) {
        return;
    }
    tracing::info!(device = hello.device, version = %hello.version, sample_rate = hello.sample_rate, "listen: a device connected");
    listen.events.record(
        "listen",
        json!({"what": "hello", "device": hello.device, "version": hello.version,
               "sample_rate": hello.sample_rate}),
    );
    conn.hello = Some(hello);
}

fn on_matrix(conn: &mut Conn, m: &Matrix, listen: &Listen) {
    let decision = take(conn.hello.as_ref(), |device| {
        listen.accepts(device, Instant::now())
    });
    let taking = decision == Take::Frames;
    if taking != conn.taking {
        conn.taking = taking;
        let device = conn.hello.as_ref().map(|h| h.device);
        listen.events.record(
            "listen",
            json!({"what": if taking { "stream_up" } else { "stream_down" }, "device": device}),
        );
    }
    if let Take::Refused(reason) = decision {
        // A device no longer selected starts its next stream afresh.
        conn.framer = Framer::default();
        listen.counters.refuse(reason);
        return;
    }
    for frame in conn.framer.push(m.planes, &m.samples) {
        listen.counters.frames.fetch_add(1, Ordering::Relaxed);
        let made = conn.encoder.get_or_insert_with(|| {
            Encoder::new().map_err(|e| {
                tracing::error!(error = %e, "listen: the Opus encoder failed to start");
            })
        });
        let Ok(encoder) = made.as_mut() else {
            listen.counters.refuse("encoder");
            return;
        };
        match encoder.encode(&frame) {
            Ok(packet) => listen.send_packet(Bytes::from(packet)),
            Err(_) => listen.counters.refuse("encode"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello(rate: i32) -> Hello {
        Hello {
            device: 3,
            version: "v1.50.0".into(),
            sample_rate: rate,
        }
    }

    #[test]
    fn a_matrix_is_taken_only_from_a_selected_device_at_48_khz() {
        assert_eq!(take(None, |_| true), Take::Refused("no_hello"));
        assert_eq!(
            take(Some(&hello(44_100)), |_| true),
            Take::Refused("sample_rate")
        );
        assert_eq!(
            take(Some(&hello(-1)), |_| true),
            Take::Refused("sample_rate")
        );
        assert_eq!(
            take(Some(&hello(48_000)), |d| d == 4),
            Take::Refused("not_selected")
        );
        assert_eq!(take(Some(&hello(48_000)), |d| d == 3), Take::Frames);
    }

    #[test]
    fn at_most_64_connections() {
        assert_eq!(MAX_CONNECTIONS, 64);
        assert!(admits(63));
        assert!(!admits(64));
        assert_eq!(READ, 65_536);
    }

    #[test]
    fn an_unfinished_chunk_stalls_after_five_seconds() {
        let t0 = Instant::now();
        let s = Duration::from_millis;
        assert!(!stalled(10, t0, t0 + s(4999)));
        assert!(stalled(10, t0, t0 + s(5000)));
        assert!(
            !stalled(0, t0, t0 + s(60_000)),
            "an idle connection is no stall"
        );
        assert!(!stalled(10, t0 + s(10), t0), "a clock that went back");
        assert_eq!((STALL, STALL_CHECK), (s(5000), s(1000)));
    }

    #[test]
    fn a_latency_answer_written_in_part_breaks_the_stream() {
        use std::io::{Error, ErrorKind};
        assert_eq!(answer_of(&Ok(LATENCY_LEN)), Answer::Sent);
        assert_eq!(answer_of(&Ok(LATENCY_LEN - 1)), Answer::Broken);
        assert_eq!(answer_of(&Ok(0)), Answer::Broken);
        assert_eq!(
            answer_of(&Err(Error::from(ErrorKind::WouldBlock))),
            Answer::Dropped
        );
        assert_eq!(
            answer_of(&Err(Error::from(ErrorKind::ConnectionReset))),
            Answer::Broken
        );
    }

    #[test]
    fn only_a_changed_hello_is_news() {
        let h = hello(48_000);
        assert!(hello_news(None, &h));
        assert!(!hello_news(Some(&h), &h));
        assert!(hello_news(Some(&hello(44_100)), &h));
    }
}
