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
- **A band-set rework (members replaced) leaves name bindings unresolved (#9):** the layout binds by track name, and the engineer renames a departed member's tracks to the new member's, marks others ` del`, reorders. Live keeps each track's `Id` (the attribute on `AudioTrack`/`GroupTrack`/… under `LiveSet/Tracks`) through renames and moves, and every save leaves the previous set as `Backup/<set> [<save time>].als`. Carry the deployed layout by identity (a script on the PC, read-only on both sets): look each band binding name up in the set the layout came from (that backup), follow its `Id` to the current set, rename; a member whose tracks changed only in the first word (`<OLD> stream` → `<NEW> stream`) is renamed in the labels too (`<Old> TU`). Confirm the set's MIDI mappings (`KeyMidi`) followed the same tracks, and that every band name resolves exactly once in the current set; the hub then logs `unresolved=0`. A track marked ` del` still resolves until the engineer deletes it. Replace the PC's import output as well (an install copies `-Layout` over the data folder's layout), keep the previous files beside it. The reworked TouchOSC project may exist only on the iPad: the band `AbletonOSC` log shows the indices a TouchOSC client subscribed to and when, which tells whether it has connected since the rework.
