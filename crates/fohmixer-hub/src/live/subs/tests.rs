use super::*;

mod live_value;
mod watches;

const VOLUME: &str = "live_set tracks[name=Hand1 #] mixer_device volume";
const VOLUME_KEY: &str = "band|live_set tracks[name=Hand1 #] mixer_device volume|value|true";
const NAME_GUARD: &str = "band|live_set tracks[name=Hand1 #]|name|false|guard";
const LIST_GUARD: &str = "band|live_set|tracks|false|guard";

fn table() -> Subs {
    Subs::new(["band".to_string(), "master".to_string()])
}

fn online() -> Subs {
    let mut subs = table();
    subs.connected("band");
    subs.connected("master");
    subs
}

fn ok(key: &str, value: Value) -> Value {
    json!({"ok": true, "data": {"key": key, "value": value}})
}

fn ok_display(key: &str, value: Value, display: &str) -> Value {
    json!({"ok": true, "data": {"key": key, "value": value, "display": display}})
}

fn fail(error: &str) -> Value {
    json!({"ok": false, "error": error, "errorType": "PathError"})
}

fn push(key: &str, value: Value) -> LiveValue {
    LiveValue {
        key: key.to_string(),
        value,
        display: None,
        error: None,
    }
}

fn gone(key: &str) -> LiveValue {
    LiveValue {
        key: key.to_string(),
        value: Value::Null,
        display: None,
        error: Some("gone".to_string()),
    }
}

/// The (target, name, prop) of every command sent.
fn commands(out: &[Outgoing]) -> Vec<(String, String, String)> {
    out.iter()
        .flat_map(|o| o.commands.iter())
        .map(|c| {
            let target = match &c["target"] {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            (
                target,
                c["name"].as_str().unwrap().to_string(),
                c["args"]["prop"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// Answers every add_listener with a key derived from its target and
/// prop, the given value for the volume and names for the guards.
fn answer_all(subs: &mut Subs, out: &[Outgoing]) {
    for o in out {
        let slots: Vec<Value> = o
            .commands
            .iter()
            .map(|c| {
                let target = c["target"].as_str().unwrap_or("");
                let prop = c["args"]["prop"].as_str().unwrap();
                match (target, prop) {
                    (VOLUME, "value") => ok_display("live_10.value", json!(0.85), "0.0 dB"),
                    ("live_set tracks[name=Hand1 #]", "name") => {
                        ok("live_11.name", json!("Hand1 #"))
                    }
                    ("live_set", "tracks") => ok("live_1.tracks", json!([])),
                    ("live_set", "is_playing") => ok("live_1.is_playing", json!(false)),
                    _ => json!({"ok": true, "data": null}),
                }
            })
            .collect();
        subs.on_result(&o.instance, &o.uuid, &slots);
    }
}

fn sub_volume(subs: &mut Subs, client: ClientId) -> SubReply {
    subs.subscribe(client, "band", VOLUME, "value", true)
        .unwrap()
}

#[test]
fn the_first_subscriber_resolves_the_path_and_its_guards() {
    let mut subs = online();
    let reply = sub_volume(&mut subs, 1);
    assert_eq!(reply.key, VOLUME_KEY);
    assert_eq!(reply.cached, None);
    let out = subs.drain_outgoing();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].instance, "band");
    let mut sent = commands(&out);
    sent.sort();
    assert_eq!(
        sent,
        vec![
            ("live_set".into(), "add_listener".into(), "tracks".into()),
            (
                "live_set tracks[name=Hand1 #]".into(),
                "add_listener".into(),
                "name".into()
            ),
            (VOLUME.into(), "add_listener".into(), "value".into()),
        ]
    );
    let volume = out[0]
        .commands
        .iter()
        .find(|c| c["target"] == VOLUME)
        .unwrap();
    assert_eq!(volume["args"], json!({"prop": "value", "display": true}));
    let guard = out[0]
        .commands
        .iter()
        .find(|c| c["target"] == "live_set")
        .unwrap();
    assert_eq!(
        guard["args"],
        json!({"prop": "tracks"}),
        "no display for guards"
    );
    assert!(subs.drain_outgoing().is_empty(), "sent once");
    answer_all(&mut subs, &out);
    assert_eq!(
        subs.take_deliveries(),
        vec![(
            1,
            ValueItem::value(VOLUME_KEY, json!(0.85), Some("0.0 dB".into()))
        )]
    );
    assert_eq!(subs.subscriptions("band"), 1);
    assert_eq!(subs.listeners("band"), 3);
    assert_eq!(subs.listeners("master"), 0);
    assert_eq!(
        subs.cached(VOLUME_KEY),
        Some(&Cached::Value {
            value: json!(0.85),
            display: Some("0.0 dB".into())
        })
    );
}

#[test]
fn a_second_subscriber_shares_the_listener_and_gets_the_cached_value() {
    let mut subs = online();
    sub_volume(&mut subs, 1);
    let out = subs.drain_outgoing();
    answer_all(&mut subs, &out);
    subs.take_deliveries();
    let reply = sub_volume(&mut subs, 2);
    assert_eq!(
        reply.cached,
        Some(Cached::Value {
            value: json!(0.85),
            display: Some("0.0 dB".into())
        })
    );
    // A name binding is resolved again for its new subscriber (a page switch
    // or a reconnect finds a renamed or ambiguous name this way): the same key comes back,
    // no second listener, and an unchanged value is no news.
    let again = subs.drain_outgoing();
    assert_eq!(
        commands(&again),
        vec![(VOLUME.into(), "add_listener".into(), "value".into())]
    );
    answer_all(&mut subs, &again);
    assert!(subs.take_deliveries().is_empty());
    assert_eq!(subs.listeners("band"), 3);
    assert_eq!(subs.subscriptions("band"), 1);
    subs.on_values("band", &[push("live_10.value", json!(0.5))]);
    let mut got = subs.take_deliveries();
    got.sort_by_key(|(c, _)| *c);
    assert_eq!(
        got,
        vec![
            (1, ValueItem::value(VOLUME_KEY, json!(0.5), None)),
            (2, ValueItem::value(VOLUME_KEY, json!(0.5), None)),
        ]
    );
    // The same value again is no news.
    subs.on_values("band", &[push("live_10.value", json!(0.5))]);
    assert!(subs.take_deliveries().is_empty());
}

#[test]
fn the_last_unsubscribe_removes_the_listeners() {
    let mut subs = online();
    sub_volume(&mut subs, 1);
    sub_volume(&mut subs, 2);
    let out = subs.drain_outgoing();
    answer_all(&mut subs, &out);
    assert!(subs.unsubscribe(1, VOLUME_KEY));
    assert!(!subs.unsubscribe(1, VOLUME_KEY), "already gone");
    assert!(!subs.unsubscribe(1, "band|nothing|x|false"));
    assert!(subs.drain_outgoing().is_empty(), "client 2 still listens");
    assert!(subs.unsubscribe(2, VOLUME_KEY));
    let out = subs.drain_outgoing();
    let mut sent = commands(&out);
    sent.sort();
    assert_eq!(
        sent,
        vec![
            (
                "{\"$ref\":\"live_1\"}".into(),
                "remove_listener".into(),
                "tracks".into()
            ),
            (
                "{\"$ref\":\"live_10\"}".into(),
                "remove_listener".into(),
                "value".into()
            ),
            (
                "{\"$ref\":\"live_11\"}".into(),
                "remove_listener".into(),
                "name".into()
            ),
        ]
    );
    assert_eq!(subs.subscriptions("band"), 0);
    assert_eq!(subs.listeners("band"), 0);
    assert!(subs.cached(VOLUME_KEY).is_none());
    let ok_all: Vec<Value> = out[0]
        .commands
        .iter()
        .map(|_| json!({"ok": true, "data": null}))
        .collect();
    assert!(subs.on_result("band", &out[0].uuid, &ok_all));
    assert!(
        !subs.on_result("band", &out[0].uuid, &ok_all),
        "answered once"
    );
    assert!(subs.drain_outgoing().is_empty());
}

#[test]
fn a_dropped_client_leaves_every_subscription() {
    let mut subs = online();
    sub_volume(&mut subs, 7);
    subs.subscribe(7, "band", "live_set", "is_playing", false)
        .unwrap();
    subs.subscribe(8, "band", "live_set", "is_playing", false)
        .unwrap();
    let out = subs.drain_outgoing();
    answer_all(&mut subs, &out);
    subs.drop_client(7);
    assert_eq!(subs.subscriptions("band"), 1, "client 8's is_playing stays");
    let out = subs.drain_outgoing();
    let mut sent: Vec<String> = commands(&out)
        .iter()
        .map(|(_, n, p)| format!("{n} {p}"))
        .collect();
    sent.sort();
    assert_eq!(
        sent,
        vec![
            "remove_listener name",
            "remove_listener tracks",
            "remove_listener value"
        ]
    );
}

#[test]
fn two_paths_to_one_object_share_its_live_key() {
    let mut subs = online();
    let by_name = subs.subscribe(
        1,
        "band",
        "live_set tracks[name=Hand1 #] mute",
        "mute",
        false,
    );
    assert!(by_name.is_ok());
    let by_name = subs
        .subscribe(1, "band", "live_set tracks[name=Hand1 #]", "mute", false)
        .unwrap();
    let by_index = subs
        .subscribe(2, "band", "live_set tracks 0", "mute", false)
        .unwrap();
    subs.unsubscribe(1, "band|live_set tracks[name=Hand1 #] mute|mute|false");
    let out = subs.drain_outgoing();
    let slots: Vec<Value> = out[0]
        .commands
        .iter()
        .map(|c| {
            match (
                c["target"].as_str().unwrap(),
                c["args"]["prop"].as_str().unwrap(),
            ) {
                ("live_set tracks[name=Hand1 #]", "mute") | ("live_set tracks 0", "mute") => {
                    ok("live_5.mute", json!(false))
                }
                ("live_set tracks[name=Hand1 #]", "name") => ok("live_5.name", json!("Hand1 #")),
                _ => ok("live_1.tracks", json!([])),
            }
        })
        .collect();
    subs.on_result("band", &out[0].uuid, &slots);
    assert_eq!(subs.listeners("band"), 3);
    subs.on_values("band", &[push("live_5.mute", json!(true))]);
    let mut got = subs.take_deliveries();
    got.sort_by_key(|(c, _)| *c);
    assert_eq!(
        got.iter()
            .filter(|(_, i)| i.value == Some(json!(true)))
            .count(),
        2
    );
    // One of the two leaves: the shared listener stays.
    subs.unsubscribe(2, &by_index.key);
    assert!(
        commands(&subs.drain_outgoing()).is_empty(),
        "live_5.mute is still held by the name path"
    );
    subs.unsubscribe(1, &by_name.key);
    let removed = commands(&subs.drain_outgoing());
    assert_eq!(removed.len(), 3, "{removed:?}");
}

#[test]
fn a_rename_turns_the_binding_into_an_error_and_a_rename_back_heals_it() {
    let mut subs = online();
    sub_volume(&mut subs, 1);
    let out = subs.drain_outgoing();
    answer_all(&mut subs, &out);
    subs.take_deliveries();
    // The track is renamed: its name guard fires.
    subs.on_values("band", &[push("live_11.name", json!("Hand9 #"))]);
    assert!(subs.take_deliveries().is_empty(), "guards deliver nothing");
    let out = subs.drain_outgoing();
    let mut sent = commands(&out);
    sent.sort();
    assert_eq!(sent.len(), 3, "the binding and both guards: {sent:?}");
    let slots: Vec<Value> = out[0]
        .commands
        .iter()
        .map(|c| match c["args"]["prop"].as_str().unwrap() {
            "tracks" => ok("live_1.tracks", json!([])),
            _ => fail("not found: tracks[name=Hand1 #]"),
        })
        .collect();
    subs.on_result("band", &out[0].uuid, &slots);
    assert_eq!(
        subs.take_deliveries(),
        vec![(
            1,
            ValueItem::error(VOLUME_KEY, "not found: tracks[name=Hand1 #]")
        )]
    );
    // The stale listener is removed; the name guard keeps its object.
    assert_eq!(
        commands(&subs.drain_outgoing()),
        vec![(
            "{\"$ref\":\"live_10\"}".into(),
            "remove_listener".into(),
            "value".into()
        )]
    );
    assert_eq!(subs.listeners("band"), 2);
    // A later push of the removed key is nobody's.
    subs.on_values("band", &[push("live_10.value", json!(0.1))]);
    assert!(subs.take_deliveries().is_empty());
    // Renamed back: the guard fires again and the binding resolves.
    subs.on_values("band", &[push("live_11.name", json!("Hand1 #"))]);
    let out = subs.drain_outgoing();
    answer_all(&mut subs, &out);
    assert_eq!(
        subs.take_deliveries(),
        vec![(
            1,
            ValueItem::value(VOLUME_KEY, json!(0.85), Some("0.0 dB".into()))
        )]
    );
}

#[test]
fn removals_wait_for_resolutions_in_flight() {
    let mut subs = online();
    let a = subs
        .subscribe(1, "band", "live_set tracks 0", "mute", false)
        .unwrap();
    let out = subs.drain_outgoing();
    subs.on_result("band", &out[0].uuid, &[ok("live_5.mute", json!(false))]);
    // B resolves to the same key while A leaves: its request is out first.
    subs.subscribe(2, "band", "live_set tracks 0", "solo", false)
        .unwrap();
    let resolving = subs.drain_outgoing();
    subs.unsubscribe(1, &a.key);
    assert!(
        subs.drain_outgoing().is_empty(),
        "no remove_listener while an add_listener is in flight"
    );
    subs.on_result(
        "band",
        &resolving[0].uuid,
        &[ok("live_5.solo", json!(false))],
    );
    assert_eq!(
        commands(&subs.drain_outgoing()),
        vec![(
            "{\"$ref\":\"live_5\"}".into(),
            "remove_listener".into(),
            "mute".into()
        )]
    );
}

#[test]
fn a_removal_is_cancelled_when_the_key_is_wanted_again() {
    let mut subs = online();
    let a = subs
        .subscribe(1, "band", "live_set tracks 0", "mute", false)
        .unwrap();
    let out = subs.drain_outgoing();
    subs.on_result("band", &out[0].uuid, &[ok("live_5.mute", json!(false))]);
    subs.subscribe(2, "band", "live_set tracks[name=Hand1 #]", "mute", false)
        .unwrap();
    let resolving = subs.drain_outgoing();
    subs.unsubscribe(1, &a.key);
    let slots: Vec<Value> = resolving[0]
        .commands
        .iter()
        .map(|c| match c["args"]["prop"].as_str().unwrap() {
            "mute" => ok("live_5.mute", json!(false)),
            "name" => ok("live_5.name", json!("Hand1 #")),
            _ => ok("live_1.tracks", json!([])),
        })
        .collect();
    subs.on_result("band", &resolving[0].uuid, &slots);
    assert!(
        commands(&subs.drain_outgoing()).is_empty(),
        "live_5.mute is held again"
    );
}

#[test]
fn a_stale_answer_is_ignored_and_an_unwanted_key_released() {
    let mut subs = online();
    let reply = subs
        .subscribe(1, "band", "live_set tracks 0", "mute", false)
        .unwrap();
    let first = subs.drain_outgoing();
    // Re-resolved (a reconnect) before the first answer arrived.
    subs.disconnected("band");
    subs.connected("band");
    let second = subs.drain_outgoing();
    assert!(
        !subs.on_result("band", &first[0].uuid, &[ok("live_9.mute", json!(true))]),
        "a disconnect forgot the old session's requests"
    );
    assert!(subs.take_deliveries().is_empty());
    subs.on_result("band", &second[0].uuid, &[ok("live_5.mute", json!(false))]);
    assert_eq!(
        subs.take_deliveries(),
        vec![(1, ValueItem::value(&reply.key, json!(false), None))]
    );
    // An answer for an entry that is gone releases its key.
    subs.subscribe(2, "band", "live_set tracks 1", "mute", false)
        .unwrap();
    let out = subs.drain_outgoing();
    subs.unsubscribe(2, "band|live_set tracks 1|mute|false");
    subs.on_result("band", &out[0].uuid, &[ok("live_6.mute", json!(false))]);
    assert_eq!(
        commands(&subs.drain_outgoing()),
        vec![(
            "{\"$ref\":\"live_6\"}".into(),
            "remove_listener".into(),
            "mute".into()
        )]
    );
    // A superseded request (a newer one of the same entry was sent after
    // it) does not win, whichever answer comes last.
    subs.on_values("band", &[]);
    let key = reply.key.clone();
    subs.connected("band");
    let older = subs.drain_outgoing();
    subs.connected("band");
    let newer = subs.drain_outgoing();
    subs.on_result("band", &newer[0].uuid, &[ok("live_5.mute", json!(true))]);
    subs.on_result("band", &older[0].uuid, &[ok("live_5.mute", json!(false))]);
    assert_eq!(
        subs.cached(&key),
        Some(&Cached::Value {
            value: json!(true),
            display: None
        })
    );
}

#[test]
fn a_removal_goes_out_while_another_instance_resolves() {
    let mut subs = online();
    let master = subs
        .subscribe(1, "master", "live_set tracks 0", "mute", false)
        .unwrap();
    let out = subs.drain_outgoing();
    subs.on_result("master", &out[0].uuid, &[ok("live_7.mute", json!(false))]);
    subs.unsubscribe(1, &master.key);
    // Band resolves in the same drain: master's removal is not held.
    subs.subscribe(2, "band", "live_set tracks 0", "mute", false)
        .unwrap();
    let out = subs.drain_outgoing();
    let by: Vec<(String, String)> = out
        .iter()
        .flat_map(|o| {
            o.commands
                .iter()
                .map(move |c| (o.instance.clone(), c["name"].as_str().unwrap().to_string()))
        })
        .collect();
    assert_eq!(
        by,
        vec![
            ("band".to_string(), "add_listener".to_string()),
            ("master".to_string(), "remove_listener".to_string()),
        ]
    );
    // A removal in flight does not hold the next removal either.
    let second = subs
        .subscribe(1, "master", "live_set tracks 1", "mute", false)
        .unwrap();
    let out = subs.drain_outgoing();
    let resolve = out.iter().find(|o| o.instance == "master").unwrap();
    subs.on_result("master", &resolve.uuid, &[ok("live_8.mute", json!(false))]);
    subs.unsubscribe(1, &second.key);
    assert_eq!(
        commands(&subs.drain_outgoing()),
        vec![(
            "{\"$ref\":\"live_8\"}".into(),
            "remove_listener".into(),
            "mute".into()
        )]
    );
}

#[test]
fn a_subscription_to_a_tracks_own_name_errs_on_a_rename_and_heals_back() {
    let mut subs = online();
    let name = subs
        .subscribe(1, "band", "live_set tracks[name=Keys]", "name", false)
        .unwrap();
    let out = subs.drain_outgoing();
    assert_eq!(
        out[0].commands.len(),
        3,
        "the name, its name guard and the track list"
    );
    let answer = |c: &Value| match c["args"]["prop"].as_str().unwrap() {
        "name" => ok("live_5.name", json!("Keys")),
        _ => ok("live_1.tracks", json!([])),
    };
    let slots: Vec<Value> = out[0].commands.iter().map(answer).collect();
    subs.on_result("band", &out[0].uuid, &slots);
    assert_eq!(subs.listeners("band"), 2, "the name listener is shared");
    assert_eq!(
        subs.take_deliveries(),
        vec![(1, ValueItem::value(&name.key, json!("Keys"), None))]
    );
    // Renamed: the new name is the renamed track's, never this binding's
    // value; resolved again, the binding no longer resolves.
    subs.on_values("band", &[push("live_5.name", json!("Other"))]);
    assert!(
        subs.take_deliveries().is_empty(),
        "{:?}",
        subs.cached(&name.key)
    );
    let out = subs.drain_outgoing();
    assert_eq!(out[0].commands.len(), 3, "itself, its guard, the list");
    let slots: Vec<Value> = out[0]
        .commands
        .iter()
        .map(|c| match c["args"]["prop"].as_str().unwrap() {
            "name" => fail("not found: tracks[name=Keys]"),
            _ => ok("live_1.tracks", json!([])),
        })
        .collect();
    subs.on_result("band", &out[0].uuid, &slots);
    assert_eq!(
        subs.take_deliveries(),
        vec![(
            1,
            ValueItem::error(&name.key, "not found: tracks[name=Keys]")
        )]
    );
    assert!(
        commands(&subs.drain_outgoing()).is_empty(),
        "the guard keeps the name listener"
    );
    // Renamed back: the guard fires and the binding heals.
    subs.on_values("band", &[push("live_5.name", json!("Keys"))]);
    let out = subs.drain_outgoing();
    let slots: Vec<Value> = out[0].commands.iter().map(answer).collect();
    subs.on_result("band", &out[0].uuid, &slots);
    assert_eq!(
        subs.take_deliveries(),
        vec![(1, ValueItem::value(&name.key, json!("Keys"), None))]
    );
    subs.unsubscribe(1, &name.key);
    assert_eq!(
        commands(&subs.drain_outgoing()).len(),
        2,
        "the name key and the track list"
    );
    assert_eq!(subs.listeners("band"), 0);
    assert_eq!(subs.subscriptions("band"), 0);
}

#[test]
fn a_value_in_the_frame_of_a_rename_is_not_the_bindings() {
    let mut subs = online();
    sub_volume(&mut subs, 1);
    let index = subs
        .subscribe(2, "band", "live_set tracks 0", "mute", false)
        .unwrap();
    let out = subs.drain_outgoing();
    let slots: Vec<Value> = out[0]
        .commands
        .iter()
        .map(|c| {
            if c["target"] == "live_set tracks 0" {
                ok("live_5.mute", json!(false))
            } else {
                match c["args"]["prop"].as_str().unwrap() {
                    "value" => ok_display("live_10.value", json!(0.85), "0.0 dB"),
                    "name" => ok("live_11.name", json!("Hand1 #")),
                    _ => ok("live_1.tracks", json!([])),
                }
            }
        })
        .collect();
    subs.on_result("band", &out[0].uuid, &slots);
    subs.take_deliveries();
    // The renamed track's volume moves in the frame of the rename: not the
    // name binding's value. An index binding's value in that frame is.
    subs.on_values(
        "band",
        &[
            push("live_10.value", json!(0.2)),
            push("live_5.mute", json!(true)),
            push("live_11.name", json!("Hand9 #")),
        ],
    );
    assert_eq!(
        subs.take_deliveries(),
        vec![(2, ValueItem::value(&index.key, json!(true), None))]
    );
    let out = subs.drain_outgoing();
    let slots: Vec<Value> = out[0]
        .commands
        .iter()
        .map(|c| match c["args"]["prop"].as_str().unwrap() {
            "tracks" => ok("live_1.tracks", json!([])),
            _ => fail("not found: tracks[name=Hand1 #]"),
        })
        .collect();
    subs.on_result("band", &out[0].uuid, &slots);
    assert_eq!(
        subs.take_deliveries(),
        vec![(
            1,
            ValueItem::error(VOLUME_KEY, "not found: tracks[name=Hand1 #]")
        )]
    );
}

#[test]
fn an_answer_for_a_dropped_entry_is_not_its_successors() {
    let mut subs = online();
    let first = subs
        .subscribe(1, "band", "live_set tracks 0", "mute", false)
        .unwrap();
    let old = subs.drain_outgoing();
    subs.unsubscribe(1, &first.key);
    subs.subscribe(1, "band", "live_set tracks 0", "mute", false)
        .unwrap();
    let new = subs.drain_outgoing();
    // The dropped entry's answer arrives first: the new entry waits for its
    // own.
    subs.on_result("band", &old[0].uuid, &[ok("live_5.mute", json!(true))]);
    assert!(subs.take_deliveries().is_empty());
    subs.on_result("band", &new[0].uuid, &[ok("live_5.mute", json!(false))]);
    assert_eq!(
        subs.take_deliveries(),
        vec![(1, ValueItem::value(&first.key, json!(false), None))]
    );
    assert!(
        commands(&subs.drain_outgoing()).is_empty(),
        "the key is held again: no removal"
    );
}

#[test]
fn a_second_subscriber_of_an_index_binding_sends_nothing_unless_in_error() {
    let mut subs = online();
    subs.subscribe(1, "band", "live_set tracks 0", "mute", false)
        .unwrap();
    let out = subs.drain_outgoing();
    subs.on_result("band", &out[0].uuid, &[ok("live_5.mute", json!(false))]);
    let second = subs
        .subscribe(2, "band", "live_set tracks 0", "mute", false)
        .unwrap();
    assert_eq!(
        second.cached,
        Some(Cached::Value {
            value: json!(false),
            display: None
        })
    );
    assert!(subs.drain_outgoing().is_empty(), "no second add_listener");
    // The track goes: a later subscriber resolves the binding again.
    subs.on_values("band", &[gone("live_5.mute")]);
    subs.subscribe(3, "band", "live_set tracks 0", "mute", false)
        .unwrap();
    assert_eq!(
        commands(&subs.drain_outgoing()),
        vec![(
            "live_set tracks 0".into(),
            "add_listener".into(),
            "mute".into()
        )]
    );
}

#[test]
fn a_guard_re_resolves_only_its_instances_name_bindings() {
    let mut subs = online();
    sub_volume(&mut subs, 1);
    subs.subscribe(1, "band", "live_set tracks 0", "mute", false)
        .unwrap();
    subs.subscribe(1, "master", "live_set tracks[name=Hand1 #]", "mute", false)
        .unwrap();
    let out = subs.drain_outgoing();
    for o in &out {
        let slots: Vec<Value> = o
            .commands
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let prop = c["args"]["prop"].as_str().unwrap();
                let key = if c["target"] == VOLUME {
                    "live_10.value".to_string()
                } else if prop == "name" && o.instance == "band" {
                    "live_11.name".to_string()
                } else {
                    format!("live_{}{i}.{prop}", o.instance.len())
                };
                ok(&key, json!(0))
            })
            .collect();
        subs.on_result(&o.instance, &o.uuid, &slots);
    }
    subs.on_values("band", &[push("live_11.name", json!("Hand9 #"))]);
    let out = subs.drain_outgoing();
    assert!(out.iter().all(|o| o.instance == "band"), "{out:?}");
    let mut sent: Vec<String> = commands(&out)
        .into_iter()
        .map(|(t, _, p)| format!("{t} {p}"))
        .collect();
    sent.sort();
    let mut expected = vec![
        "live_set tracks".to_string(),
        "live_set tracks[name=Hand1 #] name".to_string(),
        format!("{VOLUME} value"),
    ];
    expected.sort();
    assert_eq!(sent, expected, "not the index binding");
}

#[test]
fn a_superseded_answer_of_the_same_session_keeps_the_newer_state() {
    let mut subs = online();
    let reply = subs
        .subscribe(1, "band", "live_set tracks[name=Keys]", "mute", false)
        .unwrap();
    let out = subs.drain_outgoing();
    let answer = |c: &Value| match c["args"]["prop"].as_str().unwrap() {
        "mute" => ok("live_5.mute", json!(false)),
        "name" => ok("live_5.name", json!("Keys")),
        _ => ok("live_1.tracks", json!([])),
    };
    let slots: Vec<Value> = out[0].commands.iter().map(answer).collect();
    subs.on_result("band", &out[0].uuid, &slots);
    // Two list changes: two re-resolutions in flight.
    subs.on_values("band", &[push("live_1.tracks", json!([1]))]);
    let older = subs.drain_outgoing();
    subs.on_values("band", &[push("live_1.tracks", json!([1, 2]))]);
    let newer = subs.drain_outgoing();
    assert!(!older.is_empty() && !newer.is_empty());
    let newer_slots: Vec<Value> = newer[0].commands.iter().map(answer).collect();
    subs.on_result("band", &newer[0].uuid, &newer_slots);
    let older_slots: Vec<Value> = older[0]
        .commands
        .iter()
        .map(|c| match c["args"]["prop"].as_str().unwrap() {
            "mute" => ok("live_8.mute", json!(true)),
            _ => answer(c),
        })
        .collect();
    subs.on_result("band", &older[0].uuid, &older_slots);
    assert_eq!(
        subs.cached(&reply.key),
        Some(&Cached::Value {
            value: json!(false),
            display: None
        })
    );
    assert_eq!(
        commands(&subs.drain_outgoing()),
        vec![(
            "{\"$ref\":\"live_8\"}".into(),
            "remove_listener".into(),
            "mute".into()
        )],
        "the superseded answer's key is nobody's"
    );
}

#[test]
fn offline_subscriptions_resolve_on_connect_and_a_disconnect_clears_values() {
    let mut subs = table();
    let reply = subs
        .subscribe(1, "band", "live_set", "is_playing", false)
        .unwrap();
    assert!(subs.drain_outgoing().is_empty(), "offline: nothing sent");
    assert!(!subs.is_online("band"));
    subs.connected("band");
    assert!(subs.is_online("band"));
    let out = subs.drain_outgoing();
    assert_eq!(
        commands(&out),
        vec![(
            "live_set".into(),
            "add_listener".into(),
            "is_playing".into()
        )]
    );
    answer_all(&mut subs, &out);
    assert_eq!(
        subs.take_deliveries(),
        vec![(1, ValueItem::value(&reply.key, json!(false), None))]
    );
    subs.disconnected("band");
    assert!(!subs.is_online("band"));
    assert!(
        subs.cached(&reply.key).is_none(),
        "no stale value while offline"
    );
    assert_eq!(subs.listeners("band"), 0);
    subs.on_values("band", &[push("live_1.is_playing", json!(true))]);
    assert!(
        subs.take_deliveries().is_empty(),
        "offline pushes are ignored"
    );
    // A new subscriber while offline gets no value.
    assert_eq!(
        subs.subscribe(2, "band", "live_set", "is_playing", false)
            .unwrap()
            .cached,
        None
    );
    subs.connected("band");
    let out = subs.drain_outgoing();
    assert_eq!(out.len(), 1);
    answer_all(&mut subs, &out);
    let mut got = subs.take_deliveries();
    got.sort_by_key(|(c, _)| *c);
    assert_eq!(got.len(), 2, "both get the fresh value");
    // Unknown instances are no-ops.
    subs.connected("nowhere");
    subs.disconnected("nowhere");
    assert!(!subs.is_online("nowhere"));
}

#[test]
fn a_disconnect_drops_the_instances_requests_in_flight() {
    let mut subs = online();
    subs.subscribe(1, "band", "live_set", "is_playing", false)
        .unwrap();
    subs.subscribe(1, "master", "live_set", "is_playing", false)
        .unwrap();
    let out = subs.drain_outgoing();
    assert_eq!(out.len(), 2);
    subs.disconnected("band");
    let band = out.iter().find(|o| o.instance == "band").unwrap();
    let master = out.iter().find(|o| o.instance == "master").unwrap();
    assert!(!subs.on_result("band", &band.uuid, &[ok("live_1.is_playing", json!(true))]));
    assert!(subs.on_result(
        "master",
        &master.uuid,
        &[ok("live_1.is_playing", json!(true))]
    ));
    assert_eq!(subs.take_deliveries().len(), 1);
    // A uuid answered by the wrong instance is dropped.
    subs.connected("band");
    let again = subs.drain_outgoing();
    assert!(subs.on_result(
        "master",
        &again[0].uuid,
        &[ok("live_1.is_playing", json!(true))]
    ));
    assert!(subs.take_deliveries().is_empty());
}

#[test]
fn a_gone_object_is_an_error_without_a_removal() {
    let mut subs = online();
    let reply = subs
        .subscribe(1, "band", "live_set tracks 0", "mute", false)
        .unwrap();
    let out = subs.drain_outgoing();
    subs.on_result("band", &out[0].uuid, &[ok("live_5.mute", json!(false))]);
    subs.take_deliveries();
    subs.on_values("band", &[gone("live_5.mute")]);
    assert_eq!(
        subs.take_deliveries(),
        vec![(1, ValueItem::error(&reply.key, "gone"))]
    );
    assert_eq!(subs.listeners("band"), 0);
    assert!(
        subs.drain_outgoing().is_empty(),
        "the script dropped it itself"
    );
    // A gone guard re-resolves the name bindings.
    sub_volume(&mut subs, 2);
    let out = subs.drain_outgoing();
    answer_all(&mut subs, &out);
    subs.on_values("band", &[gone("live_11.name")]);
    let sent = commands(&subs.drain_outgoing());
    assert!(sent.iter().any(|(t, _, _)| t == VOLUME), "{sent:?}");
}

#[test]
fn a_failed_first_resolution_is_an_error_for_the_subscribers() {
    let mut subs = online();
    let reply = subs
        .subscribe(1, "band", "live_set tracks 99", "mute", false)
        .unwrap();
    let out = subs.drain_outgoing();
    subs.on_result("band", &out[0].uuid, &[fail("not found: tracks 99")]);
    assert_eq!(
        subs.take_deliveries(),
        vec![(1, ValueItem::error(&reply.key, "not found: tracks 99"))]
    );
    let again = subs
        .subscribe(2, "band", "live_set tracks 99", "mute", false)
        .unwrap();
    assert_eq!(
        again.cached,
        Some(Cached::Error("not found: tracks 99".into()))
    );
    // A binding in error is resolved again for its new subscriber.
    let retry = subs.drain_outgoing();
    assert_eq!(
        commands(&retry),
        vec![(
            "live_set tracks 99".into(),
            "add_listener".into(),
            "mute".into()
        )]
    );
    subs.on_result("band", &retry[0].uuid, &[fail("not found: tracks 99")]);
    assert!(
        subs.take_deliveries().is_empty(),
        "the same error is no news"
    );
    // A missing slot and a slot without a key are errors too.
    let reply = subs
        .subscribe(3, "band", "live_set tracks 1", "mute", false)
        .unwrap();
    let out = subs.drain_outgoing();
    subs.on_result("band", &out[0].uuid, &[]);
    assert_eq!(
        subs.take_deliveries(),
        vec![(3, ValueItem::error(&reply.key, "no result"))]
    );
    subs.on_values("band", &[]);
    subs.connected("band");
    let out = subs.drain_outgoing();
    let slots: Vec<Value> = out[0]
        .commands
        .iter()
        .map(|_| json!({"ok": true, "data": {"value": 1}}))
        .collect();
    subs.on_result("band", &out[0].uuid, &slots);
    let got = subs.take_deliveries();
    assert!(
        got.iter()
            .any(|(_, i)| i.error.as_deref() == Some("add_listener answered without a key")),
        "{got:?}"
    );
    assert_eq!(
        subs.cached(&reply.key),
        Some(&Cached::Error("add_listener answered without a key".into()))
    );
}

#[test]
fn a_display_string_goes_only_to_keys_that_asked_for_one() {
    let mut subs = online();
    let plain = subs
        .subscribe(
            1,
            "band",
            "live_set tracks 0 mixer_device volume",
            "value",
            false,
        )
        .unwrap();
    let out = subs.drain_outgoing();
    assert_eq!(out[0].commands[0]["args"], json!({"prop": "value"}));
    subs.on_result(
        "band",
        &out[0].uuid,
        &[ok_display("live_10.value", json!(0.85), "0.0 dB")],
    );
    assert_eq!(
        subs.take_deliveries(),
        vec![(1, ValueItem::value(&plain.key, json!(0.85), None))]
    );
    subs.on_values(
        "band",
        &[LiveValue {
            key: "live_10.value".into(),
            value: json!(0.5),
            display: Some("-6.0 dB".into()),
            error: None,
        }],
    );
    assert_eq!(
        subs.take_deliveries(),
        vec![(1, ValueItem::value(&plain.key, json!(0.5), None))]
    );
}

#[test]
fn bad_requests_are_refused_with_their_key() {
    let mut subs = online();
    assert_eq!(
        subs.subscribe(1, "drums", "live_set", "is_playing", false),
        Err((
            "drums|live_set|is_playing|false".into(),
            "unknown instance \"drums\"".into()
        ))
    );
    assert_eq!(
        subs.subscribe(1, "band", "live_set", "", false)
            .unwrap_err()
            .1,
        "bad prop \"\""
    );
    assert_eq!(
        subs.subscribe(1, "band", "live_set", "_x", false)
            .unwrap_err()
            .1,
        "bad prop \"_x\""
    );
    assert_eq!(
        subs.subscribe(1, "band", "live_set tracks[name=x", "mute", false),
        Err((
            "band|live_set tracks[name=x|mute|false".into(),
            "syntax: unterminated [name=".into()
        ))
    );
    assert_eq!(subs.subscriptions("band"), 0);
    assert!(subs.drain_outgoing().is_empty());
}

#[test]
fn requests_are_split_into_batches() {
    let mut subs = online();
    for i in 0..(BATCH_MAX + 6) {
        subs.subscribe(1, "band", &format!("live_set tracks {i}"), "mute", false)
            .unwrap();
    }
    let out = subs.drain_outgoing();
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].commands.len(), BATCH_MAX);
    assert_eq!(out[1].commands.len(), 6);
    assert_ne!(out[0].uuid, out[1].uuid);
}

#[test]
fn a_client_on_a_guards_target_has_its_own_entry_and_both_heal() {
    let mut subs = online();
    sub_volume(&mut subs, 1);
    // A client also listens to the track's name: what the volume's name
    // guard watches, in an entry of its own.
    let name = subs
        .subscribe(2, "band", "live_set tracks[name=Hand1 #]", "name", false)
        .unwrap();
    assert_eq!(name.key, "band|live_set tracks[name=Hand1 #]|name|false");
    assert_ne!(name.key, NAME_GUARD);
    let out = subs.drain_outgoing();
    answer_all(&mut subs, &out);
    assert_eq!(subs.subscriptions("band"), 2);
    assert_eq!(subs.listeners("band"), 3, "the name listener is shared");
    subs.take_deliveries();
    // Renamed: both bindings fail; the guard keeps its object.
    subs.on_values("band", &[push("live_11.name", json!("Other"))]);
    assert!(
        subs.take_deliveries().is_empty(),
        "no value of the renamed track"
    );
    let out = subs.drain_outgoing();
    let slots: Vec<Value> = out[0]
        .commands
        .iter()
        .map(|c| match c["args"]["prop"].as_str().unwrap() {
            "tracks" => ok("live_1.tracks", json!([])),
            _ => fail("not found: tracks[name=Hand1 #]"),
        })
        .collect();
    subs.on_result("band", &out[0].uuid, &slots);
    let mut got = subs.take_deliveries();
    got.sort_by_key(|(c, _)| *c);
    assert_eq!(
        got,
        vec![
            (
                1,
                ValueItem::error(VOLUME_KEY, "not found: tracks[name=Hand1 #]")
            ),
            (
                2,
                ValueItem::error(&name.key, "not found: tracks[name=Hand1 #]")
            ),
        ]
    );
    // The volume's key goes; the name key stays with the guard.
    assert_eq!(
        commands(&subs.drain_outgoing()),
        vec![(
            "{\"$ref\":\"live_10\"}".into(),
            "remove_listener".into(),
            "value".into()
        )]
    );
    assert_eq!(subs.listeners("band"), 2);
    assert!(subs.cached(LIST_GUARD).is_some());
    // Renamed back: the guard fires and both heal.
    subs.on_values("band", &[push("live_11.name", json!("Hand1 #"))]);
    let out = subs.drain_outgoing();
    answer_all(&mut subs, &out);
    let mut got = subs.take_deliveries();
    got.sort_by_key(|(c, _)| *c);
    assert_eq!(
        got,
        vec![
            (
                1,
                ValueItem::value(VOLUME_KEY, json!(0.85), Some("0.0 dB".into()))
            ),
            (2, ValueItem::value(&name.key, json!("Hand1 #"), None)),
        ]
    );
    // Both leave: every listener goes.
    subs.unsubscribe(1, VOLUME_KEY);
    subs.unsubscribe(2, &name.key);
    assert_eq!(commands(&subs.drain_outgoing()).len(), 3);
    assert_eq!(subs.listeners("band"), 0);
    assert!(subs.cached(NAME_GUARD).is_none());
}

#[test]
fn a_guard_key_is_never_a_clients() {
    assert_eq!(
        guard_key("band", "live_set tracks[name=A]", "name"),
        "band|live_set tracks[name=A]|name|false|guard"
    );
    let mut subs = online();
    sub_volume(&mut subs, 1);
    assert!(
        !subs.unsubscribe(1, LIST_GUARD),
        "a guard has no clients to leave"
    );
    assert_eq!(subs.subscriptions("band"), 1);
    assert_eq!(commands(&subs.drain_outgoing()).len(), 3, "nothing dropped");
}

#[test]
fn guard_targets_cover_every_name_step() {
    let path = LomPath::parse("live_set tracks[name=A] devices[name=B] parameters 1").unwrap();
    assert_eq!(
        guard_targets(&path),
        vec![
            ("live_set".to_string(), "tracks".to_string()),
            ("live_set tracks[name=A]".to_string(), "name".to_string()),
            ("live_set tracks[name=A]".to_string(), "devices".to_string()),
            (
                "live_set tracks[name=A] devices[name=B]".to_string(),
                "name".to_string()
            ),
        ]
    );
    assert!(guard_targets(&LomPath::parse("live_set tracks 0 mute").unwrap()).is_empty());
}

#[test]
fn cached_states_become_client_items() {
    assert_eq!(
        Cached::Value {
            value: json!(1),
            display: Some("x".into())
        }
        .item("k"),
        ValueItem::value("k", json!(1), Some("x".into()))
    );
    assert_eq!(
        Cached::Error("e".into()).item("k"),
        ValueItem::error("k", "e")
    );
}

#[test]
fn slots_parse() {
    assert_eq!(
        parse_slot(&ok_display("live_1.value", json!(0.5), "x")),
        Ok((
            "live_1.value".into(),
            Cached::Value {
                value: json!(0.5),
                display: Some("x".into())
            }
        ))
    );
    assert_eq!(
        parse_slot(&json!({"ok": true, "data": {"key": "k"}})),
        Ok((
            "k".into(),
            Cached::Value {
                value: Value::Null,
                display: None
            }
        ))
    );
    assert_eq!(parse_slot(&fail("nope")), Err("nope".into()));
    assert_eq!(parse_slot(&json!({"ok": false})), Err("no result".into()));
    assert_eq!(parse_slot(&json!({})), Err("no result".into()));
    assert_eq!(
        parse_slot(&json!({"ok": true})),
        Err("add_listener answered without a key".into())
    );
}
