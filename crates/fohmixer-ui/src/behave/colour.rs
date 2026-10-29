//! A strip's colour (#164; the owner's approved mockup
//! `docs/mockups/strip-color-v1.html`): the engineer's choice in the
//! channel detail's COLOR, kept by the hub (`fohmixer_proto::colors`), no
//! longer the Ableton track's colour. The strip's 3 px edge and the
//! detail's name chip take it (`--tc`), with the text colour that reads on
//! it (`--tt`); a strip with none chosen shows NONE's calm grey.

use fohmixer_proto::colors::{ColorKey, ColorsDoc, css, shown_rgb};
use fohmixer_proto::sectors::StripRef;

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

/// The text colour on `0xRRGGBB`.
pub fn text_on(rgb: u32) -> &'static str {
    if is_light(luminance(rgb)) {
        "#10101a"
    } else {
        "#f4f4fa"
    }
}

/// The style of a strip's name button and the detail's chip in the chosen
/// colour (NONE's grey when none) and the text colour that reads on it.
pub fn colour_style(key: Option<ColorKey>) -> String {
    let rgb = shown_rgb(key);
    format!("--tc:{};--tt:{};", css(rgb), text_on(rgb))
}

/// The COLOR section's keys in the mockup's order: NONE (two cells wide),
/// then the twelve colours.
pub fn palette_keys() -> Vec<Option<ColorKey>> {
    std::iter::once(None)
        .chain(ColorKey::ALL.into_iter().map(Some))
        .collect()
}

/// A key's id on the page (`data-colour`): the palette key, `none`.
pub fn key_id(key: Option<ColorKey>) -> &'static str {
    key.map_or("none", ColorKey::as_str)
}

/// The style of a COLOR key: its swatch's colour (`--sw`).
pub fn swatch_style(key: Option<ColorKey>) -> String {
    format!("--sw:{};", css(shown_rgb(key)))
}

/// A strip's colour in the hub's document as last told (none told yet, or
/// a strip with no identity: NONE).
pub fn colour_of(doc: Option<&ColorsDoc>, id: Option<&StripRef>) -> Option<ColorKey> {
    doc.zip(id).and_then(|(doc, id)| doc.of(id))
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(text_on(0xFFFFFF), "#10101a");
        assert_eq!(text_on(0xF5C451), "#10101a");
        assert_eq!(text_on(0x000000), "#f4f4fa");
        assert_eq!(text_on(0x1E3A8A), "#f4f4fa");
    }

    #[test]
    fn a_strip_shows_its_chosen_colour_or_nones_grey() {
        assert_eq!(colour_style(None), "--tc:#73738a;--tt:#f4f4fa;");
        assert_eq!(
            colour_style(Some(ColorKey::Amber)),
            "--tc:#d6b24e;--tt:#10101a;"
        );
        assert_eq!(
            colour_style(Some(ColorKey::Blue)),
            "--tc:#6d8fe2;--tt:#10101a;"
        );
    }

    #[test]
    fn the_palette_is_none_then_the_twelve() {
        let keys = palette_keys();
        assert_eq!(keys.len(), 13);
        assert_eq!(keys[0], None);
        assert_eq!(keys[1], Some(ColorKey::Red));
        assert_eq!(keys[12], Some(ColorKey::Sand));
        let ids: Vec<&str> = keys.iter().map(|k| key_id(*k)).collect();
        assert_eq!(
            ids,
            [
                "none", "red", "orange", "amber", "lime", "green", "teal", "cyan", "blue",
                "violet", "purple", "pink", "sand"
            ]
        );
        assert_eq!(swatch_style(None), "--sw:#73738a;");
        assert_eq!(swatch_style(Some(ColorKey::Pink)), "--sw:#e27fa8;");
    }

    #[test]
    fn a_strips_colour_is_the_documents_for_its_identity() {
        let vox = StripRef {
            instance: "band".into(),
            label: "Vox 1".into(),
        };
        let doc = ColorsDoc::empty().with(&vox, Some(ColorKey::Teal));
        assert_eq!(colour_of(Some(&doc), Some(&vox)), Some(ColorKey::Teal));
        assert_eq!(colour_of(None, Some(&vox)), None, "nothing told yet");
        assert_eq!(colour_of(Some(&doc), None), None, "no identity");
        let other = StripRef {
            instance: "master".into(),
            label: "Vox 1".into(),
        };
        assert_eq!(colour_of(Some(&doc), Some(&other)), None);
    }
}
