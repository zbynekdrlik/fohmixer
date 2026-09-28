//! The surface's behaviour as pure state machines (no DOM, no clock): the
//! components feed them pointer events and the time, and apply what they
//! return. Everything here is unit-tested natively; the parity numbers come
//! from the TouchOSC scripts of `abl-touchosc` (fader_script 2.5.4,
//! mute_button 2.7.2, pan_control 1.5.3, meter_script 2.5.2,
//! document_script 2.10.0), as the S4 design note quotes them.

pub mod fader;
pub mod label;
pub mod meter;
pub mod mute;
pub mod pan;
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

/// The screen px a knob (fader cap, pan dot) travels on a control drawn
/// `screen` px long whose frame is `frame` canvas px long: the frame less the
/// knob (at least one canvas px), scaled to the screen.
pub fn travel_px(screen: f64, frame: f64, knob: f64) -> f64 {
    screen * (frame - knob).max(1.0) / frame.max(1.0)
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

    #[test]
    fn a_knob_travels_its_frame_less_itself_scaled_to_the_screen() {
        // A 400 px fader drawn 200 px tall with a 36 px cap: 364 px of
        // travel on the canvas, 182 on the screen.
        assert_eq!(travel_px(200.0, 400.0, 36.0), 182.0);
        // A 101 px pan drawn at 1.5× with a 28 px dot.
        assert_eq!(travel_px(151.5, 101.0, 28.0), 109.5);
        // A knob as long as its frame still travels one canvas px.
        assert_eq!(travel_px(50.0, 20.0, 36.0), 2.5);
        assert_eq!(
            travel_px(10.0, 0.0, 0.0),
            10.0,
            "a zero frame counts as one px"
        );
    }
}
