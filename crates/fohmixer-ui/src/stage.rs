//! Fitting a text into its box (#9): `dom::fit_text` measures the laid-out
//! text against its box and applies [`fitted_font`]; [`fit_key`] lets a
//! component measure again only when a text's shape changes. (The TouchOSC
//! canvas, frames and tab geometry that lived here went with schema 1, #21.)

/// What a strip text's fitted size depends on (#9): the text with every digit
/// as `0`. Digits are equally wide there (`tabular-nums`), so a dB value that
/// moves within its form ("-6.00 dB" to "-5.98 dB", a fader drag) keeps its
/// size and is not measured again: a measurement lays the page out, and one
/// per value during a drag starved the animation frames.
pub fn fit_key(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_ascii_digit() { '0' } else { c })
        .collect()
}

/// A CSS length in px (`"22px"`, a computed style), or `None` for another
/// unit or no number.
pub fn px_value(text: &str) -> Option<f64> {
    text.trim().strip_suffix("px")?.trim().parse().ok()
}

/// How much of its room a fitted text may take (rounding and hinting).
pub const FIT_MARGIN: f64 = 0.96;

/// The font size at which a text measured `text` (width, height) at `base`
/// fits `room` (width, height), both in the same px: `base` when it fits
/// with the margin, else scaled down, never up. A text or room of no size
/// keeps `base`.
pub fn fitted_font(base: f64, text: (f64, f64), room: (f64, f64)) -> f64 {
    let ratio = |t: f64, r: f64| (r * FIT_MARGIN / t).min(1.0);
    let scale = ratio(text.0, room.0).min(ratio(text.1, room.1));
    if scale > 0.0 { base * scale } else { base }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(got: f64, want: f64) {
        assert!((got - want).abs() < 1e-9, "{got} is not {want}");
    }

    #[test]
    fn a_text_keeps_its_fit_while_only_its_digits_change() {
        assert_eq!(fit_key("-6.02 dB"), "-0.00 dB");
        assert_eq!(fit_key("12.3 dB"), "00.0 dB");
        assert_eq!(fit_key("-inf dB"), "-inf dB");
        assert_eq!(fit_key(""), "");
        assert_eq!(fit_key("-6.00 dB"), fit_key("-5.98 dB"));
        assert_ne!(fit_key("-9.99 dB"), fit_key("-10.0 dB"));
        assert_ne!(fit_key("-0.811 dB"), fit_key("-0.81 dB"));
    }

    #[test]
    fn a_css_length_in_px_is_read() {
        assert_eq!(px_value("22px"), Some(22.0));
        assert_eq!(px_value(" 15.5px "), Some(15.5));
        assert_eq!(px_value("1.2em"), None);
        assert_eq!(px_value("px"), None);
        assert_eq!(px_value(""), None);
    }

    #[test]
    fn a_text_is_scaled_down_to_fit_never_up() {
        // Fits with the margin: the base size.
        assert_eq!(fitted_font(22.0, (96.0, 20.0), (100.0, 30.0)), 22.0);
        // The margin itself: 96 of 100 fits, 96.5 does not.
        assert_eq!(fitted_font(20.0, (96.0, 10.0), (100.0, 30.0)), 20.0);
        assert!(fitted_font(20.0, (96.5, 10.0), (100.0, 30.0)) < 20.0);
        // Too wide: scaled so the text takes 96 % of the width.
        close(fitted_font(15.0, (48.0, 18.0), (40.0, 25.0)), 12.0);
        // Too tall: the height decides.
        close(fitted_font(52.0, (300.0, 62.5), (600.0, 50.0)), 39.936);
        // Both too large: the smaller scale wins.
        close(fitted_font(10.0, (200.0, 50.0), (100.0, 50.0)), 4.8);
        // Nothing measured (not laid out yet): the base size.
        assert_eq!(fitted_font(24.0, (0.0, 0.0), (104.0, 42.0)), 24.0);
        assert_eq!(fitted_font(24.0, (0.0, 0.0), (0.0, 0.0)), 24.0);
        // A room of no size never gives a zero font.
        assert_eq!(fitted_font(24.0, (50.0, 20.0), (0.0, 0.0)), 24.0);
    }
}
