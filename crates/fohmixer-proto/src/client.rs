//! The client protocol, hub ⇄ UI (S3 design note §3): JSON text frames on
//! `/ws?token=<jwt>&proto=2`, plus the JSON bodies of the hub's API.
//!
//! The hub never reinterprets Live's values: a `cmd` is the FohMixer script's
//! own envelope (S2 design note §3.10) with the instance added, and values
//! and display strings are Live's, as the script reported them.
//!
//! Protocol 2 (#43, `docs/superpowers/specs/2026-10-03-robust-link-design.md`
//! §2): a control's write is a `set` with the page's sequence number and
//! clock, answered by a coalesced `ack`; `ping` / `pong` carry the times of
//! the exchange; `link` reports Live's main-thread health. `cmd` stays for
//! reads and multi-command batches.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::path::canonical_target;

/// The client protocol this build speaks.
pub const UI_PROTO: u32 = 2;
/// The oldest client protocol this build serves (a page of protocol 1
/// reloads through the handshake).
pub const MIN_CLIENT_PROTO: u32 = 2;
/// WebSocket close code telling a client with another protocol to reload.
pub const CLOSE_RELOAD: u16 = 4001;
/// The hub value of the STAGE AUT flag (spec §2.4, F15).
pub const HUB_STAGE_AUT: &str = "stage_aut";

/// Whether a client speaking `proto` is served.
pub fn proto_ok(proto: u32) -> bool {
    (MIN_CLIENT_PROTO..=UI_PROTO).contains(&proto)
}

/// A field that is present, even as `null` (`Some(Value::Null)`); a missing
/// field is `None` through `#[serde(default)]`.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// One command of the script's envelope: a target, an operation name and
/// its arguments, forwarded unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveCommand {
    /// A LOM path (string) or `{"$ref": id}`.
    pub target: Value,
    pub name: String,
    #[serde(default)]
    pub args: Value,
}

/// Client → hub.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// A command batch for one instance; answered by `result` with the same id.
    Cmd {
        id: String,
        instance: String,
        commands: Vec<LiveCommand>,
    },
    /// Subscribe to a property; answered by `subbed`. Deduplicated in the hub.
    Sub {
        instance: String,
        target: String,
        prop: String,
        #[serde(default)]
        display: bool,
    },
    /// End a subscription (the `sub` key from `subbed`).
    Unsub { sub: String },
    /// Set a hub value (only `stage_aut`).
    SetHub { key: String, value: Value },
    /// A control's write (#43): the latest value the engineer set on one
    /// property, answered by an `ack` item of its key. `seq` is the page's,
    /// strictly increasing over all its sets; `t` is the page's clock
    /// (`performance.timeOrigin + performance.now()`, ms); `final` marks a
    /// release, a toggle or a tap.
    Set {
        instance: String,
        target: String,
        prop: String,
        value: Value,
        seq: u64,
        t: f64,
        #[serde(rename = "final")]
        is_final: bool,
    },
    /// A liveness probe, answered by `pong` (the client's watchdog: a
    /// socket that stays silent is half-open and is replaced): its number,
    /// the page's clock, and the round trip (ms) of the latest pong on this
    /// socket with the number of the ping it measured (`rtt_n`), so the hub
    /// pairs it with that ping's own time and arrival; none before the
    /// socket's first pong.
    Ping {
        n: u32,
        t: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rtt: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rtt_n: Option<u32>,
    },
    /// The page's own events for the hub's event log (#43), as it recorded
    /// them (PR A: each finished dropout, `{"ev": "dropout", "t", "ms",
    /// "socket_lost", "rtts"}`).
    Trace { events: Vec<Value> },
    /// The page opened (`on`) or closed the Stream Deck tab (#52): only a
    /// client viewing it gets the keys' images (`deck_keys`).
    DeckView { on: bool },
    /// A Stream Deck key's press (#52): `down` at the touch, up at the
    /// release. `seq` is the page's own counter of presses (not `set`'s);
    /// `t` the page's clock as for `set`; an up carries the hold the page
    /// measured and why it came (`up`, `cancel`, `lost`, `hidden`, `tab`).
    /// Answered by `deck_ack`; never queued, never resent.
    DeckPress {
        key: u32,
        down: bool,
        seq: u64,
        t: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hold_ms: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        why: Option<String>,
    },
}

/// One write's outcome (#43, an `ack` item): Live ran it (`value`: what
/// Live reported, when the result has one), Live refused it (`error`), or
/// another client's newer set replaced it before it was written
/// (`superseded`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AckItem {
    /// The write's key ([`set_key`]).
    pub key: String,
    /// The page's sequence number of the `set` it answers.
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub superseded: bool,
}

impl AckItem {
    /// Live ran set `seq` of `key`; `value` is what its result reported.
    pub fn applied(key: &str, seq: u64, value: Option<Value>) -> Self {
        Self {
            key: key.to_string(),
            seq,
            value,
            error: None,
            superseded: false,
        }
    }

    /// Set `seq` of `key` failed: Live refused it, it timed out, or the
    /// instance is offline.
    pub fn failed(key: &str, seq: u64, error: &str) -> Self {
        Self {
            key: key.to_string(),
            seq,
            value: None,
            error: Some(error.to_string()),
            superseded: false,
        }
    }

    /// Set `seq` of `key` was replaced by another client's newer set before
    /// it was written.
    pub fn superseded(key: &str, seq: u64) -> Self {
        Self {
            key: key.to_string(),
            seq,
            value: None,
            error: None,
            superseded: true,
        }
    }
}

/// The state of one subscription: Live's value (and display string), or why
/// it has none.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValueItem {
    pub sub: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub value: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ValueItem {
    /// Live's value (and display string) for `sub`.
    pub fn value(sub: &str, value: Value, display: Option<String>) -> Self {
        Self {
            sub: sub.to_string(),
            value: Some(value),
            display,
            error: None,
        }
    }

    /// `sub` has no value: its binding does not resolve, or the object went.
    pub fn error(sub: &str, error: &str) -> Self {
        Self {
            sub: sub.to_string(),
            value: None,
            display: None,
            error: Some(error.to_string()),
        }
    }
}

/// One Stream Deck key as Companion drew it (#52): its image (a `data:`
/// URL), its colour (`#rrggbb`) and whether Companion shows it pressed. A
/// key Companion has not drawn yet has neither image nor colour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeckKey {
    pub key: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub img: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub pressed: bool,
}

/// Hub → client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// The first message of every connection.
    Hello {
        proto: u32,
        build: String,
        min_client_proto: u32,
    },
    /// The script's result slots for a `cmd`, in command order.
    Result { id: String, data: Vec<Value> },
    /// A subscription's hub key, with its value when the hub has one.
    Subbed {
        sub: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        value: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// The latest value of each changed subscription (coalesced per client).
    Values { items: Vec<ValueItem> },
    /// An instance's connection state, sent on change.
    Instance {
        name: String,
        online: bool,
        busy: bool,
        set_name: String,
    },
    /// A hub value (STAGE AUT).
    Hub { key: String, value: Value },
    /// The layout changed: GET `/api/layout` for revision `rev`.
    Layout { rev: u64 },
    /// The outcome of this client's writes (#43), coalesced per client: per
    /// key the item of the highest `seq`.
    Ack { items: Vec<AckItem> },
    /// The answer to `ping`: its number and page time echoed, and the hub's
    /// clock when it answered (`h`, UTC ms).
    Pong { n: u32, t: f64, h: f64 },
    /// Live's main-thread health on an instance (#43): on every busy change
    /// and at most 4 times a second while busy. `tick_age_ms` is the age of
    /// Live's last main-thread tick as the hub knows it.
    Link {
        instance: String,
        tick_age_ms: f64,
        busy: bool,
    },
    /// A request the hub could not serve (`id` of the `cmd`, if any).
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
    },
    /// The Stream Deck (#52): sent on attach and whenever Companion's link
    /// goes up or down, only by a hub with a `[companion]` table (a page
    /// without it shows no tab).
    Deck {
        online: bool,
        columns: u32,
        rows: u32,
        title: String,
    },
    /// Keys' new states (#52), coalesced per key, only to a client viewing
    /// the tab.
    DeckKeys { items: Vec<DeckKey> },
    /// The answer to a `deck_press` (#52): Companion's OK (`rtt_ms`: its
    /// round trip from the hub) or why it was not done (`offline`, Companion's
    /// own error); a press the hub took without forwarding it (the key already
    /// held, or an up from a page that does not hold it) is `ok` at once.
    DeckAck {
        seq: u64,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rtt_ms: Option<f64>,
    },
}

/// The hub key of a subscription: `instance|target|prop|display`, with the
/// target in canonical form (whitespace between steps collapsed; names are
/// kept as they are).
pub fn hub_key(instance: &str, target: &str, prop: &str, display: bool) -> String {
    format!("{}|{display}", set_key(instance, target, prop))
}

/// The key of a write (#43, `set` / `ack`): `instance|target|prop`, the hub
/// key without `display`, the target in canonical form.
pub fn set_key(instance: &str, target: &str, prop: &str) -> String {
    format!("{instance}|{}|{prop}", canonical_target(target))
}

/// The write key ([`set_key`]) of a hub key ([`hub_key`]): the hub key
/// without its `display` field (#43: a page closes a write by its
/// subscription's value).
pub fn write_key_of(hub_key: &str) -> &str {
    hub_key
        .rsplit_once('|')
        .map_or(hub_key, |(key, _display)| key)
}

/// `POST /api/auth` body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthRequest {
    pub pin: String,
}

/// `POST /api/auth` answer: the engineer's token and its lifetime (s).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthResponse {
    pub token: String,
    pub expires_in: u64,
}

/// The body of an API error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
        }
    }
}

/// One Live instance in `GET /api/status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstanceStatus {
    pub name: String,
    pub port: u16,
    pub online: bool,
    pub busy: bool,
    pub set_name: String,
    pub live_version: String,
    pub script_version: String,
    /// The last heartbeat's age of Live's main-thread tick.
    pub main_tick_age_ms: Option<f64>,
    /// Client subscriptions (hub keys) on this instance.
    pub subscriptions: usize,
    /// Live listeners the hub holds on this instance (subscriptions and the
    /// name guards of name bindings, deduplicated).
    pub listeners: usize,
    /// Failed connection attempts since the last connection (0 while
    /// connected).
    pub connect_failures: u64,
    /// Why the last attempt failed (nothing listens, a timeout, the port
    /// answers as another instance), until a connection succeeds.
    pub last_error: Option<String>,
}

/// A layout binding that does not resolve on its instance (spec §2.5 D4,
/// I5): its LOM target and the script's answer (a missing or an ambiguous
/// name).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unresolved {
    pub instance: String,
    pub target: String,
    pub error: String,
}

/// The layout in `GET /api/status`: the served revision (0: none), why the
/// file on disk is not served, if it is not, and the bindings that did not
/// resolve at the last check of each connected instance (a check runs when
/// a layout is accepted and when an instance connects).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutStatus {
    pub rev: u64,
    pub error: Option<String>,
    pub unresolved: Vec<Unresolved>,
}

/// STAGE AUT in `GET /api/status`: the flag and the mute writes it made.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageAutStatus {
    pub on: bool,
    pub writes: u64,
}

/// The HTTPS listener of the public name in `GET /api/status` (#17).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpsStatus {
    /// The bound port (the configured one while it is not bound).
    pub port: u16,
    /// Whether the port is bound; when not, why (it is tried again).
    #[serde(default)]
    pub bound: bool,
    #[serde(default)]
    pub bind_error: Option<String>,
    /// Whether it serves (it has a certificate).
    pub serving: bool,
    /// The served certificate's names and its end (Unix seconds).
    pub cert_names: Vec<String>,
    pub not_after: Option<i64>,
    /// Whole days the certificate has left.
    pub days_left: Option<i64>,
    /// Why the stored certificate is not served, when it is not.
    pub cert_error: Option<String>,
    /// Whether the ACME client keeps the certificate (`[acme]`).
    pub acme: bool,
    /// The last ACME attempt's error, until one succeeds, and how many
    /// attempts in a row failed.
    pub acme_error: Option<String>,
    pub acme_failures: u32,
    /// When the ACME client last got a certificate (Unix seconds).
    pub last_issued: Option<i64>,
}

/// cloudflared's readiness in `GET /api/status` (#17).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelStatus {
    /// Ready connections to Cloudflare's edge (0: the tunnel is down).
    pub ready_connections: u32,
    /// Why the last check failed, when it did.
    pub error: Option<String>,
    /// When it was last checked (Unix seconds; none yet: `None`).
    pub checked: Option<i64>,
}

/// Remote access in `GET /api/status` (#17): the public name, its HTTPS
/// listener, whether internet requests are let in (with an Access JWT) and
/// the tunnel.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteStatus {
    /// The `[tls]` name.
    pub name: Option<String>,
    pub https: Option<HttpsStatus>,
    /// `[access]` is configured: internet requests with a valid Access JWT
    /// are served; without it every internet request is refused.
    pub access: bool,
    pub tunnel: Option<TunnelStatus>,
}

/// What a page reports about itself (#26, `POST /api/client-report`): the
/// event and the page's state, every field optional text. The hub keeps
/// these fields only (serde drops any other), each cut short and stripped of
/// control characters.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReportFields {
    /// What happened: `load`, `connected`, `disconnected`, `reconnect`,
    /// `visibility`, `sw`, `wake-lock`, `error` or `perf`.
    pub kind: Option<String>,
    /// `standalone` (a Home Screen app) or `browser` (a browser tab).
    pub display: Option<String>,
    /// The browser's user agent.
    pub ua: Option<String>,
    /// The page's build (its bundle's `VERSION`).
    pub build: Option<String>,
    /// The host the page was loaded from (`location.host`).
    pub host: Option<String>,
    /// The screen: `<width>x<height>@<device pixel ratio>`.
    pub screen: Option<String>,
    /// The service worker (`data-sw`); none off the https origin.
    pub sw: Option<String>,
    /// The Screen Wake Lock (`data-wake-lock`); none off the https origin.
    pub wake_lock: Option<String>,
    /// `visible` or `hidden`.
    pub visibility: Option<String>,
    /// The hub socket's reconnects since the page loaded.
    pub reconnects: Option<String>,
    /// The error's text (an `error` report).
    pub error: Option<String>,
    /// A `perf` report (#5, K4): frames per second over the page's last
    /// 10 s of frames, one decimal (`59.9`); none before the first window.
    pub fps: Option<String>,
    /// A `perf` report: the longest gap between two frames in that window,
    /// whole ms.
    pub long_frame_ms: Option<String>,
    /// A `perf` report: the most pointers down at once since the page's
    /// previous `perf` report.
    pub touches_max: Option<String>,
    /// A `perf` report: the latest pointer's type (`touch`, `mouse`,
    /// `pen`).
    pub pointer: Option<String>,
}

/// A page's report as the hub keeps it (`GET /api/status`
/// `client_reports`, #26).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientReport {
    /// When the hub got it (Unix seconds).
    pub at: u64,
    /// The address it came from (through the tunnel: cloudflared's).
    pub peer: String,
    /// Through the tunnel: the client's address as Cloudflare names it
    /// (`cf-connecting-ip`); none on the LAN (the peer is the client).
    #[serde(default)]
    pub client: Option<String>,
    /// `lan` or `internet`, as the Access check classifies the request.
    pub source: String,
    #[serde(flatten)]
    pub fields: ReportFields,
}

/// The Stream Deck's Companion link in `GET /api/status` (#52), from the
/// Companion task's snapshot: online, why the last attempt failed (until a
/// session starts), the failed attempts since the last session, Companion's
/// and its Satellite API's versions, and how many keys Companion drew this
/// session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompanionStatus {
    pub online: bool,
    pub last_error: Option<String>,
    pub connect_failures: u64,
    pub companion_version: Option<String>,
    pub api_version: Option<String>,
    pub keys: u32,
}

/// `GET /api/status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HubStatus {
    pub instances: Vec<InstanceStatus>,
    pub layout: LayoutStatus,
    pub stage_aut: StageAutStatus,
    /// Connected client WebSockets.
    pub clients: usize,
    /// Remote access (#17); absent in an older hub's answer.
    #[serde(default)]
    pub remote: RemoteStatus,
    /// The pages' latest reports, oldest first (#26); absent in an older
    /// hub's answer.
    #[serde(default)]
    pub client_reports: Vec<ClientReport>,
    /// The Stream Deck's Companion link (#52); none without `[companion]`
    /// (and in an older hub's answer).
    #[serde(default)]
    pub companion: Option<CompanionStatus>,
}

#[cfg(test)]
mod tests;
