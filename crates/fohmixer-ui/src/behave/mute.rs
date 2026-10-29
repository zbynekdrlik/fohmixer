//! The mute button's protection (spec F12; TouchOSC `mute_button.lua`
//! 2.7.2's `double_click_mute`, the layout's `mute_guard`).
//!
//! A guarded strip's mute (its name button and the channel detail's MUTE)
//! acts by a hold (#162, the owner's request of 2026-10-29: nobody knew a
//! double tap was needed): the press is `behave::hold::Hold` in its lift
//! mode, the rail buttons' `RAIL_HOLD_MS` of 1000 ms ([`mute_press`]). The
//! fill grows across the button while held; at full the line says what the
//! lift will do ([`full_text`]) and the lift mutes or unmutes, the only
//! write. A lift before full shows a short hint ([`HOLD_HINT`]) and writes
//! nothing; a slide, a cancel, a lost capture, a hidden page or a stale
//! press write nothing either. An unguarded mute stays one tap.
//!
//! A guarded TechAlert keeps the double tap ([`MuteGuard`]): the first tap
//! arms (the button pulses) and changes nothing, a second within 500 ms
//! applies; a tap after the window arms again.
//!
//! The linked mute (#127, the owner's design on #127, mockup v3 variant A):
//! a strip's mute writes its own track and every partner track ([`Linked`]:
//! all playing, all muted, or m of N muted by hand in Live). A plain mute is
//! a link of one.

use super::hold::Hold;
use super::rail::{Fill, RAIL_HOLD_MS};

/// A guarded mute's press (#162): full at `RAIL_HOLD_MS` (1000 ms), acting
/// at its lift, as a rail button's.
pub fn mute_press() -> Hold {
    Hold::lifted(RAIL_HOLD_MS)
}

/// The hint a lift before full shows (#162): the mute must be held.
pub const HOLD_HINT: &str = "hold";

/// The line while a hold fills a guarded mute: what it will do (`mute`:
/// the lift mutes; else it unmutes).
pub fn hold_text(mute: bool) -> &'static str {
    if mute {
        "hold… mute"
    } else {
        "hold… unmute"
    }
}

/// The line while a guarded mute's press is full: what its lift does.
pub fn full_text(mute: bool) -> &'static str {
    if mute { "lift: mute" } else { "lift: unmute" }
}

/// The fill of a guarded mute's press `filling` (down, before its hold) or
/// `full` (past it): only a press that can act shows one, `to` the mute
/// its lift would write (none: the strip's own track has no value).
pub fn mute_fill(filling: bool, full: bool, to: Option<bool>) -> Fill {
    match (to, full, filling) {
        (None, _, _) => Fill::None,
        (Some(_), true, _) => Fill::Full,
        (Some(_), false, true) => Fill::Filling,
        (Some(_), false, false) => Fill::None,
    }
}

/// The guarded mute's line (#162; on a strip it takes the marks line's
/// place, on the detail's MUTE the label's): the hold's words while it
/// fills or is full, the hint after a lift before full (`hint`), else
/// none.
pub fn hold_line(fill: Fill, to: Option<bool>, hint: bool) -> Option<&'static str> {
    match fill {
        Fill::Full => to.map(full_text),
        Fill::Filling => to.map(hold_text),
        Fill::None => hint.then_some(HOLD_HINT),
    }
}

/// The window for a guarded TechAlert's confirming second tap.
pub const GUARD_MS: f64 = 500.0;

/// What a tap on a mute button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardAction {
    /// Send the new mute.
    Apply,
    /// Only arm the guard (and show it).
    Arm,
}

/// The double-tap guard of a guarded TechAlert (#162: a strip's guarded
/// mute holds instead, [`mute_press`]).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MuteGuard {
    armed_at: Option<f64>,
}

impl MuteGuard {
    /// A tap at `now` on a button that is `guarded` (or not).
    pub fn on_tap(&mut self, guarded: bool, now: f64) -> GuardAction {
        if !guarded {
            return GuardAction::Apply;
        }
        if self.armed(now) {
            self.armed_at = None;
            GuardAction::Apply
        } else {
            self.armed_at = Some(now);
            GuardAction::Arm
        }
    }

    /// Whether the guard waits for the confirming tap at `now`.
    pub fn armed(&self, now: f64) -> bool {
        self.armed_at.is_some_and(|t| now - t <= GUARD_MS)
    }
}

/// What a mute's tracks show together (#127): its strip's own and each
/// partner's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Linked {
    /// Every track plays.
    Playing,
    /// Every track is muted.
    Muted,
    /// `muted` of them are muted (by hand in Live), the others play.
    Mixed { muted: usize },
    /// A track has no value from Live (yet).
    Unknown,
}

impl Linked {
    /// From each track's mute (`None`: no value yet).
    pub fn of(muted: &[Option<bool>]) -> Self {
        if muted.is_empty() || muted.iter().any(Option::is_none) {
            return Self::Unknown;
        }
        let count = muted.iter().filter(|m| **m == Some(true)).count();
        if count == 0 {
            Self::Playing
        } else if count == muted.len() {
            Self::Muted
        } else {
            Self::Mixed { muted: count }
        }
    }

    /// The mute a tap writes to every track: all muted unmutes all, all
    /// playing or mixed mutes all (the owner's design); none while a track
    /// has no value.
    pub fn tap(self) -> Option<bool> {
        match self {
            Self::Muted => Some(false),
            Self::Playing | Self::Mixed { .. } => Some(true),
            Self::Unknown => None,
        }
    }

    /// Whether the button is lit in the track's colour: while a track
    /// plays. It is dark, and the strip steps back, only when all are muted.
    pub fn lit(self) -> bool {
        matches!(self, Self::Playing | Self::Mixed { .. })
    }

    /// The button's `data-muted` (`true` only when all are muted).
    pub fn muted_attr(self) -> &'static str {
        match self {
            Self::Playing => "false",
            Self::Muted => "true",
            Self::Mixed { .. } => "mixed",
            Self::Unknown => "unknown",
        }
    }

    /// The chain's `data-state`: grey `all`, red `muted`, amber `mixed`.
    pub fn look(self) -> &'static str {
        match self {
            Self::Playing => "all",
            Self::Muted => "muted",
            Self::Mixed { .. } => "mixed",
            Self::Unknown => "unknown",
        }
    }

    /// The chain's number: how many mutes are linked (`count`), and how
    /// many of them are muted, `m/N`, while they disagree.
    pub fn text(self, count: usize) -> String {
        match self {
            Self::Mixed { muted } => format!("{muted}/{count}"),
            _ => count.to_string(),
        }
    }
}

/// A linked mute as a tap finds it (#127 review M2, the main session's
/// ruling): the tracks a tap writes and what they show together. A tap
/// acts once the strip's own track has Live's value; it writes that track
/// and every partner that has one. A partner without a value (waiting, or
/// its binding unresolved) is left out of the tap and of the state, and
/// never disables the strip's own mute: the PA mute always works.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reach {
    /// The tracks a tap writes, by their index in the mutes given (the
    /// strip's own first); none while the strip's own has no value.
    pub targets: Vec<usize>,
    /// What those tracks show together.
    pub state: Linked,
}

impl Reach {
    /// From each track's mute (`None`: no value), the strip's own first.
    pub fn of(muted: &[Option<bool>]) -> Self {
        let targets: Vec<usize> = match muted.first() {
            Some(Some(_)) => muted
                .iter()
                .enumerate()
                .filter_map(|(i, m)| m.map(|_| i))
                .collect(),
            _ => Vec::new(),
        };
        let shown: Vec<Option<bool>> = targets.iter().map(|&i| muted[i]).collect();
        Self {
            state: Linked::of(&shown),
            targets,
        }
    }

    /// The chain's number: how many mutes a tap writes now, and `m/N` of
    /// them while they disagree; every linked mute (`count`) while the
    /// strip's own track has no value.
    pub fn text(&self, count: usize) -> String {
        if self.targets.is_empty() {
            self.state.text(count)
        } else {
            self.state.text(self.targets.len())
        }
    }
}

/// A chain's count this many characters long or longer is drawn compact
/// (#127 re-review N1): a mixed count of a two-digit total, "1/10" to
/// "31/32", beside a problem's `!` would not fit the corner line of a 64 px
/// strip otherwise.
pub const COMPACT_CHARS: usize = 4;

/// Whether the chain's count `text` is drawn compact (LinkMark's
/// `compact`, link.css): [`COMPACT_CHARS`] characters or more. Every
/// shorter count looks as before.
pub fn chain_compact(text: &str) -> bool {
    text.chars().count() >= COMPACT_CHARS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_guarded_mute_is_full_at_exactly_1000_ms_and_acts_at_its_lift() {
        use super::super::hold::Lift;
        let mut h = mute_press();
        assert!(h.down(1, true, 0.0));
        assert!(!h.open(1000.0_f64.next_down()), "a hair before");
        assert!(h.holding() && !h.full(), "filling");
        assert!(h.open(1000.0));
        assert!(h.full());
        assert_eq!(h.up(1, 1100.0), Lift::Open, "the lift acts");
        // A lift a hair before full is the hint's, never a write.
        assert!(h.down(2, true, 0.0));
        assert_eq!(h.up(2, 1000.0_f64.next_down()), Lift::Tap);
        // One that no check saw, at exactly 1000 ms, acts.
        assert!(h.down(3, true, 0.0));
        assert_eq!(h.up(3, 1000.0), Lift::Open);
        assert_eq!(mute_press(), Hold::lifted(1000.0));
        assert_eq!(RAIL_HOLD_MS, 1000.0);
    }

    #[test]
    fn a_guarded_mute_fills_only_while_its_lift_can_write() {
        for to in [Some(true), Some(false)] {
            assert_eq!(mute_fill(false, false, to), Fill::None, "no press");
            assert_eq!(mute_fill(true, false, to), Fill::Filling);
            assert_eq!(mute_fill(false, true, to), Fill::Full);
            assert_eq!(mute_fill(true, true, to), Fill::Full);
        }
        // The strip's own track without a value: no fill, however pressed.
        for (filling, full) in [(false, false), (true, false), (false, true), (true, true)] {
            assert_eq!(mute_fill(filling, full, None), Fill::None);
        }
    }

    #[test]
    fn a_guarded_mutes_line_says_the_hold_the_lift_or_the_hint() {
        assert_eq!(hold_text(true), "hold… mute");
        assert_eq!(hold_text(false), "hold… unmute");
        assert_eq!(full_text(true), "lift: mute");
        assert_eq!(full_text(false), "lift: unmute");
        assert_eq!(HOLD_HINT, "hold");
        assert_eq!(
            hold_line(Fill::Filling, Some(true), false),
            Some("hold… mute")
        );
        assert_eq!(
            hold_line(Fill::Filling, Some(false), true),
            Some("hold… unmute")
        );
        assert_eq!(hold_line(Fill::Full, Some(true), false), Some("lift: mute"));
        assert_eq!(
            hold_line(Fill::Full, Some(false), true),
            Some("lift: unmute")
        );
        assert_eq!(hold_line(Fill::None, Some(true), true), Some("hold"));
        assert_eq!(hold_line(Fill::None, None, true), Some("hold"));
        assert_eq!(hold_line(Fill::None, Some(true), false), None);
        assert_eq!(hold_line(Fill::None, None, false), None);
        assert_eq!(hold_line(Fill::Full, None, false), None);
        assert_eq!(hold_line(Fill::Filling, None, true), None);
    }

    #[test]
    fn an_unguarded_tap_applies_at_once() {
        let mut g = MuteGuard::default();
        assert_eq!(g.on_tap(false, 0.0), GuardAction::Apply);
        assert_eq!(g.on_tap(false, 10.0), GuardAction::Apply);
        assert!(!g.armed(10.0));
    }

    #[test]
    fn a_guarded_techalert_needs_a_second_tap_within_500_ms() {
        let mut g = MuteGuard::default();
        assert_eq!(g.on_tap(true, 1000.0), GuardAction::Arm);
        assert!(g.armed(1000.0) && g.armed(1500.0) && !g.armed(1501.0));
        assert_eq!(g.on_tap(true, 1400.0), GuardAction::Apply);
        assert!(!g.armed(1400.0), "applied: disarmed");
        // A second tap at 600 ms arms again instead.
        assert_eq!(g.on_tap(true, 2000.0), GuardAction::Arm);
        assert_eq!(g.on_tap(true, 2600.0), GuardAction::Arm);
        assert_eq!(
            g.on_tap(true, 3100.0),
            GuardAction::Apply,
            "500 ms still counts"
        );
    }

    #[test]
    fn a_links_tracks_are_all_playing_all_muted_or_mixed() {
        assert_eq!(Linked::of(&[]), Linked::Unknown);
        assert_eq!(Linked::of(&[Some(false), None]), Linked::Unknown);
        assert_eq!(Linked::of(&[None, Some(true)]), Linked::Unknown);
        assert_eq!(Linked::of(&[Some(false)]), Linked::Playing);
        assert_eq!(Linked::of(&[Some(true)]), Linked::Muted);
        let f = Some(false);
        let t = Some(true);
        assert_eq!(Linked::of(&[f, f, f]), Linked::Playing);
        assert_eq!(Linked::of(&[t, t, t]), Linked::Muted);
        assert_eq!(Linked::of(&[f, t, f]), Linked::Mixed { muted: 1 });
        assert_eq!(Linked::of(&[t, t, f]), Linked::Mixed { muted: 2 });
    }

    #[test]
    fn a_tap_mutes_all_unless_all_are_muted_then_it_unmutes_all() {
        assert_eq!(Linked::Playing.tap(), Some(true));
        assert_eq!(Linked::Mixed { muted: 1 }.tap(), Some(true));
        assert_eq!(Linked::Muted.tap(), Some(false));
        assert_eq!(Linked::Unknown.tap(), None);
        // A plain mute (a link of one) toggles, as before.
        assert_eq!(Linked::of(&[Some(false)]).tap(), Some(true));
        assert_eq!(Linked::of(&[Some(true)]).tap(), Some(false));
    }

    #[test]
    fn the_button_goes_dark_only_when_all_are_muted() {
        // A plain mute: lit while its track is audible.
        assert!(Linked::of(&[Some(false)]).lit());
        assert!(!Linked::of(&[Some(true)]).lit());
        assert!(Linked::Playing.lit());
        assert!(Linked::Mixed { muted: 2 }.lit());
        assert!(!Linked::Muted.lit());
        assert!(!Linked::Unknown.lit());
        let attrs = [
            Linked::Playing,
            Linked::Muted,
            Linked::Mixed { muted: 1 },
            Linked::Unknown,
        ]
        .map(Linked::muted_attr);
        assert_eq!(attrs, ["false", "true", "mixed", "unknown"]);
    }

    #[test]
    fn the_chain_shows_the_count_and_m_of_n_while_mixed() {
        assert_eq!(Linked::Playing.text(3), "3");
        assert_eq!(Linked::Muted.text(3), "3");
        assert_eq!(Linked::Unknown.text(2), "2");
        assert_eq!(Linked::Mixed { muted: 1 }.text(3), "1/3");
        assert_eq!(Linked::Mixed { muted: 2 }.text(4), "2/4");
        let looks = [
            Linked::Playing,
            Linked::Muted,
            Linked::Mixed { muted: 1 },
            Linked::Unknown,
        ]
        .map(Linked::look);
        assert_eq!(looks, ["all", "muted", "mixed", "unknown"]);
    }

    #[test]
    fn a_tap_writes_the_own_track_and_every_partner_with_a_value() {
        // #127 review M2: a partner without a value is left out and never
        // disables the strip's own mute (the PA mute always works); the
        // state is the written tracks'.
        let (f, t) = (Some(false), Some(true));
        let reach = |targets: Vec<usize>, state: Linked| Reach { targets, state };
        assert_eq!(
            Reach::of(&[f, t, f]),
            reach(vec![0, 1, 2], Linked::Mixed { muted: 1 })
        );
        assert_eq!(Reach::of(&[f, None, f]), reach(vec![0, 2], Linked::Playing));
        assert_eq!(Reach::of(&[t, t, None]), reach(vec![0, 1], Linked::Muted));
        assert_eq!(
            Reach::of(&[t, None, f]),
            reach(vec![0, 2], Linked::Mixed { muted: 1 })
        );
        // No partner with a value: the strip's own mute alone.
        assert_eq!(Reach::of(&[f, None, None]), reach(vec![0], Linked::Playing));
        assert_eq!(Reach::of(&[f, None]).state.tap(), Some(true));
        assert_eq!(Reach::of(&[t, None]).state.tap(), Some(false));
        // The strip's own track without a value: nothing, whatever the
        // partners show.
        assert_eq!(Reach::of(&[None, t, f]), reach(vec![], Linked::Unknown));
        assert_eq!(Reach::of(&[None, t, f]).state.tap(), None);
        assert_eq!(Reach::of(&[]), reach(vec![], Linked::Unknown));
        // A plain mute: a link of one.
        assert_eq!(Reach::of(&[t]), reach(vec![0], Linked::Muted));
        assert_eq!(Reach::of(&[f]), reach(vec![0], Linked::Playing));
    }

    #[test]
    fn the_chain_counts_the_mutes_a_tap_writes_now() {
        let (f, t) = (Some(false), Some(true));
        assert_eq!(Reach::of(&[f, f, f]).text(3), "3");
        assert_eq!(Reach::of(&[t, t, t]).text(3), "3");
        assert_eq!(Reach::of(&[t, f, f]).text(3), "1/3");
        // A partner without a value is not counted.
        assert_eq!(Reach::of(&[t, None, f]).text(3), "1/2");
        assert_eq!(Reach::of(&[t, t, None]).text(3), "2");
        // While the strip's own track has none: every linked mute.
        assert_eq!(Reach::of(&[None, t, f]).text(3), "3");
    }

    #[test]
    fn a_count_of_four_characters_or_more_is_drawn_compact() {
        // #127 re-review N1: a mixed count of a two-digit total beside a
        // problem's `!` does not fit the corner line of a 64 px strip.
        for text in ["1/10", "9/12", "10/12", "12/12", "31/32"] {
            assert!(chain_compact(text), "{text} is compact");
        }
        // Every shorter count keeps its look.
        for text in ["1", "3", "9", "10", "12", "32", "1/3", "8/9"] {
            assert!(!chain_compact(text), "{text} is not compact");
        }
        // As the chain shows it.
        let (f, t) = (Some(false), Some(true));
        let mut ten = vec![f; 10];
        assert!(!chain_compact(&Reach::of(&ten).text(10)), "10");
        ten[3] = t;
        assert!(chain_compact(&Reach::of(&ten).text(10)), "1/10");
    }
}
