//! The dB scale beside every fader (#21, parity audit #12): TouchOSC's labels
//! (0, −6, −12, −18, −24, −40, and +6 at the top) placed by the fader law
//! over Live's volume taper, as TouchOSC's `value2db` has it: a cap standing
//! at a label reads that dB. The meter shares the scale (its calibration
//! places Live's levels on the same positions).

use super::fader::{UNITY, audio_for_db_change, to_pos};

/// The labelled levels, top to bottom (dB).
pub const TICKS: [f64; 7] = [6.0, 0.0, -6.0, -12.0, -18.0, -24.0, -40.0];

/// The fader position of `db`: the Live volume `db` away from 0 dB (the
/// same bisection the fader's 0.1 dB steps use), on the fader law.
pub fn tick_pos(db: f64) -> f64 {
    to_pos(audio_for_db_change(UNITY, db))
}

/// Where the meter turns yellow and red (TouchOSC's meter colours, dB on the
/// fader scale).
pub const YELLOW_DB: f64 = -12.0;
pub const RED_DB: f64 = -3.0;

/// The meter's zone boundaries as CSS variables (percent of its height from
/// the bottom), for its gradient.
pub fn zone_style() -> String {
    format!(
        "--yellow:{:.2}%;--red:{:.2}%;",
        tick_pos(YELLOW_DB) * 100.0,
        tick_pos(RED_DB) * 100.0
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

    fn close(got: f64, want: f64, tolerance: f64) {
        assert!((got - want).abs() <= tolerance, "{got} is not {want}");
    }

    #[test]
    fn the_ticks_sit_where_the_fader_reads_their_db() {
        // 0 dB is the fader's unity; +6 dB its top; −6 dB its middle.
        close(tick_pos(0.0), to_pos(UNITY), 1e-4);
        close(tick_pos(0.0), 0.7294, 1e-3);
        close(tick_pos(6.0), 1.0, 1e-3);
        close(tick_pos(-6.0), 0.5, 2e-3);
        // Every tick's position reads back its dB through the fader law.
        for db in TICKS {
            let volume = crate::behave::fader::to_live(tick_pos(db));
            close(value2db(volume), db, 0.05);
        }
    }

    #[test]
    fn the_ticks_run_down_the_fader_in_order() {
        let positions: Vec<f64> = TICKS.iter().map(|db| tick_pos(*db)).collect();
        for pair in positions.windows(2) {
            assert!(pair[0] > pair[1], "{positions:?}");
        }
        assert!(positions[6] > 0.0, "−40 dB is above the bottom");
    }

    #[test]
    fn the_meter_zones_start_at_minus_12_and_minus_3_db() {
        let style = zone_style();
        assert!(style.starts_with("--yellow:31.3"), "{style}");
        assert!(style.contains(";--red:60.9"), "{style}");
        assert!(style.ends_with("%;"), "{style}");
    }

    #[test]
    fn a_label_carries_its_sign() {
        assert_eq!(tick_label(6.0), "+6");
        assert_eq!(tick_label(0.0), "0");
        assert_eq!(tick_label(-12.0), "−12");
        assert_eq!(TICKS.map(tick_label).join(" "), "+6 0 −6 −12 −18 −24 −40");
    }
}
