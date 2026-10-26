//! The surface's behaviour as pure state machines (no DOM, no clock): the
//! components feed them pointer events and the time, and apply what they
//! return. Everything here is unit-tested natively; the parity numbers come
//! from the TouchOSC scripts of `abl-touchosc` (fader_script 2.5.4,
//! mute_button 2.7.2, pan_control 1.5.3, meter_script 2.5.2,
//! document_script 2.10.0), as the S4 design note quotes them, the Stream
//! Deck's presses (#52, `deck`), the hold that opens a channel's detail
//! (#71, `hold`), the Pro-Q 4 screen's finger and picture (#71 PR E,
//! `eq`), the backing-tracks fader's drag and look (#58, `vca`), the
//! page's idle state (#51, `idle`), AbleSet's tab (#90, `ableset`), the
//! rail buttons with an automatic mode (#104, #130, D18, `rail`), the
//! fader link's marks and same-dB move (#130, `fader_link`), REVERB's
//! button and the faders' marks (#130, `reverb`), AUTOTUNE's button
//! (#104, D19, `autotune`) and the strip's marks line (#151, `marks`).

pub mod ableset;
pub mod autotune;
pub mod colour;
pub mod db_text;
pub mod deck;
pub mod eq;
pub mod eq_view;
pub mod fader;
pub mod fader_link;
pub mod hold;
pub mod idle;
pub mod label;
pub mod link;
pub mod marks;
pub mod meter;
pub mod mute;
pub mod pan;
pub mod peak;
pub mod rail;
pub mod reverb;
pub mod scale;
pub mod solo;
pub mod status;
pub mod timing;
pub mod toggle;
pub mod vca;

use crate::diag::perf::Pointer;

/// What a continuous control (fader, pan) shows and sends in one frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Motion {
    /// The position to show; `None` until Live reported a value.
    pub pos: Option<f64>,
    /// A Live value to send now (one per frame at most).
    pub send: Option<f64>,
}

/// Where a touch of a continuous control (fader, pan) started, in positions
/// (#43 PR D, the flight recorder's `touch` down): the position the control
/// showed, Live's value as the position the control used, whether the
/// control showed its own position (a finger, a glide, an open write or the
/// post-release hold), and the position the touch starts from (`shown` when
/// local, else `live`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Start {
    pub shown: f64,
    pub live: f64,
    pub local: bool,
    pub from: f64,
}

/// A pointer's down on a control that holds it until its end (a fader, a
/// pan, a pulse toggle; #124): its id, its type (`PointerEvent.pointerType`,
/// none for another) and whether it is the primary pointer of its type (the
/// browser's `isPrimary`: no other pointer of that type is down).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Down {
    pub id: i32,
    pub kind: Option<Pointer>,
    pub primary: bool,
}

impl Down {
    /// A finger's down (a touch pointer).
    pub fn touch(id: i32, primary: bool) -> Self {
        Self {
            id,
            kind: Some(Pointer::Touch),
            primary,
        }
    }

    /// Whether this down ends a held pointer of type `held` whose end the
    /// browser never delivered: a primary pointer of that same type means
    /// no other of its type is down (as `diag::perf::Perf::down` forgets
    /// its type's missed ups); a mouse or a pen is primary among its own
    /// kind and never ends a finger's touch (#124 M1).
    pub fn ends(self, held: Option<Pointer>) -> bool {
        self.primary && self.kind == held
    }
}

/// A press a continuous control (fader, pan) took: where its touch started
/// and, when the press ended another pointer's touch, that touch (#124).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Taken {
    pub start: Start,
    pub lost: Option<Lost>,
}

/// A touch whose end the browser never delivered (#124: iOS took the finger
/// for a system gesture and sent no `pointerup`, `pointercancel` or
/// `lostpointercapture`), ended by a primary pointer's down on its control:
/// its pointer and its end, a cancel's (no tap; `touch_end`: the move it
/// left unsent as a final `set`, else its write released). The flight
/// recorder's touch `lost`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lost {
    pub pointer: i32,
    pub end: TouchEnd,
}

/// What the end of a pointer's touch means for a continuous control's write
/// (#43, L4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TouchEnd {
    /// The pointer drove another control (or none): nothing.
    NotMine,
    /// The last move was still unsent: it goes as a final `set`.
    Send(f64),
    /// The frames already sent the last move: the store records the
    /// release time of that write.
    Released,
}

impl TouchEnd {
    /// Whether it ends a touch of this control (the flight recorder's `up`
    /// or `cancel`, #43 PR C): not when the pointer drove another one.
    pub fn ends_touch(self) -> bool {
        self != Self::NotMine
    }
}

/// The end of a touch from whether the pointer drove this control (`mine`)
/// and the value its release left unsent (`last`).
pub fn touch_end(mine: bool, last: Option<f64>) -> TouchEnd {
    match (mine, last) {
        (_, Some(value)) => TouchEnd::Send(value),
        (true, None) => TouchEnd::Released,
        (false, None) => TouchEnd::NotMine,
    }
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
    fn a_touch_ends_with_a_send_a_release_or_nothing() {
        assert_eq!(touch_end(true, Some(0.4)), TouchEnd::Send(0.4));
        assert_eq!(touch_end(false, Some(-0.5)), TouchEnd::Send(-0.5));
        assert_eq!(touch_end(true, None), TouchEnd::Released);
        assert_eq!(touch_end(false, None), TouchEnd::NotMine);
    }

    #[test]
    fn a_send_or_a_release_ends_a_touch_another_controls_pointer_does_not() {
        assert!(TouchEnd::Send(0.4).ends_touch());
        assert!(TouchEnd::Released.ends_touch());
        assert!(!TouchEnd::NotMine.ends_touch());
    }

    #[test]
    fn only_a_primary_down_of_the_held_pointers_type_ends_it() {
        let touch = Some(Pointer::Touch);
        assert!(Down::touch(2, true).ends(touch));
        assert!(!Down::touch(2, false).ends(touch), "a second finger");
        let mouse = Down {
            id: 1,
            kind: Some(Pointer::Mouse),
            primary: true,
        };
        assert!(!mouse.ends(touch), "a mouse is primary among mice only");
        assert!(mouse.ends(Some(Pointer::Mouse)));
        let pen = Down {
            id: 3,
            kind: Some(Pointer::Pen),
            primary: true,
        };
        assert!(!pen.ends(touch) && !pen.ends(Some(Pointer::Mouse)));
        let other = Down {
            id: 4,
            kind: None,
            primary: true,
        };
        assert!(!other.ends(touch), "a type the page does not know");
        assert!(other.ends(None));
        assert_eq!(
            Down::touch(7, false),
            Down {
                id: 7,
                kind: Some(Pointer::Touch),
                primary: false,
            }
        );
    }

    #[test]
    fn css_rounds_and_clamps_the_channels() {
        assert_eq!(css([255.0, 204.0, 0.0]), "rgb(255, 204, 0)");
        assert_eq!(css([127.5, 0.4, 300.0]), "rgb(128, 0, 255)");
        assert_eq!(css([-3.0, 12.6, 99.49]), "rgb(0, 13, 99)");
    }
}
