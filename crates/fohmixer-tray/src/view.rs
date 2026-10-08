//! What the tray shows (decided here, carried out by the Windows glue): the
//! tooltip and the menu's version line from the hub's state, the Copy URL
//! item from the links, which menu item was chosen, and when a new state is
//! worth a log line.

use crate::poll::HubState;

/// The menu items' ids.
pub const OPEN: &str = "open";
pub const MANUAL: &str = "manual";
pub const COPY: &str = "copy";
pub const EXIT: &str = "exit";
/// The disabled line with the hub's version.
pub const VERSION: &str = "version";

/// The longest tooltip, in UTF-16 units: Windows keeps 128 including the
/// closing NUL (`NOTIFYICONDATAW::szTip`).
pub const TOOLTIP_MAX: usize = 127;

/// A menu choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    /// Open fohmixer: the hub's local URL in the default browser.
    Open,
    /// The tag manual (#68): the hub's `/znacky.html` in the browser.
    Manual,
    /// Copy URL: the public URL to the clipboard.
    Copy,
    /// Exit: the tray only; the hub keeps running.
    Exit,
}

/// The action of a menu item's id; `None` for the version line (disabled)
/// and anything else.
pub fn action(id: &str) -> Option<MenuAction> {
    match id {
        OPEN => Some(MenuAction::Open),
        MANUAL => Some(MenuAction::Manual),
        COPY => Some(MenuAction::Copy),
        EXIT => Some(MenuAction::Exit),
        _ => None,
    }
}

/// How long after a left click opens fohmixer another one opens nothing:
/// a double click is two clicks, and one tab is enough (Windows' default
/// double-click time is 500 ms).
pub const OPEN_GAP_MS: u64 = 800;

/// Whether a click on the icon opens fohmixer (#68): the left button's
/// release (the right button shows the menu), unless a click opened it
/// `since_open_ms` ago, under [`OPEN_GAP_MS`].
pub fn click_opens(left: bool, released: bool, since_open_ms: Option<u64>) -> bool {
    left && released && since_open_ms.is_none_or(|ms| ms >= OPEN_GAP_MS)
}

/// `text` cut to at most `max` UTF-16 units, never inside a character.
pub fn clip(text: &str, max: usize) -> String {
    let mut used = 0;
    text.chars()
        .take_while(|c| {
            used += c.len_utf16();
            used <= max
        })
        .collect()
}

/// The tooltip for the hub's state; `tray_version` is this tray's version,
/// named when the hub runs another one (a tray left from before an update).
pub fn tooltip(state: &HubState, tray_version: &str) -> String {
    let text = match state {
        HubState::Unknown => "fohmixer: asking the hub".to_string(),
        HubState::Up(info) if info.version == tray_version => {
            format!(
                "fohmixer v{} ({}): the hub runs",
                info.version, info.git_hash
            )
        }
        HubState::Up(info) => format!(
            "fohmixer v{} ({}): the hub runs (this tray is v{tray_version})",
            info.version, info.git_hash
        ),
        HubState::Down { reason, .. } => {
            format!("fohmixer: the hub does not answer ({reason})")
        }
    };
    clip(&text, TOOLTIP_MAX)
}

/// The menu's disabled version line.
pub fn version_line(state: &HubState) -> String {
    match state {
        HubState::Unknown => "Hub: asking".to_string(),
        HubState::Up(info) => format!("Hub v{} ({})", info.version, info.git_hash),
        HubState::Down { .. } => "Hub: not answering".to_string(),
    }
}

/// The Copy URL item: its label and whether it is enabled (only with a
/// public URL, i.e. remote access set up).
pub fn copy_item(public: Option<&str>) -> (String, bool) {
    match public {
        Some(url) => (format!("Copy URL ({url})"), true),
        None => ("Copy URL (no public name set up)".to_string(), false),
    }
}

/// Whether `next` differs from `prev` enough for a log line and a new
/// tooltip: another kind of state, another hub build, or another reason it
/// does not answer (a changing error detail alone is the same outage).
pub fn changed(prev: &HubState, next: &HubState) -> bool {
    match (prev, next) {
        (HubState::Unknown, HubState::Unknown) => false,
        (HubState::Up(a), HubState::Up(b)) => a != b,
        (HubState::Down { reason: a, .. }, HubState::Down { reason: b, .. }) => a != b,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fohmixer_proto::VersionInfo;

    fn up(version: &str, hash: &str) -> HubState {
        HubState::Up(VersionInfo {
            version: version.to_string(),
            git_hash: hash.to_string(),
            branch: "master".to_string(),
            build_time: "1790000000".to_string(),
        })
    }

    fn down(reason: &str, detail: &str) -> HubState {
        HubState::Down {
            reason: reason.to_string(),
            detail: detail.to_string(),
        }
    }

    #[test]
    fn menu_ids_map_to_their_actions() {
        assert_eq!(action("open"), Some(MenuAction::Open));
        assert_eq!(action("manual"), Some(MenuAction::Manual));
        assert_eq!(MANUAL, "manual");
        assert_eq!(action("copy"), Some(MenuAction::Copy));
        assert_eq!(action("exit"), Some(MenuAction::Exit));
        assert_eq!(action("version"), None);
        assert_eq!(action(""), None);
    }

    #[test]
    fn the_ids_are_distinct_words() {
        assert_eq!(
            [OPEN, COPY, EXIT, VERSION],
            ["open", "copy", "exit", "version"]
        );
    }

    #[test]
    fn the_tooltip_of_a_running_hub_names_its_version() {
        assert_eq!(
            tooltip(&up("0.1.0-dev.25", "abc1234"), "0.1.0-dev.25"),
            "fohmixer v0.1.0-dev.25 (abc1234): the hub runs"
        );
    }

    #[test]
    fn the_tooltip_names_a_tray_of_another_version() {
        assert_eq!(
            tooltip(&up("0.1.0-dev.26", "def5678"), "0.1.0-dev.25"),
            "fohmixer v0.1.0-dev.26 (def5678): the hub runs (this tray is v0.1.0-dev.25)"
        );
    }

    #[test]
    fn the_tooltip_of_a_hub_that_does_not_answer_gives_the_reason() {
        assert_eq!(
            tooltip(&down("no answer", "connection refused"), "0.1.0"),
            "fohmixer: the hub does not answer (no answer)"
        );
        assert_eq!(
            tooltip(&down("HTTP 503", "x"), "0.1.0"),
            "fohmixer: the hub does not answer (HTTP 503)"
        );
    }

    #[test]
    fn the_tooltip_before_the_first_answer() {
        assert_eq!(
            tooltip(&HubState::Unknown, "0.1.0"),
            "fohmixer: asking the hub"
        );
    }

    #[test]
    fn a_long_tooltip_fits_windows_128() {
        let long = down(&"r".repeat(300), "");
        let tip = tooltip(&long, "0.1.0");
        assert_eq!(tip.encode_utf16().count(), 127);
        assert!(
            tip.starts_with("fohmixer: the hub does not answer (rrr"),
            "{tip}"
        );
    }

    #[test]
    fn clip_keeps_text_that_fits_and_cuts_at_the_limit() {
        assert_eq!(clip("abc", 3), "abc");
        assert_eq!(clip("abcd", 3), "abc");
        assert_eq!(clip("ab", 3), "ab");
        assert_eq!(clip("", 3), "");
        assert_eq!(clip("abc", 0), "");
    }

    #[test]
    fn clip_counts_utf16_units_and_never_splits_a_character() {
        // U+1F3B9 (a keyboard) is two UTF-16 units.
        assert_eq!(clip("a\u{1F3B9}b", 3), "a\u{1F3B9}");
        assert_eq!(clip("a\u{1F3B9}b", 2), "a");
        assert_eq!(clip("\u{e1}\u{e1}", 1), "\u{e1}");
    }

    #[test]
    fn the_version_line_follows_the_hub() {
        assert_eq!(version_line(&HubState::Unknown), "Hub: asking");
        assert_eq!(
            version_line(&up("0.1.0-dev.25", "abc1234")),
            "Hub v0.1.0-dev.25 (abc1234)"
        );
        assert_eq!(version_line(&down("no answer", "x")), "Hub: not answering");
    }

    #[test]
    fn copy_url_is_enabled_only_with_a_public_url() {
        assert_eq!(
            copy_item(Some("https://foh.example.org/")),
            ("Copy URL (https://foh.example.org/)".to_string(), true)
        );
        assert_eq!(
            copy_item(None),
            ("Copy URL (no public name set up)".to_string(), false)
        );
    }

    #[test]
    fn a_new_kind_of_state_is_a_change() {
        assert!(changed(&HubState::Unknown, &up("1.0.0", "a")));
        assert!(changed(&HubState::Unknown, &down("no answer", "x")));
        assert!(changed(&up("1.0.0", "a"), &down("no answer", "x")));
        assert!(changed(&down("no answer", "x"), &up("1.0.0", "a")));
        assert!(changed(&up("1.0.0", "a"), &HubState::Unknown));
    }

    #[test]
    fn the_same_state_is_no_change() {
        assert!(!changed(&HubState::Unknown, &HubState::Unknown));
        assert!(!changed(&up("1.0.0", "a"), &up("1.0.0", "a")));
        // Another detail of the same outage (an OS error text) is the same outage.
        assert!(!changed(&down("no answer", "x"), &down("no answer", "y")));
    }

    #[test]
    fn another_build_or_another_reason_is_a_change() {
        assert!(changed(&up("1.0.0", "a"), &up("1.0.1", "a")));
        assert!(changed(&up("1.0.0", "a"), &up("1.0.0", "b")));
        assert!(changed(&down("no answer", "x"), &down("HTTP 503", "x")));
    }

    #[test]
    fn only_the_left_buttons_release_opens_fohmixer() {
        assert!(click_opens(true, true, None));
        assert!(!click_opens(true, false, None));
        assert!(!click_opens(false, true, None));
        assert!(!click_opens(false, false, None));
    }

    #[test]
    fn a_double_click_opens_one_tab() {
        assert!(!click_opens(true, true, Some(0)));
        assert!(!click_opens(true, true, Some(799)));
        assert!(click_opens(true, true, Some(800)));
        assert!(!click_opens(false, true, Some(800)));
    }
}
