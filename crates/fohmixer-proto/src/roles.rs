//! The logins' roles (#184, spec `docs/superpowers/specs/2026-11-12-mbc-instance-design.md`
//! §6.4): the FOH engineer reaches everything, an MBC login only the Live
//! set `mbc`. Every decision is pure and lives here; the hub enforces them
//! at its three choke points (the HTTP routes, a WebSocket's inbound
//! messages and its writer's outbound ones), the page only reads the role.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::layout::{Control, Group, Layout, Page, Pager, Row, Section, SubPage};
use crate::sectors::{SectorLayout, SectorsDoc, StripRef, trim};

/// The Live set an MBC login reaches.
pub const MBC_INSTANCE: &str = "mbc";
/// The id of the one empty page an MBC login gets when none of the layout's
/// pages holds an MBC control (the page needs something to mount).
pub const EMPTY_PAGE_ID: &str = "mbc";
/// That page's title.
pub const EMPTY_PAGE_TITLE: &str = "MBC";

/// A login's role: its token's subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// The FOH engineer: every Live set, every route.
    Engineer,
    /// The MBC operator: the Live set [`MBC_INSTANCE`] only.
    Mbc,
}

impl Role {
    /// The role a token's subject names (none for any other subject).
    pub fn from_sub(sub: &str) -> Option<Role> {
        match sub {
            "engineer" => Some(Role::Engineer),
            "mbc" => Some(Role::Mbc),
            _ => None,
        }
    }

    /// Its token's subject.
    pub fn sub(self) -> &'static str {
        match self {
            Role::Engineer => "engineer",
            Role::Mbc => "mbc",
        }
    }

    /// Its name in lines and records (the subject).
    pub fn as_str(self) -> &'static str {
        self.sub()
    }

    /// Whether it reaches the Live set `instance`.
    pub fn reaches(self, instance: &str) -> bool {
        match self {
            Role::Engineer => true,
            Role::Mbc => instance == MBC_INSTANCE,
        }
    }
}

/// Whether `role` reaches the fader `strip`.
pub fn reaches_strip(role: Role, strip: &StripRef) -> bool {
    role.reaches(&strip.instance)
}

/// The Live set a control belongs to: its binding's, a parameter control's
/// link or first target's; none for a control of the hub's own rules
/// (REVERB, AUTOTUNE), which only the engineer runs.
fn control_instance(control: &Control) -> Option<&str> {
    match control {
        Control::Strip(strip) => Some(&strip.binding.instance),
        Control::Solo { binding, .. }
        | Control::Stage { binding, .. }
        | Control::Alert { binding, .. } => Some(&binding.instance),
        Control::ParamToggle { targets, .. } => {
            targets.first().map(|t| t.binding.instance.as_str())
        }
        Control::ParamFader { targets, link, .. } => link
            .as_ref()
            .map(|l| l.instance.as_str())
            .or_else(|| targets.first().map(|t| t.binding.instance.as_str())),
        Control::Reverb { .. } | Control::Autotune { .. } => None,
    }
}

/// Whether an MBC login sees `control`.
fn mbc_sees(control: &Control) -> bool {
    control_instance(control) == Some(MBC_INSTANCE)
}

/// The MBC controls of `controls`. The helpers below build an MBC login's
/// view only: the engineer is served the layout as it is.
fn mbc_controls(controls: &[Control]) -> Vec<Control> {
    controls.iter().filter(|c| mbc_sees(c)).cloned().collect()
}

fn mbc_group(group: &Group) -> Option<Group> {
    let controls = mbc_controls(&group.controls);
    (!controls.is_empty()).then(|| Group {
        id: group.id.clone(),
        title: group.title.clone(),
        color: group.color.clone(),
        tags: group.tags.clone(),
        controls,
    })
}

fn mbc_sections(sections: &[Section]) -> Vec<Section> {
    sections
        .iter()
        .filter_map(|section| match section {
            Section::Group(group) => mbc_group(group).map(Section::Group),
            Section::Pager(pager) => mbc_pager(pager).map(Section::Pager),
        })
        .collect()
}

fn mbc_pager(pager: &Pager) -> Option<Pager> {
    let pages: Vec<SubPage> = pager
        .pages
        .iter()
        .filter_map(|sub| {
            let sections = mbc_sections(&sub.sections);
            (!sections.is_empty()).then(|| SubPage {
                id: sub.id.clone(),
                title: sub.title.clone(),
                sections,
            })
        })
        .collect();
    let first = pages.first()?.id.clone();
    let default_page = if pages.iter().any(|p| p.id == pager.default_page) {
        pager.default_page.clone()
    } else {
        first
    };
    Some(Pager {
        id: pager.id.clone(),
        default_page,
        pages,
    })
}

fn mbc_page(page: &Page) -> Option<Page> {
    let rail = mbc_controls(&page.rail);
    let rows: Vec<Row> = page
        .rows
        .iter()
        .filter_map(|row| {
            let sections = mbc_sections(&row.sections);
            (!sections.is_empty()).then_some(Row {
                sections,
                weight: row.weight,
            })
        })
        .collect();
    (!rail.is_empty() || !rows.is_empty()).then(|| Page {
        id: page.id.clone(),
        title: page.title.clone(),
        view: page.view,
        rail,
        rows,
    })
}

/// The layout `role` is served: the engineer the whole layout; an MBC login
/// only the controls of [`MBC_INSTANCE`] (every rail, row, pager and the
/// global controls), a group, sub-page, pager, row or page left empty
/// dropped, the default page kept when it is left, else the first page; a
/// layout with no page left is one empty page ([`EMPTY_PAGE_ID`]). Its
/// `report` (what the import dropped: track names) is not an MBC login's.
pub fn layout_for(role: Role, layout: &Layout) -> Layout {
    match role {
        Role::Engineer => layout.clone(),
        Role::Mbc => {
            let mut pages: Vec<Page> = layout.pages.iter().filter_map(mbc_page).collect();
            if pages.is_empty() {
                pages.push(Page {
                    id: EMPTY_PAGE_ID.to_string(),
                    title: EMPTY_PAGE_TITLE.to_string(),
                    view: false,
                    rail: Vec::new(),
                    rows: Vec::new(),
                });
            }
            let default_page = if pages.iter().any(|p| p.id == layout.default_page) {
                layout.default_page.clone()
            } else {
                pages[0].id.clone()
            };
            Layout {
                schema: layout.schema,
                default_page,
                pages,
                global: mbc_controls(&layout.global),
                config: layout.config.clone(),
                report: Value::Null,
            }
        }
    }
}

/// A layout as an MBC login sees it: none when it held faders of other Live
/// sets and is left without one (an empty layout stays as it was).
fn mbc_sector_layout(layout: &SectorLayout) -> Option<SectorLayout> {
    let strips: Vec<_> = layout
        .strips
        .iter()
        .map(|slot| slot.clone().filter(|s| reaches_strip(Role::Mbc, s)))
        .collect();
    let emptied = layout.refs().next().is_some() && strips.iter().all(Option::is_none);
    (!emptied).then(|| SectorLayout {
        id: layout.id.clone(),
        title: layout.title.clone(),
        strips: trim(strips),
    })
}

/// The sectors document `role` is served: the engineer the whole one; an
/// MBC login every layout with only its faders (another Live set's slot
/// left empty, the trailing ones dropped), a layout emptied that way
/// dropped, except that a sector keeps its first layout, emptied, when it
/// would be left with none. The revision is the document's.
pub fn sectors_for(role: Role, doc: &SectorsDoc) -> SectorsDoc {
    match role {
        Role::Engineer => doc.clone(),
        Role::Mbc => SectorsDoc {
            rev: doc.rev,
            sectors: doc
                .sectors
                .iter()
                .map(|(sector, layouts)| {
                    let mut kept: Vec<SectorLayout> =
                        layouts.iter().filter_map(mbc_sector_layout).collect();
                    if kept.is_empty()
                        && let Some(first) = layouts.first()
                    {
                        kept.push(SectorLayout {
                            id: first.id.clone(),
                            title: first.title.clone(),
                            strips: Vec::new(),
                        });
                    }
                    (*sector, kept)
                })
                .collect(),
        },
    }
}

/// Whether `role` may change the sectors document `doc` (a tick or an
/// ARRANGE save): the engineer always; an MBC login only a document it sees
/// whole (the MBC hub's, every fader an MBC one), never the FOH hub's,
/// where it is served a derived view.
pub fn may_edit_sectors(role: Role, doc: &SectorsDoc) -> bool {
    match role {
        Role::Engineer => true,
        Role::Mbc => sectors_for(role, doc) == *doc,
    }
}

/// Whether `role` may change the sectors document `doc` of a hub whose
/// pool holds `pool` (a tick or an ARRANGE save): the engineer always; an
/// MBC login only on a hub every pool fader of which it reaches (the MBC
/// hub) and whose document it sees whole ([`may_edit_sectors`]). On the
/// FOH hub (band and master faders in its pool) an MBC login changes
/// nothing, an empty document's layouts included (#184 review: a tick of a
/// band fader into an empty FOH document).
pub fn may_arrange<'a>(
    role: Role,
    doc: &SectorsDoc,
    pool: impl IntoIterator<Item = &'a StripRef>,
) -> bool {
    may_edit_sectors(role, doc) && pool.into_iter().all(|s| reaches_strip(role, s))
}

#[cfg(test)]
#[path = "roles/tests.rs"]
mod tests;
