//! REVERB on the surface (#130, D18): the button's look and the faders'
//! marks. Pure.
//!
//! The REVERB rail button (the approved mockup reverb-link-v2, screen 3):
//! a tap asks the hub for the reverb faders 10 dB down or back, a 1 s hold
//! turns REVERB AUT on or off at its lift (the press is `behave::rail`'s,
//! as the merged STAGE button's). It is lit at the song level and dark
//! while lowered; with REVERB AUT on it wears the `AUT` pill, outlined
//! while a tap overrides the rule until the next stop or start, and a
//! second line says in plain words what the rule does now. A reverb fader
//! the rule holds low shows where it returns and `AUT −n` (the live
//! difference); one the engineer took shows `MAN` (screen 1, states 3
//! and 4). Everything here reads the hub's `ReverbState`.

use fohmixer_proto::client::{ReverbFader, ReverbState};
use fohmixer_proto::volume::value2db;

use super::rail::{Aut, Fill, RailLook, full_text, hold_text};

/// Whether the REVERB button takes a tap: none before the hub's state
/// (waiting); `Some(false)` without a reverb fader (red, I5).
pub fn reverb_ready(state: Option<&ReverbState>) -> Option<bool> {
    state.map(|s| !s.faders.is_empty())
}

/// What a tap asks the hub for (`reverb_low`): down while at the song
/// level, back while lowered (`!low` as shown, so two pages' simultaneous
/// taps agree); none without faders.
pub fn tap_low(state: Option<&ReverbState>) -> Option<bool> {
    state
        .filter(|s| !s.faders.is_empty())
        .map(|s| s.low.is_none())
}

/// What a tap sends: `tap_low` of the state the hub told this connection
/// (`fresh`), only while the page is connected; none: the tap is refused.
pub fn tap_value(fresh: Option<&ReverbState>, connected: bool) -> Option<bool> {
    tap_low(fresh).filter(|_| connected)
}

/// The flight recorder's step of a tap: `tap`, or `refused` when it could
/// not be sent.
pub fn tap_step(value: Option<bool>) -> &'static str {
    if value.is_some() { "tap" } else { "refused" }
}

/// The REVERB button (D18; mockup screen 3): lit at the song level, dark
/// while lowered; the `AUT` pill outlined while a tap overrides the rule; a
/// press's fill (`behave::rail::fill_of`) shows its own line.
pub fn reverb_look(state: Option<&ReverbState>, fill: Fill) -> RailLook {
    let Some(s) = state else {
        return RailLook {
            lit: false,
            aut: Aut::Off,
            sub: None,
            low: false,
            mixed: false,
        };
    };
    let lowered = s.low.is_some();
    let aut = match (s.aut, overridden(s, lowered)) {
        (false, _) => Aut::Off,
        (true, true) => Aut::Held,
        (true, false) => Aut::Run,
    };
    let sub = match fill {
        Fill::Filling => Some(hold_text(s.aut).to_string()),
        Fill::Full => Some(full_text(s.aut).to_string()),
        Fill::None => reverb_sub(s, lowered),
    };
    RailLook {
        lit: !lowered,
        aut,
        sub,
        low: lowered,
        mixed: false,
    }
}

/// Whether a tap overrides REVERB AUT until the next stop or start: the
/// faders lowered while Live plays, or at the song level while it stands
/// with no stop's wait running, unless every fader was taken by hand (then
/// no tap brought them up: the hands did; a fader returned by a tap beside
/// one in the hand is the tap's, #130 PR 5 re-review N1).
fn overridden(s: &ReverbState, lowered: bool) -> bool {
    match (s.playing, lowered) {
        (Some(true), true) => true,
        (Some(false), false) => !s.waiting && !all_in_hand(&s.faders),
        _ => false,
    }
}

/// Every reverb fader taken by hand (`MAN`), none brought up by a tap.
fn all_in_hand(faders: &[ReverbFader]) -> bool {
    !faders.is_empty() && faders.iter().all(|f| f.hand)
}

/// What REVERB does now, in plain words; none while it says nothing (AUT
/// off at the song level, or the transport not yet heard). It names no
/// fader in the hand (the owner's ruling on #130 of 2026-10-21): the faders
/// show that themselves (`MAN`, `AUT −10`). With every fader in the hand
/// while Live stands, nothing is held and no tap overrides the rule: its
/// own `STOP · normal` (PR #139 review I3), never the tap's line.
fn reverb_sub(s: &ReverbState, lowered: bool) -> Option<String> {
    if !s.aut {
        return lowered.then(|| "MAN · −10 dB".to_string());
    }
    let playing = s.playing?;
    let line = match (lowered, playing, s.waiting) {
        (false, true, _) => "PLAY · normal",
        (false, false, true) => "STOP · normal",
        (false, false, false) if all_in_hand(&s.faders) => "STOP · normal",
        (false, false, false) => "MAN normal · to PLAY",
        (true, true, _) => "MAN −10 · until STOP",
        (true, false, _) => "STOP · −10 dB",
    };
    Some(line.to_string())
}

/// The pill beside a held fader: the live difference to its return level,
/// in whole dB (`AUTO −10`); `AUTO −∞` for a fader at −∞, and `AUTO` alone
/// under 1 dB.
pub fn aut_text(back: f64, live: f64) -> String {
    let n = (value2db(back) - value2db(live)).round();
    if n == f64::INFINITY {
        "AUTO −∞".to_string()
    } else if n >= 1.0 {
        format!("AUTO −{n:.0}")
    } else {
        "AUTO".to_string()
    }
}

/// What a reverb fader shows of the rule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mark {
    None,
    /// Held low: its return level (Live's volume).
    Aut {
        back: f64,
    },
    /// Let go (`MAN`).
    Hand,
}

/// The mark of the fader whose volume's write key is `key`.
pub fn mark_of(state: Option<&ReverbState>, key: &str) -> Mark {
    match state.and_then(|s| s.faders.iter().find(|f| f.key == key)) {
        Some(f) if f.hand => Mark::Hand,
        Some(ReverbFader {
            back: Some(back), ..
        }) => Mark::Aut { back: *back },
        _ => Mark::None,
    }
}

/// Whether the rule holds that fader low: its readout (the strip's and the
/// channel detail's) in the rule's colour.
pub fn held_low(state: Option<&ReverbState>, key: &str) -> bool {
    matches!(mark_of(state, key), Mark::Aut { .. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::client::Low;
    use fohmixer_proto::volume::volume_of;

    const ST: &str = "band|live_set tracks 4 mixer_device volume|value";
    const PA: &str = "band|live_set tracks 5 mixer_device volume|value";

    fn fader(label: &str, key: &str) -> ReverbFader {
        ReverbFader {
            key: key.to_string(),
            label: label.to_string(),
            back: None,
            hand: false,
        }
    }

    /// Rev ST and Rev PA (tracks 4 and 5) at the song level, AUT off, the
    /// transport not heard.
    fn state() -> ReverbState {
        ReverbState {
            aut: false,
            low: None,
            playing: None,
            waiting: false,
            faders: vec![fader("Rev ST", ST), fader("Rev PA", PA)],
        }
    }

    fn with(aut: bool, low: Option<Low>, playing: Option<bool>, waiting: bool) -> ReverbState {
        let mut s = state();
        s.aut = aut;
        s.low = low;
        s.playing = playing;
        s.waiting = waiting;
        if low.is_some() {
            s.faders[0].back = Some(0.85);
            s.faders[1].back = Some(0.6);
        }
        s
    }

    fn look(s: &ReverbState) -> RailLook {
        reverb_look(Some(s), Fill::None)
    }

    fn line(s: &ReverbState, fill: Fill) -> Option<String> {
        reverb_look(Some(s), fill).sub
    }

    #[test]
    fn the_button_waits_for_the_hub_and_is_red_without_faders() {
        assert_eq!(reverb_ready(None), None);
        let mut s = state();
        assert_eq!(reverb_ready(Some(&s)), Some(true));
        assert_eq!(tap_low(Some(&s)), Some(true));
        s.faders.clear();
        assert_eq!(reverb_ready(Some(&s)), Some(false));
        assert_eq!(tap_low(Some(&s)), None);
        assert_eq!(tap_low(None), None);
        let lowered = with(false, Some(Low::Tap), None, false);
        assert_eq!(tap_low(Some(&lowered)), Some(false));
        let stopped = with(true, Some(Low::Stop), Some(false), false);
        assert_eq!(tap_low(Some(&stopped)), Some(false));
    }

    #[test]
    fn a_tap_goes_only_connected_with_the_hubs_state_of_this_connection() {
        let full = state();
        let lowered = with(false, Some(Low::Tap), None, false);
        assert_eq!(tap_value(Some(&full), true), Some(true));
        assert_eq!(tap_value(Some(&lowered), true), Some(false));
        assert_eq!(tap_value(Some(&full), false), None, "not connected");
        assert_eq!(tap_value(Some(&lowered), false), None);
        assert_eq!(tap_value(None, true), None, "no state on this connection");
        let mut empty = state();
        empty.faders.clear();
        assert_eq!(tap_value(Some(&empty), true), None, "no reverb fader");
        assert_eq!(tap_step(Some(true)), "tap");
        assert_eq!(tap_step(Some(false)), "tap");
        assert_eq!(tap_step(None), "refused");
    }

    #[test]
    fn the_buttons_look_follows_the_mockup() {
        assert_eq!(
            look(&with(false, None, None, false)),
            RailLook {
                lit: true,
                aut: Aut::Off,
                sub: None,
                low: false,
                mixed: false,
            }
        );
        assert_eq!(
            look(&with(false, Some(Low::Tap), None, false)),
            RailLook {
                lit: false,
                aut: Aut::Off,
                sub: Some("MAN · −10 dB".into()),
                low: true,
                mixed: false,
            }
        );
        // AUT off: no line at the song level, whatever the transport.
        assert_eq!(look(&with(false, None, Some(true), false)).sub, None);
        assert_eq!(look(&with(false, None, Some(false), false)).sub, None);
        assert_eq!(
            look(&with(true, None, Some(true), false)),
            RailLook {
                lit: true,
                aut: Aut::Run,
                sub: Some("PLAY · normal".into()),
                low: false,
                mixed: false,
            }
        );
        assert_eq!(
            look(&with(true, None, Some(false), true)),
            RailLook {
                lit: true,
                aut: Aut::Run,
                sub: Some("STOP · normal".into()),
                low: false,
                mixed: false,
            }
        );
        assert_eq!(
            look(&with(true, Some(Low::Stop), Some(false), false)),
            RailLook {
                lit: false,
                aut: Aut::Run,
                sub: Some("STOP · −10 dB".into()),
                low: true,
                mixed: false,
            }
        );
        // A tap overrides the rule until the next stop or start: outlined.
        assert_eq!(
            look(&with(true, Some(Low::Tap), Some(true), false)),
            RailLook {
                lit: false,
                aut: Aut::Held,
                sub: Some("MAN −10 · until STOP".into()),
                low: true,
                mixed: false,
            }
        );
        assert_eq!(
            look(&with(true, None, Some(false), false)),
            RailLook {
                lit: true,
                aut: Aut::Held,
                sub: Some("MAN normal · to PLAY".into()),
                low: false,
                mixed: false,
            }
        );
        // The transport not heard yet: the pill runs, no line.
        assert_eq!(
            look(&with(true, None, None, false)),
            RailLook {
                lit: true,
                aut: Aut::Run,
                sub: None,
                low: false,
                mixed: false,
            }
        );
        assert_eq!(
            look(&with(true, Some(Low::Stop), None, false)),
            RailLook {
                lit: false,
                aut: Aut::Run,
                sub: None,
                low: true,
                mixed: false,
            }
        );
        // Before the hub's state: dark, nothing.
        assert_eq!(
            reverb_look(None, Fill::None),
            RailLook {
                lit: false,
                aut: Aut::Off,
                sub: None,
                low: false,
                mixed: false,
            }
        );
        assert_eq!(reverb_look(None, Fill::Full).sub, None);
    }

    #[test]
    fn a_press_shows_its_hold_and_full_lines_and_keeps_the_rest() {
        let playing = with(true, None, Some(true), false);
        let off = with(false, None, None, false);
        assert_eq!(
            line(&playing, Fill::Filling).as_deref(),
            Some("hold… AUTO off")
        );
        assert_eq!(line(&off, Fill::Filling).as_deref(), Some("hold… AUTO on"));
        assert_eq!(
            line(&playing, Fill::Full).as_deref(),
            Some("release — AUTO off")
        );
        assert_eq!(line(&off, Fill::Full).as_deref(), Some("release — AUTO on"));
        // The light, the pill and the line's colour stay what they were.
        let lowered = with(true, Some(Low::Tap), Some(true), false);
        let held = reverb_look(Some(&lowered), Fill::Full);
        assert_eq!((held.lit, held.aut, held.low), (false, Aut::Held, true));
        let filling = reverb_look(Some(&playing), Fill::Filling);
        assert_eq!(
            (filling.lit, filling.aut, filling.low),
            (true, Aut::Run, false)
        );
    }

    #[test]
    fn the_button_names_no_fader_in_the_hand() {
        // The owner's ruling on #130 of 2026-10-21: the faders show the
        // hand (MAN) and the lowering (AUT −10) themselves, so the line is
        // REVERB's own: Rev ST held lowered, Rev PA taken by hand.
        let mut s = with(true, Some(Low::Stop), Some(false), false);
        s.faders[1].back = None;
        s.faders[1].hand = true;
        let one = look(&s);
        assert_eq!(one.sub.as_deref(), Some("STOP · −10 dB"));
        assert_eq!((one.aut, one.lit, one.low), (Aut::Run, false, true));
        // While Live plays the tap's override, the same with a hand.
        let mut tap = with(true, Some(Low::Tap), Some(true), false);
        tap.faders[1].back = None;
        tap.faders[1].hand = true;
        assert_eq!(look(&tap).sub.as_deref(), Some("MAN −10 · until STOP"));
    }

    #[test]
    fn every_fader_in_the_hand_while_live_stands_is_the_hand_not_a_tap() {
        // The hub ends `low` when the last held fader is let go: every
        // fader taken by hand, no tap, so the pill runs (no tap overrides
        // the rule) and the line is REVERB's own state with nothing held,
        // naming no hand (the owner's ruling on #130 of 2026-10-21; PR #139
        // review I3: never the tap override's `MAN normal · to PLAY`).
        let mut s = with(true, None, Some(false), false);
        s.faders[0].hand = true;
        s.faders[1].hand = true;
        assert_eq!(
            look(&s),
            RailLook {
                lit: true,
                aut: Aut::Run,
                sub: Some("STOP · normal".into()),
                low: false,
                mixed: false,
            }
        );
        // During the stop's wait or while Live plays, the rule's own line.
        let mut waiting = with(true, None, Some(false), true);
        waiting.faders[1].hand = true;
        let wait = look(&waiting);
        assert_eq!(
            (wait.aut, wait.sub.as_deref()),
            (Aut::Run, Some("STOP · normal"))
        );
        let mut playing = with(true, None, Some(true), false);
        playing.faders[1].hand = true;
        let play = look(&playing);
        assert_eq!(
            (play.aut, play.sub.as_deref()),
            (Aut::Run, Some("PLAY · normal"))
        );
        // AUT off: a hand changes nothing on the button.
        let mut manual = with(false, Some(Low::Tap), Some(false), false);
        manual.faders[1].back = None;
        manual.faders[1].hand = true;
        assert_eq!(look(&manual).sub.as_deref(), Some("MAN · −10 dB"));
        // AUT off, every fader in the hand at the song level: no line.
        let mut idle = with(false, None, Some(false), false);
        idle.faders[0].hand = true;
        idle.faders[1].hand = true;
        assert_eq!((look(&idle).aut, look(&idle).sub), (Aut::Off, None));
    }

    #[test]
    fn a_tap_back_with_a_fader_in_the_hand_shows_the_taps_override() {
        // REVERB AUT on, Live stopped, both lowered; Rev PA taken by hand,
        // then a tap back returned Rev ST (the hand stays until the next
        // stop): the tap's override until the start, outlined (F30); Rev
        // PA's own fader shows MAN (#130 PR 5 re-review N1, the main
        // session's ruling).
        let mut s = with(true, None, Some(false), false);
        s.faders[1].hand = true;
        assert_eq!(
            look(&s),
            RailLook {
                lit: true,
                aut: Aut::Held,
                sub: Some("MAN normal · to PLAY".into()),
                low: false,
                mixed: false,
            }
        );
        assert_eq!(mark_of(Some(&s), PA), Mark::Hand);
        assert_eq!(mark_of(Some(&s), ST), Mark::None);
        // No reverb fader at all: the override's look, as before.
        let mut none = with(true, None, Some(false), false);
        none.faders.clear();
        let empty = look(&none);
        assert_eq!(
            (empty.aut, empty.sub.as_deref()),
            (Aut::Held, Some("MAN normal · to PLAY"))
        );
    }

    #[test]
    fn a_held_fader_shows_its_return_and_aut_minus_the_live_difference() {
        let s = with(true, Some(Low::Stop), Some(false), false);
        assert_eq!(mark_of(Some(&s), ST), Mark::Aut { back: 0.85 });
        assert_eq!(mark_of(Some(&s), PA), Mark::Aut { back: 0.6 });
        assert_eq!(mark_of(Some(&s), "other"), Mark::None);
        assert_eq!(mark_of(None, ST), Mark::None);
        assert_eq!(mark_of(Some(&state()), ST), Mark::None, "at the song level");
        let mut h = s.clone();
        h.faders[1].back = None;
        h.faders[1].hand = true;
        assert_eq!(mark_of(Some(&h), PA), Mark::Hand);
        assert_eq!(mark_of(Some(&h), ST), Mark::Aut { back: 0.85 });
        // A hand wins over a return level the state may still carry.
        h.faders[0].hand = true;
        assert_eq!(mark_of(Some(&h), ST), Mark::Hand);
        assert_eq!(aut_text(0.85, volume_of(-10.0)), "AUTO −10");
        assert_eq!(aut_text(0.85, 0.85), "AUTO");
        assert_eq!(aut_text(0.85, volume_of(-0.6)), "AUTO −1");
        assert_eq!(aut_text(0.85, volume_of(-0.4)), "AUTO");
        assert_eq!(aut_text(0.6, volume_of(-20.0)), "AUTO −10");
        assert_eq!(aut_text(0.85, 0.0), "AUTO −∞");
        assert_eq!(aut_text(0.0, 0.0), "AUTO");
        assert_eq!(aut_text(0.6, 0.85), "AUTO", "above its return level");
    }

    #[test]
    fn a_readout_is_in_the_rules_colour_only_while_its_fader_is_held_low() {
        let s = with(true, Some(Low::Stop), Some(false), false);
        assert!(held_low(Some(&s), ST));
        assert!(held_low(Some(&s), PA));
        assert!(!held_low(Some(&s), "other"));
        assert!(!held_low(None, ST));
        assert!(!held_low(Some(&state()), ST), "at the song level");
        let mut h = s.clone();
        h.faders[1].back = None;
        h.faders[1].hand = true;
        assert!(!held_low(Some(&h), PA), "in the hand");
        assert!(held_low(Some(&h), ST));
    }
}
