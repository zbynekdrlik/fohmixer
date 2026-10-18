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
//! difference); one the engineer took shows `RUKA` (screen 1, states 3
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

/// A label's last word (the mockup's `ST`, `PA`).
pub fn short_label(label: &str) -> &str {
    label.split_whitespace().last().unwrap_or(label)
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
    }
}

/// Whether a tap overrides REVERB AUT until the next stop or start: the
/// faders lowered while Live plays, or at the song level while it stands
/// with no stop's wait running and no fader in the hand (a hand is the
/// engineer's, not a tap's).
fn overridden(s: &ReverbState, lowered: bool) -> bool {
    match (s.playing, lowered) {
        (Some(true), true) => true,
        (Some(false), false) => !s.waiting && !s.faders.iter().any(|f| f.hand),
        _ => false,
    }
}

/// What REVERB does now, in plain words; none while it says nothing (AUT
/// off at the song level, or the transport not yet heard).
fn reverb_sub(s: &ReverbState, lowered: bool) -> Option<String> {
    if !s.aut {
        return lowered.then(|| "ručne · −10 dB".to_string());
    }
    let playing = s.playing?;
    Some(match (lowered, playing, s.waiting) {
        (false, true, _) => "hrá · plný".to_string(),
        (false, false, true) => "stojí · plný".to_string(),
        (false, false, false) => {
            hands_text(&s.faders).unwrap_or_else(|| "ručne plný · do štartu".to_string())
        }
        (true, true, _) => "ručne −10 · do zast.".to_string(),
        (true, false, _) => hands_text(&s.faders).unwrap_or_else(|| "stojí · −10 dB".to_string()),
    })
}

/// The second line while the engineer holds some faders: the lowered ones
/// and the ones in the hand, by their short labels; none without a hand.
fn hands_text(faders: &[ReverbFader]) -> Option<String> {
    let hands: Vec<&str> = faders
        .iter()
        .filter(|f| f.hand)
        .map(|f| short_label(&f.label))
        .collect();
    if hands.is_empty() {
        return None;
    }
    let down: Vec<&str> = faders
        .iter()
        .filter(|f| f.back.is_some())
        .map(|f| short_label(&f.label))
        .collect();
    let hand = format!("{} v ruke", hands.join(" "));
    Some(if down.is_empty() {
        hand
    } else {
        format!("{} −10 · {hand}", down.join(" "))
    })
}

/// The pill beside a held fader: the live difference to its return level,
/// in whole dB (`AUT −10`); `AUT −∞` for a fader at −∞, and `AUT` alone
/// under 1 dB.
pub fn aut_text(back: f64, live: f64) -> String {
    let n = (value2db(back) - value2db(live)).round();
    if n == f64::INFINITY {
        "AUT −∞".to_string()
    } else if n >= 1.0 {
        format!("AUT −{n:.0}")
    } else {
        "AUT".to_string()
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
    /// Let go (`RUKA`).
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
                low: false
            }
        );
        assert_eq!(
            look(&with(false, Some(Low::Tap), None, false)),
            RailLook {
                lit: false,
                aut: Aut::Off,
                sub: Some("ručne · −10 dB".into()),
                low: true
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
                sub: Some("hrá · plný".into()),
                low: false
            }
        );
        assert_eq!(
            look(&with(true, None, Some(false), true)),
            RailLook {
                lit: true,
                aut: Aut::Run,
                sub: Some("stojí · plný".into()),
                low: false
            }
        );
        assert_eq!(
            look(&with(true, Some(Low::Stop), Some(false), false)),
            RailLook {
                lit: false,
                aut: Aut::Run,
                sub: Some("stojí · −10 dB".into()),
                low: true
            }
        );
        // A tap overrides the rule until the next stop or start: outlined.
        assert_eq!(
            look(&with(true, Some(Low::Tap), Some(true), false)),
            RailLook {
                lit: false,
                aut: Aut::Held,
                sub: Some("ručne −10 · do zast.".into()),
                low: true
            }
        );
        assert_eq!(
            look(&with(true, None, Some(false), false)),
            RailLook {
                lit: true,
                aut: Aut::Held,
                sub: Some("ručne plný · do štartu".into()),
                low: false
            }
        );
        // The transport not heard yet: the pill runs, no line.
        assert_eq!(
            look(&with(true, None, None, false)),
            RailLook {
                lit: true,
                aut: Aut::Run,
                sub: None,
                low: false
            }
        );
        assert_eq!(
            look(&with(true, Some(Low::Stop), None, false)),
            RailLook {
                lit: false,
                aut: Aut::Run,
                sub: None,
                low: true
            }
        );
        // Before the hub's state: dark, nothing.
        assert_eq!(
            reverb_look(None, Fill::None),
            RailLook {
                lit: false,
                aut: Aut::Off,
                sub: None,
                low: false
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
            Some("drž… vypína AUT")
        );
        assert_eq!(
            line(&off, Fill::Filling).as_deref(),
            Some("drž… zapína AUT")
        );
        assert_eq!(
            line(&playing, Fill::Full).as_deref(),
            Some("pusť — vypne AUT")
        );
        assert_eq!(line(&off, Fill::Full).as_deref(), Some("pusť — zapne AUT"));
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
    fn a_hand_shows_in_the_second_line_with_short_labels() {
        let mut s = with(true, Some(Low::Stop), Some(false), false);
        s.faders[1].back = None;
        s.faders[1].hand = true;
        let one = look(&s);
        assert_eq!(one.sub.as_deref(), Some("ST −10 · PA v ruke"));
        assert_eq!((one.aut, one.lit, one.low), (Aut::Run, false, true));
        s.faders[0].back = None;
        s.faders[0].hand = true;
        assert_eq!(look(&s).sub.as_deref(), Some("ST PA v ruke"));
        assert_eq!(short_label("Rev ST"), "ST");
        assert_eq!(short_label("Hall"), "Hall");
        assert_eq!(short_label(""), "");
    }

    #[test]
    fn every_fader_in_the_hand_while_live_stands_is_the_hand_not_a_tap() {
        // The hub ends `low` when the last held fader is let go: the line
        // names the hands and the pill runs (no tap overrides the rule).
        let mut s = with(true, None, Some(false), false);
        s.faders[1].hand = true;
        assert_eq!(
            look(&s),
            RailLook {
                lit: true,
                aut: Aut::Run,
                sub: Some("PA v ruke".into()),
                low: false
            }
        );
        s.faders[0].hand = true;
        assert_eq!(look(&s).sub.as_deref(), Some("ST PA v ruke"));
        // During the stop's wait or while Live plays, the rule's own line.
        let mut waiting = with(true, None, Some(false), true);
        waiting.faders[1].hand = true;
        let wait = look(&waiting);
        assert_eq!(
            (wait.aut, wait.sub.as_deref()),
            (Aut::Run, Some("stojí · plný"))
        );
        let mut playing = with(true, None, Some(true), false);
        playing.faders[1].hand = true;
        let play = look(&playing);
        assert_eq!(
            (play.aut, play.sub.as_deref()),
            (Aut::Run, Some("hrá · plný"))
        );
        // AUT off: a hand changes nothing on the button.
        let mut manual = with(false, Some(Low::Tap), Some(false), false);
        manual.faders[1].back = None;
        manual.faders[1].hand = true;
        assert_eq!(look(&manual).sub.as_deref(), Some("ručne · −10 dB"));
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
        assert_eq!(aut_text(0.85, volume_of(-10.0)), "AUT −10");
        assert_eq!(aut_text(0.85, 0.85), "AUT");
        assert_eq!(aut_text(0.85, volume_of(-0.6)), "AUT −1");
        assert_eq!(aut_text(0.85, volume_of(-0.4)), "AUT");
        assert_eq!(aut_text(0.6, volume_of(-20.0)), "AUT −10");
        assert_eq!(aut_text(0.85, 0.0), "AUT −∞");
        assert_eq!(aut_text(0.0, 0.0), "AUT");
        assert_eq!(aut_text(0.6, 0.85), "AUT", "above its return level");
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
