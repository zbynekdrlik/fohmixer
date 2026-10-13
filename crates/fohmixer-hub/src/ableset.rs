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
//!   answer's framing headers ([`frame_policy`]): only fohmixer's own page
//!   may frame AbleSet.
//! - The upstream request ([`strip_request`]): no hop-by-hop header, no
//!   forwarding or Cloudflare header, no Access cookie. Nothing is added:
//!   AbleSet sees what a browser on the PC sends, with the public `Host`
//!   (the owner's decision: fohmixer's access rules are the lock).
//! - When AbleSet does not answer, the hub answers itself ([`down_answer`]),
//!   logs the first failure of an outage at warn, and records every failed
//!   request (`ableset_proxy`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use tower::ServiceExt as _;

use crate::access::{AccessGate, JWT_COOKIE, PROXY_HEADERS};
use crate::config::{AbleSetCfg, AccessCfg, Config, TlsCfg};
use crate::events::EventLog;
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
        let ancestors = format!("frame-ancestors {}", https_origins(&tls.name, tls.port));
        Self {
            name: name.to_string(),
            gate,
            proxy,
            frame_ancestors: HeaderValue::from_str(&ancestors)
                .expect("DNS names make a header value"),
            events,
            failing: AtomicBool::new(false),
        }
    }

    /// AbleSet answered: the outage, if any, is over.
    fn reached(&self, method: &Method, path: &str, status: StatusCode, started: Instant) {
        self.failing.store(false, Ordering::Relaxed);
        let ms = started.elapsed().as_millis() as u64;
        tracing::debug!(%method, path, status = status.as_u16(), ms, "ableset: proxied");
    }

    /// AbleSet was not reached: logged (the first failure of an outage at
    /// warn; a body over the cap and a late answer always), recorded, and
    /// answered by the hub.
    fn failed(
        &self,
        method: &Method,
        path: &str,
        dest: Option<&str>,
        failure: Failure,
    ) -> Response {
        let answer = match &failure {
            Failure::TooLarge(_) => crate::auth::error_response(
                StatusCode::PAYLOAD_TOO_LARGE,
                "ABLESET_TOO_LARGE",
                "The request body is too large for AbleSet",
            ),
            Failure::Down(_) => down_answer(dest, false),
            Failure::Timeout(_) => down_answer(dest, true),
        };
        // Only AbleSet being down makes an outage; a body over the cap or a
        // late answer is a warning each time.
        let was_failing = match failure {
            Failure::Down(_) => self.failing.swap(true, Ordering::Relaxed),
            _ => false,
        };
        let why = failure.why();
        if outage_starts(was_failing) {
            tracing::warn!(%method, path, "ableset: {why}");
        } else {
            tracing::debug!(%method, path, "ableset: {why}");
        }
        let status = answer.status().as_u16();
        self.events.record(
            "ableset_proxy",
            proxy_fields(method.as_str(), path, status, why),
        );
        answer
    }
}

/// Whether a failure starts an outage (`was_failing`: the request before
/// failed too): only that one is a warning.
pub fn outage_starts(was_failing: bool) -> bool {
    !was_failing
}

/// The `ableset_proxy` record of a request that did not reach AbleSet.
pub fn proxy_fields(method: &str, path: &str, status: u16, error: &str) -> Value {
    json!({ "method": method, "path": path, "status": status, "error": error })
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
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok());
    if for_ableset(host, &to.name) {
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
    let access = from_fn_with_state(site.gate.clone(), crate::access::middleware);
    let mut app = Router::new().fallback(forward).layer(access);
    if let Some(hub) = redirect {
        app = app.layer(from_fn_with_state(hub, https_redirect));
    }
    app.layer(from_fn_with_state(Arc::clone(&site), framed))
        .with_state(site)
}

/// The proxy's handler: the request to AbleSet, or the hub's own answer.
async fn forward(State(site): State<Arc<AbleSet>>, request: Request) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_string();
    let dest = request
        .headers()
        .get("sec-fetch-dest")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let started = Instant::now();
    match site.proxy.send(request).await {
        Ok(response) => {
            site.reached(&method, &path, response.status(), started);
            response
        }
        Err(failure) => site.failed(&method, &path, dest.as_deref(), failure),
    }
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

/// Every answer for AbleSet's name gets the framing policy.
async fn framed(State(site): State<Arc<AbleSet>>, request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    frame_policy(response.headers_mut(), &site.frame_ancestors, &site.name);
    response
}

/// The answer's framing headers: `X-Frame-Options` removed (it cannot name
/// an origin), `frame-ancestors` removed from every policy AbleSet sent
/// (two policies are both enforced) and ours added, and an absolute
/// `http://` Location of AbleSet's name made `https://` (an http navigation
/// inside the https page would be blocked).
pub fn frame_policy(headers: &mut HeaderMap, frame_ancestors: &HeaderValue, name: &str) {
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
    let location = headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|l| https_location(l, name));
    if let Some(value) = location.and_then(|l| HeaderValue::from_str(&l).ok()) {
        headers.insert(header::LOCATION, value);
    }
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
/// forwarding and Cloudflare header and the Access cookie out; everything
/// else (`Host`, `Origin`, the other cookies, …) kept.
pub fn strip_request(headers: &mut HeaderMap) {
    strip_hop_by_hop(headers);
    let forwarding: Vec<_> = headers
        .keys()
        .filter(|name| {
            let name = name.as_str();
            name.starts_with("cf-") || name == "x-real-ip" || PROXY_HEADERS.contains(&name)
        })
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

/// Removes the hop-by-hop headers and those `Connection` names.
pub fn strip_hop_by_hop(headers: &mut HeaderMap) {
    let named: Vec<String> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(|token| token.trim().to_ascii_lowercase())
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
pub fn without_access_cookie(cookies: impl Iterator<Item = &str>) -> Option<String> {
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
