# Fader sectors on the FOH page (#148)

Status: the owner approved mockup `docs/mockups/sectors-v1.html` (commit `1d5e4d0`) on 2026-10-28. The rules in §1 are the owner's (#148: DECIDED 25.10, 27.10 ×3, 28.10). Everything marked *(call)* is the main session's decision, recorded on #148; the owner asked not to be asked about details.

## 1. What the owner decided

- The FOH page has four sectors around the control column: left up (`lu`), left down (`ld`), right up (`ru`) and right down (`rd`).
- Each sector has its own named layouts. A layout is an ordered list of faders. A tap in the column pages one sector only; the other three stay as they are (LV1 banks).
- The column's LAYOUTS block (2 × 2, each cell where its sector is) replaces the pager tabs (STAGE / BAND B / OTHERS). Under it is a quiet `ARRANGE` button.
- In a strip's channel detail (SEL), a LAYOUTS section lists every layout of every sector with a checkbox. A fader shows in every ticked layout. A newly ticked fader goes to the end of that layout.
- ARRANGE starts a reorder mode: strips are dragged within their sector. Faders, MUTE, SEL and the column's other buttons are inert. Nothing touches Live. A second tap on ARRANGE saves.
- The layouts are stored in the hub. Every iPad and phone sees the same layouts. Which layout a sector shows is each device's own choice.
- Other devices do not see an arrangement in progress. They get it all at once when ARRANGE ends.
- The last change (one tick or one ARRANGE) can be undone in one step.
- A full layout cannot be ticked (FULL): 6 strips a sector on the iPad (a wide strip counts 1.1).
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
  - `Layout = { "id", "title", "strips": [StripRef] }`.
  - A layout's id is unique across the document. Its title is at most 8 characters, the column cell's room.
  - The proto type and its checks live in `fohmixer_proto::sectors`, with `deny_unknown_fields` like the layout.
  - The hub writes it atomically and keeps the version before the last change as `sectors.json.prev` (the undo).
  - Since there is no editor for layout names yet, the file can be edited by hand on the PC. The hub reads it at start and on a change, as it reads `layout.json`.
- **A ref no strip of the pool carries** stays in the document and shows nowhere. A track renamed back shows again.
- **The page.** The sectors replace the rows of the layout's default page while the document holds at least one layout. Every other page, the tag groups and the views stay as they are.
- **The first document (the seed).** With no `sectors.json`, the hub writes one from the served default page, so the owner starts from today's surface, not from an empty page:
  - row 0 is up and row 1 is down;
  - a pager's sub-pages become layouts of that row's left sector, titled with the sub-page's title;
  - the row's other groups become one layout of its right sector;
  - a row with no pager cuts its groups where the strip count halves (groups whole), left and right, one layout each, titled with the first group's title, else `MAIN`;
  - a sector the seed leaves empty gets one empty layout `MAIN`.
  - The seed is a pure function with its own tests. After the seed, only the pages change the document.

## 3. The protocol *(call)*

- `ServerMsg::Sectors { doc }` goes to every page at attach and after every change.
- `ClientMsg::SectorTick { layout, strip, on }` adds the strip to a layout's end, or removes it.
  - The hub refuses a tick for an unknown layout, a strip not in the pool, or a full layout (`FULL`: the strips' units, with a wide strip at 1.1, would pass `SECTOR_UNITS` 6.0).
  - A refusal is an `error` reply. The page's checkbox flashes refused.
- `ClientMsg::SectorOrder { layout, strips, rev }` is ARRANGE's save, one per layout the drag changed.
  - It is taken only when `strips` is the same set as the layout's strips and `rev` is the document's revision. Else it is refused as `stale`, and the page shows the hub's order.
  - Several layouts changed in one ARRANGE go as one `SectorOrders { orders, rev }`. It is one change, so it is one undo step.
- `ClientMsg::SectorUndo` swaps in `sectors.json.prev`. A second undo redoes it.
- Every change is one `sectors` event record with `what` (`tick`, `order`, `undo`, `seed`, `file`), `client` and `peer`, and one info line. The records are warn class, like the other owner actions.
- `/api/status` carries `sectors { rev, layouts, strips, unknown }`. `unknown` is the refs no pool strip carries.

## 4. The surface *(call)*

- **Arrange.** The sectors page is two lines (up, down), each cut by the column at a fixed point: the left sector's cells on the left, the right sector's on the right. `split` is not used there.
  - The strip width is `fit` over the largest layout of each sector side by side: the widest that fits every layout of every sector, 64–120 px.
  - Pinning, blanks and the pager region do not apply.
  - A sector whose largest layout does not fit at 64 px (a phone) gets today's window with ◀ ▶ in the column, per line.
  - On a low screen (one line), the line is left = up then down, right = up then down, windowed as today. A real phone design follows on #148 after the iPad ships.
- **Paging.** Each device remembers a layout id per sector in localStorage (`fohmixer_sectors`). A layout gone falls back to its sector's first.
  - A page switch keeps the strips that stay (a keyed list by `StripRef`), like today's sub-page switch.
- **The column.** The LAYOUTS block shows 2 × 2 cells, each with its sector's selected layout's title. A tap on a cell opens that sector's other layouts in one tap: the cell lists every layout of its sector as small tabs and a tap pages to it (mockup v1). Under the block are `ARRANGE` and `↶` (the undo, dim, enabled while the hub holds a previous version).
- **The detail.** `detail-right` gets a `LAYOUTS` section after the fader link: four groups (LU, LD, RU, RD), each a checkbox per layout with its title. A full layout that does not hold the strip reads `FULL` and is disabled.
  - The detail's bar gets ◀ ▶: they step to the previous and next strip of the pool, so a strip in no layout (a new track) can still be opened and placed.
- **ARRANGE.**
  - It is a UI mode: `Nav.arrange`. While it is on, each strip is covered by a layer with a ⠿ handle, which owns its touches. The column's other controls are dimmed and inert.
  - A drag moves a strip within its sector's shown layout. The page keeps the edited orders locally.
  - The second tap sends `SectorOrders` and leaves the mode once the hub answers. A refusal shows the hub's order and a short note.
  - A hide of the page, or a hub's new document during the mode, ends it without saving.
  - The touch decisions (pick, move, drop index) are pure functions in `behave/arrange_drag.rs` (*(call)*).
- **The group name** on the strip's side is not drawn on the sectors page.

## 5. Out of scope here (recorded on #148)

- Adding, renaming and deleting layouts on the tablet: hand edit of `sectors.json` for now.
- The whole-screen scene.
- The phones' own design.

## 6. Tests

- **proto:** `sectors` parse and checks (ids, title length, unknown fields), `StripRef` of a strip, pool order, `units` and full, the seed on the synthetic fixture and on a tagged frame.
- **hub:** the store (load, seed, atomic write, prev, undo/redo, file change), every refusal, the records, `/api/status`, and the attach message (`tests/it/sectors.rs`, host-free).
- **UI native:** the sector arrangement (`arrange`), the width over every layout, paging memory and fallback, the drag decisions, and the detail's FULL.
- **E2E (`e2e/tests/sectors.spec.ts`, both projects):**
  - paging one sector leaves the other three, and keeps the strip width;
  - a tick in SEL shows the strip in that layout, and a second page sees it;
  - FULL is refused;
  - ARRANGE reorders, saves, a second page sees it, and nothing reaches Live;
  - undo;
  - the travel is 322 px at 1194×834;
  - console clean.
  - The existing specs keep today's FOH page. The e2e harness writes an empty `sectors.json` (`{"rev": 0, "sectors": {}}`, sectors off) before the hub starts, so no seed runs. `sectors.spec.ts` sets its own document through a harness route `POST /hub/sectors` (file write, as `/hub/layout`) and restores the empty one at its end.
