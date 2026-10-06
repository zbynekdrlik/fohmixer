//! A scripted fake Bitfocus Companion for the Stream Deck's tests (#52): the
//! Satellite API on a tokio listener, written as Companion 5.0.7 writes it
//! (`BEGIN` and `CAPS` first, every line ending with a space before its
//! `\n`, `ADD-DEVICE OK` followed by `BRIGHTNESS` and 32 `KEY-STATE`s, a
//! press answered `OK` and then the key's new state). Connections are
//! numbered from 1; every line the hub sends is kept with its connection and
//! when it came. Host-free: these tests also run on Windows.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine as _;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

/// How the fake answers.
#[derive(Debug, Clone)]
pub struct Script {
    /// Its `ApiVersion`.
    pub api: String,
    /// `ADD-DEVICE` refused with this message.
    pub refuse_add: Option<String>,
    /// `PING` and `KEY-PRESS` answered (false: silent after the handshake).
    pub answers: bool,
}

impl Script {
    /// Companion 5.0.7 as it answers.
    pub fn companion() -> Self {
        Self {
            api: "1.12.0".into(),
            refuse_add: None,
            answers: true,
        }
    }
}

/// A line the hub sent: its connection (from 1), when it came, its text.
#[derive(Debug, Clone)]
pub struct Got {
    pub conn: usize,
    pub at: Instant,
    pub line: String,
}

/// The fake: its port, what it got, how it answers.
#[derive(Clone)]
pub struct FakeCompanion {
    pub port: u16,
    got: Arc<Mutex<Vec<Got>>>,
    script: Arc<Mutex<Script>>,
    /// The latest connection's way out: a line, or `None` to close it.
    current: Arc<Mutex<Option<mpsc::UnboundedSender<Option<String>>>>>,
    /// New connections closed at once (Companion away).
    refusing: Arc<AtomicBool>,
}

impl FakeCompanion {
    /// A fake on a free port, answering by `script`.
    pub async fn start(script: Script) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let fake = Self {
            port: listener.local_addr().unwrap().port(),
            got: Arc::default(),
            script: Arc::new(Mutex::new(script)),
            current: Arc::default(),
            refusing: Arc::new(AtomicBool::new(false)),
        };
        let accepting = fake.clone();
        tokio::spawn(async move {
            let mut n = 0;
            while let Ok((stream, _)) = listener.accept().await {
                if accepting.refusing.load(Ordering::SeqCst) {
                    drop(stream);
                    continue;
                }
                n += 1;
                tokio::spawn(accepting.clone().serve(n, stream));
            }
        });
        fake
    }

    /// How the next connections answer.
    pub fn set_script(&self, script: Script) {
        *self.script.lock().unwrap() = script;
    }

    /// Companion away (`on`): the open connection closed, new ones closed at
    /// once; or back.
    pub fn refuse(&self, on: bool) {
        self.refusing.store(on, Ordering::SeqCst);
        if on {
            self.close();
        }
    }

    /// A line to the latest connection (Companion's trailing space added).
    pub fn send(&self, line: &str) {
        if let Some(tx) = &*self.current.lock().unwrap() {
            let _ = tx.send(Some(format!("{line} \n")));
        }
    }

    /// Closes the latest connection.
    pub fn close(&self) {
        if let Some(tx) = self.current.lock().unwrap().take() {
            let _ = tx.send(None);
        }
    }

    /// Every line the hub sent so far.
    pub fn got(&self) -> Vec<Got> {
        self.got.lock().unwrap().clone()
    }

    /// The lines of connection `conn`.
    pub fn lines_of(&self, conn: usize) -> Vec<String> {
        self.got()
            .into_iter()
            .filter(|g| g.conn == conn)
            .map(|g| g.line)
            .collect()
    }

    /// Waits up to `limit` until `check` holds on the lines; them.
    pub async fn until(
        &self,
        limit: Duration,
        what: &str,
        check: impl Fn(&[Got]) -> bool,
    ) -> Vec<Got> {
        let deadline = Instant::now() + limit;
        loop {
            let got = self.got();
            if check(&got) {
                return got;
            }
            assert!(
                Instant::now() < deadline,
                "the fake never got {what}: {got:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn serve(self, n: usize, stream: TcpStream) {
        let (read, mut write) = stream.into_split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Option<String>>();
        *self.current.lock().unwrap() = Some(tx);
        let script = self.script.lock().unwrap().clone();
        let hello = format!(
            "BEGIN CompanionVersion=\"5.0.7+fake\" ApiVersion=\"{}\" \n\
             CAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS=\"rgb,png,webp\" \n",
            script.api
        );
        if write.write_all(hello.as_bytes()).await.is_err() {
            return;
        }
        let mut lines = BufReader::new(read).lines();
        loop {
            tokio::select! {
                line = lines.next_line() => {
                    let Ok(Some(line)) = line else { return };
                    self.got.lock().unwrap().push(Got { conn: n, at: Instant::now(), line: line.clone() });
                    for reply in answer(&script, &line) {
                        if write.write_all(reply.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                }
                out = rx.recv() => match out {
                    Some(Some(text)) => {
                        if write.write_all(text.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                    Some(None) | None => return,
                },
            }
        }
    }
}

/// The `DEVICEID` of a line.
pub fn device_of(line: &str) -> String {
    line.split("DEVICEID=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .unwrap_or_default()
        .to_string()
}

/// The image the fake draws for `key`: a data URL naming it and its state.
pub fn image(key: u32, pressed: bool) -> String {
    let what = format!("key {key} {}", if pressed { "down" } else { "up" });
    format!(
        "data:image/webp;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(what)
    )
}

/// A `KEY-STATE` line of `key` as Companion 5.0.7 writes it.
pub fn key_state(device: &str, key: u32, pressed: bool) -> String {
    format!(
        "KEY-STATE DEVICEID=\"{device}\" KEY={key} LOCATION=\"1/{}/{}\" PRESSED={} TYPE=\"BUTTON\" \
         BITMAP=\"{}\" COLOR=\"#000000\" TEXTCOLOR=\"#ffffff\" \n",
        key / 8,
        key % 8,
        u8::from(pressed),
        image(key, pressed)
    )
}

/// The fake's answer to one line of the hub.
fn answer(script: &Script, line: &str) -> Vec<String> {
    let device = device_of(line);
    match line.split(' ').next().unwrap_or_default() {
        "ADD-DEVICE" => match &script.refuse_add {
            Some(message) => vec![format!(
                "ADD-DEVICE ERROR DEVICEID=\"{device}\" MESSAGE=\"{message}\" \n"
            )],
            None => {
                let mut out = vec![
                    format!("ADD-DEVICE OK DEVICEID=\"{device}\" \n"),
                    format!("BRIGHTNESS DEVICEID=\"{device}\" VALUE=100 \n"),
                ];
                out.extend((0..32).map(|key| key_state(&device, key, false)));
                out
            }
        },
        "PING" if script.answers => {
            vec![format!(
                "PONG {} \n",
                line.trim_start_matches("PING ").trim()
            )]
        }
        "KEY-PRESS" if script.answers => {
            let key: u32 = line
                .split("KEY=")
                .nth(1)
                .and_then(|rest| rest.split(' ').next())
                .and_then(|k| k.parse().ok())
                .unwrap_or(0);
            let pressed = line.contains("PRESSED=1");
            vec![
                format!("KEY-PRESS OK DEVICEID=\"{device}\" \n"),
                key_state(&device, key, pressed),
            ]
        }
        "REMOVE-DEVICE" => vec![format!("REMOVE-DEVICE OK DEVICEID=\"{device}\" \n")],
        _ => Vec::new(),
    }
}
