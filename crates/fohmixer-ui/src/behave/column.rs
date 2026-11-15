//! The control column's head (#212, the owner's approved
//! `docs/mockups/column-calm-v2.html`): where the sectors page's ARRANGE
//! key (`⇄`) is drawn in the pages' bar, and whether a pick there changes
//! anything (the recorder's `tab` event counts every press, a touch being
//! a use, and says `same` for one that changed nothing).

/// Where the ⇄ key is drawn in the pages' bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPlace {
    /// At the right of the bar's tab at this position, in its row.
    Beside(usize),
    /// At the bar's end, in a row of its own: no tab of the bar is the
    /// sectors page's (ARRANGE stays reachable).
    End,
}

/// Where the key goes in a bar of `tabs` (the level-0 tabs' page ids, in
/// order) for the sectors page `page`: beside that page's own tab, else
/// at the bar's end.
pub fn key_place(tabs: &[&str], page: &str) -> KeyPlace {
    tabs.iter()
        .position(|id| *id == page)
        .map_or(KeyPlace::End, KeyPlace::Beside)
}

/// Whether a page tab's pick changes nothing: its page (`picked`, the
/// layout's page index) is the one shown (`shown`) and no extra tab lies
/// over the page (`extra_open`; a pick that closes one changes what shows).
pub fn page_pick_same(shown: Option<usize>, extra_open: bool, picked: usize) -> bool {
    !extra_open && shown == Some(picked)
}

/// Whether a layout tab's pick changes nothing: its sector shows that
/// layout (`shown`, the sector's shown layout id) already.
pub fn layout_pick_same(shown: Option<&str>, picked: &str) -> bool {
    shown == Some(picked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_tabs_pick_is_the_same_only_on_the_page_shown_with_no_extra_tab() {
        assert!(page_pick_same(Some(1), false, 1));
        assert!(!page_pick_same(Some(0), false, 1));
        // A pick that closes STREAM DECK (or any extra tab) shows the page.
        assert!(!page_pick_same(Some(1), true, 1));
        assert!(!page_pick_same(Some(0), true, 1));
        // Nothing shown yet (a disposed selection): no pick is the same.
        assert!(!page_pick_same(None, false, 1));
    }

    #[test]
    fn a_layout_tabs_pick_is_the_same_only_on_the_layout_shown() {
        assert!(layout_pick_same(Some("lu-2"), "lu-2"));
        assert!(!layout_pick_same(Some("lu-1"), "lu-2"));
        assert!(!layout_pick_same(None, "lu-2"));
    }

    #[test]
    fn the_key_sits_beside_its_own_pages_tab() {
        assert_eq!(key_place(&["cue", "foh"], "foh"), KeyPlace::Beside(1));
        assert_eq!(key_place(&["foh", "cue"], "foh"), KeyPlace::Beside(0));
        assert_eq!(key_place(&["foh"], "foh"), KeyPlace::Beside(0));
    }

    #[test]
    fn with_no_tab_of_its_page_the_key_goes_to_the_bars_end_never_away() {
        // #212 review: a layout whose sectors page has no level-0 tab (or a
        // bar that lost it) must keep ARRANGE reachable.
        assert_eq!(key_place(&["cue", "other"], "foh"), KeyPlace::End);
        assert_eq!(key_place(&[], "foh"), KeyPlace::End);
    }
}
