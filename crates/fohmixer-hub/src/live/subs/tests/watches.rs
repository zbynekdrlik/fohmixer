//! The item name watches (#58): while a binding a list guard guards is in
//! error, the hub watches the `name` of every item of that list, so a
//! binding made while its name was missing heals when a track is renamed to
//! it (the case only REFRESH ALL healed before).

use super::*;

/// A track in the list guard's value (the script's encoding of an object).
fn track(id: &str, name: &str) -> Value {
    json!({"$ref": id, "path": "live_set tracks", "class": "Track", "name": name})
}

/// Answers every request: `tracks` with `list`, the binding and its name
/// guard as found when `found`, else missing, and an item's `name` with a
/// key of its own (`live_2<index>.name`).
fn answer(subs: &mut Subs, out: &[Outgoing], list: &Value, found: bool) {
    for o in out {
        let slots: Vec<Value> = o
            .commands
            .iter()
            .map(|c| {
                let target = c["target"].as_str().unwrap_or("");
                let prop = c["args"]["prop"].as_str().unwrap();
                match (target, prop) {
                    ("live_set", "tracks") => ok("live_1.tracks", list.clone()),
                    (VOLUME, "value") if found => {
                        ok_display("live_10.value", json!(0.85), "0.0 dB")
                    }
                    ("live_set tracks[name=Hand1 #]", "name") if found => {
                        ok("live_20.name", json!("Hand1 #"))
                    }
                    (VOLUME, "value") | ("live_set tracks[name=Hand1 #]", "name") => {
                        fail("not found: tracks[name=Hand1 #]")
                    }
                    (item, "name") => {
                        let index = item.rsplit(' ').next().unwrap();
                        ok(&format!("live_2{index}.name"), json!("other"))
                    }
                    _ => json!({"ok": true, "data": null}),
                }
            })
            .collect();
        subs.on_result(&o.instance, &o.uuid, &slots);
    }
}

/// The sorted targets of the `add_listener`s of `name` sent.
fn watched(out: &[Outgoing]) -> Vec<String> {
    let mut targets: Vec<String> = commands(out)
        .into_iter()
        .filter(|(t, n, p)| n == "add_listener" && p == "name" && !t.contains("[name="))
        .map(|(t, _, _)| t)
        .collect();
    targets.sort();
    targets
}

/// The sorted Live keys of the `remove_listener`s sent.
fn removed(out: &[Outgoing]) -> Vec<String> {
    let mut keys: Vec<String> = out
        .iter()
        .flat_map(|o| o.commands.iter())
        .filter(|c| c["name"] == "remove_listener")
        .map(|c| {
            format!(
                "{}.{}",
                c["target"]["$ref"].as_str().unwrap(),
                c["args"]["prop"].as_str().unwrap()
            )
        })
        .collect();
    keys.sort();
    keys
}

/// A volume subscription made while its track is missing, with a list of
/// two other tracks: the first answers come back, nothing else is sent.
fn missing(subs: &mut Subs) -> Value {
    let list = json!([track("live_20", "Hand9 #"), track("live_21", "Vox 1")]);
    sub_volume(subs, 1);
    let out = subs.drain_outgoing();
    answer(subs, &out, &list, false);
    assert_eq!(
        subs.take_deliveries(),
        vec![(
            1,
            ValueItem::error(VOLUME_KEY, "not found: tracks[name=Hand1 #]")
        )]
    );
    list
}

#[test]
fn a_binding_made_while_its_name_is_missing_heals_when_a_track_is_renamed_to_it() {
    let mut subs = online();
    let list = missing(&mut subs);
    // In error: every item's name is watched, and only that is sent.
    let out = subs.drain_outgoing();
    assert_eq!(
        watched(&out),
        vec!["live_set tracks 0", "live_set tracks 1"]
    );
    assert_eq!(commands(&out).len(), 2, "{:?}", commands(&out));
    answer(&mut subs, &out, &list, false);
    assert!(subs.drain_outgoing().is_empty(), "watched once");
    assert_eq!(subs.listeners("band"), 3, "the list and its two items");
    // Track 0 is renamed to the binding's name: every name binding and
    // guard resolves again, and the binding heals.
    subs.on_values("band", &[push("live_20.name", json!("Hand1 #"))]);
    assert!(subs.take_deliveries().is_empty(), "guards deliver nothing");
    let out = subs.drain_outgoing();
    let list = json!([track("live_20", "Hand1 #"), track("live_21", "Vox 1")]);
    answer(&mut subs, &out, &list, true);
    assert_eq!(
        subs.take_deliveries(),
        vec![(
            1,
            ValueItem::value(VOLUME_KEY, json!(0.85), Some("0.0 dB".into()))
        )]
    );
    // Healed: the watches go. Track 0's name stays heard (the name guard
    // holds it now), track 1's is removed.
    let out = subs.drain_outgoing();
    assert_eq!(watched(&out), Vec::<String>::new());
    assert_eq!(removed(&out), vec!["live_21.name"]);
    assert_eq!(subs.listeners("band"), 3, "the list, the name, the volume");
}

#[test]
fn the_watches_follow_the_lists_length_and_go_with_the_subscriber() {
    let mut subs = online();
    let list = missing(&mut subs);
    let out = subs.drain_outgoing();
    answer(&mut subs, &out, &list, false);
    // A track is added: the list guard fires, and a third item is watched.
    let longer = json!([
        track("live_20", "Hand9 #"),
        track("live_21", "Vox 1"),
        track("live_22", "Keys")
    ]);
    subs.on_values("band", &[push("live_1.tracks", longer.clone())]);
    let out = subs.drain_outgoing();
    assert_eq!(
        watched(&out),
        vec![
            "live_set tracks 0",
            "live_set tracks 1",
            "live_set tracks 2"
        ],
        "every item again (the list changed under the indices), and the new one"
    );
    answer(&mut subs, &out, &longer, false);
    assert!(subs.drain_outgoing().is_empty());
    assert_eq!(subs.listeners("band"), 4);
    // Two tracks are deleted: their watches go, and the one left is
    // resolved again with the rest.
    let shorter = json!([track("live_20", "Hand9 #")]);
    subs.on_values("band", &[push("live_1.tracks", shorter.clone())]);
    let out = subs.drain_outgoing();
    assert_eq!(removed(&out), vec!["live_21.name", "live_22.name"]);
    assert_eq!(watched(&out), vec!["live_set tracks 0"]);
    answer(&mut subs, &out, &shorter, false);
    assert!(subs.drain_outgoing().is_empty());
    assert_eq!(subs.listeners("band"), 2, "the list and its one item");
    // The subscriber leaves: everything goes, the watch with it.
    assert!(subs.unsubscribe(1, VOLUME_KEY));
    let out = subs.drain_outgoing();
    assert_eq!(removed(&out), vec!["live_1.tracks", "live_20.name"]);
    assert_eq!(subs.listeners("band"), 0);
}

#[test]
fn a_binding_that_resolves_watches_nothing() {
    let mut subs = online();
    let list = json!([track("live_20", "Hand1 #"), track("live_21", "Vox 1")]);
    sub_volume(&mut subs, 1);
    let out = subs.drain_outgoing();
    answer(&mut subs, &out, &list, true);
    assert!(subs.drain_outgoing().is_empty(), "no watches");
    assert_eq!(subs.listeners("band"), 3);
}

#[test]
fn a_rename_away_watches_the_items_and_the_rename_back_ends_it() {
    let mut subs = online();
    let list = json!([track("live_20", "Hand1 #"), track("live_21", "Vox 1")]);
    sub_volume(&mut subs, 1);
    let out = subs.drain_outgoing();
    answer(&mut subs, &out, &list, true);
    subs.take_deliveries();
    // Renamed away: the list itself does not change, the binding errs, and
    // that alone starts the watches.
    subs.on_values("band", &[push("live_20.name", json!("Hand9 #"))]);
    let out = subs.drain_outgoing();
    assert_eq!(watched(&out), Vec::<String>::new());
    answer(&mut subs, &out, &list, false);
    assert_eq!(
        subs.take_deliveries(),
        vec![(
            1,
            ValueItem::error(VOLUME_KEY, "not found: tracks[name=Hand1 #]")
        )]
    );
    let out = subs.drain_outgoing();
    assert_eq!(
        watched(&out),
        vec!["live_set tracks 0", "live_set tracks 1"]
    );
    answer(&mut subs, &out, &list, false);
    // Renamed back: the binding heals, and the healing alone ends them.
    subs.on_values("band", &[push("live_20.name", json!("Hand1 #"))]);
    let out = subs.drain_outgoing();
    answer(&mut subs, &out, &list, true);
    assert_eq!(subs.take_deliveries().len(), 1);
    let out = subs.drain_outgoing();
    assert_eq!(removed(&out), vec!["live_21.name"]);
    assert!(subs.drain_outgoing().is_empty());
}

#[test]
fn a_disconnect_drops_the_watches_and_a_connect_brings_them_back() {
    let mut subs = online();
    let list = missing(&mut subs);
    let out = subs.drain_outgoing();
    answer(&mut subs, &out, &list, false);
    subs.disconnected("band");
    assert!(subs.drain_outgoing().is_empty(), "offline: nothing sent");
    // Live is back: the binding and its guards resolve, and only then,
    // with the binding in error again, are the items watched.
    subs.connected("band");
    let out = subs.drain_outgoing();
    assert_eq!(
        watched(&out),
        Vec::<String>::new(),
        "the watches went with the disconnect"
    );
    assert_eq!(commands(&out).len(), 3);
    answer(&mut subs, &out, &list, false);
    let out = subs.drain_outgoing();
    assert_eq!(
        watched(&out),
        vec!["live_set tracks 0", "live_set tracks 1"]
    );
}
