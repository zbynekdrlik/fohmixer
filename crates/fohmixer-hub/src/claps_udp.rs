//! The `FohMixer Claps` device's hello (#220): one UDP socket on
//! `127.0.0.1:<[claps] port>` (no config widens the address). Each hello
//! (`fohmixer_proto::claps::parse_hello`, pure) goes to the router as
//! `RouterMsg::ClapsHello`; anything else is dropped with a debug line. The
//! router itself sends the device its words (`router/claps.rs`), and its
//! tick ([`ticks`]) every `SEND_EVERY_MS`.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use fohmixer_proto::claps::{MAX_DATAGRAM, SEND_EVERY_MS, parse_hello};
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::router::RouterMsg;

/// The only address the listener binds: `127.0.0.1:<port>`.
pub fn bind_addr(port: u16) -> SocketAddrV4 {
    SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)
}

/// The pause after a receive error (never a spinning task).
pub const ERROR_PAUSE: Duration = Duration::from_millis(10);

/// One datagram's way: a hello to the router; false once the router is
/// gone.
pub fn handle_datagram(data: &[u8], tx: &mpsc::UnboundedSender<RouterMsg>) -> bool {
    match parse_hello(data) {
        Ok(hello) => tx.send(RouterMsg::ClapsHello { hello }).is_ok(),
        Err(e) => {
            tracing::debug!(reason = e.reason(), "claps: a datagram that is no hello");
            true
        }
    }
}

/// Binds `127.0.0.1:<port>` and starts the listener (inside a tokio
/// runtime); the router is told where it listens, or why it does not (the
/// hub runs on without it).
pub fn spawn(
    port: u16,
    tx: mpsc::UnboundedSender<RouterMsg>,
) -> std::io::Result<(SocketAddr, JoinHandle<()>)> {
    let wanted = bind_addr(port);
    let bound = std::net::UdpSocket::bind(wanted).and_then(|socket| {
        socket.set_nonblocking(true)?;
        UdpSocket::from_std(socket)
    });
    let socket = match bound {
        Ok(socket) => socket,
        Err(e) => {
            let _ = tx.send(RouterMsg::ClapsListen {
                addr: wanted.to_string(),
                error: Some(e.to_string()),
            });
            return Err(e);
        }
    };
    let addr = socket.local_addr()?;
    let _ = tx.send(RouterMsg::ClapsListen {
        addr: addr.to_string(),
        error: None,
    });
    Ok((addr, tokio::spawn(listen(socket, tx))))
}

async fn listen(socket: UdpSocket, tx: mpsc::UnboundedSender<RouterMsg>) {
    let mut buf = vec![0u8; MAX_DATAGRAM + 1];
    loop {
        match socket.recv(&mut buf).await {
            Ok(n) => {
                if !handle_datagram(&buf[..n], &tx) {
                    return;
                }
            }
            Err(e) => {
                tracing::debug!(error = %e, "claps: a receive failed");
                tokio::time::sleep(ERROR_PAUSE).await;
            }
        }
    }
}

/// The claps' tick: a [`RouterMsg::ClapsTick`] every `SEND_EVERY_MS`; it
/// ends with the router's channel.
pub async fn ticks(router: mpsc::UnboundedSender<RouterMsg>) {
    let period = Duration::from_millis(SEND_EVERY_MS);
    let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if router.send(RouterMsg::ClapsTick).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::claps::{Hello, Word, encode, encode_hello};

    #[test]
    fn a_hello_goes_to_the_router_and_anything_else_is_dropped() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let hello = Hello {
            version: "v1.52.0".into(),
            running: true,
        };
        assert!(handle_datagram(&encode_hello(&hello), &tx));
        assert!(handle_datagram(&encode(&Word::On(true)), &tx));
        assert!(handle_datagram(b"junk", &tx));
        match rx.try_recv() {
            Ok(RouterMsg::ClapsHello { hello: got }) => assert_eq!(got, hello),
            _ => panic!("a hello"),
        }
        assert!(rx.try_recv().is_err(), "nothing else");
        drop(rx);
        assert!(
            !handle_datagram(&encode_hello(&hello), &tx),
            "the router is gone"
        );
    }

    #[test]
    fn it_binds_loopback_only() {
        assert_eq!(bind_addr(39113).to_string(), "127.0.0.1:39113");
    }

    #[tokio::test]
    async fn the_listener_hands_on_a_hello_and_says_where_it_listens() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (addr, task) = spawn(0, tx.clone()).expect("a free port");
        match rx.recv().await {
            Some(RouterMsg::ClapsListen { addr: said, error }) => {
                assert_eq!(said, addr.to_string());
                assert_eq!(error, None);
            }
            _ => panic!("where it listens"),
        }
        let device = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let hello = Hello {
            version: "v1".into(),
            running: false,
        };
        device.send_to(&encode_hello(&hello), addr).unwrap();
        match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
            Ok(Some(RouterMsg::ClapsHello { hello: got })) => assert_eq!(got, hello),
            _ => panic!("the hello"),
        }
        // A taken port is told, never fatal.
        let taken = spawn(addr.port(), tx);
        assert!(taken.is_err());
        match rx.recv().await {
            Some(RouterMsg::ClapsListen { error: Some(_), .. }) => {}
            _ => panic!("why it does not listen"),
        }
        task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn the_tick_comes_every_second_and_ends_with_the_router() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(ticks(tx));
        tokio::time::sleep(Duration::from_millis(SEND_EVERY_MS - 1)).await;
        assert!(rx.try_recv().is_err(), "none before a period");
        tokio::time::sleep(Duration::from_millis(2)).await;
        assert!(matches!(rx.try_recv(), Ok(RouterMsg::ClapsTick)));
        drop(rx);
        tokio::time::sleep(Duration::from_millis(SEND_EVERY_MS)).await;
        assert!(task.is_finished());
    }
}
