//! The fader links' switches (#130, spec D18, F29): one per leading strip
//! (its Live set and its `+ID`), on or off, shared by every page and kept
//! in `hub-state.json` (`fader_links`: every switch a page set). Pure: the
//! router hands it each layout's leaders and the pages' switches, and tells
//! the pages what it returns. The frame's linked fader (Podklady All) has
//! no switch: it is always linked.

use std::collections::{BTreeMap, BTreeSet};

use fohmixer_proto::client::FaderLinkStatus;
use fohmixer_proto::layout::LinkId;
use serde::{Deserialize, Serialize};

/// A leader nobody switched (#130, the owner's answer to question 3: a
/// link starts on when it is first tagged).
pub const LINK_DEFAULT: bool = true;

/// A switch in `hub-state.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedLink {
    pub instance: String,
    pub id: String,
    pub on: bool,
}

/// The switches.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FaderLinks {
    /// Every switch a page set, kept while no layout names its leader (a
    /// tag fixed later finds its switch as it was).
    switched: BTreeMap<LinkId, bool>,
    /// The leaders the served layout names.
    leaders: BTreeSet<LinkId>,
}

impl FaderLinks {
    /// From `hub-state.json`.
    pub fn new(saved: &[SavedLink]) -> Self {
        let switched = saved
            .iter()
            .map(|s| {
                (
                    LinkId {
                        instance: s.instance.clone(),
                        id: s.id.clone(),
                    },
                    s.on,
                )
            })
            .collect();
        Self {
            switched,
            leaders: BTreeSet::new(),
        }
    }

    fn on(&self, link: &LinkId) -> bool {
        self.switched.get(link).copied().unwrap_or(LINK_DEFAULT)
    }

    /// A served layout's leaders: each one the last layout did not name,
    /// with its switch (every page is told it).
    pub fn set_leaders(&mut self, leaders: Vec<LinkId>) -> Vec<(LinkId, bool)> {
        let leaders: BTreeSet<LinkId> = leaders.into_iter().collect();
        let new: Vec<(LinkId, bool)> = leaders
            .difference(&self.leaders)
            .map(|l| (l.clone(), self.on(l)))
            .collect();
        self.leaders = leaders;
        new
    }

    /// A page's switch: none when no layout leads that link (an error to
    /// the page); else whether it changed (a change is saved).
    pub fn switch(&mut self, link: &LinkId, on: bool) -> Option<bool> {
        if !self.leaders.contains(link) {
            return None;
        }
        let changed = self.on(link) != on;
        self.switched.insert(link.clone(), on);
        Some(changed)
    }

    /// Every leader's switch (a page that attaches).
    pub fn values(&self) -> Vec<(LinkId, bool)> {
        self.leaders
            .iter()
            .map(|l| (l.clone(), self.on(l)))
            .collect()
    }

    /// What `hub-state.json` keeps.
    pub fn saved(&self) -> Vec<SavedLink> {
        self.switched
            .iter()
            .map(|(l, on)| SavedLink {
                instance: l.instance.clone(),
                id: l.id.clone(),
                on: *on,
            })
            .collect()
    }

    /// `/api/status` `fader_links`.
    pub fn status(&self) -> Vec<FaderLinkStatus> {
        self.values()
            .into_iter()
            .map(|(l, on)| FaderLinkStatus {
                instance: l.instance,
                id: l.id,
                on,
            })
            .collect()
    }
}

/// A switch's hub log line.
pub fn link_line(link: &LinkId, on: bool) -> String {
    format!(
        "the fader link {}/{} is {}",
        link.instance,
        link.id,
        if on { "on" } else { "off" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(id: &str) -> LinkId {
        LinkId {
            instance: "band".into(),
            id: id.into(),
        }
    }

    fn saved(id: &str, on: bool) -> SavedLink {
        SavedLink {
            instance: "band".into(),
            id: id.into(),
            on,
        }
    }

    #[test]
    fn a_new_leader_starts_as_the_default_and_a_switch_is_kept() {
        // LINK_DEFAULT: a leader nobody switched is on (its first tell).
        let mut links = FaderLinks::new(&[]);
        assert_eq!(links.values(), vec![], "no layout yet");
        assert_eq!(
            links.set_leaders(vec![link("REV")]),
            vec![(link("REV"), true)]
        );
        assert_eq!(
            links.set_leaders(vec![link("REV")]),
            vec![],
            "the same leaders: nothing to tell"
        );
        assert_eq!(links.switch(&link("REV"), false), Some(true));
        assert_eq!(
            links.switch(&link("REV"), false),
            Some(false),
            "the same value: told again, nothing saved"
        );
        assert_eq!(links.values(), vec![(link("REV"), false)]);
        assert_eq!(links.saved(), vec![saved("REV", false)]);
        assert_eq!(
            links.status(),
            vec![FaderLinkStatus {
                instance: "band".into(),
                id: "REV".into(),
                on: false
            }]
        );
        assert_eq!(
            links.switch(&link("VOX1"), true),
            None,
            "no layout leads it"
        );
        assert_eq!(links.switch(&link("REV"), true), Some(true));
        assert_eq!(
            links.saved(),
            vec![saved("REV", true)],
            "a page's switch is kept, the default's too"
        );
    }

    #[test]
    fn a_saved_switch_comes_back_with_its_leader() {
        let mut links = FaderLinks::new(&[saved("REV", false)]);
        assert_eq!(
            links.set_leaders(vec![link("VOX1"), link("REV")]),
            vec![(link("REV"), false), (link("VOX1"), true)]
        );
        assert_eq!(links.set_leaders(vec![link("VOX1")]), vec![]);
        assert_eq!(links.values(), vec![(link("VOX1"), true)]);
        assert_eq!(links.switch(&link("REV"), true), None, "its leader is gone");
        assert_eq!(
            links.saved(),
            vec![saved("REV", false)],
            "a leader gone keeps its switch"
        );
        assert_eq!(
            links.set_leaders(vec![link("REV"), link("VOX1")]),
            vec![(link("REV"), false)]
        );
    }

    #[test]
    fn each_switch_has_its_line() {
        assert_eq!(
            link_line(&link("REV"), true),
            "the fader link band/REV is on"
        );
        assert_eq!(
            link_line(&link("REV"), false),
            "the fader link band/REV is off"
        );
    }
}
