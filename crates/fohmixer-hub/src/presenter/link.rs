//! The hub's link to Presenter (#90, the stage tab): it follows Presenter's
//! live socket and switches the stage layout for a page.
//!
//! - **A session** ([`session`]): the hub opens Presenter's live socket
//!   ([`socket_url`]: `?surface=fohmixer&preview=1`, so Presenter never
//!   counts it as a stage display), then reads `GET /stage-displays` and
//!   `GET /stage/layout` (after the socket is open, so no switch between
//!   the read and the socket is missed), tells the router the list and the
//!   current layout, and follows every `stage_layout` event
//!   ([`stage_layout_event`]). It pings every [`PING_EVERY`]; a socket
//!   silent for [`SILENCE`] (a Presenter gone without a close) ends it.
//! - **Up and down** ([`LinkState::note`], AbleSet's `link_step`): up at a
//!   session's start, down only after [`ableset::link::DOWN_AFTER`] failures
//!   in a row (a session's end and the first try after it), so a restarted
//!   Presenter back within [`RETRY`] never shows the offline panel. A change
//!   is a log line and a `presenter_link` record (warn class) and goes to
//!   every page through the router.
//! - **A switch** ([`Api::switch`], [`switches`]): the hub's own
//!   `POST /stage/layout {"code"}` for a page's held key, its answer read
//!   by [`switch_answer`], the result back to the router.
//!
//! Every request is bounded by [`READ_TIMEOUT`] and its body by
//! [`MAX_BODY`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::{Method, Request, header};
use bytes::Bytes;
use fohmixer_proto::client::StageLayout;
use futures_util::{SinkExt as _, StreamExt as _};
use http_body_util::{BodyExt as _, Full, Limited};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use super::Presenter;
use crate::ableset::link::{Link, link_step};

/// How long the hub waits before it tries Presenter again.
pub const RETRY: Duration = Duration::from_secs(2);

/// The bound of one request to Presenter, the socket's opening included.
pub const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// How often the hub pings Presenter's socket.
pub const PING_EVERY: Duration = Duration::from_secs(5);

/// A socket silent this long (no frame, no pong) ends its session.
pub const SILENCE: Duration = Duration::from_secs(15);

/// The largest answer read (the list of layouts is a few kB).
pub const MAX_BODY: usize = 1_048_576;

/// The longest layout code a page's switch may name (Presenter's are a few
/// characters; anything longer is no layout of its).
pub const MAX_CODE: usize = 64;

/// What the link tells the router.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkEvent {
    /// Presenter came up or went down.
    Online(bool),
    /// A session's reads: the layouts in Presenter's order and the one on
    /// stage.
    Listed {
        layouts: Vec<StageLayout>,
        current: String,
    },
    /// Presenter switched the stage layout (anyone's switch).
    Layout { code: String },
}

/// What happened to the link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum News {
    /// A session started (its socket open, its reads done).
    Up,
    /// A try failed, or a session ended, and why.
    Failed(String),
}

/// A change of the link, for its line and record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Up { failures: u32 },
    Down { failures: u32, error: String },
}

impl Change {
    /// Whether Presenter is up now.
    pub fn up(&self) -> bool {
        matches!(self, Self::Up { .. })
    }

    /// The `presenter_link` record (warn class: `events::is_warn`).
    pub fn fields(&self) -> Value {
        match self {
            Self::Up { failures } => json!({"up": true, "failures": failures}),
            Self::Down { failures, error } => {
                json!({"up": false, "failures": failures, "error": error})
            }
        }
    }
}

/// The link as `/api/status` reads it.
#[derive(Debug, Default)]
pub struct LinkState {
    pub link: Link,
    /// Why the last try failed or session ended (none after a start).
    pub error: Option<String>,
    /// The layouts Presenter listed at the last session's start.
    pub layouts: usize,
    /// The layout on stage, as last read or told.
    pub current: Option<String>,
}

impl LinkState {
    /// The link after `news`, and its change, if any (pure).
    pub fn note(&mut self, news: News) -> Option<Change> {
        let (ok, error) = match news {
            News::Up => (true, None),
            News::Failed(why) => (false, Some(why)),
        };
        let (link, note) = link_step(self.link, ok);
        self.link = link;
        self.error = error;
        note.map(|note| match note {
            crate::ableset::link::Note::Up { failures } => Change::Up { failures },
            crate::ableset::link::Note::Down { failures } => Change::Down {
                failures,
                error: self.error.clone().unwrap_or_default(),
            },
        })
    }
}

/// `/live/ws` of Presenter at `base` as the hub opens it (pure).
pub fn socket_url(base: &str) -> String {
    let rest = base.strip_prefix("http://").unwrap_or(base);
    format!("ws://{rest}/live/ws?surface=fohmixer&preview=1")
}

/// The code of a `stage_layout` event in a frame of Presenter's socket
/// (`{"type": "stage_layout", "code": …}`); none for any other frame.
pub fn stage_layout_event(text: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Event {
        #[serde(rename = "type")]
        kind: String,
        code: Option<String>,
    }
    let event: Event = serde_json::from_str(text).ok()?;
    (event.kind == "stage_layout").then_some(event.code?)
}

/// The layouts of a `GET /stage-displays` answer (pure).
pub fn listed(status: u16, body: &[u8]) -> Result<Vec<StageLayout>, String> {
    if status != 200 {
        return Err(format!("GET /stage-displays answers {status}"));
    }
    serde_json::from_slice(body).map_err(|e| format!("GET /stage-displays: {e}"))
}

/// The code of a `GET /stage/layout` or `POST /stage/layout` answer
/// (`{"code", "layout"}`), 200 only (pure).
pub fn layout_code(status: u16, body: &[u8], what: &str) -> Result<String, String> {
    #[derive(Deserialize)]
    struct Answer {
        code: String,
    }
    if status != 200 {
        return Err(format!("{what} answers {status}"));
    }
    serde_json::from_slice::<Answer>(body)
        .map(|answer| answer.code)
        .map_err(|e| format!("{what}: {e}"))
}

/// A switch's result from Presenter's answer to `POST /stage/layout` of
/// `code` (pure): the code on stage now, or why not in words.
pub fn switch_answer(status: u16, body: &[u8], code: &str) -> Result<String, String> {
    match status {
        400 => Err(format!("Presenter refused the layout {code:?}")),
        404 => Err(format!("Presenter does not list {code:?}")),
        _ => layout_code(status, body, "POST /stage/layout"),
    }
}

/// The hub's own client of Presenter's server.
#[derive(Clone)]
pub struct Api {
    client: Client<HttpConnector, Full<Bytes>>,
    base: String,
}

impl Api {
    /// A client of Presenter at `base` (`http://<address>[:port]`).
    pub fn new(base: &str) -> Self {
        let mut connector = HttpConnector::new();
        connector.set_connect_timeout(Some(super::CONNECT_TIMEOUT));
        let client = Client::builder(TokioExecutor::new()).build(connector);
        Self {
            client,
            base: base.to_string(),
        }
    }

    /// One request within [`READ_TIMEOUT`], its body read whole (at most
    /// [`MAX_BODY`]): the status and the body, or why none came.
    async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<(u16, Bytes), String> {
        let url = format!("{}{path}", self.base);
        let mut request = Request::builder()
            .method(method.clone())
            .uri(&url)
            .header(header::ACCEPT, "application/json")
            .header(
                header::USER_AGENT,
                format!("fohmixer-hub/{}", fohmixer_proto::VERSION),
            );
        let bytes = match body {
            Some(body) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Bytes::from(body.to_string())
            }
            None => Bytes::new(),
        };
        let request = request
            .body(Full::new(bytes))
            .map_err(|e| format!("{method} {path}: {e}"))?;
        let exchange = async {
            let answer = self.client.request(request).await;
            let answer = answer.map_err(|e| crate::ableset::proxy::chain(&e))?;
            let status = answer.status().as_u16();
            let body = Limited::new(answer.into_body(), MAX_BODY).collect().await;
            let body = body.map_err(|e| format!("reading the answer: {e}"))?;
            Ok::<(u16, Bytes), String>((status, body.to_bytes()))
        };
        match tokio::time::timeout(READ_TIMEOUT, exchange).await {
            Ok(Ok(answer)) => Ok(answer),
            Ok(Err(why)) => Err(format!("{method} {path}: {why}")),
            Err(_) => Err(format!(
                "{method} {path}: no answer within {} ms",
                READ_TIMEOUT.as_millis()
            )),
        }
    }

    /// Presenter's stage layouts.
    pub async fn layouts(&self) -> Result<Vec<StageLayout>, String> {
        let (status, body) = self.call(Method::GET, "/stage-displays", None).await?;
        listed(status, &body)
    }

    /// The layout on stage.
    pub async fn current(&self) -> Result<String, String> {
        let (status, body) = self.call(Method::GET, "/stage/layout", None).await?;
        layout_code(status, &body, "GET /stage/layout")
    }

    /// Switches the stage to `code`: the code on stage now, or why not.
    pub async fn switch(&self, code: &str) -> Result<String, String> {
        let body = json!({ "code": code });
        let (status, answer) = self
            .call(Method::POST, "/stage/layout", Some(&body))
            .await?;
        switch_answer(status, &answer, code)
    }
}

/// One session with Presenter (module doc): it returns why it ended (or
/// why it never started).
pub async fn session<F>(site: &Presenter, api: &Api, tell: &F) -> String
where
    F: Fn(LinkEvent),
{
    let url = socket_url(&site.base);
    let socket = tokio::time::timeout(READ_TIMEOUT, tokio_tungstenite::connect_async(url.as_str()));
    let mut socket = match socket.await {
        Ok(Ok((socket, _))) => socket,
        Ok(Err(e)) => return format!("the live socket: {e}"),
        Err(_) => {
            return format!(
                "the live socket did not open within {} ms",
                READ_TIMEOUT.as_millis()
            );
        }
    };
    let (layouts, current) = match tokio::try_join!(api.layouts(), api.current()) {
        Ok(read) => read,
        Err(why) => return why,
    };
    tracing::info!(layouts = layouts.len(), current = %current, "presenter: stage layouts read");
    site.seen(Some(layouts.len()), Some(&current));
    tell(LinkEvent::Listed { layouts, current });
    if let Some(up) = site.noted(News::Up) {
        tell(LinkEvent::Online(up));
    }
    // Its first tick is at once: one ping as the session starts, then one
    // every PING_EVERY.
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The last frame of any kind (a pong too): the silence counts from it,
    // never from the last ping, which goes out whatever the far end does.
    let mut heard = tokio::time::Instant::now();
    loop {
        tokio::select! {
            () = tokio::time::sleep_until(heard + SILENCE) => {
                return format!("the live socket was silent for {} ms", SILENCE.as_millis());
            }
            frame = socket.next() => {
                heard = tokio::time::Instant::now();
                match frame {
                    None => return "Presenter closed its live socket".to_string(),
                    Some(Err(e)) => return format!("the live socket: {e}"),
                    Some(Ok(Message::Close(frame))) => {
                        return format!("Presenter closed its live socket: {frame:?}");
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Some(code) = stage_layout_event(&text) {
                            tracing::info!(code = ?code, "presenter: the stage layout changed");
                            site.seen(None, Some(&code));
                            tell(LinkEvent::Layout { code });
                        }
                    }
                    Some(Ok(_)) => {}
                }
            }
            _ = ping.tick() => {
                if let Err(e) = socket.send(Message::Ping(Bytes::new())).await {
                    return format!("the live socket's ping: {e}");
                }
            }
        }
    }
}

/// Follows Presenter for as long as the hub runs: a session, then
/// [`RETRY`], then the next, each end noted ([`Presenter::noted`]) and its
/// change told.
pub async fn follow<F>(site: Arc<Presenter>, api: Api, tell: F)
where
    F: Fn(LinkEvent),
{
    loop {
        let why = session(&site, &api, &tell).await;
        if let Some(what) = ended_news(site.status().online) {
            tracing::info!(why = %why, "presenter: {what}");
        } else {
            tracing::debug!(why = %why, "presenter: the try failed");
        }
        if let Some(up) = site.noted(News::Failed(why)) {
            tell(LinkEvent::Online(up));
        }
        tokio::time::sleep(RETRY).await;
    }
}

/// Whether a session's end is news for the hub's log, and its words: a
/// session of a link that was up (pure); a try that failed while Presenter
/// is down (every [`RETRY`]) is none, a debug line.
pub fn ended_news(online: Option<bool>) -> Option<&'static str> {
    (online == Some(true)).then_some("the session ended")
}

/// A page's switch the router hands to [`switches`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchRequest {
    pub client: u64,
    pub seq: u64,
    pub code: String,
    /// The layout on stage when the page asked (its record's `from`).
    pub from: Option<String>,
}

/// A switch's outcome: the request, the code on stage or why not, and how
/// long Presenter took (ms).
pub type Switched = (SwitchRequest, Result<String, String>, u64);

/// Carries every switch the router hands over to Presenter, each in a task
/// of its own (two tablets never wait for each other), its outcome to
/// `done`.
pub async fn switches<F>(api: Api, mut requests: mpsc::UnboundedReceiver<SwitchRequest>, done: F)
where
    F: Fn(Switched) + Send + Sync + 'static,
{
    let done = Arc::new(done);
    while let Some(request) = requests.recv().await {
        let (api, done) = (api.clone(), Arc::clone(&done));
        tokio::spawn(async move {
            let started = Instant::now();
            tracing::info!(client = request.client, code = %request.code, "presenter: switching the stage layout");
            let result = api.switch(&request.code).await;
            let ms = started.elapsed().as_millis() as u64;
            done((request, result, ms));
        });
    }
}

#[cfg(test)]
mod tests;
