//! `fohmixer-hub`: the hub server (CI E2E and, from S3, the Ableton PC).
//!
//!   fohmixer-hub      serve the UI and the API on 0.0.0.0:$PORT (default 8480)
//!
//! Logging: `RUST_LOG` when set (e.g. `fohmixer_hub=debug`), else
//! `fohmixer_hub=info`; an invalid `RUST_LOG` stops the start. SIGTERM or SIGINT
//! (Windows: Ctrl-Break or Ctrl-C) stop it gracefully: open requests get up
//! to 5 s, then it exits 0. Nothing is force-killed (spec I7).

use std::net::SocketAddr;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context as _;
use tokio::sync::oneshot;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // stderr, not the logger: it may not be up yet (a bad RUST_LOG).
            eprintln!("fohmixer-hub: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<()> {
    let rust_log = std::env::var("RUST_LOG").ok();
    tracing_subscriber::fmt()
        .with_env_filter(fohmixer_hub::log_filter(rust_log.as_deref())?)
        .init();
    tracing::info!("Starting fohmixer-hub v{}", fohmixer_proto::full_version());
    let port_env = std::env::var("PORT").ok();
    let port = fohmixer_hub::port_from(port_env.as_deref())?;
    tracing::info!(port, from_env = port_env.is_some(), "HTTP port resolved");
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let runtime = tokio::runtime::Runtime::new().context("creating the tokio runtime")?;
    let served = runtime.block_on(async {
        // Installed before the bind, so a stop that arrives while the server
        // starts is not lost.
        let stop = shutdown_signal().context("registering the stop signals")?;
        let (ready, _) = oneshot::channel();
        fohmixer_hub::serve_until(addr, ready, stop).await
    });
    runtime.shutdown_timeout(Duration::from_secs(1));
    served?;
    tracing::info!("fohmixer-hub stopped");
    Ok(())
}

/// The graceful stop request: SIGTERM or SIGINT. The handlers are installed
/// at the call, so a stop that arrives before the server listens is not lost;
/// the future resolves on the first one.
#[cfg(unix)]
fn shutdown_signal() -> std::io::Result<impl std::future::Future<Output = ()> + Send + 'static> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate())?;
    let mut int = signal(SignalKind::interrupt())?;
    Ok(async move {
        tokio::select! {
            _ = term.recv() => tracing::info!("SIGTERM: stopping"),
            _ = int.recv() => tracing::info!("SIGINT: stopping"),
        }
    })
}

/// The graceful stop request: Ctrl-Break (a stop request delivered to the
/// hub's own console) or Ctrl-C, installed at the call.
#[cfg(windows)]
fn shutdown_signal() -> std::io::Result<impl std::future::Future<Output = ()> + Send + 'static> {
    use tokio::signal::windows::{ctrl_break, ctrl_c};
    let mut brk = ctrl_break()?;
    let mut c = ctrl_c()?;
    Ok(async move {
        tokio::select! {
            _ = brk.recv() => tracing::info!("Ctrl-Break: stopping"),
            _ = c.recv() => tracing::info!("Ctrl-C: stopping"),
        }
    })
}
