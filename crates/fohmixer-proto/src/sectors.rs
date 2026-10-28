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
//! A layout is a row of slots (1.24.0, the owner's report of 28.10: ARRANGE
//! packed the strips against each other, so BAND could not sit at RU's right
//! edge): `strips` holds a fader or `null`, an empty slot, drawn as empty
//! space one strip wide where a strip would sit. An empty slot counts one
//! unit toward [`SECTOR_UNITS`]; trailing empty slots are dropped
//! ([`trim`]), so a file without `null` reads as before.
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

/// A layout's slot: a fader, or none (an empty slot, `null` in the file).
pub type Slot = Option<StripRef>;

/// A named layout of a sector: its slots in order, each a fader or empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectorLayout {
    pub id: String,
    pub title: String,
    pub strips: Vec<Slot>,
}

impl SectorLayout {
    /// Its faders in their order, the empty slots left out.
    pub fn refs(&self) -> impl Iterator<Item = &StripRef> {
        self.strips.iter().flatten()
    }

    /// Whether it holds `strip`.
    pub fn holds(&self, strip: &StripRef) -> bool {
        self.refs().any(|s| s == strip)
    }
}

/// `slots` without their trailing empty slots.
pub fn trim(mut slots: Vec<Slot>) -> Vec<Slot> {
    let kept = slots.len() - slots.iter().rev().take_while(|s| s.is_none()).count();
    slots.truncate(kept);
    slots
}

/// The document, `sectors.json`: the hub's revision and each sector's
/// layouts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectorsDoc {
    pub rev: u64,
    pub sectors: BTreeMap<SectorId, Vec<SectorLayout>>,
}

/// One layout's new slots (ARRANGE's save), its empty slots `null`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SectorOrder {
    pub layout: String,
    pub strips: Vec<Slot>,
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

    /// The document with every layout's trailing empty slots dropped (a
    /// hand edit's; the hub's own changes never keep one).
    pub fn trimmed(mut self) -> Self {
        for layout in self.sectors.values_mut().flatten() {
            layout.strips = trim(std::mem::take(&mut layout.strips));
        }
        self
    }

    fn find_mut(&mut self, id: &str) -> Option<&mut SectorLayout> {
        self.sectors
            .values_mut()
            .flat_map(|list| list.iter_mut())
            .find(|layout| layout.id == id)
    }

    /// Every problem of the document (empty: valid): an id empty or on two
    /// layouts, a title empty or longer than [`TITLE_MAX`] characters, a
    /// strip twice in one layout (empty slots are no strip).
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
            let refs: Vec<&StripRef> = layout.refs().collect();
            for (i, strip) in refs.iter().enumerate() {
                if refs.iter().take(i).any(|s| s == strip) {
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
            .flat_map(SectorLayout::refs)
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
        control_units(self.control)
    }

    /// Its strip, when it is one (a labelled parameter fader is not).
    pub fn strip(&self) -> Option<&Strip> {
        match self.control {
            Control::Strip(strip) => Some(strip),
            _ => None,
        }
    }
}

/// A control's units in a sector: a wide strip [`WIDE_UNITS`], anything
/// else 1.
fn control_units(control: &Control) -> f64 {
    match control {
        Control::Strip(strip) if strip.wide => WIDE_UNITS,
        _ => 1.0,
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

/// The units of these slots: an empty slot 1, a fader its own; a ref no
/// pool fader carries takes no room.
pub fn units(slots: &[Slot], pool: &[Pooled]) -> f64 {
    slots
        .iter()
        .filter_map(|slot| match slot {
            None => Some(1.0),
            Some(strip) => pool.iter().find(|p| &p.id == strip).map(Pooled::units),
        })
        .sum()
}

/// Whether `units` is past a sector's room.
pub fn is_full(units: f64) -> bool {
    units > SECTOR_UNITS
}

/// `slots` with `strip` ticked in: into the first empty slot, else at the
/// end.
pub fn ticked_in(slots: &[Slot], strip: &StripRef) -> Vec<Slot> {
    let mut out = slots.to_vec();
    match out.iter_mut().find(|slot| slot.is_none()) {
        Some(empty) => *empty = Some(strip.clone()),
        None => out.push(Some(strip.clone())),
    }
    out
}

/// `slots` with `strip` ticked out: its slot left empty (the others keep
/// their places), the trailing empty slots dropped.
pub fn ticked_out(slots: &[Slot], strip: &StripRef) -> Vec<Slot> {
    trim(
        slots
            .iter()
            .map(|slot| slot.clone().filter(|s| s != strip))
            .collect(),
    )
}

/// Whether `strip` ticked into `layout` ([`ticked_in`]) would make it full.
pub fn would_be_full(layout: &SectorLayout, strip: &StripRef, pool: &[Pooled]) -> bool {
    is_full(units(&ticked_in(&layout.strips, strip), pool))
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
    /// An ARRANGE save of no layout: nothing to change.
    Empty,
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
            Refusal::Empty => "EMPTY",
        }
    }
}

/// A tick of `strip` in `layout`: on puts it into the first empty slot,
/// else at the end (it must be in the pool and fit, the empty slots
/// counted), off leaves its slot empty (in the pool or not; [`ticked_out`]).
/// A tick that changes nothing is `Ok(None)`; a change is the document at
/// `rev + 1`.
pub fn tick(
    doc: &SectorsDoc,
    layout: &str,
    strip: &StripRef,
    on: bool,
    pool: &[Pooled],
) -> Result<Option<SectorsDoc>, Refusal> {
    let (_, current) = doc.find(layout).ok_or(Refusal::UnknownLayout)?;
    if current.holds(strip) == on {
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
        target.strips = if on {
            ticked_in(&target.strips, strip)
        } else {
            ticked_out(&target.strips, strip)
        };
    }
    Ok(Some(next))
}

/// Whether `a` and `b` hold the same refs, each as often (the empty slots
/// not counted).
fn same_set(a: &[Slot], b: &[Slot]) -> bool {
    let sorted = |slots: &[Slot]| {
        let mut refs: Vec<&StripRef> = slots.iter().flatten().collect();
        refs.sort();
        refs
    };
    sorted(a) == sorted(b)
}

/// ARRANGE's save: each layout of `orders` takes its new slots (trailing
/// empty slots dropped), as one change at `rev + 1`. It is `Empty` for no
/// layout (nothing to change, no undo step taken), `Stale` unless `rev` is
/// the document's, each layout exists and is named once, and each order
/// holds the layout's own refs (its gaps aside), and `Full` for an order
/// past [`SECTOR_UNITS`] that holds more units than the layout did (a
/// layout already past it by a hand edit can still be reordered).
pub fn orders(
    doc: &SectorsDoc,
    orders: &[SectorOrder],
    rev: u64,
    pool: &[Pooled],
) -> Result<SectorsDoc, Refusal> {
    if orders.is_empty() {
        return Err(Refusal::Empty);
    }
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
        let slots = trim(order.strips.clone());
        if grows_past_full(units(&target.strips, pool), units(&slots, pool)) {
            return Err(Refusal::Full);
        }
        target.strips = slots;
    }
    next.rev += 1;
    Ok(next)
}

/// Whether a layout of `before` units that becomes `after` units is
/// refused FULL: past [`SECTOR_UNITS`] and larger than before.
fn grows_past_full(before: f64, after: f64) -> bool {
    is_full(after) && after > before
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

/// A seeded layout's faders: these groups' faders with their units, each
/// once.
type Seeded = Vec<(StripRef, f64)>;

/// These groups' faders with their units, each once.
fn seed_strips<'a>(groups: impl IntoIterator<Item = &'a Group>) -> Seeded {
    let mut strips: Seeded = Vec::new();
    for control in groups.into_iter().flat_map(|g| &g.controls) {
        if let Some(id) = strip_ref(control)
            && !strips.iter().any(|(s, _)| *s == id)
        {
            strips.push((id, control_units(control)));
        }
    }
    strips
}

/// Where a seeded layout of these faders ends: the most first faders that
/// are not full (all of them when they fit).
fn seed_fit(strips: &[(StripRef, f64)]) -> usize {
    let mut used = 0.0_f64;
    strips
        .iter()
        .position(|(_, units)| {
            used += units;
            is_full(used)
        })
        .unwrap_or(strips.len())
}

/// A seeded layout (its id set later).
fn seed_layout(title: Option<&str>, strips: Seeded) -> SectorLayout {
    SectorLayout {
        id: String::new(),
        title: seed_title(title),
        strips: strips.into_iter().map(|(id, _)| Some(id)).collect(),
    }
}

/// A row's right side: the left side's overflow first, then its own
/// faders, each fader once (several sub-pages can overflow the same
/// one), in layouts of at most [`SECTOR_UNITS`], each of the side's title;
/// none when it holds no fader.
fn seed_right(title: Option<&str>, overflow: Seeded, own: Seeded) -> Vec<SectorLayout> {
    let mut strips: Seeded = Vec::new();
    for (id, units) in overflow.into_iter().chain(own) {
        if !strips.iter().any(|(s, _)| *s == id) {
            strips.push((id, units));
        }
    }
    let mut out = Vec::new();
    while !strips.is_empty() {
        // At least one fader a layout (no fader is past a sector alone), so
        // the loop always ends.
        let rest = strips.split_off(seed_fit(&strips).max(1));
        out.push(seed_layout(title, std::mem::replace(&mut strips, rest)));
    }
    out
}

/// A left layout of these faders: what fits, the rest added to `overflow`.
fn seed_left(title: Option<&str>, mut strips: Seeded, overflow: &mut Seeded) -> SectorLayout {
    let rest = strips.split_off(seed_fit(&strips));
    overflow.extend(rest);
    seed_layout(title, strips)
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
/// row's other groups on the right; a row with no pager cut at
/// [`seed_cut`], its left groups one layout. A left layout keeps at most
/// [`SECTOR_UNITS`]: the faders past it go, in order, to the front of the
/// right side ([`seed_right`]).
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
    let mut overflow: Seeded = Vec::new();
    if let Some(pager) = pager {
        let left = pager
            .pages
            .iter()
            .map(|sub| {
                seed_left(
                    Some(sub.title.as_str()),
                    seed_strips(sub.sections.iter().flat_map(Section::groups)),
                    &mut overflow,
                )
            })
            .collect();
        let right = seed_right(
            first_title(&groups).as_deref(),
            overflow,
            seed_strips(groups.iter().copied()),
        );
        return (left, right);
    }
    let counts: Vec<usize> = groups
        .iter()
        .map(|g| g.controls.iter().filter(|c| strip_ref(c).is_some()).count())
        .collect();
    let (left, right) = groups.split_at(seed_cut(&counts));
    let left = vec![seed_left(
        first_title(left).as_deref(),
        seed_strips(left.iter().copied()),
        &mut overflow,
    )];
    let right = seed_right(
        first_title(right).as_deref(),
        overflow,
        seed_strips(right.iter().copied()),
    );
    (left, right)
}

/// The first document, from the served layout's default page: row 0 up,
/// row 1 down (the rest ignored); a sector left empty gets one empty
/// [`SEED_TITLE`] layout; ids `<sector>-<n>` from 1; revision 1. Every
/// layout holds at most [`SECTOR_UNITS`] (`seed_row`).
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
            list.push(seed_layout(None, Vec::new()));
        }
        for (n, layout) in list.iter_mut().enumerate() {
            layout.id = format!("{}-{}", sector.as_str(), n + 1);
        }
    }
    SectorsDoc { rev: 1, sectors }
}

#[cfg(test)]
mod tests;
