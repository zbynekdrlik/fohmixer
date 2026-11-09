//! A Live that is down, said in words (#178, the owner's approved mockup
//! `docs/mockups/live-down-v2.html`): each Live instance is `not running`
//! (the hub reports it offline), has `no set` (it answers, but no binding
//! of it the page holds resolves: a Live open with no set or still loading
//! one) or is `on`. A Live that has no set is not a fault of one binding,
//! so its bindings are not drawn red (I5 stays for a single bad binding
//! while that Live's set is loaded): they wait, dimmed, and the strips and
//! the column's foot say why, the strips also what to do. Nothing covers
//! any page.

use crate::store::SlotClass;

/// What the page says about one Live instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveWord {
    /// The hub reports the instance offline: Live is not running (or its
    /// FohMixer script is not loaded).
    NotRunning,
    /// Live answers, but no binding of it the page holds resolves.
    NoSet,
    /// Live answers and its set is there.
    On,
}

impl LiveWord {
    /// The strip's and the foot line's `data-live`.
    pub fn name(self) -> &'static str {
        match self {
            Self::NotRunning => "off",
            Self::NoSet => "noset",
            Self::On => "on",
        }
    }

    /// The foot line's word after the instance's name.
    pub fn word(self) -> &'static str {
        match self {
            Self::NotRunning => "not running",
            Self::NoSet => "no set",
            Self::On => "on",
        }
    }

    /// What to do about it, dim in a down strip's fader area (#178, the
    /// main session's ruling: the column's foot keeps its height, so the
    /// hint lives on the strips; none while on).
    pub fn hint(self) -> Option<&'static str> {
        match self {
            Self::NotRunning => Some(NOT_RUNNING_HINT),
            Self::NoSet => Some(NO_SET_HINT),
            Self::On => None,
        }
    }

    /// The word a strip of this Live shows under its name (none while on).
    pub fn strip_word(self) -> Option<&'static str> {
        match self {
            Self::NotRunning => Some("NOT RUNNING"),
            Self::NoSet => Some("NO SET"),
            Self::On => None,
        }
    }
}

/// Whether a binding's target tells whether the set is there: only a
/// name the set gave (`tracks[name=Vox 1]`) does. An empty Live has the
/// song itself, the master track and the default template's tracks and
/// returns, so a binding by index resolves there too, as do the song's own
/// properties (the transport).
pub fn judges_set(target: &str) -> bool {
    target.contains("[name=")
}

/// What the page's wanted slots of one instance hold: any with Live's
/// value (fresh or stale), any whose binding does not resolve, any still
/// waiting for the hub's first answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Seen {
    pub resolved: bool,
    pub unresolved: bool,
    pub waiting: bool,
}

impl Seen {
    /// The slots' classes seen together.
    pub fn of(classes: impl IntoIterator<Item = SlotClass>) -> Self {
        let mut seen = Self::default();
        for class in classes {
            match class {
                SlotClass::Resolved => seen.resolved = true,
                SlotClass::Unresolved => seen.unresolved = true,
                SlotClass::Waiting => seen.waiting = true,
            }
        }
        seen
    }

    /// No set: something does not resolve, nothing does, and nothing still
    /// waits (at a page's first load the first answer can be a single bad
    /// binding's error while the set's values are still on their way).
    fn no_set(self) -> bool {
        self.unresolved && !self.resolved && !self.waiting
    }
}

/// Whether a slot of a Live that turned from `before` to `after` can change
/// that Live's `word`: only a change of class can, and a value can only end
/// a missing set (a Live whose set is there stays so), so the values of a
/// page's first load judge nothing.
pub fn rejudges(word: Option<LiveWord>, before: Option<SlotClass>, after: SlotClass) -> bool {
    before != Some(after) && (after != SlotClass::Resolved || word == Some(LiveWord::NoSet))
}

/// One instance's word: none while the page has no hub connection (the
/// close marks every instance offline, which is no news of Live; the
/// dropout counter says the link is down) or before the hub reported the
/// instance (`online` none).
pub fn live_word(connected: bool, online: Option<bool>, seen: Seen) -> Option<LiveWord> {
    if !connected {
        return None;
    }
    match online? {
        false => Some(LiveWord::NotRunning),
        true if seen.no_set() => Some(LiveWord::NoSet),
        true => Some(LiveWord::On),
    }
}

/// The hint while a Live is not running (the mockup's words).
pub const NOT_RUNNING_HINT: &str = "Start Live on the FOH PC. The faders come back by themselves.";

/// The hint while a Live answers with no set.
pub const NO_SET_HINT: &str = "Open the set in Live.";

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Seen = Seen {
        resolved: false,
        unresolved: false,
        waiting: false,
    };
    const ERRORS: Seen = Seen {
        resolved: false,
        unresolved: true,
        waiting: false,
    };
    const VALUES: Seen = Seen {
        resolved: true,
        unresolved: false,
        waiting: false,
    };
    const BOTH: Seen = Seen {
        resolved: true,
        unresolved: true,
        waiting: false,
    };
    const ERRORS_WAITING: Seen = Seen {
        resolved: false,
        unresolved: true,
        waiting: true,
    };

    #[test]
    fn no_hub_connection_or_no_report_says_nothing() {
        for seen in [NONE, ERRORS, VALUES, BOTH] {
            for online in [None, Some(false), Some(true)] {
                assert_eq!(live_word(false, online, seen), None);
            }
            assert_eq!(live_word(true, None, seen), None);
        }
    }

    #[test]
    fn an_offline_live_is_not_running_whatever_its_slots() {
        for seen in [NONE, ERRORS, VALUES, BOTH] {
            assert_eq!(
                live_word(true, Some(false), seen),
                Some(LiveWord::NotRunning)
            );
        }
    }

    #[test]
    fn an_online_live_has_no_set_only_while_nothing_resolves_and_something_does_not() {
        assert_eq!(live_word(true, Some(true), ERRORS), Some(LiveWord::NoSet));
        // One binding resolved: the set is there; an error is a single bad
        // binding (I5, red).
        assert_eq!(live_word(true, Some(true), BOTH), Some(LiveWord::On));
        assert_eq!(live_word(true, Some(true), VALUES), Some(LiveWord::On));
        // Everything still waits for Live: no news yet.
        assert_eq!(live_word(true, Some(true), NONE), Some(LiveWord::On));
        // A first error while others still wait: no verdict yet.
        assert_eq!(
            live_word(true, Some(true), ERRORS_WAITING),
            Some(LiveWord::On)
        );
    }

    #[test]
    fn the_slots_classes_seen_together() {
        use SlotClass::*;
        assert_eq!(Seen::of([]), NONE);
        assert_eq!(Seen::of([Unresolved, Unresolved]), ERRORS);
        assert_eq!(Seen::of([Resolved]), VALUES);
        assert_eq!(Seen::of([Unresolved, Resolved]), BOTH);
        assert_eq!(Seen::of([Waiting, Unresolved]), ERRORS_WAITING);
    }

    #[test]
    fn only_a_class_change_judges_and_a_value_only_ends_a_missing_set() {
        use LiveWord::*;
        use SlotClass::*;
        // No change of class: nothing to judge.
        assert!(!rejudges(Some(NoSet), Some(Unresolved), Unresolved));
        assert!(!rejudges(Some(On), Some(Resolved), Resolved));
        // A value: only for a Live with no set.
        assert!(rejudges(Some(NoSet), Some(Unresolved), Resolved));
        assert!(rejudges(Some(NoSet), Some(Waiting), Resolved));
        assert!(rejudges(Some(NoSet), None, Resolved));
        assert!(!rejudges(Some(On), Some(Waiting), Resolved));
        assert!(!rejudges(Some(NotRunning), Some(Waiting), Resolved));
        assert!(!rejudges(None, Some(Waiting), Resolved));
        // An error or a slot that waits again: always judged.
        assert!(rejudges(Some(On), Some(Waiting), Unresolved));
        assert!(rejudges(Some(On), Some(Resolved), Unresolved));
        assert!(rejudges(None, Some(Resolved), Waiting));
    }

    #[test]
    fn only_a_name_the_set_gave_tells_whether_a_set_is_there() {
        assert!(judges_set(
            "live_set tracks[name=Vox 1] mixer_device volume"
        ));
        assert!(judges_set("live_set return_tracks[name=A-Reverb #]"));
        assert!(judges_set(
            "live_set tracks[name=Drums #] devices[name=EQ] parameters 1"
        ));
        assert!(!judges_set("live_set tracks 3 devices 0"));
        assert!(!judges_set("live_set return_tracks 0"));
        assert!(!judges_set("live_set"));
        assert!(!judges_set("live_set master_track mixer_device volume"));
    }

    #[test]
    fn the_words_and_names() {
        assert_eq!(LiveWord::NotRunning.name(), "off");
        assert_eq!(LiveWord::NoSet.name(), "noset");
        assert_eq!(LiveWord::On.name(), "on");
        assert_eq!(LiveWord::NotRunning.word(), "not running");
        assert_eq!(LiveWord::NoSet.word(), "no set");
        assert_eq!(LiveWord::On.word(), "on");
        assert_eq!(LiveWord::NotRunning.strip_word(), Some("NOT RUNNING"));
        assert_eq!(LiveWord::NoSet.strip_word(), Some("NO SET"));
        assert_eq!(LiveWord::On.strip_word(), None);
    }

    #[test]
    fn a_down_strip_says_what_to_do() {
        assert_eq!(LiveWord::NotRunning.hint(), Some(NOT_RUNNING_HINT));
        assert_eq!(LiveWord::NoSet.hint(), Some(NO_SET_HINT));
        assert_eq!(LiveWord::On.hint(), None);
        assert_eq!(
            NOT_RUNNING_HINT,
            "Start Live on the FOH PC. The faders come back by themselves."
        );
        assert_eq!(NO_SET_HINT, "Open the set in Live.");
    }
}
