//! The Listen devices' polls (#264; the lane's design in comment
//! 7184226996 on #264): a `FohMixer Listen` device holds no clock (a Max
//! for Live clock runs in Live's audio thread, and a `deferlow` or a
//! `qmetro`'s hand-over posts a window message from there, which clicked
//! on the PA). So the hub drives it, from one task and one socket of its
//! own (bound to `127.0.0.1`, never read: an ICMP reset from a port that
//! closed lands there, not in the announces' socket):
//!
//! - every [`POLL_MS`] each device heard ([`Listen::polls`]) gets
//!   [`poll_datagram`] with its select (on only for the device of the track
//!   listened to); the device turns on or off and answers with its
//!   announce (`udp.rs`), which keeps it heard;
//! - every [`FRAME_POLL_MS`] the device selected ([`Listen::frame_port`])
//!   gets [`frame_datagram`], which bangs its `jit.catch~ @mode 1`: every
//!   whole frame caught since the bang before goes out in one matrix, so a
//!   late or bunched poll changes how many frames a matrix holds, never a
//!   sample;
//! - at its start the hub sends every port of `sweep` (the devices' whole
//!   range, the LUFS listener's, [`crate::config::ListenCfg::sweep`]) one
//!   poll saying off ([`sweep`]): a device running from before the hub's
//!   start answers and is learned. A device's `route` drops another
//!   kind's poll, as the LUFS and meter devices drop this one.
//!
//! The sends never wait (a full socket is a failed poll, counted); the
//! counts are `/api/status` `listen` `polls`, `frame_polls`,
//! `poll_failed`, `swept`, `sweep_failed`, and the socket's address
//! `poll_from`.

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fohmixer_proto::listen::wire::{FRAME_POLL_MS, POLL_MS, frame_datagram, poll_datagram};
use tokio::task::JoinHandle;

use super::Listen;
use crate::lufs_udp::{grid_clock, poll_socket, refused_sends, round_counts};

/// One round at `now`: each device heard gets its poll, on or off (one a
/// port, [`Listen::polls`]).
pub fn poll_round(poller: &std::net::UdpSocket, listen: &Listen, now: Instant) {
    let polls = listen.polls(now);
    let failed: u64 = [true, false]
        .into_iter()
        .map(|on| {
            let ports = polls
                .iter()
                .filter(|(_, o)| *o == on)
                .map(|(port, _)| *port);
            refused_sends(poller, &poll_datagram(on), ports)
        })
        .sum();
    let (sent, failed) = round_counts(polls.len(), failed);
    listen.counters.polled(sent, failed);
}

/// One frame round at `now`: the device selected, if any, gets a frame poll.
pub fn frame_round(poller: &std::net::UdpSocket, listen: &Listen, now: Instant) {
    let Some(port) = listen.frame_port(now) else {
        return;
    };
    let failed = refused_sends(poller, &frame_datagram(), [port]);
    let (sent, failed) = round_counts(1, failed);
    listen.counters.frame_polled(sent, failed);
}

/// The start's sweep: one poll saying off to each of `ports`; its counts,
/// sent and failed, also on one info line.
pub fn sweep(poller: &std::net::UdpSocket, ports: Range<u16>, listen: &Listen) -> (u64, u64) {
    let count = ports.len();
    let failed = refused_sends(poller, &poll_datagram(false), ports);
    let (sent, failed) = round_counts(count, failed);
    listen.counters.swept(sent, failed);
    tracing::info!(
        sent,
        failed,
        "listen: polled every device port once at start (a device running from before answers)"
    );
    (sent, failed)
}

/// A round's clock of `every_ms` (the LUFS listener's, `grid_clock`: its
/// first tick at once, then on its grid, `MissedTickBehavior::Skip`).
pub fn clock(every_ms: u64) -> tokio::time::Interval {
    grid_clock(Duration::from_millis(every_ms))
}

/// The polls' socket (`lufs_udp::poll_socket`: 127.0.0.1, any port,
/// non-blocking), bound before any of Listen's tasks starts.
pub fn socket() -> std::io::Result<std::net::UdpSocket> {
    poll_socket()
}

/// Starts the polls on `poller` (inside a tokio runtime): its address in
/// the counts, the sweep over `sweep_ports` once, then the task.
pub fn spawn(
    poller: std::net::UdpSocket,
    listen: Arc<Listen>,
    sweep_ports: Option<Range<u16>>,
) -> JoinHandle<()> {
    listen
        .counters
        .set_poll_from(poller.local_addr().ok().map(|a| a.to_string()));
    if let Some(ports) = sweep_ports {
        sweep(&poller, ports, &listen);
    }
    tokio::spawn(run(poller, listen))
}

async fn run(poller: std::net::UdpSocket, listen: Arc<Listen>) {
    let mut rounds = clock(POLL_MS);
    let mut frames = clock(FRAME_POLL_MS);
    loop {
        tokio::select! {
            _ = rounds.tick() => poll_round(&poller, &listen, Instant::now()),
            _ = frames.tick() => frame_round(&poller, &listen, Instant::now()),
        }
    }
}

#[cfg(test)]
mod tests;
