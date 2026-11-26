//! The Listen devices' announces (#210): one UDP socket on
//! `127.0.0.1:<[listen] port>` (no config widens the address). Each
//! announce is parsed (`fohmixer_proto::listen::wire::parse_announce`,
//! pure) and kept; a refused one is only counted (`/api/status`
//! `listen.refused`, `announce_<reason>`). An announce answers the hub's
//! poll (#264, `listen/poll.rs`, from a socket of its own): this socket
//! only reads.

use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fohmixer_proto::listen::wire::{MAX_DATAGRAM, parse_announce};
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

use super::Listen;

/// The only address Listen binds: `127.0.0.1:<port>`.
pub fn bind_addr(port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)
}

/// The pause after a receive error (never a spinning task).
pub const ERROR_PAUSE: Duration = Duration::from_millis(10);

/// The counter's word of a refused announce.
pub fn announce_reason(e: fohmixer_proto::lufs::wire::WireError) -> String {
    format!("announce_{}", e.reason())
}

/// One datagram's way: kept when it is an announce, else counted.
pub fn handle_datagram(listen: &Listen, data: &[u8], now: Instant) {
    match parse_announce(data) {
        Ok(announce) => listen.heard(announce, now),
        Err(e) => listen.counters.refuse(&announce_reason(e)),
    }
}

/// Starts the task on a socket bound to `127.0.0.1:<port>` (inside a tokio
/// runtime).
pub fn spawn(socket: std::net::UdpSocket, listen: Arc<Listen>) -> std::io::Result<JoinHandle<()>> {
    socket.set_nonblocking(true)?;
    let socket = UdpSocket::from_std(socket)?;
    Ok(tokio::spawn(run(socket, listen)))
}

async fn run(socket: UdpSocket, listen: Arc<Listen>) {
    let mut buf = vec![0u8; MAX_DATAGRAM + 1];
    loop {
        match socket.recv(&mut buf).await {
            Ok(n) => handle_datagram(&listen, &buf[..n], Instant::now()),
            Err(e) => {
                // An oversized datagram: counted, never fatal.
                tracing::debug!(error = %e, "listen: a receive failed");
                listen.counters.refuse("recv");
                tokio::time::sleep(ERROR_PAUSE).await;
            }
        }
    }
}

/// The record of the listener bound or not.
pub fn listen_fields(addr: &str, error: Option<&str>) -> serde_json::Value {
    match error {
        None => serde_json::json!({"what": "listening", "addr": addr}),
        Some(error) => serde_json::json!({"what": "listening", "addr": addr, "error": error}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventLog;
    use fohmixer_proto::listen::wire::{Announce, encode_announce, parents_in};

    fn announce(device: i32) -> Vec<u8> {
        encode_announce(&Announce {
            device,
            version: "v1.50.0".into(),
            parents: parents_in(None, "Vox 1", "FOH Band"),
            sample_rate: 48_000,
            sending: false,
            port: 40_000,
        })
    }

    #[test]
    fn an_announce_is_kept_and_junk_counted() {
        let (events, _records) = EventLog::channel(16);
        let listen = Listen::new(events, super::super::no_sets());
        let now = Instant::now();
        handle_datagram(&listen, &announce(4), now);
        handle_datagram(&listen, b"junk", now);
        let status = listen.status(now);
        assert_eq!(status.devices.len(), 1);
        assert_eq!(status.devices[0].device, 4);
        assert_eq!(status.devices[0].track, "Vox 1");
        assert_eq!(status.refused.get("announce_truncated"), Some(&1));
    }

    #[tokio::test]
    async fn the_task_keeps_the_announces_it_receives() {
        let (events, _records) = EventLog::channel(16);
        let listen = Listen::new(events, super::super::no_sets());
        let hub = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let hub_addr = hub.local_addr().unwrap();
        let task = spawn(hub, Arc::clone(&listen)).unwrap();
        let device = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        device.send_to(&announce(7), hub_addr).unwrap();
        let mut kept = false;
        for _ in 0..500 {
            if listen.status(Instant::now()).devices.len() == 1 {
                kept = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(kept, "the announce is kept");
        task.abort();
    }

    #[test]
    fn the_records_fields() {
        assert_eq!(
            listen_fields("127.0.0.1:39111", None),
            serde_json::json!({"what": "listening", "addr": "127.0.0.1:39111"})
        );
        assert_eq!(listen_fields("x", Some("in use"))["error"], "in use");
        assert_eq!(bind_addr(39111).to_string(), "127.0.0.1:39111");
        assert_eq!(ERROR_PAUSE, Duration::from_millis(10));
    }
}
