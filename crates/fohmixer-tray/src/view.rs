//! What the tray shows (decided here, carried out by the Windows glue): the
//! tooltip and the menu's version line from the hub's state, the Copy URL
//! item from the links, which menu item was chosen, when a new state is
//! worth a log line, and how Open finds or starts the app window (#280).

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

/// The menu's Open item on a hub that serves the MBC Live alone (#280).
pub const OPEN_MBC: &str = "Open MBC mixer";
/// The menu's Open item elsewhere.
pub const OPEN_FOHMIXER: &str = "Open fohmixer";

/// The Open item's label: it names what it opens (#280).
pub fn open_label(mbc_only: bool) -> &'static str {
    if mbc_only { OPEN_MBC } else { OPEN_FOHMIXER }
}

/// The page's title (`crates/fohmixer-ui/index.html`), the title of its app
/// window: a browser's tab window adds its own name to it.
pub const APP_TITLE: &str = "fohmixer";

/// How long after the tray started the app window it starts no second one
/// while none shows yet (#280): a browser's first window takes seconds.
pub const START_GAP_MS: u64 = 15_000;

/// The file name of a Windows (or any) path: what follows its last `\` or
/// `/`.
pub fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// The page an app command opens: the value of its `--app=` word (a
/// Chromium app window), `None` without one.
pub fn app_page(app: &[String]) -> Option<&str> {
    app.iter()
        .find_map(|word| word.strip_prefix("--app="))
        .filter(|page| !page.is_empty())
}

/// Whether `title` is what an app window of `page` shows while it has no
/// page title (#280, the review round): the page still loading shows its
/// address (`localhost:8480/`, with or without its scheme), the browser's
/// error page while the hub does not answer its host (`localhost`). A tab
/// window adds ` - <browser>` to any title, so a title with ` - ` is none.
/// Accepted: a window of the same program titled just the host on another
/// page of it (none on the MBC PC: Brave runs only the app window there).
pub fn titled_by_page(title: &str, page: &str) -> bool {
    let address = page.split_once("://").map_or(page, |(_, rest)| rest);
    let host = address.split([':', '/']).next().unwrap_or(address);
    let bare = address.trim_end_matches('/');
    !host.is_empty()
        && !title.contains(" - ")
        && (title == host
            || shows_address(title, bare)
            || shows_address(title, page.trim_end_matches('/')))
}

/// Whether `title` is `address`, alone or followed by its path, query or
/// fragment (never another port that begins with it: `localhost:80` is not
/// `localhost:8080`).
fn shows_address(title: &str, address: &str) -> bool {
    title
        .strip_prefix(address)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['/', '?', '#']))
}

/// Whether a visible top-level window is the app window (#280): its title is
/// the page's (or, with `page`, the one an app window of that page shows
/// before or without it, [`titled_by_page`]), and its process's image has
/// the file name of the app's program (case aside, as Windows names files).
pub fn is_app_window(title: &str, image: &str, program: &str, page: Option<&str>) -> bool {
    let name = file_name(image);
    let titled = title == APP_TITLE || page.is_some_and(|page| titled_by_page(title, page));
    titled && !name.is_empty() && name.eq_ignore_ascii_case(file_name(program))
}

/// What Open does with an app command (#280).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenStep {
    /// Bring the open app window to the front (restored when minimized).
    Focus,
    /// Start the app command.
    Start,
    /// Nothing: the tray started it less than [`START_GAP_MS`] ago and its
    /// window does not show yet.
    Wait,
}

/// Open's step: the app window when one is open, else a start, unless the
/// tray started one less than [`START_GAP_MS`] ago.
pub fn open_step(found: bool, since_start_ms: Option<u64>) -> OpenStep {
    if found {
        OpenStep::Focus
    } else if since_start_ms.is_some_and(|ms| ms < START_GAP_MS) {
        OpenStep::Wait
    } else {
        OpenStep::Start
    }
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
        HubState::Unknown => "fohmixer: checking the hub".to_string(),
        HubState::Up(info) if info.version == tray_version => {
            format!(
                "fohmixer v{} ({}): hub running",
                info.version, info.git_hash
            )
        }
        HubState::Up(info) => format!(
            "fohmixer v{} ({}): hub running (this tray is v{tray_version})",
            info.version, info.git_hash
        ),
        HubState::Down { reason, .. } => {
            format!("fohmixer: hub not answering ({reason})")
        }
    };
    clip(&text, TOOLTIP_MAX)
}

/// The menu's disabled version line.
pub fn version_line(state: &HubState) -> String {
    match state {
        HubState::Unknown => "Hub: checking".to_string(),
        HubState::Up(info) => format!("Hub v{} ({})", info.version, info.git_hash),
        HubState::Down { .. } => "Hub: not answering".to_string(),
    }
}

/// The Copy URL item: its label and whether it is enabled (only with a
/// public URL, i.e. remote access set up).
pub fn copy_item(public: Option<&str>) -> (String, bool) {
    match public {
        Some(url) => (format!("Copy URL ({url})"), true),
        None => ("Copy URL (no public address set up)".to_string(), false),
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
            "fohmixer v0.1.0-dev.25 (abc1234): hub running"
        );
    }

    #[test]
    fn the_tooltip_names_a_tray_of_another_version() {
        assert_eq!(
            tooltip(&up("0.1.0-dev.26", "def5678"), "0.1.0-dev.25"),
            "fohmixer v0.1.0-dev.26 (def5678): hub running (this tray is v0.1.0-dev.25)"
        );
    }

    #[test]
    fn the_tooltip_of_a_hub_that_does_not_answer_gives_the_reason() {
        assert_eq!(
            tooltip(&down("no answer", "connection refused"), "0.1.0"),
            "fohmixer: hub not answering (no answer)"
        );
        assert_eq!(
            tooltip(&down("HTTP 503", "x"), "0.1.0"),
            "fohmixer: hub not answering (HTTP 503)"
        );
    }

    #[test]
    fn the_tooltip_before_the_first_answer() {
        assert_eq!(
            tooltip(&HubState::Unknown, "0.1.0"),
            "fohmixer: checking the hub"
        );
    }

    #[test]
    fn a_long_tooltip_fits_windows_128() {
        let long = down(&"r".repeat(300), "");
        let tip = tooltip(&long, "0.1.0");
        assert_eq!(tip.encode_utf16().count(), 127);
        assert!(tip.starts_with("fohmixer: hub not answering (rrr"), "{tip}");
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
        assert_eq!(version_line(&HubState::Unknown), "Hub: checking");
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
            ("Copy URL (no public address set up)".to_string(), false)
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
    #[test]
    fn the_open_item_names_the_mbc_mixer_on_the_mbc_hub() {
        assert_eq!(open_label(true), "Open MBC mixer");
        assert_eq!(open_label(false), "Open fohmixer");
    }

    #[test]
    fn the_app_title_is_the_pages_title() {
        let page = include_str!("../../fohmixer-ui/index.html");
        assert!(
            page.contains(&format!("<title>{APP_TITLE}</title>")),
            "index.html's title is no longer {APP_TITLE:?}"
        );
    }

    #[test]
    fn file_name_takes_what_follows_the_last_separator() {
        assert_eq!(file_name(r"C:\Program Files\Brave\brave.exe"), "brave.exe");
        assert_eq!(file_name("/usr/bin/brave"), "brave");
        assert_eq!(file_name(r"C:\a/b\c.exe"), "c.exe");
        assert_eq!(file_name(r"C:\a\b/c.exe"), "c.exe");
        assert_eq!(file_name("brave.exe"), "brave.exe");
        assert_eq!(file_name(r"C:\dir\"), "");
        assert_eq!(file_name(""), "");
    }

    #[test]
    fn the_app_window_is_the_pages_title_in_the_apps_program() {
        let brave = r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe";
        assert!(is_app_window("fohmixer", brave, brave, PAGE));
        // Windows names files without case; the program may be named bare.
        assert!(is_app_window(
            "fohmixer",
            r"C:\PROGRAM FILES\BRAVE\BRAVE.EXE",
            "brave.exe",
            PAGE
        ));
    }

    #[test]
    fn a_tab_another_page_or_another_program_is_no_app_window() {
        let brave = r"C:\Brave\brave.exe";
        // A tab window adds the browser's name.
        assert!(!is_app_window("fohmixer - Brave", brave, brave, PAGE));
        assert!(!is_app_window("Fohmixer tags", brave, brave, PAGE));
        assert!(!is_app_window("", brave, brave, PAGE));
        assert!(!is_app_window("Fohmixer", brave, brave, PAGE));
        assert!(!is_app_window(
            "fohmixer",
            r"C:\Edge\msedge.exe",
            brave,
            PAGE
        ));
        // An image that could not be read names no program.
        assert!(!is_app_window("fohmixer", "", brave, PAGE));
        assert!(!is_app_window("fohmixer", r"C:\dir\", r"C:\other\", PAGE));
    }

    /// The page the MBC install's app command opens.
    const PAGE: Option<&str> = Some("http://localhost:8480/");

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_app_page_is_the_app_words_value() {
        let app = words(&[
            r"C:\Brave\brave.exe",
            "--profile-directory=Profile 1",
            "--app=http://localhost:8480/",
            "--start-maximized",
        ]);
        assert_eq!(app_page(&app), Some("http://localhost:8480/"));
        assert_eq!(app_page(&words(&["brave.exe", "--new-window"])), None);
        assert_eq!(app_page(&words(&["brave.exe", "--app="])), None);
        assert_eq!(app_page(&[]), None);
    }

    #[test]
    fn an_app_window_without_its_page_title_is_still_the_app_window() {
        // The review round: the hub down (the error page names the host) or
        // the page loading (its address) must not start a second window.
        let brave = r"C:\Brave\brave.exe";
        for title in [
            "localhost",
            "localhost:8480/",
            "localhost:8480",
            "localhost:8480/#main",
            "http://localhost:8480/",
        ] {
            assert!(is_app_window(title, brave, brave, PAGE), "{title}");
        }
        // Without a page only the page's own title counts.
        assert!(!is_app_window("localhost", brave, brave, None));
        assert!(is_app_window("fohmixer", brave, brave, None));
    }

    #[test]
    fn a_tab_or_another_address_is_not_titled_by_the_page() {
        let page = "http://localhost:8480/";
        assert!(!titled_by_page("localhost - Brave", page));
        assert!(!titled_by_page("localhost:8480/ - Brave", page));
        assert!(!titled_by_page("example.org", page));
        assert!(!titled_by_page("local", page));
        assert!(!titled_by_page("", page));
        assert!(!titled_by_page("localhost:9000/", page));
        // A page without a host names nothing.
        assert!(!titled_by_page("", "http://"));
        assert!(!titled_by_page("x", "http://"));
        // A page given without a scheme.
        assert!(titled_by_page("localhost", "localhost:8480/"));
        assert!(titled_by_page("localhost:8480/", "localhost:8480"));
    }

    #[test]
    fn another_port_that_begins_with_the_pages_is_not_its_address() {
        let page = "http://localhost:80/";
        assert!(!titled_by_page("localhost:8080/", page));
        assert!(!titled_by_page("http://localhost:8080/", page));
        assert!(titled_by_page("localhost:80", page));
        assert!(titled_by_page("localhost:80/x", page));
        assert!(titled_by_page("localhost:80?a", page));
        assert!(titled_by_page("localhost:80#a", page));
        assert!(titled_by_page("http://localhost:80/", page));
    }

    #[test]
    fn open_focuses_an_open_app_window_else_starts_one() {
        assert_eq!(open_step(true, None), OpenStep::Focus);
        assert_eq!(open_step(true, Some(0)), OpenStep::Focus);
        assert_eq!(open_step(false, None), OpenStep::Start);
    }

    #[test]
    fn a_start_waits_for_its_window_15_s_before_another() {
        assert_eq!(START_GAP_MS, 15_000);
        assert_eq!(open_step(false, Some(0)), OpenStep::Wait);
        assert_eq!(open_step(false, Some(14_999)), OpenStep::Wait);
        assert_eq!(open_step(false, Some(15_000)), OpenStep::Start);
    }
}
