#!/usr/bin/env python3
"""Keep the FohMixer Claps device's patch in shape (#220).

The device (`live-device/FohMixer-Claps/FohMixer Claps.maxpat`, a Max for
Live MIDI effect) shares the LUFS devices' rules (`check_lufs_device.py`'s
helpers, never a copy): no parameter, every line within its boxes' ports,
the face's version, no absolute path. Its own, box by box (`BOXES`,
`WIRING`), since nothing in CI can run the patch:

- MIDI passes through untouched: `midiin` into `midiout` and nowhere else,
  `midiout` fed by nothing else;
- the hub's words: one `udpreceive <DEVICE_PORT>` into one `route` of the
  four addresses (`notes`, `pattern`, `silent`, `on`), the pitches into a
  `zl reg`, the pattern into a `zl lookup`, `silent` into the clap gate's
  third inlet, `on` through `change` and `sel 1 0`: 1 arms, 0 stops (the
  arm and the running state cleared);
- Live's beats: a `metro 4n @quantize 4n` on Live's transport (silent while
  it stops), `transport`'s beats and units made the bar's beat 0 to 3 (a
  tick a hair before a beat counts as that beat), beat 0 turns an armed
  device into a running one (the next bar's first beat);
- a clap: the beat's pattern step AND running AND NOT silent bangs the
  pitches through `iter` into `makenote 100 8n` (velocity 100, half a beat,
  as the Claps track's clips) into `noteout`;
- the hello: one `udpsend 127.0.0.1 <HUB_PORT>` fed by one
  `pack /fohmixer/claps 1 v<version> 0` each second, its last inlet the
  running state.

The ports are read from `crates/fohmixer-proto/src/claps.rs`. Exit 0 when
all hold, else 1 with one `::error::` line per problem.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_lufs_device as lufs  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
FOLDER = Path("live-device") / "FohMixer-Claps"
MAXPAT = FOLDER / "FohMixer Claps.maxpat"
VERSION_TXT = FOLDER / "version.txt"
CLAPS_RS = Path("crates") / "fohmixer-proto" / "src" / "claps.rs"
ADDRESSES = ["notes", "pattern", "silent", "on"]
BOXES = {
    "obj-midiin": ["midiin"],
    "obj-midiout": ["midiout"],
    "obj-route": ["route", *(f"/fohmixer/claps/{a}" for a in ADDRESSES)],
    "obj-notes": ["zl", "reg"],
    "obj-lookup": ["zl", "lookup"],
    "obj-hit": ["expr", "$i1", "&&", "$i2", "&&", "!$i3"],
    "obj-on-change": ["change"],
    "obj-on-sel": ["sel", "1", "0"],
    "obj-arm": ["1"],
    "obj-stop": ["0"],
    "obj-armed": ["i"],
    "obj-thisdevice": ["live.thisdevice"],
    "obj-start": ["1"],
    "obj-beats": ["metro", "4n", "@quantize", "4n", "@autostart", "1", "@autostarttime", "0"],
    "obj-transport": ["transport"],
    "obj-beat": ["expr", "($i1", "-", "1", "+", "($f2", ">=", "240.))", "%", "4"],
    "obj-beat-t": ["t", "i", "i"],
    "obj-bar": ["sel", "0"],
    "obj-armed-sel": ["sel", "1"],
    "obj-arm-t": ["t", "b", "b"],
    "obj-disarm": ["0"],
    "obj-run": ["1"],
    "obj-hit-sel": ["sel", "1"],
    "obj-iter": ["iter"],
    "obj-makenote": ["makenote", "100", "8n"],
    "obj-noteout": ["noteout"],
    "obj-hello-metro": ["metro", "1000"],
}
WIRING = (
    (("obj-midiin", 0), ("obj-midiout", 0)),
    (("obj-udpreceive", 0), ("obj-route", 0)),
    (("obj-route", 0), ("obj-notes", 1)),
    (("obj-route", 1), ("obj-lookup", 1)),
    (("obj-route", 2), ("obj-hit", 2)),
    (("obj-route", 3), ("obj-on-change", 0)),
    (("obj-on-change", 0), ("obj-on-sel", 0)),
    (("obj-on-sel", 0), ("obj-arm", 0)),
    (("obj-on-sel", 1), ("obj-stop", 0)),
    (("obj-arm", 0), ("obj-armed", 1)),
    (("obj-stop", 0), ("obj-armed", 1)),
    (("obj-stop", 0), ("obj-hit", 1)),
    (("obj-stop", 0), ("obj-hello", 3)),
    (("obj-thisdevice", 0), ("obj-start", 0)),
    (("obj-start", 0), ("obj-beats", 0)),
    (("obj-start", 0), ("obj-hello-metro", 0)),
    (("obj-beats", 0), ("obj-transport", 0)),
    (("obj-transport", 2), ("obj-beat", 1)),
    (("obj-transport", 1), ("obj-beat", 0)),
    (("obj-beat", 0), ("obj-beat-t", 0)),
    (("obj-beat-t", 1), ("obj-bar", 0)),
    (("obj-bar", 0), ("obj-armed", 0)),
    (("obj-armed", 0), ("obj-armed-sel", 0)),
    (("obj-armed-sel", 0), ("obj-arm-t", 0)),
    (("obj-arm-t", 1), ("obj-disarm", 0)),
    (("obj-arm-t", 0), ("obj-run", 0)),
    (("obj-disarm", 0), ("obj-armed", 1)),
    (("obj-run", 0), ("obj-hit", 1)),
    (("obj-run", 0), ("obj-hello", 3)),
    (("obj-beat-t", 0), ("obj-lookup", 0)),
    (("obj-lookup", 0), ("obj-hit", 0)),
    (("obj-hit", 0), ("obj-hit-sel", 0)),
    (("obj-hit-sel", 0), ("obj-notes", 0)),
    (("obj-notes", 0), ("obj-iter", 0)),
    (("obj-iter", 0), ("obj-makenote", 0)),
    (("obj-makenote", 0), ("obj-noteout", 0)),
    (("obj-makenote", 1), ("obj-noteout", 1)),
    (("obj-hello-metro", 0), ("obj-hello", 0)),
    (("obj-hello", 0), ("obj-udpsend", 0)),
)
# Inlets only these sources may feed (a stray line there plays or arms).
ONLY = {
    ("obj-midiout", 0): [["obj-midiin", 0]],
    ("obj-noteout", 0): [["obj-makenote", 0]],
    ("obj-hit", 0): [["obj-lookup", 0]],
    ("obj-hit", 1): [["obj-run", 0], ["obj-stop", 0]],
    ("obj-armed", 0): [["obj-bar", 0]],
    ("obj-beats", 0): [["obj-start", 0]],
    ("obj-udpsend", 0): [["obj-hello", 0]],
}
# Kinds the patch holds exactly once.
SINGLE = ("midiin", "midiout", "udpreceive", "udpsend", "transport", "noteout", "makenote")


def constant(text: str, name: str) -> int | None:
    """A `pub const <name>: <type> = <n>;` line's number (`_` allowed)."""
    match = re.search(rf"^pub const {name}: \w+ = ([\d_]+);$", text, re.MULTILINE)
    return int(match[1].replace("_", "")) if match else None


def numbers_of(root: Path) -> dict[str, int] | str:
    """The ports from the proto crate, or the problem."""
    text = (root / CLAPS_RS).read_text(encoding="utf-8")
    found = {"hub": constant(text, "HUB_PORT"), "device": constant(text, "DEVICE_PORT")}
    missing = [name for name, value in found.items() if value is None]
    if missing:
        return f"{CLAPS_RS.as_posix()}: no constant for {missing}"
    return {name: int(value) for name, value in found.items() if value is not None}


def patch_errors(where: str, text: str, version: str, numbers: dict[str, int]) -> list[str]:
    """The problems of the Claps patch."""
    errors: list[str] = []
    if lufs.ABSOLUTE_PATH.search(text):
        errors.append(f"{where} holds an absolute file path")
    try:
        patcher = json.loads(text)["patcher"]
        every = lufs.boxes(patcher)
    except (ValueError, KeyError, TypeError, AttributeError) as error:
        return errors + [f"{where} is not a Max patcher: {error}"]
    if "parameters" in patcher:
        errors.append(f"{where}: a parameters table (the device has no parameters)")
    errors += [
        f"{where}: {b.get('id')} is a parameter (the device has none)"
        for b in every
        if lufs.is_parameter(b)
    ]
    errors += lufs.line_errors(where, patcher)
    ids = {b.get("id"): b for b in every}
    wanted = {
        **BOXES,
        "obj-udpreceive": ["udpreceive", str(numbers["device"])],
        "obj-udpsend": ["udpsend", "127.0.0.1", str(numbers["hub"])],
        "obj-hello": ["pack", "/fohmixer/claps", "1", f"v{version}", "0"],
    }
    errors += [
        f"{where}: {box} must be '{' '.join(words)}', has {lufs.words(ids.get(box, {}))}"
        for box, words in wanted.items()
        if lufs.words(ids.get(box, {})) != words
    ]
    lines = {
        (tuple(ln["patchline"]["source"]), tuple(ln["patchline"]["destination"]))
        for ln in patcher.get("lines", [])
        if "patchline" in ln
    }
    errors += [
        f"{where}: the line {src[0]}:{src[1]} -> {dst[0]}:{dst[1]} is missing"
        for src, dst in WIRING
        if (src, dst) not in lines
    ]
    for (box, inlet), sources in ONLY.items():
        fed = lufs.feeds(patcher, box, inlet)
        if fed != sorted(sources):
            errors.append(f"{where}: {box}:{inlet} takes {sorted(sources)} alone, has {fed}")
    midi_out = sorted(
        list(ln["patchline"]["destination"])
        for ln in patcher.get("lines", [])
        if ln.get("patchline", {}).get("source", [None])[0] == "obj-midiin"
    )
    if midi_out != [["obj-midiout", 0]]:
        errors.append(f"{where}: midiin goes to midiout alone, has {midi_out}")
    for kind in SINGLE:
        count = sum(1 for b in every if lufs.words(b)[:1] == [kind])
        if count != 1:
            errors.append(f"{where}: needs exactly one {kind}, has {count}")
    shown = [b.get("text") for b in every if b.get("varname") == "version"]
    if shown != [f"v{version}"]:
        errors.append(f"{where}: its face shows {shown}, expected ['v{version}'] (version.txt)")
    return errors


def check(root: Path) -> list[str]:
    numbers = numbers_of(root)
    if isinstance(numbers, str):
        return [numbers]
    version = (root / VERSION_TXT).read_text(encoding="utf-8").strip()
    text = (root / MAXPAT).read_text(encoding="utf-8")
    return patch_errors(MAXPAT.as_posix(), text, version, numbers)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="The Claps device's patch.")
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args(argv)
    errors = check(args.root)
    for error in errors:
        print(f"::error::{error}")
    if errors:
        return 1
    print("FohMixer Claps: the patch, its ports, its wiring and version agree")
    return 0


if __name__ == "__main__":
    sys.exit(main())
