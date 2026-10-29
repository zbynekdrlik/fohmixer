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
fn the_palette_is_the_mockups_twelve_in_its_order() {
    let rows: Vec<(&str, &str, String)> = ColorKey::ALL
        .iter()
        .map(|k| (k.as_str(), k.name(), css(k.rgb())))
        .collect();
    assert_eq!(
        rows,
        vec![
            ("red", "RED", "#d4676d".to_string()),
            ("orange", "ORANGE", "#de8a4e".to_string()),
            ("amber", "AMBER", "#d6b24e".to_string()),
            ("lime", "LIME", "#a9c45c".to_string()),
            ("green", "GREEN", "#5db87c".to_string()),
            ("teal", "TEAL", "#4bb3a6".to_string()),
            ("cyan", "CYAN", "#5aaed8".to_string()),
            ("blue", "BLUE", "#6d8fe2".to_string()),
            ("violet", "VIOLET", "#9b82e4".to_string()),
            ("purple", "PURPLE", "#c47acb".to_string()),
            ("pink", "PINK", "#e27fa8".to_string()),
            ("sand", "SAND", "#b59a7c".to_string()),
        ]
    );
    // Each key is its serde name.
    for key in ColorKey::ALL {
        assert_eq!(serde_json::to_value(key).unwrap(), json!(key.as_str()));
    }
}

#[test]
fn none_is_the_calm_grey() {
    assert_eq!(css(NONE_RGB), "#73738a");
    assert_eq!(shown_rgb(None), 0x73738a);
    assert_eq!(shown_rgb(Some(ColorKey::Teal)), 0x4bb3a6);
    assert_eq!(shown_name(None), "NONE");
    assert_eq!(shown_name(Some(ColorKey::Pink)), "PINK");
    assert_eq!(css(0x00000a), "#00000a");
}

#[test]
fn the_document_reads_as_written_and_refuses_unknown_keys_and_fields() {
    let doc: ColorsDoc =
        serde_json::from_value(json!({"colors": {"band": {"Vox 1": "pink", "Bass": "blue"}}}))
            .unwrap();
    assert_eq!(doc.of(&band("Vox 1")), Some(ColorKey::Pink));
    assert_eq!(doc.of(&band("Bass")), Some(ColorKey::Blue));
    assert_eq!(doc.of(&band("Keys")), None);
    assert_eq!(
        doc.of(&StripRef {
            instance: "master".into(),
            label: "Vox 1".into()
        }),
        None
    );
    assert_eq!(doc.count(), 2);
    assert_eq!(
        serde_json::to_value(&doc).unwrap(),
        json!({"colors": {"band": {"Bass": "blue", "Vox 1": "pink"}}})
    );
    assert!(
        serde_json::from_value::<ColorsDoc>(json!({"colors": {"band": {"Vox 1": "gold"}}}))
            .is_err()
    );
    assert!(serde_json::from_value::<ColorsDoc>(json!({"colors": {}, "rev": 1})).is_err());
    assert_eq!(
        serde_json::from_value::<ColorsDoc>(json!({"colors": {}})).unwrap(),
        ColorsDoc::empty()
    );
}

#[test]
fn with_sets_and_clears_a_colour_and_drops_an_empty_live_set() {
    let one = ColorsDoc::empty().with(&band("Vox 1"), Some(ColorKey::Pink));
    assert_eq!(one.of(&band("Vox 1")), Some(ColorKey::Pink));
    let two = one.with(&band("Bass"), Some(ColorKey::Blue));
    assert_eq!(two.count(), 2);
    let changed = two.with(&band("Vox 1"), Some(ColorKey::Red));
    assert_eq!(changed.of(&band("Vox 1")), Some(ColorKey::Red));
    assert_eq!(changed.count(), 2);
    let cleared = changed.with(&band("Vox 1"), None);
    assert_eq!(cleared.of(&band("Vox 1")), None);
    assert_eq!(cleared.count(), 1);
    // NONE everywhere is the empty document: no `"band": {}` is kept.
    assert_eq!(cleared.with(&band("Bass"), None), ColorsDoc::empty());
    // Clearing a fader that has none changes nothing.
    assert_eq!(
        ColorsDoc::empty().with(&band("Keys"), None),
        ColorsDoc::empty()
    );
}

#[test]
fn strips_lists_every_fader_with_its_colour() {
    let doc = ColorsDoc::empty()
        .with(&band("Vox 1"), Some(ColorKey::Pink))
        .with(
            &StripRef {
                instance: "master".into(),
                label: "Sub".into(),
            },
            Some(ColorKey::Red),
        );
    let all: Vec<(StripRef, ColorKey)> = doc.strips().collect();
    assert_eq!(
        all,
        vec![
            (band("Vox 1"), ColorKey::Pink),
            (
                StripRef {
                    instance: "master".into(),
                    label: "Sub".into()
                },
                ColorKey::Red
            ),
        ]
    );
}

#[test]
fn validate_refuses_an_empty_live_set_or_label() {
    assert_eq!(ColorsDoc::empty().validate(), Vec::<String>::new());
    let good = ColorsDoc::empty().with(&band("Vox 1"), Some(ColorKey::Pink));
    assert_eq!(good.validate(), Vec::<String>::new());
    let bad: ColorsDoc = serde_json::from_value(
        json!({"colors": {" ": {"Vox 1": "pink"}, "band": {"": "red", "  ": "blue"}}}),
    )
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
fn unknown_counts_the_colours_no_pool_fader_carries() {
    let controls = vec![control("Vox 1"), control("Bass")];
    let pool = pool_of(&controls);
    let doc = ColorsDoc::empty()
        .with(&band("Vox 1"), Some(ColorKey::Pink))
        .with(&band("Renamed"), Some(ColorKey::Red));
    assert_eq!(unknown(&doc, &pool), 1);
    assert_eq!(unknown(&doc, &[]), 2);
    assert_eq!(unknown(&ColorsDoc::empty(), &pool), 0);
}

#[test]
fn set_color_takes_a_pool_fader_and_refuses_another() {
    let controls = vec![control("Vox 1"), control("Bass")];
    let pool = pool_of(&controls);
    let doc = ColorsDoc::empty().with(&band("Vox 1"), Some(ColorKey::Pink));
    // A change.
    assert_eq!(
        set_color(&doc, &band("Bass"), Some(ColorKey::Blue), &pool),
        Ok(Some(doc.with(&band("Bass"), Some(ColorKey::Blue))))
    );
    assert_eq!(
        set_color(&doc, &band("Vox 1"), None, &pool),
        Ok(Some(ColorsDoc::empty()))
    );
    // Nothing changes.
    assert_eq!(
        set_color(&doc, &band("Vox 1"), Some(ColorKey::Pink), &pool),
        Ok(None)
    );
    assert_eq!(set_color(&doc, &band("Bass"), None, &pool), Ok(None));
    // A colour for a fader not in the pool is refused; NONE clears it.
    assert_eq!(
        set_color(&doc, &band("Keys"), Some(ColorKey::Red), &pool),
        Err(Refusal::NotInPool)
    );
    let stray = doc.with(&band("Renamed"), Some(ColorKey::Red));
    assert_eq!(
        set_color(&stray, &band("Renamed"), None, &pool),
        Ok(Some(doc.clone()))
    );
    assert_eq!(set_color(&doc, &band("Keys"), None, &[]), Ok(None));
}
