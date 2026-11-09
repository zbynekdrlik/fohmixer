//! Presenter's stage display behind the hub (#90, the stage tab; the
//! owner's approved `docs/mockups/presenter-stage-v1.html`).
//!
//! - **The stage page on fohmixer's origin** ([`dispatch`]): Presenter's
//!   stage page picks itself by its path (`location.pathname == "/stage"`)
//!   and calls its server at root paths (its assets under `/ui-pkg/`, its
//!   socket at `/live/ws`), so the hub passes exactly those paths
//!   ([`pass`]: [`EXACT`], [`UNDER`]) to `[presenter] url`, on both
//!   listeners, only with `[presenter]`. No fohmixer route uses them. The
//!   tab's frame (`/stage?preview=1`) is then the page's own origin: no
//!   mixed content on the https page, no CORS, the same Access check.
//! - **Reads, and the stage's video** ([`pass`]): GET and HEAD, the
//!   `/live/ws` upgrade (a byte tunnel, `ableset::pipe`, as AbleSet's
//!   socket) and, under [`WHEP`] only, POST, PATCH and DELETE (#90, the
//!   owner's ruling of 2026-11-09: the preview shows what the stage displays
//!   show, NDI video included; the stage page starts its video with WHEP, an
//!   SDP offer `POST`ed, then `PATCH`/`DELETE` on the resource the answer's
//!   `Location` names, [`local_location`]); any other method is 405 and any
//!   other upgrade 400 by the hub itself, and a path with a dot segment 400.
//!   Presenter has no login, so its write API (layouts, slides, Bible,
//!   clears) is never reachable through fohmixer, the internet path
//!   included (fohmixer's origin is behind its Access check and its token
//!   as well); the one write of it is the hub's own `POST /stage/layout` for
//!   a page's held key ([`link`]). WHEP sessions start and end the video
//!   only, nothing on stage. The socket is a
//!   byte tunnel both ways: what Presenter's live socket takes from a client
//!   (its source, `presenter-server/src/live.rs` `InboundMessage`) is a
//!   stage display's presence, heartbeat, diagnostics and goodbye, for its
//!   count of stage displays, nothing that changes what the stage shows.
//! - **Presenter's content on fohmixer's origin** ([`frame_headers`]): its
//!   stage page and assets run as the page's own (the site's app); every
//!   other answer is [`SANDBOXED`] when opened as a document, and no answer
//!   passes a header acting on the whole origin ([`ORIGIN_WIDE`]).
//! - **Its own layers** ([`router`]): the host check, the redirect to HTTPS
//!   (the plain listener) and fohmixer's Access check, as fohmixer's own
//!   routes; on every answer [`frame_headers`]: only fohmixer's own page may
//!   frame it (`X-Frame-Options: SAMEORIGIN`, `frame-ancestors 'self'`), no
//!   CORS grant, `nosniff`.
//! - **When Presenter does not answer** the hub answers itself
//!   ([`down_answer`]) and logs it at warn at most once per
//!   `ration::GAP` per kind, with a `presenter_proxy` record each
//!   ([`FAULT_RECORDS`]).
//! - **The link** ([`link`]): the hub follows Presenter's live socket and
//!   tells every page the `presenter` message through the router.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::uri::{Authority, PathAndQuery, Scheme};
use axum::http::{HeaderValue, Method, StatusCode, Uri, Version, header};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use fohmixer_proto::client::PresenterStatus;
use hyper::upgrade::Upgraded;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioIo};
use serde_json::{Value, json};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};

use crate::ableset::{self, Ended};
use crate::events::EventLog;
use crate::ration::{Ration, clip};
use crate::ws::Opener;
use crate::{Hub, routes};

pub mod link;

/// Presenter's paths the hub passes as they are: its stage page, its list
/// of layouts, its socket, its health and the group colours the page reads.
pub const EXACT: [&str; 5] = [
    "/stage",
    "/stage-displays",
    "/live/ws",
    "/healthz",
    "/group-colors",
];

/// Presenter's path prefixes the hub passes: the stage's own calls, the
/// page's assets (Trunk's `public_url`), Bible output, NDI, presentations
/// and libraries (a song's slides on stage).
pub const UNDER: [&str; 7] = [
    "/stage/",
    "/ui-pkg/",
    "/bible/",
    "/ndi/",
    "/whep/",
    "/presentations/",
    "/libraries/",
];

/// The stage's video (WHEP, #90): the one prefix that takes POST, PATCH and
/// DELETE ([`pass`]).
pub const WHEP: &str = "/whep/";

/// The one socket path ([`Pass::Socket`]).
pub const SOCKET: &str = "/live/ws";

/// The longest a request to Presenter may wait for its answer's head; also
/// the bound of a socket's two upgrades.
pub const HEAD_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a connect to Presenter may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Requests with Presenter at a time (one more: 503 at once).
pub const MAX_REQUESTS: usize = 64;

/// Stage page sockets at a time: a page's PRESENTER pane and, since 1.37.0,
/// its column's miniature (one at a time: the miniature is gone while the
/// tab is open), a few pages at once. A closed page's tunnel ends when
/// Presenter answers the close, else within [`HALF_CLOSE_GRACE`]; a page's
/// remount overlaps its old and new miniature's sockets that long.
pub const MAX_SOCKETS: usize = 16;

/// A tunnel's other end gets this long after one end's close
/// (`ableset::HALF_CLOSE_GRACE`).
pub const HALF_CLOSE_GRACE: Duration = ableset::HALF_CLOSE_GRACE;

/// `presenter_proxy` records per kind per `ration::GAP`.
pub const FAULT_RECORDS: u32 = 4;

/// `presenter_ws` records per kind per `ration::GAP`.
pub const WS_RECORDS: u32 = 8;

/// How long the hub's stop waits for every tunnel to end.
pub const CLOSED_WAIT: Duration = ableset::CLOSED_WAIT;

/// What the hub does with a request ([`pass`], pure).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    /// Not Presenter's: fohmixer's own router.
    No,
    /// A read, or a WHEP request under [`WHEP`], passed to Presenter.
    Page,
    /// The stage page's socket, tunneled to Presenter.
    Socket,
    /// Presenter's path with a method the hub never passes (405).
    Method,
    /// Presenter's path with a WebSocket upgrade it does not take (400).
    Upgrade,
    /// Presenter's path with a `.` or `..` segment (400): no path leaves
    /// the list on Presenter's side.
    Path,
}

/// Whether `path` is one of Presenter's ([`EXACT`], [`UNDER`]).
pub fn is_presenter_path(path: &str) -> bool {
    EXACT.contains(&path) || UNDER.iter().any(|under| path.starts_with(under))
}

/// Whether `path` holds a `.` or `..` segment, plain or percent-encoded
/// (`%2e`, any case).
pub fn dot_segment(path: &str) -> bool {
    path.split('/').any(|segment| {
        let lower = segment.to_ascii_lowercase().replace("%2e", ".");
        lower == "." || lower == ".."
    })
}

/// What the hub does with a request of `method` for `path`, a WebSocket
/// `upgrade` (`ableset::is_upgrade`) or not (pure).
pub fn pass(method: &Method, path: &str, upgrade: bool) -> Pass {
    if !is_presenter_path(path) {
        return Pass::No;
    }
    if dot_segment(path) {
        return Pass::Path;
    }
    match (upgrade, path == SOCKET) {
        (true, true) => Pass::Socket,
        (true, false) => Pass::Upgrade,
        (false, _) if *method == Method::GET || *method == Method::HEAD => Pass::Page,
        (false, _) if whep_write(method, path) => Pass::Page,
        (false, _) => Pass::Method,
    }
}

/// Whether `method` on `path` is the stage video's WHEP exchange (#90): an
/// offer `POST`ed, the session's `PATCH` (ICE) and `DELETE`, under [`WHEP`]
/// only.
pub fn whep_write(method: &Method, path: &str) -> bool {
    path.starts_with(WHEP)
        && (*method == Method::POST || *method == Method::PATCH || *method == Method::DELETE)
}

/// An answer's `Location` as the page must follow it (#90, WHEP's resource):
/// an absolute one at Presenter's own `authority` becomes its path and query
/// on fohmixer's origin (the page never reaches Presenter's address);
/// `None` keeps any other as it came (a relative one already is the page's).
pub fn local_location(location: &str, authority: &Authority) -> Option<String> {
    let uri: Uri = location.parse().ok()?;
    let theirs = uri.authority()?;
    let same = theirs.host().eq_ignore_ascii_case(authority.host())
        && theirs.port_u16().unwrap_or(80) == authority.port_u16().unwrap_or(80);
    same.then(|| {
        uri.path_and_query()
            .map_or_else(|| "/".to_string(), |p| p.as_str().to_string())
    })
}

/// Presenter's side of the hub: the proxy of its stage page and the link's
/// state for `/api/status`.
pub struct Presenter {
    /// `[presenter] url` without a trailing `/`.
    pub base: String,
    authority: Authority,
    client: Client<HttpConnector, Body>,
    events: EventLog,
    slots: Arc<Semaphore>,
    sockets: Arc<Semaphore>,
    tunnels: AtomicU64,
    stop: watch::Sender<bool>,
    rations: Mutex<HashMap<(&'static str, &'static str), Ration>>,
    warned: Mutex<HashMap<&'static str, Instant>>,
    link: Mutex<link::LinkState>,
}

impl Presenter {
    /// Presenter at `base` (`http://<address>[:port]`, as
    /// `config::is_presenter_url` accepts it, its `/` cut).
    pub fn new(base: &str, events: EventLog) -> Self {
        let authority = base
            .strip_prefix("http://")
            .and_then(|a| a.parse::<Authority>().ok())
            .expect("a url config::is_presenter_url accepted");
        let mut connector = HttpConnector::new();
        connector.set_connect_timeout(Some(CONNECT_TIMEOUT));
        connector.set_nodelay(true);
        let client = Client::builder(TokioExecutor::new()).build(connector);
        Self {
            base: base.to_string(),
            authority,
            client,
            events,
            slots: Arc::new(Semaphore::new(MAX_REQUESTS)),
            sockets: Arc::new(Semaphore::new(MAX_SOCKETS)),
            tunnels: AtomicU64::new(0),
            stop: watch::Sender::new(false),
            rations: Mutex::default(),
            warned: Mutex::default(),
            link: Mutex::default(),
        }
    }

    /// The link's news ([`link::LinkState::note`]): its log line and
    /// `presenter_link` record on a change; whether Presenter is up now, on
    /// a change.
    pub fn noted(&self, news: link::News) -> Option<bool> {
        let change = {
            let mut state = self.link.lock().unwrap_or_else(PoisonError::into_inner);
            state.note(news)
        }?;
        match &change {
            link::Change::Up { failures } => {
                tracing::info!(failures, "presenter: link up");
            }
            link::Change::Down { failures, error } => {
                tracing::warn!(failures, "presenter: link DOWN: {error}");
            }
        }
        self.events.record("presenter_link", change.fields());
        Some(change.up())
    }

    /// What `/api/status` shows of the stage: the layouts Presenter listed
    /// (a session's start) and the one on stage (a read or an event).
    pub fn seen(&self, layouts: Option<usize>, current: Option<&str>) {
        let mut state = self.link.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(layouts) = layouts {
            state.layouts = layouts;
        }
        if let Some(current) = current {
            state.current = Some(current.to_string());
        }
    }

    /// `/api/status` `presenter`.
    pub fn status(&self) -> PresenterStatus {
        let state = self.link.lock().unwrap_or_else(PoisonError::into_inner);
        PresenterStatus {
            url: self.base.clone(),
            online: state.link.online,
            last_error: state.error.clone(),
            layouts: state.layouts,
            current: state.current.clone(),
            sockets: MAX_SOCKETS - self.sockets.available_permits(),
        }
    }

    /// Every tunnel ends (the hub's stop): an upgraded connection is no
    /// longer the listener's, so its drain never waits for one.
    pub fn stop(&self) {
        self.stop.send_replace(true);
    }

    /// Waits up to `bound` for every tunnel to end after [`Presenter::stop`]:
    /// whether every socket slot came back.
    pub async fn closed(&self, bound: Duration) -> bool {
        let every = u32::try_from(MAX_SOCKETS).unwrap_or(u32::MAX);
        let back = tokio::time::timeout(bound, self.sockets.acquire_many(every)).await;
        matches!(back, Ok(Ok(_)))
    }

    /// Whether a `record` of `kind` is due ([`Ration`]): the count left out
    /// before it.
    fn due(&self, record: &'static str, kind: &'static str, per_gap: u32) -> Option<u64> {
        let mut rations = self.rations.lock().unwrap_or_else(PoisonError::into_inner);
        rations
            .entry((record, kind))
            .or_default()
            .take(Instant::now(), per_gap)
    }

    /// Whether a warn line of `kind` is due (one per `ration::GAP`).
    fn warn_due(&self, kind: &'static str) -> bool {
        let mut warned = self.warned.lock().unwrap_or_else(PoisonError::into_inner);
        let now = Instant::now();
        let due = warned
            .get(kind)
            .is_none_or(|at| now.saturating_duration_since(*at) >= crate::ration::GAP);
        if due {
            warned.insert(kind, now);
        }
        due
    }

    /// `request` as Presenter is sent it: Presenter's URI, no hop-by-hop or
    /// proxy header, no Access cookie (`ableset::strip_request`), Presenter's
    /// own `Host`.
    fn upstream(&self, request: Request) -> Result<Request, Failure> {
        let (mut parts, body) = request.into_parts();
        let path = parts
            .uri
            .path_and_query()
            .cloned()
            .unwrap_or_else(|| PathAndQuery::from_static("/"));
        parts.uri = Uri::builder()
            .scheme(Scheme::HTTP)
            .authority(self.authority.clone())
            .path_and_query(path)
            .build()
            .map_err(|e| Failure::Target(format!("the upstream URI: {e}")))?;
        parts.version = Version::HTTP_11;
        parts.extensions = axum::http::Extensions::new();
        ableset::strip_request(&mut parts.headers);
        let host = HeaderValue::from_str(self.authority.as_str())
            .map_err(|e| Failure::Target(format!("the Host: {e}")))?;
        parts.headers.insert(header::HOST, host);
        Ok(Request::from_parts(parts, body))
    }

    /// An answer's absolute `Location` at Presenter itself as a path on
    /// fohmixer's origin ([`local_location`]).
    fn localize(&self, headers: &mut axum::http::HeaderMap) {
        let local = headers
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|location| local_location(location, &self.authority))
            .and_then(|path| HeaderValue::from_str(&path).ok());
        if let Some(path) = local {
            headers.insert(header::LOCATION, path);
        }
    }

    /// Sends `request` to Presenter: its answer's head within
    /// [`HEAD_TIMEOUT`], or why none came.
    async fn exchange(
        &self,
        request: Request,
    ) -> Result<axum::http::Response<hyper::body::Incoming>, Failure> {
        match tokio::time::timeout(HEAD_TIMEOUT, self.client.request(request)).await {
            Ok(Ok(answer)) => Ok(answer),
            Ok(Err(e)) => Err(Failure::Down(format!(
                "Presenter does not answer: {}",
                crate::ableset::proxy::chain(&e)
            ))),
            Err(_) => Err(Failure::Timeout(format!(
                "Presenter did not answer within {} ms",
                HEAD_TIMEOUT.as_millis()
            ))),
        }
    }

    /// The hub answered a request itself (`failure`): its record and warn
    /// line, rationed per kind.
    fn failed(
        &self,
        method: &Method,
        path: &str,
        dest: Option<&str>,
        failure: Failure,
    ) -> Response {
        let answer = failure.answer(dest);
        let kind = failure.kind();
        let (method, path) = (clip(method.as_str()), clip(path));
        let status = answer.status().as_u16();
        let why = failure.why();
        if self.warn_due(kind) {
            tracing::warn!(method = %method, path = ?path, status, "presenter: {why}");
        } else {
            tracing::debug!(method = %method, path = ?path, status, "presenter: {why}");
        }
        if let Some(skipped) = self.due("presenter_proxy", kind, FAULT_RECORDS) {
            self.events.record(
                "presenter_proxy",
                proxy_fields(&method, &path, status, &failure, skipped),
            );
        }
        answer
    }

    /// A socket opened or closed: its `presenter_ws` record and info line
    /// when its ration allows, else a debug line.
    fn ws_note(&self, n: u64, opener: &Opener, ended: Option<(u64, &Ended)>) {
        let kind = if ended.is_some() { "close" } else { "open" };
        let Some(skipped) = self.due("presenter_ws", kind, WS_RECORDS) else {
            tracing::debug!(client = n, "presenter: socket {kind}");
            return;
        };
        tracing::info!(client = n, peer = %opener.peer, skipped, "presenter: socket {kind}");
        self.events
            .record("presenter_ws", ws_fields(n, opener, ended, skipped));
    }

    /// A socket's life: both upgrades within [`HEAD_TIMEOUT`], then its bytes
    /// until both ends closed, one end stayed open [`HALF_CLOSE_GRACE`] past
    /// the other's close, or the hub stopped; then its close record, and
    /// only then its slot freed.
    async fn carry<P, A>(
        self: Arc<Self>,
        n: u64,
        opener: Opener,
        page: P,
        presenter: A,
        slot: OwnedSemaphorePermit,
    ) where
        P: std::future::Future<Output = Result<Upgraded, hyper::Error>>,
        A: std::future::Future<Output = Result<Upgraded, hyper::Error>>,
    {
        let _slot = slot;
        let started = Instant::now();
        let mut stop = self.stop.subscribe();
        let ended = match ableset::upgraded(page, presenter, HEAD_TIMEOUT, &mut stop).await {
            Ok((page, presenter)) => {
                let (page, presenter) = (TokioIo::new(page), TokioIo::new(presenter));
                ableset::pipe(page, presenter, &mut stop, HALF_CLOSE_GRACE).await
            }
            Err(why) => Ended {
                up: 0,
                down: 0,
                why,
            },
        };
        let ms = started.elapsed().as_millis() as u64;
        self.ws_note(n, &opener, Some((ms, &ended)));
    }
}

/// Why the hub answered a request itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// A method never passed (405).
    Method(String),
    /// An upgrade on a path that takes none (400).
    Upgrade(String),
    /// A path with a dot segment, or one that makes no URI (400).
    Target(String),
    /// Every request or socket slot is taken (503).
    Busy(String),
    /// Presenter could not be reached (502).
    Down(String),
    /// Presenter did not answer in time (504).
    Timeout(String),
}

impl Failure {
    /// Its words.
    pub fn why(&self) -> &str {
        match self {
            Self::Method(why)
            | Self::Upgrade(why)
            | Self::Target(why)
            | Self::Busy(why)
            | Self::Down(why)
            | Self::Timeout(why) => why,
        }
    }

    /// Its kind, as its records are counted.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Method(_) => "method",
            Self::Upgrade(_) => "upgrade",
            Self::Target(_) => "target",
            Self::Busy(_) => "busy",
            Self::Down(_) => "down",
            Self::Timeout(_) => "timeout",
        }
    }

    /// Whether it is Presenter's own fault (an outage), not the client's.
    pub fn is_presenter(&self) -> bool {
        matches!(self, Self::Down(_) | Self::Timeout(_))
    }

    /// The hub's answer to a request whose `Sec-Fetch-Dest` is `dest`.
    pub fn answer(&self, dest: Option<&str>) -> Response {
        let error = crate::auth::error_response;
        match self {
            Self::Method(_) => (
                [(header::ALLOW, "GET, HEAD")],
                error(
                    StatusCode::METHOD_NOT_ALLOWED,
                    "PRESENTER_METHOD",
                    "Presenter is only read through fohmixer",
                ),
            )
                .into_response(),
            Self::Upgrade(_) => error(
                StatusCode::BAD_REQUEST,
                "PRESENTER_UPGRADE",
                "Only Presenter's live socket is passed",
            ),
            Self::Target(_) => error(
                StatusCode::BAD_REQUEST,
                "PRESENTER_TARGET",
                "Presenter is sent its own paths only",
            ),
            Self::Busy(_) => error(
                StatusCode::SERVICE_UNAVAILABLE,
                "PRESENTER_BUSY",
                "Too many requests for Presenter at once: try again",
            ),
            Self::Down(_) => down_answer(dest, false),
            Self::Timeout(_) => down_answer(dest, true),
        }
    }
}

/// The `presenter_proxy` record of a request the hub answered itself: an
/// `error` (warn class) for Presenter's own fault, else `why`.
pub fn proxy_fields(
    method: &str,
    path: &str,
    status: u16,
    failure: &Failure,
    skipped: u64,
) -> Value {
    let word = if failure.is_presenter() {
        "error"
    } else {
        "why"
    };
    let mut fields = json!({
        "method": method, "path": path, "status": status,
        "kind": failure.kind(), "skipped": skipped
    });
    fields[word] = json!(failure.why());
    fields
}

/// The `presenter_ws` record of socket `n`: its opener at the open; how
/// long it lived, its bytes and why it ended at the close.
pub fn ws_fields(n: u64, opener: &Opener, ended: Option<(u64, &Ended)>, skipped: u64) -> Value {
    match ended {
        None => json!({
            "what": "open", "client": n, "peer": opener.peer,
            "forwarded": opener.forwarded, "source": opener.source, "skipped": skipped
        }),
        Some((ms, ended)) => json!({
            "what": "close", "client": n, "ms": ms, "up": ended.up,
            "down": ended.down, "why": ended.why, "skipped": skipped
        }),
    }
}

/// The hub's own answer when Presenter did not answer (`timed_out`: 504,
/// else 502): for a navigation (`document`, `iframe`) a small page that
/// tries again in 5 s, else JSON.
pub fn down_answer(dest: Option<&str>, timed_out: bool) -> Response {
    let (status, code) = if timed_out {
        (StatusCode::GATEWAY_TIMEOUT, "PRESENTER_TIMEOUT")
    } else {
        (StatusCode::BAD_GATEWAY, "PRESENTER_DOWN")
    };
    if !matches!(dest, Some("document" | "iframe")) {
        return crate::auth::error_response(status, code, "Presenter does not answer");
    }
    let page = "<!doctype html><html lang=\"en\"><meta charset=\"utf-8\">\
        <meta name=\"color-scheme\" content=\"dark\">\
        <meta http-equiv=\"refresh\" content=\"5\"><title>Presenter</title>\
        <style>body{font:18px sans-serif;margin:2em}</style>\
        <p>Presenter can't be reached. Retrying in 5 s.</p></html>";
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

/// `frame-ancestors` of every answer on Presenter's paths: fohmixer's own
/// page only.
pub const FRAME_ANCESTORS: &str = "frame-ancestors 'self'";

/// The policy of every answer on Presenter's paths but its stage page and
/// its assets ([`runs_as_page`]): opened as a document, Presenter's content
/// (a library's file, a presentation's) runs sandboxed on an opaque origin
/// with nothing to load, never as fohmixer's own page; fetched by the stage
/// page (JSON, an image, a video) it changes nothing.
pub const SANDBOXED: &str = "sandbox; default-src 'none'";

/// Headers of Presenter's answers that act on fohmixer's whole origin (its
/// storage, the service worker's scope, a timed navigation): never passed.
pub const ORIGIN_WIDE: [&str; 3] = ["clear-site-data", "service-worker-allowed", "refresh"];

/// Whether `path` is Presenter's stage page or its build's assets: the one
/// app the hub runs on fohmixer's origin (the site's own, from Presenter's
/// own build).
pub fn runs_as_page(path: &str) -> bool {
    path == "/stage" || path.starts_with("/ui-pkg/")
}

/// Every answer's headers on Presenter's paths (`path`, the request's):
/// framed by fohmixer's own page only (`X-Frame-Options: SAMEORIGIN`,
/// Presenter's own `frame-ancestors` out and [`FRAME_ANCESTORS`] in), no
/// CORS grant, cookies host-only, `nosniff`, fohmixer's `Referrer-Policy`
/// (`ableset::framing`, `ableset::answer_policy`), no [`ORIGIN_WIDE`]
/// header, and [`SANDBOXED`] on all but the stage page and its assets.
pub fn frame_headers(path: &str, headers: &mut axum::http::HeaderMap) {
    ableset::framing(headers, &HeaderValue::from_static(FRAME_ANCESTORS));
    headers.insert(
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("SAMEORIGIN"),
    );
    ableset::answer_policy(headers);
    for name in ORIGIN_WIDE {
        headers.remove(name);
    }
    if !runs_as_page(path) {
        headers.append(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(SANDBOXED),
        );
    }
}

/// The state of [`dispatch`]: Presenter's router.
#[derive(Clone)]
pub struct Dispatch {
    pub router: Router,
}

/// The layer in front of fohmixer's router: a request for one of
/// Presenter's paths goes to Presenter's router, any other one on.
pub async fn dispatch(State(to): State<Dispatch>, request: Request, next: Next) -> Response {
    if is_presenter_path(request.uri().path()) {
        let Ok(response) = tower::ServiceExt::oneshot(to.router, request).await;
        response
    } else {
        next.run(request).await
    }
}

/// Presenter's paths' application: the proxy behind the host check, the
/// redirect to HTTPS (`redirect`: the plain listener) and fohmixer's Access
/// check, [`frame_headers`] on every answer.
pub fn router(site: Arc<Presenter>, hub: Hub, redirect: bool) -> Router {
    let guard = crate::access::Guard {
        gate: hub.access.clone(),
        lines: hub.deny_lines.clone(),
    };
    let mut app = Router::new()
        .fallback(forward)
        .with_state(site)
        .layer(from_fn_with_state(guard, crate::access::middleware));
    if redirect {
        app = app.layer(from_fn_with_state(hub.clone(), routes::https_redirect));
    }
    app.layer(from_fn_with_state(hub, routes::check_host))
        .layer(axum::middleware::from_fn(framed))
}

/// [`frame_headers`] on the answer to `request`.
async fn framed(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let mut response = next.run(request).await;
    frame_headers(&path, response.headers_mut());
    response
}

/// The proxy's handler, after the Access check.
async fn forward(State(site): State<Arc<Presenter>>, request: Request) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let dest = request
        .headers()
        .get("sec-fetch-dest")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let dest = dest.as_deref();
    let upgrade = ableset::is_upgrade(&method, request.version(), request.headers());
    match pass(&method, &path, upgrade) {
        Pass::Page => {}
        Pass::Socket => return tunnel(site, request, &path, dest).await,
        Pass::Method => {
            let why = Failure::Method(format!("{method} is never passed to Presenter"));
            return site.failed(&method, &path, dest, why);
        }
        Pass::Upgrade => {
            let why = Failure::Upgrade("an upgrade on a path that takes none".to_string());
            return site.failed(&method, &path, dest, why);
        }
        Pass::Path => {
            let why = Failure::Target("a path with a dot segment".to_string());
            return site.failed(&method, &path, dest, why);
        }
        Pass::No => {
            // `dispatch` sends only Presenter's paths here.
            let why = Failure::Target("not one of Presenter's paths".to_string());
            return site.failed(&method, &path, dest, why);
        }
    }
    let Ok(slot) = Arc::clone(&site.slots).try_acquire_owned() else {
        let busy = Failure::Busy("every request slot for Presenter is taken".to_string());
        return site.failed(&method, &path, dest, busy);
    };
    let sent = match site.upstream(request) {
        Ok(sent) => sent,
        Err(failure) => return site.failed(&method, &path, dest, failure),
    };
    match site.exchange(sent).await {
        Ok(answer) => {
            tracing::debug!(method = %method, path = ?clip(&path), status = answer.status().as_u16(), "presenter: proxied");
            let mut answer = ableset::proxy::answer(answer).map(|body| ableset::held(body, slot));
            site.localize(answer.headers_mut());
            answer
        }
        Err(failure) => site.failed(&method, &path, dest, failure),
    }
}

/// The stage page's socket: refused before it takes a slot when it
/// announces a body (`ableset::upgrade_body`), sent on to Presenter with its
/// handshake; on Presenter's 101 the page gets it and the two connections
/// become one byte tunnel ([`Presenter::carry`]); any other answer goes back
/// as it came.
async fn tunnel(
    site: Arc<Presenter>,
    mut request: Request,
    path: &str,
    dest: Option<&str>,
) -> Response {
    if ableset::upgrade_body(request.headers()).is_some() {
        let why = Failure::Upgrade("a WebSocket upgrade announced a request body".to_string());
        return site.failed(&Method::GET, path, dest, why);
    }
    let Ok(slot) = Arc::clone(&site.sockets).try_acquire_owned() else {
        let busy = Failure::Busy("every socket slot for Presenter is taken".to_string());
        return site.failed(&Method::GET, path, dest, busy);
    };
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or_else(|| SocketAddr::from(([0, 0, 0, 0], 0)), |info| info.0);
    let opener = crate::ws::opener(peer, request.headers());
    let page = hyper::upgrade::on(&mut request);
    let mut sent = match site.upstream(request) {
        Ok(sent) => sent,
        Err(failure) => return site.failed(&Method::GET, path, dest, failure),
    };
    ableset::upgrade_headers(sent.headers_mut());
    match site.exchange(sent).await {
        Ok(mut answer) if answer.status() == StatusCode::SWITCHING_PROTOCOLS => {
            let presenter = hyper::upgrade::on(&mut answer);
            let n = site.tunnels.fetch_add(1, Ordering::Relaxed) + 1;
            site.ws_note(n, &opener, None);
            tokio::spawn(Arc::clone(&site).carry(n, opener, page, presenter, slot));
            let (mut parts, _) = answer.into_parts();
            ableset::strip_hop_by_hop(&mut parts.headers);
            ableset::upgrade_headers(&mut parts.headers);
            Response::from_parts(parts, Body::empty())
        }
        Ok(answer) => ableset::proxy::answer(answer).map(|body| ableset::held(body, slot)),
        Err(failure) => site.failed(&Method::GET, path, dest, failure),
    }
}

#[cfg(test)]
mod tests;
