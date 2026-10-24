# Autotune switched by the hub (PR 3 of #140) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** the hub switches each `+AUTOTUNE` track between its owner-tuned chains by the spec's two-state rule. The FohMixer script falls back to DRY within 2 s when the hub is gone. The AUTOTUNE button and its singers' panel follow approved mockup v2.

**Architecture:**

- **PR 3a, the fallback.** The script keeps a per-connection list of "safe" writes the hub registers: every found selector's `$ref` with its off value.
  - It performs them when that connection ends, or when the hub has said nothing for `HUB_SILENCE_MS`. The hub pings its script connections every `KEEPALIVE_MS`.
- **PR 3b, the rule.** A pure per-track decision (`rules/autotune/switch.rs`) replaces 1.8.0's transport-only `AutotuneAut` writes.
  - The router feeds it every input: AUT, transport, key in force, the key's read-back, the singer's manual off, the target's problems, a selector moved by another writer.
  - It writes a macro only when the decision differs from what Live shows.
  - The protocol, `/api/status` and `hub-state.json` grow as spec §9 says.
- **PR 3c, the surface.** `behave/autotune.rs` and the rail's AUTOTUNE button are rebuilt on mockup v2:
  - a 1 s hold switches AUT at its lift;
  - a tap opens the singers' panel;
  - Playwright E2E covers it on Chromium and the iPad WebKit project, phones included.

**Tech Stack:** Rust (fohmixer-proto, fohmixer-hub: tokio, serde; fohmixer-ui: Leptos), Python 3.11 (the FohMixer script, SimLive), nextest host tests, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-23-autotune-key-design.md`. Read §2.2–2.4, §5, §7, §8, §9 and §11 first. The surface is `docs/mockups/autotune-v2.html` (approved 2026-10-23). The English terms are #141's decisions of 2026-10-24.

## Where it starts *(read before Task 1)*

- **`dev` after the retry fix** ("the connect-time reads retry until Live answers", 1.12.0). The connect-time find and the song map read retry by themselves until Live answers.
- **The keys (PR 2a/2b) are live on the PC (1.11.0):**
  - `KeyTarget` (`want`, `state()`, one retry);
  - `KeyFollow`, `in_force`, the hold (`may_write`, `autotune_hold`, `autotune_release`), M1 to M4;
  - `Target.key_sure`, `current_maps`.
  - A target is key-confirmed for the key in force only when `keyt.wanted() == key_in_force && keyt.state() == KeyState::Confirmed` (PR #144 review M2).
- **1.8.0's `AutotuneAut`** (`rules/autotune.rs`) holds the flag, each set's transport, `written`/`last`, the dead `tap` path (PR 2a review M10) and `writes`/`taps`.
  - The macro writes go through `write_commands` by `$ref` in one batch (`autotune_write_keys`, `RouterMsg::AutotuneWritten`).
- **The PC (2026-10-24, #140 comments):**
  - 12 targets: band tracks 2–7 are stream, `+AUTOTUNE`, on 127 / off 0; tracks 15–20 are PA, `+AUTOTUNE:0`, on 0 / off 127.
  - Every stream autotune chain is selectable at 127. Every PA `Effects 2.3` is at 0 and `DRY 2.3` at 1–127.
  - The PA Tuners carry `+ID:<id>` and the stream Tuners `+MUTE:<id>`. These are #127's pairs: 6 singers, each with one stream and one PA track.
  - Cue times from song 009 on carry +5.2e-5 beat. A playhead test uses Live's own value, never a rounded beat.
  - The main session restarts the band Live itself to load a script change (the owner's standing ruling, 24.10.2026).

## The split *(call)*

Three PRs, each green, deployed and verified on its own, in this order:

- **PR 3a, the fallback (Tasks 1–4), about 900 lines with tests.** The spec makes it its own PR (§8), and it ships first: no selector can be switched on before a lost hub is known to switch it back.
- **PR 3b, the rule (Tasks 5–9), about 1500 lines.** It covers the decision, the selector writes, another writer, the hold, the singers, the protocol, the status and the state.
  - The page keeps 1.8.0's button: a hold sets AUT, a tap is refused. So 3b can be checked at a stopped Live without the new surface.
- **PR 3c, the surface (Tasks 10–12), about 1300 lines.** It needs 3b's protocol.

Rejected: one PR, too big to review (about 3700 lines). Also rejected: the rule before the fallback, because a hub restart for a deploy could leave a singer tuned through speech.

## Global Constraints

- **Tier 0:** no local cargo compilation. Locally only `cargo fmt --all`, `ruff check .`, the Python tests (`python3 -m unittest discover -s live-script/tests`, `sim/tests`), `scripts/check_integrity.py`, `scripts/check_version.py` and `npx playwright test --list`.
- **Public repo:** invented names only: `Vox 1`, `Vocal A #`, `001 Test One {C 120 4/4}`.
  - Before every push run `python3 scripts/denylist_scan.py --denylist <list> --identities scripts/allowed-identities.txt --boundary scripts/denylist-boundary.txt --accepted scripts/denylist-accepted.txt --tree HEAD --commits HEAD`.
- **Only the `Latencies` rack's first macro (the chain selector) is switched (§2.3).** A device is never switched off, removed or replaced. Nothing writes a key device in this plan beyond PR 2b's key writes.
- **Writes go only by `$ref`,** never by an index path (PR #139 review I1). The script's fallback holds the objects it resolved at registration.
- **English** for every text the surface shows, every record field and log line. The surface's texts are mockup v2's, with #141's terms (`MAN`, `AUTO`, `normal`, `open`).
- **Versions:**
  - the retry fix ships as 1.12.0;
  - PR 3a bumps to 1.13.0 as its first commit after that merge, PR 3b to 1.14.0 and PR 3c to 1.15.0, each as the first commit after the previous merge.
- **Commits:**
  - the noreply identity, with `(#140)` and `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`;
  - a bug fixed in review goes `[red]` before `[green]`;
  - no history rewrite.
- **The mutation timeout is 200 s** (`ci.md`). Measure the warmup again when the hub suite passes ~100 s, and set the timeout to 2× that.

## Review Focus

The input classes most likely to bite a person, each pinned by a test in the task named:

1. **A hub connection that drops while Live keeps running** (a hub crash or a deploy's restart). Every selector is back on DRY within 2 s, and the hub's next decision after its reconnect puts back only what the rule wants (Task 2 `a_lost_connection_writes_every_safe_write_once`, Task 4 `a_killed_hub_leaves_every_selector_on_dry_within_2_s`, Task 8 `a_reconnect_decides_again_and_switches_on_only_after_the_keys`).
2. **A stop in the middle of a song.** DRY at once, with no wait for any read-back (Task 6 `stop_is_dry_at_once_whatever_else`, Task 8 `a_stop_mid_song_writes_dry_at_once`).
3. **A key clip passed while a singer tunes.** The singer stays on through the new key's write. A read-back that then disagrees sends that one track to DRY, and the singer's other track stays on (Task 6 `a_tracking_track_stays_on_through_a_key_change_until_its_read_back_disagrees`, Task 8 `a_key_change_keeps_tuning_and_a_bad_read_back_drops_only_that_track`).
4. **A selector moved by something else.**
   - Hub wants DRY and Live shows autotune: DRY again once, then it stops after a second move within 10 s.
   - Hub wants autotune and Live shows DRY: left alone.
   - (Task 6 `another_writer_turns_dry_once_and_gives_up_on_a_second_move_within_10_s`, Task 8 `another_writer_is_shown_and_dry_wins`.)
5. **The surface right after a reconnect.** Until the hub tells this connection AUTOTUNE's state, a hold and a tap send nothing, and the panel shows no stale singer (Task 11 `after_a_reconnect_a_hold_and_a_tap_send_nothing_until_the_state`).

---

## File Structure

- **PR 3a:**
  - `live-script/FohMixer/safe.py` (new: `SafeWrites`);
  - `live-script/FohMixer/lom/ops.py` (the `safe_writes` handler);
  - `live-script/FohMixer/surface.py` (wiring: the drop hook, the silence check);
  - `live-script/FohMixer/transport/server.py` (`Connection.last_heard_s`);
  - `live-script/tests/test_safe.py`;
  - `crates/fohmixer-hub/src/live/client.rs` (the keep-alive ping);
  - `crates/fohmixer-hub/src/router/autotune/safe.rs` (new: the registration);
  - `crates/fohmixer-hub/tests/autotune_safe.rs` (new host test);
  - `.claude/rules/live-script.md`, `.claude/rules/hub-rust.md`, `.claude/rules/deploy-pc.md`.
- **PR 3b:**
  - `crates/fohmixer-proto/src/layout.rs` (`AutotuneTarget.singer`);
  - `crates/fohmixer-proto/src/markers.rs` (singer ids, the `TwoSingers` problem);
  - `crates/fohmixer-proto/src/client.rs` (the protocol and status types);
  - `crates/fohmixer-hub/src/rules/autotune/switch.rs` (new: the pure decision and the selector guard; tests `switch_tests.rs`);
  - `crates/fohmixer-hub/src/rules/autotune.rs` (`AutotuneAut` slimmed, `Step`);
  - `crates/fohmixer-hub/src/router/autotune.rs` and `router/autotune/switch.rs` (new glue);
  - `crates/fohmixer-hub/src/rules.rs` (`HubState.autotune_off`);
  - `crates/fohmixer-hub/tests/autotune_switch.rs` (new host test).
- **PR 3c:**
  - `crates/fohmixer-ui/src/behave/autotune.rs` (rewritten);
  - `crates/fohmixer-ui/src/components/rail.rs`;
  - `crates/fohmixer-ui/src/components/autotune_panel.rs` (new);
  - the store's `HubAutotune`;
  - `crates/fohmixer-ui/style/*.css` (the panel, from the mockup's CSS);
  - `e2e/tests/autotune.spec.ts` (rewritten).

---

## PR 3a: the fall back to DRY

### Task 1: The script keeps and performs the hub's safe writes

**Files:**
- Create: `live-script/FohMixer/safe.py`
- Modify: `live-script/FohMixer/lom/ops.py`, `live-script/FohMixer/surface.py`
- Test: `live-script/tests/test_safe.py`

**Interfaces:**
- **Produces:**
  - **`class SafeWrites`:**
    - `arm(conn, items) -> int`, where `items` is a list of `(obj, value)`; it replaces that connection's list and returns its length; an empty list disarms;
    - `fire(conn, why) -> int`: writes each still-alive `obj.value = value` once, logs one WARNING `hub lost (<why>): set <n> autotune selectors to DRY`, clears the list, and returns n;
    - `armed(conn) -> bool`.
  - **The script command `safe_writes`** (an `ops._HANDLERS` entry). Its args are `{"writes": [{"target": {"$ref", "class"}, "value": number}]}`.
    - It resolves every target now through `lom_path.resolve`. Any that fails fails the whole command (`OpError("bad args", "<n> safe write target(s) do not resolve")`) and leaves the old list in place.
    - Otherwise it calls `SafeWrites.arm(ctx.connection, …)` and returns `{"armed": n}`.
  - **`Context` gains `safe`,** the surface's `SafeWrites`.

- [ ] **Step 1: Write the failing tests** (`test_safe.py`, on sim's `Live` fakes as `test_ops.py` does):
  - **`test_arming_replaces_the_list_and_an_empty_one_disarms`:**
    - `arm(c, [(p1, 0.0), (p2, 127.0)]) == 2`, then `armed(c)`;
    - `arm(c, []) == 0`, then `not armed(c)`.
  - **`test_fire_writes_each_once_and_clears`:**
    - `p1.value == 0.0`, `p2.value == 127.0`, and the return is 2;
    - a second `fire` returns 0.
  - **`test_a_deleted_object_is_skipped`:** an object whose liveness check (`registry.is_alive`) is false is not written and not counted.
  - **`test_one_connections_list_is_its_own`:** firing c1 leaves c2 armed.
  - **`test_the_command_refuses_a_target_that_does_not_resolve_and_keeps_the_old_list`:** the result is `{"ok": False, "errorType": "bad args"}` and the old list is still armed.
- [ ] **Step 2:** run `python3 -m unittest live-script/tests/test_safe.py`. It fails (no module).
- [ ] **Step 3: Implement `safe.py` and the handler.**
  - In `surface.py`, `Drain`'s `on_disconnect` argument (today `self._subs.drop_connection`) becomes a method that calls `self._safe.fire(conn, "disconnect")` FIRST, then `self._subs.drop_connection(conn)`.
  - `disconnect()` (Live unloading the script) fires nothing: Live is closing the set, and the selectors are saved with it.
  - The "record in Live's log" of spec §8 is the script's own log file (`log.py`: `logs/fohmixer-<INSTANCE>.log`, Live's `Log.txt` only as its fallback) *(call: one log for every script event)*.
- [ ] **Step 4:** the tests pass; `ruff check .` is clean.
- [ ] **Step 5: Commit** `feat(script): keep and perform the hub's safe writes when it is gone (#140)`

### Task 2: A silent hub counts as gone

**Files:**
- Modify: `live-script/FohMixer/transport/server.py`, `live-script/FohMixer/surface.py`, `crates/fohmixer-hub/src/live/client.rs`
- Test: `live-script/tests/test_safe.py`, `live-script/tests/test_transport.py`, the hub's `live/client` unit tests

**Interfaces:**
- **Produces:**
  - `Connection.last_heard_s`: the `time.monotonic()` at which `receive()` last read any frame (text, ping, pong). It is set at the handshake.
  - `HUB_SILENCE_MS = 1500` (`Config.py`).
  - The surface's tick runs `self._safe.check_silence(now, connections)` AFTER `poll_in`, so a Live stall that buffered the hub's pings never reads as silence. It fires every armed connection whose `now - last_heard_s > HUB_SILENCE_MS / 1000`, with why `silence`.
  - The hub's Live client sends a WebSocket ping every `KEEPALIVE_MS = 500` from its writer task (`write_requests`), never from the session's `select!` loop (#9).
- [ ] **Step 1: Write the failing tests:**
  - **`test_silence_fires_an_armed_connection_after_1_5_s_only`:**
    - `last_heard` at t = 0, a check at 1.4 s fires nothing;
    - a check at 1.6 s writes DRY and returns `silence`;
    - an unarmed connection never fires.
  - **`test_a_frame_read_after_a_stall_counts_as_heard`:** `receive()` of a ping at t = 5.0 sets `last_heard_s` to 5.0, so the check at 5.0 fires nothing.
  - **`a_lost_connection_writes_every_safe_write_once`** (an integration test on the raw client of `test_integration.py`):
    - the client arms 2 targets, then closes;
    - within the next tick both are written, and the script's log has one `hub lost (disconnect)` line.
  - **Hub unit test `the_writer_pings_every_500_ms`:** on tokio's paused clock, the writer sends a ping at 500 and 1000 ms with no request queued.
- [ ] **Step 2–4:** fail, implement, pass. Run the Python tests locally; the hub test runs in CI.
- [ ] **Step 5: Commit** `feat(script): a hub silent for 1.5 s counts as gone; the hub pings every 500 ms (#140)`

### Task 3: The hub registers every found selector's safe write

**Files:**
- Create: `crates/fohmixer-hub/src/router/autotune/safe.rs` (an `impl Router` block, tests in `safe_tests.rs`)
- Modify: `crates/fohmixer-hub/src/router/autotune.rs`, `crates/fohmixer-hub/src/rules/autotune.rs`

**Interfaces:**
- **Produces:**
  - **`pub(super) fn safe_commands(writes: &[(&str, u8)]) -> Vec<Value>`:**
    - one command `{"target": "live_set", "name": "safe_writes", "args": {"writes": [{"target": {"$ref": id, "class": "DeviceParameter"}, "value": off}]}}`;
    - the list is in the layout's order.
  - **`fn autotune_safe(&mut self, instance: &str)`:**
    - registers the set's found macros with their `spec.off` values, whatever AUT is;
    - it runs after each `autotune_target_found`, `autotune_target_missing`, `autotune_layout` and `autotune_connected` whose found set changed;
    - it is skipped when the list equals the one registered last (`AutotuneIo.safe: BTreeMap<String, Vec<(String, u8)>>`, dropped at `autotune_offline`).
  - **`Step::Safe { instance, targets: usize, problem: Option<String> }`:** a warn when the registration failed. A failure keeps nothing registered on the hub's side, so the next change registers again.
- [ ] **Step 1: Write the failing router tests:**
  - **`every_found_selector_is_registered_with_its_off_value`:** for the `tagged()` fixture, after both targets are found, the batch holds one `safe_writes` with the 2 refs and values 0 and 127.
  - **`an_unchanged_list_is_not_sent_again`:** a second find of the same macros sends no `safe_writes`.
  - **`a_lost_rack_shrinks_the_list`:** a target goes missing, so the list has 1 entry.
  - **`a_disconnect_forgets_the_registration`:** after a connect the full list is sent again.
- [ ] **Step 2–4:** fail, implement, pass in CI.
- [ ] **Step 5: Commit** `feat(hub): register every found selector's DRY write with the script (#140)`

### Task 4: End to end, PR 3a, deploy and the PC check

**Files:**
- Create: `crates/fohmixer-hub/tests/autotune_safe.rs` (add it to the `hub-hosts` group in `.config/nextest.toml`)
- Modify: `.claude/rules/live-script.md`, `.claude/rules/hub-rust.md`, `.claude/rules/deploy-pc.md`

- [ ] **Step 1: The host test `a_killed_hub_leaves_every_selector_on_dry_within_2_s`** (Task 5 of the PR 2 plan's site, through SimLive):
  - the hub finds both stream targets and registers them;
  - `Host::param` sets Stream A's macro to 127 (the "on" value);
  - the test drops the `TestHub` (a kill);
  - within 2 s, Stream A's macro reads 0 through a raw `get` of the host's script;
  - the script's log has `hub lost (disconnect): set 2 autotune selectors to DRY`.
- [ ] **Step 2:** push, open the PR "Autotune: the script falls back to DRY when the hub is gone (#140)", and drive both runs green (0 missed).
- [ ] **Step 3:** review, one fix round, merge, tag `v1.13.0`, bump to 1.14.0.
- [ ] **Step 4: Deploy by `deploy-pc.md`, then restart the band Live** (the script changed; the main session does it, Live stopped, the set saved first).
- [ ] **Step 5: The PC check** (Live stopped, the owner's standing approval):
  1. The hub log has `script_version=1.13.0` for band. Its event log has one `safe` record with 12 targets.
  2. Set band track 5's stream selector macro to 127 through the LOM. Only that one: it is on DRY by the hub's rules, and Live is stopped.
  3. Stop the hub with `\fohmixer\fohmixer-hub-stop`, time it, and read the macro again: 0 within 2 s.
  4. The script's log (`logs/fohmixer-band.log` beside the script) has the `hub lost` line.
  5. Start the hub. `/api/status` shows every target on DRY, and the `safe` record is there again.
  6. Record it on #140 (no names).

---

## PR 3b: the rule

### Task 5: Each target knows its singer

**Files:**
- Modify: `crates/fohmixer-proto/src/layout.rs`, `crates/fohmixer-proto/src/markers.rs`; tests in `markers/tests.rs`

**Interfaces:**
- **Produces:**
  - **`AutotuneTarget.singer: Option<String>`** (`serde(default, skip_serializing_if = "Option::is_none")`):
    - the marker's own `+ID`, else its one `+MUTE` id, else none (a singer of its own, named by its label);
    - a `+AUTOTUNE` Tuner with two or more `+MUTE` ids gets the problem `TwoSingers` (`markers::Problem` code `two_singers`) and `singer: None`.
  - **`pub fn singer_of(target: &AutotuneTarget) -> String`** (in `layout.rs`): the singer id, or `#<label>` for a singer of its own.
- [ ] **Step 1: Write the failing tests:**
  - **`a_pa_tuner_names_its_singer_by_its_id`:** `+G:STAGE:2 +ID:VOX1 +AUTOTUNE:0` gives `Some("VOX1")`.
  - **`a_stream_tuner_names_its_singer_by_its_one_mute`:** `+MUTE:VOX1 +AUTOTUNE` gives `Some("VOX1")`.
  - **`two_mutes_on_an_autotune_tuner_are_a_problem`:** the `two_singers` problem, with the singer none.
  - **`a_tagged_track_without_ids_is_a_singer_of_its_own`:** `singer_of` gives `#Vox 7`.
- [ ] **Step 2–4:** fail, implement (`autotune_targets` fills it), pass in CI.
- [ ] **Step 5: Commit** `feat(proto): each autotune target names its singer (#140)`

### Task 6: The decision (pure)

**Files:**
- Create: `crates/fohmixer-hub/src/rules/autotune/switch.rs`, `switch_tests.rs`

**Interfaces:**
- **Produces:**
  - **`pub enum SwitchWhy`** (serde snake_case), the decision's reason: `Aut, Play, Stop, Span, NoKey, SingerOff, Problem, Mismatch, AnotherWriter`.
    - Spec §7's other `why` names, `found`, `reconnect` and `jump`, are what made the hub decide again. They go into the `Select` record's `cause` (Task 7), never into a decision *(call: no variant that `decide` never returns)*.
  - **`pub struct TrackInput`:**
    - `aut: bool`;
    - `playing: Option<bool>`;
    - `in_force: InForce` (PR 2a);
    - `singer_off: bool`;
    - `problem: bool`: a missing rack or key device, `TwoSingers`, or the selector guard given up;
    - `key: KeyCheck`;
    - `on_now: bool`: the macro as Live shows it (`target_state` On).
  - **`pub enum KeyCheck { Confirmed, Pending, Bad }`:**
    - `Confirmed` is `want == key in force` with `state() == Confirmed`;
    - `Pending` is a write on its way, or `want` not yet the key in force;
    - `Bad` is `Mismatch | Refused | AnotherWriter` for the key in force.
  - **`pub struct Decision { pub on: bool, pub why: SwitchWhy }`.**
  - **`pub fn decide(input: &TrackInput) -> Decision`:** the first failing condition, in this order:
    1. `!aut` → `Aut`
    2. `playing != Some(true)` → `Stop`
    3. no song → `Span`
    4. a keyless song → `NoKey`
    5. `singer_off` → `SingerOff`
    6. `problem` → `Problem`
    7. `key == Bad` → `Mismatch`
    8. `key == Pending && !on_now` → off, `Mismatch` *(call: "not yet confirmed" reads as `mismatch` in the record; the surface says `checking`)*

    Otherwise on, `Play`. So a track that is on stays on while its new key's write is `Pending` (§7), and goes off once that write reads back `Bad`.
  - **`pub struct SelectorGuard`:** one per target.
    - `pub fn moved(&mut self, now_ms: f64) -> GuardAct`, where `GuardAct` is `WriteDryOnce | GiveUp`, takes a heard move to autotune that the hub did not write while it wants DRY.
    - The first move gives `WriteDryOnce`. A second within `SELECTOR_GUARD_MS = 10_000` of the first gives `GiveUp`.
    - `given_up()`, and `reset()` at the next song start.
- [ ] **Step 1: Write the failing tests** (the exhaustive table, one test per row group):
  - **`aut_off_is_dry_always`:** every combination with `aut: false` gives `{on: false, why: Aut}`.
  - **`stop_is_dry_at_once_whatever_else`:** `playing: Some(false)` and `None` give `Stop`, even with `on_now` and `Confirmed`.
  - **`out_of_every_song_is_span_and_a_keyless_song_no_key`.**
  - **`a_singer_off_by_hand_is_dry`, `a_problem_is_dry`.**
  - **`on_needs_the_key_confirmed_for_the_key_in_force`:** `Pending` with `on_now: false` is off; `Confirmed` is on, `Play`.
  - **`a_tracking_track_stays_on_through_a_key_change_until_its_read_back_disagrees`:**
    - `on_now: true` with `Pending` is on;
    - `on_now: true` with `Bad` is off, `Mismatch`.
  - **`another_writer_turns_dry_once_and_gives_up_on_a_second_move_within_10_s`:**
    - `moved(0)` gives `WriteDryOnce`;
    - `moved(9_999)` gives `GiveUp`;
    - after `reset()`, `moved(20_000)` gives `WriteDryOnce`;
    - with a fresh guard, `moved(0)` then `moved(10_001)` gives `WriteDryOnce`.
- [ ] **Step 2–4:** fail, implement, pass in CI. Every branch is a mutation target, so the table is complete.
- [ ] **Step 5: Commit** `feat(hub): the per-track autotune decision and the selector guard (#140)`

### Task 7: The rule state, the singers' manual off, AUT kept across a restart

**Files:**
- Modify: `crates/fohmixer-hub/src/rules/autotune.rs`, `rules/autotune/tests.rs`, `crates/fohmixer-hub/src/rules.rs`

**Interfaces:**
- **`AutotuneAut` keeps:** `on`, each set's `playing`, `transport_lost`, `on_disconnect`, `playing()`, `writes()`.
  - **Removed (PR 2a review M10):** `tap`, `taps`, `Followed.written` / `last`, `wanted`, `Why::Tap` / `Step::Tap`, `decide`, `follow`'s writes (`follow` returns nothing now), and `AutotuneStatus.taps` (kept in the wire type with `serde(default)` for an old page, always 0).
  - `set_flag(on)` returns nothing: the router decides every target.
- **`AutotuneAut` adds:** `singers_off: BTreeSet<String>`, with `set_singer(id, off) -> bool` (changed) and `is_off(id)`.
- **`HubState.autotune_off: Vec<String>`:** `serde(default, skip_serializing_if = "Vec::is_empty")`, sorted.
  - **`AutotuneIo::new(saved: bool, off: Vec<String>)`:** AUT starts as saved. 2b's forced off and its warn line go.
- **`Step` changes:**
  - adds `Select { instance, target, on, why: SwitchWhy, cause: String }`, where `cause` is `transport | position | key | flag | singer | found | reconnect | jump | guard | release`;
  - adds `SelectorWriter { instance, target, act: String }`, a warn;
  - adds `Singer { id, off }`;
  - `Write` / `Failed` stay for the macro batch.
  - `REFUSED_WHY` becomes `"the AUTOTUNE tap is gone; a tap opens the singers' panel"`. Only `autotune` (the tap key) is refused now.
- [ ] **Step 1: Write the failing tests:**
  - `a_singer_off_is_kept_and_saved_sorted`;
  - `aut_is_kept_across_a_restart`: `HubState` round trip with `autotune_aut: true`, and `AutotuneIo::new(true, vec![])` is on;
  - `a_tap_is_refused_and_aut_on_is_taken`.
- [ ] **Step 2–4:** fail, implement, pass in CI.
- [ ] **Step 5: Commit** `feat(hub): AUT kept across a restart, the singers' manual off; 1.8.0's tap removed (#140)`

### Task 8: The router decides and writes every target

**Files:**
- Create: `crates/fohmixer-hub/src/router/autotune/switch.rs` (`impl Router`, tests `switch_tests.rs`)
- Modify: `crates/fohmixer-hub/src/router/autotune.rs`, `router/autotune/key_writes.rs`, `router/autotune/keys.rs`

**Interfaces:**
- **`Target` gains:**
  - `decision: Option<Decision>`, the last one written or held;
  - `guard: SelectorGuard`;
  - `wrote_on: Option<bool>`: what the hub's last write set, to tell its own echo from another writer.
- **`fn autotune_decide(&mut self, instance: &str, cause: &str)`:**
  - builds each target's `TrackInput` (the singer by `singer_of`, `KeyCheck` from `keyt` against the set's `Position.follow.now()`);
  - calls `decide`;
  - collects the targets whose decision differs from what Live shows (`on_now`) into ONE `write_commands` batch per set, with each target's own `macro_value`;
  - records one `Select` per changed decision;
  - is held as today (`may_write`).
  - **Order at a switch-on (§7):** keys first. A target turns on only with `KeyCheck::Confirmed`, so the key write's read-back handler (`autotune_keys_read`, from `RouterMsg::AutotuneKeysRead`) ends with `autotune_decide(.., "key")`.
  - A switch-off writes at once.
- **Called after:**
  - `autotune_transport` (`transport`);
  - `autotune_position` when the song or key changed (`position`), or the playhead jumped (`jump`, the PR 2b crossing rule's non-crossing move);
  - every read-back (`key`);
  - `autotune_flag` (`flag`) and `autotune_singer` (`singer`);
  - `autotune_target_found` (`found`), `autotune_connected` (`reconnect`) and `autotune_release` (`release`).
- **Another writer on a selector:** a macro delivery that differs from `wrote_on` while the decision is DRY calls `guard.moved(now)`:
  - `WriteDryOnce` writes DRY and records `SelectorWriter {act: "dry_again"}`;
  - `GiveUp` records `act: "gave_up"` and makes `problem` true for that target until the next song start (`KeyWhy::Song` resets the guard).
  - A delivery showing DRY while the decision is on records `act: "left_dry"` and writes nothing.
- **The hold (§7):** when a hold starts (`autotune_hold`) for a set with a target whose decision is on, write DRY to it through `found.id` (the last known `$ref`) FIRST, before the hold. A stale ref answers `StaleRef`, and nothing more is written.
- **`fn autotune_singer(&mut self, client, id: &str, off: bool)`:** from `set_hub` key `HUB_AUTOTUNE_SINGER = "autotune_singer"` with value `{"id": "...", "off": bool}`. It records `Singer`, saves, tells the pages and decides every set.
  - This task adds `pub const HUB_AUTOTUNE_SINGER: &str = "autotune_singer";` to `fohmixer_proto::client`, beside `HUB_AUTOTUNE_AUT`.
  - A value that is not `{id: string, off: bool}`, or an id no target's singer carries, gets the usual error reply.
- [ ] **Step 1: Write the failing router tests** (`tagged()` with singers `VOX1` on both targets, keys confirmed through the PR 2b test helpers):
  - **`aut_on_and_playing_in_a_keyed_song_switches_on_after_the_keys`:** no macro write before the read-back's `confirmed`; then one batch with both on values.
  - **`a_stop_mid_song_writes_dry_at_once`:** a transport false gives one batch with the off values, before any read-back.
  - **`out_of_a_song_and_a_keyless_song_are_dry`.**
  - **`a_singer_off_covers_both_tracks_and_is_saved`.**
  - **`a_key_change_keeps_tuning_and_a_bad_read_back_drops_only_that_track`:** the stream read-back is refused (the held-parameter error), so only the stream macro goes off and the PA stays on.
  - **`another_writer_is_shown_and_dry_wins`:**
    - a delivery of 127 on a DRY target gives `dry_again` and a write of 0;
    - a second within 10 s gives `gave_up` and no write;
    - the next song start resets it.
  - **`a_hold_writes_dry_first_through_the_last_ref`:** a track-list change while on writes 0 by `$ref`, then holds.
  - **`a_reconnect_decides_again_and_switches_on_only_after_the_keys`.**
  - **`a_tap_is_refused_and_writes_nothing`.**
  - **`no_write_when_live_already_shows_the_decision`:** one write per change.
- [ ] **Step 2–4:** fail, implement, pass in CI (0 missed).
- [ ] **Step 5: Commit** `feat(hub): the hub switches every autotune track by the rule (#140)`

### Task 9: Protocol, status, end to end, PR 3b, deploy, the PC check

**Files:**
- Modify: `crates/fohmixer-proto/src/client.rs`, `crates/fohmixer-hub/src/router/autotune.rs` (`state`, `status`)
- Create: `crates/fohmixer-hub/tests/autotune_switch.rs` (in `hub-hosts`)

**Interfaces:**
- **`AutotuneState` gains** (every field `serde(default)`):
  - `song: Option<u16>`, `key: Option<String>`: the first target's set's key in force;
  - `checking: bool`: AUT on, playing in a keyed song, and some target not confirmed yet;
  - `singers: Vec<SingerState>`;
  - `map_problems: usize`;
  - `map_changes: usize`;
  - `map_keyed: usize`;
  - `bar: Option<String>`.
- **`pub struct SingerState { id, label, stream: TrackState, pa: TrackState, off: bool, key_ok: bool, why: Option<String> }`.**
  - `pub enum TrackState { Tune, Dry, None }`.
  - `why` reads e.g. `stream Key reads G, wrote F`, from the target's `KeyTargetStatus`.
- **`AutotuneTargetStatus` gains** `decision: Option<SwitchWhy>` and `on: bool` (the decision). `AutotuneStatus` gains `singers_off: Vec<String>`.
- **`ClientMsg::SetHub` keys:** `autotune_aut` (bool) and `autotune_singer` (`{id, off}`). `autotune` is refused.
- [ ] **Step 1: The host tests:**
  - **`a_song_played_with_aut_on_tunes_after_the_keys_and_a_stop_goes_dry`:**
    - SimLive plays from inside song 1 at 120 bpm;
    - the macros reach their on values only after the `readback` records;
    - `song time` jumps out of the song, so DRY;
    - a stop gives DRY at once;
    - `Host::sets` counts exactly the expected macro writes.
  - **`a_held_stream_slot_keeps_that_track_dry_and_the_pa_tunes`.**
  - **`the_manual_off_survives_a_hub_restart`.**
  - **`the_pages_are_told_the_singers_and_the_key`:** the `Autotune` message carries both singers, `key: "C"` and `song: 1`.
- [ ] **Step 2:** push, open the PR "Autotune: the hub switches each singer by the rule (#140)", and drive it green.
- [ ] **Step 3:** review, one fix round, merge, tag `v1.14.0`, bump to 1.15.0.
- [ ] **Step 4: Deploy** (hub only; no script change).
- [ ] **Step 5: The PC check, Live stopped:**
  1. With 1.8.0's button, a 1 s hold turns AUT on. `/api/status` shows every target `decision: stop`, `on: false`, and no macro write in the event log.
  2. Set one singer off through a `set_hub` from a script client. `hub-state.json` has it.
  3. Restart the hub: AUT is still on and the singer still off.
  4. Set the singer back on and AUT off.
  5. Record it on #140.
  6. The switching itself is proven at the rehearsal, after PR 3c.

---

## PR 3c: the surface (mockup v2)

### Task 10: The button's look and the panel's model (pure)

**Files:**
- Modify: `crates/fohmixer-ui/src/behave/autotune.rs` (rewritten; tests beside it)

**Interfaces:**
- **Produces:**
  - **`pub fn autotune_look(state: Option<&AutotuneState>, fill: Fill) -> RailLook`:**
    - **lit:** AUT on, playing, and every singer not off by hand tunes;
    - **mixed:** AUT on with some tuning and some not (off by hand, mismatch);
    - **the `aut` pill:** `Run` while AUT is on;
    - **the second line, exactly mockup v2's:**
      - `off · F` (AUT off, key F);
      - `stopped · F`;
      - `tuning · F`;
      - `no song`;
      - `no key`;
      - `checking · F`;
      - `tuning · F · 1 off`;
      - `tuning · F · 1 !`;
      - a press's fill shows `behave::rail`'s shared hold and full texts.
  - **`pub fn has_problem(state) -> bool`:** the yellow corner for any `key_ok: false` or `map_problems > 0`.
  - **`pub fn autotune_ready(state) -> Option<bool>`:** red (`Some(false)`) without a target, as today.
  - **`pub struct PanelRow { label, stream: &'static str, pa: &'static str, key: String, ok: bool, switch: &'static str, why: Option<String> }`:**
    - `stream` and `pa` are the chips `stream TUNE`, `stream DRY` or `stream —`, and the same with `PA`;
    - `key` is `F ✓`, or `!` when not ok;
    - `switch` is the action the tap does: `OFF` while on, `ON` while off by hand;
    - `why` is `"<label>: <why> · on DRY, rewrite at the next change"`.
  - **`pub fn panel_rows(state) -> Vec<PanelRow>`.**
  - **`pub fn panel_head(state) -> String`:** `song 031 · F`. With a key change in force it reads `song 031 · F → G at Key {G}` *(call: the head names the current key and, when a change is in force, the clip)*.
  - **`pub fn panel_foot(state) -> [String; 2]`:**
    - `Song map: <keyed> songs with a key, <changes> key changes, <n> problem(s).`;
    - `Live: <bar>. Hold AUTOTUNE 1 s: AUT on/off.` The pill word follows the shared badge (#141 renames `AUT` to `AUTO` in its own PR; this text uses the same constant).
  - **`pub fn tap_opens_panel(state, connected) -> bool`:** only with this connection's state and online.
- [ ] **Step 1: Write the failing tests:** one per second line above, with mockup v2's exact strings, and one each for:
  - `panel_rows` (all chip combinations);
  - `panel_head`;
  - `panel_foot`;
  - `has_problem`;
  - `tap_opens_panel`.
- [ ] **Step 2–4:** fail, implement, pass in CI.
- [ ] **Step 5: Commit** `feat(ui): the AUTOTUNE button's look and the singers' panel, as mockup v2 (#140)`

### Task 11: The button, the panel and the E2E

**Files:**
- Create: `crates/fohmixer-ui/src/components/autotune_panel.rs`
- Modify: `crates/fohmixer-ui/src/components/rail.rs`, the store's `HubAutotune`, the styles (from mockup v2's `.tpanel`, `.tp-*` and `.chip` rules)
- Rewrite: `e2e/tests/autotune.spec.ts`

**Interfaces:**
- **The gestures:**
  - the rail's AUTOTUNE tap (lift) toggles the panel (`data-open="true"` on the button; the panel `data-testid="autotune-panel"`);
  - a tap outside the panel, or on AUTOTUNE again, closes it;
  - the 1 s hold sends `autotune_aut` as today;
  - a row's switch (`data-testid="autotune-singer-switch"`, `data-id`) sends `set_hub autotune_singer {id, off}`.
- **Every surface tap target uses `use:owns_touches`,** with no mouse event (`scripts/check_integrity.py`).
- **The phone layouts (mockup v2):**
  - upright: no second line, the AUT dot, the panel at full width;
  - landscape: the panel beside the column.
- [ ] **Step 1: Write the Playwright tests** on the E2E harness with SimLive. Every test ends with the console-clean assertion (`.claude/rules/e2e.md`):
  - **`the_button_shows_off_stopped_tuning_and_checking_lines`:**
    - drive AUT and the transport through the hub;
    - read `data-state`, `data-aut` and the second line's text;
    - 118 × 44, no fader height lost.
  - **`a_tap_opens_the_panel_with_every_singer_and_a_tap_outside_closes_it`.**
  - **`a_singers_off_switch_takes_both_tracks_to_dry`:** the hub's `/api/status` decision `singer_off`, and the macros in SimLive at their off values.
  - **`a_mismatch_shows_the_yellow_corner_and_its_why_in_the_panel`:** a held slot through the host line.
  - **`a_hold_turns_aut_on_at_its_lift`.**
  - **`after_a_reconnect_a_hold_and_a_tap_send_nothing_until_the_state`.**
  - **Phones** (390 × 844, 844 × 390): the button and the panel fit, and every rail button is whole.
- [ ] **Step 2:** `npx playwright test --list` parses; push, and read every E2E leg green.
- [ ] **Step 3: Commit** `feat(ui): the AUTOTUNE button and the singers' panel, with E2E (#140)`

### Task 12: PR 3c, deploy, then the rehearsal

- [ ] **Step 1:** PR "Autotune: the button and the singers' panel (#140)". Drive it green, review, one fix round, merge, tag `v1.15.0`, bump to 1.16.0.
- [ ] **Step 2: Deploy.** Then the post-deploy Playwright check:
  - the version label;
  - the AUTOTUNE button at 118 × 44;
  - the panel opens with 6 singers;
  - console clean.
- [ ] **Step 3: Before the rehearsal**, the latency measurement of spec §10 (§11 step 4). It is its own design, written on #140 first and run with Live stopped.
- [ ] **Step 4: The rehearsal (§11 step 5),** with no audience, the band Live playing and AUT on. The main session runs it on the PC under the owner's standing approval and tells the owner before it plays.
  - Every song is started:
    - the surface's key against the panel;
    - Live's bar;
    - one PA Auto-Tune window opened through `is_editor_open`, with no click.
  - The 6 songs with key changes are played through each change. The `late` records stay ≤ 100 ms.
  - A stop in the middle of a song gives DRY.
  - A position out of every song, and song 088 (no key), give DRY.
  - A singer off by hand holds through a song start.
  - The song map report matches verification 2.
  - Record it on #140 (no names).
- [ ] **Step 5:** the first event with AUT on is the owner's. The old chain's removal (§11 step 6) follows it, on the owner's yes.
