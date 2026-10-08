//! Test support for the hub's integration tests: real `sim/host.py`
//! processes (the real FohMixer script on SimLive, S2), an in-process hub on
//! an ephemeral port, and a WebSocket client of the client protocol.
//!
//! A host is only ever asked to stop (SIGTERM, then a bounded wait): spec
//! I7, nothing is force-ended. The hosts need a Unix shell (`kill`); the
//! host-free tests (layout, auth) also run on Windows, the hub's platform.
#![allow(dead_code)] // every test binary uses its own part of this module

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fohmixer_hub::config::{Config, InstanceCfg};
use fohmixer_proto::client::{AckItem, ClientMsg, HubStatus, LiveCommand, ServerMsg, ValueItem};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

#[cfg(unix)]
mod host;
#[cfg(unix)]
#[allow(unused_imports)] // the host-free test binaries do not use it
pub use host::Host;
pub mod companion;

/// One host-backed test at a time within a test binary (`cargo test` runs
/// a binary's tests on parallel threads; nextest runs them in the
/// `hub-hosts` group instead): the timing checks measure the hub, not a
/// crowded runner.
pub fn serial() -> std::sync::MutexGuard<'static, ()> {
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
    SERIAL
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The repository root.
pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A hub running in this process on 127.0.0.1, port 0.
pub struct TestHub {
    pub addr: SocketAddr,
    pub dir: PathBuf,
    pub token: String,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<anyhow::Result<()>>>,
}

/// Every event-log record in the data folder `dir` (#43), oldest day first
/// (a stopped hub's too).
pub fn events_in(dir: &Path) -> Vec<Value> {
    let logs = dir.join("logs");
    let mut days: Vec<PathBuf> = std::fs::read_dir(&logs)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with("events-"))
                })
                .collect()
        })
        .unwrap_or_default();
    days.sort();
    days.iter()
        .flat_map(|day| {
            std::fs::read_to_string(day)
                .unwrap_or_default()
                .lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .collect::<Vec<_>>()
        })
        .collect()
}

impl TestHub {
    /// A hub over these instances in a new temporary data folder.
    pub async fn start(instances: Vec<InstanceCfg>, dir: &Path) -> Self {
        let mut config = Config::defaults(dir);
        config.instances = instances;
        config.layout_poll_ms = 100;
        Self::start_config(config).await
    }

    /// A hub of `config`.
    pub async fn start_config(config: Config) -> Self {
        let dir = config.data_dir.clone();
        let (ready_tx, ready_rx) = oneshot::channel();
        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let task = tokio::spawn(fohmixer_hub::serve_until(
            SocketAddr::from(([127, 0, 0, 1], 0)),
            config,
            ready_tx,
            async move {
                let _ = stop_rx.await;
            },
        ));
        let addr = tokio::time::timeout(Duration::from_secs(10), ready_rx)
            .await
            .expect("the hub is ready within 10 s")
            .expect("the hub reports its address");
        let secret = std::fs::read_to_string(dir.join("secrets").join("jwt_secret")).unwrap();
        let token =
            fohmixer_hub::auth::issue_token(secret.trim(), fohmixer_hub::auth::now_secs()).unwrap();
        Self {
            addr,
            dir,
            token,
            stop: Some(stop_tx),
            task: Some(task),
        }
    }

    /// The client WebSocket URL with the token and protocol 2.
    pub fn ws_url(&self) -> String {
        format!("ws://{}/ws?token={}&proto=2", self.addr, self.token)
    }

    /// Every event-log record written so far (#43), oldest day first.
    pub fn events(&self) -> Vec<Value> {
        events_in(&self.dir)
    }

    /// Waits up to `limit` for the event log to satisfy `check`; the records.
    pub async fn events_until(
        &self,
        limit: Duration,
        check: impl Fn(&[Value]) -> bool,
    ) -> Vec<Value> {
        let deadline = Instant::now() + limit;
        loop {
            let records = self.events();
            if check(&records) {
                return records;
            }
            assert!(
                Instant::now() < deadline,
                "the event log never matched: {records:?}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// A connected client (past its hello).
    pub async fn client(&self) -> Client {
        Client::connect(&self.ws_url()).await
    }

    /// `GET path` with the token: the status code and the body.
    pub async fn get(&self, path: &str) -> (u16, Value) {
        http(self.addr, "GET", path, Some(&self.token), None).await
    }

    /// `POST path` with the token and no body: the status code and the body.
    pub async fn post(&self, path: &str) -> (u16, Value) {
        http(self.addr, "POST", path, Some(&self.token), None).await
    }

    /// `GET /api/status`.
    pub async fn status(&self) -> HubStatus {
        let (code, body) = self.get("/api/status").await;
        assert_eq!(code, 200, "{body}");
        serde_json::from_value(body).unwrap()
    }

    /// Waits up to `limit` for `check` on the status.
    pub async fn status_until(
        &self,
        limit: Duration,
        check: impl Fn(&HubStatus) -> bool,
    ) -> HubStatus {
        let deadline = Instant::now() + limit;
        loop {
            let status = self.status().await;
            if check(&status) {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "status never matched: {status:?}"
            );
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    /// Stops the hub and waits for `serve_until` to return.
    pub async fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(task) = self.task.take() {
            tokio::time::timeout(Duration::from_secs(10), task)
                .await
                .expect("the hub stops within 10 s")
                .unwrap()
                .unwrap();
        }
    }
}

/// One HTTP/1.1 request on its own connection: the status code and the JSON
/// body (`null` when the body is empty or not JSON).
pub async fn http(
    addr: SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&Value>,
) -> (u16, Value) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let body = body.map(Value::to_string).unwrap_or_default();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    if !body.is_empty() {
        request.push_str("Content-Type: application/json\r\n");
    }
    if let Some(token) = token {
        request.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(&body);
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut text = String::new();
    tokio::time::timeout(Duration::from_secs(10), stream.read_to_string(&mut text))
        .await
        .expect("an answer within 10 s")
        .unwrap();
    let code: u16 = text
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or_else(|| panic!("not an HTTP answer: {text}"));
    let json = text
        .split_once("\r\n\r\n")
        .and_then(|(_, b)| serde_json::from_str(b).ok())
        .unwrap_or(Value::Null);
    (code, json)
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A client of the client protocol that keeps every message it received.
pub struct Client {
    ws: Socket,
    pub hello: ServerMsg,
    /// Messages received but not yet taken by a `wait`.
    pending: Vec<ServerMsg>,
    next_id: u64,
}

impl Client {
    /// Connects and reads the hello.
    pub async fn connect(url: &str) -> Self {
        let (mut ws, _) = tokio_tungstenite::connect_async(url)
            .await
            .expect("the WebSocket opens");
        let hello = next_msg(&mut ws, Duration::from_secs(5))
            .await
            .expect("a hello");
        Self {
            ws,
            hello,
            pending: Vec::new(),
            next_id: 0,
        }
    }

    /// Forgets the messages received but not yet taken.
    pub fn clear(&mut self) {
        self.pending.clear();
    }

    /// The raw socket (a client that never reads).
    pub fn into_socket(self) -> Socket {
        self.ws
    }

    pub async fn send(&mut self, msg: &ClientMsg) {
        self.ws
            .send(Message::Text(serde_json::to_string(msg).unwrap().into()))
            .await
            .unwrap();
    }

    /// Sends raw text.
    pub async fn send_text(&mut self, text: &str) {
        self.ws
            .send(Message::Text(text.to_string().into()))
            .await
            .unwrap();
    }

    /// The first message (received earlier or within `limit`) that `take`
    /// maps to something.
    pub async fn wait<T>(&mut self, limit: Duration, take: impl Fn(&ServerMsg) -> Option<T>) -> T {
        if let Some(i) = self.pending.iter().position(|m| take(m).is_some()) {
            let msg = self.pending.remove(i);
            return take(&msg).unwrap();
        }
        let deadline = Instant::now() + limit;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let msg = next_msg(&mut self.ws, left).await.unwrap_or_else(|| {
                panic!("nothing matched within {limit:?}; got {:?}", self.pending)
            });
            if let Some(found) = take(&msg) {
                return found;
            }
            self.pending.push(msg);
        }
    }

    /// The next message in arrival order (one received earlier first);
    /// `None` when nothing comes within `limit`.
    pub async fn next_message(&mut self, limit: Duration) -> Option<ServerMsg> {
        if !self.pending.is_empty() {
            return Some(self.pending.remove(0));
        }
        next_msg(&mut self.ws, limit).await
    }

    /// Whether a message `take` maps to arrives within `limit`.
    pub async fn gets<T>(
        &mut self,
        limit: Duration,
        take: impl Fn(&ServerMsg) -> Option<T>,
    ) -> Option<T> {
        if let Some(i) = self.pending.iter().position(|m| take(m).is_some()) {
            let msg = self.pending.remove(i);
            return take(&msg);
        }
        let deadline = Instant::now() + limit;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let msg = next_msg(&mut self.ws, left).await?;
            if let Some(found) = take(&msg) {
                return Some(found);
            }
            self.pending.push(msg);
        }
    }

    /// Subscribes; the `subbed` answer.
    pub async fn sub(
        &mut self,
        instance: &str,
        target: &str,
        prop: &str,
        display: bool,
    ) -> ServerMsg {
        self.send(&ClientMsg::Sub {
            instance: instance.into(),
            target: target.into(),
            prop: prop.into(),
            display,
        })
        .await;
        self.wait(Duration::from_secs(5), |m| {
            matches!(m, ServerMsg::Subbed { .. }).then(|| m.clone())
        })
        .await
    }

    /// The hub key of a subscription (from its `subbed`).
    pub async fn sub_key(
        &mut self,
        instance: &str,
        target: &str,
        prop: &str,
        display: bool,
    ) -> String {
        match self.sub(instance, target, prop, display).await {
            ServerMsg::Subbed { sub, .. } => sub,
            other => panic!("not subbed: {other:?}"),
        }
    }

    /// The next item for `sub` in a `values` message.
    pub async fn value_of(&mut self, sub: &str, limit: Duration) -> ValueItem {
        let sub = sub.to_string();
        self.wait(limit, move |m| match m {
            ServerMsg::Values { items } => items.iter().find(|i| i.sub == sub).cloned(),
            _ => None,
        })
        .await
    }

    /// Waits for `sub`'s value to satisfy `check`.
    pub async fn value_until(
        &mut self,
        sub: &str,
        limit: Duration,
        check: impl Fn(&ValueItem) -> bool,
    ) -> ValueItem {
        let deadline = Instant::now() + limit;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let item = self.value_of(sub, left).await;
            if check(&item) {
                return item;
            }
        }
    }

    /// A command batch on `instance`: the result slots, or the error message.
    pub async fn cmd(
        &mut self,
        instance: &str,
        commands: Vec<Value>,
    ) -> Result<Vec<Value>, String> {
        self.next_id += 1;
        let id = format!("t{}", self.next_id);
        let commands: Vec<LiveCommand> = commands
            .into_iter()
            .map(|c| serde_json::from_value(c).unwrap())
            .collect();
        self.send(&ClientMsg::Cmd {
            id: id.clone(),
            instance: instance.into(),
            commands,
        })
        .await;
        self.wait(Duration::from_secs(5), move |m| match m {
            ServerMsg::Result { id: got, data } if *got == id => Some(Ok(data.clone())),
            ServerMsg::Error {
                id: Some(got),
                message,
            } if *got == id => Some(Err(message.clone())),
            _ => None,
        })
        .await
    }

    /// `set_prop` through a command; panics unless the slot is ok.
    pub async fn set(&mut self, instance: &str, target: &str, prop: &str, value: Value) {
        let slots = self
            .cmd(
                instance,
                vec![json!({"target": target, "name": "set_prop", "args": {"prop": prop, "value": value}})],
            )
            .await
            .unwrap();
        assert_eq!(slots[0]["ok"], json!(true), "{slots:?}");
    }

    /// A write (`set`, #43) of `prop` on `target`, sequence `seq`.
    pub async fn write(
        &mut self,
        instance: &str,
        target: &str,
        prop: &str,
        value: Value,
        seq: u64,
        is_final: bool,
    ) {
        self.send(&ClientMsg::Set {
            instance: instance.into(),
            target: target.into(),
            prop: prop.into(),
            value,
            seq,
            t: 1_000.0 + seq as f64,
            is_final,
        })
        .await;
    }

    /// The ack item of write `seq` of `key`, within `limit`.
    pub async fn ack_of(&mut self, key: &str, seq: u64, limit: Duration) -> AckItem {
        let key = key.to_string();
        self.wait(limit, move |m| match m {
            ServerMsg::Ack { items } => {
                items.iter().find(|i| i.key == key && i.seq == seq).cloned()
            }
            _ => None,
        })
        .await
    }

    /// `get_prop` through a command.
    pub async fn get(&mut self, instance: &str, target: &str, prop: &str) -> Value {
        let slots = self
            .cmd(
                instance,
                vec![json!({"target": target, "name": "get_prop", "args": {"prop": prop}})],
            )
            .await
            .unwrap();
        assert_eq!(slots[0]["ok"], json!(true), "{slots:?}");
        slots[0]["data"].clone()
    }

    /// Waits for an `instance` message of `name` matching `online`/`busy`.
    pub async fn instance_state(
        &mut self,
        name: &str,
        online: bool,
        busy: Option<bool>,
        limit: Duration,
    ) {
        let name = name.to_string();
        self.wait(limit, move |m| match m {
            ServerMsg::Instance {
                name: n,
                online: o,
                busy: b,
                ..
            } if *n == name && *o == online && busy.is_none_or(|want| want == *b) => Some(()),
            _ => None,
        })
        .await;
    }

    pub async fn close(mut self) {
        let _ = self.ws.close(None).await;
    }
}

/// The next client-protocol message within `limit` (`None`: nothing, or the
/// socket closed).
async fn next_msg(ws: &mut Socket, limit: Duration) -> Option<ServerMsg> {
    let deadline = Instant::now() + limit;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match tokio::time::timeout(left, ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                return Some(serde_json::from_str(text.as_str()).expect("a protocol message"));
            }
            Ok(Some(Ok(Message::Close(_)) | Err(_)) | None) | Err(_) => return None,
            Ok(Some(Ok(_))) => {}
        }
    }
}

/// A multi-threaded runtime for a test (blocking host calls do not starve
/// the hub).
pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap()
}
