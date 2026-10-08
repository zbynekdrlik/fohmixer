//! The layout document, `layout.json` schema 2 (the UI redesign, #21;
//! `docs/superpowers/specs/2026-09-28-ui-redesign-design.md` §2): pages with a
//! rail of function controls and rows of sections; a section is a group of
//! controls or a nested pager whose sub-pages hold groups. Nothing is placed
//! here: the UI computes the geometry. Controls bind by the general binding
//! form (`instance` + `anchor` + `path`, spec §2.5). The import tool writes
//! it, the hub validates and serves it, the UI renders it.
//!
//! An unknown field is an error, not ignored: the file is edited by hand
//! later (D4), and a mistyped field name must not pass silently.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::HUB_STAGE_AUT;
use crate::path::{PathError, escape_name, parse_steps, steps_text};

/// The schema version this build reads.
pub const LAYOUT_SCHEMA: u32 = 2;

/// The layout document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub schema: u32,
    /// The id of the page shown first.
    pub default_page: String,
    pub pages: Vec<Page>,
    /// Controls shown on every page, in the rail's footer (TechAlert).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global: Vec<Control>,
    #[serde(default)]
    pub config: LayoutConfig,
    /// What the import dropped, could not reproduce or had to guess
    /// (free-form).
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

/// A top-level page (a tab in the control column, #63): its rail and its
/// rows. A view (#68) is a page the markers made for a tag group the frame
/// does not show: a button in the column's POHĽADY, never a page tab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Page {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub view: bool,
    /// The function controls down the left side (stage mics, STAGE AUT,
    /// solos, the former MIDI toggles), top to bottom.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rail: Vec<Control>,
    /// The rows of sections, top to bottom.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<Row>,
}

/// A row of sections, left to right, and its share of the height.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Row {
    pub sections: Vec<Section>,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub weight: f64,
}

fn one() -> f64 {
    1.0
}

fn is_one(weight: &f64) -> bool {
    weight.to_bits() == 1.0f64.to_bits()
}

/// What a row holds: a group of controls, or a nested pager (at most one per
/// page) whose selected sub-page's groups show in its place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Section {
    Group(Group),
    Pager(Pager),
}

/// A titled (or untitled) group of controls, left to right. `color` is an
/// identity hint (the section's marker), not a fill.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// The tag group whose marker strips this group shows (#68, D16): the
    /// served layout fills `controls` with them (the frame leaves it empty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<String>,
    #[serde(default)]
    pub controls: Vec<Control>,
}

/// A nested pager: its sub-pages, the one shown first, and its tabs in the
/// control column (#63).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pager {
    pub id: String,
    pub default_page: String,
    pub pages: Vec<SubPage>,
}

/// A pager's sub-page: groups only (a pager holds no pager).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubPage {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub sections: Vec<Section>,
}

/// What a control is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Control {
    /// A mixer strip (boxed: by far the largest kind).
    Strip(Box<Strip>),
    /// A group-track solo toggle (spec F14).
    Solo {
        binding: Binding,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// The stage-mic button: the mute of the stage-mic track, lit while the
    /// mics are live (spec F15, #9). `aut`: the hub's STAGE AUT rule drives
    /// this binding's mute.
    Stage {
        binding: Binding,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        aut: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// A toggle of a hub value (the STAGE AUT button).
    HubToggle { key: String, label: String },
    /// A former MIDI toggle writing its targets directly (spec F17, F18);
    /// `color` is its lit colour.
    ParamToggle {
        label: String,
        targets: Vec<ParamTarget>,
        press: Press,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
    /// A former MIDI fader writing its targets directly (spec F18).
    ParamFader {
        label: String,
        targets: Vec<ParamTarget>,
    },
    /// TechAlert: the mute of its track, and the full-screen blink while it
    /// is unmuted (spec F16). `mute_guard`: a change needs a second tap, as
    /// a guarded strip's mute (spec F12).
    Alert {
        binding: Binding,
        period_ms: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        mute_guard: bool,
    },
    /// A static text (one line).
    Text { text: String },
}

/// A strip: its binding, its kind, and how it is drawn and guarded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Strip {
    pub binding: Binding,
    pub strip_kind: StripKind,
    /// A bus strip drawn wider (the top-right strips, returns).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub wide: bool,
    /// A mute change needs a second tap (spec F12).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mute_guard: bool,
    /// Never leaves the screen (#63): it keeps its place on every sub-page
    /// of its pager and stays shown when the column's arrows shift a line
    /// that does not fit. The owner chooses these strips.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pinned: bool,
    /// The name it shows (#68): a marker strip's quoted label, as written;
    /// none for a strip bound by name (its track's name's first word).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// A marker problem shown on the strip (#68, I9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<StripMark>,
}

/// How a marker strip is marked (#68): a conflict (its label is on another
/// marker too) disables it; any other problem leaves it working.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StripMark {
    Conflict,
    Problem,
}

/// The strip kinds: a track, or a return track (its name carries the
/// `X-` letter, which the label drops).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StripKind {
    Standard,
    Return,
}

impl Section {
    /// The groups this section shows: itself, or every group of every
    /// sub-page of the pager.
    pub fn groups(&self) -> Vec<&Group> {
        match self {
            Section::Group(group) => vec![group],
            Section::Pager(pager) => pager
                .pages
                .iter()
                .flat_map(|sub| sub.sections.iter().flat_map(Section::groups))
                .collect(),
        }
    }
}

impl Page {
    /// Every control of the page in document order: the rail, then the rows
    /// (a pager's sub-pages in order).
    pub fn controls(&self) -> Vec<&Control> {
        let mut out: Vec<&Control> = self.rail.iter().collect();
        for row in &self.rows {
            for section in &row.sections {
                for group in section.groups() {
                    out.extend(group.controls.iter());
                }
            }
        }
        out
    }

    /// The page's nested pager, if it has one.
    pub fn pager(&self) -> Option<&Pager> {
        self.rows
            .iter()
            .flat_map(|r| &r.sections)
            .find_map(|s| match s {
                Section::Pager(pager) => Some(pager),
                Section::Group(_) => None,
            })
    }
}

impl Control {
    /// Whether the control never leaves the screen (#63): a pinned strip.
    pub fn pinned(&self) -> bool {
        matches!(self, Control::Strip(strip) if strip.pinned)
    }

    /// The bindings this control reads or writes.
    pub fn bindings(&self) -> Vec<&Binding> {
        match self {
            Control::Strip(strip) => vec![&strip.binding],
            Control::Solo { binding, .. }
            | Control::Stage { binding, .. }
            | Control::Alert { binding, .. } => vec![binding],
            Control::ParamToggle { targets, .. } | Control::ParamFader { targets, .. } => {
                targets.iter().map(|t| &t.binding).collect()
            }
            Control::HubToggle { .. } | Control::Text { .. } => vec![],
        }
    }
}

/// Layout-wide settings (the TouchOSC `Conf` text and UI switches).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutConfig {
    /// Fader touch shaping and the post-release delay (spec X3); the UI's
    /// default when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fader_shaping: Option<bool>,
    /// The Live properties the strip meters show (spec X2): switchable until
    /// the K2 measurement decides (#5); `level` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meter_source: Option<MeterSource>,
    /// How a strip's volume fader maps its position to Live's volume (#63):
    /// `live` (Live's own law) when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fader_law: Option<FaderLaw>,
}

/// The volume law of the strip faders (#63).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FaderLaw {
    /// Live's own: the fader's position is Live's volume value.
    #[default]
    Live,
    /// TouchOSC's: `v = p^0.515`.
    Touchosc,
}

/// The meter source of the strips (spec X2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeterSource {
    /// `output_meter_level`: one bar (TouchOSC parity).
    #[default]
    Level,
    /// `output_meter_left` and `output_meter_right`: two bars.
    Lr,
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
    /// A track by its index in `live_set tracks` (a marker strip, #68).
    TrackAt {
        index: u32,
    },
    /// A return track by its index in `live_set return_tracks` (#68).
    ReturnAt {
        index: u32,
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
            Anchor::TrackAt { index } => format!("live_set tracks {index}"),
            Anchor::ReturnAt { index } => format!("live_set return_tracks {index}"),
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
            errors: Vec::new(),
            ids: HashSet::new(),
        };
        if self.schema != LAYOUT_SCHEMA {
            v.error(
                "schema",
                format!("schema {} is not {LAYOUT_SCHEMA}", self.schema),
            );
        }
        if self.pages.is_empty() {
            v.error("pages", "no pages".to_string());
        } else if !self
            .pages
            .iter()
            .any(|p| p.id == self.default_page && !p.view)
        {
            v.error(
                "default_page",
                format!("{:?} is not a page", self.default_page),
            );
        }
        for (i, page) in self.pages.iter().enumerate() {
            v.page(&format!("pages[{i}]"), page);
        }
        for (i, control) in self.global.iter().enumerate() {
            v.control(&format!("global[{i}]"), control);
        }
        v.errors
    }

    /// Every binding in the document (pages: rail, then rows with every
    /// sub-page; then the global controls), in document order.
    pub fn bindings(&self) -> Vec<&Binding> {
        self.controls()
            .into_iter()
            .flat_map(Control::bindings)
            .collect()
    }

    /// Every control in document order: each page's rail and rows, then the
    /// global controls.
    pub fn controls(&self) -> Vec<&Control> {
        let mut out: Vec<&Control> = self.pages.iter().flat_map(Page::controls).collect();
        out.extend(self.global.iter());
        out
    }

    /// The binding the hub's STAGE AUT rule drives: the first `stage`
    /// control with `aut` set, in document order.
    pub fn stage_aut_binding(&self) -> Option<&Binding> {
        self.controls().into_iter().find_map(|c| match c {
            Control::Stage {
                binding, aut: true, ..
            } => Some(binding),
            _ => None,
        })
    }

    /// The tracks the strips show, as `(instance, LOM target)`, each once
    /// and sorted (#58: the hub keeps the groups they sit in unfolded, since
    /// Live meters no track inside a folded group; #68: a marker strip's
    /// track by its index). Return tracks sit in no group.
    pub fn strip_tracks(&self) -> Vec<(String, String)> {
        let tracks: std::collections::BTreeSet<(String, String)> = self
            .controls()
            .into_iter()
            .filter_map(|c| match c {
                Control::Strip(strip) => match &strip.binding.anchor {
                    Anchor::Track { .. } | Anchor::TrackAt { .. } => {
                        let target = Binding {
                            instance: strip.binding.instance.clone(),
                            anchor: strip.binding.anchor.clone(),
                            path: None,
                        }
                        .target()
                        .ok()?;
                        Some((strip.binding.instance.clone(), target))
                    }
                    _ => None,
                },
                _ => None,
            })
            .collect();
        tracks.into_iter().collect()
    }
}

struct Validator {
    errors: Vec<LayoutError>,
    /// Page, sub-page, group and pager ids: one namespace.
    ids: HashSet<String>,
}

impl Validator {
    fn error(&mut self, at: &str, message: String) {
        self.errors.push(LayoutError {
            at: at.to_string(),
            message,
        });
    }

    fn id(&mut self, at: &str, id: &str) {
        if id.is_empty() {
            self.error(at, "empty id".to_string());
        } else if !self.ids.insert(id.to_string()) {
            self.error(at, format!("duplicate id {id:?}"));
        }
    }

    fn page(&mut self, at: &str, page: &Page) {
        self.id(at, &page.id);
        for (i, control) in page.rail.iter().enumerate() {
            self.control(&format!("{at}.rail[{i}]"), control);
        }
        let mut pagers = 0;
        for (r, row) in page.rows.iter().enumerate() {
            let row_at = format!("{at}.rows[{r}]");
            let weight = row.weight.is_finite() && row.weight > 0.0;
            if !weight {
                self.error(&row_at, format!("weight {} is not above 0", row.weight));
            }
            for (s, section) in row.sections.iter().enumerate() {
                let section_at = format!("{row_at}.sections[{s}]");
                match section {
                    Section::Group(group) => self.group(&section_at, group),
                    Section::Pager(pager) => {
                        pagers += 1;
                        if pagers > 1 {
                            self.error(&section_at, "a second pager on the page".to_string());
                        }
                        self.pager(&section_at, pager);
                    }
                }
            }
        }
    }

    fn pager(&mut self, at: &str, pager: &Pager) {
        self.id(at, &pager.id);
        if pager.pages.is_empty() {
            self.error(at, "no pages".to_string());
        } else if !pager.pages.iter().any(|p| p.id == pager.default_page) {
            self.error(
                at,
                format!("default page {:?} is not a page", pager.default_page),
            );
        }
        for (i, sub) in pager.pages.iter().enumerate() {
            let sub_at = format!("{at}.pages[{i}]");
            self.id(&sub_at, &sub.id);
            for (s, section) in sub.sections.iter().enumerate() {
                let section_at = format!("{sub_at}.sections[{s}]");
                match section {
                    Section::Group(group) => self.group(&section_at, group),
                    Section::Pager(_) => {
                        self.error(&section_at, "a pager inside a pager".to_string());
                    }
                }
            }
        }
    }

    fn group(&mut self, at: &str, group: &Group) {
        if let Some(id) = &group.id {
            self.id(at, id);
        }
        if let Some(color) = &group.color {
            self.color(&format!("{at}.color"), color);
        }
        if let Some(tags) = &group.tags
            && !crate::markers::valid_group_name(tags)
        {
            self.error(
                &format!("{at}.tags"),
                format!("{tags:?} is not a tag group name"),
            );
        }
        for (i, control) in group.controls.iter().enumerate() {
            self.control(&format!("{at}.controls[{i}]"), control);
        }
    }

    fn color(&mut self, at: &str, color: &str) {
        if !is_color(color) {
            self.error(at, format!("{color:?} is not #RRGGBB or #RRGGBBAA"));
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

    fn control(&mut self, at: &str, control: &Control) {
        match control {
            Control::Strip(strip) => {
                self.binding(&format!("{at}.binding"), &strip.binding);
                if strip.label.as_deref().is_some_and(|l| l.trim().is_empty()) {
                    self.error(&format!("{at}.label"), "an empty label".to_string());
                }
            }
            Control::Solo { binding, .. } | Control::Stage { binding, .. } => {
                self.binding(&format!("{at}.binding"), binding);
            }
            Control::Alert {
                binding, period_ms, ..
            } => {
                self.binding(&format!("{at}.binding"), binding);
                if *period_ms == 0 {
                    self.error(at, "alert period 0 ms".to_string());
                }
            }
            Control::HubToggle { key, .. } => {
                if key != HUB_STAGE_AUT {
                    self.error(at, format!("unknown hub value {key:?}"));
                }
            }
            Control::ParamToggle { targets, color, .. } => {
                if let Some(color) = color {
                    self.color(&format!("{at}.color"), color);
                }
                self.targets(at, targets, true);
            }
            Control::ParamFader { targets, .. } => self.targets(at, targets, false),
            Control::Text { .. } => {}
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
