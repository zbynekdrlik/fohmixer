# FohMixer LUFS and FohMixer Meter (Max for Live audio effects, #151)

Two devices without parameters that send their readings to the FohMixer hub
on the same PC over UDP (spec `docs/superpowers/specs/2026-10-26-lufs-design.md`
§3.3, §3.4, §4.2, §4.4):

- **`FohMixer LUFS`** (this folder) measures BS.1770-4 loudness on a stereo
  input and sends, 10×/s:
  - `S`, the 3 s short-term loudness, no gate;
  - `V`, the loudness of the voiced 100 ms sub-blocks among the last 30 (a
    sub-block is voiced when its 400 ms momentary loudness is at least
    −40 LUFS), valid with at least 10 of them, else −70.
- **`FohMixer Meter`** (`../FohMixer-Meter/`) sends its L and R peak levels
  over each 33 ms, 30×/s. It goes LAST in an armed track's chain; the hub
  turns its levels into the track's post-fader meter.

Both pass their input through unchanged (`plugin~` → `plugout~`, nothing
else into `plugout~`), so either is safe anywhere in a chain. In a rack's
parallel chain (the LUFS device beside trueLevel), the rollout sets that
chain's volume to −inf: the devices in it still see its input, and the
rack's sum is unchanged.

Nothing is a parameter (probe P1: a value a device writes into a parameter
is an Undo step, and with "Stored Only" it leaves the LOM). The faces show
the readings in plain number boxes and the version.

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
by a `metro`: the hub gets one datagram per reading.

The hub binds `127.0.0.1` only. Another port than 39110 (the hub's
`[lufs] port`) means the devices rebuilt with `--port` (below): a device
cannot read the hub's config.

## Files

| File | What it is |
|---|---|
| `FohMixer LUFS.maxpat` | The LUFS device: `plugin~` → `plugout~`; `plugin~` → `gen~` (its code embedded) → `snapshot~ 100` ×2 → `* 10.` → `round` → `clip` → the `pack`'s two values; the identity; `udpsend`. |
| `fohmixer_lufs.genexpr` | The readable copy of the gen~'s code. |
| `../FohMixer-Meter/FohMixer Meter.maxpat` | The meter device: `plugin~` → `plugout~`; `peakamp~ 33` per channel → `atodb` → tenths → the `pack`; the identity; `udpsend`. |
| `version.txt`, `../FohMixer-Meter/version.txt` | The workspace version (`scripts/check_version.py` keeps both equal to it); each face and each `pack` carry `v<version>`. |
| `build_amxd.py` | Writes either device's `.amxd` from its patcher. |

`scripts/check_lufs_device.py` (CI) fails, for either patch, on a parameter
object, on anything but `plugin~` into `plugout~`, on a `udpsend` other than
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
2. Copy them to the PC and open each in Live's Max editor once: no Max
   error, the face shows its readings and `v<version>`, and a receiver on
   the port gets one datagram per reading.
3. Freeze them in the Max editor if Live asks for frozen devices, save them
   into the Live user's User Library, and never copy the frozen files back
   into this repository.

## The check against trueLevel

The LUFS device's numbers are checked on the PC against Sonible trueLevel's
short-term on the same signal (spec §9, the owner's ruling of 27.10): our `V`
right after trueLevel in the same chain, within 0.5 LU on a played song.
