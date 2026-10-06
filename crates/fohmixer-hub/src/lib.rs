//! fohmixer-hub: the hub on the Ableton PC (spec §2.4).
//!
//! It keeps one reconnecting client per Live instance's FohMixer script,
//! forwards client commands unchanged, deduplicates subscriptions (one Live
//! listener per key, the latest value cached and fanned out, coalesced per
//! client), resubscribes by the original path after every Live `connect`,
//! reports busy instances, serves and validates the layout (the last good
//! one stays), runs the one STAGE AUT rule and authenticates the engineer
//! (PIN → JWT, from iemmixer). It also serves the embedded UI,
//! `/api/version` and `/api/client-error`, behind the security headers, and
//! stops gracefully (S0, trimmed from iemmixer's `iem-server` @ 22372bc).
//! The pages' diagnostic reports (#26, `client_report.rs`) go to the log
//! and `/api/status`.
//!
//! The clients' writes (#43, protocol 2): `set` / `ack` through one setter
//! per instance (`setter.rs`, latest-wins, one batch in flight), and every
//! hop of a move in the dated event log (`events.rs`, `<data>/logs/`).
//!
//! Remote access (#17): one public name for the LAN and the Cloudflare
//! tunnel. The HTTPS listener of that name (`https.rs`, `tls.rs`) with its
//! Let's Encrypt certificate (`acme.rs` by DNS-01 on Cloudflare,
//! `cloudflare.rs`, `cf_token.rs`), the Access check of every internet
//! request (`access.rs`), cloudflared's readiness (`tunnel.rs`), all in
//! `/api/status` (`remote.rs`). The plain-HTTP listener always stays: it is
//! the emergency path by IP and the tunnel's origin.

use std::collections::BTreeMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use anyhow::Context as _;
use axum::Router;
use axum::extract::FromRef;
use axum::http::{HeaderName, HeaderValue};
use fohmixer_proto::client::{HubStatus, InstanceStatus, LayoutStatus};
use tokio::sync::{Notify, mpsc, oneshot};
use tokio::task::JoinHandle;
use tower_http::set_header::SetResponseHeaderLayer;

pub mod access;
pub mod acme;
pub mod auth;
pub mod cf_token;
pub mod client_report;
pub mod clock;
pub mod cloudflare;
pub mod companion;
pub mod config;
#[cfg(windows)]
mod dpapi;
pub mod events;
pub mod http_client;
pub mod https;
pub mod layout;
pub mod live;
pub mod login_guard;
pub mod outbox;
pub mod pepper;
pub mod pin_hash;
pub mod pin_store;
pub mod provision;
pub mod remote;
pub mod router;
pub mod routes;
pub mod rules;
pub mod sealed;
pub mod secrets;
pub mod setter;
#[cfg(test)]
pub(crate) mod test_keys;
pub mod tls;
pub mod tunnel;
pub mod ws;

use access::AccessGate;
use auth::Auth;
use config::Config;
use events::EventLog;
use https::Https;
use layout::LayoutStore;
use live::client::{Events, LiveEvent, LiveHandle};
use remote::RemoteState;
use router::RouterMsg;

/// The HTTP port when neither `PORT` nor the config sets one.
pub const DEFAULT_PORT: u16 = config::DEFAULT_HTTP_PORT;

/// How long a stop waits for open requests before the server returns anyway.
pub const STOP_DRAIN: Duration = Duration::from_secs(5);

/// The UI bundle (Trunk's `dist/`), embedded at build time. CI builds the
/// real bundle for the release binary; native lint and test jobs create a
/// placeholder `dist/index.html` first, because the folder must exist.
#[derive(rust_embed::Embed)]
#[folder = "../fohmixer-ui/dist/"]
pub struct Assets;

/// Writes `data` to `path` atomically: a temporary file, then a rename (a
/// crash never leaves half a file).
pub fn atomic_write(path: &std::path::Path, data: &str) -> std::io::Result<()> {
    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, data)?;
    std::fs::rename(&tmp_path, path)
}

/// The running hub: cheap to clone, shared by every request.
#[derive(Clone)]
pub struct Hub(Arc<HubInner>);

/// What a [`Hub`] holds.
pub struct HubInner {
    pub config: Config,
    pub auth: Arc<Auth>,
    pub layout: Arc<LayoutStore>,
    /// The Access check of internet requests (`[access]`); none: they are
    /// refused.
    pub access: Option<Arc<AccessGate>>,
    /// Remote access's live state for `/api/status`.
    pub remote: Arc<RemoteState>,
    /// The host names a request may name besides addresses (`check_host`).
    pub trusted_hosts: Vec<String>,
    /// The HTTPS listener (`[tls]`), once `serve_until` made it.
    pub https: std::sync::OnceLock<Arc<Https>>,
    /// The pages' diagnostic reports (#26, `POST /api/client-report`).
    pub reports: client_report::Reports,
    /// The event log (#43, `<data>/logs/events-YYYY-MM-DD.jsonl`).
    pub events: EventLog,
    live: BTreeMap<String, LiveHandle>,
    router: mpsc::UnboundedSender<RouterMsg>,
    next_client: AtomicU64,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl std::ops::Deref for Hub {
    type Target = HubInner;

    fn deref(&self) -> &HubInner {
        &self.0
    }
}

impl FromRef<Hub> for Arc<Auth> {
    fn from_ref(hub: &Hub) -> Self {
        Arc::clone(&hub.auth)
    }
}

impl HubInner {
    /// Closes every client and ends the hub's tasks (idempotent).
    pub fn stop(&self) {
        let _ = self.router.send(RouterMsg::Stop);
        for task in self
            .tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
        {
            task.abort();
        }
    }

    /// The handle of an instance.
    pub fn live(&self, name: &str) -> Option<&LiveHandle> {
        self.live.get(name)
    }

    /// Sends a message to the router.
    pub fn route(&self, msg: RouterMsg) {
        let _ = self.router.send(msg);
    }

    /// A new client connection id (1, 2, …; 0 is the router's own).
    pub fn next_client(&self) -> u64 {
        self.next_client.fetch_add(1, Ordering::Relaxed)
    }

    /// A task that ends with the hub.
    pub fn add_task(&self, task: JoinHandle<()>) {
        self.tasks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(task);
    }

    /// `GET /api/status`.
    pub async fn status(&self) -> HubStatus {
        let (reply, answer) = oneshot::channel();
        self.route(RouterMsg::Status { reply });
        let router = answer.await.unwrap_or_default();
        let instances = self
            .config
            .instances
            .iter()
            .map(|cfg| {
                let snap = self
                    .live
                    .get(&cfg.name)
                    .map(LiveHandle::snapshot)
                    .unwrap_or_default();
                InstanceStatus {
                    name: cfg.name.clone(),
                    port: cfg.port,
                    online: snap.online,
                    busy: snap.busy,
                    set_name: snap.info.set_name,
                    live_version: snap.info.live_version,
                    script_version: snap.info.script_version,
                    main_tick_age_ms: snap.main_tick_age_ms,
                    subscriptions: router.subscriptions.get(&cfg.name).copied().unwrap_or(0),
                    listeners: router.listeners.get(&cfg.name).copied().unwrap_or(0),
                    connect_failures: snap.connect_failures,
                    last_error: snap.last_error,
                }
            })
            .collect();
        HubStatus {
            instances,
            layout: LayoutStatus {
                rev: self.layout.current().0,
                error: self.layout.error(),
                unresolved: router.unresolved,
            },
            stage_aut: router.stage_aut,
            clients: router.clients,
            client_reports: self.reports.list(),
            remote: self.remote.snapshot(
                &self.config,
                self.https.get().map(Arc::as_ref),
                auth::now_secs() as i64,
            ),
        }
    }
}

impl Hub {
    /// Starts the hub of `config` (inside a tokio runtime): loads the
    /// secrets (created on first use), starts one client per instance, the
    /// router and the layout check. A corrupt secret, pepper or PIN store
    /// is an error, never replaced.
    pub fn start(config: Config) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&config.data_dir)
            .with_context(|| format!("creating the data folder {}", config.data_dir.display()))?;
        let auth = Arc::new(Auth::load(&config.data_dir).context("loading the secrets")?);
        let events =
            EventLog::start(config.data_dir.join("logs")).context("starting the event log")?;
        let (router_tx, router_rx) = mpsc::unbounded_channel();
        let mut tasks = Vec::new();
        let remote = Arc::new(RemoteState::default());
        let access = match &config.access {
            Some(cfg) => {
                let gate = AccessGate::new(cfg, remote_http()?);
                tasks.push(gate.spawn_refresher());
                Some(gate)
            }
            None => None,
        };
        if let Some(cfg) = &config.tunnel {
            tasks.push(tunnel::spawn(
                remote_http()?,
                cfg.ready_url.clone(),
                Arc::clone(&remote),
            ));
        }
        let mut live = BTreeMap::new();
        for instance in &config.instances {
            let tx = router_tx.clone();
            let events: Events = Arc::new(move |name: &str, event: LiveEvent| {
                let _ = tx.send(RouterMsg::Live {
                    instance: name.to_string(),
                    event,
                });
            });
            let (handle, task) = LiveHandle::spawn(instance, events);
            live.insert(instance.name.clone(), handle);
            tasks.push(task);
        }
        let names: Vec<String> = config.instances.iter().map(|i| i.name.clone()).collect();
        let layout = Arc::new(layout::store_in(&config.data_dir, &config.layout, names));
        let hub_state = rules::HubState::load(&config.data_dir);
        tracing::info!(
            instances = config.instances.len(),
            layout = %config.layout_path().display(),
            stage_aut = hub_state.stage_aut,
            public_name = config.tls.as_ref().map(|t| t.name.as_str()).unwrap_or("-"),
            access = config.access.is_some(),
            "hub started"
        );
        let router = router::Router::new(
            live.clone(),
            hub_state.stage_aut,
            config.data_dir.clone(),
            router::RouterIo {
                tx: router_tx.clone(),
                events: events.clone(),
            },
        );
        tokio::spawn(router.run(router_rx));
        tasks.push(tokio::spawn(poll_layout(
            Arc::clone(&layout),
            router_tx.clone(),
            Duration::from_millis(config.layout_poll_ms),
        )));
        let trusted_hosts = config.trusted_hosts();
        Ok(Self(Arc::new(HubInner {
            config,
            auth,
            layout,
            access,
            remote,
            trusted_hosts,
            https: std::sync::OnceLock::new(),
            reports: client_report::Reports::default(),
            events,
            live,
            router: router_tx,
            next_client: AtomicU64::new(1),
            tasks: Mutex::new(tasks),
        })))
    }
}

/// The HTTP client of the remote-access tasks, on rustls with ring.
fn remote_http() -> anyhow::Result<http_client::HttpClient> {
    // The crypto provider for the clients that take the process default
    // (instant-acme's, the platform verifier); ignored when already set.
    let _ = rustls::crypto::ring::default_provider().install_default();
    http_client::HttpClient::new()
}

/// Checks the layout file every `period`; a new layout goes to the router.
async fn poll_layout(
    layout: Arc<LayoutStore>,
    router: mpsc::UnboundedSender<RouterMsg>,
    period: Duration,
) {
    let mut tick = tokio::time::interval(period);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if let Some(rev) = layout.poll() {
            let served = layout.current().1;
            let stage = served.as_ref().and_then(|l| l.stage_aut_binding().cloned());
            let targets = served
                .as_deref()
                .map(live::names::layout_targets)
                .unwrap_or_default();
            if router
                .send(RouterMsg::Layout {
                    rev,
                    stage,
                    targets,
                })
                .is_err()
            {
                return;
            }
        }
    }
}

/// A hub on `dir` with no instances (tests).
#[cfg(test)]
pub(crate) fn test_hub(dir: &std::path::Path) -> Hub {
    let mut config = Config::defaults(dir);
    config.instances.clear();
    Hub::start(config).expect("a test hub")
}

/// The HTTP application: API and static routes behind the security headers,
/// the host check (DNS rebinding), the redirect to HTTPS (`redirect`, the
/// plain-HTTP listener only) and the Access check (`access.rs`), in that
/// order, in front of every route. No CORS layer: the UI is always loaded
/// from this server, so its requests are same-origin; a foreign page gets
/// no `Access-Control-Allow-Origin` and cannot read API responses.
fn router(hub: Hub, redirect: bool) -> Router {
    let x_frame_options = SetResponseHeaderLayer::overriding(
        HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    let x_content_type_options = SetResponseHeaderLayer::overriding(
        HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    let referrer_policy = SetResponseHeaderLayer::overriding(
        HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    // CSP allows WASM + inline scripts (Trunk's loader) and inline styles
    // (Leptos); connections go to this server only (the client WebSocket is
    // same-origin, which 'self' covers).
    let csp = SetResponseHeaderLayer::overriding(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; font-src 'self'",
        ),
    );

    let check_host = axum::middleware::from_fn_with_state(hub.clone(), routes::check_host);
    let access = axum::middleware::from_fn_with_state(hub.access.clone(), access::middleware);
    let mut app = Router::new()
        .merge(routes::api_routes())
        .merge(routes::static_routes())
        .layer(access);
    if redirect {
        app = app.layer(axum::middleware::from_fn_with_state(
            hub.clone(),
            routes::https_redirect,
        ));
    }
    app.layer(check_host)
        .layer(x_frame_options)
        .layer(x_content_type_options)
        .layer(referrer_policy)
        .layer(csp)
        .with_state(hub)
}

/// The plain-HTTP listener's application: [`router`] with the redirect of
/// the public name to HTTPS (`[tls] redirect_http`).
pub fn app_router(hub: Hub) -> Router {
    router(hub, true)
}

/// The HTTPS listener's application: [`router`] without the redirect.
pub fn https_router(hub: Hub) -> Router {
    router(hub, false)
}

/// The log filter when `RUST_LOG` is not set (or empty).
pub const DEFAULT_LOG_FILTER: &str = "fohmixer_hub=info";

/// The log filter from the `RUST_LOG` environment value: exactly what it
/// says when set (so `fohmixer_hub=debug` raises the hub's own level),
/// [`DEFAULT_LOG_FILTER`] when it is not set or empty, and an error naming it
/// when it does not parse (never silently ignored).
pub fn log_filter(rust_log: Option<&str>) -> anyhow::Result<tracing_subscriber::EnvFilter> {
    match rust_log.map(str::trim).filter(|spec| !spec.is_empty()) {
        None => Ok(tracing_subscriber::EnvFilter::new(DEFAULT_LOG_FILTER)),
        Some(spec) => tracing_subscriber::EnvFilter::try_new(spec)
            .with_context(|| format!("RUST_LOG={spec} is not a valid log filter")),
    }
}

/// The HTTP port from the `PORT` environment value: `default` (the
/// config's) when it is not set, an error when it is not a port number.
pub fn port_from(value: Option<&str>, default: u16) -> anyhow::Result<u16> {
    match value {
        None => Ok(default),
        Some(text) => text
            .parse()
            .with_context(|| format!("PORT={text} is not a port number")),
    }
}

/// Serve the hub of `config` on `addr` until `stop` resolves (graceful stop,
/// as in iemmixer): `ready` gets the bound address once the listener is up
/// and the hub started; at the stop the clients' WebSockets close, the
/// listener closes at once (the port is free), idle connections close, open
/// requests get up to [`STOP_DRAIN`] to finish, and it returns `Ok`.
///
/// With `[tls]` the HTTPS listener binds `addr`'s IP on its port next to
/// it, serves the stored certificate or waits for the ACME client's first
/// one (`[acme]`), and stops with the HTTP listener. A port it cannot bind
/// never stops plain HTTP (the emergency path): the error is in
/// `/api/status` and the bind is tried again every [`HTTPS_BIND_RETRY`].
pub async fn serve_until<F>(
    addr: SocketAddr,
    config: Config,
    ready: oneshot::Sender<SocketAddr>,
    stop: F,
) -> anyhow::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    let local = listener.local_addr().context("reading the bound address")?;
    let https_at = config
        .tls
        .as_ref()
        .map(|tls| (SocketAddr::new(addr.ip(), tls.port), tls.name.clone()));
    let hub = Hub::start(config).context("starting the hub")?;
    if let Some((at, name)) = https_at {
        start_https(&hub, at, &name, HTTPS_BIND_RETRY);
    }
    tracing::info!(
        addr = %local,
        version = %fohmixer_proto::full_version(),
        git_hash = fohmixer_proto::git_hash(),
        branch = fohmixer_proto::git_branch(),
        "HTTP server listening"
    );
    // Signal readiness AFTER a successful bind. Nobody waiting is fine (the
    // binary does not wait).
    if ready.send(local).is_err() {
        tracing::debug!("nobody waits for the ready signal");
    }

    // The stop closes the clients and the listener; open requests get
    // STOP_DRAIN.
    let stopping = Arc::new(Notify::new());
    let stop_seen = Arc::clone(&stopping);
    let hub_at_stop = hub.clone();
    let serve = axum::serve(
        listener,
        app_router(hub.clone()).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        stop.await;
        tracing::info!(
            drain_s = STOP_DRAIN.as_secs(),
            "stop requested: the clients close, the listener closes, open requests get the drain"
        );
        hub_at_stop.stop();
        if let Some(https) = hub_at_stop.https.get() {
            https.stop(STOP_DRAIN);
        }
        stop_seen.notify_one();
    });
    tokio::select! {
        result = serve => result.context("serving HTTP")?,
        () = async {
            stopping.notified().await;
            tokio::time::sleep(STOP_DRAIN).await;
        } => tracing::warn!(
            drain_s = STOP_DRAIN.as_secs(),
            "HTTP requests still open after the drain: stopping without them"
        ),
    }
    hub.stop();
    tracing::info!("HTTP server stopped");
    if let Some(https) = hub.https.get() {
        // It took the stop with the HTTP server and ends its connections
        // STOP_DRAIN after it; this bound is only the backstop.
        https.stop(STOP_DRAIN);
        if let Some(warning) = https_drain_warning(https.stopped(STOP_DRAIN).await) {
            tracing::warn!("{warning}");
        }
    }
    Ok(())
}

/// The warning when the HTTPS listener had not ended within the backstop
/// after the stop (`ended`: whether it had).
fn https_drain_warning(ended: bool) -> Option<&'static str> {
    (!ended).then_some("HTTPS requests still open after the drain: stopping without them")
}

/// How often an HTTPS port that could not be bound is tried again.
pub const HTTPS_BIND_RETRY: Duration = Duration::from_secs(60);

/// The HTTPS listener of `name` on `at`: bound now when the port is free,
/// else the bind error is recorded for `/api/status` and the bind is tried
/// again every `retry` in the background, until it works or the hub stops.
fn start_https(hub: &Hub, at: SocketAddr, name: &str, retry: Duration) {
    if let Err(why) = bind_https(hub, at, name) {
        tracing::error!(
            name,
            "{why}: plain HTTP serves; the bind is tried again every {} s",
            retry.as_secs()
        );
        hub.remote.set_bind_error(Some(why));
        let hub_again = hub.clone();
        let name = name.to_string();
        hub.add_task(tokio::spawn(async move {
            loop {
                tokio::time::sleep(retry).await;
                match bind_https(&hub_again, at, &name) {
                    Ok(()) => return,
                    Err(why) => hub_again.remote.set_bind_error(Some(why)),
                }
            }
        }));
    }
}

/// Binds the HTTPS listener on `at` and starts it: the stored certificate
/// served when there is one, the ACME keeper started with `[acme]`.
fn bind_https(hub: &Hub, at: SocketAddr, name: &str) -> Result<(), String> {
    // The ACME client's HTTP client first: its failure leaves nothing bound.
    let acme_http = hub
        .config
        .acme
        .as_ref()
        .map(|_| remote_http())
        .transpose()
        .map_err(|e| format!("the ACME client's HTTP client: {e:#}"))?;
    let socket = Https::listen(at).map_err(|e| format!("binding HTTPS {at}: {e}"))?;
    let https = Arc::new(
        Https::new(socket, https_router(hub.clone()))
            .map_err(|e| format!("the HTTPS listener: {e}"))?,
    );
    let _ = hub.https.set(Arc::clone(&https));
    hub.remote.set_bind_error(None);
    tracing::info!(addr = %https.addr(), name, "HTTPS listener bound");
    remote::serve_stored(&https, &hub.remote, &hub.config.data_dir, name);
    if let (Some(cfg), Some(http)) = (&hub.config.acme, acme_http) {
        let acme = acme::Acme::new(name, cfg, &hub.config.data_dir, http);
        hub.add_task(tokio::spawn(acme::keep(
            acme,
            Arc::clone(&https),
            Arc::clone(&hub.remote),
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_port_defaults_to_the_configured_one() {
        assert_eq!(port_from(None, 8480).unwrap(), 8480);
        assert_eq!(port_from(None, 9100).unwrap(), 9100);
        assert_eq!(DEFAULT_PORT, 8480);
    }

    #[test]
    fn the_port_comes_from_the_environment_value() {
        assert_eq!(port_from(Some("9000"), 8480).unwrap(), 9000);
        assert_eq!(port_from(Some("0"), 8480).unwrap(), 0);
    }

    #[test]
    fn the_log_filter_defaults_to_the_hub_at_info() {
        assert_eq!(log_filter(None).unwrap().to_string(), "fohmixer_hub=info");
        assert_eq!(
            log_filter(Some("")).unwrap().to_string(),
            "fohmixer_hub=info"
        );
        assert_eq!(
            log_filter(Some("  ")).unwrap().to_string(),
            "fohmixer_hub=info"
        );
    }

    #[test]
    fn rust_log_is_the_whole_filter() {
        assert_eq!(
            log_filter(Some("fohmixer_hub=debug")).unwrap().to_string(),
            "fohmixer_hub=debug"
        );
        assert_eq!(log_filter(Some("warn")).unwrap().to_string(), "warn");
    }

    #[test]
    fn a_bad_rust_log_is_an_error_naming_it() {
        let error = log_filter(Some("fohmixer_hub=loud")).unwrap_err();
        assert_eq!(
            error.to_string(),
            "RUST_LOG=fohmixer_hub=loud is not a valid log filter"
        );
    }

    #[test]
    fn a_bad_port_is_an_error_naming_it() {
        for bad in ["", "http", "65536", "-1"] {
            let error = port_from(Some(bad), 8480).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("PORT={bad} is not a port number")
            );
        }
    }

    #[test]
    fn atomic_write_replaces_the_whole_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        atomic_write(&path, "first").unwrap();
        atomic_write(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        assert!(!dir.path().join("state.tmp").exists());
    }

    #[tokio::test]
    async fn a_taken_https_port_is_bound_again_once_it_is_free() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::defaults(dir.path());
        config.instances.clear();
        config.tls = Some(config::TlsCfg {
            name: "foh.example.org".into(),
            port: 0,
            redirect_http: true,
        });
        let hub = Hub::start(config.clone()).unwrap();
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let at = taken.local_addr().unwrap();
        start_https(&hub, at, "foh.example.org", Duration::from_millis(50));
        assert!(hub.https.get().is_none());
        let status = hub.remote.snapshot(&config, None, 0).https.unwrap();
        let why = status.bind_error.unwrap();
        assert!(why.starts_with(&format!("binding HTTPS {at}: ")), "{why}");
        assert!(!status.bound);
        drop(taken);
        for _ in 0..200 {
            if hub.https.get().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let https = hub.https.get().expect("bound once the port is free");
        assert_eq!(https.addr(), at);
        let status = hub.remote.snapshot(&config, Some(https), 0).https.unwrap();
        assert_eq!((status.bound, status.bind_error), (true, None));
        hub.stop();
    }

    #[test]
    fn only_an_https_listener_still_draining_is_warned_about() {
        assert_eq!(https_drain_warning(true), None);
        assert!(https_drain_warning(false).unwrap().contains("still open"));
    }

    #[tokio::test]
    async fn a_task_added_to_the_hub_ends_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let hub = test_hub(dir.path());
        let (held, ended) = oneshot::channel::<()>();
        hub.add_task(tokio::spawn(async move {
            let _held = held;
            std::future::pending::<()>().await;
        }));
        hub.stop();
        // The task was aborted: it dropped its sender.
        tokio::time::timeout(Duration::from_secs(5), ended)
            .await
            .expect("the hub's stop ends its tasks")
            .unwrap_err();
    }

    #[tokio::test]
    async fn a_hub_hands_out_client_ids_and_stops_twice_quietly() {
        let dir = tempfile::tempdir().unwrap();
        let hub = test_hub(dir.path());
        assert_eq!(hub.next_client(), 1);
        assert_eq!(hub.next_client(), 2);
        assert!(hub.live("band").is_none());
        let status = hub.status().await;
        assert!(status.instances.is_empty());
        hub.stop();
        hub.stop();
        // After the stop the router is gone: the status still answers.
        let status = hub.status().await;
        assert_eq!(status.clients, 0);
    }

    #[tokio::test]
    async fn a_hub_on_an_unusable_data_folder_does_not_start() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a-file");
        std::fs::write(&file, b"x").unwrap();
        let mut config = Config::defaults(&file);
        config.instances.clear();
        let error = Hub::start(config).err().expect("a file is no data folder");
        assert!(
            format!("{error:#}").starts_with("creating the data folder"),
            "{error:#}"
        );
    }
}
