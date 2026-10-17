use super::*;

fn fader(label: &str, index: u32) -> RuleFader {
    RuleFader {
        instance: "band".into(),
        label: label.into(),
        target: format!("live_set tracks {index} mixer_device volume"),
    }
}

const ST: &str = "band|live_set tracks 4 mixer_device volume|value";
const PA: &str = "band|live_set tracks 5 mixer_device volume|value";

/// Two faders at 0.85 and 0.6, AUT as asked, the transport as given.
fn rule(aut: bool, playing: Option<bool>) -> Reverb {
    let mut r = Reverb::new(&Saved { aut, low: None });
    r.set_faders(vec![fader("Rev ST", 4), fader("Rev PA", 5)]);
    r.live(ST, 0.85);
    r.live(PA, 0.6);
    if let Some(p) = playing {
        r.transport("band", p, 0.0);
    }
    r
}

fn writes(acts: &[Act]) -> Vec<(&str, f64, bool)> {
    acts.iter()
        .filter_map(|a| match a {
            Act::Write {
                key, value, last, ..
            } => Some((key.as_str(), *value, *last)),
            _ => None,
        })
        .collect()
}

fn steps(acts: &[Act]) -> Vec<&Step> {
    acts.iter()
        .filter_map(|a| match a {
            Act::Step(s) => Some(s),
            _ => None,
        })
        .collect()
}

/// Ticks every STEP_MS from `from` to `to` (both included).
fn run(r: &mut Reverb, from: f64, to: f64) -> Vec<Act> {
    let n = ((to - from) / STEP_MS) as usize;
    (0..=n)
        .flat_map(|i| r.tick(from + i as f64 * STEP_MS))
        .collect()
}

#[test]
fn the_rules_numbers() {
    assert_eq!(
        (
            DROP_DB,
            STOP_WAIT_MS,
            LOWER_MS,
            TAP_MS,
            RETURN_MS,
            STEP_MS,
            SAME_VOLUME
        ),
        (10.0, 2000.0, 2000.0, 500.0, 500.0, 25.0, 0.000_001)
    );
}

#[test]
fn the_thresholds_are_exact() {
    assert!(same_volume(0.0, 1e-6));
    assert!(!same_volume(0.0, 1e-6_f64.next_up()));
    assert!(same_volume(1e-6, 0.0));
    assert!(same_volume(0.5, 0.5), "a level is its own");
    assert!(due(1000.0, 1000.0));
    assert!(!due(1000.0, 1000.0_f64.next_down()));
    assert!(due(1000.0, 1000.0_f64.next_up()));
    assert!(silent(0.0));
    assert!(!silent(f64::MIN_POSITIVE));
    assert!((value2db(lowered(0.85)) + 10.0).abs() < 0.01);
    assert!((value2db(lowered(0.6)) - (value2db(0.6) - 10.0)).abs() < 0.01);
    assert_eq!(lowered(0.0), 0.0);
}

#[test]
fn a_glide_is_linear_in_db_and_lands_exactly() {
    let g = Glide::new(0.85, 0.6, 1000.0, 2000.0);
    assert_eq!(g.at(1000.0), (volume_of(value2db(0.85)), false));
    let (half, done) = g.at(2000.0);
    assert!(!done);
    assert!((value2db(half) - (value2db(0.85) + value2db(0.6)) / 2.0).abs() < 0.01);
    assert!(!g.at(3000.0_f64.next_down()).1);
    assert_eq!(g.at(3000.0), (0.6, true), "exactly its end");
    assert_eq!(g.at(9000.0), (0.6, true));
    // From −∞ (the floor) up: no NaN, the end exact.
    let up = Glide::new(0.0, 0.3, 0.0, 500.0);
    assert!(up.at(250.0).0.is_finite() && up.at(250.0).0 > 0.0);
    assert_eq!(up.at(500.0), (0.3, true));
}

#[test]
fn the_faders_come_from_the_layout_each_with_its_write_key() {
    let mut rule = Reverb::new(&Saved::default());
    assert_eq!(
        rule.set_faders(vec![fader("Rev ST", 4), fader("Rev PA", 5)]),
        vec![
            Act::Step(Step::Faders {
                labels: vec!["Rev ST".into(), "Rev PA".into()]
            }),
            Act::Save,
            Act::Tell,
        ]
    );
    assert_eq!(
        rule.set_faders(vec![fader("Rev ST", 4), fader("Rev PA", 5)]),
        vec![],
        "the same faders: nothing"
    );
    let state = rule.state();
    assert_eq!(
        state
            .faders
            .iter()
            .map(|f| (f.key.as_str(), f.label.as_str()))
            .collect::<Vec<_>>(),
        vec![(ST, "Rev ST"), (PA, "Rev PA")]
    );
    assert!(!state.aut && state.low.is_none() && state.playing.is_none() && !state.waiting);
    assert_eq!(rule.status().faders, 2);
    // A layout without them: told, and the line says so.
    assert_eq!(
        rule.set_faders(vec![]),
        vec![
            Act::Step(Step::Faders { labels: vec![] }),
            Act::Save,
            Act::Tell
        ]
    );
    assert_eq!(rule.status().faders, 0);
}

#[test]
fn each_step_has_its_line() {
    assert_eq!(
        step_line(&Step::Faders {
            labels: vec!["Rev ST".into(), "Rev PA".into()]
        }),
        "REVERB's faders: Rev ST, Rev PA"
    );
    assert_eq!(
        step_line(&Step::Faders { labels: vec![] }),
        "REVERB has no faders (no +REVERB strip)"
    );
    assert_eq!(
        serde_json::to_value(Step::Faders {
            labels: vec!["A".into()]
        })
        .unwrap(),
        serde_json::json!({"step": "faders", "labels": ["A"]})
    );
    assert!(
        Reverb::new(&Saved {
            aut: true,
            low: None
        })
        .state()
        .aut
    );
    assert!(
        Reverb::new(&Saved {
            aut: true,
            low: None
        })
        .status()
        .aut
    );
}

#[test]
fn a_stop_lowers_after_the_wait_and_a_start_returns_exactly() {
    let mut r = rule(true, Some(true));
    let acts = r.transport("band", false, 1000.0);
    assert_eq!(steps(&acts), vec![&Step::Wait { ms: 2000.0 }]);
    assert!(r.state().waiting && r.state().low.is_none());
    assert_eq!(r.tick(3000.0_f64.next_down()), vec![], "the wait not over");
    let acts = r.tick(3000.0);
    assert!(matches!(steps(&acts)[0], Step::Lower { why: Low::Stop, faders } if faders.len() == 2));
    assert!(acts.contains(&Act::Tell) && acts.contains(&Act::Save));
    assert_eq!(r.state().low, Some(Low::Stop));
    assert!(!r.state().waiting);
    assert_eq!(
        r.state().faders.iter().map(|f| f.back).collect::<Vec<_>>(),
        vec![Some(0.85), Some(0.6)]
    );
    let acts = run(&mut r, 3025.0, 5000.0);
    let ends: Vec<_> = writes(&acts).into_iter().filter(|w| w.2).collect();
    assert_eq!(
        ends,
        vec![(ST, lowered(0.85), true), (PA, lowered(0.6), true)]
    );
    assert!(steps(&acts).contains(&&Step::Arrived {
        label: "Rev PA".into(),
        value: lowered(0.6)
    }));
    assert_eq!(r.next_tick(5000.0), None, "nothing left to do");
    let acts = r.transport("band", true, 6000.0);
    assert!(matches!(steps(&acts)[0], Step::Return { faders } if faders.len() == 2));
    assert_eq!(r.state().low, None);
    let acts = run(&mut r, 6025.0, 6500.0);
    let ends: Vec<_> = writes(&acts).into_iter().filter(|w| w.2).collect();
    assert_eq!(ends, vec![(ST, 0.85, true), (PA, 0.6, true)]);
    // Every step counted: the wait's end writes both at p = 0 (2), the
    // lowering 3025..=5000 is 80 ticks × 2, the return 6025..=6500 is
    // 20 ticks × 2: 2 + 160 + 40.
    assert_eq!(r.status().writes, 202);
    assert_eq!(r.status().low, None);
}

#[test]
fn a_start_within_the_wait_lowers_nothing_and_without_aut_nothing_moves() {
    let mut r = rule(true, Some(true));
    r.transport("band", false, 0.0);
    assert_eq!(
        steps(&r.transport("band", true, 1999.0)),
        vec![&Step::Cancelled]
    );
    assert_eq!(run(&mut r, 2000.0, 5000.0), vec![]);
    let mut off = rule(false, Some(true));
    assert!(steps(&off.transport("band", false, 0.0)).is_empty());
    assert_eq!(run(&mut off, 0.0, 5000.0), vec![]);
    assert_eq!(
        off.transport("master", true, 0.0),
        vec![],
        "another Live's transport"
    );
    assert_eq!(
        off.transport("band", false, 10.0),
        vec![],
        "the same value: nothing"
    );
}

#[test]
fn the_first_transport_value_never_lowers() {
    let mut r = rule(true, None);
    assert!(steps(&r.transport("band", false, 0.0)).is_empty());
    assert_eq!(run(&mut r, 0.0, 5000.0), vec![]);
    // A first value that plays, with nothing low: nothing either.
    let mut playing = rule(true, None);
    assert!(steps(&playing.transport("band", true, 0.0)).is_empty());
    // A restored lowering returns on a first value that plays.
    let saved = Saved {
        aut: true,
        low: Some(SavedLow {
            by: Low::Stop,
            faders: vec![SavedFader {
                instance: "band".into(),
                label: "Rev ST".into(),
                back: 0.85,
                left: Some(lowered(0.85)),
            }],
        }),
    };
    let mut back = Reverb::new(&saved);
    back.set_faders(vec![fader("Rev ST", 4), fader("Rev PA", 5)]);
    back.live(ST, lowered(0.85));
    assert!(matches!(
        steps(&back.transport("band", true, 0.0))[0],
        Step::Return { .. }
    ));
    // A tap's lowering, or AUT off, does not return on a first value.
    for (aut, by) in [(true, Low::Tap), (false, Low::Stop)] {
        let mut kept = Reverb::new(&Saved {
            aut,
            low: Some(SavedLow {
                by,
                faders: saved.low.clone().unwrap().faders,
            }),
        });
        kept.set_faders(vec![fader("Rev ST", 4)]);
        assert!(steps(&kept.transport("band", true, 0.0)).is_empty());
        assert!(kept.state().low.is_some());
    }
}

#[test]
fn a_tap_lowers_two_taps_agree_and_the_next_returns() {
    let mut r = rule(false, Some(true));
    assert!(matches!(
        steps(&r.tap(true, 0.0))[1],
        Step::Lower { why: Low::Tap, .. }
    ));
    assert_eq!(
        steps(&r.tap(true, 10.0)),
        vec![&Step::Tap { low: true }],
        "already low: one lowering"
    );
    assert!(writes(&run(&mut r, 25.0, 500.0)).contains(&(PA, lowered(0.6), true)));
    // AUT off: Live's stop and start move nothing.
    assert!(steps(&r.transport("band", false, 600.0)).is_empty());
    assert!(steps(&r.transport("band", true, 700.0)).is_empty());
    assert_eq!(r.state().low, Some(Low::Tap));
    assert!(matches!(
        steps(&r.tap(false, 1000.0))[1],
        Step::Return { .. }
    ));
    assert!(writes(&run(&mut r, 1025.0, 1500.0)).contains(&(PA, 0.6, true)));
    assert_eq!(
        steps(&r.tap(false, 2000.0)),
        vec![&Step::Tap { low: false }],
        "already back: nothing"
    );
}

#[test]
fn a_tap_in_aut_overrides_until_the_next_change() {
    let mut r = rule(true, Some(true));
    r.tap(true, 0.0);
    assert_eq!(r.state().low, Some(Low::Tap), "a prayer in a song");
    run(&mut r, 25.0, 500.0);
    r.transport("band", false, 1000.0);
    let acts = r.tick(3000.0);
    assert!(writes(&acts).is_empty());
    assert_eq!(
        step_line(steps(&acts)[0]),
        "REVERB AUT takes over the reverb faders already low: they return at the start"
    );
    assert_eq!(
        r.state().low,
        Some(Low::Stop),
        "held already: none lowered again"
    );
    assert!(matches!(
        steps(&r.transport("band", true, 4000.0))[0],
        Step::Return { .. }
    ));
}

#[test]
fn a_tap_while_aut_waits_lowers_for_the_stop_and_a_tap_back_ends_the_wait() {
    let mut r = rule(true, Some(true));
    r.transport("band", false, 0.0);
    let acts = r.tap(true, 100.0);
    assert_eq!(steps(&acts)[1], &Step::Cancelled, "the tap took over");
    assert!(matches!(
        steps(&acts)[2],
        Step::Lower { why: Low::Stop, .. }
    ));
    assert!(!r.state().waiting);
    let mut r = rule(true, Some(true));
    r.transport("band", false, 0.0);
    assert_eq!(
        steps(&r.tap(false, 100.0)),
        vec![&Step::Tap { low: false }, &Step::Cancelled],
        "nothing low: the wait ends"
    );
    assert_eq!(run(&mut r, 2000.0, 4500.0), vec![]);
}

#[test]
fn a_page_set_lets_go_during_the_wait_the_glide_and_the_rest() {
    let mut r = rule(true, Some(true));
    r.transport("band", false, 0.0);
    assert!(matches!(
        steps(&r.hand(PA))[0],
        Step::Hand { by: By::Page, .. }
    ));
    r.tick(2000.0);
    assert_eq!(r.state().faders[1].back, None);
    assert!(r.state().faders[1].hand);
    assert!(
        run(&mut r, 2025.0, 4000.0)
            .iter()
            .all(|a| !matches!(a, Act::Write { key, .. } if key == PA))
    );
    let mut r = rule(true, Some(true));
    r.transport("band", false, 0.0);
    r.tick(2000.0);
    run(&mut r, 2025.0, 2500.0);
    r.hand(ST);
    assert!(
        run(&mut r, 2525.0, 4500.0)
            .iter()
            .all(|a| !matches!(a, Act::Write { key, .. } if key == ST))
    );
    let acts = r.transport("band", true, 5000.0);
    let Step::Return { faders } = steps(&acts)[0] else {
        panic!("{acts:?}")
    };
    assert_eq!(
        faders.iter().map(|m| m.label.as_str()).collect::<Vec<_>>(),
        vec!["Rev PA"]
    );
    assert!(!r.state().faders[0].hand, "the rule let everything go");
    assert_eq!(r.hand(ST), vec![], "not held: a set changes nothing");
    assert_eq!(
        r.hand("band|live_set tracks 9 mixer_device volume|value"),
        vec![]
    );
    assert_eq!(r.status().hands, 1);
    // A page's set during the return only stops that glide.
    assert_eq!(
        r.hand(PA),
        vec![Act::Step(Step::Stopped {
            label: "Rev PA".into(),
            why: "hand".into()
        })]
    );
    assert_eq!(r.next_tick(5000.0), None);
    assert_eq!(r.status().hands, 1);
}

#[test]
fn a_wait_whose_every_fader_is_in_the_hand_ends_with_nothing_lowered() {
    let mut r = rule(true, Some(true));
    r.transport("band", false, 0.0);
    r.hand(ST);
    r.hand(PA);
    let acts = r.tick(2000.0);
    assert_eq!(steps(&acts), vec![&Step::Cancelled]);
    assert!(acts.contains(&Act::Tell), "the pages see the wait end");
    assert!(!r.state().waiting && r.state().low.is_none());
    assert_eq!(r.next_tick(2000.0), None);
}

#[test]
fn a_live_value_off_where_the_rule_left_it_lets_go_only_once_settled() {
    let mut r = rule(false, Some(true));
    r.tap(true, 0.0);
    run(&mut r, 25.0, 500.0);
    assert_eq!(
        r.live(PA, 0.5),
        vec![],
        "an echo before the last write's ack"
    );
    r.applied(PA);
    assert_eq!(r.live(PA, lowered(0.6)), vec![], "where the rule left it");
    let acts = r.live(PA, lowered(0.6) + 1e-3);
    assert!(matches!(steps(&acts)[0], Step::Hand { by: By::Live, .. }));
    assert!(r.state().faders[1].hand && r.state().faders[1].back.is_none());
    assert_eq!(r.status().hands, 1);
    // An ack while its glide still runs settles nothing.
    let mut r = rule(false, Some(true));
    r.tap(true, 0.0);
    r.tick(25.0);
    r.applied(ST);
    assert_eq!(r.live(ST, 0.5), vec![]);
    run(&mut r, 50.0, 500.0);
    assert_eq!(
        r.live(ST, 0.5),
        vec![],
        "settled only by its last write's ack"
    );
}

#[test]
fn aut_off_while_lowered_keeps_them_and_aut_on_waits_only_while_live_stands() {
    let mut r = rule(true, Some(true));
    r.transport("band", false, 0.0);
    r.tick(2000.0);
    let acts = r.set_aut(false, 3000.0);
    assert!(steps(&acts).contains(&&Step::Off) && writes(&acts).is_empty());
    assert_eq!(r.state().low, Some(Low::Tap));
    assert!(matches!(
        steps(&r.tap(false, 3100.0))[1],
        Step::Return { .. }
    ));
    let mut r = rule(false, Some(false));
    assert!(steps(&r.set_aut(true, 0.0)).contains(&&Step::Wait { ms: 2000.0 }));
    assert!(matches!(
        steps(&r.tick(2000.0))[0],
        Step::Lower { why: Low::Stop, .. }
    ));
    let mut r = rule(false, Some(true));
    assert!(
        !r.set_aut(true, 0.0)
            .iter()
            .any(|a| matches!(a, Act::Step(Step::Wait { .. })))
    );
    assert_eq!(r.set_aut(true, 1.0), vec![Act::Tell]);
    // AUT off during the wait ends it; on again over a tap's lowering while
    // Live stands makes it the stop's.
    let mut r = rule(true, Some(true));
    r.transport("band", false, 0.0);
    assert!(steps(&r.set_aut(false, 100.0)).contains(&&Step::Cancelled));
    assert!(!r.state().waiting);
    r.tap(true, 200.0);
    assert_eq!(r.state().low, Some(Low::Tap));
    assert!(!steps(&r.set_aut(true, 300.0)).contains(&&Step::Wait { ms: 2000.0 }));
    assert_eq!(r.state().low, Some(Low::Stop));
    assert!(matches!(
        steps(&r.transport("band", true, 400.0))[0],
        Step::Return { .. }
    ));
}

#[test]
fn the_next_stop_takes_a_hand_fader_again_from_where_it_is() {
    let mut r = rule(true, Some(true));
    r.transport("band", false, 0.0);
    r.tick(2000.0);
    r.hand(PA);
    r.live(PA, 0.7);
    r.transport("band", true, 5000.0);
    r.transport("band", false, 6000.0);
    let acts = r.tick(8000.0);
    let Step::Lower { faders, .. } = steps(&acts)[0] else {
        panic!("{acts:?}")
    };
    assert!(faders.iter().any(|m| m.label == "Rev PA" && m.from == 0.7));
}

#[test]
fn a_silent_fader_or_one_without_a_value_is_skipped() {
    let mut r = Reverb::new(&Saved::default());
    r.set_faders(vec![fader("Rev ST", 4), fader("Rev PA", 5)]);
    r.live(ST, 0.0);
    let acts = r.tap(true, 0.0);
    assert!(steps(&acts).contains(&&Step::Skipped {
        label: "Rev ST".into(),
        why: "silent".into()
    }));
    assert!(steps(&acts).contains(&&Step::Skipped {
        label: "Rev PA".into(),
        why: "no value".into()
    }));
    assert_eq!(r.state().low, None, "nothing lowered: the button stays lit");
    assert!(!acts.contains(&Act::Save));
}

#[test]
fn a_track_list_change_stops_the_instances_glides_until_its_tracks_are_back() {
    let mut r = rule(false, Some(true));
    r.tap(true, 0.0);
    assert_eq!(steps(&r.tracks_changed("band")).len(), 2);
    assert!(
        run(&mut r, 25.0, 600.0)
            .iter()
            .all(|a| !matches!(a, Act::Write { .. }))
    );
    assert_eq!(
        r.live(PA, 0.33),
        vec![],
        "another track at the index: no hand"
    );
    assert_eq!(
        r.live(PA, lowered(0.6)),
        vec![],
        "its own value again: settled"
    );
    assert!(matches!(
        steps(&r.live(PA, 0.7))[0],
        Step::Hand { by: By::Live, .. }
    ));
    assert_eq!(r.tracks_changed("master"), vec![]);
}

#[test]
fn an_offline_instance_stops_its_glides_and_keeps_what_is_held() {
    let mut r = rule(true, Some(true));
    r.transport("band", false, 0.0);
    r.tick(2000.0);
    let acts = r.offline("band");
    assert_eq!(
        steps(&acts)
            .iter()
            .filter(|s| matches!(s, Step::Stopped { .. }))
            .count(),
        2
    );
    assert!(acts.contains(&Act::Tell));
    assert_eq!(r.state().playing, None);
    assert_eq!(r.state().low, Some(Low::Stop));
    assert!(r.state().faders.iter().all(|f| f.back.is_some()));
    assert_eq!(r.next_tick(2000.0), None);
    // Back online, Live's values are its own: off where the rule left a
    // fader is the hand (a set load).
    assert!(matches!(
        steps(&r.live(PA, 0.7))[0],
        Step::Hand { by: By::Live, .. }
    ));
    // Offline during the wait: the wait ends; another Live's: nothing.
    let mut r = rule(true, Some(true));
    r.transport("band", false, 0.0);
    assert_eq!(r.offline("master"), vec![]);
    assert!(steps(&r.offline("band")).contains(&&Step::Cancelled));
    assert!(!r.state().waiting);
}

#[test]
fn a_new_layout_takes_the_held_state_by_label() {
    let mut r = rule(false, Some(true));
    r.tap(true, 0.0);
    let acts = r.set_faders(vec![fader("Rev ST", 4), fader("Rev PA", 6)]);
    assert!(steps(&acts).contains(&&Step::Stopped {
        label: "Rev PA".into(),
        why: "layout".into()
    }));
    assert_eq!(
        r.state().faders[1].key,
        "band|live_set tracks 6 mixer_device volume|value"
    );
    assert_eq!(r.state().faders[1].back, Some(0.6));
    assert_eq!(
        r.state().playing,
        Some(true),
        "the same Live: still followed"
    );
    let acts = r.set_faders(vec![fader("Rev ST", 4)]);
    assert!(
        !steps(&acts)
            .iter()
            .any(|s| matches!(s, Step::Restored { .. }))
    );
    assert_eq!(
        r.saved().low.unwrap().faders.len(),
        2,
        "Rev PA waits for its strip"
    );
    let acts = r.set_faders(vec![fader("Rev ST", 4), fader("Rev PA", 6)]);
    assert!(steps(&acts).contains(&&Step::Restored {
        faders: 1,
        waiting: 0
    }));
    assert_eq!(r.state().faders[1].back, Some(0.6));
    // Another Live's faders: its transport is not known yet.
    let master = RuleFader {
        instance: "master".into(),
        label: "Rev M".into(),
        target: "live_set tracks 1 mixer_device volume".into(),
    };
    r.set_faders(vec![master]);
    assert_eq!(r.followed(), Some("master"));
    assert_eq!(r.state().playing, None);
    // A new label in the same Live takes over nothing.
    let mut r = rule(false, Some(true));
    r.tap(true, 0.0);
    r.set_faders(vec![
        fader("Rev X", 7),
        fader("Rev ST", 4),
        fader("Rev PA", 5),
    ]);
    assert_eq!(
        r.state().faders.iter().map(|f| f.back).collect::<Vec<_>>(),
        vec![None, Some(0.85), Some(0.6)]
    );
}

#[test]
fn a_new_followed_live_ends_the_wait() {
    let mut r = rule(true, Some(true));
    r.transport("band", false, 0.0);
    let master = RuleFader {
        instance: "master".into(),
        label: "Rev M".into(),
        target: "live_set tracks 1 mixer_device volume".into(),
    };
    assert!(steps(&r.set_faders(vec![master])).contains(&&Step::Cancelled));
    assert!(!r.state().waiting);
    assert_eq!(run(&mut r, 2000.0, 2500.0), vec![]);
}

#[test]
fn a_restored_fader_found_elsewhere_is_let_go() {
    let saved = Saved {
        aut: false,
        low: Some(SavedLow {
            by: Low::Tap,
            faders: vec![
                SavedFader {
                    instance: "band".into(),
                    label: "Rev ST".into(),
                    back: 0.85,
                    left: Some(lowered(0.85)),
                },
                SavedFader {
                    instance: "band".into(),
                    label: "Rev PA".into(),
                    back: 0.6,
                    left: Some(lowered(0.6)),
                },
            ],
        }),
    };
    let mut r = Reverb::new(&saved);
    assert_eq!(r.saved(), saved, "nothing named yet: kept as it was");
    // A layout naming neither: nothing restored, nothing low.
    let acts = r.set_faders(vec![fader("Rev X", 7)]);
    assert!(
        !steps(&acts)
            .iter()
            .any(|s| matches!(s, Step::Restored { .. }))
    );
    assert_eq!(r.state().low, None);
    assert_eq!(r.saved(), saved);
    let acts = r.set_faders(vec![fader("Rev ST", 4), fader("Rev PA", 5)]);
    assert!(steps(&acts).contains(&&Step::Restored {
        faders: 2,
        waiting: 0
    }));
    assert_eq!(r.state().low, Some(Low::Tap));
    assert_eq!(r.live(ST, lowered(0.85)), vec![]);
    assert!(
        matches!(steps(&r.live(PA, 0.7))[0], Step::Hand { by: By::Live, .. }),
        "a set loaded meanwhile"
    );
    let acts = r.tap(false, 0.0);
    let Step::Return { faders } = steps(&acts)[1] else {
        panic!("{acts:?}")
    };
    assert_eq!(faders.len(), 1);
    assert_eq!(r.saved().low, None, "all back: nothing held");
}

#[test]
fn a_return_without_lives_value_glides_from_where_the_rule_left_it() {
    let saved = Saved {
        aut: true,
        low: Some(SavedLow {
            by: Low::Stop,
            faders: vec![SavedFader {
                instance: "band".into(),
                label: "Rev ST".into(),
                back: 0.85,
                left: Some(lowered(0.85)),
            }],
        }),
    };
    let mut r = Reverb::new(&saved);
    r.set_faders(vec![fader("Rev ST", 4)]);
    let acts = r.transport("band", true, 0.0);
    assert_eq!(
        steps(&acts)[0],
        &Step::Return {
            faders: vec![Move {
                label: "Rev ST".into(),
                from: lowered(0.85),
                to: 0.85,
                ms: RETURN_MS
            }]
        }
    );
    assert!(writes(&run(&mut r, 25.0, 500.0)).contains(&(ST, 0.85, true)));
}

#[test]
fn next_tick_is_the_wait_or_the_next_step() {
    let mut r = rule(true, Some(true));
    assert_eq!(r.next_tick(0.0), None);
    r.transport("band", false, 100.0);
    assert_eq!(r.next_tick(150.0), Some(2100.0));
    r.tap(true, 200.0);
    assert_eq!(r.next_tick(200.0), Some(225.0));
    // A glide and a wait: the sooner.
    let mut r = rule(true, Some(true));
    r.tap(true, 0.0);
    r.transport("band", false, 100.0);
    assert_eq!(r.next_tick(100.0), Some(125.0));
    assert!(r.state().waiting);
    run(&mut r, 125.0, 500.0);
    assert_eq!(r.next_tick(500.0), Some(2100.0));
}

#[test]
fn each_step_has_its_line_and_warns_where_it_should() {
    let m = |label: &str| Move {
        label: label.into(),
        from: 0.85,
        to: 0.6,
        ms: 2000.0,
    };
    assert_eq!(
        step_line(&Step::Lower {
            why: Low::Stop,
            faders: vec![m("Rev ST"), m("Rev PA")]
        }),
        "REVERB lowers Rev ST, Rev PA by 10 dB (stop)"
    );
    assert_eq!(
        step_line(&Step::Lower {
            why: Low::Tap,
            faders: vec![m("Rev ST")]
        }),
        "REVERB lowers Rev ST by 10 dB (tap)"
    );
    assert_eq!(
        step_line(&Step::Return {
            faders: vec![m("Rev ST")]
        }),
        "REVERB returns Rev ST to the song level"
    );
    assert_eq!(
        step_line(&Step::Hand {
            label: "Rev PA".into(),
            by: By::Page,
            value: None
        }),
        "REVERB lets go of Rev PA: moved on a page"
    );
    assert_eq!(
        step_line(&Step::Hand {
            label: "Rev PA".into(),
            by: By::Live,
            value: Some(0.7)
        }),
        "REVERB lets go of Rev PA: Live has it at 0.7"
    );
    assert_eq!(
        step_line(&Step::Wait { ms: 2000.0 }),
        "Live stopped: REVERB AUT lowers the reverb faders in 2000 ms"
    );
    assert_eq!(
        step_line(&Step::Off),
        "REVERB AUT off while the reverb faders are low: they stay, a tap returns them"
    );
    assert_eq!(step_line(&Step::Aut { on: true }), "REVERB AUT turned on");
    assert_eq!(step_line(&Step::Aut { on: false }), "REVERB AUT turned off");
    assert_eq!(step_line(&Step::Tap { low: false }), "REVERB tapped: back");
    assert_eq!(step_line(&Step::Tap { low: true }), "REVERB tapped: down");
    assert_eq!(
        step_line(&Step::Cancelled),
        "REVERB AUT's wait ended with nothing lowered"
    );
    assert_eq!(
        step_line(&Step::Arrived {
            label: "Rev ST".into(),
            value: 0.5
        }),
        "REVERB: Rev ST arrived at 0.5"
    );
    assert_eq!(
        step_line(&Step::Stopped {
            label: "Rev ST".into(),
            why: "tracks".into()
        }),
        "REVERB stopped the glide of Rev ST: tracks"
    );
    assert_eq!(
        step_line(&Step::Skipped {
            label: "Rev ST".into(),
            why: "silent".into()
        }),
        "REVERB skips Rev ST: silent"
    );
    assert_eq!(
        step_line(&Step::Restored {
            faders: 1,
            waiting: 1
        }),
        "REVERB holds 1 restored fader(s) low; 1 wait for their strip"
    );
    assert!(step_warn(&Step::Stopped {
        label: "x".into(),
        why: "tracks".into()
    }));
    assert!(step_warn(&Step::Skipped {
        label: "x".into(),
        why: "silent".into()
    }));
    assert!(step_warn(&Step::Restored {
        faders: 1,
        waiting: 1
    }));
    assert!(!step_warn(&Step::Restored {
        faders: 2,
        waiting: 0
    }));
    assert!(!step_warn(&Step::Off));
    assert_eq!(
        serde_json::to_value(Step::Hand {
            label: "Rev PA".into(),
            by: By::Page,
            value: None
        })
        .unwrap(),
        serde_json::json!({"step": "hand", "label": "Rev PA", "by": "page", "value": null})
    );
    assert_eq!(
        serde_json::to_value(Step::Lower {
            why: Low::Stop,
            faders: vec![m("Rev ST")]
        })
        .unwrap(),
        serde_json::json!({"step": "lower", "why": "stop",
            "faders": [{"label": "Rev ST", "from": 0.85, "to": 0.6, "ms": 2000.0}]})
    );
}
