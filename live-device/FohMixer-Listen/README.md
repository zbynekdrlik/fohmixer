# FohMixer Listen (Max for Live audio effect, #210)

Streams its input, the track's sound before the track's fader, to the
FohMixer hub on `127.0.0.1` while the hub selects it, so a page can listen to
the track. No parameters. Its output is silence: it goes in a rack of its own
(an empty dry chain plus the device's chain), never in series, as the LUFS
devices (`../FohMixer-LUFS/README.md`).

**It holds no clock (#264, the lane's design in comment 7184226996 on
#264).** Max for Live runs its scheduler inside Live's audio thread, and
every clock's output, a `deferlow` or a `qmetro`'s hand-over to the
low-priority queue, posts a window message from there, which can wait on the
desktop's window lock: a click on the PA (dasio's stack probes). So nothing
in the device runs on its own: no `metro`, `qmetro`, `delay`, `pipe` or
`deferlow`. The hub drives it, as it drives the LUFS and Meter devices; every
message arrives through `udpreceive @defer 1`, in Max's main thread.

- **Its port and its announce:** at load it draws its poll port (`random
  10000` `+ 40000`, the LUFS devices' range) for its `udpreceive` and,
  when its Device On is 1, sends one announce datagram to the hub's UDP `127.0.0.1:39111`
  (`/fohmixer/listen`, wire version 2: the device's listen id, its Max id
  mixed with a random number drawn at load so the PC's two Lives never share
  one, `v<version>`, its four canonical parents' names, Live's sample rate,
  whether it streams, its poll port). It announces again when its Device On
  turns 1 (`is_active` observed: the hub switches the `Listen` rack on for a
  listen, #238) and answers every poll of the hub. A device whose Device On
  is 0 answers nothing (the gate, as the meter's). The hub's move
  (`/fohmixer/listen/move`, #264: another device drew the same port, and
  only the first binder hears it) draws the port again and announces it
  through the load's own path (`t b b`: the draw, then the gate), so the
  device is off until the hub's next poll.
- **The poll** (`/fohmixer/listen/poll <on>`, every 250 ms from the hub to
  each device it heard): the select pushed, never asked. 1 turns it on: the
  hello (`fohmixer_listen 2 <id> v<version> <sample rate>`) through
  `jit.net.send`'s right inlet, then the frame gate opens; 0 turns it off at
  once. Each poll is answered with the announce (and, while on, the hello
  again: the hub records only a change). The hub forgets a device unheard
  for 3 s. A hub started under a running Live polls every port of the range
  once (its start sweep). The names are the LUFS devices' identity
  (`live.object` `get name` for each parent, read when its path gives an id
  and when a `live.observer` on its `name` reports, the observer's output
  only a trigger; the set has no observer; the format is proved only on the
  PC, `../FohMixer-LUFS/README.md`), read at load, on a change and on every
  40th poll (10 s), at a count drawn at load.
- **The frame poll** (`/fohmixer/listen/frame`, every 20 ms from the hub to
  the device selected): through the frame gate it bangs
  `jit.catch~ 2 @mode 1 @framesize 960`, which hands out every whole 20 ms
  frame caught since the bang before (2 planes of float32, 960 × k cells),
  and `jit.net.send` writes the matrix to the hub's TCP `127.0.0.1:39111`.
  A late or bunched poll only puts more frames in one matrix (the hub takes
  up to 50). `jit.catch~` keeps copying the signal into its own buffer all
  the while; nothing else is banged out of it.

*History:* until #264 (1.60.4) the device asked the hub's HTTP route
`GET /api/listen/select?device=<id>` with a `qmetro 250` → `maxurl`,
stopped 1 s after its last `on` (`delay 1000` → `deferlow`), announced each
second (`metro 1000` → `deferlow`), read its parents again on a `qmetro
10000`, and a `qmetro 10` banged the catch while selected.

The wire, the hub's side and the PC probe: `.claude/rules/listen.md` and the
design comments on #210 and #264. `scripts/check_listen_device.py` (CI, with
`scripts/listen_pull_check.py`) holds the patch to it and fails on any clock.
Built like the other devices (never committed as `.amxd`):

    python3 live-device/FohMixer-LUFS/build_amxd.py --patch "live-device/FohMixer-Listen/FohMixer Listen.maxpat"

`--port` (the `udpsend` and `jit.net.send` boxes) rewrites its port for
another hub config. `version.txt` is the workspace version
(`scripts/check_version.py`).
