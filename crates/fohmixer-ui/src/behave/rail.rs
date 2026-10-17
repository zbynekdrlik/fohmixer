//! The rail buttons with an automatic mode (#104, #130, D18; the approved
//! mockup `docs/mockups/reverb-link-v1.html`, screen 2): a tap on the
//! finger's lift does what the button did by hand, a 3 s hold turns the
//! automatic mode on or off. While it is on, the button wears an `AUT`
//! pill, outlined while a tap overrides the rule until the next stop or
//! start, and a second line says in plain words what the rule does now.
//!
//! The press's own decisions are here too (#104 review I1, M3, M4, M5): a
//! press whose end never came ends at the next primary down anywhere on the
//! page (`stale_press`); a hold acts only while the page is connected and
//! the hub's flag of this connection is known (`hold_value`), else it shows
//! no fill and is refused; a mismatch of the transport and the mics reads
//! as a tap's override only once it outlasts the transport's last change by
//! `SETTLE_MS` (`settling`).

use super::Down;
use super::hold::Hold;
use crate::diag::perf::Pointer;

/// How long a rail button's press lasts before it turns its automatic mode
/// on or off.
pub const RAIL_HOLD_MS: f64 = 3000.0;

/// How long after a change of the transport a mismatch of the mics still
/// reads as the rule at work (the hub's write is on its way, one Live round
/// trip), not as a tap's override.
pub const SETTLE_MS: f64 = 500.0;

/// A rail button's press.
pub fn rail_press() -> Hold {
    Hold::with(RAIL_HOLD_MS)
}

/// The press a pointer's down anywhere on the page ends with nothing done
/// (`lost`): `pressing` the press's pointer, `kind` its type. A press whose
/// end never came (#124: iOS can take a finger without an end) ends at the
/// next primary down of its type (`Down::ends`: a mouse or a pen never ends
/// a finger's press), or when its own pointer goes down again; a second
/// finger (not primary) leaves a real hold alone.
pub fn stale_press(pressing: Option<i32>, kind: Option<Pointer>, down: Down) -> Option<i32> {
    pressing.filter(|old| down.ends(kind) || *old == down.id)
}

/// Whether a cancel or a lost capture of `pointer` ends the press with a
/// `cancel` step: only the press's own pointer.
pub fn cancels(pressing: Option<i32>, pointer: i32) -> bool {
    pressing == Some(pointer)
}

/// What a hold sends: the automatic mode the other way, only while the
/// page is connected and the hub's flag (of this connection) is known;
/// none: the hold is refused.
pub fn hold_value(aut: Option<bool>, connected: bool) -> Option<bool> {
    aut.filter(|_| connected).map(|on| !on)
}

/// Whether a press shows the hold's fill and line: a press down that can
/// act at its hold.
pub fn shows_hold(pressing: bool, aut: Option<bool>, connected: bool) -> bool {
    pressing && hold_value(aut, connected).is_some()
}

/// The flight recorder's step of a press that lasted its hold: `hold`, or
/// `refused` when it could not act.
pub fn hold_step(value: Option<bool>) -> &'static str {
    if value.is_some() { "hold" } else { "refused" }
}

/// Whether a new transport value is a change: from one known value to the
/// other (a first value, or one after an outage that kept it, is none).
pub fn transport_changed(before: Option<bool>, now: Option<bool>) -> bool {
    matches!((before, now), (Some(before), Some(now)) if before != now)
}

/// Whether the transport's last change (`changed_at`, the page clock; none
/// while none is unsettled) is younger than `SETTLE_MS` at `now`.
pub fn settling(changed_at: Option<f64>, now: f64) -> bool {
    changed_at.is_some_and(|at| now - at < SETTLE_MS)
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
/// before it is known), `holding` a press filling it (`shows_hold`),
/// `settling` the transport changed less than `SETTLE_MS` ago: the mics are
/// read as the rule is setting them, so its write on the way never flashes
/// the override's look.
pub fn stage_look(
    aut: Option<bool>,
    playing: Option<bool>,
    muted: Option<bool>,
    holding: bool,
    settling: bool,
) -> RailLook {
    let on = aut == Some(true);
    let known = playing
        .zip(muted)
        .map(|(playing, muted)| (playing, if settling { playing } else { muted }));
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
        assert!(stage_look(None, None, Some(false), false, false).lit);
        assert!(!stage_look(None, None, Some(true), false, false).lit);
        assert!(
            !stage_look(None, None, None, false, false).lit,
            "unknown stays dark"
        );
        assert!(stage_look(Some(true), Some(true), Some(false), false, false).lit);
        assert!(!stage_look(Some(true), Some(false), Some(true), false, false).lit);
        assert!(!stage_look(Some(true), Some(true), Some(false), false, false).low);
        assert!(!stage_look(Some(true), Some(false), Some(false), true, false).low);
    }

    #[test]
    fn without_aut_there_is_no_pill_and_no_second_line() {
        for (playing, muted) in [
            (Some(true), Some(true)),
            (Some(false), Some(false)),
            (Some(true), Some(false)),
            (None, None),
        ] {
            let look = stage_look(Some(false), playing, muted, false, false);
            assert_eq!((look.aut, look.sub), (Aut::Off, None));
            let unknown = stage_look(None, playing, muted, false, false);
            assert_eq!((unknown.aut, unknown.sub), (Aut::Off, None));
        }
    }

    #[test]
    fn with_aut_the_pill_runs_and_the_line_says_what_the_rule_does() {
        let line =
            |playing, muted| stage_look(Some(true), Some(playing), Some(muted), false, false);
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
            stage_look(Some(true), None, Some(true), false, false),
            RailLook {
                lit: false,
                aut: Aut::Run,
                sub: None,
                low: false
            }
        );
        assert_eq!(
            stage_look(Some(true), Some(true), None, false, false),
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
            stage_look(Some(false), Some(true), Some(true), true, false)
                .sub
                .as_deref(),
            Some("drž… zapína AUT")
        );
        assert_eq!(
            stage_look(Some(true), Some(true), Some(true), true, false)
                .sub
                .as_deref(),
            Some("drž… vypína AUT")
        );
        assert_eq!(
            stage_look(None, None, None, true, false).sub.as_deref(),
            Some("drž… zapína AUT")
        );
        // The pill and the light stay what they were.
        let look = stage_look(Some(true), Some(true), Some(false), true, false);
        assert_eq!((look.aut, look.lit), (Aut::Held, true));
    }

    #[test]
    fn a_settling_mismatch_reads_as_the_rule_at_work() {
        // The transport just changed: the mics read as the rule sets them.
        let just_changed =
            |playing, muted| stage_look(Some(true), Some(playing), Some(muted), false, true);
        assert_eq!(
            just_changed(true, false),
            RailLook {
                lit: true,
                aut: Aut::Run,
                sub: Some("hrá · stíšené".into()),
                low: false
            }
        );
        assert_eq!(
            just_changed(false, true),
            RailLook {
                lit: false,
                aut: Aut::Run,
                sub: Some("stojí · zapnuté".into()),
                low: false
            }
        );
        // A match reads the same settled or not.
        for (playing, muted) in [(true, true), (false, false)] {
            assert_eq!(
                just_changed(playing, muted),
                stage_look(Some(true), Some(playing), Some(muted), false, false)
            );
        }
        // Without AUT, unknown or while a hold fills it: as before.
        assert_eq!(
            stage_look(Some(false), Some(true), Some(false), false, true).aut,
            Aut::Off
        );
        assert_eq!(
            stage_look(Some(true), None, Some(false), false, true).sub,
            None
        );
        assert_eq!(
            stage_look(Some(true), Some(true), Some(false), true, true)
                .sub
                .as_deref(),
            Some("drž… vypína AUT")
        );
    }

    #[test]
    fn the_transport_settles_500_ms_after_its_change() {
        assert_eq!(SETTLE_MS, 500.0);
        assert!(settling(Some(1000.0), 1000.0));
        assert!(settling(Some(1000.0), 1499.99));
        assert!(settling(Some(1000.0), 1500.0_f64.next_down()));
        assert!(!settling(Some(1000.0), 1500.0));
        assert!(!settling(Some(1000.0), 2400.0));
        assert!(!settling(None, 1000.0), "no change unsettled");
        // A change is from one known value to the other.
        assert!(transport_changed(Some(false), Some(true)));
        assert!(transport_changed(Some(true), Some(false)));
        assert!(!transport_changed(Some(true), Some(true)));
        assert!(!transport_changed(Some(false), Some(false)));
        assert!(!transport_changed(None, Some(true)), "a first value");
        assert!(!transport_changed(Some(true), None), "a value gone");
        assert!(!transport_changed(None, None));
    }

    #[test]
    fn a_primary_down_anywhere_or_its_own_pointer_ends_a_stale_press() {
        let finger = Some(Pointer::Touch);
        let mouse = |id, primary| Down {
            id,
            kind: Some(Pointer::Mouse),
            primary,
        };
        // Another finger: only a primary one ends the press (a second
        // finger during a real hold does not).
        assert_eq!(stale_press(Some(5), finger, Down::touch(6, true)), Some(5));
        assert_eq!(stale_press(Some(5), finger, Down::touch(6, false)), None);
        // A mouse never ends a finger's press, and ends a mouse's.
        assert_eq!(stale_press(Some(5), finger, mouse(1, true)), None);
        assert_eq!(
            stale_press(Some(1), Some(Pointer::Mouse), mouse(1, true)),
            Some(1)
        );
        assert_eq!(
            stale_press(Some(2), Some(Pointer::Mouse), mouse(3, true)),
            Some(2)
        );
        // Its own pointer down again: the old press is over either way.
        assert_eq!(stale_press(Some(5), finger, Down::touch(5, false)), Some(5));
        assert_eq!(stale_press(Some(5), finger, Down::touch(5, true)), Some(5));
        assert_eq!(stale_press(None, finger, Down::touch(5, true)), None);
        // A cancel or a lost capture: the press's own pointer only.
        assert!(cancels(Some(3), 3));
        assert!(!cancels(Some(3), 4));
        assert!(!cancels(None, 3));
    }

    #[test]
    fn a_hold_acts_only_connected_with_the_hubs_flag_known() {
        assert_eq!(hold_value(Some(true), true), Some(false));
        assert_eq!(hold_value(Some(false), true), Some(true));
        assert_eq!(hold_value(None, true), None, "the flag not known");
        assert_eq!(hold_value(Some(true), false), None, "not connected");
        assert_eq!(hold_value(Some(false), false), None);
        assert_eq!(hold_value(None, false), None);
        // The fill and the hold's line only for a press that can act.
        assert!(shows_hold(true, Some(false), true));
        assert!(shows_hold(true, Some(true), true));
        assert!(!shows_hold(false, Some(true), true));
        assert!(!shows_hold(true, None, true));
        assert!(!shows_hold(true, Some(true), false));
        // The recorder's step at the hold.
        assert_eq!(hold_step(Some(true)), "hold");
        assert_eq!(hold_step(Some(false)), "hold");
        assert_eq!(hold_step(None), "refused");
    }
}
