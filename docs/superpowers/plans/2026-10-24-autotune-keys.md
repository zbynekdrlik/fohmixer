# Autotune keys written and read back (PR 2 of #140) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** the hub finds each `+AUTOTUNE` track's key device, follows the key in force along each set's playhead, and writes that key into every Auto-Tune (PA directly, stream through AudioGridder's slots) and into Live's bar, reading every write back. Nothing switches: no selector moves, AUT cannot be turned on, and a tap writes nothing.

**Architecture:**

- **Pure parts in the hub's rules** (`rules/autotune/keys.rs`):
  - the key device's kind from its parameter names;
  - the key a pair of values shows;
  - the key in force at a position with its lookahead;
  - the per-set key follower;
  - the value to write and the read-back judgement.

  All sit under the mutation gate.
- **The existing `Latencies` find** (`rules/autotune/find.rs`) goes three reads further, into the rack's chain 0, its plug-in and that plug-in's parameters.
- **The router's glue** (`router/autotune/keys.rs`, an `impl Router` block beside `router/autotune.rs`):
  - it follows the key parameters, `current_song_time` and `tempo` (`AUTOTUNE_CLIENT`);
  - it reads the key in force from `SongMapIo`'s newest map;
  - it records each step, and writes and reads back through each parameter's `$ref`.
- **SimLive** gains:
  - a song clock;
  - Live's bar;
  - the two key devices;
  - a parameter that refuses writes as Live does when a Max map holds it.

**Tech Stack:** Rust (fohmixer-proto, fohmixer-hub: tokio, serde), Python (SimLive), nextest host tests, Playwright (one spec adjusted).

**Spec:** `docs/superpowers/specs/2026-10-23-autotune-key-design.md` (§2.1–2.3, §5 with the spike's results and the owner's decision of 2026-10-24, §6, §7's hold, §12's PR 2, Appendix A.4–A.5). Read it first.

## The split *(call)*

The whole of PR 2 is about 3500 lines with tests, more than twice the ~1500 a review holds well. It ships as two PRs, each green, deployed and verified on its own:

- **PR 2a, read-only (Tasks 1–5).**
  - It finds the key devices, follows their values, and follows the key in force along the playhead, with a `key` record per change.
  - It reports all of that in `/api/status`, refuses AUT on and the tap, and writes nothing.
  - On the PC it proves, at a stopped Live and at the next event, that every target is found and that the key in force follows the songs. That is the read the writes will trust.
- **PR 2b, the writes (Tasks 6–9).**
  - It covers the value with its quarter-step rule, the batch write with Live's bar, the explicit read-back, mismatch with one retry, another writer, lateness, and the rewrite at every song start.
  - It is deployed together with the maps' switch-off.

Rejected: one PR. Its review would cover the find, the position, SimLive and the writes at once, and its PC check would be the first time the targets are seen while the hub already writes to them.

## Global Constraints

- **Tier 0:** no local cargo compilation. Push `dev` and read CI. Locally only:
  - `cargo fmt --all`, `ruff check .` and the Python tests;
  - `python3 scripts/check_integrity.py` and `python3 scripts/check_version.py`.
- **Public repo:** invented names only in code, tests, fixtures, commits and tickets: `Vocal A #`, `Stream FX`, `001 Test One {C 120 4/4}`.
  - No song, singer or host name, no IP, no user path.
  - Run the denylist scan before every push: `python3 scripts/denylist_scan.py --denylist <list> --identities scripts/allowed-identities.txt --boundary scripts/denylist-boundary.txt --accepted scripts/denylist-accepted.txt --tree HEAD --commits HEAD`.
- **Nothing moves a selector in PR 2.**
  - The only `set_prop` PR 2b adds is on a key device's Key/Scale parameter through its `$ref`, and on `live_set` `root_note` / `scale_name`.
  - PR 2a adds none.
- **Version:**
  - PR 2a: `dev` is 1.10.0 (> master 1.9.0), so no bump. 2a is a feature (+0.1.0) and ships as 1.10.0.
  - PR 2b: bump to 1.11.0 as its first commit after 2a's merge.
- **Commits:** the noreply identity, with `(#140)` and `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`. A fix found in review goes `[red]` before `[green]`.
- **Mutation gate:** every pure function has a direct unit test. No hand-stepped index loops, and no struct-update base in a new literal (`.claude/rules/hub-rust.md`).
- **English:** every hub text, record field and log line (#141).
- **The values (spec A.4, A.5, measured on the PC):**
  - Key `k/11`, Scale Major `0`, Minor `1/28`.
  - Read-back `round(p × 11)` and `round(p × 28)`, half up. A scale step other than 0 or 1 is no key.
  - Live's bar: `root_note = k`, `scale_name = "Major" | "Minor"`.
  - Every key parameter runs 0.0–1.0 and is not quantized.
- **The constants:**
  - `LOOKAHEAD_MS` = 35.0 (one script tick), at most `MAX_LOOKAHEAD_BEATS` = 1.0;
  - `LATE_BOUND_MS` = 100.0;
  - `NUDGE` = 0.25 (a quarter step);
  - `SAME_VALUE_EPS` = 1e-6 (Live keeps values as float32: 2/11 reads back 0.1818181872367859).
- **The Live error a held parameter gives (spike part 1, exact):** `Value cannot be set, the parameter is disabled`.

## Review Focus

1. **A float32 echo of the value written.**
   - Live answers 2/11 as `0.1818181872367859`. The quarter-step rule must see that as the centre and write the nudged value next. Otherwise "rewrite at song start" writes an equal value that may never reach the remote plug-in.
   - Test `a_float32_echo_of_the_centre_counts_as_the_centre` (Task 6).
2. **A position at a locator as Live reports it.**
   - Live keeps every cue from song 009 on at the beat plus ~5.2e-5. A jump to a song's locator puts `current_song_time` exactly at that value, which must be inside the song. A hand-placed playhead at the rounded beat is 5e-5 before it and is not.
   - Test `the_key_in_force_uses_the_cue_time_as_live_reports_it` (Task 1).
3. **No tempo heard yet** (the first song-time delivery before the tempo's): no lookahead, never a division by zero or a NaN.
   - Test `no_tempo_means_no_lookahead` (Task 1).
4. **A map read while the playhead runs.** Every transport change re-reads the map, and the read takes up to ~200 ms on the PC. Until the new map answers, the old one gives the key, so the key in force never flickers to none.
   - Test `a_map_read_keeps_the_key_in_force_until_the_new_map` (Task 4).
5. **A jump back before a key change in the same song:** the song key again, with why `change`. The song does not start again, so there is no song-start rewrite.
   - Test `a_jump_back_before_a_change_is_a_change_not_a_song_start` (Task 1).

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/fohmixer-hub/src/rules/autotune/keys.rs` (create) | Pure: `KeyDevice`, `device_of`, `shows`, `lookahead_beats`, `in_force`, `KeyFollow` (2a); `write_value`, `judge`, `KeyTarget` (2b) |
| `crates/fohmixer-hub/src/rules/autotune/keys_tests.rs` (create) | Its unit tests |
| `crates/fohmixer-hub/src/rules/autotune/find.rs`, `find_tests.rs` (modify) | The find goes on into chain 0, the plug-in and its parameters |
| `crates/fohmixer-hub/src/rules/autotune.rs` (modify) | `#[path] pub mod keys;`, new `Step` variants and their lines and warn class |
| `crates/fohmixer-hub/src/router/autotune.rs` (modify) | `Target` gains its key device and values; AUT on and the tap refused; the delivery routes key and position subscriptions |
| `crates/fohmixer-hub/src/router/autotune/keys.rs` (create) | Glue: the position subscriptions, the key in force per set, records, and (2b) writes and read-back |
| `crates/fohmixer-hub/src/router/autotune/keys_tests.rs` (create) | Router unit tests |
| `crates/fohmixer-hub/src/router.rs` (modify) | `RouterMsg::AutotuneKeysWritten`, `AutotuneKeysRead` (2b); the song map's read hands its set to `autotune_map_read` |
| `crates/fohmixer-proto/src/client.rs` (modify) | `KeyTargetStatus`, `KeyInForce`, `AutotuneStatus.keys`, `AutotuneTargetStatus.key` |
| `sim/Live/__init__.py`, `sim/site_builder.py`, `sim/host.py`, `sim/tests/test_keys.py` (modify, create) | The song clock, `root_note` / `scale_name`, a disabled parameter, host lines |
| `crates/fohmixer-hub/tests/autotune_keys.rs` (create), `.config/nextest.toml` (modify) | Host tests in `hub-hosts` |
| `e2e/tests/autotune.spec.ts` (modify) | The tap and the hold are refused |
| `.claude/rules/hub-rust.md` (modify) | The key path's design facts |

---

## PR 2a: read-only

### Task 1: The key device, the shown key and the key in force (pure)

**Files:**
- Create: `crates/fohmixer-hub/src/rules/autotune/keys.rs`, `keys_tests.rs`
- Modify: `crates/fohmixer-hub/src/rules/autotune.rs` (`#[path = "autotune/keys.rs"] pub mod keys;`)

**Interfaces:**
- Consumes:
  - `fohmixer_proto::songmap::{Key, SongMap, Span, key_at}`;
  - `fohmixer_proto::songmap::key::{key_index_of, scale_index_of, KEY_STEPS, SCALE_STEPS}`.
- Produces:
  - **`pub enum KeyDevice { AutoTune, Gridder }`** (serde snake_case: `auto_tune`, `gridder`).
  - **`pub struct KeyParams { pub device: KeyDevice, pub key: usize, pub scale: usize }`:** the parameter indexes.
  - **`pub enum KeyMissing { NoChain, Plugins(usize), NoKeyParams }`,** with `Display`:
    - `the Latencies rack has no chain`;
    - `chain 0 holds {n} plug-ins, not one`;
    - `the plug-in has no Key/Scale parameters`.
  - **`pub fn device_of(names: &[&str]) -> Result<KeyParams, KeyMissing>`:**
    - exactly one name `Key` and one `Scale` → `AutoTune` with their indexes;
    - otherwise names[1] starting `0:` and names[2] starting `1:` → `Gridder` `{key: 1, scale: 2}`;
    - otherwise `NoKeyParams`. `AutoTune` wins when both match.
  - **`pub fn shows(key_value: f64, scale_value: f64) -> Option<Key>`:** the step indexes half up; a scale step of 0 is Major, 1 is Minor, any other none.
  - **`pub const LOOKAHEAD_MS: f64 = 35.0;` `pub const MAX_LOOKAHEAD_BEATS: f64 = 1.0;`**
  - **`pub fn lookahead_beats(tempo: Option<f64>) -> f64`:** `min(LOOKAHEAD_MS / 60000 × tempo, MAX_LOOKAHEAD_BEATS)`. It is 0 for none, and for a tempo that is not finite or not > 0.
  - **`pub fn in_force(map: &SongMap, time: f64, playing: bool, tempo: Option<f64>) -> Option<(u16, Key)>`:** `key_at(map, time + lookahead)` while playing, `key_at(map, time)` stopped.
  - **`pub fn change_time(map: &SongMap, song: u16, time: f64) -> Option<f64>`:** the start of the last key change of span `song` at or before `time`. None before its first change, or for a song with none.
  - **`pub enum KeyWhy { Song, Change, NoKey, Found, Retry }`** (serde snake_case): `found` and `retry` are 2b's.
  - **`pub struct KeyFollow { song: Option<u16>, key: Option<Key> }`:**
    - `pub fn new() -> Self`;
    - `pub fn on_position(&mut self, now: Option<(u16, Key)>) -> Option<KeyWhy>`:
      - a span other than the last one in force → `Song`;
      - the same span with another key → `Change`;
      - leaving a key in force (now none, before some) → `NoKey`;
      - otherwise none.
    - `pub fn now(&self) -> Option<(u16, Key)>`;
    - `pub fn reset(&mut self)`: a reconnect forgets the last song.

- [ ] **Step 1: Write the failing tests** in `keys_tests.rs`, with these helpers:
  - `cue(name, t)` and `clip(name, t)`, as in the song map's tests;
  - `map_031()`: `build` of `031 Test {F 95 4/4}` at 22766, `SONG END` at 23501 and the clip `Key {G}` at 22897.

```rust
#[test]
fn auto_tune_artist_is_found_by_its_key_and_scale_names() {
    let names = ["Device On", "Input Type", "Key", "Scale", "Retune Speed"];
    assert_eq!(device_of(&names), Ok(KeyParams { device: KeyDevice::AutoTune, key: 2, scale: 3 }));
}
#[test]
fn audiogridder_is_found_by_its_first_two_slots() {
    let names = ["Device On", "0:(unused):(unassigned)", "1:(unused):(unassigned)", "2:(unused):(unassigned)"];
    assert_eq!(device_of(&names), Ok(KeyParams { device: KeyDevice::Gridder, key: 1, scale: 2 }));
}
#[test]
fn a_device_without_key_parameters_is_missing() {
    assert_eq!(device_of(&["Device On", "Mix"]), Err(KeyMissing::NoKeyParams));
    assert_eq!(device_of(&["Device On", "Key", "Key", "Scale"]), Err(KeyMissing::NoKeyParams));
    assert_eq!(device_of(&["Device On", "1:(unused):(unassigned)", "0:(unused):(unassigned)"]), Err(KeyMissing::NoKeyParams));
}
#[test]
fn the_shown_key_reads_the_steps_half_up() {
    assert_eq!(shows(0.787401556968689, 0.007874015718698502), Some(Key { root: 9, minor: false })); // spike: A major
    assert_eq!(shows(10.0 / 11.0, 1.0 / 28.0), Some(Key { root: 10, minor: true }));
    assert_eq!(shows(0.5, 0.0), Some(Key { root: 6, minor: false }));
    assert_eq!(shows(0.0, 2.0 / 28.0), None); // Chromatic, Auto-Tune's default
}
#[test]
fn no_tempo_means_no_lookahead() {
    assert_eq!(lookahead_beats(None), 0.0);
    assert_eq!(lookahead_beats(Some(0.0)), 0.0);
    assert_eq!(lookahead_beats(Some(f64::NAN)), 0.0);
    assert!((lookahead_beats(Some(120.0)) - 0.07).abs() < 1e-12);
    assert_eq!(lookahead_beats(Some(5000.0)), 1.0);
}
#[test]
fn a_key_clip_is_in_force_one_tick_early_only_while_playing() {
    // 031 → G at 22897 (spec §2.1), 120 bpm: 0.07 beats ahead.
    let m = map_031(); // F from 22766, G from 22897, SONG END 23501
    assert_eq!(in_force(&m, 22896.94, true, Some(120.0)), Some((31, Key { root: 7, minor: false })));
    assert_eq!(in_force(&m, 22896.92, true, Some(120.0)), Some((31, Key { root: 5, minor: false })));
    assert_eq!(in_force(&m, 22896.94, false, Some(120.0)), Some((31, Key { root: 5, minor: false })));
}
#[test]
fn the_key_in_force_uses_the_cue_time_as_live_reports_it() {
    let m = build(&[cue("009 Test {D 100}", 10960.000052291458), cue("SONG END", 11200.000052291458)], &[]);
    assert_eq!(in_force(&m, 10960.000052291458, false, None), Some((9, Key { root: 2, minor: false })));
    assert_eq!(in_force(&m, 10960.0, false, None), None);
}
#[test]
fn change_time_names_the_clip_start_in_force() {
    let m = map_031();
    assert_eq!(change_time(&m, 31, 22897.0), Some(22897.0));
    assert_eq!(change_time(&m, 31, 22896.9), None);
    assert_eq!(change_time(&m, 32, 22897.0), None);
}
#[test]
fn the_follower_starts_a_song_once_then_follows_its_changes() {
    let (f, g) = (Key { root: 5, minor: false }, Key { root: 7, minor: false });
    let mut k = KeyFollow::new();
    assert_eq!(k.on_position(Some((31, f))), Some(KeyWhy::Song));
    assert_eq!(k.on_position(Some((31, f))), None);
    assert_eq!(k.on_position(Some((31, g))), Some(KeyWhy::Change));
    assert_eq!(k.on_position(None), Some(KeyWhy::NoKey));
    assert_eq!(k.on_position(None), None);
    assert_eq!(k.on_position(Some((32, g))), Some(KeyWhy::Song)); // same key, new song
}
#[test]
fn a_jump_back_before_a_change_is_a_change_not_a_song_start() {
    let (f, g) = (Key { root: 5, minor: false }, Key { root: 7, minor: false });
    let mut k = KeyFollow::new();
    k.on_position(Some((31, g)));
    assert_eq!(k.on_position(Some((31, f))), Some(KeyWhy::Change));
}
#[test]
fn a_reset_makes_the_same_song_a_start_again() {
    let f = Key { root: 5, minor: false };
    let mut k = KeyFollow::new();
    k.on_position(Some((31, f)));
    k.reset();
    assert_eq!(k.on_position(Some((31, f))), Some(KeyWhy::Song));
}
```

- [ ] **Step 2: Run** `cargo fmt --all --check`. Tier 0: the tests run in CI.
- [ ] **Step 3: Implement** the items listed in Interfaces. Iterators only; `change_time` filters the span's `changes`.
- [ ] **Step 4: Push** with Task 2 and read CI. The `rules::autotune::keys` tests pass, with 0 missed in the mutation shards for the diff.
- [ ] **Step 5: Commit** `feat(hub): the key device, the shown key and the key in force (#140)`

### Task 2: The find reaches the key device

**Files:**
- Modify: `crates/fohmixer-hub/src/rules/autotune/find.rs`, `find_tests.rs`

**Interfaces:**
- Consumes: `keys::{device_of, KeyParams, KeyMissing}`.
- Produces:
  - **`pub struct Param { pub path: String, pub id: String }`.**
  - **`pub struct KeyFound { pub params: KeyParams, pub device: String, pub key: Param, pub scale: Param }`:** `device` is the plug-in's path.
  - **`Found` gains `pub key: Result<KeyFound, KeyMissing>`.** A key device that is missing never makes the selector find missing: the macro is still `Found`, and PR 3 uses its own rule.
  - **`pub const MAX_READS: usize = 2 * MAX_RACKS + 4;`**
- **The reads after the one match:**
  1. One batch: the rack's `parameters` and its `chains`.
  2. Chain 0's `devices`. It is skipped with `NoChain` when the rack has no chain.
  3. When exactly one item of class `PluginDevice` is there, its `parameters`. Otherwise `Plugins(n)`.

  `device_of` runs on the parameter names. A refused read at any of these steps is `Step::Failed`, as the first read's is.

- [ ] **Step 1: Write the failing tests** in `find_tests.rs`, driving `Find` with answers as the existing tests do:
  - `the_find_reads_chain_zero_and_its_plug_in_parameters`: the commands are, in order, `parameters` and `chains` on the rack, `devices` on `… chains 0`, then `parameters` on `… chains 0 devices 0`. `Done(Ok(found))` gives the macro and `found.key == Ok(KeyFound { params: Gridder 1/2, key.path == "… devices 0 parameters 1", key.id, … })`.
  - `only_chain_zero_is_searched_for_the_key_device`: the master's rack has a native Auto-Tune in chain 2 and the AudioGridder in chain 0, so the find returns the AudioGridder.
  - `a_rack_without_chains_keeps_its_macro_and_misses_the_key_device`: `found.key == Err(KeyMissing::NoChain)`.
  - `two_plug_ins_in_chain_zero_miss_the_key_device`: `Err(KeyMissing::Plugins(2))`.
  - `a_plug_in_without_key_parameters_misses_the_key_device`: `Err(NoKeyParams)`.
  - `the_find_ends_within_max_reads`: the deepest nesting plus the three new reads end in at most `MAX_READS` answers.
- [ ] **Step 2: Run** `cargo fmt --all --check`.
- [ ] **Step 3: Implement** the new `Ask` variants (`RackParts { path }`, `ChainDevices { path }`, `PluginParameters { path }`), `Found.key`, and `MAX_READS`. Keep the rest of the find as it is.
- [ ] **Step 4: Push** with Task 1. The find tests pass; the existing host test `tests/autotune.rs` still passes.
- [ ] **Step 5: Commit** `feat(hub): the Latencies find reaches chain 0's key device (#140)`

### Task 3: SimLive's song clock, Live's bar, the key devices and a held parameter

**Files:**
- Modify: `sim/Live/__init__.py`, `sim/site_builder.py`, `sim/host.py`
- Create: `sim/tests/test_keys.py`

**Interfaces:**
- **`Song`:**
  - `current_song_time`: observable and settable; a set is a jump.
  - `root_note`: observable, an int 0–11. Anything else raises `RuntimeError("Invalid value")`.
  - `scale_name`: observable, a str.
  - `tempo` already exists.
- **`SongClock` in `host.py`** (a `Live.Base.Timer` at 30 Hz, as `MeterAnimation`): while `is_playing`, it advances `current_song_time` by the wall time since its last tick × tempo / 60 and fires the listener.
- **`DeviceParameter` gains `_sim_enable(on)`.**
  - It sets `is_enabled` and `state` (`_ParameterState.enabled` / `.disabled`).
  - A `value` set on a disabled parameter raises `RuntimeError("Value cannot be set, the parameter is disabled")` and changes nothing.
- **Fixture keys:**
  - a parameter's `"enabled": false`;
  - the song's `"current_song_time"`, `"root_note"` and `"scale_name"`.
- **Host lines** (each answer printed like the others):
  - `song time <beats>`: set as a user's jump; prints `SONG <time>`.
  - `param <lom path> value <v>`: as another writer would; prints `PARAM <value>`, or `PARAM -1` for a bad path or a refused value.
  - `param <lom path> enabled <0|1>`: prints `PARAM <0|1>`, or `PARAM -1`.

  The path reuses `count_listeners`' resolver.

- [ ] **Step 1: Write the failing tests** in `sim/tests/test_keys.py`:
  - `current_song_time` set fires its listener, and the clock advances it at the tempo while playing (two hand-ticked steps of 0.5 s at 120 bpm give +2.0 beats; stopped gives +0);
  - `root_note = 12` raises `Invalid value`, and `scale_name = "Minor"` fires;
  - a disabled parameter refuses a set with the exact text and keeps its value, while `_sim_enable(True)` lets it through;
  - a fixture with `"enabled": false` builds a disabled parameter (`is_enabled` False, `state` disabled);
  - `param … value` fires the parameter's listener, and `param … value` on a disabled one answers `PARAM -1`;
  - `song time 12.5` prints `SONG 12.5`.
- [ ] **Step 2: Run** `python3 -m unittest discover -s sim/tests -p 'test_keys.py'`. Expected: FAIL.
- [ ] **Step 3: Implement** them, following the `_Prop` / `_fire` patterns and `MeterAnimation`. Start the clock in `main()` with the meter animation.
- [ ] **Step 4: Run** `python3 -m unittest discover -s sim/tests` and `ruff check .`. Expected: all pass.
- [ ] **Step 5: Commit** `feat(sim): the song clock, Live's bar and a parameter a map holds (#140)`

### Task 4: The router follows the key devices and the key in force; AUT on and the tap refused

**Files:**
- Create: `crates/fohmixer-hub/src/router/autotune/keys.rs`, `keys_tests.rs`
- Modify:
  - `crates/fohmixer-hub/src/router/autotune.rs`;
  - `crates/fohmixer-hub/src/router.rs`;
  - `crates/fohmixer-hub/src/rules/autotune.rs`;
  - `crates/fohmixer-proto/src/client.rs`;
  - `e2e/tests/autotune.spec.ts`;
  - `.claude/rules/hub-rust.md`.

**Interfaces:**
- Consumes:
  - Task 1's pure items and Task 2's `Found.key`;
  - `SongMapIo.maps[instance].map` (the newest map, stale or not);
  - `AUTOTUNE_CLIENT`;
  - the subscription table.
- **`Target` gains:**
  - `key: Option<Result<KeyFound, String>>`: none until found, `Err` holding the `KeyMissing` words;
  - `key_sub: Option<String>`, `scale_sub: Option<String>`;
  - `values: (Option<f64>, Option<f64>)`.
- **`AutotuneIo` gains `positions: BTreeMap<String, Position>`,** one per set with targets. `Position` is:
  - `time: Option<f64>`, `playing: Option<bool>`, `tempo: Option<f64>`;
  - `time_sub`, `tempo_sub`;
  - `follow: KeyFollow`.
- **`Step` gains:**
  - `Key { instance, song: Option<u16>, key: Option<String>, why: KeyWhy }`: the key's `name()`;
  - `KeyDevice { instance, target, device: KeyDevice, path: String }`;
  - `KeyMissing { instance, target, problem }`: warn class;
  - `Refused { what: String }`: `aut` or `tap`, why `keys only until #140 PR 3`.

  Each has a `step_line` text.
- **Status (`fohmixer_proto::client`), every field `serde(default)`:**
  - `KeyTargetStatus { device: Option<String>, path: Option<String>, values: Option<[f64; 2]>, shows: Option<String>, problem: Option<String> }`;
  - `AutotuneTargetStatus.key: Option<KeyTargetStatus>`;
  - `KeyInForce { instance, song: Option<u16>, key: Option<String>, time: Option<f64>, playing: Option<bool>, tempo: Option<f64> }`;
  - `AutotuneStatus.keys: Vec<KeyInForce>`.
- **Router functions** (`impl Router` in `router/autotune/keys.rs`):
  - `autotune_position_follow(&mut self, instance: &str)`: subscribes `live_set` `current_song_time` and `tempo`. It is called from `autotune_layout` for each set with targets; a set without targets is let go.
  - `autotune_key_found(&mut self, key: &str)`: from `autotune_target_found`. It follows the Key and Scale parameters' `value` at their paths and records `KeyDevice`, or records `KeyMissing` once per new problem (`missing_news`).
  - `autotune_position(&mut self, instance: &str)`: computes `in_force` from the set's newest map and position, and feeds `follow.on_position`. A `Some(why)` is one `Key` record.
  - `autotune_map_read(&mut self, instance: &str)`: called by `songmap_read` after a new map is stored. It runs `autotune_position`.
  - **`autotune_delivery` routes by subscription key:**
    - a macro → as today;
    - a key or scale parameter → `values`;
    - `current_song_time` / `tempo` → the set's `Position`, then `autotune_position`.

    `playing` comes from `autotune_transport`.
  - **Refusals:**
    - `autotune_set_hub(HUB_AUTOTUNE_AUT, true)` and `(HUB_AUTOTUNE, _)` → `Refused`, and the state is told again to that page (its outbox) even when unchanged. Nothing is written and nothing saved.
    - `autotune_aut` false is accepted, as today.
    - `AutotuneIo::new(_)` always starts AUT off. A persisted `true` is logged once (`AUTOTUNE AUT stays off until #140 PR 3`) and saved false.
  - **The offline path:** `autotune_offline` also ends the position subscriptions' values, resets `follow` and drops `values` and `key`.

- [ ] **Step 1: Write the failing router tests** in `router/autotune/keys_tests.rs` (on the connected test router, as `router/autotune/tests.rs` does):
  - **`a_found_target_follows_its_key_and_scale`:** the find's answers through chain 0 give two `add_listener value` commands at the key paths, a `key_device` record, and `/api/status` `targets[0].key.device == "gridder"` with `shows` after the values come.
  - **`a_missing_key_device_is_recorded_once_and_keeps_the_macro`:** `key_missing` once over two walks, and `state` is still from the macro.
  - **`the_key_in_force_follows_the_song_time`:** a map `001 Test One {C 120}` 0–100 plus `Key {G}` at 50. Song-time deliveries 10, 49.9 and 50.1 (playing, tempo 120) give exactly two `key` records, `song` C then `change` G, and status `keys[0].key == "G"`.
  - **`a_stopped_jump_re_evaluates_at_once`:** stopped, a delivery at 60 and then at 10 gives `change` G, then C.
  - **`leaving_the_song_records_no_key`:** a delivery at 150 gives `key: null, why: "no_key"`.
  - **`a_map_read_keeps_the_key_in_force_until_the_new_map`:** the transport change triggers a read; deliveries during it still give the old map's key, with no `no_key` record; the new map's answer runs `autotune_map_read`.
  - **`aut_on_is_refused_and_writes_nothing`:**
    - `set_hub autotune_aut true` gives a `refused` record, and `aut` stays false;
    - no `set_prop` is sent;
    - the page is told the state again;
    - `hub-state.json` is not written.
  - **`a_tap_is_refused_and_writes_nothing`**, the same way.
  - **`a_persisted_aut_on_starts_off`.**
  - **`a_disconnect_drops_the_position_and_key_values`.**
- [ ] **Step 2: Run** `cargo fmt --all --check`. Push the red commit alone and read the failures (`.claude/rules/ci.md`).
- [ ] **Step 3: Implement** it, with the status fields and the `step_line` texts.
- [ ] **Step 4: Adjust `e2e/tests/autotune.spec.ts`:**
  - "a tap turns every target off from mixed…" becomes `a tap is refused: both macros stay as they were`;
  - "a 1 s hold turns AUTOTUNE AUT on…" becomes `a 1 s hold leaves AUT off; a start writes nothing`;
  - "a tap overrides the rule…" is removed: no rule writes in PR 2;
  - the missing-rack test keeps its problem corner and drops "the others still switch".

  The console guard stays. The page itself is unchanged: mockup v2 waits.
- [ ] **Step 5: Push and read CI:** every job green, the mutation shards 0 missed, the coverage floor held.
- [ ] **Step 6: Commit** `feat(hub): follow the key devices and the key in force; AUT on and the tap refused (#140)`

### Task 5: End to end through SimLive, PR 2a, deploy, the PC read

**Files:**
- Create: `crates/fohmixer-hub/tests/autotune_keys.rs` (`#![cfg(unix)]`)
- Modify: `.config/nextest.toml` (add `binary_id(fohmixer-hub::autotune_keys)` to `hub-hosts`), `.claude/rules/hub-rust.md`

**The site** (invented names):

- **Three tracks, each with a `Latencies` rack and a Tuner:**
  - `Stream A #`: `+AUTOTUNE`, the rack inside `Stream FX`, chain 0 holding an AudioGridder plug-in with params `Device On`, `0:(unused):(unassigned)`, `1:(unused):(unassigned)`, `2:(unused):(unassigned)`;
  - `Stream B #`: the same, its slots `"enabled": false`;
  - `Vocal A #`: `+AUTOTUNE:0`, an `Auto-Tune Artist` plug-in with `Device On`, `Key`, `Scale` in chain 0.
- **A `Sections` MIDI track.**
- **Cues:** `001 Test One {C 120 4/4}` 0, `SONG END` 100, `002 Test Two {F#m 90}` 120, `SONG END` 200.
- **A clip** `Key {G}` at 50.
- **A frame** with the AUTOTUNE rail button.

- [ ] **Step 1: Write the host tests:**
  - **`every_target_finds_its_key_device`:** status shows `gridder`, `gridder` and `auto_tune` with their paths, and `shows` from the values.
  - **`the_key_in_force_follows_play_and_jumps`:**
    - `song time 10` gives `key` song 1 C;
    - `start_playing` crosses 50 within 30 s of wall time at 120 bpm (the clock, ~25 s), with a `change` to G;
    - `song time 130` gives song 2 F#m.

    Read through the event log's `autotune` records.
  - **`nothing_is_written_to_live`:** after all of the above, every macro and key parameter holds its fixture value, `root_note` / `scale_name` are unchanged, and the `listeners` counts are one per followed key.
  - **`aut_on_and_a_tap_are_refused_end_to_end`:** a page's `set_hub` gives two `refused` records and no macro moves.
- [ ] **Step 2: Push and read CI.** Then open PR 2a, `dev` → `master`, titled "Autotune keys, read-only: the key devices and the key in force (#140)".
- [ ] **Step 3: Drive CI green** (both runs, `mergeable_state: clean`), then a fresh-context review. Fix its findings in one round.
- [ ] **Step 4: Merge** with `gh pr merge --merge`, tag `v1.10.0`, and bump `dev` to 1.11.0.
- [ ] **Step 5: The PC steps, each on the owner's yes:**
  - the 11 Tuner tags: `+AUTOTUNE` on the 5 mapped stream tracks, `+AUTOTUNE:0` on the 6 PA tracks;
  - the frame edit: the old `param_toggle` out, the autotune rail control in.

  Then the deploy, by `deploy-pc.md`.
- [ ] **Step 6: Post-deploy, read-only:**
  - `/api/status` `autotune`: 11 targets, each with its key device (5 `gridder`, 6 `auto_tune`) and its values;
  - `keys[band]` at the stopped playhead equals verification 2's key for that song;
  - at the next event, the event log's `key` records follow the songs, compared with the song map.

  Record the numbers on #140 (no names). Update `.claude/rules/hub-rust.md` with what the PC showed.

---

## PR 2b: the writes

### Task 6: The value to write and the read-back judgement (pure)

**Files:**
- Modify: `crates/fohmixer-hub/src/rules/autotune/keys.rs`, `keys_tests.rs`

**Interfaces:**
- Produces:
  - `pub const NUDGE: f64 = 0.25;`, `pub const SAME_VALUE_EPS: f64 = 1e-6;`, `pub const LATE_BOUND_MS: f64 = 100.0;`
  - **`pub fn write_value(index: u8, steps: u8, current: Option<f64>) -> f64`:**
    - the centre is `index/steps`;
    - when `current` is within `SAME_VALUE_EPS` of it, the value is `(index + NUDGE)/steps`, or `(index − NUDGE)/steps` when `index == steps`.
  - **`pub fn key_values(key: Key, current: (Option<f64>, Option<f64>)) -> (f64, f64)`:** `write_value` for the root over `KEY_STEPS`, and for the scale index (0 or 1) over `SCALE_STEPS`.
  - **`pub enum Readback { Confirmed, Shows(Option<Key>), Refused(String) }`** (serde: `confirmed`, `shows`, `refused`).
  - **`pub fn judge(want: Key, values: (f64, f64)) -> Readback`:** `Confirmed` when `shows(values) == Some(want)`, else `Shows(shows(values))`.
  - **`pub struct KeyTarget { want: Option<Key>, retried: bool, writing: bool, last: Option<Readback> }`:**
    - `pub fn new() -> Self`.
    - `pub fn want(&mut self, key: Key)`: a key other than the one wanted resets `retried`. The caller writes either way: a song start and a found target write even an equal key.
    - `pub fn sent(&mut self)`: a write went out (`writing`).
    - `pub fn written(&mut self, outcome: Readback) -> Option<KeyWhy>`: ends `writing` and stores `last`. `Some(Retry)` once per key when the outcome is not `Confirmed` and not yet retried.
    - `pub fn heard(&self, values: (f64, f64)) -> bool`: true when not `writing`, a `want` is set, and `judge` is not `Confirmed`. That is another writer.
    - `pub fn state(&self) -> KeyState`: `NoKey` without a want, `Writing` while writing, else from `last`, with `AnotherWriter` once `heard` was recorded (`pub fn another(&mut self)`).
  - **`pub enum KeyState { NoKey, Writing, Confirmed, Mismatch, Refused, AnotherWriter }`** (serde snake_case).
  - **`pub fn late_ms(time: f64, change: f64, tempo: f64, rtt_ms: f64) -> f64`:** `(time − change) × 60000 / tempo + rtt_ms`.

- [ ] **Step 1: Write the failing tests:**

```rust
#[test]
fn the_centre_is_written_unless_it_is_already_there() {
    assert_eq!(write_value(2, 11, None), 2.0 / 11.0);
    assert_eq!(write_value(2, 11, Some(0.0)), 2.0 / 11.0);
    assert_eq!(write_value(2, 11, Some(2.0 / 11.0)), 2.25 / 11.0);
    assert_eq!(write_value(11, 11, Some(1.0)), 10.75 / 11.0);
}
#[test]
fn a_float32_echo_of_the_centre_counts_as_the_centre() {
    assert_eq!(write_value(2, 11, Some(0.1818181872367859)), 2.25 / 11.0); // spike part 2
    assert_eq!(write_value(10, 11, Some(0.9090909361839294)), 10.25 / 11.0);
}
#[test]
fn a_nudged_value_reads_back_as_the_same_key() {
    for root in 0..12u8 {
        for minor in [false, true] {
            let k = Key { root, minor };
            let centre = key_values(k, (None, None));
            let nudged = key_values(k, (Some(centre.0), Some(centre.1)));
            assert_eq!(judge(k, centre), Readback::Confirmed);
            assert_eq!(judge(k, nudged), Readback::Confirmed, "{k:?}");
            assert_ne!(centre, nudged);
        }
    }
}
#[test]
fn a_wrong_step_is_shown_not_confirmed() {
    let a = Key { root: 9, minor: false };
    assert_eq!(judge(a, (2.0 / 11.0, 0.0)), Readback::Shows(Some(Key { root: 2, minor: false })));
    assert_eq!(judge(a, (9.0 / 11.0, 2.0 / 28.0)), Readback::Shows(None));
}
#[test]
fn a_mismatch_is_retried_once_per_key() {
    let a = Key { root: 9, minor: false };
    let mut t = KeyTarget::new();
    t.want(a);
    t.sent();
    assert_eq!(t.state(), KeyState::Writing);
    assert_eq!(t.written(Readback::Refused("Value cannot be set, the parameter is disabled".into())), Some(KeyWhy::Retry));
    assert_eq!(t.written(Readback::Refused("Value cannot be set, the parameter is disabled".into())), None);
    assert_eq!(t.state(), KeyState::Refused);
    t.want(Key { root: 2, minor: false });
    assert_eq!(t.written(Readback::Shows(None)), Some(KeyWhy::Retry));
}
#[test]
fn a_value_heard_between_writes_is_another_writer() { /* want A, written Confirmed, heard (2/11, 0) → true; while writing → false; heard A's values → false */ }
#[test]
fn lateness_counts_beats_at_the_tempo_plus_the_round_trip() {
    assert_eq!(late_ms(22897.0, 22897.0, 120.0, 12.0), 12.0);
    assert!((late_ms(22896.93, 22897.0, 120.0, 12.0) - (-23.0)).abs() < 1e-6);
}
```

  The bodies given as comments are written out the same way as the full ones, with the exact values in the comment.
- [ ] **Step 2–4:** fmt, implement, then push with Task 7 and read CI.
- [ ] **Step 5: Commit** `feat(hub): the key's value, its read-back and one retry (#140)`

### Task 7: The router writes and reads back

**Files:**
- Modify:
  - `crates/fohmixer-hub/src/router/autotune/keys.rs`, `keys_tests.rs`;
  - `crates/fohmixer-hub/src/router.rs`;
  - `crates/fohmixer-hub/src/rules/autotune.rs`;
  - `crates/fohmixer-proto/src/client.rs`.

**Interfaces:**
- **Each target gains `keyt: KeyTarget`.**
- **`Step` gains:**
  - `KeyWrite { instance, key, why: KeyWhy, targets: Vec<String>, bar: bool }`;
  - `Readback { instance, target, key, state: KeyState, shows: Option<String>, problem: Option<String> }`: warn unless `confirmed`;
  - `AnotherWriter { instance, target, shows: Option<String> }`: warn;
  - `Late { instance, song, ms }`: warn when `ms > LATE_BOUND_MS`;
  - `Bar { instance, state: "confirmed" | "shows", shows }`: info. Live's bar gates nothing.
- **`RouterMsg` gains:**
  - `AutotuneKeysWritten { instance, write: u64, keys: Vec<String>, outcome, rtt_ms: f64 }`;
  - `AutotuneKeysRead { instance, write: u64, keys: Vec<String>, outcome }`.

  `write` voids an older answer, as the walk's number does.
- **`autotune_key_write(&mut self, instance: &str, keys: &[String], why: KeyWhy)`:**
  - held while `!may_write(holding, settled)`, as the macro writes are (`autotune_hold`);
  - each target's `keyt.want(key)` and `keyt.sent()`, then one `call` with each found key device's two `set_prop value` through `{"$ref", "class": "DeviceParameter"}`, at `key_values(want, values)`;
  - plus, for the set's own write (not a single found target's), `live_set` `root_note` and `scale_name`;
  - recorded `KeyWrite` first.
- **On the answer:**
  - a slot refused → `written(Refused(why))`;
  - a `StaleRef` also walks the target again;
  - the others get one `call` of `get_prop value` per parameter through its `$ref`, plus `root_note` and `scale_name`. Its answer → `judge` → `written(..)`;
  - a `Some(Retry)` writes that target again at once with why `retry`;
  - each target's outcome is one `Readback` record. A `change` write with a known tempo also records `Late` with `late_ms(time at the decision, change_time, tempo, rtt_ms)`. With no tempo heard there is no `Late` record.
- **When it writes** (`why`):
  - `autotune_position`'s `Song` and `Change` → every found target of the set;
  - `NoKey` → nothing: the keys stay as they are;
  - a target found anew (another `$ref`, or none written since a disconnect or a hold) → that target alone, with the key in force, `found`;
  - a hold's release → the set's targets, why `found`, through the walk.
- **Another writer:** a key or scale delivery with `keyt.heard(values)` records `AnotherWriter` once per key and is never written against. The next song start or change rewrites as usual.
- **Status:** `KeyTargetStatus` gains `want: Option<String>` and `state: Option<KeyState>`. `KeyInForce` gains `bar: Option<String>`, which Live's bar shows.

- [ ] **Step 1: Write the failing router tests:**
  - **`a_song_start_writes_every_key_device_and_the_bar`:** one call with 4 `set_prop`s by `$ref` (two targets × key/scale) plus `root_note` 0 and `scale_name` "Major". The read-back call follows; with the answers, each `readback` is `confirmed` and status `state` is `confirmed`.
  - **`an_equal_key_at_the_next_song_start_is_written_nudged`:** the second song in the same key gets `2.25/11`-style values.
  - **`a_refused_write_is_a_mismatch_retried_once`:** the slot answers the disabled error. One more write follows (`retry`), then no third. Both are `readback` records with state `refused` and the exact problem text.
  - **`a_wrong_read_back_is_a_mismatch_retried_once`.**
  - **`another_writer_is_recorded_and_not_fought`:** a delivery of other values after `confirmed` gives one `another_writer` and no write.
  - **`a_key_change_records_its_lateness`.**
  - **`writes_are_held_while_the_markers_are_unsettled`:** a track-list change, then a song start writes nothing until the release; the release writes once.
  - **`a_stale_ref_walks_the_target_again_and_writes_it_found`.**
  - **`no_selector_is_ever_written`:** across all of the above, no `set_prop` on a macro's `$ref`.
- [ ] **Step 2: Run** `cargo fmt --all --check`. Push the red commit alone and read the failures.
- [ ] **Step 3: Implement** it.
- [ ] **Step 4: Push and read CI** to green, with 0 missed.
- [ ] **Step 5: Commit** `feat(hub): write the key in force to every Auto-Tune and Live's bar, read back (#140)`

### Task 8: End to end through SimLive

**Files:**
- Modify: `crates/fohmixer-hub/tests/autotune_keys.rs`, `.claude/rules/hub-rust.md`

- [ ] **Step 1: Write the host tests** (Task 5's site):
  - **`a_song_start_writes_and_confirms_the_key`:**
    - `song time 10` → `Stream A`'s slots `0.0`/`0.0`, `Vocal A`'s Key/Scale the same;
    - `root_note` 0 and `scale_name` "Major";
    - status `confirmed` for both, and `refused` for `Stream B` (disabled), with one retry in the event log.
  - **`a_key_clip_is_written_within_the_lateness_bound`:**
    - playing from 45 at 120 bpm: G reaches every slot after the crossing of 50;
    - the `late` record's `ms` is ≤ 100;
    - the slots read G.
  - **`a_released_map_lets_the_write_through`:**
    - `param <Stream B slot 0> enabled 1` and the same for slot 1, then `song time 130`;
    - `Stream B` is `confirmed` F#m: 6/11 and 1/28.
  - **`another_writer_is_shown_not_fought`:**
    - `param <Stream A slot 0> value 0.9` → `another_writer` and status `another_writer`;
    - the slot stays 0.9 until the next song start rewrites it.
  - **`a_reconnect_writes_the_key_again`:** a host restart with the same fixture is a set load. The walk finds the targets and the key in force is written (`found`) once.
  - **`no_selector_moves`:** every macro holds its fixture value at the end.
- [ ] **Step 2: Push and read CI.**
- [ ] **Step 3: Update `.claude/rules/hub-rust.md`:** the write path, the nudge, the held-parameter refusal, lateness, and what never writes.
- [ ] **Step 4: Commit** `test(hub): keys written and read back end to end through SimLive (#140)`

### Task 9: PR 2b, the maps off, deploy, the PC check

- [ ] **Step 1: Open the PR** `dev` → `master`, titled "Autotune keys: write and read back every Auto-Tune's key (#140)". The body says that no selector moves and AUT stays off.
- [ ] **Step 2: Drive CI green.** Then a fresh-context review and one fix round.
- [ ] **Step 3: Merge, tag `v1.11.0`, and bump `dev` to 1.12.0.**
- [ ] **Step 4: Before the deploy, on the PC, each step on the owner's yes, Live stopped:**
  - switch map A off. Read one of its stream slots' `is_enabled`: it must turn `true`. Then switch it on again: that is spec §5's open check, read-only apart from the switch;
  - with the owner's yes for the whole step, switch maps A–D off, then show that no audio path or latency changed (the reported latencies before and after, and the file diff after Ctrl+S);
  - switch on the 3 PA Auto-Tune Artists still off (tracks 16, 17, 19).
- [ ] **Step 5: Deploy** by `deploy-pc.md`.
- [ ] **Step 6: Post-deploy, Live stopped:**
  - `/api/status` `autotune` shows all 11 targets `confirmed` for the key at the playhead, and `keys[band].bar` equals it;
  - two Auto-Tune windows are looked at through `is_editor_open` (one stream, one PA), with no click: they show that key;
  - every selector still reads its DRY value.

  Record it on #140 (no names).
