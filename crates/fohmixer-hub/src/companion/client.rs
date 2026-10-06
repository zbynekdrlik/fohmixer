//! The task that holds the hub's one Companion connection (#52, spec §4),
//! shaped like `live/client.rs`: it reconnects with [`Backoff`] and never
//! gives up; while there is no session a press is answered `offline` at once
//! (never queued); lines go out through a writer task of its own and come in
//! through a reader task of its own (each at most [`MAX_LINE`]), so the
//! session's `select!` never awaits a socket; it pings every 2 s and gives
//! the link up after 5 s without a line. A session is one connection that
//! reached `ADD-DEVICE OK`; each connection registers a fresh device id
//! (`fohmixer-<n>`) under the one serial `fohmixer`. The stop closes the
//! link gracefully: its last lines (the releases, then `REMOVE-DEVICE`) and
//! the hub's FIN go out, the reader reads on until Companion's answer or
//! close, at most [`STOP_BOUND`](super::STOP_BOUND) after the stop, and only
//! then is the socket dropped, so Companion never meets a reset before it
//! has read them.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use fohmixer_proto::client::CompanionStatus;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::{
    Answer, CompanionEvent, Fifo, Inbound, Left, MAX_LINE, PING_EVERY, Phase, Press, Read,
    add_device, api_ok, classify, device_id, first_seen, key_press, ms_between, overdue, ping,
    pong, read_outcome, remove_device, stop_over,
};
use crate::config::CompanionCfg;
use crate::live::Backoff;
use crate::live::client::{CONNECT_TIMEOUT, first_of_outage};

/// How often the session checks its deadlines.
const CHECK_EVERY: Duration = Duration::from_millis(100);
/// Lines read ahead of the session.
const INBOUND_QUEUE: usize = 256;
/// How often the stop's wait checks the clock against the bound.
const STOP_CHECK: Duration = Duration::from_millis(10);

/// Where the task's events go, called from the task in order.
pub type Events = Arc<dyn Fn(CompanionEvent) + Send + Sync>;

/// A request for the task.
enum Request {
    Press(Press),
    Stop,
}

/// The link as the hub last saw it (`/api/status`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub online: bool,
    pub last_error: Option<String>,
    pub connect_failures: u64,
    pub companion_version: Option<String>,
    pub api_version: Option<String>,
    /// The keys Companion drew this session.
    pub keys: u32,
}

fn lock(m: &Mutex<Snapshot>) -> std::sync::MutexGuard<'_, Snapshot> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The handle of the Companion task.
#[derive(Clone)]
pub struct CompanionHandle {
    tx: mpsc::UnboundedSender<Request>,
    snapshot: Arc<Mutex<Snapshot>>,
}

impl CompanionHandle {
    /// Starts the task of `deck`; its events go to `events`. It ends on
    /// [`CompanionHandle::stop`] or when every handle is gone.
    pub fn spawn(deck: &CompanionCfg, events: Events) -> (Self, JoinHandle<()>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let task = tokio::spawn(run(deck.clone(), rx, events, Arc::clone(&snapshot)));
        (Self { tx, snapshot }, task)
    }

    /// Writes a press at once; without a session it is answered `offline`
    /// at once (never queued).
    pub fn press(&self, press: Press) {
        let _ = self.tx.send(Request::Press(press));
    }

    /// Ends the task after every press sent before: `REMOVE-DEVICE` and the
    /// hub's FIN, then Companion's answer or close, at most
    /// [`STOP_BOUND`](super::STOP_BOUND) after the stop; then the close.
    pub fn stop(&self) {
        let _ = self.tx.send(Request::Stop);
    }

    /// The link now.
    pub fn snapshot(&self) -> Snapshot {
        lock(&self.snapshot).clone()
    }

    /// `/api/status`'s companion block.
    pub fn status(&self) -> CompanionStatus {
        let snap = self.snapshot();
        CompanionStatus {
            online: snap.online,
            last_error: snap.last_error,
            connect_failures: snap.connect_failures,
            companion_version: snap.companion_version,
            api_version: snap.api_version,
            keys: snap.keys,
        }
    }
}

/// Why a connection never became a session.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Failure {
    error: String,
    /// Companion refused it (its API version, `ADD-DEVICE ERROR`).
    refused: bool,
    companion: Option<String>,
    api: Option<String>,
}

impl Failure {
    fn plain(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            refused: false,
            companion: None,
            api: None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum End {
    /// A session ended: reconnect.
    Lost(String),
    /// The connection never became a session.
    Failed(Failure),
    /// The hub stops.
    Stop,
}

/// Waits for `work` while answering every press `offline` (no session);
/// `None` when the hub stops (a stop, or every handle gone).
async fn refusing<T>(
    rx: &mut mpsc::UnboundedReceiver<Request>,
    events: &Events,
    work: impl std::future::Future<Output = T>,
) -> Option<T> {
    tokio::pin!(work);
    loop {
        tokio::select! {
            out = &mut work => return Some(out),
            request = rx.recv() => match request {
                Some(Request::Press(press)) => events(CompanionEvent::Answered(Answer::offline(press))),
                Some(Request::Stop) | None => return None,
            },
        }
    }
}

async fn run(
    deck: CompanionCfg,
    mut rx: mpsc::UnboundedReceiver<Request>,
    events: Events,
    snapshot: Arc<Mutex<Snapshot>>,
) {
    let mut backoff = Backoff::default();
    // One counter per kind of id: the device ids count attempts.
    let mut attempts: u64 = 0;
    let mut failures: u64 = 0;
    let mut down_since: Option<Instant> = None;
    loop {
        attempts += 1;
        let device = device_id(attempts);
        let connect = tokio::time::timeout(
            CONNECT_TIMEOUT,
            TcpStream::connect((deck.host.as_str(), deck.port)),
        );
        let Some(attempt) = refusing(&mut rx, &events, connect).await else {
            return;
        };
        let failure = match attempt {
            Ok(Ok(stream)) => {
                let session = Session {
                    deck: &deck,
                    device: &device,
                    events: &events,
                    snapshot: &snapshot,
                };
                match session.run(stream, &mut rx, failures + 1, down_since).await {
                    End::Stop => return,
                    End::Lost(why) => {
                        tracing::info!(device = %device, error = %why, "the Companion link was lost");
                        backoff.reset();
                        failures = 0;
                        down_since = Some(Instant::now());
                        events(CompanionEvent::Down { error: why });
                        None
                    }
                    End::Failed(failure) => Some(failure),
                }
            }
            Ok(Err(error)) => Some(Failure::plain(error.to_string())),
            Err(_) => Some(Failure::plain("connection attempt timed out")),
        };
        if let Some(failure) = failure {
            failures += 1;
            {
                let mut snap = lock(&snapshot);
                snap.connect_failures = failures;
                snap.last_error = Some(failure.error.clone());
            }
            if first_of_outage(failures) {
                tracing::warn!(host = %deck.host, port = deck.port, error = %failure.error, "no usable Companion: retrying every 2 s at most");
                events(CompanionEvent::Failed {
                    error: failure.error,
                    refused: failure.refused,
                    companion: failure.companion,
                    api: failure.api,
                    attempts: failures,
                });
            } else {
                tracing::debug!(host = %deck.host, port = deck.port, failures, error = %failure.error, "still no usable Companion");
            }
        }
        let wait = tokio::time::sleep(backoff.next_delay());
        if refusing(&mut rx, &events, wait).await.is_none() {
            return;
        }
    }
}

/// Reads Companion's lines, each at most [`MAX_LINE`] bytes, into `lines`
/// until the connection ends, a line is too long or the session is gone.
async fn read_lines(mut reader: BufReader<OwnedReadHalf>, lines: mpsc::Sender<Read>) {
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let read = (&mut reader)
            .take(MAX_LINE as u64 + 1)
            .read_until(b'\n', &mut buf)
            .await
            .unwrap_or(0);
        let outcome = read_outcome(read, &buf);
        let last = !matches!(outcome, Read::Line(_));
        if lines.send(outcome).await.is_err() || last {
            return;
        }
    }
}

/// Writes the session's lines in order until a write fails or the session
/// drops its sender; then closes the connection's write side.
async fn write_lines(mut half: OwnedWriteHalf, mut lines: mpsc::UnboundedReceiver<String>) {
    while let Some(line) = lines.recv().await {
        if half.write_all(line.as_bytes()).await.is_err() {
            return;
        }
    }
    let _ = half.shutdown().await;
}

/// The stop's wait after its last lines: Companion's lines are read and
/// dropped until [`stop_over`] says the wait is over (its answer to
/// `REMOVE-DEVICE`, its close, or the bound), the clock checked every
/// [`STOP_CHECK`].
async fn leave(inbound: &mut mpsc::Receiver<Read>, began: Instant) -> Left {
    let mut clock = tokio::time::interval(STOP_CHECK);
    clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let read = tokio::select! {
            // None: the reader is gone, as after a close.
            read = inbound.recv() => Some(read.unwrap_or(Read::Closed)),
            _ = clock.tick() => None,
        };
        if let Some(left) = stop_over(read.as_ref(), began.elapsed()) {
            return left;
        }
    }
}

/// One connection.
struct Session<'a> {
    deck: &'a CompanionCfg,
    device: &'a str,
    events: &'a Events,
    snapshot: &'a Mutex<Snapshot>,
}

/// A connection's state.
struct State {
    phase: Phase,
    /// When the phase began (its deadline).
    since: Instant,
    /// The last line from Companion.
    heard: Instant,
    pings: u64,
    fifo: Fifo,
    seen: BTreeSet<u32>,
    others: BTreeMap<String, u64>,
    companion: Option<String>,
    api: Option<String>,
}

impl State {
    fn new(now: Instant) -> Self {
        Self {
            phase: Phase::Begin,
            since: now,
            heard: now,
            pings: 0,
            fifo: Fifo::default(),
            seen: BTreeSet::new(),
            others: BTreeMap::new(),
            companion: None,
            api: None,
        }
    }

    /// The connection ends for `why`: a lost link once registered, else a
    /// failed attempt.
    fn end(&self, why: String) -> End {
        if self.phase == Phase::Up {
            End::Lost(why)
        } else {
            End::Failed(Failure {
                error: why,
                refused: false,
                companion: self.companion.clone(),
                api: self.api.clone(),
            })
        }
    }
}

impl Session<'_> {
    fn emit(&self, event: CompanionEvent) {
        (self.events)(event);
    }

    async fn run(
        &self,
        stream: TcpStream,
        rx: &mut mpsc::UnboundedReceiver<Request>,
        attempts: u64,
        down_since: Option<Instant>,
    ) -> End {
        let (read_half, write_half) = stream.into_split();
        let (out, lines_out) = mpsc::unbounded_channel::<String>();
        let mut writer = tokio::spawn(write_lines(write_half, lines_out));
        let (lines_in, mut inbound) = mpsc::channel::<Read>(INBOUND_QUEUE);
        let reader = tokio::spawn(read_lines(BufReader::new(read_half), lines_in));
        let mut state = State::new(Instant::now());
        let mut pinger =
            tokio::time::interval_at(tokio::time::Instant::now() + PING_EVERY, PING_EVERY);
        pinger.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut check = tokio::time::interval(CHECK_EVERY);
        check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let end = loop {
            tokio::select! {
                read = inbound.recv() => {
                    let now = Instant::now();
                    state.heard = now;
                    match read {
                        Some(Read::Line(text)) => {
                            if let Some(end) = self.on_line(&mut state, &out, &text, now, (attempts, down_since)) {
                                break end;
                            }
                        }
                        Some(Read::TooLong) => break state.end(format!("a line over {} KiB", MAX_LINE / 1024)),
                        Some(Read::Closed) | None => break state.end("Companion closed the connection".to_string()),
                    }
                }
                request = rx.recv() => match request {
                    Some(Request::Press(press)) => self.on_press(&mut state, &out, press),
                    Some(Request::Stop) | None => {
                        let began = Instant::now();
                        if state.phase == Phase::Up {
                            let _ = out.send(remove_device(self.device));
                        }
                        // The writer writes what waits, then shuts its half
                        // down (the FIN); the reader reads on until the wait
                        // is over, and only then is the socket dropped.
                        drop(out);
                        let left = leave(&mut inbound, began).await;
                        let written = writer.is_finished();
                        writer.abort();
                        reader.abort();
                        tracing::info!(device = self.device, left = ?left, written, "the Stream Deck left Companion: the hub stops");
                        return End::Stop;
                    }
                },
                _ = &mut writer => break state.end("a write to Companion failed".to_string()),
                _ = pinger.tick() => {
                    state.pings += 1;
                    let _ = out.send(ping(state.pings));
                }
                _ = check.tick() => {
                    let now = Instant::now();
                    if let Some(why) = overdue(state.phase, now - state.since, now - state.heard) {
                        break state.end(why.to_string());
                    }
                }
            }
        };
        writer.abort();
        reader.abort();
        for answer in state.fifo.offline() {
            self.emit(CompanionEvent::Answered(answer));
        }
        if let End::Lost(why) = &end {
            tracing::info!(device = self.device, others = ?state.others, "the Companion session ended");
            // Every field spelled out: no struct-update base.
            *lock(self.snapshot) = Snapshot {
                online: false,
                last_error: Some(why.clone()),
                connect_failures: 0,
                companion_version: None,
                api_version: None,
                keys: 0,
            };
        }
        end
    }

    /// One line from Companion; the connection's end when it ends it. The
    /// match names every kind (no wildcard): a line that does not belong to
    /// the phase is counted and logged once per command per session.
    fn on_line(
        &self,
        state: &mut State,
        out: &mpsc::UnboundedSender<String>,
        text: &str,
        now: Instant,
        (attempts, down_since): (u64, Option<Instant>),
    ) -> Option<End> {
        let up = state.phase == Phase::Up;
        let ignored = match classify(text) {
            Inbound::Begin { companion, api } => {
                if state.phase != Phase::Begin {
                    Some("BEGIN".to_string())
                } else {
                    return self.on_begin(state, out, companion, api, now);
                }
            }
            Inbound::Caps(caps) => {
                tracing::info!(device = self.device, caps = %caps, "Companion's capabilities");
                None
            }
            Inbound::Added(result) => {
                if state.phase != Phase::Adding {
                    Some("ADD-DEVICE".to_string())
                } else {
                    return self.on_added(state, result, now, attempts, down_since);
                }
            }
            Inbound::KeyState(update) => {
                if up {
                    state.seen.insert(update.key);
                    lock(self.snapshot).keys = u32::try_from(state.seen.len()).unwrap_or(u32::MAX);
                    self.emit(CompanionEvent::Key(update));
                    None
                } else {
                    Some("KEY-STATE".to_string())
                }
            }
            Inbound::KeysClear => {
                if up {
                    self.emit(CompanionEvent::Clear);
                    None
                } else {
                    Some("KEYS-CLEAR".to_string())
                }
            }
            Inbound::Pressed(result) => {
                if up {
                    match state.fifo.answer(result, now) {
                        Some(answer) => {
                            self.emit(CompanionEvent::Answered(answer));
                            None
                        }
                        None => Some("KEY-PRESS".to_string()),
                    }
                } else {
                    Some("KEY-PRESS".to_string())
                }
            }
            Inbound::Ping(payload) => {
                let _ = out.send(pong(&payload));
                None
            }
            Inbound::Pong => None,
            Inbound::Other(cmd) => Some(cmd),
        };
        if let Some(cmd) = ignored
            && first_seen(&mut state.others, &cmd)
        {
            tracing::debug!(device = self.device, cmd = %cmd, "a Companion line the hub ignores");
        }
        None
    }

    /// `BEGIN`: the API gate, then `ADD-DEVICE`.
    fn on_begin(
        &self,
        state: &mut State,
        out: &mpsc::UnboundedSender<String>,
        companion: String,
        api: String,
        now: Instant,
    ) -> Option<End> {
        state.companion = Some(companion.clone());
        state.api = Some(api.clone());
        if !api_ok(&api) {
            return Some(End::Failed(Failure {
                error: format!("Companion's Satellite API {api} is not 1.12 or a later 1.x"),
                refused: true,
                companion: Some(companion),
                api: Some(api),
            }));
        }
        let _ = out.send(add_device(self.device, self.deck));
        state.phase = Phase::Adding;
        state.since = now;
        None
    }

    /// The `ADD-DEVICE` answer: registered (the `Up` event), or refused.
    fn on_added(
        &self,
        state: &mut State,
        result: Result<(), String>,
        now: Instant,
        attempts: u64,
        down_since: Option<Instant>,
    ) -> Option<End> {
        if let Err(message) = result {
            return Some(End::Failed(Failure {
                error: format!("ADD-DEVICE refused: {message}"),
                refused: true,
                companion: state.companion.clone(),
                api: state.api.clone(),
            }));
        }
        state.phase = Phase::Up;
        state.since = now;
        let companion = state.companion.clone().unwrap_or_default();
        let api = state.api.clone().unwrap_or_default();
        tracing::info!(device = self.device, companion = %companion, api = %api, "Stream Deck registered with Companion");
        // Every field spelled out: no struct-update base.
        *lock(self.snapshot) = Snapshot {
            online: true,
            last_error: None,
            connect_failures: 0,
            companion_version: Some(companion.clone()),
            api_version: Some(api.clone()),
            keys: 0,
        };
        self.emit(CompanionEvent::Up {
            companion,
            api,
            attempts,
            down_ms: down_since.map(|at| ms_between(at, now)),
        });
        None
    }

    /// A press: written at once in a session, else (no session yet, or the
    /// writer gone) answered `offline`, never written later.
    fn on_press(&self, state: &mut State, out: &mpsc::UnboundedSender<String>, press: Press) {
        let written = state.phase == Phase::Up
            && out
                .send(key_press(self.device, press.key, press.down))
                .is_ok();
        if written {
            state.fifo.sent(press, Instant::now());
        } else {
            self.emit(CompanionEvent::Answered(Answer::offline(press)));
        }
    }
}

#[cfg(test)]
mod tests;
