//! A track's colour from Live (#21): the LOM's `color` is `0xRRGGBB`; the
//! name button takes it while the track is audible, with the text colour
//! that reads on it.

/// The CSS colour of Live's `color` value; `None` for anything that is not
/// a 24-bit integer.
pub fn css_color(value: f64) -> Option<String> {
    let whole = value.fract() == 0.0 && (0.0..=f64::from(0x00FF_FFFF)).contains(&value);
    whole.then(|| format!("#{:06X}", value as u32))
}

/// The last 8-bit channel value on the linear part of the sRGB transfer
/// function (10/255 = 0.0392 ≤ 0.04045 < 11/255).
const KNEE: u32 = 10;

/// A channel's linear light (sRGB transfer function).
fn linear(channel: u32) -> f64 {
    let c = f64::from(channel) / 255.0;
    if channel <= KNEE {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The relative luminance of `0xRRGGBB` (WCAG).
pub fn luminance(rgb: u32) -> f64 {
    let (r, g, b) = ((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// The luminance above which dark text reads better than light (where the
/// WCAG contrast with black and with white are equal).
pub const LIGHT: f64 = 0.179;

/// Whether a luminance is light (dark text on it).
pub fn is_light(luminance: f64) -> bool {
    luminance > LIGHT
}

/// The text colour on Live's `color` value.
pub fn text_on(value: f64) -> &'static str {
    if is_light(luminance(value as u32)) {
        "#10101a"
    } else {
        "#f4f4fa"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_live_colour_is_css_hex() {
        assert_eq!(css_color(16_149_507.0).as_deref(), Some("#F66C03"));
        assert_eq!(css_color(0.0).as_deref(), Some("#000000"));
        assert_eq!(css_color(16_777_215.0).as_deref(), Some("#FFFFFF"));
        assert_eq!(css_color(16_777_216.0), None);
        assert_eq!(css_color(-1.0), None);
        assert_eq!(css_color(12.5), None);
        assert_eq!(css_color(f64::NAN), None);
    }

    #[test]
    fn luminance_follows_wcag() {
        assert_eq!(luminance(0x000000), 0.0);
        assert!((luminance(0xFFFFFF) - 1.0).abs() < 1e-12);
        assert!((luminance(0xFF0000) - 0.2126).abs() < 1e-12);
        assert!((luminance(0x00FF00) - 0.7152).abs() < 1e-12);
        assert!((luminance(0x0000FF) - 0.0722).abs() < 1e-12);
        // Below the transfer function's knee, and just above it.
        assert!((luminance(0x0A0A0A) - 10.0 / 255.0 / 12.92).abs() < 1e-12);
        let knee = ((11.0 / 255.0 + 0.055) / 1.055_f64).powf(2.4);
        assert!((luminance(0x0B0B0B) - knee).abs() < 1e-12);
    }

    #[test]
    fn light_is_strictly_above_the_threshold() {
        assert!(!is_light(LIGHT));
        assert!(is_light(0.179_000_000_000_000_02));
        assert!(!is_light(0.0));
    }

    #[test]
    fn the_text_reads_on_the_colour() {
        assert_eq!(text_on(f64::from(0xFFFFFFu32)), "#10101a");
        assert_eq!(text_on(f64::from(0xF5C451u32)), "#10101a");
        assert_eq!(text_on(f64::from(0x000000u32)), "#f4f4fa");
        assert_eq!(text_on(f64::from(0x1E3A8Au32)), "#f4f4fa");
    }
}
