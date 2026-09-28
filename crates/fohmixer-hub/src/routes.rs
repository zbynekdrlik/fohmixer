//! HTTP routes (trimmed from iemmixer's `iem-server/src/routes.rs` @
//! 22372bc): the version, client panic reports and the embedded UI (public);
//! the engineer login; the layout, the status and the client WebSocket (a
//! token).

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode, header},
    response::Response,
    routing::{get, post},
};
use fohmixer_proto::VersionInfo;
use fohmixer_proto::client::HubStatus;
use fohmixer_proto::layout::LayoutResponse;
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
        .route("/api/auth", post(crate::auth::login))
        .route("/api/layout", get(get_layout))
        .route("/api/status", get(get_status))
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

    async fn send(request: Request<Body>) -> Response {
        let dir = tempfile::tempdir().unwrap();
        let hub = crate::test_hub(dir.path());
        let response = crate::app_router(hub.clone())
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
        for path in ["/api/layout", "/api/status"] {
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
    }

    async fn get_with_token(hub: &crate::Hub, path: &str) -> Response {
        let token = hub.auth.issue().unwrap();
        crate::app_router(hub.clone())
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
        let response = get_with_token(&hub, "/api/layout").await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body: serde_json::Value = serde_json::from_slice(&body_bytes(response).await).unwrap();
        assert_eq!(body["code"], "NO_LAYOUT");
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
