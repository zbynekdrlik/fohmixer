//! The mute button's protection (spec F12; TouchOSC `mute_button.lua`
//! 2.7.2): a strip listed in `double_click_mute` changes its mute only on a
//! second tap within 500 ms. The first tap arms (the button pulses) and
//! changes nothing; a tap after the window arms again.
//!
//! The linked mute (#127, the owner's design on #127, mockup v3 variant A):
//! a strip's mute writes its own track and every partner track ([`Linked`]:
//! all playing, all muted, or m of N muted by hand in Live). A plain mute is
//! a link of one.

/// The window for the confirming second tap.
pub const GUARD_MS: f64 = 500.0;

/// What a tap on a mute button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardAction {
    /// Send the new mute.
    Apply,
    /// Only arm the guard (and show it).
    Arm,
}

/// The guard of one mute button.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unguarded_tap_applies_at_once() {
        let mut g = MuteGuard::default();
        assert_eq!(g.on_tap(false, 0.0), GuardAction::Apply);
        assert_eq!(g.on_tap(false, 10.0), GuardAction::Apply);
        assert!(!g.armed(10.0));
    }

    #[test]
    fn a_guarded_mute_needs_a_second_tap_within_500_ms() {
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
}
