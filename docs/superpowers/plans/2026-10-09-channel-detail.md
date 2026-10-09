# Channel detail and the real Pro-Q 4 (#71): implementation plan

> For agentic workers: implement task by task on `dev`; Tier 0 (no local cargo compilation, CI verifies). Steps use checkboxes.

**Goal:** a hold on a strip's ☰ opens that channel over the whole screen (fader, mute, Live's dB, the pan that leaves the strip) — PR D; from there the channel's Pro-Q 4 opens full screen as the live picture of its own editor, touched like the editor itself, locked to the device that opened it — PR E.

**Architecture:** the hold is a pure state machine (`behave/hold.rs`); the detail's open strip is `Nav.detail` (the surface's navigation, so a new layout keeps it while the strip is in it); its subscriptions come from one pure function with the page's (`binding`); the detail reuses the strip's components (mute, dB, scale, meter, fader, pan) with its own stylesheet. PR E adds a hub service that opens and captures a plug-in's editor window and the page's EQ screen; its input path waits for the owner's answer on #71.

**Spec:** `docs/superpowers/specs/2026-09-28-fohmixer-program.md` D17, F11, F27, F28 (D14 and D15 for the strip); #71 (the owner's design and approvals of 2026-10-09; the PC tests and the crash); the approved mockup `docs/mockups/channel-detail-v2.html` at 9f47355.

## Global constraints

- Public repo: invented names only in fixtures, tests and docs; denylist scan before every push.
- Tier 0: no `cargo build/test/check/clippy`, no `trunk build` locally; `cargo fmt` only. Python tests, `ruff` and `npx playwright test --list` run locally; a static render of the stylesheets with Playwright (`PLAYWRIGHT_BROWSERS_PATH=/opt/ms-playwright`, never `playwright install`) is how a CSS size formula is swept.
- `.claude/rules/ui-rust.md` and `.claude/rules/e2e.md` govern the page: every tap target owns its touches (`use:owns_touches`), Pointer Events only (no `click`, no mouse events), every reactive read in a component is `try_*`, a decision is a pure tested function, the glue decides nothing, no text overflows its box at strip widths 64–120 px in Chromium and WebKit, nothing measures text, no looping animation on a lasting state, no blurred shadow on a moving part.
- Zero console errors and warnings in every E2E test; both projects (Chromium and the WebKit iPad).
- Nothing that touches a plug-in's window runs in a running Live before it is proven against Pro-Q in a separate host process (D17).

## PR D: the channel detail (no Pro-Q yet)

### The rules (decided)

- **The strip loses its pan.** `StripView` draws no `PanView`; the strip's grid loses its foot row; `strip_subs` keeps the pan's spec, but a strip on the page does not subscribe it (`StripSubs::shown()` = everything but the pan; `control_subs` uses it), so the hub follows fewer listeners. The fader zone keeps its bottom padding (the cap at −∞ stays off the screen's edge).
- **☰ on the readout line.** The head's second line becomes `.strip-info`: a grid `minmax(0, 1fr) auto`, 22 px high; `.strip-readout` (status light, Live's dB, a mark's word; `pointer-events: none`, centred in the first column) and the button `.strip-menu` (`data-testid="strip-menu"`, `aria-label="Detail kanála (podrž)"`, the text ☰, about 22 × 20 px, `use:owns_touches` with no keys). The dB's font shrinks with the strip width so readout and ☰ fit side by side from 64 to 120 px in both engines (swept with a static render before CI; the E2E `clipped()` check stays green). A conflicted strip's ☰ is inert like the rest of it (`states.css`).
- **The hold** (`behave/hold.rs`, pure, tested at its boundaries): `HOLD_MS = 500.0`; a press records its pointer id and time; `due(now)` is true once the same press has lasted `HOLD_MS` (a one-line helper `held_long(start, now)`, tested at 499.999…/500.0); the press opens the detail once (`opened`); its release before that is a **tap**, after it nothing; a cancel or a lost capture ends it without a tap. The glue: `pointerdown` presses, sets `data-holding="true"` (CSS fills the button in 500 ms with a `transition`, never an animation) and schedules one check 500 ms later (`set_timeout_with_handle`, cleared on release and on cleanup); the check opens the detail if `due`. A tap shows a hint for 1.2 s (`data-hint="true"`, CSS text "podrž ☰ na detail" above the button, no layout change). A finger may slide up to 10 px from where its first move lands (`SLIDE_PX`, `Hold::moved` on `pointermove`; the first move only anchors the press, as a fader's does: the iPad's comes late and 6–15 px away); a slide further ends the press with neither a tap nor an open (a fader grab that lands on ☰ must never open the detail); otherwise nothing but the release ends the press. For `OPEN_GUARD_MS` (400 ms) after a hold opens the detail, its parts take no touch (`data-guard`; counted from the open, `guard_left`, so a new layout's remount does not guard again): a second tap at ☰'s spot would land on its MUTE (review of PR #72).
- **The open strip:** `Nav.detail: RwSignal<Option<Strip>>` (the strip as held). The detail is drawn inside `Shell` (it needs `Settings` and the layout), from `binding::detail_strip(&layout, &strip) -> Option<Strip>`: the layout's strip that is the held one (its label, guard and mark current): a marker strip (a label on an index anchor) by its instance, its anchor's kind and its label, so it follows its marker when the track order changes and never takes another marker at its old index; any other strip, a labelled one bound by name too, by an equal `binding` (reviews of PR #72). `Shell` writes the resolved strip back into `Nav.detail` when it differs (`binding::detail_update`), so `wanted_subs` subscribes its keys; a marker that shows its placeholder label (`#<index + 1>`, no label in its Tuner) carried into a new layout closes, as it has no identity to follow. When a new layout has no such strip, the detail closes (the same write sets `Nav.detail` to `None`). Each step (press, slid, tap, open, close with its why) is an essential flight-recorder event with the strip's keys, listed by the forensics timeline. A new layout re-mounts the detail (as it does the page).
- **Subscriptions:** one pure function `binding::wanted_subs(layout, path, detail: Option<&Strip>)`: `visible_subs` plus, for an open detail, `detail_subs(strip, source)` (the strip's subscriptions with the pan, the pan with Live's display string, `display: true`), each key once. `Surface`'s wanted-set effect calls it with `nav.detail`. Tests: a detail adds exactly its pan (with display) to a page that shows its strip; a detail of a strip on no visible page adds all its subscriptions; no detail = `visible_subs`.
- **The detail's view** (`components/detail.rs`, `detail.css` linked after `strip.css`; the mockup's look): a layer over the whole screen (`data-testid="detail"`, `data-track`, `data-label`, `data-instance`) with:
  - a bar: `← SPÄŤ NA MIX` (`data-testid="detail-exit"`, closes it), the name chip in the track's colour (the strip's `color` slot, as the name button's `--tc`), the group's title and the instance;
  - on the left (the left thumb): `MuteView` with the label `MUTE` and the strip's guard (the detail's look: dark when audible, red while muted), the status light and `DbView` large, and the fader zone (`ScaleView`, `MeterView`, `FaderView` with the strip's targets and `Settings`), as tall as the screen allows;
  - on the right (the right thumb): `PANORÁMA`, `PanView` large (64 px high, the dot 48 px; `pan.rs`'s `DOT` becomes the dot's measured width or a CSS variable — the component must not assume 12 px), Live's display string of the pan, and `STRED` (`data-testid="detail-centre"`: a tap writes panning 0.0 as a final `LiveStore::set`, owning its touches with the pan's key);
  - the middle stays empty in PR D (PR E puts the Pro-Q cards there).
  - TechAlert (F16) still flashes over the detail: the wash must not end up under it (check how `.alert-wash` stacks; the column is `z-index: 10`). An E2E proves it.
- **Phones:** the same detail; under `(max-width: 520px)` the right column goes under the left one's top (as the mockup's portrait), never a horizontal scroll.

### Tasks

1. [x] `behave/hold.rs` + tests (boundaries, a second pointer, cancel, re-press).
2. [x] `binding`: `StripSubs::shown`, `detail_subs`, `detail_strip`, `wanted_subs` + tests; `Surface` uses `wanted_subs`.
3. [x] The strip: no pan, `.strip-info` with `.strip-menu`, the hold glue, the hint; CSS swept 64–120 px in Chromium and WebKit.
4. [x] `Nav.detail`, `DetailView` in `Shell`, `detail.css` (+ `index.html` link, `Trunk.toml` watch), the STRED write, the exit.
5. [x] E2E: `e2e/tests/detail.spec.ts` (a short touch on ☰ shows the hint and opens nothing; a hold opens the detail of that strip with its name; its fader drag reaches the track's volume in SimLive; its mute toggles the track's mute; its pan drag and `STRED` reach the track's panning; `← SPÄŤ NA MIX` returns to the page; a new layout keeps it open; TechAlert flashes over it; strips show no pan; clipped() and console clean); a helper `openDetail(page, track)` in `e2e/tests/support`; the pan tests of `column.spec.ts`, `intent.spec.ts`, `resilience.spec.ts`, `strip.spec.ts`, `touch-guard.spec.ts` move to the detail's pan.
6. [x] Docs: `.claude/rules/ui-rust.md` (the detail, the hold, `wanted_subs`), `.claude/rules/e2e.md` (the helper), the spec's F27 status (the detail without its Pro-Q 4 until PR E).

## PR E: Pro-Q 4 on the surface

The owner's decisions (#71): the real editor as a live picture; touches as **real Windows touch input** with the editor on top on the PC's screen (the cursor may jump during a drag, as with RustDesk); nothing tried in a running Live before it is proven in the isolated host; the FabFilter update postponed until the screen is shown working, so the hub guards against the 4.02 close crash itself. The PC findings are in `.claude/rules/plugin-window.md`; read it first.

### The rules (decided)

- **The model is the Stream Deck's (#52):** a pure state module in the hub (`eq.rs`, like `deck.rs`: who holds which editor, which client views which, the gesture in progress), router glue (`router/eq.rs`), a platform backend behind a trait (`plugwin`: a Windows implementation, and a simulated one for tests and CI), proto messages beside the deck's, and the page's components.
- **Discovery:** `ClientMsg::EqList { strip binding }` → the hub resolves the strip's track as it does a strip (by name or by index) and walks its devices through the script's `get_prop`: a `PluginDevice` whose `class_display_name` is `Pro-Q 4` is one; a `RackDevice`'s `chains` → `devices` are walked too, three levels deep at most. The answer lists each with its LOM path (`live_set tracks N devices D [chains C devices E …]`) and where it sits (`na tracku`, or the rack's and chain's names: `<rack> › <chain>`), and the lock state. A list is read when asked (a detail opening), never followed.
- **Open:** `ClientMsg::EqOpen { instance, path }`.
  - If another client holds that editor, the answer is `locked`.
  - Otherwise the client takes the lock (one editor per client: opening another one closes its previous one first).
  - The hub lists the top-level `Vst3PlugWindow`s, sets `is_editor_open = true`, and takes the new window (the list after minus before, within 3 s; else `failed: no window` and `is_editor_open = false`).
  - It makes the window topmost (`SetWindowPos HWND_TOPMOST`, no move or size), keeping Live's position, and records its `FF_UIWindow` child's client rect, the picture.
- **Capture:** while open, a capture thread copies the child's screen rectangle (BitBlt from the screen DC: the window is on top, so the screen holds it; nothing runs on Live's thread) at up to 25 fps, encodes JPEG (quality about 70, the `jpeg-encoder` crate), skips a frame equal to the last one, and hands the newest to the holder's socket as a **binary** WebSocket message (the newest frame wins, never a queue: a slow tablet gets fewer frames, never old ones). The last frame per editor is kept for the card's picture (`GET /api/eq/picture?instance=&path=`, token, `image/jpeg`; 404 before a first open).
- **Input:** `ClientMsg::EqInput { kind: down|move|up|cancel, x, y }` in the picture's pixels (the page maps its finger into them).
  - The hub clamps to the picture.
  - It checks that the point's root window (`WindowFromPoint` → `GetAncestor(GA_ROOT)`) is the editor; otherwise it injects nothing and ends any contact.
  - It injects touch: `InjectTouchInput` with DOWN `INRANGE|INCONTACT|DOWN`, UPDATE `INRANGE|INCONTACT|UPDATE`, UP `UP` at the last point, cancel `UP|CANCELED`; **never `PRIMARY`**.
  - While a contact rests it re-sends the last point every 100 ms; a contact silent for 2 s ends with a cancel (a page that went away).
  - After the contact ends it puts the cursor back where it was (`SetCursorPos`).
  - One contact at a time per editor.
- **Close:** `ClientMsg::EqClose`, the holder's socket closing, or the holder opening another editor.
  - The hub ends any contact (UP).
  - The **close guard** comes next: it injects a tap on an inert spot of the editor, a point verified in the isolated host (a value text field closes on a click elsewhere; the top bar's empty middle worked; pick a point no control covers and pin it in the rules), then waits 300 ms.
  - It sets `is_editor_open = false`, restores the window's z-order (not topmost), and releases the lock.
  - It never closes while a contact is down.
- **Locks are broadcast** (`ServerMsg::EqLocks`: instance, path, held by this client or another, since), so every detail shows a locked card at once.
- **The simulated backend** (`plugwin::Sim`, the hub's default off Windows and in the e2e harness):
  - It makes a synthetic picture (a fixed size, a frame counter drawn in, so frames differ).
  - It records every input and close.
  - The harness reads them (`/sim/eq`), and SimLive gets plug-in devices: `class_display_name`, `is_editor_open` get/set/observe, a Pro-Q 4 on a track and one in a rack chain, with Python tests.
- **The isolated check before any Live:** a hub CLI `fohmixer-hub eq-probe --pid <host pid>` drives the Windows backend against a window of the given process (the Carla bridge): capture N frames (their size and rate), a scripted drag, a double-click on a band, the close guard and a close. The PC run against Carla is part of the PR's verification, recorded on #71, before the first real open in Live.
- **The page:**
  - **The detail's middle:** one card per listed Pro-Q 4. A card shows:
    - its picture: the last frame, fetched with the token into a blob URL, or a plain placeholder before a first open;
    - where it sits;
    - `OTVORIŤ EQ NA CELÚ OBRAZOVKU`, or `ZAMKNUTÉ` with "Upravuje ho iný zvukár (od HH:MM)" when another client holds it.
  - **The EQ screen** (full screen, over the detail; the approved mockup):
    - the bar: `← SPÄŤ NA KANÁL`, the chip, `Pro-Q 4 · <where>`, the lock mark, `?` with the touch legend;
    - the picture: binary frames into a canvas, through `createImageBitmap`, fitted;
    - one finger maps to `EqInput` (moves at most once per animation frame; the up and cancel always sent; `lostpointercapture` is a cancel);
    - a second finger is ignored;
    - leaving the screen sends `EqClose`.
  - The touch's state machine and the coordinate mapping are pure and tested (`behave/eq.rs`). Every tap target owns its touches.
- **Logging and the flight recorder:**
  - The hub logs each open, close and lock with its reason, and the capture's rate once a minute while open.
  - The page records the EQ's open, close and lock refusals as `detail` events (the existing kind; new `what`s).

### Tasks

1. [x] Proto messages and their tests (serde shapes, binary frame contract documented).
2. [x] Hub pure state `eq.rs` (holders, locks, gestures, the close sequence's steps) with tests; router glue; discovery through the script; logs.
3. [x] `plugwin` trait, the Windows backend (window diff, topmost, BitBlt capture, JPEG, touch injection, cursor restore, the guard), the simulated backend; the `eq-probe` CLI.
4. [x] SimLive plug-in devices, the harness `/sim/eq`, Python tests.
5. [x] The page: cards, the EQ screen, the touch state machine, binary frames; CSS (the mockup's look).
6. [x] E2E (both projects): list, open, frames arrive, a drag reaches the simulated backend at the mapped coordinates, exit closes with the guard first, a second context sees the lock, the console clean.
7. [ ] The isolated check on the PC against Carla (the probe CLI), recorded on #71; only then the first open in the band Live, then the owner's demonstration.
8. [x] Docs: `plugin-window.md` (the inert spot, the backend), `ui-rust.md`, `e2e.md`, `live-script.md`, the spec's F28.
