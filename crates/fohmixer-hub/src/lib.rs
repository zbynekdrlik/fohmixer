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
//!
//! AbleSet's own page (#90, `[ableset]`): requests for its name go, on both
//! listeners and before fohmixer's own layers, to AbleSet's web app on this
//! PC (`ableset.rs`), under fohmixer's access rules with an Access gate of
//! their own.
//!
//! The Stream Deck tab (#52, `[companion]`): one more Stream Deck on Bitfocus
//! Companion's Satellite API (`companion.rs`), its state in the router
//! (`deck.rs`, `router/deck.rs`), its link in `/api/status`. The stop
//! releases the held keys and waits (bounded) for the Companion task's
//! `REMOVE-DEVICE`.
//!
//! The Pro-Q 4 screen (#71 PR E, `[eq]`): a window worker (`plugwin.rs`:
//! the Windows backend on the PC, the simulated one elsewhere) opens,
//! captures and touches a plug-in's editor; the router holds who holds which
//! (`eq.rs`, `router/eq.rs`); the editors' last pictures are kept for
//! `GET /api/eq/picture`.

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
use hyper_util::rt::{TokioExecutor, TokioTimer};
use hyper_util::server::conn::auto;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tower_http::set_header::SetResponseHeaderLayer;

pub mod ableset;
pub mod accept;
pub mod access;
pub mod acme;
pub mod auth;
pub mod cf_token;
pub mod client_report;
pub mod clock;
pub mod cloudflare;
pub mod companion;
pub mod config;
pub mod deck;
#[cfg(windows)]
mod dpapi;
pub mod eq;
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
pub mod plugwin;
pub mod provision;
pub mod ration;
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
use companion::CompanionHandle;
use config::Config;
use events::EventLog;
use https::Https;
use layout::LayoutStore;
use live::client::{Events, LiveEvent, LiveHandle};
use remote::RemoteState;
use router::RouterMsg;
use tls::Slot;

/// The HTTP port when neither `PORT` nor the config sets one.
pub const DEFAULT_PORT: u16 = config::DEFAULT_HTTP_PORT;

/// How long a stop waits for open requests before the server returns anyway.
pub const STOP_DRAIN: Duration = Duration::from_secs(5);

/// How long a client may take to send a request's head, on both listeners
/// (#90; before, a client that never finished one held its connection for
/// good). hyper counts it from when a connection waits for a head, so an
/// idle kept-alive connection also closes after it: 2 minutes is above
/// cloudflared's 90 s idle pool (the hub never closes a connection the
/// tunnel may still reuse, where a POST could fail) and far above any real
/// client's head (a phone's is a packet or two; its TLS handshake already
/// has axum-server's 10 s).
///
/// What it does not bound: a connection that never sends a byte (after
/// the TLS handshake on HTTPS), or only part of HTTP/2's 24-byte preface
/// (hyper-util's auto builder reads that far, untimed, to choose HTTP/1 or
/// HTTP/2), and an HTTP/2 connection (no head timer there). Never close
/// that gap with axum-server's `http1_only()`: it serves connections
/// without upgrades, which ends every WebSocket (the mixer's `/ws` and
/// AbleSet's tunnel).
pub const HEADER_READ: Duration = Duration::from_secs(120);

/// The HTTP/1 settings of both listeners: a request head within
/// `header_read`, on Tokio's timer (hyper runs no timeout without one).
pub fn http1_limits(builder: &mut auto::Builder<TokioExecutor>, header_read: Duration) {
    builder
        .http1()
        .timer(TokioTimer::new())
        .header_read_timeout(header_read);
}

/// How long the stop waits for the Companion task's `REMOVE-DEVICE` (#52; the
/// task bounds its own wait for Companion at 500 ms) before it ends the task.
pub const COMPANION_STOP_WAIT: Duration = Duration::from_secs(1);

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
    /// The rations of its `access: DENY` lines (#90), shared by both
    /// listeners' routers.
    pub deny_lines: access::DenyLines,
    /// AbleSet's name behind the hub (#90, `[ableset]`).
    pub ableset: Option<Arc<ableset::AbleSet>>,
    /// Remote access's live state for `/api/status`.
    pub remote: Arc<RemoteState>,
    /// The host names a request may name besides addresses (`check_host`).
    pub trusted_hosts: Vec<String>,
    /// The Content-Security-Policy sent with every page ([`page_policy`]).
    pub page_policy: String,
    /// Its revision ([`policy_rev`], #90): in the hello and in the served
    /// `index.html`, so an open page that loaded with another reloads.
    pub policy_rev: String,
    /// The HTTPS listener (`[tls]`), once `serve_until` made it.
    pub https: std::sync::OnceLock<Arc<Https>>,
    /// The pages' diagnostic reports (#26, `POST /api/client-report`).
    pub reports: client_report::Reports,
    /// The event log (#43, `<data>/logs/events-YYYY-MM-DD.jsonl`).
    pub events: EventLog,
    /// The Stream Deck's Companion task (#52, `[companion]`).
    pub(crate) companion: Option<CompanionHandle>,
    /// Its task, awaited after the stop so its `REMOVE-DEVICE` goes out.
    companion_task: Mutex<Option<JoinHandle<()>>>,
    /// The Pro-Q 4 editors' last pictures (#71 PR E, `GET /api/eq/picture`).
    pub pictures: Arc<eq::Pictures>,
    /// The Pro-Q 4 screen is on (`[eq] backend` is not `off`).
    pub eq: bool,
    /// Its window worker, awaited after the stop so its windows are handed
    /// back and its contact ended before the process ends.
    plugwin: Option<plugwin::Plugwin>,
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
    /// Closes every client and ends the hub's tasks (idempotent). The
    /// router first releases the Stream Deck's held keys and tells the
    /// Companion task to stop (#52); [`HubInner::companion_stopped`] waits
    /// for it.
    pub fn stop(&self) {
        let _ = self.router.send(RouterMsg::Stop);
        // AbleSet's WebSocket tunnels (#90 B2): an upgraded connection is
        // no longer the listener's, so its drain never waits for one.
        if let Some(site) = &self.ableset {
            site.stop();
        }
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

    /// Whether every client message tells the router the page was heard:
    /// a silent page's Stream Deck keys are released (#52) and its Pro-Q
    /// contact ends (#71 PR E).
    pub fn hears_clients(&self) -> bool {
        self.companion.is_some() || self.eq
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

    /// Waits up to [`COMPANION_STOP_WAIT`] for the Companion task to end
    /// after a stop (the router tells it to, after its releases), then ends
    /// it.
    pub async fn companion_stopped(&self) {
        let task = self
            .companion_task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(mut task) = task
            && tokio::time::timeout(COMPANION_STOP_WAIT, &mut task)
                .await
                .is_err()
        {
            tracing::warn!(
                wait_ms = COMPANION_STOP_WAIT.as_millis() as u64,
                "the Companion task did not end within the stop's wait: ending it"
            );
            task.abort();
        }
    }

    /// Waits up to [`ableset::CLOSED_WAIT`] for AbleSet's WebSocket tunnels
    /// to end after a stop (#90 review 3): their connections left the
    /// listeners at the upgrade, so neither drain waits for them, and the
    /// binary's runtime shutdown would cut a close (AbleSet's end without
    /// its FIN) and its record.
    pub async fn ableset_closed(&self) {
        if let Some(site) = &self.ableset
            && let Some(warning) = tunnels_warning(site.closed(ableset::CLOSED_WAIT).await)
        {
            tracing::warn!(
                wait_ms = ableset::CLOSED_WAIT.as_millis() as u64,
                "{warning}"
            );
        }
    }

    /// Waits up to [`plugwin::STOP_WAIT`] for the Pro-Q 4 window worker to
    /// end after a stop (the router tells it to, at its end): its windows
    /// handed back (their z-order as it was), its contact ended.
    pub async fn plugwin_stopped(&self) {
        if let Some(plugwin) = &self.plugwin
            && let Some(warning) = plugwin_stop_warning(plugwin.stopped().await)
        {
            tracing::warn!(wait_ms = plugwin::STOP_WAIT.as_millis() as u64, "{warning}");
        }
    }

    /// `/api/markers/migration` (#68 PR C): the planned tracks as the marker
    /// keeper finds them; with `apply`, the ready ones' Tuners are renamed.
    pub async fn migration(
        &self,
        apply: bool,
    ) -> fohmixer_proto::markers::migrate::MigrationStatus {
        let (reply, answer) = oneshot::channel();
        self.route(RouterMsg::Migration { apply, reply });
        answer.await.unwrap_or_default()
    }

    /// `GET /api/status`.
    pub async fn status(&self) -> HubStatus {
        let now = auth::now_secs() as i64;
        let https = self.https.get().map(Arc::as_ref);
        let (reply, answer) = oneshot::channel();
        self.route(RouterMsg::Status { reply });
        let router = answer.await.unwrap_or_default();
        let mut ableset = self.remote.ableset(&self.config, https, now);
        if let (Some(status), Some(site)) = (ableset.as_mut(), self.ableset.as_ref()) {
            site.fill(status);
        }
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
                    unfolded: router.unfolded.get(&cfg.name).cloned().unwrap_or_default(),
                    playing: router.playing.get(&cfg.name).copied().flatten(),
                }
            })
            .collect();
        HubStatus {
            instances,
            layout: LayoutStatus {
                rev: self.layout.current().0,
                error: self.layout.error(),
                unresolved: router.unresolved,
                markers: self.layout.markers_status(),
            },
            stage_aut: router.stage_aut,
            reverb: router.reverb,
            fader_links: Vec::new(),
            clients: router.clients,
            client_reports: self.reports.list(),
            remote: self.remote.snapshot(&self.config, https, now),
            companion: self.companion.as_ref().map(CompanionHandle::status),
            ableset,
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
        let access = access_gate(config.access.as_ref(), &mut tasks)?;
        // AbleSet's name (#90): its own gate, the AbleSet application's
        // audience, so neither application's assertion opens the other.
        let ableset_gate = access_gate(ableset::access_cfg(&config).as_ref(), &mut tasks)?;
        let ableset = config
            .ableset
            .as_ref()
            .zip(config.tls.as_ref())
            .map(|(cfg, tls)| {
                Arc::new(ableset::AbleSet::new(
                    cfg,
                    tls,
                    ableset_gate,
                    events.clone(),
                ))
            });
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
        // The Stream Deck (#52): its Companion task and the router's tick.
        let companion = config.companion.as_ref().map(|cfg| {
            let tx = router_tx.clone();
            let events: companion::Events = Arc::new(move |event: companion::CompanionEvent| {
                let _ = tx.send(RouterMsg::Deck { event });
            });
            CompanionHandle::spawn(cfg, events)
        });
        // The Pro-Q 4 screen (#71 PR E): the window worker, its events to
        // the router.
        let plugwin = match plugwin_backend(&config)? {
            Some(backend) => {
                let tx = router_tx.clone();
                let events: plugwin::Events = Arc::new(move |event: plugwin::PlugwinEvent| {
                    let _ = tx.send(RouterMsg::EqWorker { event });
                });
                Some(
                    plugwin::Plugwin::spawn(backend, events)
                        .context("starting the plug-in window worker")?,
                )
            }
            None => None,
        };
        if config.companion.is_some() || plugwin.is_some() {
            tasks.push(tokio::spawn(deck_ticks(router_tx.clone(), deck::TICK)));
        }
        let names: Vec<String> = config.instances.iter().map(|i| i.name.clone()).collect();
        let layout = Arc::new(layout::store_in(&config.data_dir, &config.layout, names));
        let hub_state = rules::HubState::load(&config.data_dir);
        let policy = page_policy(&config);
        let revision = policy_rev(&policy);
        tracing::info!(
            instances = config.instances.len(),
            layout = %config.layout_path().display(),
            stage_aut = hub_state.stage_aut,
            public_name = config.tls.as_ref().map(|t| t.name.as_str()).unwrap_or("-"),
            ableset = config.ableset.as_ref().map(|a| a.name.as_str()).unwrap_or("-"),
            access = config.access.is_some(),
            companion = config.companion.is_some(),
            eq = ?config.eq_backend(),
            policy = %revision,
            "hub started"
        );
        let mut router = router::Router::new(
            live.clone(),
            hub_state,
            config.data_dir.clone(),
            router::RouterIo {
                tx: router_tx.clone(),
                events: events.clone(),
            },
        );
        if let (Some(cfg), Some((handle, _))) = (&config.companion, &companion) {
            router = router.with_deck(cfg.clone(), handle.clone());
        }
        let pictures = Arc::new(eq::Pictures::default());
        let eq = plugwin.is_some();
        if let Some(plugwin) = plugwin.clone() {
            router = router.with_eq(plugwin, Arc::clone(&pictures));
        }
        // The Tuner markers (#68) go to the layout store.
        router = router.with_layout(Arc::clone(&layout));
        // AbleSet's page (#90 B3): every page is told of it.
        if let (Some(cfg), Some(tls)) = (&config.ableset, &config.tls) {
            router = router.with_ableset(ableset::link::Page::of(cfg, tls));
        }
        tokio::spawn(router.run(router_rx));
        tasks.push(tokio::spawn(poll_layout(
            Arc::clone(&layout),
            router_tx.clone(),
            Duration::from_millis(config.layout_poll_ms),
        )));
        let trusted_hosts = config.trusted_hosts();
        let (companion, companion_task) = companion.unzip();
        Ok(Self(Arc::new(HubInner {
            config,
            auth,
            layout,
            access,
            page_policy: policy,
            policy_rev: revision,
            deny_lines: access::DenyLines::default(),
            ableset,
            remote,
            trusted_hosts,
            https: std::sync::OnceLock::new(),
            reports: client_report::Reports::default(),
            events,
            companion,
            companion_task: Mutex::new(companion_task),
            pictures,
            eq,
            plugwin,
            live,
            router: router_tx,
            next_client: AtomicU64::new(1),
            tasks: Mutex::new(tasks),
        })))
    }
}

/// The window backend of `[eq]` (#71 PR E): none when it is `off`; the
/// simulated one records into the data folder (`eq-sim.jsonl`, the E2E
/// harness's `/sim/eq`) and keeps Pro-Q's undo history from the start (PR
/// P2a: its pictures show a grey ↶, which a band or a drag lights, so a
/// page's undo reads and taps it as on the PC).
fn plugwin_backend(config: &Config) -> anyhow::Result<Option<Box<dyn plugwin::Backend>>> {
    match config.eq_backend() {
        config::EqBackend::Off => Ok(None),
        config::EqBackend::Sim => {
            let file = config.data_dir.join(plugwin::sim::RECORD_FILE);
            let (sim, handle) = plugwin::sim::Sim::new(Some(file));
            handle.undo_history(0);
            Ok(Some(Box::new(sim)))
        }
        config::EqBackend::Windows => windows_backend().map(Some),
    }
}

#[cfg(windows)]
use crate::plugwin::win::hub_backend as windows_backend;

#[cfg(not(windows))]
fn windows_backend() -> anyhow::Result<Box<dyn plugwin::Backend>> {
    anyhow::bail!("[eq] backend \"windows\" runs on Windows only")
}

/// The Access gate of `cfg` with its key refresher among `tasks`; none
/// without a config (internet requests are then refused).
fn access_gate(
    cfg: Option<&config::AccessCfg>,
    tasks: &mut Vec<JoinHandle<()>>,
) -> anyhow::Result<Option<Arc<AccessGate>>> {
    let Some(cfg) = cfg else {
        return Ok(None);
    };
    let gate = AccessGate::new(cfg, remote_http()?);
    tasks.push(gate.spawn_refresher());
    Ok(Some(gate))
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
            if router
                .send(router::layout_msg(rev, served.as_deref()))
                .is_err()
            {
                return;
            }
        }
    }
}

/// The Stream Deck's clock (#52): a tick to the router every `period`.
async fn deck_ticks(router: mpsc::UnboundedSender<RouterMsg>, period: Duration) {
    let mut tick = tokio::time::interval(period);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        if router.send(RouterMsg::Tick).is_err() {
            return;
        }
    }
}

/// Half a request head sent on `stream`, which must then close after about
/// `bound` (a listener's head bound, #90 follow-up 5), within 5 s (tests).
#[cfg(test)]
pub(crate) async fn closes_half_a_head<S>(mut stream: S, bound: Duration)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: x\r\n")
        .await
        .unwrap();
    stream.flush().await.unwrap();
    let started = std::time::Instant::now();
    let mut rest = Vec::new();
    // A TLS peer may end without close_notify: any end is the close.
    let closed = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut rest));
    assert!(closed.await.is_ok(), "the connection stayed open");
    let after = started.elapsed();
    assert!(
        after >= bound * 2 / 3,
        "closed after {after:?}, before the bound"
    );
}

/// A hub on `dir` with no instances (tests).
#[cfg(test)]
pub(crate) fn test_hub(dir: &std::path::Path) -> Hub {
    let mut config = Config::defaults(dir);
    config.instances.clear();
    Hub::start(config).expect("a test hub")
}

/// The Content-Security-Policy the hub of `config` sends with its pages:
/// [`page_csp`] of `[ableset]`'s name and the `[tls]` port (#90).
pub fn page_policy(config: &Config) -> String {
    let framed = config.ableset.as_ref().zip(config.tls.as_ref());
    page_csp(framed.map(|(a, tls)| (a.name.as_str(), tls.port)))
}

/// The revision of a page policy (#90): its FNV-1a hash in 16 hex digits
/// (as a key image's identity, `deck::img_hash`). A page keeps the one it
/// loaded with (the hub writes it into the `index.html` it serves,
/// `routes::with_policy`) and reloads when a hello names another: a restart
/// with `[ableset]` added or removed changes it, one with the same policy
/// never does.
pub fn policy_rev(policy: &str) -> String {
    deck::img_hash(policy)
}

/// fohmixer's own Content-Security-Policy: WASM + inline scripts (Trunk's
/// loader) and inline styles (Leptos); connections to this server only (the
/// client WebSocket is same-origin, which 'self' covers); images may also be
/// the page's own blobs (a Pro-Q 4 card's last picture, fetched with the
/// token, #71). With `[ableset]` (#90; `framed`: its name and the `[tls]`
/// port) AbleSet's origins may be framed and probed as an image.
pub fn page_csp(framed: Option<(&str, u16)>) -> String {
    let (img, frame) = match framed {
        Some((name, port)) => {
            let origins = ableset::https_origins(name, port);
            (format!(" {origins}"), format!("; frame-src {origins}"))
        }
        None => (String::new(), String::new()),
    };
    format!(
        "default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data: blob:{img}; font-src 'self'{frame}"
    )
}

/// The HTTP application: API and static routes behind the security headers,
/// the host check (DNS rebinding), the redirect to HTTPS (`redirect`, the
/// plain-HTTP listener only) and the Access check (`access.rs`), in that
/// order, in front of every route. No CORS layer: the UI is always loaded
/// from this server, so its requests are same-origin; a foreign page gets
/// no `Access-Control-Allow-Origin` and cannot read API responses. With
/// `[ableset]` (#90) a request for AbleSet's name goes to its own router
/// before any of that (`ableset::dispatch`).
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
    let csp = SetResponseHeaderLayer::overriding(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_str(&hub.page_policy).expect("a policy of DNS names is a header value"),
    );

    let check_host = axum::middleware::from_fn_with_state(hub.clone(), routes::check_host);
    let guard = access::Guard {
        gate: hub.access.clone(),
        lines: hub.deny_lines.clone(),
    };
    let access = axum::middleware::from_fn_with_state(guard, access::middleware);
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
    let app = app
        .layer(check_host)
        .layer(x_frame_options)
        .layer(x_content_type_options)
        .layer(referrer_policy)
        .layer(csp)
        .with_state(hub.clone());
    match hub.ableset.clone() {
        Some(site) => {
            let redirect = if redirect { Some(hub.clone()) } else { None };
            let to = ableset::Dispatch {
                name: site.name.clone(),
                router: ableset::router(site, redirect),
            };
            app.layer(axum::middleware::from_fn_with_state(to, ableset::dispatch))
        }
        None => app,
    }
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
/// requests get up to [`STOP_DRAIN`] to finish, the Stream Deck's Companion
/// task gets up to [`COMPANION_STOP_WAIT`] to remove the device (#52), the
/// Pro-Q 4 window worker up to [`plugwin::STOP_WAIT`] to hand its windows
/// back (#71), AbleSet's WebSocket tunnels up to [`ableset::CLOSED_WAIT`]
/// to close and record it (#90), and it returns `Ok`.
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
    serve_bounded(addr, config, ready, stop, HEADER_READ).await
}

/// [`serve_until`] with `header_read` as its listener's request-head bound
/// (#90 follow-up 5: a test proves the listener applies it itself).
pub(crate) async fn serve_bounded<F>(
    addr: SocketAddr,
    config: Config,
    ready: oneshot::Sender<SocketAddr>,
    stop: F,
    header_read: Duration,
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
    start_link(&hub, ableset::link::PERIOD);
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

    // axum-server, as the HTTPS listener: its HTTP/1 settings can be set
    // (`http1_limits`; `axum::serve` takes none) and its failed accepts are
    // logged (`accept::Logged`). The stop closes the clients and the
    // listener; open requests get STOP_DRAIN.
    let handle: axum_server::Handle<accept::Peer> = axum_server::Handle::new();
    let server = accept::server(listener, header_read).handle(handle.clone());
    let serve =
        server.serve(app_router(hub.clone()).into_make_service_with_connect_info::<SocketAddr>());
    tokio::pin!(serve);
    let ended = tokio::select! {
        result = &mut serve => Some(result),
        () = stop => {
            tracing::info!(
                drain_s = STOP_DRAIN.as_secs(),
                "stop requested: the clients close, the listener closes, open requests get the drain"
            );
            hub.stop();
            if let Some(https) = hub.https.get() {
                https.stop(STOP_DRAIN);
            }
            handle.graceful_shutdown(None);
            tokio::time::timeout(STOP_DRAIN, &mut serve).await.ok()
        }
    };
    match ended {
        Some(result) => result.context("serving HTTP")?,
        None => tracing::warn!(
            drain_s = STOP_DRAIN.as_secs(),
            "HTTP requests still open after the drain: stopping without them"
        ),
    }
    hub.stop();
    tracing::info!("HTTP server stopped");
    hub.companion_stopped().await;
    hub.plugwin_stopped().await;
    hub.ableset_closed().await;
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

/// The warning when the window worker had not ended within the stop's wait
/// (`ended`: whether it had).
fn plugwin_stop_warning(ended: bool) -> Option<&'static str> {
    (!ended).then_some("the plug-in window worker did not end within the stop's wait")
}

/// The warning when AbleSet's tunnels had not all ended within the stop's
/// wait (`ended`: whether they had).
fn tunnels_warning(ended: bool) -> Option<&'static str> {
    (!ended).then_some("AbleSet's WebSocket tunnels did not all end within the stop's wait")
}

/// The warning when the HTTPS listener had not ended within the backstop
/// after the stop (`ended`: whether it had).
fn https_drain_warning(ended: bool) -> Option<&'static str> {
    (!ended).then_some("HTTPS requests still open after the drain: stopping without them")
}

/// AbleSet's link (#90 B3): with `[ableset]`, the served hub reads AbleSet's
/// index every `period` (`ableset::link::watch`), each change going to every
/// page through the router. It starts with the listeners, as the ACME
/// keepers do: a hub only built (`Hub::start`, the router tests) never
/// reads AbleSet by itself.
fn start_link(hub: &Hub, period: Duration) {
    let (Some(site), Some(cfg)) = (&hub.ableset, &hub.config.ableset) else {
        return;
    };
    let router = hub.router.clone();
    let watch = ableset::link::watch(
        Arc::clone(site),
        ableset::link::client(period),
        format!("{}/", cfg.upstream),
        period,
        move |online| {
            let _ = router.send(RouterMsg::AbleSetLink { online });
        },
    );
    hub.add_task(tokio::spawn(watch));
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

/// Binds the HTTPS listener on `at` and starts it: the stored certificates
/// served when there are (the `[tls]` name's first, then AbleSet's, #90),
/// an ACME keeper per name started with `[acme]`, one turn between them.
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
    let ableset = hub
        .config
        .ableset
        .iter()
        .map(|a| (Slot::AbleSet, a.name.as_str()));
    let names: Vec<(Slot, &str)> = std::iter::once((Slot::Primary, name))
        .chain(ableset)
        .collect();
    let data_dir = &hub.config.data_dir;
    for &(slot, name) in &names {
        remote::serve_stored(&https, &hub.remote, data_dir, slot, name);
    }
    if let (Some(cfg), Some(http)) = (&hub.config.acme, acme_http) {
        let turn: Arc<tokio::sync::Mutex<()>> = Arc::default();
        for (slot, name) in names {
            let mut acme = acme::Acme::new(slot, name, cfg, data_dir, http.clone());
            acme.turn = Arc::clone(&turn);
            let keeper = acme::keep(acme, Arc::clone(&https), Arc::clone(&hub.remote));
            hub.add_task(tokio::spawn(keeper));
        }
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

    #[tokio::test]
    async fn with_acme_each_configured_name_gets_its_own_keeper() {
        let ableset_cfg = config::AbleSetCfg {
            name: "ableset.example.org".into(),
            upstream: config::DEFAULT_ABLESET_UPSTREAM.into(),
            access_aud: Vec::new(),
            title: config::DEFAULT_ABLESET_TITLE.into(),
        };
        for ableset in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let mut config = Config::defaults(dir.path());
            config.instances.clear();
            config.tls = Some(config::TlsCfg {
                name: "foh.example.org".into(),
                port: 0,
                redirect_http: true,
            });
            // No Cloudflare token: each keeper's attempt fails at once,
            // asking nothing of anyone, and shows in its own name's status.
            config.acme = Some(config::AcmeCfg {
                email: None,
                directory: config::LETS_ENCRYPT.into(),
                propagation_s: 0,
            });
            config.ableset = if ableset {
                Some(ableset_cfg.clone())
            } else {
                None
            };
            let hub = Hub::start(config.clone()).unwrap();
            let at = "127.0.0.1:0".parse().unwrap();
            start_https(&hub, at, "foh.example.org", Duration::from_millis(50));
            // Read as if [ableset] were set, so a keeper of AbleSet's name
            // that should not run would show.
            let mut read = config.clone();
            read.ableset = Some(ableset_cfg.clone());
            let attempts = || {
                let tls = hub.remote.snapshot(&read, None, 0).https.unwrap();
                let cert = hub.remote.ableset(&read, None, 0).unwrap().cert;
                (
                    (tls.acme_error, tls.acme_failures),
                    (cert.acme_error, cert.acme_failures),
                )
            };
            let failed = (Some(acme::NO_TOKEN.to_string()), 1);
            for _ in 0..200 {
                if attempts().0 == failed && (!ableset || attempts().1 == failed) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            // A keeper of AbleSet's name would have had its turn by now.
            tokio::time::sleep(Duration::from_millis(100)).await;
            let none = (None, 0);
            let expected = (failed.clone(), if ableset { failed } else { none });
            assert_eq!(attempts(), expected, "[ableset]: {ableset}");
            hub.stop();
        }
    }

    #[test]
    fn the_page_may_frame_ablesets_origins_only_with_ableset() {
        let today = "default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; connect-src 'self'; img-src 'self' data: blob:; font-src 'self'";
        assert_eq!(page_csp(None), today);
        assert_eq!(
            page_csp(Some(("ableset.example.org", 8443))),
            "default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; connect-src 'self'; \
             img-src 'self' data: blob: https://ableset.example.org https://ableset.example.org:8443; font-src 'self'; \
             frame-src https://ableset.example.org https://ableset.example.org:8443"
        );
    }

    #[test]
    fn the_page_policys_revision_changes_with_ableset_and_only_with_the_policy() {
        // #90: a page reloads when the hello names another revision than
        // the one it loaded with, and only then.
        let data = std::path::Path::new("data");
        let config = |text: &str| Config::parse(text, data).unwrap();
        let tls = "[tls]\nname = \"foh.example.org\"\nport = 8443\n";
        let ableset = |port: u16, title: &str| {
            format!(
                "[tls]\nname = \"foh.example.org\"\nport = {port}\n\
                 [ableset]\nname = \"ableset.example.org\"\ntitle = \"{title}\"\n"
            )
        };
        let with = ableset(8443, "AbleSet");
        // The policy is the page's CSP of the config.
        assert_eq!(page_policy(&config(tls)), page_csp(None));
        assert_eq!(page_policy(&config("")), page_csp(None));
        assert_eq!(
            page_policy(&config(&with)),
            page_csp(Some(("ableset.example.org", 8443)))
        );
        let rev = |text: &str| policy_rev(&page_policy(&config(text)));
        // The same config: the same revision, so a plain restart reloads nothing.
        assert_eq!(rev(tls), rev(tls));
        assert_eq!(rev(&with), rev(&with));
        // [ableset] added or removed: another one.
        assert_ne!(rev(tls), rev(&with));
        // So is another [tls] port under [ableset] (AbleSet's origins name it).
        assert_ne!(rev(&ableset(9443, "AbleSet")), rev(&with));
        // A change outside the policy is none.
        assert_eq!(rev(&ableset(8443, "Show")), rev(&with));
        assert_eq!(rev(""), rev(tls));
        // FNV-1a in 16 hex digits.
        assert_eq!(policy_rev(""), "cbf29ce484222325");
        assert_eq!(policy_rev("a"), "af63dc4c8601ec8c");
        assert_eq!(rev(tls).len(), 16);
    }

    #[test]
    fn only_an_https_listener_still_draining_is_warned_about() {
        assert_eq!(https_drain_warning(true), None);
        assert!(https_drain_warning(false).unwrap().contains("still open"));
    }

    #[test]
    fn only_ablesets_tunnels_still_open_are_warned_about() {
        assert_eq!(tunnels_warning(true), None);
        assert!(tunnels_warning(false).unwrap().contains("did not all end"));
    }

    #[test]
    fn only_a_window_worker_still_running_is_warned_about() {
        assert_eq!(plugwin_stop_warning(true), None);
        assert!(plugwin_stop_warning(false).unwrap().contains("did not end"));
    }

    #[tokio::test]
    async fn serve_until_bounds_a_request_head_on_its_own_listener() {
        // #90 follow-up 5: the listener serve_until makes applies the bound
        // itself (serve_until passes HEADER_READ to serve_bounded).
        assert_eq!(HEADER_READ, Duration::from_secs(120));
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::defaults(dir.path());
        config.instances.clear();
        let (ready, at) = oneshot::channel();
        let (stop, stopped) = oneshot::channel::<()>();
        let bound = Duration::from_millis(300);
        let addr = "127.0.0.1:0".parse().unwrap();
        let stop_on = async move {
            let _ = stopped.await;
        };
        let served = tokio::spawn(serve_bounded(addr, config, ready, stop_on, bound));
        let at = at.await.unwrap();
        let page = tokio::net::TcpStream::connect(at).await.unwrap();
        closes_half_a_head(page, bound).await;
        stop.send(()).unwrap();
        served.await.unwrap().unwrap();
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
    async fn a_hub_hears_its_clients_only_for_the_stream_deck_or_the_eq_screen() {
        for (backend, on) in [
            (config::EqBackend::Off, false),
            (config::EqBackend::Sim, true),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut config = Config::defaults(dir.path());
            config.instances.clear();
            config.companion = None;
            config.eq = Some(config::EqCfg {
                backend: Some(backend),
            });
            let hub = Hub::start(config).unwrap();
            assert_eq!((hub.eq, hub.hears_clients()), (on, on), "{backend:?}");
            hub.stop();
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn the_windows_window_backend_is_refused_off_windows() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::defaults(dir.path());
        config.eq = Some(config::EqCfg {
            backend: Some(config::EqBackend::Windows),
        });
        let refused = plugwin_backend(&config)
            .err()
            .expect("the Windows backend off Windows");
        assert!(
            refused.to_string().contains("runs on Windows only"),
            "{refused}"
        );
    }

    #[tokio::test]
    async fn the_stop_waits_for_the_window_worker_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::defaults(dir.path());
        config.instances.clear();
        config.eq = Some(config::EqCfg {
            backend: Some(config::EqBackend::Sim),
        });
        let hub = Hub::start(config).unwrap();
        let plugwin = hub
            .plugwin
            .clone()
            .expect("a hub with the screen has its worker");
        assert_eq!(plugwin.list().await, Ok(Vec::new()), "serving");
        hub.stop();
        hub.plugwin_stopped().await;
        // Its thread ended: nothing serves its channel any more.
        assert_eq!(plugwin.list().await, Err(plugwin::STOPPED.to_string()));
        assert!(plugwin.stopped().await);
        // A hub without the screen has nothing to wait for.
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::defaults(dir.path());
        config.instances.clear();
        config.eq = Some(config::EqCfg {
            backend: Some(config::EqBackend::Off),
        });
        let hub = Hub::start(config).unwrap();
        assert!(hub.plugwin.is_none());
        hub.stop();
        hub.plugwin_stopped().await;
    }

    #[tokio::test]
    async fn the_stop_waits_for_the_companion_task_to_remove_its_device() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::defaults(dir.path());
        config.instances.clear();
        config.companion = Some(config::CompanionCfg {
            host: "127.0.0.1".into(),
            port,
            columns: 8,
            rows: 4,
            bitmap_px: 72,
            title: "Stream Deck".into(),
        });
        let hub = Hub::start(config).unwrap();
        let (socket, _) = listener.accept().await.unwrap();
        let (read, mut write) = socket.into_split();
        write
            .write_all(b"BEGIN CompanionVersion=\"5.0.7+fake\" ApiVersion=\"1.12.0\" \n")
            .await
            .unwrap();
        let mut lines = BufReader::new(read).lines();
        let add = lines.next_line().await.unwrap().unwrap();
        assert!(
            add.starts_with("ADD-DEVICE DEVICEID=\"fohmixer-1\""),
            "{add}"
        );
        write
            .write_all(b"ADD-DEVICE OK DEVICEID=\"fohmixer-1\" \n")
            .await
            .unwrap();
        let handle = hub
            .companion
            .clone()
            .expect("a hub with [companion] has the task");
        for _ in 0..300 {
            if handle.snapshot().online {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(handle.snapshot().online);
        // This Companion never answers REMOVE-DEVICE and keeps its socket
        // open: the task ends at the stop's bound, and the stop waits for it.
        let stopping = std::time::Instant::now();
        hub.stop();
        hub.companion_stopped().await;
        let waited = stopping.elapsed();
        assert!(
            waited >= companion::STOP_BOUND,
            "the stop did not wait for the Companion task: {waited:?}"
        );
        // Its REMOVE-DEVICE went out before the wait ended: read at once.
        let removed = tokio::time::timeout(Duration::from_millis(50), async {
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) if line.starts_with("REMOVE-DEVICE") => return line,
                    Ok(Some(_)) => {}
                    other => panic!("the connection ended without REMOVE-DEVICE: {other:?}"),
                }
            }
        })
        .await
        .expect("REMOVE-DEVICE within 50 ms of the stop's wait");
        assert_eq!(removed, "REMOVE-DEVICE DEVICEID=\"fohmixer-1\"");
        // It was the last line: the connection ends after it.
        assert_eq!(lines.next_line().await.ok().flatten(), None);
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
