//! The client protocol, hub ⇄ UI (S3 design note §3): JSON text frames on
//! `/ws?token=<jwt>&proto=1`, plus the JSON bodies of the hub's API.
//!
//! The hub never reinterprets Live's values: a `cmd` is the FohMixer script's
//! own envelope (S2 design note §3.10) with the instance added, and values
//! and display strings are Live's, as the script reported them.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::path::canonical_target;

/// The client protocol this build speaks.
pub const UI_PROTO: u32 = 1;
/// The oldest client protocol this build serves.
pub const MIN_CLIENT_PROTO: u32 = 1;
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
    /// A liveness probe, answered by `pong` (the client's watchdog: a
    /// socket that stays silent is half-open and is replaced).
    Ping,
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
    /// The answer to `ping`.
    Pong,
    /// A request the hub could not serve (`id` of the `cmd`, if any).
    Error {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        message: String,
    },
}

/// The hub key of a subscription: `instance|target|prop|display`, with the
/// target in canonical form (whitespace between steps collapsed; names are
/// kept as they are).
pub fn hub_key(instance: &str, target: &str, prop: &str, display: bool) -> String {
    format!("{instance}|{}|{prop}|{display}", canonical_target(target))
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
    /// `visibility`, `sw`, `wake-lock` or `error`.
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip_client(msg: ClientMsg, wire: Value) {
        assert_eq!(serde_json::to_value(&msg).unwrap(), wire);
        assert_eq!(serde_json::from_value::<ClientMsg>(wire).unwrap(), msg);
    }

    fn round_trip_server(msg: ServerMsg, wire: Value) {
        assert_eq!(serde_json::to_value(&msg).unwrap(), wire);
        assert_eq!(serde_json::from_value::<ServerMsg>(wire).unwrap(), msg);
    }

    #[test]
    fn client_messages_round_trip() {
        round_trip_client(
            ClientMsg::Cmd {
                id: "c1".into(),
                instance: "band".into(),
                commands: vec![LiveCommand {
                    target: json!("live_set tracks 0"),
                    name: "get_prop".into(),
                    args: json!({"prop": "name"}),
                }],
            },
            json!({"type": "cmd", "id": "c1", "instance": "band",
                   "commands": [{"target": "live_set tracks 0", "name": "get_prop", "args": {"prop": "name"}}]}),
        );
        round_trip_client(
            ClientMsg::Sub {
                instance: "band".into(),
                target: "live_set tracks 0 mixer_device volume".into(),
                prop: "value".into(),
                display: true,
            },
            json!({"type": "sub", "instance": "band", "target": "live_set tracks 0 mixer_device volume",
                   "prop": "value", "display": true}),
        );
        round_trip_client(
            ClientMsg::Unsub { sub: "k".into() },
            json!({"type": "unsub", "sub": "k"}),
        );
        round_trip_client(
            ClientMsg::SetHub {
                key: HUB_STAGE_AUT.into(),
                value: json!(true),
            },
            json!({"type": "set_hub", "key": "stage_aut", "value": true}),
        );
        round_trip_client(ClientMsg::Ping, json!({"type": "ping"}));
    }

    #[test]
    fn optional_client_fields_have_defaults() {
        let sub: ClientMsg = serde_json::from_value(
            json!({"type": "sub", "instance": "band", "target": "live_set", "prop": "is_playing"}),
        )
        .unwrap();
        assert!(matches!(sub, ClientMsg::Sub { display: false, .. }));
        let cmd: ClientMsg =
            serde_json::from_value(json!({"type": "cmd", "id": "x", "instance": "band",
            "commands": [{"target": {"$ref": "live_1"}, "name": "start_playing"}]}))
            .unwrap();
        let ClientMsg::Cmd { commands, .. } = cmd else {
            panic!("a cmd")
        };
        assert_eq!(commands[0].args, Value::Null);
        assert_eq!(commands[0].target, json!({"$ref": "live_1"}));
        assert!(serde_json::from_value::<ClientMsg>(json!({"type": "nope"})).is_err());
    }

    #[test]
    fn server_messages_round_trip() {
        round_trip_server(
            ServerMsg::Hello {
                proto: UI_PROTO,
                build: "0.1.0".into(),
                min_client_proto: MIN_CLIENT_PROTO,
            },
            json!({"type": "hello", "proto": 1, "build": "0.1.0", "min_client_proto": 1}),
        );
        round_trip_server(
            ServerMsg::Result {
                id: "c1".into(),
                data: vec![json!({"ok": true, "data": "Hand1 #"})],
            },
            json!({"type": "result", "id": "c1", "data": [{"ok": true, "data": "Hand1 #"}]}),
        );
        round_trip_server(
            ServerMsg::Subbed {
                sub: "k".into(),
                value: Some(json!(0.85)),
                display: Some("0.0 dB".into()),
                error: None,
            },
            json!({"type": "subbed", "sub": "k", "value": 0.85, "display": "0.0 dB"}),
        );
        round_trip_server(
            ServerMsg::Subbed {
                sub: "k".into(),
                value: None,
                display: None,
                error: None,
            },
            json!({"type": "subbed", "sub": "k"}),
        );
        round_trip_server(
            ServerMsg::Values {
                items: vec![
                    ValueItem::value("a", json!(true), None),
                    ValueItem::error("b", "not found: tracks[name=X]"),
                ],
            },
            json!({"type": "values", "items": [{"sub": "a", "value": true},
                                                {"sub": "b", "error": "not found: tracks[name=X]"}]}),
        );
        round_trip_server(
            ServerMsg::Instance {
                name: "band".into(),
                online: true,
                busy: false,
                set_name: "Test Site".into(),
            },
            json!({"type": "instance", "name": "band", "online": true, "busy": false, "set_name": "Test Site"}),
        );
        round_trip_server(
            ServerMsg::Hub {
                key: HUB_STAGE_AUT.into(),
                value: json!(false),
            },
            json!({"type": "hub", "key": "stage_aut", "value": false}),
        );
        round_trip_server(
            ServerMsg::Layout { rev: 3 },
            json!({"type": "layout", "rev": 3}),
        );
        round_trip_server(ServerMsg::Pong, json!({"type": "pong"}));
        round_trip_server(
            ServerMsg::Error {
                id: Some("c1".into()),
                message: "instance offline".into(),
            },
            json!({"type": "error", "id": "c1", "message": "instance offline"}),
        );
        round_trip_server(
            ServerMsg::Error {
                id: None,
                message: "unreadable".into(),
            },
            json!({"type": "error", "message": "unreadable"}),
        );
    }

    #[test]
    fn a_null_value_is_a_value_not_a_missing_one() {
        let wire = json!({"type": "values", "items": [{"sub": "g", "value": null}]});
        let msg: ServerMsg = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(
            msg,
            ServerMsg::Values {
                items: vec![ValueItem::value("g", Value::Null, None)]
            }
        );
        assert_eq!(serde_json::to_value(&msg).unwrap(), wire);
        let subbed: ServerMsg =
            serde_json::from_value(json!({"type": "subbed", "sub": "g", "value": null})).unwrap();
        assert!(matches!(
            subbed,
            ServerMsg::Subbed {
                value: Some(Value::Null),
                ..
            }
        ));
    }

    #[test]
    fn hub_key_collapses_whitespace_between_steps() {
        assert_eq!(
            hub_key(
                "band",
                "live_set   tracks 0  mixer_device volume",
                "value",
                true
            ),
            "band|live_set tracks 0 mixer_device volume|value|true"
        );
        assert_eq!(
            hub_key(
                "master",
                " live_set  tracks[name=Hand1 #] mute",
                "value",
                false
            ),
            "master|live_set tracks[name=Hand1 #] mute|value|false"
        );
        assert_ne!(
            hub_key("band", "live_set tracks 0", "mute", false),
            hub_key("band", "live_set tracks 0", "mute", true)
        );
    }

    #[test]
    fn the_served_protocol_range() {
        assert!(proto_ok(1));
        assert!(!proto_ok(0));
        assert!(!proto_ok(2));
    }

    #[test]
    fn api_bodies_round_trip() {
        let status = HubStatus {
            instances: vec![InstanceStatus {
                name: "band".into(),
                port: 39101,
                online: true,
                busy: false,
                set_name: "Test Site".into(),
                live_version: "12.2.5".into(),
                script_version: "0.1.0".into(),
                main_tick_age_ms: Some(4.5),
                subscriptions: 2,
                listeners: 4,
                connect_failures: 0,
                last_error: None,
            }],
            layout: LayoutStatus {
                rev: 1,
                error: None,
                unresolved: vec![Unresolved {
                    instance: "band".into(),
                    target: "live_set tracks[name=Nobody #]".into(),
                    error: "not found: tracks[name=Nobody #]".into(),
                }],
            },
            stage_aut: StageAutStatus {
                on: true,
                writes: 3,
            },
            clients: 1,
            remote: RemoteStatus {
                name: Some("foh.example.org".into()),
                https: Some(HttpsStatus {
                    port: 443,
                    bound: true,
                    bind_error: None,
                    serving: true,
                    cert_names: vec!["foh.example.org".into()],
                    not_after: Some(1_800_000_000),
                    days_left: Some(60),
                    cert_error: None,
                    acme: true,
                    acme_error: Some("no Cloudflare API token".into()),
                    acme_failures: 2,
                    last_issued: Some(1_790_000_000),
                }),
                access: true,
                tunnel: Some(TunnelStatus {
                    ready_connections: 4,
                    error: None,
                    checked: Some(1_790_000_100),
                }),
            },
            client_reports: vec![ClientReport {
                at: 1_790_000_200,
                peer: "127.0.0.1".into(),
                client: Some("203.0.113.7".into()),
                source: "internet".into(),
                fields: ReportFields {
                    kind: Some("load".into()),
                    display: Some("standalone".into()),
                    ua: Some("Mozilla/5.0 (iPad)".into()),
                    build: Some("0.1.0".into()),
                    host: Some("foh.example.org".into()),
                    screen: Some("1194x834@2".into()),
                    sw: Some("registered".into()),
                    wake_lock: Some("held".into()),
                    visibility: Some("visible".into()),
                    reconnects: Some("0".into()),
                    error: None,
                },
            }],
        };
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["remote"]["https"]["days_left"], 60);
        assert_eq!(json["remote"]["tunnel"]["ready_connections"], 4);
        assert_eq!(json["remote"]["access"], json!(true));
        // A report's fields sit next to when and where it came from.
        assert_eq!(
            json["client_reports"][0],
            json!({"at": 1_790_000_200_u64, "peer": "127.0.0.1", "client": "203.0.113.7",
                   "source": "internet",
                   "kind": "load", "display": "standalone", "ua": "Mozilla/5.0 (iPad)",
                   "build": "0.1.0", "host": "foh.example.org", "screen": "1194x834@2",
                   "sw": "registered", "wake_lock": "held", "visibility": "visible",
                   "reconnects": "0", "error": null})
        );
        // An older hub's answer has no `remote` and no `client_reports`: the
        // defaults.
        let mut older = json.clone();
        older.as_object_mut().unwrap().remove("remote");
        older.as_object_mut().unwrap().remove("client_reports");
        let older = serde_json::from_value::<HubStatus>(older).unwrap();
        assert_eq!(older.remote, RemoteStatus::default());
        assert!(older.client_reports.is_empty());
        assert_eq!(json["instances"][0]["listeners"], 4);
        assert_eq!(json["stage_aut"]["writes"], 3);
        assert_eq!(json["instances"][0]["connect_failures"], 0);
        assert_eq!(
            json["layout"]["unresolved"][0],
            json!({"instance": "band", "target": "live_set tracks[name=Nobody #]",
                   "error": "not found: tracks[name=Nobody #]"})
        );
        assert_eq!(serde_json::from_value::<HubStatus>(json).unwrap(), status);
        let auth = AuthResponse {
            token: "t".into(),
            expires_in: 60,
        };
        assert_eq!(
            serde_json::to_value(&auth).unwrap(),
            json!({"token": "t", "expires_in": 60})
        );
        assert_eq!(
            serde_json::from_value::<AuthRequest>(json!({"pin": "2468"})).unwrap(),
            AuthRequest { pin: "2468".into() }
        );
        assert_eq!(
            serde_json::to_value(ApiError::new("INVALID_PIN", "Invalid PIN")).unwrap(),
            json!({"code": "INVALID_PIN", "message": "Invalid PIN"})
        );
    }

    #[test]
    fn a_report_body_keeps_its_known_fields_only() {
        // Missing fields are none; a field the hub does not know is dropped.
        let body = json!({"kind": "error", "error": "boom", "cookie": "secret", "ua": null});
        assert_eq!(
            serde_json::from_value::<ReportFields>(body).unwrap(),
            ReportFields {
                kind: Some("error".into()),
                display: None,
                ua: None,
                build: None,
                host: None,
                screen: None,
                sw: None,
                wake_lock: None,
                visibility: None,
                reconnects: None,
                error: Some("boom".into()),
            }
        );
        assert_eq!(
            serde_json::from_value::<ReportFields>(json!({})).unwrap(),
            ReportFields::default()
        );
    }
}
