//! The rail buttons with an automatic mode (#104, #130, D18; the approved
//! mockup `docs/mockups/reverb-link-v1.html`, screen 2): a tap on the
//! finger's lift does what the button did by hand, a 3 s hold turns the
//! automatic mode on or off. While it is on, the button wears an `AUT`
//! pill, outlined while a tap overrides the rule until the next stop or
//! start, and a second line says in plain words what the rule does now.

use super::hold::Hold;

/// How long a rail button's press lasts before it turns its automatic mode
/// on or off.
pub const RAIL_HOLD_MS: f64 = 3000.0;

/// A rail button's press.
pub fn rail_press() -> Hold {
    Hold::with(RAIL_HOLD_MS)
}

/// The `AUT` pill: none, white (the rule runs) or outlined (a tap
/// overrides it until the next stop or start).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aut {
    Off,
    Run,
    Held,
}

impl Aut {
    /// Its `data-aut`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Run => "run",
            Self::Held => "held",
        }
    }
}

/// What a rail button with an automatic mode shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RailLook {
    /// Lit: the stage mics live; the reverbs at the song level.
    pub lit: bool,
    pub aut: Aut,
    /// The second line; none when it says nothing.
    pub sub: Option<String>,
    /// The second line in the rule's colour (the reverbs lowered).
    pub low: bool,
}

/// The second line while a hold fills the button.
pub fn hold_text(aut_on: bool) -> &'static str {
    if aut_on {
        "drž… vypína AUT"
    } else {
        "drž… zapína AUT"
    }
}

/// The merged STAGE button (#104): `aut` the hub's STAGE AUT flag,
/// `playing` its Live's transport, `muted` the stage mics' mute (each none
/// before it is known), `holding` a press filling it.
pub fn stage_look(
    aut: Option<bool>,
    playing: Option<bool>,
    muted: Option<bool>,
    holding: bool,
) -> RailLook {
    let on = aut == Some(true);
    let known = playing.zip(muted);
    let pill = match (on, known) {
        (false, _) => Aut::Off,
        (true, Some((playing, muted))) if playing != muted => Aut::Held,
        (true, _) => Aut::Run,
    };
    let sub = if holding {
        Some(hold_text(on).to_string())
    } else if on {
        known.map(|state| stage_sub(state).to_string())
    } else {
        None
    };
    RailLook {
        lit: muted == Some(false),
        aut: pill,
        sub,
        low: false,
    }
}

/// What STAGE AUT does now, transport and mute known: the rule mutes the
/// mics while Live plays and opens them while it stands; a tap the other
/// way holds until the next stop or start.
fn stage_sub((playing, muted): (bool, bool)) -> &'static str {
    match (playing, muted) {
        (true, true) => "hrá · stíšené",
        (false, false) => "stojí · zapnuté",
        (true, false) => "ručne do zastavenia",
        (false, true) => "ručne do štartu",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rail_holds_for_three_seconds() {
        assert_eq!(RAIL_HOLD_MS, 3000.0);
        let mut press = rail_press();
        assert!(press.down(1, true, 0.0));
        assert!(!press.open(2999.0));
        assert!(press.open(3000.0));
        assert_eq!(rail_press(), Hold::with(3000.0));
    }

    #[test]
    fn the_hold_says_what_it_will_do() {
        assert_eq!(hold_text(false), "drž… zapína AUT");
        assert_eq!(hold_text(true), "drž… vypína AUT");
        assert_eq!(
            [Aut::Off.name(), Aut::Run.name(), Aut::Held.name()],
            ["off", "run", "held"]
        );
    }

    #[test]
    fn stage_is_lit_while_the_mics_are_live_whatever_aut_says() {
        assert!(stage_look(None, None, Some(false), false).lit);
        assert!(!stage_look(None, None, Some(true), false).lit);
        assert!(
            !stage_look(None, None, None, false).lit,
            "unknown stays dark"
        );
        assert!(stage_look(Some(true), Some(true), Some(false), false).lit);
        assert!(!stage_look(Some(true), Some(false), Some(true), false).lit);
        assert!(!stage_look(Some(true), Some(true), Some(false), false).low);
        assert!(!stage_look(Some(true), Some(false), Some(false), true).low);
    }

    #[test]
    fn without_aut_there_is_no_pill_and_no_second_line() {
        for (playing, muted) in [
            (Some(true), Some(true)),
            (Some(false), Some(false)),
            (Some(true), Some(false)),
            (None, None),
        ] {
            let look = stage_look(Some(false), playing, muted, false);
            assert_eq!((look.aut, look.sub), (Aut::Off, None));
            let unknown = stage_look(None, playing, muted, false);
            assert_eq!((unknown.aut, unknown.sub), (Aut::Off, None));
        }
    }

    #[test]
    fn with_aut_the_pill_runs_and_the_line_says_what_the_rule_does() {
        let line = |playing, muted| stage_look(Some(true), Some(playing), Some(muted), false);
        assert_eq!(
            line(true, true),
            RailLook {
                lit: false,
                aut: Aut::Run,
                sub: Some("hrá · stíšené".into()),
                low: false
            }
        );
        assert_eq!(
            line(false, false),
            RailLook {
                lit: true,
                aut: Aut::Run,
                sub: Some("stojí · zapnuté".into()),
                low: false
            }
        );
        // A tap overrides the rule until the next stop or start: outlined.
        assert_eq!(
            line(true, false),
            RailLook {
                lit: true,
                aut: Aut::Held,
                sub: Some("ručne do zastavenia".into()),
                low: false
            }
        );
        assert_eq!(
            line(false, true),
            RailLook {
                lit: false,
                aut: Aut::Held,
                sub: Some("ručne do štartu".into()),
                low: false
            }
        );
        // Unknown transport or mute: the pill runs, no line.
        assert_eq!(
            stage_look(Some(true), None, Some(true), false),
            RailLook {
                lit: false,
                aut: Aut::Run,
                sub: None,
                low: false
            }
        );
        assert_eq!(
            stage_look(Some(true), Some(true), None, false),
            RailLook {
                lit: false,
                aut: Aut::Run,
                sub: None,
                low: false
            }
        );
    }

    #[test]
    fn a_hold_in_progress_replaces_the_line() {
        assert_eq!(
            stage_look(Some(false), Some(true), Some(true), true)
                .sub
                .as_deref(),
            Some("drž… zapína AUT")
        );
        assert_eq!(
            stage_look(Some(true), Some(true), Some(true), true)
                .sub
                .as_deref(),
            Some("drž… vypína AUT")
        );
        assert_eq!(
            stage_look(None, None, None, true).sub.as_deref(),
            Some("drž… zapína AUT")
        );
        // The pill and the light stay what they were.
        let look = stage_look(Some(true), Some(true), Some(false), true);
        assert_eq!((look.aut, look.lit), (Aut::Held, true));
    }
}
