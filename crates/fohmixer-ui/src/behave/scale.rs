//! The dB scale beside every fader (#21, parity audit #12): labels placed
//! by the fader's volume law over Live's volume taper, as TouchOSC's
//! `value2db` has it: a cap standing at a label reads that dB. The meter
//! shares the scale (its calibration places Live's levels on the same
//! positions). Live's own law (#63) spreads the low end like a console's
//! fader, so its scale also labels −30 and −60.

use super::fader::{UNITY, VolumeLaw, audio_for_db_change};

/// The labelled levels on TouchOSC's law, top to bottom (dB).
pub const TICKS_TOUCHOSC: [f64; 7] = [6.0, 0.0, -6.0, -12.0, -18.0, -24.0, -40.0];
/// The labelled levels on Live's law, top to bottom (dB).
pub const TICKS_LIVE: [f64; 9] = [6.0, 0.0, -6.0, -12.0, -18.0, -24.0, -30.0, -40.0, -60.0];

/// The labelled levels of law `law`, top to bottom (dB).
pub fn ticks(law: VolumeLaw) -> &'static [f64] {
    match law {
        VolumeLaw::Live => &TICKS_LIVE,
        VolumeLaw::TouchOsc => &TICKS_TOUCHOSC,
    }
}

/// The fader position of `db` on law `law`: the Live volume `db` away from
/// 0 dB (the same bisection the fader's 0.1 dB steps use), on the law.
pub fn tick_pos(law: VolumeLaw, db: f64) -> f64 {
    law.to_pos(audio_for_db_change(UNITY, db))
}

/// Where the meter turns yellow and red (TouchOSC's meter colours, dB on the
/// fader scale).
pub const YELLOW_DB: f64 = -12.0;
pub const RED_DB: f64 = -3.0;

/// The meter's zone boundaries on law `law` as CSS variables (percent of its
/// height from the bottom), for its gradient.
pub fn zone_style(law: VolumeLaw) -> String {
    format!(
        "--yellow:{:.2}%;--red:{:.2}%;",
        tick_pos(law, YELLOW_DB) * 100.0,
        tick_pos(law, RED_DB) * 100.0
    )
}

/// A label's text: a sign on the positive ones, a real minus.
pub fn tick_label(db: f64) -> String {
    if db > 0.0 {
        format!("+{db}")
    } else if db < 0.0 {
        format!("−{}", -db)
    } else {
        "0".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::behave::fader::value2db;

    const LAWS: [VolumeLaw; 2] = [VolumeLaw::Live, VolumeLaw::TouchOsc];

    fn close(got: f64, want: f64, tolerance: f64) {
        assert!((got - want).abs() <= tolerance, "{got} is not {want}");
    }

    #[test]
    fn the_ticks_sit_where_the_fader_reads_their_db() {
        // TouchOSC's law: 0 dB is the fader's unity; +6 dB its top; −6 dB
        // its middle.
        let tosc = VolumeLaw::TouchOsc;
        close(tick_pos(tosc, 0.0), tosc.to_pos(UNITY), 1e-4);
        close(tick_pos(tosc, 0.0), 0.7294, 1e-3);
        close(tick_pos(tosc, 6.0), 1.0, 1e-3);
        close(tick_pos(tosc, -6.0), 0.5, 2e-3);
        // Live's law: the position is Live's volume (0 dB at 0.85, a dB
        // step of 6 every 0.15 above −18 dB, −40 dB at 0.157).
        let live = VolumeLaw::Live;
        close(tick_pos(live, 0.0), 0.85, 1e-4);
        close(tick_pos(live, 6.0), 1.0, 1e-3);
        close(tick_pos(live, -6.0), 0.70, 1e-3);
        close(tick_pos(live, -12.0), 0.55, 1e-3);
        close(tick_pos(live, -18.0), 0.40, 1e-3);
        close(tick_pos(live, -40.0), 0.157, 1e-3);
        close(tick_pos(live, -60.0), 0.0357, 1e-3);
        // Every tick's position reads back its dB through the law.
        for law in LAWS {
            for db in ticks(law) {
                let volume = law.to_live(tick_pos(law, *db));
                close(value2db(volume), *db, 0.05);
            }
        }
    }

    #[test]
    fn the_ticks_run_down_the_fader_in_order() {
        for law in LAWS {
            let positions: Vec<f64> = ticks(law).iter().map(|db| tick_pos(law, *db)).collect();
            for pair in positions.windows(2) {
                assert!(pair[0] > pair[1], "{law:?} {positions:?}");
            }
            assert!(
                positions[positions.len() - 1] > 0.0,
                "the lowest is above the bottom"
            );
        }
    }

    #[test]
    fn each_law_has_its_labels() {
        assert_eq!(ticks(VolumeLaw::Live), &TICKS_LIVE[..]);
        assert_eq!(ticks(VolumeLaw::TouchOsc), &TICKS_TOUCHOSC[..]);
    }

    #[test]
    fn the_meter_zones_start_at_minus_12_and_minus_3_db() {
        let style = zone_style(VolumeLaw::TouchOsc);
        assert!(style.starts_with("--yellow:31.3"), "{style}");
        assert!(style.contains(";--red:60.9"), "{style}");
        assert!(style.ends_with("%;"), "{style}");
        let style = zone_style(VolumeLaw::Live);
        assert!(style.starts_with("--yellow:55.0"), "{style}");
        assert!(style.contains(";--red:77.5"), "{style}");
    }

    #[test]
    fn a_label_carries_its_sign() {
        assert_eq!(tick_label(6.0), "+6");
        assert_eq!(tick_label(0.0), "0");
        assert_eq!(tick_label(-12.0), "−12");
        assert_eq!(
            TICKS_TOUCHOSC.map(tick_label).join(" "),
            "+6 0 −6 −12 −18 −24 −40"
        );
        assert_eq!(
            TICKS_LIVE.map(tick_label).join(" "),
            "+6 0 −6 −12 −18 −24 −30 −40 −60"
        );
    }
}
