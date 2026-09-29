//! One reconnecting client per Live instance (spec §2.4): a task that holds
//! the WebSocket to the FohMixer script on `127.0.0.1:<port>`.
//!
//! - It reconnects with a backoff of 250 ms doubling to 2 s, and never gives
//!   up. A connection is a session only once the script's `connect` names
//!   this instance: a port that answers as another instance (a swapped
//!   `Config.py`) is refused, never used. Until then — offline, connecting,
//!   waiting for `connect` — a request is refused at once (never queued):
//!   its reply is dropped, which the caller reads as [`LiveError::Offline`].
//! - Failed attempts are counted and the last reason kept for
//!   `/api/status`; the first failure of an outage is a warning, the retries
//!   are debug lines.
//! - Frames go out in the order requests arrive, written by a task of their
//!   own: a write that waits (the script reads slowly) never stops the
//!   session from reading the script's frames and stamping its heartbeats
//!   (#9: a heartbeat read late made the instance busy while Live was
//!   fine). Each result is matched by its uuid: a caller's own request
//!   ([`LiveHandle::call`]) gets it back directly, anything else (the
//!   subscription table's requests) goes to the event sink in frame order,
//!   with the value pushes.
//! - Busy: Live's main-thread tick older than 150 ms, or no heartbeat for
//!   300 ms (checked every 50 ms), reported on change with its reason; a
//!   heartbeat after an overdue gap is logged with where it was held.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::{
    Backoff, ConnectInfo, Frame, LiveValue, busy_reason, late_heartbeat, parse_frame, wall_ms,
};
use crate::config::InstanceCfg;

/// How long a caller waits for a result.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a connection attempt may take (TCP and the handshake).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// How often the heartbeat's age is checked for "busy".
pub const BUSY_CHECK: Duration = Duration::from_millis(50);

/// What an instance's task reports, in the order the script sent it.
#[derive(Debug, Clone, PartialEq)]
pub enum LiveEvent {
    Connected(ConnectInfo),
    Disconnected,
    Busy {
        busy: bool,
    },
    /// The result of a request sent with [`LiveHandle::send`].
    Result {
        uuid: String,
        data: Vec<Value>,
    },
    Values(Vec<LiveValue>),
}

/// Where the events go: called with the instance name, from its task.
pub type Events = Arc<dyn Fn(&str, LiveEvent) + Send + Sync>;

/// Why a request got no result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveError {
    /// Not connected (or the connection ended before the result).
    Offline,
    /// No result within [`REQUEST_TIMEOUT`].
    Timeout,
    /// The script refused the whole request.
    Refused(String),
}

impl std::fmt::Display for LiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Offline => write!(f, "instance offline"),
            Self::Timeout => write!(f, "no result within {} s", REQUEST_TIMEOUT.as_secs()),
            Self::Refused(message) => write!(f, "refused: {message}"),
        }
    }
}

type Reply = oneshot::Sender<Result<Vec<Value>, LiveError>>;

/// A request for the script. Dropping it unanswered (offline, or the
/// connection ends) tells its caller [`LiveError::Offline`].
struct Request {
    uuid: String,
    commands: Vec<Value>,
    reply: Option<Reply>,
}

/// The instance as the hub last saw it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub online: bool,
    pub busy: bool,
    pub info: ConnectInfo,
    pub main_tick_age_ms: Option<f64>,
    /// Failed connection attempts since the last session.
    pub connect_failures: u64,
    /// Why the last attempt failed, until a session starts.
    pub last_error: Option<String>,
}

/// The handle of one instance's task.
#[derive(Clone)]
pub struct LiveHandle {
    name: String,
    port: u16,
    tx: mpsc::UnboundedSender<Request>,
    snapshot: Arc<Mutex<Snapshot>>,
    requests: Arc<AtomicU64>,
}

fn lock(m: &Mutex<Snapshot>) -> std::sync::MutexGuard<'_, Snapshot> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl LiveHandle {
    /// Starts the task of `cfg`; its events go to `events`. The task ends
    /// when every handle is dropped (or the returned task is aborted).
    pub fn spawn(cfg: &InstanceCfg, events: Events) -> (Self, JoinHandle<()>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let handle = Self {
            name: cfg.name.clone(),
            port: cfg.port,
            tx,
            snapshot: Arc::clone(&snapshot),
            requests: Arc::new(AtomicU64::new(0)),
        };
        let task = tokio::spawn(run(cfg.clone(), rx, events, snapshot));
        (handle, task)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The current state.
    pub fn snapshot(&self) -> Snapshot {
        lock(&self.snapshot).clone()
    }

    /// Sends a request whose result comes back as [`LiveEvent::Result`].
    pub fn send(&self, uuid: String, commands: Vec<Value>) {
        let _ = self.tx.send(Request {
            uuid,
            commands,
            reply: None,
        });
    }

    /// Queues a request now (so requests keep their order) and waits up to
    /// [`REQUEST_TIMEOUT`] for its result slots.
    pub fn call(
        &self,
        commands: Vec<Value>,
    ) -> impl std::future::Future<Output = Result<Vec<Value>, LiveError>> + Send + 'static {
        let n = self.requests.fetch_add(1, Ordering::Relaxed);
        let (reply, result) = oneshot::channel();
        let queued = self
            .tx
            .send(Request {
                uuid: format!("c{n}"),
                commands,
                reply: Some(reply),
            })
            .is_ok();
        async move {
            if !queued {
                return Err(LiveError::Offline);
            }
            match tokio::time::timeout(REQUEST_TIMEOUT, result).await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err(LiveError::Offline),
                Err(_) => Err(LiveError::Timeout),
            }
        }
    }
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Writes the session's requests to the script, in order, until a write
/// fails or the session drops its sender.
async fn write_requests(
    mut sink: SplitSink<Socket, Message>,
    mut texts: mpsc::UnboundedReceiver<String>,
) {
    while let Some(text) = texts.recv().await {
        if sink.send(Message::Text(text.into())).await.is_err() {
            return;
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum End {
    /// A session ended: reconnect.
    Lost,
    /// The connection never became a session: it closed before the script's
    /// `connect`, or the port answers as another instance.
    Failed(String),
    /// Every handle is gone: the task ends.
    Stop,
}

/// Waits for `work` while refusing every request (no session); `None` when
/// every handle is gone.
async fn refusing<T>(
    rx: &mut mpsc::UnboundedReceiver<Request>,
    work: impl std::future::Future<Output = T>,
) -> Option<T> {
    tokio::pin!(work);
    loop {
        tokio::select! {
            out = &mut work => return Some(out),
            request = rx.recv() => {
                // None: every handle is gone. A request is refused: dropping
                // it tells its caller "offline".
                request?;
            }
        }
    }
}

/// Whether a failed attempt is the first of an outage (a warning; the
/// retries are debug lines).
fn first_of_outage(failures: u64) -> bool {
    failures == 1
}

async fn run(
    cfg: InstanceCfg,
    mut rx: mpsc::UnboundedReceiver<Request>,
    events: Events,
    snapshot: Arc<Mutex<Snapshot>>,
) {
    let url = format!("ws://127.0.0.1:{}", cfg.port);
    let mut backoff = Backoff::default();
    let mut failures: u64 = 0;
    loop {
        let connect = tokio::time::timeout(
            CONNECT_TIMEOUT,
            tokio_tungstenite::connect_async(url.as_str()),
        );
        let Some(attempt) = refusing(&mut rx, connect).await else {
            return;
        };
        let failure = match attempt {
            Ok(Ok((socket, _))) => {
                tracing::debug!(instance = %cfg.name, port = cfg.port, "WebSocket open: waiting for the script's connect");
                let session = Session {
                    name: &cfg.name,
                    port: cfg.port,
                    events: &events,
                    snapshot: &snapshot,
                };
                match session.run(socket, &mut rx).await {
                    End::Stop => return,
                    End::Lost => {
                        backoff.reset();
                        failures = 0;
                        None
                    }
                    End::Failed(why) => Some(why),
                }
            }
            Ok(Err(error)) => Some(error.to_string()),
            Err(_) => Some("connection attempt timed out".to_string()),
        };
        if let Some(why) = failure {
            failures += 1;
            {
                let mut snap = lock(&snapshot);
                snap.connect_failures = failures;
                snap.last_error = Some(why.clone());
            }
            if first_of_outage(failures) {
                tracing::warn!(instance = %cfg.name, port = cfg.port, error = %why, "no usable FohMixer script: retrying every 2 s at most");
            } else {
                tracing::debug!(instance = %cfg.name, port = cfg.port, failures, error = %why, "still no usable FohMixer script");
            }
        }
        let wait = tokio::time::sleep(backoff.next_delay());
        if refusing(&mut rx, wait).await.is_none() {
            return;
        }
    }
}

/// One connection's state.
struct Session<'a> {
    name: &'a str,
    port: u16,
    events: &'a Events,
    snapshot: &'a Mutex<Snapshot>,
}

/// The busy state of one connection.
struct Health {
    connected: bool,
    busy: bool,
    age_ms: f64,
    last_heartbeat: Instant,
}

impl Session<'_> {
    fn emit(&self, event: LiveEvent) {
        (self.events)(self.name, event);
    }

    /// Reports a busy change, with its reason.
    fn check_busy(&self, health: &mut Health) {
        let reason = busy_reason(health.age_ms, health.last_heartbeat.elapsed());
        let busy = reason.is_some();
        if busy != health.busy {
            health.busy = busy;
            lock(self.snapshot).busy = busy;
            tracing::info!(
                instance = self.name,
                busy,
                main_tick_age_ms = health.age_ms,
                reason = reason
                    .as_deref()
                    .unwrap_or("heartbeats in time, Live's main thread ticking"),
                "Live busy changed"
            );
            self.emit(LiveEvent::Busy { busy });
        }
    }

    async fn run(&self, socket: Socket, rx: &mut mpsc::UnboundedReceiver<Request>) -> End {
        let (sink, mut stream) = socket.split();
        // Requests go out through their own task, so this loop never waits
        // for a write and keeps reading the script's frames.
        let (out, texts) = mpsc::unbounded_channel::<String>();
        let mut writer = tokio::spawn(write_requests(sink, texts));
        let mut pending: HashMap<String, Reply> = HashMap::new();
        let mut health = Health {
            connected: false,
            busy: false,
            age_ms: 0.0,
            last_heartbeat: Instant::now(),
        };
        let mut tick = tokio::time::interval(BUSY_CHECK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let end = loop {
            tokio::select! {
                frame = stream.next() => {
                    let text = match frame {
                        Some(Ok(Message::Text(text))) => text,
                        Some(Ok(Message::Close(_)) | Err(_)) | None => break End::Lost,
                        Some(Ok(_)) => continue,
                    };
                    match parse_frame(text.as_str()) {
                        Ok(Frame::Disconnect) => {
                            tracing::info!(instance = self.name, "the script said disconnect (a set load or Live closing)");
                            break End::Lost;
                        }
                        Ok(Frame::Connect(info)) if info.instance != self.name => {
                            // A swapped port: commands and STAGE AUT writes
                            // would go to the wrong Live.
                            break End::Failed(format!(
                                "port {} answers as instance {:?}, not {:?}",
                                self.port, info.instance, self.name
                            ));
                        }
                        Ok(frame) => self.on_frame(frame, &mut health, &mut pending),
                        Err(error) => {
                            tracing::warn!(instance = self.name, error, "unreadable frame from the script");
                        }
                    }
                }
                request = rx.recv() => {
                    let Some(request) = request else { break End::Stop };
                    if !health.connected {
                        // No session yet (no `connect`): refused, as offline.
                        continue;
                    }
                    let text = json!({"uuid": request.uuid, "commands": request.commands}).to_string();
                    if out.send(text).is_err() {
                        // The writer ended: dropped with the request, its
                        // caller reads "offline".
                        break End::Lost;
                    }
                    if let Some(reply) = request.reply {
                        pending.insert(request.uuid, reply);
                    }
                }
                // A write failed: the connection is gone.
                _ = &mut writer => break End::Lost,
                _ = tick.tick() => {
                    if health.connected {
                        self.check_busy(&mut health);
                    }
                }
            }
        };
        writer.abort();
        if !health.connected {
            return match end {
                End::Lost => {
                    End::Failed("the connection closed before the script's connect".into())
                }
                other => other,
            };
        }
        *lock(self.snapshot) = Snapshot::default();
        tracing::info!(
            instance = self.name,
            "disconnected from the FohMixer script"
        );
        self.emit(LiveEvent::Disconnected);
        end
    }

    fn on_frame(&self, frame: Frame, health: &mut Health, pending: &mut HashMap<String, Reply>) {
        match frame {
            Frame::Connect(info) => {
                tracing::info!(
                    instance = self.name,
                    set_name = %info.set_name,
                    live_version = %info.live_version,
                    script_version = %info.script_version,
                    "script connected"
                );
                health.connected = true;
                health.busy = false;
                health.last_heartbeat = Instant::now();
                // Every field spelled out: no struct-update base (its
                // "delete field" mutants escape the recheck's --re filter).
                *lock(self.snapshot) = Snapshot {
                    online: true,
                    busy: false,
                    info: info.clone(),
                    main_tick_age_ms: None,
                    connect_failures: 0,
                    last_error: None,
                };
                self.emit(LiveEvent::Connected(info));
            }
            Frame::Heartbeat {
                main_tick_age_ms,
                gap_ms,
                sent_ms,
            } => {
                // A heartbeat can come before the connect from a script
                // older than the main-tick transport: it listed a new
                // connection for heartbeats before its sender thread had
                // sent the queued connect, and Live runs it until Live
                // restarts (#5; the current script sends connect first).
                // It is recorded, and busy is checked only once connected.
                if let Some(note) =
                    late_heartbeat(health.last_heartbeat.elapsed(), gap_ms, sent_ms, wall_ms())
                {
                    tracing::warn!(instance = self.name, main_tick_age_ms, "{note}");
                }
                health.age_ms = main_tick_age_ms;
                health.last_heartbeat = Instant::now();
                lock(self.snapshot).main_tick_age_ms = Some(main_tick_age_ms);
                if health.connected {
                    self.check_busy(health);
                }
            }
            Frame::Result { uuid, data } => match pending.remove(&uuid) {
                Some(reply) => {
                    let _ = reply.send(Ok(data));
                }
                None => self.emit(LiveEvent::Result { uuid, data }),
            },
            Frame::Values(items) => self.emit(LiveEvent::Values(items)),
            Frame::Error { uuid, message } => {
                tracing::warn!(instance = self.name, uuid = ?uuid, message, "the script refused a request");
                let Some(uuid) = uuid else { return };
                match pending.remove(&uuid) {
                    Some(reply) => {
                        let _ = reply.send(Err(LiveError::Refused(message)));
                    }
                    None => self.emit(LiveEvent::Result {
                        uuid,
                        data: Vec::new(),
                    }),
                }
            }
            Frame::Disconnect => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_first_failure_of_an_outage_warns() {
        assert!(first_of_outage(1));
        assert!(!first_of_outage(0));
        assert!(!first_of_outage(2));
    }

    #[tokio::test]
    async fn a_request_while_connecting_is_refused_at_once() {
        // A port that accepts TCP but never answers the WebSocket handshake:
        // the task waits CONNECT_TIMEOUT for it.
        let silent = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = silent.local_addr().unwrap().port();
        let events: Events = Arc::new(|_: &str, _: LiveEvent| {});
        let (handle, _task) = LiveHandle::spawn(
            &InstanceCfg {
                name: "band".into(),
                port,
            },
            events,
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        let started = Instant::now();
        let refused = handle
            .call(vec![
                json!({"target": "live_set", "name": "get_prop", "args": {"prop": "tempo"}}),
            ])
            .await;
        assert_eq!(refused, Err(LiveError::Offline));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "refused at once, not queued for the next session: {:?}",
            started.elapsed()
        );
        // The attempt times out and is counted with its reason.
        let deadline = Instant::now() + CONNECT_TIMEOUT + Duration::from_secs(2);
        while handle.snapshot().connect_failures == 0 {
            assert!(Instant::now() < deadline, "{:?}", handle.snapshot());
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(
            handle.snapshot().last_error.as_deref(),
            Some("connection attempt timed out")
        );
        drop(silent);
    }

    #[test]
    fn errors_read_well() {
        assert_eq!(LiveError::Offline.to_string(), "instance offline");
        assert_eq!(LiveError::Timeout.to_string(), "no result within 3 s");
        assert_eq!(LiveError::Refused("bad".into()).to_string(), "refused: bad");
    }

    #[tokio::test]
    async fn a_request_while_offline_is_refused_at_once() {
        // A port nothing listens on: the task stays offline.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let seen = Arc::new(Mutex::new(Vec::<LiveEvent>::new()));
        let sink = Arc::clone(&seen);
        let events: Events = Arc::new(move |_: &str, e: LiveEvent| {
            sink.lock().unwrap().push(e);
        });
        let (handle, task) = LiveHandle::spawn(
            &InstanceCfg {
                name: "band".into(),
                port,
            },
            events,
        );
        assert_eq!(handle.name(), "band");
        assert_eq!(handle.port(), port);
        let started = Instant::now();
        assert_eq!(
            handle
                .call(vec![
                    json!({"target": "live_set", "name": "get_prop", "args": {"prop": "tempo"}})
                ])
                .await,
            Err(LiveError::Offline)
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "refused at once"
        );
        assert!(!handle.snapshot().online);
        // Every refused attempt is counted, with its reason (250 ms, 500 ms…
        // apart: a few within a second).
        let deadline = Instant::now() + Duration::from_secs(3);
        while handle.snapshot().connect_failures < 2 {
            assert!(Instant::now() < deadline, "{:?}", handle.snapshot());
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let snap = handle.snapshot();
        assert!((2..=10).contains(&snap.connect_failures), "{snap:?}");
        assert!(snap.last_error.is_some(), "{snap:?}");
        assert!(
            seen.lock().unwrap().is_empty(),
            "never connected: no events"
        );
        // Dropping every handle ends the task.
        drop(handle);
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the task ends without handles")
            .unwrap();
    }
}
