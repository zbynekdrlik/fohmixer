# FohMixer Listen (Max for Live audio effect, #210)

Streams its input, the track's sound before the track's fader, to the
FohMixer hub on `127.0.0.1` while the hub selects it, so a page can listen to
the track. No parameters. Its output is silence: it goes in a rack of its own
(an empty dry chain plus the device's chain), never in series, as the LUFS
devices (`../FohMixer-LUFS/README.md`).

- **Idle** (not selected): each second one announce datagram to the hub's
  UDP `127.0.0.1:39111` (`/fohmixer/listen`: the device's listen id, its Max
  id mixed with a random number drawn at load so the PC's two Lives never
  share one, `v<version>`,
  its four canonical parents' names, Live's sample rate, whether it streams).
  The names are read at load and again only on a change (#240, the LUFS
  devices' identity: `live.object` `get name` for each parent, read when its
  path's follow output gives an id and when a `live.observer` on its `name`
  reports, the observer's output only a trigger; the set has no observer
  and takes its id from both `live.path` outlets; the format is proved only
  on the PC, `../FohMixer-LUFS/README.md`); the
  announce resends the stored ones each second (the hub forgets a device
  after 3 s) and asks Live nothing.
  `jit.catch~` keeps copying the signal into its own buffer; nothing is
  banged out of it.
- **Asking** (always): every 250 ms a `maxurl` request
  `GET http://127.0.0.1:8480/api/listen/select?device=<id>` (the hub's HTTP
  port, `parse_type json`); an answer whose `on` is true selects it, any
  other answer stops it, and with no answer it stops 1 s after the last
  `on`. Each device asks for itself: one UDP select port shared by every
  device reached only one of them on the PC (#210). The poll is a
  `qmetro 250` and the hold's end goes through a `deferlow` (#240): Max for
  Live runs its scheduler inside Live's audio thread, so no request, and no
  hello it leads to, starts there.
- **Selected**: it sends its hello
  (`fohmixer_listen 1 <id> v<version> <sample rate>`) through `jit.net.send`'s
  right inlet, then a `qmetro 10` bangs `jit.catch~ 2 @mode 1 @framesize 960`,
  whose matrices (2 planes of float32, 960 × k cells: whole 20 ms frames at
  48 kHz) `jit.net.send` writes to the hub's TCP `127.0.0.1:39111`.

The wire, the hub's side and the PC probe: `.claude/rules/listen.md` and the
design comment on #210. `scripts/check_listen_device.py` (CI) holds the patch
to it. Built like the other devices (never committed as `.amxd`):

    python3 live-device/FohMixer-LUFS/build_amxd.py --patch "live-device/FohMixer-Listen/FohMixer Listen.maxpat"

`--port` (the `udpsend` and `jit.net.send` boxes) and `--hub-port` (the
select poll's URL) rewrite its ports for another hub config. `version.txt` is the workspace version (`scripts/check_version.py`).
