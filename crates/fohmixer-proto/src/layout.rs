//! The layout document, `layout.json` schema 1 (S3 design note §5; spec
//! §2.5): pages with nested pagers and tab bars, a root overlay, and placed
//! items bound by the general binding form (`instance` + `anchor` + `path`).
//! Every frame is in canvas coordinates (the TouchOSC canvas, 2360×1640);
//! node order is z-order. The import tool writes it, the hub validates and
//! serves it, the UI renders it.
//!
//! An unknown field is an error, not ignored: the file is edited by hand
//! later (D4), and a mistyped field name must not pass silently. (`Item`
//! itself cannot deny them — it flattens its kind — so each kind does.)

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::HUB_STAGE_AUT;
use crate::path::{PathError, escape_name, parse_steps, steps_text};

/// The schema version this build reads.
pub const LAYOUT_SCHEMA: u32 = 1;

/// How far (px) a frame may stick out of the canvas before it is an error
/// (rounding in the import).
const CANVAS_SLACK: f64 = 0.5;

/// The layout document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub schema: u32,
    pub canvas: Canvas,
    /// The root tab bar (the top-level pages' tabs).
    #[serde(default)]
    pub tabbar: TabBar,
    pub pages: Vec<Page>,
    /// Items drawn over every page (TechAlert, REFRESH ALL, the alert box).
    #[serde(default)]
    pub overlay: Vec<Item>,
    #[serde(default)]
    pub config: LayoutConfig,
    /// What the import dropped or could not reproduce (free-form).
    #[serde(default)]
    pub report: Value,
}

/// `GET /api/layout`: the served layout and its revision (the `layout`
/// message's `rev`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutResponse {
    pub rev: u64,
    pub layout: Layout,
}

/// The canvas size in px.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Canvas {
    pub w: f64,
    pub h: f64,
}

/// A rectangle in canvas coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Where a tab bar sits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    #[default]
    Top,
    Right,
    Bottom,
    Left,
}

/// A tab bar: its side, its thickness (px) and the page shown first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TabBar {
    #[serde(default)]
    pub orientation: Orientation,
    #[serde(default)]
    pub bar_size: f64,
    #[serde(default)]
    pub default_page: usize,
}

/// One page's tab.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tab {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_size: Option<f64>,
}

/// A page (a tab of the root tab bar or of a nested pager).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub tab: Tab,
    #[serde(default)]
    pub items: Vec<Item>,
    /// A nested pager on this page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pager: Option<Pager>,
}

/// A nested pager: its frame, its tab bar and its pages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pager {
    pub frame: Frame,
    #[serde(default)]
    pub tabbar: TabBar,
    pub pages: Vec<Page>,
}

/// Layout-wide settings (the TouchOSC `Conf` text and UI switches).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutConfig {
    /// Group tracks unfolded on every refresh (spec F7).
    #[serde(default)]
    pub unfold: Vec<UnfoldTarget>,
    /// Fader touch shaping and the post-release delay (spec X3); the UI's
    /// default when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fader_shaping: Option<bool>,
}

/// A group track to unfold: its instance and exact name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnfoldTarget {
    pub instance: String,
    pub name: String,
}

/// A placed item: its frame, z-order, style and kind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub frame: Frame,
    #[serde(default)]
    pub z: i64,
    #[serde(default)]
    pub style: Style,
    #[serde(flatten)]
    pub kind: ItemKind,
}

/// Colours are `#RRGGBB` or `#RRGGBBAA`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Style {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bg: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_size: Option<f64>,
    /// Text drawn vertically (area titles).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub vertical: bool,
}

/// What an item is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ItemKind {
    /// A background box, optionally with a title.
    Area {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    /// A mixer strip (boxed: by far the largest kind).
    Strip(Box<Strip>),
    /// A group-track solo toggle (spec F14).
    Solo { binding: Binding },
    /// The stage-mic button: an inverted mute (spec F15). `aut`: the hub's
    /// STAGE AUT rule drives this binding's mute.
    Stage {
        binding: Binding,
        #[serde(default)]
        aut: bool,
    },
    /// A toggle of a hub value (the STAGE AUT button).
    HubToggle { key: String, label: String },
    /// A former MIDI toggle writing its targets directly (spec F17, F18).
    ParamToggle {
        label: String,
        targets: Vec<ParamTarget>,
        press: Press,
    },
    /// A former MIDI fader writing its targets directly (spec F18).
    ParamFader {
        label: String,
        targets: Vec<ParamTarget>,
    },
    /// The TechAlert overlay: blinks while its track is unmuted (spec F16).
    Alert { binding: Binding, period_ms: u32 },
    /// REFRESH ALL (spec F6).
    Refresh {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// A static text.
    Label { text: String },
}

/// A strip: its binding, kind and per-child geometry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Strip {
    pub binding: Binding,
    pub strip_kind: StripKind,
    #[serde(default)]
    pub children: StripChildren,
    /// A mute change needs a second tap (spec F12).
    #[serde(default)]
    pub mute_guard: bool,
}

/// The strip kinds the import recognises (spec §2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StripKind {
    Standard,
    Narrow,
    Return,
    Solid,
    MeterMuteOnly,
}

/// The frames of a strip's parts (canvas coordinates); a missing part is not
/// drawn.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StripChildren {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fader: Option<Frame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pan: Option<Frame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mute: Option<Frame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meter: Option<Frame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Frame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db: Option<Frame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Frame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_label: Option<Frame>,
}

impl StripChildren {
    /// The present parts, named.
    pub fn frames(&self) -> Vec<(&'static str, Frame)> {
        [
            ("fader", self.fader),
            ("pan", self.pan),
            ("mute", self.mute),
            ("meter", self.meter),
            ("status", self.status),
            ("db", self.db),
            ("label", self.label),
            ("instance_label", self.instance_label),
        ]
        .into_iter()
        .filter_map(|(name, frame)| frame.map(|f| (name, f)))
        .collect()
    }
}

/// One target of a former MIDI control: a binding, a property and either
/// the values written for on/off (toggle) or the scaling (fader).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParamTarget {
    pub binding: Binding,
    pub prop: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<Scale>,
}

/// How a former MIDI toggle reacts to touch (the TouchOSC button type and
/// its script).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Press {
    Toggle,
    DoubleTapLatch,
    PulseAndDoubleTapLatch,
}

/// How a fader position maps onto a target: `cc_linear` is the MIDI map's
/// full range, LOM value = position × (max − min) + min.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scale {
    CcLinear,
}

/// The general binding form (spec §2.5): an instance, an anchor, and a path
/// relative to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub instance: String,
    pub anchor: Anchor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// What a binding starts from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Anchor {
    /// A track by exact name.
    Track {
        name: String,
    },
    /// A return track by exact name.
    Return {
        name: String,
    },
    Master,
    Song,
}

impl Binding {
    /// The LOM target path (S2 design note §3.1) the binding names.
    pub fn target(&self) -> Result<String, PathError> {
        let mut text = match &self.anchor {
            Anchor::Track { name } => format!("live_set tracks[name={}]", escape_name(name)),
            Anchor::Return { name } => {
                format!("live_set return_tracks[name={}]", escape_name(name))
            }
            Anchor::Master => "live_set master_track".to_string(),
            Anchor::Song => "live_set".to_string(),
        };
        let steps = parse_steps(self.path.as_deref().unwrap_or(""))?;
        if !steps.is_empty() {
            text.push(' ');
            text.push_str(&steps_text(&steps));
        }
        Ok(text)
    }
}

/// One problem `validate` found: where (`pages[0].items[3]`) and what.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutError {
    pub at: String,
    pub message: String,
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.at, self.message)
    }
}

/// Whether `color` is `#RRGGBB` or `#RRGGBBAA`.
pub fn is_color(color: &str) -> bool {
    let Some(hex) = color.strip_prefix('#') else {
        return false;
    };
    (hex.len() == 6 || hex.len() == 8) && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

impl Layout {
    /// Every problem of the document; empty when it may be served.
    pub fn validate(&self) -> Vec<LayoutError> {
        let mut v = Validator {
            canvas: self.canvas,
            errors: Vec::new(),
            page_ids: HashSet::new(),
            item_ids: HashSet::new(),
        };
        if self.schema != LAYOUT_SCHEMA {
            v.error(
                "schema",
                format!("schema {} is not {LAYOUT_SCHEMA}", self.schema),
            );
        }
        let (w, h) = (self.canvas.w, self.canvas.h);
        let sized = w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0;
        if !sized {
            v.error("canvas", format!("{w}×{h} is not a canvas size"));
        }
        v.pages("pages", &self.pages, &self.tabbar);
        for (i, item) in self.overlay.iter().enumerate() {
            v.item(&format!("overlay[{i}]"), item);
        }
        for (i, target) in self.config.unfold.iter().enumerate() {
            if target.instance.is_empty() || target.name.is_empty() {
                v.error(
                    &format!("config.unfold[{i}]"),
                    "needs an instance and a name".to_string(),
                );
            }
        }
        v.errors
    }

    /// Every binding in the document (pages, pagers, overlay; strips, solos,
    /// stage buttons, alerts and former MIDI targets), in document order.
    pub fn bindings(&self) -> Vec<&Binding> {
        let mut out = Vec::new();
        for page in &self.pages {
            page_bindings(page, &mut out);
        }
        for item in &self.overlay {
            item_bindings(item, &mut out);
        }
        out
    }

    /// The binding the hub's STAGE AUT rule drives: the first `stage` item
    /// with `aut` set, in document order.
    pub fn stage_aut_binding(&self) -> Option<&Binding> {
        fn in_items(items: &[Item]) -> Option<&Binding> {
            items.iter().find_map(|item| match &item.kind {
                ItemKind::Stage { binding, aut: true } => Some(binding),
                _ => None,
            })
        }
        fn in_page(page: &Page) -> Option<&Binding> {
            in_items(&page.items).or_else(|| {
                page.pager
                    .as_ref()
                    .and_then(|pager| pager.pages.iter().find_map(in_page))
            })
        }
        self.pages
            .iter()
            .find_map(in_page)
            .or_else(|| in_items(&self.overlay))
    }
}

fn page_bindings<'a>(page: &'a Page, out: &mut Vec<&'a Binding>) {
    for item in &page.items {
        item_bindings(item, out);
    }
    if let Some(pager) = &page.pager {
        for sub in &pager.pages {
            page_bindings(sub, out);
        }
    }
}

fn item_bindings<'a>(item: &'a Item, out: &mut Vec<&'a Binding>) {
    match &item.kind {
        ItemKind::Strip(strip) => out.push(&strip.binding),
        ItemKind::Solo { binding }
        | ItemKind::Stage { binding, .. }
        | ItemKind::Alert { binding, .. } => out.push(binding),
        ItemKind::ParamToggle { targets, .. } | ItemKind::ParamFader { targets, .. } => {
            out.extend(targets.iter().map(|t| &t.binding));
        }
        ItemKind::Area { .. }
        | ItemKind::HubToggle { .. }
        | ItemKind::Refresh { .. }
        | ItemKind::Label { .. } => {}
    }
}

struct Validator {
    canvas: Canvas,
    errors: Vec<LayoutError>,
    page_ids: HashSet<String>,
    item_ids: HashSet<String>,
}

impl Validator {
    fn error(&mut self, at: &str, message: String) {
        self.errors.push(LayoutError {
            at: at.to_string(),
            message,
        });
    }

    fn pages(&mut self, at: &str, pages: &[Page], tabbar: &TabBar) {
        if pages.is_empty() {
            self.error(at, "no pages".to_string());
        } else if tabbar.default_page >= pages.len() {
            self.error(
                at,
                format!(
                    "default page {} of {} pages",
                    tabbar.default_page,
                    pages.len()
                ),
            );
        }
        let bar = tabbar.bar_size.is_finite() && tabbar.bar_size >= 0.0;
        if !bar {
            self.error(at, format!("tab bar size {}", tabbar.bar_size));
        }
        for (i, page) in pages.iter().enumerate() {
            self.page(&format!("{at}[{i}]"), page);
        }
    }

    fn page(&mut self, at: &str, page: &Page) {
        if page.id.is_empty() {
            self.error(at, "empty page id".to_string());
        } else if !self.page_ids.insert(page.id.clone()) {
            self.error(at, format!("duplicate page id {:?}", page.id));
        }
        if let Some(color) = &page.tab.color {
            self.color(&format!("{at}.tab.color"), color);
        }
        for (i, item) in page.items.iter().enumerate() {
            self.item(&format!("{at}.items[{i}]"), item);
        }
        if let Some(pager) = &page.pager {
            let pager_at = format!("{at}.pager");
            self.frame(&pager_at, &pager.frame);
            self.pages(&format!("{pager_at}.pages"), &pager.pages, &pager.tabbar);
        }
    }

    fn color(&mut self, at: &str, color: &str) {
        if !is_color(color) {
            self.error(at, format!("{color:?} is not #RRGGBB or #RRGGBBAA"));
        }
    }

    fn frame(&mut self, at: &str, f: &Frame) {
        let finite = [f.x, f.y, f.w, f.h].iter().all(|n| n.is_finite());
        let inside = f.x >= -CANVAS_SLACK
            && f.y >= -CANVAS_SLACK
            && f.x + f.w <= self.canvas.w + CANVAS_SLACK
            && f.y + f.h <= self.canvas.h + CANVAS_SLACK;
        if !finite || f.w <= 0.0 || f.h <= 0.0 || !inside {
            self.error(
                at,
                format!(
                    "frame ({}, {}, {}×{}) is not inside the {}×{} canvas",
                    f.x, f.y, f.w, f.h, self.canvas.w, self.canvas.h
                ),
            );
        }
    }

    fn binding(&mut self, at: &str, b: &Binding) {
        if b.instance.is_empty() {
            self.error(at, "binding without an instance".to_string());
        }
        match &b.anchor {
            Anchor::Track { name } | Anchor::Return { name } if name.is_empty() => {
                self.error(at, "anchor without a name".to_string());
            }
            _ => {}
        }
        if let Err(e) = b.target() {
            self.error(at, format!("path: {e}"));
        }
    }

    fn item(&mut self, at: &str, item: &Item) {
        if let Some(id) = &item.id
            && !self.item_ids.insert(id.clone())
        {
            self.error(at, format!("duplicate item id {id:?}"));
        }
        self.frame(&format!("{at}.frame"), &item.frame);
        for (name, color) in [
            ("bg", &item.style.bg),
            ("color", &item.style.color),
            ("text_color", &item.style.text_color),
        ] {
            if let Some(color) = color {
                self.color(&format!("{at}.style.{name}"), color);
            }
        }
        match &item.kind {
            ItemKind::Strip(strip) => {
                self.binding(&format!("{at}.binding"), &strip.binding);
                for (name, frame) in strip.children.frames() {
                    self.frame(&format!("{at}.children.{name}"), &frame);
                }
            }
            ItemKind::Solo { binding } | ItemKind::Stage { binding, .. } => {
                self.binding(&format!("{at}.binding"), binding);
            }
            ItemKind::Alert { binding, period_ms } => {
                self.binding(&format!("{at}.binding"), binding);
                if *period_ms == 0 {
                    self.error(at, "alert period 0 ms".to_string());
                }
            }
            ItemKind::HubToggle { key, .. } => {
                if key != HUB_STAGE_AUT {
                    self.error(at, format!("unknown hub value {key:?}"));
                }
            }
            ItemKind::ParamToggle { targets, .. } => self.targets(at, targets, true),
            ItemKind::ParamFader { targets, .. } => self.targets(at, targets, false),
            ItemKind::Area { .. } | ItemKind::Refresh { .. } | ItemKind::Label { .. } => {}
        }
    }

    fn targets(&mut self, at: &str, targets: &[ParamTarget], toggle: bool) {
        if targets.is_empty() {
            self.error(at, "no targets".to_string());
        }
        for (i, t) in targets.iter().enumerate() {
            let t_at = format!("{at}.targets[{i}]");
            self.binding(&t_at, &t.binding);
            if t.prop.is_empty() {
                self.error(&t_at, "target without a prop".to_string());
            }
            if toggle && (t.on.is_none() || t.off.is_none()) {
                self.error(&t_at, "a toggle target needs on and off".to_string());
            }
            if !toggle && t.scale.is_none() {
                self.error(&t_at, "a fader target needs a scale".to_string());
            }
        }
    }
}

#[cfg(test)]
mod tests;
