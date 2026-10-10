//! HTTP routes (trimmed from iemmixer's `iem-server/src/routes.rs` @
//! 22372bc): the version, client panic reports, the pages' diagnostic
//! reports (#26, `client_report.rs`) and the embedded UI (public);
//! the engineer login; the layout, the status, a Pro-Q 4 editor's last
//! picture (#71 PR E) and the client WebSocket (a token). Every request must
//! name this hub as its `Host` ([`check_host`]).

use std::net::{Ipv4Addr, Ipv6Addr};

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use fohmixer_proto::VersionInfo;
use fohmixer_proto::client::HubStatus;
use fohmixer_proto::layout::LayoutResponse;
use fohmixer_proto::markers::migrate::MigrationStatus;
use rust_embed::RustEmbed;

use crate::auth::{Rejection, error_response};
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

/// `GET /api/layout` (a token): the served layout and its revision; 503 while
/// none is served.
async fn get_layout(
    State(hub): State<Hub>,
    headers: HeaderMap,
) -> Result<Json<LayoutResponse>, Rejection> {
    hub.auth.require(&headers)?;
    match hub.layout.current() {
        (rev, Some(layout)) => Ok(Json(LayoutResponse {
            rev,
            layout: (*layout).clone(),
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

/// `GET /api/status` (a token): the instances, the layout, STAGE AUT.
async fn get_status(
    State(hub): State<Hub>,
    headers: HeaderMap,
) -> Result<Json<HubStatus>, Rejection> {
    hub.auth.require(&headers)?;
    Ok(Json(hub.status().await))
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
fn host_name(host: &str) -> Option<(&str, bool)> {
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
    let host = request
        .headers()
        .get(header::HOST)
        .map(|h| h.to_str().unwrap_or(""));
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
    let tls = tls.filter(|tls| tls.redirect_http && !proxied)?;
    let https_port = https_port?;
    let (name, _bracketed) = host_name(host?)?;
    if !name.eq_ignore_ascii_case(&tls.name) {
        return None;
    }
    let port = if https_port == 443 {
        String::new()
    } else {
        format!(":{https_port}")
    };
    Some(format!("https://{}{port}{path_and_query}", tls.name))
}

/// The redirect of the public name to HTTPS (the plain-HTTP listener only).
pub async fn https_redirect(State(hub): State<Hub>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok());
    let path = request.uri().path_and_query().map_or("/", |pq| pq.as_str());
    let proxied = crate::access::has_proxy_header(request.headers());
    let serving = hub
        .https
        .get()
        .filter(|https| https.serving())
        .map(|https| https.addr().port());
    match redirect_target(host, proxied, hub.config.tls.as_ref(), serving, path) {
        Some(location) => axum::response::Redirect::temporary(&location).into_response(),
        None => next.run(request).await,
    }
}

/// API routes.
pub fn api_routes() -> Router<Hub> {
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
        .route("/ws", get(crate::ws::ws_handler))
}

/// Static routes: the embedded UI (Trunk's `dist/`). A path without a file
/// extension is a client route and gets `index.html` (deep links); a path
/// with one is a file, served when it exists and 404 otherwise.
pub fn static_routes() -> Router<Hub> {
    Router::new()
        .route("/", get(serve_index))
        .route("/{*path}", get(serve_spa_route))
}

async fn serve_index() -> Response {
    serve_embedded_file("index.html")
}

async fn serve_spa_route(Path(path): Path<String>) -> Response {
    if path.contains('.') {
        serve_embedded_file(&path)
    } else {
        serve_embedded_file("index.html")
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
    let Some(file) = <Assets as RustEmbed>::get(path) else {
        tracing::debug!(path, "static file not found");
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("Not found"))
            .expect("a static 404 response is valid");
    };
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
        .body(Body::from(file.data.into_owned()))
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
        assert_eq!(body_bytes(response).await, index.data.to_vec());
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
        // No frame: nothing planned, nothing renamed (tests/markers.rs runs
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
        let token = hub.auth.issue().unwrap();
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
        let token = hub.auth.issue().unwrap();
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
        https
            .serve(crate::tls::server_config(&chain, &key).unwrap())
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
