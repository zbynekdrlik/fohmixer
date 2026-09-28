# S2 FohMixer Live Script + SimLive Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task by task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Ticket:** #6.

**Goal:** Build the FohMixer remote script (a generic LOM proxy on a localhost WebSocket) and SimLive, with the real script tested in CI on top of SimLive.

**Architecture:** see the design note `docs/superpowers/specs/2026-09-28-s2-fohmixer-script-design.md`; its section numbers are cited below. The transport is vendored from `leolabs/ableton-js` v5.0.3 `midi-script/` (MIT). The LOM layer is our own, generic, with no per-class code.

**Tech Stack:**
- Python 3.11 (Live 12 embeds CPython 3.11), standard library only inside the script.
- Tests: `unittest`, plus `websockets==13.1` for the dev-only integration client.
- `ruff`.

**Spec:** `docs/superpowers/specs/2026-09-28-fohmixer-program.md` (§2.2, §2.3, §2.7, §5.3). Design: the note above.

## Global Constraints

**The script**
- Standard library only. No third-party imports inside `live-script/FohMixer/`; Live cannot install packages.
- Relative imports only. Logger name `fohmixer.<INSTANCE>`, WARNING level, never per message.
- The main thread never calls `sendall` or blocks on I/O (design §3.7).
- Bind `127.0.0.1` only. `SO_EXCLUSIVEADDRUSE` on Windows.
- Defaults: `PORT` 39101 (band) / 39102 (master), never 39031. `TIMER_INTERVAL_MS=10`, `DRAIN_BUDGET_MS=5`, window 5 ms per 20 ms, `METER_MIN_INTERVAL_MS=33`, `HEARTBEAT_INTERVAL_MS=100`, `RESULT_QUEUE_MAX=1000`.

**The vendored code** keeps the MIT notice: `live-script/FohMixer/transport/LICENSE-ableton-js`, plus a header line in each vendored file with the source commit `69de331`.

**Workflow**
- Python runs locally (unittest, ruff): iterate locally until green, then push once.
- The version is bumped first on `dev`: the next free `-dev.N`, updated in `Cargo.toml`, `Cargo.lock` and `version.py`, per `check_version.py`.

## Review Focus

- **A name that appears twice** (`Keys 1` in two groups) must give "ambiguous", never the first match. It is tested in Task 2.
- **A track deleted while subscribed** gives `error: gone` with no exception on the main thread. Tested in Task 4.
- **A client that stops reading** leaves the main thread and the other clients unaffected. Tested in Task 6.
- **A set reload** (`disconnect()` then a new instance on the same port) lets the port re-bind at once, and the hub's reconnect then works. Tested in Task 6.
- **A malformed request** (not JSON, no `commands`, an unknown `name`, a `target` of the wrong type) gets an error reply, never kills the reader thread. Tested in Task 5.

---

### Task 1: SimLive and the main-thread host

**Files:**
- Create:
  - `sim/Live/__init__.py`
  - `sim/_Framework/__init__.py`, `sim/_Framework/ControlSurface.py`
  - `sim/main_thread.py`, `sim/site.py`, `sim/fixtures/test-site.json`
  - `sim/tests/test_sim.py`
- Modify: whatever S0 created under `sim/` (replace the skeleton).

**Interfaces:**
- Produces:
  - `Live.Base.Timer(callback, interval, repeat)` with `.start()` and `.stop()`, driven by `MainThread`.
  - `Live.Application.get_application()` returning an object with `get_major_version()`, `get_minor_version()` and `get_bugfix_version()`.
  - `Song`: `tracks`, `return_tracks`, `master_track`, `visible_tracks`, `is_playing`, `name`, `start_playing()`, `stop_playing()`, `add_tracks_listener`, `add_is_playing_listener`, and `delete_track(i)` for tests.
  - `Track`: `name`, `mute`, `solo`, `fold_state`, `is_foldable`, `is_grouped`, `group_track`, `mixer_device`, `devices`, `output_meter_left/right/level`, `color`, `_live_ptr`.
  - `MixerDevice`: `volume`, `panning`, `sends`, `track_activator`.
  - `DeviceParameter`: `name`, `value`, `min`, `max`, `is_quantized`, `value_items`, `__str__` (volume → `"-6.0 dB"`, using the formula in §5), `str_for_value(v)`, `add_value_listener`.
  - `Device`, `RackDevice` (`chains`, `can_have_chains`), `Chain` (`name`, `mute`, `solo`, `mixer_device`, `devices`) and `ChainMixerDevice`.
  - A Boost-like enum class `Live.Track.Track.monitoring_states` with `names` and `values` dicts.
  - Each listened property supports `add_<p>_listener`, `remove_<p>_listener` and `<p>_has_listener`. A setter fires its listeners synchronously **and** raises `RuntimeError("Changes cannot be triggered by notifications")` if it is called while listeners for any property are running (Live's rule).
  - `ControlSurface(c_instance)` with `song()`, `application()`, `schedule_message(ticks, cb)` (1 tick = 100 ms on `MainThread`), `show_message` and `disconnect()`.
  - `MainThread`: `run_forever()`, `call(fn) -> result` (runs on the main thread and waits), `stall(ms)`, `stop()`.
  - `site.build(path) -> Song`.

- [ ] **Step 1: Write the failing tests** in `sim/tests/test_sim.py`:
  - loading `test-site.json` gives 13 tracks including groups, 2 returns and a master;
  - a volume `__str__` at 0.85 is `"0.0 dB"` and at 0.0 is `"-inf dB"`;
  - setting `mute` fires the listener once;
  - a set from inside a listener raises `RuntimeError`;
  - `Timer(interval=10)` fires about 10 times in 100 ms on `MainThread`;
  - `stall(200)` delays the timer by 200 ms or more;
  - `schedule_message(1, cb)` fires after about 100 ms;
  - `_live_ptr` values are unique.
- [ ] **Step 2: Run** `python3 -m unittest discover -s sim/tests -v`. Expected: FAIL (the modules are missing).
- [ ] **Step 3: Implement the classes.**
  - The volume display formula (SimLive only; §5.3 later checks it against real Live strings) is the piecewise TouchOSC `value2db`:
    - for v ≥ 0.4: `40v − 34`;
    - for 0.15 ≤ v < 0.4: `−((399.751894v − 201.871345)² + 12630.61132)/799.503788`;
    - for v < 0.15: `118.426374·v^(5567/7504) − 70`;
    - `-inf dB` at or below −70.

    Format `f"{dB:.1f} dB"`.
- [ ] **Step 4: Run** the tests. Expected: PASS. Then `ruff check sim`.
- [ ] **Step 5: Commit** — `feat(sim): SimLive fake Live module, main thread host and synthetic site (#6)`.

### Task 2: Paths, registry and codec

**Files:**
- Create: `live-script/FohMixer/lom/__init__.py`, `lom/path.py`, `lom/registry.py`, `lom/codec.py`, `live-script/tests/test_lom.py`.

**Interfaces:**
- Produces:
  - `parse(path:str) -> (root, steps)`
  - `resolve(target, song, app, registry) -> (obj, path_str)`
  - `PathError(kind, step)`
  - `Registry.put(obj) -> id`, `Registry.get(id) -> obj`, raising `StaleRef`/`UnknownRef`, and `Registry.clear()`
  - `encode(value, registry, path_hint) -> json-able`
  - `decode(value, registry, song, app, expected=None) -> python`

- [ ] **Step 1: Write the failing tests:**
  - `live_set tracks 0` gives Hand1;
  - `live_set tracks[name=Vocal 2 repro#] mixer_device volume` gives the parameter;
  - `[name=Keys 1]` raises `PathError("ambiguous")`;
  - `[name=Nope]` raises `PathError("not found")`;
  - escapes: `[name=a\]b]` matches a track named `a]b`;
  - `live_app` resolves to the application;
  - `{"$ref": id}` round-trips through the registry;
  - a registry id whose object class changed raises `StaleRef`;
  - encode: a Track becomes `{"$ref","path","class":"Track","name"}`; the tracks vector becomes a list of those; an enum becomes `{"$enum","value"}`; a float stays as-is; an unknown object becomes `str()`;
  - decode: `{"$ref":id}` gives the object, `{"path":...}` gives the object, and `{"$enum":"In"}` with the current monitoring state gives the enum member.
- [ ] **Step 2: Run** `python3 -m unittest live-script.tests.test_lom -v` (run from the repo root with `sys.path` set by the test module). Expected: FAIL.
- [ ] **Step 3: Implement** per design §3.1–3.3.
- [ ] **Step 4: Run.** Expected: PASS. Then ruff.
- [ ] **Step 5: Commit** — `feat(live-script): generic LOM paths, registry and codec (#6)`.

### Task 3: Operations

**Files:**
- Create: `live-script/FohMixer/lom/ops.py`; tests in `live-script/tests/test_ops.py`.

**Interfaces:**
- Consumes: Task 2.
- Produces: `execute(command: dict, ctx) -> data`, which raises `OpError`. `ctx` carries the song, app, registry, subscriptions and connection.

- [ ] **Step 1: Write the failing tests:**
  - `get_prop name` on a track;
  - `get_prop value` with `display` on volume gives `{"value":0.85,"display":"0.0 dB"}`;
  - `set_prop mute true`;
  - `set_prop value` on a DeviceParameter with a value outside `[min,max]` raises `OpError`, and the value is unchanged (Live raises; SimLive does the same);
  - a raw call `str_for_value [0.5]` gives a string;
  - `describe` on a Track lists `mute` as observable and `delete_device` as a function;
  - `name` starting with `_` raises `OpError("forbidden")`;
  - an unknown `name` raises `OpError("no such function")`.
- [ ] **Step 2: Run.** Expected: FAIL. **Step 3: Implement** per design §3.4. **Step 4: Run.** Expected: PASS.
- [ ] **Step 5: Commit** — `feat(live-script): generic get/set/call/describe (#6)`.

### Task 4: Subscriptions and flush

**Files:**
- Create: `live-script/FohMixer/subscriptions.py`, `live-script/tests/test_subscriptions.py`.

**Interfaces:**
- Produces: `Subscriptions.add(obj, prop, display, conn) -> (key, initial)`, `.remove(key, conn)`, `.drop_connection(conn)`, `.flush(now_ms) -> None` (pushes into `conn.push_value(key, item)`), and `.clear()`.

- [ ] **Step 1: Write the failing tests** with a fake `conn` that records `push_value`:
  - 100 sets of volume, then one flush, give exactly one push carrying the last value;
  - two connections on the same key share one Live listener (`value_has_listener` is True, and after both remove it is False);
  - meter: 10 changes within 20 ms give one push at the first flush; the next flush before 33 ms gives none; the flush after 33 ms gives the latest;
  - a deleted track (`song.delete_track`) gives the push `{"key","error":"gone"}`, and the subscription is removed;
  - the listener callback performs no set (asserted by setting in a listener raising in SimLive, which never happens).
- [ ] **Step 2: Run.** Expected: FAIL. **Step 3: Implement** per design §3.5. **Step 4: Run.** Expected: PASS.
- [ ] **Step 5: Commit** — `feat(live-script): coalesced subscriptions with meter cap (#6)`.

### Task 5: Transport (vendored and changed)

**Files:**
- Create:
  - `live-script/FohMixer/transport/__init__.py`
  - `transport/websocket.py`, vendored `WebSocket.py`
  - `transport/server.py`, from `Socket.py` with changes
  - `transport/LICENSE-ableton-js`
  - `live-script/tests/test_transport.py`

**Interfaces:**
- Produces:
  - `Server(host, port, on_payload(conn, payload), on_disconnect(conn))` with `.start()`, `.shutdown()` and `.broadcast(event, data)`.
  - `Connection` with `.push_result(uuid, data)`, `.push_value(key, item)`, `.send_event(event, data)` and `.close()`.
  - Inbound payloads are put on `Server.inbox` (a `queue.Queue`) for the main thread.

- [ ] **Step 1: Write the failing tests** (real sockets on an ephemeral port, test client from `websockets`):
  - handshake and the `connect` event are delivered;
  - 3 `push_value` calls for one key before the sender wakes produce one `values` frame with the last value;
  - results keep their order;
  - a client that never reads: pushing 5,000 values and 900 results never blocks the calling thread (each call takes < 1 ms), and pushing past `RESULT_QUEUE_MAX` closes that connection only;
  - not-JSON input and a missing `commands` produce an `error` event, and the connection stays open;
  - `shutdown()` then a new `Server` on the same port binds at once.
- [ ] **Step 2: Run.** Expected: FAIL.
- [ ] **Step 3: Implement.**
  - Vendor `WebSocket.py` verbatim, adding the header.
  - From `Socket.py` keep `_serve`, `_handle_connection` (drop `StaticServer`, auth and the port files), and replace `ClientConnection` with `Connection` per design §3.7.
  - Use `SO_EXCLUSIVEADDRUSE` when `os.name == "nt"`.
- [ ] **Step 4: Run.** Expected: PASS.
- [ ] **Step 5: Commit** — `feat(live-script): non-blocking WebSocket transport vendored from ableton-js (#6)`.

### Task 6: Surface, heartbeat, lifecycle and the end-to-end host

**Files:**
- Create: `live-script/FohMixer/surface.py`, `log.py`, `sim/host.py`, `live-script/tests/test_integration.py`.
- Modify: `live-script/FohMixer/__init__.py` and `Config.py`.

**Interfaces:**
- Consumes: Tasks 1–5.
- Produces:
  - `FohMixer(c_instance)` with `disconnect()`.
  - The CLI `python3 sim/host.py --port P --instance band --site sim/fixtures/test-site.json`. It prints `READY <port>` on stdout once bound, and exits 0 on SIGTERM.

- [ ] **Step 1: Write the failing integration tests** (subprocess host, `websockets` client):
  - `connect` has `instance`, `proto: 1` and `script_version == VERSION`;
  - a batch of 3 commands returns 3 slots in order, and one failing slot does not abort the others;
  - client A sets `mute` and client B (subscribed) receives the push within 100 ms;
  - `add_listener` returns the initial value (I8);
  - with a non-reading client C connected and 20 meters subscribed, client A's round trip stays < 50 ms and `heartbeat.main_tick_age_ms` stays < 50;
  - a host-side stall of 500 ms shows heartbeat ages ≥ 400 during the stall. `host.py` reads control lines on stdin; `stall 500` calls `MainThread.stall(500)`. This control never goes through the WebSocket;
  - SIGTERM, then `disconnect` is received, and a new host on the same port accepts a new client;
  - two hosts (`band` on P, `master` on P+1) run at once.
- [ ] **Step 2: Run.** Expected: FAIL.
- [ ] **Step 3: Implement** `surface.py` per design §3.6, §3.8 and §3.9: timer, tick fallback, budgeted drain, flush, heartbeat thread and lifecycle. `log.py` is a rotating WARNING logger. Then `host.py`.
- [ ] **Step 4: Run all Python tests.** Expected: PASS. Then `ruff check .`.
- [ ] **Step 5: CI.** Extend the S0 `python` job:
  - `pip install websockets==13.1` into a venv;
  - run `sim/tests` and `live-script/tests`;
  - the job fails if any test is skipped (use unittest's result counts: `--verbose` output checked by a small script, or `python -m unittest` exit status plus no `skip` decorators, which the integrity check already bans).
- [ ] **Step 6: Commit** — `feat(live-script): FohMixer surface with budgeted drain and off-thread heartbeat (#6)`.

### Task 7: Push, PR, merge

- [ ] **Step 1:** `git fetch origin && git merge origin/master`, then push `dev` once and monitor CI to terminal (foreground bounded poll).
- [ ] **Step 2:** Open a PR from `dev` to `master`, "S2: FohMixer Live script v1 + SimLive (#6)", with a body carrying `Closes #6` and the attribution line.
- [ ] **Step 3:** Merge when every check is green and `mergeStateStatus` is CLEAN. Then bump `dev` to the next `-dev.N`.
