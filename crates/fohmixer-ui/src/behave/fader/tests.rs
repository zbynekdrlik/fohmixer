use super::*;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-12
}

#[track_caller]
fn assert_close(a: f64, b: f64) {
    assert!(close(a, b), "{a} != {b}");
}

#[test]
fn the_law_puts_minus_six_db_at_half_travel() {
    // Spec F8: v = p^0.515 and its inverse.
    assert_close(to_live(0.5), 0.6997929327975979);
    assert_close(to_pos(0.85), 0.7293724756757368);
    assert_close(to_live(0.25), 0.4897101487934634);
    assert_close(to_pos(0.5), 0.2603009400817625);
    assert_close(to_pos(to_live(0.3)), 0.3);
    for (p, v) in [(0.0, 0.0), (-0.5, 0.0), (1.0, 1.0), (1.5, 1.0)] {
        assert_eq!(to_live(p), v, "to_live({p})");
        assert_eq!(to_pos(p), v, "to_pos({p})");
    }
}

#[test]
fn the_cc_linear_scale_spans_the_parameter_range() {
    assert_eq!(linear_pos(0.85, (0.0, 1.0)), 0.85);
    assert_eq!(linear_pos(5.0, (-15.0, 15.0)), 20.0 / 30.0);
    assert_eq!(linear_pos(-20.0, (-15.0, 15.0)), 0.0);
    assert_eq!(linear_pos(127.0, (0.0, 100.0)), 1.0);
    assert_eq!(linear_value(0.25, (0.0, 127.0)), 31.75);
    assert_eq!(linear_value(0.5, (-15.0, 15.0)), 0.0);
    assert_eq!(linear_value(1.5, (200.0, 60000.0)), 60000.0);
    assert_eq!(linear_value(-1.0, (200.0, 60000.0)), 200.0);
}

#[test]
fn value2db_is_the_touchosc_curve_in_each_range() {
    for (v, db) in [
        (1.0, 6.0),
        (0.85, 0.0),
        (0.4, -18.0),
        (0.39999, -18.001762236950533),
        (0.2, -34.39049787108893),
        (0.15, -40.986236048588935),
        (0.14999, -41.01368891610561),
        (0.1, -48.54258355581338),
        (1e-6, -69.995809918992),
        (1.5, 0.0),
    ] {
        assert!(close(value2db(v), db), "value2db({v}) = {}", value2db(v));
    }
    assert_eq!(value2db(0.0), f64::NEG_INFINITY);
}

#[test]
fn the_bisection_splits_at_an_exact_hit_like_the_lua() {
    // value2db(0.5) is exactly -14 dB: the first midpoint hits the target
    // and the Lua moves `high` (its test is `mid_db < target`).
    assert_close(audio_for_db_change(0.5, 0.0), 0.499969482421875);
}

#[test]
fn the_shaping_thresholds_are_strict_where_the_lua_is() {
    assert!(!is_emergency(0.03) && is_emergency(0.030000000000000002));
    assert!(!is_calm(0.015) && is_calm(0.014999999999999998));
    assert!(!short_of_step(0.1) && !short_of_step(-0.1));
    assert!(short_of_step(0.09999999999999999) && short_of_step(-0.09999999999999999));
    assert!(!writes_back(0.0001) && !writes_back(-0.0001));
    assert!(writes_back(0.00010000000000000002) && writes_back(-0.00010000000000000002));
    // A first move of exactly 3 % is shaped (0.9), not an emergency.
    assert_eq!(shaped(true, 0.0, &[0.03]), vec![0.027]);
}

#[test]
fn a_db_change_is_found_by_bisection() {
    assert_close(audio_for_db_change(0.85, 0.1), 0.852508544921875);
    assert_close(audio_for_db_change(0.85, -0.1), 0.847503662109375);
    assert_close(audio_for_db_change(0.2, 1.0), 0.208343505859375);
    assert_close(audio_for_db_change(0.5, -6.0), 0.360015869140625);
}

/// Runs a touch from `start` through the finger `deltas`.
fn shaped(enabled: bool, start: f64, deltas: &[f64]) -> Vec<f64> {
    let mut shaper = Shaper::new(enabled);
    shaper.start(start);
    deltas.iter().map(|d| shaper.move_by(*d)).collect()
}

#[track_caller]
fn assert_trace(start: f64, deltas: &[f64], expected: &[f64]) {
    let got = shaped(true, start, deltas);
    assert_eq!(got.len(), expected.len());
    for (i, (g, e)) in got.iter().zip(expected).enumerate() {
        assert!(close(*g, *e), "move {i}: {g} != {e} (all: {got:?})");
    }
}

// The reference traces: `fader_script.lua` 2.5.4's own
// `applyFirstMovementScaling`, run under Lua 5.4 with TouchOSC's relative
// fader (each value = the previous value + the finger's delta, clamped to
// 0..1; the script's result is written back when it differs by > 1e-4).

#[test]
fn a_small_first_move_is_forced_to_one_tenth_db_then_compensated() {
    assert_trace(
        0.7294,
        &[0.002, 0.004, 0.004, 0.004, 0.004, 0.004, 0.004],
        &[
            0.7335579835647315,
            0.7347579835647314,
            0.7367579835647314,
            0.7395579835647315,
            0.7426179835647314,
            0.7457157613425092,
            0.7488513168980647,
        ],
    );
    assert_trace(
        0.7294,
        &[-0.001, -0.005, -0.005, -0.005, -0.005, -0.005],
        &[
            0.7252188672575739,
            0.7237188672575738,
            0.7212188672575739,
            0.717718867257574,
            0.713893867257574,
            0.7100216450353517,
        ],
    );
}

#[test]
fn the_forced_first_step_is_one_tenth_db_and_the_reaction_scales_are_03_05_07() {
    let start = 0.7294;
    let got = shaped(true, start, &[0.002, 0.004, 0.004, 0.004]);
    let step_db = value2db(to_live(got[0])) - value2db(to_live(start));
    // The bisection finds the step within 1e-4 of Live's value (< 0.005 dB).
    assert!((step_db - 0.1).abs() < 0.005, "{step_db}");
    for (i, scale) in [0.3, 0.5, 0.7].iter().enumerate() {
        let moved = got[i + 1] - got[i];
        assert!(
            (moved - 0.004 * scale).abs() < 1e-12,
            "move {}: {moved}",
            i + 1
        );
    }
}

#[test]
fn a_first_move_that_its_scaling_leaves_short_of_one_tenth_db_is_forced() {
    // The raw first move is 0.105 dB, the 0.9-scaled one 0.095 dB: forced to
    // 0.1 dB, with no reaction moves after it (the speed then grows 0.9→1.0).
    assert_trace(
        0.7294,
        &[0.0044, 0.004, 0.004, 0.004],
        &[
            0.7335579835647315,
            0.7366557613425092,
            0.7397913168980648,
            0.7429646502313981,
        ],
    );
    assert_trace(
        0.7294,
        &[-0.0044, -0.004, -0.004, -0.004],
        &[
            0.7252188672575739,
            0.7221210894797961,
            0.7189855339242406,
            0.7158122005909072,
        ],
    );
    // The same at −6 dB, far from 0 dB: the change is the difference to the
    // start's level (+0.106 dB raw, +0.096 dB scaled).
    assert_trace(
        0.5,
        &[0.0037, 0.004, 0.004, 0.004],
        &[
            0.5034855318101449,
            0.5065833095879226,
            0.5097188651434782,
            0.5128921984768116,
        ],
    );
    assert_trace(
        0.5,
        &[-0.0037, -0.004, -0.004, -0.004],
        &[
            0.49654184077695207,
            0.4928973963325076,
            0.4892085074436187,
            0.4854751741102854,
        ],
    );
}

#[test]
fn a_large_move_passes_unscaled() {
    assert_trace(
        0.5,
        &[0.05, 0.01, 0.01, 0.005],
        &[
            0.55,
            0.5576500000000001,
            0.5653944444444445,
            0.569313888888889,
        ],
    );
    assert_trace(
        0.3,
        &[0.001, 0.003, 0.003, 0.003, 0.04, 0.02, 0.01, 0.003, 0.003],
        &[
            0.30273073174101717,
            0.3036307317410172,
            0.3051307317410172,
            0.30723073174101717,
            0.34723073174101715,
            0.36723073174101717,
            0.3762307317410172,
            0.3789640650743505,
            0.3817307317410172,
        ],
    );
}

#[test]
fn the_speed_grows_from_09_to_10_over_the_first_moves_then_is_raw() {
    assert_trace(
        0.5,
        &[
            0.02, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01, 0.01,
        ],
        &[
            0.518,
            0.5257444444444445,
            0.5335833333333334,
            0.5415166666666668,
            0.5495444444444445,
            0.5576666666666668,
            0.5658833333333334,
            0.5741944444444446,
            0.5826000000000001,
            0.5911000000000002,
            0.5996944444444446,
            0.6096944444444446,
            0.6196944444444447,
        ],
    );
    // Below the linear range (Live value < 0.7) there is no 0.85 factor.
    assert_trace(
        0.1,
        &[0.003; 16],
        &[
            0.10270000000000001,
            0.10543333333333335,
            0.10820000000000002,
            0.11100000000000002,
            0.11383333333333336,
            0.11670000000000003,
            0.11960000000000003,
            0.12260000000000003,
            0.12560000000000002,
            0.12860000000000002,
            0.13160000000000002,
            0.13460000000000003,
            0.13760000000000003,
            0.14060000000000003,
            0.14360000000000003,
            0.14660000000000004,
        ],
    );
}

#[test]
fn the_value_is_clamped_before_the_move_is_measured() {
    assert_trace(
        0.99,
        &[0.001, 0.02, 0.02, 0.02],
        &[
            0.9948510488324502,
            0.9963957341827152,
            0.9981978670913576,
            0.9994593601274073,
        ],
    );
}

#[test]
fn a_reshape_under_1e_4_leaves_touchoscs_value_raw() {
    assert_trace(
        0.2,
        &[0.02, 0.0002, 0.0002, 0.0002, 0.0002],
        &[
            0.218,
            0.2182,
            0.2184,
            0.21860000000000002,
            0.21880000000000002,
        ],
    );
}

#[test]
fn no_move_changes_nothing_and_shaping_off_is_raw() {
    assert_eq!(shaped(true, 0.4, &[0.0, 0.0]), vec![0.4, 0.4]);
    assert_eq!(
        shaped(false, 0.5, &[0.002, 0.004, 0.6, -2.0]),
        vec![0.502, 0.506, 1.0, 0.0]
    );
    // A new touch starts over: the first move is forced again.
    let mut shaper = Shaper::new(true);
    shaper.start(0.7294);
    shaper.move_by(0.05);
    shaper.start(0.7294);
    assert_close(shaper.move_by(0.002), 0.7335579835647315);
    // A start outside 0..1 is clamped.
    shaper.start(1.4);
    assert_eq!(shaper.move_by(0.0), 1.0);
}

fn tap(tracker: &mut TapTracker, at: f64, held: f64) -> bool {
    tracker.down(0.5, at);
    tracker.up(0.5, at + held)
}

#[test]
fn two_taps_50_to_250_ms_apart_are_a_double_tap() {
    for (gap, double) in [
        (60.0, true),
        (249.0, true),
        (51.0, true),
        (300.0, false),
        (250.0, false),
        (50.0, false),
        (40.0, false),
    ] {
        let mut t = TapTracker::default();
        assert!(!tap(&mut t, 0.0, 10.0), "a first tap is single");
        assert_eq!(tap(&mut t, gap, 10.0), double, "gap {gap}");
    }
}

#[test]
fn a_double_tap_resets_and_a_non_tap_forgets_the_last_tap() {
    let mut t = TapTracker::default();
    assert!(!tap(&mut t, 0.0, 10.0));
    assert!(tap(&mut t, 60.0, 10.0));
    assert!(!tap(&mut t, 130.0, 10.0), "the third tap starts over");
    assert!(tap(&mut t, 200.0, 10.0));
    // A long press between two taps is not a tap and forgets the first.
    let mut t = TapTracker::default();
    assert!(!tap(&mut t, 0.0, 10.0));
    assert!(!tap(&mut t, 20.0, 200.0));
    assert!(!tap(&mut t, 230.0, 10.0));
}

#[test]
fn a_tap_is_short_and_still() {
    let mut t = TapTracker::default();
    t.down(0.5, 0.0);
    assert!(!t.up(0.5, 199.0) && t.last_tap.is_some(), "199 ms is a tap");
    let mut t = TapTracker::default();
    t.down(0.5, 0.0);
    assert!(!t.up(0.5, 200.0) && t.last_tap.is_none(), "200 ms is not");
    let mut t = TapTracker::default();
    t.down(0.5, 0.0);
    assert!(
        !t.up(0.5099, 10.0) && t.last_tap.is_some(),
        "a 0.0099 move is"
    );
    let mut t = TapTracker::default();
    t.down(0.0, 0.0);
    assert!(
        !t.up(0.01, 10.0) && t.last_tap.is_none(),
        "exactly 0.01 is not"
    );
    let mut t = TapTracker::default();
    t.down(0.5, 0.0);
    assert!(
        !t.up(0.51, 10.0) && t.last_tap.is_none(),
        "a 0.01 move is not"
    );
    let mut t = TapTracker::default();
    t.down(0.5, 0.0);
    assert!(!t.up(0.49, 10.0) && t.last_tap.is_none(), "down counts too");
}

#[test]
fn a_finger_that_strayed_must_rest_100_ms_before_the_release() {
    // Strayed 0.02 away and came back: a tap only after 100 ms of rest.
    for (release, is_tap) in [(111.0, true), (110.0, false), (60.0, false)] {
        let mut t = TapTracker::default();
        t.down(0.5, 0.0);
        t.moved(0.52, 10.0);
        t.moved(0.5, 20.0);
        t.up(0.5, release);
        assert_eq!(t.last_tap.is_some(), is_tap, "release at {release}");
    }
    // 0.015 exactly is not a stray: a quick release is still a tap.
    let mut t = TapTracker::default();
    t.down(0.0, 0.0);
    t.moved(0.015, 10.0);
    t.moved(0.0, 12.0);
    t.up(0.0, 20.0);
    assert!(t.last_tap.is_some());
    let mut t = TapTracker::default();
    t.down(0.5, 0.0);
    t.moved(0.4849, 10.0);
    t.moved(0.5, 12.0);
    t.up(0.5, 20.0);
    assert!(t.last_tap.is_none(), "a stray down counts");
}

#[test]
fn the_glide_reaches_0_db_at_0_3_positions_per_second() {
    let target = to_pos(UNITY);
    let glide = Glide::new(0.0, target, 1000.0);
    assert_eq!(glide.at(1000.0), (0.0, false));
    assert_eq!(glide.at(900.0), (0.0, false), "never before its start");
    let (p, done) = glide.at(2000.0);
    assert!(close(p, 0.3) && !done, "{p}");
    let (p, done) = glide.at(1000.0 + 2431.0);
    assert!(!done && (p - 0.7293).abs() < 1e-12, "{p}");
    assert_eq!(glide.at(1000.0 + 2432.0), (target, true));
    // Arriving exactly on time counts as arrived.
    assert_eq!(Glide::new(0.0, 0.3, 0.0).at(1000.0), (0.3, true));
    // Downwards, and a glide with nowhere to go.
    let (p, done) = Glide::new(1.0, target, 0.0).at(500.0);
    assert!(close(p, 0.85) && !done, "{p}");
    assert_eq!(Glide::new(target, target, 0.0).at(0.0), (target, true));
}

#[test]
fn the_post_release_hold_follows_the_shaping_switch() {
    assert_eq!(hold_ms(true), 1000.0);
    assert_eq!(hold_ms(false), 100.0);
}

const TRAVEL: f64 = 100.0;

/// Where a double tap glides: 0 dB.
fn zero_db() -> Option<f64> {
    Some(to_pos(UNITY))
}

/// A plain (shaping off) fader touched by pointer 1 at y = 500 at t = 0,
/// Live's value sitting at position `live`.
fn touched(live: f64) -> FaderCtl {
    let mut f = FaderCtl::new(false, zero_db());
    assert!(f.down(1, 500.0, TRAVEL, 0.0, live));
    f
}

/// The touch's first pointer move, at the down's own coordinate: it only
/// anchors the drag (#43 PR F), so the moves after it are the drag.
#[track_caller]
fn anchor(f: &mut FaderCtl, id: i32, y: f64, now: f64) {
    assert!(f.moved(id, y, now), "pointer {id} drives the fader");
}

#[test]
fn a_fader_moves_with_its_finger_and_sends_once_per_frame() {
    let start = to_pos(UNITY);
    let mut f = touched(start);
    assert_eq!(
        f.frame(1.0, Some(start)),
        Motion {
            pos: Some(start),
            send: None
        },
        "nothing moved yet"
    );
    anchor(&mut f, 1, 500.0, 5.0);
    assert!(f.moved(1, 490.0, 10.0));
    assert!(f.moved(1, 480.0, 12.0));
    let frame = f.frame(16.0, Some(start));
    assert_close(frame.pos.unwrap(), start + 0.2);
    assert_close(frame.send.unwrap(), start + 0.2);
    assert_eq!(f.frame(32.0, Some(start)).send, None, "sent once");
    // Moving down, a finger below the fader's travel stops at 0.
    assert!(f.moved(1, 2000.0, 40.0));
    assert_eq!(f.frame(48.0, Some(start)).send, Some(0.0));
}

#[test]
fn a_touched_fader_shows_the_finger_then_live_after_the_hold() {
    // Spec I4: while touched, Live's echo is not shown.
    let mut f = FaderCtl::new(true, zero_db());
    assert!(f.down(1, 500.0, TRAVEL, 0.0, 0.25));
    anchor(&mut f, 1, 500.0, 5.0);
    assert!(f.moved(1, 450.0, 10.0));
    let finger = f.frame(16.0, Some(0.1)).pos.unwrap();
    assert_close(finger, 0.75);
    assert_eq!(f.up(1, 100.0), None, "the last move went in the frame");
    assert_eq!(
        f.frame(1099.0, Some(0.1)).pos,
        Some(finger),
        "shaping: 1 s hold"
    );
    assert_eq!(f.frame(1100.0, Some(0.1)).pos, Some(0.1));
    let mut f = touched(0.25);
    anchor(&mut f, 1, 500.0, 5.0);
    f.moved(1, 450.0, 10.0);
    let finger = f.frame(16.0, Some(0.1)).pos.unwrap();
    assert!(f.up(1, 100.0).is_none());
    assert_eq!(f.frame(199.0, Some(0.1)).pos, Some(finger), "plain: 100 ms");
    assert_eq!(f.frame(200.0, Some(0.1)).pos, Some(0.1));
}

#[test]
fn the_release_sends_what_no_frame_sent() {
    let mut f = touched(0.5);
    anchor(&mut f, 1, 500.0, 5.0);
    f.moved(1, 480.0, 10.0);
    let sent = f.up(1, 20.0).expect("an unsent move");
    assert_close(sent, 0.7);
    assert_eq!(f.frame(30.0, Some(0.5)).send, None);
    let mut f = touched(0.5);
    anchor(&mut f, 1, 500.0, 5.0);
    f.moved(1, 450.0, 10.0);
    assert_eq!(f.cancel(1, 20.0), Some(1.0), "a cancel sends it too");
}

#[test]
fn a_second_pointer_does_not_move_a_held_fader() {
    let mut f = touched(0.5);
    assert!(!f.down(2, 100.0, TRAVEL, 5.0, 0.5));
    assert!(!f.moved(2, 50.0, 10.0));
    assert_eq!(f.up(2, 20.0), None);
    assert_eq!(f.cancel(2, 20.0), None);
    assert_eq!(f.frame(30.0, Some(0.5)).pos, Some(0.5));
    // Pointer 1 still drives it.
    anchor(&mut f, 1, 500.0, 35.0);
    assert!(f.moved(1, 490.0, 40.0));
    assert_close(f.frame(50.0, Some(0.5)).pos.unwrap(), 0.6);
    // Once released, another pointer may take it.
    f.up(1, 60.0);
    assert!(f.down(2, 100.0, TRAVEL, 70.0, 0.5));
}

#[test]
fn a_double_tap_glides_to_0_db_sending_each_frame() {
    let mut f = FaderCtl::new(false, zero_db());
    let live = 0.25;
    assert!(f.down(1, 500.0, TRAVEL, 0.0, live));
    assert_eq!(f.up(1, 20.0), None);
    assert!(f.down(1, 500.0, TRAVEL, 60.0, live));
    assert_eq!(f.up(1, 80.0), None);
    let frame = f.frame(1080.0, Some(live));
    assert_close(frame.pos.unwrap(), live + 0.3);
    assert_close(frame.send.unwrap(), live + 0.3);
    let target = to_pos(UNITY);
    let arrive = 80.0 + (target - live) / GLIDE_SPEED * 1000.0 + 1.0;
    let frame = f.frame(arrive, Some(live));
    assert_eq!(frame.pos, Some(target));
    assert_eq!(frame.send, Some(target));
    // Then the plain hold (100 ms) before Live's value shows again.
    assert_eq!(f.frame(arrive + 99.0, Some(live)).pos, Some(target));
    assert_eq!(f.frame(arrive + 99.0, Some(live)).send, None);
    assert_eq!(f.frame(arrive + 100.0, Some(target)).pos, Some(target));
    assert_eq!(f.frame(arrive + 101.0, Some(live)).pos, Some(live));
}

#[test]
fn a_glide_stops_where_it_is_when_lives_value_goes() {
    let mut f = FaderCtl::new(false, zero_db());
    let live = 0.25;
    assert!(f.down(1, 500.0, TRAVEL, 0.0, live));
    assert_eq!(f.up(1, 20.0), None);
    assert!(f.down(1, 500.0, TRAVEL, 60.0, live));
    assert_eq!(f.up(1, 80.0), None);
    let gliding = f.frame(1080.0, Some(live));
    assert_close(gliding.send.unwrap(), live + 0.3);
    // The instance went offline: no more sends, the fader holds.
    let stopped = f.frame(1100.0, None);
    assert_close(stopped.pos.unwrap(), live + 0.3);
    assert_eq!(stopped.send, None);
    let held = f.frame(1199.0, Some(live));
    assert_close(held.pos.unwrap(), live + 0.3);
    assert_eq!(held.send, None, "the glide does not resume");
    let after = f.frame(1200.0, Some(live));
    assert_eq!(after.pos, Some(live), "then Live's value shows");
    assert_eq!(after.send, None);
}

#[test]
fn without_a_glide_target_a_double_tap_does_nothing() {
    let mut f = FaderCtl::new(false, None);
    f.down(1, 500.0, TRAVEL, 0.0, 0.25);
    f.up(1, 20.0);
    f.down(1, 500.0, TRAVEL, 60.0, 0.25);
    f.up(1, 80.0);
    assert_eq!(f.frame(1080.0, Some(0.25)).send, None);
    assert_eq!(f.frame(1080.0, Some(0.25)).pos, Some(0.25));
}

#[test]
fn a_touch_stops_the_glide_where_it_is_and_a_cancel_never_taps() {
    let mut f = FaderCtl::new(false, zero_db());
    f.down(1, 500.0, TRAVEL, 0.0, 0.0);
    f.up(1, 10.0);
    f.down(1, 500.0, TRAVEL, 70.0, 0.0);
    f.up(1, 80.0);
    let gliding = f.frame(580.0, Some(0.0)).pos.unwrap();
    assert_close(gliding, 0.15);
    assert!(f.down(3, 200.0, TRAVEL, 590.0, 0.0));
    assert_eq!(f.frame(600.0, Some(0.0)).pos, Some(gliding), "no jump");
    anchor(&mut f, 3, 200.0, 605.0);
    assert!(f.moved(3, 190.0, 610.0));
    assert_close(f.frame(620.0, Some(0.0)).pos.unwrap(), gliding + 0.1);
    // Two cancelled taps start no glide.
    let mut f = FaderCtl::new(false, zero_db());
    f.down(1, 500.0, TRAVEL, 0.0, 0.5);
    f.cancel(1, 10.0);
    f.down(1, 500.0, TRAVEL, 70.0, 0.5);
    f.cancel(1, 80.0);
    assert_eq!(f.frame(1000.0, Some(0.5)).send, None);
}

#[test]
fn a_touch_during_the_hold_starts_from_the_shown_position() {
    let mut f = touched(0.25);
    anchor(&mut f, 1, 500.0, 5.0);
    f.moved(1, 400.0, 10.0);
    f.up(1, 20.0);
    let shown = f.frame(30.0, Some(0.1)).pos.unwrap();
    assert_eq!(shown, 1.0);
    assert!(f.down(1, 500.0, TRAVEL, 50.0, 0.1));
    assert_eq!(f.frame(60.0, Some(0.1)).pos, Some(shown));
    // After the hold, a touch starts from Live's value.
    f.up(1, 70.0);
    assert!(f.down(1, 500.0, TRAVEL, 500.0, 0.1));
    assert_eq!(f.frame(510.0, Some(0.2)).pos, Some(0.1));
}

#[test]
fn without_a_value_from_live_nothing_is_shown() {
    let mut f = FaderCtl::new(true, zero_db());
    assert_eq!(
        f.frame(0.0, None),
        Motion {
            pos: None,
            send: None
        }
    );
    assert_eq!(f.frame(0.0, Some(0.5)).pos, Some(0.5));
}

#[test]
fn a_zero_travel_counts_as_one_pixel() {
    let mut f = FaderCtl::new(false, None);
    f.down(1, 10.0, 0.0, 0.0, 0.0);
    anchor(&mut f, 1, 10.0, 0.5);
    f.moved(1, 9.9, 1.0);
    assert_close(f.frame(2.0, None).pos.unwrap(), 0.1);
}

#[test]
fn while_its_write_is_open_the_fader_shows_its_own_position_not_lives() {
    // L3: released inside a stall, its write not yet acked.
    let mut f = touched(0.25);
    f.intent(true, 5.0);
    anchor(&mut f, 1, 500.0, 7.0);
    f.moved(1, 450.0, 10.0);
    let finger = f.frame(16.0, Some(0.25)).pos.unwrap();
    assert_close(finger, 0.75);
    assert_eq!(f.up(1, 20.0), None);
    // The plain hold (100 ms) is long over, but the write is open.
    assert_eq!(f.frame(500.0, Some(0.25)).pos, Some(finger));
    f.intent(true, 1_000.0);
    let late = f.frame(5_000.0, Some(0.1));
    assert_eq!(late.pos, Some(finger), "Live's value only moves the ghost");
    assert_eq!(late.send, None);
    // The ack closes it: the hold runs again from then.
    f.intent(false, 6_000.0);
    assert_eq!(f.frame(6_099.0, Some(0.1)).pos, Some(finger));
    assert_eq!(
        f.frame(6_100.0, Some(0.75)).pos,
        Some(0.75),
        "then Live's value"
    );
    // A write that was never open starts no hold when it is not.
    f.intent(false, 7_000.0);
    assert_eq!(f.frame(7_001.0, Some(0.3)).pos, Some(0.3));
    // With shaping, the hold after the close is a second.
    let mut f = FaderCtl::new(true, zero_db());
    assert!(f.down(1, 500.0, TRAVEL, 0.0, 0.25));
    f.intent(true, 1.0);
    f.up(1, 10.0);
    f.intent(false, 3_000.0);
    assert_eq!(f.frame(3_999.0, Some(0.1)).pos, Some(0.25));
    assert_eq!(f.frame(4_000.0, Some(0.1)).pos, Some(0.1));
}

#[test]
fn a_touch_while_its_write_is_open_starts_from_the_cap() {
    let mut f = touched(0.25);
    anchor(&mut f, 1, 500.0, 5.0);
    f.moved(1, 450.0, 10.0);
    let cap = f.frame(16.0, Some(0.25)).pos.unwrap();
    f.up(1, 20.0);
    // Its release was not sent (L4): long after the hold the cap stays.
    f.intent(true, 30.0);
    assert!(f.down(1, 500.0, TRAVEL, 5_000.0, 0.25));
    assert_eq!(f.frame(5_001.0, Some(0.25)).pos, Some(cap));
    anchor(&mut f, 1, 500.0, 5_005.0);
    f.moved(1, 490.0, 5_010.0);
    assert_close(f.frame(5_016.0, Some(0.25)).pos.unwrap(), cap + 0.1);
}

#[test]
fn the_end_of_a_glide_is_told_once() {
    let live = 0.25;
    let double_tapped = || {
        let mut f = FaderCtl::new(false, zero_db());
        f.down(1, 500.0, TRAVEL, 0.0, live);
        f.up(1, 20.0);
        f.down(1, 500.0, TRAVEL, 60.0, live);
        f.up(1, 80.0);
        f
    };
    let mut f = double_tapped();
    assert!(!f.take_ended(), "a release is told by the control itself");
    f.frame(1080.0, Some(live));
    assert!(!f.take_ended(), "still gliding");
    let arrive = 80.0 + (to_pos(UNITY) - live) / GLIDE_SPEED * 1000.0 + 1.0;
    assert_eq!(f.frame(arrive, Some(live)).send, Some(to_pos(UNITY)));
    assert!(f.take_ended(), "arrived");
    assert!(!f.take_ended(), "once");
    f.frame(arrive + 10.0, Some(live));
    assert!(!f.take_ended());
    // Stopped because Live's value went: ended too.
    let mut f = double_tapped();
    f.frame(1080.0, Some(live));
    f.frame(1100.0, None);
    assert!(f.take_ended());
    // Stopped by a touch: the touch's own release tells.
    let mut f = double_tapped();
    f.frame(1080.0, Some(live));
    assert!(f.down(2, 300.0, TRAVEL, 1090.0, live));
    f.frame(1100.0, Some(live));
    assert!(!f.take_ended());
}

#[test]
fn the_fader_says_which_pointer_drives_it() {
    let mut f = FaderCtl::new(false, None);
    assert!(!f.drives(1), "untouched");
    assert!(f.down(1, 500.0, TRAVEL, 0.0, 0.5));
    assert!(f.drives(1) && !f.drives(2));
    assert_eq!(f.up(2, 5.0), None);
    assert!(f.drives(1), "another pointer's release is not its");
    assert_eq!(f.up(1, 10.0), None, "nothing unsent");
    assert!(!f.drives(1), "released");
    assert!(f.down(3, 500.0, TRAVEL, 500.0, 0.5));
    f.cancel(3, 510.0);
    assert!(!f.drives(3), "cancelled");
}

#[test]
fn a_fader_built_while_its_write_is_open_shows_the_write_and_a_touch_starts_there() {
    // A page switch rebuilt it (position 0) while its write waits.
    let mut f = FaderCtl::new(false, zero_db());
    f.intent(true, 0.0);
    f.write_at(0.7);
    assert_eq!(f.frame(1.0, Some(0.25)).pos, Some(0.7), "the write, not 0");
    assert!(f.down(1, 500.0, TRAVEL, 10.0, 0.25));
    // Under a finger the fader follows the finger, not the write.
    f.write_at(0.2);
    assert_eq!(f.frame(11.0, Some(0.25)).pos, Some(0.7));
    anchor(&mut f, 1, 500.0, 15.0);
    f.moved(1, 490.0, 20.0);
    assert_close(f.frame(30.0, Some(0.25)).pos.unwrap(), 0.8);
    f.up(1, 40.0);
    // Gliding, the glide drives it.
    let mut f = FaderCtl::new(false, zero_db());
    f.down(1, 500.0, TRAVEL, 0.0, 0.25);
    f.up(1, 20.0);
    f.down(1, 500.0, TRAVEL, 60.0, 0.25);
    f.up(1, 80.0);
    f.write_at(0.9);
    assert_close(f.frame(1080.0, Some(0.25)).pos.unwrap(), 0.55);
}

#[test]
fn a_press_says_where_the_touch_started() {
    let mut f = FaderCtl::new(false, zero_db());
    let _ = f.frame(0.0, Some(0.25));
    // Not local: the touch starts from Live's position, whatever the fader
    // showed.
    let start = |shown, live, local, from| {
        Some(Start {
            shown,
            live,
            local,
            from,
        })
    };
    assert_eq!(
        f.press(1, 500.0, TRAVEL, 10.0, 0.5),
        start(0.25, 0.5, false, 0.5)
    );
    assert_eq!(
        f.press(2, 400.0, TRAVEL, 11.0, 0.5),
        None,
        "another pointer"
    );
    anchor(&mut f, 1, 500.0, 11.5);
    assert!(f.moved(1, 487.5, 12.0));
    assert_eq!(f.frame(16.0, Some(0.5)).send, Some(0.625));
    assert_eq!(f.up(1, 20.0), None);
    // Within the hold the touch starts from the fader's own position.
    assert_eq!(
        f.press(1, 500.0, TRAVEL, 119.0, 0.5),
        start(0.625, 0.5, true, 0.625)
    );
    assert_eq!(f.cancel(1, 130.0), None);
    // After the hold, from Live's again.
    let _ = f.frame(231.0, Some(0.375));
    assert_eq!(
        f.press(1, 500.0, TRAVEL, 232.0, 0.25),
        start(0.375, 0.25, false, 0.25)
    );
    assert_eq!(f.cancel(1, 240.0), None);
    // An open write keeps it local (L3).
    let _ = f.frame(400.0, Some(0.25));
    f.intent(true, 401.0);
    assert_eq!(
        f.press(1, 500.0, TRAVEL, 402.0, 0.75),
        start(0.25, 0.75, true, 0.25)
    );
}
