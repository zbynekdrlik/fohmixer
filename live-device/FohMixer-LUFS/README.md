# FohMixer LUFS and FohMixer Meter (Max for Live audio effects, #151)

Two devices without parameters that send their readings to the FohMixer hub
on the same PC over UDP (spec `docs/superpowers/specs/2026-10-26-lufs-design.md`
§3.3, §3.4, §4.2, §4.4):

- **`FohMixer LUFS`** (this folder) measures BS.1770-4 loudness on a stereo
  input and answers the hub's poll, 5×/s (every 200 ms: the readings are a
  3 s window shown as integers, the owner's ruling of 27.10; since #264 the
  hub asks, below):
  - `S`, the 3 s short-term loudness, no gate;
  - `V`, the loudness of the voiced part of the last 3 s (an instant is
    voiced when its 400 ms momentary loudness is at least −40 LUFS), valid
    when at least a third of the 3 s is voiced (the block form's 10 of 30
    sub-blocks), else −70. Since #264 the gate is continuous (below), not
    per 100 ms sub-block.
- **`FohMixer Meter`** (`../FohMixer-Meter/`) answers the hub's poll with
  its L and R peak levels since the previous poll, 30×/s (every 33 ms;
  pulled since its step of #264, `../FohMixer-Meter/README.md`). It goes
  last in an armed track's chain (in a rack, below); the hub turns its
  levels into the track's post-fader meter.

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
`/fohmixer/lufs` or `/fohmixer/meter`, type tags `,iisssssiii` (both
devices since #264, wire version 2: the pushed form's `,iisssssii` and one
more field, its poll port):

| # | type | value |
|---|---|---|
| 1 | `i` | the wire version, `2` (`1` before the pull) |
| 2 | `i` | the device's Max id (`live.path this_device`, per Live process) |
| 3 | `s` | `v<version.txt>` |
| 4–7 | `s` | the names of the device's first four canonical parents, nearest first, `-` past the top |
| 8 | `i` | `S`, or the L peak, in tenths (LUFS: −700…0; dBFS: −700…120) |
| 9 | `i` | `V`, or the R peak, in tenths |
| 10 | `i` | the port it takes the hub's poll on, 40000–49999 |

The hub still reads wire version 1 (a device from before its pull during
the rollout: LUFS before 1.58.0, a meter before its step of #264): it then
only listens, as before.

The parents name the device's place: on a track `[track, set, -, -]`, in a
rack's chain `[chain, rack, track, set]`; the hub takes the last named one as
the set and the one before it as the track (`fohmixer_proto::lufs::wire`).
Max's `live.path` gives no absolute path (`getpath` echoes the path it was
given), so the device reads its id and each parent's `name` instead: at
load (straight from `live.thisdevice` since #264; a `deferlow` before),
on a change, and every 10 s
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
`path` messages again every 10 s (the DECIDED of 2026-11-17). Since #264
(the meter's step, then the LUFS device's) the re-read rides the hub's
poll, with no clock: `live.thisdevice` → `random 50` (the meter's `random
300`) → `prepend set` → `counter 0 49` (`counter 0 299`), banged first by
each poll (the pull's rightmost outlet) → `sel 0` → `t b b b b` → the
paths, parent 1 first and the set last, as at the load: every 50th poll at
200 ms (the meter's 300th at 33 ms, 9.9 s), each device from its own place
in the count, and before that poll's readings, so its datagram already
names the new parents. Each `live.path` resolves its path anew, its left
outlet gives the id, and the same `live.object` read follows, so a move is
followed within 10 s, in Max's main thread, and the devices loaded together
with a set read at their own instants. (Until #264 it was `random 10000` →
`delay` → `deferlow` → `1` → `qmetro 10000`; the Listen device keeps that
clock.) `scripts/poll_reread_check.py` pins the re-read's boxes and every
line into and out of them, and names any other clock. Per tick and
device: 4 `live.path` resolves, then for each parent an id from the left
outlet and possibly one from the middle, each setting the `live.object`
(and the observer of parents 1 to 3, which may report once more) and
reading `get name`: at most 8 ids, 6 observer sets and 14 `get name`, the
same name read again: at most about 30 LOM calls every 10 s against the
old 90 (5 paths and 4 names each second), a third or less. A pulled
device that is off (it, or a rack around it) answers no poll, so it
re-reads nothing until Device On turns 1 again (a datagram at once, and the
hub polls it again); a `Listen` device in a rack that is off may keep its
clock (Max's scheduler may run on).

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

**The meter** is pulled too since its step of #264 (the design of
26.11.2026, `../FohMixer-Meter/README.md`): the hub polls it every 33 ms
with its own address, it bangs two interval-less `peakamp~` in Max's main
thread and sends straight into `udpsend`; its load is not deferred and its
10 s re-read rides the poll (every 300th), so the meter holds no
`deferlow`, `metro`, `qmetro` or `delay`; the LUFS device neither since
1.60.3 (below). *History:* until then it sent on its own
`peakamp~ 33` clock, the `pack` through a `deferlow` (#240), both from
Live's audio thread.

**The LUFS device is pulled by the hub (#264, the DECIDED of 25.11.2026).**
That `deferlow` hand-over is a `PostMessageW` from Live's audio thread, and
it waits on the desktop's window lock while a GUI thread holds it: a missed
audio period, a click on the PA (dasio's kernel traces of 25.11, the stack
`PostMessageW <- maxplug <- gen~.mxe64`). The probes of that day: a `qmetro`
pull still posts (`metro.mxe64`), the send without `deferlow` still posts
from `gen~` and from the face's number boxes, and only a reading pulled from
Max's main thread posts nothing (probe B3). So nothing in the device starts
a message in the audio thread:

- the three readers are `snapshot~ 0` (no clock of their own); a `bang`
  reads them in the thread that sends it;
- `obj-poll-receive` (`udpreceive 39999 @defer 1`) takes the hub's poll,
  `/fohmixer/lufs/poll` with no argument, every 200 ms (`route` → the
  gate's right inlet). `@defer 1`: Max's reference says "messages are sent
  out on the low-priority queue instead of the scheduler thread.
  Especially in Max for Live, this can reduce the chances of audio glitches
  and dropouts"; without it v1.58.0 delivered the poll on the scheduler,
  i.e. in Live's audio thread (dasio's trace of 25.11 20:01Z: 5 stacks
  `udpreceive -> snapshot~ -> number -> PostMessageW`). It is created on
  39999, below the range the hub
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
- the gate opens to `obj-pull` (`t b b b b`: the re-read's count first,
  above, then the short-term, then the voiced share into the Voice level's
  cold inlet, then the voiced mean) →
  the level (`expr`, below) → `* 10.` → `round` → `clip` → the `pack` →
  `udpsend`, with no `deferlow`; the face's `Short-term` and `Voice` show
  the clipped tenths (`* 0.1`), fed from the poll only, so they too redraw
  in the main thread only;
- one datagram goes at load (the load's last step, after the identity), so
  the hub learns a new device and its port; after that the device sends
  only when polled (and when Device On turns 1): until 1.60.3 the 10 s
  re-read's tick sent one too, from its `qmetro`;
- nothing in the device runs a clock or hands a message over (#264, 1.60.3:
  the identity's `qmetro 10000`, its `delay` and the two `deferlow`s went;
  `live.thisdevice` bangs the load and `dspstate~` directly):
  `scripts/poll_reread_check.py` fails on any `deferlow`, `defer`,
  `metro`, `qmetro`, `delay`, `del` or `pipe` in either pulled device.

**The hub** (`crates/fohmixer-hub/src/lufs_udp.rs`) sends every port a
polled-form datagram named the poll each 200 ms from a socket of its own
(`127.0.0.1`), as long as that key is heard; a key lost (1 s, the LUFS
rule's stale window, five polls) drops its port. `/api/status`
`lufs.listener` counts `polled` (polls sent), `poll_failed` and `polling`
(the ports polled now). After a hub restart every device's readings come
back at once, from the hub's start sweep (#264: each kind's poll once to
every port of the range, the LUFS device's then the meter's; each device
answers its own address and its `route` drops the other's; `swept` /
`sweep_failed` count both, `../FohMixer-Meter/README.md`). Until 1.60.3 a
LUFS device came back at its next 10 s re-read's datagram.

Known limit: two devices that draw the same port
(1 in 10 000 per pair) share one socket; Max's `udpreceive` cannot say a
bind failed, so one of them may go stale: delete it and load it again
(a new draw). Max's `udpreceive` may listen on every interface, so a
datagram from the LAN with the poll's address could ask for a reading; it
only makes the device send its reading to the local hub.

The hub binds `127.0.0.1` only. Another port than 39110 (the hub's
`[lufs] port`) means the devices rebuilt with `--port` (below): a device
cannot read the hub's config.

## The DSP without gen~ (#264, the DECIDED of 25.11.2026)

Until 1.59 the measurement ran in one `gen~`. dasio's symbolized stacks of
25.11 showed why it still posted after the pull: `plugrunner_process ->
sched_setqueuethrottle -> gen~.mxe64+0xc442 -> qelem_set -> PostMessageW`,
on a 4 Hz grid independent of the poll: the `gen~` runs a clock of its own,
Max for Live services it inside Live's audio thread, and each tick wakes
Max's main thread with a `PostMessageW`, which waits on the desktop's
window lock (the v1.58.0 proof run looped master at start 8 on exactly
that). Its code has no message outputs, so no change of the code removes
it. So the device has no `gen~` (the check fails on any gen object,
subpatchers included), and the same math runs in plain MSP objects, none
with a clock of its own:

| Step | Boxes |
|---|---|
| K-weighting, per channel: BS.1770's high shelf, then the RLB high-pass (b = 1, −2, 1) | `obj-plugin` L/R → `obj-shelf-l`/`-r` → `obj-hp-l`/`-r` (`biquad~`) |
| Squared and summed over the channels | `obj-square-l`/`-r` (`*~`, the signal in both inlets) → `obj-sum` (`+~`) |
| Short-term: the 3 s mean of the sum | `obj-avg-short` (`average~ 576000 bipolar`) → `obj-snap-s` |
| The gate: the 400 ms mean ≥ the gate as a mean square, 10^((−40 + 0.691)/10) | `obj-avg-momentary` (`average~ 76800 bipolar`) → `obj-gate` (`>=~ 0.000117…`) |
| The voiced share: the 3 s mean of the gate (0…1) | `obj-avg-share` → `obj-snap-g` |
| The voiced mean: the 3 s mean of sum × gate | `obj-voiced` (`*~`) → `obj-avg-voiced` → `obj-snap-v` |
| Short-term level: 10·log10(max(mean, 1e-30)) − 0.691 | `obj-level-s` (`expr`) → `* 10.` → `round 1.` → `clip -700 0` |
| Voice level: the same of voiced mean ÷ share when the share ≥ 10/30, else −70 | `obj-level-v` (`expr`, the share in its cold inlet) → the same three |

The coefficients and the windows are computed in Max messages, never on a
clock: `obj-sr-state` (`dspstate~`) gives the sample rate when the audio
starts (a rate change restarts it) and when banged, which the load does
once from `live.thisdevice` (since #264; the identity's `deferlow` before); `t f
f f f` → `expr tan(π f0 / sr)` per filter → one `expr` per coefficient,
the analog-prototype formulas the gen~ code used (libebur128's form) →
`pack` → the `biquad~`'s left inlet as one list, so the five change
together (each filter's `t` fires right to left: the cold inlets first, the
pack's hot inlet, b0 or the high-pass's bang, last; a test plays that
order); the windows `expr int(3.0 sr + 0.5)` and `int(0.4 sr + 0.5)`
into each `average~`. The texts are generated by
`scripts/lufs_dsp_check.py` from its constants, which the check compares
with the patch and a test evaluates against BS.1770's published 48 kHz
coefficients (the test allows 1e-6; they agree to 1e-15) and the
0.691 dB gain at 997 Hz (48 and 96 kHz). Every long number in an `expr` is glued to an operator (a lone
number between spaces would become a float atom), commas inside calls are
escaped (`\,`) and no expression starts with a unary minus.

The continuous gate is close to the block form (a sub-block of 100 ms was
voiced when its 400 ms momentary reached −40 LUFS; now every instant is),
and Voice is fohmixer's own reading (§3.3); `S` is BS.1770's short-term,
a plain 3 s mean, as before.

What Max's reference says about the objects (docs.cycling74.com, read
25.11.2026) and what it leaves open:

- `biquad~`: `y[n] = a0*x[n] + a1*x[n-1] + a2*x[n-2] - b1*y[n-1] -
  b2*y[n-2]`; a list of five into the left inlet sets a0 a1 a2 b1 b2 in
  that order (so the shelf's list is BS.1770's b0 b1 b2 a1 a2, and the
  high-pass's `1 -2 1 a1 a2`); inlets 2–6 take one coefficient each (left
  empty: a signal there would override the list); `smooth` (on by
  default) smooths coefficient changes.
- `average~`: the first argument is the maximum interval in samples
  (default 100, no upper limit stated), the second the mode; `bipolar`
  averages the sample values (the plain mean we need; `rms` takes the root
  and costs more); an `int` sets the interval. Not stated: that the
  interval is bounded by the maximum, how it starts before a full window
  has arrived (taken here as zeros, like the gen~'s rings), and its cost
  per sample at a 576 000-sample window.
- `dspstate~`: outputs automatically when the audio is turned on or off,
  and on a bang; its second outlet is the sample rate; it outputs nothing
  while the audio is off.
- `snapshot~ 0`: "will only output data when it receives a bang message";
  a bang reports the latest signal vector's sample at its `offset`.
- `expr`: `tan`, `pow`, `log10`, `min`, `max`, `int` exist; no `pi`
  constant (written out); its arithmetic precision is not stated.
- `udpreceive`: `@defer 1`, quoted above.

**Open for the PC** (only dasio's trace and the device on the PC settle
these): master's `gen~` stacks go from ~10 per 20 s to 0 with both master
LUFS devices on 1.60.0, and no `PostMessageW` stack on a Live audio thread
names `udpreceive`, `snapshot~`, `number`, `dspstate~` or `average~`
(`@defer 1` takes the poll off the scheduler; `dspstate~` outputs only when
the audio starts and at load); `dspstate~` gives the rate at load inside
Live (else every coefficient stays 0 and both readings sit at −70); the
four `average~` per device (3 × 576 000 + 76 800 samples, about 14 MB of
doubles) cost little CPU in Live; `expr` computes in double precision (in
single precision the high-pass's a1 = −1.990047 would carry ~1e-7, a
negligible change below 40 Hz); the numbers against trueLevel (§9), as
before.

## Files

| File | What it is |
|---|---|
| `FohMixer LUFS.maxpat` | The LUFS device: `sig~ 0.` → `plugout~`; `plugin~` → the K-weighting `biquad~` pairs → squared, summed → the `average~` readers (above) → `snapshot~ 0` ×3, banged by the hub's poll through the `is_active` gate → the levels (`expr`) → `* 10.` → `round` → `clip` → the `pack`'s two values, `V` through `t b f`, which bangs the `pack`; the coefficients from `dspstate~`; the identity, re-read on every 50th poll; the poll port; `udpsend` (no `deferlow`). No `gen~` (#264). |
| `../FohMixer-Meter/FohMixer Meter.maxpat` | The meter device: `sig~ 0.` → `plugout~`; `peakamp~` (no interval) per channel, banged by the hub's poll through the `is_active` gate → `atodb` → tenths → the `pack`, `R` through `t b f`, which bangs it; the identity, re-read on every 300th poll; the poll port; `udpsend` (no `deferlow`). |
| `version.txt`, `../FohMixer-Meter/version.txt` | The workspace version (`scripts/check_version.py` keeps both equal to it); each face and each `pack` carry `v<version>`. |
| `build_amxd.py` | Writes either device's `.amxd` from its patcher. |

`scripts/check_lufs_device.py` (CI) fails, for the LUFS patch, on any
`deferlow` on the send path, on a `snapshot~`/`peakamp~` with its own clock
on, on any `deferlow`, `metro`, `qmetro` or `delay` at all (`poll_reread_check`), on a clock driving a
reading, on a face object fed from an audio clock, on any change to the
pull's boxes and lines (`pull_errors`, `scripts/lufs_pull_check.py`), and on
any change to the DSP chain's boxes, lines or `expr` texts
(`dsp_errors`, `scripts/lufs_dsp_check.py`); for either patch, on a gen
object or codebox anywhere (`gen_errors`), on a parameter
object, on anything but silence into `plugout~`, on a `udpsend` other than
one `udpsend 127.0.0.1 <DEFAULT_PORT>` (the hub's, read from
`crates/fohmixer-proto/src/lufs/wire.rs`), on a `pack` with another address
or version, on a missing identity message, on an identity read on a clock
or a name slot fed by anything but `live.object` → `route name` → `zl reg`
→ `tosymbol` (an observer's output reaching it), on a line past a box's ports, on a
face whose version is not `version.txt`, and on an absolute file path. A
change to the DSP's math is made in `scripts/lufs_dsp_check.py` (its
constants and `EXPRS`) and copied into the patch's `expr` boxes; the check
names every box that differs. The audio lines (an outlet typed `signal`)
carry no message, so the thread checks do not walk through them.

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
