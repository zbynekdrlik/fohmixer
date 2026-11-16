//! The dB readout (#21, parity audit #13): Live's own value (spec X1, never
//! the UI's arithmetic) in TouchOSC's form: one decimal, no unit, −∞ for
//! silence, white exactly at 0 dB and light green otherwise. A strip with a
//! home level (#225, `+HOME:<dB>`) is white exactly at its home instead.

/// What the readout shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbText {
    pub text: String,
    /// Live shows exactly 0 dB, or the strip's home level (the white
    /// readout).
    pub unity: bool,
}

/// Half of the last digit of Live's display string (two decimals): a level
/// within it of the home reads as the home exactly.
const HALF_DIGIT: f64 = 0.005;

/// The readout of Live's display string (`"-5.996 dB"`, `"0.00 dB"`,
/// `"-inf dB"`); `None` for no string or one that is not a level.
pub fn db_text(display: &str) -> Option<DbText> {
    db_text_at(display, 0.0)
}

/// The readout of Live's display string on a strip whose home level is
/// `home` dB (#225): white exactly there (to Live's two decimals).
pub fn db_text_at(display: &str, home: f64) -> Option<DbText> {
    let number = display.trim().trim_end_matches("dB").trim();
    if number.eq_ignore_ascii_case("-inf") {
        return Some(DbText {
            text: "−∞".to_string(),
            unity: false,
        });
    }
    let db: f64 = number.parse().ok()?;
    let rounded = (db * 10.0).round() / 10.0;
    // A rounded −0 reads "0.0", not "-0.0" (−0 == 0).
    let tenths = if rounded == 0.0 { 0.0 } else { rounded };
    let text = if tenths < 0.0 {
        format!("−{:.1}", -tenths)
    } else {
        format!("{tenths:.1}")
    };
    Some(DbText {
        text,
        unity: (db - home).abs() < HALF_DIGIT,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(display: &str) -> Option<(String, bool)> {
        db_text(display).map(|d| (d.text, d.unity))
    }

    #[test]
    fn a_level_shows_one_decimal_without_its_unit() {
        assert_eq!(shown("-5.996 dB"), Some(("−6.0".into(), false)));
        assert_eq!(shown("-0.811 dB"), Some(("−0.8".into(), false)));
        assert_eq!(shown("-41.383 dB"), Some(("−41.4".into(), false)));
        assert_eq!(shown("6.00 dB"), Some(("6.0".into(), false)));
        assert_eq!(shown("1.25 dB"), Some(("1.3".into(), false)));
        assert_eq!(shown(" -12.0dB "), Some(("−12.0".into(), false)));
    }

    #[test]
    fn exactly_0_db_is_the_unity_readout() {
        assert_eq!(shown("0.00 dB"), Some(("0.0".into(), true)));
        assert_eq!(shown("0 dB"), Some(("0.0".into(), true)));
        // Close to 0 but not 0: rounds to 0.0 and stays green.
        assert_eq!(shown("-0.04 dB"), Some(("0.0".into(), false)));
        assert_eq!(shown("0.04 dB"), Some(("0.0".into(), false)));
    }

    #[test]
    fn a_home_strip_is_white_exactly_at_its_home() {
        let at = |display: &str| db_text_at(display, -6.0).map(|d| (d.text, d.unity));
        assert_eq!(at("-6.00 dB"), Some(("−6.0".into(), true)));
        assert_eq!(at("-5.996 dB"), Some(("−6.0".into(), true)));
        assert_eq!(at("-6.004 dB"), Some(("−6.0".into(), true)));
        assert_eq!(at("-5.99 dB"), Some(("−6.0".into(), false)));
        assert_eq!(at("-6.01 dB"), Some(("−6.0".into(), false)));
        assert_eq!(
            at("0.00 dB"),
            Some(("0.0".into(), false)),
            "0 dB is no home there"
        );
        assert_eq!(at("-inf dB"), Some(("−∞".into(), false)));
        let half = db_text_at("-2.50 dB", -2.5).unwrap();
        assert!(half.unity);
        // Half a digit away is no longer the home (the bound is strict).
        assert!(!db_text_at("0.005 dB", 0.0).unwrap().unity);
        assert!(db_text_at("0.0049 dB", 0.0).unwrap().unity);
    }

    #[test]
    fn silence_is_minus_infinity_and_no_level_is_nothing() {
        assert_eq!(shown("-inf dB"), Some(("−∞".into(), false)));
        assert_eq!(shown("-INF dB"), Some(("−∞".into(), false)));
        assert_eq!(shown(""), None);
        assert_eq!(shown("C"), None);
        assert_eq!(shown("50 %"), None);
    }
}
