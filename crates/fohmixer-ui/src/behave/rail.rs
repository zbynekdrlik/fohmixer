//! The rail buttons with an automatic mode (#104, #130, D18; the approved
//! mockup `docs/mockups/reverb-link-v1.html`, screen 2): a tap on the
//! finger's lift does what the button did by hand, a 1 s hold turns the
//! automatic mode on or off at its lift (the owner's ruling on #104: 1 s,
//! not the mockup's 3 s). While it is on, the button wears an `AUT`
//! pill, outlined while a tap overrides the rule until the next stop or
//! start, and a second line says in plain words what the rule does now.
//!
//! The press's own decisions are here too (#104 review I1, M3, M4, M5; the
//! owner's ruling on #104 of 2026-10-17): the hold acts at the LIFT once
//! the fill is full (`rail_press`, `Hold::lifted`): at `RAIL_HOLD_MS` the
//! press is full (`full_text`, recorded `full`, nothing sent), a lift then
//! acts (`hold`) and a slide, a cancel, a lost capture, a hidden page or a
//! stale press ends it with nothing done, so a lost end never acts. A press
//! whose end never came ends at the next primary down of its type anywhere
//! on the page (`stale_press`); a hold acts only while the page is connected
//! and the hub's flag of this connection is known (`hold_value`), else it
//! shows no fill and is refused (`full_step`, `hold_step`); a mismatch of
//! the transport and the mics reads as a tap's override only once it
//! outlasts the transport's last change by `SETTLE_MS` (`settling`).

use fohmixer_proto::client::{HUB_AUTOTUNE_AUT, HUB_REVERB_AUT, HUB_STAGE_AUT};

use super::Down;
use super::hold::Hold;
use crate::diag::perf::Pointer;

/// How long a rail button's press lasts before it is full (its lift then
/// turns its automatic mode on or off; the owner's ruling on #104: 1 s).
pub const RAIL_HOLD_MS: f64 = 1000.0;

/// How long after a change of the transport a mismatch of the mics still
/// reads as the rule at work (the hub's write is on its way, one Live round
/// trip), not as a tap's override.
pub const SETTLE_MS: f64 = 500.0;

/// The rule a rail button carries (#104, #130): what tells the buttons
/// apart where they share the press, the pill and the second line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    /// The merged STAGE button: the stage mics' mute and STAGE AUT.
    Stage,
    /// REVERB (#130): the reverb faders lowered or back, and REVERB AUT.
    Reverb,
    /// AUTOTUNE (#104, D19): every `+AUTOTUNE` track's autotune, and
    /// AUTOTUNE AUT.
    Autotune,
}

impl Rule {
    /// The flight recorder's `button` of its `rail` steps.
    pub fn button(self) -> &'static str {
        match self {
            Self::Stage => "stage",
            Self::Reverb => "reverb",
            Self::Autotune => "autotune",
        }
    }

    /// The hub value its hold turns on or off.
    pub fn aut_key(self) -> &'static str {
        match self {
            Self::Stage => HUB_STAGE_AUT,
            Self::Reverb => HUB_REVERB_AUT,
            Self::Autotune => HUB_AUTOTUNE_AUT,
        }
    }

    /// Its root's classes (`rule.css`).
    pub fn class(self) -> &'static str {
        match self {
            Self::Stage => "btn stage-mics rule-btn",
            Self::Reverb => "btn reverb-btn rule-btn",
            Self::Autotune => "btn autotune-btn rule-btn",
        }
    }

    /// Its root's `data-testid`.
    pub fn testid(self) -> &'static str {
        match self {
            Self::Stage => "stage-mics",
            Self::Reverb => "reverb",
            Self::Autotune => "autotune",
        }
    }

    /// Why a press it refuses did nothing (the failure flash's words).
    pub fn refusal(self) -> &'static str {
        match self {
            Self::Stage => "STAGE AUTO cannot be switched now",
            Self::Reverb => "REVERB cannot be switched now",
            Self::Autotune => "AUTOTUNE cannot be switched now",
        }
    }
}

/// A rail button's press: full at `RAIL_HOLD_MS`, acting at its lift.
pub fn rail_press() -> Hold {
    Hold::lifted(RAIL_HOLD_MS)
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

/// The earlier pointer a press's restart dropped, recorded `lost`: a down of
/// another pointer that `Hold::down` took over a press still down (a
/// primary down of another type, which `stale_press` leaves alone; #130
/// re-review N2).
pub fn restarted(before: Option<i32>, down: i32) -> Option<i32> {
    before.filter(|old| *old != down)
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

/// The automatic mode a hold may act on: the hub's (`aut`) only while the
/// button is ready (PR #150 review M2: a red or waiting REVERB or AUTOTUNE
/// takes no hold, the mockup's cell 13); none: the press shows no fill and
/// is refused.
pub fn ready_hold(aut: Option<bool>, ready: bool) -> Option<bool> {
    aut.filter(|_| ready)
}

/// What a rail press shows: nothing, the fill growing, or the fill full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fill {
    None,
    Filling,
    Full,
}

impl Fill {
    /// Its `data-holding`: a fill shows (growing or full).
    pub fn holding(self) -> bool {
        self != Fill::None
    }

    /// Its `data-full`.
    pub fn full(self) -> bool {
        self == Fill::Full
    }
}

/// The fill of a press `filling` (down, before its hold) or `full` (past
/// it): only a press that can act shows one (`hold_value`).
pub fn fill_of(filling: bool, full: bool, aut: Option<bool>, connected: bool) -> Fill {
    match (hold_value(aut, connected), full, filling) {
        (None, _, _) => Fill::None,
        (Some(_), true, _) => Fill::Full,
        (Some(_), false, true) => Fill::Filling,
        (Some(_), false, false) => Fill::None,
    }
}

/// The flight recorder's step at a press's full mark: `full` (nothing is
/// sent yet), or `refused` when it could not act (the press is dead).
pub fn full_step(value: Option<bool>) -> &'static str {
    if value.is_some() { "full" } else { "refused" }
}

/// The flight recorder's step of a lift that acts: `hold`, or `refused`
/// when it could not act.
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

/// The pill's word, shared by every rail button with an automatic mode and
/// by the texts that name it (#140 PR 3c: AUTOTUNE's panel; #141: the
/// console word, AUTO).
pub const AUT_BADGE: &str = "AUTO";

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
    /// Neither lit nor dark (#104, AUTOTUNE): some targets on, some off;
    /// the button dark with a ring in its colour and half its light.
    pub mixed: bool,
}

/// The second line while the press is full: what its lift will do.
pub fn full_text(aut_on: bool) -> &'static str {
    if aut_on {
        "release — AUTO off"
    } else {
        "release — AUTO on"
    }
}

/// The second line while a hold fills the button.
pub fn hold_text(aut_on: bool) -> &'static str {
    if aut_on {
        "hold… AUTO off"
    } else {
        "hold… AUTO on"
    }
}

/// The merged STAGE button (#104): `aut` the hub's STAGE AUT flag,
/// `playing` its Live's transport, `muted` the stage mics' mute (each none
/// before it is known), `fill` the press's (`fill_of`),
/// `settling` the transport changed less than `SETTLE_MS` ago: the mics are
/// read as the rule is setting them, so its write on the way never flashes
/// the override's look.
pub fn stage_look(
    aut: Option<bool>,
    playing: Option<bool>,
    muted: Option<bool>,
    fill: Fill,
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
    let sub = match (fill, on) {
        (Fill::Filling, _) => Some(hold_text(on).to_string()),
        (Fill::Full, _) => Some(full_text(on).to_string()),
        (Fill::None, true) => known.map(|state| stage_sub(state).to_string()),
        (Fill::None, false) => None,
    };
    RailLook {
        lit: muted == Some(false),
        aut: pill,
        sub,
        low: false,
        mixed: false,
    }
}

/// What STAGE AUT does now, transport and mute known: the rule mutes the
/// mics while Live plays and opens them while it stands; a tap the other
/// way holds until the next stop or start.
fn stage_sub((playing, muted): (bool, bool)) -> &'static str {
    match (playing, muted) {
        (true, true) => "PLAY · muted",
        (false, false) => "STOP · open",
        (true, false) => "MAN · until STOP",
        (false, true) => "MAN · until PLAY",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hold_acts_only_on_a_ready_button() {
        assert_eq!(ready_hold(Some(true), true), Some(true));
        assert_eq!(ready_hold(Some(false), true), Some(false));
        assert_eq!(ready_hold(Some(true), false), None);
        assert_eq!(ready_hold(None, true), None);
        // Not ready: no fill, the full mark refused.
        assert_eq!(
            fill_of(true, false, ready_hold(Some(false), false), true),
            Fill::None
        );
        assert_eq!(hold_value(ready_hold(Some(false), false), true), None);
    }

    #[test]
    fn the_pill_says_auto() {
        assert_eq!(AUT_BADGE, "AUTO");
    }

    #[test]
    fn the_rail_holds_for_one_second() {
        assert_eq!(RAIL_HOLD_MS, 1000.0);
        let mut press = rail_press();
        assert!(press.down(1, true, 0.0));
        assert!(!press.open(1000.0_f64.next_down()));
        assert!(press.open(1000.0));
        assert_eq!(rail_press(), Hold::lifted(1000.0));
    }

    #[test]
    fn the_hold_says_what_it_will_do() {
        assert_eq!(hold_text(false), "hold… AUTO on");
        assert_eq!(hold_text(true), "hold… AUTO off");
        assert_eq!(
            [Aut::Off.name(), Aut::Run.name(), Aut::Held.name()],
            ["off", "run", "held"]
        );
    }

    #[test]
    fn stage_is_lit_while_the_mics_are_live_whatever_aut_says() {
        assert!(stage_look(None, None, Some(false), Fill::None, false).lit);
        assert!(!stage_look(None, None, Some(true), Fill::None, false).lit);
        assert!(
            !stage_look(None, None, None, Fill::None, false).lit,
            "unknown stays dark"
        );
        assert!(stage_look(Some(true), Some(true), Some(false), Fill::None, false).lit);
        assert!(!stage_look(Some(true), Some(false), Some(true), Fill::None, false).lit);
        assert!(!stage_look(Some(true), Some(true), Some(false), Fill::None, false).low);
        assert!(!stage_look(Some(true), Some(false), Some(false), Fill::Filling, false).low);
    }

    #[test]
    fn without_aut_there_is_no_pill_and_no_second_line() {
        for (playing, muted) in [
            (Some(true), Some(true)),
            (Some(false), Some(false)),
            (Some(true), Some(false)),
            (None, None),
        ] {
            let look = stage_look(Some(false), playing, muted, Fill::None, false);
            assert_eq!((look.aut, look.sub), (Aut::Off, None));
            let unknown = stage_look(None, playing, muted, Fill::None, false);
            assert_eq!((unknown.aut, unknown.sub), (Aut::Off, None));
        }
    }

    #[test]
    fn with_aut_the_pill_runs_and_the_line_says_what_the_rule_does() {
        let line =
            |playing, muted| stage_look(Some(true), Some(playing), Some(muted), Fill::None, false);
        assert_eq!(
            line(true, true),
            RailLook {
                lit: false,
                aut: Aut::Run,
                sub: Some("PLAY · muted".into()),
                low: false,
                mixed: false,
            }
        );
        assert_eq!(
            line(false, false),
            RailLook {
                lit: true,
                aut: Aut::Run,
                sub: Some("STOP · open".into()),
                low: false,
                mixed: false,
            }
        );
        // A tap overrides the rule until the next stop or start: outlined.
        assert_eq!(
            line(true, false),
            RailLook {
                lit: true,
                aut: Aut::Held,
                sub: Some("MAN · until STOP".into()),
                low: false,
                mixed: false,
            }
        );
        assert_eq!(
            line(false, true),
            RailLook {
                lit: false,
                aut: Aut::Held,
                sub: Some("MAN · until PLAY".into()),
                low: false,
                mixed: false,
            }
        );
        // Unknown transport or mute: the pill runs, no line.
        assert_eq!(
            stage_look(Some(true), None, Some(true), Fill::None, false),
            RailLook {
                lit: false,
                aut: Aut::Run,
                sub: None,
                low: false,
                mixed: false,
            }
        );
        assert_eq!(
            stage_look(Some(true), Some(true), None, Fill::None, false),
            RailLook {
                lit: false,
                aut: Aut::Run,
                sub: None,
                low: false,
                mixed: false,
            }
        );
    }

    #[test]
    fn a_hold_in_progress_replaces_the_line() {
        assert_eq!(
            stage_look(Some(false), Some(true), Some(true), Fill::Filling, false)
                .sub
                .as_deref(),
            Some("hold… AUTO on")
        );
        assert_eq!(
            stage_look(Some(true), Some(true), Some(true), Fill::Filling, false)
                .sub
                .as_deref(),
            Some("hold… AUTO off")
        );
        assert_eq!(
            stage_look(None, None, None, Fill::Filling, false)
                .sub
                .as_deref(),
            Some("hold… AUTO on")
        );
        // The pill and the light stay what they were.
        let look = stage_look(Some(true), Some(true), Some(false), Fill::Filling, false);
        assert_eq!((look.aut, look.lit), (Aut::Held, true));
    }

    #[test]
    fn a_settling_mismatch_reads_as_the_rule_at_work() {
        // The transport just changed: the mics read as the rule sets them.
        let just_changed =
            |playing, muted| stage_look(Some(true), Some(playing), Some(muted), Fill::None, true);
        assert_eq!(
            just_changed(true, false),
            RailLook {
                lit: true,
                aut: Aut::Run,
                sub: Some("PLAY · muted".into()),
                low: false,
                mixed: false,
            }
        );
        assert_eq!(
            just_changed(false, true),
            RailLook {
                lit: false,
                aut: Aut::Run,
                sub: Some("STOP · open".into()),
                low: false,
                mixed: false,
            }
        );
        // A match reads the same settled or not.
        for (playing, muted) in [(true, true), (false, false)] {
            assert_eq!(
                just_changed(playing, muted),
                stage_look(Some(true), Some(playing), Some(muted), Fill::None, false)
            );
        }
        // Without AUT, unknown or while a hold fills it: as before.
        assert_eq!(
            stage_look(Some(false), Some(true), Some(false), Fill::None, true).aut,
            Aut::Off
        );
        assert_eq!(
            stage_look(Some(true), None, Some(false), Fill::None, true).sub,
            None
        );
        assert_eq!(
            stage_look(Some(true), Some(true), Some(false), Fill::Filling, true)
                .sub
                .as_deref(),
            Some("hold… AUTO off")
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
        // The fill and the hold's lines only for a press that can act.
        assert_eq!(fill_of(true, false, Some(false), true), Fill::Filling);
        assert_eq!(fill_of(true, false, Some(true), true), Fill::Filling);
        assert_eq!(fill_of(false, true, Some(true), true), Fill::Full);
        assert_eq!(fill_of(false, true, Some(false), true), Fill::Full);
        assert_eq!(fill_of(false, false, Some(true), true), Fill::None);
        assert_eq!(fill_of(true, false, None, true), Fill::None);
        assert_eq!(fill_of(false, true, None, true), Fill::None);
        assert_eq!(fill_of(true, false, Some(true), false), Fill::None);
        assert_eq!(fill_of(false, true, Some(true), false), Fill::None);
        assert_eq!(
            [
                Fill::None.holding(),
                Fill::Filling.holding(),
                Fill::Full.holding()
            ],
            [false, true, true]
        );
        assert_eq!(
            [Fill::None.full(), Fill::Filling.full(), Fill::Full.full()],
            [false, false, true]
        );
        // The recorder's step at the full mark.
        assert_eq!(full_step(Some(true)), "full");
        assert_eq!(full_step(Some(false)), "full");
        assert_eq!(full_step(None), "refused");
        // The recorder's step at the hold.
        assert_eq!(hold_step(Some(true)), "hold");
        assert_eq!(hold_step(Some(false)), "hold");
        assert_eq!(hold_step(None), "refused");
    }

    #[test]
    fn a_full_press_says_what_its_lift_will_do() {
        assert_eq!(full_text(false), "release — AUTO on");
        assert_eq!(full_text(true), "release — AUTO off");
        let line = |aut, fill| stage_look(aut, Some(true), Some(true), fill, false).sub;
        assert_eq!(
            line(Some(false), Fill::Full).as_deref(),
            Some("release — AUTO on")
        );
        assert_eq!(
            line(Some(true), Fill::Full).as_deref(),
            Some("release — AUTO off")
        );
        assert_eq!(line(None, Fill::Full).as_deref(), Some("release — AUTO on"));
        assert_eq!(
            line(Some(true), Fill::Filling).as_deref(),
            Some("hold… AUTO off")
        );
        assert_eq!(
            line(Some(true), Fill::None).as_deref(),
            Some("PLAY · muted")
        );
        assert_eq!(line(Some(false), Fill::None), None);
        // The pill and the light stay what they were.
        let look = stage_look(Some(true), Some(true), Some(false), Fill::Full, false);
        assert_eq!((look.aut, look.lit), (Aut::Held, true));
    }

    #[test]
    fn each_rule_names_its_button_its_hub_value_its_look_and_its_refusal() {
        assert_eq!(
            [Rule::Stage, Rule::Reverb, Rule::Autotune].map(Rule::button),
            ["stage", "reverb", "autotune"]
        );
        assert_eq!(
            [Rule::Stage, Rule::Reverb, Rule::Autotune].map(Rule::aut_key),
            ["stage_aut", "reverb_aut", "autotune_aut"]
        );
        assert_eq!(
            [Rule::Stage, Rule::Reverb, Rule::Autotune].map(Rule::class),
            [
                "btn stage-mics rule-btn",
                "btn reverb-btn rule-btn",
                "btn autotune-btn rule-btn"
            ]
        );
        assert_eq!(
            [Rule::Stage, Rule::Reverb, Rule::Autotune].map(Rule::testid),
            ["stage-mics", "reverb", "autotune"]
        );
        assert_eq!(
            [Rule::Stage, Rule::Reverb, Rule::Autotune].map(Rule::refusal),
            [
                "STAGE AUTO cannot be switched now",
                "REVERB cannot be switched now",
                "AUTOTUNE cannot be switched now"
            ]
        );
    }

    #[test]
    fn a_restart_by_another_pointer_drops_the_earlier_one() {
        assert_eq!(restarted(Some(5), 1), Some(5));
        assert_eq!(restarted(Some(5), 5), None, "the same pointer again");
        assert_eq!(restarted(None, 5), None);
    }
}
