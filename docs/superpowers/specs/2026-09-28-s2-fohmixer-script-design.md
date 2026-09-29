# S2: FohMixer Live script and SimLive, design note

**Ticket:** #6. **Spec:** `docs/superpowers/specs/2026-09-28-fohmixer-program.md` §2.2, §2.3, §2.7 (I1–I3), §5.3. **Plan:** `docs/superpowers/plans/2026-09-28-s2-fohmixer-script.md`.

**Inputs**
- The reference script `leolabs/ableton-js` v5.0.3 @69de331, `midi-script/` (MIT).
- The S0 skeleton: `live-script/FohMixer/`, `sim/`, and the Python CI job.

## 1. Goal

A remote script that exposes Live's native LOM 1:1 over a localhost WebSocket, plus a fake `Live` module (SimLive) on which the real script runs in CI.

**Done means:**
- The Python CI job runs the script's unit tests and **socket-level integration tests**. The integration tests use a real WebSocket client against the real script running on SimLive, in a SimLive host with a Live-like single main thread.
- The tests cover:
  - path resolution with name selectors;
  - get, set, call and describe;
  - `$ref`/`$enum` out and in;
  - listener coalescing and the meter cap;
  - the work budget;
  - non-blocking send with a stalled client;
  - the heartbeat around a simulated main-thread stall (#5: none during it, the first one after it reports it);
  - reconnect after disconnect;
  - two instances on two ports.
- `sim/host.py` runs as a process (`python3 sim/host.py --port N --site <fixture.json>`). S3's hub tests start it.

## 2. Module layout

`live-script/FohMixer/`:

| File | Responsibility |
|---|---|
| `__init__.py` | `create_instance(c_instance)` returns `FohMixer(c_instance)` |
| `Config.py` | `INSTANCE = "band"`, `PORT = 39101`, `TIMER_INTERVAL_MS = 10`, `DRAIN_BUDGET_MS = 5`, `WINDOW_BUDGET_MS = 5`, `WINDOW_MS = 20`, `METER_MIN_INTERVAL_MS = 33`, `HEARTBEAT_INTERVAL_MS = 100`, `RESULT_QUEUE_MAX = 1000`. Each Windows user's copy edits `INSTANCE` and `PORT`. |
| `version.py` | `VERSION` (kept equal to the workspace version by `check_version.py`) |
| `log.py` | Logger `fohmixer.<INSTANCE>`: WARNING level, `RotatingFileHandler` (1 MB × 3) in the script folder `logs/`, never per message |
| `transport/websocket.py` | Vendored `WebSocket.py` (RFC 6455 framing, handshake); #5 added a bytes-in request parser and a bytes-out 101 response |
| `transport/server.py` | Vendored and changed `Socket.py`: no threads since #5; Live's main thread polls it in the tick (`poll_in`, `poll_out`), `Connection` with a write buffer (§3.7) |
| `lom/path.py` | Path parsing and resolution (§3.1) |
| `lom/registry.py` | Object id registry, `(ptr, class)` checked on lookup |
| `lom/codec.py` | Values out (`$ref`, `$enum`, vectors) and in (decoding) |
| `lom/ops.py` | `get_prop`, `set_prop`, `add_listener`, `remove_listener`, `describe`, raw call |
| `subscriptions.py` | Listener registry, dirty set, coalesced flush, meter cap |
| `surface.py` | `FohMixer(ControlSurface)`: timer, tick fallback, socket polls, drain with budget, heartbeat made in the tick, lifecycle |

The package uses relative imports only, and module names never collide with AbleSet or AbletonOSC.

`sim/`:

| File | Responsibility |
|---|---|
| `Live/__init__.py` | The fake `Live` module: `Base.Timer`, `Application.get_application()`, `Song`, `Track`, `MixerDevice`, `DeviceParameter`, `Device`, `RackDevice`, `Chain`, `ChainMixerDevice`, enums, listeners |
| `_Framework/ControlSurface.py` | Fake `ControlSurface`: `__init__(c_instance)`, `song()`, `application()`, `schedule_message(ticks, cb)`, `show_message`, `disconnect` |
| `main_thread.py` | `MainThread`: one thread that runs timer callbacks and scheduled messages, plus `stall(ms)` for tests |
| `site.py` | Builds a Song from a JSON fixture: tracks, groups, returns, master, devices, parameters, meters |
| `fixtures/test-site.json` | A synthetic site (see below) |
| `host.py` | CLI: `--port`, `--instance`, `--site`; builds the song and runs FohMixer on `MainThread` until SIGTERM |

The synthetic site in `fixtures/test-site.json` has:
- `Hand1 #`…`Hand4 #`;
- `Vocals Repro grp#` (a group) with `Vocal 1 repro#`…`Vocal 3 repro#`;
- `Stems grp#` (a group) with `Drums #` and `Bass #`;
- `Mics Stage #`, `TechAlert #`;
- a duplicate name `Keys 1` in two groups;
- returns `A-Reverb #` and `B-Main repro #`;
- an `EQ Eight`-like device with named band parameters;
- a rack with two chains and a chain selector.

## 3. Behaviour

### 3.1 Paths

**Grammar:** a root followed by steps, separated by single spaces.
- The root is `live_set` (the Song) or `live_app` (the Application).
- A step is `<attr>`, `<attr> <index>` or `<attr>[name=<text>]`.
- The text inside `[name=…]` runs to the matching `]`. Escapes: `\]` and `\\`.

Example: `live_set tracks[name=Vocal 1 repro#] mixer_device volume`.

**Resolution**
- Resolution walks with `getattr`, then indexes into LOM vectors.
- A `[name=…]` step selects the single element whose `name` equals the text exactly.
- Zero matches or several matches give `PathError("not found" | "ambiguous", step)`.
- A `target` may also be `{"$ref": "<id>"}`.

**Output path.** Every object sent out carries `path`, built from how it was reached, in index form (e.g. `live_set tracks 3`). It is informational; bindings use names or ids.

### 3.2 Ids

- An object's id is `live_<obj._live_ptr>`, or `id_<id(obj)>` if it has no `_live_ptr`.
- The registry stores the id → `(weakref-or-obj, class name)`. On lookup it checks `type(obj).__name__` against the stored class, and on a mismatch raises `StaleRef`.
- The registry is cleared on `disconnect()`.

### 3.3 Values

**Out**
- `None`, bool, int, float and str are sent as-is. A tuple or list is sent as a list.
- A Live object (anything whose class lives in the `Live` module) is sent as `{"$ref","path","class","name"?}`.
- An enum (an int subclass whose class has `values`/`names`, the Boost.Python enum) is sent as `{"$enum": name, "value": int}`.
- `list()` is applied **only** to Live vector types. Anything else falls back to `str()`.

**In**
- A dict with `$ref` or `path` is resolved to the object.
- A dict with `$enum` is converted:
  - for `set_prop`, via `type(current_value).names[name]`;
  - for a call, via a qualified name `Live.X.Y.name` looked up in the `Live` module.

**Display strings.** `add_listener(prop, display=true)` on a `DeviceParameter` `value` pushes `{"value", "display": str(param)}`. `get_prop` takes the same `display` flag.

### 3.4 Operations (the `name` field)

| `name` | `args` | Result |
|---|---|---|
| `get_prop` | `{prop, display?}` | the encoded value |
| `set_prop` | `{prop, value}` | `null` |
| `add_listener` | `{prop, display?}` | `{key, value, display?}`, where `key = "<id>.<prop>"` and the initial value is included (I8) |
| `remove_listener` | `{prop}` | `null` |
| `describe` | `{}` | `{class, observable: [...], properties: [...], functions: [...]}`, from `dir()` without underscores |
| any other | positional list or keyword dict | a call on the target; the result is encoded |

Names that start with `_` are rejected (the reference script's dispatch had no filter).

### 3.5 Subscriptions and flush

**Registry.** The registry maps `key → Sub{obj, prop, display, connections:set, dirty:bool, last_flush_ms}`.
- The first subscriber registers the Live listener `add_<prop>_listener(cb)`. The last one to leave removes it.
- A dropped connection is removed from every subscription (main thread, sentinel as in the reference).

**Callback.** The callback only sets `dirty = True` and records the key in `dirty_keys`. It does no LOM writes; Live forbids changes inside notifications.

**Flush.** Flush runs in every timer call, after the drain. For each dirty key:
- a meter prop (`output_meter_left`/`right`/`level`) is skipped until `METER_MIN_INTERVAL_MS` has passed since its last flush, and stays dirty;
- otherwise the value (and display) is read and put into each subscribed connection's **latest-value map**, and the key is cleared.

A read that raises (the object was deleted) sends `{"key", "error": "gone"}` and drops the subscription.

### 3.6 Main thread: drain with budget

- The timer is `Live.Base.Timer(callback=self._on_timer, interval=TIMER_INTERVAL_MS, repeat=True)`. The fallback is `schedule_message(1, self._tick)`, which also checks lag.
- `_on_timer`:
  1. Write `last_main_tick = monotonic()`.
  2. Read the sockets (`Server.poll_in`, §3.7).
  3. Run commands from the inbound queue until `DRAIN_BUDGET_MS` has elapsed or the window budget (`WINDOW_BUDGET_MS` per `WINDOW_MS`) is used up. It is checked between commands.
  4. Flush (§3.5), and make the heartbeat when one is due (§3.8).
  5. Write the sockets (`Server.poll_out`, §3.7).
- The longest single command is tracked in `stats.max_cmd_ms`.

### 3.7 Connections: never block the main thread

Changed on #5 (K1): this section first specified the reference's threads (an accept thread, a reader and a sender thread per connection). Measured in the real Live, those threads got Python only around the main thread's ~30 Hz tick: a connection sent about one frame per tick. The script now runs no threads; the main thread does the socket I/O in its tick, and never waits on a socket:

- every socket is non-blocking; `poll_in` selects with timeout 0, accepts (at most 64 clients, of which 16 still in their handshake; every accepted socket the server closes is reset, so no TIME_WAIT keeps the port busy), reads handshakes and frames (each connection within a 5 ms budget per tick) and puts requests into the inbound queue;
- one client's failure closes that client only; each step of the tick (reads, drain, subscription flush, heartbeat, writes) is guarded on its own;
- `Connection` has `results` (a bounded `collections.deque`; `RESULT_QUEUE_MAX` is a hard cap, on overflow the connection is closed and the hub resyncs), `values` (a dict `key → item`, the latest value only), the pending heartbeat, and a write buffer;
- `poll_out` writes, per connection: the heartbeat, then all pending results in order, then one `values` frame from a swapped-out dict, as far as the socket takes them (within a 5 ms budget per tick; at most 64 KB encoded ahead of the socket); the rest waits for the next tick;
- a socket that took no byte for 3 s while output waited closes its connection (the client stopped reading);
- the 101 response and the `connect` frame are the first bytes of every connection.

The reference script's direct `sendall` path is removed.

### 3.8 Heartbeat

The main thread makes `{"event":"heartbeat","data":{"main_tick_age_ms", "max_cmd_ms", "gap_ms"}}` in its tick for every connection, one per `HEARTBEAT_INTERVAL_MS` (#5; a daemon thread before). `main_tick_age_ms` is the silence before it: the time since the previous tick reached the same point (after its work, before its writes), so a tick held by its own work counts too; `gap_ms` the time since the previous heartbeat. During a stall none goes out, so the hub marks the instance busy once one is 300 ms overdue; the first one after the stall reports it.

### 3.9 Lifecycle

- **`__init__`:** start the server bound to `127.0.0.1:PORT` with `SO_EXCLUSIVEADDRUSE` on Windows (`SO_REUSEADDR` elsewhere), then the timer and the tick. If the bind fails, the ticks retry it (0.25 s, 0.5 s, 1 s, 2 s, then every 5 s) and log a WARNING once per distinct error.
- **On each new connection:** send `connect{instance, set_name: song.name if present else "", script_version, live_version: application.get_major_version()/minor/bugfix as "12.2.x", proto: 1}`.
- **`disconnect()`:**
  1. stop the timer (the heartbeat stops with it);
  2. broadcast `disconnect`;
  3. close every connection and the listening socket;
  4. remove every Live listener;
  5. clear the registry.

### 3.10 Envelope (spec §2.3)

- **Request:** `{"uuid": str, "commands": [{"target": str|{"$ref":id}, "name": str, "args": dict|list}]}`.
- **Result:** `{"event":"result","uuid","data":[{"ok":true,"data"}|{"ok":false,"error","errorType"}],"ts"}`.
- **Values:** `{"event":"values","data":[{"key","value","display"?}|{"key","error"}],"ts"}`.
- **Other events:** `connect`, `disconnect`, `heartbeat`.
- A malformed request returns `{"event":"error","uuid","data":"<msg>"}`.

## 4. Proof

**Unit tests** (in-process, SimLive, with no sockets):
- `path` grammar and resolution: index, name, missing, ambiguous (duplicate `Keys 1`), escapes, `live_app`;
- the registry and a stale class after a pointer is reused;
- the codec both ways, including enums and an id inside call args;
- `describe` contents;
- underscore names rejected;
- subscription coalescing: 100 changes before a flush become one item with the last value;
- the meter cap: 10 meter changes within 33 ms become one item, and the rest follow on a later flush;
- a deleted object gives `error: "gone"`;
- the budget: 50 slow fake commands run across several timer calls, and each call is ≤ budget + one command.

**Integration tests** (`host.py` subprocess plus a WebSocket client from the `websockets` library, pinned, dev-only):
- `connect` event;
- a get/set round trip;
- a listener push after a set from another client;
- two clients each get the push;
- a client that stops reading does not block the other client or the main thread (`main_tick_age_ms` stays low);
- `host.stall(700)`: no heartbeat during the stall, and the first one after it reports an age ≥ 400 and a gap ≥ 600 (#5; while the heartbeat was a thread it showed the age growing during the stall);
- disconnect, reconnect and resubscribe;
- two hosts on two ports.

**Live check (L, S5):** the script loads in both Live instances next to AbleSet and AbletonOSC, with no errors in `Log.txt`. SimLive's volume display strings are compared with strings captured from the real Live.

## 5. Not in S2

The hub (S3), the UI (S4), installation on the PC (S1/S5), and MIDI (never, D10).
