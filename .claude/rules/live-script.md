---
paths:
  - "live-script/**"
  - "sim/**"
---

# FohMixer script and SimLive (learned on #9)

- **The sender sends one frame at a time by priority** (`transport/server.py` `Connection._take_next`): the pending heartbeat, then results and events in order, then the values map as one frame. A heartbeat waits for at most the frame in flight, never behind queued results: a late heartbeat makes the hub report Live busy. `test_transport` sizes its check from the kernel's `tcp_wmem`.
- **The heartbeat carries `gap_ms`**, the heartbeat thread's own time since its previous heartbeat (`surface.heartbeat_data`): long means the thread itself did not run in Live's Python, as opposed to a heartbeat held on the way (the hub logs both, `hub-rust.md`).
- **SimLive writes dB strings as real Live does** (`sim/Live/__init__.py` `_live_number`): 3 significant digits, at most 3 decimals, 0 as `0.00` (`-0.811 dB`, `-6.00 dB`, `-14.0 dB`), from the one string captured on the PC. Tests that pin a SimLive display string use that form; the UI and hub never format it (spec P2: Live's string as it is).
- A SimLive load (60 metered strips at 30 Hz, two clients refreshing) never delayed a heartbeat on CI or here (#9): SimLive does not reproduce Live's thread scheduling. Prove transport ordering with targeted tests, not load.
