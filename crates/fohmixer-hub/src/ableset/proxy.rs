//! The HTTP leg of AbleSet's proxy (#90 §3.2, §3.4): one hyper-util legacy
//! client per hub, after axum's own `examples/reverse-proxy`. The request
//! goes to `upstream` with its path and query unchanged, its headers
//! filtered (`ableset::strip_request`); the bodies stream both ways, never
//! buffered. Bounded: the connect (2 s: on loopback a dead AbleSet refuses
//! at once), the response head ([`HEAD_TIMEOUT`]), the request body
//! ([`MAX_BODY`]); the response body is not (a long poll or a large asset
//! must not be cut).

use std::error::Error as StdError;
use std::time::Duration;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{Extensions, Version};
use axum::response::Response;
use http_body_util::{LengthLimitError, Limited};
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
            Self::TooLarge(why) | Self::Down(why) | Self::Timeout(why) => why,
        }
    }
}

/// The client to AbleSet's web app.
pub struct Proxy {
    client: Client<HttpConnector, Body>,
    /// `http://<loopback>[:port]` (`config::is_upstream`).
    upstream: String,
    head_timeout: Duration,
    max_body: u64,
}

impl Proxy {
    /// A client to `upstream` with these bounds.
    pub fn new(upstream: &str, head_timeout: Duration, max_body: u64) -> Self {
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
            upstream: upstream.to_string(),
            head_timeout,
            max_body,
        }
    }

    /// Forwards `request` to AbleSet: its answer (hop-by-hop headers
    /// removed), or why it did not get one.
    pub async fn send(&self, request: Request) -> Result<Response, Failure> {
        let (mut parts, body) = request.into_parts();
        if let Some(size) = super::too_large(&parts.headers, self.max_body) {
            return Err(Failure::TooLarge(format!(
                "a request body of {size} bytes is over the {} bytes AbleSet is sent",
                self.max_body
            )));
        }
        let path = parts.uri.path_and_query().map_or("/", |pq| pq.as_str());
        parts.uri = format!("{}{path}", self.upstream)
            .parse()
            .map_err(|e| Failure::Down(format!("the upstream URI: {e}")))?;
        parts.version = Version::HTTP_11;
        parts.extensions = Extensions::new();
        super::strip_request(&mut parts.headers);
        let body = Body::new(Limited::new(body, self.max_body as usize));
        let exchange = self.client.request(Request::from_parts(parts, body));
        let response = match tokio::time::timeout(self.head_timeout, exchange).await {
            Ok(Ok(response)) => response,
            Ok(Err(e)) if over_limit(&e) => {
                return Err(Failure::TooLarge(format!(
                    "a request body went over the {} bytes AbleSet is sent",
                    self.max_body
                )));
            }
            Ok(Err(e)) => {
                return Err(Failure::Down(format!(
                    "AbleSet does not answer: {}",
                    chain(&e)
                )));
            }
            Err(_) => {
                return Err(Failure::Timeout(format!(
                    "AbleSet did not answer within {} ms",
                    self.head_timeout.as_millis()
                )));
            }
        };
        let (mut parts, body) = response.into_parts();
        super::strip_hop_by_hop(&mut parts.headers);
        Ok(Response::from_parts(parts, Body::new(body)))
    }
}

/// An error and its causes, `: `-joined (hyper's own text alone says only
/// "client error (Connect)").
pub fn chain(error: &(dyn StdError + 'static)) -> String {
    std::iter::successors(Some(error), |e| e.source())
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(": ")
}

/// Whether the request failed because its body went over the cap.
pub fn over_limit(error: &(dyn StdError + 'static)) -> bool {
    std::iter::successors(Some(error), |e| e.source()).any(|e| e.is::<LengthLimitError>())
}
