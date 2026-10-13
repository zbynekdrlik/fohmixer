//! AbleSet's own page behind the hub (#90, design §3–4): requests for the
//! `[ableset]` name go to AbleSet's web app on this PC, under fohmixer's
//! access rules, on both listeners.
//!
//! - [`dispatch`] is the outermost layer of both listeners' application: a
//!   `Host` of the AbleSet name (read by `routes::host_name`, the one Host
//!   parser) goes to the AbleSet router and never into fohmixer's; anything
//!   else goes on to fohmixer's router unchanged (`check_host`, the
//!   redirect, Access and its own security headers). Without `[ableset]`
//!   the layer is not there at all.
//! - The AbleSet router, inner to outer: the proxy ([`proxy`]), the Access
//!   check with the AbleSet application's own audience (`access.rs`
//!   unchanged: the LAN is never authenticated and does no I/O; a
//!   fohmixer assertion never opens AbleSet, nor the reverse), the
//!   redirect to HTTPS (the plain-HTTP listener only, once AbleSet's
//!   certificate serves; never by IP or for a tunnel request), and the
//!   answer's headers ([`framing`], [`frame_policy`], [`answer_policy`]):
//!   only fohmixer's own page and AbleSet's own may frame AbleSet, no
//!   other site may read it, and no cookie of it leaves its name.
//! - Before the proxy ([`refusal`], #90's review): only a path is
//!   forwarded, never CONNECT or TRACE, never another site's request that
//!   is no navigation (a LAN browser opens AbleSet without its password),
//!   and at most [`MAX_REQUESTS`] at a time (one more: 503 at once).
//! - The upstream request ([`strip_request`]): no hop-by-hop header, no
//!   forwarding or Cloudflare header, no Access cookie, exactly one `Host`
//!   (the one the dispatch matched). Nothing is added: AbleSet sees what a
//!   browser on the PC sends, with the public `Host` (the owner's decision:
//!   fohmixer's access rules are the lock).
//! - When AbleSet does not answer, or the hub refuses, the hub answers
//!   itself ([`down_answer`]), records every such request (`ableset_proxy`)
//!   and logs it at warn at most once per [`WARN_GAP`] for each kind
//!   ([`warn_line`]).
//! - A WebSocket upgrade ([`is_upgrade`]: Socket.IO's `websocket`
//!   transport, on HTTP/1.1), after the same checks, is a transparent byte
//!   tunnel (B2): AbleSet's 101 goes back to the page and each way's bytes
//!   are copied on their own, an end's close passed on to the other
//!   ([`pipe`]). It ends when both ends closed, when one end is still open
//!   [`HALF_CLOSE_GRACE`] after the other's close, or at the hub's stop
//!   ([`AbleSet::stop`]); its two upgrades get the head bound. At most
//!   [`MAX_SOCKETS`] at a time (one more: 503 at once), each slot held until
//!   the tunnel's close record (or a non-101 answer's body) ends, each
//!   opened and closed in an `ableset_ws` record ([`ws_fields`],
//!   [`WS_RECORDS`] per kind per [`WARN_GAP`]).

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri, Version, header};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use http_body_util::BodyExt as _;
use hyper::upgrade::OnUpgrade;
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt as _};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};
use tower::ServiceExt as _;

use crate::access::{
    AccessGate, JWT_COOKIE, has_proxy_header, is_proxy_header, is_websocket_upgrade, names_token,
};
use crate::config::{AbleSetCfg, AccessCfg, Config, TlsCfg};
use crate::events::EventLog;
use crate::ration::Ration;
use crate::ws::Opener;
use crate::{Hub, routes, tls::Slot};

pub mod proxy;

use proxy::{Failure, Proxy};

/// Hop-by-hop headers (RFC 9110 §7.6.1): never forwarded either way, with
/// every header the `Connection` header names.
pub const HOP_BY_HOP: [&str; 9] = [
    "connection",
    "keep-alive",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "proxy-authenticate",
    "proxy-authorization",
];

/// Headers a `Connection` header never names away (#90): the hub sets
/// AbleSet's `Host` itself, AbleSet's own checks read `Origin`, and the
/// body's length frames the message.
const NEVER_HOP: [&str; 3] = ["host", "origin", "content-length"];

/// At most this many requests are with AbleSet at a time, each until its
/// answer's body ends (#90): a browser opens at most 6 connections to a
/// name and an AbleSet page holds one Socket.IO poll, so this only keeps a
/// flood from piling up on the PC. One more is answered 503 at once.
pub const MAX_REQUESTS: usize = 64;

/// The least time between two warn lines of one kind (#90): AbleSet's own
/// faults, and everything else (a client's, a full hub), so a client's
/// faults never hide AbleSet's. Also the window of the records' rations.
pub const WARN_GAP: Duration = crate::ration::GAP;

/// At most this many WebSocket tunnels to AbleSet at a time (#90 B2): an
/// AbleSet page opens one Engine.IO socket and multiplexes over it, so this
/// only keeps a flood from piling up on the PC. One more is answered 503 at
/// once.
pub const MAX_SOCKETS: usize = 64;

/// At most this many `ableset_ws` records (and info lines) of one kind,
/// `open` or `close`, per [`WARN_GAP`]: a tab opened and closed now and
/// then is all recorded, a client opening sockets in a loop is not (the
/// count left out rides on the next record).
pub const WS_RECORDS: u32 = 8;

/// How long the hub's stop waits for a tunnel's two ends to close (each
/// end's close sent: a FIN, over TLS its `close_notify`).
pub const CLOSE_BOUND: Duration = Duration::from_secs(1);

/// Why a tunnel ended at the hub's stop.
pub const STOPPED: &str = "the hub stops";

/// How long a tunnel's other end may stay open once one end closed (#90
/// review 1). A WebSocket has no use for a half-open connection: RFC 6455
/// §7.1.1 has the server (AbleSet) close the TCP connection first and the
/// client (the page) close its end at once after. 5 s lets that close come
/// back over a weak Wi-Fi, two retransmissions at TCP's 1 s minimum RTO
/// (RFC 6298) included. A page still silent after it (a suspended tab on a
/// tablet that then leaves the network: nothing in flight, so no TCP timer
/// would ever end it) is cut, and its slot comes back.
pub const HALF_CLOSE_GRACE: Duration = Duration::from_secs(5);

/// How long the hub's stop waits for every tunnel to end (#90 review 3):
/// [`CLOSE_BOUND`] for their two ends to close, and half a second for their
/// tasks to write their close records and give their slots back.
pub const CLOSED_WAIT: Duration = Duration::from_millis(1_500);

/// The AbleSet name's side of the hub.
pub struct AbleSet {
    /// The `[ableset]` name.
    pub name: String,
    /// The Access check of its internet requests (`access_aud`); none: they
    /// are refused.
    gate: Option<Arc<AccessGate>>,
    proxy: Proxy,
    /// `frame-ancestors` of fohmixer's own page, on every answer.
    frame_ancestors: HeaderValue,
    events: EventLog,
    /// The last request did not reach AbleSet (one warn per outage).
    failing: AtomicBool,
    /// The requests with AbleSet now ([`MAX_REQUESTS`]).
    slots: Arc<Semaphore>,
    /// The last warn line about AbleSet's own fault, and about anything
    /// else ([`WARN_GAP`]).
    ableset_warned: Mutex<Option<Instant>>,
    other_warned: Mutex<Option<Instant>>,
    /// The taken WebSocket slots, of at most `max_sockets`: the open
    /// tunnels, the upgrades waiting for AbleSet's 101 or for their two
    /// ends, and the non-101 answers still streaming.
    sockets: Arc<Semaphore>,
    max_sockets: usize,
    /// The last tunnel's number (1, 2, …; its own counter).
    tunnels: AtomicU64,
    /// The hub's stop: every tunnel ends at it.
    stop: watch::Sender<bool>,
    /// The records' rations by kind.
    rations: Mutex<HashMap<&'static str, Ration>>,
}

impl AbleSet {
    /// AbleSet behind the `[tls]` name's page, its internet requests checked
    /// by `gate`.
    pub fn new(
        cfg: &AbleSetCfg,
        tls: &TlsCfg,
        gate: Option<Arc<AccessGate>>,
        events: EventLog,
    ) -> Self {
        let proxy = Proxy::new(&cfg.upstream, proxy::HEAD_TIMEOUT, proxy::MAX_BODY);
        Self::with_proxy(&cfg.name, tls, gate, events, proxy)
    }

    fn with_proxy(
        name: &str,
        tls: &TlsCfg,
        gate: Option<Arc<AccessGate>>,
        events: EventLog,
        proxy: Proxy,
    ) -> Self {
        Self::limited(name, tls, gate, events, proxy, MAX_REQUESTS, MAX_SOCKETS)
    }

    /// [`AbleSet::with_proxy`] with `slots` requests and `sockets` tunnels
    /// at a time.
    fn limited(
        name: &str,
        tls: &TlsCfg,
        gate: Option<Arc<AccessGate>>,
        events: EventLog,
        proxy: Proxy,
        slots: usize,
        sockets: usize,
    ) -> Self {
        // AbleSet's own pages may frame each other ('self'), as AbleSet's
        // own policy allows.
        let origins = https_origins(&tls.name, tls.port);
        let ancestors = format!("frame-ancestors 'self' {origins}");
        Self {
            name: name.to_string(),
            gate,
            proxy,
            frame_ancestors: HeaderValue::from_str(&ancestors)
                .expect("DNS names make a header value"),
            events,
            failing: AtomicBool::new(false),
            slots: Arc::new(Semaphore::new(slots)),
            ableset_warned: Mutex::new(None),
            other_warned: Mutex::new(None),
            sockets: Arc::new(Semaphore::new(sockets)),
            max_sockets: sockets,
            tunnels: AtomicU64::new(0),
            stop: watch::Sender::new(false),
            rations: Mutex::default(),
        }
    }

    /// The hub's stop (#90 B2): every tunnel open now, and any opened after
    /// it, is closed.
    pub fn stop(&self) {
        self.stop.send_replace(true);
    }

    /// The taken WebSocket slots (`/api/status` `ableset.sockets`): the open
    /// tunnels, the upgrades not yet carried and the non-101 answers still
    /// streaming.
    pub fn open_sockets(&self) -> usize {
        self.max_sockets - self.sockets.available_permits()
    }

    /// Waits up to `bound` for every tunnel to end after [`AbleSet::stop`]
    /// (#90 review 3: their connections left the listeners at the upgrade,
    /// so no drain waits for them): whether every socket slot came back (a
    /// tunnel gives its slot back only after its close record).
    pub async fn closed(&self, bound: Duration) -> bool {
        let every = u32::try_from(self.max_sockets).unwrap_or(u32::MAX);
        let back = tokio::time::timeout(bound, self.sockets.acquire_many(every)).await;
        matches!(back, Ok(Ok(_)))
    }

    /// Whether a record of `kind` is due, at most `per_gap` per
    /// [`WARN_GAP`]: the count of its kind left out before it.
    fn due(&self, kind: &'static str, per_gap: u32) -> Option<u64> {
        let mut rations = self.rations.lock().unwrap_or_else(PoisonError::into_inner);
        rations
            .entry(kind)
            .or_default()
            .take(Instant::now(), per_gap)
    }

    /// Tunnel `n` opened or closed: its `ableset_ws` record and info line
    /// when the ration of its kind allows ([`WS_RECORDS`]), else a debug
    /// line.
    fn ws_note(&self, n: u64, opener: &Opener, ws: &Ws<'_>) {
        let kind = ws.kind();
        let Some(skipped) = self.due(kind, WS_RECORDS) else {
            tracing::debug!(client = n, "ableset: WebSocket {kind}");
            return;
        };
        match ws {
            Ws::Open => tracing::info!(
                client = n,
                peer = %opener.peer,
                forwarded = ?opener.forwarded,
                source = opener.source,
                origin = ?opener.origin,
                skipped,
                "ableset: WebSocket open"
            ),
            Ws::Close { ms, ended } => tracing::info!(
                client = n,
                ms,
                up = ended.up,
                down = ended.down,
                why = ?ended.why,
                skipped,
                "ableset: WebSocket close"
            ),
        }
        self.events
            .record("ableset_ws", ws_fields(n, opener, ws, skipped));
    }

    /// A tunnel's life (#90 B2): both connections handed over within the
    /// head bound ([`upgraded`]), then its bytes ([`pipe`]) until both ends
    /// closed, one end stayed open [`HALF_CLOSE_GRACE`] past the other's
    /// close, or the hub stopped; then its `close` record, and only then
    /// its socket slot freed.
    async fn carry(
        self: Arc<Self>,
        n: u64,
        opener: Opener,
        page: OnUpgrade,
        ableset: OnUpgrade,
        slot: OwnedSemaphorePermit,
    ) {
        let _slot = slot;
        let started = Instant::now();
        let mut stop = self.stop.subscribe();
        let bound = self.proxy.head_timeout();
        let ended = match upgraded(page, ableset, bound, &mut stop).await {
            Ok((page, ableset)) => {
                let (page, ableset) = (TokioIo::new(page), TokioIo::new(ableset));
                pipe(page, ableset, &mut stop, HALF_CLOSE_GRACE).await
            }
            Err(why) => Ended {
                up: 0,
                down: 0,
                why,
            },
        };
        let ms = started.elapsed().as_millis() as u64;
        self.ws_note(n, &opener, &Ws::Close { ms, ended: &ended });
    }

    /// AbleSet answered: the outage, if any, is over.
    fn reached(&self, method: &Method, path: &str, status: StatusCode, started: Instant) {
        self.failing.store(false, Ordering::Relaxed);
        let ms = started.elapsed().as_millis() as u64;
        tracing::debug!(%method, path, status = status.as_u16(), ms, "ableset: proxied");
    }

    /// AbleSet was not reached: answered by the hub, recorded, and logged
    /// at warn when [`warn_line`] says so (else at debug).
    fn failed(
        &self,
        method: &Method,
        path: &str,
        dest: Option<&str>,
        failure: Failure,
    ) -> Response {
        let error = crate::auth::error_response;
        let answer = match &failure {
            Failure::Method(_) => {
                let allow = [(
                    header::ALLOW,
                    "GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS",
                )];
                let refused = error(
                    StatusCode::METHOD_NOT_ALLOWED,
                    "ABLESET_METHOD",
                    "This method is never sent to AbleSet",
                );
                (allow, refused).into_response()
            }
            Failure::Target(_) => error(
                StatusCode::BAD_REQUEST,
                "ABLESET_TARGET",
                "AbleSet is sent paths only",
            ),
            Failure::CrossSite(_) => crate::access::forbidden("cross_site"),
            Failure::Closed(_) => error(
                StatusCode::BAD_GATEWAY,
                "ABLESET_CLOSED",
                "AbleSet closed the WebSocket request without an answer",
            ),
            Failure::Busy(_) => error(
                StatusCode::SERVICE_UNAVAILABLE,
                "ABLESET_BUSY",
                "Too many requests for AbleSet at once: try again",
            ),
            Failure::TooLarge(_) => error(
                StatusCode::PAYLOAD_TOO_LARGE,
                "ABLESET_TOO_LARGE",
                "The request body is too large for AbleSet",
            ),
            Failure::Down(_) => down_answer(dest, false),
            Failure::Timeout(_) => down_answer(dest, true),
        };
        // Only AbleSet being down makes an outage; its later failures are
        // no news.
        let continues = match failure {
            Failure::Down(_) => self.failing.swap(true, Ordering::Relaxed),
            _ => false,
        };
        let ableset = failure.is_ableset();
        let warned = if ableset {
            &self.ableset_warned
        } else {
            &self.other_warned
        };
        let warn = {
            let mut last = warned.lock().unwrap_or_else(PoisonError::into_inner);
            let warn = warn_line(!continues, last.map(|at| at.elapsed()));
            if warn {
                *last = Some(Instant::now());
            }
            warn
        };
        let why = failure.why();
        let status = answer.status().as_u16();
        if warn {
            tracing::warn!(%method, path, status, "ableset: {why}");
        } else {
            tracing::debug!(%method, path, status, "ableset: {why}");
        }
        let fields = proxy_fields(method.as_str(), path, status, why, warn && ableset);
        self.events.record("ableset_proxy", fields);
        answer
    }
}

/// Whether a request the hub answered itself gets a warn line: only news
/// (`wanted`: not the rest of an outage), and none within [`WARN_GAP`] of
/// the last one of its kind (`since`; none yet: due).
pub fn warn_line(wanted: bool, since: Option<Duration>) -> bool {
    wanted && since.is_none_or(|since| since >= WARN_GAP)
}

/// The `ableset_proxy` record of a request that did not reach AbleSet: why
/// under `error` for an `alarm` (a warned fault of AbleSet itself: a
/// warn-class record, kept past the event log's daily cap), else under
/// `why`, so no client can fill the PC's disk with warn-class records.
pub fn proxy_fields(method: &str, path: &str, status: u16, why: &str, alarm: bool) -> Value {
    let mut fields = json!({ "method": method, "path": path, "status": status });
    fields[if alarm { "error" } else { "why" }] = json!(why);
    fields
}

/// The `[access]` config of AbleSet's gate: `[access]`'s team and keys with
/// `[ableset] access_aud`; none without an AbleSet audience.
pub fn access_cfg(config: &Config) -> Option<AccessCfg> {
    let ableset = config.ableset.as_ref()?;
    let access = config.access.as_ref()?;
    (!ableset.access_aud.is_empty()).then(|| AccessCfg {
        team_domain: access.team_domain.clone(),
        aud: ableset.access_aud.clone(),
        jwks_url: access.jwks_url.clone(),
    })
}

/// `https://<name>`, and `https://<name>:<port>` beside it when the port is
/// not 443: what a page opened through the tunnel (443) or on the LAN (the
/// `[tls]` port) is.
pub fn https_origins(name: &str, port: u16) -> String {
    if port == 443 {
        format!("https://{name}")
    } else {
        format!("https://{name} https://{name}:{port}")
    }
}

/// Whether a `Host` value names AbleSet's name (any port, any case).
pub fn for_ableset(host: Option<&str>, name: &str) -> bool {
    host.and_then(routes::host_name)
        .is_some_and(|(host, _)| host.eq_ignore_ascii_case(name))
}

/// The AbleSet name and its router, the state of [`dispatch`].
#[derive(Clone)]
pub struct Dispatch {
    pub name: String,
    pub router: Router,
}

/// The outermost layer: a request for AbleSet's name goes to its router,
/// any other one on to fohmixer's.
pub async fn dispatch(State(to): State<Dispatch>, request: Request, next: Next) -> Response {
    if for_ableset(routes::request_host(&request), &to.name) {
        let Ok(response) = to.router.oneshot(request).await;
        response
    } else {
        next.run(request).await
    }
}

/// The AbleSet name's application: the proxy behind its Access check, the
/// redirect to HTTPS when `redirect` (the plain-HTTP listener's hub), and
/// the framing headers on every answer.
pub fn router(site: Arc<AbleSet>, redirect: Option<Hub>) -> Router {
    let https = redirect.is_none();
    let access = from_fn_with_state(site.gate.clone(), crate::access::middleware);
    let mut app = Router::new().fallback(forward).layer(access);
    if let Some(hub) = redirect {
        app = app.layer(from_fn_with_state(hub, https_redirect));
    }
    app.layer(from_fn_with_state((Arc::clone(&site), https), framed))
        .with_state(site)
}

/// The proxy's handler, after the Access check: the request to AbleSet,
/// holding a slot until its answer's body ends, or the hub's own answer
/// ([`refusal`], every slot taken, AbleSet not reached).
async fn forward(State(site): State<Arc<AbleSet>>, request: Request) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let dest = request
        .headers()
        .get("sec-fetch-dest")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    if let Some(failure) = refusal(&method, request.uri(), request.headers()) {
        return site.failed(&method, &path, dest.as_deref(), failure);
    }
    if is_upgrade(&method, request.version(), request.headers()) {
        return tunnel(site, request, &path, dest.as_deref()).await;
    }
    let Ok(slot) = Arc::clone(&site.slots).try_acquire_owned() else {
        let busy = Failure::Busy("every request slot for AbleSet is taken".to_string());
        return site.failed(&method, &path, dest.as_deref(), busy);
    };
    let started = Instant::now();
    match site.proxy.send(request).await {
        Ok(response) => {
            site.reached(&method, &path, response.status(), started);
            response.map(|body| held(body, slot))
        }
        Err(failure) => site.failed(&method, &path, dest.as_deref(), failure),
    }
}

/// `body`, holding its request's slot until it ends or is dropped (the
/// client gone): a long poll or a large file counts while it streams.
fn held(body: Body, slot: OwnedSemaphorePermit) -> Body {
    Body::new(body.map_frame(move |frame| {
        let _slot = &slot;
        frame
    }))
}

/// A WebSocket upgrade (#90 B2), after the Access check and the refusals:
/// sent on to AbleSet with its handshake headers. When AbleSet switches
/// (101) the page gets AbleSet's 101, and its connection and AbleSet's
/// become one byte tunnel ([`AbleSet::carry`]) holding a socket slot; any
/// other answer is passed on as it came, holding the slot until its body
/// ends (#90 review 2: no upgrade streams an answer with no slot).
async fn tunnel(
    site: Arc<AbleSet>,
    mut request: Request,
    path: &str,
    dest: Option<&str>,
) -> Response {
    let Ok(slot) = Arc::clone(&site.sockets).try_acquire_owned() else {
        let busy = Failure::Busy("every WebSocket slot for AbleSet is taken".to_string());
        return site.failed(&Method::GET, path, dest, busy);
    };
    // The Access check refused a request without its peer.
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or_else(|| SocketAddr::from(([0, 0, 0, 0], 0)), |info| info.0);
    let opener = crate::ws::opener(peer, request.headers());
    let page = hyper::upgrade::on(&mut request);
    let started = Instant::now();
    match site.proxy.open(request).await {
        Ok(mut answer) if answer.status() == StatusCode::SWITCHING_PROTOCOLS => {
            site.reached(&Method::GET, path, answer.status(), started);
            let ableset = hyper::upgrade::on(&mut answer);
            let n = site.tunnels.fetch_add(1, Ordering::Relaxed) + 1;
            site.ws_note(n, &opener, &Ws::Open);
            tokio::spawn(Arc::clone(&site).carry(n, opener, page, ableset, slot));
            let (mut parts, _) = answer.into_parts();
            strip_hop_by_hop(&mut parts.headers);
            upgrade_headers(&mut parts.headers);
            Response::from_parts(parts, Body::empty())
        }
        Ok(answer) => {
            site.reached(&Method::GET, path, answer.status(), started);
            proxy::answer(answer).map(|body| held(body, slot))
        }
        Err(failure) => site.failed(&Method::GET, path, dest, failure),
    }
}

/// Whether a request is tunneled: a WebSocket upgrade by the Origin
/// guard's own test (`access::is_websocket_upgrade`, #90 review 4), a GET
/// this hop can switch: HTTP/1.1 (hyper offers no upgrade on HTTP/1.0, so
/// AbleSet would switch for nothing, #90 review 5) with a `Connection` that
/// names `upgrade` (RFC 9110 §7.8). Socket.IO's polling is plain HTTP, and
/// so is any other request: its hop-by-hop pair goes with the rest.
pub fn is_upgrade(method: &Method, version: Version, headers: &HeaderMap) -> bool {
    *method == Method::GET
        && version == Version::HTTP_11
        && names_token(headers, &header::CONNECTION, "upgrade")
        && is_websocket_upgrade(headers)
}

/// The upgrade's pair, back after the hop-by-hop headers went: the request
/// to AbleSet and AbleSet's 101 to the page carry `Connection: upgrade` and
/// `Upgrade: websocket`. The `Sec-WebSocket-*` headers are end-to-end and
/// pass as they are, so subprotocols and compression are agreed between
/// the page and AbleSet.
pub fn upgrade_headers(headers: &mut HeaderMap) {
    headers.insert(header::CONNECTION, HeaderValue::from_static("upgrade"));
    headers.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
}

/// A tunnel opened or closed, for its record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ws<'a> {
    Open,
    /// Closed after `ms`, as `ended` says.
    Close {
        ms: u64,
        ended: &'a Ended,
    },
}

impl Ws<'_> {
    /// The record's `what`, and the kind its ration counts.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Close { .. } => "close",
        }
    }
}

/// How a tunnel ended: the bytes it carried each way (`up`: the page's to
/// AbleSet, `down`: AbleSet's to the page; counted as they go, so an
/// error or the stop keeps them, #90 review 9) and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ended {
    pub up: u64,
    pub down: u64,
    pub why: String,
}

/// One way of a tunnel's bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Way {
    /// The page's bytes to AbleSet.
    Up,
    /// AbleSet's bytes to the page.
    Down,
}

impl Way {
    /// The other way.
    pub fn other(self) -> Self {
        match self {
            Self::Up => Self::Down,
            Self::Down => Self::Up,
        }
    }

    /// The way, as a record's `why` names it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Up => "the page to AbleSet",
            Self::Down => "AbleSet to the page",
        }
    }

    /// The end its bytes come from.
    pub fn source(self) -> &'static str {
        match self {
            Self::Up => "the page",
            Self::Down => "AbleSet",
        }
    }
}

/// Why a tunnel ended (#90 review 1, pure): `first` the way that ended
/// first and `how` (its source's close passed on, or an error), `then` how
/// the other way ended within `grace` (none: it was still open, and both
/// ends were dropped). Both closed: `closed`; else the first error, named
/// by its way; else the end that did not close in time.
pub fn why_ended(
    first: Way,
    how: &io::Result<()>,
    then: Option<&io::Result<()>>,
    grace: Duration,
) -> String {
    let other = first.other();
    match (how, then) {
        (Err(e), _) => format!("{}: {e}", first.name()),
        (Ok(()), Some(Err(e))) => format!("{}: {e}", other.name()),
        (Ok(()), Some(Ok(()))) => "closed".to_string(),
        (Ok(()), None) => format!(
            "{} did not close within {} ms of {}'s close",
            other.source(),
            grace.as_millis(),
            first.source()
        ),
    }
}

/// How a way's copy ended (pure): its source closed (`Ok`), or the error
/// that ended it. A page that ends its TLS without `close_notify` makes
/// rustls report `UnexpectedEof`: that is its close too (#90 review 9;
/// only TLS reports it, and only a page's end is TLS).
pub fn source_closed(copied: io::Result<u64>) -> io::Result<()> {
    match copied {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(()),
        Err(e) => Err(e),
    }
}

/// A writer that counts the bytes it took (a way's count, kept through an
/// error or the stop).
struct Counted<'a, W> {
    to: &'a mut W,
    count: &'a AtomicU64,
}

impl<W: AsyncWrite + Unpin> AsyncWrite for Counted<'_, W> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let written = Pin::new(&mut *this.to).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = &written {
            this.count.fetch_add(*n as u64, Ordering::Relaxed);
        }
        written
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.get_mut().to).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.get_mut().to).poll_shutdown(cx)
    }
}

/// One way of a tunnel: `from`'s bytes into `to` (counted in `count`),
/// then `from`'s close passed on (`to` shut down: a FIN, over TLS its
/// `close_notify`).
async fn pass<R, W>(from: &mut R, to: &mut W, count: &AtomicU64) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut counted = Counted { to, count };
    source_closed(tokio::io::copy(from, &mut counted).await)?;
    counted.shutdown().await
}

/// What the way still open did after the other ended.
enum Rest {
    /// It ended too, so.
    Ended(io::Result<()>),
    /// It was still open when the grace ran out.
    Open,
    /// The hub stopped first.
    Stop,
}

/// The way still open after the other ended, for at most `grace`.
async fn rest<F>(way: Pin<&mut F>, stop: &mut watch::Receiver<bool>, grace: Duration) -> Rest
where
    F: Future<Output = io::Result<()>>,
{
    tokio::select! {
        biased;
        () = stopped(stop) => Rest::Stop,
        how = way => Rest::Ended(how),
        () = tokio::time::sleep(grace) => Rest::Open,
    }
}

/// A tunnel's bytes (#90 review 1): each way copied on its own
/// ([`pass`]), its source's close passed on to the other end. Once one way
/// ended the other gets `grace`, then both ends are dropped (a WebSocket
/// has no use for half a connection); at the hub's stop both ends are shut
/// down together (#90 review 8), bounded by [`CLOSE_BOUND`]. How it ended,
/// with the bytes each way so far.
async fn pipe<P, A>(page: P, ableset: A, stop: &mut watch::Receiver<bool>, grace: Duration) -> Ended
where
    P: AsyncRead + AsyncWrite,
    A: AsyncRead + AsyncWrite,
{
    let (up, down) = (AtomicU64::new(0), AtomicU64::new(0));
    let (mut page_in, mut page_out) = tokio::io::split(page);
    let (mut ableset_in, mut ableset_out) = tokio::io::split(ableset);
    let why = {
        let ups = pass(&mut page_in, &mut ableset_out, &up);
        let downs = pass(&mut ableset_in, &mut page_out, &down);
        tokio::pin!(ups, downs);
        let first = tokio::select! {
            biased;
            () = stopped(stop) => None,
            how = &mut ups => Some((Way::Up, how)),
            how = &mut downs => Some((Way::Down, how)),
        };
        match first {
            None => None,
            Some((way, how)) => {
                let after = match way {
                    Way::Up => rest(downs, stop, grace).await,
                    Way::Down => rest(ups, stop, grace).await,
                };
                match after {
                    Rest::Stop => None,
                    Rest::Ended(then) => Some(why_ended(way, &how, Some(&then), grace)),
                    Rest::Open => Some(why_ended(way, &how, None, grace)),
                }
            }
        }
    };
    let why = match why {
        Some(why) => why,
        None => {
            let close = async { tokio::join!(page_out.shutdown(), ableset_out.shutdown()) };
            let _ = tokio::time::timeout(CLOSE_BOUND, close).await;
            STOPPED.to_string()
        }
    };
    Ended {
        up: up.into_inner(),
        down: down.into_inner(),
        why,
    }
}

/// Both ends' upgrades (#90 B2), within `bound` (#90 review 1: AbleSet's
/// head bound, [`proxy::HEAD_TIMEOUT`]): a page whose upgrade never
/// completes holds no slot for good. Else why not: the upgrade failed, did
/// not complete in time, or the hub stopped.
async fn upgraded<P, A, T, E>(
    page: P,
    ableset: A,
    bound: Duration,
    stop: &mut watch::Receiver<bool>,
) -> Result<(T, T), String>
where
    P: Future<Output = Result<T, E>>,
    A: Future<Output = Result<T, E>>,
    E: std::fmt::Display,
{
    let both = tokio::time::timeout(bound, async { tokio::try_join!(page, ableset) });
    tokio::select! {
        biased;
        () = stopped(stop) => Err(STOPPED.to_string()),
        joined = both => match joined {
            Ok(Ok(ends)) => Ok(ends),
            Ok(Err(e)) => Err(format!("the upgrade failed: {e}")),
            Err(_) => Err(format!(
                "the upgrade did not complete within {} ms",
                bound.as_millis()
            )),
        },
    }
}

/// Resolves once the hub stops (at once when it has; a gone hub is one).
async fn stopped(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|stopped| *stopped).await;
}

/// The `ableset_ws` record of tunnel `n` (`client`, as a socket line names
/// its socket): its opener when it opened (`ws::opener`, the Access
/// check's own classification), how long it lived, its bytes and why it
/// ended when it closed; `skipped`: the records of its kind the ration
/// left out before it.
pub fn ws_fields(n: u64, opener: &Opener, ws: &Ws<'_>, skipped: u64) -> Value {
    match ws {
        Ws::Open => json!({
            "what": "open", "client": n, "peer": opener.peer,
            "forwarded": opener.forwarded, "source": opener.source,
            "origin": opener.origin, "skipped": skipped
        }),
        Ws::Close { ms, ended } => json!({
            "what": "close", "client": n, "ms": ms,
            "up": ended.up, "down": ended.down,
            "why": ended.why, "skipped": skipped
        }),
    }
}

/// Why AbleSet's router refuses a request before AbleSet sees it (#90): a
/// method never forwarded (CONNECT, TRACE: 405), a target that is no path
/// (`*`, an authority: 400), and another site's request that is no
/// navigation (403): a LAN browser opens AbleSet without its password, so
/// another site's image, script or fetch must not drive it. A navigation
/// (a link, a bookmark, the iframe of fohmixer's page) passes.
pub fn refusal(method: &Method, uri: &Uri, headers: &HeaderMap) -> Option<Failure> {
    if [Method::CONNECT, Method::TRACE].contains(method) {
        return Some(Failure::Method(format!("{method} is never forwarded")));
    }
    if !uri.path().starts_with('/') {
        return Some(Failure::Target(format!("the target {uri:?} is no path")));
    }
    let fetch = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    };
    let mode = fetch("sec-fetch-mode");
    let cross_site = fetch("sec-fetch-site").eq_ignore_ascii_case("cross-site");
    (cross_site && !mode.eq_ignore_ascii_case("navigate")).then(|| {
        let dest = fetch("sec-fetch-dest");
        Failure::CrossSite(format!(
            "a cross-site request: mode {mode:?}, dest {dest:?}"
        ))
    })
}

/// The plain-HTTP listener's redirect of AbleSet's name to HTTPS (307),
/// under `[tls] redirect_http`, once its certificate serves.
async fn https_redirect(State(hub): State<Hub>, request: Request, next: Next) -> Response {
    let serving = hub
        .https
        .get()
        .filter(|https| https.serves(Slot::AbleSet))
        .map(|https| https.addr().port());
    let name = hub
        .config
        .ableset
        .as_ref()
        .filter(|_| hub.config.tls.as_ref().is_some_and(|tls| tls.redirect_http));
    let (host, proxied, path) = routes::redirect_inputs(&request);
    let target = name.and_then(|cfg| routes::redirect_to(host, proxied, &cfg.name, serving, path));
    match target {
        Some(location) => axum::response::Redirect::temporary(&location).into_response(),
        None => next.run(request).await,
    }
}

/// Every answer for AbleSet's name gets its headers: [`frame_policy`] where
/// the browser is on https (the HTTPS listener, `https`, and a tunnel
/// request: Cloudflare's edge is https), else [`framing`] (the plain
/// listener's LAN: an https Location would point TLS at the plain port),
/// then [`answer_policy`].
async fn framed(
    State((site, https)): State<(Arc<AbleSet>, bool)>,
    request: Request,
    next: Next,
) -> Response {
    let to_https = https || has_proxy_header(request.headers());
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    if to_https {
        frame_policy(headers, &site.frame_ancestors, &site.name);
    } else {
        framing(headers, &site.frame_ancestors);
    }
    answer_policy(headers);
    response
}

/// The answer's framing headers ([`framing`]), and an absolute `http://`
/// Location of AbleSet's name made `https://` (an http navigation inside
/// an https page would be blocked).
pub fn frame_policy(headers: &mut HeaderMap, frame_ancestors: &HeaderValue, name: &str) {
    framing(headers, frame_ancestors);
    let location = headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|l| https_location(l, name));
    if let Some(value) = location.and_then(|l| HeaderValue::from_str(&l).ok()) {
        headers.insert(header::LOCATION, value);
    }
}

/// The answer's framing: `X-Frame-Options` removed (it cannot name an
/// origin), `frame-ancestors` removed from every policy AbleSet sent (two
/// policies are both enforced) and ours added.
pub fn framing(headers: &mut HeaderMap, frame_ancestors: &HeaderValue) {
    headers.remove(header::X_FRAME_OPTIONS);
    let policies: Vec<String> = headers
        .get_all(header::CONTENT_SECURITY_POLICY)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(strip_frame_ancestors)
        .collect();
    headers.remove(header::CONTENT_SECURITY_POLICY);
    for policy in policies {
        if let Ok(value) = HeaderValue::from_str(&policy) {
            headers.append(header::CONTENT_SECURITY_POLICY, value);
        }
    }
    headers.append(header::CONTENT_SECURITY_POLICY, frame_ancestors.clone());
}

/// The answer's other headers (#90): no CORS grant of AbleSet's (no other
/// site's page may read it), its cookies through [`host_only_cookie`],
/// `nosniff` and fohmixer's own `Referrer-Policy`.
pub fn answer_policy(headers: &mut HeaderMap) {
    let cors: Vec<HeaderName> = headers
        .keys()
        .filter(|name| name.as_str().starts_with("access-control-"))
        .cloned()
        .collect();
    for name in cors {
        headers.remove(name);
    }
    let cookies: Vec<HeaderValue> = headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(host_only_cookie)
        .filter_map(|c| HeaderValue::from_str(&c).ok())
        .collect();
    headers.remove(header::SET_COOKIE);
    for cookie in cookies {
        headers.append(header::SET_COOKIE, cookie);
    }
    let nosniff = HeaderValue::from_static("nosniff");
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, nosniff);
    let referrer = HeaderValue::from_static("strict-origin-when-cross-origin");
    headers.insert(header::REFERRER_POLICY, referrer);
}

/// A `Set-Cookie` value of AbleSet's kept on its own name (its `Domain`
/// attribute out: host-only); none for a Cloudflare cookie (a name
/// starting `CF_`, any case), which on a parent domain would shadow
/// fohmixer's Access cookie at Cloudflare's edge.
pub fn host_only_cookie(set_cookie: &str) -> Option<String> {
    let mut parts = set_cookie.split(';').map(str::trim);
    let pair = parts.next().unwrap_or("");
    let name = pair.split('=').next().unwrap_or("").trim();
    if name.get(..3).is_some_and(|p| p.eq_ignore_ascii_case("cf_")) {
        return None;
    }
    let attributes = parts.filter(|attribute| {
        let key = attribute.split('=').next().unwrap_or("").trim();
        !attribute.is_empty() && !key.eq_ignore_ascii_case("domain")
    });
    let kept: Vec<&str> = std::iter::once(pair).chain(attributes).collect();
    Some(kept.join("; "))
}

/// A policy without its `frame-ancestors` directive; none when nothing
/// else is left.
pub fn strip_frame_ancestors(policy: &str) -> Option<String> {
    let kept: Vec<&str> = policy
        .split(';')
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .filter(|d| {
            let name = d.split_whitespace().next().unwrap_or("");
            !name.eq_ignore_ascii_case("frame-ancestors")
        })
        .collect();
    (!kept.is_empty()).then(|| kept.join("; "))
}

/// `https://…` for an absolute `http://` Location of AbleSet's name (its
/// port, path and query kept); none for any other Location.
pub fn https_location(location: &str, name: &str) -> Option<String> {
    let rest = location.strip_prefix("http://")?;
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (host, _) = routes::host_name(&rest[..end])?;
    host.eq_ignore_ascii_case(name)
        .then(|| format!("https://{rest}"))
}

/// The upstream request's headers (#90 §3.3): hop-by-hop ones, every
/// proxy header (`access::PROXY_HEADERS`) and the Access cookie out;
/// everything else (`Host`, `Origin`, the other cookies, …) kept.
pub fn strip_request(headers: &mut HeaderMap) {
    strip_hop_by_hop(headers);
    let forwarding: Vec<_> = headers
        .keys()
        .filter(|name| is_proxy_header(name.as_str()))
        .cloned()
        .collect();
    for name in forwarding {
        headers.remove(name);
    }
    let cookies = without_access_cookie(
        headers
            .get_all(header::COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok()),
    );
    headers.remove(header::COOKIE);
    if let Some(value) = cookies.and_then(|c| HeaderValue::from_str(&c).ok()) {
        headers.insert(header::COOKIE, value);
    }
}

/// Removes the hop-by-hop headers and those `Connection` names (never
/// one of [`NEVER_HOP`]).
pub fn strip_hop_by_hop(headers: &mut HeaderMap) {
    let named: Vec<String> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|name| !NEVER_HOP.contains(&name.as_str()))
        .collect();
    for name in HOP_BY_HOP
        .iter()
        .copied()
        .chain(named.iter().map(String::as_str))
    {
        headers.remove(name);
    }
}

/// The cookies without the Access cookie (the hub's to check, never
/// AbleSet's to see), joined; none when no other is left.
pub fn without_access_cookie<'a>(cookies: impl Iterator<Item = &'a str>) -> Option<String> {
    let kept: Vec<&str> = cookies
        .flat_map(|c| c.split(';'))
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .filter(|pair| pair.split('=').next().map(str::trim) != Some(JWT_COOKIE))
        .collect();
    (!kept.is_empty()).then(|| kept.join("; "))
}

/// The announced size of a request body over `cap`: refused (413) before
/// AbleSet is asked. A chunked body is cut at the cap by the proxy.
pub fn too_large(headers: &HeaderMap, cap: u64) -> Option<u64> {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&size| size > cap)
}

/// The hub's own answer when AbleSet did not answer (`timed_out`: no
/// response head in time, 504; else 502): for a navigation (`Sec-Fetch-Dest`
/// `document` or `iframe`) a page that tries again in 5 s, else JSON (AbleSet's
/// own Socket.IO client reconnects by itself).
pub fn down_answer(dest: Option<&str>, timed_out: bool) -> Response {
    let (status, code) = if timed_out {
        (StatusCode::GATEWAY_TIMEOUT, "ABLESET_TIMEOUT")
    } else {
        (StatusCode::BAD_GATEWAY, "ABLESET_DOWN")
    };
    if !matches!(dest, Some("document" | "iframe")) {
        return crate::auth::error_response(status, code, "AbleSet on the PC does not answer");
    }
    let page = "<!doctype html><html lang=\"sk\"><meta charset=\"utf-8\">\
        <meta http-equiv=\"refresh\" content=\"5\"><title>AbleSet</title>\
        <style>body{font:18px sans-serif;margin:2em}</style>\
        <p>AbleSet na PC neodpovedá. Skúšam znova o 5 s.</p></html>";
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'unsafe-inline'",
            ),
        ],
        page,
    )
        .into_response()
}

#[cfg(test)]
mod tests;
