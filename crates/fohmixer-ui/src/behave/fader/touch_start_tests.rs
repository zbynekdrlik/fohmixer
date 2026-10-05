//! A touch's start (#43 PR F, design comment 5990698988): the touch's first
//! pointer move only anchors the drag, and the shaping's size thresholds
//! are the same finger distance as TouchOSC's on the FOH iPad (its strips
//! were 355 px tall there; our fader's travel is 207 px).
//!
//! The owner's test of 2026-10-05: the first pointer event of a touch came a
//! median 83 ms after the down and 6 px away (up to 15 px), later ones 2 px
//! every 17 ms. Applied at once, that first event moved the cap up to ~7 %
//! of the travel in one frame, and over 3 % of the travel it bypassed the
//! TouchOSC shaping (an emergency), so the fine first step never came.

use super::*;

/// The fader's travel on the FOH iPad (px).
const IPAD: f64 = 207.0;
/// TouchOSC's strips' travel on the FOH iPad (px).
const TOUCHOSC: f64 = 355.0;

#[track_caller]
fn assert_close(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-12, "{a} != {b}");
}

/// A shaped volume fader touched by pointer 1 at y = 500 at t = 0, on a
/// travel of `travel` px, Live's value sitting at position `live`.
fn touched(travel: f64, live: f64) -> FaderCtl {
    let mut f = FaderCtl::new(true, Some(to_pos(UNITY)));
    assert!(f.down(1, 500.0, travel, 0.0, live));
    f
}

#[test]
fn a_touchs_first_move_only_anchors_the_drag_and_the_next_gets_the_fine_first_step() {
    let mut f = touched(IPAD, 0.5);
    // The first pointer event, 10 px up: the fader takes it, moves nothing
    // and sends nothing.
    assert!(f.moved(1, 490.0, 83.0));
    assert_eq!(
        f.frame(90.0, Some(0.5)),
        Motion {
            pos: Some(0.5),
            send: None
        },
        "no jump on the first move's frame"
    );
    // 2 px more: TouchOSC's first step from where the touch started (0.9 of
    // the move: 2 px here are 0.28 dB, over the 0.1 dB minimum).
    assert!(f.moved(1, 488.0, 100.0));
    let frame = f.frame(106.0, Some(0.5));
    assert_close(frame.pos.unwrap(), 0.5 + 0.9 * 2.0 / IPAD);
    assert_close(frame.send.unwrap(), 0.5 + 0.9 * 2.0 / IPAD);
    // And on: the drag is relative to the anchor.
    assert!(f.moved(1, 478.0, 117.0));
    assert!(f.frame(122.0, Some(0.5)).pos.unwrap() > 0.5 + 0.9 * 2.0 / IPAD);
}

#[test]
fn the_forced_one_tenth_db_first_step_comes_after_the_anchor() {
    // On TouchOSC's travel 1 px from 0 dB is 0.07 dB: forced to 0.1 dB, the
    // value of the Lua trace (`a_small_first_move_is_forced_…`).
    let mut f = touched(TOUCHOSC, 0.7294);
    assert!(f.moved(1, 490.0, 80.0));
    assert!(f.moved(1, 489.0, 97.0));
    assert_close(
        f.frame(100.0, Some(0.7294)).send.unwrap(),
        0.7335579835647315,
    );
}

#[test]
fn a_touch_whose_finger_only_anchored_sends_nothing() {
    let mut f = touched(IPAD, 0.5);
    assert!(f.moved(1, 494.0, 83.0));
    assert_eq!(f.up(1, 120.0), None, "nothing unsent");
    assert_eq!(f.frame(130.0, Some(0.5)).pos, Some(0.5));
    assert_eq!(f.frame(130.0, Some(0.5)).send, None);
}

#[test]
fn each_touch_anchors_its_own_first_move() {
    let mut f = touched(IPAD, 0.5);
    assert!(f.moved(1, 494.0, 83.0));
    assert!(f.moved(1, 474.0, 100.0));
    let first = f.frame(110.0, Some(0.5)).pos.unwrap();
    assert!(first > 0.5);
    assert_eq!(f.up(1, 120.0), None);
    // Within the hold: the next touch starts from the cap, and its first
    // move (15 px down) anchors again.
    assert!(f.down(2, 300.0, IPAD, 300.0, 0.5));
    assert!(f.moved(2, 315.0, 380.0));
    assert_eq!(f.frame(390.0, Some(0.5)).pos, Some(first));
    assert_eq!(f.frame(390.0, Some(0.5)).send, None);
}

#[test]
fn another_pointers_move_is_not_the_touchs_first() {
    let mut f = touched(IPAD, 0.5);
    assert!(!f.moved(2, 400.0, 50.0), "another finger");
    // The touch's own first move still anchors.
    assert!(f.moved(1, 490.0, 83.0));
    assert_eq!(f.frame(90.0, Some(0.5)).pos, Some(0.5));
}

#[test]
fn a_tap_whose_finger_slid_at_the_start_still_counts_toward_a_double_tap() {
    // Two taps whose fingers' first events came 6 px off: the fader never
    // moved, so they are taps, and the second glides to 0 dB.
    let mut f = FaderCtl::new(false, Some(to_pos(UNITY)));
    assert!(f.down(1, 500.0, IPAD, 0.0, 0.25));
    assert!(f.moved(1, 494.0, 30.0));
    assert_eq!(f.up(1, 60.0), None);
    assert!(f.down(1, 500.0, IPAD, 160.0, 0.25));
    assert!(f.moved(1, 506.0, 190.0));
    assert_eq!(f.up(1, 220.0), None);
    let frame = f.frame(1_220.0, Some(0.25));
    let sent = frame.send.expect("the double tap's glide sends");
    assert_close(sent, 0.25 + GLIDE_SPEED);
}

/// A shaped touch on `travel` px from position `start`: its first move
/// anchors (0 px), then the finger goes `px` up. The position sent.
fn first_step(travel: f64, start: f64, px: f64) -> f64 {
    let mut f = touched(travel, start);
    assert!(f.moved(1, 500.0, 80.0));
    assert!(f.moved(1, 500.0 - px, 100.0));
    f.frame(110.0, Some(start)).send.unwrap()
}

#[test]
fn a_first_step_bypasses_the_shaping_at_touchoscs_finger_distance_on_a_short_fader() {
    // The emergency is over 10.65 px (3 % of 355 px). On 207 px, 10.6 px is
    // 5.1 % of the travel and still shaped (0.9); 10.7 px passes raw.
    assert_close(first_step(IPAD, 0.5, 10.6), 0.5 + 0.9 * 10.6 / IPAD);
    assert_close(first_step(IPAD, 0.5, 10.7), 0.5 + 10.7 / IPAD);
}

#[test]
fn on_touchoscs_travel_the_thresholds_are_todays() {
    // 3 % of the travel, as before: 10.6 px shaped, 10.7 px raw.
    assert_close(first_step(TOUCHOSC, 0.5, 10.6), 0.5 + 0.9 * 10.6 / TOUCHOSC);
    assert_close(first_step(TOUCHOSC, 0.5, 10.7), 0.5 + 10.7 / TOUCHOSC);
}

/// A shaped touch on `travel` px from position 0.2 (below the linear
/// range): the anchor, a 30 px emergency, then `px` up. How far that last
/// move took the fader.
fn after_an_emergency(travel: f64, px: f64) -> f64 {
    let mut f = touched(travel, 0.2);
    assert!(f.moved(1, 500.0, 80.0));
    assert!(f.moved(1, 470.0, 100.0));
    let fast = f.frame(105.0, Some(0.2)).send.unwrap();
    assert_close(fast, 0.2 + 30.0 / travel);
    assert!(f.moved(1, 470.0 - px, 117.0));
    f.frame(120.0, Some(0.2)).send.unwrap() - fast
}

#[test]
fn an_emergency_ends_at_touchoscs_calm_finger_distance() {
    // Calm is under 5.325 px (1.5 % of 355 px): the scaled moves start
    // (0.9); 5.4 px stays an emergency (raw).
    assert_close(after_an_emergency(IPAD, 5.3), 0.9 * 5.3 / IPAD);
    assert_close(after_an_emergency(IPAD, 5.4), 5.4 / IPAD);
    // On TouchOSC's travel, as before (1.5 % of it).
    assert_close(after_an_emergency(TOUCHOSC, 5.3), 0.9 * 5.3 / TOUCHOSC);
    assert_close(after_an_emergency(TOUCHOSC, 5.4), 5.4 / TOUCHOSC);
}

#[test]
fn a_move_is_measured_as_the_finger_distance_on_touchoscs_strip() {
    // TouchOSC's own travel leaves the size as it is (3 % stays 0.03).
    assert_eq!(finger_size(0.03, TOUCHOSC), 0.03);
    // 5 % of 207 px is 10.35 px: 2.9 % of 355 px. Direction does not count.
    assert_close(finger_size(-0.05, IPAD), 0.029_154_929_577_464_79);
    assert_close(finger_size(0.1, 710.0), 0.2);
    assert_eq!(finger_size(0.0, IPAD), 0.0);
}

#[test]
fn a_fader_that_keeps_touchoscs_first_move_takes_it_whole() {
    // The imported parameter faders (shaping off): their first move moves.
    let mut f = FaderCtl::new(false, None).anchoring(false);
    assert!(f.down(1, 500.0, IPAD, 0.0, 0.5));
    assert!(f.moved(1, 490.0, 83.0));
    let frame = f.frame(90.0, Some(0.5));
    assert_close(frame.pos.unwrap(), 0.5 + 10.0 / IPAD);
    assert_close(frame.send.unwrap(), 0.5 + 10.0 / IPAD);
    // And anchoring on is the default's.
    let mut f = FaderCtl::new(false, None).anchoring(true);
    assert!(f.down(1, 500.0, IPAD, 0.0, 0.5));
    assert!(f.moved(1, 490.0, 83.0));
    assert_eq!(f.frame(90.0, Some(0.5)).send, None);
}

#[test]
fn the_fader_says_where_a_move_left_it() {
    // The flight recorder's `a` (#43 PR F): read right after the move.
    let mut f = touched(IPAD, 0.5);
    assert_eq!(f.pos(), 0.5, "the touch's start");
    assert!(f.moved(1, 490.0, 83.0));
    assert_eq!(f.pos(), 0.5, "the anchor moved nothing");
    assert!(f.moved(1, 488.0, 100.0));
    assert_close(f.pos(), 0.5 + 0.9 * 2.0 / IPAD);
}
