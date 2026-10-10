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

## PR F: pinch zoom on the Pro-Q 4 screen (page only)

The owner approved the mockup `docs/mockups/proq-zoom-v1.html` as it is (#71, 2026-10-10): Pro-Q 4's picture was too small on a phone, and a tablet's band points are about 2 mm. The page already maps a finger to the picture's pixels, so zoom is the page's own view of the picture: the hub, the protocol and the PC are unchanged, and the other engineer and the PC's screen never see it.

### The rules (decided)

- **The view** (`behave/eq.rs`, pure, tested): `View { zoom, pan }`, the zoom 1 to `MAX_ZOOM` (4) over the fitted picture, the pan the zoomed picture's corner in the area. `placed` gives its `Fit`: the fitted scale × the zoom, the corner kept so the picture covers the area where it is larger and centred where it is smaller (`place`, written with `clamp`/`min`/`max` so no comparison has an equivalent mutant). `to_picture` maps through it unchanged. `Viewer::follow` (once a frame): two fingers new to it start a pinch (the picture point under their midpoint, their distance, at least 1 px, and the zoom); the same two move it (`zoom = start × distance / start distance`, the start's point under the midpoint as far as the clamp allows); none end it, and a pinch that ends under `ZOOMED_FROM` (1.05) goes back to the whole picture, so the bar's group shows exactly when the factor reads `1,1×` or more. `Viewer::whole` is `CELÝ EQ`. Each open starts at 1× (`Viewer::START`, the screen's own state).
- **The fingers** (`Finger`, stages): a first finger waits `HOLD_MS` (120 ms) or until it moved more than `SLOP` (6 px, on the area) before its down goes, at its first point, then its move to where it is; a lift meanwhile sends the down and the up together (a tap); a first finger off the picture never reaches the editor (it can still pinch). A second finger while the first waits makes a pinch and nothing goes; while the first is on the editor it gets its cancel at its last point, then the pinch. While pinching nothing goes; one finger lifted leaves the pinch (no pair, nothing sent), a finger again pairs again; the pinch ends when every finger lifted. The hold-back is checked on the screen's frame loop (`Finger::frame(now)`), never a timer per finger. A third finger, or a pointer already down, is nothing.
- **The drawing** (`look`, pure): the canvas keeps `object-fit: contain` and gets a CSS transform (`translate(…) scale(zoom)` from `transform-origin: 0 0`) that lands the fitted picture's corner where the view places it; the frame loop writes it, the overview's frame (`seen`: the part in sight as fractions of the picture) and the bar's signals only when the look changed, so nothing is restyled while the view rests. The area's box (`Grip`) is read at each down and on `resize`, never per move or frame.
- **The bar** (only while zoomed, `data-shown`): the overview (`.eq-overview`, the bar's height at the picture's 67:40, the latest frame drawn small into its own canvas, the frame around the part in sight), the factor (`2,5×`, a decimal comma) and `CELÝ EQ` (`use:owns_touches`, Pointer Events only). Upright on a phone the group is tighter and the chip and place step aside while it shows; on a low screen the legend is tighter. The legend (`?`) gets the two-finger lines and says the PC's picture never changes.
- **Tests:** unit tests of the view, the clamp, the pinch, the look and the finger stages (wait, slop at 6.0 and the next float, the hold at 120.0 and the float before, tap, off the picture, pinch from a wait and from a contact with its cancel, one finger left, a third finger); E2E in both projects with dispatched two-pointer events: a pinch zooms (`2,5×`, the point under the midpoint kept, the overview's frame), sends the backend nothing; a drag while zoomed reaches the backend at the picture's pixels under the finger; a quick tap sends its down and its up; `CELÝ EQ` and a new open show the whole picture; the bar fits 844×390 and 390×844; a turned screen redraws the zoomed picture (a tap there lands where it shows).

### Tasks

1. [x] `behave/eq.rs`: `View`, `placed`, `Viewer`, `look`, `zoomed`, `zoom_text`; `Finger` stages; tests.
2. [x] `components/eq.rs`: the view through `Grip` and `point`, the frame loop's hold-back and look, `CELÝ EQ`, the overview canvas; `eq.css`; `.cargo/mutants.toml` (the new glue).
3. [x] E2E: `eq.spec.ts` (both projects).
4. [x] Docs: this section, `.claude/rules/ui-rust.md`, `.claude/rules/plugin-window.md`, `.claude/rules/e2e.md`, the spec's F28.

## PR G: the editor takes the shape of the device that opens it

The owner asked for Pro-Q 4 in the aspect ratio of the device that opens it (#71, 2026-10-10): on a phone the picture was a small rectangle in the middle. FabFilter's VST3 editor resizes freely by its window: in Carla's bridge on the PC, `SetWindowPos` of the host window to 766 × 1300 gave a 760 × 1271 client and Pro-Q 4 laid itself out upright; the old size back gave 1349 × 809 again. Live's editor window is a `Vst3PlugWindow` of 1365 × 848 around the `FF_UIWindow` picture at (8, 31), 1349 × 809; the PC's screen is 2560 × 1440. The inert spot (546, 15) is known only at the original size (at 760 px wide, 0.405 × the width lands on Pro-Q's Undo button). The design comment on #71 is binding.

### The rules (decided)

- **Protocol** (`fohmixer-proto`): `eq_open` carries the page's picture area `area: {w, h}` (CSS px, `eq::Area`; absent: the editor keeps its own size). `eq_area {w, h}` is the area changed while the page holds an open editor. The hub sends no new message: the page sizes the picture from its frames (`Painter::size`, `data-width` / `data-height`).
- **The size** (`eq/size.rs`, pure, tested with exact numbers): `editor_size(area, room, min)` is the area's aspect at Pro-Q 4's pixel count at 100 % (`PIXELS` = 1349 × 809), scaled down to fit `room`, each side at least `min` (a side the minimum holds up bends the aspect); an area with no shape (a side 0, below 0, not a number, an aspect past any number) is none: the editor keeps its size. An upright phone (0.49) is 667 × 1361 in the PC's room, a tablet on its side (1.53) 1293 × 844, a phone on its side (2.5) 1652 × 661.
- **The backend** (`plugwin::Backend`): `room(taken)` (the room for the picture from where the window stands to the work area's far edges, less the frame: `plugwin::room`; the Windows backend reads the monitor's `rcWork` and `GetWindowRect`), `min_size()` (`MIN_PICTURE`, 600 × 400, until the PC check reads Pro-Q's own), `resize(taken, client)` (the window's size for that picture, `window_size`: the frame beyond the picture measured at the take, `Taken::rect`; posted with `SWP_ASYNCWINDOWPOS | SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE`) and `client(taken)` (the picture child's size, `GetClientRect`: no message). The simulated backend's windows have a size (Live's frame around the picture, in the corner of a 2560 × 1400 work area), record `{"op": "resize", "window", "w", "h"}`, never go below `SIM_MIN` (320 × 240, standing in for Pro-Q's minimum), and a test can make a resize late (`resize_late`) or refused (`resize_refused`), or resize a window as the PC would (`resize_window`).
- **The worker:** `Command::Resize` computes the size, posts it and reads the picture's size each step until it lands within `RESIZE_SLACK` (2 px) or `RESIZE_MS` (1 s) passed (`settled`); its caller hears `Resized { asked, client, ok, ms }`. A newer resize replaces one on its way. The capture goes on and the frames carry the new size; a grab of another size with no resize on its way (a late landing, a window resized on the PC) is `PlugwinEvent::Sized`. **The guard puts the editor's own size back first:** an editor whose size was changed waits for a resize on its way to settle (posted sizes land in order), posts the original size, reads it back each step, and taps the inert spot of the original width only once it is back (`restore_step`); not back within `RESIZE_MS`, the guard fails and taps nothing (the router leaves the editor open: `left open in Live`). A resize is refused while its guard runs. The release posts the original size again when it was changed (best effort, not waited for: a failed guard, the stop).
- **Router and state:** each client's area is the state's (`Eqs::keep_area` from the open, `Eqs::area` from `eq_area`); `Act::Open` carries it, and the open sequence resizes the taken editor **before the open answers** (ruling: the first frame is the new picture, and the page never sees the editor's own shape first; an open takes one more worker step or so). A changed area of an open editor is `Act::Resize`, carried out in a task (`RouterMsg::EqResized`). The state's picture size, which the points are clamped to, follows each settled resize and each `Sized` (`Eqs::sized`). Records: `eq` `resize` (`w`, `h` asked, `client_w`, `client_h`, `ok`, `ms`) and `sized` (`w`, `h`).
- **The page:** the screen marks its editor opening at mount and sends `eq_open` with the area once the area is in the page (`raf::animate`'s load: `Grip::of`), so the open carries the measured box. `behave::eq::AreaWatch` (pure) looks at the area each frame (the grip, read at each down and on `resize`): a change of a pixel or more goes as `eq_area` once it rested `AREA_SETTLE_MS` (300 ms) while the editor is open; a change back before it rested sends nothing.
- **Tap spacing:** a quick tap (a lift while the first finger waits) sends its down at once and its up on the first frame `TAP_MS` (40 ms) after it (`Finger::tap`, the frame loop, no timer), so the PC never gets a contact of no time; a finger reaching the editor, a second tap, leaving the screen or a hidden page sends a tap's up still to go first.
- **`eq-probe`:** `--size WxH` resizes after the take (`resize=ok client=WxH ms=…`, `after-resize.jpg`; one that does not land fails the probe), the gestures then run at the new size (`--band` / `--to` scaled, `probe::scaled`), and the guard puts the size back first (`restore=ok client=… ms=…`; not back: exit 1, no tap). `--min-probe` asks 160 × 120 and prints the size the editor took after the wait (`min=WxH asked=160x120`: Pro-Q's minimum), then the size back (`min_restore=…`). A step that fails posts a changed size back before the release.
- **Tests:** the size's exact numbers; the worker's resize (lands, settles unlanded after 1 s, replaced, a grab of another size), the guard's restore (posted, then the tap; waiting for a resize on its way; failing without a tap; a window lost meanwhile), the release's and the stop's restore, the handle through the worker thread; the state's area and size; the router's records and clamping; the hub (`tests/eq.rs`: an open with an upright area answers 667 × 1361, a turn resizes again, the close's records put the size back before the guard's tap); the probe (unit and CLI); the page's `AreaWatch` and the tap's 40 ms; E2E in both projects (the open carries its area, the sim's resize has its aspect, the frames are that size, the close restores before the tap, a turned viewport sends a new area and the editor follows, a quick tap's up comes 20 ms or more after its down).

### Tasks

1. [x] Protocol: `Area`, `eq_open`'s `area`, `eq_area`; wire tests.
2. [x] Hub: `eq/size.rs`; the backend's `room` / `resize` / `client` and `Taken::rect` (`win.rs`, `sim.rs`); the worker's resize, `Sized`, the guard's restore and the release's; the state's areas and size; the router's open with its resize, `eq_area`, the records; `eq-probe --size` and `--min-probe`; tests.
3. [x] Page: the open with its area at load, `AreaWatch` and `eq_area`, the tap's up 40 ms later; `.cargo/mutants.toml` (the glue).
4. [x] E2E: `eq.spec.ts` (both projects).
5. [ ] The PC check against Carla's bridge with the new build: `eq-probe --pid <bridge> --min-probe --size 760x1271 --out <folder outside the repo>` (the `min=` line is Pro-Q's minimum: `MIN_PICTURE` follows it; `after-resize.jpg` shows Pro-Q upright, `field-open.jpg` the field open at the new size, `after-guard.jpg` the field closed at the original size), recorded on #71 without host names or paths; only then an open in the band Live.
6. [x] Docs: this section, `.claude/rules/plugin-window.md`, `.claude/rules/hub-rust.md`, `.claude/rules/ui-rust.md`, `.claude/rules/e2e.md`, the spec's F28.
