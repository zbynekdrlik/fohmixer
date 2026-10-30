//! Tuner markers (#68, spec D16): what is on the surface comes from a Tuner
//! in each track's device chain, renamed with the strip's label and its
//! tags, AbleSet style: `"Vox 1" +G:VOCALS:2 +G:TALKSHOW:1 +PIN +MG`.
//!
//! - [`parse`] reads one Tuner's name: the quoted label, the groups with
//!   their places, `+PIN`, `+MG`, the linked mute's `+ID:<id>` and
//!   `+MUTE:<id>` (#127), the fader link's `+FADER:<id>` and the REVERB
//!   rule's `+REVERB` (#130), the AUTOTUNE rule's `+AUTOTUNE` (#104), a
//!   frame control's `+CTRL:<id>` (#104), and every problem as a value
//!   (never an error: a problem is shown on the strip, I9).
//! - [`compose`] turns the frame (the layout file: pages, rows, the rail)
//!   and the markers the hub found into the served layout: a frame group
//!   with `tags` shows that tag group's strips, every other tag group
//!   becomes a view (a page flagged `view`), equal labels are conflicts,
//!   a strip whose `+ID` other Tuners of its Live set name with `+MUTE`
//!   mutes their tracks with its own, a strip whose `+ID` other Tuners
//!   name with `+FADER` leads their faders, the frame's linked fader
//!   takes the `+FADER` tracks of its link, every `autotune` control
//!   takes the `+AUTOTUNE` tracks (#104), and a control naming a `+CTRL` id
//!   takes that Tuner's track by index (#104).
//!
//! Pure, shared by the hub, `layout check` and the tests.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::layout::{
    Anchor, AutotuneTarget, Binding, Control, CtrlId, FaderFollow, FaderLink, Group, Layout,
    LayoutError, LinkId, LinkPartner, LufsSpec, Page, ParamTarget, Row, Scale, Section, Strip,
    StripKind, StripMark, lufs_target_ok,
};

/// The `class_name` of Live's Tuner (only a Tuner can be a marker).
pub const TUNER_CLASS: &str = "Tuner";
/// The longest group name a `+G:` tag takes.
pub const MAX_GROUP_NAME: usize = 32;
/// The largest place a `+G:NAME:N` tag takes.
pub const MAX_PLACE: u32 = 999;
/// A view page's id: this prefix and its group's name.
pub const VIEW_PREFIX: &str = "view-";

/// Where a `+AUTOTUNE` track's autotune is on (#104): `+AUTOTUNE` on 127
/// (off 0, the stream racks), `+AUTOTUNE:0` on 0 (off 127, the repro racks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutotuneOn {
    Max,
    Zero,
}

impl AutotuneOn {
    /// The value of the rack's first macro that turns autotune on.
    pub fn on(self) -> u8 {
        match self {
            AutotuneOn::Max => 127,
            AutotuneOn::Zero => 0,
        }
    }

    /// The value that turns it off.
    pub fn off(self) -> u8 {
        match self {
            AutotuneOn::Max => 0,
            AutotuneOn::Zero => 127,
        }
    }
}

/// A group a marker puts its strip in, and its place there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupTag {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<u32>,
}

/// What one Tuner's name says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marker {
    /// The quoted label; none when it is missing or empty (a problem).
    pub label: Option<String>,
    /// The `+G:` tags, in their order, each name once.
    pub groups: Vec<GroupTag>,
    /// `+PIN`: always on screen (D15).
    pub pin: bool,
    /// `+MG`: the mute guard (F12).
    pub mute_guard: bool,
    /// `+ID:<id>`: the strip's id, which other Tuners' `+MUTE` name (#127).
    pub id: Option<String>,
    /// `+MUTE:<id>`: the strips whose mute writes this track's mute too
    /// (#127), each id once.
    pub mutes: Vec<String>,
    /// `+FADER:<id>` (#130, D18): the leaders whose fader moves this
    /// track's volume by the same dB while their link is on, each id once.
    pub faders: Vec<String>,
    /// `+REVERB` (#130, D18): the strip is one of the REVERB rule's faders.
    pub reverb: bool,
    /// `+AUTOTUNE` / `+AUTOTUNE:0` (#104, D19): the track is one of the
    /// AUTOTUNE rule's, and where its autotune is on.
    pub autotune: Option<AutotuneOn>,
    /// `+LUFS` / `+LUFS:<target>` (#151): the strip is a loudness point.
    pub lufs: Option<LufsSpec>,
    /// `+CTRL:<id>` (#104): the frame controls this track binds, each id
    /// once.
    pub ctrls: Vec<CtrlId>,
    pub problems: Vec<TagProblem>,
}

impl Marker {
    /// Whether the Tuner makes no strip (#127, #130, #104): `+MUTE`,
    /// `+FADER`, `+AUTOTUNE` or `+CTRL` tags with no quote, no group and no
    /// other tag (its first problem is [`TagProblem::NoLabel`] while
    /// [`parse`] reads it). A missing label or group is no problem there.
    fn makes_no_strip(&self) -> bool {
        !(self.mutes.is_empty()
            && self.faders.is_empty()
            && self.autotune.is_none()
            && self.ctrls.is_empty())
            && self.groups.is_empty()
            && self.id.is_none()
            && !self.pin
            && !self.mute_guard
            && !self.reverb
            && self.lufs.is_none()
            && self.problems.first() == Some(&TagProblem::NoLabel)
    }
}

/// A problem of a marker or of its place among the others.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum TagProblem {
    /// The name does not start with a quoted label.
    NoLabel,
    /// The label is `""`.
    EmptyLabel,
    /// The label's closing quote is missing.
    UnclosedLabel,
    /// A `+` tag fohmixer does not know (AbleSet's are uppercase too).
    UnknownTag { tag: String },
    /// A `+G:` tag whose name or place is not valid.
    BadGroup { tag: String },
    /// The same group named twice (the first counts).
    DuplicateGroup { name: String },
    /// Text outside the label that is no tag.
    StrayText { text: String },
    /// No `+G:` tag: the strip shows on no page.
    NoGroup,
    /// Another marker has the same label: both strips are disabled.
    Conflict { label: String },
    /// The track holds more than one marker (the first counts).
    DoubleTuner,
    /// Another strip of the group has the same place.
    SamePlace { group: String, place: u32 },
    /// A `+ID:`, `+MUTE:` or `+FADER:` tag whose id is not valid (#127,
    /// #130; the rule of a group name).
    BadId { tag: String },
    /// A second `+ID:` tag (the first counts).
    SecondId { tag: String },
    /// The same `+MUTE:` id twice (it counts once).
    DuplicateMute { id: String },
    /// Another marker of the Live set carries the same `+ID`: neither
    /// strip's mute links (#127).
    DuplicateId { id: String },
    /// A `+MUTE:` whose id no marker of the Live set carries (#127).
    OrphanMute { id: String },
    /// A `+MUTE:` naming its own strip's `+ID` (#127).
    SelfMute { id: String },
    /// The same `+FADER:` id twice (it counts once; #130).
    DuplicateFader { id: String },
    /// A `+FADER:` whose id no strip of the Live set carries, nor the
    /// frame's linked fader (#130).
    OrphanFader { id: String },
    /// A `+FADER:` naming its own strip's `+ID` (#130).
    SelfFader { id: String },
    /// A `+AUTOTUNE:` tag whose argument is not `0` (#104).
    BadAutotune { tag: String },
    /// A second `+AUTOTUNE` tag (the first counts; #104).
    SecondAutotune { tag: String },
    /// A `+AUTOTUNE` Tuner with no `+ID` and two or more `+MUTE` ids
    /// (#140 PR 3b, spec §7): which singer it is is not known, so the track
    /// gets no autotune.
    TwoSingers,
    /// A second `+LUFS` tag (the first counts; #151).
    DuplicateLufs { tag: String },
    /// A `+LUFS:` tag whose target is not an integer in
    /// [`crate::layout::LUFS_TARGET_MIN`]..=[`crate::layout::LUFS_TARGET_MAX`] (#151).
    BadLufs { tag: String },
    /// A `+CTRL:` tag whose id is none of [`CtrlId`] (#104).
    BadCtrl { tag: String },
    /// The same `+CTRL:` id twice on a Tuner (it counts once; #104).
    RepeatedCtrl { id: CtrlId },
    /// Another Tuner of the Live set carries the same `+CTRL:` id: which
    /// track is meant is not known, so neither binds (#104).
    DuplicateCtrl { id: CtrlId },
}

impl TagProblem {
    /// The words of a LUFS tag's problem (#151, spec §6.5): `bad LUFS tag`
    /// for both; none for the other problems.
    pub fn lufs_text(&self) -> Option<&'static str> {
        match self {
            TagProblem::DuplicateLufs { .. } | TagProblem::BadLufs { .. } => Some("bad LUFS tag"),
            _ => None,
        }
    }
}

/// Whether a Tuner's name makes it a marker: a quote or a `+` token.
pub fn is_marker(name: &str) -> bool {
    name.contains('"') || name.split_whitespace().any(|t| t.starts_with('+'))
}

/// Whether `name` is a valid group name: `A-Z`, `0-9`, `-`, `_`, 1 to
/// [`MAX_GROUP_NAME`] characters.
pub fn valid_group_name(name: &str) -> bool {
    (1..=MAX_GROUP_NAME).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// A group's title as shown: its name with `_` as a space (AbleSet's rule).
pub fn group_title(name: &str) -> String {
    name.replace('_', " ")
}

/// A `+G:` tag's argument: `NAME` or `NAME:N`.
fn group_tag(arg: &str) -> Option<GroupTag> {
    let mut parts = arg.split(':');
    let name = parts.next()?;
    let place = match parts.next() {
        None => None,
        Some(text) => {
            let place: u32 = text.parse().ok()?;
            if !(1..=MAX_PLACE).contains(&place) {
                return None;
            }
            Some(place)
        }
    };
    if parts.next().is_some() || !valid_group_name(name) {
        return None;
    }
    Some(GroupTag {
        name: name.to_string(),
        place,
    })
}

/// Reads one Tuner's name.
pub fn parse(name: &str) -> Marker {
    let mut marker = Marker::default();
    let text = name.trim();
    let rest = match text.strip_prefix('"') {
        Some(after) => match after.split_once('"') {
            Some((label, rest)) => {
                let label = label.trim();
                if label.is_empty() {
                    marker.problems.push(TagProblem::EmptyLabel);
                } else {
                    marker.label = Some(label.to_string());
                }
                rest
            }
            None => {
                marker.problems.push(TagProblem::UnclosedLabel);
                after
            }
        },
        None => {
            marker.problems.push(TagProblem::NoLabel);
            text
        }
    };
    for token in rest.split_whitespace() {
        let Some(tag) = token.strip_prefix('+') else {
            marker.problems.push(TagProblem::StrayText {
                text: token.to_string(),
            });
            continue;
        };
        match tag {
            "PIN" => marker.pin = true,
            "MG" => marker.mute_guard = true,
            "REVERB" => marker.reverb = true,
            "AUTOTUNE" => autotune_tag(&mut marker, token, Some(AutotuneOn::Max)),
            "LUFS" => lufs_tag(&mut marker, token, Some(LufsSpec { target: None })),
            _ => {
                if let Some(arg) = tag.strip_prefix("G:") {
                    match group_tag(arg) {
                        Some(group) if marker.groups.iter().any(|g| g.name == group.name) => {
                            marker
                                .problems
                                .push(TagProblem::DuplicateGroup { name: group.name });
                        }
                        Some(group) => marker.groups.push(group),
                        None => marker.problems.push(TagProblem::BadGroup {
                            tag: token.to_string(),
                        }),
                    }
                } else if let Some(arg) = tag.strip_prefix("ID:") {
                    id_tag(&mut marker, token, arg);
                } else if let Some(arg) = tag.strip_prefix("MUTE:") {
                    link_tag(&mut marker.mutes, &mut marker.problems, token, arg, |id| {
                        TagProblem::DuplicateMute { id }
                    });
                } else if let Some(arg) = tag.strip_prefix("FADER:") {
                    link_tag(&mut marker.faders, &mut marker.problems, token, arg, |id| {
                        TagProblem::DuplicateFader { id }
                    });
                } else if let Some(arg) = tag.strip_prefix("AUTOTUNE:") {
                    autotune_tag(&mut marker, token, autotune_arg(arg));
                } else if let Some(arg) = tag.strip_prefix("LUFS:") {
                    lufs_tag(&mut marker, token, lufs_arg(arg));
                } else if let Some(arg) = tag.strip_prefix("CTRL:") {
                    ctrl_tag(&mut marker, token, arg);
                } else {
                    marker.problems.push(TagProblem::UnknownTag {
                        tag: token.to_string(),
                    });
                }
            }
        }
    }
    if marker.makes_no_strip() {
        // Its NoLabel: no strip, no label to miss.
        marker.problems.remove(0);
    } else if marker.groups.is_empty() {
        marker.problems.push(TagProblem::NoGroup);
    }
    if names_two_singers(&marker) {
        marker.problems.push(TagProblem::TwoSingers);
    }
    marker
}

/// Whether a marker's `+AUTOTUNE` track names two singers or more (#140 PR
/// 3b): no `+ID` of its own and two or more `+MUTE` ids.
fn names_two_singers(marker: &Marker) -> bool {
    marker.autotune.is_some() && marker.id.is_none() && marker.mutes.len() > 1
}

/// The singer of a marker's `+AUTOTUNE` track (#140 PR 3b, spec §7): its
/// own `+ID`, else its one `+MUTE` id; none for two or more (a problem) or
/// none (a singer of its own).
fn singer(marker: &Marker) -> Option<String> {
    match (&marker.id, marker.mutes.as_slice()) {
        (Some(id), _) => Some(id.clone()),
        (None, [one]) => Some(one.clone()),
        (None, _) => None,
    }
}

/// A `+ID:<id>` tag (`token`, its argument `arg`): the strip's id, once.
fn id_tag(marker: &mut Marker, token: &str, arg: &str) {
    if !valid_group_name(arg) {
        marker.problems.push(TagProblem::BadId {
            tag: token.to_string(),
        });
    } else if marker.id.is_some() {
        marker.problems.push(TagProblem::SecondId {
            tag: token.to_string(),
        });
    } else {
        marker.id = Some(arg.to_string());
    }
}

/// A `+AUTOTUNE:<arg>` tag's argument (#104): `0` only.
fn autotune_arg(arg: &str) -> Option<AutotuneOn> {
    (arg == "0").then_some(AutotuneOn::Zero)
}

/// A `+AUTOTUNE` tag (`token`; `on` none: a bad argument), once (#104).
fn autotune_tag(marker: &mut Marker, token: &str, on: Option<AutotuneOn>) {
    match on {
        None => marker.problems.push(TagProblem::BadAutotune {
            tag: token.to_string(),
        }),
        Some(_) if marker.autotune.is_some() => marker.problems.push(TagProblem::SecondAutotune {
            tag: token.to_string(),
        }),
        Some(on) => marker.autotune = Some(on),
    }
}

/// A `+LUFS:<target>` tag's argument (#151): an integer in
/// [`crate::layout::LUFS_TARGET_MIN`]..=[`crate::layout::LUFS_TARGET_MAX`].
fn lufs_arg(arg: &str) -> Option<LufsSpec> {
    arg.parse::<i8>()
        .ok()
        .filter(|target| lufs_target_ok(*target))
        .map(|target| LufsSpec {
            target: Some(target),
        })
}

/// A `+LUFS` tag (`token`; `spec` none: a bad target), once (#151).
fn lufs_tag(marker: &mut Marker, token: &str, spec: Option<LufsSpec>) {
    match spec {
        None => marker.problems.push(TagProblem::BadLufs {
            tag: token.to_string(),
        }),
        Some(_) if marker.lufs.is_some() => marker.problems.push(TagProblem::DuplicateLufs {
            tag: token.to_string(),
        }),
        Some(spec) => marker.lufs = Some(spec),
    }
}

/// A `+CTRL:<id>` tag (`token`, its argument `arg`): one of [`CtrlId`],
/// each once (#104).
fn ctrl_tag(marker: &mut Marker, token: &str, arg: &str) {
    match CtrlId::parse(arg) {
        None => marker.problems.push(TagProblem::BadCtrl {
            tag: token.to_string(),
        }),
        Some(id) if marker.ctrls.contains(&id) => {
            marker.problems.push(TagProblem::RepeatedCtrl { id });
        }
        Some(id) => marker.ctrls.push(id),
    }
}

/// A `+MUTE:<id>` or `+FADER:<id>` tag (`token`, its argument `arg`): an id
/// into `ids`, each once (`twice` names a repeat's problem; #127, #130).
fn link_tag(
    ids: &mut Vec<String>,
    problems: &mut Vec<TagProblem>,
    token: &str,
    arg: &str,
    twice: fn(String) -> TagProblem,
) {
    if !valid_group_name(arg) {
        problems.push(TagProblem::BadId {
            tag: token.to_string(),
        });
    } else if ids.iter().any(|id| id == arg) {
        problems.push(twice(arg.to_string()));
    } else {
        ids.push(arg.to_string());
    }
}

/// What a frame (the layout file) may not hold (#68): a view page or an
/// id starting with [`VIEW_PREFIX`] (the markers make those), a strip in a
/// `tags` group (its strips are its markers'; other controls follow them),
/// a strip with `mute_links` (#127 review M4: a linked mute is the Tuner
/// markers' `+ID` and `+MUTE`, never the frame's) or with `reverb` (#130:
/// the markers' `+REVERB`), a strip with a fader link or a follow, a
/// linked fader with its own targets, or one link on two faders (#130), an
/// autotune control naming tracks (#104: the markers' `+AUTOTUNE`), or a
/// `ctrl` anchor on a control that takes none (#104: [`names_ctrl_wrongly`]).
pub fn frame_problems(frame: &Layout) -> Vec<LayoutError> {
    let mut out = Vec::new();
    let mut id = |at: &str, id: &str| {
        if id.starts_with(VIEW_PREFIX) {
            out.push(LayoutError {
                at: at.to_string(),
                message: format!("{id:?} starts with {VIEW_PREFIX:?}, the Tuner markers' views"),
            });
        }
    };
    for (i, page) in frame.pages.iter().enumerate() {
        let at = format!("pages[{i}]");
        id(&at, page.id.as_str());
        for (r, row) in page.rows.iter().enumerate() {
            for (s, section) in row.sections.iter().enumerate() {
                let section_at = format!("{at}.rows[{r}].sections[{s}]");
                if let Section::Pager(pager) = section {
                    id(&section_at, pager.id.as_str());
                    for sub in &pager.pages {
                        id(&section_at, sub.id.as_str());
                    }
                }
                for group in section.groups() {
                    if let Some(group_id) = &group.id {
                        id(&section_at, group_id.as_str());
                    }
                }
            }
        }
    }
    for (i, page) in frame.pages.iter().enumerate() {
        let at = format!("pages[{i}]");
        if page.view {
            out.push(LayoutError {
                at: at.clone(),
                message: "a view page: the Tuner markers make those".to_string(),
            });
        }
        let groups = page
            .rows
            .iter()
            .flat_map(|r| &r.sections)
            .flat_map(Section::groups);
        // A tags group may keep other controls (shown after its marker
        // strips, PR C: a group ending with a parameter fader), never a strip.
        for group in groups {
            if group.tags.is_some()
                && group
                    .controls
                    .iter()
                    .any(|c| matches!(c, Control::Strip(_)))
            {
                out.push(LayoutError {
                    at: at.clone(),
                    message: format!(
                        "the tags group {:?} holds a strip of its own (the markers' strips fill it)",
                        group.id.as_deref().unwrap_or("")
                    ),
                });
            }
        }
    }
    refuse(
        frame,
        &mut out,
        |c| matches!(c, Control::Strip(s) if !s.mute_links.is_empty()),
        "a linked mute: the Tuner markers make those",
    );
    refuse(
        frame,
        &mut out,
        |c| matches!(c, Control::Strip(s) if s.reverb),
        "a reverb fader: the Tuner markers make those",
    );
    refuse(
        frame,
        &mut out,
        |c| matches!(c, Control::Strip(s) if s.fader_link.is_some() || !s.fader_follows.is_empty()),
        "a fader link: the Tuner markers make those",
    );
    refuse(
        frame,
        &mut out,
        |c| matches!(c, Control::ParamFader { link: Some(_), targets, .. } if !targets.is_empty()),
        "a linked fader's tracks: the Tuner markers make those",
    );
    refuse(
        frame,
        &mut out,
        |c| matches!(c, Control::Autotune { targets, .. } if !targets.is_empty()),
        "the autotune tracks: the Tuner markers make those",
    );
    refuse(
        frame,
        &mut out,
        names_ctrl_wrongly,
        "a ctrl anchor: only stage, alert and param_toggle take one",
    );
    // One link on two faders: named at the second, with its label (#130
    // review M4).
    let places = frame
        .pages
        .iter()
        .enumerate()
        .flat_map(|(i, page)| {
            page.controls()
                .into_iter()
                .map(move |c| (format!("pages[{i}]"), c))
        })
        .chain(
            frame
                .global
                .iter()
                .enumerate()
                .map(|(i, c)| (format!("global[{i}]"), c)),
        );
    let mut named: Vec<&LinkId> = Vec::new();
    for (at, control) in places {
        if let Control::ParamFader {
            label,
            link: Some(link),
            ..
        } = control
        {
            if named.contains(&link) {
                out.push(LayoutError {
                    at,
                    message: format!(
                        "the fader link {}/{} is named twice: again by {label:?}",
                        link.instance, link.id
                    ),
                });
            } else {
                named.push(link);
            }
        }
    }
    out
}

/// Whether a control other than `stage`, `alert` and `param_toggle` names a
/// `+CTRL` id (#104): those three are all a Tuner's tag binds.
fn names_ctrl_wrongly(control: &Control) -> bool {
    !matches!(
        control,
        Control::Stage { .. } | Control::Alert { .. } | Control::ParamToggle { .. }
    ) && control
        .bindings()
        .iter()
        .any(|b| matches!(b.anchor, Anchor::Ctrl { .. }))
}

/// Refuses each page whose controls, and each global control, `bad` finds,
/// with `message` (#127, #130: what only the Tuner markers make).
fn refuse(
    frame: &Layout,
    out: &mut Vec<LayoutError>,
    bad: impl Fn(&Control) -> bool,
    message: &str,
) {
    for (i, page) in frame.pages.iter().enumerate() {
        if page.controls().into_iter().any(&bad) {
            out.push(LayoutError {
                at: format!("pages[{i}]"),
                message: message.to_string(),
            });
        }
    }
    for (i, control) in frame.global.iter().enumerate() {
        if bad(control) {
            out.push(LayoutError {
                at: format!("global[{i}]"),
                message: message.to_string(),
            });
        }
    }
}

/// Whether a marker's track is a track or a return track.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Track,
    Return,
}

/// A marker the hub found: where, and its Tuner's name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Found {
    pub instance: String,
    pub kind: TrackKind,
    /// The track's index in `live_set tracks` (or `return_tracks`).
    pub index: u32,
    /// The marker Tuner's name (the track's first marker).
    pub name: String,
    /// How many marker Tuners the track holds (more than one: a problem).
    pub tuners: u32,
}

impl Found {
    /// Where it is, in the order strips without a place are shown.
    fn key(&self) -> (&str, TrackKind, u32) {
        (self.instance.as_str(), self.kind, self.index)
    }
}

/// A marker's problems, for `/api/status` (I9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkerReport {
    pub instance: String,
    pub kind: TrackKind,
    pub index: u32,
    /// The Tuner's name as Live shows it.
    pub name: String,
    pub problems: Vec<TagProblem>,
}

/// The served layout, every marker problem and every fader link made.
#[derive(Debug, Clone, PartialEq)]
pub struct Composed {
    pub layout: Layout,
    pub problems: Vec<MarkerReport>,
    pub links: Vec<LinkReport>,
    pub ctrls: Vec<CtrlReport>,
}

/// Which leader a link has (#130).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Leader {
    /// A strip's `+ID`.
    Strip,
    /// The frame's linked fader (Podklady All).
    Frame,
}

/// A fader link the composition made (#130, `/api/status`
/// `layout.markers.links`): its Live set, its id, its leader and how many
/// partners it has (a frame's linked fader may have none).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LinkReport {
    pub instance: String,
    pub id: String,
    pub leader: Leader,
    pub partners: usize,
}

/// A `+CTRL` id the frame names (#104, `/api/status`
/// `layout.markers.ctrls`): its Live set and how many Tuners carry it. It
/// binds only with exactly one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CtrlReport {
    pub instance: String,
    pub id: CtrlId,
    pub tuners: usize,
}

/// The label a marker strip shows when its Tuner names none (a missing,
/// empty or unclosed label): its track's number, `#<index + 1>`, as Live
/// numbers tracks. It is no identity: after a track-list change the same
/// label names another track.
pub fn placeholder_label(index: u32) -> String {
    format!("#{}", index + 1)
}

/// Whether `strip` is a marker strip that shows [`placeholder_label`]
/// (#71: a channel detail cannot follow it through a new layout). The one
/// test of it, beside where the placeholder is made.
pub fn has_placeholder_label(strip: &Strip) -> bool {
    let index = match strip.binding.anchor {
        Anchor::TrackAt { index } | Anchor::ReturnAt { index } => index,
        _ => return false,
    };
    strip.label.as_deref() == Some(placeholder_label(index).as_str())
}

/// One marker read: where, what it says, every problem it has, the
/// tracks its strip's mute writes with its own (#127), the fader link it
/// leads and the leaders it follows (#130).
struct Entry<'a> {
    found: &'a Found,
    marker: Marker,
    problems: Vec<TagProblem>,
    links: Vec<Binding>,
    leads: Option<FaderLink>,
    follows: Vec<FaderFollow>,
}

impl Entry<'_> {
    /// Its track, by index (D16 binds everything so).
    fn track(&self) -> Binding {
        let anchor = match self.found.kind {
            TrackKind::Track => Anchor::TrackAt {
                index: self.found.index,
            },
            TrackKind::Return => Anchor::ReturnAt {
                index: self.found.index,
            },
        };
        Binding {
            instance: self.found.instance.clone(),
            anchor,
            path: None,
        }
    }

    /// The label its strip shows, and its partners' leaders read (#130).
    fn label(&self) -> String {
        self.marker
            .label
            .clone()
            .unwrap_or_else(|| placeholder_label(self.found.index))
    }

    /// Whether its strip is placed: in a tag group, so in the served layout
    /// (a marker without `+G:` shows nowhere, `NoGroup`).
    fn placed(&self) -> bool {
        !self.marker.groups.is_empty()
    }

    /// Whether its strip shows a conflict (an equal label elsewhere).
    fn conflict(&self) -> bool {
        self.problems
            .iter()
            .any(|p| matches!(p, TagProblem::Conflict { .. }))
    }

    /// The strip it shows: bound to its track by index, its label (or the
    /// track's number when the label is missing), its mark, its linked
    /// mute, whether it is a reverb fader (#130: only with a label of its
    /// own, never a placeholder or one in conflict: the rule knows a fader
    /// by its label across layouts and restarts).
    fn strip(&self) -> Strip {
        let mark = if self.conflict() {
            Some(StripMark::Conflict)
        } else if self.problems.is_empty() {
            None
        } else {
            Some(StripMark::Problem)
        };
        Strip {
            binding: self.track(),
            strip_kind: match self.found.kind {
                TrackKind::Track => StripKind::Standard,
                TrackKind::Return => StripKind::Return,
            },
            wide: false,
            mute_guard: self.marker.mute_guard,
            pinned: self.marker.pin,
            label: Some(self.label()),
            mark,
            mute_links: self.links.clone(),
            reverb: self.marker.reverb && self.marker.label.is_some() && !self.conflict(),
            fader_link: self.leads.clone(),
            fader_follows: self.follows.clone(),
            lufs: self.marker.lufs,
        }
    }
}

/// Who carries an id of a Live set (#127, #130): a marker's `+ID` (its
/// entry) or a linked fader of the frame (its index in `linked_faders`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Carrier {
    Marker(usize),
    Frame(usize),
}

/// Every id of every Live set and its carriers.
type Ids = BTreeMap<(String, String), Vec<Carrier>>;

/// The ids the markers' `+ID` and the frame's linked faders carry.
fn carriers(entries: &[Entry<'_>], frame: &[(LinkId, String)]) -> Ids {
    let mut ids = Ids::new();
    for (i, entry) in entries.iter().enumerate() {
        if let Some(id) = &entry.marker.id {
            ids.entry((entry.found.instance.clone(), id.clone()))
                .or_default()
                .push(Carrier::Marker(i));
        }
    }
    for (f, (link, _)) in frame.iter().enumerate() {
        ids.entry((link.instance.clone(), link.id.clone()))
            .or_default()
            .push(Carrier::Frame(f));
    }
    ids
}

/// An id on two or more carriers is a problem on each marker that carries
/// it, and links nothing (#127; the frame's fader has no marker to mark).
fn duplicate_ids(entries: &mut [Entry<'_>], ids: &Ids) {
    for ((_, id), list) in ids {
        if list.len() > 1 {
            for carrier in list {
                if let Carrier::Marker(i) = carrier {
                    entries[*i]
                        .problems
                        .push(TagProblem::DuplicateId { id: id.clone() });
                }
            }
        }
    }
}

/// The carriers of `id` in `entry`'s Live set.
fn carried<'a>(ids: &'a Ids, entry: &Entry<'_>, id: &str) -> Option<&'a [Carrier]> {
    ids.get(&(entry.found.instance.clone(), id.to_string()))
        .map(Vec::as_slice)
}

/// The linked mutes (#127): a strip's `+ID` named by other markers'
/// `+MUTE` tags of its Live set (an id names a strip of its own instance)
/// gets their tracks, in Live's order. A `+MUTE` whose id no marker
/// carries (the frame's linked fader has no mute), or its own marker's, is
/// a problem on its marker (I9); a duplicate id links nothing.
fn link_mutes(entries: &mut [Entry<'_>], ids: &Ids) {
    let mut problems: Vec<(usize, TagProblem)> = Vec::new();
    let mut links: Vec<(usize, Binding)> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        for id in &entry.marker.mutes {
            match carried(ids, entry, id) {
                // No strip of its set carries the id (the frame's fader has
                // no mute).
                None | Some([Carrier::Frame(_)]) => {
                    problems.push((i, TagProblem::OrphanMute { id: id.clone() }));
                }
                Some([Carrier::Marker(own)]) if *own == i => {
                    problems.push((i, TagProblem::SelfMute { id: id.clone() }));
                }
                Some([Carrier::Marker(strip)]) => links.push((*strip, entry.track())),
                // A duplicate: marked on its carriers, nothing links.
                Some(_) => {}
            }
        }
    }
    for (i, problem) in problems {
        entries[i].problems.push(problem);
    }
    for (i, link) in links {
        entries[i].links.push(link);
    }
}

/// The fader links (#130, D18): a `+FADER:<id>` names the strip that
/// carries `+ID:<id>` in its Live set, or the frame's linked fader of that
/// link; the partner (its track by index and its label) goes to its
/// leader in Live's order (the entries are in it), and the leader's label
/// to the partner's strip. A `+FADER` whose id nothing carries, or only its
/// own strip, is a problem on its marker; a duplicate, or a leader that is
/// not placed (no `+G:`), links nothing.
/// Returns each frame fader's partners, by its index.
fn link_faders(
    entries: &mut [Entry<'_>],
    ids: &Ids,
    frame: &[(LinkId, String)],
) -> Vec<Vec<LinkPartner>> {
    let mut problems: Vec<(usize, TagProblem)> = Vec::new();
    let mut leads: Vec<(usize, String, LinkPartner)> = Vec::new();
    let mut follows: Vec<(usize, FaderFollow)> = Vec::new();
    let mut framed: Vec<Vec<LinkPartner>> = vec![Vec::new(); frame.len()];
    for (i, entry) in entries.iter().enumerate() {
        for id in &entry.marker.faders {
            let partner = LinkPartner {
                binding: entry.track(),
                label: entry.label(),
            };
            match carried(ids, entry, id) {
                None => problems.push((i, TagProblem::OrphanFader { id: id.clone() })),
                Some([Carrier::Marker(own)]) if *own == i => {
                    problems.push((i, TagProblem::SelfFader { id: id.clone() }));
                }
                // A placed leader only (#130 review M1): the hub keeps a
                // switch for a strip of the served layout.
                Some([Carrier::Marker(lead)]) if entries[*lead].placed() => {
                    follows.push((
                        i,
                        FaderFollow {
                            id: id.clone(),
                            leader: entries[*lead].label(),
                            fixed: false,
                        },
                    ));
                    leads.push((*lead, id.clone(), partner));
                }
                Some([Carrier::Frame(f)]) => {
                    follows.push((
                        i,
                        FaderFollow {
                            id: id.clone(),
                            leader: frame[*f].1.clone(),
                            fixed: true,
                        },
                    ));
                    framed[*f].push(partner);
                }
                // A duplicate (marked on its carriers), or a leader shown
                // nowhere (its own `NoGroup`): nothing links.
                Some(_) => {}
            }
        }
    }
    for (i, problem) in problems {
        entries[i].problems.push(problem);
    }
    for (i, follow) in follows {
        entries[i].follows.push(follow);
    }
    for (lead, id, partner) in leads {
        entries[lead]
            .leads
            .get_or_insert_with(|| FaderLink {
                id,
                partners: Vec::new(),
            })
            .partners
            .push(partner);
    }
    framed
}

/// A partner's volume as a linked fader's target (#130): its track by
/// index, `mixer_device volume`, the `value`, the MIDI map's scale (the
/// backing-tracks fader reads it as a volume, `vca::is_track_volume`).
fn volume_target(partner: &LinkPartner) -> ParamTarget {
    ParamTarget {
        binding: Binding {
            instance: partner.binding.instance.clone(),
            anchor: partner.binding.anchor.clone(),
            path: Some("mixer_device volume".to_string()),
        },
        prop: "value".to_string(),
        on: None,
        off: None,
        scale: Some(Scale::CcLinear),
    }
}

/// Fills every linked fader of the served layout with its partners'
/// volumes (one whose link nothing names gets none).
fn fill_links(layout: &mut Layout, frame: &[(LinkId, String)], partners: &[Vec<LinkPartner>]) {
    each_control_mut(layout, &mut |control: &mut Control| {
        if let Control::ParamFader {
            link: Some(link),
            targets,
            ..
        } = control
            && let Some(i) = frame.iter().position(|(l, _)| l == link)
        {
            *targets = partners[i].iter().map(volume_target).collect();
        }
    });
}

/// The `+AUTOTUNE` tracks (#104), placed or not, in Live's order (the
/// entries are in it): each track by index, its label, its macro's values,
/// its singer and whether it names two (#140 PR 3b).
fn autotune_targets(entries: &[Entry<'_>]) -> Vec<AutotuneTarget> {
    entries
        .iter()
        .filter_map(|entry| {
            entry.marker.autotune.map(|on| AutotuneTarget {
                binding: entry.track(),
                label: entry.label(),
                on: on.on(),
                off: on.off(),
                singer: singer(&entry.marker),
                two_singers: names_two_singers(&entry.marker),
            })
        })
        .collect()
}

/// Fills every autotune control of the served layout with the `+AUTOTUNE`
/// tracks (#104; a view's copied rail too, so it runs after the views).
fn fill_autotune(layout: &mut Layout, tracks: &[AutotuneTarget]) {
    each_control_mut(layout, &mut |control: &mut Control| {
        if let Control::Autotune { targets, .. } = control {
            *targets = tracks.to_vec();
        }
    });
}

/// Every control of the layout, mutable: each page's rail and rows (a
/// pager's sub-pages too), then the global controls.
fn each_control_mut(layout: &mut Layout, f: &mut impl FnMut(&mut Control)) {
    for page in &mut layout.pages {
        page.rail.iter_mut().for_each(&mut *f);
        for row in &mut page.rows {
            for section in &mut row.sections {
                each_group_mut(section, &mut |g: &mut Group| {
                    g.controls.iter_mut().for_each(&mut *f)
                });
            }
        }
    }
    layout.global.iter_mut().for_each(&mut *f);
}

/// Every group of `section`, mutable (a pager's sub-pages' too).
fn each_group_mut(section: &mut Section, f: &mut impl FnMut(&mut Group)) {
    match section {
        Section::Group(group) => f(group),
        Section::Pager(pager) => {
            for sub in &mut pager.pages {
                for s in &mut sub.sections {
                    each_group_mut(s, f);
                }
            }
        }
    }
}

/// Every tag group's members, by group name: the entries' indices in the
/// group's order (by place, then without a place in Live's order).
fn members(entries: &[Entry<'_>]) -> BTreeMap<String, Vec<usize>> {
    let mut groups: BTreeMap<String, Vec<(Option<u32>, usize)>> = BTreeMap::new();
    for (i, entry) in entries.iter().enumerate() {
        for tag in &entry.marker.groups {
            groups
                .entry(tag.name.clone())
                .or_default()
                .push((tag.place, i));
        }
    }
    groups
        .into_iter()
        .map(|(name, mut list)| {
            // A place first (smallest first), then none; equal: Live's order
            // (the entries are in it).
            list.sort_by_key(|&(place, i)| (place.is_none(), place, i));
            (name, list.into_iter().map(|(_, i)| i).collect())
        })
        .collect()
}

/// The tag group names a frame group shows (`Group.tags`), in document
/// order.
fn frame_tags(frame: &Layout) -> BTreeSet<String> {
    frame
        .pages
        .iter()
        .flat_map(|p| &p.rows)
        .flat_map(|r| &r.sections)
        .flat_map(Section::groups)
        .filter_map(|g| g.tags.clone())
        .collect()
}

/// Fills every frame group that names a tag group.
fn fill(sections: &mut [Section], strips: &BTreeMap<String, Vec<Strip>>) {
    for section in sections {
        match section {
            Section::Group(group) => {
                if let Some(name) = &group.tags {
                    // Its marker strips, then its own controls (no strip).
                    let own = std::mem::take(&mut group.controls);
                    group.controls = strips
                        .get(name)
                        .map(|list| {
                            list.iter()
                                .cloned()
                                .map(|s| Control::Strip(Box::new(s)))
                                .collect()
                        })
                        .unwrap_or_default();
                    group.controls.extend(own);
                }
            }
            Section::Pager(pager) => {
                for sub in &mut pager.pages {
                    fill(&mut sub.sections, strips);
                }
            }
        }
    }
}

/// The frame and the markers as the served layout, and every problem.
pub fn compose(frame: &Layout, found: &[Found]) -> Composed {
    let mut sorted: Vec<&Found> = found.iter().collect();
    sorted.sort_by(|a, b| a.key().cmp(&b.key()));
    let mut entries: Vec<Entry<'_>> = sorted
        .into_iter()
        .map(|f| {
            let marker = parse(&f.name);
            let mut problems = marker.problems.clone();
            if f.tuners > 1 {
                problems.push(TagProblem::DoubleTuner);
            }
            Entry {
                found: f,
                marker,
                problems,
                links: Vec::new(),
                leads: None,
                follows: Vec::new(),
            }
        })
        .collect();
    // An equal label on two or more markers: a conflict on each.
    let mut labels: BTreeMap<String, usize> = BTreeMap::new();
    for entry in &entries {
        if let Some(label) = &entry.marker.label {
            *labels.entry(label.clone()).or_default() += 1;
        }
    }
    for entry in &mut entries {
        if let Some(label) = &entry.marker.label
            && labels.get(label).copied().unwrap_or(0) > 1
        {
            entry.problems.push(TagProblem::Conflict {
                label: label.clone(),
            });
        }
    }
    let frame_links = frame.linked_faders();
    let ids = carriers(&entries, &frame_links);
    duplicate_ids(&mut entries, &ids);
    link_mutes(&mut entries, &ids);
    let frame_partners = link_faders(&mut entries, &ids, &frame_links);
    let ctrls = ctrl::carriers(&entries);
    ctrl::mark_duplicates(&mut entries, &ctrls);
    let groups = members(&entries);
    // Two strips on one place of a group: both marked.
    for (name, list) in &groups {
        let places: Vec<(usize, u32)> = list
            .iter()
            .filter_map(|&i| {
                entries[i]
                    .marker
                    .groups
                    .iter()
                    .find(|g| &g.name == name)
                    .and_then(|g| g.place)
                    .map(|place| (i, place))
            })
            .collect();
        for &(i, place) in &places {
            if places.iter().filter(|&&(_, p)| p == place).count() > 1 {
                entries[i].problems.push(TagProblem::SamePlace {
                    group: name.clone(),
                    place,
                });
            }
        }
    }
    let strips: Vec<Strip> = entries.iter().map(Entry::strip).collect();
    let by_group: BTreeMap<String, Vec<Strip>> = groups
        .iter()
        .map(|(name, list)| {
            (
                name.clone(),
                list.iter().map(|&i| strips[i].clone()).collect(),
            )
        })
        .collect();
    let mut layout = frame.clone();
    for page in &mut layout.pages {
        for row in &mut page.rows {
            fill(&mut row.sections, &by_group);
        }
    }
    // Every tag group the frame does not show is a view, in the order of
    // its first marker.
    let shown = frame_tags(frame);
    let mut views: Vec<(usize, &String)> = groups
        .iter()
        .filter(|(name, _)| !shown.contains(*name))
        .map(|(name, list)| (list.iter().copied().min().unwrap_or(0), name))
        .collect();
    views.sort();
    // A view keeps the rail of the frame's default page (the approved
    // mockup: the column's rail stays while a view is shown).
    let rail = frame
        .pages
        .iter()
        .find(|p| p.id == frame.default_page)
        .or_else(|| frame.pages.first())
        .map(|p| p.rail.clone())
        .unwrap_or_default();
    for (_, name) in views {
        layout
            .pages
            .push(view_page(name, &groups, &entries, &strips, &rail));
    }
    fill_links(&mut layout, &frame_links, &frame_partners);
    fill_autotune(&mut layout, &autotune_targets(&entries));
    ctrl::fill(&mut layout, &ctrl::tracks(&entries, &ctrls));
    let mut links: Vec<LinkReport> = entries
        .iter()
        .filter_map(|e| {
            e.leads.as_ref().map(|l| LinkReport {
                instance: e.found.instance.clone(),
                id: l.id.clone(),
                leader: Leader::Strip,
                partners: l.partners.len(),
            })
        })
        .collect();
    links.extend(
        frame_links
            .iter()
            .zip(&frame_partners)
            .map(|((l, _), p)| LinkReport {
                instance: l.instance.clone(),
                id: l.id.clone(),
                leader: Leader::Frame,
                partners: p.len(),
            }),
    );
    links.sort();
    let problems = entries
        .iter()
        .filter(|e| !e.problems.is_empty())
        .map(|e| MarkerReport {
            instance: e.found.instance.clone(),
            kind: e.found.kind,
            index: e.found.index,
            name: e.found.name.clone(),
            problems: e.problems.clone(),
        })
        .collect();
    Composed {
        layout,
        problems,
        links,
        ctrls: ctrl::reports(frame, &ctrls),
    }
}

/// A view: its group's strips, then every pinned strip not in it, under
/// the title of each one's first group (in the order of the markers).
fn view_page(
    name: &str,
    groups: &BTreeMap<String, Vec<usize>>,
    entries: &[Entry<'_>],
    strips: &[Strip],
    rail: &[Control],
) -> Page {
    let own = groups.get(name).cloned().unwrap_or_default();
    let mut sections = vec![Section::Group(Group {
        id: Some(format!("{VIEW_PREFIX}{name}-strips")),
        title: Some(group_title(name)),
        color: None,
        tags: None,
        controls: own
            .iter()
            .map(|&i| Control::Strip(Box::new(strips[i].clone())))
            .collect(),
    })];
    let mut pins: Vec<(String, Vec<usize>)> = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let Some(first) = entry.marker.groups.first() else {
            continue;
        };
        if !entry.marker.pin || own.contains(&i) {
            continue;
        }
        match pins.iter_mut().find(|(g, _)| g == &first.name) {
            Some((_, list)) => list.push(i),
            None => pins.push((first.name.clone(), vec![i])),
        }
    }
    for (group, list) in pins {
        sections.push(Section::Group(Group {
            id: Some(format!("{VIEW_PREFIX}{name}-pins-{group}")),
            title: Some(group_title(&group)),
            color: None,
            tags: None,
            controls: list
                .iter()
                .map(|&i| Control::Strip(Box::new(strips[i].clone())))
                .collect(),
        }));
    }
    Page {
        id: format!("{VIEW_PREFIX}{name}"),
        title: group_title(name),
        view: true,
        rail: rail.to_vec(),
        rows: vec![Row {
            sections,
            weight: 1.0,
        }],
    }
}

mod ctrl;
pub mod migrate;

#[cfg(test)]
mod tests;
