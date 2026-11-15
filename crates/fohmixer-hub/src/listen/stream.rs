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
use std::time::Instant;

use bytes::Bytes;
use fohmixer_proto::listen::SAMPLE_RATE;
use fohmixer_proto::listen::frames::Framer;
use fohmixer_proto::listen::jitnet::{Chunk, Matrix, Reader, latency};
use fohmixer_proto::listen::wire::{Hello, parse_hello};
use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use super::Listen;
use super::encoder::Encoder;

/// The most audio connections at once (one device streams at a time; a
/// few more cover a device's reconnect and a stray one).
pub const MAX_CONNECTIONS: usize = 8;
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
    encoder: Option<Encoder>,
    opened: Instant,
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
        taking: false,
    };
    let mut buf = vec![0u8; READ];
    loop {
        let n = match socket.read(&mut buf).await {
            Ok(0) => return "closed",
            Ok(n) => n,
            Err(_) => return "read error",
        };
        conn.reader.push(&buf[..n]);
        for _ in 0..=n {
            match conn.reader.next_chunk() {
                Ok(Some(Chunk::Message(atoms))) => match parse_hello(&atoms) {
                    Some(hello) => on_hello(&mut conn, hello, listen),
                    None => listen.counters.refuse("message"),
                },
                Ok(Some(Chunk::Matrix(m))) => {
                    let before = conn.opened.elapsed().as_secs_f64() * 1000.0;
                    on_matrix(&mut conn, &m, listen);
                    let after = conn.opened.elapsed().as_secs_f64() * 1000.0;
                    // Never wait on the device: a full socket drops it.
                    let answer = latency(m.order, m.time, before, after);
                    if socket.try_write(&answer).is_err() {
                        listen.counters.refuse("latency_dropped");
                    }
                }
                Ok(None) => break,
                Err(e) => {
                    listen.counters.refuse(e.reason());
                    return e.reason();
                }
            }
        }
    }
}

fn on_hello(conn: &mut Conn, hello: Hello, listen: &Listen) {
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
        if conn.encoder.is_none() {
            match Encoder::new() {
                Ok(encoder) => conn.encoder = Some(encoder),
                Err(e) => {
                    tracing::error!(error = %e, "listen: the Opus encoder failed to start");
                    listen.counters.refuse("encoder");
                    return;
                }
            }
        }
        let Some(encoder) = conn.encoder.as_mut() else {
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
    fn at_most_eight_connections() {
        assert_eq!(MAX_CONNECTIONS, 8);
        assert!(admits(7));
        assert!(!admits(8));
        assert_eq!(READ, 65_536);
    }
}
