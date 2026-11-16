use fohmixer_proto::claps::{ClapsPhase, ClapsState};

use super::*;

fn told(
    state: ClapsPhase,
    pattern: [bool; 4],
    beat: Option<f64>,
    tempo: Option<f64>,
) -> ClapsState {
    ClapsState {
        state,
        pattern,
        beat,
        tempo,
    }
}

#[test]
fn the_line_says_each_phase_and_nothing_while_off() {
    let words: Vec<(&str, bool, &str)> = [
        ClapsPhase::Off,
        ClapsPhase::Tapping,
        ClapsPhase::NextBar,
        ClapsPhase::On,
        ClapsPhase::Clip,
        ClapsPhase::Waits,
        ClapsPhase::Signature,
    ]
    .into_iter()
    .map(|p| (state_text(p), state_lit(p), phase_name(p)))
    .collect();
    assert_eq!(
        words,
        vec![
            ("", false, "off"),
            ("TAPPING", true, "tapping"),
            ("NEXT BAR", true, "next_bar"),
            ("CLAPS", true, "on"),
            ("CLIP", false, "clip"),
            ("WAITS", false, "waits"),
            ("4/4 only", false, "signature"),
        ]
    );
}

#[test]
fn the_playhead_moves_on_at_the_tempo_from_the_one_told() {
    let s = told(ClapsPhase::On, [true; 4], Some(8.0), Some(120.0));
    assert_eq!(position(&s, 1000.0, 1000.0), Some(8.0));
    assert_eq!(position(&s, 1000.0, 1750.0), Some(9.5));
    assert_eq!(
        position(&s, 1000.0, 1000.0 + RING_MS),
        Some(8.0 + RING_MS / 500.0)
    );
    assert_eq!(
        position(&s, 1000.0, 1000.0 + RING_MS + 1.0),
        None,
        "too old"
    );
    assert_eq!(position(&s, 1000.0, 999.0), None, "before it was told");
    for (beat, tempo) in [
        (None, Some(120.0)),
        (Some(8.0), None),
        (Some(8.0), Some(0.0)),
        (Some(8.0), Some(f64::NAN)),
    ] {
        let s = told(ClapsPhase::On, [true; 4], beat, tempo);
        assert_eq!(position(&s, 0.0, 10.0), None, "{beat:?} {tempo:?}");
    }
}

#[test]
fn the_ring_shows_the_beat_of_the_bar_only_while_something_moves() {
    let s = told(ClapsPhase::On, [true; 4], Some(8.0), Some(120.0));
    assert_eq!(ring(&s, 0.0, 0.0), Some(0));
    assert_eq!(ring(&s, 0.0, 499.0), Some(0));
    assert_eq!(ring(&s, 0.0, 500.0), Some(1));
    assert_eq!(ring(&s, 0.0, 1999.0), Some(3));
    assert_eq!(ring(&s, 0.0, 2000.0), Some(0), "the next bar");
    for phase in [ClapsPhase::Tapping, ClapsPhase::NextBar, ClapsPhase::Clip] {
        let s = told(phase, [false; 4], Some(5.0), Some(120.0));
        assert_eq!(ring(&s, 0.0, 0.0), Some(1), "{phase:?}");
    }
    for phase in [ClapsPhase::Off, ClapsPhase::Waits, ClapsPhase::Signature] {
        let s = told(phase, [true; 4], Some(5.0), Some(120.0));
        assert_eq!(ring(&s, 0.0, 0.0), None, "{phase:?}");
    }
    let stopped = told(ClapsPhase::On, [true; 4], None, Some(120.0));
    assert_eq!(ring(&stopped, 0.0, 0.0), None);
}

#[test]
fn a_dot_flashes_at_the_start_of_a_beat_live_claps() {
    let s = told(
        ClapsPhase::On,
        [false, true, false, true],
        Some(4.0),
        Some(120.0),
    );
    assert!(!hit(&s, 0.0, 0.0), "beat 1 is not in the pattern");
    assert!(hit(&s, 0.0, 500.0), "beat 2 begins");
    assert!(hit(&s, 0.0, 500.0 + 0.23 * 500.0));
    assert!(!hit(&s, 0.0, 500.0 + 0.25 * 500.0), "past the flash");
    let at_end = told(ClapsPhase::On, [true; 4], Some(HIT_BEATS), Some(120.0));
    assert!(
        !hit(&at_end, 0.0, 0.0),
        "a beat exactly HIT_BEATS old no longer flashes"
    );
    let clip = told(ClapsPhase::Clip, [true; 4], Some(4.0), Some(120.0));
    assert!(!hit(&clip, 0.0, 0.0), "the clip claps, not the device");
    let tapping = told(ClapsPhase::Tapping, [true; 4], Some(4.0), Some(120.0));
    assert!(!hit(&tapping, 0.0, 0.0));
    let unknown = told(ClapsPhase::On, [true; 4], None, None);
    assert!(!hit(&unknown, 0.0, 0.0));
}

#[test]
fn a_dot_asks_its_beat_the_other_way() {
    let pattern = [false, true, false, false];
    assert!(dot_tap(&pattern, 0));
    assert!(!dot_tap(&pattern, 1));
    assert!(
        dot_tap(&pattern, 9),
        "past the bar: asks on, the hub refuses it"
    );
}

#[test]
fn a_hold_stops_only_on_this_connections_state() {
    assert_eq!(stop_value(true, true), Some(()));
    assert_eq!(stop_value(false, true), None);
    assert_eq!(stop_value(true, false), None);
}
