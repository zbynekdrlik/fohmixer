//! The HTTP leg of AbleSet's proxy (#90 §3.2, §3.4): one hyper-util legacy
//! client per hub, after axum's own `examples/reverse-proxy`. The request
//! goes to `upstream` with its path and query unchanged (the URI built
//! from its parts), its headers filtered (`ableset::strip_request`) with
//! exactly one `Host`, the one the dispatch matched; the bodies stream
//! both ways, never buffered. Bounded: the connect (2 s: on loopback a dead AbleSet refuses
//! at once), the response head ([`HEAD_TIMEOUT`]), the request body
//! ([`MAX_BODY`]); the response body is not (a long poll or a large asset
//! must not be cut). A WebSocket upgrade ([`Proxy::open`], B2) goes the
//! same way with its pair back; the client makes its connections with
//! upgrades, so AbleSet's 101 can be taken over. An upgrade AbleSet reads
//! and closes without an answer is the client's fault ([`failure`]).

use std::error::Error as StdError;
use std::time::Duration;

use axum::body::Body;
use axum::extract::Request;
use axum::http::uri::{Authority, PathAndQuery, Scheme};
use axum::http::{Extensions, HeaderValue, Uri, Version, header};
use axum::response::Response;
use http_body_util::{LengthLimitError, Limited};
use hyper::body::Incoming;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::{TokioExecutor, TokioTimer};

/// How long a connection to AbleSet may take.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// How long AbleSet may take to answer with its response head: Socket.IO
/// long-polls hold a GET up to its ping interval (25 s by default).
pub const HEAD_TIMEOUT: Duration = Duration::from_secs(60);
/// The largest request body forwarded: 64 MiB (AbleSet's uploads are far
/// smaller), a literal so cargo-mutants has no operator to mutate.
pub const MAX_BODY: u64 = 67_108_864;
/// How long an idle connection to AbleSet is kept.
pub const POOL_IDLE: Duration = Duration::from_secs(30);

/// Why a request did not reach AbleSet (the text for the log and the
/// record).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failure {
    /// A method never forwarded (CONNECT, TRACE): 405.
    Method(String),
    /// A target that is no path (`*`, an authority): 400.
    Target(String),
    /// Another site's request that is no navigation: 403.
    CrossSite(String),
    /// A WebSocket upgrade that announces a body
    /// (`ableset::upgrade_body`): 400.
    UpgradeBody(String),
    /// AbleSet read a WebSocket upgrade and closed the connection without
    /// an answer (it serves no WebSocket there): 502, the client's fault.
    Closed(String),
    /// Every request slot is taken (`ableset::MAX_REQUESTS`): 503.
    Busy(String),
    /// The request body is over [`MAX_BODY`].
    TooLarge(String),
    /// AbleSet does not answer (refused, reset, …).
    Down(String),
    /// No response head within [`HEAD_TIMEOUT`].
    Timeout(String),
}

impl Failure {
    /// Why, for the log and the record.
    pub fn why(&self) -> &str {
        match self {
            Self::Method(why)
            | Self::Target(why)
            | Self::CrossSite(why)
            | Self::UpgradeBody(why)
            | Self::Closed(why)
            | Self::Busy(why)
            | Self::TooLarge(why)
            | Self::Down(why)
            | Self::Timeout(why) => why,
        }
    }

    /// Whether it is AbleSet's own fault (down, late), not a client's or a
    /// full hub's.
    pub fn is_ableset(&self) -> bool {
        matches!(self, Self::Down(_) | Self::Timeout(_))
    }
}

/// The client to AbleSet's web app.
pub struct Proxy {
    client: Client<HttpConnector, Body>,
    /// The authority of `upstream`: `<loopback>[:port]`.
    authority: Authority,
    head_timeout: Duration,
    max_body: u64,
}

impl Proxy {
    /// A client to `upstream` (`http://<loopback>[:port]`, as
    /// `config::is_upstream` accepts it) with these bounds.
    pub fn new(upstream: &str, head_timeout: Duration, max_body: u64) -> Self {
        let authority = upstream
            .strip_prefix("http://")
            .and_then(|a| a.parse::<Authority>().ok())
            .expect("an upstream config::is_upstream accepted");
        let mut connector = HttpConnector::new();
        connector.set_connect_timeout(Some(CONNECT_TIMEOUT));
        connector.set_nodelay(true);
        let client = Client::builder(TokioExecutor::new())
            .pool_idle_timeout(POOL_IDLE)
            .pool_max_idle_per_host(8)
            .pool_timer(TokioTimer::new())
            .build(connector);
        Self {
            client,
            authority,
            head_timeout,
            max_body,
        }
    }

    /// How long AbleSet may take to answer with its head: also the bound
    /// of a tunnel's two upgrades (#90 review 1).
    pub fn head_timeout(&self) -> Duration {
        self.head_timeout
    }

    /// Forwards `request` to AbleSet: its answer (hop-by-hop headers
    /// removed), or why it did not get one.
    pub async fn send(&self, request: Request) -> Result<Response, Failure> {
        let request = self.upstream(request)?;
        self.exchange(request, false).await.map(answer)
    }

    /// Forwards a WebSocket upgrade to AbleSet (#90 B2): the request with
    /// the upgrade's pair back (`ableset::upgrade_headers`); AbleSet's
    /// answer as it came, so a 101 can be taken over (`hyper::upgrade::on`).
    pub async fn open(&self, request: Request) -> Result<Response<Incoming>, Failure> {
        let mut request = self.upstream(request)?;
        super::upgrade_headers(request.headers_mut());
        self.exchange(request, true).await
    }

    /// `request` as AbleSet is sent it, or why it is not sent (a body
    /// announced over the cap, a target that makes no URI).
    fn upstream(&self, request: Request) -> Result<Request, Failure> {
        let (mut parts, body) = request.into_parts();
        if let Some(size) = super::too_large(&parts.headers, self.max_body) {
            return Err(Failure::TooLarge(format!(
                "a request body of {size} bytes is over the {} bytes AbleSet is sent",
                self.max_body
            )));
        }
        // The Host the dispatch matched: the first, else the target's
        // authority (HTTP/2).
        let authority = parts.uri.authority().map(Authority::as_str);
        let host = match parts.headers.get(header::HOST) {
            Some(host) => Some(host.clone()),
            None => authority.and_then(|a| HeaderValue::from_str(a).ok()),
        };
        let path = parts.uri.path_and_query().cloned();
        let path = path.unwrap_or_else(|| PathAndQuery::from_static("/"));
        parts.uri = Uri::builder()
            .scheme(Scheme::HTTP)
            .authority(self.authority.clone())
            .path_and_query(path)
            .build()
            .map_err(|e| Failure::Target(format!("the upstream URI: {e}")))?;
        parts.version = Version::HTTP_11;
        parts.extensions = Extensions::new();
        super::strip_request(&mut parts.headers);
        if let Some(host) = host {
            parts.headers.insert(header::HOST, host);
        }
        let body = Body::new(Limited::new(body, self.max_body as usize));
        Ok(Request::from_parts(parts, body))
    }

    /// Sends `request` (a WebSocket `upgrade` or not) to AbleSet: its
    /// answer's head within the head timeout, or why none came.
    async fn exchange(
        &self,
        request: Request,
        upgrade: bool,
    ) -> Result<Response<Incoming>, Failure> {
        let exchange = self.client.request(request);
        match tokio::time::timeout(self.head_timeout, exchange).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(e)) => Err(failure(&e, upgrade, self.max_body)),
            Err(_) => Err(Failure::Timeout(format!(
                "AbleSet did not answer within {} ms",
                self.head_timeout.as_millis()
            ))),
        }
    }
}

/// Why a request AbleSet did not answer failed (pure): its body over
/// `max_body`; a WebSocket `upgrade` AbleSet read and closed without an
/// answer (#90 review 2: the client asked for a WebSocket AbleSet does not
/// serve there, never an outage); else AbleSet down (a refused or failed
/// connect always: it never read the request).
pub fn failure(error: &(dyn StdError + 'static), upgrade: bool, max_body: u64) -> Failure {
    if over_limit(error) {
        Failure::TooLarge(format!(
            "a request body went over the {max_body} bytes AbleSet is sent"
        ))
    } else if upgrade && closed_unanswered(error) {
        Failure::Closed(format!(
            "AbleSet closed the WebSocket request without an answer: {}",
            chain(error)
        ))
    } else {
        Failure::Down(format!("AbleSet does not answer: {}", chain(error)))
    }
}

/// Whether AbleSet closed the connection after it read the request, before
/// any answer (hyper's incomplete message); a connect error never is one.
pub fn closed_unanswered(error: &(dyn StdError + 'static)) -> bool {
    causes(error).any(|e| {
        e.downcast_ref::<hyper::Error>()
            .is_some_and(hyper::Error::is_incomplete_message)
    })
}

/// AbleSet's answer as the page gets it: its hop-by-hop headers removed,
/// its body streamed.
pub fn answer(response: Response<Incoming>) -> Response {
    let (mut parts, body) = response.into_parts();
    super::strip_hop_by_hop(&mut parts.headers);
    Response::from_parts(parts, Body::new(body))
}

/// An error and its causes, `: `-joined (hyper's own text alone says only
/// "client error (Connect)").
pub fn chain(error: &(dyn StdError + 'static)) -> String {
    causes(error)
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

/// Whether the request failed because its body went over the cap.
pub fn over_limit(error: &(dyn StdError + 'static)) -> bool {
    causes(error).any(|e| e.is::<LengthLimitError>())
}

/// An error, then each of its sources.
fn causes<'a>(
    error: &'a (dyn StdError + 'static),
) -> impl Iterator<Item = &'a (dyn StdError + 'static)> {
    std::iter::successors(Some(error), |&e| e.source())
}
