//! HTTP routes (trimmed from iemmixer's `iem-server/src/routes.rs` @
//! 22372bc): the version, client panic reports, the pages' diagnostic
//! reports (#26, `client_report.rs`) and the embedded UI (public);
//! the engineer login; the layout, the status, a Pro-Q 4 editor's last
//! picture (#71 PR E) and the client WebSocket (a token). Every request must
//! name this hub as its `Host` ([`check_host`]).
//!
//! Roles (#184): every API route has one decision in [`route_role`] (the
//! table [`API_PATHS`] lists them), enforced by [`role_gate`] on every
//! matched API route, so a route added without a decision is refused to
//! everyone; an MBC login gets 403 `ROLE` on an engineer-only route, and
//! its layout and status hold only the Live set `mbc`.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

use axum::{
    Json, Router,
    body::Body,
    extract::{ConnectInfo, DefaultBodyLimit, MatchedPath, Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use fohmixer_proto::VersionInfo;
use fohmixer_proto::client::{HubStatus, LayoutStatus, SongMapReport};
use fohmixer_proto::layout::LayoutResponse;
use fohmixer_proto::markers::migrate::MigrationStatus;
use fohmixer_proto::roles::{self, MBC_INSTANCE, Role};
use rust_embed::RustEmbed;

use crate::auth::{Rejection, bearer, error_response};
use crate::{Assets, Hub};

/// `GET /api/version`: the build this hub runs (deploy verification, the UI's
/// version label check).
async fn get_version() -> Json<VersionInfo> {
    Json(fohmixer_proto::version_info())
}

/// Error report the WASM client sends when it panics.
///
/// Every field except `panic_message` is optional, so a degraded client
/// (a broken Leptos graph, missing window globals) can still report.
#[derive(Debug, serde::Deserialize)]
pub struct ClientErrorReport {
    pub panic_message: String,
    pub version: Option<String>,
    pub git_hash: Option<String>,
    pub url: Option<String>,
    pub user_agent: Option<String>,
    pub location: Option<String>,
    pub backtrace: Option<String>,
}

/// `POST /api/client-error`: log a client-side panic report. Public (a panic
/// may happen before anything else works); the body is capped at 10 KiB by
/// the route's `DefaultBodyLimit`.
async fn client_error(Json(report): Json<ClientErrorReport>) -> StatusCode {
    tracing::warn!(
        target: "fohmixer_hub::client_error",
        version = report.version.as_deref().unwrap_or("?"),
        git_hash = report.git_hash.as_deref().unwrap_or("?"),
        url = report.url.as_deref().unwrap_or("?"),
        user_agent = report.user_agent.as_deref().unwrap_or("?"),
        location = report.location.as_deref().unwrap_or("?"),
        panic = %report.panic_message,
        "client_error",
    );
    if let Some(bt) = report.backtrace.as_deref() {
        tracing::warn!(
            target: "fohmixer_hub::client_error",
            backtrace = %bt,
            "client_error_backtrace",
        );
    }
    StatusCode::NO_CONTENT
}

/// Who may call an API route (#184).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteAccess {
    /// No token: the version, the pages' reports, the login.
    Open,
    /// Any login; the handler serves a role its own view.
    AnyRole,
    /// The engineer's login only: an MBC token gets 403 `ROLE`.
    EngineerOnly,
}

/// Every path [`api_routes`] registers, each with its decision in
/// [`route_role`] (a test holds the two together).
pub const API_PATHS: &[&str] = &[
    "/api/version",
    "/api/client-error",
    "/api/client-report",
    "/api/auth",
    "/api/layout",
    "/api/status",
    "/api/markers/migration",
    "/api/eq/picture",
    "/api/songmap",
    "/ws",
];

/// The role decision of an API route (none: a route with no decision,
/// which [`role_gate`] refuses to everyone).
pub fn route_role(path: &str) -> Option<RouteAccess> {
    match path {
        "/api/version" | "/api/client-error" | "/api/client-report" | "/api/auth" => {
            Some(RouteAccess::Open)
        }
        "/api/layout" | "/api/status" | "/ws" => Some(RouteAccess::AnyRole),
        "/api/markers/migration" | "/api/eq/picture" | "/api/songmap" => {
            Some(RouteAccess::EngineerOnly)
        }
        _ => None,
    }
}

/// Whether a request with a token of `role` (none: no valid token, which
/// the handler answers 401) is refused before its handler.
pub fn refused(access: Option<RouteAccess>, role: Option<Role>) -> bool {
    match access {
        None => true,
        Some(RouteAccess::Open | RouteAccess::AnyRole) => false,
        Some(RouteAccess::EngineerOnly) => role == Some(Role::Mbc),
    }
}

/// The 403 of a refused route.
pub fn role_forbidden() -> Response {
    error_response(StatusCode::FORBIDDEN, "ROLE", "Not allowed for this login")
}

/// The role check of every matched API route ([`route_role`]).
pub async fn role_gate(
    State(hub): State<Hub>,
    path: MatchedPath,
    request: Request,
    next: Next,
) -> Response {
    let access = route_role(path.as_str());
    let role = hub.auth.role(bearer(request.headers()));
    if !refused(access, role) {
        return next.run(request).await;
    }
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(addr)| addr.ip().to_string())
        .unwrap_or_else(|| "-".to_string());
    match access {
        None => tracing::error!(
            path = path.as_str(),
            "an API route without a role decision refused"
        ),
        Some(_) => tracing::info!(
            role = role.map_or("-", Role::as_str),
            path = path.as_str(),
            peer = %peer,
            "role refused"
        ),
    }
    role_forbidden()
}

/// The status an MBC login is served: its Live set only, the layout's
/// revision and that set's unresolved bindings, and nothing of the FOH
/// side (every other field empty). The engineer's is the whole status.
pub fn status_for(role: Role, status: HubStatus) -> HubStatus {
    match role {
        Role::Engineer => status,
        Role::Mbc => HubStatus {
            instances: status
                .instances
                .into_iter()
                .filter(|i| i.name == MBC_INSTANCE)
                .collect(),
            layout: LayoutStatus {
                rev: status.layout.rev,
                error: None,
                unresolved: status
                    .layout
                    .unresolved
                    .into_iter()
                    .filter(|u| u.instance == MBC_INSTANCE)
                    .collect(),
                markers: Default::default(),
            },
            stage_aut: Default::default(),
            clients: 0,
            remote: Default::default(),
            client_reports: Vec::new(),
            companion: None,
            ableset: None,
            presenter: None,
            reverb: Default::default(),
            fader_links: Vec::new(),
            autotune: Default::default(),
            songmap: Vec::new(),
            lufs: Default::default(),
            sectors: Default::default(),
            colors: Default::default(),
            mute_alarms: Default::default(),
        },
    }
}

/// `GET /api/layout` (a token): the served layout as the login's role sees
/// it (`roles::layout_for`) and its revision; 503 while none is served.
async fn get_layout(
    State(hub): State<Hub>,
    headers: HeaderMap,
) -> Result<Json<LayoutResponse>, Rejection> {
    let (_, role) = hub.auth.require(&headers)?;
    match hub.layout.current() {
        (rev, Some(layout)) => Ok(Json(LayoutResponse {
            rev,
            layout: roles::layout_for(role, &layout),
        })),
        (_, None) => Err(Rejection::from(error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "NO_LAYOUT",
            &hub.layout
                .error()
                .unwrap_or_else(|| "no layout yet".to_string()),
        ))),
    }
}

/// `GET /api/status` (a token): the instances, the layout, STAGE AUT; an
/// MBC login's trimmed ([`status_for`]).
async fn get_status(
    State(hub): State<Hub>,
    headers: HeaderMap,
) -> Result<Json<HubStatus>, Rejection> {
    let (_, role) = hub.auth.require(&headers)?;
    Ok(Json(status_for(role, hub.status().await)))
}

/// `GET /api/markers/migration` (a token, #68 PR C): each planned track of
/// the frame (the migration of its strips bound by name to Tuner markers)
/// as the marker keeper finds it in Live. It names tracks: a token only.
async fn get_migration(
    State(hub): State<Hub>,
    headers: HeaderMap,
) -> Result<Json<MigrationStatus>, Rejection> {
    hub.auth.require(&headers)?;
    Ok(Json(hub.migration(false).await))
}

/// `POST /api/markers/migration` (a token, #68 PR C): names the one plain
/// Tuner of each ready planned track its marker; the rows as they were and
/// how many renames went to Live.
async fn post_migration(
    State(hub): State<Hub>,
    headers: HeaderMap,
) -> Result<Json<MigrationStatus>, Rejection> {
    hub.auth.require(&headers)?;
    Ok(Json(hub.migration(true).await))
}

/// The query of `GET /api/songmap`: the Live set.
#[derive(Debug, serde::Deserialize)]
pub struct SongMapQuery {
    pub instance: String,
}

/// The error answer of `GET /api/songmap` when there is no map.
fn songmap_missing(missing: crate::router::SongMapMissing) -> Rejection {
    let (status, code, message) = match missing {
        crate::router::SongMapMissing::Unknown => (
            StatusCode::NOT_FOUND,
            "UNKNOWN_INSTANCE",
            "no Live set of that name",
        ),
        crate::router::SongMapMissing::NotRead => (
            StatusCode::SERVICE_UNAVAILABLE,
            "NO_SONGMAP",
            "the song map was not read yet",
        ),
    };
    Rejection::from(error_response(status, code, message))
}

/// `GET /api/songmap?instance=<name>` (a token, #140): the Live set's newest
/// song map (every span, its key and key changes, every problem in words),
/// 404 for an unknown set, 503 before its first read.
async fn get_songmap(
    State(hub): State<Hub>,
    headers: HeaderMap,
    Query(query): Query<SongMapQuery>,
) -> Result<Json<SongMapReport>, Rejection> {
    hub.auth.require(&headers)?;
    hub.songmap(&query.instance)
        .await
        .map(Json)
        .map_err(songmap_missing)
}

/// The query of `GET /api/eq/picture`: the editor's instance and path.
#[derive(Debug, serde::Deserialize)]
pub struct PictureQuery {
    pub instance: String,
    pub path: String,
}

/// `GET /api/eq/picture?instance=&path=` (a token, #71 PR E): the last
/// picture of a Pro-Q 4 editor (`image/jpeg`, never cached), or 404 before
/// its first open.
async fn get_eq_picture(
    State(hub): State<Hub>,
    headers: HeaderMap,
    Query(query): Query<PictureQuery>,
) -> Result<Response, Rejection> {
    hub.auth.require(&headers)?;
    let key = crate::eq::EditorKey::new(&query.instance, &query.path);
    let Some(jpeg) = hub.pictures.get(&key) else {
        return Err(Rejection::from(error_response(
            StatusCode::NOT_FOUND,
            "NO_PICTURE",
            "no picture of this editor yet",
        )));
    };
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/jpeg")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(jpeg))
        .expect("a picture response is valid"))
}

/// Whether `port` is a port number's digits (one or more, nothing else).
fn is_port(port: &str) -> bool {
    !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit())
}

/// The one reading of a `Host` value (#9): a name or an address, then an
/// optional `:<digits>` port and nothing else; brackets hold an IPv6
/// address only. Returns the name or address (an IPv6 one without its
/// brackets) and whether it was bracketed, or `None` for anything else
/// (`[::1] x`, `127.0.0.1:1 x`, an empty or non-digit port, a name in
/// brackets).
pub(crate) fn host_name(host: &str) -> Option<(&str, bool)> {
    if let Some(rest) = host.strip_prefix('[') {
        let (addr, after) = rest.split_once(']')?;
        let port_ok = after.is_empty() || after.strip_prefix(':').is_some_and(is_port);
        return (port_ok && addr.parse::<Ipv6Addr>().is_ok()).then_some((addr, true));
    }
    match host.rsplit_once(':') {
        Some((name, port)) => is_port(port).then_some((name, false)),
        None => Some((host, false)),
    }
}

/// The port a `Host` value names (#90 B3), read only once [`host_name`]
/// takes the value; none without one.
pub(crate) fn host_port(host: &str) -> Option<u16> {
    host_name(host)?;
    host.rsplit_once(':')?.1.parse().ok()
}

/// The host a request names (#90): its first `Host` (one that is no text as
/// `""`, which no check accepts), else its target's authority. An HTTP/2
/// request carries no `Host` (hyper makes none from `:authority`), and an
/// absolute-form HTTP/1 target may carry the name alone. `dispatch`,
/// `check_host` and the redirects read it here.
pub(crate) fn request_host(request: &Request) -> Option<&str> {
    match request.headers().get(header::HOST) {
        Some(host) => Some(host.to_str().unwrap_or("")),
        None => request.uri().authority().map(|a| a.as_str()),
    }
}

/// Whether a `Host` header value (read by `host_name`) names this hub: an IP
/// address (IPv6 in brackets), `localhost`, or one of `allowed` (the
/// config's `allowed_hosts`). A request without one (not a browser) passes.
/// A Host with anything after its address is refused (#9): it would pass
/// the Origin guard with a matching `Origin` and reach the log lines.
pub fn host_allowed(host: Option<&str>, allowed: &[String]) -> bool {
    let Some(host) = host else {
        return true;
    };
    match host_name(host) {
        None => false,
        // An IPv6 address: `host_name` parsed it.
        Some((_, true)) => true,
        Some((name, false)) => {
            name.parse::<Ipv4Addr>().is_ok()
                || name.eq_ignore_ascii_case("localhost")
                || allowed.iter().any(|a| a.eq_ignore_ascii_case(name))
        }
    }
}

/// Refuses (421) a request whose `Host` is not this hub's: a page on any
/// site a LAN browser visits could otherwise reach the hub from that browser
/// by DNS rebinding — and try PINs from its address (spec D7: the login
/// guard budgets per address).
pub async fn check_host(State(hub): State<Hub>, request: Request, next: Next) -> Response {
    let host = request_host(&request);
    if host_allowed(host, &hub.trusted_hosts) {
        next.run(request).await
    } else {
        tracing::warn!(host = ?host, "a request for a foreign host name refused (DNS rebinding?)");
        error_response(
            StatusCode::MISDIRECTED_REQUEST,
            "UNKNOWN_HOST",
            "This hub does not serve that host name",
        )
    }
}

/// Where a plain-HTTP request is redirected (307, the method kept): to
/// `https://<name>[:https_port]<path>` when the HTTPS listener serves
/// (`https_port`: its bound port; `None` while it has no certificate or no
/// port: the request is then served here), the request names the `[tls]` name, that
/// redirect is on, and no proxy handled it (the tunnel's requests arrive
/// here as plain HTTP from cloudflared: redirecting them would loop). A
/// request by IP address — the emergency path — or by another name is never
/// redirected. A temporary redirect: browsers do not keep it, so taking the
/// name off `[tls]` needs no clearing of caches.
pub fn redirect_target(
    host: Option<&str>,
    proxied: bool,
    tls: Option<&crate::config::TlsCfg>,
    https_port: Option<u16>,
    path_and_query: &str,
) -> Option<String> {
    let tls = tls.filter(|tls| tls.redirect_http)?;
    redirect_to(host, proxied, &tls.name, https_port, path_and_query)
}

/// [`redirect_target`] for `name` (the `[tls]` name, or AbleSet's #90): to
/// `https://<name>[:https_port]<path>` when the request names it, no proxy
/// handled it and the HTTPS listener serves its certificate (`https_port`).
pub fn redirect_to(
    host: Option<&str>,
    proxied: bool,
    name: &str,
    https_port: Option<u16>,
    path_and_query: &str,
) -> Option<String> {
    if proxied {
        return None;
    }
    let https_port = https_port?;
    let (host, _bracketed) = host_name(host?)?;
    if !host.eq_ignore_ascii_case(name) {
        return None;
    }
    let port = if https_port == 443 {
        String::new()
    } else {
        format!(":{https_port}")
    };
    Some(format!("https://{name}{port}{path_and_query}"))
}

/// What a redirect reads of a request: its `Host`, whether a proxy handled
/// it, and its path and query.
pub(crate) fn redirect_inputs(request: &Request) -> (Option<&str>, bool, &str) {
    let host = request_host(request);
    let path = request.uri().path_and_query().map_or("/", |pq| pq.as_str());
    let proxied = crate::access::has_proxy_header(request.headers());
    (host, proxied, path)
}

/// The redirect of the public name to HTTPS (the plain-HTTP listener only).
pub async fn https_redirect(State(hub): State<Hub>, request: Request, next: Next) -> Response {
    let serving = hub
        .https
        .get()
        .filter(|https| https.serving())
        .map(|https| https.addr().port());
    let (host, proxied, path) = redirect_inputs(&request);
    match redirect_target(host, proxied, hub.config.tls.as_ref(), serving, path) {
        Some(location) => axum::response::Redirect::temporary(&location).into_response(),
        None => next.run(request).await,
    }
}

/// API routes.
pub fn api_routes(hub: Hub) -> Router<Hub> {
    Router::new()
        // Version endpoint (deploy verification, the UI's version label)
        .route("/api/version", get(get_version))
        .route(
            "/api/client-error",
            // 10_240 bytes = 10 KiB, written as a literal so cargo-mutants has
            // no operator to mutate; pinned by the router tests below.
            post(client_error).layer(DefaultBodyLimit::max(10_240)),
        )
        .route(
            "/api/client-report",
            // The same 10 KiB, a literal for the same reason; pinned by
            // `client_report`'s tests.
            post(crate::client_report::client_report).layer(DefaultBodyLimit::max(10_240)),
        )
        .route("/api/auth", post(crate::auth::login))
        .route("/api/layout", get(get_layout))
        .route("/api/status", get(get_status))
        .route(
            "/api/markers/migration",
            get(get_migration).post(post_migration),
        )
        .route("/api/eq/picture", get(get_eq_picture))
        .route("/api/songmap", get(get_songmap))
        .route("/ws", get(crate::ws::ws_handler))
        .route_layer(axum::middleware::from_fn_with_state(hub, role_gate))
}

/// Static routes: the embedded UI (Trunk's `dist/`). A path without a file
/// extension is a client route and gets `index.html` (deep links); a path
/// with one is a file, served when it exists and 404 otherwise.
pub fn static_routes() -> Router<Hub> {
    Router::new()
        .route("/", get(serve_index))
        .route("/{*path}", get(serve_spa_route))
}

async fn serve_index(State(hub): State<Hub>) -> Response {
    serve_page(&hub.policy_rev)
}

async fn serve_spa_route(State(hub): State<Hub>, Path(path): Path<String>) -> Response {
    if path.contains('.') {
        serve_embedded_file(&path)
    } else {
        serve_page(&hub.policy_rev)
    }
}

/// The page: `index.html` with the revision of the page policy it is
/// served with (#90, [`with_policy`]).
fn serve_page(policy_rev: &str) -> Response {
    const INDEX: &str = "index.html";
    match <Assets as RustEmbed>::get(INDEX) {
        Some(file) => {
            let page = with_policy(&String::from_utf8_lossy(&file.data), policy_rev);
            file_response(INDEX, page.into_bytes())
        }
        None => not_found(INDEX),
    }
}

/// `page` with `<meta name="fohmixer-policy" content="<policy_rev>">` before
/// its `</head>` (#90, `fohmixer_proto::client::POLICY_META`): in the same
/// answer as the policy itself, so the page knows exactly which policy it
/// loaded with and reloads when a hello names another (a hub restarted with
/// `[ableset]` added). A page without a head (the native builds'
/// placeholder) is served as it is.
pub fn with_policy(page: &str, policy_rev: &str) -> String {
    match page.split_once("</head>") {
        Some((head, rest)) => format!(
            "{head}<meta name=\"{}\" content=\"{policy_rev}\">\n</head>{rest}",
            fohmixer_proto::client::POLICY_META
        ),
        None => page.to_string(),
    }
}

/// Whether a file name carries a content hash (12 or more contiguous hex
/// characters, as Trunk names the app's JS and WASM). Content-hashed files are
/// safe to cache for good; anything else must be revalidated.
fn has_content_hash(path: &str) -> bool {
    let filename = path.rsplit('/').next().unwrap_or(path);
    filename
        .as_bytes()
        .windows(12)
        .any(|w| w.iter().all(u8::is_ascii_hexdigit))
}

/// One embedded file with its MIME type and cache policy, or 404.
fn serve_embedded_file(path: &str) -> Response {
    match <Assets as RustEmbed>::get(path) {
        Some(file) => file_response(path, file.data.into_owned()),
        None => not_found(path),
    }
}

/// The 404 of a static file.
fn not_found(path: &str) -> Response {
    tracing::debug!(path, "static file not found");
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from("Not found"))
        .expect("a static 404 response is valid")
}

/// The answer of a static file at `path` with its MIME type and cache policy.
fn file_response(path: &str, body: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path)
        .first_or_octet_stream()
        .to_string();
    let cache_control = if has_content_hash(path) {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache, must-revalidate"
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, cache_control)
        .body(Body::from(body))
        .expect("an embedded file response is valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// The router as a LAN client reaches it: the peer address the
    /// listener inserts (`ConnectInfo`; without one the Access check takes
    /// the request for the internet). `MockConnectInfo` would not do: only
    /// the `ConnectInfo` extractor reads it, the Access middleware reads the
    /// extension itself.
    fn lan(router: axum::Router) -> axum::Router {
        router.layer(axum::Extension(axum::extract::ConnectInfo(
            std::net::SocketAddr::from(([10, 0, 0, 5], 40000)),
        )))
    }

    async fn send(request: Request<Body>) -> Response {
        let dir = tempfile::tempdir().unwrap();
        let hub = crate::test_hub(dir.path());
        let response = lan(crate::app_router(hub.clone()))
            .oneshot(request)
            .await
            .unwrap();
        hub.stop();
        response
    }

    async fn get_path(path: &str) -> Response {
        send(Request::get(path).body(Body::empty()).unwrap()).await
    }

    async fn body_bytes(response: Response) -> Vec<u8> {
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec()
    }

    fn header_of<'a>(response: &'a Response, name: &str) -> &'a str {
        response
            .headers()
            .get(name)
            .unwrap_or_else(|| panic!("no {name} header"))
            .to_str()
            .unwrap()
    }

    fn client_error_request(body: String) -> Request<Body> {
        Request::post("/api/client-error")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body))
            .unwrap()
    }

    #[tokio::test]
    async fn version_answers_this_build() {
        let response = get_path("/api/version").await;
        assert_eq!(response.status(), StatusCode::OK);
        let info: VersionInfo = serde_json::from_slice(&body_bytes(response).await).unwrap();
        assert_eq!(info.version, fohmixer_proto::VERSION);
        assert_eq!(info, fohmixer_proto::version_info());
    }

    #[tokio::test]
    async fn a_client_error_report_is_accepted() {
        let response = send(client_error_request(r#"{"panic_message":"x"}"#.to_string())).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        let full = serde_json::json!({
            "panic_message": "boom",
            "version": "0.1.0",
            "git_hash": "abc1234",
            "url": "/",
            "user_agent": "test",
            "location": "src/app.rs:1:1",
            "backtrace": "frame 0",
        });
        let response = send(client_error_request(full.to_string())).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn a_client_error_report_over_10_kib_is_refused() {
        // 10_240 bytes is the limit: a report just under it passes, one over
        // it is refused before it is parsed.
        let under = format!(r#"{{"panic_message":"{}"}}"#, "x".repeat(10_200));
        assert!(under.len() <= 10_240);
        assert_eq!(
            send(client_error_request(under)).await.status(),
            StatusCode::NO_CONTENT
        );
        let over = format!(r#"{{"panic_message":"{}"}}"#, "x".repeat(10_240));
        assert!(over.len() > 10_240);
        assert_eq!(
            send(client_error_request(over)).await.status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }

    #[tokio::test]
    async fn the_root_serves_index_html_uncached() {
        let response = get_path("/").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(header_of(&response, "content-type").starts_with("text/html"));
        assert_eq!(
            header_of(&response, "cache-control"),
            "no-cache, must-revalidate"
        );
        let index = <Assets as RustEmbed>::get("index.html").expect("dist/index.html");
        // With the policy's revision of a hub without [ableset] in it (#90;
        // the native builds' placeholder has no head, `with_policy` has its
        // own test).
        let rev = crate::policy_rev(&crate::page_csp(None, None, false));
        let page = with_policy(&String::from_utf8_lossy(&index.data), &rev);
        assert_eq!(body_bytes(response).await, page.into_bytes());
    }

    #[test]
    fn the_page_carries_its_policys_revision_before_the_end_of_its_head() {
        // #90: the page reads it to know the policy it loaded with.
        let page =
            "<!DOCTYPE html>\n<html>\n<head>\n<title>t</title>\n</head>\n<body></body>\n</html>\n";
        assert_eq!(
            with_policy(page, "0123456789abcdef"),
            "<!DOCTYPE html>\n<html>\n<head>\n<title>t</title>\n\
             <meta name=\"fohmixer-policy\" content=\"0123456789abcdef\">\n</head>\n<body></body>\n</html>\n"
        );
        // Only the first head's end: a text in the body naming it stays.
        let twice = "<head></head><body></head></body>";
        assert_eq!(
            with_policy(twice, "r"),
            "<head><meta name=\"fohmixer-policy\" content=\"r\">\n</head><body></head></body>"
        );
        // No head (the native builds' placeholder): as it is.
        assert_eq!(with_policy("placeholder", "r"), "placeholder");
    }

    #[tokio::test]
    async fn a_deep_link_serves_the_same_index_html() {
        let root = body_bytes(get_path("/").await).await;
        for path in ["/deep/route", "/foo/bar", "/foo"] {
            let response = get_path(path).await;
            assert_eq!(response.status(), StatusCode::OK, "{path}");
            assert!(header_of(&response, "content-type").starts_with("text/html"));
            assert_eq!(body_bytes(response).await, root, "{path}");
        }
    }

    #[tokio::test]
    async fn a_missing_file_is_404_not_index_html() {
        for path in ["/missing.js", "/deep/missing.wasm", "/index.htm"] {
            let response = get_path(path).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
            assert_eq!(body_bytes(response).await, b"Not found".to_vec(), "{path}");
        }
    }

    #[tokio::test]
    async fn an_existing_file_is_served_by_its_path() {
        let response = get_path("/index.html").await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(header_of(&response, "content-type").starts_with("text/html"));
    }

    #[tokio::test]
    async fn every_response_carries_the_security_headers() {
        for path in ["/", "/api/version", "/missing.js"] {
            let response = get_path(path).await;
            assert_eq!(header_of(&response, "x-frame-options"), "DENY", "{path}");
            assert_eq!(
                header_of(&response, "x-content-type-options"),
                "nosniff",
                "{path}"
            );
            assert_eq!(
                header_of(&response, "referrer-policy"),
                "strict-origin-when-cross-origin",
                "{path}"
            );
            let csp = header_of(&response, "content-security-policy");
            assert!(csp.starts_with("default-src 'self';"), "{path}: {csp}");
            assert!(csp.contains("'wasm-unsafe-eval'"), "{path}: {csp}");
            assert!(csp.contains("connect-src 'self';"), "{path}: {csp}");
            assert!(csp.contains("img-src 'self' data: blob:;"), "{path}: {csp}");
        }
    }

    #[tokio::test]
    async fn no_cors_header_is_sent() {
        let response = send(
            Request::get("/api/version")
                .header(header::ORIGIN, "http://elsewhere.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .is_none()
        );
    }

    #[tokio::test]
    async fn the_layout_and_the_status_need_a_token() {
        for path in [
            "/api/layout",
            "/api/status",
            "/api/markers/migration",
            "/api/eq/picture?instance=band&path=p",
        ] {
            let response = get_path(path).await;
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{path}");
            let body: serde_json::Value =
                serde_json::from_slice(&body_bytes(response).await).unwrap();
            assert_eq!(body["code"], "UNAUTHORIZED", "{path}");
        }
        let response = send(
            Request::get("/api/status")
                .header(header::AUTHORIZATION, "Bearer not.a.token")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        // The migration's renames too (#68 PR C).
        let response = send(
            Request::post("/api/markers/migration")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn with_a_token_the_migration_answers_its_rows() {
        // No frame: nothing planned, nothing renamed (tests/it/markers.rs runs
        // it against SimLive).
        let dir = tempfile::tempdir().unwrap();
        let hub = crate::test_hub(dir.path());
        let response = get_with_token(&hub, "/api/markers/migration").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_slice(&body_bytes(response).await).unwrap();
        assert_eq!(
            body,
            serde_json::json!({"rows": [], "problems": [], "reading": false, "renamed": 0})
        );
        let token = hub
            .auth
            .issue(fohmixer_proto::roles::Role::Engineer)
            .unwrap();
        let response = lan(crate::app_router(hub.clone()))
            .oneshot(
                Request::post("/api/markers/migration")
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        hub.stop();
    }

    async fn get_with_token(hub: &crate::Hub, path: &str) -> Response {
        let token = hub
            .auth
            .issue(fohmixer_proto::roles::Role::Engineer)
            .unwrap();
        lan(crate::app_router(hub.clone()))
            .oneshot(
                Request::get(path)
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn with_a_token_a_pro_q_picture_is_its_last_jpeg_or_404() {
        let dir = tempfile::tempdir().unwrap();
        let hub = crate::test_hub(dir.path());
        let path = "/api/eq/picture?instance=band&path=live_set%20tracks%201%20devices%200";
        let response = get_with_token(&hub, path).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body: serde_json::Value = serde_json::from_slice(&body_bytes(response).await).unwrap();
        assert_eq!(body["code"], "NO_PICTURE");
        let key = crate::eq::EditorKey::new("band", "live_set tracks 1 devices 0");
        hub.pictures.named(std::collections::BTreeMap::from([(
            key.clone(),
            "live_1".to_string(),
        )]));
        hub.pictures
            .put(&key, "live_1", bytes::Bytes::from_static(b"\xFF\xD8jpeg"));
        let response = get_with_token(&hub, path).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(header_of(&response, "content-type"), "image/jpeg");
        assert_eq!(header_of(&response, "cache-control"), "no-store");
        assert_eq!(body_bytes(response).await, b"\xFF\xD8jpeg");
        // Another editor has none; a query without its path is refused.
        let response = get_with_token(
            &hub,
            "/api/eq/picture?instance=master&path=live_set%20tracks%201%20devices%200",
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let response = get_with_token(&hub, "/api/eq/picture?instance=band").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        hub.stop();
    }

    #[tokio::test]
    async fn songmap_route_answers_404_for_an_unknown_instance_and_503_before_a_read() {
        // #140: a hub of one Live set on a port nothing listens on (it stays
        // offline, so its map is never read).
        let dir = tempfile::tempdir().unwrap();
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = free.local_addr().unwrap().port();
        drop(free);
        let mut config = crate::config::Config::defaults(dir.path());
        config.instances = vec![crate::config::InstanceCfg {
            name: "band".into(),
            port,
        }];
        let hub = crate::Hub::start(config).unwrap();
        let response = get_with_token(&hub, "/api/songmap?instance=band").await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body: serde_json::Value = serde_json::from_slice(&body_bytes(response).await).unwrap();
        assert_eq!(body["code"], "NO_SONGMAP");
        let response = get_with_token(&hub, "/api/songmap?instance=master").await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body: serde_json::Value = serde_json::from_slice(&body_bytes(response).await).unwrap();
        assert_eq!(body["code"], "UNKNOWN_INSTANCE");
        // A query without the set is refused; without a token, nothing.
        let response = get_with_token(&hub, "/api/songmap").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let response = lan(crate::app_router(hub.clone()))
            .oneshot(
                Request::get("/api/songmap?instance=band")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        // The status names no map before a read.
        let response = get_with_token(&hub, "/api/status").await;
        let status: fohmixer_proto::client::HubStatus =
            serde_json::from_slice(&body_bytes(response).await).unwrap();
        assert_eq!(status.songmap, vec![]);
        hub.stop();
    }

    #[tokio::test]
    async fn a_missing_song_map_answers_its_code() {
        let codes = [
            (
                crate::router::SongMapMissing::Unknown,
                StatusCode::NOT_FOUND,
                "UNKNOWN_INSTANCE",
                "no Live set of that name",
            ),
            (
                crate::router::SongMapMissing::NotRead,
                StatusCode::SERVICE_UNAVAILABLE,
                "NO_SONGMAP",
                "the song map was not read yet",
            ),
        ];
        for (missing, status, code, message) in codes {
            let response = songmap_missing(missing).into_response();
            assert_eq!(response.status(), status, "{missing:?}");
            let body: serde_json::Value =
                serde_json::from_slice(&body_bytes(response).await).unwrap();
            assert_eq!(
                (body["code"].as_str(), body["message"].as_str()),
                (Some(code), Some(message))
            );
        }
    }

    async fn get_as(hub: &crate::Hub, role: Role, path: &str) -> Response {
        send_as(hub, role, Request::get(path)).await
    }

    async fn send_as(
        hub: &crate::Hub,
        role: Role,
        request: axum::http::request::Builder,
    ) -> Response {
        let token = hub.auth.issue(role).unwrap();
        lan(crate::app_router(hub.clone()))
            .oneshot(
                request
                    .header(header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[test]
    fn every_api_route_has_a_role_decision() {
        for path in API_PATHS {
            assert!(route_role(path).is_some(), "{path}");
        }
        assert_eq!(route_role("/api/nothing"), None);
        assert_eq!(route_role("/api/version"), Some(RouteAccess::Open));
        assert_eq!(route_role("/api/auth"), Some(RouteAccess::Open));
        assert_eq!(route_role("/api/client-error"), Some(RouteAccess::Open));
        assert_eq!(route_role("/api/client-report"), Some(RouteAccess::Open));
        assert_eq!(route_role("/api/layout"), Some(RouteAccess::AnyRole));
        assert_eq!(route_role("/api/status"), Some(RouteAccess::AnyRole));
        assert_eq!(route_role("/ws"), Some(RouteAccess::AnyRole));
        for path in ["/api/markers/migration", "/api/eq/picture", "/api/songmap"] {
            assert_eq!(route_role(path), Some(RouteAccess::EngineerOnly), "{path}");
        }
    }

    #[tokio::test]
    async fn every_listed_path_is_a_registered_route() {
        // A path in the table that no route serves would be a stale entry;
        // a registered route missing from it is refused to everyone (the
        // gate's `None`), which its own tests then show.
        let dir = tempfile::tempdir().unwrap();
        let hub = crate::test_hub(dir.path());
        for path in API_PATHS {
            let response = get_as(&hub, Role::Engineer, path).await;
            assert_ne!(response.status(), StatusCode::NOT_FOUND, "{path}");
            assert_ne!(response.status(), StatusCode::FORBIDDEN, "{path}");
            // Not the SPA's index either.
            let body = body_bytes(response).await;
            assert!(!body.starts_with(b"<!"), "{path}");
        }
        hub.stop();
    }

    #[test]
    fn only_an_mbc_token_on_an_engineer_route_or_a_route_without_a_decision_is_refused() {
        for role in [None, Some(Role::Engineer), Some(Role::Mbc)] {
            assert!(!refused(Some(RouteAccess::Open), role), "{role:?}");
            assert!(!refused(Some(RouteAccess::AnyRole), role), "{role:?}");
            assert!(refused(None, role), "{role:?}");
        }
        assert!(refused(Some(RouteAccess::EngineerOnly), Some(Role::Mbc)));
        assert!(!refused(
            Some(RouteAccess::EngineerOnly),
            Some(Role::Engineer)
        ));
        // No token: the handler's own 401.
        assert!(!refused(Some(RouteAccess::EngineerOnly), None));
    }

    #[tokio::test]
    async fn an_mbc_token_gets_403_on_every_engineer_route() {
        let dir = tempfile::tempdir().unwrap();
        let hub = crate::test_hub(dir.path());
        let engineer_only: Vec<&str> = API_PATHS
            .iter()
            .copied()
            .filter(|p| route_role(p) == Some(RouteAccess::EngineerOnly))
            .collect();
        assert_eq!(engineer_only.len(), 3);
        for path in engineer_only {
            for request in [Request::get(path), Request::post(path)] {
                let response = send_as(&hub, Role::Mbc, request).await;
                if response.status() == StatusCode::METHOD_NOT_ALLOWED {
                    continue;
                }
                assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
                let body: serde_json::Value =
                    serde_json::from_slice(&body_bytes(response).await).unwrap();
                assert_eq!(body["code"], "ROLE", "{path}");
            }
            let response = get_as(&hub, Role::Engineer, path).await;
            assert_ne!(response.status(), StatusCode::FORBIDDEN, "{path}");
        }
        // Without a token: 401 as before, never 403.
        let response = get_path("/api/songmap?instance=band").await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        hub.stop();
    }

    /// A hub of band, master and mbc (nothing listening) serving
    /// `layout-ok.json` with one more group: an mbc strip.
    async fn three_lives(dir: &std::path::Path) -> crate::Hub {
        let text = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/layout-ok.json"),
        )
        .unwrap();
        let mut layout: serde_json::Value = serde_json::from_str(&text).unwrap();
        layout["pages"][0]["rows"][0]["sections"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "kind": "group", "id": "mbc-1", "title": "MBC", "color": "#1E3A1E",
                "controls": [{"kind": "strip", "strip_kind": "standard",
                    "binding": {"instance": "mbc", "anchor": {"kind": "track", "name": "Hand 1"}}}]
            }));
        std::fs::write(dir.join("layout.json"), layout.to_string()).unwrap();
        let mut config = crate::config::Config::defaults(dir);
        config.instances = ["band", "master", "mbc"]
            .into_iter()
            .map(|name| {
                let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                let port = free.local_addr().unwrap().port();
                crate::config::InstanceCfg {
                    name: name.into(),
                    port,
                }
            })
            .collect();
        let hub = crate::Hub::start(config).unwrap();
        for _ in 0..100 {
            if get_as(&hub, Role::Engineer, "/api/layout").await.status() == StatusCode::OK {
                return hub;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("the layout is served within 5 s: {:?}", hub.layout.error());
    }

    #[tokio::test]
    async fn an_mbc_token_gets_only_mbc_in_the_layout_and_the_status() {
        let dir = tempfile::tempdir().unwrap();
        let hub = three_lives(dir.path()).await;
        let mbc: serde_json::Value =
            serde_json::from_slice(&body_bytes(get_as(&hub, Role::Mbc, "/api/layout").await).await)
                .unwrap();
        let text = mbc["layout"].to_string();
        assert!(!text.contains("\"band\""), "{text}");
        assert!(!text.contains("\"master\""), "{text}");
        assert!(text.contains("\"mbc\""), "{text}");
        let response = get_as(&hub, Role::Mbc, "/api/status").await;
        assert_eq!(response.status(), StatusCode::OK);
        let status: HubStatus = serde_json::from_slice(&body_bytes(response).await).unwrap();
        let names: Vec<&str> = status.instances.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["mbc"]);
        assert!(status.layout.rev > 0);
        hub.stop();
    }

    #[tokio::test]
    async fn an_engineer_token_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let hub = three_lives(dir.path()).await;
        let body: serde_json::Value = serde_json::from_slice(
            &body_bytes(get_as(&hub, Role::Engineer, "/api/layout").await).await,
        )
        .unwrap();
        let served = hub.layout.current().1.unwrap();
        assert_eq!(body["layout"], serde_json::to_value(&*served).unwrap());
        let status: HubStatus = serde_json::from_slice(
            &body_bytes(get_as(&hub, Role::Engineer, "/api/status").await).await,
        )
        .unwrap();
        let names: Vec<&str> = status.instances.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, ["band", "master", "mbc"]);
        hub.stop();
    }

    #[test]
    fn the_mbc_status_holds_its_live_set_only() {
        let mut status: HubStatus = serde_json::from_value(serde_json::json!({
            "instances": [], "layout": {"rev": 7, "error": "e", "unresolved": [
                {"instance": "band", "target": "t", "error": "x"},
                {"instance": "mbc", "target": "u", "error": "y"}]},
            "stage_aut": {"on": true, "writes": 3}, "clients": 4,
        }))
        .unwrap();
        for name in ["band", "mbc", "master"] {
            status.instances.push(
                serde_json::from_value(serde_json::json!({
                    "name": name, "port": 1, "online": true, "busy": false,
                    "set_name": "s", "live_version": "v", "script_version": "w",
                    "main_tick_age_ms": null, "subscriptions": 2, "listeners": 2,
                    "connect_failures": 0, "last_error": null}))
                .unwrap(),
            );
        }
        assert_eq!(status_for(Role::Engineer, status.clone()), status);
        let mbc = status_for(Role::Mbc, status);
        assert_eq!(mbc.instances.len(), 1);
        assert_eq!(mbc.instances[0].name, "mbc");
        assert_eq!(mbc.layout.rev, 7);
        assert_eq!(mbc.layout.error, None);
        assert_eq!(mbc.layout.unresolved.len(), 1);
        assert_eq!(mbc.layout.unresolved[0].target, "u");
        assert!(!mbc.stage_aut.on);
        assert_eq!(mbc.stage_aut.writes, 0);
        assert_eq!(mbc.clients, 0);
    }

    #[tokio::test]
    async fn with_a_token_the_status_answers_and_a_missing_layout_is_503() {
        let dir = tempfile::tempdir().unwrap();
        let hub = crate::test_hub(dir.path());
        let response = get_with_token(&hub, "/api/status").await;
        assert_eq!(response.status(), StatusCode::OK);
        let status: fohmixer_proto::client::HubStatus =
            serde_json::from_slice(&body_bytes(response).await).unwrap();
        assert!(status.instances.is_empty());
        assert_eq!(status.layout.rev, 0);
        assert!(!status.stage_aut.on);
        assert_eq!(status.clients, 0);
        assert!(status.client_reports.is_empty());
        let response = get_with_token(&hub, "/api/layout").await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body: serde_json::Value = serde_json::from_slice(&body_bytes(response).await).unwrap();
        assert_eq!(body["code"], "NO_LAYOUT");
        hub.stop();
    }

    #[test]
    fn a_host_is_an_ip_address_localhost_or_a_configured_name() {
        let allowed = vec!["foh.local".to_string()];
        for host in [
            "192.168.1.20:8480",
            "192.168.1.20",
            "127.0.0.1:8480",
            "localhost:8480",
            "LocalHost",
            "[::1]:8480",
            "[fe80::1]",
            "foh.local:8480",
            "FOH.local",
        ] {
            assert!(host_allowed(Some(host), &allowed), "{host}");
        }
        for host in [
            "evil.example",
            "evil.example:8480",
            "foh.local.evil.example",
            "192.168.1.20.evil.example",
            "[not-an-address]:8480",
            "[::1",
            "",
        ] {
            assert!(!host_allowed(Some(host), &allowed), "{host}");
        }
        assert!(!host_allowed(Some("foh.local"), &[]), "not configured");
        assert!(host_allowed(None, &[]), "no Host: not a browser");
    }

    #[test]
    fn after_the_host_only_a_port_may_follow() {
        let allowed = vec!["foh.local".to_string()];
        for host in ["[::1]", "[::1]:8443", "127.0.0.1:8480", "foh.local:443"] {
            assert!(host_allowed(Some(host), &allowed), "{host}");
        }
        // Anything else after the address would pass the Origin guard with a
        // matching Origin and reach a log line (#9): refused.
        for host in [
            "[::1] x",
            "[::1]junk",
            "[::1]:",
            "[::1]:84a0",
            "[::1]: 8443",
            "127.0.0.1:1 x",
            "127.0.0.1:",
            "localhost:80x",
            "foh.local:443 source=lan",
        ] {
            assert!(!host_allowed(Some(host), &allowed), "{host}");
        }
    }

    #[tokio::test]
    async fn a_request_for_a_foreign_host_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = crate::config::Config::defaults(dir.path());
        config.instances.clear();
        config.allowed_hosts = vec!["foh.local".into()];
        let hub = crate::Hub::start(config).unwrap();
        for (host, code) in [
            ("evil.example:8480", StatusCode::MISDIRECTED_REQUEST),
            ("192.168.1.20:8480", StatusCode::OK),
            ("foh.local:8480", StatusCode::OK),
        ] {
            let response = lan(crate::app_router(hub.clone()))
                .oneshot(
                    Request::get("/api/version")
                        .header(header::HOST, host)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), code, "{host}");
            assert_eq!(header_of(&response, "x-frame-options"), "DENY", "{host}");
            if code == StatusCode::MISDIRECTED_REQUEST {
                let body: serde_json::Value =
                    serde_json::from_slice(&body_bytes(response).await).unwrap();
                assert_eq!(body["code"], "UNKNOWN_HOST");
            }
        }
        hub.stop();
    }

    fn tls(port: u16, redirect_http: bool) -> crate::config::TlsCfg {
        crate::config::TlsCfg {
            name: "foh.example.org".into(),
            port,
            redirect_http,
        }
    }

    #[test]
    fn only_the_public_name_is_redirected_to_https() {
        let on = tls(443, true);
        let t = Some(&on);
        assert_eq!(
            redirect_target(Some("foh.example.org:8480"), false, t, Some(443), "/p?x=1").as_deref(),
            Some("https://foh.example.org/p?x=1")
        );
        assert_eq!(
            redirect_target(Some("FOH.example.org"), false, t, Some(443), "/").as_deref(),
            Some("https://foh.example.org/")
        );
        assert_eq!(
            redirect_target(Some("foh.example.org:8480"), false, t, Some(8443), "/").as_deref(),
            Some("https://foh.example.org:8443/")
        );
        // Never by IP (the emergency path), another name, the tunnel, no
        // Host, redirect off, or no [tls].
        for host in [
            "10.0.0.5:8480",
            "127.0.0.1",
            "localhost:8480",
            "foh.local",
            "example.org",
        ] {
            assert_eq!(
                redirect_target(Some(host), false, t, Some(443), "/"),
                None,
                "{host}"
            );
        }
        assert_eq!(
            redirect_target(Some("foh.example.org"), true, t, Some(443), "/"),
            None
        );
        assert_eq!(redirect_target(None, false, t, Some(443), "/"), None);
        // Not while the HTTPS listener does not serve (no certificate yet).
        assert_eq!(
            redirect_target(Some("foh.example.org"), false, t, None, "/"),
            None
        );
        let off = tls(443, false);
        assert_eq!(
            redirect_target(Some("foh.example.org"), false, Some(&off), Some(443), "/"),
            None
        );
        assert_eq!(
            redirect_target(Some("foh.example.org"), false, None, Some(443), "/"),
            None
        );
    }

    #[test]
    fn the_redirect_reads_the_host_as_the_host_check_does() {
        // One Host parser (#9): a Host the check refuses is not a name to
        // redirect either.
        let on = tls(443, true);
        let allowed = vec!["foh.example.org".to_string()];
        for host in [
            "foh.example.org:8480 x",
            "foh.example.org:",
            "foh.example.org:84a0",
        ] {
            assert!(!host_allowed(Some(host), &allowed), "{host}");
            assert_eq!(
                redirect_target(Some(host), false, Some(&on), Some(443), "/"),
                None,
                "{host}"
            );
        }
    }

    #[test]
    fn a_bracketed_name_is_neither_allowed_nor_redirected() {
        // Brackets hold an IPv6 address only: the parser itself refuses a
        // name in them, so the redirect does not depend on the check
        // running first.
        let on = tls(443, true);
        let allowed = vec!["foh.example.org".to_string()];
        for host in ["[foh.example.org]", "[foh.example.org]:8480"] {
            assert!(!host_allowed(Some(host), &allowed), "{host}");
            assert_eq!(
                redirect_target(Some(host), false, Some(&on), Some(443), "/"),
                None,
                "{host}"
            );
        }
    }

    #[tokio::test]
    async fn the_plain_http_listener_redirects_the_public_name_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = crate::config::Config::defaults(dir.path());
        config.instances.clear();
        config.tls = Some(tls(443, true));
        let hub = crate::Hub::start(config).unwrap();
        let https = std::sync::Arc::new(
            crate::https::Https::bind("127.0.0.1:0".parse().unwrap(), axum::Router::new()).unwrap(),
        );
        let _ = hub.https.set(std::sync::Arc::clone(&https));
        let get = |host: &str, extra: Option<(&str, &str)>| {
            let mut request = Request::get("/api/version?a=b").header(header::HOST, host);
            if let Some((name, value)) = extra {
                request = request.header(name, value);
            }
            request.body(Body::empty()).unwrap()
        };
        // No certificate yet: the HTTPS listener does not serve, so the name
        // is served here.
        let not_yet = lan(crate::app_router(hub.clone()))
            .oneshot(get("foh.example.org:8480", None))
            .await
            .unwrap();
        assert_eq!(not_yet.status(), StatusCode::OK);
        let ca = crate::tls::test_certs::TestCa::new();
        let now = crate::auth::now_secs() as i64;
        let (chain, key) = ca.leaf(&["foh.example.org"], now - 3600, now + 86_400);
        let pem = crate::tls::checked(chain, key, "foh.example.org").unwrap();
        https
            .set_cert(crate::tls::Slot::Primary, "foh.example.org", &pem)
            .unwrap();
        let redirected = lan(crate::app_router(hub.clone()))
            .oneshot(get("foh.example.org:8480", None))
            .await
            .unwrap();
        assert_eq!(redirected.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            header_of(&redirected, "location"),
            format!(
                "https://foh.example.org:{}/api/version?a=b",
                https.addr().port()
            )
        );
        assert_eq!(header_of(&redirected, "x-frame-options"), "DENY");
        // The name is a trusted host; by IP it is served.
        let by_ip = lan(crate::app_router(hub.clone()))
            .oneshot(get("10.0.0.5:8480", None))
            .await
            .unwrap();
        assert_eq!(by_ip.status(), StatusCode::OK);
        // The HTTPS listener's router never redirects.
        let https = lan(crate::https_router(hub.clone()))
            .oneshot(get("foh.example.org", None))
            .await
            .unwrap();
        assert_eq!(https.status(), StatusCode::OK);
        // A tunnel request is not redirected (it goes on to the Access
        // check: no [access], so it is refused).
        let tunnel = lan(crate::app_router(hub.clone()))
            .oneshot(get(
                "foh.example.org",
                Some(("cf-connecting-ip", "203.0.113.7")),
            ))
            .await
            .unwrap();
        assert_eq!(tunnel.status(), StatusCode::FORBIDDEN);
        let body = tunnel.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            body["message"]
                .as_str()
                .unwrap()
                .starts_with("Refused (no_access)"),
            "{body}"
        );
        hub.stop();
    }

    #[test]
    fn content_hash_detection_hashed_files() {
        assert!(has_content_hash("fohmixer-ui-2b1f3c4d5e6f7a8b.js"));
        assert!(has_content_hash("fohmixer-ui-2b1f3c4d5e6f7a8b_bg.wasm"));
        assert!(has_content_hash("0123456789ab"));
    }

    #[test]
    fn content_hash_detection_unhashed_files() {
        assert!(!has_content_hash("index.html"));
        assert!(!has_content_hash("manifest.json"));
        assert!(!has_content_hash("icon-192.png"));
        assert!(!has_content_hash("0123456789a"));
        assert!(!has_content_hash("0123456789a-bcdef"));
    }

    #[test]
    fn content_hash_detection_looks_at_the_file_name_only() {
        assert!(has_content_hash("icons/0123456789abcdef.png"));
        assert!(!has_content_hash("0123456789abcdef/icon.png"));
    }
}
