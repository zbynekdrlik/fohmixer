use super::*;
use crate::layout::Control;
use crate::sectors::{Pooled, Refusal, StripRef};
use crate::strip_marks::{switch, unknown};
use serde_json::json;

fn band(label: &str) -> StripRef {
    StripRef {
        instance: "band".into(),
        label: label.into(),
    }
}

fn mbc(label: &str) -> StripRef {
    StripRef {
        instance: "mbc".into(),
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

#[test]
fn the_document_reads_as_written_and_refuses_unknown_fields() {
    let doc: MuteGuardsDoc =
        serde_json::from_value(json!({"mute_guards": {"band": ["Vox 1", "Bass"]}})).unwrap();
    assert!(doc.is_on(&band("Vox 1")));
    assert!(doc.is_on(&band("Bass")));
    assert!(!doc.is_on(&band("Keys")));
    assert!(!doc.is_on(&mbc("Vox 1")));
    assert_eq!(doc.count(), 2);
    assert_eq!(
        serde_json::to_value(&doc).unwrap(),
        json!({"mute_guards": {"band": ["Bass", "Vox 1"]}})
    );
    // The alarm's field is not the guard's.
    assert!(
        serde_json::from_value::<MuteGuardsDoc>(json!({"mute_alarms": {"band": ["Vox 1"]}}))
            .is_err()
    );
    assert!(serde_json::from_value::<MuteGuardsDoc>(json!({"mute_guards": {}, "on": 1})).is_err());
    assert_eq!(
        serde_json::from_value::<MuteGuardsDoc>(json!({"mute_guards": {}})).unwrap(),
        MuteGuardsDoc::empty()
    );
}

#[test]
fn with_marks_clears_and_drops_an_empty_live_set() {
    let one = MuteGuardsDoc::empty().with(&band("Vox 1"), true);
    assert!(one.is_on(&band("Vox 1")));
    assert_eq!(one.with(&band("Vox 1"), true), one);
    assert_eq!(one.with(&band("Vox 1"), false), MuteGuardsDoc::empty());
    assert_eq!(one.strips(), vec![band("Vox 1")]);
}

#[test]
fn only_keeps_one_live_sets_marks() {
    let doc = MuteGuardsDoc::empty()
        .with(&band("Vox 1"), true)
        .with(&mbc("Hand 1"), true)
        .with(&mbc("Hand 2"), true);
    let only = doc.only("mbc");
    assert_eq!(only.strips(), vec![mbc("Hand 1"), mbc("Hand 2")]);
    assert_eq!(doc.only("band").strips(), vec![band("Vox 1")]);
    assert_eq!(doc.only("master"), MuteGuardsDoc::empty());
}

#[test]
fn validate_refuses_an_empty_live_set_or_label() {
    let bad: MuteGuardsDoc =
        serde_json::from_value(json!({"mute_guards": {"": ["Vox 1"], "band": [" "]}})).unwrap();
    assert_eq!(
        bad.validate(),
        vec![
            "a Live set's name is empty".to_string(),
            "a fader of \"band\" has an empty label".to_string(),
        ]
    );
    assert!(MuteGuardsDoc::empty().validate().is_empty());
}

#[test]
fn the_switch_takes_a_pool_fader_and_counts_the_unknown() {
    let controls = [control("Vox 1"), control("Bass")];
    let pool: Vec<Pooled> = controls
        .iter()
        .map(|c| Pooled {
            id: crate::sectors::strip_ref(c).expect("a strip with a name"),
            control: c,
        })
        .collect();
    let doc = MuteGuardsDoc::empty();
    assert_eq!(
        switch(&doc, &band("Bass"), true, &pool),
        Ok(Some(doc.with(&band("Bass"), true)))
    );
    assert_eq!(switch(&doc, &band("Bass"), false, &pool), Ok(None));
    assert_eq!(
        switch(&doc, &band("Keys"), true, &pool),
        Err(Refusal::NotInPool)
    );
    let stray = doc.with(&band("Renamed"), true).with(&band("Vox 1"), true);
    assert_eq!(unknown(&stray, &pool), 1);
    assert_eq!(
        switch(&stray, &band("Renamed"), false, &pool),
        Ok(Some(doc.with(&band("Vox 1"), true)))
    );
}
