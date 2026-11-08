//! A fake Presenter for the stage tab's tests (#90): the endpoints the hub
//! and the stage page use, shaped as Presenter answers them (its source,
//! `presenter-server/src/router/stage.rs`, `presenter-core/src/live.rs`):
//! `GET /stage` (a page), `GET /ui-pkg/*` (an asset), `GET /stage-displays`
//! (`[{code, name, description}]`), `GET /stage/layout` and
//! `POST /stage/layout {"code"}` (`{code, layout}`; 400 empty, 404 unknown),
//! `POST /stage/state` (a write the hub must never pass), and `/live/ws`,
//! whose every client gets `{"type": "stage_layout", "code"}` at each switch
//! and its own text back (`echo: …`, so a tunnel's both ways show; with
//! `?refuse` a 403 instead of the switch, with `?flood` 64 KiB frames as
//! fast as they go). Every
//! request is counted by method and path; [`FakePresenter::stop`] takes it
//! off the network (its sockets closed, its port refused) and
//! [`FakePresenter::start_on`] brings it back on the same port.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};
use tokio::sync::{broadcast, watch};

/// The layouts the fake lists (Presenter's own names, invented descriptions).
pub const LAYOUTS: [(&str, &str); 3] = [
    ("worship", "WORSHIP"),
    ("timer", "TIMER"),
    ("preach", "PREACH"),
];

/// The stage page the fake serves.
pub const PAGE: &str = "<!doctype html><title>fake stage</title><p>stage</p>";

#[derive(Clone)]
struct Shared {
    current: Arc<Mutex<String>>,
    seen: Arc<Mutex<Vec<String>>>,
    events: broadcast::Sender<String>,
    down: watch::Receiver<bool>,
    pushed: Arc<AtomicU64>,
}

/// A running fake Presenter.
pub struct FakePresenter {
    pub addr: SocketAddr,
    current: Arc<Mutex<String>>,
    seen: Arc<Mutex<Vec<String>>>,
    events: broadcast::Sender<String>,
    down: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
    /// The bytes a `?flood` socket wrote so far.
    pub pushed: Arc<AtomicU64>,
}

impl FakePresenter {
    /// A fake on an ephemeral port, `worship` on stage.
    pub async fn start() -> Self {
        Self::start_on(SocketAddr::from(([127, 0, 0, 1], 0)), "worship").await
    }

    /// A fake on `addr` with `current` on stage.
    pub async fn start_on(addr: SocketAddr, current: &str) -> Self {
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (down, down_rx) = watch::channel(false);
        let (events, _) = broadcast::channel(16);
        let shared = Shared {
            current: Arc::new(Mutex::new(current.to_string())),
            seen: Arc::default(),
            events: events.clone(),
            down: down_rx.clone(),
            pushed: Arc::default(),
        };
        let app = Router::new()
            .route(
                "/stage",
                get(|| async { ([("content-type", "text/html")], PAGE) }),
            )
            .route(
                "/ui-pkg/{*path}",
                get(|| async { ([("content-type", "text/javascript")], "/* app */") }),
            )
            .route("/stage-displays", get(displays))
            .route("/stage/layout", get(layout).post(set_layout))
            .route("/stage/state", post(|| async { StatusCode::NO_CONTENT }))
            .route("/live/ws", get(live))
            .layer(axum::middleware::from_fn_with_state(shared.clone(), count))
            .with_state(shared.clone());
        let mut stop = down_rx;
        let task = tokio::spawn(async move {
            let shutdown = async move {
                let _ = stop.wait_for(|down| *down).await;
            };
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(shutdown)
                .await;
        });
        Self {
            addr,
            current: shared.current,
            seen: shared.seen,
            events,
            down,
            task,
            pushed: shared.pushed,
        }
    }

    /// `http://<addr>`: its `[presenter] url`.
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// The layout on stage.
    pub fn current(&self) -> String {
        self.current.lock().unwrap().clone()
    }

    /// Every request so far as `<METHOD> <path>`.
    pub fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }

    /// The operator switches in Presenter itself.
    pub fn switch(&self, code: &str) {
        *self.current.lock().unwrap() = code.to_string();
        let _ = self
            .events
            .send(json!({"type": "stage_layout", "code": code}).to_string());
    }

    /// Off the network: its sockets closed, its listener gone.
    pub async fn stop(self) -> SocketAddr {
        self.down.send_replace(true);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), self.task).await;
        self.addr
    }
}

async fn count(
    State(shared): State<Shared>,
    uri: Uri,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let line = format!("{} {}", request.method(), uri.path());
    shared.seen.lock().unwrap().push(line);
    next.run(request).await
}

fn layout_json(code: &str) -> Option<Value> {
    LAYOUTS.iter().find(|(c, _)| *c == code).map(|(c, name)| {
        json!({"code": c, "name": name, "description": format!("{name} on the stage display")})
    })
}

async fn displays() -> impl IntoResponse {
    let list: Vec<Value> = LAYOUTS.iter().filter_map(|(c, _)| layout_json(c)).collect();
    axum::Json(list)
}

async fn layout(State(shared): State<Shared>) -> impl IntoResponse {
    let code = shared.current.lock().unwrap().clone();
    axum::Json(json!({"code": code, "layout": layout_json(&code)}))
}

async fn set_layout(State(shared): State<Shared>, axum::Json(body): axum::Json<Value>) -> Response {
    let code = body["code"].as_str().unwrap_or("").trim().to_string();
    if code.is_empty() {
        return (StatusCode::BAD_REQUEST, "code cannot be empty").into_response();
    }
    let Some(found) = layout_json(&code) else {
        return (StatusCode::NOT_FOUND, "unknown layout").into_response();
    };
    *shared.current.lock().unwrap() = code.clone();
    let _ = shared
        .events
        .send(json!({"type": "stage_layout", "code": code}).to_string());
    axum::Json(json!({"code": code, "layout": found})).into_response()
}

async fn live(State(shared): State<Shared>, uri: Uri, ws: WebSocketUpgrade) -> Response {
    // `?refuse` (tests): Presenter answers the upgrade without switching.
    if uri.query().is_some_and(|q| q.contains("refuse")) {
        return (StatusCode::FORBIDDEN, "refused by the fake").into_response();
    }
    // `?flood` (tests): 64 KiB frames to the page as fast as they go.
    if uri.query().is_some_and(|q| q.contains("flood")) {
        return ws.on_upgrade(move |socket| flood(socket, shared.pushed));
    }
    ws.on_upgrade(move |socket| serve_socket(socket, shared))
}

async fn flood(mut socket: WebSocket, pushed: Arc<AtomicU64>) {
    let frame = vec![7_u8; 65_536];
    while socket
        .send(Message::Binary(frame.clone().into()))
        .await
        .is_ok()
    {
        pushed.fetch_add(frame.len() as u64, Ordering::Relaxed);
    }
}

async fn serve_socket(mut socket: WebSocket, shared: Shared) {
    let mut events = shared.events.subscribe();
    let mut down = shared.down.clone();
    loop {
        tokio::select! {
            _ = async { let _ = down.wait_for(|down| *down).await; } => {
                let _ = socket.send(Message::Close(None)).await;
                return;
            }
            event = events.recv() => match event {
                Ok(text) => {
                    if socket.send(Message::Text(text.into())).await.is_err() {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return,
            },
            frame = socket.recv() => match frame {
                Some(Ok(Message::Text(text))) => {
                    let echo = format!("echo: {}", text.as_str());
                    if socket.send(Message::Text(echo.into())).await.is_err() {
                        return;
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            },
        }
    }
}
