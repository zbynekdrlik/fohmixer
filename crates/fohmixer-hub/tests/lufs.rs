//! LUFS end to end (#151, spec §4.4, §5): SimLive's Max devices send their
//! datagrams to the hub's listener; the hub matches each key to the
//! `+LUFS` strip of the Live that holds the device (its own walk of the
//! track), and serves the reading to the pages.
#![cfg(unix)]

mod support;

use std::net::UdpSocket;
use std::path::{Path, PathBuf};
use std::time::Duration;

use fohmixer_hub::config::Config;
use fohmixer_proto::client::{HubStatus, LufsItem, ServerMsg};
use fohmixer_proto::lufs::{LufsProblem, LufsState};
use fohmixer_proto::volume::volume_of;
use serde_json::{Value, json};
use support::{Client, Host, TestHub, repo, runtime, serial};

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
        assert_eq!(host.maxdev("lufs track 1 -300 -253"), 1);
        let mut client = hub.client().await;
        let got = item(&mut client, "band", SECS_5, |i| i.tenths.is_some()).await;
        assert_eq!(got.tenths, Some(-253), "an input point shows V");
        assert_eq!(got.problem, None);
        let status = hub
            .status_until(SECS_5, |s| s.lufs.strips[0].state == LufsState::Fine)
            .await;
        let device = &status.lufs.devices[0];
        assert_eq!(device.state, "matched");
        assert_eq!(device.instance.as_deref(), Some("band"));
        assert_eq!(device.parents, vec!["LUFS", "LUFS", "Hand2 #", "Test Site"]);
        assert_eq!(device.version, "v1.19.0");
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
        assert_eq!(band.maxdev("lufs track 1 -300 -253"), 1);
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
        assert_eq!(band.maxdev("lufs track 1 -300 -253"), 1);
        let mut client = hub.client().await;
        for instance in ["band", "master"] {
            let got = item(&mut client, instance, SECS_5, |i| {
                i.problem == Some(LufsProblem::TwoLufsMeters)
            })
            .await;
            assert_eq!(got.tenths, None);
        }
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
