use super::*;
use crate::layout::Control;
use serde_json::json;

fn band(label: &str) -> StripRef {
    StripRef {
        instance: "band".into(),
        label: label.into(),
    }
}

fn strip(instance: &str, label: &str) -> StripRef {
    StripRef {
        instance: instance.into(),
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

fn rule(db: Option<f64>, mute: Option<SceneMute>) -> SceneRule {
    SceneRule {
        db,
        mute,
        time_ms: DEFAULT_TIME_MS,
    }
}

/// A document that heard `intro` and `talk`.
fn heard_two() -> ScenesDoc {
    let doc = heard(&ScenesDoc::empty(), "intro").unwrap();
    heard(&doc, "talk").unwrap()
}

#[test]
fn the_numbers_are_the_rulings() {
    assert_eq!(MAX_NAME_CHARS, 64);
    assert_eq!(MAX_KNOWN, 128);
    assert_eq!(DEFAULT_TIME_MS, 2000);
    assert_eq!(MAX_TIME_MS, 60_000);
}

#[test]
fn the_document_reads_as_written_and_refuses_unknown_fields() {
    let doc: ScenesDoc = serde_json::from_value(json!({
        "known": ["intro", "talk"],
        "rules": {"band": {"Vox 1": {"intro": {"db": -10.0, "mute": "unmute", "time_ms": 3000},
                                     "talk": {"mute": "mute"}}}}
    }))
    .unwrap();
    assert_eq!(doc.known, vec!["intro", "talk"]);
    assert_eq!(
        doc.rule(&band("Vox 1"), "intro"),
        Some(SceneRule {
            db: Some(-10.0),
            mute: Some(SceneMute::Unmute),
            time_ms: 3000
        })
    );
    // A rule without a time takes the default; without a dB it moves none.
    assert_eq!(
        doc.rule(&band("Vox 1"), "talk"),
        Some(rule(None, Some(SceneMute::Mute)))
    );
    assert_eq!(doc.rule(&band("Vox 1"), "song"), None);
    assert_eq!(doc.rule(&band("Bass"), "intro"), None);
    assert!(doc.validate().is_empty());
    // The empty object is the empty document.
    assert_eq!(
        serde_json::from_value::<ScenesDoc>(json!({})).unwrap(),
        ScenesDoc::empty()
    );
    for bad in [
        json!({"known": [], "rules": {}, "extra": 1}),
        json!({"rules": {"band": {"Vox 1": {"intro": {"db": 1.0, "fade": 2}}}}}),
        json!({"rules": {"band": {"Vox 1": {"intro": {"mute": "solo"}}}}}),
    ] {
        assert!(
            serde_json::from_value::<ScenesDoc>(bad.clone()).is_err(),
            "{bad}"
        );
    }
    // It writes back as it read.
    let back = serde_json::to_value(&doc).unwrap();
    assert_eq!(
        back["rules"]["band"]["Vox 1"]["talk"],
        json!({"db": null, "mute": "mute", "time_ms": 2000})
    );
}

#[test]
fn a_mute_writes_true_and_an_unmute_false() {
    assert!(SceneMute::Mute.value());
    assert!(!SceneMute::Unmute.value());
    assert_eq!(SceneMute::Mute.word(), "MUTE");
    assert_eq!(SceneMute::Unmute.word(), "UNMUTE");
}

#[test]
fn a_rule_does_something_with_a_db_or_a_mute() {
    assert!(!rule(None, None).does_something());
    assert!(rule(Some(0.0), None).does_something());
    assert!(rule(None, Some(SceneMute::Mute)).does_something());
}

#[test]
fn a_rules_problem_is_its_db_its_time_or_nothing_to_do() {
    assert_eq!(rule(None, None).problem(), Some("a rule moves nothing"));
    assert_eq!(rule(Some(-10.0), None).problem(), None);
    // The law's ends are taken, past them not.
    assert_eq!(rule(Some(FLOOR_DB), None).problem(), None);
    assert_eq!(rule(Some(TOP_DB), None).problem(), None);
    for db in [
        FLOOR_DB.next_down(),
        TOP_DB.next_up(),
        f64::NAN,
        f64::INFINITY,
    ] {
        assert_eq!(
            rule(Some(db), None).problem(),
            Some("a rule's dB is outside -70..+6"),
            "{db}"
        );
    }
    let long = SceneRule {
        db: None,
        mute: Some(SceneMute::Mute),
        time_ms: MAX_TIME_MS,
    };
    assert_eq!(long.problem(), None);
    let longer = SceneRule {
        time_ms: MAX_TIME_MS + 1,
        db: None,
        mute: Some(SceneMute::Mute),
    };
    assert_eq!(longer.problem(), Some("a rule's time is longer than 60 s"));
}

#[test]
fn a_scene_name_is_trimmed_and_checked() {
    assert_eq!(scene_name("  intro \n"), Ok("intro".to_string()));
    assert_eq!(scene_name("talk 2"), Ok("talk 2".to_string()));
    assert_eq!(scene_name(""), Err("the scene name is empty"));
    assert_eq!(scene_name("   "), Err("the scene name is empty"));
    let long: String = "é".repeat(MAX_NAME_CHARS);
    assert_eq!(
        scene_name(&long),
        Ok(long.clone()),
        "64 characters, not bytes"
    );
    assert_eq!(
        scene_name(&format!("{long}x")),
        Err("the scene name is longer than 64 characters")
    );
    assert_eq!(
        scene_name("in\ttro"),
        Err("the scene name holds a control character")
    );
    assert_eq!(
        scene_name("in\u{7f}tro"),
        Err("the scene name holds a control character")
    );
}

#[test]
fn heard_learns_a_new_name_once_and_keeps_the_order() {
    let doc = heard_two();
    assert_eq!(doc.known, vec!["intro", "talk"]);
    assert!(doc.knows("talk"));
    assert!(!doc.knows("song"));
    assert_eq!(heard(&doc, "intro"), None, "known already");
    assert!(!is_full(&doc, "song"));
    // Past the most names kept a scene is not learned.
    let mut full = ScenesDoc::empty();
    for i in 0..MAX_KNOWN {
        full = heard(&full, &format!("s{i}")).unwrap();
    }
    assert_eq!(full.known.len(), MAX_KNOWN);
    assert_eq!(heard(&full, "song"), None);
    assert!(is_full(&full, "song"));
    assert!(!is_full(&full, "s0"), "a known name is never past it");
    let one_less = ScenesDoc {
        known: full.known[1..].to_vec(),
        rules: BTreeMap::new(),
    };
    assert!(heard(&one_less, "song").is_some());
    assert!(!is_full(&one_less, "song"));
}

#[test]
fn with_sets_and_removes_and_drops_what_is_left_empty() {
    let doc = heard_two().with(&band("Vox 1"), "intro", Some(rule(Some(-5.0), None)));
    assert_eq!(
        doc.rule(&band("Vox 1"), "intro"),
        Some(rule(Some(-5.0), None))
    );
    assert_eq!(doc.known, vec!["intro", "talk"], "the names stay");
    assert_eq!(doc.count(), 1);
    let two = doc.with(
        &strip("mbc", "Hand"),
        "talk",
        Some(rule(None, Some(SceneMute::Unmute))),
    );
    assert_eq!(two.count(), 2);
    assert_eq!(two.strips(), vec![band("Vox 1"), strip("mbc", "Hand")]);
    // A rule that does nothing removes as none does.
    let removed = two.with(&band("Vox 1"), "intro", Some(rule(None, None)));
    assert_eq!(
        removed.rules.get("band"),
        None,
        "a Live set left with none goes"
    );
    assert_eq!(removed.count(), 1);
    let none = removed.with(&strip("mbc", "Hand"), "talk", None);
    assert_eq!(none, heard_two());
    // A fader keeps its other scenes.
    let both = doc.with(&band("Vox 1"), "talk", Some(rule(Some(0.0), None)));
    let one = both.with(&band("Vox 1"), "intro", None);
    assert_eq!(one.of(&band("Vox 1")).len(), 1);
    assert_eq!(one.of(&band("Bass")), FaderRules::new());
}

#[test]
fn rules_of_a_scene_are_every_faders_rule_for_it() {
    let doc = heard_two()
        .with(&band("Vox 1"), "intro", Some(rule(Some(-5.0), None)))
        .with(&band("Bass"), "talk", Some(rule(Some(-1.0), None)))
        .with(
            &strip("mbc", "Hand"),
            "intro",
            Some(rule(None, Some(SceneMute::Mute))),
        );
    assert_eq!(
        doc.rules_of("intro"),
        vec![
            (band("Vox 1"), rule(Some(-5.0), None)),
            (strip("mbc", "Hand"), rule(None, Some(SceneMute::Mute))),
        ]
    );
    assert_eq!(doc.rules_of("song"), Vec::new());
}

#[test]
fn only_keeps_one_live_sets_rules_and_every_name() {
    let doc = heard_two()
        .with(&band("Vox 1"), "intro", Some(rule(Some(-5.0), None)))
        .with(&strip("mbc", "Hand"), "intro", Some(rule(Some(-1.0), None)));
    let mbc = doc.only("mbc");
    assert_eq!(mbc.known, doc.known);
    assert_eq!(mbc.strips(), vec![strip("mbc", "Hand")]);
    assert_eq!(doc.only("none").count(), 0);
}

#[test]
fn validate_names_every_problem() {
    assert!(heard_two().validate().is_empty());
    let mut doc = ScenesDoc {
        known: vec!["intro".into(), " pad".into(), "intro".into()],
        rules: BTreeMap::new(),
    };
    doc.rules
        .entry(" ".into())
        .or_default()
        .entry("".into())
        .or_default()
        .insert("bad\u{1}".into(), rule(None, None));
    assert_eq!(
        doc.validate(),
        vec![
            "known scene \" pad\" is no scene name".to_string(),
            "known scene \"intro\" is listed twice".to_string(),
            "a Live set's name is empty".to_string(),
            "a fader of \" \" has an empty label".to_string(),
            " |: \"bad\\u{1}\" is no scene name".to_string(),
            " | bad\u{1}: a rule moves nothing".to_string(),
        ]
    );
}

#[test]
fn set_rule_takes_a_pool_fader_of_a_known_scene_only() {
    let controls = [control("Vox 1"), control("Bass")];
    let pool = pool_of(&controls);
    let doc = heard_two();
    let take = rule(Some(-6.0), Some(SceneMute::Unmute));
    let next = set_rule(&doc, &band("Vox 1"), "intro", Some(take), &pool)
        .unwrap()
        .unwrap();
    assert_eq!(next.rule(&band("Vox 1"), "intro"), Some(take));
    // The same again changes nothing.
    assert_eq!(
        set_rule(&next, &band("Vox 1"), "intro", Some(take), &pool),
        Ok(None)
    );
    assert_eq!(
        set_rule(&doc, &band("Keys"), "intro", Some(take), &pool),
        Err(RuleRefusal::NotInPool)
    );
    assert_eq!(
        set_rule(&doc, &band("Vox 1"), "song", Some(take), &pool),
        Err(RuleRefusal::UnknownScene)
    );
    assert_eq!(
        set_rule(
            &doc,
            &band("Vox 1"),
            "intro",
            Some(rule(Some(9.0), None)),
            &pool
        ),
        Err(RuleRefusal::Invalid)
    );
    // A removal is always taken, a fader gone from the pool too.
    let off_pool = doc.with(&band("Keys"), "intro", Some(take));
    assert_eq!(
        set_rule(&off_pool, &band("Keys"), "intro", None, &pool),
        Ok(Some(doc.clone()))
    );
    // A rule that does nothing is a removal, for an unknown scene too.
    assert_eq!(
        set_rule(
            &next,
            &band("Vox 1"),
            "intro",
            Some(rule(None, None)),
            &pool
        ),
        Ok(Some(doc.clone()))
    );
    assert_eq!(set_rule(&doc, &band("Vox 1"), "song", None, &[]), Ok(None));
}

/// The master strip of band.
fn master() -> Control {
    serde_json::from_value(json!({"kind": "strip",
        "binding": {"instance": "band", "anchor": {"kind": "master"}},
        "strip_kind": "standard"}))
    .expect("the control parses")
}

#[test]
fn the_master_track_takes_a_volume_rule_and_no_mute() {
    let controls = [master(), control("Vox 1")];
    let pool = pool_of(&controls);
    assert!(!has_mute(&controls[0]));
    assert!(has_mute(&controls[1]));
    let param: Control = serde_json::from_value(json!({"kind": "param_fader", "label": "FX",
        "targets": [{"binding": {"instance": "band", "anchor": {"kind": "track", "name": "Vox 1"},
                     "path": "devices 0 parameters 1"}, "prop": "value"}]}))
    .expect("the control parses");
    assert!(!has_mute(&param), "a parameter fader is no track");
    let doc = heard_two();
    let main = band("Master");
    assert!(
        set_rule(&doc, &main, "intro", Some(rule(Some(-3.0), None)), &pool)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        set_rule(
            &doc,
            &main,
            "intro",
            Some(rule(Some(-3.0), Some(SceneMute::Mute))),
            &pool
        ),
        Err(RuleRefusal::NoMute)
    );
    assert!(
        set_rule(
            &doc,
            &band("Vox 1"),
            "intro",
            Some(rule(None, Some(SceneMute::Mute))),
            &pool
        )
        .unwrap()
        .is_some()
    );
}

#[test]
fn a_refusal_says_why_in_words() {
    assert_eq!(RuleRefusal::NotInPool.text(), "not in the pool");
    assert_eq!(RuleRefusal::UnknownScene.text(), "unknown scene");
    assert_eq!(RuleRefusal::Invalid.text(), "invalid rule");
    assert_eq!(RuleRefusal::NoMute.text(), "no mute");
}

#[test]
fn unknown_counts_the_faders_no_pool_fader_carries() {
    let controls = [control("Vox 1")];
    let pool = pool_of(&controls);
    let doc = heard_two()
        .with(&band("Vox 1"), "intro", Some(rule(Some(-5.0), None)))
        .with(&band("Gone"), "intro", Some(rule(Some(-5.0), None)))
        .with(&band("Gone"), "talk", Some(rule(Some(-5.0), None)));
    assert_eq!(unknown(&doc, &pool), 1);
    assert_eq!(unknown(&doc, &[]), 2);
}
