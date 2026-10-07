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
    round_trip_client(
        ClientMsg::Set {
            instance: "band".into(),
            target: "live_set tracks 0 mixer_device volume".into(),
            prop: "value".into(),
            value: json!(0.85),
            seq: 41,
            t: 1_790_000_000_123.5,
            is_final: true,
        },
        json!({"type": "set", "instance": "band", "target": "live_set tracks 0 mixer_device volume",
               "prop": "value", "value": 0.85, "seq": 41, "t": 1_790_000_000_123.5, "final": true}),
    );
    round_trip_client(
        ClientMsg::Ping {
            n: 7,
            t: 1_790_000_000_000.25,
            rtt: Some(12.5),
            rtt_n: Some(5),
        },
        json!({"type": "ping", "n": 7, "t": 1_790_000_000_000.25, "rtt": 12.5, "rtt_n": 5}),
    );
    round_trip_client(
        ClientMsg::Trace {
            events: vec![json!({"ev": "dropout", "t": 5_000.5, "ms": 420.0,
                                "socket_lost": false, "rtts": [12.0, 14.5]})],
        },
        json!({"type": "trace", "events": [{"ev": "dropout", "t": 5_000.5, "ms": 420.0,
                                            "socket_lost": false, "rtts": [12.0, 14.5]}]}),
    );
    // The first ping has no round trip yet: no `rtt` on the wire.
    round_trip_client(
        ClientMsg::Ping {
            n: 0,
            t: 5.0,
            rtt: None,
            rtt_n: None,
        },
        json!({"type": "ping", "n": 0, "t": 5.0}),
    );
}

#[test]
fn a_set_needs_its_sequence_clock_and_final_flag() {
    let full = json!({"type": "set", "instance": "band", "target": "live_set", "prop": "tempo",
                      "value": 120, "seq": 1, "t": 2.0, "final": false});
    assert!(serde_json::from_value::<ClientMsg>(full.clone()).is_ok());
    for missing in ["seq", "t", "final", "value"] {
        let mut wire = full.clone();
        wire.as_object_mut().unwrap().remove(missing);
        assert!(
            serde_json::from_value::<ClientMsg>(wire).is_err(),
            "a set without {missing}"
        );
    }
    // A protocol 1 ping (no number, no time) is not protocol 2.
    assert!(serde_json::from_value::<ClientMsg>(json!({"type": "ping"})).is_err());
}

#[test]
fn ack_items_carry_one_outcome_each() {
    assert_eq!(
        serde_json::to_value(AckItem::applied(
            "band|live_set|tempo",
            3,
            Some(json!(120.0))
        ))
        .unwrap(),
        json!({"key": "band|live_set|tempo", "seq": 3, "value": 120.0})
    );
    assert_eq!(
        serde_json::to_value(AckItem::applied("k", 4, None)).unwrap(),
        json!({"key": "k", "seq": 4})
    );
    assert_eq!(
        serde_json::to_value(AckItem::failed("k", 5, "no result within 3 s")).unwrap(),
        json!({"key": "k", "seq": 5, "error": "no result within 3 s"})
    );
    assert_eq!(
        serde_json::to_value(AckItem::superseded("k", 6)).unwrap(),
        json!({"key": "k", "seq": 6, "superseded": true})
    );
    // Missing fields read as none / not superseded.
    assert_eq!(
        serde_json::from_value::<AckItem>(json!({"key": "k", "seq": 4})).unwrap(),
        AckItem::applied("k", 4, None)
    );
    assert_eq!(
        serde_json::from_value::<AckItem>(json!({"key": "k", "seq": 6, "superseded": true}))
            .unwrap(),
        AckItem::superseded("k", 6)
    );
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
        json!({"type": "hello", "proto": 2, "build": "0.1.0", "min_client_proto": 2}),
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
    round_trip_server(
        ServerMsg::Pong {
            n: 7,
            t: 1_790_000_000_000.25,
            h: 1_790_000_000_004.0,
        },
        json!({"type": "pong", "n": 7, "t": 1_790_000_000_000.25, "h": 1_790_000_000_004.0}),
    );
    round_trip_server(
        ServerMsg::Ack {
            items: vec![
                AckItem::applied("band|live_set tracks 0 mixer_device volume|value", 41, None),
                AckItem::failed("band|live_set|tempo", 9, "instance offline"),
                AckItem::superseded("master|live_set tracks 1 mute|value", 2),
            ],
        },
        json!({"type": "ack", "items": [
            {"key": "band|live_set tracks 0 mixer_device volume|value", "seq": 41},
            {"key": "band|live_set|tempo", "seq": 9, "error": "instance offline"},
            {"key": "master|live_set tracks 1 mute|value", "seq": 2, "superseded": true}]}),
    );
    round_trip_server(
        ServerMsg::Link {
            instance: "band".into(),
            tick_age_ms: 412.5,
            busy: true,
        },
        json!({"type": "link", "instance": "band", "tick_age_ms": 412.5, "busy": true}),
    );
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
fn set_keys_are_hub_keys_without_display() {
    assert_eq!(
        set_key("band", "live_set   tracks 0  mixer_device volume", "value"),
        "band|live_set tracks 0 mixer_device volume|value"
    );
    assert_eq!(
        set_key("master", " live_set tracks[name=Hand1 #] mute", "value"),
        "master|live_set tracks[name=Hand1 #] mute|value"
    );
    assert_ne!(
        set_key("band", "live_set tracks 0", "mute"),
        set_key("band", "live_set tracks 0", "solo")
    );
}

#[test]
fn a_hub_keys_write_key_drops_its_display_flag() {
    for display in [true, false] {
        let key = hub_key(
            "band",
            "live_set  tracks[name=Hand1 #] mixer_device volume",
            "value",
            display,
        );
        assert_eq!(
            write_key_of(&key),
            set_key(
                "band",
                "live_set tracks[name=Hand1 #] mixer_device volume",
                "value"
            )
        );
    }
    assert_eq!(
        write_key_of("band|live_set tracks 0|mute|false"),
        "band|live_set tracks 0|mute"
    );
    assert_eq!(write_key_of("no-bar"), "no-bar");
}

#[test]
fn the_served_protocol_range() {
    // Protocol 2 only (#43): a protocol 1 page reloads.
    assert_eq!((UI_PROTO, MIN_CLIENT_PROTO), (2, 2));
    assert!(proto_ok(2));
    assert!(!proto_ok(1));
    assert!(!proto_ok(3));
}

#[test]
fn deck_messages_round_trip() {
    round_trip_client(
        ClientMsg::DeckView { on: true },
        json!({"type": "deck_view", "on": true}),
    );
    // A down has no hold and no why: neither is on the wire.
    round_trip_client(
        ClientMsg::DeckPress {
            key: 3,
            down: true,
            seq: 7,
            t: 1_790_000_000_123.5,
            hold_ms: None,
            why: None,
        },
        json!({"type": "deck_press", "key": 3, "down": true, "seq": 7, "t": 1_790_000_000_123.5}),
    );
    round_trip_client(
        ClientMsg::DeckPress {
            key: 3,
            down: false,
            seq: 8,
            t: 1_790_000_000_323.5,
            hold_ms: Some(200.0),
            why: Some("cancel".into()),
        },
        json!({"type": "deck_press", "key": 3, "down": false, "seq": 8,
               "t": 1_790_000_000_323.5, "hold_ms": 200.0, "why": "cancel"}),
    );
    round_trip_server(
        ServerMsg::Deck {
            online: false,
            columns: 8,
            rows: 4,
            title: "Stream Deck".into(),
        },
        json!({"type": "deck", "online": false, "columns": 8, "rows": 4, "title": "Stream Deck"}),
    );
    round_trip_server(
        ServerMsg::DeckKeys {
            items: vec![
                DeckKey {
                    key: 0,
                    img: Some("data:image/webp;base64,UklGRg+/=".into()),
                    color: Some("#00aa00".into()),
                    pressed: true,
                },
                DeckKey {
                    key: 31,
                    img: None,
                    color: None,
                    pressed: false,
                },
            ],
        },
        json!({"type": "deck_keys", "items": [
            {"key": 0, "img": "data:image/webp;base64,UklGRg+/=", "color": "#00aa00", "pressed": true},
            {"key": 31, "pressed": false}]}),
    );
    round_trip_server(
        ServerMsg::DeckAck {
            seq: 7,
            ok: true,
            error: None,
            rtt_ms: Some(3.5),
        },
        json!({"type": "deck_ack", "seq": 7, "ok": true, "rtt_ms": 3.5}),
    );
    round_trip_server(
        ServerMsg::DeckAck {
            seq: 9,
            ok: false,
            error: Some("offline".into()),
            rtt_ms: None,
        },
        json!({"type": "deck_ack", "seq": 9, "ok": false, "error": "offline"}),
    );
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
            unfolded: vec!["Stems grp#".into()],
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
                fps: None,
                long_frame_ms: None,
                touches_max: None,
                pointer: None,
            },
        }],
        companion: Some(CompanionStatus {
            online: true,
            last_error: None,
            connect_failures: 0,
            companion_version: Some("5.0.7+9763-stable-cec2f88e6f".into()),
            api_version: Some("1.12.0".into()),
            keys: 32,
        }),
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
               "reconnects": "0", "error": null,
               "fps": null, "long_frame_ms": null, "touches_max": null, "pointer": null})
    );
    // An older hub's answer has no `remote` and no `client_reports`: the
    // defaults.
    let mut older = json.clone();
    older.as_object_mut().unwrap().remove("remote");
    older.as_object_mut().unwrap().remove("client_reports");
    older.as_object_mut().unwrap().remove("companion");
    let older = serde_json::from_value::<HubStatus>(older).unwrap();
    assert_eq!(older.remote, RemoteStatus::default());
    assert!(older.client_reports.is_empty());
    assert_eq!(
        older.companion, None,
        "an older hub's answer has no Stream Deck"
    );
    assert_eq!(json["companion"]["keys"], 32);
    assert_eq!(json["companion"]["api_version"], "1.12.0");
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
            fps: None,
            long_frame_ms: None,
            touches_max: None,
            pointer: None,
        }
    );
    // A perf report's numbers (#5, K4) are fields of their own.
    let perf = json!({"kind": "perf", "fps": "59.9", "long_frame_ms": "34",
                      "touches_max": "4", "pointer": "touch"});
    assert_eq!(
        serde_json::from_value::<ReportFields>(perf).unwrap(),
        ReportFields {
            kind: Some("perf".into()),
            fps: Some("59.9".into()),
            long_frame_ms: Some("34".into()),
            touches_max: Some("4".into()),
            pointer: Some("touch".into()),
            ..ReportFields::default()
        }
    );
    assert_eq!(
        serde_json::from_value::<ReportFields>(json!({})).unwrap(),
        ReportFields::default()
    );
}
