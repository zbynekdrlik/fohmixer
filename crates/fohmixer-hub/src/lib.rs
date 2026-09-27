//! fohmixer-hub: the HTTP server on the Ableton PC.
//!
//! S0 serves the embedded UI (`crates/fohmixer-ui/dist`, built by Trunk),
//! `GET /api/version` and `POST /api/client-error`, behind the security
//! headers, and stops gracefully. The Live connections, the client protocol
//! and auth arrive in S3 (spec §2.4). Trimmed from iemmixer's `iem-server`
//! @ 22372bc (no TLS, auth, WebSocket or tunnel code yet).

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context as _;
use axum::Router;
use axum::http::{HeaderName, HeaderValue};
use tokio::sync::{Notify, oneshot};
use tower_http::set_header::SetResponseHeaderLayer;

pub mod routes;

/// The HTTP port when `PORT` is not set.
pub const DEFAULT_PORT: u16 = 8480;

/// How long a stop waits for open requests before the server returns anyway.
pub const STOP_DRAIN: Duration = Duration::from_secs(5);

/// The UI bundle (Trunk's `dist/`), embedded at build time. CI builds the
/// real bundle for the release binary; native lint and test jobs create a
/// placeholder `dist/index.html` first, because the folder must exist.
#[derive(rust_embed::Embed)]
#[folder = "../fohmixer-ui/dist/"]
pub struct Assets;

/// The HTTP application: API and static routes behind the security headers.
/// No CORS layer: the UI is always loaded from this server, so its requests
/// are same-origin; a foreign page gets no `Access-Control-Allow-Origin` and
/// cannot read API responses.
pub fn app_router() -> Router {
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
    // CSP allows WASM + inline scripts (Trunk's loader), inline styles
    // (Leptos) and WebSocket connections (S3).
    let csp = SetResponseHeaderLayer::overriding(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; connect-src 'self' ws: wss:; img-src 'self' data:; font-src 'self'",
        ),
    );

    Router::new()
        .merge(routes::api_routes())
        .merge(routes::static_routes())
        .layer(x_frame_options)
        .layer(x_content_type_options)
        .layer(referrer_policy)
        .layer(csp)
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

/// The HTTP port from the `PORT` environment value: [`DEFAULT_PORT`] when it
/// is not set, an error when it is not a port number.
pub fn port_from(value: Option<&str>) -> anyhow::Result<u16> {
    match value {
        None => Ok(DEFAULT_PORT),
        Some(text) => text
            .parse()
            .with_context(|| format!("PORT={text} is not a port number")),
    }
}

/// Serve [`app_router`] on `addr` until `stop` resolves (graceful stop, as in
/// iemmixer): `ready` gets the bound address once the listener is up; at the
/// stop the listener closes at once (the port is free), idle connections
/// close, open requests get up to [`STOP_DRAIN`] to finish, and it returns
/// `Ok`.
pub async fn serve_until<F>(
    addr: SocketAddr,
    ready: oneshot::Sender<SocketAddr>,
    stop: F,
) -> anyhow::Result<()>
where
    F: Future<Output = ()> + Send + 'static,
{
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding {addr}"))?;
    let local = listener.local_addr().context("reading the bound address")?;
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

    // The stop closes the listener; open requests get STOP_DRAIN.
    let stopping = Arc::new(Notify::new());
    let stop_seen = Arc::clone(&stopping);
    let serve = axum::serve(listener, app_router().into_make_service()).with_graceful_shutdown(
        async move {
            stop.await;
            tracing::info!("stop requested: the listener closes, open requests get up to 5 s");
            stop_seen.notify_one();
        },
    );
    tokio::select! {
        result = serve => result.context("serving HTTP")?,
        () = async {
            stopping.notified().await;
            tokio::time::sleep(STOP_DRAIN).await;
        } => tracing::warn!("HTTP requests still open 5 s after the stop: stopping without them"),
    }
    tracing::info!("HTTP server stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_port_defaults_to_8480() {
        assert_eq!(port_from(None).unwrap(), 8480);
    }

    #[test]
    fn the_port_comes_from_the_environment_value() {
        assert_eq!(port_from(Some("9000")).unwrap(), 9000);
        assert_eq!(port_from(Some("0")).unwrap(), 0);
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
            let error = port_from(Some(bad)).unwrap_err();
            assert_eq!(
                error.to_string(),
                format!("PORT={bad} is not a port number")
            );
        }
    }
}
