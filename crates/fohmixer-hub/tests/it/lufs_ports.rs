//! Polled devices on one poll port (#264, the design comment 7228766032):
//! about 50 devices draw a poll port from 40000-49999 at their load, and
//! Max's `udpreceive` binds with `SO_REUSEADDR`, so two of them can hold
//! one port. On the FOH PC Windows gives such a port's datagrams to its
//! first binder only: the device that loaded later is deaf. SimLive's
//! polled stand-ins (`sim/maxdev.py` `PollPorts`) play that delivery, and
//! the hub moves the device that still hears, so the deaf one hears.
#![cfg(unix)]

use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::support::{Host, TestHub, repo, runtime, serial};
use fohmixer_hub::config::Config;
use serde_json::{Value, json};

const SECS_5: Duration = Duration::from_secs(5);

fn free_port() -> u16 {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.local_addr().unwrap().port()
}

/// A poll port of the devices' range that nothing holds now.
fn free_poll_port() -> u16 {
    let seed = u16::try_from(std::process::id() % 10_000).unwrap();
    (0..10_000u16)
        .map(|n| 40_000 + (seed + n * 7) % 10_000)
        .find(|port| UdpSocket::bind(("127.0.0.1", *port)).is_ok())
        .expect("a free poll port")
}

fn site() -> PathBuf {
    repo().join("sim").join("fixtures").join("test-site.json")
}

async fn hub(dir: &Path, host: &Host, port: u16) -> TestHub {
    let frame = json!({"schema": 2, "default_page": "p", "pages": [{"id": "p", "title": "P",
        "rows": [{"sections": [{"kind": "group", "id": "lufs", "title": "LUFS", "tags": "LUFS"}]}]}]});
    std::fs::write(dir.join("layout.json"), frame.to_string()).unwrap();
    let mut config = Config::defaults(dir);
    config.instances = vec![host.cfg()];
    config.layout_poll_ms = 100;
    config.lufs.port = port;
    config.lufs.sweep = None;
    let hub = TestHub::start_config(config).await;
    hub.status_until(SECS_5, |s| s.instances.iter().all(|i| i.online))
        .await;
    hub
}

/// Waits until the host's answer to `line` passes `pred` (5 s).
async fn host_until(host: &mut Host, line: &str, pred: impl Fn(i64) -> bool) -> i64 {
    let end = Instant::now() + SECS_5;
    loop {
        let answer = host.maxdev(line);
        if pred(answer) {
            return answer;
        }
        assert!(Instant::now() < end, "`{line}` still answers {answer}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The `lufs` records of `what`, once at least one is written.
async fn records(hub: &TestHub, what: &str) -> Vec<Value> {
    let of = |r: &[Value]| -> Vec<Value> {
        r.iter()
            .filter(|e| e["ev"] == "lufs" && e["what"] == what)
            .cloned()
            .collect()
    };
    of(&hub.events_until(SECS_5, |r| !of(r).is_empty()).await)
}

#[test]
fn a_deaf_device_on_a_shared_port_is_heard_once_the_hub_moves_the_one_that_hears() {
    let _serial = serial();
    runtime().block_on(async {
        let lufs_port = free_port();
        let mut host = Host::start_lufs("band", &site(), lufs_port);
        let dir = tempfile::tempdir().unwrap();
        let hub = hub(dir.path(), &host, lufs_port).await;
        let port = free_poll_port();
        let shared = i64::from(port);
        assert_eq!(
            host.maxdev(&format!("maxdev meter track 1 add rack polled {port}")),
            1
        );
        host_until(&mut host, "polled meter track 1", |n| n > 0).await;
        // A LUFS device of a set loaded later draws the same port: deaf.
        assert_eq!(
            host.maxdev(&format!("maxdev lufs track 2 add polled {port}")),
            1
        );
        // The hub moves the meter (the first binder) and polls on: the
        // LUFS device hears on the port it drew.
        host_until(&mut host, "polled lufs track 2", |n| n > 0).await;
        let moved_to = host.maxdev("pollport meter track 1");
        assert_ne!(moved_to, shared, "the meter took another port");
        assert_eq!(host.maxdev("pollport lufs track 2"), shared);
        let share = records(&hub, "shared_port").await;
        assert_eq!(share.len(), 1, "{share:?}");
        assert_eq!(share[0]["port"], json!(port));
        assert_eq!(share[0]["moved"]["kind"], "meter");
        assert_eq!(share[0]["stays"]["kind"], "lufs");
        assert_eq!(share[0]["move"], "/fohmixer/meter/move");
        assert_eq!(share[0]["sent"], true);
        let moved = records(&hub, "moved").await;
        assert_eq!(moved[0]["kind"], "meter");
        assert_eq!(moved[0]["from"], json!(port));
        assert_eq!(moved[0]["to"], json!(moved_to));
        // Both answer from then on, each on its own port.
        let meter = host.maxdev("polled meter track 1");
        let lufs = host.maxdev("polled lufs track 2");
        host_until(&mut host, "polled meter track 1", |n| n > meter + 10).await;
        host_until(&mut host, "polled lufs track 2", |n| n > lufs + 2).await;
        let status = hub.status().await;
        assert_eq!(
            status.lufs.listener.polling, 2,
            "{:?}",
            status.lufs.listener
        );
    });
}
