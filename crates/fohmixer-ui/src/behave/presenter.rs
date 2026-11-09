//! Presenter's stage tab (#90; the owner's approved
//! `docs/mockups/presenter-stage-v1.html`), pure: what the hub said of
//! Presenter's stage, what the pane shows (the live preview or the offline
//! panel, the header's words), each layout key's look and words, and when a
//! key's 1 s hold may switch the stage. `pages/presenter.rs` carries it out.
//! Since 1.37.0 also the column's stage miniature (way A of the owner's
//! approved `docs/mockups/presenter-stage-v2.html`): whether it shows and
//! its size (since 1.38.2 never under its least picture: the rail's rows
//! give it room, the faders never) and its caption.

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

/// The least picture the miniature shows (px high, #90's DECIDED of
/// 2026-11-09 evening: the caption and about 40 px of picture); with less
/// room the rail's rows shrink for it.
pub const MINI_MIN_PICTURE: f64 = 40.0;

/// The miniature's picture width in a rail `rail_width` px wide (none when
/// there is no room for one).
pub fn mini_width(rail_width: f64) -> Option<f64> {
    let width = rail_width - 2.0 * MINI_EDGE;
    (width > 0.0).then_some(width)
}

/// The miniature's box around a picture `height` px high: its border and
/// padding, its caption's line and the gap under it.
pub fn mini_height(height: f64) -> f64 {
    2.0 * MINI_EDGE + MINI_CAPTION + MINI_CAPTION_GAP + height
}

/// The rail's rows from its resolved `grid-template-rows` (each track a
/// length in px; `none` for an empty rail).
pub fn track_count(tracks: &str) -> usize {
    tracks
        .split_whitespace()
        .filter(|t| t.ends_with("px"))
        .count()
}

/// The rail as measured: its width, the room its buttons have without the
/// miniature (the rail less its foot and the gap between them) and the
/// buttons' rows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RailRoom {
    pub width: f64,
    pub room: f64,
    pub rows: usize,
}

/// The rows' height at their full 44 px, 4 px apart.
pub fn rows_height(rows: usize) -> f64 {
    let gaps = rows.saturating_sub(1);
    rows as f64 * RAIL_ROW + gaps as f64 * RAIL_ROW_GAP
}

/// The miniature's picture (width, height) in the rail (#90's DECIDED of
/// 2026-11-09 evening, option A): 16:9, as wide as the rail allows, as high
/// as the room the rail's rows leave at their full 44 px (less the
/// miniature's box and its 6 px gap), a whole px high when that room cuts
/// it, and never under `MINI_MIN_PICTURE` (unless the rail is too narrow for
/// it: its width at 16:9): below it the rows give the room up (`page.css`
/// lets them shrink to 24 px); the faders never. None only for a rail with
/// no width for a picture.
pub fn mini_picture(rail: RailRoom) -> Option<(f64, f64)> {
    let widest = mini_width(rail.width)?;
    let full = widest * STAGE_H / STAGE_W;
    let room = rail.room - rows_height(rail.rows) - RAIL_GAP - mini_height(0.0);
    if room >= full {
        return Some((widest, full));
    }
    let height = room.floor().max(MINI_MIN_PICTURE).min(full);
    Some((height * STAGE_W / STAGE_H, height))
}

/// Why the miniature is not there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiniHide {
    /// The hub has no `[presenter]`.
    Unconfigured,
    /// The PRESENTER tab is open (its big preview is beside it).
    Open,
    /// A phone: its column lies behind ☰.
    Phone,
    /// The rail not measured yet.
    Unmeasured,
    /// A rail with no width for a picture.
    Room,
}

impl MiniHide {
    /// The recorder's word.
    pub fn name(self) -> &'static str {
        match self {
            Self::Unconfigured => "unconfigured",
            Self::Open => "open",
            Self::Phone => "phone",
            Self::Unmeasured => "unmeasured",
            Self::Room => "room",
        }
    }
}

/// Whether the column shows the miniature, and its picture's size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MiniFit {
    Shown { width: f64, height: f64 },
    Hidden(MiniHide),
}

impl MiniFit {
    /// The picture's size while it shows.
    pub fn picture(self) -> Option<(f64, f64)> {
        match self {
            Self::Shown { width, height } => Some((width, height)),
            Self::Hidden(_) => None,
        }
    }
}

/// The column's miniature (#90): with `[presenter]` (`configured`), the
/// PRESENTER tab shut, not a phone and a measured rail, at `mini_picture`'s
/// size; the first reason that fails, in that order.
pub fn mini_fit(configured: bool, tab_open: bool, phone: bool, rail: Option<RailRoom>) -> MiniFit {
    if !configured {
        return MiniFit::Hidden(MiniHide::Unconfigured);
    }
    if tab_open {
        return MiniFit::Hidden(MiniHide::Open);
    }
    if phone {
        return MiniFit::Hidden(MiniHide::Phone);
    }
    let Some(rail) = rail else {
        return MiniFit::Hidden(MiniHide::Unmeasured);
    };
    mini_picture(rail).map_or(MiniFit::Hidden(MiniHide::Room), |(width, height)| {
        MiniFit::Shown { width, height }
    })
}

/// Whether the recorder notes a decision (`mini_fit`): every one but the
/// moments before the hub told of Presenter (a page's first, and every
/// page of a hub without `[presenter]`) and before the rail is measured.
pub fn mini_fit_noted(fit: MiniFit) -> bool {
    !matches!(
        fit,
        MiniFit::Hidden(MiniHide::Unconfigured | MiniHide::Unmeasured)
    )
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
    fn the_miniatures_box_is_the_rails_width_less_its_edges_and_its_caption() {
        assert_eq!(mini_width(118.0), Some(108.0));
        assert_eq!(mini_width(10.5), Some(0.5));
        assert_eq!(mini_width(10.0), None);
        assert_eq!(mini_width(0.0), None);
        // 10 for the box, 11 + 3 for the caption, then the picture.
        assert_eq!(mini_height(108.0), 132.0);
        assert_eq!(mini_height(0.0), 24.0);
        assert_eq!(
            (MINI_EDGE, MINI_CAPTION, MINI_CAPTION_GAP, MINI_MIN_PICTURE),
            (5.0, 11.0, 3.0, 40.0)
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
    fn the_rows_keep_44_px_each_and_4_px_between() {
        assert_eq!(rows_height(0), 0.0);
        assert_eq!(rows_height(1), 44.0);
        assert_eq!(rows_height(3), 140.0);
        assert_eq!((RAIL_ROW, RAIL_ROW_GAP, RAIL_GAP), (44.0, 4.0, 6.0));
    }

    fn rail(width: f64, room: f64, rows: usize) -> RailRoom {
        RailRoom { width, room, rows }
    }

    #[test]
    fn with_room_the_picture_is_the_rails_whole_width() {
        // 118 px rail: a 108 x 60.75 picture, its box 84.75 px. Three rows
        // 140 px, the gap 6: 230.75 px of room is exactly enough.
        assert_eq!(mini_picture(rail(118.0, 230.75, 3)), Some((108.0, 60.75)));
        assert_eq!(mini_picture(rail(118.0, 500.0, 3)), Some((108.0, 60.75)));
    }

    #[test]
    fn with_less_room_the_picture_shrinks_to_a_whole_px_high_16_by_9() {
        // 230.5 px: 60.5 px of picture height left, floored to 60.
        assert_eq!(
            mini_picture(rail(118.0, 230.5, 3)),
            Some((60.0 * 16.0 / 9.0, 60.0))
        );
        // 50 px of room left for the picture.
        assert_eq!(
            mini_picture(rail(118.0, 220.0, 3)),
            Some((50.0 * 16.0 / 9.0, 50.0))
        );
        // 40.99 px left: 40.
        assert_eq!(
            mini_picture(rail(118.0, 210.99, 3)),
            Some((40.0 * 16.0 / 9.0, 40.0))
        );
    }

    #[test]
    fn under_the_least_picture_the_rows_give_up_the_room() {
        let least = Some((40.0 * 16.0 / 9.0, 40.0));
        // Exactly the least picture after 3 rows at 44 px.
        assert_eq!(mini_picture(rail(118.0, 210.0, 3)), least);
        // Less: still the least picture, the rows shrink.
        assert_eq!(mini_picture(rail(118.0, 209.99, 3)), least);
        assert_eq!(mini_picture(rail(118.0, 100.0, 8)), least);
        assert_eq!(mini_picture(rail(118.0, 0.0, 0)), least);
        // One row, no gap between rows: 44 + 6 + 24 + 41 = 115.
        assert_eq!(
            mini_picture(rail(118.0, 115.0, 1)),
            Some((41.0 * 16.0 / 9.0, 41.0))
        );
        // No buttons: 6 + 24 + 41 = 71.
        assert_eq!(
            mini_picture(rail(118.0, 71.0, 0)),
            Some((41.0 * 16.0 / 9.0, 41.0))
        );
    }

    #[test]
    fn a_narrow_rail_caps_the_picture_by_its_width() {
        // 90 px rail: at most 80 x 45; with no room the least picture.
        assert_eq!(mini_picture(rail(90.0, 500.0, 0)), Some((80.0, 45.0)));
        assert_eq!(
            mini_picture(rail(90.0, 0.0, 8)),
            Some((40.0 * 16.0 / 9.0, 40.0))
        );
        // A rail narrower than the least picture: its width, under 40 px high.
        assert_eq!(mini_picture(rail(50.0, 0.0, 0)), Some((40.0, 22.5)));
        assert_eq!(mini_picture(rail(10.0, 500.0, 0)), None);
    }

    #[test]
    fn the_miniature_hides_for_the_first_reason_in_order() {
        let roomy = Some(rail(118.0, 500.0, 3));
        let shown = MiniFit::Shown {
            width: 108.0,
            height: 60.75,
        };
        assert_eq!(mini_fit(true, false, false, roomy), shown);
        assert_eq!(shown.picture(), Some((108.0, 60.75)));
        assert_eq!(
            mini_fit(false, true, true, None),
            MiniFit::Hidden(MiniHide::Unconfigured)
        );
        assert_eq!(
            mini_fit(true, true, true, None),
            MiniFit::Hidden(MiniHide::Open)
        );
        assert_eq!(
            mini_fit(true, false, true, None),
            MiniFit::Hidden(MiniHide::Phone)
        );
        assert_eq!(
            mini_fit(true, false, false, None),
            MiniFit::Hidden(MiniHide::Unmeasured)
        );
        assert_eq!(
            mini_fit(true, false, false, Some(rail(118.0, 100.0, 3))),
            MiniFit::Shown {
                width: 40.0 * 16.0 / 9.0,
                height: 40.0
            }
        );
        assert_eq!(
            mini_fit(true, false, false, Some(rail(10.0, 500.0, 3))),
            MiniFit::Hidden(MiniHide::Room)
        );
        assert_eq!(MiniFit::Hidden(MiniHide::Room).picture(), None);
    }

    #[test]
    fn every_decision_but_the_unmeasured_rail_is_noted_by_its_word() {
        assert!(mini_fit_noted(MiniFit::Shown {
            width: 1.0,
            height: 1.0
        }));
        assert!(mini_fit_noted(MiniFit::Hidden(MiniHide::Room)));
        assert!(mini_fit_noted(MiniFit::Hidden(MiniHide::Open)));
        assert!(mini_fit_noted(MiniFit::Hidden(MiniHide::Phone)));
        assert!(!mini_fit_noted(MiniFit::Hidden(MiniHide::Unmeasured)));
        assert!(!mini_fit_noted(MiniFit::Hidden(MiniHide::Unconfigured)));
        let words: Vec<_> = [
            MiniHide::Unconfigured,
            MiniHide::Open,
            MiniHide::Phone,
            MiniHide::Unmeasured,
            MiniHide::Room,
        ]
        .map(MiniHide::name)
        .into();
        assert_eq!(
            words,
            ["unconfigured", "open", "phone", "unmeasured", "room"]
        );
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
