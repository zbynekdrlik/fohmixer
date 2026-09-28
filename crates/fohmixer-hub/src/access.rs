//! Access control in front of every route (#17), ported from restreamer's
//! `crates/rs-api/src/access.rs` (its #273 model) — read
//! `.claude/rules/remote-access.md` before changing it.
//!
//! Two layers. The outer one is a Cloudflare Access application in front of
//! the tunnel's public name: an unauthenticated request never reaches the
//! PC. This module is the inner one: it verifies the signed Access
//! assertion again in the hub, so a second ingress rule, a router
//! port-forward or another cloudflared cannot bypass the edge.
//!
//! Every request is classified [`Origin::Local`] or [`Origin::Internet`]:
//! a request that carries any forwarded header ([`PROXY_HEADERS`]: the
//! tunnel sets them) or comes from a public address is `Internet`, anything
//! else (the church LAN, the PC itself, a private or CGNAT address) is
//! `Local`. Then, in this order:
//!
//! 1. **Origin guard** (both classes): a browser's cross-site POST or
//!    WebSocket upgrade is refused — a page on the internet opened by a LAN
//!    browser must not drive the mixer or try PINs from that browser
//!    (CSRF; a WebSocket upgrade is exempt from CORS). `check_host` already
//!    refused a foreign `Host` (DNS rebinding).
//! 2. **LAN is never authenticated, and the `Local` branch does no network
//!    I/O at all**: with the internet, Cloudflare or the tunnel down, the
//!    mixer still opens on the LAN (and on the emergency plain-http path).
//! 3. **Internet** needs a valid Access JWT (`Cf-Access-Jwt-Assertion`, or
//!    the `CF_Authorization` cookie a browser and its WebSocket upgrade
//!    carry), RS256 against the team's key set, with `exp`, `aud` and `iss`
//!    required. Without `[access]` in the config every internet request is
//!    refused.
//!
//! The engineer PIN stays required behind it everywhere (`auth.rs`).

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::Response;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use tokio::sync::{Mutex, RwLock};

use crate::Hub;
use crate::config::AccessCfg;
use crate::http_client::HttpClient;

/// Headers a proxy sets and a request made straight to the hub never
/// carries. Any of them makes a request `Internet`: a LAN client that sends
/// one only makes itself need Access (the safe direction).
pub const PROXY_HEADERS: [&str; 5] = [
    "cf-connecting-ip",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-proto",
    "forwarded",
];

/// The header Cloudflare Access injects with the signed assertion.
pub const JWT_HEADER: &str = "cf-access-jwt-assertion";
/// The cookie Access sets in the browser: what a page load, its `fetch`es
/// and the `/ws` upgrade carry.
pub const JWT_COOKIE: &str = "CF_Authorization";

/// How long a fetched key set is served before a refresh; a failed refresh
/// keeps the last good set (a network blip never locks a remote user out).
pub const JWKS_MAX_AGE: Duration = Duration::from_secs(6 * 60 * 60);
/// The least time between two key fetches a request can trigger (an
/// unknown `kid` from the internet must not make the hub hammer Cloudflare).
pub const JWKS_MIN_REFRESH: Duration = Duration::from_secs(60);

/// Where a request came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The PC or the church network. Never authenticated.
    Local,
    /// Through the tunnel (a proxy) or from a public address.
    Internet,
}

/// Whether `ip` is on a network treated as ours: loopback, RFC 1918, CGNAT
/// (also Tailscale), link-local, IPv6 unique-local; an IPv4-mapped address
/// by its IPv4 part.
pub fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || (a == 100 && (64..128).contains(&b))
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| is_private_ip(IpAddr::V4(v4)))
        }
    }
}

/// Whether a proxy handled the request.
pub fn has_proxy_header(headers: &HeaderMap) -> bool {
    PROXY_HEADERS.iter().any(|h| headers.contains_key(*h))
}

/// Classifies a request. `peer` is `None` only for tests that drive the
/// router without a listener (a client cannot remove an extension the
/// listener inserts): it counts as the PC itself; the header half of the
/// rule applies either way.
pub fn classify(peer: Option<SocketAddr>, headers: &HeaderMap) -> Origin {
    let public_peer = peer.is_some_and(|addr| !is_private_ip(addr.ip()));
    if has_proxy_header(headers) || public_peer {
        Origin::Internet
    } else {
        Origin::Local
    }
}

// ---------------------------------------------------------------------------
// The Origin guard
// ---------------------------------------------------------------------------

/// A method with a side effect.
fn is_mutating(method: &Method) -> bool {
    [Method::POST, Method::PUT, Method::PATCH, Method::DELETE].contains(method)
}

/// A WebSocket handshake: a GET, exempt from CORS.
fn is_websocket_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
}

/// `host[:port]` of an `Origin` value (`http(s)://host[:port]`).
fn origin_authority(origin: &str) -> Option<&str> {
    origin
        .strip_prefix("https://")
        .or_else(|| origin.strip_prefix("http://"))
        .filter(|rest| !rest.is_empty())
}

/// The authorities the request was addressed to: `Host`, and
/// `X-Forwarded-Host` behind a proxy.
fn request_authorities(headers: &HeaderMap) -> Vec<String> {
    ["host", "x-forwarded-host"]
        .iter()
        .filter_map(|name| headers.get(*name).and_then(|v| v.to_str().ok()))
        .map(|v| v.trim().to_ascii_lowercase())
        .collect()
}

/// Why a browser's POST (PUT, PATCH, DELETE) or WebSocket upgrade is
/// cross-site: `Sec-Fetch-Site: cross-site`, or an `Origin` that is not the
/// authority the request was sent to. A request without `Origin` is not a
/// browser's cross-site request (curl, the installer's checks, the E2E
/// request API) and passes; `Origin: null` is refused.
pub fn origin_violation(headers: &HeaderMap, method: &Method) -> Option<String> {
    if !is_mutating(method) && !is_websocket_upgrade(headers) {
        return None;
    }
    let site = headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if site.eq_ignore_ascii_case("cross-site") {
        return Some("Sec-Fetch-Site: cross-site".to_string());
    }
    let origin = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok())?;
    let authorities = request_authorities(headers);
    match origin_authority(origin) {
        Some(authority) if authorities.contains(&authority.to_ascii_lowercase()) => None,
        _ => Some(format!("Origin {origin} is not {authorities:?}")),
    }
}

// ---------------------------------------------------------------------------
// The Access JWT
// ---------------------------------------------------------------------------

/// One key of the team's key set (`/cdn-cgi/access/certs`).
#[derive(Debug, Deserialize)]
struct Jwk {
    kid: String,
    #[serde(default)]
    kty: Option<String>,
    #[serde(default)]
    n: Option<String>,
    #[serde(default)]
    e: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Jwks {
    keys: Vec<Jwk>,
}

/// The RSA verifying key of one key-set entry, or `None` for another key
/// type or an unusable entry.
fn usable(jwk: Jwk) -> Option<(String, DecodingKey)> {
    if jwk.kty.as_deref() != Some("RSA") {
        return None;
    }
    let key = DecodingKey::from_rsa_components(jwk.n.as_deref()?, jwk.e.as_deref()?).ok()?;
    Some((jwk.kid, key))
}

/// The usable keys of a key-set document by `kid`.
pub fn parse_jwks(body: &[u8]) -> Result<HashMap<String, DecodingKey>, String> {
    let jwks: Jwks =
        serde_json::from_slice(body).map_err(|e| format!("the key set does not parse: {e}"))?;
    Ok(jwks.keys.into_iter().filter_map(usable).collect())
}

/// Whether a key set fetched `age` ago is still served without a refresh.
pub fn cache_fresh(age: Option<Duration>) -> bool {
    age.is_some_and(|age| age < JWKS_MAX_AGE)
}

/// Whether a key fetch may run now: always when `forced` (the refresher),
/// else only [`JWKS_MIN_REFRESH`] after the last successful one.
pub fn refresh_allowed(age: Option<Duration>, forced: bool) -> bool {
    forced || age.is_none_or(|age| age >= JWKS_MIN_REFRESH)
}

/// The claims of an Access assertion the hub reads.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AccessClaims {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub common_name: Option<String>,
    #[serde(default)]
    pub sub: Option<String>,
}

impl AccessClaims {
    /// Who it is, for the log.
    pub fn identity(&self) -> String {
        self.email
            .clone()
            .or_else(|| self.common_name.clone())
            .or_else(|| self.sub.clone())
            .unwrap_or_else(|| "unknown".to_string())
    }
}

#[derive(Default)]
struct KeyCache {
    keys: HashMap<String, DecodingKey>,
    fetched_at: Option<Instant>,
}

/// Verifies Access assertions against the team's cached key set.
pub struct AccessGate {
    audiences: Vec<String>,
    issuer: String,
    jwks_url: String,
    http: HttpClient,
    cache: RwLock<KeyCache>,
    /// One fetch at a time: a burst of requests with an unknown `kid`
    /// triggers one fetch, the others find its result.
    fetching: Mutex<()>,
}

impl AccessGate {
    /// A gate for the configured Access application.
    pub fn new(cfg: &AccessCfg, http: HttpClient) -> Arc<Self> {
        Arc::new(Self {
            audiences: cfg.aud.iter().map(|a| a.trim().to_string()).collect(),
            issuer: cfg.issuer(),
            jwks_url: cfg.jwks(),
            http,
            cache: RwLock::new(KeyCache::default()),
            fetching: Mutex::new(()),
        })
    }

    /// The age of the cached key set.
    async fn age(&self) -> Option<Duration> {
        self.cache.read().await.fetched_at.map(|at| at.elapsed())
    }

    /// Fetches the key set and replaces the cache on success (`true`); a
    /// failure keeps the last good set. Without `forced` it runs at most
    /// once per [`JWKS_MIN_REFRESH`].
    pub async fn refresh_keys(&self, forced: bool) -> bool {
        let _one_at_a_time = self.fetching.lock().await;
        if !refresh_allowed(self.age().await, forced) {
            return false;
        }
        match self.fetch().await {
            Ok(keys) => {
                let count = keys.len();
                let mut cache = self.cache.write().await;
                cache.keys = keys;
                cache.fetched_at = Some(Instant::now());
                tracing::info!(count, url = %self.jwks_url, "access: cached the Access signing keys");
                true
            }
            Err(why) => {
                tracing::warn!(url = %self.jwks_url, "access: {why}");
                false
            }
        }
    }

    async fn fetch(&self) -> Result<HashMap<String, DecodingKey>, String> {
        let reply = self
            .http
            .get(&self.jwks_url)
            .await
            .map_err(|e| format!("the key set fetch failed: {e:#}"))?;
        if reply.status != 200 {
            return Err(format!(
                "the key set fetch answered {}: {}",
                reply.status,
                reply.snippet()
            ));
        }
        let keys = parse_jwks(&reply.body)?;
        if keys.is_empty() {
            return Err("the key set holds no usable RSA key".to_string());
        }
        Ok(keys)
    }

    /// Warms the key cache now and refreshes it every [`JWKS_MAX_AGE`].
    pub fn spawn_refresher(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let gate = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                gate.refresh_keys(true).await;
                tokio::time::sleep(JWKS_MAX_AGE).await;
            }
        })
    }

    /// The key for `kid`: from a fresh cache, else after one (rate-limited)
    /// refresh; a stale set is still used when the refresh fails.
    async fn key_for(&self, kid: &str) -> Option<DecodingKey> {
        {
            let cache = self.cache.read().await;
            let fresh = cache_fresh(cache.fetched_at.map(|at| at.elapsed()));
            if let Some(key) = cache.keys.get(kid).filter(|_| fresh) {
                return Some(key.clone());
            }
        }
        self.refresh_keys(false).await;
        self.cache.read().await.keys.get(kid).cloned()
    }

    /// Verifies an assertion: its claims, or why it is refused (never the
    /// token itself).
    pub async fn verify(&self, token: &str) -> Result<AccessClaims, String> {
        let header = decode_header(token).map_err(|e| format!("malformed token: {e}"))?;
        if header.alg != Algorithm::RS256 {
            return Err(format!("algorithm {:?}, not RS256", header.alg));
        }
        let kid = header.kid.ok_or("the token has no kid")?;
        let key = self
            .key_for(&kid)
            .await
            .ok_or_else(|| format!("no Access signing key {kid}"))?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_audience(&self.audiences);
        validation.set_issuer(&[&self.issuer]);
        validation.validate_nbf = true;
        // `set_audience` / `set_issuer` check a claim only when it is
        // PRESENT: without this line a token of the same team without `aud`
        // would pass for this application.
        validation.set_required_spec_claims(&["exp", "aud", "iss"]);
        decode::<AccessClaims>(token, &key, &validation)
            .map(|data| data.claims)
            .map_err(|e| format!("refused: {e}"))
    }
}

/// The assertion of a request: the header, else the cookie.
pub fn extract_token(headers: &HeaderMap) -> Option<String> {
    let from_header = headers
        .get(JWT_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    if let Some(token) = from_header {
        return Some(token.to_string());
    }
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|cookies| cookies.split(';'))
        .filter_map(|pair| pair.trim().strip_prefix(JWT_COOKIE)?.strip_prefix('='))
        .find(|value| !value.is_empty())
        .map(str::to_string)
}

/// The decision on one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Served; `identity` is the Access user of an internet request.
    Allow { identity: Option<String> },
    /// Refused (403).
    Deny {
        origin: Origin,
        reason: &'static str,
        detail: String,
    },
}

/// The whole policy, in its order (see the module documentation).
pub async fn decide(
    gate: Option<&AccessGate>,
    peer: Option<SocketAddr>,
    headers: &HeaderMap,
    method: &Method,
) -> Decision {
    let origin = classify(peer, headers);
    let deny = |reason: &'static str, detail: String| Decision::Deny {
        origin,
        reason,
        detail,
    };
    if let Some(detail) = origin_violation(headers, method) {
        return deny("cross_site", detail);
    }
    // No network I/O on this path, ever.
    if origin == Origin::Local {
        return Decision::Allow { identity: None };
    }
    let Some(gate) = gate else {
        return deny(
            "no_access",
            "no [access] in the config: internet requests are refused".to_string(),
        );
    };
    let Some(token) = extract_token(headers) else {
        return deny(
            "no_access_token",
            format!("neither {JWT_HEADER} nor a {JWT_COOKIE} cookie"),
        );
    };
    match gate.verify(&token).await {
        Ok(claims) => Decision::Allow {
            identity: Some(claims.identity()),
        },
        Err(detail) => deny("invalid_access_token", detail),
    }
}

/// The 403 of a refused request.
pub fn forbidden(reason: &str) -> Response {
    crate::auth::error_response(
        StatusCode::FORBIDDEN,
        "ACCESS_DENIED",
        &format!(
            "Refused ({reason}): open the mixer on the church network, or sign in through Cloudflare Access"
        ),
    )
}

/// The middleware in front of every route (the SPA's catch-all included).
pub async fn middleware(State(hub): State<Hub>, request: Request, next: Next) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0);
    let decision = decide(
        hub.access.as_deref(),
        peer,
        request.headers(),
        request.method(),
    )
    .await;
    match decision {
        Decision::Allow { identity } => {
            if let Some(who) = identity {
                tracing::debug!(who = %who, path = %request.uri().path(), "access: internet request allowed");
            }
            next.run(request).await
        }
        Decision::Deny {
            origin,
            reason,
            detail,
        } => {
            // Every refusal says why: a LAN client that somehow sends a
            // forwarded header gets a 403 whose cause is this line.
            tracing::warn!(
                method = %request.method(),
                path = %request.uri().path(),
                peer = ?peer,
                origin = ?origin,
                reason,
                "access: DENY ({detail})"
            );
            forbidden(reason)
        }
    }
}

#[cfg(test)]
#[path = "access/tests.rs"]
mod tests;
