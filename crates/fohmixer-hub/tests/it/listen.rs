//! Listen end to end (#210): SimLive's `FohMixer Listen` devices announce
//! themselves, a page's `/ws/audio` socket starts listening to a track, the
//! hub selects that track's device in its poll (#264) and drives its
//! stream with its frame poll, the device streams its tone as Jitter
//! matrices, and the page decodes the hub's Opus packets back to the tone;
//! a second page shares the track, a page starting another track takes the
//! hub's one track from them (`taken`), and when the last page leaves the
//! device stops streaming. The hub switches a track's `Listen` rack on only
//! while a page listens to it (#238): a device in a rack that is off neither
//! announces nor streams.
#![cfg(unix)]

use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::support::{Host, TestHub, repo, runtime, serial};
use fohmixer_hub::config::Config;
use fohmixer_proto::listen::frames::FRAME_SAMPLES;
use fohmixer_proto::listen::{FRAME, SAMPLE_RATE};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

const SECS_10: Duration = Duration::from_secs(10);
const VOX: &str = "Hand2 #";
const OTHER: &str = "Hand3 #";

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A port free now on 127.0.0.1 for both UDP and TCP (the hub binds it right
/// after).
fn free_port() -> u16 {
    for _ in 0..64 {
        let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = udp.local_addr().unwrap().port();
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("no port free for both UDP and TCP");
}

fn site() -> PathBuf {
    repo().join("sim").join("fixtures").join("test-site.json")
}

/// A hub over `host` with Listen on `port`; it learns the host's devices
/// from their announces (each at its load) and polls them (#264).
async fn hub(dir: &Path, host: &mut Host, port: u16) -> TestHub {
    let mut config = Config::defaults(dir);
    config.instances = vec![host.cfg()];
    config.layout_poll_ms = 100;
    config.lufs.port = 0;
    config.listen.port = port;
    // No start sweep: it reaches every socket of the range, other tests'
    // too (`listen_wire.rs` sweeps its own device's port only).
    config.listen.sweep = None;
    let hub = TestHub::start_config(config).await;
    // The devices were loaded before the hub: each announces once more, as
    // at a load after the hub's start (the sweep's own test is
    // `listen_wire.rs`'s).
    assert!(host.listendev("listenload") >= 0);
    hub.status_until(SECS_10, |s| s.instances.iter().all(|i| i.online))
        .await;
    hub
}

/// A page's audio socket.
async fn audio(hub: &TestHub) -> Socket {
    let url = format!("ws://{}/ws/audio?token={}", hub.addr, hub.token);
    tokio_tungstenite::connect_async(url)
        .await
        .expect("/ws/audio opens")
        .0
}

async fn send(ws: &mut Socket, msg: Value) {
    ws.send(Message::Text(msg.to_string().into()))
        .await
        .unwrap();
}

/// The next status message (packets passed over).
async fn told(ws: &mut Socket) -> Value {
    let deadline = Instant::now() + SECS_10;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let msg = tokio::time::timeout(left, ws.next())
            .await
            .expect("a status within 10 s")
            .expect("the socket is open")
            .unwrap();
        if let Message::Text(text) = msg {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

/// The next `n` packets (status messages passed over).
async fn packets(ws: &mut Socket, n: usize) -> Vec<Vec<u8>> {
    let deadline = Instant::now() + SECS_10;
    let mut out = Vec::new();
    while out.len() < n {
        let left = deadline.saturating_duration_since(Instant::now());
        let msg = tokio::time::timeout(left, ws.next())
            .await
            .expect("packets within 10 s")
            .expect("the socket is open")
            .unwrap();
        if let Message::Binary(data) = msg {
            out.push(data.to_vec());
        }
    }
    out
}

/// The power of `hz` in `samples` (Goertzel).
fn power(samples: &[f32], hz: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * hz / f64::from(SAMPLE_RATE);
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &x in samples {
        let s = f64::from(x) + coeff * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    s1 * s1 + s2 * s2 - coeff * s1 * s2
}

fn rms(samples: &[f32]) -> f64 {
    let sum: f64 = samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    (sum / samples.len() as f64).sqrt()
}

/// The packets decoded, left and right.
fn decode(packets: &[Vec<u8>]) -> (Vec<f32>, Vec<f32>) {
    let mut dec = opus::Decoder::new(SAMPLE_RATE, opus::Channels::Stereo).unwrap();
    let (mut left, mut right) = (Vec::new(), Vec::new());
    for packet in packets {
        let mut pcm = vec![0f32; FRAME_SAMPLES];
        assert_eq!(dec.decode_float(packet, &mut pcm, false).unwrap(), FRAME);
        for pair in pcm.as_chunks::<2>().0 {
            left.push(pair[0]);
            right.push(pair[1]);
        }
    }
    (left, right)
}

fn start(track: &str) -> Value {
    json!({"type": "listen_start", "instance": "band", "track": track})
}

/// Waits until the device's streamed count stands still, and returns it.
async fn settled(host: &mut Host, line: &str) -> i64 {
    let deadline = Instant::now() + SECS_10;
    let mut last = host.listendev(line);
    loop {
        tokio::time::sleep(Duration::from_millis(400)).await;
        let now = host.listendev(line);
        if now == last {
            return now;
        }
        assert!(Instant::now() < deadline, "{line} still moves: {now}");
        last = now;
    }
}

#[test]
fn a_page_hears_the_tracks_tone_and_the_device_stops_when_it_leaves() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let port = free_port();
        let mut host = Host::start_listen("band", &site(), port);
        assert_eq!(host.listendev("listendev track 1 add rack"), 1);
        assert_eq!(host.listendev("tone track 1 1000"), 1000);
        assert_eq!(host.listendev("listendev track 2 add rack"), 1);
        assert_eq!(host.listendev("tone track 2 3000"), 3000);
        let hub = hub(dir.path(), &mut host, port).await;
        let status = hub
            .status_until(SECS_10, |s| s.listen.devices.len() == 2)
            .await;
        assert_eq!(status.listen.addr, Some(format!("127.0.0.1:{port}")));
        let vox = status
            .listen
            .devices
            .iter()
            .find(|d| d.track == VOX)
            .expect("the rack's device names its track");
        assert_eq!((vox.set.as_str(), vox.sample_rate), ("Test Site", 48_000));
        assert!(!vox.sending);

        let mut a = audio(&hub).await;
        send(&mut a, start(VOX)).await;
        assert_eq!(
            told(&mut a).await,
            json!({"type": "audio_status", "status": "listening", "instance": "band", "track": VOX})
        );
        // Half a second of packets after the codec's first ones.
        packets(&mut a, 10).await;
        let (left, right) = decode(&packets(&mut a, 25).await);
        let (l, r) = (rms(&left), rms(&right));
        assert!((l - 0.354).abs() < 0.05, "left rms {l}");
        assert!((r - 0.177).abs() < 0.03, "right rms {r}");
        let (at_1k, at_3k) = (power(&left, 1000.0), power(&left, 3000.0));
        assert!(at_1k > 1000.0 * at_3k, "1 kHz {at_1k}, 3 kHz {at_3k}");
        assert_eq!(host.listendev("streamed track 2"), 0, "never selected");

        // A second page shares the track.
        let mut b = audio(&hub).await;
        send(&mut b, start(VOX)).await;
        assert_eq!(told(&mut b).await["status"], "listening");
        assert!(!packets(&mut b, 3).await.is_empty());
        let status = hub
            .status_until(SECS_10, |s| {
                s.listen.target.as_ref().is_some_and(|t| t.listeners == 2)
            })
            .await;
        let target = status.listen.target.unwrap();
        assert_eq!(target.device, Some(vox.device));
        assert!(status.listen.frames > 0 && status.listen.packets > 0);

        // One leaves: the other still hears it. The last stop ends it.
        send(&mut a, json!({"type": "listen_stop"})).await;
        assert_eq!(told(&mut a).await["status"], "stopped");
        assert!(!packets(&mut b, 5).await.is_empty());
        drop(b);
        hub.status_until(SECS_10, |s| s.listen.target.is_none())
            .await;
        let streamed = settled(&mut host, "streamed track 1").await;
        assert!(streamed > 0);
        tokio::time::sleep(Duration::from_millis(1600)).await;
        assert_eq!(host.listendev("streamed track 1"), streamed, "stopped");

        // Now the other track.
        let mut c = audio(&hub).await;
        send(&mut c, start(OTHER)).await;
        assert_eq!(told(&mut c).await["status"], "listening");
        let (left, _) = decode(&packets(&mut c, 20).await[10..]);
        assert!(power(&left, 3000.0) > 1000.0 * power(&left, 1000.0));

        let records = hub
            .events_until(SECS_10, |r| {
                r.iter().any(|e| e["ev"] == "listen" && e["what"] == "stop")
            })
            .await;
        let starts: Vec<&Value> = records
            .iter()
            .filter(|e| e["ev"] == "listen" && e["what"] == "start")
            .collect();
        assert_eq!(starts.len(), 2, "one per track, a join records none");
        assert_eq!(starts[0]["track"], VOX);
        assert!(
            records
                .iter()
                .any(|e| e["ev"] == "listen" && e["what"] == "hello" && e["device"] == vox.device)
        );
        drop(c);
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_device_at_96_khz_is_decimated_to_the_pages_48_khz_tone() {
    // #210's PC probe: the master Live runs at 96 kHz; its devices stream
    // 10 ms matrices of 960 samples. The hub decimates them 2:1 and the
    // page hears the tone at its own pitch and level.
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let port = free_port();
        let mut host = Host::start_listen("band", &site(), port);
        assert_eq!(host.listendev("listendev track 1 add rack 96000"), 1);
        assert_eq!(host.listendev("tone track 1 1000"), 1000);
        let hub = hub(dir.path(), &mut host, port).await;
        let status = hub
            .status_until(SECS_10, |s| s.listen.devices.len() == 1)
            .await;
        assert_eq!(status.listen.devices[0].sample_rate, 96_000);

        let mut a = audio(&hub).await;
        send(&mut a, start(VOX)).await;
        assert_eq!(told(&mut a).await["status"], "listening");
        packets(&mut a, 10).await;
        let (left, right) = decode(&packets(&mut a, 25).await);
        // 25 packets of 20 ms at 48 kHz: half a second, as at 48 kHz.
        assert_eq!(left.len(), 25 * FRAME);
        let (l, r) = (rms(&left), rms(&right));
        assert!((l - 0.354).abs() < 0.05, "left rms {l}");
        assert!((r - 0.177).abs() < 0.03, "right rms {r}");
        let (at_1k, at_2k) = (power(&left, 1000.0), power(&left, 2000.0));
        assert!(at_1k > 1000.0 * at_2k, "1 kHz {at_1k}, 2 kHz {at_2k}");
        let status = hub.status_until(SECS_10, |s| s.listen.frames > 0).await;
        assert!(
            !status.listen.refused.contains_key("sample_rate"),
            "{:?}",
            status.listen.refused
        );
        drop(a);
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_device_at_another_rate_is_refused() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let port = free_port();
        let mut host = Host::start_listen("band", &site(), port);
        assert_eq!(host.listendev("listendev track 1 add rack 44100"), 1);
        assert_eq!(host.listendev("tone track 1 1000"), 1000);
        let hub = hub(dir.path(), &mut host, port).await;
        hub.status_until(SECS_10, |s| s.listen.devices.len() == 1)
            .await;
        let mut a = audio(&hub).await;
        send(&mut a, start(VOX)).await;
        assert_eq!(told(&mut a).await["status"], "listening");
        // The silence counts from here: the rack's switch-on answered.
        let told_at = Instant::now();
        let status = hub
            .status_until(SECS_10, |s| {
                s.listen.refused.get("sample_rate").is_some_and(|n| *n > 0)
            })
            .await;
        assert_eq!(status.listen.frames, 0);
        // Its rack was switched on, yet nothing comes: told once after two
        // seconds of silence (#238's review: the 2 s silence end to end), at
        // the first 1 s tick past them.
        let silent = told(&mut a).await;
        assert_eq!(
            silent,
            json!({"type": "audio_status", "status": "no_source", "instance": "band", "track": VOX})
        );
        let after = told_at.elapsed();
        assert!(
            after >= Duration::from_millis(1900) && after < Duration::from_millis(3600),
            "2 to 3 s after the start: {after:?}"
        );
        assert!(
            next_within(&mut a, Duration::from_millis(2500))
                .await
                .is_none(),
            "told once, and no packet"
        );
        assert_eq!(host.listendev("listenrack track 1"), 1);
        drop(a);
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_track_without_a_listen_rack_or_an_unknown_instance_is_no_source() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let port = free_port();
        let mut host = Host::start_listen("band", &site(), port);
        let hub = hub(dir.path(), &mut host, port).await;
        let mut a = audio(&hub).await;
        send(
            &mut a,
            json!({"type": "listen_start", "instance": "nope", "track": VOX}),
        )
        .await;
        assert_eq!(told(&mut a).await["status"], "no_source");
        // The track has no `Listen` rack (#238): told at once, no wait.
        send(&mut a, start(VOX)).await;
        assert_eq!(
            told(&mut a).await,
            json!({"type": "audio_status", "status": "no_source", "instance": "band", "track": VOX})
        );
        let target = hub.status().await.listen.target.unwrap();
        assert_eq!(target.problem.as_deref(), Some("no device"));
        let records = hub.events_until(SECS_10, |r| !racks(r).is_empty()).await;
        let missing = racks(&records);
        assert_eq!(missing.len(), 1, "{missing:?}");
        assert_eq!(missing[0]["track"], VOX);
        assert_eq!(missing[0]["on"], true);
        assert_eq!(missing[0]["result"], "missing");
        assert_eq!(missing[0]["problem"], "no Listen rack");
        // A device in a rack that is on comes: told listening again, before
        // its first packet.
        assert_eq!(host.listendev("listendev track 1 add rack"), 1);
        assert_eq!(host.listendev("tone track 1 1000"), 1000);
        let deadline = Instant::now() + SECS_10;
        let again = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let msg = tokio::time::timeout(left, a.next())
                .await
                .expect("a message within 10 s")
                .expect("the socket is open")
                .unwrap();
            match msg {
                Message::Text(text) => break serde_json::from_str::<Value>(&text).unwrap(),
                Message::Binary(_) => panic!("a packet before the listening status"),
                _ => {}
            }
        };
        assert_eq!(again["status"], "listening");
        assert_eq!(again["track"], VOX);
        assert!(!packets(&mut a, 3).await.is_empty());
        send(&mut a, json!({"type": "listen"})).await;
        assert_eq!(told(&mut a).await["status"], "refused");
        // Too long a name is refused too.
        let long = "x".repeat(257);
        send(&mut a, start(&long)).await;
        assert_eq!(told(&mut a).await["status"], "refused");
        hub.stop().await;
        host.stop();
    });
}

/// The hub's `listen` `rack` records (#238), in order.
fn racks(records: &[Value]) -> Vec<Value> {
    records
        .iter()
        .filter(|e| e["ev"] == "listen" && e["what"] == "rack")
        .cloned()
        .collect()
}

/// The `(track, on, result)` of each `rack` record.
fn rack_steps(records: &[Value]) -> Vec<(String, bool, String)> {
    racks(records)
        .iter()
        .map(|e| {
            (
                e["track"].as_str().unwrap().to_string(),
                e["on"].as_bool().unwrap(),
                e["result"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn step(track: &str, on: bool) -> (String, bool, String) {
    (track.to_string(), on, "done".to_string())
}

/// Waits until the Device On of the newest Listen device's rack on `track`
/// (`listenrack track <i>`) reads `want`.
async fn rack_until(host: &mut Host, track: usize, want: i64) {
    let line = format!("listenrack track {track}");
    let deadline = Instant::now() + SECS_10;
    loop {
        let now = host.listendev(&line);
        if now == want {
            return;
        }
        assert!(Instant::now() < deadline, "{line} reads {now}, not {want}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A host whose tracks 1 (VOX, 1 kHz) and 2 (OTHER, 3 kHz) hold a Listen
/// device in a rack that is off, as the PC's sets are saved (#238).
fn racks_off(port: u16) -> Host {
    let mut host = Host::start_listen("band", &site(), port);
    assert_eq!(host.listendev("listendev track 1 add rack off"), 1);
    assert_eq!(host.listendev("tone track 1 1000"), 1000);
    assert_eq!(host.listendev("listendev track 2 add rack off"), 1);
    assert_eq!(host.listendev("tone track 2 3000"), 3000);
    host
}

#[test]
fn a_start_switches_the_tracks_listen_rack_on_and_the_stop_off() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let port = free_port();
        let mut host = racks_off(port);
        let hub = hub(dir.path(), &mut host, port).await;
        // Off, the devices announce nothing; the hub's start switches
        // nothing on (no sweep of the tracks).
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert!(hub.status().await.listen.devices.is_empty());
        assert_eq!(host.listendev("listenrack track 1"), 0);
        assert_eq!(host.listendev("listenrack track 2"), 0);

        let mut a = audio(&hub).await;
        send(&mut a, start(VOX)).await;
        assert_eq!(told(&mut a).await["status"], "listening");
        // On before the page is told: the device comes up and streams.
        assert_eq!(host.listendev("listenrack track 1"), 1);
        assert_eq!(host.listendev("listenrack track 2"), 0);
        packets(&mut a, 10).await;
        let (left, _) = decode(&packets(&mut a, 25).await);
        assert!(power(&left, 1000.0) > 1000.0 * power(&left, 3000.0));

        send(&mut a, json!({"type": "listen_stop"})).await;
        assert_eq!(told(&mut a).await["status"], "stopped");
        rack_until(&mut host, 1, 0).await;
        let streamed = settled(&mut host, "streamed track 1").await;
        assert!(streamed > 0);
        let records = hub
            .events_until(SECS_10, |r| rack_steps(r).len() == 2)
            .await;
        assert_eq!(
            rack_steps(&records),
            vec![step(VOX, true), step(VOX, false)]
        );
        assert_eq!(host.listendev("streamed track 2"), 0, "never on");
        drop(a);
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn a_take_switches_the_new_tracks_rack_on_then_the_old_ones_off() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let port = free_port();
        let mut host = racks_off(port);
        let hub = hub(dir.path(), &mut host, port).await;
        let mut a = audio(&hub).await;
        send(&mut a, start(VOX)).await;
        assert_eq!(told(&mut a).await["status"], "listening");
        assert!(!packets(&mut a, 3).await.is_empty());

        let mut c = audio(&hub).await;
        send(&mut c, start(OTHER)).await;
        assert_eq!(told(&mut c).await["status"], "listening");
        assert_eq!(host.listendev("listenrack track 2"), 1);
        assert_eq!(told(&mut a).await["status"], "taken");
        rack_until(&mut host, 1, 0).await;
        let (left, _) = decode(&packets(&mut c, 20).await[10..]);
        assert!(power(&left, 3000.0) > 1000.0 * power(&left, 1000.0));
        // The page it was taken from leaves: the new track's rack stays on.
        drop(a);
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(host.listendev("listenrack track 2"), 1);
        let records = hub
            .events_until(SECS_10, |r| rack_steps(r).len() == 3)
            .await;
        assert_eq!(
            rack_steps(&records),
            vec![step(VOX, true), step(OTHER, true), step(VOX, false)]
        );
        drop(c);
        rack_until(&mut host, 2, 0).await;
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn with_two_pages_on_a_track_its_rack_goes_off_only_after_both_leave() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let port = free_port();
        let mut host = racks_off(port);
        let hub = hub(dir.path(), &mut host, port).await;
        let mut a = audio(&hub).await;
        send(&mut a, start(VOX)).await;
        assert_eq!(told(&mut a).await["status"], "listening");
        let mut b = audio(&hub).await;
        send(&mut b, start(VOX)).await;
        assert_eq!(told(&mut b).await["status"], "listening");
        assert!(!packets(&mut b, 3).await.is_empty());

        send(&mut a, json!({"type": "listen_stop"})).await;
        assert_eq!(told(&mut a).await["status"], "stopped");
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(host.listendev("listenrack track 1"), 1, "b still listens");
        assert!(!packets(&mut b, 5).await.is_empty());

        drop(b);
        rack_until(&mut host, 1, 0).await;
        let records = hub
            .events_until(SECS_10, |r| rack_steps(r).len() == 2)
            .await;
        assert_eq!(
            rack_steps(&records),
            vec![step(VOX, true), step(VOX, false)]
        );
        drop(a);
        hub.stop().await;
        host.stop();
    });
}

/// The next message within `limit` that is no ping or pong; none when
/// nothing came.
async fn next_within(ws: &mut Socket, limit: Duration) -> Option<Message> {
    let deadline = Instant::now() + limit;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, ws.next()).await {
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
            Ok(Some(Ok(msg))) => return Some(msg),
            _ => return None,
        }
    }
}

#[test]
fn the_newest_start_takes_the_track_from_the_pages_listening() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let port = free_port();
        let mut host = Host::start_listen("band", &site(), port);
        assert_eq!(host.listendev("listendev track 1 add rack"), 1);
        assert_eq!(host.listendev("tone track 1 1000"), 1000);
        assert_eq!(host.listendev("listendev track 2 add rack"), 1);
        assert_eq!(host.listendev("tone track 2 3000"), 3000);
        let hub = hub(dir.path(), &mut host, port).await;
        hub.status_until(SECS_10, |s| s.listen.devices.len() == 2)
            .await;
        // Two pages on one track (a tab left open, an iPad).
        let mut a = audio(&hub).await;
        send(&mut a, start(VOX)).await;
        assert_eq!(told(&mut a).await["status"], "listening");
        let mut b = audio(&hub).await;
        send(&mut b, start(VOX)).await;
        assert_eq!(told(&mut b).await["status"], "listening");
        assert!(!packets(&mut a, 3).await.is_empty());
        // A third page starts another track: it takes it, both are told.
        let mut c = audio(&hub).await;
        send(&mut c, start(OTHER)).await;
        assert_eq!(
            told(&mut c).await,
            json!({"type": "audio_status", "status": "listening", "instance": "band", "track": OTHER})
        );
        for page in [&mut a, &mut b] {
            assert_eq!(
                told(page).await,
                json!({"type": "audio_status", "status": "taken", "instance": "band", "track": VOX})
            );
        }
        let (left, _) = decode(&packets(&mut c, 20).await[10..]);
        assert!(power(&left, 3000.0) > 1000.0 * power(&left, 1000.0));
        // The pages it was taken from get nothing more, and leaving changes
        // nothing for the new one.
        assert!(
            next_within(&mut a, Duration::from_millis(1500))
                .await
                .is_none()
        );
        drop(a);
        drop(b);
        let status = hub
            .status_until(SECS_10, |s| {
                s.listen.target.as_ref().is_some_and(|t| t.track == OTHER)
            })
            .await;
        assert_eq!(status.listen.target.unwrap().listeners, 1);
        assert!(!packets(&mut c, 5).await.is_empty());
        let records = hub
            .events_until(SECS_10, |r| {
                r.iter().any(|e| e["ev"] == "listen" && e["what"] == "taken")
            })
            .await;
        let taken = records
            .iter()
            .find(|e| e["ev"] == "listen" && e["what"] == "taken")
            .unwrap();
        assert_eq!(taken["from"]["track"], VOX);
        assert_eq!(taken["from"]["listeners"], 2);
        assert_eq!(taken["to"]["track"], OTHER);
        assert_eq!(taken["peer"], "127.0.0.1");
        assert!(taken["client"].is_u64());
        drop(c);
        hub.stop().await;
        host.stop();
    });
}

/// The tracks whose device announces it is sending, in the hub's status.
fn sending(status: &fohmixer_proto::client::HubStatus) -> Vec<String> {
    let mut tracks: Vec<String> = status
        .listen
        .devices
        .iter()
        .filter(|d| d.sending)
        .map(|d| d.track.clone())
        .collect();
    tracks.sort();
    tracks
}

#[test]
fn each_device_is_told_in_its_own_poll_and_only_the_selected_one_streams() {
    // #264: the hub pushes the select in its poll (`/fohmixer/listen/poll
    // <on>` to each device's own port, the patch's `udpreceive @defer 1`)
    // and drives the selected device's catch with its frame poll; the
    // device has no clock of its own. #210's PC finding stays answered:
    // each device has a port of its own, so both of a Live's devices hear
    // the hub.
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let port = free_port();
        let mut host = Host::start_listen("band", &site(), port);
        assert_eq!(host.listendev("listendev track 1 add rack"), 1);
        assert_eq!(host.listendev("listendev track 2 add rack"), 1);
        let hub = hub(dir.path(), &mut host, port).await;
        let status = hub
            .status_until(SECS_10, |s| s.listen.devices.len() == 2)
            .await;
        assert!(sending(&status).is_empty(), "nothing listened to");
        let ports: Vec<u16> = status.listen.devices.iter().map(|d| d.port).collect();
        assert_ne!(ports[0], ports[1], "a port each");

        let mut a = audio(&hub).await;
        send(&mut a, start(VOX)).await;
        assert_eq!(told(&mut a).await["status"], "listening");
        hub.status_until(SECS_10, |s| sending(s) == [VOX]).await;
        packets(&mut a, 5).await;
        assert_eq!(host.listendev("streamed track 2"), 0, "never selected");

        // Another track: each device is told on its own.
        send(&mut a, start(OTHER)).await;
        assert_eq!(told(&mut a).await["status"], "listening");
        hub.status_until(SECS_10, |s| sending(s) == [OTHER]).await;
        packets(&mut a, 5).await;
        drop(a);
        let status = hub
            .status_until(SECS_10, |s| {
                s.listen.target.is_none() && sending(s).is_empty()
            })
            .await;
        assert!(status.listen.polls > 0 && status.listen.frame_polls > 0);
        assert_eq!(status.listen.poll_failed, 0, "{:?}", status.listen);
        let streamed = settled(&mut host, "streamed track 2").await;
        tokio::time::sleep(Duration::from_millis(600)).await;
        assert_eq!(host.listendev("streamed track 2"), streamed, "stopped");
        hub.stop().await;
        host.stop();
    });
}
