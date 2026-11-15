//! The Listen devices' announces and the hub's selects (#210): one UDP
//! socket on `127.0.0.1:<[listen] port>` (no config widens the address).
//! Each announce is parsed (`fohmixer_proto::listen::wire::parse_announce`,
//! pure) and kept; a refused one is only counted (`/api/status`
//! `listen.refused`, `announce_<reason>`). The same task sends the selects
//! to `127.0.0.1:<[listen] select_port>` every
//! [`SELECT_EVERY_MS`](fohmixer_proto::listen::wire::SELECT_EVERY_MS), and
//! at once when a target starts or ends ([`Listen::wake`]).

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::{Duration, Instant};

use fohmixer_proto::listen::wire::{MAX_DATAGRAM, SELECT_EVERY_MS, encode_select, parse_announce};
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

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
pub fn spawn(
    socket: std::net::UdpSocket,
    select_port: u16,
    listen: Arc<Listen>,
) -> std::io::Result<JoinHandle<()>> {
    socket.set_nonblocking(true)?;
    let socket = UdpSocket::from_std(socket)?;
    let to = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, select_port));
    Ok(tokio::spawn(run(socket, to, listen)))
}

async fn run(socket: UdpSocket, to: SocketAddr, listen: Arc<Listen>) {
    let mut buf = vec![0u8; MAX_DATAGRAM + 1];
    let mut tick = tokio::time::interval(Duration::from_millis(SELECT_EVERY_MS));
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            got = socket.recv(&mut buf) => match got {
                Ok(n) => handle_datagram(&listen, &buf[..n], Instant::now()),
                Err(e) => {
                    // An oversized datagram or an ICMP reset of a select:
                    // counted, never fatal.
                    tracing::debug!(error = %e, "listen: a receive failed");
                    listen.counters.refuse("recv");
                    tokio::time::sleep(ERROR_PAUSE).await;
                }
            },
            _ = tick.tick() => send_select(&socket, to, &listen).await,
            () = listen.wake.notified() => send_select(&socket, to, &listen).await,
        }
    }
}

async fn send_select(socket: &UdpSocket, to: SocketAddr, listen: &Listen) {
    let Some(select) = listen.next_select(Instant::now()) else {
        return;
    };
    if let Err(e) = socket.send_to(&encode_select(select), to).await {
        tracing::debug!(error = %e, "listen: a select failed");
        listen.counters.refuse("select_send");
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
    use fohmixer_proto::listen::wire::{Announce, encode_announce, parents_in, parse_select};

    fn announce(device: i32) -> Vec<u8> {
        encode_announce(&Announce {
            device,
            version: "v1.50.0".into(),
            parents: parents_in(None, "Vox 1", "FOH Band"),
            sample_rate: 48_000,
            sending: false,
        })
    }

    #[test]
    fn an_announce_is_kept_and_junk_counted() {
        let (events, _records) = EventLog::channel(16);
        let listen = Listen::new(events);
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
    async fn a_start_selects_the_device_at_once_and_the_end_stops_it() {
        let (events, _records) = EventLog::channel(16);
        let listen = Listen::new(events);
        let devices = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        devices
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let select_port = devices.local_addr().unwrap().port();
        let hub = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let hub_addr = hub.local_addr().unwrap();
        let task = spawn(hub, select_port, Arc::clone(&listen)).unwrap();
        devices.send_to(&announce(7), hub_addr).unwrap();
        // The announce is in once the status shows it.
        for _ in 0..500 {
            if !listen.status(Instant::now()).devices.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let lease = listen.start("band", "FOH Band", "Vox 1").unwrap();
        let mut buf = [0u8; 64];
        let n = tokio::task::spawn_blocking({
            let devices = devices.try_clone().unwrap();
            move || devices.recv(&mut buf).map(|n| (n, buf))
        })
        .await
        .unwrap()
        .unwrap();
        let select = parse_select(&n.1[..n.0]).unwrap();
        assert_eq!((select.device, select.turn), (7, 1));
        drop(lease);
        // The next select after the end names no device.
        let stop = tokio::task::spawn_blocking(move || {
            for _ in 0..10 {
                let mut buf = [0u8; 64];
                let n = devices.recv(&mut buf).unwrap();
                let s = parse_select(&buf[..n]).unwrap();
                if s.device == 0 {
                    return s;
                }
            }
            panic!("no stop select");
        })
        .await
        .unwrap();
        assert_eq!(stop.turn, 1);
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
