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
    let held = Step::Held {
        instance: "band".into(),
        on: true,
        why: "offline".into(),
    };
    let released = Step::Held {
        instance: "band".into(),
        on: false,
        why: "layout".into(),
    };
    assert_eq!(
        step_line(&held),
        "AUTOTUNE writes nothing to band until its markers are settled (offline)"
    );
    assert_eq!(
        step_line(&released),
        "AUTOTUNE writes band again: its markers are settled (layout)"
    );
    assert_eq!(
        serde_json::to_value(&held).unwrap(),
        json!({"step": "held", "instance": "band", "on": true, "why": "offline"})
    );
    let tap_write = write_step(Why::Tap);
    for step in [&flag, &tap, &tap_write, &found, &held, &released] {
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

#[test]
fn the_key_steps_are_recorded_and_logged() {
    let song = Step::Key {
        instance: "band".into(),
        song: Some(31),
        key: Some("G".into()),
        why: keys::KeyWhy::Change,
    };
    let none = Step::Key {
        instance: "band".into(),
        song: None,
        key: None,
        why: keys::KeyWhy::Span,
    };
    let keyless = Step::Key {
        instance: "band".into(),
        song: Some(88),
        key: None,
        why: keys::KeyWhy::NoKey,
    };
    let device = Step::KeyDevice {
        instance: "band".into(),
        target: "live_set tracks 7".into(),
        device: keys::KeyDevice::Gridder,
        path: "live_set tracks 7 devices 0 chains 0 devices 0 chains 0 devices 0".into(),
    };
    let missing = Step::KeyMissing {
        instance: "band".into(),
        target: "live_set tracks 6".into(),
        problem: "chain 0 holds 2 plug-ins, not one".into(),
    };
    let refused = Step::Refused {
        what: "aut".into(),
        why: REFUSED_WHY.into(),
    };
    assert_eq!(REFUSED_WHY, "keys only until #140 PR 3");
    assert_eq!(
        step_line(&song),
        "AUTOTUNE key in force in band: song 31 G (change)"
    );
    assert_eq!(
        step_line(&none),
        "AUTOTUNE key in force in band: no song (span)"
    );
    assert_eq!(
        step_line(&keyless),
        "AUTOTUNE key in force in band: song 88, no key (no_key)"
    );
    assert_eq!(
        step_line(&Step::Key {
            instance: "master".into(),
            song: Some(2),
            key: Some("F#m".into()),
            why: keys::KeyWhy::Song,
        }),
        "AUTOTUNE key in force in master: song 2 F#m (song)"
    );
    assert_eq!(
        step_line(&device),
        "AUTOTUNE found band live_set tracks 7's key device (gridder): \
         live_set tracks 7 devices 0 chains 0 devices 0 chains 0 devices 0"
    );
    assert_eq!(
        step_line(&Step::KeyDevice {
            instance: "band".into(),
            target: "live_set tracks 1".into(),
            device: keys::KeyDevice::AutoTune,
            path: "p".into(),
        }),
        "AUTOTUNE found band live_set tracks 1's key device (auto_tune): p"
    );
    assert_eq!(
        step_line(&missing),
        "AUTOTUNE has no key device for band live_set tracks 6: chain 0 holds 2 plug-ins, not one"
    );
    assert_eq!(
        step_line(&refused),
        "AUTOTUNE refuses a page's aut: keys only until #140 PR 3"
    );
    assert_eq!(
        step_line(&Step::Refused {
            what: "tap".into(),
            why: "w".into(),
        }),
        "AUTOTUNE refuses a page's tap: w"
    );
    for step in [&song, &none, &keyless, &device, &refused] {
        assert!(!step_warn(step), "{step:?}");
    }
    assert!(step_warn(&missing));
    assert_eq!(
        serde_json::to_value(&song).unwrap(),
        json!({"step": "key", "instance": "band", "song": 31, "key": "G", "why": "change"})
    );
    assert_eq!(
        serde_json::to_value(&none).unwrap(),
        json!({"step": "key", "instance": "band", "song": null, "key": null, "why": "span"})
    );
    assert_eq!(
        serde_json::to_value(&keyless).unwrap(),
        json!({"step": "key", "instance": "band", "song": 88, "key": null, "why": "no_key"})
    );
    assert_eq!(
        serde_json::to_value(&device).unwrap(),
        json!({"step": "key_device", "instance": "band", "target": "live_set tracks 7",
               "device": "gridder",
               "path": "live_set tracks 7 devices 0 chains 0 devices 0 chains 0 devices 0"})
    );
    assert_eq!(
        serde_json::to_value(&missing).unwrap(),
        json!({"step": "key_missing", "instance": "band", "target": "live_set tracks 6",
               "problem": "chain 0 holds 2 plug-ins, not one"})
    );
    assert_eq!(
        serde_json::to_value(&refused).unwrap(),
        json!({"step": "refused", "what": "aut", "why": "keys only until #140 PR 3"})
    );
}

#[test]
fn the_write_steps_are_recorded_and_logged() {
    // #140 PR 2b: the key written, its read-back, another writer, the
    // lateness and Live's bar.
    let write = Step::KeyWrite {
        instance: "band".into(),
        key: "F#m".into(),
        why: keys::KeyWhy::Song,
        targets: vec!["#1".into(), "Vox 2".into()],
        bar: true,
    };
    let retry = Step::KeyWrite {
        instance: "band".into(),
        key: "G".into(),
        why: keys::KeyWhy::Retry,
        targets: vec!["#2".into()],
        bar: false,
    };
    let confirmed = Step::Readback {
        instance: "band".into(),
        target: "live_set tracks 0".into(),
        key: "F#m".into(),
        state: keys::KeyState::Confirmed,
        shows: Some("F#m".into()),
        problem: None,
    };
    let refused = Step::Readback {
        instance: "band".into(),
        target: "live_set tracks 1".into(),
        key: "F#m".into(),
        state: keys::KeyState::Refused,
        shows: None,
        problem: Some("live error: Value cannot be set, the parameter is disabled".into()),
    };
    let mismatch = Step::Readback {
        instance: "band".into(),
        target: "live_set tracks 2".into(),
        key: "F#m".into(),
        state: keys::KeyState::Mismatch,
        shows: Some("C".into()),
        problem: None,
    };
    let another = Step::AnotherWriter {
        instance: "band".into(),
        target: "live_set tracks 0".into(),
        shows: Some("A".into()),
    };
    let on_time = Step::Late {
        instance: "band".into(),
        song: 1,
        ms: 100.0,
    };
    let late = Step::Late {
        instance: "band".into(),
        song: 1,
        ms: 100.5,
    };
    let early = Step::Late {
        instance: "band".into(),
        song: 31,
        ms: -23.0,
    };
    let bar = Step::Bar {
        instance: "band".into(),
        key: "F#m".into(),
        state: keys::BarState::Confirmed,
        shows: Some("F#m".into()),
    };
    let bar_off = Step::Bar {
        instance: "band".into(),
        key: "F#m".into(),
        state: keys::BarState::Shows,
        shows: None,
    };
    assert_eq!(
        step_line(&write),
        "AUTOTUNE writes F#m to band: 2 key devices and Live's bar (song)"
    );
    assert_eq!(
        step_line(&retry),
        "AUTOTUNE writes G to band: 1 key device (retry)"
    );
    assert_eq!(
        step_line(&confirmed),
        "AUTOTUNE read back band live_set tracks 0: confirmed for F#m, it shows F#m"
    );
    assert_eq!(
        step_line(&refused),
        "AUTOTUNE band live_set tracks 1: refused for F#m, nothing read back: \
         live error: Value cannot be set, the parameter is disabled"
    );
    assert_eq!(
        step_line(&mismatch),
        "AUTOTUNE read back band live_set tracks 2: mismatch for F#m, it shows C"
    );
    assert_eq!(
        step_line(&another),
        "AUTOTUNE sees another writer on band live_set tracks 0: it shows A, not written against"
    );
    assert_eq!(
        step_line(&Step::AnotherWriter {
            instance: "band".into(),
            target: "t".into(),
            shows: None,
        }),
        "AUTOTUNE sees another writer on band t: it shows no key, not written against"
    );
    assert_eq!(
        step_line(&late),
        "AUTOTUNE wrote band song 1's key change 100.5 ms after its start"
    );
    assert_eq!(
        step_line(&early),
        "AUTOTUNE wrote band song 31's key change 23.0 ms before its start"
    );
    assert_eq!(
        step_line(&on_time),
        "AUTOTUNE wrote band song 1's key change 100.0 ms after its start"
    );
    assert_eq!(
        step_line(&Step::Late {
            instance: "band".into(),
            song: 2,
            ms: 0.0,
        }),
        "AUTOTUNE wrote band song 2's key change 0.0 ms after its start"
    );
    assert_eq!(
        step_line(&Step::KeyWrite {
            instance: "band".into(),
            key: "C".into(),
            why: keys::KeyWhy::Found,
            targets: Vec::new(),
            bar: true,
        }),
        "AUTOTUNE writes C to band: 0 key devices and Live's bar (found)"
    );
    assert_eq!(
        step_line(&bar),
        "AUTOTUNE read back band's Live bar: confirmed for F#m, it shows F#m"
    );
    assert_eq!(
        step_line(&bar_off),
        "AUTOTUNE read back band's Live bar: another key for F#m, it shows no key"
    );
    for step in [&write, &retry, &confirmed, &on_time, &early, &bar, &bar_off] {
        assert!(!step_warn(step), "{step:?}");
    }
    for step in [&refused, &mismatch, &another, &late] {
        assert!(step_warn(step), "{step:?}");
    }
    for state in [
        keys::KeyState::NoKey,
        keys::KeyState::Writing,
        keys::KeyState::AnotherWriter,
    ] {
        let step = Step::Readback {
            instance: "band".into(),
            target: "t".into(),
            key: "C".into(),
            state,
            shows: None,
            problem: None,
        };
        assert!(step_warn(&step), "{state:?}");
    }
    assert_eq!(
        serde_json::to_value(&write).unwrap(),
        json!({"step": "key_write", "instance": "band", "key": "F#m", "why": "song",
               "targets": ["#1", "Vox 2"], "bar": true})
    );
    assert_eq!(
        serde_json::to_value(&refused).unwrap(),
        json!({"step": "readback", "instance": "band", "target": "live_set tracks 1",
               "key": "F#m", "state": "refused", "shows": null,
               "problem": "live error: Value cannot be set, the parameter is disabled"})
    );
    assert_eq!(
        serde_json::to_value(&another).unwrap(),
        json!({"step": "another_writer", "instance": "band", "target": "live_set tracks 0",
               "shows": "A"})
    );
    assert_eq!(
        serde_json::to_value(&early).unwrap(),
        json!({"step": "late", "instance": "band", "song": 31, "ms": -23.0})
    );
    assert_eq!(
        serde_json::to_value(&bar).unwrap(),
        json!({"step": "bar", "instance": "band", "key": "F#m", "state": "confirmed",
               "shows": "F#m"})
    );
}

#[test]
fn a_registration_of_the_dry_writes_is_recorded_and_warns_when_it_failed() {
    // #140 PR 3a: the script makes them when the hub is gone.
    let kept = Step::Safe {
        instance: "band".into(),
        targets: 12,
        problem: None,
    };
    let failed = Step::Safe {
        instance: "band".into(),
        targets: 2,
        problem: Some("refused: no such function: safe_writes".into()),
    };
    assert_eq!(
        step_line(&kept),
        "AUTOTUNE gave band's script the DRY writes of 12 selectors, for when the hub is gone"
    );
    assert_eq!(
        step_line(&failed),
        "AUTOTUNE could not give band's script the DRY writes of 2 selectors: \
         refused: no such function: safe_writes"
    );
    assert!(!step_warn(&kept));
    assert!(step_warn(&failed));
    assert_eq!(
        serde_json::to_value(&kept).unwrap(),
        json!({"step": "safe", "instance": "band", "targets": 12, "problem": null})
    );
    assert_eq!(
        serde_json::to_value(&failed).unwrap(),
        json!({"step": "safe", "instance": "band", "targets": 2,
               "problem": "refused: no such function: safe_writes"})
    );
}

#[test]
fn a_target_given_up_is_recorded_and_warns() {
    // #140: a walk that failed CONNECT_RETRIES + 1 times in a row.
    let gave_up = Step::GaveUp {
        instance: "band".into(),
        target: "live_set tracks 4".into(),
        problem: "no result within 3 s".into(),
        tries: 25,
    };
    assert_eq!(
        step_line(&gave_up),
        "AUTOTUNE gave up walking band live_set tracks 4 after 25 failed walks, \
         until the next trigger: no result within 3 s"
    );
    assert!(step_warn(&gave_up));
    assert_eq!(
        serde_json::to_value(&gave_up).unwrap(),
        json!({"step": "gave_up", "instance": "band", "target": "live_set tracks 4",
               "problem": "no result within 3 s", "tries": 25})
    );
}
