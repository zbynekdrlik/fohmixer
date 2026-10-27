# FohMixer LUFS (Max for Live audio effect, #151)

Measures BS.1770-4 short-term loudness on a stereo input and writes two
parameters the hub follows through the LOM (spec
`docs/superpowers/specs/2026-10-26-lufs-design.md` §3.3, §4.2):

- `Short-term`: `S`, the 3 s short-term loudness, no gate;
- `Voice`: `V`, the loudness of the voiced 100 ms sub-blocks among the last
  30 (a sub-block is voiced when its 400 ms momentary loudness is at least
  −40 LUFS), valid with at least 10 of them, else −70.

Both are floats −70…0 in LUFS, updated at most 10×/s and only when the value
rounded to 0.1 changed. The device's audio output is silence, so the chain it
sits in adds nothing to the signal.

## Files

| File | What it is |
|---|---|
| `FohMixer LUFS.maxpat` | The device's patcher: `plugin~` → `gen~` (its code embedded) → `snapshot~ 100` → `change` → the two `live.numbox` parameters; `plugout~` gets the gen~'s two zero outputs. Its face shows the two numbers and the version. |
| `fohmixer_lufs.genexpr` | The readable copy of the gen~'s code. |
| `version.txt` | The workspace version (`scripts/check_version.py` keeps it equal); the face shows `v<version>`. |
| `build_amxd.py` | Writes `FohMixer LUFS.amxd` from the patcher. |

`scripts/check_lufs_device.py` (CI) fails when the code the patcher embeds
differs from the `.genexpr`, when a parameter's name, range or unit is off,
when the face's version is not `version.txt`, or when the patcher holds an
absolute file path. A change to the DSP is made in the `.genexpr` and copied
into the patcher's codebox (or made in Max's codebox and copied back).

## The one open setting: parameter visibility

Both numboxes carry `parameter_invisible` (in
`saved_attribute_attributes.valueof`): `0` Automated and Stored (today), `1`
Stored Only. Probe P1 on the PC (spec §4.3 step 4) keeps the first that adds
no Undo steps and leaves the set unchanged; change it on both numboxes (the
check refuses two different values).

## Building the device

The `.amxd` is never committed (`.gitignore`): a frozen device can embed the
author's file paths, which hold the Windows user name.

1. `python3 live-device/FohMixer-LUFS/build_amxd.py` writes
   `FohMixer LUFS.amxd` beside the patcher: the unfrozen audio effect
   (`ampf`/`aaaa` header, then the patcher JSON byte for byte). It has no
   file dependencies (the gen~ code is inside the patcher).
2. Copy it to the PC and open it in Live's Max editor once: the device
   shows `Short-term`, `Voice` and `v<version>`, and Live lists the two
   parameters.
3. Freeze it in the Max editor if Live asks for a frozen device, save it as
   `FohMixer LUFS.amxd` into the Live user's User Library, and never copy
   the frozen file back into this repository.

The device on the PC is checked against Sonible trueLevel's short-term on the
same signal (spec §9, the owner's ruling of 27.10): within 0.5 LU.
