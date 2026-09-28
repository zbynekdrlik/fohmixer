//! The HTTPS listener of the `[tls]` name (#17), iemmixer's axum-server
//! rustls listener: bound at the start next to the plain-HTTP one (a taken
//! port costs HTTPS only: `lib.rs` `start_https` tries it again), serving
//! from the first certificate on (the stored one at the start, else the
//! first the ACME client gets), a renewed certificate swapped in without a
//! restart, and stopped with the HTTP listener (graceful, open requests get
//! the same drain).

use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Router;
use axum_server::Handle;
use axum_server::tls_rustls::RustlsConfig;
use tokio::task::JoinHandle;

/// The listener: bound, serving once it has a certificate.
pub struct Https {
    addr: SocketAddr,
    app: Router,
    /// The bound socket until the first certificate starts the server.
    unused: Mutex<Option<std::net::TcpListener>>,
    /// The served configuration (`None` until the first certificate).
    config: Mutex<Option<RustlsConfig>>,
    handle: Handle<SocketAddr>,
    task: Mutex<Option<JoinHandle<io::Result<()>>>>,
}

impl Https {
    /// Binds `addr` (port 0: any): the socket for [`Https::new`].
    pub fn listen(addr: SocketAddr) -> io::Result<std::net::TcpListener> {
        let listener = std::net::TcpListener::bind(addr)?;
        listener.set_nonblocking(true)?;
        Ok(listener)
    }

    /// Binds `addr` for `app` (tests).
    pub fn bind(addr: SocketAddr, app: Router) -> io::Result<Self> {
        Self::new(Self::listen(addr)?, app)
    }

    /// The listener on the bound `listener` for `app`.
    pub fn new(listener: std::net::TcpListener, app: Router) -> io::Result<Self> {
        Ok(Self {
            addr: listener.local_addr()?,
            app,
            unused: Mutex::new(Some(listener)),
            config: Mutex::new(None),
            handle: Handle::new(),
            task: Mutex::new(None),
        })
    }

    /// The bound address.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Whether it serves (it has a certificate).
    pub fn serving(&self) -> bool {
        lock(&self.config).is_some()
    }

    /// Serves `config`: the first call starts the server on the bound
    /// socket, a later one swaps the configuration (new connections get the
    /// new certificate). Inside a tokio runtime.
    pub fn serve(&self, config: Arc<rustls::ServerConfig>) -> io::Result<()> {
        let mut served = lock(&self.config);
        if let Some(current) = served.as_ref() {
            current.reload_from_config(config);
            tracing::info!(addr = %self.addr, "HTTPS certificate replaced");
            return Ok(());
        }
        let Some(listener) = lock(&self.unused).take() else {
            return Err(io::Error::other("the HTTPS listener was stopped"));
        };
        let rustls = RustlsConfig::from_config(config);
        let server = axum_server::from_tcp_rustls(listener, rustls.clone())?
            .handle(self.handle.clone())
            .serve(
                self.app
                    .clone()
                    .into_make_service_with_connect_info::<SocketAddr>(),
            );
        *lock(&self.task) = Some(tokio::spawn(server));
        *served = Some(rustls);
        tracing::info!(addr = %self.addr, "HTTPS server listening");
        Ok(())
    }

    /// The stop: the socket closes at once, open requests get `drain`.
    pub fn stop(&self, drain: Duration) {
        drop(lock(&self.unused).take());
        self.handle.graceful_shutdown(Some(drain));
    }

    /// Waits up to `bound` for the server to end after [`Https::stop`];
    /// whether it ended (true when it never served).
    pub async fn stopped(&self, bound: Duration) -> bool {
        let Some(task) = lock(&self.task).take() else {
            return true;
        };
        match tokio::time::timeout(bound, task).await {
            Ok(Ok(Ok(()))) => true,
            Ok(Ok(Err(e))) => {
                tracing::error!(error = %e, "the HTTPS server failed");
                true
            }
            Ok(Err(e)) => {
                tracing::error!(error = %e, "the HTTPS server task failed");
                true
            }
            Err(_) => false,
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::test_certs::{TestCa, handshake};

    fn serving() -> (Https, TestCa) {
        let ca = TestCa::new();
        let now = crate::auth::now_secs() as i64;
        let (chain, key) = ca.leaf(&["foh.example.org"], now - 3600, now + 86_400);
        let https = Https::bind("127.0.0.1:0".parse().unwrap(), Router::new()).unwrap();
        https
            .serve(crate::tls::server_config(&chain, &key).unwrap())
            .unwrap();
        (https, ca)
    }

    #[tokio::test]
    async fn the_stop_waits_for_an_open_connection_up_to_its_drain() {
        let (https, _) = serving();
        assert!(https.serving());
        // A client that connects and never finishes its handshake holds the
        // server in its drain.
        let _client = tokio::net::TcpStream::connect(https.addr()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        https.stop(Duration::from_millis(1500));
        assert!(
            !https.stopped(Duration::from_millis(100)).await,
            "still draining"
        );
        // The task was taken by the first wait: nothing left to wait for.
        assert!(https.stopped(Duration::from_millis(100)).await);
    }

    #[tokio::test]
    async fn an_idle_server_ends_at_once_and_a_second_certificate_is_swapped_in() {
        let (https, first) = serving();
        let name = "foh.example.org";
        let served = handshake(https.addr(), name, &first.pem).await.unwrap();
        assert_eq!(served.names, vec![name.to_string()]);
        // A second certificate, of another CA, replaces the first one for
        // the next connection.
        let ca = TestCa::new();
        let now = crate::auth::now_secs() as i64;
        let (chain, key) = ca.leaf(&[name], now - 3600, now + 2 * 86_400);
        https
            .serve(crate::tls::server_config(&chain, &key).unwrap())
            .unwrap();
        assert!(https.serving());
        let swapped = handshake(https.addr(), name, &ca.pem).await.unwrap();
        assert_eq!(swapped.not_after, now + 2 * 86_400);
        assert!(handshake(https.addr(), name, &first.pem).await.is_err());
        https.stop(Duration::from_millis(100));
        assert!(https.stopped(Duration::from_secs(5)).await);
        // Stopped before it ever served: its socket is closed, it cannot start.
        let unused = Https::bind("127.0.0.1:0".parse().unwrap(), Router::new()).unwrap();
        unused.stop(Duration::from_millis(10));
        assert!(
            unused.stopped(Duration::from_millis(10)).await,
            "never served"
        );
        let error = unused
            .serve(crate::tls::server_config(&chain, &key).unwrap())
            .unwrap_err();
        assert_eq!(error.to_string(), "the HTTPS listener was stopped");
        assert!(!unused.serving());
    }
}
