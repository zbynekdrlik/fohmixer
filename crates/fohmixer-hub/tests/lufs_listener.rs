//! The LUFS devices' listener end to end (#151, spec §4.4): bound to
//! 127.0.0.1 only, a good datagram taken, garbage counted by its reason, a
//! flood held to the key's bound, port 0 off, a taken port an error the hub
//! runs on with. Host-free (no SimLive): it also runs in the `windows` job.

mod support;

use std::net::UdpSocket;
use std::path::Path;
use std::time::{Duration, Instant};

use fohmixer_hub::config::Config;
use fohmixer_proto::client::HubStatus;
use fohmixer_proto::lufs::wire::{DeviceKind, MAX_DATAGRAM, Reading, encode};
use support::{TestHub, runtime, serial};

const SECS_5: Duration = Duration::from_secs(5);

/// A UDP port free now on 127.0.0.1 (the hub binds it right after).
fn free_port() -> u16 {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.local_addr().unwrap().port()
}

fn config(dir: &Path, port: u16) -> Config {
    let mut config = Config::defaults(dir);
    config.instances.clear();
    config.lufs.port = port;
    config
}

fn datagram(device: i32) -> Vec<u8> {
    encode(&Reading {
        kind: DeviceKind::Lufs,
        device,
        version: "v1.19.0".to_string(),
        parents: ["Vox 1", "FOH Band", "-", "-"].map(str::to_string),
        first: -300,
        second: -253,
    })
}

fn sender() -> UdpSocket {
    UdpSocket::bind("127.0.0.1:0").unwrap()
}

#[test]
fn listens_on_localhost_only_and_takes_a_good_datagram() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let port = free_port();
        let hub = TestHub::start_config(config(dir.path(), port)).await;
        let status = hub.status().await;
        assert_eq!(status.lufs.listener.addr, Some(format!("127.0.0.1:{port}")));
        assert_eq!(status.lufs.listener.error, None);
        sender().send_to(&datagram(1), ("127.0.0.1", port)).unwrap();
        // Taken, and the router's rule keeps the key (unmatched: no Live)
        // until it goes stale a second later.
        let status = hub
            .status_until(SECS_5, |s| s.lufs.devices.len() == 1)
            .await;
        assert_eq!(status.lufs.listener.taken, 1);
        assert_eq!(status.lufs.devices[0].device, 1);
        assert_eq!(status.lufs.devices[0].kind, "lufs");
        let records = hub
            .events_until(SECS_5, |r| {
                r.iter()
                    .any(|e| e["ev"] == "lufs" && e["what"] == "listening")
                    && r.iter().any(|e| e["ev"] == "lufs" && e["what"] == "found")
            })
            .await;
        let listening = records
            .iter()
            .find(|e| e["ev"] == "lufs" && e["what"] == "listening")
            .unwrap();
        assert_eq!(listening["addr"], format!("127.0.0.1:{port}"));
        hub.stop().await;
    });
}

#[test]
fn garbage_is_counted_by_reason() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let port = free_port();
        let hub = TestHub::start_config(config(dir.path(), port)).await;
        let socket = sender();
        let mut long = datagram(1);
        long.resize(MAX_DATAGRAM + 1, 0);
        let other = {
            let mut d = datagram(1);
            d[10] = b'x';
            d
        };
        let mut tags = datagram(1);
        let at = tags.windows(10).position(|w| w == b",iisssssii").unwrap();
        tags[at + 8] = b'f';
        for d in [&long, &other, &tags] {
            socket.send_to(d, ("127.0.0.1", port)).unwrap();
        }
        let status = hub
            .status_until(SECS_5, |s: &HubStatus| {
                s.lufs.listener.refused.values().sum::<u64>() == 3
            })
            .await;
        let refused = &status.lufs.listener.refused;
        assert_eq!(refused.get("too_long"), Some(&1), "{refused:?}");
        assert_eq!(refused.get("address"), Some(&1), "{refused:?}");
        assert_eq!(refused.get("tags"), Some(&1), "{refused:?}");
        assert_eq!(status.lufs.listener.taken, 0);
        assert!(status.lufs.devices.is_empty());
        hub.stop().await;
    });
}

#[test]
fn a_flood_is_held_to_its_keys_bound_and_another_key_goes_through() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let port = free_port();
        let hub = TestHub::start_config(config(dir.path(), port)).await;
        let socket = sender();
        // 200 from one key within about 0.9 s, 4 from another among them.
        let started = Instant::now();
        for i in 0..200u32 {
            socket.send_to(&datagram(1), ("127.0.0.1", port)).unwrap();
            if i % 50 == 0 {
                socket.send_to(&datagram(2), ("127.0.0.1", port)).unwrap();
            }
            tokio::time::sleep(Duration::from_millis(4)).await;
        }
        let elapsed = started.elapsed();
        let status = hub
            .status_until(SECS_5, |s| {
                let l = &s.lufs.listener;
                l.taken + l.dropped == 204
            })
            .await;
        let listener = &status.lufs.listener;
        // One window per started second of the flood (its last may be a
        // new one), at most 10 each, plus the other key's 4.
        let windows = elapsed.as_secs() + 1;
        assert!(
            listener.taken <= 10 * windows + 4,
            "{listener:?} in {elapsed:?}"
        );
        assert!(listener.taken >= 14, "{listener:?}");
        assert!(listener.dropped >= 200 - 10 * windows, "{listener:?}");
        assert_eq!(listener.full, 0);
        let records = hub
            .events_until(SECS_5, |r| {
                r.iter()
                    .any(|e| e["ev"] == "lufs" && e["what"] == "dropping")
            })
            .await;
        let dropping: Vec<_> = records
            .iter()
            .filter(|e| e["ev"] == "lufs" && e["what"] == "dropping")
            .collect();
        assert_eq!(dropping.len(), 1, "once per key: {dropping:?}");
        assert_eq!(dropping[0]["device"], 1);
        hub.stop().await;
    });
}

#[test]
fn port_zero_listens_nowhere() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = TestHub::start_config(config(dir.path(), 0)).await;
        let status = hub.status().await;
        assert_eq!(status.lufs.listener.addr, None);
        assert_eq!(status.lufs.listener.error, None);
        hub.stop().await;
    });
}

#[test]
fn a_taken_port_is_an_error_and_the_hub_runs() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let taken = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = taken.local_addr().unwrap().port();
        let hub = TestHub::start_config(config(dir.path(), port)).await;
        let status = hub.status().await;
        assert_eq!(status.lufs.listener.addr, None);
        assert!(status.lufs.listener.error.is_some(), "{status:?}");
        let (code, _) = hub.get("/api/version").await;
        assert_eq!(code, 200);
        let records = hub
            .events_until(SECS_5, |r| {
                r.iter()
                    .any(|e| e["ev"] == "lufs" && e["what"] == "listening")
            })
            .await;
        let failed = records
            .iter()
            .find(|e| e["ev"] == "lufs" && e["what"] == "listening")
            .unwrap();
        assert!(failed["error"].is_string(), "{failed}");
        drop(taken);
        hub.stop().await;
    });
}
