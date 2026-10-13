//! The HTTPS listener of the `[tls]` name (#17), iemmixer's axum-server
//! rustls listener: bound at the start next to the plain-HTTP one (a taken
//! port costs HTTPS only: `lib.rs` `start_https` tries it again), serving
//! from the `[tls]` name's first certificate on (the stored one at the
//! start, else the first the ACME client gets), a renewed certificate
//! swapped in without a restart, and stopped with the HTTP listener
//! (graceful, open requests get the same drain). AbleSet's name (#90) gets
//! its own certificate on the same listener, chosen by SNI (`tls::SniCerts`):
//! nothing about it ever holds back or replaces the `[tls]` name's.

use std::io;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use axum::Router;
use axum_server::Handle;
use axum_server::tls_rustls::{RustlsAcceptor, RustlsConfig};
use rustls::sign::CertifiedKey;
use tokio::task::JoinHandle;

use crate::accept::Peer;
use crate::tls::{Pem, Slot, SniCerts};

/// The latest certificate of each name and the served configuration.
#[derive(Default)]
struct Served {
    primary: Option<Arc<CertifiedKey>>,
    /// AbleSet's name and certificate (#90).
    ableset: Option<(String, Arc<CertifiedKey>)>,
    /// `None` until the `[tls]` name's first certificate.
    config: Option<RustlsConfig>,
}

/// The listener: bound, serving once it has the `[tls]` name's certificate.
pub struct Https {
    addr: SocketAddr,
    app: Router,
    /// The bound socket until the first certificate starts the server.
    unused: Mutex<Option<std::net::TcpListener>>,
    served: Mutex<Served>,
    handle: Handle<Peer>,
    task: Mutex<Option<JoinHandle<io::Result<()>>>>,
    /// The bound of a request head ([`crate::HEADER_READ`]; a test's own).
    header_read: Duration,
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
            served: Mutex::default(),
            handle: Handle::new(),
            task: Mutex::new(None),
            header_read: crate::HEADER_READ,
        })
    }

    /// The listener with `bound` as its request heads' bound (tests: the
    /// listener's own bound, short).
    #[cfg(test)]
    pub fn header_read(mut self, bound: Duration) -> Self {
        self.header_read = bound;
        self
    }

    /// The bound address.
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Whether it serves (it has the `[tls]` name's certificate).
    pub fn serving(&self) -> bool {
        lock(&self.served).config.is_some()
    }

    /// Whether it serves the certificate of `slot` (AbleSet's: once the
    /// server runs with it).
    pub fn serves(&self, slot: Slot) -> bool {
        let served = lock(&self.served);
        served.config.is_some() && (slot == Slot::Primary || served.ableset.is_some())
    }

    /// Serves `pem` as the certificate of `slot` (`name`: AbleSet's name).
    /// A pair that does not go together is refused and changes nothing. The
    /// `[tls]` name's first certificate starts the server on the bound
    /// socket (AbleSet's, when it comes first, waits for it); after that a
    /// new one is swapped in for new connections. Inside a tokio runtime.
    pub fn set_cert(&self, slot: Slot, name: &str, pem: &Pem) -> Result<(), String> {
        let key = crate::tls::certified_key(&pem.chain, &pem.key)?;
        let mut served = lock(&self.served);
        match slot {
            Slot::Primary => served.primary = Some(key),
            Slot::AbleSet => served.ableset = Some((name.to_string(), key)),
        }
        let Some(primary) = served.primary.clone() else {
            tracing::info!(
                name,
                "HTTPS: AbleSet's certificate waits for the [tls] name's"
            );
            return Ok(());
        };
        let ableset = served.ableset.clone();
        let config = crate::tls::sni_config(SniCerts { primary, ableset })?;
        self.serve(&mut served, config)
            .map_err(|e| format!("starting the HTTPS listener: {e}"))
    }

    /// Serves `config`: the first call starts the server on the bound
    /// socket, a later one swaps the configuration (new connections get the
    /// new certificate). Inside a tokio runtime.
    fn serve(&self, served: &mut Served, config: Arc<rustls::ServerConfig>) -> io::Result<()> {
        if let Some(current) = served.config.as_ref() {
            current.reload_from_config(config);
            tracing::info!(addr = %self.addr, "HTTPS certificate replaced");
            return Ok(());
        }
        let Some(listener) = lock(&self.unused).take() else {
            return Err(io::Error::other("the HTTPS listener was stopped"));
        };
        let rustls = RustlsConfig::from_config(config);
        let listener = tokio::net::TcpListener::from_std(listener)?;
        let server = crate::accept::server(listener, self.header_read)
            .acceptor(RustlsAcceptor::new(rustls.clone()))
            .handle(self.handle.clone());
        let server = server.serve(
            self.app
                .clone()
                .into_make_service_with_connect_info::<SocketAddr>(),
        );
        *lock(&self.task) = Some(tokio::spawn(server));
        served.config = Some(rustls);
        tracing::info!(addr = %self.addr, "HTTPS server listening");
        Ok(())
    }

    /// The stop: the socket closes at once, open requests get `drain`.
    pub fn stop(&self, drain: Duration) {
        drop(lock(&self.unused).take());
        self.handle.graceful_shutdown(Some(drain));
    }

    /// Waits up to `bound` for the server to end after [`Https::stop`];
    /// whether it ended (true when it never served). A wait that timed out
    /// can be repeated.
    pub async fn stopped(&self, bound: Duration) -> bool {
        let Some(mut task) = lock(&self.task).take() else {
            return true;
        };
        match tokio::time::timeout(bound, &mut task).await {
            Ok(Ok(Ok(()))) => true,
            Ok(Ok(Err(e))) => {
                tracing::error!(error = %e, "the HTTPS server failed");
                true
            }
            Ok(Err(e)) => {
                tracing::error!(error = %e, "the HTTPS server task failed");
                true
            }
            Err(_) => {
                *lock(&self.task) = Some(task);
                false
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tls::checked;
    use crate::tls::test_certs::{TestCa, connect, handshake, handshake_sni};

    const NAME: &str = "foh.example.org";
    const ABLESET: &str = "ableset.example.org";

    fn serving() -> (Https, TestCa) {
        let ca = TestCa::new();
        let now = crate::auth::now_secs() as i64;
        let (chain, key) = ca.leaf(&[NAME], now - 3600, now + 86_400);
        let https = Https::bind("127.0.0.1:0".parse().unwrap(), Router::new()).unwrap();
        let pem = checked(chain, key, NAME).unwrap();
        https.set_cert(Slot::Primary, NAME, &pem).unwrap();
        (https, ca)
    }

    #[tokio::test]
    async fn the_stop_waits_for_an_open_connection_up_to_its_drain() {
        let (https, _) = serving();
        assert!(https.serving());
        // A client that connects and never finishes its handshake holds the
        // server in its drain (once the server has accepted it).
        let _client = tokio::net::TcpStream::connect(https.addr()).await.unwrap();
        for _ in 0..500 {
            if https.handle.connection_count() > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(https.handle.connection_count(), 1);
        https.stop(Duration::from_millis(1500));
        assert!(
            !https.stopped(Duration::from_millis(100)).await,
            "still draining"
        );
        // The wait can be repeated: the server ends with its drain.
        assert!(https.stopped(Duration::from_secs(5)).await);
        assert!(https.stopped(Duration::from_millis(1)).await, "ended");
    }

    #[tokio::test]
    async fn the_listener_bounds_a_request_head_itself() {
        // #90 follow-up 5: serve applies the listener's own bound.
        let ca = TestCa::new();
        let now = crate::auth::now_secs() as i64;
        let (chain, key) = ca.leaf(&[NAME], now - 3600, now + 86_400);
        let bound = Duration::from_millis(300);
        let https = Https::bind("127.0.0.1:0".parse().unwrap(), Router::new())
            .unwrap()
            .header_read(bound);
        let pem = checked(chain, key, NAME).unwrap();
        https.set_cert(Slot::Primary, NAME, &pem).unwrap();
        let page = connect(https.addr(), NAME, &ca.pem, true).await.unwrap();
        crate::closes_half_a_head(page, bound).await;
        https.stop(Duration::from_millis(10));
        assert!(https.stopped(Duration::from_secs(5)).await);
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
        let pem = checked(chain, key, name).unwrap();
        https.set_cert(Slot::Primary, name, &pem).unwrap();
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
        let error = unused.set_cert(Slot::Primary, name, &pem).unwrap_err();
        assert_eq!(
            error,
            "starting the HTTPS listener: the HTTPS listener was stopped"
        );
        assert!(!unused.serving());
    }

    #[tokio::test]
    async fn ablesets_name_gets_its_own_certificate_by_sni_and_never_holds_back_the_tls_names() {
        let ca = TestCa::new();
        let now = crate::auth::now_secs() as i64;
        let day = 86_400;
        let pem = |names: &[&str], days: i64| {
            let (chain, key) = ca.leaf(names, now - 3600, now + days * day);
            checked(chain, key, names[0]).unwrap()
        };
        let https = Https::bind("127.0.0.1:0".parse().unwrap(), Router::new()).unwrap();
        // AbleSet's certificate first: kept, but only the [tls] name's
        // starts the server.
        https
            .set_cert(Slot::AbleSet, ABLESET, &pem(&[ABLESET], 10))
            .unwrap();
        assert!(!https.serving());
        assert!(!https.serves(Slot::AbleSet));
        // The [tls] name's (it also covers another name) starts it with both.
        let other = "other.example.org";
        https
            .set_cert(Slot::Primary, NAME, &pem(&[NAME, other], 20))
            .unwrap();
        assert!(https.serves(Slot::Primary) && https.serves(Slot::AbleSet));
        let (addr, ca_pem) = (https.addr(), ca.pem.as_str());
        let end = |sni: &'static str| async move {
            handshake(addr, sni, ca_pem)
                .await
                .map(|info| info.not_after)
        };
        // A certificate of the wrong name fails the client's check, so each
        // answer below proves which one was served.
        assert_eq!(end(ABLESET).await, Ok(now + 10 * day));
        assert_eq!(end(NAME).await, Ok(now + 20 * day));
        // Any other name, and no SNI at all: the [tls] name's.
        assert_eq!(end(other).await, Ok(now + 20 * day));
        let no_sni = handshake_sni(addr, NAME, ca_pem, false).await.unwrap();
        assert_eq!(no_sni.not_after, now + 20 * day);
        // A renewal of one name leaves the other served.
        https
            .set_cert(Slot::AbleSet, ABLESET, &pem(&[ABLESET], 30))
            .unwrap();
        assert_eq!(end(ABLESET).await, Ok(now + 30 * day));
        assert_eq!(end(NAME).await, Ok(now + 20 * day));
        https
            .set_cert(Slot::Primary, NAME, &pem(&[NAME], 40))
            .unwrap();
        assert_eq!(end(NAME).await, Ok(now + 40 * day));
        assert_eq!(end(ABLESET).await, Ok(now + 30 * day));
        // A pair that does not go together is refused and changes nothing.
        let mut foreign = pem(&[ABLESET], 50);
        foreign.key = pem(&[ABLESET], 50).key;
        let error = https
            .set_cert(Slot::AbleSet, ABLESET, &foreign)
            .unwrap_err();
        assert!(
            error.starts_with("the certificate and the key do not go together"),
            "{error}"
        );
        assert_eq!(end(ABLESET).await, Ok(now + 30 * day));
        https.stop(Duration::from_millis(10));
        assert!(https.stopped(Duration::from_secs(5)).await);
        // Without AbleSet's certificate only the [tls] name's is served.
        let (alone, _) = serving();
        assert!(alone.serves(Slot::Primary));
        assert!(!alone.serves(Slot::AbleSet));
        alone.stop(Duration::from_millis(10));
        assert!(alone.stopped(Duration::from_secs(5)).await);
    }
}
