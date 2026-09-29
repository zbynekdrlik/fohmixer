---
paths:
  - "live-script/**"
  - "sim/**"
---

# FohMixer script and SimLive (learned on #9)

- **The sender sends one frame at a time by priority** (`transport/server.py` `Connection._take_next`): the pending heartbeat, then results and events in order, then the values map as one frame. A heartbeat waits for at most the frame in flight, never behind queued results: a late heartbeat makes the hub report Live busy. `test_transport` sizes its check from the kernel's `tcp_wmem`.
- **The heartbeat carries `gap_ms`**, the heartbeat thread's own time since its previous heartbeat (`surface.heartbeat_data`): long means the thread itself did not run in Live's Python, as opposed to a heartbeat held on the way (the hub logs both, `hub-rust.md`).
- **SimLive writes dB strings as real Live does** (`sim/Live/__init__.py` `_live_number`): 3 significant digits, at most 3 decimals, 0 as `0.00` (`-0.811 dB`, `-6.00 dB`, `-14.0 dB`), from the one string captured on the PC. Tests that pin a SimLive display string use that form; the UI and hub never format it (spec P2: Live's string as it is).
- A SimLive load (60 metered strips at 30 Hz, two clients refreshing) never delayed a heartbeat on CI or here (#9): SimLive does not reproduce Live's thread scheduling. Prove transport ordering with targeted tests, not load. (The dev box under swap pressure does delay SimLive heartbeats by up to ~360 ms, #5: a SimLive test must not bound the machine's timing.)
- **On the real Live the script's threads run in step with Live's ~30 Hz main loop (#5, measured with `tools/live-probe`):** the timer fires about every 34 ms whatever `TIMER_INTERVAL_MS` asks, and the reader and sender threads get Live's Python about once per tick, so a connection's sender sends about one frame per tick (~30–37 frames/s; a result queued waits ~32 ms, a read ~43 ms to be drained). Reads every 2 ms filled a connection's 1000-result queue in 2 s. Whatever adds frames per connection (results, value pushes, another client) competes for those slots; the heartbeat's priority keeps it at one tick's delay on the band Live, the master Live's send path was slower and varied.
- **`time.monotonic()` in Live on Windows is `GetTickCount64`** (15/16 ms steps): `main_tick_age_ms`, `gap_ms` and the stall log come in those steps; `time.time()` (the frames' `ts`, `GetSystemTimeAsFileTime`) is ~0.5 ms there only while an audio app holds a raised timer resolution (15.625 ms by default; the probe's `wall_clock_step_ms` measures it).
- **A heartbeat can go out before `connect`** (#5, seen on the PC): `_handle` queues the connect event, then adds the connection to the heartbeat broadcast before its sender has run, and `_take_next` sends a pending heartbeat first. A client must not assume `connect` is the first frame (the hub records such a heartbeat and checks busy only once connected; the probe skips it).
