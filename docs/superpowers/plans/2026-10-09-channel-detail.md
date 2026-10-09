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
- **The hold** (`behave/hold.rs`, pure, tested at its boundaries): `HOLD_MS = 500.0`; a press records its pointer id and time; `due(now)` is true once the same press has lasted `HOLD_MS` (a one-line helper `held_long(start, now)`, tested at 499.999…/500.0); the press opens the detail once (`opened`); its release before that is a **tap**, after it nothing; a cancel or a lost capture ends it without a tap. The glue: `pointerdown` presses, sets `data-holding="true"` (CSS fills the button in 500 ms with a `transition`, never an animation) and schedules one check 500 ms later (`set_timeout_with_handle`, cleared on release and on cleanup); the check opens the detail if `due`. A tap shows a hint for 1.2 s (`data-hint="true"`, CSS text "podrž ☰ na detail" above the button, no layout change). A finger may slide up to 10 px from its down point (`SLIDE_PX`, `Hold::moved` on `pointermove`); a slide further ends the press with neither a tap nor an open (a fader grab that lands on ☰ must never open the detail); otherwise nothing but the release ends the press. For `OPEN_GUARD_MS` (400 ms) after the detail opens, its parts take no touch (`data-guard`): a second tap at ☰'s spot would land on its MUTE (review of PR #72).
- **The open strip:** `Nav.detail: RwSignal<Option<Strip>>` (the strip as held). The detail is drawn inside `Shell` (it needs `Settings` and the layout), from `binding::detail_strip(&layout, &strip) -> Option<Strip>`: the layout's strip that is the held one (its label, guard and mark current): a marker strip (one with a label) by its instance, its anchor's kind and its label, so it follows its marker when the track order changes and never takes another marker at its old index; any other strip by an equal `binding` (review of PR #72). `Shell` writes the resolved strip back into `Nav.detail` when it differs (`binding::detail_update`), so `wanted_subs` subscribes its keys. When a new layout has no such strip, the detail closes (the same write sets `Nav.detail` to `None`). A new layout re-mounts the detail (as it does the page).
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

## PR E: Pro-Q 4 on the surface (after the owner's answer on #71)

Outline, refined into tasks once the input path is decided:

- The hub finds each Pro-Q 4 of a strip's track (`class_display_name == "Pro-Q 4"`, on the track and in rack chains, nested) and serves them to the detail (instance, device path, where: on the track / rack › chain).
- The hub opens an editor (`is_editor_open = true`), finds its new `Vst3PlugWindow`, captures it (Windows Graphics Capture; PrintWindow only as a fallback: it renders on Live's thread), and streams JPEG frames to the one device holding its lock; it keeps the last frame for the card's picture; it closes the editor when the device leaves the EQ or its connection closes, never while a gesture is in progress.
- The lock per Pro-Q 4 instance: held by one connection; others see it locked.
- The input path, proven first against Pro-Q in a separate host process (Carla portable or REAPER portable with a dedicated plug-in process): per the owner's answer on #71.
- The FabFilter update on the PC (the close crash fixed after 4.02) is asked of the owner separately.
