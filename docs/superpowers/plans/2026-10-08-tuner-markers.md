# Strips from Tuner markers (#68): implementation plan

> For agentic workers: implement task by task on `dev`; Tier 0 (no local cargo compilation, CI verifies). Steps use checkboxes.

**Goal:** what is on the surface, under which name, in which groups, pinned or mute-guarded, comes from a Tuner in each track's device chain (spec D16): `"Vox 1" +G:VOCALS:2 +G:TALKSHOW:1 +PIN +MG`. The frame file stays the default view; every other tag group is a view; problems are shown, never dropped; a tag manual in the app and the tray; the tray's left click opens fohmixer.

**Architecture:** pure parts in `fohmixer-proto` (`markers.rs`: `parse`, `compose`), so the hub, `layout check` and the tests share them. In the hub, a marker keeper (`router/markers.rs`, the unfold keeper's pattern) finds the Tuners of both instances and follows them; the layout store composes its frame file with what the keeper found and serves a new revision only when the composition changed. The UI shows a strip's marker label, the views (a POHĽADY section in the column), the conflict and problem marks and the manual overlay, as the approved mockup shows. The tray opens fohmixer on a left click and the manual from its menu.

**Spec:** `docs/superpowers/specs/2026-09-28-fohmixer-program.md` D16, F3, F4, F12, F23–F26, I9–I11, R9–R11; #68 (design, probe, the approved mockup `docs/mockups/surface-views-v4.html` at 5594a80); the owner's decisions on #3 (2026-10-08).

## Global constraints

- Public repo: invented names only in fixtures, tests and docs; denylist scan before every push.
- Tier 0: no `cargo build/test/check/clippy`, no `trunk build` locally; `cargo fmt` only. Python tests and `ruff` run locally.
- fohmixer only reads a marker: it never writes a Tuner, except the migration's renames on the owner's go (PR C).
- A marker strip's label is its Tuner's quoted label, never the track's name.
- A problem is never dropped silently (I9); an unchanged composition never bumps the revision (I10).
- Every surface tap target owns its touches (`use:owns_touches`), no mouse events; every reactive read in a component is `try_*`.
- Zero console errors and warnings in every E2E test.

## The rules (decided)

- **A marker** is a device whose `class_name` is `Tuner` (`TUNER_CLASS`, one constant; confirmed on the first Tuner the owner adds) and whose name has a `"` or a ` +`. A Tuner without either is an ordinary Tuner and is ignored.
- **The grammar** (`parse`): the name is trimmed; an optional label first, `"` … `"` (no `"` inside; may be empty: a problem); then tokens split on whitespace. A token is a tag when it starts with `+`:
  - `+G:NAME` or `+G:NAME:N`: NAME of `A-Z`, `0-9`, `-`, `_` (1–32 chars); N a positive integer up to 999. Several allowed; the same NAME twice is a problem (the first counts).
  - `+PIN`, `+MG`: flags; repeated is harmless.
  - Anything else starting with `+` is an unknown tag (a problem); text outside the label that is no tag is a problem (`stray text`).
  - No label: a problem (`no label`); the strip still shows under the track's index (`#12`) so it can be found.
  - Uppercase only, as AbleSet: `+g:x` is an unknown tag.
- **Found** (what the keeper reports): instance, track kind (track / return), track index, the Tuner's name, and how many marker Tuners the track holds.
- **Compose** (`compose(frame, found) -> Composed { layout, problems }`):
  - Every found marker is a strip: `Anchor::TrackAt { index }` or `Anchor::ReturnAt { index }`, `strip_kind` from the kind, `label`, `mute_guard` from `+MG`, `pinned` from `+PIN`, `problem` (the first problem's code, for the mark).
  - A frame group with `tags: "NAME"` gets the strips of tag group NAME, by place, then by (instance, kind, index) for those without a place; two strips on one place: both shown in that order, both marked `place`.
  - An equal label (exact, after trimming) on two or more markers anywhere: all of them `problem: conflict`, disabled in the UI.
  - Two marker Tuners in one track: the first counts, the strip marked `double`.
  - Every tag group no frame group shows becomes a **view**: a page `{ id: "view-<NAME>", title: NAME, view: true }` with one row of one group (the group's strips by place) followed by every pinned strip not in it, grouped by their first tag group; views ordered by the group's first marker (instance, kind, index).
  - A marker with no group is a problem (`no group`) and shows on no page, only in `/api/status`.
  - The problems list: every problem with instance, track index, the Tuner's name and the code.
- **The keeper** (per instance, the unfold keeper's discipline: its own client id `MARKERS_CLIENT`, outside the status subscription count, capped retries, a newer read voids an older answer):
  - Listens to `live_set tracks` and `live_set return_tracks`; on each value reads, in one batch, every track's `devices` (one `get_prop devices` per track).
  - From the device lists (objects with `class`, `name`, `path`): devices of class `Device` (native, not a rack or plugin) get a second batch: `class_name` each. The Tuners found get a `name` listener each (a rename fires only that listener), and every track's `devices` gets a listener (a Tuner added or removed).
  - A Tuner's `name` value, a `devices` value or a list value refreshes the found set (a list change rereads everything; a name change updates one entry).
  - A change of the found set goes to the layout store (`set_markers`), debounced 300 ms; the store answers with a new revision only when the composition changed.
- **The store:** keeps the frame (the file, validated as today), the found markers and the composition; `current()` serves the composition; the frame's backups stay as today; each new composition is also kept as `served.json.<stamp>` (newest 30).
- **Status:** `/api/status` gains `markers: { found: N per instance, problems: [...] }`.
- **The manual:** one static page (`crates/fohmixer-hub/assets/znacky.html`, Slovak, the syntax table and examples of the mockup), served at `/znacky` without a login (it holds no data); the UI's `ZNAČKY` chip opens it in an overlay (an `iframe` with "✕ Zavrieť"); the tray's menu item opens `<local URL>/znacky`.
- **The tray:** `show_menu_on_left_click(false)`; `on_tray_icon_event`: a left button's `Up` click opens fohmixer; the right click shows the menu, which gains "Návod k značkám" (`MenuAction::Manual`).

## PR A: markers in the hub (no surface change but the label)

1. **Proto anchors and fields** (`layout.rs`, tests): `Anchor::TrackAt { index: u32 }`, `Anchor::ReturnAt { index: u32 }` (`target()`: `live_set tracks N`, `live_set return_tracks N`); `Strip.label: Option<String>`, `Strip.problem: Option<MarkerProblem>` (`conflict`, `double`, `place`, `tag`); `Group.tags: Option<String>`; `Page.view: bool`. `strip_tracks()` and the unfold keeper move from names to targets (`(instance, target)`), so index strips stay unfolded too.
2. **`markers.rs`** (proto, tests first): `TUNER_CLASS`, `Marker`, `Problem`, `parse`, `Found`, `compose`. Exhaustive tests of the grammar (each problem at its boundary: 32/33-char names, N 0/1/999/1000, empty label, unclosed quote, lowercase tag, a stray word) and of compose (frame groups filled by place then index, conflicts across instances, double Tuner, views with pins, no group).
3. **SimLive** (`sim/Live/__init__.py`, tests): devices on tracks and return tracks (`devices` list with listeners; a `Device` with `name`, `class_name`, a `name` listener); the e2e harness gains `/sim/tuner` (add / rename / remove a Tuner on a track of an instance).
4. **The keeper** (`router/markers.rs` pure, tests; router glue): as decided above; the router hands each found set to the store and broadcasts the new revision as `RouterMsg::Layout` does.
5. **The store** (`layout.rs`): `set_markers(found) -> Option<u64>`; poll composes; `served.json` backups; `layout check` composes the frame with no markers (the frame alone must be valid).
6. **Status** (`client.rs`, router): `markers`.
7. **UI label:** a strip with `label` shows it (`behave/label.rs`: the marker's label as written, sized by `--n` like today's).
8. **E2E** (`markers.spec.ts`): a Tuner added in SimLive shows a strip with its label in a frame group within seconds; a rename moves it to another group; two tracks with one name and their own Tuners are two working strips (a fader drag reaches the right track); `/api/status` lists a malformed marker.
9. **Docs and rules:** `.claude/rules/hub-rust.md` (the keeper), `.claude/rules/live-script.md` (SimLive devices), spec D16 done.

## PR B: views, marks, manual, tray (the approved mockup)

1. **Views** (`arrange.rs` unchanged; `surface.rs` `ColumnHead`): a POHĽADY section under the pager's tabs, one button per `view` page; a tap shows the view page (the pager's tabs dimmed), a second tap returns to the page shown before; E2E.
2. **Marks** (`strip.css`, `components/strip.rs`): `conflict` = disabled, red hatching, `KONFLIKT` on the readout line; other problems = yellow corner `!`, `ZNAČKA?` on the readout line, still working; E2E for both, WebKit included.
3. **Manual:** the hub serves `/znacky`; the column's status line gets the `ZNAČKY` chip opening it in an overlay; E2E (opens, closes, console clean).
4. **Tray:** left click opens fohmixer; the menu item opens the manual (`view.rs` tests; `Test-Fohmixer.ps1` reads back the menu).

## PR C: migration

1. A hub command (`fohmixer-hub markers plan <layout>`) prints, for today's frame, the Tuner name each strip would get (label from the strip's track name, its group, its place, `+PIN`, `+MG`) and the track it belongs to; the owner adds Tuners in Live; on his go the hub renames them (`set_prop name`), then the frame's strip groups become `tags` groups.
2. Remove `tools/import-tosc` and its fixture once the PC's frame names no strip by track name (#10).
