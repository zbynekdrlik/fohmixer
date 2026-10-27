//! LUFS end to end (#151, spec §4.4, §5): SimLive's Max devices send their
//! datagrams to the hub's listener; the hub matches each key to the
//! `+LUFS` strip of the Live that holds the device (its own walk of the
//! track), and serves the reading to the pages.
#![cfg(unix)]

use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::support::{Client, Host, TestHub, repo, runtime, serial};
use fohmixer_hub::config::Config;
use fohmixer_proto::client::{HubStatus, LufsItem, ServerMsg};
use fohmixer_proto::lufs::{LufsProblem, LufsState};
use fohmixer_proto::meter::post_fader;
use fohmixer_proto::volume::volume_of;
use serde_json::{Value, json};

const LEFT: &str = "output_meter_left";

const SECS_5: Duration = Duration::from_secs(5);
const TRACK: &str = "live_set tracks 1";

fn free_port() -> u16 {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.local_addr().unwrap().port()
}

fn site() -> PathBuf {
    repo().join("sim").join("fixtures").join("test-site.json")
}

/// The test site under another set name (the master Live's set).
fn renamed_site(dir: &Path, name: &str) -> PathBuf {
    let mut spec: Value = serde_json::from_str(&std::fs::read_to_string(site()).unwrap()).unwrap();
    spec["name"] = json!(name);
    let path = dir.join("master-site.json");
    std::fs::write(&path, spec.to_string()).unwrap();
    path
}

/// The frame: one tag group LUFS.
fn frame(dir: &Path) {
    let frame = json!({"schema": 2, "default_page": "p", "pages": [{"id": "p", "title": "P",
        "rows": [{"sections": [{"kind": "group", "id": "lufs", "title": "LUFS", "tags": "LUFS"}]}]}]});
    std::fs::write(dir.join("layout.json"), frame.to_string()).unwrap();
}

async fn hub(dir: &Path, hosts: &[&Host], port: u16) -> TestHub {
    frame(dir);
    let mut config = Config::defaults(dir);
    config.instances = hosts.iter().map(|h| h.cfg()).collect();
    config.layout_poll_ms = 100;
    config.lufs.port = port;
    let hub = TestHub::start_config(config).await;
    hub.status_until(SECS_5, |s| s.instances.iter().all(|i| i.online))
        .await;
    hub
}

/// Waits for a `lufs` item of `instance`'s Hand2 # strip that `pred` takes.
async fn item(
    client: &mut Client,
    instance: &str,
    limit: Duration,
    pred: impl Fn(&LufsItem) -> bool,
) -> LufsItem {
    client
        .wait(limit, |m| match m {
            ServerMsg::Lufs { items } => items
                .iter()
                .rev()
                .find(|i| i.instance == instance && i.target == TRACK && pred(i))
                .cloned(),
            _ => None,
        })
        .await
}

/// Waits until every instance's Hand2 # is a loudness point named.
async fn points(hub: &TestHub, n: usize) -> HubStatus {
    hub.status_until(SECS_5, |s| {
        s.lufs.strips.len() == n
            && s.lufs
                .strips
                .iter()
                .all(|p| p.track.as_deref() == Some("Hand2 #"))
    })
    .await
}

/// The `lufs` records of `what`, once at least one is written (the event
/// log's writer flushes on its own thread).
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
fn a_device_on_a_lufs_strip_reaches_the_page_and_clears_once_it_stops() {
    let _serial = serial();
    runtime().block_on(async {
        let port = free_port();
        let mut host = Host::start_lufs("band", &site(), port);
        let dir = tempfile::tempdir().unwrap();
        let hub = hub(dir.path(), &[&host], port).await;
        assert_eq!(
            host.tuner("track", 1, "set", r#""Vox 1" +G:LUFS +LUFS:-25"#),
            1
        );
        points(&hub, 1).await;
        assert_eq!(host.maxdev("maxdev lufs track 1 add rack"), 1);
        assert_eq!(host.maxdev("lufs track 1 -253 -300"), 1);
        let mut client = hub.client().await;
        let got = item(&mut client, "band", SECS_5, |i| i.tenths.is_some()).await;
        assert_eq!(got.tenths, Some(-253), "an input point shows S, not V");
        assert_eq!(got.problem, None);
        let status = hub
            .status_until(SECS_5, |s| s.lufs.strips[0].state == LufsState::In)
            .await;
        let device = &status.lufs.devices[0];
        assert_eq!(device.state, "matched");
        assert_eq!(device.instance.as_deref(), Some("band"));
        assert_eq!(device.parents, vec!["LUFS", "LUFS", "Hand2 #", "Test Site"]);
        // SimLive sends the devices' version.txt, which tracks the workspace.
        assert_eq!(device.version, format!("v{}", env!("CARGO_PKG_VERSION")));
        assert_eq!(status.lufs.strips[0].tenths, Some(-253));
        assert_eq!(status.lufs.listener.refused.len(), 0);
        // The device stops: the number goes within 1.5 s, one lost record.
        assert_eq!(host.maxdev("lufs track 1 off"), 0);
        let gone = item(&mut client, "band", Duration::from_millis(1500), |i| {
            i.tenths.is_none()
        })
        .await;
        assert_eq!(gone.problem, None);
        assert_eq!(records(&hub, "lost").await.len(), 1);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(records(&hub, "lost").await.len(), 1, "one lost record");
        assert_eq!(records(&hub, "matched").await.len(), 1);
        assert_eq!(records(&hub, "found").await.len(), 1);
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn the_same_id_and_place_in_two_sets_goes_to_the_set_that_holds_the_strip() {
    let _serial = serial();
    runtime().block_on(async {
        let port = free_port();
        let dir = tempfile::tempdir().unwrap();
        let mut band = Host::start_lufs("band", &site(), port);
        let mut master = Host::start_lufs("master", &renamed_site(dir.path(), "Test Master"), port);
        let hub = hub(dir.path(), &[&band, &master], port).await;
        assert_eq!(
            band.tuner("track", 1, "set", r#""Vox 1" +G:LUFS +LUFS:-25"#),
            1
        );
        points(&hub, 1).await;
        // Both devices: Max id 1 at the same place in each set.
        assert_eq!(band.maxdev("maxdev lufs track 1 add rack"), 1);
        assert_eq!(master.maxdev("maxdev lufs track 1 add rack"), 1);
        assert_eq!(master.maxdev("lufs track 1 -200 -200"), 1);
        let status = hub
            .status_until(SECS_5, |s| {
                s.lufs
                    .devices
                    .iter()
                    .any(|d| d.parents[3] == "Test Master" && d.state == "unmatched")
            })
            .await;
        assert_eq!(
            status.lufs.strips[0].tenths, None,
            "the master's is not band's"
        );
        assert_eq!(band.maxdev("lufs track 1 -253 -300"), 1);
        let mut client = hub.client().await;
        let got = item(&mut client, "band", SECS_5, |i| i.tenths.is_some()).await;
        assert_eq!(got.tenths, Some(-253));
        let status = hub.status().await;
        let band_key = status
            .lufs
            .devices
            .iter()
            .find(|d| d.parents[3] == "Test Site")
            .unwrap();
        assert_eq!(band_key.instance.as_deref(), Some("band"));
        assert_eq!(status.lufs.strips[0].tenths, Some(-253));
        hub.stop().await;
        band.stop();
        master.stop();
    });
}

#[test]
fn a_place_held_in_both_sets_is_two_lufs_meters_and_shows_nothing() {
    let _serial = serial();
    runtime().block_on(async {
        let port = free_port();
        let dir = tempfile::tempdir().unwrap();
        // Both sets of one name: the key cannot tell them apart.
        let mut band = Host::start_lufs("band", &site(), port);
        let mut master = Host::start_lufs("master", &site(), port);
        let hub = hub(dir.path(), &[&band, &master], port).await;
        for host in [&mut band, &mut master] {
            assert_eq!(
                host.tuner("track", 1, "set", r#""Vox 1" +G:LUFS +LUFS:-25"#),
                1
            );
            assert_eq!(host.maxdev("maxdev lufs track 1 add rack"), 1);
        }
        points(&hub, 2).await;
        assert_eq!(band.maxdev("lufs track 1 -253 -300"), 1);
        let mut client = hub.client().await;
        // Both points change in one message: one wait reads both (a second
        // wait would look for a message that never comes, `wait` takes the
        // one it matched).
        let both = client
            .wait(SECS_5, |m| match m {
                ServerMsg::Lufs { items } => {
                    let two = |instance: &str| {
                        items.iter().any(|i| {
                            i.instance == instance
                                && i.target == TRACK
                                && i.problem == Some(LufsProblem::TwoLufsMeters)
                        })
                    };
                    (two("band") && two("master")).then(|| items.clone())
                }
                _ => None,
            })
            .await;
        assert!(both.iter().all(|i| i.tenths.is_none()), "{both:?}");
        let status = hub.status().await;
        assert_eq!(status.lufs.devices[0].state, "two");
        assert!(status.lufs.strips.iter().all(|s| s.tenths.is_none()));
        assert_eq!(records(&hub, "two").await.len(), 1);
        hub.stop().await;
        band.stop();
        master.stop();
    });
}

#[test]
fn a_missing_device_is_no_lufs_meter() {
    let _serial = serial();
    runtime().block_on(async {
        let port = free_port();
        let mut host = Host::start_lufs("band", &site(), port);
        let dir = tempfile::tempdir().unwrap();
        let hub = hub(dir.path(), &[&host], port).await;
        assert_eq!(
            host.tuner("track", 1, "set", r#""Vox 1" +G:LUFS +LUFS:-25"#),
            1
        );
        let mut client = hub.client().await;
        let got = item(&mut client, "band", SECS_5, |i| i.problem.is_some()).await;
        assert_eq!(got.problem, Some(LufsProblem::NoLufsMeter));
        assert_eq!(got.tenths, None);
        let problems = records(&hub, "problem").await;
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0]["problem"], "no LUFS meter");
        assert_eq!(problems[0]["warn"], true);
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn an_output_point_reads_after_its_fader_and_a_disconnect_clears() {
    let _serial = serial();
    runtime().block_on(async {
        let port = free_port();
        let mut host = Host::start_lufs("band", &site(), port);
        let dir = tempfile::tempdir().unwrap();
        let hub = hub(dir.path(), &[&host], port).await;
        assert_eq!(
            host.tuner("track", 1, "set", r#""Vocals" +G:LUFS +LUFS"#),
            1
        );
        points(&hub, 1).await;
        let mut client = hub.client().await;
        let volume = format!("{TRACK} mixer_device volume");
        client
            .set("band", &volume, "value", json!(volume_of(0.0)))
            .await;
        assert_eq!(host.maxdev("maxdev lufs track 1 add rack"), 1);
        assert_eq!(host.maxdev("lufs track 1 -300 -253"), 1);
        let got = item(&mut client, "band", SECS_5, |i| i.tenths == Some(-300)).await;
        assert_eq!(got.problem, None, "an output point shows S at 0 dB");
        client
            .set("band", &volume, "value", json!(volume_of(-10.0)))
            .await;
        item(&mut client, "band", SECS_5, |i| i.tenths == Some(-400)).await;
        client.set("band", TRACK, "mute", json!(true)).await;
        item(&mut client, "band", SECS_5, |i| i.tenths == Some(-700)).await;
        let status = hub.status().await;
        assert_eq!(status.lufs.strips[0].state, LufsState::Silent);
        // Live goes away: the point shows nothing.
        host.stop();
        let gone = item(&mut client, "band", SECS_5, |i| i.tenths.is_none()).await;
        assert_eq!(gone.problem, None);
        hub.stop().await;
    });
}

// --- the armed tracks' output meter (#151 Task 13, spec §3.4) ---

/// Hand2 # as an input point with the fader at 0 dB and the pan centred
/// (both followed by the hub before the meter device sends).
async fn input_point(hub: &TestHub, host: &mut Host, client: &mut Client) {
    assert_eq!(
        host.tuner("track", 1, "set", r#""Vox 1" +G:LUFS +LUFS:-25"#),
        1
    );
    points(hub, 1).await;
    let volume = format!("{TRACK} mixer_device volume");
    client
        .set("band", &volume, "value", json!(volume_of(0.0)))
        .await;
    let panning = format!("{TRACK} mixer_device panning");
    client.set("band", &panning, "value", json!(0.0)).await;
}

/// Whether a meter value is `want` (the hub's dB maths, within 1e-6).
fn is(item: &fohmixer_proto::client::ValueItem, want: f64) -> bool {
    item.value
        .as_ref()
        .and_then(Value::as_f64)
        .is_some_and(|v| (v - want).abs() < 1e-6)
}

/// Waits until the latest values heard of `left` and `right` are `want`
/// (L, R; within 1e-6). Both keys' items often come in one `values`
/// message, so they are read together: a `value_until` of one consumes the
/// other's item with the message.
async fn pair_until(client: &mut Client, left: &str, right: &str, want: (f64, f64)) {
    let deadline = tokio::time::Instant::now() + SECS_5;
    let mut heard: (Option<f64>, Option<f64>) = (None, None);
    let near = |v: Option<f64>, w: f64| v.is_some_and(|v| (v - w).abs() < 1e-6);
    while !(near(heard.0, want.0) && near(heard.1, want.1)) {
        let left_time = deadline.saturating_duration_since(tokio::time::Instant::now());
        let msg = client
            .next_message(left_time)
            .await
            .unwrap_or_else(|| panic!("no levels {want:?} within 5 s; last heard {heard:?}"));
        if let ServerMsg::Values { items } = msg {
            for item in items {
                let value = item.value.as_ref().and_then(Value::as_f64);
                if item.sub == left {
                    heard.0 = value;
                } else if item.sub == right {
                    heard.1 = value;
                }
            }
        }
    }
}

/// Every value of `sub` that arrives within `limit`.
async fn values_for(client: &mut Client, sub: &str, limit: Duration) -> Vec<f64> {
    let deadline = tokio::time::Instant::now() + limit;
    let mut out = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return out;
        }
        let Some(msg) = client.next_message(left).await else {
            return out;
        };
        if let ServerMsg::Values { items } = msg {
            out.extend(
                items
                    .iter()
                    .filter(|i| i.sub == sub)
                    .filter_map(|i| i.value.as_ref().and_then(Value::as_f64)),
            );
        }
    }
}

#[test]
fn an_armed_strips_meter_follows_its_fader() {
    let _serial = serial();
    runtime().block_on(async {
        let port = free_port();
        let mut host = Host::start_lufs("band", &site(), port);
        let dir = tempfile::tempdir().unwrap();
        let hub = hub(dir.path(), &[&host], port).await;
        let mut client = hub.client().await;
        input_point(&hub, &mut host, &mut client).await;
        let left = client.sub_key("band", TRACK, LEFT, false).await;
        let right = client
            .sub_key("band", TRACK, "output_meter_right", false)
            .await;
        // Armed, with its LUFS device and no meter device: `no meter device`.
        assert_eq!(host.maxdev("maxdev lufs track 1 add rack"), 1);
        assert_eq!(host.maxdev("lufs track 1 -253 -300"), 1);
        client.set("band", TRACK, "arm", json!(true)).await;
        let got = item(&mut client, "band", SECS_5, |i| i.problem.is_some()).await;
        assert_eq!(got.problem, Some(LufsProblem::NoMeterDevice));
        assert_eq!(got.tenths, Some(-253), "the LUFS reading stays");
        // Only what comes from here on (`wait` would find an older item).
        client.clear();
        // The meter device: the bars read its levels at the fader's 0 dB.
        assert_eq!(host.maxdev("maxdev meter track 1 add rack"), 1);
        // SimLive counts every device sending on the host: the LUFS one too.
        assert_eq!(host.maxdev("level track 1 -120 -180"), 2);
        // The expected levels from the volume Live holds (volume_of's
        // bisection is not exactly 0 dB).
        let at = |volume: f64| post_fader(-120, -180, volume, 0.0, false);
        let (l, r) = at(volume_of(0.0));
        pair_until(&mut client, &left, &right, (l, r)).await;
        assert!((l - fohmixer_proto::meter::live_value(-12.0)).abs() < 1e-3);
        // The problem cleared, the reading kept (read from the status:
        // `pair_until` reads the page's messages in order and passes the
        // `lufs` ones over).
        let status = hub
            .status_until(SECS_5, |s| s.lufs.strips[0].problem.is_none())
            .await;
        assert_eq!(status.lufs.strips[0].tenths, Some(-253));
        // The fader 10 dB down: both bars fall by 10 dB.
        let volume = format!("{TRACK} mixer_device volume");
        client
            .set("band", &volume, "value", json!(volume_of(-10.0)))
            .await;
        let (l, r) = at(volume_of(-10.0));
        pair_until(&mut client, &left, &right, (l, r)).await;
        assert!((l - fohmixer_proto::meter::live_value(-22.0)).abs() < 1e-3);
        // Muted: the floor.
        client.set("band", TRACK, "mute", json!(true)).await;
        pair_until(&mut client, &left, &right, (0.0, 0.0)).await;
        let status = hub.status().await;
        assert!(
            status
                .lufs
                .devices
                .iter()
                .any(|d| d.kind == "meter" && d.state == "matched"),
            "{:?}",
            status.lufs.devices
        );
        hub.stop().await;
        host.stop();
    });
}

#[test]
fn live_values_never_interleave_while_the_device_is_fresh_and_come_back_after() {
    let _serial = serial();
    runtime().block_on(async {
        let port = free_port();
        // SimLive's meters move 30 times a second, as music would.
        let mut host = Host::start_lufs_meters("band", &site(), port, 30.0);
        let dir = tempfile::tempdir().unwrap();
        let hub = hub(dir.path(), &[&host], port).await;
        let mut client = hub.client().await;
        input_point(&hub, &mut host, &mut client).await;
        let left = client.sub_key("band", TRACK, LEFT, false).await;
        // From the volume Live holds (volume_of's bisection is not exactly 0 dB).
        let want = post_fader(-120, -180, volume_of(0.0), 0.0, false).0;
        // Live's own values flow before the device.
        let before = values_for(&mut client, &left, Duration::from_millis(500)).await;
        assert!(
            before.iter().any(|v| (v - want).abs() > 0.01),
            "Live's meters move: {before:?}"
        );
        assert_eq!(host.maxdev("maxdev meter track 1 add rack"), 1);
        assert_eq!(host.maxdev("level track 1 -120 -180"), 1);
        client.value_until(&left, SECS_5, |i| is(i, want)).await;
        // While the device sends, every value on the key is the computed one.
        let during = values_for(&mut client, &left, Duration::from_millis(1500)).await;
        assert!(
            during.iter().all(|v| (v - want).abs() < 1e-6),
            "only the device's level: {during:?}"
        );
        // A page subscribing now is answered with the device's level too.
        let mut late = hub.client().await;
        match late.sub("band", TRACK, LEFT, false).await {
            ServerMsg::Subbed { value, .. } => {
                let v = value.and_then(|v| v.as_f64()).unwrap();
                assert!((v - want).abs() < 1e-6, "{v}");
            }
            other => panic!("not subbed: {other:?}"),
        }
        // The device stops: Live's own values come back once it is stale.
        assert_eq!(host.maxdev("level track 1 off"), 0);
        client
            .value_until(&left, Duration::from_millis(1500), |i| {
                i.value
                    .as_ref()
                    .and_then(Value::as_f64)
                    .is_some_and(|v| (v - want).abs() > 0.01)
            })
            .await;
        hub.stop().await;
        host.stop();
    });
}
