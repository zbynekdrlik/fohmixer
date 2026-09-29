//! The surface's behaviour as pure state machines (no DOM, no clock): the
//! components feed them pointer events and the time, and apply what they
//! return. Everything here is unit-tested natively; the parity numbers come
//! from the TouchOSC scripts of `abl-touchosc` (fader_script 2.5.4,
//! mute_button 2.7.2, pan_control 1.5.3, meter_script 2.5.2,
//! document_script 2.10.0), as the S4 design note quotes them.

pub mod colour;
pub mod db_text;
pub mod fader;
pub mod label;
pub mod meter;
pub mod mute;
pub mod pan;
pub mod peak;
pub mod scale;
pub mod solo;
pub mod status;
pub mod timing;
pub mod toggle;

/// What a continuous control (fader, pan) shows and sends in one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    /// The position to show; `None` until Live reported a value.
    pub pos: Option<f64>,
    /// A Live value to send now (one per frame at most).
    pub send: Option<f64>,
}

/// A colour as 0–255 channels (fractional while it fades).
pub type Rgb = [f64; 3];

/// The CSS text of a colour, channels rounded.
pub fn css(color: Rgb) -> String {
    let [r, g, b] = color.map(|c| c.clamp(0.0, 255.0).round());
    format!("rgb({r}, {g}, {b})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_rounds_and_clamps_the_channels() {
        assert_eq!(css([255.0, 204.0, 0.0]), "rgb(255, 204, 0)");
        assert_eq!(css([127.5, 0.4, 300.0]), "rgb(128, 0, 255)");
        assert_eq!(css([-3.0, 12.6, 99.49]), "rgb(0, 13, 99)");
    }
}
