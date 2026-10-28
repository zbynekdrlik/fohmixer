//! The fader sectors of the FOH page (#148; spec
//! `docs/superpowers/specs/2026-10-28-sectors-design.md`): four sectors
//! around the control column (`lu`, `ld`, `ru`, `rd`), each with its own
//! named layouts, each layout an ordered list of faders. The hub keeps the
//! document (`sectors.json`), the pages show it and change it with a tick or
//! an ARRANGE save.
//!
//! A fader is known by [`StripRef`] (its Live set and its label), the
//! identity the REVERB rule keeps across layouts and restarts; the pool is
//! every fader of the served layout that has one. Everything here is pure:
//! the hub and the UI share it.
//!
//! An unknown field is an error, not ignored (the file is edited by hand,
//! as the layout is).

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::layout::{Anchor, Control, Group, Layout, Row, Section, Strip, StripMark};
use crate::markers::has_placeholder_label;

/// How many units a sector holds on the iPad: a layout past it is FULL.
pub const SECTOR_UNITS: f64 = 6.0;
/// A wide strip's units.
pub const WIDE_UNITS: f64 = 1.1;
/// A layout title's most characters (the column cell's room).
pub const TITLE_MAX: usize = 8;
/// The title of a seeded layout whose group has none, and of an empty one.
pub const SEED_TITLE: &str = "MAIN";

/// A sector: left up, left down, right up, right down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SectorId {
    Lu,
    Ld,
    Ru,
    Rd,
}

impl SectorId {
    /// Every sector, in the document's order.
    pub const ALL: [SectorId; 4] = [SectorId::Lu, SectorId::Ld, SectorId::Ru, SectorId::Rd];

    /// Its id in the document: `lu`, `ld`, `ru`, `rd`.
    pub fn as_str(self) -> &'static str {
        match self {
            SectorId::Lu => "lu",
            SectorId::Ld => "ld",
            SectorId::Ru => "ru",
            SectorId::Rd => "rd",
        }
    }

    /// Whether it is on the column's left.
    pub fn left(self) -> bool {
        matches!(self, SectorId::Lu | SectorId::Ld)
    }

    /// Whether it is the upper one of its side.
    pub fn up(self) -> bool {
        matches!(self, SectorId::Lu | SectorId::Ru)
    }

    /// The sector up or down, left or right.
    pub fn of(up: bool, left: bool) -> SectorId {
        match (up, left) {
            (true, true) => SectorId::Lu,
            (false, true) => SectorId::Ld,
            (true, false) => SectorId::Ru,
            (false, false) => SectorId::Rd,
        }
    }
}

/// A fader's identity: its Live set and its label.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StripRef {
    pub instance: String,
    pub label: String,
}

/// A named layout of a sector: its faders in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectorLayout {
    pub id: String,
    pub title: String,
    pub strips: Vec<StripRef>,
}

/// The document, `sectors.json`: the hub's revision and each sector's
/// layouts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectorsDoc {
    pub rev: u64,
    pub sectors: BTreeMap<SectorId, Vec<SectorLayout>>,
}

/// One layout's new order (ARRANGE's save).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectorOrder {
    pub layout: String,
    pub strips: Vec<StripRef>,
}

impl SectorsDoc {
    /// The empty document: revision 0, no layout (sectors off).
    pub fn empty() -> Self {
        Self {
            rev: 0,
            sectors: BTreeMap::new(),
        }
    }

    /// Whether it holds at least one layout (the sectors replace the
    /// default page's rows).
    pub fn is_on(&self) -> bool {
        self.sectors.values().any(|list| !list.is_empty())
    }

    /// A sector's layouts (none for a sector it does not name).
    pub fn layouts(&self, sector: SectorId) -> &[SectorLayout] {
        self.sectors.get(&sector).map_or(&[], Vec::as_slice)
    }

    /// The layout `id` and its sector.
    pub fn find(&self, id: &str) -> Option<(SectorId, &SectorLayout)> {
        self.sectors.iter().find_map(|(sector, list)| {
            list.iter()
                .find(|layout| layout.id == id)
                .map(|layout| (*sector, layout))
        })
    }

    fn find_mut(&mut self, id: &str) -> Option<&mut SectorLayout> {
        self.sectors
            .values_mut()
            .flat_map(|list| list.iter_mut())
            .find(|layout| layout.id == id)
    }

    /// Every problem of the document (empty: valid): an id empty or on two
    /// layouts, a title empty or longer than [`TITLE_MAX`] characters, a
    /// strip twice in one layout.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let mut ids: BTreeMap<&str, usize> = BTreeMap::new();
        for layout in self.sectors.values().flatten() {
            *ids.entry(layout.id.as_str()).or_default() += 1;
            if layout.id.trim().is_empty() {
                errors.push("a layout has an empty id".to_string());
            }
            if layout.title.trim().is_empty() {
                errors.push(format!("layout {:?} has an empty title", layout.id));
            }
            if layout.title.chars().count() > TITLE_MAX {
                errors.push(format!(
                    "layout {:?}: its title {:?} is longer than {TITLE_MAX} characters",
                    layout.id, layout.title
                ));
            }
            for (i, strip) in layout.strips.iter().enumerate() {
                if layout.strips.iter().take(i).any(|s| s == strip) {
                    errors.push(format!(
                        "layout {:?} holds {}|{} twice",
                        layout.id, strip.instance, strip.label
                    ));
                }
            }
        }
        for (id, n) in ids {
            if n > 1 {
                errors.push(format!("the layout id {id:?} is used {n} times"));
            }
        }
        errors
    }

    /// How many layouts it holds.
    pub fn layout_count(&self) -> usize {
        self.sectors.values().map(Vec::len).sum()
    }

    /// How many distinct faders its layouts name.
    pub fn strip_count(&self) -> usize {
        self.refs().len()
    }

    fn refs(&self) -> BTreeSet<&StripRef> {
        self.sectors
            .values()
            .flatten()
            .flat_map(|layout| &layout.strips)
            .collect()
    }
}

/// A fader of the pool: its identity and its control in the served layout.
#[derive(Debug, Clone, PartialEq)]
pub struct Pooled<'a> {
    pub id: StripRef,
    pub control: &'a Control,
}

impl Pooled<'_> {
    /// Its units: a wide strip [`WIDE_UNITS`], anything else 1.
    pub fn units(&self) -> f64 {
        match self.control {
            Control::Strip(strip) if strip.wide => WIDE_UNITS,
            _ => 1.0,
        }
    }

    /// Its strip, when it is one (a labelled parameter fader is not).
    pub fn strip(&self) -> Option<&Strip> {
        match self.control {
            Control::Strip(strip) => Some(strip),
            _ => None,
        }
    }
}

/// A control's identity: a strip's label (none for a placeholder label or
/// a strip in conflict), else a frame strip's anchor name (`Master` for the
/// master track); a parameter fader's label, in its link's Live set, else
/// its first target's (call 1 of the plan). Nothing else has one.
pub fn strip_ref(control: &Control) -> Option<StripRef> {
    match control {
        Control::Strip(strip) => {
            if strip.mark == Some(StripMark::Conflict) || has_placeholder_label(strip) {
                return None;
            }
            let label = match (&strip.label, &strip.binding.anchor) {
                (Some(label), _) => label.clone(),
                (None, Anchor::Track { name } | Anchor::Return { name }) => name.clone(),
                (None, Anchor::Master) => "Master".to_string(),
                (None, _) => return None,
            };
            Some(StripRef {
                instance: strip.binding.instance.clone(),
                label,
            })
            .filter(|r| !r.label.is_empty())
        }
        Control::ParamFader {
            label,
            targets,
            link,
        } => {
            let instance = link
                .as_ref()
                .map(|l| l.instance.clone())
                .or_else(|| targets.first().map(|t| t.binding.instance.clone()))?;
            Some(StripRef {
                instance,
                label: label.clone(),
            })
            .filter(|r| !r.label.is_empty())
        }
        _ => None,
    }
}

/// The pool: every control of the layout with an identity (every page, the
/// views and rails included, in document order), each identity once, the
/// first place winning.
pub fn pool(layout: &Layout) -> Vec<Pooled<'_>> {
    let mut out: Vec<Pooled<'_>> = Vec::new();
    for control in layout.controls() {
        if let Some(id) = strip_ref(control)
            && !out.iter().any(|p| p.id == id)
        {
            out.push(Pooled { id, control });
        }
    }
    out
}

/// The units of these faders: a ref no pool fader carries takes no room.
pub fn units(strips: &[StripRef], pool: &[Pooled]) -> f64 {
    strips
        .iter()
        .filter_map(|strip| pool.iter().find(|p| &p.id == strip))
        .map(|p| p.units())
        .sum()
}

/// Whether `units` is past a sector's room.
pub fn is_full(units: f64) -> bool {
    units > SECTOR_UNITS
}

/// Whether `strip` added to `layout` would make it full.
pub fn would_be_full(layout: &SectorLayout, strip: &StripRef, pool: &[Pooled]) -> bool {
    is_full(units(&layout.strips, pool) + units(std::slice::from_ref(strip), pool))
}

/// How many distinct refs of the document no pool fader carries (a track
/// renamed in Live: it shows nowhere until renamed back).
pub fn unknown(doc: &SectorsDoc, pool: &[Pooled]) -> usize {
    doc.refs()
        .into_iter()
        .filter(|r| !pool.iter().any(|p| &p.id == *r))
        .count()
}

/// Why the hub refuses a page's change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    UnknownLayout,
    NotInPool,
    Full,
    Stale,
    NothingToUndo,
    NoLayout,
}

impl Refusal {
    /// The words the hub's `error` answer carries.
    pub fn text(self) -> &'static str {
        match self {
            Refusal::UnknownLayout => "unknown layout",
            Refusal::NotInPool => "not in the pool",
            Refusal::Full => "FULL",
            Refusal::Stale => "stale",
            Refusal::NothingToUndo => "nothing to undo",
            Refusal::NoLayout => "no layout served",
        }
    }
}

/// A tick of `strip` in `layout`: on appends it at the end (it must be in
/// the pool and fit), off removes it (in the pool or not). A tick that
/// changes nothing is `Ok(None)`; a change is the document at `rev + 1`.
pub fn tick(
    doc: &SectorsDoc,
    layout: &str,
    strip: &StripRef,
    on: bool,
    pool: &[Pooled],
) -> Result<Option<SectorsDoc>, Refusal> {
    let (_, current) = doc.find(layout).ok_or(Refusal::UnknownLayout)?;
    if current.strips.contains(strip) == on {
        return Ok(None);
    }
    if on && !pool.iter().any(|p| &p.id == strip) {
        return Err(Refusal::NotInPool);
    }
    if on && would_be_full(current, strip, pool) {
        return Err(Refusal::Full);
    }
    let mut next = doc.clone();
    next.rev += 1;
    if let Some(target) = next.find_mut(layout) {
        if on {
            target.strips.push(strip.clone());
        } else {
            target.strips.retain(|s| s != strip);
        }
    }
    Ok(Some(next))
}

/// Whether `a` and `b` hold the same refs, each as often.
fn same_set(a: &[StripRef], b: &[StripRef]) -> bool {
    let mut a = a.to_vec();
    let mut b = b.to_vec();
    a.sort();
    b.sort();
    a == b
}

/// ARRANGE's save: each layout of `orders` takes its new order, as one
/// change at `rev + 1`. It is `Stale` unless `rev` is the document's, each
/// layout exists and is named once, and each order holds the layout's own
/// refs.
pub fn orders(doc: &SectorsDoc, orders: &[SectorOrder], rev: u64) -> Result<SectorsDoc, Refusal> {
    if rev != doc.rev {
        return Err(Refusal::Stale);
    }
    let mut next = doc.clone();
    for (i, order) in orders.iter().enumerate() {
        if orders.iter().take(i).any(|o| o.layout == order.layout) {
            return Err(Refusal::Stale);
        }
        let target = next.find_mut(&order.layout).ok_or(Refusal::Stale)?;
        if !same_set(&target.strips, &order.strips) {
            return Err(Refusal::Stale);
        }
        target.strips = order.strips.clone();
    }
    next.rev += 1;
    Ok(next)
}

/// A seeded layout's title: the given one trimmed and cut to
/// [`TITLE_MAX`] characters, else [`SEED_TITLE`].
fn seed_title(title: Option<&str>) -> String {
    title
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or(SEED_TITLE)
        .chars()
        .take(TITLE_MAX)
        .collect()
}

/// A seeded layout of these groups' faders, each once (its id set later).
fn seed_layout<'a>(
    title: Option<&str>,
    groups: impl IntoIterator<Item = &'a Group>,
) -> SectorLayout {
    let mut strips: Vec<StripRef> = Vec::new();
    for control in groups.into_iter().flat_map(|g| &g.controls) {
        if let Some(id) = strip_ref(control)
            && !strips.contains(&id)
        {
            strips.push(id);
        }
    }
    SectorLayout {
        id: String::new(),
        title: seed_title(title),
        strips,
    }
}

/// Where a row with no pager is cut, as a count of groups on the left: the
/// group boundary where the larger side holds the fewest faders; on a tie
/// the cut with more on the left.
fn seed_cut(counts: &[usize]) -> usize {
    let total: usize = counts.iter().sum();
    std::iter::once(0)
        .chain(counts.iter().scan(0, |left, n| {
            *left += n;
            Some(*left)
        }))
        .enumerate()
        .min_by_key(|&(_, left)| (left.max(total - left), Reverse(left)))
        .map_or(0, |(cut, _)| cut)
}

/// A row's left and right layouts: a pager's sub-pages on the left and the
/// row's other groups as one right layout; a row with no pager cut at
/// [`seed_cut`], one layout a side.
fn seed_row(row: &Row) -> (Vec<SectorLayout>, Vec<SectorLayout>) {
    let groups: Vec<&Group> = row
        .sections
        .iter()
        .filter_map(|s| match s {
            Section::Group(group) => Some(group),
            Section::Pager(_) => None,
        })
        .collect();
    let first_title = |side: &[&Group]| side.first().and_then(|g| g.title.clone());
    let pager = row.sections.iter().find_map(|s| match s {
        Section::Pager(pager) => Some(pager),
        Section::Group(_) => None,
    });
    if let Some(pager) = pager {
        let left = pager
            .pages
            .iter()
            .map(|sub| {
                seed_layout(
                    Some(sub.title.as_str()),
                    sub.sections.iter().flat_map(Section::groups),
                )
            })
            .collect();
        let right = seed_layout(first_title(&groups).as_deref(), groups.iter().copied());
        return (left, vec![right]);
    }
    let counts: Vec<usize> = groups
        .iter()
        .map(|g| g.controls.iter().filter(|c| strip_ref(c).is_some()).count())
        .collect();
    let (left, right) = groups.split_at(seed_cut(&counts));
    (
        vec![seed_layout(
            first_title(left).as_deref(),
            left.iter().copied(),
        )],
        vec![seed_layout(
            first_title(right).as_deref(),
            right.iter().copied(),
        )],
    )
}

/// The first document, from the served layout's default page: row 0 up,
/// row 1 down (the rest ignored); a sector left empty gets one empty
/// [`SEED_TITLE`] layout; ids `<sector>-<n>` from 1; revision 1. FULL is
/// not enforced here.
pub fn seed(layout: &Layout) -> SectorsDoc {
    let mut sectors: BTreeMap<SectorId, Vec<SectorLayout>> = BTreeMap::new();
    if let Some(page) = layout.pages.iter().find(|p| p.id == layout.default_page) {
        for (row, up) in page.rows.iter().zip([true, false]) {
            let (left, right) = seed_row(row);
            sectors.insert(SectorId::of(up, true), left);
            sectors.insert(SectorId::of(up, false), right);
        }
    }
    for sector in SectorId::ALL {
        let list = sectors.entry(sector).or_default();
        if list.is_empty() {
            list.push(seed_layout(None, std::iter::empty()));
        }
        for (n, layout) in list.iter_mut().enumerate() {
            layout.id = format!("{}-{}", sector.as_str(), n + 1);
        }
    }
    SectorsDoc { rev: 1, sectors }
}

#[cfg(test)]
mod tests;
