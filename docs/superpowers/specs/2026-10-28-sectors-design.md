# Fader sectors on the FOH page (#148)

Status: the owner approved mockup `docs/mockups/sectors-v1.html` (commit `1d5e4d0`) on 2026-10-28, and mockup `docs/mockups/sectors-v2.html` (commit `a5373ff`: the slots and the layouts' rename, add and remove in ARRANGE) the same day. The rules in §1 are the owner's (#148: DECIDED 25.10, 27.10 ×3, 28.10). Everything marked *(call)* is the main session's decision, recorded on #148; the owner asked not to be asked about details.

## 1. What the owner decided

- The FOH page has four sectors around the control column: left up (`lu`), left down (`ld`), right up (`ru`) and right down (`rd`).
- Each sector has its own named layouts. A layout is a row of slots, each a fader or empty (*1.24.0, the slot model*: the owner on 1.23.0, 28.10, "ARRANGE does not let me put BAND in the right-up sector at the right edge; the strips are always packed against each other"). An empty slot is empty space one strip wide where a strip would sit. A tap in the column pages one sector only; the other three stay as they are (LV1 banks).
- The column's LAYOUTS block (2 × 2, each cell where its sector is) replaces the pager tabs (STAGE / BAND B / OTHERS). Under it is a quiet `ARRANGE` button.
- In a strip's channel detail (SEL), a LAYOUTS section lists every layout of every sector with a checkbox. A fader shows in every ticked layout. A newly ticked fader goes into the layout's first empty slot, else to its end; an unticked one leaves its slot empty *(call: the others keep their places)*.
- ARRANGE starts a reorder mode: strips are dragged within their sector. Every position a sector can show at the page's strip width is a cell, the empty ones drawn as calm dashed outlines (only in ARRANGE). A strip dropped on an empty cell moves there and leaves its old place empty; dropped on another strip, the two swap. Faders, MUTE, SEL and the column's other buttons are inert. Nothing touches Live. A second tap on ARRANGE saves.
- *(mockup v2, 28.10)* In ARRANGE the layouts are renamed, added and removed in the LAYOUTS block, which stays live there (outside ARRANGE it only pages):
  - each layout tab gets ✎: an inline text field (the iPad's keyboard), at most 8 characters, upper case; Enter or a tap elsewhere keeps the text, a blank field keeps the old title;
  - and ✕: the first tap turns the tab into a red `REMOVE?`, a second tap on it removes the layout, a tap elsewhere cancels; a sector's last layout has no ✕;
  - each sector's cell gets a `+` that adds an empty layout `NEW` (then `NEW 2`, …), shown at once on this device; at most 4 layouts a sector (the `+` is gone at 4);
  - a tap on a layout's name pages its sector in ARRANGE too, so every layout can be edited;
  - everything (the slots, the renames, the added and removed layouts) is saved with the second tap on ARRANGE as one change, one ↶ step; nothing touches Live;
  - a removed layout hides nothing in Live; its strips stay in the other layouts that hold them; a device that showed it falls back to its sector's first.
- The layouts are stored in the hub. Every iPad and phone sees the same layouts. Which layout a sector shows is each device's own choice.
- Other devices do not see an arrangement in progress. They get it all at once when ARRANGE ends.
- The last change (one tick or one ARRANGE) can be undone in one step.
- A full layout cannot be ticked (FULL): 6 units a sector on the iPad (a strip 1, a wide strip 1.1, an empty slot 1).
- Nothing above the faders. The travel stays 322 px on the iPad. One strip width for the whole page, the same in every layout of every sector: paging never narrows or moves anything.
- Left sectors fill from the screen's left edge, right sectors from the column.
- The group's name on a strip's side goes on this page. The column shows the layout's name.
- Unticking a layout never changes Live.

## 2. The model *(call)*

- **A fader's identity** is `StripRef { instance, label }`, the identity the REVERB rule already keeps across layouts and restarts.
  - A marker strip uses its own quoted label. A placeholder label (`#<n>`) or a strip in conflict has no identity: it cannot be placed.
  - A frame strip uses its `label` when it has one, else its anchor's name.
- **The pool** is every strip of the served layout with an identity, on any page or view, each once, in document order.
- **The document** is `sectors.json` in the hub's data folder:
  - `{ "rev": n, "sectors": { "lu": [Layout], "ld": [...], "ru": [...], "rd": [...] } }`;
  - `Layout = { "id", "title", "strips": [StripRef | null] }`: `null` is an empty slot. A file without `null` reads as before. Trailing empty slots are dropped: the hub never writes one, and a hand edit's are dropped when the hub reads it.
  - `validate` sees no strip in an empty slot; the status's `strips` and `unknown` count no empty slot.
  - `validate` refuses a sector the document names with no layout (`"rd": []`): a page's edit could never keep it, so a hand edit like that is refused as a whole (`sectors.error`, the last good document served). The empty document names no sector.
  - A layout's id is unique across the document. Its title is at most 8 characters, the column cell's room. A title the page sets (a new layout's, a rename) is 1 to 8 upper-case characters, no space at either end, no control character (`title_ok`); a seeded or hand-edited title the edit leaves alone is kept whatever its case.
  - A sector holds 1 to 4 layouts (`MAX_LAYOUTS`) after a page's edit; a hand edit's more are kept (and can be reordered or removed), never added to. A layout added on the tablet gets the id `<sector>-r<rev>-<n>`, `rev` the revision its save makes and `n` the edit's n-th add: revisions only grow (an undo and a hand edit make new ones), so an id removed or undone is never used again and a device's pick never names another layout (#148 review).
  - The proto type and its checks live in `fohmixer_proto::sectors`, with `deny_unknown_fields` like the layout.
  - The hub writes it atomically and keeps the version before the last change as `sectors.json.prev` (the undo).
  - The file can also be edited by hand on the PC (since mockup v2 the layouts are renamed, added and removed in ARRANGE). The hub reads it at start and on a change, as it reads `layout.json`.
- **A ref no strip of the pool carries** stays in the document and shows nowhere. A track renamed back shows again.
- **The page.** The sectors replace the rows of the layout's default page while the document holds at least one layout. Every other page, the tag groups and the views stay as they are.
- **The first document (the seed).** With no `sectors.json`, the hub writes one from the served default page, so the owner starts from today's surface, not from an empty page:
  - row 0 is up and row 1 is down;
  - a pager's sub-pages become layouts of that row's left sector, titled with the sub-page's title;
  - the row's other groups become one layout of its right sector;
  - the seed never makes an empty slot;
  - *(1.24.0, found on the PC)* a seeded layout keeps at most `SECTOR_UNITS`: the faders past it go, in order, to the front of the same row's right sector, each fader once; a right side past 6 units continues in another layout of the same title;
  - a row with no pager cuts its groups where the strip count halves (groups whole), left and right, one layout each, titled with the first group's title, else `MAIN`;
  - a sector the seed leaves empty gets one empty layout `MAIN`.
  - The seed is a pure function with its own tests. After the seed, only the pages change the document.

## 3. The protocol *(call)*

- `ServerMsg::Sectors { doc }` goes to every page at attach and after every change.
- `ClientMsg::SectorTick { layout, strip, on }` puts the strip into a layout's first empty slot (else at its end), or empties its slot (trailing empty slots dropped).
  - The hub refuses a tick for an unknown layout, a strip not in the pool, or a full layout (`FULL`: the slots' units after the tick, a wide strip at 1.1 and an empty slot at 1, would pass `SECTOR_UNITS` 6.0).
  - A refusal is an `error` reply. The page's checkbox flashes refused.
- `ClientMsg::SectorEdit { sectors, rev }` is ARRANGE's save (mockup v2; it replaced 1.24.0's `SectorOrders`): every sector's layouts as the page edited them, their slots with `null` for an empty one, renamed, added and removed layouts, at the document's revision. It is one change, so it is one undo step (`fohmixer_proto::sectors::edit`):
  - `stale` unless `rev` is the document's and `sectors` names the document's sectors; a layout the document holds must stay in its sector and keep the same strips (its empty slots not counted); a new layout (an id the document does not hold) starts empty. The page then shows the hub's document again.
  - `FULL` for a layout past `SECTOR_UNITS` that holds more units than it did (ARRANGE never offers such a cell; a layout a hand edit put past it can still be reordered).
  - `invalid` for a sector with no layout or more than 4 (unless it had as many), a new or changed title that is not 1 to 8 upper-case characters, or an id empty or twice.
  - `EMPTY` for an edit that changes nothing (no revision, no undo step); the page sends none then.
  - Trailing empty slots are dropped; a removed layout's strips simply leave it.
- `ClientMsg::SectorUndo` swaps in `sectors.json.prev`. A second undo redoes it.
- Every change is one `sectors` event record with `what` (`tick`, `edit`, `undo`, `seed`, `file`), `client` and `peer`, and one info line. An `edit` record counts the layouts `added`, `removed`, `renamed` and `reordered` (`edit_counts`). The records are warn class, like the other owner actions.
- `/api/status` carries `sectors { rev, layouts, strips, unknown }`. `unknown` is the refs no pool strip carries.

## 4. The surface *(call)*

- **Arrange.** The sectors page is two lines (up, down), each cut by the column at a fixed point: the left sector's cells on the left, the right sector's on the right. `split` is not used there.
  - The strip width is `fit` over the largest layout of each sector side by side: the widest that fits every layout of every sector, its empty slots counted, 64–120 px.
  - An empty slot is a blank cell one strip wide in its place (`.slot.sector-gap`, keyed `<sector>:#<slot>`), so a right sector's strip can sit at the screen's edge with empty space between it and the column.
  - Pinning, blanks and the pager region do not apply.
  - A sector whose largest layout does not fit at 64 px (a phone) gets today's window with ◀ ▶ in the column, per line.
  - On a low screen (one line), the line is left = up then down, right = up then down, windowed as today. A real phone design follows on #148 after the iPad ships.
- **Paging.** Each device remembers a layout id per sector in localStorage (`fohmixer_sectors`). A layout gone falls back to its sector's first.
  - A page switch keeps the strips that stay (a keyed list by `StripRef`), like today's sub-page switch.
- **The column.** The LAYOUTS block shows 2 × 2 cells, each with its sector's selected layout's title. A tap on a cell opens that sector's other layouts in one tap: the cell lists every layout of its sector as small tabs and a tap pages to it (mockup v1). Under the block are `ARRANGE` and `↶` (the undo, dim, enabled while the hub holds a previous version).
  - In ARRANGE (mockup v2) the block shows the edit: under each tab ✎ and ✕ (24 px keys, dim text, none filled; no ✕ on a sector's last layout), the cell's `+` (a dashed outline) while it holds fewer than 4 layouts, and the tabs a little lower (6 px padding), so 4 layouts with their keys still fit the column at 1194×834 (the rail shrinks, the faders keep 322 px). A `REMOVE?` tab has red text and a thin red edge, no fill. The rename field is a real `<input>` lying over its tab, unseen and untouchable until ✎, which fills it, focuses it and selects its text within the tap (the iPad brings its keyboard up only for a field focused in a touch); a down anywhere else (a capture-phase listener) keeps its text and drops a pending `REMOVE?` before that down's own handler. The edit's decisions are pure, `behave/arrange_edit.rs` (`Edit`, `TabRow`).
- **The detail.** `detail-right` gets a `LAYOUTS` section after the fader link: four groups (LU, LD, RU, RD), each a checkbox per layout with its title. A full layout that does not hold the strip reads `FULL` and is disabled.
  - The detail's bar gets ◀ ▶: they step to the previous and next strip of the pool, so a strip in no layout (a new track) can still be opened and placed.
- **ARRANGE.**
  - It is a UI mode: `Nav.arrange`. While it is on, each strip is covered by a layer with a ⠿ handle, which owns its touches. The column's other controls are dimmed and inert.
  - While it is on, a one-sector side without a window also shows, after its layout's slots, every further position that fits the side at the page's strip width while the sector stays within `SECTOR_UNITS` (each an empty cell); the strip width never depends on them. A side of two sectors (a low screen) or with a window adds none (the phone design follows).
  - The empty cells are calm dashed outlines and take no touch; outside ARRANGE an empty slot is plain empty space.
  - A drag moves a strip within its sector's shown layout (the edit's, a page in ARRANGE included): it lands on the cell whose centre is nearest the lift. An empty cell takes it and its old slot is left empty; another strip swaps with it. The page keeps the edited slots locally and the save sends them with their `null`s.
  - The second tap sends `SectorEdit` (a rename still in its field kept) and leaves the mode once the hub's new document arrives. A refusal shows the hub's document and a short note by the hub's word (`refusal_note`): `stale` "Not saved: the layouts changed", `FULL` "Not saved: a layout is full", `invalid` "Not saved: the layouts are not valid", the page not connected "Not saved: the hub is not connected", any other "Not saved". The note while on is `drag · ✎ ✕ + · tap to save`.
  - A hide of the page, or a hub's new document during the mode, ends it without saving.
  - The touch decisions (pick, move, the drop's slot, the swap) are pure functions in `behave/arrange_drag.rs` (*(call)*).
- **The group name** on the strip's side is not drawn on the sectors page.

## 5. Out of scope here (recorded on #148)

- The whole-screen scene.
- The phones' own design.

## 6. Tests

- **proto:** `sectors` parse and checks (ids, title length, unknown fields), `StripRef` of a strip, pool order, `units` and full, the seed on the synthetic fixture and on a tagged frame.
- **hub:** the store (load, seed, atomic write, prev, undo/redo, file change), every refusal, the records, `/api/status`, and the attach message (`tests/it/sectors.rs`, host-free).
- **UI native:** the sector arrangement (`arrange`), the width over every layout, paging memory and fallback, the drag decisions, the edit's decisions (the field, rename, add, remove and its ask, new ids and titles, the tabs), and the detail's FULL.
- **E2E (`e2e/tests/sectors.spec.ts`, both projects):**
  - paging one sector leaves the other three, and keeps the strip width;
  - a tick in SEL shows the strip in that layout, and a second page sees it;
  - FULL is refused;
  - ARRANGE reorders, saves, a second page sees it, and nothing reaches Live;
  - ARRANGE puts a strip at the right edge with a gap (a second page shows it there with empty space before it), and swaps a strip dropped on another;
  - ARRANGE renames (Enter, a tap elsewhere), adds (shown at once, the `+` gone at 4, the column fitting) and removes (`REMOVE?` and its cancel) layouts, a sector's last layout has no ✕, it all saves as one `edit` with a second page seeing it and nothing reaching Live, one ↶ restores it, and a layout another page showed falls back to its sector's first;
  - undo;
  - the travel is 322 px at 1194×834;
  - console clean.
  - The existing specs keep today's FOH page. The e2e harness writes an empty `sectors.json` (`{"rev": 0, "sectors": {}}`, sectors off) before the hub starts, so no seed runs. `sectors.spec.ts` sets its own document through a harness route `POST /hub/sectors` (file write, as `/hub/layout`) and restores the empty one at its end.
