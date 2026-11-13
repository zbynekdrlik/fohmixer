//! The instance link end to end (#184, spec §6.1): two hubs, one MBC Live.
//! The MBC hub holds a SimLive `mbc` (the MBC set, `sim/fixtures/mbc-site.json`)
//! and serves `[link]`; the FOH hub holds a SimLive `band` and reaches `mbc`
//! through the link. A fader moved on either hub shows on both; the FOH
//! hub reads `mbc` offline while the MBC hub is gone and comes back by
//! itself; a wrong secret keeps it offline with the reason; the MBC LUFS
//! readings reach the FOH page; the MBC hub works with no FOH hub at all.
#![cfg(unix)]

use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::support::{Host, TestHub, repo, runtime, serial};
use fohmixer_hub::config::{Config, InstanceCfg, LinkCfg};
use fohmixer_hub::link::LinkSecret;
use fohmixer_proto::client::ServerMsg;
use serde_json::{Value, json};

const SECS_5: Duration = Duration::from_secs(5);
const SECS_10: Duration = Duration::from_secs(10);
const MBC_VOLUME: &str = "live_set tracks[name=Hand 1 mbc#] mixer_device volume";
const BAND_VOLUME: &str = "live_set tracks[name=Hand1 #] mixer_device volume";

fn mbc_site() -> PathBuf {
    repo().join("sim").join("fixtures").join("mbc-site.json")
}

fn secret(byte: u8) -> LinkSecret {
    LinkSecret::from_bytes([byte; 32])
}

fn free_udp_port() -> u16 {
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.local_addr().unwrap().port()
}

/// The MBC hub: its local `mbc`, the link on `listen` (port 0: any), its
/// LUFS listener on `lufs_port` (0: none), the link secret `secret`.
async fn mbc_hub(
    dir: &Path,
    mbc: &Host,
    listen: SocketAddr,
    lufs_port: u16,
    secret: &LinkSecret,
) -> TestHub {
    fohmixer_hub::link::store(dir, secret).unwrap();
    let mut config = Config::defaults(dir);
    config.instances = vec![mbc.cfg()];
    config.layout_poll_ms = 100;
    config.lufs.port = lufs_port;
    config.link = Some(LinkCfg {
        listen,
        instance: "mbc".to_string(),
    });
    TestHub::start_config(config).await
}

/// The FOH hub: its local `band` and `mbc` linked at `link`.
async fn foh_hub(dir: &Path, band: &Host, link: SocketAddr, secret: &LinkSecret) -> TestHub {
    fohmixer_hub::link::store(dir, secret).unwrap();
    let mut config = Config::defaults(dir);
    config.instances = vec![
        band.cfg(),
        InstanceCfg {
            name: "mbc".to_string(),
            port: 0,
            link: Some(format!("ws://{link}")),
        },
    ];
    config.layout_poll_ms = 100;
    config.lufs.port = 0;
    TestHub::start_config(config).await
}

/// The address the MBC hub's link serves on.
async fn link_addr(hub: &TestHub) -> SocketAddr {
    let status = hub.status().await;
    status
        .link
        .expect("the MBC hub serves a link")
        .listen
        .parse()
        .unwrap()
}

fn online(status: &fohmixer_proto::client::HubStatus, name: &str) -> bool {
    status.instances.iter().any(|i| i.name == name && i.online)
}

fn any_port() -> SocketAddr {
    "127.0.0.1:0".parse().unwrap()
}

#[test]
fn a_fader_moved_on_either_hub_shows_on_both() {
    let _serial = serial();
    runtime().block_on(async {
        let (mbc_dir, foh_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let mbc = Host::start_site("mbc", &mbc_site(), 0, 0.0);
        let band = Host::start("band");
        let mbc_hub = mbc_hub(mbc_dir.path(), &mbc, any_port(), 0, &secret(1)).await;
        let foh = foh_hub(foh_dir.path(), &band, link_addr(&mbc_hub).await, &secret(1)).await;
        foh.status_until(SECS_10, |s| online(s, "mbc") && online(s, "band"))
            .await;
        let link = mbc_hub.status().await.link.unwrap();
        assert_eq!(link.links, 1);
        assert!(link.peer.is_some());
        let mut on_foh = foh.client().await;
        let mut on_mbc = mbc_hub.client().await;
        // The FOH hub tells its pages which Live is linked (its foot leaves
        // it out); the MBC hub's own mbc is local.
        let linked = |client_msg: &ServerMsg, want: &str| match client_msg {
            ServerMsg::Instance { name, linked, .. } if name == want => Some(*linked),
            _ => None,
        };
        assert!(on_foh.wait(SECS_5, |m| linked(m, "mbc")).await);
        assert!(!on_foh.wait(SECS_5, |m| linked(m, "band")).await);
        assert!(!on_mbc.wait(SECS_5, |m| linked(m, "mbc")).await);
        let foh_key = on_foh.sub_key("mbc", MBC_VOLUME, "value", false).await;
        let mbc_key = on_mbc.sub_key("mbc", MBC_VOLUME, "value", false).await;
        // Moved on the MBC hub: the FOH page sees it.
        on_mbc.set("mbc", MBC_VOLUME, "value", json!(0.42)).await;
        on_foh
            .value_until(&foh_key, SECS_5, |i| i.value == Some(json!(0.42)))
            .await;
        // Moved on the FOH hub (through the link): the MBC page sees it.
        on_foh.set("mbc", MBC_VOLUME, "value", json!(0.61)).await;
        on_mbc
            .value_until(&mbc_key, SECS_5, |i| i.value == Some(json!(0.61)))
            .await;
        assert_eq!(on_mbc.get("mbc", MBC_VOLUME, "value").await, json!(0.61));
        foh.stop().await;
        mbc_hub.stop().await;
    });
}

#[test]
fn the_foh_hub_reads_mbc_offline_when_the_mbc_hub_stops_and_it_comes_back_by_itself() {
    let _serial = serial();
    runtime().block_on(async {
        let (mbc_dir, foh_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let mbc = Host::start_site("mbc", &mbc_site(), 0, 0.0);
        let band = Host::start("band");
        let first = mbc_hub(mbc_dir.path(), &mbc, any_port(), 0, &secret(1)).await;
        let addr = link_addr(&first).await;
        let foh = foh_hub(foh_dir.path(), &band, addr, &secret(1)).await;
        foh.status_until(SECS_10, |s| online(s, "mbc")).await;
        let mut page = foh.client().await;
        first.stop().await;
        // Offline within the backoff; band is untouched.
        page.instance_state("mbc", false, None, SECS_5).await;
        let status = foh.status_until(SECS_5, |s| !online(s, "mbc")).await;
        assert!(online(&status, "band"));
        page.set("band", BAND_VOLUME, "value", json!(0.5)).await;
        assert_eq!(page.get("band", BAND_VOLUME, "value").await, json!(0.5));
        // The MBC hub back on the same address: online by itself.
        let again = mbc_hub(mbc_dir.path(), &mbc, addr, 0, &secret(1)).await;
        foh.status_until(SECS_10, |s| online(s, "mbc")).await;
        assert_eq!(page.get("mbc", MBC_VOLUME, "value").await, json!(0.85));
        foh.stop().await;
        again.stop().await;
    });
}

#[test]
fn a_foh_hub_with_another_secret_stays_offline_with_the_reason() {
    let _serial = serial();
    runtime().block_on(async {
        let (mbc_dir, foh_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let mbc = Host::start_site("mbc", &mbc_site(), 0, 0.0);
        let band = Host::start("band");
        let mbc_hub = mbc_hub(mbc_dir.path(), &mbc, any_port(), 0, &secret(1)).await;
        let foh = foh_hub(foh_dir.path(), &band, link_addr(&mbc_hub).await, &secret(2)).await;
        let status = foh
            .status_until(SECS_10, |s| {
                s.instances.iter().any(|i| {
                    i.name == "mbc" && i.last_error.as_deref().is_some_and(|e| e.contains("401"))
                })
            })
            .await;
        assert!(!online(&status, "mbc"));
        foh.status_until(SECS_5, |s| online(s, "band")).await;
        let link = mbc_hub
            .status_until(SECS_5, |s| s.link.as_ref().is_some_and(|l| l.refusals > 0))
            .await
            .link
            .unwrap();
        assert_eq!(link.links, 0);
        assert!(
            link.last_refusal
                .as_deref()
                .is_some_and(|r| r.starts_with("not a token of the link secret")),
            "{link:?}"
        );
        let refused = mbc_hub
            .events_until(SECS_5, |r| {
                r.iter()
                    .any(|e| e["ev"] == "instance_link" && e["what"] == "refused")
            })
            .await;
        assert!(
            refused
                .iter()
                .all(|e| e["ev"] != "instance_link" || e["what"] != "connected")
        );
        foh.stop().await;
        mbc_hub.stop().await;
    });
}

#[test]
fn an_mbc_lufs_reading_reaches_the_foh_page() {
    let _serial = serial();
    runtime().block_on(async {
        let (mbc_dir, foh_dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let lufs_port = free_udp_port();
        let mut mbc = Host::start_lufs("mbc", &mbc_site(), lufs_port);
        let band = Host::start("band");
        // The FOH frame places band's VOCALS only: MBC is a pool-only view.
        let frame = json!({"schema": 2, "default_page": "foh", "pages": [{"id": "foh",
            "title": "FOH", "rows": [{"sections": [
                {"kind": "group", "id": "vocals", "title": "VOCALS", "tags": "VOCALS"}]}]}]});
        std::fs::write(foh_dir.path().join("layout.json"), frame.to_string()).unwrap();
        let mbc_hub = mbc_hub(mbc_dir.path(), &mbc, any_port(), lufs_port, &secret(1)).await;
        let foh = foh_hub(foh_dir.path(), &band, link_addr(&mbc_hub).await, &secret(1)).await;
        foh.status_until(SECS_10, |s| online(s, "mbc")).await;
        assert_eq!(
            mbc.tuner("track", 0, "set", r#""Hand 1" +G:MBC:1 +LUFS:-14"#),
            1
        );
        let status = foh
            .status_until(SECS_10, |s| {
                s.lufs.strips.iter().any(|p| p.instance == "mbc")
            })
            .await;
        assert!(status.layout.markers.problems.is_empty(), "{status:?}");
        assert_eq!(mbc.maxdev("maxdev lufs track 0 add rack"), 1);
        assert_eq!(mbc.maxdev("lufs track 0 -230 -230"), 1);
        let mut page = foh.client().await;
        let item = page
            .wait(SECS_10, |m| match m {
                ServerMsg::Lufs { items } => items
                    .iter()
                    .find(|i| i.instance == "mbc" && i.tenths == Some(-230))
                    .cloned(),
                _ => None,
            })
            .await;
        assert_eq!(item.problem, None);
        let status = foh.status().await;
        assert!(status.lufs.listener.taken > 0, "counted as the hub's own");
        let device = status
            .lufs
            .devices
            .iter()
            .find(|d| d.instance.as_deref() == Some("mbc"))
            .expect("the MBC device matched on the FOH hub");
        assert_eq!(device.state, "matched");
        foh.stop().await;
        mbc_hub.stop().await;
    });
}

#[test]
fn the_mbc_hub_runs_without_the_foh_hub() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mbc = Host::start_site("mbc", &mbc_site(), 0, 0.0);
        let hub = mbc_hub(dir.path(), &mbc, any_port(), 0, &secret(1)).await;
        hub.status_until(SECS_10, |s| online(s, "mbc")).await;
        let mut page = hub.client().await;
        page.set("mbc", MBC_VOLUME, "value", json!(0.33)).await;
        assert_eq!(page.get("mbc", MBC_VOLUME, "value").await, json!(0.33));
        let link: Value = serde_json::to_value(hub.status().await.link).unwrap();
        assert_eq!(link["links"], 0);
        assert_eq!(link["peer"], Value::Null);
        hub.stop().await;
    });
}
