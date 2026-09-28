# UI Redesign Implementation Plan (#21)

> Executed natively by the session that wrote the spec (Tier 0: Rust and Playwright verify in CI; Python locally). The importer task may run in a worktree worker against the contract below.

**Goal:** the approved modern surface on every page, TouchOSC behaviour 1:1, the audit gaps closed.

**Architecture:** semantic layout schema 2 (pages → rail + rows → sections → controls); the UI computes placement (`flow.rs`); the importer groups TouchOSC nodes once.

**Tech stack:** Rust (fohmixer-proto, fohmixer-hub), Leptos 0.7 CSR (fohmixer-ui), Python 3.11 (import tool), Playwright (Chromium + WebKit iPad).

**Spec:** `docs/superpowers/specs/2026-09-28-ui-redesign-design.md`

## Global constraints

- Tier 0: no local cargo compilation; `cargo fmt`, ruff, the Python tests and `npx playwright test --list` locally.
- Public repo: no names, hosts, IPs, track names from the real site in code, fixtures, commits.
- Every `behave/*` module keeps its behaviour and tests; new logic is pure and mutation-proof (`.claude/rules/ui-rust.md`).
- No `continue-on-error`, no skips, no retries; console guard in every E2E test.
- Commit messages name `(#21)`; bug fixes RED before GREEN.

## Contract: layout.json schema 2 (exact JSON)

```json
{
  "schema": 2,
  "default_page": "foh",
  "pages": [
    { "id": "foh", "title": "FOH",
      "rail": [
        { "kind": "stage", "binding": { "instance": "band", "anchor": { "kind": "track", "name": "Mics #" } }, "aut": true, "label": "STAGE" },
        { "kind": "hub_toggle", "key": "stage_aut", "label": "STAGE AUT" },
        { "kind": "solo", "binding": { "instance": "band", "anchor": { "kind": "track", "name": "Vocals grp" } }, "label": "Vocals" },
        { "kind": "param_toggle", "label": "REVERB", "targets": [ … ], "press": "toggle", "color": "#F39420" }
      ],
      "rows": [
        { "sections": [
            { "kind": "pager", "id": "foh-pager", "default_page": "stage",
              "pages": [ { "id": "stage", "title": "STAGE", "sections": [ { "kind": "group", "id": "stage-1", "title": "STAGE", "color": "#1E3A1E", "controls": [ … ] } ] } ] },
            { "kind": "group", "id": "foh-2", "controls": [
                { "kind": "strip", "binding": { … }, "strip_kind": "standard", "wide": true } ] } ] },
        { "sections": [ { "kind": "group", "id": "foh-3", "title": "EFFECTS", "color": "#636363", "controls": [ … ] } ], "weight": 1.0 }
      ] },
    { "id": "conf", "title": "Conf", "rows": [ { "sections": [ { "kind": "group", "id": "conf-1", "controls": [ { "kind": "text", "text": "unfold_band: 'Vocals grp'" } ] } ] } ] }
  ],
  "global": [
    { "kind": "alert", "binding": { … }, "period_ms": 300, "label": "TechAlert" },
    { "kind": "refresh", "label": "REFRESH ALL" }
  ],
  "config": { "unfold": [ … ], "fader_shaping": true },
  "report": { "groups": { "foh": [ [ "stage-1 STAGE: Vox 1, Vox 2", "foh-2: Podklady, Band" ], [ "foh-3 EFFECTS: …" ] ] }, "…": "…" }
}
```

- Control kinds and fields: `strip {binding, strip_kind: "standard"|"return", wide?: false, mute_guard?: false}`, `solo {binding, label?}`, `stage {binding, aut?: false, label?}`, `hub_toggle {key, label}`, `param_toggle {label, targets, press, color?}`, `param_fader {label, targets}`, `alert {binding, period_ms, label?}`, `refresh {label?}`, `text {text}`. Defaults are omitted in the output.
- `Row.weight` defaults to 1 and is omitted when 1. `rail` and `rows` default to empty. A sub-page's `sections` hold groups only. Section/page ids are unique across the document.
- `ParamTarget`, `Press`, `Scale`, `Binding`, `LayoutConfig` are exactly schema 1's.

## Milestone 1: schema 2, hub, importer

### Task 1.1: proto schema 2
Files: `crates/fohmixer-proto/src/layout.rs`, `crates/fohmixer-proto/src/layout/tests.rs`.
- Types per the contract; `LAYOUT_SCHEMA = 2`; drop `Canvas`, `Frame`, `TabBar`, `Tab`, `Item`, `Style`, `ItemKind`, `StripChildren`, `Narrow`/`Solid`/`MeterMuteOnly`, the canvas check.
- `validate()`: schema, unique ids (pages, sub-pages, sections), default pages exist, ≤ 1 pager per page and none in a sub-page, colours, bindings parse, alert period > 0, hub_toggle key, targets complete.
- `bindings()` walks rail, rows (groups and every sub-page), global; `stage_aut_binding()` = first `stage` with `aut` anywhere.
- Tests: one valid document; one failing document per rule; walks; the imported fixture parses and validates.

### Task 1.2: hub
Files: `crates/fohmixer-hub/src/{layout,router,outbox,routes,lib}.rs`, `live/names.rs`, `tests/fixtures/layout-ok.json`, `layout-bad.json`, `scripts/fohmixer-pc/Test-Fohmixer.ps1` (its layout fixture).
- Only type paths change; behaviour stays (load, poll, last-good, fallback, unresolved names, STAGE AUT target).
- Fixtures rewritten in schema 2 with the same bindings.

### Task 1.3: importer (worktree worker, Python only)
Files: `tools/import-tosc/import_tosc.py`, `test_import_tosc.py`, `build_fixtures.py`, `fixtures/*`.
- Emit the contract; group per spec §3 (rail, area containment by centre, titles beside/above, rows by vertical overlap, x order, pager section, global, `wide`, report `groups`).
- Tests for each grouping rule; regenerate `expected-layout.json`; fixture shapes as spec §3.

## Milestone 2: shell, flow, strip on the new DOM

### Task 2.1: `flow.rs` (pure) + tests
Strip width shared by all rows, clamp [64, 120], wide 1.1×, per-row horizontal scroll flag, row heights by weight.

### Task 2.2: shell
`pages/surface.rs` rewritten: top bar (root tabs, pager tabs, SOLO ✕ slot, badges, version), rail (page rail + global footer), rows → sections → controls; the selected page/sub-page remembered as today; `data-testid="stage"` on the body. Remove `stage.rs`, `components/area.rs`.

### Task 2.3: strip and controls on the new DOM
`components/{strip,fader,pan,meter,buttons,params,overlay,mod}.rs` + `style.css` (tokens §4.1). Fader travel = track height. Name button = mute (`data-testid="mute"`, inner `strip-label`). Rail button forms of solo/stage/hub/param/alert/refresh.

### Task 2.4: `binding.rs` walks schema 2
`visible_subs` over rail + rows + selected sub-page + global; tests updated (`binding/tests.rs`, `store/conn/tests.rs`).

### Task 2.5: E2E moved
Every spec on the new DOM, assertions kept; geometry specs → structure specs; screenshots of every page uploaded; harness writes schema 2.

## Milestone 3: new features

- 3.1 `behave/scale.rs` table (measured on the real Live with `str_for_value`, read-only) + scale ticks; E2E positions.
- 3.2 `behave/db_text.rs` + readout; E2E forms.
- 3.3 `behave/colour.rs` + `color` subscription + name button colour; SimLive colour; E2E.
- 3.4 `behave/peak.rs` + peak line + clip light; E2E.
- 3.5 `behave/solo.rs` + SOLO ✕; E2E.
- 3.6 stereo bars under `meter_source: lr` (already supported by `binding.rs`); E2E with `lr`.

## Milestone 4: deploy

- Re-import on the PC (schema 2), identity carry of the band members, `unresolved=0`, K2 for `lr`, install, post-deploy Playwright with screenshots of every page, the engineer's feedback on #9.

## Review focus

1. A strip whose Live colour is very light (white): the name text must stay readable (contrast text colour).
2. A row with more strips than fit at 64 px: it scrolls inside itself; faders still drag vertically (touch-action).
3. A layout change while a sub-page is open whose id disappears: fall back to the pager's default page.
4. A track renamed live: its strip goes red (I5) and the colour subscription is released with the others.
5. The iPad rotated to portrait: the surface stays usable (rows scroll), no page scroll.
