//! iemmixer's tab (#166; the owner's approved `docs/mockups/iem-tab-v1.html`),
//! pure: what the hub said of iemmixer's page, and the frame's life. Unlike
//! AbleSet's frame it outlives its tab, so listening goes on while the
//! engineer works the faders: the first open mounts it, another tab only
//! hides it (the tab then shows a dot), and only the ✕ beside the tab ends
//! it. `pages/iem.rs` carries it out.

/// iemmixer's page as the hub's `iem` message described it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IemInfo {
    /// The tab's title.
    pub title: String,
    /// The page's address (`[iem] url`), framed directly: iemmixer allows
    /// the frame itself.
    pub url: String,
}

/// The frame's permissions (`allow`): talking to the band needs the
/// microphone, and listening starts without a tap inside the frame.
pub const FRAME_ALLOW: &str = "microphone; autoplay";

/// Whether the frame lives after a tab was shown (`shown`: iemmixer's own):
/// the first open mounts it, and no other tab ends it.
pub fn alive_after(alive: bool, shown: bool) -> bool {
    alive || shown
}

/// The tab's dot: the frame lives behind another tab, so its audio goes on
/// out of sight.
pub fn tab_dot(alive: bool, shown: bool) -> bool {
    alive && !shown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_open_mounts_the_frame_and_no_other_tab_ends_it() {
        assert!(!alive_after(false, false), "never opened");
        assert!(alive_after(false, true), "the first open");
        assert!(alive_after(true, false), "another tab keeps it");
        assert!(alive_after(true, true), "its own tab again");
    }

    #[test]
    fn the_dot_says_the_frame_lives_out_of_sight() {
        assert!(!tab_dot(false, false), "no frame");
        assert!(!tab_dot(true, true), "the frame is in sight");
        assert!(tab_dot(true, false), "behind another tab");
        assert!(!tab_dot(false, true), "shown without a frame says nothing");
    }
}
