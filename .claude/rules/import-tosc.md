---
paths:
  - "tools/import-tosc/**"
---

# TouchOSC import tool (learned on #7, #9)

- **The synthetic project models the real one's structure** (checked read-only on the PC; the real file never enters the repo). A fixture shaped differently hides the importer's bugs (#7's list). Known shapes:
  - every tab is grey in `tabColorOff`; the page's colour is `tabColorOn`; `textSizeOff` / `textSizeOn` are the pager's;
  - pagers and pages fill themselves (`background` + `color`): the grey root pager (the layout's `background`), the black nested pager, the near-black STAGE page;
  - stage, STAGE AUT and solo groups are transparent; the colour is on the inner `btn_mute` / `btn_stage_aut` / `btn_solo` (`control_style`);
  - a narrow strip's instance label is 39 × 25 under its dB text.
- After a deliberate change, regenerate `fixtures/expected-layout.json` with `FOHMIXER_REGENERATE=1 python3 -m unittest discover -s tools/import-tosc` and keep its readers consistent: `fohmixer-proto` `imported_layout_parses_and_validates`, the UI's `binding/tests.rs` and `store/conn/tests.rs`, `e2e/tests/pages.spec.ts`, the E2E harness.
- It is a one-shot tool, copied to the PC as the single file `import_tosc.py` and deleted at S6 (spec §2.6): keep it one file.
