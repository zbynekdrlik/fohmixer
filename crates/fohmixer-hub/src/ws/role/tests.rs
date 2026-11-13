use fohmixer_proto::client::{ClientMsg, ServerMsg};
use fohmixer_proto::colors::ColorsDoc;
use fohmixer_proto::mute_alarms::MuteAlarmsDoc;
use fohmixer_proto::roles::{Role, sectors_for};
use fohmixer_proto::sectors::SectorsDoc;
use serde_json::{Value, json};

use super::{REFUSED, inbound, outbound, refusal, refused_id};

fn client(wire: Value) -> ClientMsg {
    serde_json::from_value(wire.clone()).unwrap_or_else(|e| panic!("{wire}: {e}"))
}

fn server(wire: Value) -> ServerMsg {
    serde_json::from_value(wire.clone()).unwrap_or_else(|e| panic!("{wire}: {e}"))
}

fn set_to(instance: &str) -> ClientMsg {
    client(json!({"type": "set", "instance": instance,
        "target": "live_set tracks 0 mixer_device volume", "prop": "value", "value": 0.85,
        "seq": 41, "t": 1_790_000_000_123.5, "final": true}))
}

fn item(instance: &str, seq: u64) -> Value {
    json!({"instance": instance, "target": "live_set tracks 3", "prop": "mute", "value": true,
        "seq": seq, "t": 1_790_000_000_123.5, "final": true})
}

#[test]
fn mbc_writes_and_reads_only_its_instance() {
    assert_eq!(inbound(Role::Mbc, &set_to("band")), Err(REFUSED));
    assert_eq!(inbound(Role::Mbc, &set_to("master")), Err(REFUSED));
    assert_eq!(inbound(Role::Mbc, &set_to("mbc")), Ok(()));
    let sub = |instance: &str| {
        client(json!({"type": "sub", "instance": instance,
            "target": "live_set tracks 0 mixer_device volume", "prop": "value", "display": true}))
    };
    assert_eq!(inbound(Role::Mbc, &sub("band")), Err(REFUSED));
    assert_eq!(inbound(Role::Mbc, &sub("mbc")), Ok(()));
    let cmd = |instance: &str| {
        client(json!({"type": "cmd", "id": "c1", "instance": instance,
            "commands": [{"target": "live_set", "name": "get_prop", "args": {"prop": "tempo"}}]}))
    };
    assert_eq!(inbound(Role::Mbc, &cmd("band")), Err(REFUSED));
    assert_eq!(inbound(Role::Mbc, &cmd("mbc")), Ok(()));
    assert_eq!(refused_id(&cmd("band")), Some("c1".to_string()));
    let sets = |items: Vec<Value>| client(json!({"type": "sets", "sets": items}));
    assert_eq!(
        inbound(Role::Mbc, &sets(vec![item("mbc", 1), item("band", 2)])),
        Err(REFUSED)
    );
    assert_eq!(
        inbound(Role::Mbc, &sets(vec![item("mbc", 1), item("mbc", 2)])),
        Ok(())
    );
    assert_eq!(refused_id(&set_to("band")), None);
}

#[test]
fn mbc_may_not_touch_hub_values_deck_presenter_or_eq() {
    for wire in [
        json!({"type": "set_hub", "key": "stage_aut", "value": true}),
        json!({"type": "deck_view", "on": true}),
        json!({"type": "deck_press", "key": 3, "down": true, "seq": 7, "t": 1_790_000_000_123.5}),
        json!({"type": "presenter_switch", "code": "preach", "seq": 3}),
        json!({"type": "eq_open", "instance": "mbc", "path": "live_set tracks 1 devices 0"}),
        json!({"type": "eq_area", "w": 844.0, "h": 346.0}),
        json!({"type": "eq_input", "touch": "down", "x": 431.5, "y": 315.0, "n": 12, "seen": 11}),
        json!({"type": "eq_close"}),
        json!({"type": "eq_view"}),
        json!({"type": "eq_undo", "n": 14}),
        json!({"type": "eq_list", "binding": {"instance": "mbc",
            "anchor": {"kind": "track", "name": "Hand 1"}}}),
    ] {
        let msg = client(wire.clone());
        assert_eq!(inbound(Role::Mbc, &msg), Err(REFUSED), "{wire}");
        assert_eq!(inbound(Role::Engineer, &msg), Ok(()), "{wire}");
    }
}

#[test]
fn mbc_keeps_its_own_socket_messages() {
    for wire in [
        json!({"type": "unsub", "sub": "k"}),
        json!({"type": "ping", "n": 7, "t": 1_790_000_000_000.25}),
        json!({"type": "trace", "events": []}),
        json!({"type": "idle", "on": true, "meter_ms": 250}),
        json!({"type": "eq_frame_ack", "session": 7, "seq": 12}),
        // The router checks the documents.
        json!({"type": "sector_tick", "id": "s1", "layout": "lu-1",
            "strip": {"instance": "band", "label": "Bass"}, "on": true}),
        json!({"type": "set_color", "id": "c1", "strip": {"instance": "band", "label": "Bass"},
            "color": "blue"}),
        json!({"type": "set_mute_alarm", "id": "a1",
            "strip": {"instance": "band", "label": "Bass"}, "on": true}),
    ] {
        assert_eq!(inbound(Role::Mbc, &client(wire.clone())), Ok(()), "{wire}");
    }
    let tick = client(json!({"type": "sector_tick", "id": "s1", "layout": "lu-1",
        "strip": {"instance": "band", "label": "Bass"}, "on": true}));
    assert_eq!(refused_id(&tick), Some("s1".to_string()));
}

/// One sample of every server message type.
fn samples() -> Vec<ServerMsg> {
    [
        json!({"type": "hello", "proto": 2, "build": "0.1.0", "min_client_proto": 2}),
        json!({"type": "result", "id": "c1", "data": []}),
        json!({"type": "subbed", "sub": "band|live_set|tempo|false", "value": 120.0}),
        json!({"type": "values", "items": [{"sub": "band|live_set|tempo|false", "value": 1}]}),
        json!({"type": "instance", "name": "band", "online": true, "busy": false, "set_name": "S"}),
        json!({"type": "hub", "key": "stage_aut", "value": false}),
        json!({"type": "layout", "rev": 3}),
        json!({"type": "ack", "items": [{"key": "band|live_set|tempo", "seq": 9}]}),
        json!({"type": "pong", "n": 7, "t": 1.0, "h": 2.0}),
        json!({"type": "link", "instance": "band", "tick_age_ms": 412.5, "busy": true}),
        json!({"type": "error", "id": "c1", "message": "instance offline"}),
        json!({"type": "deck", "online": false, "columns": 8, "rows": 4, "title": "Stream Deck"}),
        json!({"type": "deck_keys", "items": []}),
        json!({"type": "deck_ack", "seq": 7, "ok": true}),
        json!({"type": "eq_locks", "items": []}),
        json!({"type": "ableset", "title": "AbleSet", "name": "a.example.org",
            "parent": "f.example.org", "online": null, "lan_port": 80}),
        json!({"type": "iem", "url": "https://iem.example.org/", "title": "IEM MIXER"}),
        json!({"type": "presenter", "online": true, "current": null, "layouts": []}),
        json!({"type": "presenter_ack", "seq": 4, "code": "timer", "ok": true}),
        json!({"type": "reverb", "aut": true, "faders": []}),
        json!({"type": "autotune", "aut": true, "targets": []}),
        json!({"type": "rec", "playing": true, "record_mode": true, "tracks": []}),
        json!({"type": "lufs", "items": [{"instance": "band", "target": "live_set tracks 1",
            "tenths": -253}]}),
        json!({"type": "sectors", "doc": {"rev": 2, "sectors": {"lu": [
            {"id": "lu-1", "title": "VOX", "strips": [{"instance": "band", "label": "Vox 1"}]}]}}}),
        json!({"type": "colors", "doc": {"colors": {"band": {"Vox 1": "teal"}}}}),
        json!({"type": "mute_alarms", "doc": {"mute_alarms": {"band": ["Vox 1"]}}}),
        json!({"type": "a_later_message"}),
    ]
    .into_iter()
    .map(server)
    .collect()
}

#[test]
fn the_engineer_is_never_filtered() {
    for msg in samples() {
        assert_eq!(outbound(Role::Engineer, msg.clone()), Some(msg));
    }
}

#[test]
fn mbc_gets_no_foh_message() {
    for msg in samples() {
        let kind = serde_json::to_value(&msg).unwrap()["type"].clone();
        let shown = outbound(Role::Mbc, msg.clone());
        let kept = matches!(
            msg,
            ServerMsg::Hello { .. }
                | ServerMsg::Result { .. }
                | ServerMsg::Subbed { .. }
                | ServerMsg::Ack { .. }
                | ServerMsg::Layout { .. }
                | ServerMsg::Pong { .. }
                | ServerMsg::Error { .. }
                | ServerMsg::Unknown
                | ServerMsg::Sectors { .. }
                | ServerMsg::Colors { .. }
                | ServerMsg::MuteAlarms { .. }
        );
        assert_eq!(shown.is_some(), kept, "{kind}");
    }
    // The documents lose every band entry.
    let colors = server(json!({"type": "colors", "doc": {"colors": {
        "band": {"Vox 1": "teal"}, "mbc": {"Hand 1": "blue"}}}}));
    let Some(ServerMsg::Colors { doc }) = outbound(Role::Mbc, colors) else {
        panic!("a colours message");
    };
    assert_eq!(
        doc,
        serde_json::from_value::<ColorsDoc>(json!({"colors": {"mbc": {"Hand 1": "blue"}}}))
            .unwrap()
    );
    let alarms = server(json!({"type": "mute_alarms", "doc": {"mute_alarms": {
        "band": ["Vox 1"], "mbc": ["Hand 1"]}}}));
    let Some(ServerMsg::MuteAlarms { doc }) = outbound(Role::Mbc, alarms) else {
        panic!("a mute alarms message");
    };
    assert_eq!(
        doc,
        serde_json::from_value::<MuteAlarmsDoc>(json!({"mute_alarms": {"mbc": ["Hand 1"]}}))
            .unwrap()
    );
}

#[test]
fn mbc_gets_only_its_items_of_a_mixed_message() {
    let values = server(json!({"type": "values", "items": [
        {"sub": "band|live_set tracks 0 mixer_device volume|value|false", "value": 0.5},
        {"sub": "mbc|live_set tracks 0 mixer_device volume|value|false", "value": 0.7},
        {"sub": "mbcx|live_set|tempo|false", "value": 1}]}));
    assert_eq!(
        outbound(Role::Mbc, values),
        Some(server(json!({"type": "values", "items": [
            {"sub": "mbc|live_set tracks 0 mixer_device volume|value|false", "value": 0.7}]})))
    );
    // The socket's own answers pass whole: its band writes were refused
    // with a failed ack of their own keys.
    let ack = server(json!({"type": "ack", "items": [
        {"key": "band|live_set|tempo", "seq": 9, "error": "role"}, {"key": "mbc|live_set|tempo", "seq": 10}]}));
    assert_eq!(outbound(Role::Mbc, ack.clone()), Some(ack));
    let lufs = server(json!({"type": "lufs", "items": [
        {"instance": "band", "target": "live_set tracks 1", "tenths": -253},
        {"instance": "mbc", "target": "live_set tracks 2", "tenths": -140}]}));
    assert_eq!(
        outbound(Role::Mbc, lufs),
        Some(server(json!({"type": "lufs", "items": [
            {"instance": "mbc", "target": "live_set tracks 2", "tenths": -140}]})))
    );
    for wire in [
        json!({"type": "instance", "name": "mbc", "online": true, "busy": false, "set_name": "S"}),
        json!({"type": "link", "instance": "mbc", "tick_age_ms": 4.5, "busy": false}),
        json!({"type": "subbed", "sub": "mbc|live_set|tempo|false", "value": 120.0}),
        json!({"type": "subbed", "sub": "band|live_set|tempo|false", "error": "role"}),
    ] {
        let msg = server(wire.clone());
        assert_eq!(outbound(Role::Mbc, msg.clone()), Some(msg), "{wire}");
    }
    // A message left with nothing is dropped.
    let band_only = server(json!({"type": "values", "items": [
        {"sub": "band|live_set|tempo|false", "value": 1}]}));
    assert_eq!(outbound(Role::Mbc, band_only), None);
    let band_lufs = server(json!({"type": "lufs", "items": [
        {"instance": "band", "target": "live_set tracks 1", "tenths": -253}]}));
    assert_eq!(outbound(Role::Mbc, band_lufs), None);
}

#[test]
fn mbc_gets_its_own_sectors_view() {
    let doc: SectorsDoc = serde_json::from_value(json!({"rev": 2, "sectors": {"lu": [
        {"id": "lu-1", "title": "VOX", "strips": [
            {"instance": "band", "label": "Vox 1"}, {"instance": "mbc", "label": "Hand 1"}]}]}}))
    .unwrap();
    let msg = ServerMsg::Sectors { doc: doc.clone() };
    assert_eq!(
        outbound(Role::Mbc, msg),
        Some(ServerMsg::Sectors {
            doc: sectors_for(Role::Mbc, &doc)
        })
    );
    assert_ne!(sectors_for(Role::Mbc, &doc), doc);
}

#[test]
fn a_refused_write_is_answered_by_a_failed_ack_and_anything_else_by_an_error() {
    assert_eq!(
        refusal(&set_to("band"), REFUSED),
        server(json!({"type": "ack", "items": [
            {"key": "band|live_set tracks 0 mixer_device volume|value", "seq": 41, "error": "role"}]}))
    );
    let sets = client(json!({"type": "sets", "sets": [item("mbc", 1), item("band", 2)]}));
    assert_eq!(
        refusal(&sets, REFUSED),
        server(json!({"type": "ack", "items": [
            {"key": "mbc|live_set tracks 3|mute", "seq": 1, "error": "role"},
            {"key": "band|live_set tracks 3|mute", "seq": 2, "error": "role"}]}))
    );
    let cmd = client(json!({"type": "cmd", "id": "c1", "instance": "band", "commands": []}));
    assert_eq!(
        refusal(&cmd, REFUSED),
        server(json!({"type": "error", "id": "c1", "message": "role"}))
    );
    let hub = client(json!({"type": "set_hub", "key": "stage_aut", "value": true}));
    assert_eq!(
        refusal(&hub, REFUSED),
        server(json!({"type": "error", "message": "role"}))
    );
}
