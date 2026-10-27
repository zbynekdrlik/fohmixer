use super::*;

const SET: &str = "FOH Set";

fn strip(instance: &str, index: u32, track: &str, target: Option<i8>) -> LufsStrip {
    LufsStrip {
        instance: instance.to_string(),
        target: format!("live_set tracks {index}"),
        track: Some(track.to_string()),
        spec: LufsSpec { target },
        label: track.to_string(),
    }
}

fn reading(device: i32, parents: [&str; 4], first: i16, second: i16) -> Reading {
    Reading {
        kind: DeviceKind::Lufs,
        device,
        version: "v1.19.0".to_string(),
        parents: parents.map(str::to_string),
        first,
        second,
    }
}

fn on_vox(first: i16, second: i16) -> Reading {
    reading(1, ["Vox 1", SET, "-", "-"], first, second)
}

/// Both instances online and named `SET` at 0, the strips served at 0.
fn rule(strips: Vec<LufsStrip>) -> Lufs {
    let mut rule = Lufs::new();
    for instance in ["band", "master"] {
        rule.connected(instance, 0);
        rule.named(instance, SET, 0);
    }
    rule.set_strips(strips, true, 0);
    rule
}

fn confirms(acts: &[Act]) -> Vec<(String, String, u64)> {
    acts.iter()
        .filter_map(|a| match a {
            Act::Confirm {
                instance,
                track,
                ask,
                ..
            } => Some((instance.clone(), track.clone(), *ask)),
            _ => None,
        })
        .collect()
}

fn sends(acts: &[Act]) -> Vec<LufsItem> {
    acts.iter()
        .filter_map(|a| match a {
            Act::Send(item) => Some(item.clone()),
            _ => None,
        })
        .collect()
}

fn whats(acts: &[Act]) -> Vec<String> {
    acts.iter()
        .filter_map(|a| match a {
            Act::Log(line) => line.fields["what"].as_str().map(str::to_string),
            _ => None,
        })
        .collect()
}

fn item(instance: &str, index: u32, tenths: Option<i16>) -> LufsItem {
    LufsItem {
        instance: instance.to_string(),
        target: format!("live_set tracks {index}"),
        tenths,
        problem: None,
    }
}

fn key_of(r: &Reading) -> Key {
    Key::of(r)
}

/// A rule whose band strip on track 1 has matched the key of [`on_vox`].
fn matched(target: Option<i8>) -> Lufs {
    let mut rule = rule(vec![strip("band", 1, "Vox 1", target)]);
    let r = on_vox(-300, -253);
    let acts = rule.reading(&r, 10);
    let ask = confirms(&acts)[0].2;
    rule.answered(&key_of(&r), "band", ask, Answer::Ours, 20);
    rule
}

#[test]
fn the_constants() {
    assert_eq!(
        (
            STALE_MS,
            METER_STALE_MS,
            NO_METER_GRACE_MS,
            RECONFIRM_MS,
            STATE_LOG_MS
        ),
        (1000, 500, 3000, 10_000, 3000)
    );
    assert_eq!(stale_ms(DeviceKind::Lufs), 1000);
    assert_eq!(stale_ms(DeviceKind::Meter), 500);
    assert_eq!(kind_word(DeviceKind::Lufs), "lufs");
    assert_eq!(kind_word(DeviceKind::Meter), "meter");
}

#[test]
fn a_new_key_asks_each_candidate_instance() {
    let mut rule = rule(vec![
        strip("band", 3, "Vox 1", Some(-25)),
        strip("master", 5, "Vox 1", None),
        strip("band", 4, "Vox 2", Some(-25)),
    ]);
    let acts = rule.reading(&on_vox(-300, -253), 10);
    assert_eq!(whats(&acts)[0], "found");
    let asked = confirms(&acts);
    assert_eq!(
        asked
            .iter()
            .map(|(i, t, _)| (i.as_str(), t.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("band", "live_set tracks 3"),
            ("master", "live_set tracks 5")
        ]
    );
    assert_eq!(asked[0].2, asked[1].2);
    // A second reading while asking asks nothing again.
    assert!(confirms(&rule.reading(&on_vox(-300, -253), 20)).is_empty());
    assert_eq!(rule.device_status(20)[0].state, "asking");
}

#[test]
fn only_the_instance_that_holds_it_matches() {
    let mut rule = rule(vec![
        strip("band", 3, "Vox 1", Some(-25)),
        strip("master", 3, "Vox 1", Some(-25)),
    ]);
    let r = on_vox(-300, -253);
    let ask = confirms(&rule.reading(&r, 10))[0].2;
    assert!(
        rule.answered(&key_of(&r), "band", ask, Answer::Ours, 20)
            .iter()
            .all(|a| !matches!(a, Act::Send(_)))
    );
    let acts = rule.answered(&key_of(&r), "master", ask, Answer::NotOurs, 30);
    assert_eq!(whats(&acts)[0], "matched");
    assert_eq!(sends(&acts), vec![item("band", 3, Some(-253))]);
    let status = rule.device_status(30);
    assert_eq!(status[0].state, "matched");
    assert_eq!(status[0].instance.as_deref(), Some("band"));
    // The next reading reaches the band strip only.
    assert_eq!(
        sends(&rule.reading(&on_vox(-300, -240), 40)),
        vec![item("band", 3, Some(-240))]
    );
    // An answer of an older walk, or of a Live not asked, is void.
    assert!(
        rule.answered(&key_of(&r), "band", ask + 1, Answer::NotOurs, 50)
            .is_empty()
    );
}

#[test]
fn both_confirm_is_two_meters_and_nothing_shown() {
    let mut rule = rule(vec![
        strip("band", 3, "Vox 1", Some(-25)),
        strip("master", 3, "Vox 1", Some(-25)),
    ]);
    let r = on_vox(-300, -253);
    let ask = confirms(&rule.reading(&r, 10))[0].2;
    rule.answered(&key_of(&r), "band", ask, Answer::Ours, 20);
    let acts = rule.answered(&key_of(&r), "master", ask, Answer::Ours, 30);
    assert_eq!(whats(&acts)[0], "two");
    let problem = Some(LufsProblem::TwoLufsMeters);
    let mut want = vec![item("band", 3, None), item("master", 3, None)];
    for w in &mut want {
        w.problem = problem;
    }
    assert_eq!(sends(&acts), want);
    assert!(sends(&rule.reading(&r, 40)).is_empty(), "nothing shown");
    assert_eq!(rule.device_status(40)[0].state, "two");
}

#[test]
fn a_live_with_two_at_the_place_is_two() {
    let mut rule = rule(vec![strip("band", 3, "Vox 1", Some(-25))]);
    let r = on_vox(-300, -253);
    let ask = confirms(&rule.reading(&r, 10))[0].2;
    let acts = rule.answered(&key_of(&r), "band", ask, Answer::Two, 20);
    assert_eq!(sends(&acts)[0].problem, Some(LufsProblem::TwoLufsMeters));
    assert_eq!(sends(&acts)[0].tenths, None);
}

#[test]
fn two_matched_keys_on_one_point_are_two_meters() {
    let mut rule = matched(Some(-25));
    let other = reading(2, ["Vox 1", SET, "-", "-"], -200, -200);
    let ask = confirms(&rule.reading(&other, 30))[0].2;
    let acts = rule.answered(&key_of(&other), "band", ask, Answer::Ours, 40);
    let mut want = item("band", 1, None);
    want.problem = Some(LufsProblem::TwoLufsMeters);
    assert_eq!(sends(&acts), vec![want]);
}

#[test]
fn none_confirms_asks_again_after_ten_seconds_or_a_track_change() {
    let mut rule = rule(vec![strip("band", 3, "Vox 1", Some(-25))]);
    let r = on_vox(-300, -253);
    let ask = confirms(&rule.reading(&r, 10))[0].2;
    let acts = rule.answered(&key_of(&r), "band", ask, Answer::NotOurs, 20);
    assert_eq!(whats(&acts), vec!["unmatched".to_string()]);
    assert!(matches!(&acts[0], Act::Log(line) if line.warn));
    assert!(confirms(&rule.reading(&r, 10_019)).is_empty());
    let again = confirms(&rule.reading(&r, 10_020));
    assert_eq!(again.len(), 1);
    // Unmatched again: recorded once per key.
    let acts = rule.answered(
        &key_of(&r),
        "band",
        again[0].2,
        Answer::Failed("x".into()),
        10_030,
    );
    assert!(whats(&acts).is_empty());
    assert!(confirms(&rule.reading(&r, 10_040)).is_empty());
    rule.tracks_changed();
    assert_eq!(confirms(&rule.reading(&r, 10_050)).len(), 1);
}

#[test]
fn a_frame_strip_matches_by_track_name() {
    let mut rule = rule(vec![strip("master", 7, "Vocals", None)]);
    let vocal = reading(4, ["Vocal", SET, "-", "-"], -200, -200);
    let acts = rule.reading(&vocal, 10);
    assert!(confirms(&acts).is_empty());
    assert_eq!(whats(&acts), vec!["found", "unmatched"]);
    let vocals = reading(5, ["LUFS", "LUFS", "Vocals", SET], -200, -200);
    assert_eq!(confirms(&rule.reading(&vocals, 20)).len(), 1);
}

#[test]
fn a_set_of_another_name_is_no_candidate() {
    let mut rule = rule(vec![strip("band", 1, "Vox 1", Some(-25))]);
    let r = reading(1, ["Vox 1", "Other Set", "-", "-"], -300, -300);
    assert!(confirms(&rule.reading(&r, 10)).is_empty());
}

#[test]
fn a_lufs_device_on_an_untagged_track_matches_nothing() {
    let mut rule = rule(vec![strip("band", 1, "Vox 1", Some(-25))]);
    let r = reading(1, ["Keys", SET, "-", "-"], -300, -300);
    let acts = rule.reading(&r, 10);
    assert!(confirms(&acts).is_empty());
    assert!(sends(&acts).is_empty());
    assert_eq!(rule.device_status(10)[0].state, "unmatched");
}

#[test]
fn a_key_before_its_lives_set_name_waits_without_a_record() {
    let mut rule = Lufs::new();
    rule.connected("band", 0);
    rule.set_strips(vec![strip("band", 1, "Vox 1", Some(-25))], true, 0);
    let acts = rule.reading(&on_vox(-300, -253), 10);
    assert_eq!(whats(&acts), vec!["found"]);
    assert!(confirms(&acts).is_empty());
    rule.named("band", SET, 20);
    assert_eq!(confirms(&rule.reading(&on_vox(-300, -253), 30)).len(), 1);
}

#[test]
fn an_input_point_sends_v_an_output_point_sends_s() {
    let mut rule = matched(Some(-25));
    assert_eq!(
        sends(&rule.reading(&on_vox(-300, -253), 30)),
        Vec::<LufsItem>::new(),
        "unchanged: nothing sent"
    );
    assert_eq!(
        sends(&rule.reading(&on_vox(-310, -250), 40)),
        vec![item("band", 1, Some(-250))]
    );
    // An output point: nothing until its fader is known, then S + fader dB.
    let mut out = matched(None);
    assert_eq!(sends(&out.reading(&on_vox(-300, -253), 30)), vec![]);
    let unity = fohmixer_proto::volume::volume_of(0.0);
    let acts = out.fader(
        "band",
        "live_set tracks 1",
        Fader {
            volume: Some(unity),
            mute: Some(false),
        },
        40,
    );
    assert_eq!(sends(&acts), vec![item("band", 1, Some(-300))]);
    let acts = out.fader(
        "band",
        "live_set tracks 1",
        Fader {
            volume: Some(fohmixer_proto::volume::volume_of(-10.0)),
            mute: Some(false),
        },
        50,
    );
    assert_eq!(sends(&acts), vec![item("band", 1, Some(-400))]);
    let acts = out.fader(
        "band",
        "live_set tracks 1",
        Fader {
            volume: Some(unity),
            mute: Some(true),
        },
        60,
    );
    assert_eq!(sends(&acts), vec![item("band", 1, Some(SILENT_TENTHS))]);
    assert_eq!(
        out.output_points(),
        vec![("band".to_string(), "live_set tracks 1".to_string())]
    );
}

#[test]
fn the_out_rule_on_its_boundaries() {
    let out = LufsSpec { target: None };
    let inp = LufsSpec { target: Some(-25) };
    let open = |db: f64| Fader {
        volume: Some(fohmixer_proto::volume::volume_of(db)),
        mute: Some(false),
    };
    assert_eq!(reading_of(inp, -300, -253, Fader::default()), Some(-253));
    assert_eq!(reading_of(inp, -300, -253, open(-20.0)), Some(-253));
    assert_eq!(reading_of(out, -300, -253, Fader::default()), None);
    let volume_only = Fader {
        volume: Some(0.85),
        mute: None,
    };
    assert_eq!(reading_of(out, -300, -253, volume_only), None);
    let mute_only = Fader {
        volume: None,
        mute: Some(false),
    };
    assert_eq!(reading_of(out, -300, -253, mute_only), None);
    let muted = Fader {
        volume: None,
        mute: Some(true),
    };
    assert_eq!(reading_of(out, -300, -253, muted), Some(SILENT_TENTHS));
    assert_eq!(reading_of(out, -300, -253, open(6.0)), Some(-240));
    assert_eq!(post_fader(-300, 0.0), -300);
    assert_eq!(post_fader(-300, -3.04), -330);
    assert_eq!(post_fader(-300, -3.06), -331);
    assert_eq!(post_fader(-300, 2.5), -275);
    assert_eq!(post_fader(-700, 6.0), -700, "silence stays silence");
    assert_eq!(post_fader(-699, 0.0), -699);
    assert_eq!(post_fader(-699, -0.1), -700);
    assert_eq!(post_fader(-650, -20.0), -700, "clamped to silence");
    assert_eq!(post_fader(-300, f64::NEG_INFINITY), -700);
    assert_eq!(post_fader(-300, f64::NAN), -700);
    assert_eq!(post_fader(0, 6.0), 60);
    assert_eq!(post_fader(i16::MAX, 10.0), i16::MAX);
}

#[test]
fn stale_at_1000_ms_not_at_999() {
    let mut rule = matched(Some(-25));
    rule.reading(&on_vox(-300, -253), 1000);
    assert!(rule.tick(1999).is_empty());
    let acts = rule.tick(2000);
    assert_eq!(whats(&acts)[0], "lost");
    assert_eq!(sends(&acts), vec![item("band", 1, None)]);
    assert!(
        rule.tick(2100)
            .iter()
            .all(|a| !matches!(a, Act::Log(l) if l.fields["what"] == "lost"))
    );
    assert!(rule.device_status(2100).is_empty());
    assert!(is_stale(DeviceKind::Meter, 0, 500));
    assert!(!is_stale(DeviceKind::Meter, 0, 499));
}

#[test]
fn the_lost_record_says_how_long_and_its_last_values() {
    let mut rule = matched(Some(-25));
    rule.reading(&on_vox(-310, -251), 500);
    let acts = rule.tick(1500);
    let Some(Act::Log(line)) = acts.iter().find(|a| matches!(a, Act::Log(_))) else {
        panic!("no record");
    };
    assert_eq!(line.fields["lived_ms"], 490);
    assert_eq!(line.fields["first"], -310);
    assert_eq!(line.fields["second"], -251);
    assert_eq!(line.fields["instance"], "band");
    assert!(!line.warn);
}

#[test]
fn a_moved_device_is_a_new_key_and_the_old_one_goes_stale() {
    let mut rule = matched(Some(-25));
    let moved = reading(1, ["LUFS", "LUFS", "Vox 1", SET], -300, -260);
    let acts = rule.reading(&moved, 500);
    assert_eq!(whats(&acts)[0], "found");
    let ask = confirms(&acts)[0].2;
    let acts = rule.answered(&key_of(&moved), "band", ask, Answer::Ours, 510);
    // Two keys now matched there until the old one goes stale.
    assert_eq!(sends(&acts)[0].problem, Some(LufsProblem::TwoLufsMeters));
    rule.reading(&moved, 1100);
    let acts = rule.tick(1100);
    assert_eq!(whats(&acts)[0], "lost");
    assert_eq!(sends(&acts), vec![item("band", 1, Some(-260))]);
}

#[test]
fn no_lufs_meter_after_the_grace() {
    let mut rule = Lufs::new();
    rule.set_strips(vec![strip("band", 1, "Vox 1", Some(-25))], true, 0);
    rule.connected("band", 1000);
    rule.named("band", SET, 1000);
    assert!(sends(&rule.tick(3999)).is_empty());
    let acts = rule.tick(4000);
    let mut want = item("band", 1, None);
    want.problem = Some(LufsProblem::NoLufsMeter);
    assert_eq!(sends(&acts), vec![want]);
    assert!(whats(&acts).contains(&"problem".to_string()));
    // A new layout starts the grace again.
    let acts = rule.set_strips(vec![strip("band", 1, "Vox 1", Some(-25))], true, 5000);
    assert_eq!(sends(&acts), vec![item("band", 1, None)]);
    // A device clears it.
    let r = on_vox(-300, -253);
    let ask = confirms(&rule.reading(&r, 6000))[0].2;
    let acts = rule.answered(&key_of(&r), "band", ask, Answer::Ours, 6010);
    assert_eq!(sends(&acts), vec![item("band", 1, Some(-253))]);
    assert!(grace_over(1000, Some(5000), 8000));
    assert!(!grace_over(1000, Some(5000), 7999));
    assert!(grace_over(1000, None, 4000));
    assert!(!grace_over(1000, None, 3999));
}

#[test]
fn an_offline_point_has_no_problem() {
    let mut rule = Lufs::new();
    rule.set_strips(vec![strip("band", 1, "Vox 1", Some(-25))], true, 0);
    assert!(sends(&rule.tick(10_000)).is_empty());
}

#[test]
fn the_problems_rule() {
    assert_eq!(problem_of(0, false, false), None);
    assert_eq!(problem_of(0, false, true), Some(LufsProblem::NoLufsMeter));
    assert_eq!(problem_of(1, false, true), None);
    assert_eq!(
        problem_of(2, false, false),
        Some(LufsProblem::TwoLufsMeters)
    );
    assert_eq!(problem_of(0, true, true), Some(LufsProblem::TwoLufsMeters));
    assert_eq!(problem_of(1, true, false), Some(LufsProblem::TwoLufsMeters));
}

#[test]
fn offline_drops_the_instances_matches() {
    let mut rule = matched(Some(-25));
    let acts = rule.offline("band", 30);
    assert_eq!(sends(&acts), vec![item("band", 1, None)]);
    assert!(sends(&rule.reading(&on_vox(-300, -253), 40)).is_empty());
    // Back online and named: asked again at the next reading.
    rule.connected("band", 50);
    rule.named("band", SET, 50);
    assert_eq!(confirms(&rule.reading(&on_vox(-300, -253), 60)).len(), 1);
}

#[test]
fn offline_answers_a_walk_still_out() {
    let mut rule = rule(vec![
        strip("band", 3, "Vox 1", Some(-25)),
        strip("master", 3, "Vox 1", Some(-25)),
    ]);
    let r = on_vox(-300, -253);
    let ask = confirms(&rule.reading(&r, 10))[0].2;
    rule.answered(&key_of(&r), "band", ask, Answer::Ours, 20);
    let acts = rule.offline("master", 30);
    assert_eq!(whats(&acts)[0], "matched");
    assert_eq!(sends(&acts), vec![item("band", 3, Some(-253))]);
}

#[test]
fn state_line_once_per_three_seconds() {
    assert!(state_due(LufsState::None, LufsState::Fine, None));
    assert!(!state_due(LufsState::Fine, LufsState::Fine, None));
    assert!(!state_due(LufsState::Fine, LufsState::Warn, Some(2999)));
    assert!(state_due(LufsState::Fine, LufsState::Warn, Some(3000)));
    let mut rule = matched(Some(-25));
    // Matched at 20: `fine` recorded then.
    let states = |acts: &[Act]| {
        acts.iter()
            .filter_map(|a| match a {
                Act::Log(l) if l.fields["what"] == "state" => {
                    l.fields["state"].as_str().map(str::to_string)
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    assert!(
        states(&rule.reading(&on_vox(-300, -200), 100)).is_empty(),
        "within 3 s"
    );
    assert_eq!(
        states(&rule.reading(&on_vox(-300, -200), 3020)),
        vec!["warn"]
    );
    assert!(states(&rule.reading(&on_vox(-300, -100), 3100)).is_empty());
    assert_eq!(
        states(&rule.reading(&on_vox(-300, -100), 6020)),
        vec!["far"]
    );
}

#[test]
fn a_page_gets_every_point_at_attach_and_the_status_lists_them() {
    let mut rule = matched(Some(-25));
    rule.set_strips(
        vec![
            strip("band", 1, "Vox 1", Some(-25)),
            strip("band", 2, "Vox 2", Some(-25)),
        ],
        false,
        30,
    );
    assert_eq!(
        rule.items(),
        vec![item("band", 1, Some(-253)), item("band", 2, None)]
    );
    let strips = rule.strip_status();
    assert_eq!(strips[0].state, LufsState::Fine);
    assert_eq!(strips[0].tenths, Some(-253));
    assert_eq!(strips[1].state, LufsState::None);
    assert_eq!(strips[1].track.as_deref(), Some("Vox 2"));
    let devices = rule.device_status(130);
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0].age_ms, 120);
    assert_eq!((devices[0].first, devices[0].second), (-300, -253));
    assert_eq!(devices[0].version, "v1.19.0");
    assert_eq!(devices[0].kind, "lufs");
    assert_eq!(rule.strips().len(), 2);
}

#[test]
fn candidates_match_track_and_set() {
    let strips = vec![
        strip("band", 1, "Vox 1", Some(-25)),
        strip("master", 1, "Vox 1", None),
        LufsStrip {
            instance: "band".to_string(),
            target: "live_set tracks 2".to_string(),
            track: None,
            spec: LufsSpec { target: None },
            label: "#3".to_string(),
        },
    ];
    let sets = BTreeMap::from([
        ("band".to_string(), "A".to_string()),
        ("master".to_string(), "B".to_string()),
    ]);
    let got: Vec<&str> = candidates(&strips, &sets, "Vox 1", "A")
        .iter()
        .map(|s| s.instance.as_str())
        .collect();
    assert_eq!(got, vec!["band"]);
    assert!(candidates(&strips, &sets, "Vox 2", "A").is_empty());
    assert!(candidates(&strips, &sets, "Vox 1", "C").is_empty());
}

#[test]
fn the_records_are_warn_only_for_problems() {
    let key = key_of(&on_vox(0, 0));
    let s = strip("band", 1, "Vox 1", Some(-25));
    assert!(!found_line(&key, "v1").warn);
    assert!(!matched_line(&key, "band").warn);
    assert!(!lost_line(&key, 1, (0, 0), None).warn);
    assert!(unmatched_line(&key, &BTreeSet::new()).warn);
    assert!(two_line(&key, &BTreeSet::new()).warn);
    assert!(problem_line(&s, Some(LufsProblem::NoLufsMeter)).warn);
    assert_eq!(
        problem_line(&s, Some(LufsProblem::NoLufsMeter)).fields["problem"],
        "no LUFS meter"
    );
    assert!(!problem_line(&s, None).warn);
    assert!(problem_line(&s, None).fields.get("warn").is_none());
    assert_eq!(unmatched_line(&key, &BTreeSet::new()).fields["warn"], true);
    assert!(!state_line(&s, LufsState::Far, Some(-100)).warn);
    assert_eq!(found_line(&key, "v1").fields["parents"][0], "Vox 1");
}

#[test]
fn an_unchanged_reading_is_never_sent_again() {
    // The owner, 27.10: a point goes to the pages only when its tenths
    // changed, never once per datagram.
    let mut rule = matched(Some(-25));
    for n in 0..20 {
        let acts = rule.reading(&on_vox(-300 - n, -253), 30 + 200 * n as u64);
        assert!(sends(&acts).is_empty(), "datagram {n}: V unchanged");
    }
    assert_eq!(
        sends(&rule.reading(&on_vox(-300, -252), 5000)),
        vec![item("band", 1, Some(-252))]
    );
    assert!(sends(&rule.reading(&on_vox(-300, -252), 5200)).is_empty());
    assert!(sends(&rule.tick(5300)).is_empty(), "a tick resends nothing");
}
