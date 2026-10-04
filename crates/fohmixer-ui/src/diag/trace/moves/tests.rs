//! Tests of `moves.rs`: a touch's start, a frame's moves, the finger's 1:1
//! position, and which records are a touch's first.

use super::*;

const KEY: &str = "band|live_set tracks[name=Vox 1] mixer_device volume|value";

fn press(at: f64, c: f64) -> Press {
    Press {
        at,
        c,
        travel: 300.0,
    }
}

#[test]
fn numbers_are_rounded_to_keep_records_small() {
    assert_eq!(FIRST_MOVES, 8);
    assert_eq!(round1(412.46), 412.5);
    assert_eq!(round1(-3.04), -3.0);
    assert_eq!(round1(1_791_072_079_811.823_7), 1_791_072_079_811.8);
    assert_eq!(round5(0.512_345_6), 0.512_35);
    assert_eq!(round5(0.299_994), 0.299_99);
}

#[test]
fn a_fader_rises_as_the_finger_goes_up_a_pan_as_it_goes_right() {
    assert_eq!(Axis::default(), Axis::Up);
    assert_eq!(Axis::Up.travelled(500.0, 470.0, 300.0), 0.1);
    assert_eq!(Axis::Up.travelled(500.0, 530.0, 300.0), -0.1);
    assert_eq!(Axis::Right.travelled(100.0, 130.0, 300.0), 0.1);
    assert_eq!(Axis::Right.travelled(100.0, 70.0, 150.0), -0.2);
    // A control with no travel yet counts 1 px.
    assert_eq!(Axis::Up.travelled(10.0, 8.0, 0.0), 2.0);
}

#[test]
fn a_touch_start_says_where_the_finger_went_down_and_where_the_touch_starts() {
    let keys = vec![KEY.to_string()];
    let start = Start {
        shown: 0.512_341,
        live: 0.433_336,
        local: true,
        from: 0.512_341,
    };
    assert_eq!(
        touch_start(10_000.25, &keys, 7, press(9_996.25, 412.46), start),
        json!({
            "ev": "touch",
            "t": 10_000.25,
            "what": "down",
            "keys": keys,
            "pointer": 7,
            "dt": -4.0,
            "c": 412.5,
            "travel": 300.0,
            "pos": 0.51234,
            "live": 0.43334,
            "local": true,
            "from": 0.51234,
        })
    );
}

#[test]
fn a_frame_takes_the_moves_since_the_last_one_with_the_raw_and_sent_position() {
    let mut trail = Trail::default();
    trail.start(7, Axis::Up, press(1_000.0, 500.0), 0.5);
    assert_eq!(trail.take(1_010.0, KEY, 0.5, Some(1)), None, "no move yet");
    trail.moved(7, 1_012.0, 497.0);
    trail.moved(8, 1_013.0, 100.0);
    trail.moved(7, 1_020.0, 494.0);
    let (record, first) = trail
        .take(1_024.0, KEY, 0.519_8, Some(12))
        .expect("a frame");
    assert_eq!(
        record,
        json!({
            "ev": "mv",
            "t": 1_024.0,
            "key": KEY,
            "p": 7,
            "e": [[-12.0, 497.0], [-4.0, 494.0]],
            "r": 0.52,
            "s": 0.5198,
            "q": 12,
        }),
        "another pointer's move is not this touch's"
    );
    assert!(first);
    assert_eq!(trail.take(1_040.0, KEY, 0.52, Some(13)), None, "taken");
    trail.moved(7, 1_041.0, 464.0);
    let (record, _) = trail.take(1_042.06, KEY, 0.6, None).expect("a frame");
    assert_eq!(record["r"], json!(0.62), "from 0.5, 36 px up of 300");
    assert_eq!(record["q"], Value::Null, "no set");
    assert_eq!(
        record["t"],
        json!(1_042.06),
        "its own time whole (the timeline matches it against the lift)"
    );
    assert_eq!(record["e"], json!([[-1.1, 464.0]]));
}

#[test]
fn the_raw_position_stays_within_the_travel() {
    let mut trail = Trail::default();
    trail.start(1, Axis::Right, press(0.0, 100.0), 0.9);
    assert_eq!(trail.raw(160.0), 1.0);
    trail.start(1, Axis::Right, press(0.0, 100.0), 0.5);
    assert_eq!(trail.raw(130.0), 0.6);
    trail.start(1, Axis::Up, press(0.0, 100.0), 0.05);
    assert_eq!(trail.raw(130.0), 0.0);
}

#[test]
fn a_touchs_first_eight_frames_are_essential() {
    let mut trail = Trail::default();
    trail.start(3, Axis::Up, press(0.0, 500.0), 0.5);
    let mut firsts = Vec::new();
    for i in 1..=10 {
        trail.moved(3, f64::from(i), 500.0 - f64::from(i));
        firsts.push(trail.take(f64::from(i), KEY, 0.5, None).expect("a frame").1);
    }
    assert_eq!(
        firsts,
        [true, true, true, true, true, true, true, true, false, false]
    );
    // A new touch counts again.
    trail.start(3, Axis::Up, press(20.0, 500.0), 0.5);
    trail.moved(3, 21.0, 499.0);
    assert!(trail.take(21.0, KEY, 0.5, None).expect("a frame").1);
}

#[test]
fn a_touchs_end_drops_its_moves_another_pointers_end_does_not() {
    let mut trail = Trail::default();
    trail.start(3, Axis::Up, press(0.0, 500.0), 0.5);
    trail.moved(3, 1.0, 499.0);
    trail.end(4);
    assert!(
        trail.take(2.0, KEY, 0.5, None).is_some(),
        "another finger's lift"
    );
    trail.moved(3, 3.0, 498.0);
    trail.end(3);
    assert_eq!(trail.take(4.0, KEY, 0.5, None), None);
    trail.moved(3, 5.0, 497.0);
    assert_eq!(trail.take(6.0, KEY, 0.5, None), None, "no finger drives it");
}
