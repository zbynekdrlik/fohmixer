//! Listen's device side without SimLive (#210): bound to 127.0.0.1 only,
//! an announce kept, junk and every stream a device may not send counted by
//! reason, at most eight audio connections, `/ws/audio` refused without a
//! token and when Listen is off. Host-free: it also runs in the `windows`
//! job (the hub's platform, its libopus build).

use std::io::{Read, Write};
use std::net::{TcpStream, UdpSocket};
use std::path::Path;
use std::time::Duration;

use crate::support::{TestHub, runtime, serial};
use fohmixer_hub::config::Config;
use fohmixer_proto::client::HubStatus;
use fohmixer_proto::listen::jitnet::{Order, encode_matrix, encode_message};
use fohmixer_proto::listen::wire::{Announce, Hello, encode_announce, hello_atoms, parents_in};

const SECS_10: Duration = Duration::from_secs(10);

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

async fn hub(dir: &Path, port: u16) -> TestHub {
    let mut config = Config::defaults(dir);
    config.instances.clear();
    config.lufs.port = 0;
    config.listen.port = port;
    TestHub::start_config(config).await
}

fn port_of(status: &HubStatus) -> u16 {
    let addr = status.listen.addr.clone().expect("Listen is bound");
    assert!(addr.starts_with("127.0.0.1:"), "{addr}");
    addr.rsplit(':').next().unwrap().parse().unwrap()
}

fn hello(rate: i32) -> Vec<u8> {
    encode_message(
        Order::Big,
        &hello_atoms(&Hello {
            device: 5,
            version: "v1.50.0".into(),
            sample_rate: rate,
        }),
    )
}

fn matrix() -> Vec<u8> {
    encode_matrix(Order::Big, 2, 960, 1, &[0.0; 1920], 1.0)
}

async fn refused_until(hub: &TestHub, reason: &str, n: u64) -> HubStatus {
    hub.status_until(SECS_10, |s| s.listen.refused.get(reason) == Some(&n))
        .await
}

#[test]
fn a_device_side_that_may_not_stream_is_counted_by_its_reason() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = hub(dir.path(), free_port()).await;
        let port = port_of(&hub.status().await);

        let announce = encode_announce(&Announce {
            device: 5,
            version: "v1.50.0".into(),
            parents: parents_in(None, "Vox 1", "FOH Band"),
            sample_rate: 48_000,
            sending: false,
        });
        let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
        udp.send_to(&announce, ("127.0.0.1", port)).unwrap();
        udp.send_to(b"junk", ("127.0.0.1", port)).unwrap();
        let status = hub
            .status_until(SECS_10, |s| s.listen.devices.len() == 1)
            .await;
        assert_eq!(status.listen.devices[0].track, "Vox 1");
        refused_until(&hub, "announce_truncated", 1).await;

        // A matrix before the hello; at 44.1 kHz; not selected.
        let mut tcp = TcpStream::connect(("127.0.0.1", port)).unwrap();
        tcp.write_all(&matrix()).unwrap();
        refused_until(&hub, "no_hello", 1).await;
        // Each matrix is answered with a latency chunk.
        let mut answer = [0u8; 32];
        tcp.set_read_timeout(Some(SECS_10)).unwrap();
        tcp.read_exact(&mut answer).unwrap();
        assert_eq!(&answer[..4], b"JMLP");
        assert_eq!(&answer[8..16], &1.0f64.to_be_bytes());
        tcp.write_all(&hello(44_100)).unwrap();
        tcp.write_all(&matrix()).unwrap();
        refused_until(&hub, "sample_rate", 1).await;
        tcp.write_all(&hello(48_000)).unwrap();
        tcp.write_all(&matrix()).unwrap();
        let status = refused_until(&hub, "not_selected", 1).await;
        assert_eq!(status.listen.frames, 0);
        assert_eq!(status.listen.connections, 1);
        // Junk ends the connection.
        tcp.write_all(b"ABCDEFGH").unwrap();
        let status = refused_until(&hub, "chunk_id", 1).await;
        let status = if status.listen.connections == 0 {
            status
        } else {
            hub.status_until(SECS_10, |s| s.listen.connections == 0)
                .await
        };
        assert_eq!(status.listen.target, None);
        hub.stop().await;
    });
}

#[test]
fn at_most_eight_audio_connections() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let hub = hub(dir.path(), free_port()).await;
        let port = port_of(&hub.status().await);
        let held: Vec<TcpStream> = (0..8)
            .map(|_| TcpStream::connect(("127.0.0.1", port)).unwrap())
            .collect();
        hub.status_until(SECS_10, |s| s.listen.connections == 8)
            .await;
        let mut ninth = TcpStream::connect(("127.0.0.1", port)).unwrap();
        refused_until(&hub, "connections", 1).await;
        ninth.set_read_timeout(Some(SECS_10)).unwrap();
        let mut buf = [0u8; 1];
        assert!(matches!(ninth.read(&mut buf), Ok(0) | Err(_)), "closed");
        drop(held);
        hub.status_until(SECS_10, |s| s.listen.connections == 0)
            .await;
        hub.stop().await;
    });
}

#[test]
fn the_audio_socket_needs_a_token_and_listen_on() {
    let _serial = serial();
    runtime().block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let on = hub(dir.path(), free_port()).await;
        for url in [
            format!("ws://{}/ws/audio", on.addr),
            format!("ws://{}/ws/audio?token=nope", on.addr),
        ] {
            let refused = tokio_tungstenite::connect_async(url).await.unwrap_err();
            assert!(refused.to_string().contains("401"), "{refused}");
        }
        on.stop().await;
        let dir = tempfile::tempdir().unwrap();
        let off = hub(dir.path(), 0).await;
        let status = off.status().await;
        assert_eq!(status.listen, Default::default(), "off: nothing bound");
        let url = format!("ws://{}/ws/audio?token={}", off.addr, off.token);
        let refused = tokio_tungstenite::connect_async(url).await.unwrap_err();
        assert!(refused.to_string().contains("503"), "{refused}");
        off.stop().await;
    });
}
