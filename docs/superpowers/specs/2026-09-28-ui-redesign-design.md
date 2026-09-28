# UI redesign: a modern FOH surface with TouchOSC's behaviour (#21)

Status: design approved by the owner on #21 (2026-09-28, the STAGE mockup `docs/mockups/redesign-stage-v1.html`); this note turns it into a buildable design. It supersedes the parts of the program spec and the S4 design note that fix the look to TouchOSC (§8).

## 0. Zhrnutie pre vlastníka

- Nový vzhľad podľa schváleného mockupu na všetkých stránkach (Petka, FOH so Stage / Band B / Others, Conf): tmavé pásy, pri každom fadri dB stupnica a merač, farba pásu z farby tracku v Abletone, meno pásu je mute, funkcie v stĺpci vľavo, sekcie s nadpismi v dvoch riadkoch.
- **Správanie ostáva 1:1 s TouchOSC** (krivka fadera, relatívny ťah, double tap → 0 dB rovnakou rýchlosťou, ochrana mute, pan, STAGE AUT, TechAlert, REFRESH ALL, …) — audit na #21.
- **Pod kapotou:** layout už neopisuje TouchOSC súradnice, ale čo je na stránke: stĺpec funkcií, riadky, sekcie, pásy. Mixér si rozloženie spočíta sám a nový ovládač (plugin, EQ, LUFS) sa pridá jedným záznamom do sekcie — bez ťahania súradníc.
- Import z TouchOSC ostáva (nič sa znovu nezadáva); na PC sa layout vyrobí nanovo a prenesú sa noví členovia kapely.

## 1. Goal and scope

**Goal.** Replace the TouchOSC-shaped rendering (scaled 2360×1640 canvas, absolute frames, TouchOSC colours) with the approved design, on every page, while every behaviour the parity audit lists as DONE stays exactly as it is and the audit's PARTIAL/MISSING items are closed.

**Owner decision D13 (2026-09-28, #21):** functionality of TouchOSC, not its look. D3 ("v1 as close as possible to TouchOSC; nothing re-entered") now reads: same pages, sections, strip order and behaviour, nothing re-entered; the look is our own.

**In scope**
- R1 Layout schema 2, semantic (§2); the hub validates and serves it; the import tool writes it (§3).
- R2 A new UI shell and component set (§4) with the design tokens of §4.1.
- R3 Close the audit gaps: dB scale beside every fader, readable meters, the TouchOSC dB readout form, fader travel 1:1 with the finger, pan bar from the centre.
- R4 New, approved in the mockup: Live track colour on the name button, stereo meter with peak hold and a clip light, the SOLO ✕ pill.

**Not in scope:** new controls beyond the mockup (plugins, EQ, LUFS: later, by prompt, D4); a phone layout (the targets are the iPad Pro 11 landscape and the PC browser); light theme.

## 2. Layout schema 2

`layout.json` stays one JSON document, hand-editable, `deny_unknown_fields` everywhere. No frames, no z, no canvas, no TouchOSC colours.

```text
Layout      { schema: 2, default_page, pages: [Page], global: [Control], config, report }
Page        { id, title, rail: [Control], rows: [Row] }          # rail and rows may be empty
Row         { sections: [Section], weight?: f64 (default 1) }     # height share
Section     = group { id?, title?, color?, controls: [Control] }
            | pager { id, default_page, pages: [SubPage] }         # at most one per Page
SubPage     { id, title, sections: [group] }
Control     = strip { binding, strip_kind: standard|return, wide?: bool, mute_guard?: bool }
            | solo { binding, label? } | stage { binding, aut, label? } | hub_toggle { key, label }
            | param_toggle { label, targets, press, color? } | param_fader { label, targets }
            | alert { binding, period_ms, label? } | refresh { label? } | text { text }
```

- `Binding`, `ParamTarget`, `Press`, `Scale`, `LayoutConfig` are unchanged from schema 1 (the binding form is the D2 core).
- `strip_kind`: `narrow` and `solid` were TouchOSC geometry and go; `meter_mute_only` (the TechAlert strip) merges into `alert` (one control: mute toggle of the TechAlert track + the full-screen blink). `wide` marks a bus strip drawn wider (the two top-right strips, returns).
- `color` (`#RRGGBB`) on a group or a toggle is an identity hint (the section marker, a toggle's lit colour); the UI maps it onto its palette. A strip's colour comes from Live (§4.3).
- `global` controls show on every page, in the rail's footer (TechAlert, REFRESH ALL).
- `text` holds the Conf page's configuration text; `ConfInfo` still renders on the page with id `conf`.
- **Validation** (proto `validate`): `schema == 2`; ids unique across pages, sub-pages and sections; every `default_page` names an existing page; at most one `pager` per page; bindings parse; alert `period_ms > 0`; `hub_toggle.key == stage_aut`; targets complete; `color` is `#RRGGBB[AA]`. `bindings()` and `stage_aut_binding()` walk the new tree.
- The hub's load/poll/last-good/backup logic is unchanged; a schema-1 backup no longer validates, so the install ships a schema-2 `layout.json` (§6).

## 3. Import (tools/import-tosc)

The importer keeps reading the TouchOSC project and the set; it now **groups** what it used to place. Grouping runs once, on the PC, and its result is listed in the report for an eye check; the output is what gets maintained afterwards (D4).

- **Pages and pager:** the page tree and titles as today; a nested pager becomes a `pager` section placed where its frame sits (§ rows below). The pager's own tab colours go.
- **Rail:** on a page with strips, every non-strip control (stage, hub toggle, solos, param toggles) goes to `rail`, top to bottom by `frame.y`. On a page without strips (the cue page) they go into groups like strips do.
- **Groups:** a strip, toggle or param fader belongs to the innermost `area` frame that contains its frame's centre; its title is the titled area whose frame touches or overlaps that box (the vertical `EFFECTS` title left of its box, the `HANDS` label above); the area's fill is the group `color`. Controls in no area form one untitled group per contiguous run.
- **Rows:** groups and the pager section are clustered by vertical overlap into rows (top to bottom), ordered by `frame.x` inside a row; controls inside a group by `frame.x`, then `frame.y`.
- **Strips:** `return` for `X-` names, `wide` when the TouchOSC strip is ≥ 1.2× the page's median strip width; `mute_guard` from `double_click_mute`; everything else as today.
- **Global:** the overlay's TechAlert strip + alert box become one `alert`; REFRESH ALL becomes `refresh`.
- **Report:** a `groups` section: for each page, rows → groups → control names, and every control whose group was a guess (no area).

The synthetic fixture (`fixtures/*.tosc`, `expected-layout.json`) is rebuilt in schema 2 and keeps modelling the real project's shapes (a vertical section title beside its box, a label above, a nested pager next to fixed strips, a rail).

## 4. UI

### 4.1 Design tokens (from iemmixer `iem-ui/style.css`, adapted)

- Surfaces: ground `#07070c`, page `#0b0b12`, bars `#11111b`, section `#151522`, strip `#1a1a28`, track `#0a0a14`, line `#2a2a44`.
- Accent teal `#4ecdc4` (+ 28 % / 45 % alpha), mute red `#e63946`, solo yellow `#f4d35e`, band `#4ecdc4`, master `#8b7dff`; text `#eaeaf0` / `#9a9ab4` / `#5c5c78`.
- Meter zones as TouchOSC: green below −12 dB, yellow from −12, red from −3 (`behave/meter.rs` colours move to these tokens).
- Type: Barlow Condensed 700 (names), Barlow 600–700 (UI labels, uppercase + letter-spacing), JetBrains Mono 500–600 tabular (dB, scale). Bundled with the app (no font CDN: the app must work offline on the LAN, D7).
- Radii 7–12, glow only on the active fader, the lit mute/solo and the SOLO ✕ pill; `prefers-reduced-motion` stops the pulses and blinks except the TechAlert wash (it is the alert).

### 4.2 Shell and layout

- **Top bar (46 px on the iPad):** root tabs as a segmented control, the current page's pager tabs as a second one, the SOLO ✕ pill, the instance badges (heartbeat dot), the version (`data-testid="version"`).
- **Rail (112 px):** the page's rail controls, then a flexible gap, then `global` (TechAlert, REFRESH ALL). Every page has the rail (the cue and Conf pages show only `global`).
- **Rows:** each row gets `weight`-proportional height; a row lays out its sections left to right with a 10 px gap; the pager section shows the selected sub-page's groups in place.
- **Pure fit function** (`flow.rs`, replaces `stage.rs`'s canvas code): input = rows as (section, control count, wide count, pager sub-page counts) + the available size; output = one strip width shared by every row (so strips align), each section's width, each row's height. Strip width = the largest width at which the widest row fits, clamped to [64, 120] px (wide = 1.1×); a row that does not fit at 64 px scrolls horizontally inside itself (never the page). Recomputed on resize and on a layout change.
- The surface root keeps `data-testid="stage"` (the E2E `openSurface` and the version/remote specs use it).

### 4.3 Components

- **Strip:** pan bar (fill from the centre, double tap centres); instance tag (BAND / MASTER, `· RET`); fader zone = dB scale | meter | fader; status LED + dB readout; name button = mute. Test ids unchanged (`strip`, `fader`, `pan`, `mute`, `meter`, `status`, `db`, `strip-label`: the name button carries `mute` and holds the `strip-label` text).
- **Fader:** groove + fill + cap (32×22 on the iPad), unity line at 0 dB. **Travel = the track's full height** (the cap's centre runs the whole track, the groove extends half a cap past each end), 1:1 with the finger as TouchOSC's `responseFactor 100` (audit #2). Shaping, double tap and the echo guard stay in `behave/fader.rs` untouched.
- **dB scale:** ticks and labels at +6, 0, −6, −12, −18, −24, −40 dB, placed by the fader law (`v = p^0.515`) over Live's volume taper. The taper points come from Live itself: `DeviceParameter.str_for_value(v)` on a mixer volume (read-only, through the script's raw LOM call), measured once on the real Live and baked as a table in `behave/scale.rs` with a test per point; the meter calibration (`behave/meter.rs`) shares the scale.
- **Meter:** one or two bars (`config.meter_source`), the ballistics and calibration of `behave/meter.rs` unchanged; a peak-hold line (1.5 s, then falls with the bar) and a clip light when the level reaches the calibration's 0 dBFS point, lit until tapped (pure state in `behave/peak.rs`).
- **dB readout:** Live's display value (X1) shown as TouchOSC did: one decimal, no unit, `−∞` for `-inf`, white when Live shows exactly 0 dB, light green otherwise (`behave/db_text.rs`, from Live's string, never from the UI's own maths).
- **Name button (mute):** lit in the track's Live colour while the track is audible, dark with a red `MUTE` mark while muted (F12, TouchOSC semantics); `mute_guard` arms on the first tap with a pulse (`behave/mute.rs`). The colour is the track's `color` property (an int `0xRRGGBB`), subscribed for every strip; until it arrives the group colour stands in. The colour never gates the control (I8 gates on the mute value only).
- **Rail buttons:** stage (lit while the mics are live, #9), STAGE AUT, solos (lit = soloed), param toggles (on / off / mixed states, `press` modes unchanged), alert (lit while TechAlert is unmuted; the full-screen red wash blinks every `period_ms`), refresh (0.5 s debounce, yellow 300 ms).
- **SOLO ✕ pill:** shown while any solo control of the layout is on; a tap writes `solo=false` to each of them (owning instance only, X4); it is hidden again when Live reports them off.
- **Param fader** (Podklady All): a strip without pan, meter and mute; its label is Live's display string of the first target (X10).
- **Cue page:** its groups render param toggles as a grid of large buttons.

### 4.4 Code structure

- Pure, natively tested (mutation-gated): `flow.rs`, `behave/scale.rs`, `behave/db_text.rs`, `behave/peak.rs`, `behave/colour.rs` (Live int → CSS, contrast text colour), the SOLO ✕ decision (`behave/solo.rs`), the schema-2 walks in `binding.rs` (`visible_subs` over rail + rows + the selected sub-page + global).
- Glue (excluded per function in `.cargo/mutants.toml` with a reason): component bodies, the resize observer, DOM writes. `raf.rs` stays the one animation loop.
- `stage.rs`, `area.rs`, the canvas/tab geometry helpers and their CSS go.

## 5. Tests

- **proto:** schema-2 parse + every validation rule (one failing document per rule), `bindings()` / `stage_aut_binding()` over the tree, the imported fixture parses and validates.
- **import:** grouping (area containment, a title beside and above its box, row clustering, x order, the rail, a pager section between fixed groups, a control in no area, `wide`), the report's `groups`, the regenerated `expected-layout.json`.
- **hub:** the layout fixtures in schema 2 (`tests/fixtures/layout-ok.json`, `layout-bad.json`), unchanged behaviour of load/poll/fallback.
- **UI unit:** `flow.rs` (fits, the clamp boundaries, the scroll case, wide strips, pager sub-pages of different sizes), the scale table (each point, monotonic), `db_text` (each form, the exact-0 cue), `peak` (hold, fall, clip, reset), `colour`, the SOLO ✕ decision, `visible_subs` counts.
- **E2E (Chromium + WebKit iPad, console guard):** every existing behaviour spec moved to the new DOM with its assertions kept (fader drag and shaping, double tap glide, multitouch, pan, mute + guard, solo, stage mics + STAGE AUT, TechAlert blink, REFRESH ALL, params, pages); geometry specs replaced by structure specs (rows, sections in order, strips in order, rail content, a pager switch keeping the fixed groups); new specs: the scale ticks beside a fader at the table's positions, the colour from SimLive's track `color` on the name button (and a rename of the colour live), stereo meter bars, peak hold and the clip reset, SOLO ✕, the dB readout forms, a drag of one track height moving the fader over its whole range.
- **Visual evidence:** each E2E run uploads screenshots of every page in both projects (artifact); the post-deploy check screenshots the real surface.

## 6. Deploy and migration

- The install ships as today; the PC's `layout.json` is regenerated with the new importer from the TouchOSC project and the saved band set, then the band-member identity carry (`.claude/rules/import-tosc.md`) renames the departed members' bindings, then the hub must report `unresolved=0` before the surface is shown to the engineer.
- K2 for the stereo meter: with `meter_source: lr` on the real set, Live's CPU and the hub's frame rate are measured against `level`; `lr` is deployed only when the cost is acceptable (X2), otherwise one bar.
- The scale table is measured on the real Live before the release that shows it.

## 7. Delivery

One feature on `dev`, pushed in milestones (each green in CI), one PR to `master`:

1. Schema 2 (proto) + hub + the importer and its fixture.
2. The shell, `flow.rs`, rail, rows, sections, the strip with fader, pan, mute, status, meter (existing behaviour on the new DOM), the E2E suite moved.
3. Scale (with the measured table), dB readout, colour, peak hold + clip, SOLO ✕, stereo meter switch, the new E2E specs.
4. Deploy on the PC (re-import, identity carry, K2), post-deploy verification with screenshots of every page.

## 8. Spec amendments

- Program spec: F1 (same pages and sections; placement is computed), F11 (pan look), F14 (solo look), F19 (visual layout: semantic layout + design system, not imported geometry), §2.5 (UI), §2.6 (the importer groups), X1 (Live's value, TouchOSC's form), X2 (stereo under K2 as before), §4.2 (track colours are v1 now), §8 D13.
- S4 design note §1–§2 (canvas, frames, colours), §5–§6 (tab bars, status) are superseded by §4 here; its behaviour sections stay.
- `.claude/rules/ui-rust.md` (stage/pager z, text fitting on tabs) and `.claude/rules/import-tosc.md` (schema 1 fixture notes) follow the code.
