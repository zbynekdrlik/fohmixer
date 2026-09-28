//! The dB readout (#21, parity audit #13): Live's own value (spec X1, never
//! the UI's arithmetic) in TouchOSC's form: one decimal, no unit, −∞ for
//! silence, white exactly at 0 dB and light green otherwise.

/// What the readout shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbText {
    pub text: String,
    /// Live shows exactly 0 dB (the white readout).
    pub unity: bool,
}

/// The readout of Live's display string (`"-5.996 dB"`, `"0.00 dB"`,
/// `"-inf dB"`); `None` for no string or one that is not a level.
pub fn db_text(display: &str) -> Option<DbText> {
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
        unity: db == 0.0,
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
    fn silence_is_minus_infinity_and_no_level_is_nothing() {
        assert_eq!(shown("-inf dB"), Some(("−∞".into(), false)));
        assert_eq!(shown("-INF dB"), Some(("−∞".into(), false)));
        assert_eq!(shown(""), None);
        assert_eq!(shown("C"), None);
        assert_eq!(shown("50 %"), None);
    }
}
