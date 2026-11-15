//! The service's scene read from Companion (#207; the main session's
//! ruling of 2026-11-15): with `[scenes]` the served hub reads Companion's
//! custom variable (`GET /api/custom-variable/<name>/value`, the plain
//! value) every [`POLL`], each read bounded by [`TIMEOUT`], one at a time,
//! in a task of its own (never the router's). A changed value is a scene
//! through the same path as `POST /api/scene` (`RouterMsg::Scene`, source
//! [`SOURCE`]); nothing changes in Companion.
//!
//! The decisions are pure ([`Follow`]): a value equal to the last one does
//! nothing; the first value after the start, or after Companion was down,
//! only becomes the known value and moves nothing (no glide at a hub's or
//! Companion's restart); a changed value that is no scene name
//! (`scenes::scene_name`: an empty variable too) is a warn line and moves
//! nothing. Companion is down after [`DOWN_AFTER`] failed reads in a row
//! (one lost read on a kept-alive connection is no outage), which forgets
//! the known value; it is up again at its first answer. One warn line and
//! one `scene_source` record per outage, one info line and record when it
//! answers again (also at its first answer after the start). A read that
//! gives no value (Companion down, a timeout, a misspelled variable's 404)
//! is a failed read: the lines name the variable's read, not Companion's
//! health. `/api/status` `scenes.companion` is [`Follow::status`].
//!
//! The client keeps its connection (no `pool_max_idle_per_host(0)` as
//! AbleSet's proxy): a read every 500 ms never lets it idle near Node's
//! 5 s keep-alive, and one read lost on a closing connection is no outage
//! ([`DOWN_AFTER`]).

use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use fohmixer_proto::client::SceneSourceStatus;
use fohmixer_proto::scenes::scene_name;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::ableset::link::{Link as Reach, Note, link_step};
use crate::events::EventLog;
use crate::http_client::{HttpClient, Reply};
use crate::ration::{self, Ration};
use crate::router::RouterMsg;

/// How often the variable is read.
pub const POLL: Duration = Duration::from_millis(500);
/// The bound of one read (the request and its whole answer).
pub const TIMEOUT: Duration = Duration::from_secs(2);
/// Failed reads in a row that make Companion down: AbleSet's link rule,
/// shared (`ableset::link::link_step`).
pub use crate::ableset::link::DOWN_AFTER;
/// The `source` of a scene read from Companion (its `scene` record).
pub const SOURCE: &str = "companion";
/// The most refusal lines per `ration::GAP` (a variable that flips between
/// values that are no scene names never floods the log).
pub const REFUSED_LINES: u32 = 4;

/// A read's answer as the poll takes it: the variable's value (a 200 with
/// a UTF-8 body), else why it is no value.
pub fn read_of(reply: anyhow::Result<Reply>) -> Result<String, String> {
    let reply = reply.map_err(|e| format!("{e:#}"))?;
    if reply.status != 200 {
        return Err(format!(
            "Companion answered {}: {}",
            reply.status,
            reply.snippet()
        ));
    }
    String::from_utf8(reply.body.to_vec()).map_err(|_| "the value is no UTF-8 text".to_string())
}

/// Companion's reachability changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    /// It answers (again): the failed reads before, and how long it was down
    /// (none when it was never down since the start).
    Up { failures: u32, down_ms: Option<u64> },
    /// It is down: the read's error and the failed reads in a row.
    Down { error: String, failures: u32 },
}

/// What one read decided.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// A scene to take.
    pub scene: Option<String>,
    /// A changed value that is no scene name: the value and why.
    pub refused: Option<(String, &'static str)>,
    /// Companion went up or down.
    pub link: Option<Link>,
}

/// The poll's state: the known value, Companion up or down.
#[derive(Debug, Default)]
pub struct Follow {
    /// The scene a change is judged against: none at the start and after an
    /// outage; empty when the first value read was no scene name.
    known: Option<String>,
    /// The value read last (kept through an outage, for the status, and so
    /// a refused value is said once while it stays).
    last: Option<String>,
    /// Up or down and the failed reads in a row, by AbleSet's `link_step`.
    reach: Reach,
    /// The last failed read's error.
    error: Option<String>,
    /// UTC ms of the last change of up or down.
    since: Option<u64>,
}

impl Follow {
    /// A value read at `now_ms` (UTC). A refused value never replaces the
    /// known scene, so Companion naming that scene again moves nothing.
    pub fn value(&mut self, raw: &str, now_ms: u64) -> Outcome {
        let was_down = self.reach.online == Some(false);
        let (reach, note) = link_step(self.reach, true);
        self.reach = reach;
        let mut outcome = Outcome::default();
        if let Some(Note::Up { failures }) = note {
            let down_ms = self
                .since
                .filter(|_| was_down)
                .map(|since| now_ms.saturating_sub(since));
            outcome.link = Some(Link::Up { failures, down_ms });
            self.since = Some(now_ms);
        }
        self.error = None;
        let value = raw.trim();
        let changed = self.last.as_deref() != Some(value);
        self.last = Some(value.to_string());
        match scene_name(value) {
            Err(why) => {
                // The known state of a first value that is no scene: none.
                self.known.get_or_insert_with(String::new);
                if changed {
                    outcome.refused = Some((value.to_string(), why));
                }
            }
            Ok(name) => {
                if self.known.as_deref() != Some(name.as_str()) {
                    let first = self.known.replace(name.clone()).is_none();
                    outcome.scene = (!first).then_some(name);
                }
            }
        }
        outcome
    }

    /// A read that failed at `now_ms` (UTC), `error` why.
    pub fn failed(&mut self, error: &str, now_ms: u64) -> Outcome {
        let (reach, note) = link_step(self.reach, false);
        self.reach = reach;
        self.error = Some(error.to_string());
        let mut outcome = Outcome::default();
        if let Some(Note::Down { failures }) = note {
            self.since = Some(now_ms);
            self.known = None;
            outcome.link = Some(Link::Down {
                error: error.to_string(),
                failures,
            });
        }
        outcome
    }

    /// `/api/status` `scenes.companion`: the error only while Companion is
    /// not known to answer.
    pub fn status(&self) -> SceneSourceStatus {
        SceneSourceStatus {
            on: true,
            last: self.last.as_deref().map(|v| ration::clip(v).into_owned()),
            ok: self.reach.online,
            error: self
                .error
                .clone()
                .filter(|_| self.reach.online != Some(true)),
            since: self.since,
        }
    }
}

/// The line of a link change.
pub fn link_line(link: &Link, peer: &str) -> String {
    match link {
        Link::Up {
            failures,
            down_ms: Some(ms),
        } => format!(
            "scenes: Companion's variable at {peer} is read again after {ms} ms, {failures} failed reads"
        ),
        Link::Up { failures, .. } => {
            format!(
                "scenes: Companion's variable at {peer} is read, {failures} failed reads before"
            )
        }
        Link::Down { error, failures } => format!(
            "scenes: Companion's variable at {peer} is NOT READ after {failures} failed reads: {error}"
        ),
    }
}

/// The `scene_source` record of a link change.
pub fn link_fields(link: &Link, peer: &str) -> Value {
    match link {
        Link::Up { failures, down_ms } => {
            json!({"up": true, "peer": peer, "failures": failures, "down_ms": down_ms})
        }
        Link::Down { error, failures } => {
            json!({"up": false, "peer": peer, "failures": failures, "error": error})
        }
    }
}

/// The warn line of a value that is no scene name (`skipped`: the refusal
/// lines left out before it).
pub fn refused_line(value: &str, why: &str, skipped: u64) -> String {
    format!(
        "scenes: Companion's value {:?} is no scene ({why}); {skipped} left out before",
        ration::clip(value)
    )
}

/// The poll of one Companion variable and its state.
pub struct SceneSource {
    /// The variable's value URL.
    pub url: String,
    /// Companion's web server, `host:port` (the records' peer).
    pub peer: String,
    follow: Mutex<Follow>,
}

impl SceneSource {
    pub fn new(url: String, peer: String) -> Self {
        Self {
            url,
            peer,
            follow: Mutex::new(Follow::default()),
        }
    }

    /// A read's answer at `now_ms` decided.
    pub fn heard(&self, read: Result<String, String>, now_ms: u64) -> Outcome {
        let mut follow = self.follow.lock().unwrap_or_else(PoisonError::into_inner);
        match read {
            Ok(value) => follow.value(&value, now_ms),
            Err(error) => follow.failed(&error, now_ms),
        }
    }

    /// The poll could not start (`why`): the status says so.
    pub fn no_client(&self, why: String) {
        self.follow
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .error = Some(why);
    }

    /// `/api/status` `scenes.companion`.
    pub fn status(&self) -> SceneSourceStatus {
        self.follow
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .status()
    }
}

/// UTC ms now.
fn now_ms() -> u64 {
    crate::live::wall_ms().map_or(0, |ms| ms as u64)
}

/// Carries out an outcome: the lines, the records, and the scene handed to
/// `scene` (the router).
pub fn act(
    source: &SceneSource,
    events: &EventLog,
    refusals: &mut Ration,
    outcome: Outcome,
    scene: &impl Fn(String),
) {
    if let Some(link) = &outcome.link {
        let line = link_line(link, &source.peer);
        match link {
            Link::Up { .. } => tracing::info!("{line}"),
            Link::Down { .. } => tracing::warn!("{line}"),
        }
        events.record("scene_source", link_fields(link, &source.peer));
    }
    if let Some((value, why)) = &outcome.refused
        && let Some(skipped) = refusals.take(Instant::now(), REFUSED_LINES)
    {
        tracing::warn!("{}", refused_line(value, why, skipped));
        events.record(
            "scene_source",
            json!({"refused": why, "value": ration::clip(value), "peer": source.peer,
                   "skipped": skipped}),
        );
    }
    if let Some(name) = outcome.scene {
        scene(name);
    }
}

/// The poll: every [`POLL`] one read (a slow read delays the next, never
/// two at once), decided and carried out. Runs until the hub's stop
/// aborts it.
pub async fn follow(
    source: Arc<SceneSource>,
    client: HttpClient,
    events: EventLog,
    scene: impl Fn(String) + Send + 'static,
) {
    tracing::info!(
        peer = %source.peer,
        "scenes: reading the scene from Companion every {} ms",
        POLL.as_millis()
    );
    let mut tick = tokio::time::interval(POLL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut refusals = Ration::default();
    loop {
        tick.tick().await;
        let read = read_of(client.get(&source.url).await);
        let outcome = source.heard(read, now_ms());
        act(&source, &events, &mut refusals, outcome, &scene);
    }
}

/// Starts the poll of `source` (the served hub, `serve_bounded`): each
/// scene goes to `router` as `POST /api/scene`'s does (the answer, how many
/// faders moved, is the router's own `scene` record; nothing waits for it).
/// None, a warn line and the status's `error` when no HTTP client can be
/// made.
pub fn spawn(
    source: &Arc<SceneSource>,
    router: mpsc::UnboundedSender<RouterMsg>,
    events: EventLog,
) -> Option<JoinHandle<()>> {
    let client = match HttpClient::new() {
        Ok(client) => client.with_timeout(TIMEOUT),
        Err(e) => {
            let why = format!("no HTTP client: {e:#}");
            tracing::warn!("scenes: {why}; Companion is not read");
            source.no_client(why);
            return None;
        }
    };
    let peer = source.peer.clone();
    Some(tokio::spawn(follow(
        Arc::clone(source),
        client,
        events,
        move |name| {
            let (reply, _) = oneshot::channel();
            let _ = router.send(RouterMsg::Scene {
                name,
                source: SOURCE.to_string(),
                peer: peer.clone(),
                reply,
            });
        },
    )))
}

#[cfg(test)]
#[path = "scene_source/tests.rs"]
mod tests;
