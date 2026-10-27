# FohMixer LUFS and FohMixer Meter (Max for Live audio effects, #151)

Two devices without parameters that send their readings to the FohMixer hub
on the same PC over UDP (spec `docs/superpowers/specs/2026-10-26-lufs-design.md`
§3.3, §3.4, §4.2, §4.4):

- **`FohMixer LUFS`** (this folder) measures BS.1770-4 loudness on a stereo
  input and sends, 5×/s (every 200 ms: the readings are a 3 s window shown
  as integers, the owner's ruling of 27.10):
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
`/fohmixer/lufs` or `/fohmixer/meter`, type tags `,iisssssii`:

| # | type | value |
|---|---|---|
| 1 | `i` | the wire version, `1` |
| 2 | `i` | the device's Max id (`live.path this_device`, per Live process) |
| 3 | `s` | `v<version.txt>` |
| 4–7 | `s` | the names of the device's first four canonical parents, nearest first, `-` past the top |
| 8 | `i` | `S`, or the L peak, in tenths (LUFS: −700…0; dBFS: −700…120) |
| 9 | `i` | `V`, or the R peak, in tenths |

The parents name the device's place: on a track `[track, set, -, -]`, in a
rack's chain `[chain, rack, track, set]`; the hub takes the last named one as
the set and the one before it as the track (`fohmixer_proto::lufs::wire`).
Max's `live.path` gives no absolute path (`getpath` echoes the path it was
given), so the device reads its id and each parent's `name` instead, at load
and every second, on Max's low-priority thread (`deferlow`). The whole
message is one `pack` (its slots cold, the identity cached in them), banged
by `obj-send-trigger` (`t b f`) each time the last value comes out of its
audio object's own clock (`snapshot~ 200` for the LUFS device, `peakamp~ 33`
for the meter): those output only while Live's DSP runs the device, so a
device switched off (or its chain) sends nothing and the hub sees it go
stale; no `metro` drives the send (the review of PR 2: one kept sending a
frozen value). The hub gets one datagram per reading.

The hub binds `127.0.0.1` only. Another port than 39110 (the hub's
`[lufs] port`) means the devices rebuilt with `--port` (below): a device
cannot read the hub's config.

## Files

| File | What it is |
|---|---|
| `FohMixer LUFS.maxpat` | The LUFS device: `sig~ 0.` → `plugout~`; `plugin~` → `gen~` (its code embedded) → `snapshot~ 200` ×2 → `* 10.` → `round` → `clip` → the `pack`'s two values, `V` through `t b f`, which bangs the `pack`; the identity; `udpsend`. |
| `fohmixer_lufs.genexpr` | The readable copy of the gen~'s code. |
| `../FohMixer-Meter/FohMixer Meter.maxpat` | The meter device: `sig~ 0.` → `plugout~`; `peakamp~ 33` per channel → `atodb` → tenths → the `pack`, `R` through `t b f`, which bangs it; the identity; `udpsend`. |
| `version.txt`, `../FohMixer-Meter/version.txt` | The workspace version (`scripts/check_version.py` keeps both equal to it); each face and each `pack` carry `v<version>`. |
| `build_amxd.py` | Writes either device's `.amxd` from its patcher. |

`scripts/check_lufs_device.py` (CI) fails, for either patch, on a parameter
object, on anything but silence into `plugout~`, on a `udpsend` other than
one `udpsend 127.0.0.1 <DEFAULT_PORT>` (the hub's, read from
`crates/fohmixer-proto/src/lufs/wire.rs`), on a `pack` with another address
or version, on a missing identity message, on a line past a box's ports, on a
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
   receiver on the port gets one datagram per reading.
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
