//! Presenter's stage tab (#90; the owner's approved
//! `docs/mockups/presenter-stage-v1.html`), pure: what the hub said of
//! Presenter's stage, what the pane shows (the live preview or the offline
//! panel, the header's words), each layout key's look and words, and when a
//! key's 1 s hold may switch the stage. `pages/presenter.rs` carries it out.
//! Since 1.37.0 also the column's stage miniature (way A of the owner's
//! approved `docs/mockups/presenter-stage-v2.html`): whether it shows (only
//! in the rail's free room, so nothing gets shorter), its size and its
//! caption.

use fohmixer_proto::client::StageLayout;

/// Presenter's stage as the hub's latest `presenter` message described it
/// (kept through a reconnect of the page's socket until the next one).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PresenterInfo {
    /// The hub's link to Presenter (none: not known yet).
    pub online: Option<bool>,
    /// Presenter's stage layouts, in its order.
    pub layouts: Vec<StageLayout>,
    /// The layout on stage now (none: not known).
    pub current: Option<String>,
}

/// The tab's title (the mockup's).
pub const TAB_TITLE: &str = "PRESENTER";

/// The pane's header (the mockup's).
pub const HEAD: &str = "STAGE";

/// The caption over the layout keys (the mockup's).
pub const CAPTION: &str = "STAGE LAYOUT · hold 1 s to switch";

/// The stage page the preview frames: Presenter's own, on the hub's origin,
/// as a preview (no wake lock, never counted as a stage display).
pub const FRAME_URL: &str = "/stage?preview=1";

/// The size Presenter's stage page is laid out at (1080p, as Presenter's
/// own operator preview frames it).
pub const STAGE_W: f64 = 1920.0;
pub const STAGE_H: f64 = 1080.0;

/// The layout keys' columns (the mockup's 3 × 3).
pub const COLUMNS: usize = 3;

/// The offline panel's two lines (the mockup's).
pub const DOWN_TITLE: &str = "Presenter can\u{2019}t be reached";
pub const DOWN_TEXT: &str = "trying again\u{2026}";

/// A key's second line while its press is full (the mockup's).
pub const RELEASE_TEXT: &str = "release \u{2014} switch";

/// A lit key's second line (the mockup's).
pub const ON_STAGE_TEXT: &str = "on stage now";

/// What the preview area shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preview {
    /// Presenter's stage page, framed.
    Frame,
    /// The offline panel: Presenter can't be reached.
    Down,
    /// Nothing yet: the hub has not decided (a black stage).
    Waiting,
}

impl Preview {
    /// Its `data-preview`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Frame => "frame",
            Self::Down => "down",
            Self::Waiting => "waiting",
        }
    }
}

/// What the preview shows for the hub's link (`online`): the frame only
/// while Presenter is up, so a frame mounts afresh when it comes back.
pub fn preview(online: Option<bool>) -> Preview {
    match online {
        Some(true) => Preview::Frame,
        Some(false) => Preview::Down,
        None => Preview::Waiting,
    }
}

/// The header's `on stage:` words: the layout on stage by its name (its
/// code when Presenter lists none such), `—` while Presenter is not up or
/// the layout is not known.
pub fn on_stage(info: &PresenterInfo) -> String {
    let name = info
        .current
        .as_deref()
        .filter(|_| info.online == Some(true))
        .map(|code| name_of(&info.layouts, code));
    format!(
        "on stage: {}",
        name.unwrap_or_else(|| "\u{2014}".to_string())
    )
}

/// A layout's name by its code (the code itself when none is listed).
pub fn name_of(layouts: &[StageLayout], code: &str) -> String {
    layouts
        .iter()
        .find(|l| l.code == code)
        .map_or_else(|| code.to_string(), |l| l.name.clone())
}

/// A layout key's look.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyLook {
    /// The one on stage: lit.
    On,
    /// Any other, ready for a hold.
    Off,
    /// Presenter (or the page's link) is down: dim, no touch acts.
    Dim,
}

impl KeyLook {
    /// Its `data-state`.
    pub fn name(self) -> &'static str {
        match self {
            Self::On => "on",
            Self::Off => "off",
            Self::Dim => "dim",
        }
    }
}

/// The look of the key of `code` (`ready`: [`can_switch`]).
pub fn key_look(code: &str, current: Option<&str>, ready: bool) -> KeyLook {
    if !ready {
        KeyLook::Dim
    } else if current == Some(code) {
        KeyLook::On
    } else {
        KeyLook::Off
    }
}

/// A key's second line: `on stage now` for the lit one, else Presenter's
/// own description. While its press is full the key shows [`RELEASE_TEXT`]
/// instead (its `data-full`, `presenter.css`): it switches at the lift.
pub fn key_note(look: KeyLook, description: &str) -> &str {
    if look == KeyLook::On {
        ON_STAGE_TEXT
    } else {
        description
    }
}

/// Whether a key's hold may switch the stage now: Presenter up, the page's
/// socket past its hello (`connected`), and no dropout under way
/// (`dropping`: a switch never waits in a stalled link, a stage must not
/// switch late).
pub fn can_switch(online: Option<bool>, connected: bool, dropping: bool) -> bool {
    online == Some(true) && connected && !dropping
}

/// What a key's lift switches to: `code`, only while [`can_switch`]; none:
/// refused.
pub fn switch_to(code: &str, ready: bool) -> Option<String> {
    ready.then(|| code.to_string())
}

/// The tab's red dot: Presenter down, or the page's own link to the hub
/// (not known yet is no dot).
pub fn tab_dot(online: Option<bool>, connected: bool) -> bool {
    online == Some(false) || !connected
}

/// The keys' rows for `n` layouts in [`COLUMNS`] columns (at least one).
pub fn rows(n: usize) -> usize {
    n.div_ceil(COLUMNS).max(1)
}

/// The scale of the 1920 × 1080 stage page in a preview `width` px wide
/// (none for no width yet: nothing is drawn).
pub fn scale(width: f64) -> Option<f64> {
    (width > 0.0).then_some(width / STAGE_W)
}

/// The flight recorder's key of a layout key (its `rail` steps' `keys`).
pub fn key_of(code: &str) -> String {
    format!("presenter|{code}")
}

/// Where a stage frame is shown: the PRESENTER tab's pane, or the column's
/// miniature (both the same frame, `pages::presenter::StageFrame`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// The PRESENTER tab's preview.
    Pane,
    /// The column's miniature, above TECHALERT.
    Column,
}

impl Place {
    /// The frame's class.
    pub fn class(self) -> &'static str {
        match self {
            Self::Pane => "pst-frame",
            Self::Column => "pst-frame mini-frame",
        }
    }

    /// The frame's `data-testid`.
    pub fn testid(self) -> &'static str {
        match self {
            Self::Pane => "presenter-frame",
            Self::Column => "mini-frame",
        }
    }
}

/// The miniature's caption on its left (the mockup's).
pub const MINI_HEAD: &str = "STAGE";

/// The miniature's caption on its right while Presenter can't be reached.
pub const MINI_OFFLINE: &str = "offline";

/// A rail button's row at its full height and the rows' gap (`page.css`
/// `.rail-main`: `minmax(24px, 44px)`, 4 px apart): with less room the rows
/// shrink, which the miniature must never cause.
pub const RAIL_ROW: f64 = 44.0;
pub const RAIL_ROW_GAP: f64 = 4.0;

/// The gap between the rail's parts (`page.css` `.rail`): the miniature
/// adds one.
pub const RAIL_GAP: f64 = 6.0;

/// The miniature's box around its picture (`presenter.css` `.stage-mini`):
/// its border and padding on each side, its caption's line and the gap
/// under it.
pub const MINI_EDGE: f64 = 5.0;
pub const MINI_CAPTION: f64 = 11.0;
pub const MINI_CAPTION_GAP: f64 = 3.0;

/// The widest screen that is a phone (`page.css`: `max-width: 520px` or
/// `max-height: 520px`, the thin strip with ☰ in the column's place).
pub const PHONE_MAX: f64 = 520.0;

/// Whether the viewport (`width`, `height`) is a phone's layout.
pub fn phone(width: f64, height: f64) -> bool {
    width <= PHONE_MAX || height <= PHONE_MAX
}

/// The miniature's picture width in a rail `rail_width` px wide (none when
/// there is no room for one).
pub fn mini_width(rail_width: f64) -> Option<f64> {
    let width = rail_width - 2.0 * MINI_EDGE;
    (width > 0.0).then_some(width)
}

/// The miniature's whole height for a picture `width` px wide: the stage's
/// 16:9 and the box around it.
pub fn mini_height(width: f64) -> f64 {
    2.0 * MINI_EDGE + MINI_CAPTION + MINI_CAPTION_GAP + width * STAGE_H / STAGE_W
}

/// The rail's rows from its resolved `grid-template-rows` (each track a
/// length in px; `none` for an empty rail).
pub fn track_count(tracks: &str) -> usize {
    tracks
        .split_whitespace()
        .filter(|t| t.ends_with("px"))
        .count()
}

/// Whether the miniature, `mini` px high, fits the rail's free room: `room`
/// is what the rail's buttons have without it (the rail less its foot and
/// the gap between them), `rows` the buttons' rows. With the miniature and
/// its gap taken out, every row must still get its full height, else it
/// stays hidden (the owner's rule: nothing gets shorter for it).
pub fn mini_fits(room: f64, rows: usize, mini: f64) -> bool {
    let need = if rows == 0 {
        0.0
    } else {
        rows as f64 * RAIL_ROW + (rows - 1) as f64 * RAIL_ROW_GAP
    };
    room - mini - RAIL_GAP >= need
}

/// Whether the column shows the miniature: a hub with `[presenter]`
/// (`configured`), the PRESENTER tab not open (its big preview is beside
/// it), not a phone (its column lies behind ☰) and room in the rail.
pub fn mini_shows(configured: bool, tab_open: bool, phone: bool, fits: bool) -> bool {
    configured && !tab_open && !phone && fits
}

/// The miniature's caption on its right, and whether it is the red
/// `offline`: the layout on stage by its name, `—` while it is not known.
pub fn mini_note(info: &PresenterInfo) -> (String, bool) {
    match info.online {
        Some(false) => (MINI_OFFLINE.to_string(), true),
        Some(true) => (
            info.current.as_deref().map_or_else(
                || "\u{2014}".to_string(),
                |code| name_of(&info.layouts, code),
            ),
            false,
        ),
        None => ("\u{2014}".to_string(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(code: &str, name: &str) -> StageLayout {
        StageLayout {
            code: code.into(),
            name: name.into(),
            description: format!("{name} shown"),
        }
    }

    fn info(online: Option<bool>, current: Option<&str>) -> PresenterInfo {
        PresenterInfo {
            online,
            layouts: vec![layout("timer", "TIMER"), layout("preach", "PREACH")],
            current: current.map(str::to_string),
        }
    }

    #[test]
    fn the_preview_is_the_frame_only_while_presenter_is_up() {
        assert_eq!(preview(Some(true)), Preview::Frame);
        assert_eq!(preview(Some(false)), Preview::Down);
        assert_eq!(preview(None), Preview::Waiting);
        assert_eq!(Preview::Frame.name(), "frame");
        assert_eq!(Preview::Down.name(), "down");
        assert_eq!(Preview::Waiting.name(), "waiting");
    }

    #[test]
    fn the_header_names_the_layout_on_stage_only_while_presenter_is_up() {
        assert_eq!(
            on_stage(&info(Some(true), Some("preach"))),
            "on stage: PREACH"
        );
        assert_eq!(on_stage(&info(Some(true), Some("api"))), "on stage: api");
        for down in [
            info(Some(false), Some("preach")),
            info(None, Some("preach")),
            info(Some(true), None),
        ] {
            assert_eq!(on_stage(&down), "on stage: \u{2014}", "{down:?}");
        }
        assert_eq!(name_of(&[], "x"), "x");
    }

    #[test]
    fn the_lit_key_is_the_one_on_stage_and_every_key_dims_when_none_can_switch() {
        assert_eq!(key_look("timer", Some("timer"), true), KeyLook::On);
        assert_eq!(key_look("preach", Some("timer"), true), KeyLook::Off);
        assert_eq!(key_look("timer", None, true), KeyLook::Off);
        assert_eq!(key_look("timer", Some("timer"), false), KeyLook::Dim);
        assert_eq!(key_look("preach", Some("timer"), false), KeyLook::Dim);
        assert_eq!(KeyLook::On.name(), "on");
        assert_eq!(KeyLook::Off.name(), "off");
        assert_eq!(KeyLook::Dim.name(), "dim");
    }

    #[test]
    fn a_keys_second_line_is_on_stage_when_lit_else_its_description() {
        assert_eq!(key_note(KeyLook::On, "d"), "on stage now");
        assert_eq!(key_note(KeyLook::Off, "d"), "d");
        assert_eq!(key_note(KeyLook::Dim, "d"), "d");
        assert_eq!(RELEASE_TEXT, "release \u{2014} switch");
        assert_eq!(ON_STAGE_TEXT, "on stage now");
    }

    #[test]
    fn a_hold_switches_only_with_presenter_up_the_page_connected_and_no_dropout() {
        assert!(can_switch(Some(true), true, false));
        assert!(!can_switch(Some(false), true, false));
        assert!(!can_switch(None, true, false));
        assert!(!can_switch(Some(true), false, false));
        assert!(!can_switch(Some(true), true, true));
        assert_eq!(switch_to("timer", true).as_deref(), Some("timer"));
        assert_eq!(switch_to("timer", false), None);
    }

    #[test]
    fn the_tab_shows_its_dot_while_presenter_or_the_page_is_down() {
        assert!(!tab_dot(Some(true), true));
        assert!(!tab_dot(None, true), "not known yet");
        assert!(tab_dot(Some(false), true));
        assert!(tab_dot(Some(true), false));
        assert!(tab_dot(None, false));
    }

    #[test]
    fn the_keys_fill_three_columns_and_the_stage_scales_to_the_preview() {
        assert_eq!(rows(0), 1);
        assert_eq!(rows(1), 1);
        assert_eq!(rows(3), 1);
        assert_eq!(rows(4), 2);
        assert_eq!(rows(9), 3);
        assert_eq!(rows(10), 4);
        assert_eq!(scale(960.0), Some(0.5));
        assert_eq!(scale(1920.0), Some(1.0));
        assert_eq!(scale(0.0), None);
        assert_eq!(scale(-4.0), None);
        assert_eq!((STAGE_W, STAGE_H, COLUMNS), (1920.0, 1080.0, 3));
    }

    #[test]
    fn the_texts_are_the_mockups() {
        assert_eq!(TAB_TITLE, "PRESENTER");
        assert_eq!(HEAD, "STAGE");
        assert_eq!(CAPTION, "STAGE LAYOUT \u{b7} hold 1 s to switch");
        assert_eq!(DOWN_TITLE, "Presenter can\u{2019}t be reached");
        assert_eq!(DOWN_TEXT, "trying again\u{2026}");
        assert_eq!(FRAME_URL, "/stage?preview=1");
        assert_eq!(key_of("timer"), "presenter|timer");
    }
    #[test]
    fn a_frame_is_named_by_its_place() {
        assert_eq!(Place::Pane.class(), "pst-frame");
        assert_eq!(Place::Column.class(), "pst-frame mini-frame");
        assert_eq!(Place::Pane.testid(), "presenter-frame");
        assert_eq!(Place::Column.testid(), "mini-frame");
    }

    #[test]
    fn a_phone_is_520_px_or_less_either_way() {
        assert!(!phone(1194.0, 834.0));
        assert!(!phone(521.0, 521.0));
        assert!(phone(520.0, 834.0));
        assert!(phone(1194.0, 520.0));
        assert!(phone(390.0, 844.0));
        assert!(phone(844.0, 390.0));
    }

    #[test]
    fn the_miniature_is_the_rails_width_less_its_box_and_16_by_9() {
        assert_eq!(mini_width(118.0), Some(108.0));
        assert_eq!(mini_width(10.5), Some(0.5));
        assert_eq!(mini_width(10.0), None);
        assert_eq!(mini_width(0.0), None);
        // 10 for the box, 11 + 3 for the caption, 192 x 108 for the picture.
        assert_eq!(mini_height(192.0), 132.0);
        assert_eq!(mini_height(0.0), 24.0);
        assert_eq!(
            (MINI_EDGE, MINI_CAPTION, MINI_CAPTION_GAP),
            (5.0, 11.0, 3.0)
        );
    }

    #[test]
    fn the_rails_rows_are_its_resolved_tracks() {
        assert_eq!(track_count("44px 44px 44px"), 3);
        assert_eq!(track_count("43.5px"), 1);
        assert_eq!(track_count("none"), 0);
        assert_eq!(track_count(""), 0);
        assert_eq!(track_count("[a] 44px [b] 24px"), 2);
    }

    #[test]
    fn the_miniature_fits_only_where_every_rail_row_keeps_its_full_height() {
        // Three rows: 3 x 44 + 2 x 4 = 140; a 100 px miniature and its 6 px gap.
        assert!(mini_fits(246.0, 3, 100.0));
        assert!(!mini_fits(245.9, 3, 100.0));
        // One row: no gap between rows.
        assert!(mini_fits(150.0, 1, 100.0));
        assert!(!mini_fits(149.9, 1, 100.0));
        // No buttons: only the miniature and its gap.
        assert!(mini_fits(106.0, 0, 100.0));
        assert!(!mini_fits(105.9, 0, 100.0));
        assert_eq!((RAIL_ROW, RAIL_ROW_GAP, RAIL_GAP), (44.0, 4.0, 6.0));
    }

    #[test]
    fn the_miniature_shows_with_presenter_set_up_its_tab_shut_on_a_tablet_with_room() {
        assert!(mini_shows(true, false, false, true));
        assert!(!mini_shows(false, false, false, true));
        assert!(!mini_shows(true, true, false, true));
        assert!(!mini_shows(true, false, true, true));
        assert!(!mini_shows(true, false, false, false));
    }

    #[test]
    fn the_miniatures_caption_names_the_layout_or_says_offline() {
        assert_eq!(
            mini_note(&info(Some(true), Some("preach"))),
            ("PREACH".to_string(), false)
        );
        assert_eq!(
            mini_note(&info(Some(true), Some("api"))),
            ("api".to_string(), false)
        );
        assert_eq!(
            mini_note(&info(Some(true), None)),
            ("\u{2014}".to_string(), false)
        );
        assert_eq!(
            mini_note(&info(Some(false), Some("preach"))),
            ("offline".to_string(), true)
        );
        assert_eq!(
            mini_note(&info(None, Some("preach"))),
            ("\u{2014}".to_string(), false)
        );
        assert_eq!(MINI_HEAD, "STAGE");
        assert_eq!(PHONE_MAX, 520.0);
    }
}
