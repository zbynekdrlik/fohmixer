use super::*;
use crate::layout::Control;
use serde_json::json;

fn band(label: &str) -> StripRef {
    StripRef {
        instance: "band".into(),
        label: label.into(),
    }
}

/// A band strip of the track `name`.
fn control(name: &str) -> Control {
    serde_json::from_value(json!({"kind": "strip",
        "binding": {"instance": "band", "anchor": {"kind": "track", "name": name}},
        "strip_kind": "standard"}))
    .expect("the control parses")
}

/// The pool of these band tracks.
fn pool_of(controls: &[Control]) -> Vec<Pooled<'_>> {
    controls
        .iter()
        .map(|c| Pooled {
            id: crate::sectors::strip_ref(c).expect("a strip with a name"),
            control: c,
        })
        .collect()
}

#[test]
fn the_document_reads_as_written_and_refuses_unknown_fields() {
    let doc: MuteAlarmsDoc =
        serde_json::from_value(json!({"mute_alarms": {"band": ["Vox 1", "Bass"]}})).unwrap();
    assert!(doc.is_on(&band("Vox 1")));
    assert!(doc.is_on(&band("Bass")));
    assert!(!doc.is_on(&band("Keys")));
    assert!(!doc.is_on(&StripRef {
        instance: "master".into(),
        label: "Vox 1".into(),
    }));
    assert_eq!(doc.count(), 2);
    // Sorted labels, one each.
    assert_eq!(
        serde_json::to_value(&doc).unwrap(),
        json!({"mute_alarms": {"band": ["Bass", "Vox 1"]}})
    );
    assert!(serde_json::from_value::<MuteAlarmsDoc>(json!({"mute_alarms": {}, "rev": 1})).is_err());
    assert!(
        serde_json::from_value::<MuteAlarmsDoc>(json!({"mute_alarms": {"band": "Vox 1"}})).is_err()
    );
    assert_eq!(
        serde_json::from_value::<MuteAlarmsDoc>(json!({"mute_alarms": {}})).unwrap(),
        MuteAlarmsDoc::empty()
    );
}

#[test]
fn with_marks_and_clears_and_drops_an_empty_live_set() {
    let one = MuteAlarmsDoc::empty().with(&band("Vox 1"), true);
    assert!(one.is_on(&band("Vox 1")));
    assert_eq!(one.count(), 1);
    let two = one.with(&band("Bass"), true);
    assert_eq!(two.count(), 2);
    // Marking a marked fader again keeps one mark.
    assert_eq!(two.with(&band("Bass"), true), two);
    let cleared = two.with(&band("Vox 1"), false);
    assert!(!cleared.is_on(&band("Vox 1")));
    assert!(cleared.is_on(&band("Bass")));
    assert_eq!(cleared.count(), 1);
    // No mark anywhere is the empty document: no `"band": []` is kept.
    assert_eq!(cleared.with(&band("Bass"), false), MuteAlarmsDoc::empty());
    // Clearing a fader with no mark changes nothing.
    assert_eq!(
        MuteAlarmsDoc::empty().with(&band("Keys"), false),
        MuteAlarmsDoc::empty()
    );
}

#[test]
fn strips_lists_every_marked_fader() {
    let doc = MuteAlarmsDoc::empty().with(&band("Vox 1"), true).with(
        &StripRef {
            instance: "master".into(),
            label: "Sub".into(),
        },
        true,
    );
    let all: Vec<StripRef> = doc.strips().collect();
    assert_eq!(
        all,
        vec![
            band("Vox 1"),
            StripRef {
                instance: "master".into(),
                label: "Sub".into()
            },
        ]
    );
}

#[test]
fn validate_refuses_an_empty_live_set_or_label() {
    assert_eq!(MuteAlarmsDoc::empty().validate(), Vec::<String>::new());
    let good = MuteAlarmsDoc::empty().with(&band("Vox 1"), true);
    assert_eq!(good.validate(), Vec::<String>::new());
    let bad: MuteAlarmsDoc =
        serde_json::from_value(json!({"mute_alarms": {" ": ["Vox 1"], "band": ["", "  "]}}))
            .unwrap();
    assert_eq!(
        bad.validate(),
        vec![
            "a Live set's name is empty".to_string(),
            "a fader of \"band\" has an empty label".to_string(),
            "a fader of \"band\" has an empty label".to_string(),
        ]
    );
}

#[test]
fn unknown_counts_the_marks_no_pool_fader_carries() {
    let controls = vec![control("Vox 1"), control("Bass")];
    let pool = pool_of(&controls);
    let doc = MuteAlarmsDoc::empty()
        .with(&band("Vox 1"), true)
        .with(&band("Renamed"), true);
    assert_eq!(unknown(&doc, &pool), 1);
    assert_eq!(unknown(&doc, &[]), 2);
    assert_eq!(unknown(&MuteAlarmsDoc::empty(), &pool), 0);
}

#[test]
fn set_mute_alarm_takes_a_pool_fader_and_refuses_another() {
    let controls = vec![control("Vox 1"), control("Bass")];
    let pool = pool_of(&controls);
    let doc = MuteAlarmsDoc::empty().with(&band("Vox 1"), true);
    // A change.
    assert_eq!(
        set_mute_alarm(&doc, &band("Bass"), true, &pool),
        Ok(Some(doc.with(&band("Bass"), true)))
    );
    assert_eq!(
        set_mute_alarm(&doc, &band("Vox 1"), false, &pool),
        Ok(Some(MuteAlarmsDoc::empty()))
    );
    // Nothing changes.
    assert_eq!(set_mute_alarm(&doc, &band("Vox 1"), true, &pool), Ok(None));
    assert_eq!(set_mute_alarm(&doc, &band("Bass"), false, &pool), Ok(None));
    // A mark for a fader not in the pool is refused; off clears it.
    assert_eq!(
        set_mute_alarm(&doc, &band("Keys"), true, &pool),
        Err(Refusal::NotInPool)
    );
    let stray = doc.with(&band("Renamed"), true);
    assert_eq!(
        set_mute_alarm(&stray, &band("Renamed"), false, &pool),
        Ok(Some(doc.clone()))
    );
    assert_eq!(set_mute_alarm(&doc, &band("Keys"), false, &[]), Ok(None));
}
