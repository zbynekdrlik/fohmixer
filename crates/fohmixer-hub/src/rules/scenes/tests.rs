use super::*;
use fohmixer_proto::scenes::{DEFAULT_TIME_MS, SceneMute};

const VOX: &str = "live_set tracks[name=Vox 1] mixer_device volume";
const VOX_TRACK: &str = "live_set tracks[name=Vox 1]";

fn band(label: &str) -> StripRef {
    StripRef {
        instance: "band".into(),
        label: label.into(),
    }
}

fn targets() -> FaderTargets {
    FaderTargets {
        volume: Some(("band".into(), VOX.into())),
        mutes: vec![
            ("band".into(), VOX_TRACK.into()),
            ("band".into(), "live_set tracks 7".into()),
        ],
    }
}

fn fader(db: Option<f64>, mute: Option<SceneMute>, time_ms: u32, now: Option<f64>) -> SceneFader {
    SceneFader {
        strip: band("Vox 1"),
        rule: SceneRule { db, mute, time_ms },
        targets: targets(),
        volume_now: now,
        mute_now: Some(false),
    }
}

fn key() -> String {
    set_key("band", VOX, "value")
}

fn write_value(act: &Act) -> Option<f64> {
    match act {
        Act::Write { prop: "value", value, .. } => value.as_f64(),
        _ => None,
    }
}

fn stops(acts: &[Act]) -> Vec<Why> {
    acts.iter()
        .filter_map(|a| match a {
            Act::Step(Step::Stop { why, .. }) => Some(*why),
            _ => None,
        })
        .collect()
}

#[test]
fn a_strips_targets_are_its_volume_and_its_mutes_linked_ones_after() {
    let control: Control = serde_json::from_value(json!({"kind": "strip",
        "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Vox 1"}},
        "strip_kind": "standard",
        "mute_links": [{"instance": "band", "anchor": {"kind": "track_at", "index": 7}}]}))
    .unwrap();
    assert_eq!(fader_targets(&control), Some(targets()));
    let solo: Control = serde_json::from_value(json!({"kind": "solo",
        "binding": {"instance": "band", "anchor": {"kind": "track", "name": "Drums"}}}))
    .unwrap();
    assert_eq!(fader_targets(&solo), None);
}

#[test]
fn moves_needs_a_volume_off_its_target_or_a_mute_that_changes() {
    assert!(!moves(Some(0.5), None, Some(true), None), "a rule of nothing");
    assert!(moves(Some(0.5), Some(0.6), None, None));
    assert!(!moves(Some(0.5), Some(0.5 + SAME_VOLUME / 2.0), None, None));
    assert!(moves(Some(0.5), Some(0.5 + 2.0 * SAME_VOLUME), None, None));
    assert!(moves(None, Some(0.5), None, None), "a volume not heard");
    assert!(moves(Some(0.5), None, Some(false), Some(true)));
    assert!(!moves(Some(0.5), None, Some(true), Some(true)));
    assert!(moves(Some(0.5), None, None, Some(false)), "a mute not heard");
    assert!(moves(Some(0.5), Some(0.5), Some(false), Some(true)));
}

#[test]
fn a_scene_glides_linear_in_db_and_ends_exactly_at_the_target() {
    let mut rule = Scenes::default();
    let from = volume_of(-20.0);
    let (acts, moved) = rule.scene(
        "intro",
        vec![fader(Some(0.0), None, DEFAULT_TIME_MS, Some(from))],
        1000.0,
    );
    assert_eq!(moved, 1);
    assert_eq!(
        acts,
        vec![Act::Step(Step::Start {
            instance: "band".into(),
            label: "Vox 1".into(),
            scene: "intro".into(),
            from_db: -20.0,
            to_db: 0.0,
            ms: 2000,
        })]
    );
    assert_eq!(rule.glides(), 1);
    assert_eq!(rule.next_tick(1000.0), Some(1000.0 + STEP_MS));
    // Half way: −10 dB.
    let half = rule.tick(2000.0);
    assert_eq!(half.len(), 1);
    let v = write_value(&half[0]).unwrap();
    assert!((value2db(v) + 10.0).abs() < 0.01, "{}", value2db(v));
    assert_eq!(rule.writes(), 1);
    // The end: exactly 0 dB's volume, then the stop.
    let end = rule.tick(3000.0);
    assert_eq!(write_value(&end[0]), Some(volume_of(0.0)));
    assert_eq!(stops(&end), vec![Why::Done]);
    assert_eq!(rule.glides(), 0);
    assert_eq!(rule.next_tick(3000.0), None);
    assert_eq!(rule.writes(), 2);
    assert_eq!(
        end[0],
        Act::Write {
            instance: "band".into(),
            target: VOX.into(),
            prop: "value",
            key: key(),
            value: json!(volume_of(0.0)),
        }
    );
}

#[test]
fn a_mute_is_written_once_to_every_track_of_the_strip() {
    let mut rule = Scenes::default();
    let (acts, moved) = rule.scene(
        "talk",
        vec![fader(None, Some(SceneMute::Mute), DEFAULT_TIME_MS, Some(0.5))],
        0.0,
    );
    assert_eq!(moved, 1);
    assert_eq!(
        acts,
        vec![
            Act::Write {
                instance: "band".into(),
                target: VOX_TRACK.into(),
                prop: "mute",
                key: set_key("band", VOX_TRACK, "mute"),
                value: json!(true),
            },
            Act::Write {
                instance: "band".into(),
                target: "live_set tracks 7".into(),
                prop: "mute",
                key: set_key("band", "live_set tracks 7", "mute"),
                value: json!(true),
            },
        ]
    );
    assert_eq!(rule.glides(), 0);
    // Already unmuted: an unmute writes nothing and moves nothing.
    let (acts, moved) = rule.scene(
        "talk",
        vec![fader(None, Some(SceneMute::Unmute), DEFAULT_TIME_MS, Some(0.5))],
        0.0,
    );
    assert_eq!((acts, moved), (Vec::new(), 0));
}

#[test]
fn a_fader_at_its_target_moves_nothing_and_its_mute_still_goes() {
    let mut rule = Scenes::default();
    let at = volume_of(-6.0);
    let (acts, moved) = rule.scene(
        "intro",
        vec![fader(Some(-6.0), None, DEFAULT_TIME_MS, Some(at))],
        0.0,
    );
    assert_eq!((acts, moved), (Vec::new(), 0));
    let (acts, moved) = rule.scene(
        "intro",
        vec![fader(Some(-6.0), Some(SceneMute::Mute), DEFAULT_TIME_MS, Some(at))],
        0.0,
    );
    assert_eq!(moved, 1);
    assert_eq!(acts.len(), 2, "the two mutes, no glide");
    assert_eq!(rule.glides(), 0);
}

#[test]
fn no_time_or_no_value_heard_writes_the_target_at_once() {
    let mut rule = Scenes::default();
    let (acts, moved) = rule.scene("a", vec![fader(Some(-3.0), None, 0, Some(0.2))], 0.0);
    assert_eq!(moved, 1);
    assert_eq!(acts.len(), 1);
    assert_eq!(write_value(&acts[0]), Some(volume_of(-3.0)));
    assert_eq!(rule.glides(), 0);
    assert_eq!(rule.writes(), 1);
    let (acts, _) = rule.scene("a", vec![fader(Some(-3.0), None, 2000, None)], 0.0);
    assert_eq!(write_value(&acts[0]), Some(volume_of(-3.0)));
    assert_eq!(rule.glides(), 0);
    assert_eq!(rule.writes(), 2);
}

#[test]
fn the_floor_glides_to_silence_and_shows_minus_70() {
    let mut rule = Scenes::default();
    let (acts, _) = rule.scene("a", vec![fader(Some(FLOOR_DB), None, 1000, Some(0.85))], 0.0);
    assert!(matches!(&acts[0], Act::Step(Step::Start { to_db, .. }) if *to_db == FLOOR_DB));
    let end = rule.tick(1000.0);
    assert_eq!(write_value(&end[0]), Some(0.0));
    assert_eq!(shown_db(0.0), FLOOR_DB);
    assert_eq!(shown_db(0.85), 0.0);
    assert_eq!(shown_db(volume_of(-10.0)), -10.0, "two decimals");
    assert_eq!(shown_db(0.5), -14.0);
}

#[test]
fn a_hand_stops_its_glide_at_once_and_another_key_changes_nothing() {
    let mut rule = Scenes::default();
    rule.scene("intro", vec![fader(Some(0.0), None, 2000, Some(0.3))], 0.0);
    assert_eq!(rule.hand("band|other|value"), Vec::new());
    assert_eq!(rule.glides(), 1);
    let acts = rule.hand(&key());
    assert_eq!(
        acts,
        vec![Act::Step(Step::Stop {
            instance: "band".into(),
            label: "Vox 1".into(),
            scene: "intro".into(),
            why: Why::Hand,
        })]
    );
    assert_eq!(rule.glides(), 0);
    assert_eq!(rule.tick(500.0), Vec::new(), "nothing more is written");
    assert_eq!(rule.hand(&key()), Vec::new());
}

#[test]
fn a_new_scene_replaces_a_glide_from_where_it_is() {
    let mut rule = Scenes::default();
    rule.scene("intro", vec![fader(Some(0.0), None, 2000, Some(volume_of(-20.0)))], 0.0);
    // Half way (−10 dB) a new scene glides the same key back to −20 dB;
    // Live's stale value is not the start.
    let (acts, moved) = rule.scene(
        "talk",
        vec![fader(Some(-20.0), None, 1000, Some(volume_of(-20.0)))],
        1000.0,
    );
    assert_eq!(moved, 1);
    assert_eq!(stops(&acts), vec![Why::Replaced]);
    assert!(
        matches!(&acts[1], Act::Step(Step::Start { scene, from_db, .. })
            if scene == "talk" && (*from_db + 10.0).abs() < 0.01),
        "{acts:?}"
    );
    assert_eq!(rule.glides(), 1);
    let end = rule.tick(2000.0);
    assert_eq!(write_value(&end[0]), Some(volume_of(-20.0)));
}

#[test]
fn a_new_scene_whose_target_a_running_glide_is_at_ends_it_exactly_there() {
    let mut rule = Scenes::default();
    rule.scene("intro", vec![fader(Some(0.0), None, 2000, Some(volume_of(-20.0)))], 0.0);
    let half = rule.tick(1000.0);
    let there = write_value(&half[0]).unwrap();
    let target = value2db(there);
    // A scene asking for exactly where the glide is: it moves nothing, the
    // old glide stops, and its end is written exactly.
    let (acts, moved) = rule.scene(
        "talk",
        vec![fader(Some(target), None, 1000, Some(volume_of(-20.0)))],
        1000.0,
    );
    assert_eq!(moved, 0);
    assert_eq!(stops(&acts), vec![Why::Replaced]);
    assert!(write_value(&acts[1]).is_some_and(|v| same_volume(v, volume_of(target))));
    assert_eq!(rule.glides(), 0);
}

#[test]
fn an_instance_offline_ends_only_its_glides() {
    let mut rule = Scenes::default();
    let mut mbc = fader(Some(0.0), None, 2000, Some(0.3));
    mbc.strip = StripRef {
        instance: "mbc".into(),
        label: "Hand".into(),
    };
    mbc.targets.volume = Some(("mbc".into(), VOX.into()));
    rule.scene(
        "intro",
        vec![fader(Some(0.0), None, 2000, Some(0.3)), mbc],
        0.0,
    );
    assert_eq!(rule.glides(), 2);
    let acts = rule.offline("mbc");
    assert_eq!(
        acts,
        vec![Act::Step(Step::Stop {
            instance: "mbc".into(),
            label: "Hand".into(),
            scene: "intro".into(),
            why: Why::Offline,
        })]
    );
    assert_eq!(rule.glides(), 1);
    assert_eq!(rule.offline("mbc"), Vec::new());
}

#[test]
fn a_fader_without_a_volume_target_only_mutes() {
    let mut rule = Scenes::default();
    let mut f = fader(Some(0.0), Some(SceneMute::Mute), 2000, Some(0.3));
    f.targets.volume = None;
    let (acts, moved) = rule.scene("a", vec![f], 0.0);
    assert_eq!(moved, 1);
    assert_eq!(acts.len(), 2);
    assert_eq!(rule.glides(), 0);
}

#[test]
fn the_steps_read_as_records_and_lines() {
    let start = Step::Start {
        instance: "band".into(),
        label: "Vox 1".into(),
        scene: "intro".into(),
        from_db: -20.0,
        to_db: -70.0,
        ms: 2000,
    };
    assert_eq!(
        serde_json::to_value(&start).unwrap(),
        json!({"what": "start", "instance": "band", "label": "Vox 1", "scene": "intro",
               "from_db": -20.0, "to_db": -70.0, "ms": 2000})
    );
    assert_eq!(
        step_line(&start),
        "scene \"intro\" glides band|Vox 1 from -20.0 dB to -70.0 dB in 2000 ms"
    );
    for (why, word) in [
        (Why::Done, "done"),
        (Why::Hand, "hand"),
        (Why::Replaced, "replaced"),
        (Why::Offline, "offline"),
    ] {
        let stop = Step::Stop {
            instance: "band".into(),
            label: "Vox 1".into(),
            scene: "intro".into(),
            why,
        };
        assert_eq!(why.as_str(), word);
        assert_eq!(serde_json::to_value(&stop).unwrap()["why"], json!(word));
        assert_eq!(serde_json::to_value(&stop).unwrap()["what"], json!("stop"));
        assert_eq!(
            step_line(&stop),
            format!("scene \"intro\" stopped the glide of band|Vox 1: {word}")
        );
    }
}
