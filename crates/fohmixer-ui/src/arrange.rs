//! Where the controls go (#63, the owner's design of 2026-10-08): one
//! control column in the middle of every screen, the page's rows cut in two
//! by it. Each row is a line of its own while every line keeps a long fader
//! (`Metrics::line_min` of height); a lower screen shows one line holding
//! the rows in order. A line is a run of cells, each a strip wide (a wide
//! strip 1.1) and one gap apart, cut where its halves are most even. One
//! strip width serves every line: the widest at which each line's halves
//! fit a side. A line that does not fit at the narrowest width shows its
//! pinned cells and a window over the others, which the column's arrows
//! move. A pager's sub-pages share one region of slots: a pinned control
//! keeps its slot on every sub-page and the slots are one unit each, so
//! nothing after the region moves when the sub-page changes. The layout's
//! own order is kept everywhere: which strips sit where is the owner's
//! choice. This module is pure; `pages::surface` measures and renders.

use fohmixer_proto::layout::{Control, Group, Page, Section};

use crate::flow::is_column;

#[cfg(test)]
mod tests;

/// The arrangement's measures (px): keep them equal to the stylesheet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// Between two cells (`.half` and `.group-body` gaps).
    pub gap: f64,
    /// The strip width's bounds.
    pub min: f64,
    pub max: f64,
    /// A wide strip's width, in strips.
    pub wide: f64,
    /// A group of buttons or texts, in strips.
    pub block: f64,
    /// The least height of a line while each row is a line of its own.
    pub line_min: f64,
}

/// The surface's measures.
pub const METRICS: Metrics = Metrics {
    gap: 4.0,
    min: 64.0,
    max: 120.0,
    wide: 1.1,
    block: 2.4,
    line_min: 340.0,
};

/// A control of the page: its group's index in [`PageModel::groups`] and
/// its own index in the group.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct At {
    pub group: usize,
    pub control: usize,
}

/// One place of a line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cell {
    /// A control, `units` strips wide; `pinned` never leaves the screen.
    Control { at: At, units: f64, pinned: bool },
    /// A group of buttons or texts, as one block.
    Block { group: usize },
    /// A pager slot the shown sub-page leaves empty.
    Blank { slot: usize },
}

impl Cell {
    /// How many strips wide the cell is.
    pub fn units(&self, m: &Metrics) -> f64 {
        match self {
            Cell::Control { units, .. } => *units,
            Cell::Block { .. } => m.block,
            Cell::Blank { .. } => 1.0,
        }
    }

    /// Whether it never leaves the screen.
    pub fn pinned(&self) -> bool {
        matches!(self, Cell::Control { pinned: true, .. })
    }

    /// The group it belongs to (a blank: none).
    pub fn group(&self) -> Option<usize> {
        match self {
            Cell::Control { at, .. } => Some(at.group),
            Cell::Block { group } => Some(*group),
            Cell::Blank { .. } => None,
        }
    }

    /// Its key among the page's cells (a keyed list keeps its element).
    pub fn key(&self) -> String {
        match self {
            Cell::Control { at, .. } => format!("c{}-{}", at.group, at.control),
            Cell::Block { group } => format!("b{group}"),
            Cell::Blank { slot } => format!("s{slot}"),
        }
    }
}

/// What a row holds, in order: a group, or the pager (per sub-page, its
/// groups).
#[derive(Debug, Clone, PartialEq)]
enum Part {
    Group(usize),
    Pager(Vec<Vec<usize>>),
}

/// A page as the arrangement reads it: every group once (the rows' and the
/// pager's sub-pages', in document order) and its rows.
#[derive(Debug, Clone, PartialEq)]
pub struct PageModel {
    pub groups: Vec<Group>,
    rows: Vec<Vec<Part>>,
}

impl PageModel {
    pub fn of(page: &Page) -> Self {
        let mut groups: Vec<Group> = Vec::new();
        let mut rows = Vec::new();
        for row in &page.rows {
            let mut parts = Vec::new();
            for section in &row.sections {
                match section {
                    Section::Group(group) => {
                        parts.push(Part::Group(groups.len()));
                        groups.push(group.clone());
                    }
                    Section::Pager(pager) => {
                        let mut subs = Vec::new();
                        for sub in &pager.pages {
                            let mut indices = Vec::new();
                            for group in sub.sections.iter().flat_map(Section::groups) {
                                indices.push(groups.len());
                                groups.push(group.clone());
                            }
                            subs.push(indices);
                        }
                        parts.push(Part::Pager(subs));
                    }
                }
            }
            rows.push(parts);
        }
        PageModel { groups, rows }
    }

    /// How many rows the page has.
    pub fn rows(&self) -> usize {
        self.rows.len()
    }

    /// The control at `at`.
    pub fn control(&self, at: At) -> Option<&Control> {
        self.groups.get(at.group)?.controls.get(at.control)
    }
}

/// A group's cells: each control one (a wide strip `m.wide` units, but one
/// unit in a pager's region, whose slots all are); a group without a column
/// control is one block; an empty group nothing.
fn group_cells(group: &Group, index: usize, region: bool, m: &Metrics) -> Vec<Cell> {
    if group.controls.is_empty() {
        return Vec::new();
    }
    if !group.controls.iter().any(is_column) {
        return vec![Cell::Block { group: index }];
    }
    group
        .controls
        .iter()
        .enumerate()
        .map(|(control, c)| {
            let wide = !region && matches!(c, Control::Strip(s) if s.wide);
            Cell::Control {
                at: At {
                    group: index,
                    control,
                },
                units: if wide { m.wide } else { 1.0 },
                pinned: c.pinned(),
            }
        })
        .collect()
}

/// A pager's region on sub-page `sub`: as many slots as the largest
/// sub-page's other controls and every pinned control together; each pinned
/// control in the slot of its place in its sub-page (the next free one when
/// that is taken), the shown sub-page's other controls in the free slots in
/// order, blanks in the rest.
fn region(model: &PageModel, subs: &[Vec<usize>], sub: Option<usize>, m: &Metrics) -> Vec<Cell> {
    let lists: Vec<Vec<Cell>> = subs
        .iter()
        .map(|groups| {
            groups
                .iter()
                .flat_map(|g| group_cells(&model.groups[*g], *g, true, m))
                .collect()
        })
        .collect();
    let pinned = lists.iter().flatten().filter(|c| c.pinned()).count();
    let others = |list: &Vec<Cell>| list.iter().filter(|c| !c.pinned()).count();
    let size = lists.iter().map(others).max().unwrap_or(0) + pinned;
    let mut slots: Vec<Option<Cell>> = vec![None; size];
    for list in &lists {
        for (place, cell) in list.iter().enumerate().filter(|(_, c)| c.pinned()) {
            if let Some(free) = (place..size).chain(0..place).find(|&s| slots[s].is_none()) {
                slots[free] = Some(*cell);
            }
        }
    }
    let mut shown = sub
        .and_then(|s| lists.get(s))
        .into_iter()
        .flatten()
        .filter(|c| !c.pinned());
    for slot in slots.iter_mut().filter(|s| s.is_none()) {
        *slot = shown.next().copied();
    }
    slots
        .into_iter()
        .enumerate()
        .map(|(slot, cell)| cell.unwrap_or(Cell::Blank { slot }))
        .collect()
}

/// Row `row`'s cells with the pager on sub-page `sub`, in order.
pub fn row_cells(model: &PageModel, row: usize, sub: Option<usize>, m: &Metrics) -> Vec<Cell> {
    let Some(parts) = model.rows.get(row) else {
        return Vec::new();
    };
    parts
        .iter()
        .flat_map(|part| match part {
            Part::Group(g) => group_cells(&model.groups[*g], *g, false, m),
            Part::Pager(subs) => region(model, subs, sub, m),
        })
        .collect()
}

/// The page's `rows` as lines in `height` px: each row its own line while
/// every line keeps `m.line_min`; otherwise one line holding them all.
pub fn lines(rows: usize, height: f64, m: &Metrics) -> Vec<Vec<usize>> {
    if rows == 0 {
        Vec::new()
    } else if height >= m.line_min * rows as f64 {
        (0..rows).map(|row| vec![row]).collect()
    } else {
        vec![(0..rows).collect()]
    }
}

/// The px `cells` take side by side with strips `width` wide.
pub fn span(cells: &[Cell], width: f64, m: &Metrics) -> f64 {
    let units: f64 = cells.iter().map(|c| c.units(m)).sum();
    units * width + cells.len().saturating_sub(1) as f64 * m.gap
}

/// The wider half of `cells` cut at `at`, at the narrowest strip.
fn wider_half(cells: &[Cell], at: usize, m: &Metrics) -> f64 {
    span(&cells[..at], m.min, m).max(span(&cells[at..], m.min, m))
}

/// Where a line's cells are cut: where the wider half is the narrowest (the
/// last such place: an odd cell goes to the left).
pub fn split(cells: &[Cell], m: &Metrics) -> usize {
    (0..=cells.len())
        .rev()
        .min_by(|&a, &b| wider_half(cells, a, m).total_cmp(&wider_half(cells, b, m)))
        .unwrap_or(0)
}

/// The widest strip at which both halves fit a side `side` px wide
/// (unbounded while they hold nothing).
pub fn fit(left: &[Cell], right: &[Cell], side: f64, m: &Metrics) -> f64 {
    [left, right]
        .iter()
        .filter_map(|half| {
            let units: f64 = half.iter().map(|c| c.units(m)).sum();
            (units > 0.0).then(|| (side - half.len().saturating_sub(1) as f64 * m.gap) / units)
        })
        .fold(f64::INFINITY, f64::min)
}

/// Whether `cells`, cut where they are most even, fit a side at the
/// narrowest strip.
fn fits(cells: &[Cell], side: f64, m: &Metrics) -> bool {
    let at = split(cells, m);
    fit(&cells[..at], &cells[at..], side, m) >= m.min
}

/// Every pinned cell and the other cells numbered `offset` to
/// `offset + free` (counting the others only), in order.
fn window(cells: &[Cell], offset: usize, free: usize) -> Vec<Cell> {
    let mut other = 0;
    cells
        .iter()
        .filter(|c| {
            if c.pinned() {
                return true;
            }
            let index = other;
            other += 1;
            (offset..offset + free).contains(&index)
        })
        .copied()
        .collect()
}

/// A line's window: the first of its other cells shown, how many show (the
/// arrows' step), the last first one and how many other cells there are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shift {
    pub offset: usize,
    pub step: usize,
    pub last: usize,
    pub total: usize,
}

impl Shift {
    /// The window's first other cell after an arrow: a step on (`forward`)
    /// or back, within the line.
    pub fn moved(self, forward: bool) -> usize {
        if forward {
            (self.offset + self.step).min(self.last)
        } else {
            self.offset.saturating_sub(self.step)
        }
    }

    /// Where the window stands, for the column: the numbers of its first
    /// and last other cells, of how many.
    pub fn label(self) -> String {
        let end = (self.offset + self.step).min(self.total);
        format!("{}–{end}/{}", self.offset + 1, self.total)
    }
}

/// What a line shows: every cell while its halves fit a side at the
/// narrowest strip; otherwise (its blanks dropped) every pinned cell and a
/// window over the others from `offset`, as many as fit. The last window is
/// as many of the last others as fit, so the last cell can always be
/// reached; `offset` is clamped to its start.
pub fn shown(cells: &[Cell], side: f64, offset: usize, m: &Metrics) -> (Vec<Cell>, Option<Shift>) {
    if fits(cells, side, m) {
        return (cells.to_vec(), None);
    }
    let real: Vec<Cell> = cells
        .iter()
        .filter(|c| !matches!(c, Cell::Blank { .. }))
        .copied()
        .collect();
    let total = real.iter().filter(|c| !c.pinned()).count();
    if total == 0 || fits(&real, side, m) {
        return (real, None);
    }
    let mut tail = total;
    while tail > 0 && !fits(&window(&real, total - tail, tail), side, m) {
        tail -= 1;
    }
    let last = total - tail;
    let offset = offset.min(last);
    let mut free = total - offset;
    while free > 0 && !fits(&window(&real, offset, free), side, m) {
        free -= 1;
    }
    let shift = Shift {
        offset,
        step: free.max(1),
        last,
        total,
    };
    (window(&real, offset, free), Some(shift))
}

/// One line as arranged: the page rows it holds, its halves and its window.
#[derive(Debug, Clone, PartialEq)]
pub struct LineArrangement {
    pub rows: Vec<usize>,
    pub left: Vec<Cell>,
    pub right: Vec<Cell>,
    pub shift: Option<Shift>,
}

/// A page as arranged: the strip width every line shares and the lines.
#[derive(Debug, Clone, PartialEq)]
pub struct Arrangement {
    pub width: f64,
    pub lines: Vec<LineArrangement>,
}

/// The page arranged with its pager on sub-page `sub`, a side `side` px
/// wide and the lines `height` px high together, each line's window at its
/// `offsets` entry (0 when it has none).
pub fn arrange(
    model: &PageModel,
    sub: Option<usize>,
    side: f64,
    height: f64,
    offsets: &[usize],
    m: &Metrics,
) -> Arrangement {
    let mut width = m.max;
    let mut out = Vec::new();
    for (index, rows) in lines(model.rows(), height, m).into_iter().enumerate() {
        let cells: Vec<Cell> = rows
            .iter()
            .flat_map(|row| row_cells(model, *row, sub, m))
            .collect();
        let offset = offsets.get(index).copied().unwrap_or(0);
        let (cells, shift) = shown(&cells, side, offset, m);
        let at = split(&cells, m);
        width = width.min(fit(&cells[..at], &cells[at..], side, m));
        out.push(LineArrangement {
            rows,
            left: cells[..at].to_vec(),
            right: cells[at..].to_vec(),
            shift,
        });
    }
    Arrangement {
        width: width.clamp(m.min, m.max),
        lines: out,
    }
}

/// A half's cells of one group side by side; blanks are a run of their own.
/// Keyed by the group and how many runs of it come before in the half.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub key: String,
    pub group: Option<usize>,
    pub cells: Vec<Cell>,
}

/// A half's runs, in order.
pub fn runs(cells: &[Cell]) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for cell in cells {
        let group = cell.group();
        match out.last_mut() {
            Some(run) if run.group == group => run.cells.push(*cell),
            _ => {
                let before = out.iter().filter(|r| r.group == group).count();
                let key = match group {
                    Some(g) => format!("g{g}-{before}"),
                    None => format!("blank-{before}"),
                };
                out.push(Run {
                    key,
                    group,
                    cells: vec![*cell],
                });
            }
        }
    }
    out
}
