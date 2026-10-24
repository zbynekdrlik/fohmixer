use super::*;
use serde_json::json;

fn sets(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
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
fn the_rule_follows_each_sets_transport_and_the_flag() {
    let mut rule = AutotuneAut::new(false, Vec::new());
    assert!(!rule.is_on());
    rule.follow(&sets(&["band", "master"]), |s| {
        (s == "master").then_some(true)
    });
    assert_eq!(rule.playing("band"), None, "not heard yet");
    assert_eq!(rule.playing("master"), Some(true), "as last heard");
    rule.on_playing("band", true);
    assert_eq!(rule.playing("band"), Some(true));
    rule.on_playing("band", false);
    assert_eq!(rule.playing("band"), Some(false));
    // A set no layout names is not followed.
    rule.on_playing("other", true);
    assert_eq!(rule.playing("other"), None);
    // A new layout keeps a followed set as it is and drops a gone one.
    rule.follow(&sets(&["band"]), |_| Some(true));
    assert_eq!(rule.playing("band"), Some(false), "kept, not read again");
    assert_eq!(rule.playing("master"), None, "dropped");
    rule.on_playing("master", true);
    assert_eq!(rule.playing("master"), None);
    // A lost listener or a disconnect forgets the transport only.
    rule.transport_lost("band");
    assert_eq!(rule.playing("band"), None);
    rule.on_playing("band", true);
    rule.on_disconnect("band");
    assert_eq!(rule.playing("band"), None);
    rule.on_playing("band", true);
    rule.on_disconnect("master");
    rule.transport_lost("master");
    assert_eq!(rule.playing("band"), Some(true));
    // The flag, and the selector batches counted.
    rule.set_flag(true);
    assert!(rule.is_on());
    rule.set_flag(false);
    assert!(!rule.is_on());
    assert_eq!(rule.writes(), 0);
    rule.wrote();
    rule.wrote();
    assert_eq!(rule.writes(), 2);
    assert!(AutotuneAut::new(true, Vec::new()).is_on(), "AUT kept on");
}

#[test]
fn a_singer_off_is_kept_and_saved_sorted() {
    // #140 PR 3b, spec §2.2: per singer, kept across a restart.
    let mut rule = AutotuneAut::new(true, vec!["VOX2".into()]);
    assert!(rule.is_off("VOX2"));
    assert!(!rule.is_off("VOX1"));
    assert!(rule.set_singer("VOX1", true), "a change");
    assert!(!rule.set_singer("VOX1", true), "already off");
    assert!(rule.set_singer("#Vox 7", true));
    assert_eq!(rule.singers_off(), ["#Vox 7", "VOX1", "VOX2"]);
    assert!(rule.set_singer("VOX2", false));
    assert!(!rule.set_singer("VOX2", false), "already on");
    assert!(!rule.set_singer("VOX9", false));
    assert!(!rule.is_off("VOX2"));
    assert_eq!(rule.singers_off(), ["#Vox 7", "VOX1"]);
    assert!(rule.is_on(), "the flag stays");
}

#[test]
fn every_step_has_its_line_and_only_failures_warn() {
    let write_step = |cause: Cause| Step::Write {
        instance: "band".into(),
        cause,
        on: vec!["#8".into(), "Vox 2".into()],
        off: vec!["#9".into()],
    };
    let flag = Step::Flag { on: true };
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
    assert_eq!(
        step_line(&write_step(Cause::Transport)),
        "AUTOTUNE writes band's selectors: 2 on, 1 to DRY (transport)"
    );
    assert_eq!(
        step_line(&Step::Write {
            instance: "master".into(),
            cause: Cause::Hold,
            on: Vec::new(),
            off: vec!["#8".into(), "#9".into(), "#10".into()],
        }),
        "AUTOTUNE writes master's selectors: 0 on, 3 to DRY (hold)"
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
    let write = write_step(Cause::Found);
    for step in [&flag, &write, &found, &held, &released] {
        assert!(!step_warn(step), "{step:?}");
    }
    assert!(step_warn(&failed));
    assert!(step_warn(&missing));
    assert_eq!(
        serde_json::to_value(Step::Write {
            instance: "band".into(),
            cause: Cause::Transport,
            on: vec!["#8".into()],
            off: Vec::new(),
        })
        .unwrap(),
        json!({"step": "write", "instance": "band", "cause": "transport", "on": ["#8"],
               "off": []})
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
}

#[test]
fn the_switch_steps_are_recorded_and_logged() {
    // #140 PR 3b: a decision switched, another writer on a selector, a
    // singer off by hand.
    let on = Step::Select {
        instance: "band".into(),
        target: "live_set tracks 7".into(),
        on: true,
        why: SwitchWhy::Play,
        cause: Cause::Key,
    };
    let dry = Step::Select {
        instance: "band".into(),
        target: "live_set tracks 7".into(),
        on: false,
        why: SwitchWhy::Mismatch,
        cause: Cause::Key,
    };
    let writer = |act| Step::SelectorWriter {
        instance: "band".into(),
        target: "live_set tracks 6".into(),
        act,
    };
    let singer = Step::Singer {
        id: "VOX1".into(),
        off: true,
    };
    assert_eq!(
        step_line(&on),
        "AUTOTUNE switches band live_set tracks 7 on (play, key)"
    );
    assert_eq!(
        step_line(&dry),
        "AUTOTUNE switches band live_set tracks 7 to DRY (mismatch, key)"
    );
    assert_eq!(
        step_line(&writer(WriterAct::DryAgain)),
        "AUTOTUNE sees another writer on band live_set tracks 6's selector: DRY again"
    );
    assert_eq!(
        step_line(&writer(WriterAct::GaveUp)),
        "AUTOTUNE sees another writer on band live_set tracks 6's selector: \
         the hub stops writing it until the next song start"
    );
    assert_eq!(
        step_line(&writer(WriterAct::LeftDry)),
        "AUTOTUNE sees another writer on band live_set tracks 6's selector: left on DRY"
    );
    assert_eq!(step_line(&singer), "AUTOTUNE singer VOX1 off by hand");
    assert_eq!(
        step_line(&Step::Singer {
            id: "#Vox 7".into(),
            off: false,
        }),
        "AUTOTUNE singer #Vox 7 back on the rule"
    );
    for step in [&on, &dry, &singer] {
        assert!(!step_warn(step), "{step:?}");
    }
    for act in [WriterAct::DryAgain, WriterAct::GaveUp, WriterAct::LeftDry] {
        assert!(step_warn(&writer(act)), "{act:?}");
    }
    assert_eq!(
        serde_json::to_value(&dry).unwrap(),
        json!({"step": "select", "instance": "band", "target": "live_set tracks 7",
               "on": false, "why": "mismatch", "cause": "key"})
    );
    assert_eq!(
        serde_json::to_value(writer(WriterAct::GaveUp)).unwrap(),
        json!({"step": "selector_writer", "instance": "band", "target": "live_set tracks 6",
               "act": "gave_up"})
    );
    assert_eq!(
        serde_json::to_value([WriterAct::DryAgain, WriterAct::LeftDry]).unwrap(),
        json!(["dry_again", "left_dry"])
    );
    assert_eq!(
        serde_json::to_value(&singer).unwrap(),
        json!({"step": "singer", "id": "VOX1", "off": true})
    );
    let causes = [
        Cause::Transport,
        Cause::Position,
        Cause::Key,
        Cause::Flag,
        Cause::Singer,
        Cause::Found,
        Cause::Guard,
        Cause::Release,
        Cause::Hold,
        Cause::Safe,
        Cause::Layout,
    ];
    let words = [
        "transport",
        "position",
        "key",
        "flag",
        "singer",
        "found",
        "guard",
        "release",
        "hold",
        "safe",
        "layout",
    ];
    assert_eq!(causes.map(Cause::word), words);
    assert_eq!(serde_json::to_value(causes).unwrap(), json!(words));
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
        what: "tap".into(),
        why: REFUSED_WHY.into(),
    };
    assert_eq!(
        REFUSED_WHY,
        "the AUTOTUNE tap is gone; a tap opens the singers' panel"
    );
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
        "AUTOTUNE refuses a page's tap: the AUTOTUNE tap is gone; a tap opens the singers' panel"
    );
    assert_eq!(
        step_line(&Step::Refused {
            what: "aut".into(),
            why: "w".into(),
        }),
        "AUTOTUNE refuses a page's aut: w"
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
        json!({"step": "refused", "what": "tap",
               "why": "the AUTOTUNE tap is gone; a tap opens the singers' panel"})
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
