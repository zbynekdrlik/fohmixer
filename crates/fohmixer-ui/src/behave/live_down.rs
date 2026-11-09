//! A Live that is down, said in words (#178, the owner's approved mockup
//! `docs/mockups/live-down-v2.html`): each Live instance is `not running`
//! (the hub reports it offline), has `no set` (it answers, but no binding
//! of it the page holds resolves: a Live open with no set or still loading
//! one) or is `on`. A Live that has no set is not a fault of one binding,
//! so its bindings are not drawn red (I5 stays for a single bad binding
//! while that Live's set is loaded): they wait, dimmed, and the strips and
//! the column's foot say why. Nothing covers any page.

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

    /// The word a strip of this Live shows under its name (none while on).
    pub fn strip_word(self) -> Option<&'static str> {
        match self {
            Self::NotRunning => Some("NOT RUNNING"),
            Self::NoSet => Some("NO SET"),
            Self::On => None,
        }
    }
}

/// What the page's wanted slots of one instance hold: any with Live's
/// value (fresh or stale), any whose binding does not resolve.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Seen {
    pub resolved: bool,
    pub unresolved: bool,
}

/// Whether a binding's target tells whether the set is there: anything
/// of the set's contents (a track, a return, a device, a scene) does; the
/// song itself (`live_set` and its own properties, the transport) and the
/// master track do not, since an empty Live has them too.
pub fn judges_set(target: &str) -> bool {
    let rest = target.strip_prefix("live_set").unwrap_or(target);
    let rest = rest.trim_start();
    !(rest.is_empty() || rest == "master_track" || rest.starts_with("master_track "))
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
        true if seen.unresolved && !seen.resolved => Some(LiveWord::NoSet),
        true => Some(LiveWord::On),
    }
}

/// The foot's hint line under the instances: what to do about the Live
/// that is down, a Live not running first (it covers a missing set too).
pub fn hint(words: impl IntoIterator<Item = LiveWord>) -> Option<&'static str> {
    let mut no_set = false;
    for word in words {
        match word {
            LiveWord::NotRunning => return Some(NOT_RUNNING_HINT),
            LiveWord::NoSet => no_set = true,
            LiveWord::On => {}
        }
    }
    no_set.then_some(NO_SET_HINT)
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
    };
    const ERRORS: Seen = Seen {
        resolved: false,
        unresolved: true,
    };
    const VALUES: Seen = Seen {
        resolved: true,
        unresolved: false,
    };
    const BOTH: Seen = Seen {
        resolved: true,
        unresolved: true,
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
    }

    #[test]
    fn only_the_sets_contents_tell_whether_a_set_is_there() {
        assert!(judges_set(
            "live_set tracks[name=Vox 1] mixer_device volume"
        ));
        assert!(judges_set("live_set tracks[name=Vox 1]"));
        assert!(judges_set("live_set return_tracks[name=A-Reverb #]"));
        assert!(judges_set("live_set tracks 3 devices 0"));
        assert!(judges_set("live_set scenes 0"));
        assert!(!judges_set("live_set"));
        assert!(!judges_set("live_set master_track"));
        assert!(!judges_set("live_set master_track mixer_device volume"));
        assert!(judges_set("live_set master_tracks"));
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
    fn the_hint_names_a_live_not_running_first() {
        use LiveWord::*;
        assert_eq!(hint([]), None);
        assert_eq!(hint([On, On]), None);
        assert_eq!(hint([On, NoSet]), Some(NO_SET_HINT));
        assert_eq!(hint([NoSet, On]), Some(NO_SET_HINT));
        assert_eq!(hint([NoSet, NotRunning]), Some(NOT_RUNNING_HINT));
        assert_eq!(hint([NotRunning, NoSet]), Some(NOT_RUNNING_HINT));
        assert_eq!(hint([On, NotRunning]), Some(NOT_RUNNING_HINT));
        assert_eq!(
            NOT_RUNNING_HINT,
            "Start Live on the FOH PC. The faders come back by themselves."
        );
        assert_eq!(NO_SET_HINT, "Open the set in Live.");
    }
}
