//! Both listeners' sockets (#90 B3h, follow-up 3): axum-server's accept
//! loop tries a failed accept again every 50 ms and logs nothing, so a
//! listener that keeps failing (out of file handles, …) was silent.
//! [`Logged`] wraps the bound socket: a failed accept is logged at warn at
//! most once per [`crate::ration::GAP`], with the count left out, then
//! handed back (axum-server's retry is unchanged). [`Peer`] is the address
//! that names it to axum-server; a request still sees its peer as a
//! `ConnectInfo<SocketAddr>`, so the Access check is unchanged.

use std::io;
use std::net::SocketAddr;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::extract::connect_info::Connected;
use axum_server::{AddrListener, Address, Server};
use tokio::net::{TcpListener, TcpStream};

use crate::ration::Ration;

/// A connection's peer, as a [`Logged`] socket accepted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Peer(pub SocketAddr);

impl Address for Peer {
    type Stream = TcpStream;
    type Listener = Logged;
}

/// The application sees the peer as before: `ConnectInfo<SocketAddr>`.
impl Connected<Peer> for SocketAddr {
    fn connect_info(peer: Peer) -> Self {
        peer.0
    }
}

/// A bound socket whose failed accepts are logged at a limited rate.
pub struct Logged {
    listener: TcpListener,
    ration: Mutex<Ration>,
}

impl Logged {
    /// `listener`, its failed accepts logged.
    pub fn new(listener: TcpListener) -> Self {
        Self {
            listener,
            ration: Mutex::default(),
        }
    }

    /// A failed accept at `now`: a warn line when its ration allows (one per
    /// `ration::GAP`, with the failures left out since the last one), else a
    /// debug line.
    pub fn failed(&self, error: &io::Error, now: Instant) {
        let mut ration = self.ration.lock().unwrap_or_else(PoisonError::into_inner);
        match ration.take(now, 1) {
            Some(skipped) => tracing::warn!(
                error = %error,
                skipped,
                "a connection could not be accepted (tried again every 50 ms)"
            ),
            None => tracing::debug!(error = %error, "a connection could not be accepted"),
        }
    }
}

impl AddrListener<TcpStream, Peer> for Logged {
    async fn bind_to(addr: Peer) -> io::Result<Self> {
        TcpListener::bind(addr.0).await.map(Self::new)
    }

    async fn accept_stream(&self) -> io::Result<(TcpStream, Peer)> {
        match self.listener.accept().await {
            Ok((stream, peer)) => Ok((stream, Peer(peer))),
            Err(error) => {
                self.failed(&error, Instant::now());
                Err(error)
            }
        }
    }

    fn get_local_addr(&self) -> io::Result<Peer> {
        self.listener.local_addr().map(Peer)
    }
}

/// The server of a bound socket, as both listeners run it: its failed
/// accepts logged ([`Logged`]), a request head within `header_read`
/// (`crate::http1_limits`).
pub fn server(listener: TcpListener, header_read: Duration) -> Server<Peer> {
    let mut server = Server::from_listener(Logged::new(listener));
    crate::http1_limits(server.http_builder(), header_read);
    server
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ws::tests::logged;

    #[tokio::test]
    async fn a_failed_accept_is_logged_at_warn_once_per_gap_with_the_count_left_out() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socket = Logged::new(listener);
        let error = io::Error::other("too many open files");
        let t = Instant::now();
        let ms = Duration::from_millis;
        let log = logged(|| {
            for at in [ms(0), ms(50), ms(100), crate::ration::GAP + ms(1)] {
                socket.failed(&error, t + at);
            }
        });
        let warned: Vec<&str> = log.lines().filter(|l| l.contains(" WARN ")).collect();
        assert_eq!(warned.len(), 2, "{log}");
        assert!(warned[0].contains("error=too many open files"), "{log}");
        assert!(warned[0].contains("skipped=0"), "{log}");
        assert!(warned[1].contains("skipped=2"), "{log}");
    }

    #[tokio::test]
    async fn a_logged_socket_binds_accepts_and_names_the_peer() {
        let socket = Logged::bind_to(Peer("127.0.0.1:0".parse().unwrap()))
            .await
            .unwrap();
        let at = socket.get_local_addr().unwrap().0;
        assert_ne!(at.port(), 0);
        let client = TcpStream::connect(at).await.unwrap();
        let (_, peer) = socket.accept_stream().await.unwrap();
        assert_eq!(peer, Peer(client.local_addr().unwrap()));
        assert_eq!(<SocketAddr as Connected<Peer>>::connect_info(peer), peer.0);
    }
}
