//! The control column's head (#212, the owner's approved
//! `docs/mockups/column-calm-v2.html`): where the sectors page's ARRANGE
//! key (`⇄`) is drawn in the pages' bar.

/// Where the ⇄ key is drawn in the pages' bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyPlace {
    /// At the right of the bar's tab at this position, in its row.
    Beside(usize),
    /// At the bar's end, in a row of its own: no tab of the bar is the
    /// sectors page's.
    End,
    /// Not drawn.
    Nowhere,
}

/// Where the key goes in a bar of `tabs` (the level-0 tabs' page ids, in
/// order) for the sectors page `page`: beside that page's own tab.
pub fn key_place(tabs: &[&str], page: &str) -> KeyPlace {
    tabs.iter()
        .position(|id| *id == page)
        .map_or(KeyPlace::Nowhere, KeyPlace::Beside)
}

#[cfg(test)]
mod tests {
    use super::*;

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
