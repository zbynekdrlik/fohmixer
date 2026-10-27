//! A TCP proxy whose downlink (the hub's bytes to the client) can be slowed
//! (#71, the PR I review): a page on a link that carries fewer of the hub's
//! frames than the hub would write. The client's bytes to the hub pass at
//! once. The downlink passes in pieces of [`PIECE`] bytes, each when a link
//! of the set rate would have carried it; what waits stays in the hub's
//! socket, so the hub's writer meets the slow link as on a real one.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// The pieces the downlink is cut into (bytes).
const PIECE: usize = 1024;

/// The proxy: its address and the downlink's rate.
pub struct Throttle {
    pub addr: SocketAddr,
    rate: Arc<AtomicU64>,
    task: JoinHandle<()>,
}

impl Throttle {
    /// Listens on a free local port and proxies each connection to
    /// `upstream`; the downlink is not slowed until [`Throttle::rate`].
    pub async fn start(upstream: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let rate = Arc::new(AtomicU64::new(0));
        let theirs = Arc::clone(&rate);
        let task = tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                tokio::spawn(proxy(client, upstream, Arc::clone(&theirs)));
            }
        });
        Self { addr, rate, task }
    }

    /// The downlink carries at most `bytes_per_s` from now on (0: no
    /// limit).
    pub fn rate(&self, bytes_per_s: u64) {
        self.rate.store(bytes_per_s, Ordering::Relaxed);
    }

    /// The hub's WebSocket URL through the proxy.
    pub fn ws_url(&self, token: &str) -> String {
        format!("ws://{}/ws?token={token}&proto=2", self.addr)
    }
}

impl Drop for Throttle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// One connection: the uplink copied as it comes, the downlink paced.
async fn proxy(client: TcpStream, upstream: SocketAddr, rate: Arc<AtomicU64>) {
    let Ok(hub) = TcpStream::connect(upstream).await else {
        return;
    };
    let (mut client_rd, mut client_wr) = client.into_split();
    let (mut hub_rd, mut hub_wr) = hub.into_split();
    let up = tokio::spawn(async move {
        let _ = tokio::io::copy(&mut client_rd, &mut hub_wr).await;
    });
    let mut piece = vec![0_u8; PIECE];
    let mut next = tokio::time::Instant::now();
    loop {
        let n = match hub_rd.read(&mut piece).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let bytes_per_s = rate.load(Ordering::Relaxed);
        if bytes_per_s > 0 {
            next = next.max(tokio::time::Instant::now())
                + Duration::from_secs_f64(n as f64 / bytes_per_s as f64);
            tokio::time::sleep_until(next).await;
        }
        if client_wr.write_all(&piece[..n]).await.is_err() {
            break;
        }
    }
    up.abort();
}
