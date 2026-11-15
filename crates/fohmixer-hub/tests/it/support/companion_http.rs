//! A fake of Companion's web server for the scene poll (#207): a tokio
//! listener on 127.0.0.1 that answers `GET /api/custom-variable/<name>/value`
//! with the variable's plain value (any other path 404, as Companion's
//! "Not found"), keeps a connection alive as Companion does, and can go
//! down (the listener and every open connection closed: a refused connect)
//! and come up again on the same port.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

struct State {
    variable: String,
    value: String,
    reads: u64,
}

pub struct FakeCompanionHttp {
    pub addr: SocketAddr,
    state: Arc<Mutex<State>>,
    tasks: Option<JoinSet<()>>,
}

impl FakeCompanionHttp {
    /// Serves `variable` = `value`.
    pub async fn start(variable: &str, value: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(State {
            variable: variable.into(),
            value: value.into(),
            reads: 0,
        }));
        let mut fake = Self {
            addr,
            state,
            tasks: None,
        };
        fake.serve(listener);
        fake
    }

    fn serve(&mut self, listener: TcpListener) {
        let mut tasks = JoinSet::new();
        let state = Arc::clone(&self.state);
        let conns = Arc::new(Mutex::new(JoinSet::new()));
        let held = Arc::clone(&conns);
        tasks.spawn(async move {
            // The connections live in a set this task owns: aborting the
            // task drops the set, which aborts every connection.
            let _held = held;
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    return;
                };
                let state = Arc::clone(&state);
                conns.lock().unwrap().spawn(answer(stream, state));
            }
        });
        self.tasks = Some(tasks);
    }

    /// The variable's new value.
    pub fn set(&self, value: &str) {
        self.state.lock().unwrap().value = value.into();
    }

    /// The reads answered so far.
    pub fn reads(&self) -> u64 {
        self.state.lock().unwrap().reads
    }

    /// Down: the listener and every connection closed.
    pub fn down(&mut self) {
        self.tasks = None;
    }

    /// Up again on the same port.
    pub async fn up(&mut self) {
        let listener = TcpListener::bind(self.addr).await.unwrap();
        self.serve(listener);
    }
}

/// One connection: requests until the client closes it.
async fn answer(mut stream: TcpStream, state: Arc<Mutex<State>>) {
    let mut buf = Vec::new();
    loop {
        let head_end = loop {
            if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break end + 4;
            }
            let mut chunk = [0u8; 1024];
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => return,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        buf.drain(..head_end);
        let path = head.split_whitespace().nth(1).unwrap_or("").to_string();
        let (status, body) = {
            let mut state = state.lock().unwrap();
            if path == format!("/api/custom-variable/{}/value", state.variable) {
                state.reads += 1;
                ("200 OK", state.value.clone())
            } else {
                ("404 Not Found", "Not found".to_string())
            }
        };
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{body}",
            body.len()
        );
        if stream.write_all(response.as_bytes()).await.is_err() {
            return;
        }
    }
}
