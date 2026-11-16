use super::*;

/// A rule hearing Live play in 4/4.
fn playing() -> Claps {
    let mut claps = Claps::new();
    claps.signature(Some(true));
    claps.transport(Some(true));
    claps
}

#[test]
fn a_tap_snaps_to_the_nearest_whole_beat_of_the_bar() {
    assert_eq!(beat_of(0.0), 0);
    assert_eq!(beat_of(0.49), 0);
    assert_eq!(beat_of(0.5), 1);
    assert_eq!(beat_of(1.2), 1);
    assert_eq!(beat_of(3.4), 3);
    assert_eq!(beat_of(3.6), 0, "the next bar's first beat");
    assert_eq!(beat_of(102.9), 3);
    assert_eq!(beat_of(-0.2), 0);
    assert_eq!(beat_of(-1.0), 3);
}

#[test]
fn a_bar_is_four_beats_from_the_songs_start() {
    assert_eq!(bar_of(0.0), 0);
    assert_eq!(bar_of(3.99), 0);
    assert_eq!(bar_of(4.0), 1);
    assert_eq!(bar_of(17.5), 4);
    assert_eq!(bar_of(-0.5), -1);
}

#[test]
fn the_position_of_a_tap_moves_on_from_the_playhead_heard_at_the_tempo() {
    // 120 bpm: two beats a second.
    assert_eq!(position_at(10.0, 1000.0, Some(120.0), 1500.0), Some(11.0));
    assert_eq!(position_at(10.0, 1000.0, Some(120.0), 750.0), Some(9.5));
    assert_eq!(position_at(10.0, 1000.0, Some(200.0), 1300.0), Some(11.0));
    // Exactly the bound still counts; past it the playhead is stale.
    assert_eq!(position_at(10.0, 1000.0, Some(120.0), 3000.0), Some(14.0));
    assert_eq!(position_at(10.0, 1000.0, Some(120.0), 3000.5), None);
    assert_eq!(position_at(10.0, 3000.0, Some(120.0), 999.0), None);
    for tempo in [
        None,
        Some(0.0),
        Some(-120.0),
        Some(f64::NAN),
        Some(f64::INFINITY),
    ] {
        assert_eq!(position_at(10.0, 1000.0, tempo, 1500.0), None, "{tempo:?}");
    }
}

#[test]
fn only_four_four_is_four_four() {
    assert_eq!(four_four(Some(4.0), Some(4.0)), Some(true));
    assert_eq!(four_four(Some(3.0), Some(4.0)), Some(false));
    assert_eq!(four_four(Some(4.0), Some(8.0)), Some(false));
    assert_eq!(four_four(None, Some(4.0)), None);
    assert_eq!(four_four(Some(4.0), None), None);
    assert_eq!(signature_text(Some(6.0), Some(8.0)).as_deref(), Some("6/8"));
    assert_eq!(signature_text(Some(6.0), None), None);
    assert_eq!(signature_text(None, Some(8.0)), None);
}

#[test]
fn an_active_clip_covers_from_its_start_up_to_its_end() {
    let clips = [
        Clip {
            start: 8.0,
            end: 16.0,
            muted: false,
        },
        Clip {
            start: 32.0,
            end: 64.0,
            muted: true,
        },
    ];
    assert!(!covers(&clips, 7.99));
    assert!(covers(&clips, 8.0));
    assert!(covers(&clips, 15.99));
    assert!(!covers(&clips, 16.0));
    assert!(!covers(&clips, 40.0), "a deactivated clip claps nothing");
    assert!(!covers(&[], 8.0));
    assert_eq!(active(&clips), 1);
    assert_eq!(active(&[]), 0);
}

#[test]
fn taps_gather_until_a_bar_without_a_tap_then_the_pattern_starts_at_the_next_bar() {
    let mut claps = playing();
    assert_eq!(claps.tap(Some(5.1)), Ok(1));
    assert_eq!(claps.tap(Some(6.9)), Ok(3));
    assert_eq!(claps.phase(false), ClapsPhase::Tapping);
    assert_eq!(claps.shown(), [false, true, false, true]);
    assert_eq!(claps.pattern(), NONE, "the taps are no pattern yet");
    assert!(!claps.words(false, &[50]).on, "silent while tapping");
    // Within a bar of the last tap: still tapping.
    assert!(!claps.position(10.8));
    assert_eq!(claps.phase(false), ClapsPhase::Tapping);
    // A bar with no tap: the taps are the pattern, on from the next bar.
    assert!(claps.position(10.9));
    assert_eq!(claps.mode(), Mode::Armed { bar: Some(2) });
    assert_eq!(claps.phase(false), ClapsPhase::NextBar);
    let words = claps.words(false, &[50]);
    assert_eq!(words.pattern, [false, true, false, true]);
    assert!(words.on);
    assert_eq!(words.notes, vec![50]);
    // Still that bar; then the next one: the device claps.
    assert!(!claps.position(11.9));
    assert!(claps.position(12.0));
    assert_eq!(claps.phase(false), ClapsPhase::On);
    assert!(!claps.position(13.0));
}

#[test]
fn a_new_tapping_replaces_the_pattern_and_silences_the_device_meanwhile() {
    let mut claps = playing();
    claps.tap(Some(1.0)).unwrap();
    claps.position(6.0);
    claps.position(8.0);
    assert_eq!(claps.mode(), Mode::On);
    assert_eq!(claps.tap(Some(10.0)), Ok(2));
    assert_eq!(claps.shown(), [false, false, true, false]);
    assert_eq!(
        claps.pattern(),
        [false, true, false, false],
        "until it ends"
    );
    assert!(!claps.words(false, &[50]).on);
    claps.position(14.5);
    assert_eq!(claps.pattern(), [false, false, true, false]);
    assert_eq!(claps.mode(), Mode::Armed { bar: Some(3) });
}

#[test]
fn a_jump_back_before_the_last_tap_ends_the_tapping() {
    let mut claps = playing();
    claps.tap(Some(40.0)).unwrap();
    assert!(!claps.position(36.0), "a bar back is still the tapping");
    assert!(claps.position(35.9));
    assert_eq!(claps.mode(), Mode::Armed { bar: Some(8) });
}

#[test]
fn a_tap_needs_four_four_a_playing_live_and_a_playhead() {
    let mut claps = Claps::new();
    assert_eq!(claps.tap(Some(1.0)), Err(Refusal::Signature));
    claps.signature(Some(false));
    assert_eq!(claps.tap(Some(1.0)), Err(Refusal::Signature));
    claps.signature(Some(true));
    assert_eq!(claps.tap(Some(1.0)), Err(Refusal::Stopped));
    claps.transport(Some(false));
    assert_eq!(claps.tap(Some(1.0)), Err(Refusal::Stopped));
    claps.transport(Some(true));
    assert_eq!(claps.tap(None), Err(Refusal::NoPosition));
    assert_eq!(claps.mode(), Mode::Off);
    assert_eq!(claps.tap(Some(1.0)), Ok(1));
}

#[test]
fn every_refusal_has_its_words() {
    assert_eq!(Refusal::NoTrack.words(), "no +CLAPS track");
    assert_eq!(Refusal::Signature.words(), "not 4/4");
    assert_eq!(Refusal::Stopped.words(), "Live does not play");
    assert_eq!(Refusal::NoPosition.words(), "no playhead heard");
}

#[test]
fn a_stop_while_tapping_keeps_the_taps_waiting_for_the_play() {
    let mut claps = playing();
    claps.tap(Some(1.0)).unwrap();
    claps.tap(Some(3.0)).unwrap();
    assert!(claps.transport(Some(false)));
    assert_eq!(claps.mode(), Mode::Armed { bar: None });
    assert_eq!(claps.pattern(), [false, true, false, true]);
    assert_eq!(claps.phase(false), ClapsPhase::Waits);
    assert!(
        claps.words(false, &[50]).on,
        "the device waits for the play"
    );
    // The play's first position names the bar; the next bar claps.
    assert!(!claps.transport(Some(true)), "the play alone names no bar");
    assert!(claps.position(17.5));
    assert_eq!(claps.mode(), Mode::Armed { bar: Some(4) });
    assert_eq!(claps.phase(false), ClapsPhase::NextBar);
    assert!(claps.position(20.0));
    assert_eq!(claps.phase(false), ClapsPhase::On);
}

#[test]
fn a_stop_while_armed_waits_for_the_bar_after_the_play_and_while_on_it_waits() {
    let mut claps = playing();
    claps.toggle(1, true);
    claps.position(2.0);
    assert_eq!(claps.mode(), Mode::Armed { bar: Some(0) });
    assert!(claps.transport(Some(false)));
    assert_eq!(claps.mode(), Mode::Armed { bar: None });
    assert!(!claps.position(2.0), "a stopped playhead names no bar");
    claps.transport(Some(true));
    claps.position(9.0);
    claps.position(12.0);
    assert_eq!(claps.mode(), Mode::On);
    assert!(!claps.transport(Some(false)), "on stays on");
    assert_eq!(claps.phase(false), ClapsPhase::Waits);
    assert!(claps.words(false, &[50]).on);
    assert!(!claps.transport(None));
    assert_eq!(claps.phase(false), ClapsPhase::Waits);
}

#[test]
fn a_clip_over_the_playhead_silences_the_device_and_shows_clip() {
    let mut claps = playing();
    claps.toggle(0, true);
    claps.position(1.0);
    claps.position(4.0);
    assert_eq!(claps.phase(true), ClapsPhase::Clip);
    let words = claps.words(true, &[52, 57]);
    assert!(words.on && words.silent);
    assert_eq!(words.notes, vec![52, 57]);
    assert_eq!(claps.phase(false), ClapsPhase::On);
    // A clip claps on its own whatever the rule does.
    claps.stop();
    assert_eq!(claps.phase(true), ClapsPhase::Off);
}

#[test]
fn another_signature_claps_nothing_and_four_four_again_starts_at_a_bar() {
    let mut claps = playing();
    claps.toggle(1, true);
    claps.position(1.0);
    claps.position(4.0);
    assert_eq!(claps.mode(), Mode::On);
    assert!(claps.signature(Some(false)));
    assert_eq!(claps.phase(false), ClapsPhase::Signature);
    assert!(!claps.words(false, &[50]).on);
    assert_eq!(claps.mode(), Mode::Armed { bar: None });
    assert!(claps.signature(Some(true)));
    assert!(claps.words(false, &[50]).on);
    assert_eq!(claps.phase(false), ClapsPhase::NextBar);
    // While off too: the strip says why nothing claps.
    claps.stop();
    claps.signature(Some(false));
    assert_eq!(claps.phase(false), ClapsPhase::Signature);
    assert!(!claps.signature(Some(false)), "the same signature again");
    claps.signature(None);
    assert_eq!(claps.phase(false), ClapsPhase::Off);
}

#[test]
fn leaving_four_four_while_tapping_ends_it() {
    let mut claps = playing();
    claps.tap(Some(2.0)).unwrap();
    assert!(claps.signature(Some(false)));
    assert_eq!(claps.mode(), Mode::Armed { bar: None });
    assert_eq!(claps.pattern(), [false, false, true, false]);
    assert!(!claps.words(false, &[50]).on);
}

#[test]
fn a_dot_set_while_off_starts_that_beat_from_the_next_bar() {
    let mut claps = playing();
    assert!(!claps.toggle(2, false), "clearing a dot of nothing");
    assert!(claps.toggle(2, true));
    assert_eq!(claps.pattern(), [false, false, true, false]);
    assert_eq!(claps.mode(), Mode::Armed { bar: None });
    assert!(claps.position(5.0));
    assert_eq!(claps.mode(), Mode::Armed { bar: Some(1) });
    assert!(claps.toggle(0, true));
    assert!(!claps.toggle(0, true), "set twice");
    assert_eq!(claps.pattern(), [true, false, true, false]);
    assert!(claps.toggle(2, false));
    assert_eq!(claps.mode(), Mode::Armed { bar: Some(1) });
    // The last beat cleared stops.
    assert!(claps.toggle(0, false));
    assert_eq!(claps.mode(), Mode::Off);
    assert!(!claps.toggle(4, true), "past the bar");
    assert_eq!(claps.mode(), Mode::Off);
}

#[test]
fn a_dot_while_tapping_switches_a_tap() {
    let mut claps = playing();
    claps.tap(Some(1.0)).unwrap();
    assert!(claps.toggle(3, true));
    assert!(!claps.toggle(3, true));
    assert!(claps.toggle(1, false));
    assert_eq!(claps.shown(), [false, false, false, true]);
    // A tapping whose taps were all cleared ends with nothing clapping.
    claps.toggle(3, false);
    claps.position(6.0);
    assert_eq!(claps.mode(), Mode::Off);
}

#[test]
fn a_dot_on_an_on_pattern_changes_it_without_rearming() {
    let mut claps = playing();
    claps.toggle(1, true);
    claps.position(1.0);
    claps.position(4.0);
    assert!(claps.toggle(3, true));
    assert_eq!(claps.mode(), Mode::On);
    assert_eq!(claps.pattern(), [false, true, false, true]);
}

#[test]
fn a_stop_clears_everything_once() {
    let mut claps = playing();
    assert!(!claps.stop(), "nothing to stop");
    claps.tap(Some(0.0)).unwrap();
    assert!(claps.stop());
    assert_eq!(claps.mode(), Mode::Off);
    assert_eq!(claps.shown(), NONE);
    claps.toggle(1, true);
    assert!(claps.stop());
    assert_eq!(claps.pattern(), NONE);
    assert!(!claps.words(false, &[50]).on);
}

#[test]
fn offline_forgets_live_and_waits() {
    let mut claps = playing();
    claps.tap(Some(1.0)).unwrap();
    claps.offline();
    assert_eq!(claps.mode(), Mode::Armed { bar: None });
    assert!(!claps.words(false, &[50]).on, "no signature known");
    assert_eq!(claps.phase(false), ClapsPhase::Waits);
    assert_eq!(claps.tap(Some(1.0)), Err(Refusal::Signature));
}

#[test]
fn the_ring_is_told_again_past_a_quarter_beat_of_drift() {
    assert!(ring_off(None, 1.0, 0.0), "never told");
    // Told 4.0 at 1000 ms, 120 bpm: at 1500 ms it expects 5.0.
    let told = Some((4.0, 1000.0, 120.0));
    assert!(!ring_off(told, 5.0, 1500.0));
    assert!(!ring_off(told, 5.25, 1500.0));
    assert!(ring_off(told, 5.26, 1500.0));
    assert!(!ring_off(told, 4.75, 1500.0));
    assert!(ring_off(told, 4.74, 1500.0));
    assert!(ring_off(told, 1.0, 1500.0), "a jump back");
}

#[test]
fn a_default_rule_claps_nothing() {
    let claps = Claps::default();
    assert_eq!(claps.mode(), Mode::Off);
    assert_eq!(claps.phase(false), ClapsPhase::Off);
    assert_eq!(claps.words(false, &[]), ClapsWords::default());
}
