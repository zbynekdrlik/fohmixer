//! The MBC hub's instance link server (#184, spec §6.1; plan Task B4).
//!
//! A TCP listener on the `[link] listen` address (a given LAN address,
//! never every address: `config::link`). Per connection:
//!
//! - The WebSocket upgrade is checked before it is answered ([`admit`]): a
//!   missing or bad link token, or no link secret stored, is a 401; a
//!   second link while one is open is a 409 (`busy`). Each refusal is one
//!   warn line and one `instance_link` record, rationed ([`REFUSED_LINES`]
//!   per `ration::GAP`).
//! - An admitted link gets a script connection of its own: a WebSocket to
//!   `ws://127.0.0.1:<script port>` within `CONNECT_TIMEOUT`. The FohMixer
//!   script serves several clients, each with its own listeners and
//!   results (the plan's DECIDED: no request-id mapping), so a fader moved
//!   on either hub reaches both through Live's listeners. A script that
//!   does not answer closes the link with 1011 `script unreachable`; the
//!   dialling hub reads the instance offline.
//! - Then one loop relays: the script's text frames to the link verbatim,
//!   the link's text frames to the script verbatim, the link's pings to the
//!   script (the script counts a client silent for 1.5 s as gone: it hears
//!   the dialling hub's own keep-alive, which follows that hub's router),
//!   and each LUFS datagram of the tap as [`lufs_frame`]. A close of
//!   either side ends both (the script's close frame is passed on, so its
//!   reason reaches the dialling hub's log). A write that makes no progress
//!   for [`LINK_STALL`] ends the link.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use fohmixer_proto::client::LinkStatus;
use futures_util::{Sink, SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio::task::{JoinHandle, JoinSet};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

use super::{LinkRefusal, LinkSecret, link_claims_ok};
use crate::config::LinkCfg;
use crate::events::EventLog;
use crate::live::client::CONNECT_TIMEOUT;
use crate::ration::Ration;

/// A write to either side that makes no progress for this long ends the
/// link (the script's own `SEND_STALL_S`).
pub const LINK_STALL: Duration = Duration::from_secs(3);
/// The LUFS tap's capacity: datagrams a slow link may lag behind before the
/// oldest are dropped (counted).
pub const LUFS_TAP: usize = 256;
/// Refusal lines per `ration::GAP`.
pub const REFUSED_LINES: u32 = 8;
/// The close code of a link whose script does not answer.
pub const SCRIPT_UNREACHABLE: &str = "script unreachable";

/// The LUFS tap: every datagram the hub's LUFS listener reads.
pub type Tap = broadcast::Sender<Arc<[u8]>>;

/// A new LUFS tap.
pub fn tap() -> Tap {
    broadcast::channel(LUFS_TAP).0
}

/// What an upgrade gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission {
    Accept,
    /// 401 with its reason.
    Refuse(LinkRefusal),
    /// 409: a link is open.
    Busy,
}

/// The upgrade's answer: the token first (a caller without one learns
/// nothing, not even that a link is open), then one link at a time.
pub fn admit(open: bool, check: Result<(), LinkRefusal>) -> Admission {
    match check {
        Err(why) => Admission::Refuse(why),
        Ok(()) if open => Admission::Busy,
        Ok(()) => Admission::Accept,
    }
}

/// A LUFS datagram as a link frame: `{"event":"lufs","osc":"<base64>"}`.
pub fn lufs_frame(datagram: &[u8]) -> String {
    json!({"event": "lufs", "osc": STANDARD.encode(datagram)}).to_string()
}

/// The `instance_link` record of a step: what happened, with the peer and
/// the reason.
pub fn link_fields(what: &str, peer: &SocketAddr, why: Option<&str>) -> Value {
    let mut fields = json!({"what": what, "peer": peer.to_string()});
    if let Some(why) = why {
        fields["why"] = json!(why);
    }
    if what != "connected" {
        fields["warn"] = json!(true);
    }
    fields
}

/// The server's state, read by `/api/status` `link`.
#[derive(Debug)]
pub struct LinkState {
    open: AtomicBool,
    status: Mutex<LinkStatus>,
    lines: Mutex<Ration>,
}

impl LinkState {
    fn new(cfg: &LinkCfg) -> Self {
        Self {
            open: AtomicBool::new(false),
            status: Mutex::new(LinkStatus {
                listen: cfg.listen.to_string(),
                instance: cfg.instance.clone(),
                peer: None,
                connected_since_ms: None,
                links: 0,
                refusals: 0,
                last_refusal: None,
                lufs_dropped: 0,
            }),
            lines: Mutex::new(Ration::default()),
        }
    }

    fn with<T>(&self, f: impl FnOnce(&mut LinkStatus) -> T) -> T {
        f(&mut self.status.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// `/api/status` `link`.
    pub fn status(&self) -> LinkStatus {
        self.with(|s| s.clone())
    }
}

/// Binds `cfg.listen` and starts the server (inside a tokio runtime). Its
/// task holds every link's task: aborting it ends them all.
pub fn spawn(
    cfg: &LinkCfg,
    script_port: u16,
    secret: Option<LinkSecret>,
    tap: &Tap,
    events: EventLog,
) -> std::io::Result<(SocketAddr, Arc<LinkState>, JoinHandle<()>)> {
    let std_listener = std::net::TcpListener::bind(cfg.listen)?;
    std_listener.set_nonblocking(true)?;
    let listener = TcpListener::from_std(std_listener)?;
    let addr = listener.local_addr()?;
    let state = Arc::new(LinkState::new(cfg));
    // The address bound (a test's port 0 names its port).
    state.with(|s| s.listen = addr.to_string());
    if secret.is_none() {
        tracing::warn!(addr = %addr, "instance link: no link secret: every link is refused (`fohmixer-hub link set-secret`)");
    }
    tracing::info!(addr = %addr, instance = %cfg.instance, script_port, "instance link: listening");
    let server = Server {
        script_port,
        secret,
        tap: tap.clone(),
        events,
        state: Arc::clone(&state),
    };
    let task = tokio::spawn(server.accept(listener));
    Ok((addr, state, task))
}

#[derive(Clone)]
struct Server {
    script_port: u16,
    secret: Option<LinkSecret>,
    tap: Tap,
    events: EventLog,
    state: Arc<LinkState>,
}

/// Clears the open link when it ends (a panic in its task too).
struct OpenGuard(Arc<LinkState>);

impl Drop for OpenGuard {
    fn drop(&mut self) {
        self.0.with(|s| {
            s.peer = None;
            s.connected_since_ms = None;
        });
        self.0.open.store(false, Ordering::SeqCst);
    }
}

impl Server {
    async fn accept(self, listener: TcpListener) {
        let mut links = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => match accepted {
                    Ok((stream, peer)) => {
                        let _ = stream.set_nodelay(true);
                        links.spawn(self.clone().serve(stream, peer));
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "instance link: an accept failed");
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                },
                Some(_) = links.join_next(), if !links.is_empty() => {}
            }
        }
    }

    fn refused(&self, peer: &SocketAddr, why: &str) {
        self.state.with(|s| {
            s.refusals += 1;
            s.last_refusal = Some(format!("{why} ({peer})"));
        });
        let allowed = self
            .state
            .lines
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take(Instant::now(), REFUSED_LINES);
        if let Some(skipped) = allowed {
            tracing::warn!(peer = %peer, why, skipped, "instance link: refused");
            self.events
                .record("instance_link", link_fields("refused", peer, Some(why)));
        }
    }

    async fn serve(self, stream: TcpStream, peer: SocketAddr) {
        let now = crate::auth::now_secs();
        let mut admission = Admission::Busy;
        // The callback's error type is tungstenite's own `ErrorResponse`.
        #[allow(clippy::result_large_err)]
        let callback = |request: &Request, response: Response| {
            let header = request
                .headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok());
            let check = match &self.secret {
                Some(secret) => link_claims_ok(header, secret, now),
                None => Err(LinkRefusal::NoSecret),
            };
            // Taken atomically: two upgrades at once never both open.
            let open = check.is_ok()
                && self
                    .state
                    .open
                    .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err();
            admission = admit(open, check);
            match admission {
                Admission::Accept => Ok(response),
                Admission::Refuse(why) => Err(refusal(StatusCode::UNAUTHORIZED, why.text())),
                Admission::Busy => Err(refusal(StatusCode::CONFLICT, "busy")),
            }
        };
        let upgraded = tokio_tungstenite::accept_hdr_async(stream, callback).await;
        let link = match (admission, upgraded) {
            (Admission::Accept, Ok(link)) => link,
            (Admission::Accept, Err(e)) => {
                self.state.open.store(false, Ordering::SeqCst);
                tracing::warn!(peer = %peer, error = %e, "instance link: the upgrade failed");
                return;
            }
            (Admission::Refuse(why), _) => return self.refused(&peer, why.text()),
            (Admission::Busy, _) => return self.refused(&peer, "busy"),
        };
        let _guard = OpenGuard(Arc::clone(&self.state));
        self.state.with(|s| {
            s.links += 1;
            s.peer = Some(peer.to_string());
            s.connected_since_ms = crate::live::wall_ms().map(|ms| ms as u64);
        });
        tracing::info!(peer = %peer, "instance link: connected");
        self.events
            .record("instance_link", link_fields("connected", &peer, None));
        let why = self.relay(link).await;
        tracing::info!(peer = %peer, why, "instance link: dropped");
        self.events
            .record("instance_link", link_fields("dropped", &peer, Some(why)));
    }

    /// Relays one link until either side ends it; why it ended.
    async fn relay(&self, link: tokio_tungstenite::WebSocketStream<TcpStream>) -> &'static str {
        let url = format!("ws://127.0.0.1:{}", self.script_port);
        let (mut to_link, mut from_link) = link.split();
        let script = tokio::time::timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(url))
            .await
            .ok()
            .and_then(Result::ok);
        let Some((script, _)) = script else {
            let close = CloseFrame {
                code: CloseCode::Error,
                reason: SCRIPT_UNREACHABLE.into(),
            };
            let _ = send(&mut to_link, Message::Close(Some(close))).await;
            return SCRIPT_UNREACHABLE;
        };
        let (mut to_script, mut from_script) = script.split();
        let mut lufs = self.tap.subscribe();
        loop {
            tokio::select! {
                frame = from_script.next() => match frame {
                    Some(Ok(Message::Text(text))) => {
                        if !send(&mut to_link, Message::Text(text)).await {
                            return "the link stalled";
                        }
                    }
                    Some(Ok(Message::Close(frame))) => {
                        let _ = send(&mut to_link, Message::Close(frame)).await;
                        return "the script closed";
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => return "the script closed",
                },
                frame = from_link.next() => match frame {
                    Some(Ok(Message::Text(text))) => {
                        if !send(&mut to_script, Message::Text(text)).await {
                            return "the script stalled";
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if !send(&mut to_script, Message::Ping(data)).await {
                            return "the script stalled";
                        }
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                        let _ = send(&mut to_script, Message::Close(None)).await;
                        return "the link closed";
                    }
                    Some(Ok(_)) => {}
                },
                datagram = lufs.recv() => match datagram {
                    Ok(datagram) => {
                        if !send(&mut to_link, Message::Text(lufs_frame(&datagram).into())).await {
                            return "the link stalled";
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        self.state.with(|s| s.lufs_dropped += n);
                    }
                    // The server holds the tap's sender: closed only as the
                    // hub stops.
                    Err(broadcast::error::RecvError::Closed) => return "the hub stopped",
                },
            }
        }
    }
}

/// One write, ended after [`LINK_STALL`]: whether it went.
async fn send<S>(sink: &mut S, message: Message) -> bool
where
    S: Sink<Message, Error = WsError> + Unpin,
{
    matches!(
        tokio::time::timeout(LINK_STALL, sink.send(message)).await,
        Ok(Ok(()))
    )
}

/// An upgrade's refusal with its words as the body.
fn refusal(code: StatusCode, why: &str) -> ErrorResponse {
    let mut response = ErrorResponse::new(Some(why.to_string()));
    *response.status_mut() = code;
    response
}

#[cfg(test)]
mod tests;
