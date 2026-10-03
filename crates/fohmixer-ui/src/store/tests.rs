use super::*;

fn spec(instance: &str, track: &str, prop: &str) -> SubSpec {
    SubSpec::new(
        instance,
        format!("live_set tracks[name={track}]"),
        prop,
        false,
    )
}

#[test]
fn a_hub_item_leaves_a_value_an_error_or_nothing() {
    assert_eq!(
        Slot::from_item(Some(json!(0.85)), Some("0.0 dB".into()), None, 12.0),
        Some(Slot::Value {
            value: json!(0.85),
            display: Some("0.0 dB".into()),
            at: 12.0
        })
    );
    assert_eq!(
        Slot::from_item(None, None, Some("not found".into()), 1.0),
        Some(Slot::Error("not found".into()))
    );
    assert_eq!(
        Slot::from_item(Some(json!(1)), None, Some("gone".into()), 1.0),
        Some(Slot::Error("gone".into())),
        "an error wins"
    );
    assert_eq!(Slot::from_item(None, None, None, 1.0), None);
    // A null from Live is a value (the hub keeps `null` apart from absent).
    assert!(
        Slot::from_item(Some(Value::Null), None, None, 1.0)
            .unwrap()
            .is_ready()
    );
}

#[test]
fn a_slot_reads_as_a_number_a_flag_or_a_display_string() {
    let volume = Slot::Value {
        value: json!(0.7),
        display: Some("-6.0 dB".into()),
        at: 5.0,
    };
    assert!(volume.is_ready() && !volume.is_error());
    assert_eq!(volume.number(), Some(0.7));
    assert_eq!(volume.flag(), None);
    assert_eq!(volume.display(), Some("-6.0 dB"));
    assert_eq!(volume.at(), Some(5.0));
    assert_eq!(volume.value(), Some(&json!(0.7)));
    let mute = Slot::Value {
        value: json!(true),
        display: None,
        at: 1.0,
    };
    assert_eq!(
        (mute.flag(), mute.number(), mute.display()),
        (Some(true), None, None)
    );
    for other in [Slot::Pending, Slot::Error("x".into())] {
        assert!(!other.is_ready());
        assert_eq!(other.is_error(), matches!(other, Slot::Error(_)));
        assert_eq!(
            (
                other.value(),
                other.number(),
                other.flag(),
                other.display(),
                other.at()
            ),
            (None, None, None, None, None)
        );
    }
}

#[test]
fn a_lost_connection_keeps_a_slots_value_as_stale() {
    let fresh = Slot::Value {
        value: json!(0.7),
        display: Some("-6.0 dB".into()),
        at: 5.0,
    };
    let stale = fresh.clone().into_stale();
    assert_eq!(
        stale,
        Slot::Stale {
            value: json!(0.7),
            display: Some("-6.0 dB".into()),
            at: 5.0
        }
    );
    // Shown and touched as before (L2)...
    assert!(stale.is_ready() && !stale.is_error());
    assert_eq!(Readiness::of_slot(&stale), Readiness::Ready);
    assert_eq!(
        (stale.number(), stale.display(), stale.at(), stale.value()),
        (Some(0.7), Some("-6.0 dB"), Some(5.0), Some(&json!(0.7)))
    );
    assert_eq!(
        Slot::Value {
            value: json!(true),
            display: None,
            at: 1.0
        }
        .into_stale()
        .flag(),
        Some(true)
    );
    // ...but not fresh: a meter and the status light take only a fresh one.
    assert!(fresh.is_fresh() && !stale.is_fresh());
    assert_eq!(fresh.fresh_number(), Some(0.7));
    assert_eq!(stale.fresh_number(), None);
    assert_eq!(stale.clone().into_stale(), stale, "stale stays stale");
    // Without a value there is nothing to keep.
    for other in [Slot::Pending, Slot::Error("gone".into())] {
        assert_eq!(other.clone().into_stale(), other);
        assert!(!other.is_fresh());
        assert_eq!(other.fresh_number(), None);
    }
    assert_eq!(
        Slot::Value {
            value: json!("text"),
            display: None,
            at: 1.0
        }
        .fresh_number(),
        None,
        "a fresh value that is no number"
    );
}

#[test]
fn a_page_switch_waits_while_connected_and_keeps_a_known_value_during_an_outage() {
    let fresh = Slot::Value {
        value: json!(0.7),
        display: Some("-6.0 dB".into()),
        at: 5.0,
    };
    let stale = fresh.clone().into_stale();
    // The hub connected: a key subscribed now waits for its fresh value
    // (I8), one unsubscribed has nothing to keep it current.
    for slot in [fresh.clone(), stale.clone(), Slot::Error("gone".into())] {
        assert_eq!(slot.rewanted(true), Slot::Pending);
    }
    // During an outage, added or removed: a known value is kept (L2).
    assert_eq!(fresh.clone().rewanted(false), stale);
    assert_eq!(stale.clone().rewanted(false), stale);
    assert!(fresh.rewanted(false).is_ready(), "it still takes touches");
    assert_eq!(
        Slot::Pending.rewanted(false),
        Slot::Pending,
        "nothing known"
    );
    assert_eq!(
        Slot::Error("gone".into()).rewanted(false),
        Slot::Error("gone".into())
    );
}

#[test]
fn a_subscriptions_write_key_drops_its_display_flag() {
    let volume = SubSpec::new(
        "band",
        "live_set  tracks[name=Hand1 #] mixer_device volume".into(),
        "value",
        true,
    );
    assert_eq!(
        write_key(&volume.key()),
        fohmixer_proto::client::set_key(&volume.instance, &volume.target, &volume.prop)
    );
    assert_eq!(
        write_key("band|live_set tracks 1|mute|false"),
        "band|live_set tracks 1|mute"
    );
    assert_eq!(write_key("no-bar"), "no-bar");
}

#[test]
fn the_wanted_set_subscribes_each_key_once_and_releases_the_rest() {
    let mut wanted = Wanted::default();
    assert!(wanted.is_empty());
    let mute = spec("band", "A", "mute");
    let change = wanted.replace(vec![mute.clone(), mute.clone(), spec("band", "B", "solo")]);
    assert_eq!(change.removed, Vec::<String>::new());
    assert_eq!(change.added, [mute.clone(), spec("band", "B", "solo")]);
    assert_eq!(wanted.len(), 2, "two binds of one key are one subscription");
    assert!(!wanted.is_empty());
    // The same set again changes nothing.
    assert_eq!(
        wanted.replace(vec![spec("band", "B", "solo"), mute.clone()]),
        Change::default()
    );
    // A new page: B leaves (one unsub), C comes.
    let change = wanted.replace(vec![mute.clone(), spec("master", "C", "mute")]);
    assert_eq!(change.removed, [spec("band", "B", "solo").key()]);
    assert_eq!(change.added, [spec("master", "C", "mute")]);
    assert_eq!(wanted.specs(), [mute.clone(), spec("master", "C", "mute")]);
    // Everything leaves.
    let change = wanted.replace(Vec::new());
    assert_eq!(
        change.removed,
        [mute.key(), spec("master", "C", "mute").key()]
    );
    assert!(change.added.is_empty() && wanted.is_empty());
}

#[test]
fn a_key_is_wanted_until_its_page_goes() {
    let mut wanted = Wanted::default();
    let mute = spec("band", "A", "mute");
    assert!(!wanted.contains(&mute.key()), "nothing wanted yet");
    wanted.replace(vec![mute.clone()]);
    assert!(wanted.contains(&mute.key()));
    assert!(!wanted.contains(&spec("band", "B", "mute").key()));
    wanted.replace(vec![spec("band", "B", "mute")]);
    assert!(!wanted.contains(&mute.key()), "its page went");
}

#[test]
fn the_wanted_keys_of_one_instance() {
    let mut wanted = Wanted::default();
    wanted.replace(vec![
        spec("band", "A", "mute"),
        spec("master", "C", "mute"),
        spec("band", "B", "solo"),
    ]);
    assert_eq!(
        wanted.keys_of(Some("band")),
        [
            spec("band", "A", "mute").key(),
            spec("band", "B", "solo").key()
        ]
    );
    assert_eq!(
        wanted.keys_of(Some("master")),
        [spec("master", "C", "mute").key()]
    );
    assert!(wanted.keys_of(Some("other")).is_empty());
    assert_eq!(wanted.keys_of(None).len(), 3);
}

#[test]
fn badges_are_offline_busy_or_online() {
    let view = |online, busy| InstanceView {
        online,
        busy,
        set_name: String::new(),
    };
    assert_eq!(Badge::of(&view(false, false)), Badge::Offline);
    assert_eq!(Badge::of(&view(false, true)), Badge::Offline);
    assert_eq!(Badge::of(&view(true, true)), Badge::Busy);
    assert_eq!(Badge::of(&view(true, false)), Badge::Online);
    assert_eq!(
        [Badge::Online, Badge::Busy, Badge::Offline].map(Badge::name),
        ["online", "busy", "offline"]
    );
}

#[test]
fn a_failed_set_says_why() {
    assert_eq!(
        slot_failure(&Ok(vec![json!({"ok": true, "data": null})])),
        None
    );
    assert_eq!(
        slot_failure(&Ok(vec![
            json!({"ok": false, "error": "Invalid value", "errorType": "RuntimeError"})
        ])),
        Some("Invalid value".to_string())
    );
    assert_eq!(
        slot_failure(&Ok(vec![json!({"ok": false})])),
        Some("failed".to_string())
    );
    assert_eq!(slot_failure(&Ok(vec![])), Some("no result".to_string()));
    assert_eq!(
        slot_failure(&Err("instance offline".into())),
        Some("instance offline".to_string())
    );
}

#[test]
fn a_parameter_range_needs_two_numbers_in_order() {
    let ok = |v: Value| json!({"ok": true, "data": v});
    assert_eq!(
        range_from(&Ok(vec![ok(json!(0.0)), ok(json!(1.0))])),
        Some((0.0, 1.0))
    );
    assert_eq!(
        range_from(&Ok(vec![ok(json!(-15)), ok(json!(15))])),
        Some((-15.0, 15.0))
    );
    assert_eq!(
        range_from(&Ok(vec![ok(json!(1.0)), ok(json!(1.0))])),
        None,
        "empty"
    );
    assert_eq!(range_from(&Ok(vec![ok(json!(2.0)), ok(json!(1.0))])), None);
    assert_eq!(range_from(&Ok(vec![ok(json!(0.0))])), None);
    assert_eq!(range_from(&Ok(vec![ok(json!("a")), ok(json!(1.0))])), None);
    assert_eq!(
        range_from(&Ok(vec![json!({"ok": false, "data": 0.0}), ok(json!(1.0))])),
        None
    );
    assert_eq!(range_from(&Err("offline".into())), None);
}

#[test]
fn a_control_is_ready_with_every_value_and_red_with_any_unresolved_binding() {
    let value = Slot::Value {
        value: json!(0.5),
        display: None,
        at: 0.0,
    };
    let error = Slot::Error("no track named Hand9".into());
    let of = |slots: &[&Slot]| Readiness::all(slots.iter().map(|s| Readiness::of_slot(s)));
    assert_eq!(of(&[&value, &value]), Readiness::Ready);
    assert_eq!(of(&[&value, &Slot::Pending]), Readiness::Waiting);
    assert_eq!(
        of(&[&Slot::Pending, &error, &value]),
        Readiness::Unresolved,
        "an unresolved binding outranks a missing value"
    );
    assert_eq!(of(&[&value, &error]), Readiness::Unresolved);
    assert_eq!(of(&[]), Readiness::Ready, "nothing to wait for");
    assert_eq!(
        [Readiness::Ready, Readiness::Waiting, Readiness::Unresolved].map(Readiness::name),
        ["ready", "waiting", "unresolved"]
    );
    assert_eq!(
        [Readiness::Ready, Readiness::Waiting, Readiness::Unresolved].map(Readiness::disabled),
        ["false", "true", "true"]
    );
}

#[test]
fn a_range_read_without_an_answer_keeps_the_range_before() {
    let range = |min: f64, max: f64| -> Result<Vec<Value>, String> {
        Ok(vec![
            json!({"ok": true, "data": min}),
            json!({"ok": true, "data": max}),
        ])
    };
    let before = Some((0.0, 1.0));
    assert_eq!(next_range(before, &range(-15.0, 15.0)), Some((-15.0, 15.0)));
    assert_eq!(
        next_range(before, &Err("no result within 3 s".into())),
        before,
        "a stalled Live: keep it"
    );
    let gone: Result<Vec<Value>, String> = Ok(vec![
        json!({"ok": false, "error": "no parameter"}),
        json!({"ok": false, "error": "no parameter"}),
    ]);
    assert_eq!(next_range(before, &gone), None, "Live says it is gone");
    assert_eq!(next_range(None, &Err("offline".into())), None);
}
