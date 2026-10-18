//! Live's volume in dB (#130, D18; moved from fohmixer-ui, where the strips'
//! scale, the backing-tracks fader and the touch shaping use it):
//! TouchOSC's `value2db` fit of Live's taper (0 dB at 0.85, −20 dB at 0.36,
//! −40 dB at 0.16, spec F8), the dB change found by the Lua's bisection, and
//! a dB's volume with the +6 dB top and the −70 dB floor. The hub's REVERB
//! rule computes its 10 dB on it; nothing else in the hub does dB maths
//! (§2.4).

/// Live's volume at 0 dB: the double-tap target.
pub const UNITY: f64 = 0.85;
/// The law's top: Live's volume 1.0 reads +6 dB.
pub const TOP_DB: f64 = 6.0;
/// The law's floor: at or below it [`value2db`] reads −∞ (volume 0).
pub const FLOOR_DB: f64 = -70.0;
/// The bisection steps of `audio_for_db_change`: the Lua halves [0, 1]
/// until it is at most 1e-4 wide, which takes 14 halvings.
const BISECT_STEPS: u32 = 14;

/// TouchOSC's `value2db`, the dB approximation of a Live volume. The touch
/// shaping, the meter colour and the backing-tracks fader use it, and from
/// 1.6.0 the hub's REVERB rule; every dB text on screen is Live's own
/// display string (spec X1).
pub fn value2db(v: f64) -> f64 {
    // The Lua's ranges, tested from the top: above 1 (or no number) is 0 dB.
    if v > 1.0 || v.is_nan() {
        0.0
    } else if v >= 0.4 {
        40.0 * v - 34.0
    } else if v >= 0.15 {
        -((399.751894 * v - 201.871345).powi(2) + 12630.61132) / 799.503788
    } else {
        // Spelled as the Lua computes it (gamma = 7504/5567, then 1/gamma).
        let gamma = 7504.0 / 5567.0;
        let db = 118.426374 * v.powf(1.0 / gamma) - 70.0;
        if db <= -70.0 { f64::NEG_INFINITY } else { db }
    }
}

/// The Live volume `db_change` dB away from `start` (TouchOSC's
/// `dbChangeToAudioChange`: a bisection over 0..1).
pub fn audio_for_db_change(start: f64, db_change: f64) -> f64 {
    let target = value2db(start) + db_change;
    let (mut low, mut high) = (0.0_f64, 1.0_f64);
    for _ in 0..BISECT_STEPS {
        let mid = (low + high) / 2.0;
        if value2db(mid) < target {
            low = mid;
        } else {
            high = mid;
        }
    }
    (low + high) / 2.0
}

/// The Live volume of `db` on the strips' law: 1 at +6 dB and above, 0 at
/// the floor and below (−∞), else the volume `db` away from 0 dB.
pub fn volume_of(db: f64) -> f64 {
    if db >= TOP_DB {
        1.0
    } else if db > FLOOR_DB {
        audio_for_db_change(UNITY, db)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-12
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
        // Above 1 and no number read 0 dB, as the Lua's first test.
        assert_eq!(value2db(1.0_f64.next_up()), 0.0);
        assert_eq!(value2db(f64::NAN), 0.0);
        assert_eq!(value2db(0.4), -18.0);
        assert!(value2db(0.4_f64.next_down()) < -18.0);
        assert!(close(value2db(0.15), -40.986236048588935));
    }

    #[test]
    fn the_bisection_splits_at_an_exact_hit_like_the_lua() {
        // value2db(0.5) is exactly -14 dB: the first midpoint hits the
        // target and the Lua moves `high` (its test is `mid_db < target`).
        assert!(close(audio_for_db_change(0.5, 0.0), 0.499969482421875));
    }

    #[test]
    fn a_db_change_is_found_by_bisection() {
        assert!(close(audio_for_db_change(0.85, 0.1), 0.852508544921875));
        assert!(close(audio_for_db_change(0.85, -0.1), 0.847503662109375));
        assert!(close(audio_for_db_change(0.2, 1.0), 0.208343505859375));
        assert!(close(audio_for_db_change(0.5, -6.0), 0.360015869140625));
    }

    #[test]
    fn the_law_ends_are_its_own() {
        assert_eq!(UNITY, 0.85);
        assert_eq!(TOP_DB, 6.0);
        assert_eq!(FLOOR_DB, -70.0);
        assert_eq!(value2db(1.0), TOP_DB);
        assert!(value2db(1e-9) > FLOOR_DB, "only silence is below the floor");
    }

    #[test]
    fn a_db_maps_to_its_volume_with_both_ends_exact() {
        // The top: +6 dB and above is volume 1, a hair under it the
        // bisection's.
        assert_eq!(volume_of(6.0), 1.0);
        assert_eq!(volume_of(9.5), 1.0);
        assert_eq!(volume_of(f64::INFINITY), 1.0);
        assert_eq!(volume_of(6.0_f64.next_down()), 0.999969482421875);
        // The floor: −70 dB and below (−∞) is volume 0, a hair above it not.
        assert_eq!(volume_of(-70.0), 0.0);
        assert_eq!(volume_of(-85.0), 0.0);
        assert_eq!(volume_of(f64::NEG_INFINITY), 0.0);
        assert_eq!(volume_of(f64::NAN), 0.0);
        assert_eq!(volume_of((-70.0_f64).next_up()), 3.0517578125e-05);
        // In between, the strips' own bisection from 0 dB.
        assert_eq!(volume_of(0.0), 0.850006103515625);
        assert_eq!(volume_of(-14.0), 0.499969482421875);
        assert_eq!(volume_of(-20.0), 0.360015869140625);
        assert!((value2db(volume_of(-40.0)) + 40.0).abs() <= 0.01);
        assert!((value2db(volume_of(-60.0)) + 60.0).abs() <= 0.01);
    }
}
