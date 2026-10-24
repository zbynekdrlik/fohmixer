//! AUTOTUNE on the surface (#104, D19; the approved mockup
//! `docs/mockups/autotune-v1.html`): the rail button's look. Pure.
//!
//! A tap switches autotune on or off on every `+AUTOTUNE` track at once
//! (the hub writes each `Latencies` rack's first macro), a 1 s hold turns
//! AUTOTUNE AUT on or off at its lift (the press is `behave::rail`'s, as
//! the merged STAGE button's). The button is lit in the former toggle's
//! orange while every found track is on, dark while every one is off, and
//! mixed (dark, an orange ring, half its light, the count on the second
//! line) while some are on and some off; a tap on mixed turns every one
//! off (the linked mute's rule: a partial state goes dark). With AUTOTUNE
//! AUT on it wears the `AUT` pill, outlined while Live's autotune is not
//! the transport's (a tap's override until the next stop or start, or a
//! song's own switch), and the second line says what the rule does now. A
//! track the hub found no rack for shows the yellow `!` corner while the
//! others work; with none found the button is red. Everything here reads
//! the hub's `AutotuneState`.

use fohmixer_proto::client::{AutotuneState, TargetState};

use super::rail::{Aut, Fill, RailLook, full_text, hold_text};

/// What the button shows of the targets the hub found with a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    /// Every one on: lit.
    On,
    /// Every one off: dark.
    Off,
    /// `on` of `of` on: mixed.
    Mixed { on: usize, of: usize },
}

impl Shown {
    /// Autotune on (`true`) or off on every target.
    fn of(on: bool) -> Self {
        if on { Self::On } else { Self::Off }
    }
}

/// What the found targets with a value show (`Waiting` and `Missing` ones
/// left out); none while none has one.
pub fn shown(state: &AutotuneState) -> Option<Shown> {
    let count = |want: TargetState| state.targets.iter().filter(|t| t.state == want).count();
    match (count(TargetState::On), count(TargetState::Off)) {
        (0, 0) => None,
        (_, 0) => Some(Shown::On),
        (0, _) => Some(Shown::Off),
        (on, off) => Some(Shown::Mixed { on, of: on + off }),
    }
}

/// The button's `data-state`: `on`, `off`, `mixed`, or `waiting` before
/// the hub's state and while no found target has a value.
pub fn state_name(state: Option<&AutotuneState>) -> &'static str {
    match state.and_then(shown) {
        None => "waiting",
        Some(Shown::On) => "on",
        Some(Shown::Off) => "off",
        Some(Shown::Mixed { .. }) => "mixed",
    }
}

/// Whether the button takes a tap: none before the hub's state, and while
/// nothing is shown but a target still waits (its walk, its value, its
/// Live set offline); `Some(false)` without a found target: no `+AUTOTUNE`
/// track, or none with its rack (red, I5).
pub fn autotune_ready(state: Option<&AutotuneState>) -> Option<bool> {
    let s = state?;
    let waits = s.targets.iter().any(|t| t.state == TargetState::Waiting);
    match (shown(s), waits) {
        (Some(_), _) => Some(true),
        (None, true) => None,
        (None, false) => Some(false),
    }
}

/// What a tap asks the hub for (`autotune`): every target off while the
/// button is lit or mixed (the linked mute's rule: a partial state goes
/// dark), on while dark, as it shows the state the hub told this connection
/// (`fresh`), only while the page is connected; none: the tap is refused.
pub fn tap_value(fresh: Option<&AutotuneState>, connected: bool) -> Option<bool> {
    fresh
        .and_then(shown)
        .filter(|_| connected)
        .map(|s| s == Shown::Off)
}

/// Whether a target has no rack to switch while others work: the yellow
/// `!` corner (with none found the button is red instead).
pub fn has_problem(state: Option<&AutotuneState>) -> bool {
    state.is_some_and(|s| {
        shown(s).is_some() && s.targets.iter().any(|t| t.state == TargetState::Missing)
    })
}

/// The AUTOTUNE button (D19; the mockup's screen 1): lit while every found
/// target is on, mixed while some are, dark else; with AUT on the pill
/// runs, outlined while what Live shows is not the transport's once the
/// transport has settled (`settling`: the hub's write is on its way, as
/// STAGE's mics), and the second line says what the rule does; the mixed
/// count shows with AUT off too; a press's fill (`behave::rail::fill_of`)
/// shows its own line.
pub fn autotune_look(state: Option<&AutotuneState>, fill: Fill, settling: bool) -> RailLook {
    let Some(s) = state else {
        return RailLook {
            lit: false,
            aut: Aut::Off,
            sub: None,
            low: false,
            mixed: false,
        };
    };
    let live = shown(s);
    // The transport the rule follows (AUT on, heard), and what the button
    // reads against it: Live's autotune, or the rule's own while the
    // transport settles.
    let rule = s.playing.filter(|_| s.aut);
    let seen = match (rule, settling) {
        (Some(playing), true) => Some(Shown::of(playing)),
        _ => live,
    };
    let aut = match (s.aut, rule, seen) {
        (false, _, _) => Aut::Off,
        (true, Some(playing), Some(seen)) if seen != Shown::of(playing) => Aut::Held,
        (true, _, _) => Aut::Run,
    };
    let sub = match fill {
        Fill::Filling => Some(hold_text(s.aut).to_string()),
        Fill::Full => Some(full_text(s.aut).to_string()),
        Fill::None => autotune_sub(s.aut, rule, seen),
    };
    RailLook {
        lit: live == Some(Shown::On),
        aut,
        sub,
        low: false,
        mixed: matches!(live, Some(Shown::Mixed { .. })),
    }
}

/// What AUTOTUNE does now, in plain words: with AUT on and the transport
/// heard, the rule's line; else the mixed count; none while it says
/// nothing (on or off by hand, the transport not yet heard).
fn autotune_sub(aut: bool, rule: Option<bool>, seen: Option<Shown>) -> Option<String> {
    match (rule, seen) {
        (Some(playing), Some(seen)) => Some(rule_sub(playing, seen)),
        (None, Some(Shown::Mixed { on, of })) => Some(count_text(aut, on, of)),
        _ => None,
    }
}

/// The rule's line: Live plays and autotune is on, or stands and it is
/// off; a tap the other way holds until the next stop or start.
fn rule_sub(playing: bool, seen: Shown) -> String {
    match (playing, seen) {
        (_, Shown::Mixed { on, of }) => count_text(true, on, of),
        (true, Shown::On) => "hrá · zapnutý".to_string(),
        (false, Shown::Off) => "stojí · vypnutý".to_string(),
        (true, Shown::Off) => "ručne vyp. · do zast.".to_string(),
        (false, Shown::On) => "ručne zap. · do štartu".to_string(),
    }
}

/// The mixed count (`2/10 zap.`), `zmiešané` first with AUT on.
fn count_text(aut: bool, on: usize, of: usize) -> String {
    if aut {
        format!("zmiešané · {on}/{of} zap.")
    } else {
        format!("{on}/{of} zap.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::client::AutotuneTargetState;
    use fohmixer_proto::client::TargetState::{Missing, Off, On, Waiting};

    /// A state of targets `#1`, `#2`, … in these states.
    fn state(aut: bool, playing: Option<bool>, states: &[TargetState]) -> AutotuneState {
        AutotuneState {
            aut,
            playing,
            targets: states
                .iter()
                .enumerate()
                .map(|(i, state)| AutotuneTargetState {
                    label: format!("#{}", i + 1),
                    state: *state,
                })
                .collect(),
            song: None,
            key: None,
            checking: false,
            singers: Vec::new(),
            map_problems: 0,
            map_changes: 0,
            map_keyed: 0,
            bar: None,
        }
    }

    fn look(s: &AutotuneState) -> RailLook {
        autotune_look(Some(s), Fill::None, false)
    }

    #[test]
    fn it_is_lit_only_when_every_found_target_is_on() {
        assert_eq!(shown(&state(false, None, &[On, On])), Some(Shown::On));
        assert_eq!(shown(&state(false, None, &[Off, Off])), Some(Shown::Off));
        assert_eq!(
            shown(&state(false, None, &[On, Off, On])),
            Some(Shown::Mixed { on: 2, of: 3 })
        );
        // Waiting and missing targets are left out of the count.
        assert_eq!(
            shown(&state(false, None, &[On, Waiting, Missing])),
            Some(Shown::On)
        );
        assert_eq!(
            shown(&state(false, None, &[Waiting, Off, Missing])),
            Some(Shown::Off)
        );
        assert_eq!(
            shown(&state(false, None, &[On, Waiting, Off, Missing])),
            Some(Shown::Mixed { on: 1, of: 2 })
        );
        assert_eq!(shown(&state(false, None, &[Waiting, Missing])), None);
        assert_eq!(shown(&state(false, None, &[])), None);
        // Lit only when every one is on; mixed is neither lit nor dark.
        let on = look(&state(false, None, &[On, On]));
        assert_eq!((on.lit, on.mixed), (true, false));
        let off = look(&state(false, None, &[Off, Off]));
        assert_eq!((off.lit, off.mixed), (false, false));
        let mixed = look(&state(false, None, &[On, Off, On]));
        assert_eq!((mixed.lit, mixed.mixed), (false, true));
        // Its `data-state`.
        assert_eq!(state_name(None), "waiting");
        assert_eq!(state_name(Some(&state(false, None, &[Waiting]))), "waiting");
        assert_eq!(state_name(Some(&state(false, None, &[On]))), "on");
        assert_eq!(state_name(Some(&state(false, None, &[Off]))), "off");
        assert_eq!(state_name(Some(&state(false, None, &[On, Off]))), "mixed");
    }

    #[test]
    fn mixed_counts_the_targets_on() {
        let s = state(false, None, &[On, Off, On, Off, Off]);
        assert_eq!(shown(&s), Some(Shown::Mixed { on: 2, of: 5 }));
        assert_eq!(look(&s).sub.as_deref(), Some("2/5 zap."));
        let lone = state(false, None, &[Off, Off, On]);
        assert_eq!(look(&lone).sub.as_deref(), Some("1/3 zap."));
        // With AUT on, `zmiešané` first.
        let aut = state(true, None, &[On, Off, On, Off, Off]);
        assert_eq!(look(&aut).sub.as_deref(), Some("zmiešané · 2/5 zap."));
    }

    #[test]
    fn a_tap_turns_every_target_off_when_on_or_mixed_and_on_when_off() {
        let on = state(false, None, &[On, On]);
        let off = state(false, None, &[Off, Off]);
        let mixed = state(false, None, &[On, Off]);
        assert_eq!(tap_value(Some(&on), true), Some(false));
        assert_eq!(tap_value(Some(&mixed), true), Some(false));
        assert_eq!(tap_value(Some(&off), true), Some(true));
        // Waiting and missing targets do not count.
        let hand = state(true, Some(true), &[Off, Missing, Waiting]);
        assert_eq!(tap_value(Some(&hand), true), Some(true));
    }

    #[test]
    fn a_tap_is_refused_before_the_hubs_state_of_this_connection_or_offline() {
        let on = state(false, None, &[On]);
        assert_eq!(tap_value(Some(&on), false), None, "not connected");
        assert_eq!(tap_value(None, true), None, "no state on this connection");
        assert_eq!(tap_value(None, false), None);
        let none = state(false, None, &[Waiting, Missing]);
        assert_eq!(tap_value(Some(&none), true), None, "nothing shown");
        assert_eq!(tap_value(Some(&state(false, None, &[])), true), None);
    }

    #[test]
    fn it_is_waiting_before_the_state_and_red_without_a_found_target() {
        assert_eq!(autotune_ready(None), None);
        assert_eq!(autotune_ready(Some(&state(false, None, &[]))), Some(false));
        assert_eq!(
            autotune_ready(Some(&state(false, None, &[Missing, Missing]))),
            Some(false),
            "no rack found"
        );
        assert_eq!(
            autotune_ready(Some(&state(false, None, &[Waiting, Missing]))),
            None,
            "a target still waits"
        );
        assert_eq!(autotune_ready(Some(&state(false, None, &[Waiting]))), None);
        assert_eq!(autotune_ready(Some(&state(false, None, &[On]))), Some(true));
        assert_eq!(
            autotune_ready(Some(&state(false, None, &[Off, Missing]))),
            Some(true)
        );
        assert_eq!(
            autotune_ready(Some(&state(false, None, &[Waiting, On]))),
            Some(true)
        );
        // Before the hub's state: dark, nothing.
        assert_eq!(
            autotune_look(None, Fill::None, false),
            RailLook {
                lit: false,
                aut: Aut::Off,
                sub: None,
                low: false,
                mixed: false,
            }
        );
        assert_eq!(autotune_look(None, Fill::Full, false).sub, None);
    }

    #[test]
    fn a_missing_target_shows_the_problem_corner() {
        assert!(has_problem(Some(&state(false, None, &[On, Missing]))));
        assert!(has_problem(Some(&state(
            true,
            Some(true),
            &[Missing, Off, On]
        ))));
        assert!(!has_problem(Some(&state(false, None, &[On, Off]))));
        assert!(!has_problem(Some(&state(false, None, &[On, Waiting]))));
        // No found target: red, no corner.
        assert!(!has_problem(Some(&state(false, None, &[Missing]))));
        assert!(!has_problem(Some(&state(false, None, &[Missing, Waiting]))));
        assert!(!has_problem(None));
    }

    #[test]
    fn the_second_lines_say_what_the_rule_does() {
        let line = |playing, states: &[TargetState]| look(&state(true, Some(playing), states));
        assert_eq!(
            line(true, &[On, On]),
            RailLook {
                lit: true,
                aut: Aut::Run,
                sub: Some("hrá · zapnutý".into()),
                low: false,
                mixed: false,
            }
        );
        assert_eq!(
            line(false, &[Off, Off]),
            RailLook {
                lit: false,
                aut: Aut::Run,
                sub: Some("stojí · vypnutý".into()),
                low: false,
                mixed: false,
            }
        );
        // A tap the other way: outlined until the next stop or start.
        assert_eq!(
            line(true, &[Off, Off]),
            RailLook {
                lit: false,
                aut: Aut::Held,
                sub: Some("ručne vyp. · do zast.".into()),
                low: false,
                mixed: false,
            }
        );
        assert_eq!(
            line(false, &[On, On]),
            RailLook {
                lit: true,
                aut: Aut::Held,
                sub: Some("ručne zap. · do štartu".into()),
                low: false,
                mixed: false,
            }
        );
        // Mixed is no transport's state: outlined, the count.
        assert_eq!(
            line(true, &[On, Off]),
            RailLook {
                lit: false,
                aut: Aut::Held,
                sub: Some("zmiešané · 1/2 zap.".into()),
                low: false,
                mixed: true,
            }
        );
        assert_eq!(
            line(false, &[Off, On, On]),
            RailLook {
                lit: false,
                aut: Aut::Held,
                sub: Some("zmiešané · 2/3 zap.".into()),
                low: false,
                mixed: true,
            }
        );
        // AUT off: no pill, and no line but the mixed count.
        for playing in [Some(true), Some(false), None] {
            let on = look(&state(false, playing, &[On, On]));
            assert_eq!((on.aut, on.sub), (Aut::Off, None));
            let off = look(&state(false, playing, &[Off]));
            assert_eq!((off.aut, off.sub), (Aut::Off, None));
            let mixed = look(&state(false, playing, &[On, Off]));
            assert_eq!(
                (mixed.aut, mixed.sub.as_deref()),
                (Aut::Off, Some("1/2 zap."))
            );
        }
        // AUT on, the transport not heard yet: the pill runs, no line but
        // the mixed count.
        let unheard = look(&state(true, None, &[On, On]));
        assert_eq!((unheard.aut, unheard.sub), (Aut::Run, None));
        let unheard_mixed = look(&state(true, None, &[Off, On]));
        assert_eq!(
            (unheard_mixed.aut, unheard_mixed.sub.as_deref()),
            (Aut::Run, Some("zmiešané · 1/2 zap."))
        );
        // AUT on, nothing shown yet: the pill runs, no line.
        let waiting = look(&state(true, Some(true), &[Waiting]));
        assert_eq!((waiting.aut, waiting.sub), (Aut::Run, None));
    }

    #[test]
    fn the_pill_is_outlined_while_live_disagrees_with_the_transport_after_settling() {
        // The transport just changed: Live reads as the rule sets it (its
        // write is on its way), the light as it is.
        let settling = |playing, states: &[TargetState]| {
            autotune_look(Some(&state(true, Some(playing), states)), Fill::None, true)
        };
        assert_eq!(
            settling(true, &[Off, Off]),
            RailLook {
                lit: false,
                aut: Aut::Run,
                sub: Some("hrá · zapnutý".into()),
                low: false,
                mixed: false,
            }
        );
        assert_eq!(
            settling(false, &[On, On]),
            RailLook {
                lit: true,
                aut: Aut::Run,
                sub: Some("stojí · vypnutý".into()),
                low: false,
                mixed: false,
            }
        );
        let mixed = settling(true, &[On, Off]);
        assert_eq!(
            (mixed.aut, mixed.sub.as_deref(), mixed.mixed),
            (Aut::Run, Some("hrá · zapnutý"), true)
        );
        // A match reads the same settled or not.
        for (playing, states) in [(true, [On, On]), (false, [Off, Off])] {
            assert_eq!(
                settling(playing, &states),
                look(&state(true, Some(playing), &states))
            );
        }
        // Without AUT or with the transport unknown: as before.
        assert_eq!(
            autotune_look(Some(&state(false, Some(true), &[Off])), Fill::None, true),
            look(&state(false, Some(true), &[Off]))
        );
        assert_eq!(
            autotune_look(Some(&state(true, None, &[On, Off])), Fill::None, true),
            look(&state(true, None, &[On, Off]))
        );
    }

    #[test]
    fn a_press_shows_its_hold_and_full_lines_and_keeps_the_rest() {
        let playing = state(true, Some(true), &[On]);
        let off = state(false, None, &[On, Off]);
        let line = |s: &AutotuneState, fill| autotune_look(Some(s), fill, false).sub;
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
        // The light, the pill and the ring stay what they were.
        let held = autotune_look(
            Some(&state(true, Some(true), &[On, Off])),
            Fill::Full,
            false,
        );
        assert_eq!((held.lit, held.aut, held.mixed), (false, Aut::Held, true));
        let filling = autotune_look(Some(&playing), Fill::Filling, false);
        assert_eq!(
            (filling.lit, filling.aut, filling.mixed),
            (true, Aut::Run, false)
        );
    }
}
