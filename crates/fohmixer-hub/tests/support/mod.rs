//! Test support for the hub's integration tests: real `sim/host.py`
//! processes (the real FohMixer script on SimLive, S2), an in-process hub on
//! an ephemeral port, and a WebSocket client of the client protocol.
//!
//! A host is only ever asked to stop (SIGTERM, then a bounded wait): spec
//! I7, nothing is force-ended.
#![allow(dead_code)] // every test binary uses its own part of this module

use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use fohmixer_hub::config::{Config, InstanceCfg};
use fohmixer_proto::client::{ClientMsg, HubStatus, LiveCommand, ServerMsg, ValueItem};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

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

/// A `sim/host.py` process.
pub struct Host {
    child: Child,
    stdin: ChildStdin,
    lines: mpsc::Receiver<String>,
    pub port: u16,
    pub instance: String,
    _logs: tempfile::TempDir,
}

impl Host {
    /// A host of `instance` on a port of its own.
    pub fn start(instance: &str) -> Self {
        Self::start_with(instance, 0, 0.0)
    }

    /// A host on `port` (0: the OS picks), its meters moving `meters_hz`
    /// times a second (0: still).
    pub fn start_with(instance: &str, port: u16, meters_hz: f64) -> Self {
        let python = std::env::var("FOHMIXER_PYTHON").unwrap_or_else(|_| "python3".to_string());
        let logs = tempfile::tempdir().unwrap();
        let root = repo();
        let mut child = Command::new(python)
            .arg(root.join("sim").join("host.py"))
            .args(["--port", &port.to_string(), "--instance", instance])
            .arg("--site")
            .arg(root.join("sim").join("fixtures").join("test-site.json"))
            .args(["--meters-hz", &meters_hz.to_string()])
            .arg("--log-dir")
            .arg(logs.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn python3 sim/host.py");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let ready = lines
            .recv_timeout(Duration::from_secs(15))
            .expect("the host prints READY");
        let port = ready
            .strip_prefix("READY ")
            .and_then(|p| p.trim().parse().ok())
            .unwrap_or_else(|| panic!("not a READY line: {ready}"));
        Self {
            child,
            stdin,
            lines,
            port,
            instance: instance.to_string(),
            _logs: logs,
        }
    }

    /// The hub's config entry of this host.
    pub fn cfg(&self) -> InstanceCfg {
        InstanceCfg {
            name: self.instance.clone(),
            port: self.port,
        }
    }

    fn line(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Blocks the host's Live main thread for `ms`.
    pub fn stall(&mut self, ms: u64) {
        self.line(&format!("stall {ms}"));
    }

    fn answer(&mut self, line: &str) -> String {
        self.line(line);
        self.lines
            .recv_timeout(Duration::from_secs(5))
            .expect("the host answers")
    }

    /// Renames every track named `old` (Live's listeners fire); the count.
    pub fn rename(&mut self, old: &str, new: &str) -> usize {
        let answer = self.answer(&format!("rename \"{old}\" \"{new}\""));
        answer
            .strip_prefix("RENAMED ")
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("not a RENAMED line: {answer}"))
    }

    /// The Live listeners on `prop` of the object at `path` (-1: none there).
    pub fn listeners(&mut self, prop: &str, path: &str) -> i64 {
        let answer = self.answer(&format!("listeners {prop} {path}"));
        answer
            .strip_prefix("LISTENERS ")
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("not a LISTENERS line: {answer}"))
    }

    /// Asks the host to stop (SIGTERM) and waits for it (bounded).
    pub fn stop(mut self) -> ExitStatus {
        self.request_stop()
    }

    fn request_stop(&mut self) -> ExitStatus {
        if let Ok(Some(status)) = self.child.try_wait() {
            return status;
        }
        let sent = Command::new("kill")
            .args(["-s", "TERM", &self.child.id().to_string()])
            .status()
            .expect("run kill");
        assert!(sent.success(), "SIGTERM not sent");
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "the host did not stop within 10 s"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = Command::new("kill")
                .args(["-s", "TERM", &self.child.id().to_string()])
                .status();
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if let Ok(Some(_)) = self.child.try_wait() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// A hub running in this process on 127.0.0.1, port 0.
pub struct TestHub {
    pub addr: SocketAddr,
    pub dir: PathBuf,
    pub token: String,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<anyhow::Result<()>>>,
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

    /// The client WebSocket URL with the token and protocol 1.
    pub fn ws_url(&self) -> String {
        format!("ws://{}/ws?token={}&proto=1", self.addr, self.token)
    }

    /// A connected client (past its hello).
    pub async fn client(&self) -> Client {
        Client::connect(&self.ws_url()).await
    }

    /// `GET path` with the token: the status code and the body.
    pub async fn get(&self, path: &str) -> (u16, Value) {
        http(self.addr, "GET", path, Some(&self.token), None).await
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
