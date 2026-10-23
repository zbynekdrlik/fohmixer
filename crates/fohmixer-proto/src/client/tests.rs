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
            policy: Some("0123456789abcdef".into()),
        },
        json!({"type": "hello", "proto": 2, "build": "0.1.0", "min_client_proto": 2,
            "policy": "0123456789abcdef"}),
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
fn the_hellos_page_policy_is_optional_both_ways() {
    // #90: a hub before the page policy's revision says hello without it,
    // and a page reads that as no policy (no reload on it).
    let older: ServerMsg = serde_json::from_value(
        json!({"type": "hello", "proto": 2, "build": "1.0.8", "min_client_proto": 2}),
    )
    .unwrap();
    assert_eq!(
        older,
        ServerMsg::Hello {
            proto: 2,
            build: "1.0.8".into(),
            min_client_proto: 2,
            policy: None,
        }
    );
    // A field the page does not know is ignored, as a page before #90
    // ignores `policy` (`ServerMsg` denies no unknown field).
    let newer: ServerMsg = serde_json::from_value(json!({"type": "hello", "proto": 2,
        "build": "1.0.9", "min_client_proto": 2, "policy": "p", "later": [1]}))
    .unwrap();
    assert!(matches!(newer, ServerMsg::Hello { policy: Some(p), .. } if p == "p"));
    assert_eq!(POLICY_META, "fohmixer-policy");
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
fn ablesets_message_round_trips_and_a_message_this_page_does_not_know_is_ignored() {
    round_trip_server(
        ServerMsg::Ableset {
            title: "AbleSet".into(),
            name: "ableset.example.org".into(),
            parent: "foh.example.org".into(),
            online: Some(true),
            lan_port: 80,
            lan: true,
        },
        json!({"type": "ableset", "title": "AbleSet", "name": "ableset.example.org",
               "parent": "foh.example.org", "online": true, "lan_port": 80, "lan": true}),
    );
    // Not known yet (#90 B3 review 4): before the hub's first read; a page
    // that came through the tunnel (#90 PR C review M2).
    round_trip_server(
        ServerMsg::Ableset {
            title: "AbleSet".into(),
            name: "ableset.example.org".into(),
            parent: "foh.example.org".into(),
            online: None,
            lan_port: 3000,
            lan: false,
        },
        json!({"type": "ableset", "title": "AbleSet", "name": "ableset.example.org",
               "parent": "foh.example.org", "online": null, "lan_port": 3000, "lan": false}),
    );
    // An older hub's message names no `lan`: the tunnel's, as before.
    let older = json!({"type": "ableset", "title": "AbleSet", "name": "ableset.example.org",
                       "parent": "foh.example.org", "online": true, "lan_port": 80});
    assert_eq!(
        serde_json::from_value::<ServerMsg>(older).unwrap(),
        ServerMsg::Ableset {
            title: "AbleSet".into(),
            name: "ableset.example.org".into(),
            parent: "foh.example.org".into(),
            online: Some(true),
            lan_port: 80,
            lan: false,
        }
    );
    // A newer hub's message (#90 B3): any type this page does not know,
    // with any fields, reads as `Unknown` (the page ignores it), never as
    // an error; a known type still has to be whole.
    for newer in [
        json!({"type": "presenter", "stage": {"rows": 2}, "online": true}),
        json!({"type": "a_later_message"}),
    ] {
        let read = serde_json::from_value::<ServerMsg>(newer.clone());
        assert_eq!(read.unwrap(), ServerMsg::Unknown, "{newer}");
    }
    let broken = json!({"type": "ableset", "title": "AbleSet"});
    assert!(serde_json::from_value::<ServerMsg>(broken).is_err());
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
            playing: Some(true),
        }],
        layout: LayoutStatus {
            rev: 1,
            error: None,
            unresolved: vec![Unresolved {
                instance: "band".into(),
                target: "live_set tracks[name=Nobody #]".into(),
                error: "not found: tracks[name=Nobody #]".into(),
            }],
            markers: MarkersStatus {
                found: 2,
                problems: vec![crate::markers::MarkerReport {
                    instance: "band".into(),
                    kind: crate::markers::TrackKind::Track,
                    index: 4,
                    name: r#""Vox" +G:x"#.into(),
                    problems: vec![crate::markers::TagProblem::BadGroup { tag: "+G:x".into() }],
                }],
                links: vec![crate::markers::LinkReport {
                    instance: "band".into(),
                    id: "PODKLADY".into(),
                    leader: crate::markers::Leader::Frame,
                    partners: 5,
                }],
            },
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
        ableset: Some(AbleSetStatus {
            name: "ableset.example.org".into(),
            upstream: "http://127.0.0.1:80".into(),
            online: Some(false),
            last_error: Some("AbleSet answers 503".into()),
            checked: Some(1_790_000_100),
            cert: CertStatus {
                serving: true,
                cert_names: vec!["ableset.example.org".into()],
                not_after: Some(1_800_000_000),
                days_left: Some(89),
                cert_error: None,
                acme_error: None,
                acme_failures: 0,
                last_issued: Some(1_790_000_000),
            },
            sockets: 2,
        }),
        reverb: ReverbStatus::default(),
        fader_links: Vec::new(),
        autotune: AutotuneStatus {
            aut: true,
            writes: 3,
            taps: 1,
            targets: vec![
                AutotuneTargetStatus {
                    instance: "band".into(),
                    track: "live_set tracks 6".into(),
                    label: "Vox 2".into(),
                    on: 0,
                    state: TargetState::On,
                    path: Some("live_set tracks 6 devices 0 parameters 1".into()),
                    problem: None,
                },
                AutotuneTargetStatus {
                    instance: "band".into(),
                    track: "live_set tracks 8".into(),
                    label: "#9".into(),
                    on: 127,
                    state: TargetState::Missing,
                    path: None,
                    problem: Some("no rack named Latencies".into()),
                },
            ],
        },
        songmap: vec![SongMapStatus {
            instance: "band".into(),
            songs: 166,
            keyed: 165,
            changes: 8,
            problems: 0,
            sections: crate::songmap::SectionsState::FirstOf(2),
            at_ms: 1_790_000_300,
            stale: true,
        }],
    };
    let json = serde_json::to_value(&status).unwrap();
    assert_eq!(
        json["autotune"],
        json!({"aut": true, "writes": 3, "taps": 1, "targets": [
            {"instance": "band", "track": "live_set tracks 6", "label": "Vox 2", "on": 0,
             "state": "on", "path": "live_set tracks 6 devices 0 parameters 1"},
            {"instance": "band", "track": "live_set tracks 8", "label": "#9", "on": 127,
             "state": "missing", "problem": "no rack named Latencies"}]})
    );
    assert_eq!(
        json["songmap"],
        json!([{"instance": "band", "songs": 166, "keyed": 165, "changes": 8, "problems": 0,
                "sections": {"first_of": 2}, "at_ms": 1_790_000_300_u64, "stale": true}])
    );
    // A hub before the song map (#140) names none.
    let mut older = json.clone();
    older.as_object_mut().unwrap().remove("songmap");
    assert_eq!(
        serde_json::from_value::<HubStatus>(older).unwrap().songmap,
        vec![]
    );
    assert_eq!(json["remote"]["https"]["days_left"], 60);
    assert_eq!(json["remote"]["tunnel"]["ready_connections"], 4);
    assert_eq!(json["remote"]["access"], json!(true));
    assert_eq!(
        json["layout"]["markers"],
        json!({"found": 2, "problems": [{"instance": "band", "kind": "track", "index": 4,
               "name": "\"Vox\" +G:x", "problems": [{"code": "bad_group", "tag": "+G:x"}]}],
               "links": [{"instance": "band", "id": "PODKLADY", "leader": "frame", "partners": 5}]})
    );
    // A hub before the fader links (#130) names none.
    let mut unlinked = json["layout"]["markers"].clone();
    unlinked.as_object_mut().unwrap().remove("links");
    assert_eq!(
        serde_json::from_value::<MarkersStatus>(unlinked)
            .unwrap()
            .links,
        vec![]
    );
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
    older.as_object_mut().unwrap().remove("ableset");
    older["layout"].as_object_mut().unwrap().remove("markers");
    let older = serde_json::from_value::<HubStatus>(older).unwrap();
    assert_eq!(older.remote, RemoteStatus::default());
    assert_eq!(older.layout.markers, MarkersStatus::default());
    assert!(older.client_reports.is_empty());
    assert_eq!(
        older.companion, None,
        "an older hub's answer has no Stream Deck"
    );
    assert_eq!(older.ableset, None, "nor AbleSet's page");
    let mut before_reverb = json.clone();
    before_reverb.as_object_mut().unwrap().remove("reverb");
    before_reverb.as_object_mut().unwrap().remove("fader_links");
    let before_reverb = serde_json::from_value::<HubStatus>(before_reverb).unwrap();
    assert_eq!(before_reverb.reverb, ReverbStatus::default());
    assert!(before_reverb.fader_links.is_empty());
    // A hub before AUTOTUNE (#104) names none: its defaults.
    let mut before_autotune = json.clone();
    before_autotune.as_object_mut().unwrap().remove("autotune");
    let before_autotune = serde_json::from_value::<HubStatus>(before_autotune).unwrap();
    assert_eq!(before_autotune.autotune, AutotuneStatus::default());
    assert_eq!(
        serde_json::to_value(AutotuneStatus::default()).unwrap(),
        json!({"aut": false, "writes": 0, "taps": 0, "targets": []})
    );
    assert_eq!(
        serde_json::from_value::<HubStatus>(json.clone())
            .unwrap()
            .autotune,
        status.autotune
    );
    assert_eq!(json["ableset"]["cert"]["days_left"], 89);
    assert_eq!(json["ableset"]["sockets"], 2);
    // A hub before the WebSocket tunnel (#90 B2) names none.
    let mut before = json["ableset"].clone();
    before.as_object_mut().unwrap().remove("sockets");
    let before = serde_json::from_value::<AbleSetStatus>(before).unwrap();
    assert_eq!(before.sockets, 0);
    // The link (#90 B3): a hub before it names none, so it reads as not
    // known.
    assert_eq!(json["ableset"]["upstream"], "http://127.0.0.1:80");
    assert_eq!(json["ableset"]["online"], json!(false));
    assert_eq!(json["ableset"]["last_error"], "AbleSet answers 503");
    assert_eq!(json["ableset"]["checked"], 1_790_000_100_i64);
    let mut unlinked = json["ableset"].clone();
    for field in ["upstream", "online", "last_error", "checked"] {
        unlinked.as_object_mut().unwrap().remove(field);
    }
    let unlinked = serde_json::from_value::<AbleSetStatus>(unlinked).unwrap();
    assert_eq!(
        (
            unlinked.upstream.as_str(),
            unlinked.online,
            unlinked.last_error,
            unlinked.checked
        ),
        ("", None, None, None)
    );
    assert_eq!(json["companion"]["keys"], 32);
    assert_eq!(json["companion"]["api_version"], "1.12.0");
    assert_eq!(json["instances"][0]["listeners"], 4);
    assert_eq!(json["stage_aut"]["writes"], 3);
    // REVERB and the fader links' switches (#130): the defaults here, and
    // the defaults from an older hub.
    assert_eq!(
        json["reverb"],
        json!({"aut": false, "faders": 0, "writes": 0, "hands": 0})
    );
    assert_eq!(json["fader_links"], json!([]));
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

#[test]
fn eq_messages_round_trip() {
    use crate::eq::{Crop, EqItem, EqLock, EqState, Part, Touch};
    use crate::layout::{Anchor, Binding};
    let binding = Binding {
        instance: "band".into(),
        anchor: Anchor::Track {
            name: "Hand2 #".into(),
        },
        path: None,
    };
    let binding_wire = json!({"instance": "band", "anchor": {"kind": "track", "name": "Hand2 #"}});
    round_trip_client(
        ClientMsg::EqList {
            binding: binding.clone(),
        },
        json!({"type": "eq_list", "binding": binding_wire.clone()}),
    );
    round_trip_client(
        ClientMsg::EqOpen {
            instance: "band".into(),
            path: "live_set tracks 1 devices 0".into(),
            area: None,
        },
        json!({"type": "eq_open", "instance": "band", "path": "live_set tracks 1 devices 0"}),
    );
    // PR G: the page's picture area (CSS px) with the open, and again when
    // it changes.
    round_trip_client(
        ClientMsg::EqOpen {
            instance: "band".into(),
            path: "live_set tracks 1 devices 0".into(),
            area: Some(crate::eq::Area { w: 390.0, h: 765.5 }),
        },
        json!({"type": "eq_open", "instance": "band", "path": "live_set tracks 1 devices 0",
               "area": {"w": 390.0, "h": 765.5}}),
    );
    round_trip_client(
        ClientMsg::EqArea { w: 844.0, h: 346.0 },
        json!({"type": "eq_area", "w": 844.0, "h": 346.0}),
    );
    round_trip_client(
        ClientMsg::EqInput {
            touch: Touch::Down,
            x: 431.5,
            y: 315.0,
            n: 12,
            seen: Some(11),
        },
        json!({"type": "eq_input", "touch": "down", "x": 431.5, "y": 315.0, "n": 12, "seen": 11}),
    );
    // PR J: a move has no `seen`.
    round_trip_client(
        ClientMsg::EqInput {
            touch: Touch::Move,
            x: 432.0,
            y: 316.0,
            n: 13,
            seen: None,
        },
        json!({"type": "eq_input", "touch": "move", "x": 432.0, "y": 316.0, "n": 13}),
    );
    round_trip_client(ClientMsg::EqClose, json!({"type": "eq_close"}));
    // PR L: the part of the picture the page shows, of the picture's size it
    // drew, or the whole picture.
    round_trip_client(
        ClientMsg::EqView {
            part: Some(Part {
                crop: Crop {
                    x: 320,
                    y: 160,
                    w: 640,
                    h: 400,
                },
                picture: (1349, 809),
            }),
        },
        json!({
            "type": "eq_view",
            "part": {"crop": {"x": 320, "y": 160, "w": 640, "h": 400}, "picture": [1349, 809]}
        }),
    );
    round_trip_client(ClientMsg::EqView { part: None }, json!({"type": "eq_view"}));
    // PR I: the page got a frame of its open editor.
    round_trip_client(
        ClientMsg::EqFrameAck {
            session: 7,
            seq: 12,
        },
        json!({"type": "eq_frame_ack", "session": 7, "seq": 12}),
    );
    // PR P2a: Pro-Q's own undo, numbered with the page's inputs, and its
    // answer to that page.
    round_trip_client(
        ClientMsg::EqUndo { n: 14 },
        json!({"type": "eq_undo", "n": 14}),
    );
    round_trip_server(
        ServerMsg::EqUndo {
            session: 3,
            n: 14,
            result: "done".into(),
        },
        json!({"type": "eq_undo", "session": 3, "n": 14, "result": "done"}),
    );
    round_trip_server(
        ServerMsg::EqUndo {
            session: 0,
            n: 2,
            result: crate::eq::reason::NO_EDITOR.into(),
        },
        json!({"type": "eq_undo", "session": 0, "n": 2, "result": "no such editor"}),
    );
    round_trip_server(
        ServerMsg::EqList {
            binding: binding.clone(),
            items: vec![EqItem {
                path: "live_set tracks 1 devices 1 chains 0 devices 0".into(),
                place: "Vocal FX › Main".into(),
                name: "Pro-Q 4".into(),
                picture: true,
            }],
            error: None,
        },
        json!({"type": "eq_list", "binding": binding_wire.clone(), "items": [
            {"path": "live_set tracks 1 devices 1 chains 0 devices 0", "place": "Vocal FX › Main",
             "name": "Pro-Q 4", "picture": true}]}),
    );
    round_trip_server(
        ServerMsg::EqList {
            binding,
            items: Vec::new(),
            error: Some("instance offline".into()),
        },
        json!({"type": "eq_list", "binding": binding_wire, "items": [], "error": "instance offline"}),
    );
    round_trip_server(
        ServerMsg::EqLocks {
            items: vec![EqLock {
                instance: "band".into(),
                path: "live_set tracks 1 devices 0".into(),
                mine: false,
                since: 1_790_000_000_000.0,
            }],
        },
        json!({"type": "eq_locks", "items": [{"instance": "band", "path": "live_set tracks 1 devices 0",
                                              "mine": false, "since": 1_790_000_000_000.0}]}),
    );
    round_trip_server(
        ServerMsg::Eq {
            instance: "band".into(),
            path: "live_set tracks 1 devices 0".into(),
            state: EqState::Open,
            session: Some(3),
            width: Some(1349),
            height: Some(809),
            reason: None,
            since: None,
        },
        json!({"type": "eq", "instance": "band", "path": "live_set tracks 1 devices 0",
               "state": "open", "session": 3, "width": 1349, "height": 809}),
    );
    // A refusal names why and, for a lock, since when.
    round_trip_server(
        ServerMsg::Eq {
            instance: "band".into(),
            path: "live_set tracks 1 devices 0".into(),
            state: EqState::Closed,
            session: None,
            width: None,
            height: None,
            reason: Some("locked".into()),
            since: Some(1_790_000_000_000.0),
        },
        json!({"type": "eq", "instance": "band", "path": "live_set tracks 1 devices 0",
               "state": "closed", "reason": "locked", "since": 1_790_000_000_000.0}),
    );
}

#[test]
fn idle_messages_round_trip() {
    // #51: an idle page names its meter period; a paused one 0; a woken
    // one none, and nothing of it is on the wire.
    round_trip_client(
        ClientMsg::Idle {
            on: true,
            meter_ms: Some(250),
        },
        json!({"type": "idle", "on": true, "meter_ms": 250}),
    );
    round_trip_client(
        ClientMsg::Idle {
            on: true,
            meter_ms: Some(0),
        },
        json!({"type": "idle", "on": true, "meter_ms": 0}),
    );
    round_trip_client(
        ClientMsg::Idle {
            on: false,
            meter_ms: None,
        },
        json!({"type": "idle", "on": false}),
    );
    // An idle page that names no period reads as none (the default).
    assert_eq!(
        serde_json::from_value::<ClientMsg>(json!({"type": "idle", "on": true})).unwrap(),
        ClientMsg::Idle {
            on: true,
            meter_ms: None
        }
    );
}

#[test]
fn the_meter_props_are_lives_three_meters() {
    assert_eq!(
        METER_PROPS,
        [
            "output_meter_level",
            "output_meter_left",
            "output_meter_right"
        ]
    );
}

#[test]
fn a_meter_key_is_read_from_the_right() {
    for prop in METER_PROPS {
        for display in [false, true] {
            let key = hub_key("band", "live_set tracks[name=Hand1 #]", prop, display);
            assert!(is_meter_key(&key), "{key}");
        }
    }
    for prop in ["mute", "value", "color", "output_meter", "meter_level"] {
        let key = hub_key("band", "live_set tracks[name=Hand1 #]", prop, false);
        assert!(!is_meter_key(&key), "{key}");
    }
    // A `|` inside a track name cannot fool it: read from the left, the
    // first key's third field would be a meter prop, the second's not one.
    assert!(!is_meter_key(
        "band|live_set tracks[name=a|output_meter_level|b]|mute|false"
    ));
    assert!(is_meter_key(
        "band|live_set tracks[name=a|b]|output_meter_left|false"
    ));
    // The display field is not the prop; a key without one has none.
    assert!(!is_meter_key(
        "band|live_set tracks 0|mute|output_meter_level"
    ));
    assert!(!is_meter_key("output_meter_level"));
    assert!(!is_meter_key(""));
}

#[test]
fn the_meter_pace_of_an_idle_message() {
    assert_eq!(
        (IDLE_METER_MS, IDLE_METER_MIN_MS, IDLE_METER_MAX_MS),
        (250, 100, 2000)
    );
    let every = |ms: u64| Pace::Every(Duration::from_millis(ms));
    // Awake: full rate, whatever the period says.
    assert_eq!(meter_pace(false, None), Pace::Full);
    assert_eq!(meter_pace(false, Some(250)), Pace::Full);
    assert_eq!(meter_pace(false, Some(0)), Pace::Full);
    assert_eq!(Pace::default(), Pace::Full);
    // 0 pauses: a hidden page (a locked iPad) gets no meters.
    assert_eq!(meter_pace(true, Some(0)), Pace::Paused);
    // Absent: the default period; any other clamped to 100..=2000.
    assert_eq!(meter_pace(true, None), every(250));
    assert_eq!(meter_pace(true, Some(1)), every(100));
    assert_eq!(meter_pace(true, Some(99)), every(100));
    assert_eq!(meter_pace(true, Some(100)), every(100));
    assert_eq!(meter_pace(true, Some(101)), every(101));
    assert_eq!(meter_pace(true, Some(250)), every(250));
    assert_eq!(meter_pace(true, Some(1999)), every(1999));
    assert_eq!(meter_pace(true, Some(2000)), every(2000));
    assert_eq!(meter_pace(true, Some(2001)), every(2000));
    assert_eq!(meter_pace(true, Some(u32::MAX)), every(2000));
}

#[test]
fn a_linked_mutes_writes_go_as_one_sets_message_of_set_items() {
    // #127: a `set`'s fields, each item its own key, seq and ack.
    let set = |target: &str, seq: u64| ClientMsg::Set {
        instance: "band".into(),
        target: target.into(),
        prop: "mute".into(),
        value: json!(true),
        seq,
        t: 1_790_000_000_123.5,
        is_final: true,
    };
    let items: Vec<SetItem> = [
        set("live_set tracks 3", 7),
        set("live_set return_tracks 1", 8),
    ]
    .into_iter()
    .filter_map(SetItem::of)
    .collect();
    assert_eq!(
        items[1],
        SetItem {
            instance: "band".into(),
            target: "live_set return_tracks 1".into(),
            prop: "mute".into(),
            value: json!(true),
            seq: 8,
            t: 1_790_000_000_123.5,
            is_final: true,
        }
    );
    round_trip_client(
        ClientMsg::Sets { sets: items },
        json!({"type": "sets", "sets": [
            {"instance": "band", "target": "live_set tracks 3", "prop": "mute", "value": true,
             "seq": 7, "t": 1_790_000_000_123.5, "final": true},
            {"instance": "band", "target": "live_set return_tracks 1", "prop": "mute", "value": true,
             "seq": 8, "t": 1_790_000_000_123.5, "final": true}]}),
    );
    // Any other message carries no write.
    assert_eq!(SetItem::of(ClientMsg::Unsub { sub: "k".into() }), None);
}

#[test]
fn the_reverb_state_is_one_message_with_its_faders() {
    let state = ReverbState {
        aut: true,
        low: Some(Low::Stop),
        playing: Some(false),
        waiting: false,
        faders: vec![
            ReverbFader {
                key: "band|live_set tracks 4 mixer_device volume|value".into(),
                label: "Rev ST".into(),
                back: Some(0.85),
                hand: false,
            },
            ReverbFader {
                key: "band|live_set tracks 5 mixer_device volume|value".into(),
                label: "Rev PA".into(),
                back: None,
                hand: true,
            },
        ],
    };
    let wire = json!({"type": "reverb", "aut": true, "low": "stop", "playing": false,
        "faders": [
            {"key": "band|live_set tracks 4 mixer_device volume|value", "label": "Rev ST", "back": 0.85},
            {"key": "band|live_set tracks 5 mixer_device volume|value", "label": "Rev PA", "hand": true}]});
    assert_eq!(
        serde_json::to_value(ServerMsg::Reverb(state.clone())).unwrap(),
        wire
    );
    assert_eq!(
        serde_json::from_value::<ServerMsg>(wire).unwrap(),
        ServerMsg::Reverb(state)
    );
    let bare = json!({"type": "reverb", "aut": false, "faders": []});
    match serde_json::from_value::<ServerMsg>(bare).unwrap() {
        ServerMsg::Reverb(s) => {
            assert!(s.low.is_none() && s.playing.is_none() && !s.waiting && !s.aut)
        }
        other => panic!("{other:?}"),
    }
    let waiting = json!({"type": "reverb", "aut": true, "waiting": true, "faders": []});
    match serde_json::from_value::<ServerMsg>(waiting.clone()).unwrap() {
        ServerMsg::Reverb(s) => {
            assert!(s.waiting);
            assert_eq!(serde_json::to_value(ServerMsg::Reverb(s)).unwrap(), waiting);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        (HUB_REVERB_AUT, HUB_REVERB_LOW),
        ("reverb_aut", "reverb_low")
    );
    assert_eq!(serde_json::to_value(Low::Tap).unwrap(), json!("tap"));
}

#[test]
fn a_fader_links_hub_key_names_its_set_and_id_and_reads_back() {
    let link = LinkId {
        instance: "band".into(),
        id: "REV".into(),
    };
    assert_eq!(FADER_LINK_PREFIX, "fader_link|");
    assert_eq!(fader_link_key(&link), "fader_link|band|REV");
    assert_eq!(fader_link_of("fader_link|band|REV"), Some(link));
    for key in [
        "fader_link|band|",
        "fader_link||REV",
        "fader_link|REV",
        "stage_aut",
        "reverb_aut|band|REV",
    ] {
        assert_eq!(fader_link_of(key), None, "{key}");
    }
    let status = FaderLinkStatus {
        instance: "band".into(),
        id: "REV".into(),
        on: true,
    };
    assert_eq!(
        serde_json::to_value(&status).unwrap(),
        json!({"instance": "band", "id": "REV", "on": true})
    );
}

#[test]
fn an_autotune_message_round_trips_and_an_older_page_reads_it_as_unknown() {
    let state = AutotuneState {
        aut: true,
        playing: Some(false),
        targets: vec![
            AutotuneTargetState {
                label: "Vox 2".into(),
                state: TargetState::On,
            },
            AutotuneTargetState {
                label: "#8".into(),
                state: TargetState::Missing,
            },
        ],
    };
    let wire = json!({"type": "autotune", "aut": true, "playing": false, "targets": [
        {"label": "Vox 2", "state": "on"}, {"label": "#8", "state": "missing"}]});
    round_trip_server(ServerMsg::Autotune(state), wire);
    // Before the transport is heard: no `playing` key.
    let unheard = AutotuneState {
        aut: false,
        playing: None,
        targets: vec![AutotuneTargetState {
            label: "#8".into(),
            state: TargetState::Waiting,
        }],
    };
    round_trip_server(
        ServerMsg::Autotune(unheard),
        json!({"type": "autotune", "aut": false, "targets": [{"label": "#8", "state": "waiting"}]}),
    );
    assert_eq!(
        serde_json::to_value(TargetState::Off).unwrap(),
        json!("off")
    );
    assert_eq!(
        (HUB_AUTOTUNE_AUT, HUB_AUTOTUNE),
        ("autotune_aut", "autotune")
    );
}

#[test]
fn a_song_map_reports_keys_with_their_names() {
    // #140 review M7: every key of the report carries its name, in spans
    // and in changes; an older hub's answer without names still reads.
    let span = crate::songmap::Span {
        number: 31,
        start: 22766.0,
        end: Some(23501.0),
        key: Some(crate::songmap::Key {
            root: 6,
            minor: true,
        }),
        changes: vec![crate::songmap::KeyChange {
            time: 22897.0,
            key: crate::songmap::Key {
                root: 3,
                minor: false,
            },
        }],
    };
    let report = SpanReport::from(&span);
    assert_eq!(
        serde_json::to_value(&report).unwrap(),
        json!({"number": 31, "start": 22766.0, "end": 23501.0,
               "key": {"root": 6, "minor": true, "name": "F#m"},
               "changes": [{"time": 22897.0, "key": {"root": 3, "minor": false, "name": "Eb"}}]})
    );
    let keyless = crate::songmap::Span {
        number: 88,
        start: 0.0,
        end: None,
        key: None,
        changes: Vec::new(),
    };
    assert_eq!(
        serde_json::to_value(SpanReport::from(&keyless)).unwrap(),
        json!({"number": 88, "start": 0.0, "end": null, "key": null, "changes": []})
    );
    let older: SpanReport = serde_json::from_value(json!({
        "number": 31, "start": 0.0, "end": 9.0, "key": {"root": 6, "minor": true},
        "changes": [{"time": 1.0, "key": {"root": 3, "minor": false}}]}))
    .unwrap();
    assert_eq!(
        older.key,
        Some(KeyReport {
            root: 6,
            minor: true,
            name: String::new()
        })
    );
    assert_eq!(older.changes[0].key.name, "");
}
