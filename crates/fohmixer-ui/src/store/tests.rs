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
    assert_eq!(Readiness::of([&value, &value]), Readiness::Ready);
    assert_eq!(Readiness::of([&value, &Slot::Pending]), Readiness::Waiting);
    assert_eq!(
        Readiness::of([&Slot::Pending, &error, &value]),
        Readiness::Unresolved,
        "an unresolved binding outranks a missing value"
    );
    assert_eq!(Readiness::of([&value, &error]), Readiness::Unresolved);
    assert_eq!(
        [Readiness::Ready, Readiness::Waiting, Readiness::Unresolved].map(Readiness::name),
        ["ready", "waiting", "unresolved"]
    );
    assert_eq!(
        [Readiness::Ready, Readiness::Waiting, Readiness::Unresolved].map(Readiness::disabled),
        ["false", "true", "true"]
    );
}
