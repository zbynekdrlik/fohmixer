use super::*;
use crate::volume::UNITY;

/// Probe P2's points (#151, 27.10): a 1 kHz sine's peak in dBFS and Live's
/// meter value on a track at 0 dB, pan centre.
const P2: [(f64, f64); 9] = [
    (0.0, 0.9211),
    (-3.0, 0.8816),
    (-6.0, 0.8421),
    (-12.0, 0.7632),
    (-18.0, 0.6842),
    (-24.0, 0.6053),
    (-36.0, 0.4474),
    (-48.0, 0.2895),
    (-60.0, 0.1316),
];

fn close(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() <= eps
}

#[test]
fn scale_points_round_trip() {
    for (db, value) in P2 {
        let got = live_value(db);
        assert!(close(got, value, 0.001), "{db} dB: {got} against {value}");
    }
}

#[test]
fn between_points_is_monotonic() {
    let mut last = live_value(-70.0);
    for step in -699..=60 {
        let now = live_value(f64::from(step) / 10.0);
        assert!(now > last, "{step} tenths: {now} after {last}");
        last = now;
    }
}

#[test]
fn below_the_lowest_is_zero_and_the_top_is_one() {
    assert_eq!(live_value(-70.0), 0.0);
    assert_eq!(live_value(-80.0), 0.0);
    assert_eq!(live_value(f64::NEG_INFINITY), 0.0);
    assert_eq!(live_value(f64::NAN), 0.0);
    assert_eq!(live_value(6.0), 1.0);
    assert_eq!(live_value(12.0), 1.0);
    assert!(close(live_value(-69.0), 1.0 / 76.0, 1e-12));
    assert!(close(live_value(5.0), 75.0 / 76.0, 1e-12));
}

#[test]
fn pan_centre_and_extremes() {
    let (l, r) = pan_gains_db(0.0);
    assert!(close(l, 0.0, 1e-9) && close(r, 0.0, 1e-9), "{l} {r}");
    let (l, r) = pan_gains_db(-1.0);
    assert!(close(l, 3.0103, 0.001), "{l}");
    assert_eq!(r, f64::NEG_INFINITY);
    let (l, r) = pan_gains_db(1.0);
    assert_eq!(l, f64::NEG_INFINITY);
    assert!(close(r, 3.0103, 0.001), "{r}");
    // P2's half-way points, mirrored.
    let (l, r) = pan_gains_db(-0.5);
    assert!(close(l, 2.32, 0.01) && close(r, -5.33, 0.01), "{l} {r}");
    let (l, r) = pan_gains_db(0.5);
    assert!(close(l, -5.33, 0.01) && close(r, 2.32, 0.01), "{l} {r}");
    // Out of range is clamped, NaN is the centre.
    assert_eq!(pan_gains_db(-3.0), pan_gains_db(-1.0));
    assert_eq!(pan_gains_db(2.0), pan_gains_db(1.0));
    assert_eq!(pan_gains_db(f64::NAN), pan_gains_db(0.0));
}

#[test]
fn muted_is_zero() {
    assert_eq!(post_fader(-60, -60, UNITY, 0.0, true), (0.0, 0.0));
}

#[test]
fn fader_at_unity_adds_nothing() {
    assert!(close(value2db(UNITY), 0.0, 1e-9));
    let (l, r) = post_fader(-120, -180, UNITY, 0.0, false);
    assert!(close(l, live_value(-12.0), 1e-9), "{l}");
    assert!(close(r, live_value(-18.0), 1e-9), "{r}");
}

#[test]
fn the_fader_and_the_pan_add_their_db() {
    // 10 dB down: 40·v − 34 = −10 at v = 0.6.
    let (l, r) = post_fader(-120, -120, 0.6, 0.0, false);
    assert!(close(l, live_value(-22.0), 1e-9), "{l}");
    assert!(close(r, live_value(-22.0), 1e-9), "{r}");
    let (gl, gr) = pan_gains_db(-0.5);
    let (l, r) = post_fader(-120, -120, UNITY, -0.5, false);
    assert!(close(l, live_value(-12.0 + gl), 1e-9), "{l}");
    assert!(close(r, live_value(-12.0 + gr), 1e-9), "{r}");
    // Hard left: the right side reads nothing.
    assert_eq!(post_fader(-120, -120, UNITY, -1.0, false).1, 0.0);
}

#[test]
fn the_devices_silence_stays_silent() {
    // −70 dB from the device plus +6 dB of fader and +3 dB of pan is still
    // silence, never 9 dB above the floor.
    assert_eq!(post_fader(-700, -700, 1.0, -1.0, false), (0.0, 0.0));
    assert_eq!(post_fader(-701, -699, 1.0, 0.0, false).0, 0.0);
    assert!(post_fader(-701, -699, 1.0, 0.0, false).1 > 0.0);
    // A fader at −∞ silences both sides.
    assert_eq!(post_fader(0, 0, 0.0, 0.0, false), (0.0, 0.0));
}
