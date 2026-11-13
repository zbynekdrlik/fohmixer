//! The roles on a socket (#184), end to end against three SimLive hosts
//! (band, master and mbc, each the synthetic test site): an MBC login moves
//! only the Live set mbc's faders and is sent nothing of band or master;
//! the engineer's login is unchanged.
#![cfg(unix)]

use std::time::{Duration, Instant};

use crate::support::{Host, TestHub, runtime, serial};
use fohmixer_proto::client::{ClientMsg, ServerMsg};
use fohmixer_proto::roles::Role;
use serde_json::json;

const VOLUME: &str = "live_set tracks[name=Hand1 #] mixer_device volume";
const SECS_5: Duration = Duration::from_secs(5);

/// Three hosts and a hub over them.
async fn three(dir: &std::path::Path) -> (Vec<Host>, TestHub) {
    let hosts: Vec<Host> = ["band", "master", "mbc"]
        .into_iter()
        .map(Host::start)
        .collect();
    let hub = TestHub::start(hosts.iter().map(Host::cfg).collect(), dir).await;
    (hosts, hub)
}

#[test]
fn an_mbc_socket_cannot_move_a_band_fader() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let (_hosts, hub) = three(dir.path()).await;
        let mut engineer = hub.client().await;
        engineer
            .instance_state("band", true, Some(false), SECS_5)
            .await;
        engineer.set("band", VOLUME, "value", json!(0.4)).await;
        let mut mbc = hub.client_as(Role::Mbc).await;
        assert!(
            matches!(
                &mbc.hello,
                ServerMsg::Hello {
                    role: Some(Role::Mbc),
                    ..
                }
            ),
            "{:?}",
            mbc.hello
        );
        mbc.write("band", VOLUME, "value", json!(0.9), 1, true)
            .await;
        // The write is answered by its own failed ack: the page settles it.
        let ack = mbc
            .ack_of(
                &fohmixer_proto::client::set_key("band", VOLUME, "value"),
                1,
                SECS_5,
            )
            .await;
        assert_eq!(ack.error.as_deref(), Some("role"), "{ack:?}");
        // A command to band is refused too, with its id.
        assert_eq!(
            mbc.cmd(
                "band",
                vec![json!({"target": VOLUME, "name": "get_prop", "args": {"prop": "value"}})]
            )
            .await,
            Err("role".to_string())
        );
        // Live's value stays the engineer's.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(engineer.get("band", VOLUME, "value").await, json!(0.4));
        hub.stop().await;
    });
}

#[test]
fn an_mbc_socket_sees_no_band_or_master_state() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let (_hosts, hub) = three(dir.path()).await;
        let mut engineer = hub.client().await;
        for name in ["band", "master", "mbc"] {
            engineer
                .instance_state(name, true, Some(false), SECS_5)
                .await;
        }
        let mut mbc = hub.client_as(Role::Mbc).await;
        // A sub of band is refused; a sub of mbc is answered.
        mbc.send(&ClientMsg::Sub {
            instance: "band".into(),
            target: VOLUME.into(),
            prop: "value".into(),
            display: false,
        })
        .await;
        mbc.send(&ClientMsg::Sub {
            instance: "mbc".into(),
            target: VOLUME.into(),
            prop: "value".into(),
            display: false,
        })
        .await;
        // The engineer moves band, master and mbc meanwhile.
        engineer.set("band", VOLUME, "value", json!(0.3)).await;
        engineer.set("master", VOLUME, "value", json!(0.3)).await;
        engineer.set("mbc", VOLUME, "value", json!(0.3)).await;
        let until = Instant::now() + Duration::from_secs(2);
        let mut seen = Vec::new();
        while let Some(msg) = mbc
            .next_message(until.saturating_duration_since(Instant::now()))
            .await
        {
            seen.push(msg);
        }
        let text = serde_json::to_string(&seen).unwrap();
        for word in ["\"band\"", "\"master\"", "band|", "master|"] {
            assert!(!text.contains(word), "{word} in {text}");
        }
        assert!(
            !seen.iter().any(|m| matches!(
                m,
                ServerMsg::Hub { .. }
                    | ServerMsg::Reverb(_)
                    | ServerMsg::Autotune(_)
                    | ServerMsg::Rec(_)
            )),
            "{text}"
        );
        assert!(
            seen.iter()
                .any(|m| matches!(m, ServerMsg::Instance { name, .. } if name == "mbc")),
            "{text}"
        );
        assert!(
            seen.iter()
                .any(|m| matches!(m, ServerMsg::Error { message, .. } if message == "role")),
            "{text}"
        );
        assert!(
            seen.iter()
                .any(|m| matches!(m, ServerMsg::Subbed { sub, .. } if sub.starts_with("mbc|"))),
            "{text}"
        );
        hub.stop().await;
    });
}

#[test]
fn an_mbc_socket_moves_an_mbc_fader() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let (_hosts, hub) = three(dir.path()).await;
        let mut engineer = hub.client().await;
        engineer
            .instance_state("mbc", true, Some(false), SECS_5)
            .await;
        let mut mbc = hub.client_as(Role::Mbc).await;
        mbc.write("mbc", VOLUME, "value", json!(0.6), 1, true).await;
        let ack = mbc
            .ack_of(
                &fohmixer_proto::client::set_key("mbc", VOLUME, "value"),
                1,
                SECS_5,
            )
            .await;
        assert_eq!(ack.error, None, "{ack:?}");
        assert_eq!(engineer.get("mbc", VOLUME, "value").await, json!(0.6));
        hub.stop().await;
    });
}
