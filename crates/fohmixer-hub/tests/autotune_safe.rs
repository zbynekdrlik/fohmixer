//! The fall back to DRY end to end (#140 PR 3a, spec §8) on SimLive: the
//! autotune keys' set (two stream tracks, each a `Latencies` rack inside
//! `Stream FX`, `+AUTOTUNE`: on 127, off 0; a PA track's rack on top,
//! `+AUTOTUNE:0`: on 0, off 127; invented names). The hub finds the three
//! targets and registers each selector's DRY write with the script, and
//! every selector reads DRY within 2 s through a raw `get` of the host's
//! script, the script's log saying so once, when the hub is gone:
//!
//! - **frozen** (PR #146 review M1): the hub binary stopped by `SIGSTOP`,
//!   a hung hub whose TCP stays open; its pings stop, and the script's
//!   silence check falls back (before, its pings kept it heard);
//! - **stopped**: the in-process hub's stop, its tasks ended and its
//!   sockets closed, as a killed process's are.
#![cfg(unix)]

mod support;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use support::{Host, TestHub, events_in, runtime, serial};
use tokio_tungstenite::tungstenite::Message;

const SECS_10: Duration = Duration::from_secs(10);

/// The racks' first macros: Stream A's, Stream B's, Vocal A's.
const MACROS: [&str; 3] = [
    "live_set tracks 0 devices 0 chains 0 devices 0 parameters 1",
    "live_set tracks 1 devices 0 chains 0 devices 0 parameters 1",
    "live_set tracks 2 devices 0 parameters 1",
];
/// Their on values (the stream racks 127, the PA rack 0) and DRY values.
const ON: [f64; 3] = [127.0, 127.0, 0.0];
const DRY: [f64; 3] = [0.0, 0.0, 127.0];

/// The test's own set, as `tests/autotune_keys.rs`'s: no songs.
fn site(dir: &Path) -> PathBuf {
    let tuner = |name: &str| json!({"template": "plain", "name": name, "class_name": "Tuner"});
    let slot = |n: u32| json!({"name": format!("{n}:(unused):(unassigned)"), "value": 0.0});
    let gridder = json!({"template": "plugin", "name": "AudioGridder", "product": "AudioGridder",
                         "parameters": [slot(0), slot(1), slot(2)]});
    let latencies = |plugin: Value| {
        json!({"template": "rack", "name": "Latencies",
               "chains": [{"name": "Auto", "devices": [plugin]}, {"name": "Dry"}]})
    };
    let stream = |name: &str| {
        json!({"name": name, "devices": [
            {"template": "rack", "name": "Stream FX",
             "chains": [{"name": "Chain 1", "devices": [latencies(gridder.clone())]}]},
            tuner("+AUTOTUNE")]})
    };
    let auto_tune = json!({"template": "plugin", "name": "Auto-Tune Artist",
        "product": "Auto-Tune Artist",
        "parameters": [{"name": "Key", "value": 0.0}, {"name": "Scale", "value": 0.0}]});
    let set = json!({"name": "Safe Site", "tracks": [
        stream("Stream A #"),
        stream("Stream B #"),
        {"name": "Vocal A #", "devices": [latencies(auto_tune), tuner("+AUTOTUNE:0")]}]});
    let path = dir.join("safe-site.json");
    std::fs::write(&path, set.to_string()).unwrap();
    path
}

/// One page whose rail holds the AUTOTUNE button.
fn frame(dir: &Path) {
    let frame = json!({"schema": 2, "default_page": "p", "pages": [{"id": "p", "title": "P",
        "rail": [{"kind": "autotune", "label": "AUTOTUNE"}]}]});
    std::fs::write(dir.join("layout.json"), frame.to_string()).unwrap();
}

/// The `autotune` records of `records` that are the step `step`.
fn steps_of(records: &[Value], step: &str) -> Vec<Value> {
    records
        .iter()
        .filter(|r| r["ev"] == "autotune" && r["step"] == step)
        .cloned()
        .collect()
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A raw client of the host's script: the test reads Live itself, the hub
/// gone. It registers nothing, so the script never takes it for a hub.
struct Script {
    ws: Socket,
    requests: u64,
}

impl Script {
    async fn connect(port: u16) -> Self {
        let url = format!("ws://127.0.0.1:{port}");
        let (ws, _) = tokio_tungstenite::connect_async(url.as_str())
            .await
            .expect("the script's WebSocket opens");
        Self { ws, requests: 0 }
    }

    /// Live's value of the parameter at `path`, through `get_prop`.
    async fn value(&mut self, path: &str) -> f64 {
        self.requests += 1;
        let uuid = format!("get{}", self.requests);
        let request = json!({"uuid": uuid, "commands": [
            {"target": path, "name": "get_prop", "args": {"prop": "value"}}]});
        self.ws
            .send(Message::Text(request.to_string().into()))
            .await
            .unwrap();
        loop {
            let message = tokio::time::timeout(Duration::from_secs(5), self.ws.next())
                .await
                .expect("the script answers within 5 s")
                .expect("the script's WebSocket stays open")
                .expect("a frame");
            let Message::Text(text) = message else {
                continue;
            };
            let frame: Value = serde_json::from_str(text.as_str()).unwrap();
            if frame["event"] == "result" && frame["uuid"] == uuid.as_str() {
                let slot = &frame["data"][0];
                assert_eq!(slot["ok"], json!(true), "{frame}");
                return slot["data"].as_f64().unwrap();
            }
        }
    }

    /// Every macro's value, in [`MACROS`]' order.
    async fn macros(&mut self) -> [f64; 3] {
        let mut values = [0.0; 3];
        for (value, path) in values.iter_mut().zip(MACROS) {
            *value = self.value(path).await;
        }
        values
    }
}

/// Whether `records` hold the script's OK of a registration of all three
/// selectors.
fn registered(records: &[Value]) -> bool {
    steps_of(records, "safe")
        .iter()
        .any(|s| s["targets"] == json!(3) && s["problem"].is_null())
}

/// Every macro set on (here by another writer: the rule that switches them
/// is #140 PR 3b).
fn all_on(host: &mut Host) {
    for (path, on) in MACROS.into_iter().zip(ON) {
        assert_eq!(host.param(path, "value", on), on, "{path}");
    }
}

/// Reads the macros until each is DRY; how long that took from `since`.
async fn dry_within(live: &mut Script, since: Instant, limit: Duration) -> Duration {
    loop {
        let values = live.macros().await;
        let elapsed = since.elapsed();
        if values == DRY {
            return elapsed;
        }
        assert!(elapsed < limit, "{values:?} after {elapsed:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The hub binary over the host on `port`, its data folder `dir` (the
/// frame there), on a port of its own; asked to stop when dropped, woken
/// first when frozen.
struct HubProcess {
    child: Child,
}

impl HubProcess {
    fn start(dir: &Path, port: u16) -> Self {
        let toml = format!("instances = [{{ name = \"band\", port = {port} }}]\n");
        std::fs::write(dir.join("fohmixer-hub.toml"), toml).unwrap();
        let log = std::fs::File::create(dir.join("hub.log")).unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_fohmixer-hub"))
            .env("PORT", "0")
            .env("FOHMIXER_DATA", dir)
            .env("NO_COLOR", "1")
            .env_remove("RUST_LOG")
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .expect("spawn fohmixer-hub");
        Self { child }
    }

    /// Sends the signal `name` (STOP, CONT, TERM): never a force-end.
    fn signal(&self, name: &str) {
        let sent = Command::new("kill")
            .args(["-s", name, &self.child.id().to_string()])
            .status()
            .expect("run kill");
        assert!(sent.success(), "SIG{name} not sent");
    }
}

impl Drop for HubProcess {
    /// Woken (a frozen hub takes no TERM), asked to stop, waited for.
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            for name in ["CONT", "TERM"] {
                let _ = Command::new("kill")
                    .args(["-s", name, &self.child.id().to_string()])
                    .status();
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if let Ok(Some(_)) = self.child.try_wait() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

#[test]
fn a_frozen_hub_leaves_every_selector_on_dry_within_2_s() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut host = Host::start_site("band", &site(dir.path()), 0, 0.0);
        frame(dir.path());
        let hub = HubProcess::start(dir.path(), host.port);
        // The hub finds the three targets and the script keeps their DRY
        // writes, AUT off.
        let deadline = Instant::now() + SECS_10;
        while !registered(&events_in(dir.path())) {
            assert!(
                Instant::now() < deadline,
                "never registered: {}",
                std::fs::read_to_string(dir.path().join("hub.log")).unwrap_or_default()
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        all_on(&mut host);
        // The hub's pings keep it heard: past the script's 1.5 s of
        // silence, nothing has fallen back.
        tokio::time::sleep(Duration::from_millis(1700)).await;
        let log = host.log_text();
        assert!(!log.contains("hub lost"), "{log}");
        let mut live = Script::connect(host.port).await;
        assert_eq!(live.macros().await, ON);
        // Frozen with its TCP open: its pings stop.
        let frozen = Instant::now();
        hub.signal("STOP");
        let dry_after = dry_within(&mut live, frozen, Duration::from_secs(2)).await;
        assert!(dry_after <= Duration::from_secs(2), "{dry_after:?}");
        println!(
            "every selector on DRY {} ms after the freeze",
            dry_after.as_millis()
        );
        let log = host.log_text();
        assert!(
            log.contains("hub lost (silence): set 3 autotune selectors to DRY, 0 already there"),
            "{log}"
        );
        assert!(
            log.contains("the hub said nothing for 1.5 s, closing it"),
            "{log}"
        );
        assert_eq!(log.matches("hub lost").count(), 1, "{log}");
        for path in MACROS {
            assert_eq!(host.sets("value", path), 2, "{path}");
        }
        hub.signal("CONT");
        drop(hub);
        drop(live);
        host.stop();
    });
}

#[test]
fn a_stopped_hub_leaves_every_selector_on_dry_within_2_s() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut host = Host::start_site("band", &site(dir.path()), 0, 0.0);
        frame(dir.path());
        let hub = TestHub::start(vec![host.cfg()], dir.path()).await;
        let records = hub.events_until(SECS_10, registered).await;
        assert!(
            steps_of(&records, "safe")
                .iter()
                .all(|s| s["instance"] == "band" && s["problem"].is_null()),
            "{records:?}"
        );
        assert!(!hub.status().await.autotune.aut);
        all_on(&mut host);
        let mut live = Script::connect(host.port).await;
        assert_eq!(live.macros().await, ON);
        // Stopped: its tasks end and its sockets close, as a killed
        // process's do.
        let stopped = Instant::now();
        drop(hub);
        let dry_after = dry_within(&mut live, stopped, Duration::from_secs(2)).await;
        assert!(dry_after <= Duration::from_secs(2), "{dry_after:?}");
        println!(
            "every selector on DRY {} ms after the stop",
            dry_after.as_millis()
        );
        let log = host.log_text();
        assert!(
            log.contains("hub lost (disconnect): set 3 autotune selectors to DRY, 0 already there"),
            "{log}"
        );
        assert_eq!(log.matches("hub lost").count(), 1, "{log}");
        // Each selector set twice through Live's setter: the test's on, then
        // the script's DRY. Nothing else wrote them.
        for path in MACROS {
            assert_eq!(host.sets("value", path), 2, "{path}");
        }
        drop(live);
        host.stop();
    });
}
