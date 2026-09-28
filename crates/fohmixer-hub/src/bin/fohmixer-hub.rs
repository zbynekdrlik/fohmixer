//! `fohmixer-hub`: the hub (the Ableton PC, CI E2E).
//!
//!   fohmixer-hub                        serve on 0.0.0.0:$PORT (default: the config's, 8480)
//!   fohmixer-hub pin set-engineer       read a PIN from stdin, store its hash as the engineer PIN
//!   fohmixer-hub cloudflare set-token   read a Cloudflare API token from stdin, store it sealed
//!                                       (the ACME client's DNS-01 records, #17)
//!
//! Both commands run as the hub's user: what they store is sealed (DPAPI)
//! for that account.
//! The data folder is `$FOHMIXER_DATA` (default: the working folder): the
//! config `fohmixer-hub.toml`, `secrets/`, the layout, its backups and
//! `hub-state.json`. Logging: `RUST_LOG` when set (e.g.
//! `fohmixer_hub=debug`), else `fohmixer_hub=info`; an invalid `RUST_LOG`
//! stops the start. SIGTERM or SIGINT (Windows: Ctrl-Break or Ctrl-C) stop
//! it gracefully: open requests get up to 5 s, then it exits 0. Nothing is
//! force-killed (spec I7).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::Context as _;
use fohmixer_hub::config::Config;
use fohmixer_hub::provision::{self, ProvisionError};
use tokio::sync::oneshot;

const USAGE: &str = "usage: fohmixer-hub [pin set-engineer | cloudflare set-token]   (the PIN or the token is read from stdin)";

fn data_dir() -> PathBuf {
    PathBuf::from(std::env::var("FOHMIXER_DATA").unwrap_or_else(|_| ".".to_string()))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] => match run() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                // stderr, not the logger: it may not be up yet (a bad RUST_LOG).
                eprintln!("fohmixer-hub: {e:#}");
                ExitCode::FAILURE
            }
        },
        ["pin", "set-engineer"] => pin_command(),
        ["cloudflare", "set-token"] => token_command(),
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

/// `pin set-engineer`: exit 0 when stored, 2 for a bad PIN, 1 on an error.
fn pin_command() -> ExitCode {
    let stdin = std::io::stdin();
    match provision::run(&data_dir(), stdin.lock()) {
        Ok(()) => {
            eprintln!("fohmixer-hub: stored the engineer PIN hash");
            ExitCode::SUCCESS
        }
        Err(e @ ProvisionError::Invalid(_)) => {
            eprintln!("fohmixer-hub: {e}");
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("fohmixer-hub: {e}");
            ExitCode::FAILURE
        }
    }
}

/// `cloudflare set-token`: exit 0 when stored, 2 for a bad token, 1 on an
/// error. The token is never printed.
fn token_command() -> ExitCode {
    let stdin = std::io::stdin();
    match fohmixer_hub::cf_token::run(&data_dir(), stdin.lock()) {
        Ok(()) => {
            eprintln!("fohmixer-hub: stored the Cloudflare API token (sealed)");
            ExitCode::SUCCESS
        }
        Err(e @ ProvisionError::Invalid(_)) => {
            eprintln!("fohmixer-hub: {e}");
            ExitCode::from(2)
        }
        Err(e) => {
            eprintln!("fohmixer-hub: {e}");
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
    let data = data_dir();
    let config = Config::load(&data)?;
    let port_env = std::env::var("PORT").ok();
    let port = fohmixer_hub::port_from(port_env.as_deref(), config.http_port)?;
    tracing::info!(port, from_env = port_env.is_some(), data = %data.display(), "HTTP port resolved");
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let runtime = tokio::runtime::Runtime::new().context("creating the tokio runtime")?;
    let served = runtime.block_on(async {
        // Installed before the bind, so a stop that arrives while the server
        // starts is not lost.
        let stop = shutdown_signal().context("registering the stop signals")?;
        let (ready, _) = oneshot::channel();
        fohmixer_hub::serve_until(addr, config, ready, stop).await
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
