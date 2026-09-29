//! The pages' diagnostic reports (#26): `POST /api/client-report`.
//!
//! Every page (a browser tab or a Home Screen app, on the LAN or through
//! the tunnel) reports its load, its hub socket's transitions, its
//! visibility, its service worker and wake lock, its errors, and (`perf`,
//! #5 K4) its frame rate, its longest frame and the most pointers down at
//! once (`fohmixer-ui` `diag.rs`, `diag/perf.rs`). The hub logs each report at INFO as
//! `client report`, with the peer and the source (`lan` / `internet`, the
//! Access check's classification), and keeps the last [`RING`] for
//! `GET /api/status` (`client_reports`): what a tablet does is readable from
//! the hub log and the status, without asking anyone to look at it.
//!
//! Public like `/api/client-error`: it must work before a login, and on the
//! internet path the Access check guards every route anyway. So nothing a
//! page sends is trusted: only the fields of [`ReportFields`] are kept
//! (serde drops any other), each is stripped of control characters (no
//! forged log lines) and cut ([`WORD_MAX_CHARS`], [`TEXT_MAX_CHARS`] for the
//! user agent and the error), and a peer gets at most [`RATE_MAX`] reports
//! per [`RATE_WINDOW`] (a looping page cannot fill the log).

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use fohmixer_proto::client::{ClientReport, ReportFields};

use crate::Hub;
use crate::access::{self, Origin};

/// The longest value kept of the free-text fields (`ua`, `error`), in
/// characters (an iPad's user agent is about 150).
pub const TEXT_MAX_CHARS: usize = 300;
/// The longest value kept of every other field (a word, a version, a host,
/// a size, an address): a long one is not what the page sends.
pub const WORD_MAX_CHARS: usize = 64;
/// How many reports `/api/status` keeps (the newest).
pub const RING: usize = 50;
/// A peer's report budget: at most [`RATE_MAX`] per this window.
pub const RATE_WINDOW: Duration = Duration::from_secs(10);
/// Reports a peer may send per [`RATE_WINDOW`]. A page sends at most one
/// per kind per 5 s (nine kinds: 18 per window), a quiet page a few a
/// minute; the rest is room for a few pages behind one address (a tab and a
/// Home Screen app on one tablet, every internet page behind cloudflared, the
/// E2E suite's pages) — a flood gets 6 a second into the log.
pub const RATE_MAX: u32 = 60;
/// Peers whose budget is tracked at once (a memory bound under a flood).
pub const MAX_PEERS: usize = 256;

/// `value` fit for one log line: control characters (line breaks, escape
/// sequences) dropped, then cut to `max` characters, a cut marked with an
/// ellipsis.
pub fn clean(value: &str, max: usize) -> String {
    let kept: String = value.chars().filter(|c| !c.is_control()).collect();
    if kept.chars().count() <= max {
        return kept;
    }
    let mut cut: String = kept.chars().take(max).collect();
    cut.push('…');
    cut
}

/// Every field of a report, [`clean`]ed: the user agent and the error to
/// [`TEXT_MAX_CHARS`], the others (the `perf` numbers too: the page sends
/// them as short words) to [`WORD_MAX_CHARS`].
pub fn clean_fields(fields: ReportFields) -> ReportFields {
    let word = |value: Option<String>| value.map(|v| clean(&v, WORD_MAX_CHARS));
    let text = |value: Option<String>| value.map(|v| clean(&v, TEXT_MAX_CHARS));
    ReportFields {
        kind: word(fields.kind),
        display: word(fields.display),
        ua: text(fields.ua),
        build: word(fields.build),
        host: word(fields.host),
        screen: word(fields.screen),
        sw: word(fields.sw),
        wake_lock: word(fields.wake_lock),
        visibility: word(fields.visibility),
        reconnects: word(fields.reconnects),
        error: text(fields.error),
        fps: word(fields.fps),
        long_frame_ms: word(fields.long_frame_ms),
        touches_max: word(fields.touches_max),
        pointer: word(fields.pointer),
    }
}

/// The source a report names: the Access check's class of the request.
pub fn source(origin: Origin) -> &'static str {
    match origin {
        Origin::Local => "lan",
        Origin::Internet => "internet",
    }
}

/// The client address Cloudflare names (`cf-connecting-ip`) for an
/// internet request, whose peer is cloudflared on the PC; none for a LAN
/// one, whose peer is the client. Only an internet request that passed the
/// Access check gets this far.
pub fn forwarded_client(origin: Origin, headers: &HeaderMap) -> Option<String> {
    if origin != Origin::Internet {
        return None;
    }
    let value = headers.get("cf-connecting-ip")?.to_str().ok()?;
    Some(clean(value, WORD_MAX_CHARS))
}

/// What becomes of a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admit {
    /// Logged and kept.
    Keep,
    /// Over its peer's budget, the first in this window (logged once).
    DropFirst,
    /// Over its peer's budget again: dropped silently.
    Drop,
}

/// One peer's current window.
#[derive(Debug, Clone, Copy)]
struct Window {
    start: Instant,
    count: u32,
}

/// Whether `now` is past a window that started at `start`.
fn expired(start: Instant, now: Instant) -> bool {
    now.saturating_duration_since(start) >= RATE_WINDOW
}

/// The per-peer report budgets (clock injected).
#[derive(Debug, Default)]
pub struct Budget {
    windows: HashMap<IpAddr, Window>,
}

impl Budget {
    /// A report of peer `key` at `now`. A new peer first clears the
    /// windows that ended (and the oldest, when [`MAX_PEERS`] are live).
    pub fn admit(&mut self, key: IpAddr, now: Instant) -> Admit {
        if !self.windows.contains_key(&key) {
            self.make_room(now);
        }
        let window = self.windows.entry(key).or_insert(Window {
            start: now,
            count: 0,
        });
        if expired(window.start, now) {
            window.start = now;
            window.count = 0;
        }
        window.count = window.count.saturating_add(1);
        if window.count <= RATE_MAX {
            Admit::Keep
        } else if window.count == RATE_MAX + 1 {
            Admit::DropFirst
        } else {
            Admit::Drop
        }
    }

    /// Room for one more peer: the windows that ended go, and the oldest
    /// one when [`MAX_PEERS`] are still live.
    fn make_room(&mut self, now: Instant) {
        self.windows.retain(|_, w| !expired(w.start, now));
        if self.windows.len() < MAX_PEERS {
            return;
        }
        if let Some(oldest) = self
            .windows
            .iter()
            .min_by_key(|(_, w)| w.start)
            .map(|(key, _)| *key)
        {
            self.windows.remove(&oldest);
        }
    }

    /// Peers tracked.
    pub fn peers(&self) -> usize {
        self.windows.len()
    }
}

/// What [`Reports`] guards.
#[derive(Debug, Default)]
struct Kept {
    ring: VecDeque<ClientReport>,
    budget: Budget,
}

/// The kept reports and the budgets (shared by every request).
#[derive(Debug, Default)]
pub struct Reports {
    state: Mutex<Kept>,
}

impl Reports {
    /// Records a report of peer `key` at `now`: kept (the oldest beyond
    /// [`RING`] forgotten) when its peer is within budget.
    pub fn record(&self, key: IpAddr, now: Instant, report: ClientReport) -> Admit {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let admit = state.budget.admit(key, now);
        if admit == Admit::Keep {
            // One in, at most one out: an `if`, never a loop a mutant could
            // spin in (`.claude/rules/hub-rust.md`).
            state.ring.push_back(report);
            if state.ring.len() > RING {
                state.ring.pop_front();
            }
        }
        admit
    }

    /// The kept reports, oldest first.
    pub fn list(&self) -> Vec<ClientReport> {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.ring.iter().cloned().collect()
    }
}

/// A field for the log line: its value, or `-`.
fn shown(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}

/// `POST /api/client-report`: logs and keeps a page's report. The body is
/// capped at 10 KiB by the route's `DefaultBodyLimit` (`routes.rs`). The
/// answer is 204 also when the peer is over its budget: a 429 would be a
/// console error on the page, and the page does nothing with the answer.
pub async fn client_report(
    State(hub): State<Hub>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(fields): Json<ReportFields>,
) -> StatusCode {
    let origin = access::classify(Some(peer), &headers);
    let source = source(origin);
    let report = ClientReport {
        at: crate::auth::now_secs(),
        peer: peer.ip().to_string(),
        client: forwarded_client(origin, &headers),
        source: source.to_string(),
        fields: clean_fields(fields),
    };
    let key = crate::login_guard::client_key(peer.ip());
    let f = &report.fields;
    match hub.reports.record(key, Instant::now(), report.clone()) {
        Admit::Keep => tracing::info!(
            target: "fohmixer_hub::client_report",
            peer = %report.peer,
            client = shown(report.client.as_deref()),
            source,
            kind = shown(f.kind.as_deref()),
            display = shown(f.display.as_deref()),
            build = shown(f.build.as_deref()),
            host = shown(f.host.as_deref()),
            screen = shown(f.screen.as_deref()),
            sw = shown(f.sw.as_deref()),
            wake_lock = shown(f.wake_lock.as_deref()),
            visibility = shown(f.visibility.as_deref()),
            reconnects = shown(f.reconnects.as_deref()),
            fps = shown(f.fps.as_deref()),
            long_frame_ms = shown(f.long_frame_ms.as_deref()),
            touches_max = shown(f.touches_max.as_deref()),
            pointer = shown(f.pointer.as_deref()),
            error = shown(f.error.as_deref()),
            ua = shown(f.ua.as_deref()),
            "client report",
        ),
        Admit::DropFirst => tracing::warn!(
            target: "fohmixer_hub::client_report",
            peer = %report.peer,
            source,
            max = RATE_MAX,
            window_s = RATE_WINDOW.as_secs(),
            "client reports over the budget: dropped until the window ends",
        ),
        Admit::Drop => {}
    }
    StatusCode::NO_CONTENT
}

#[cfg(test)]
#[path = "client_report/tests.rs"]
mod tests;
