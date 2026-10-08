# The control column (#63): implementation plan

> For agentic workers: implement task by task on `dev`; Tier 0 (no local cargo compilation, CI verifies). Steps use checkboxes.

**Goal:** the surface the owner approved on #63 (mockup `docs/mockups/surface-column-v3.html`, commit 7d05682): one control column in the middle of every screen, the page's rows cut in two by it, pinned strips that never leave the screen, one rule for every screen size, no bar above the faders.

**Architecture:** a pure arrangement module (`arrange.rs`) turns a page, the selected sub-page, the measured side width and height and the shift offsets into lines of cells (left half, right half, strip width, shift state); `PageView` renders it with keyed lists so a sub-page switch or a shift remounts only the cells that change; the column (`ColumnView`) holds the status, the tabs, SOLO ✕, the shift arrows, the rail and the global controls. The top bar, the phone's screen bar and overview bar and `phone.css` go.

**Spec:** `docs/superpowers/specs/2026-09-28-fohmixer-program.md` (F4, F5, F8, F11, F12, D14, amended for #63); the owner's decisions on #63 (comments 6051980546, 6052335220, 6052651015).

## Global constraints

- Public repo: invented names only in fixtures, tests and docs; denylist scan before every push.
- Tier 0: no `cargo build/test/check/clippy`, no `trunk build` locally; `cargo fmt` only.
- The layout's own arrangement is kept: Claude moves no strip and invents no grouping (owner, #63).
- Pinned strips never leave the screen: not on a sub-page switch, not on a shift.
- Nothing above the faders; the strip's name button 30 px, Live's dB under it.
- Every surface tap target owns its touches (`use:owns_touches`), no mouse events (`scripts/check_integrity.py`).
- Zero console errors and warnings in every E2E test.

## The arrangement (decided)

- **Lines:** each page row is its own line while every line keeps `LINE_MIN` = 340 px of height (the body's height / rows); otherwise one line holds all rows in order. Tablet 834 and desktop 720 high: two lines; phone landscape 390: one line; phone portrait 844: two lines.
- **Cells:** a line is a sequence of cells, each one strip wide (a `wide` strip 1.1) with one gap (4 px) between any two cells: no wider gap between groups, so a pager region keeps one width whatever it shows. A group's controls are cells in order; a group with no column control (buttons, texts) is one block cell of `BLOCK` = 2.4 units.
- **The pager region:** as many slots as its largest sub-page has cells. A pinned control of any sub-page keeps its slot on every sub-page (a later pinned control that finds its slot taken takes the next free one); the shown sub-page's other controls fill the free slots in order; the rest are blank cells. Region strips are never wide (one unit each), so the region's width never changes.
- **The split:** the line's cells cut at the index that makes the larger half the smallest (by units); the left half, the column, the right half.
- **The strip width:** shared by every line, the largest at which each line's halves fit the side's width, within 64–120 px.
- **Window mode:** a line whose halves do not fit at 64 px drops its blank cells and shows its pinned cells plus a window over its other cells, as many as fit at 64 px; the column shows ◀ ▶ for it; a step moves the window by its size; the offset is clamped.
- **Subscriptions:** the selected path's controls plus every pinned control of the page's other sub-pages (a pinned strip shown on another sub-page must be live). Cells outside a window stay subscribed (simple, the page's own controls).

## Tasks

1. **Proto `Strip.pinned`** (`fohmixer-proto/src/layout.rs`, tests): `#[serde(default, skip_serializing_if = "is_false")] pub pinned: bool`.
2. **Subscriptions** (`binding.rs`, tests): `visible_controls` adds the pinned controls of the other sub-pages.
3. **`arrange.rs`** (new, tests): `PageModel::of(&Page)`, `Cell`, `row_cells(model, row, sub)`, `lines(rows, height)`, `split(cells)`, `fit_width(lines, side)`, `window(...)`, `arrange(...) -> Arrangement`. Pure, mutation-proof tests for every rule above.
4. **`flow.rs`:** keep `is_column`, `shared_instance`; delete the row/pager shapes, `strip_width`, `overflows`, the overview's and the screen bar's helpers and their tests.
5. **The surface** (`pages/surface.rs`): `Shell` = `.mixer` grid `1fr column 1fr`; `ColumnView` (status, tabs, pager tabs, SOLO ✕, shifts, rail, global); `PageView` measures a side and the height, renders lines' halves with keyed `For` (runs keyed by group and occurrence, cells keyed by group and control); `GroupView` becomes a run (title line over its cells). `DeckView`: the key grid split in two halves left and right of the column.
6. **Remove** `pages/overview.rs`, `phone.css` (index.html, Trunk.toml), its `.cargo/mutants.toml` exclusion.
7. **CSS:** `style.css` (no top bar; the column; lines; runs), `strip.css` (name button 30 px, the readout line under it), `deck.css` (two halves).
8. **E2E:** `column.spec.ts` replaces `phone.spec.ts` (tablet, desktop, phone landscape and portrait; pinned strip keeps its slot on a sub-page switch; window mode arrows; column in the middle); update `pages`, `strip`, `touch-guard`, `deck`, `version`, `counter` and every other spec that reads the top bar or rows.
9. **Docs and rules:** the program spec (F-items), `.claude/rules/ui-rust.md`, `.claude/rules/e2e.md`; the mockups stay.
10. **Ship:** PR, CI green, reviews, merge, bump, deploy to the PC, set `pinned` on the owner's six strips in the PC's `layout.json` (the hub backs up every accepted file), verify live on the tablet and phone sizes.
