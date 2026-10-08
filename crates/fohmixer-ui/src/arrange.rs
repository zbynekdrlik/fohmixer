//! Where the controls go (#63, the owner's design of 2026-10-08): one
//! control column in the middle of every screen, the page's rows cut in two
//! by it. Each row is a line of its own while every line keeps a long fader
//! (`Metrics::line_min` of height, by the rows' weights); a lower screen
//! shows one line holding the rows in order. A line is a run of cells, each
//! a strip wide (a wide strip 1.1) and one gap apart, cut where its halves
//! are most even. One strip width serves every line: the widest at which
//! each line's halves fit a side (a line with a window: the narrowest, so a
//! shift resizes nothing). A line that does not fit at the narrowest width
//! shows its pinned cells and a window over the others, which the column's
//! arrows move. A pager's sub-pages share one region of slots: a pinned
//! control keeps its slot on every sub-page and the slots are one unit
//! each, so nothing after the region moves when the sub-page changes. A
//! line renders as one keyed list of its cells and its groups' titles
//! (`items`), each on its side, so a cell that changes side or group run
//! moves without being rebuilt. The layout's own order is kept everywhere:
//! which strips sit where is the owner's choice. This module is pure;
//! `pages::surface` measures and renders.

use fohmixer_proto::layout::{Control, Group, Page, Section};

use crate::flow::is_column;

#[cfg(test)]
mod tests;

/// The arrangement's measures (px): keep them equal to the stylesheet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    /// Between two cells (`.line`'s gap).
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
    /// Between two lines (`.body`'s gap).
    pub line_gap: f64,
}

/// The surface's measures.
pub const METRICS: Metrics = Metrics {
    gap: 4.0,
    min: 64.0,
    max: 120.0,
    wide: 1.1,
    block: 2.4,
    line_min: 340.0,
    line_gap: 6.0,
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
    /// A group of buttons or texts, as one block `units` strips wide.
    Block { group: usize, units: f64 },
    /// A pager slot the shown sub-page leaves empty.
    Blank { slot: usize },
}

impl Cell {
    /// How many strips wide the cell is.
    pub fn units(&self) -> f64 {
        match self {
            Cell::Control { units, .. } | Cell::Block { units, .. } => *units,
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
            Cell::Block { group, .. } => Some(*group),
            Cell::Blank { .. } => None,
        }
    }

    /// Its key among the page's cells (a keyed list keeps its element).
    pub fn key(&self) -> String {
        match self {
            Cell::Control { at, .. } => format!("c{}-{}", at.group, at.control),
            Cell::Block { group, .. } => format!("b{group}"),
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
/// pager's sub-pages', in document order), its rows and their weights.
#[derive(Debug, Clone, PartialEq)]
pub struct PageModel {
    pub groups: Vec<Group>,
    rows: Vec<Vec<Part>>,
    weights: Vec<f64>,
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
        let weights = page.rows.iter().map(|row| row.weight).collect();
        PageModel {
            groups,
            rows,
            weights,
        }
    }

    /// How many rows the page has.
    pub fn rows(&self) -> usize {
        self.rows.len()
    }

    /// The rows' weights (their shares of the height).
    pub fn weights(&self) -> &[f64] {
        &self.weights
    }

    /// The row that holds the pager, if any.
    pub fn pager_row(&self) -> Option<usize> {
        self.rows
            .iter()
            .position(|parts| parts.iter().any(|p| matches!(p, Part::Pager(_))))
    }

    /// What a line's window is kept for: its rows, and the shown sub-page
    /// when one of them holds the pager (another sub-page shows its window
    /// from the start, coming back finds it where it was; the other lines
    /// keep theirs).
    pub fn line_key(&self, rows: &[usize], sub: Option<usize>) -> LineKey {
        let paged = self.pager_row().is_some_and(|row| rows.contains(&row));
        LineKey {
            rows: rows.to_vec(),
            sub: if paged { sub } else { None },
        }
    }

    /// The control at `at`.
    pub fn control(&self, at: At) -> Option<&Control> {
        self.groups.get(at.group)?.controls.get(at.control)
    }
}

/// A group's cells: each control one (a wide strip `m.wide` units), a
/// group without a column control one block (`m.block` units), an empty
/// group nothing; in a pager's region every cell is one unit, so its slots
/// are all as wide and the region's width never changes.
fn group_cells(group: &Group, index: usize, region: bool, m: &Metrics) -> Vec<Cell> {
    if group.controls.is_empty() {
        return Vec::new();
    }
    if !group.controls.iter().any(is_column) {
        let units = if region { 1.0 } else { m.block };
        return vec![Cell::Block {
            group: index,
            units,
        }];
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

/// The page's rows (their `weights`) as lines in `height` px: each row its
/// own line while every line keeps `m.line_min` of its share (by weight,
/// the gaps between the lines taken first); otherwise one line holding them
/// all.
pub fn lines(weights: &[f64], height: f64, m: &Metrics) -> Vec<Vec<usize>> {
    let rows = weights.len();
    if rows == 0 {
        return Vec::new();
    }
    let room = height - (rows - 1) as f64 * m.line_gap;
    let total: f64 = weights.iter().sum();
    let lightest = weights.iter().copied().fold(f64::INFINITY, f64::min);
    if room * lightest >= m.line_min * total {
        (0..rows).map(|row| vec![row]).collect()
    } else {
        vec![(0..rows).collect()]
    }
}

/// The px `cells` take side by side with strips `width` wide.
pub fn span(cells: &[Cell], width: f64, m: &Metrics) -> f64 {
    let units: f64 = cells.iter().map(|c| c.units()).sum();
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
            let units: f64 = half.iter().map(|c| c.units()).sum();
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

/// A line's window: the first of its other cells shown, how many show
/// (`free`; the arrows' step is at least one), the last first one and how
/// many other cells there are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shift {
    pub offset: usize,
    pub free: usize,
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
    /// and last other cells, of how many (none shown: 0 of how many).
    pub fn label(self) -> String {
        if self.free == 0 {
            return format!("0/{}", self.total);
        }
        let end = (self.offset + self.free).min(self.total);
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
    let tail = (1..=total)
        .rev()
        .find(|&t| fits(&window(&real, total - t, t), side, m))
        .unwrap_or(0);
    let last = total - tail;
    let offset = offset.min(last);
    let free = (1..=total - offset)
        .rev()
        .find(|&f| fits(&window(&real, offset, f), side, m))
        .unwrap_or(0);
    let shift = Shift {
        offset,
        free,
        step: free.max(1),
        last,
        total,
    };
    (window(&real, offset, free), Some(shift))
}

/// What a line's window is kept for (`PageModel::line_key`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LineKey {
    pub rows: Vec<usize>,
    pub sub: Option<usize>,
}

/// One line as arranged: the page rows it holds (and its window's key), its
/// halves and its window.
#[derive(Debug, Clone, PartialEq)]
pub struct LineArrangement {
    pub rows: Vec<usize>,
    pub key: LineKey,
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
/// wide and the lines `height` px high together, each line's window at
/// `offset(its key)`. A block wider than a side at the narrowest strip is
/// narrowed to the side.
pub fn arrange(
    model: &PageModel,
    sub: Option<usize>,
    side: f64,
    height: f64,
    offset: impl Fn(&LineKey) -> usize,
    m: &Metrics,
) -> Arrangement {
    let mut width = m.max;
    let mut out = Vec::new();
    let widest = (side / m.min).max(1.0);
    for rows in lines(model.weights(), height, m) {
        let cells: Vec<Cell> = rows
            .iter()
            .flat_map(|row| row_cells(model, *row, sub, m))
            .map(|cell| match cell {
                Cell::Block { group, units } => Cell::Block {
                    group,
                    units: units.min(widest),
                },
                other => other,
            })
            .collect();
        let key = model.line_key(&rows, sub);
        let offset = offset(&key);
        let (cells, shift) = shown(&cells, side, offset, m);
        let at = split(&cells, m);
        // A line with a window takes the narrowest strip: its window moves,
        // the strips keep their width.
        let line_width = if shift.is_some() {
            m.min
        } else {
            fit(&cells[..at], &cells[at..], side, m)
        };
        width = width.min(line_width);
        out.push(LineArrangement {
            rows,
            key,
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

/// A side of the column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// One item of a line's keyed list: a group's title over its run of cells
/// on one side (`cells` of them, `units` strips wide), or a cell.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Title {
        key: String,
        group: usize,
        side: Side,
        cells: usize,
        units: f64,
    },
    Cell {
        cell: Cell,
        side: Side,
    },
}

impl Item {
    /// Its key in the line's list: a title by its group and how many titles
    /// of that group come before it in the line, a cell by the cell's.
    pub fn key(&self) -> String {
        match self {
            Item::Title { key, .. } => key.clone(),
            Item::Cell { cell, .. } => cell.key(),
        }
    }

    /// The side of the column it sits on.
    pub fn side(&self) -> Side {
        match self {
            Item::Title { side, .. } | Item::Cell { side, .. } => *side,
        }
    }
}

/// A line as one list, the left half's then the right half's: each run of
/// a group's cells after its title (a run of blanks has none).
pub fn items(line: &LineArrangement) -> Vec<Item> {
    let mut out = Vec::new();
    let mut titles: Vec<usize> = Vec::new();
    for (side, half) in [(Side::Left, &line.left), (Side::Right, &line.right)] {
        for run in runs(half) {
            if let Some(group) = run.group {
                let before = titles.iter().filter(|g| **g == group).count();
                titles.push(group);
                out.push(Item::Title {
                    key: format!("t{group}-{before}"),
                    group,
                    side,
                    cells: run.cells.len(),
                    units: run.cells.iter().map(|c| c.units()).sum(),
                });
            }
            out.extend(run.cells.iter().map(|&cell| Item::Cell { cell, side }));
        }
    }
    out
}
