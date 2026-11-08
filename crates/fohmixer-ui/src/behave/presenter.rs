//! Presenter's stage tab (#90; the owner's approved
//! `docs/mockups/presenter-stage-v1.html`), pure: what the hub said of
//! Presenter's stage, what the pane shows (the live preview or the offline
//! panel, the header's words), each layout key's look and words, and when a
//! key's 1 s hold may switch the stage. `pages/presenter.rs` carries it out.

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
}
