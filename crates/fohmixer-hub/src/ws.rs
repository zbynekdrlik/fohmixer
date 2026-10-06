//! `/ws?token=<jwt>&proto=2` — a client's WebSocket (S3 design note §3; the
//! session shape of iemmixer's `mixer_ws.rs` @ 22372bc).
//!
//! The first message is `hello`; a client without a protocol, or with one
//! this hub does not serve, is closed with code 4001 and reloads. Then:
//! commands go straight to their instance (in order) and their results
//! come back as `result`; `sub`/`unsub`/`set_hub` and the writes (`set`,
//! #43) go to the router; the Stream Deck's `deck_view` and `deck_press`
//! (#52) too, and with `[companion]` every message tells the router the page
//! was heard (a holding page silent for 2 s is released); `ping` is
//! answered `pong` with the hub's clock (the client's watchdog and round
//! trip). The socket's open and close and every ping are event-log records
//! (#43; a ping's carries the page's clock offset, `clock.rs`). A writer
//! task drains the client's outbox, so a slow client never holds up anyone
//! else; a client that takes longer than [`SEND_TIMEOUT`] to accept a
//! message is closed (it reconnects and resyncs).
//!
//! A socket's connect and disconnect lines name who opened it (#9, an
//! [`Opener`]): the peer, the address Cloudflare forwarded (through the
//! tunnel the peer is cloudflared), `lan` / `internet` as the Access check
//! classifies the upgrade, and the host of the page's `Origin`. A tab that
//! holds a token reconnects without a login line, and a bundle from before
//! the client reports sends none, so for every socket that passes the
//! protocol check these two lines are the only trace sure to name its
//! client.

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
use serde_json::{Value, json};

use crate::Hub;
use crate::access;
use crate::client_report;
use crate::clock::ClockSync;
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
    /// The client Cloudflare names (`cf-connecting-ip`) for an internet
    /// upgrade, read by [`client_report::forwarded_client`] as for a report;
    /// `-` without one: a LAN or port-forwarded client is its own peer, and
    /// another proxy's headers are not read. The client's own text, so the
    /// line quotes it.
    pub forwarded: String,
    /// `lan` / `internet`: the Access check's class of the upgrade, named
    /// as a client report names it ([`client_report::source`]).
    pub source: &'static str,
    /// The host (`host[:port]`) of the page's `Origin`: the name or address
    /// the page was opened by; `-` without an `Origin` (not a browser). The
    /// client's own text, so the line quotes it.
    pub origin: String,
}

/// The [`Opener`] of an upgrade from `peer` with `headers`: the Access
/// check's own classification (no second classifier), the forwarded client
/// as a client report reads it, and the `Origin` host
/// [`client_report::clean`]ed like a report's field (control characters
/// out, [`client_report::WORD_MAX_CHARS`]).
pub fn opener(peer: SocketAddr, headers: &HeaderMap) -> Opener {
    let class = access::classify(Some(peer), headers);
    Opener {
        peer: peer.ip().to_string(),
        forwarded: client_report::forwarded_client(class, headers)
            .unwrap_or_else(|| "-".to_string()),
        source: client_report::source(class),
        origin: origin_host(headers),
    }
}

/// The host of the request's `Origin`; an `Origin` that is not
/// `http(s)://…` as it came (the Origin guard refuses such a visible-ASCII
/// one, but one with other bytes passes it); `-` without one.
fn origin_host(headers: &HeaderMap) -> String {
    let Some(value) = headers.get(header::ORIGIN) else {
        return "-".to_string();
    };
    let value = String::from_utf8_lossy(value.as_bytes());
    let host = access::origin_authority(&value).unwrap_or(&value);
    client_report::clean(host, client_report::WORD_MAX_CHARS)
}

/// A socket's connect or disconnect line (`what`), with its [`Opener`]. The
/// forwarded address and the page host are Debug-quoted, like a client
/// report's fields: spaces in them cannot add fields to the line. The
/// source (a `&str`) is quoted too, as a client report's is: one grep finds
/// both kinds of line. `client` is the socket number, as on every `ws` line.
fn log_socket(what: &str, client: ClientId, opener: &Opener) {
    tracing::info!(
        client,
        peer = %opener.peer,
        forwarded = ?opener.forwarded,
        source = opener.source,
        origin = ?opener.origin,
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
        peer: who.peer.clone(),
    });
    log_socket("client connected", client, &who);
    hub.events
        .record("sock", sock_fields("open", client, &who, None));
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
                            return "a write to the client failed";
                        }
                        Err(_) => {
                            tracing::warn!(
                                client,
                                "a client took over 5 s to take a message: closing it"
                            );
                            outbox.close();
                            return "the client took over 5 s to take a message";
                        }
                    }
                }
            }
            let _ = sink.close().await;
            "the hub closed it (a stop, or the client stopped reading)"
        })
    };
    let mut conn = Conn {
        client,
        outbox: Arc::clone(&outbox),
        who: who.clone(),
        clock: ClockSync::default(),
        offset: None,
    };
    let reason = loop {
        tokio::select! {
            msg = stream.next() => match msg {
                Some(Ok(Message::Text(t))) => handle_text(&hub, &mut conn, t.as_str()),
                Some(Ok(Message::Close(_))) => break "the client closed it",
                Some(Err(_)) => break "a read from the client failed",
                None => break "the connection ended",
                Some(Ok(_)) => {}
            },
            ended = &mut writer => break ended.unwrap_or("the writer ended"),
        }
    };
    outbox.close();
    hub.route(RouterMsg::Detach { client });
    writer.abort();
    log_socket("client disconnected", client, &who);
    hub.events
        .record("sock", sock_fields("close", client, &who, Some(reason)));
}

/// The event-log fields of a socket's open or close (`what`), with why it
/// closed.
pub fn sock_fields(what: &str, client: ClientId, who: &Opener, reason: Option<&str>) -> Value {
    json!({
        "what": what,
        "client": client,
        "peer": who.peer,
        "forwarded": who.forwarded,
        "source": who.source,
        "origin": who.origin,
        "reason": reason,
    })
}

/// One socket as its messages see it: its client, outbox and opener, and
/// the page's clock against the hub's.
struct Conn {
    client: ClientId,
    outbox: Arc<Outbox>,
    who: Opener,
    clock: ClockSync,
    /// The page clock's offset (hub − page, ms) as of the last ping.
    offset: Option<f64>,
}

/// A ping (`n`, page time `t`, the latest round trip `rtt` and the ping
/// `rtt_n` it measured) that reached the hub at `arrival` (hub UTC ms): its
/// `pong`, and its event-log fields with the page clock's offset (kept for
/// the socket's sets).
fn pong(
    conn: &mut Conn,
    (n, t): (u32, f64),
    rtt: Option<f64>,
    rtt_n: Option<u32>,
    arrival: f64,
) -> (ServerMsg, Value) {
    conn.offset = conn.clock.on_ping(n, arrival, t, rtt_n.zip(rtt));
    let mut fields = json!({
        "client": conn.client,
        "peer": conn.who.peer,
        "n": n,
        "t": t,
        "hub_ms": arrival,
        "rtt": rtt,
        "rtt_n": rtt_n,
        "offset_ms": conn.offset,
    });
    // A clock step (#52): the page's or the hub's clock jumped, and the
    // offset starts over from this exchange. Once per step, on the ping
    // record too (the delays of the sets and presses around it change).
    if let Some(step_ms) = conn.clock.take_step() {
        tracing::warn!(
            client = conn.client,
            step_ms,
            offset_ms = ?conn.offset,
            "a page's clock stepped against the hub's: its offset starts over"
        );
        fields["clock_step_ms"] = json!(step_ms);
    }
    (ServerMsg::Pong { n, t, h: arrival }, fields)
}

/// The event-log fields of a page's `trace` (#43): its events as sent.
pub fn trace_fields(client: ClientId, peer: &str, events: &[Value]) -> Value {
    json!({"client": client, "peer": peer, "events": events})
}

fn handle_text(hub: &Hub, conn: &mut Conn, text: &str) {
    let client = conn.client;
    let outbox = Arc::clone(&conn.outbox);
    let outbox = &outbox;
    // A holding page's silence releases its Stream Deck keys (#52): every
    // message it sends counts.
    if hub.config.companion.is_some() {
        hub.route(RouterMsg::Heard { client });
    }
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
        Ok(ClientMsg::Set {
            instance,
            target,
            prop,
            value,
            seq,
            t,
            is_final,
        }) => hub.route(RouterMsg::Set {
            client,
            instance,
            target,
            prop,
            value,
            seq,
            t,
            is_final,
            hub_ms: crate::live::wall_ms().unwrap_or(0.0),
            offset_ms: conn.offset,
        }),
        Ok(ClientMsg::Trace { events }) => hub
            .events
            .record("trace", trace_fields(client, &conn.who.peer, &events)),
        Ok(ClientMsg::DeckView { on }) => hub.route(RouterMsg::DeckView { client, on }),
        Ok(ClientMsg::DeckPress {
            key,
            down,
            seq,
            t,
            hold_ms,
            why,
        }) => hub.route(RouterMsg::DeckPress {
            client,
            key,
            down,
            seq,
            t,
            hold_ms,
            why,
            hub_ms: crate::live::wall_ms().unwrap_or(0.0),
            offset_ms: conn.offset,
        }),
        // Through the outbox like every answer: a pong proves the writer
        // still reaches the client.
        Ok(ClientMsg::Ping { n, t, rtt, rtt_n }) => {
            let arrival = crate::live::wall_ms().unwrap_or(0.0);
            let (answer, fields) = pong(conn, (n, t), rtt, rtt_n, arrival);
            outbox.reply(answer);
            hub.events.record("ping", fields);
        }
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
