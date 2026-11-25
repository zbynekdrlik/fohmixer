# FohMixer LUFS and FohMixer Meter (Max for Live audio effects, #151)

Two devices without parameters that send their readings to the FohMixer hub
on the same PC over UDP (spec `docs/superpowers/specs/2026-10-26-lufs-design.md`
§3.3, §3.4, §4.2, §4.4):

- **`FohMixer LUFS`** (this folder) measures BS.1770-4 loudness on a stereo
  input and answers the hub's poll, 5×/s (every 200 ms: the readings are a
  3 s window shown as integers, the owner's ruling of 27.10; since #264 the
  hub asks, below):
  - `S`, the 3 s short-term loudness, no gate;
  - `V`, the loudness of the voiced 100 ms sub-blocks among the last 30 (a
    sub-block is voiced when its 400 ms momentary loudness is at least
    −40 LUFS), valid with at least 10 of them, else −70.
- **`FohMixer Meter`** (`../FohMixer-Meter/`) sends its L and R peak levels
  over each 33 ms, 30×/s. It goes last in an armed track's chain (in a rack,
  below); the hub turns its levels into the track's post-fader meter.

Both output silence (`plugout~` takes a `sig~ 0.` and nothing else) and are
ALWAYS placed in a parallel chain of an Audio Effect Rack, never in series
(the owner's ruling of 27.10: a plug-in or licence failure must never cut the
sound, and a doubled signal is worse than none): a mic's LUFS device in its
own chain of `CORE Effects` beside trueLevel; an output point's LUFS device
and an armed track's meter device in a rack of their own, an empty dry chain
plus a chain holding the device.

Nothing is a parameter (probe P1: a value a device writes into a parameter
is an Undo step, and with "Stored Only" it leaves the LOM). The LUFS face
shows `Short-term` and `Voice` big under the caption `LUFS` (one device for
every point: whether a number is IN or OUT is the hub's tag), the version
small; the meter's face its L and R. All display only (`flonum` with
`ignoreclick`, comments).

## What they send

One OSC message per reading to `127.0.0.1:39110` (Max's `udpsend`): address
`/fohmixer/lufs` or `/fohmixer/meter`, type tags `,iisssssii` (the meter,
wire version 1) or `,iisssssiii` (the LUFS device since #264, wire version
2: one more field, its poll port):

| # | type | value |
|---|---|---|
| 1 | `i` | the wire version, `1` (meter) or `2` (LUFS) |
| 2 | `i` | the device's Max id (`live.path this_device`, per Live process) |
| 3 | `s` | `v<version.txt>` |
| 4–7 | `s` | the names of the device's first four canonical parents, nearest first, `-` past the top |
| 8 | `i` | `S`, or the L peak, in tenths (LUFS: −700…0; dBFS: −700…120) |
| 9 | `i` | `V`, or the R peak, in tenths |
| 10 | `i` | LUFS only: the port it takes the hub's poll on, 40000–49999 |

The hub still reads wire version 1 from a LUFS device (one from before
1.58.0 during the rollout): it then only listens, as before.

The parents name the device's place: on a track `[track, set, -, -]`, in a
rack's chain `[chain, rack, track, set]`; the hub takes the last named one as
the set and the one before it as the track (`fohmixer_proto::lufs::wire`).
Max's `live.path` gives no absolute path (`getpath` echoes the path it was
given), so the device reads its id and each parent's `name` instead: at
load on Max's low-priority thread (`deferlow`), on a change, and every 10 s
(#240; the old read every second cost every device 9 LOM calls a second):
each parent's `live.path` (both outlets into its `route id`: the left
answers every `path` message, the load's and the 10 s re-read's, as the
devices before #240 read every parent each second for months; the middle
follows the path) sets that id into a `live.object`
and reads `get name` → `route name` → `zl reg` → `tosymbol` into the
parent's slot, the path the devices before #240 used; a path that holds
nothing gives `-`. A `live.observer` on the `name` of parents 1 to 3 gives
the name at once and at each rename, and its output only bangs that read
(`t b`), never a value: through `live.observer` → `tosymbol` a name with a
space kept literal double quotes and the set's level read `-`, so on the PC
no device matched (the FINDING of 2026-11-17 on #240). The set (parent 4)
read `-` there too, and the cause is not determined: the `pack`'s own
default is `-`, so either its observer reported nothing or the middle
outlet never gave an id for the set's path. So the set has no observer;
like every parent, it takes its id from both of its `live.path`'s outlets.

The follow does not cover every move: on the PC a device moved from its
track into a rack's chain kept announcing its track and set for over 30 s
(the PC proof's attempt 2 on #240). So each device sends its four parents'
`path` messages again every 10 s (the DECIDED of 2026-11-17):
`live.thisdevice` → `random 10000` (an offset drawn once at load) → `delay`
→ `deferlow` → `1` → `qmetro 10000` → `t b b b b` → the paths, parent 1
first and the set last, as at the load. Each `live.path` resolves its path
anew, its left outlet gives the id, and the same `live.object` read
follows, so a move is followed within
10 s, on the low-priority queue, and the devices loaded together with a set
read at their own instants. It is the only clock of the identity
(`check_lufs_device.py` pins its boxes and every line into and out of
them, and names any other clock, a faster one included). Per tick and
device: 4 `live.path` resolves, then for each parent an id from the left
outlet and possibly one from the middle, each setting the `live.object`
(and the observer of parents 1 to 3, which may report once more) and
reading `get name`: at most 8 ids, 6 observer sets and 14 `get name`, the
same name read again: at most about 30 LOM calls every 10 s against the
old 90 (5 paths and 4 names each second), a third or less. A `Listen`
device in a rack that is off may keep
this clock too (Max's scheduler may run on), so the PC's A/B includes the
idle racks.

**The identity's format is proved only on the PC** (CI checks which boxes
are wired to which, never what Max outputs; `scripts/check_lufs_device.py`
`value_errors` holds the value path). Before a changed device reaches a
set:

1. Copy its `.amxd` under a test name into the band user's
   `Presets\Audio Effects\Max Audio Effect`.
2. Pick a track whose existing device sits in a chain whose chain or rack
   name holds a space, so the quote fault is exercised.
3. Load the copy through the LOM (the hub's `cmd`: `load_item` appends it
   to the selected track) and `move_device` it into that same chain, as the
   rollout places a device (`.claude/rules/deploy-pc.md`): only there are
   its parents the old device's.
4. Read its announce from the hub (`/api/status` `lufs.devices`, or
   `listen.devices` for Listen) **within 10 s of the move** (the 10 s
   re-read follows a move the `live.path` follow misses; the attempt 2 of
   #240 read the old parents for over 30 s): `parents` equal the old
   device's, no quotes, and **the set's name last** (the decisive check:
   the set read `-` in the failed rollout). For that moment two devices
   announce the same parents. Its device id stays the one it announced
   before the move. Move it once more (another chain, or back to the
   track's end) and read the new parents within 10 s again; rename its
   chain and read the new name within a second (the observer). A pass
   proves that a `path` sent again gives the new parent's id (the left
   outlet answers every `path` message, as before #240); old parents after
   10 s mean it does not: record a FINDING on #240, roll nothing out and
   save no set.
5. Delete the copy from the chain, leave the set unsaved (the set file's
   time unchanged) and delete the test `.amxd` from the Presets folder.

The whole
message is one `pack` (its slots cold, the identity cached in them), banged
by `obj-send-trigger` (`t b f`) each time the last value comes out.

**The meter** sends on its audio object's own clock (`peakamp~ 33`): it
outputs only while Live's DSP runs the device, so a device switched off (or
its chain) sends nothing and the hub sees it go stale; no `metro` drives the
send (the review of PR 2: one kept sending a frozen value). The `pack` goes
to `udpsend` through `obj-send-defer` (`deferlow`, #240): the clock runs on
Max's scheduler, which Max for Live runs inside Live's audio thread, and the
socket write leaves it for the low-priority queue.

**The LUFS device is pulled by the hub (#264, the DECIDED of 25.11.2026).**
That `deferlow` hand-over is a `PostMessageW` from Live's audio thread, and
it waits on the desktop's window lock while a GUI thread holds it: a missed
audio period, a click on the PA (dasio's kernel traces of 25.11, the stack
`PostMessageW <- maxplug <- gen~.mxe64`). The probes of that day: a `qmetro`
pull still posts (`metro.mxe64`), the send without `deferlow` still posts
from `gen~` and from the face's number boxes, and only a reading pulled from
Max's main thread posts nothing (probe B3). So nothing in the device starts
a message in the audio thread:

- both readers are `snapshot~ 0` (no clock of their own); a `bang` reads
  them in the thread that sends it;
- `obj-poll-receive` (`udpreceive`) takes the hub's poll,
  `/fohmixer/lufs/poll` with no argument, every 200 ms (`route` → the
  gate's right inlet); it is created on 39999, below the range the hub
  polls, and the port is drawn once at load (`random 10000` → `+ 40000`, the
  load's first step) into the `udpreceive` (`port N`) and the `pack`'s last
  slot, so the hub learns it from the datagram;
- `obj-pull-gate` (`gate`) passes a bang only while the device is on: a
  `live.observer` on `this_device` `is_active` (its `live.path`'s left
  outlet straight into the observer's right inlet, as in probe B3; through
  a `route id` the id never came, B2) sets its left inlet; `is_active` is
  false when the device or a rack around it is off, so a switched-off
  device answers nothing and the hub sees it go stale (its port polled for
  one stale window more, then dropped). Device On turning 1 sends one
  datagram at once (`t i i` → `sel 1`), so the hub finds it again;
- the gate opens to `obj-pull` (`t b b`: the short-term first, then the
  voice) → `* 10.` → `round` → `clip` → the `pack` → `udpsend`, with no
  `deferlow`; the face's `Short-term` and `Voice` are fed from the same
  readers, so they too redraw in the main thread only;
- one datagram goes at load (the load's last step, after the identity) and
  at each 10 s re-read (the tick's last outlet), so the hub learns a new
  device and its port.

**The hub** (`crates/fohmixer-hub/src/lufs_udp.rs`) sends every port a
polled-form datagram named the poll each 200 ms from a socket of its own
(`127.0.0.1`), as long as that key is heard; a key lost (1 s, the LUFS
rule's stale window, five polls) drops its port. `/api/status`
`lufs.listener` counts `polled` (polls sent), `poll_failed` and `polling`
(the ports polled now). After a hub restart a device's readings come back
at its next 10 s re-read (the hub learns the port from that datagram).

**Not yet proved on the PC:** that `udpreceive` outputs its messages in
Max's main thread. Probe B3 proved the pull for a `live.observer`'s output
only; the design (the DECIDED of 25.11) takes the network object's output
as the main thread's too. dasio's stack trace with the new device answers
it (no `PostMessageW` stack on a Live audio thread naming `udpreceive`,
`snapshot~`, `number` or `gen~`). If it posts, the next step is a design
decision on #264, never a quiet `deferlow`: the check refuses one on the
send path.

Known limit: two devices that draw the same port
(1 in 10 000 per pair) share one socket; Max's `udpreceive` cannot say a
bind failed, so one of them may go stale: delete it and load it again
(a new draw). Max's `udpreceive` may listen on every interface, so a
datagram from the LAN with the poll's address could ask for a reading; it
only makes the device send its reading to the local hub.

The hub binds `127.0.0.1` only. Another port than 39110 (the hub's
`[lufs] port`) means the devices rebuilt with `--port` (below): a device
cannot read the hub's config.

## Files

| File | What it is |
|---|---|
| `FohMixer LUFS.maxpat` | The LUFS device: `sig~ 0.` → `plugout~`; `plugin~` → `gen~` (its code embedded) → `snapshot~ 0` ×2, banged by the hub's poll through the `is_active` gate → `* 10.` → `round` → `clip` → the `pack`'s two values, `V` through `t b f`, which bangs the `pack`; the identity; the poll port; `udpsend` (no `deferlow`). |
| `fohmixer_lufs.genexpr` | The readable copy of the gen~'s code. |
| `../FohMixer-Meter/FohMixer Meter.maxpat` | The meter device: `sig~ 0.` → `plugout~`; `peakamp~ 33` per channel → `atodb` → tenths → the `pack`, `R` through `t b f`, which bangs it; the identity; `udpsend`. |
| `version.txt`, `../FohMixer-Meter/version.txt` | The workspace version (`scripts/check_version.py` keeps both equal to it); each face and each `pack` carry `v<version>`. |
| `build_amxd.py` | Writes either device's `.amxd` from its patcher. |

`scripts/check_lufs_device.py` (CI) fails, for the LUFS patch, on any
`deferlow` on the send path, on a `snapshot~`/`peakamp~` with its own clock
on, on a clock (`metro`, `qmetro`, `delay`, ...) but the identity's driving a
reading, on a face object fed from an audio clock, and on any change to the
pull's boxes and lines (`pull_errors`); for either patch, on a parameter
object, on anything but silence into `plugout~`, on a `udpsend` other than
one `udpsend 127.0.0.1 <DEFAULT_PORT>` (the hub's, read from
`crates/fohmixer-proto/src/lufs/wire.rs`), on a `pack` with another address
or version, on a missing identity message, on an identity read on a clock
or a name slot fed by anything but `live.object` → `route name` → `zl reg`
→ `tosymbol` (an observer's output reaching it), on a line past a box's ports, on a
face whose version is not `version.txt`, on an absolute file path, and for
the LUFS patch when its embedded code differs from the `.genexpr`. A change to
the DSP is made in the `.genexpr` and copied into the patcher's codebox (or
made in Max's codebox and copied back).

## Building the devices

The `.amxd` is never committed (`.gitignore`): a frozen device can embed the
author's file paths, which hold the Windows user name.

1. `python3 live-device/FohMixer-LUFS/build_amxd.py` writes
   `FohMixer LUFS.amxd` beside its patcher, and
   `python3 live-device/FohMixer-LUFS/build_amxd.py --patch "live-device/FohMixer-Meter/FohMixer Meter.maxpat"`
   writes `FohMixer Meter.amxd` beside that one: the unfrozen audio effect
   (`ampf`/`aaaa` header, then the patcher JSON byte for byte). `--port <n>`
   rewrites only the `udpsend` box's port in the written file, never the
   source; `--out <file>` writes elsewhere.
   CI's `bundle` job builds both this way into the release bundle's
   `live-device/` folder.
2. `Install-Fohmixer.ps1` writes both into each Live user's
   `<User Library>\Presets\Audio Effects\Max Audio Effect\` (only when they
   differ), where Live's browser lists them. Open each in Live's Max editor
   once: no Max error, the face shows its readings and `v<version>`, and a
   receiver on the port gets one datagram per reading (the LUFS device's
   only while the hub polls it: `/api/status` `lufs.listener.polling`
   counts it, `polled` grows by 5 a second per device). A changed `.amxd`
   is held while a Live runs it: replace it with both Lives closed.
3. Place them through the LOM, never by a GUI drag (the owner's ruling of
   27.10): select the track, load the User Library's browser item with
   `browser load_item` (it appends the device to the selected track's chain),
   then `live_set move_device(device, chain, position)` into its rack's
   chain (above: a parallel chain, never in series).
4. A device frozen in the Max editor is never copied back into this
   repository (the next install that differs replaces it with the bundle's).

## The check against trueLevel

The LUFS device's numbers are checked on the PC against Sonible trueLevel's
short-term on the same signal (spec §9, the owner's ruling of 27.10): our `V`
right after trueLevel in the same chain, within 0.5 LU on a played song.
