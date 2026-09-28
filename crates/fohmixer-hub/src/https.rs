//! The HTTPS listener of the `[tls]` name (#17), iemmixer's axum-server
//! rustls listener: bound at the start next to the plain-HTTP one (a taken
//! port stops the start at once), serving from the first certificate on
//! (the stored one at the start, else the first the ACME client gets), a
//! renewed certificate swapped in without a restart, and stopped with the
//! HTTP listener (graceful, open requests get the same drain).

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
    /// Binds `addr` (port 0: any): the socket for [`Https::new`], bound
    /// before the hub starts so a taken port stops the start at once.
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
