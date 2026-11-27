//! The app window stays in front at logon (#280): on the MBC PC the tray and
//! the MBC mixer's app window start at logon, the MBC Live after them; Live
//! loads its set for a minute or two and then takes the foreground over the
//! app window. The tray watches for that moment ([`Watch`]) and brings the
//! app window forward once, a little after Live's set has loaded. It acts
//! only when it saw Live not yet loaded first (a tray restarted by an update
//! while Live runs never takes the foreground) and only within [`WATCH_MS`]
//! of its start.

/// How often the glue looks at the windows.
pub const EVERY_MS: u64 = 1000;
/// How long after the tray's start it watches (a Live started later by hand
/// is the operator's own doing).
pub const WATCH_MS: u64 = 10 * 60 * 1000;
/// How long after Live's set shows loaded the app window comes forward
/// (Live finishes its start and takes the foreground meanwhile).
pub const SETTLE_MS: u64 = 5000;

/// What the glue does after a look at the windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Look again in [`EVERY_MS`].
    Keep,
    /// Bring the app window to the front now; the watch ends.
    Front,
    /// The watch ends with nothing done.
    Stop,
}

/// The watch's state between looks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Watch {
    /// Live was seen not yet loaded (or not running) since the start.
    saw_unloaded: bool,
    /// When (ms since the start) Live's set was first seen loaded after that.
    loaded_at: Option<u64>,
}

impl Watch {
    /// One look at `elapsed_ms` after the tray's start: whether Live's set
    /// shows loaded now.
    pub fn step(&mut self, elapsed_ms: u64, live_loaded: bool) -> Step {
        let _ = (elapsed_ms, live_loaded);
        Step::Stop
    }
}

/// Whether a window titled `title` in the process `image` is a Live whose
/// set has loaded: Live's main window names its set (`MBC - Ableton Live 12
/// Suite`, `MBC* - …` with unsaved changes); its start screen does not.
pub fn is_loaded_live(title: &str, image: &str) -> bool {
    let _ = (title, image);
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIVE: &str = r"C:\ProgramData\Ableton\Live 12 Suite\Program\Ableton Live 12 Suite.exe";

    #[test]
    fn a_live_window_naming_its_set_is_loaded() {
        assert!(is_loaded_live("MBC - Ableton Live 12 Suite", LIVE));
        assert!(is_loaded_live("MBC* - Ableton Live 12 Suite", LIVE));
        assert!(is_loaded_live(
            "Untitled - Ableton Live 12 Suite",
            LIVE.to_uppercase().as_str()
        ));
    }

    #[test]
    fn the_start_screen_and_other_programs_are_not() {
        assert!(!is_loaded_live("Ableton Live 12 Suite", LIVE));
        assert!(!is_loaded_live("", LIVE));
        // A browser tab about Live is no Live.
        assert!(!is_loaded_live(
            "MBC - Ableton Live 12 Suite",
            r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe"
        ));
        assert!(!is_loaded_live("MBC - Ableton Live 12 Suite", ""));
    }

    #[test]
    fn the_app_comes_forward_once_after_live_loads_and_settles() {
        let mut watch = Watch::default();
        assert_eq!(watch.step(0, false), Step::Keep);
        assert_eq!(watch.step(60_000, false), Step::Keep);
        assert_eq!(watch.step(90_000, true), Step::Keep);
        assert_eq!(watch.step(90_000 + SETTLE_MS - 1, true), Step::Keep);
        assert_eq!(watch.step(90_000 + SETTLE_MS, true), Step::Front);
    }

    #[test]
    fn a_live_already_loaded_at_the_start_is_left_alone() {
        let mut watch = Watch::default();
        assert_eq!(watch.step(0, true), Step::Stop);
    }

    #[test]
    fn a_live_closing_again_before_it_settles_waits_for_the_next_load() {
        let mut watch = Watch::default();
        assert_eq!(watch.step(0, false), Step::Keep);
        assert_eq!(watch.step(10_000, true), Step::Keep);
        assert_eq!(watch.step(11_000, false), Step::Keep);
        assert_eq!(watch.step(10_000 + SETTLE_MS, true), Step::Keep);
        assert_eq!(watch.step(10_000 + 2 * SETTLE_MS, true), Step::Front);
    }

    #[test]
    fn the_watch_ends_after_its_window_with_nothing_done() {
        let mut watch = Watch::default();
        assert_eq!(watch.step(0, false), Step::Keep);
        assert_eq!(watch.step(WATCH_MS - 1, false), Step::Keep);
        assert_eq!(watch.step(WATCH_MS, false), Step::Stop);
        // Loaded only after the window: nothing.
        let mut watch = Watch::default();
        assert_eq!(watch.step(0, false), Step::Keep);
        assert_eq!(watch.step(WATCH_MS, true), Step::Stop);
    }

    #[test]
    fn a_load_inside_the_window_still_settles_past_it() {
        let mut watch = Watch::default();
        assert_eq!(watch.step(0, false), Step::Keep);
        assert_eq!(watch.step(WATCH_MS - 1000, true), Step::Keep);
        assert_eq!(watch.step(WATCH_MS - 1000 + SETTLE_MS, true), Step::Front);
    }
}
