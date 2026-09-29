//! `/ws?token=<jwt>&proto=1` — a client's WebSocket (S3 design note §3; the
//! session shape of iemmixer's `mixer_ws.rs` @ 22372bc).
//!
//! The first message is `hello`; a client without a protocol, or with one
//! this hub does not serve, is closed with code 4001 and reloads. Then:
//! commands go straight to their instance (in order) and their results
//! come back as `result`; `sub`/`unsub`/`set_hub` go to the router; `ping`
//! is answered `pong` (the client's watchdog). A
//! writer task drains the client's outbox, so a slow client never holds up
//! anyone else; a client that takes longer than [`SEND_TIMEOUT`] to accept
//! a message is closed (it reconnects and resyncs).
//!
//! A socket's connect and disconnect lines name who opened it (#9, an
//! [`Opener`]): the peer, `lan` / `internet` as the Access check classifies
//! the upgrade, and the host of the page's `Origin`. A tab that holds a
//! token reconnects without a login line, and a bundle from before the
//! client reports sends none, so these two lines are the one trace every
//! client leaves in the hub log.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    extract::{ConnectInfo, Query, State},
    http::{HeaderMap, header},
    response::{IntoResponse, Response},
};
use fohmixer_proto::client::{
    CLOSE_RELOAD, ClientMsg, LiveCommand, MIN_CLIENT_PROTO, ServerMsg, UI_PROTO, proto_ok,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;

use crate::Hub;
use crate::access;
use crate::client_report;
use crate::live::subs::ClientId;
use crate::outbox::Outbox;
use crate::router::RouterMsg;

/// How long one message may take to reach a client.
pub const SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// The query of `/ws`.
#[derive(Debug, serde::Deserialize)]
pub struct WsQuery {
    pub token: Option<String>,
    pub proto: Option<u32>,
}

/// Who opened a socket, as its connect and disconnect lines name it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opener {
    /// The peer's address: the client on the LAN, cloudflared on the PC
    /// through the tunnel.
    pub peer: String,
    /// `lan` / `internet`: the Access check's class of the upgrade, named
    /// as a client report names it ([`client_report::source`]).
    pub source: &'static str,
    /// The host (`host[:port]`) of the page's `Origin`: the name or address
    /// the page was opened by; `-` without an `Origin` (not a browser).
    pub origin: String,
}

/// The [`Opener`] of an upgrade from `peer` with `headers`: the Access
/// check's own classification (no second classifier), and the `Origin`
/// host [`client_report::clean`]ed like a report's field (control
/// characters out, [`client_report::WORD_MAX_CHARS`]).
pub fn opener(peer: SocketAddr, headers: &HeaderMap) -> Opener {
    Opener {
        peer: peer.ip().to_string(),
        source: client_report::source(access::classify(Some(peer), headers)),
        origin: origin_host(headers),
    }
}

/// The host of the request's `Origin`; an `Origin` that is not
/// `http(s)://…` as it came (the Origin guard refuses such an upgrade, but
/// the line never hides a value); `-` without one.
fn origin_host(headers: &HeaderMap) -> String {
    let Some(value) = headers.get(header::ORIGIN) else {
        return "-".to_string();
    };
    let value = String::from_utf8_lossy(value.as_bytes());
    let host = access::origin_authority(&value).unwrap_or(&value);
    client_report::clean(host, client_report::WORD_MAX_CHARS)
}

/// A socket's connect or disconnect line (`what`), with its [`Opener`].
fn log_socket(what: &str, client: ClientId, opener: &Opener) {
    tracing::info!(
        client,
        peer = %opener.peer,
        source = %opener.source,
        origin = %opener.origin,
        "{what}"
    );
}

/// `GET /ws`: 401 without a valid token, else the upgrade. Both listeners
/// insert the peer (`ConnectInfo`), and the Access check refuses a request
/// without one before it gets here.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(hub): State<Hub>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(query): Query<WsQuery>,
) -> Response {
    if hub.auth.claims(query.token.as_deref()).is_none() {
        return crate::auth::unauthorized();
    }
    let who = opener(peer, &headers);
    ws.on_upgrade(move |socket| session(socket, hub, query.proto, who))
        .into_response()
}

fn text(msg: &ServerMsg) -> Message {
    Message::Text(serde_json::to_string(msg).unwrap_or_default().into())
}

async fn close_reload(mut socket: WebSocket) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code: CLOSE_RELOAD,
            reason: "reload".into(),
        })))
        .await;
}

async fn session(mut socket: WebSocket, hub: Hub, proto: Option<u32>, who: Opener) {
    let Some(proto) = proto else {
        tracing::info!("WebSocket without a protocol: asking the client to reload");
        close_reload(socket).await;
        return;
    };
    let hello = ServerMsg::Hello {
        proto: UI_PROTO,
        build: fohmixer_proto::VERSION.to_string(),
        min_client_proto: MIN_CLIENT_PROTO,
    };
    if socket.send(text(&hello)).await.is_err() {
        return;
    }
    if !proto_ok(proto) {
        tracing::info!(proto, "WebSocket protocol out of range: the client reloads");
        close_reload(socket).await;
        return;
    }
    let client = hub.next_client();
    let outbox = Arc::new(Outbox::new());
    hub.route(RouterMsg::Attach {
        client,
        outbox: Arc::clone(&outbox),
    });
    log_socket("client connected", client, &who);
    let (mut sink, mut stream) = socket.split();
    let mut writer = {
        let outbox = Arc::clone(&outbox);
        tokio::spawn(async move {
            while let Some(batch) = outbox.next_batch().await {
                for msg in &batch {
                    match tokio::time::timeout(SEND_TIMEOUT, sink.send(text(msg))).await {
                        Ok(Ok(())) => {}
                        Ok(Err(_)) => {
                            outbox.close();
                            return;
                        }
                        Err(_) => {
                            tracing::warn!(
                                client,
                                "a client took over 5 s to take a message: closing it"
                            );
                            outbox.close();
                            return;
                        }
                    }
                }
            }
            let _ = sink.close().await;
        })
    };
    loop {
        tokio::select! {
            msg = stream.next() => match msg {
                Some(Ok(Message::Text(t))) => handle_text(&hub, client, &outbox, t.as_str()),
                Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
            _ = &mut writer => break,
        }
    }
    outbox.close();
    hub.route(RouterMsg::Detach { client });
    writer.abort();
    log_socket("client disconnected", client, &who);
}

fn handle_text(hub: &Hub, client: ClientId, outbox: &Arc<Outbox>, text: &str) {
    match serde_json::from_str::<ClientMsg>(text) {
        Ok(ClientMsg::Cmd {
            id,
            instance,
            commands,
        }) => pass_through(hub, outbox, id, &instance, commands),
        Ok(ClientMsg::Sub {
            instance,
            target,
            prop,
            display,
        }) => hub.route(RouterMsg::Sub {
            client,
            instance,
            target,
            prop,
            display,
        }),
        Ok(ClientMsg::Unsub { sub }) => hub.route(RouterMsg::Unsub { client, sub }),
        Ok(ClientMsg::SetHub { key, value }) => hub.route(RouterMsg::SetHub { client, key, value }),
        // Through the outbox like every answer: a pong proves the writer
        // still reaches the client.
        Ok(ClientMsg::Ping) => outbox.reply(ServerMsg::Pong),
        Err(e) => {
            tracing::warn!(client, error = %e, "unreadable client message");
            outbox.reply(ServerMsg::Error {
                id: None,
                message: format!("unreadable message: {e}"),
            });
        }
    }
}

/// Forwards a command batch unchanged (spec §2.4 pass-through); listeners
/// are the router's (`sub`/`unsub`), so those two operations are refused.
fn pass_through(
    hub: &Hub,
    outbox: &Arc<Outbox>,
    id: String,
    instance: &str,
    commands: Vec<LiveCommand>,
) {
    let refuse = |message: String| {
        outbox.reply(ServerMsg::Error {
            id: Some(id.clone()),
            message,
        });
    };
    let Some(live) = hub.live(instance) else {
        refuse(format!("unknown instance {instance:?}"));
        return;
    };
    if let Some(listener) = commands
        .iter()
        .find(|c| c.name == "add_listener" || c.name == "remove_listener")
    {
        refuse(format!("{} goes through sub/unsub", listener.name));
        return;
    }
    let commands: Vec<Value> = commands
        .iter()
        .map(|c| serde_json::to_value(c).unwrap_or(Value::Null))
        .collect();
    let result = live.call(commands);
    let outbox = Arc::clone(outbox);
    tokio::spawn(async move {
        match result.await {
            Ok(data) => outbox.reply(ServerMsg::Result { id, data }),
            Err(e) => outbox.reply(ServerMsg::Error {
                id: Some(id),
                message: e.to_string(),
            }),
        }
    });
}

#[cfg(test)]
#[path = "ws/tests.rs"]
mod tests;
