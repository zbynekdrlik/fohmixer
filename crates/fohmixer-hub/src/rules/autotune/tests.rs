use super::*;
use serde_json::json;

fn sets(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

/// The rule following `band`, its transport unheard.
fn band(on: bool) -> AutotuneAut {
    let mut rule = AutotuneAut::new(on);
    assert_eq!(rule.follow(&sets(&["band"]), |_| None), vec![]);
    rule
}

fn write(set: &str, on: bool) -> (String, bool) {
    (set.to_string(), on)
}

#[test]
fn a_target_is_on_only_within_half_a_step_of_its_on_value() {
    assert_eq!(SAME_VALUE, 0.5);
    assert_eq!(target_state(Some(127.0), 127), TargetState::On);
    assert_eq!(target_state(Some(126.6), 127), TargetState::On);
    assert_eq!(target_state(Some(127.4), 127), TargetState::On);
    assert_eq!(target_state(Some(126.5), 127), TargetState::Off);
    assert_eq!(target_state(Some(126.4), 127), TargetState::Off);
    assert_eq!(target_state(Some(0.0), 127), TargetState::Off);
    assert_eq!(target_state(Some(0.0), 0), TargetState::On);
    assert_eq!(target_state(Some(0.4), 0), TargetState::On);
    assert_eq!(target_state(Some(0.5), 0), TargetState::Off);
    // Today's repro "off", set by hand: off, never a third state.
    assert_eq!(target_state(Some(11.0), 0), TargetState::Off);
    assert_eq!(target_state(Some(127.0), 0), TargetState::Off);
    assert_eq!(target_state(None, 127), TargetState::Waiting);
    assert_eq!(target_state(None, 0), TargetState::Waiting);
}

#[test]
fn it_turns_autotune_on_while_playing_and_off_when_stopped_once_per_change() {
    let mut rule = band(true);
    assert!(rule.is_on());
    assert_eq!(
        rule.on_playing("band", false),
        Some(false),
        "the first value"
    );
    assert_eq!(rule.on_playing("band", false), None, "no change, no write");
    assert_eq!(rule.on_playing("band", true), Some(true));
    assert_eq!(rule.on_playing("band", true), None);
    assert_eq!(rule.writes(), 2);
    assert_eq!(rule.playing("band"), Some(true));
    assert_eq!(rule.wanted("band"), Some(true));
    // A set no layout names is not followed.
    assert_eq!(rule.on_playing("master", true), None);
    assert_eq!(rule.playing("master"), None);
    assert_eq!(rule.writes(), 2);
}

#[test]
fn with_aut_off_it_never_writes_and_a_tap_still_sets_every_set() {
    let mut rule = band(false);
    assert!(!rule.is_on());
    assert_eq!(rule.on_playing("band", true), None);
    assert_eq!(rule.on_playing("band", false), None);
    assert_eq!(rule.wanted("band"), None);
    assert_eq!(rule.tap(true), vec![write("band", true)]);
    assert_eq!(rule.wanted("band"), Some(true));
    assert_eq!(rule.writes(), 0);
    assert_eq!(rule.taps(), 1);
    // Every followed set, each tap.
    let mut two = AutotuneAut::new(false);
    two.follow(&sets(&["band", "master"]), |_| None);
    assert_eq!(
        two.tap(false),
        vec![write("band", false), write("master", false)]
    );
    assert_eq!(two.tap(true).len(), 2);
    assert_eq!(two.taps(), 2);
    assert_eq!(two.wanted("master"), Some(true));
}

#[test]
fn turning_aut_on_applies_each_sets_transport_at_once() {
    let mut rule = AutotuneAut::new(false);
    rule.follow(&sets(&["band", "master"]), |_| None);
    assert_eq!(rule.on_playing("band", true), None);
    assert_eq!(rule.set_flag(true), vec![write("band", true)]);
    assert_eq!(rule.set_flag(true), vec![], "already on");
    assert_eq!(rule.writes(), 1);
    // Off writes nothing, and the transport is not followed meanwhile.
    assert_eq!(rule.set_flag(false), vec![]);
    assert_eq!(rule.on_playing("band", false), None);
    assert_eq!(rule.on_playing("band", true), None);
    // On again: applied at once, even with the value it wrote before.
    assert_eq!(rule.set_flag(true), vec![write("band", true)]);
    assert_eq!(rule.writes(), 2);
    // The unheard set gets its write with its first value.
    assert_eq!(rule.on_playing("master", false), Some(false));
}

#[test]
fn a_tap_overrides_until_the_next_transport_change() {
    let mut rule = band(true);
    assert_eq!(rule.on_playing("band", true), Some(true));
    assert_eq!(rule.tap(false), vec![write("band", false)]);
    assert_eq!(rule.wanted("band"), Some(false), "a target found now");
    // The same transport again: the tap holds.
    assert_eq!(rule.on_playing("band", true), None);
    // A song's CC36 turned the stream racks on meanwhile: the stop still
    // writes off (the rule wrote on last, whatever the tap did).
    assert_eq!(rule.on_playing("band", false), Some(false));
    assert_eq!(rule.on_playing("band", true), Some(true));
    assert_eq!(rule.writes(), 3);
    assert_eq!(rule.taps(), 1);
    // A tap on while stopped: the start writes on again, harmless.
    assert_eq!(rule.on_playing("band", false), Some(false));
    rule.tap(true);
    assert_eq!(rule.on_playing("band", true), Some(true));
}

#[test]
fn a_reconnect_forgets_the_set_and_its_first_value_writes_once() {
    let mut rule = band(true);
    assert_eq!(rule.on_playing("band", true), Some(true));
    rule.on_disconnect("band");
    assert_eq!(rule.wanted("band"), None, "a target found now gets nothing");
    assert_eq!(rule.playing("band"), None);
    assert_eq!(rule.set_flag(true), vec![], "no transport while away");
    assert_eq!(
        rule.on_playing("band", true),
        Some(true),
        "one write after the reconnect"
    );
    assert_eq!(rule.on_playing("band", true), None);
    assert_eq!(rule.wanted("band"), Some(true));
    assert_eq!(rule.writes(), 2);
    // A tap's write is forgotten too.
    rule.tap(false);
    rule.on_disconnect("band");
    assert_eq!(rule.wanted("band"), None);
    // Another set's disconnect leaves band alone; an unknown one is nothing.
    assert_eq!(rule.on_playing("band", false), Some(false));
    rule.on_disconnect("master");
    assert_eq!(rule.wanted("band"), Some(false));
}

#[test]
fn a_lost_transport_forgets_playing_but_not_what_was_written() {
    let mut rule = band(true);
    assert_eq!(rule.on_playing("band", true), Some(true));
    rule.transport_lost("band");
    assert_eq!(rule.playing("band"), None);
    assert_eq!(rule.wanted("band"), Some(true));
    assert_eq!(
        rule.on_playing("band", true),
        None,
        "as written: no second write"
    );
    assert_eq!(rule.on_playing("band", false), Some(false));
    rule.transport_lost("master");
    assert_eq!(rule.playing("band"), Some(false));
}

#[test]
fn a_new_layout_follows_new_sets_with_their_transport_and_drops_gone_ones() {
    let mut rule = band(true);
    assert_eq!(rule.on_playing("band", false), Some(false));
    assert_eq!(
        rule.follow(&sets(&["band", "master"]), |s| (s == "master")
            .then_some(true)),
        vec![write("master", true)]
    );
    assert_eq!(rule.playing("master"), Some(true));
    // Band kept its state: no second write for the same value.
    assert_eq!(rule.on_playing("band", false), None);
    assert_eq!(rule.writes(), 2);
    assert_eq!(rule.follow(&sets(&["band"]), |_| Some(true)), vec![]);
    assert_eq!(rule.playing("master"), None, "dropped");
    assert_eq!(rule.wanted("master"), None);
    assert_eq!(rule.on_playing("master", false), None);
    assert_eq!(rule.playing("band"), Some(false), "kept, not read again");
    assert_eq!(rule.wanted("band"), Some(false));
    // With AUT off a new set writes nothing.
    let mut off = AutotuneAut::new(false);
    assert_eq!(off.follow(&sets(&["band"]), |_| Some(true)), vec![]);
    assert_eq!(off.playing("band"), Some(true));
    assert_eq!(off.follow(&sets(&[]), |_| None), vec![]);
    assert_eq!(off.playing("band"), None);
}

#[test]
fn every_step_has_its_line_and_only_failures_warn() {
    let write_step = |why: Why| Step::Write {
        instance: "band".into(),
        on: true,
        why,
        targets: vec!["#8".into(), "Vox 2".into()],
    };
    let flag = Step::Flag { on: true };
    let tap = Step::Tap { on: false };
    let failed = Step::Failed {
        instance: "band".into(),
        target: "live_set tracks 7".into(),
        problem: "stale ref".into(),
    };
    let found = Step::Found {
        instance: "band".into(),
        target: "live_set tracks 6".into(),
        path: "live_set tracks 6 devices 0 parameters 1".into(),
    };
    let missing = Step::Missing {
        instance: "band".into(),
        target: "live_set tracks 8".into(),
        problem: "no rack named Latencies".into(),
    };
    assert_eq!(step_line(&flag), "AUTOTUNE AUT turned on");
    assert_eq!(
        step_line(&Step::Flag { on: false }),
        "AUTOTUNE AUT turned off"
    );
    assert_eq!(step_line(&tap), "AUTOTUNE tapped: off");
    assert_eq!(step_line(&Step::Tap { on: true }), "AUTOTUNE tapped: on");
    assert_eq!(
        step_line(&write_step(Why::Transport)),
        "AUTOTUNE AUT writes band on: 2 tracks (transport)"
    );
    assert_eq!(
        step_line(&write_step(Why::Flag)),
        "AUTOTUNE AUT writes band on: 2 tracks (flag)"
    );
    assert_eq!(
        step_line(&write_step(Why::Tap)),
        "AUTOTUNE AUT writes band on: 2 tracks (tap)"
    );
    assert_eq!(
        step_line(&Step::Write {
            instance: "master".into(),
            on: false,
            why: Why::Found,
            targets: vec!["#8".into()],
        }),
        "AUTOTUNE AUT writes master off: 1 tracks (found)"
    );
    assert_eq!(
        step_line(&failed),
        "AUTOTUNE could not write band live_set tracks 7: stale ref"
    );
    assert_eq!(
        step_line(&found),
        "AUTOTUNE found band live_set tracks 6's rack: live_set tracks 6 devices 0 parameters 1"
    );
    assert_eq!(
        step_line(&missing),
        "AUTOTUNE has no rack for band live_set tracks 8: no rack named Latencies"
    );
    let tap_write = write_step(Why::Tap);
    for step in [&flag, &tap, &tap_write, &found] {
        assert!(!step_warn(step), "{step:?}");
    }
    assert!(step_warn(&failed));
    assert!(step_warn(&missing));
    assert_eq!(
        serde_json::to_value(Step::Write {
            instance: "band".into(),
            on: true,
            why: Why::Transport,
            targets: vec!["#8".into()],
        })
        .unwrap(),
        json!({"step": "write", "instance": "band", "on": true, "why": "transport", "targets": ["#8"]})
    );
    assert_eq!(
        serde_json::to_value(&flag).unwrap(),
        json!({"step": "flag", "on": true})
    );
    assert_eq!(
        serde_json::to_value(&missing).unwrap(),
        json!({"step": "missing", "instance": "band", "target": "live_set tracks 8",
               "problem": "no rack named Latencies"})
    );
    assert_eq!(
        serde_json::to_value([Why::Flag, Why::Tap, Why::Found]).unwrap(),
        json!(["flag", "tap", "found"])
    );
}
