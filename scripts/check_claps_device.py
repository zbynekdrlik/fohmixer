#!/usr/bin/env python3
"""Keep the FohMixer Claps device's patch in shape (#220).

The device (`live-device/FohMixer-Claps/FohMixer Claps.maxpat`, a Max for
Live MIDI effect) shares the LUFS devices' rules (`check_lufs_device.py`'s
helpers, never a copy): no parameter, every line within its boxes' ports,
the face's version, no absolute path. Its own, box by box (`BOXES`,
`WIRING`), since nothing in CI can run the patch:

- MIDI passes through untouched: `midiin` into `midiout` and nowhere else,
  `midiout` fed by nothing else;
- the hub's words: one `udpreceive <DEVICE_PORT> @defer 1` (#264: it
  outputs in Live's main thread) into one `route` of the five addresses
  (`notes`, `pattern`, `silent`, `on`, `poll`), the pitches into a `zl
  reg`, the pattern into a `zl lookup`, `silent` into the clap gate's third
  inlet, `on` through `change` and `sel 1 0`: 1 arms, 0 stops (the arm and
  the running state cleared);
- Live's beats: a `metro 4n @quantize 4n` on Live's transport (silent while
  it stops), `transport`'s beats and units made the bar's beat 0 to 3 (a
  tick a hair before a beat counts as that beat), beat 0 turns an armed
  device into a running one (the next bar's first beat);
- a clap: the beat's pattern step AND running AND NOT silent bangs the
  pitches through `iter` into `makenote 100 150` (velocity 100, 150 ms: half
  a beat at 200 bpm; a length in ms, so a note-off is never frozen by a stop
  of Live's transport) into `noteout`;
- the watchdog: every `on` word from the hub (sent with its whole state
  every second while it has a +CLAPS track) restarts a `delay
  <FORGET_MS>`; when it runs out (the hub is gone) the device stops and its
  `change` forgets the switch (`set 0`), so the hub's next `on 1` arms it
  again (a poll alone never keeps a device clapping);
- the hello (#264): the hub's `poll` -> `t b b`: the running state kept in
  an `i` (its cold inlet from the arm and the stop) shown on the face, then
  one `pack /fohmixer/claps 1 v<version> 0` (its last inlet the running
  state) straight into one `udpsend 127.0.0.1 <HUB_PORT>`, all in the main
  thread call of the poll;
- the threads (`check_lufs_device.thread_errors`, #240): no timed box
  reaches the `udpsend`; no `deferlow`, `defer`, `qmetro` and no `metro`
  but the beats' (`still_errors`: the hello's `metro 1000` and its
  `deferlow` posted from Live's audio thread); the face fed from no
  scheduler box through hot inlets (`face_errors`: a redraw from the audio
  thread posts); every `udpreceive` with `@defer 1` (`defer_errors`).

The ports and the watchdog's time are read from
`crates/fohmixer-proto/src/claps.rs`. Exit 0 when
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
ADDRESSES = ["notes", "pattern", "silent", "on", "poll"]
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
    "obj-makenote": ["makenote", "100", "150"],
    "obj-heard": ["t", "b"],
    "obj-poll": ["t", "b", "b"],
    "obj-running": ["i"],
    "obj-forget": ["set", "0"],
    "obj-noteout": ["noteout"],
}
# The beats' metro, the device's one clock (MIDI to Live posts nothing).
BEATS = "obj-beats"
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
    (("obj-stop", 0), ("obj-running", 1)),
    (("obj-thisdevice", 0), ("obj-start", 0)),
    (("obj-start", 0), ("obj-beats", 0)),
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
    (("obj-run", 0), ("obj-running", 1)),
    (("obj-beat-t", 0), ("obj-lookup", 0)),
    (("obj-lookup", 0), ("obj-hit", 0)),
    (("obj-hit", 0), ("obj-hit-sel", 0)),
    (("obj-hit-sel", 0), ("obj-notes", 0)),
    (("obj-notes", 0), ("obj-iter", 0)),
    (("obj-iter", 0), ("obj-makenote", 0)),
    (("obj-makenote", 0), ("obj-noteout", 0)),
    (("obj-makenote", 1), ("obj-noteout", 1)),
    (("obj-route", 4), ("obj-poll", 0)),
    (("obj-poll", 1), ("obj-running", 0)),
    (("obj-running", 0), ("obj-clapping", 0)),
    (("obj-poll", 0), ("obj-hello", 0)),
    (("obj-hello", 0), ("obj-udpsend", 0)),
    (("obj-route", 3), ("obj-heard", 0)),
    (("obj-heard", 0), ("obj-watchdog", 0)),
    (("obj-watchdog", 0), ("obj-stop", 0)),
    (("obj-watchdog", 0), ("obj-forget", 0)),
    (("obj-forget", 0), ("obj-on-change", 0)),
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
    ("obj-hello", 0): [["obj-poll", 0]],
    ("obj-poll", 0): [["obj-route", 4]],
    ("obj-running", 0): [["obj-poll", 1]],
    ("obj-running", 1): [["obj-run", 0], ["obj-stop", 0]],
    ("obj-clapping", 0): [["obj-running", 0]],
    ("obj-heard", 0): [["obj-route", 3]],
}
# What the device never holds (#264): a hand-over to Live's main thread.
STILL = ("deferlow", "defer", "qmetro")
# Kinds the patch holds exactly once.
SINGLE = ("midiin", "midiout", "udpreceive", "udpsend", "transport", "noteout", "makenote")


def constant(text: str, name: str) -> int | None:
    """A `pub const <name>: <type> = <n>;` line's number (`_` allowed)."""
    match = re.search(rf"^pub const {name}: \w+ = ([\d_]+);$", text, re.MULTILINE)
    return int(match[1].replace("_", "")) if match else None


def numbers_of(root: Path) -> dict[str, int] | str:
    """The ports from the proto crate, or the problem."""
    text = (root / CLAPS_RS).read_text(encoding="utf-8")
    found = {
        "hub": constant(text, "HUB_PORT"),
        "device": constant(text, "DEVICE_PORT"),
        "forget": constant(text, "FORGET_MS"),
    }
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
        "obj-udpreceive": ["udpreceive", str(numbers["device"]), "@defer", "1"],
        "obj-udpsend": ["udpsend", "127.0.0.1", str(numbers["hub"])],
        "obj-hello": ["pack", "/fohmixer/claps", "1", f"v{version}", "0"],
        "obj-watchdog": ["delay", str(numbers["forget"])],
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
    errors += lufs.thread_errors(where, patcher, every)
    errors += still_errors(where, every)
    errors += face_errors(where, patcher, every)
    errors += defer_errors(where, every)
    shown = [b.get("text") for b in every if b.get("varname") == "version"]
    if shown != [f"v{version}"]:
        errors.append(f"{where}: its face shows {shown}, expected ['v{version}'] (version.txt)")
    return errors


def still_errors(where: str, every: list[dict]) -> list[str]:
    """No hand-over and no clock but the beats' (#264)."""
    return [
        f"{where}: {b.get('id')} ('{b.get('text')}') has no place in the Claps device (its "
        "hello answers the hub's poll in Live's main thread; its one clock is the beats' metro)"
        for b in every
        if lufs.kind(b) in STILL or (lufs.kind(b) == "metro" and b.get("id") != BEATS)
    ]


def face_errors(where: str, patcher: dict, every: list[dict]) -> list[str]:
    """No number box fed from a scheduler box through hot inlets (#264: its
    redraw would post from Live's audio thread)."""
    ids = {b.get("id"): b for b in every}
    return [
        f"{where}: the face's {face.get('id')} is fed from {src} ('{ids[src].get('text')}'), "
        "a scheduler box (a redraw from Live's audio thread posts)"
        for face in every
        if face.get("maxclass") in ("number", "flonum")
        for src in lufs.upstream(
            patcher, ids, [(face.get("id"), None)], lufs.hot_inlets, lufs.TIMED, ends=()
        )
    ]


def defer_errors(where: str, every: list[dict]) -> list[str]:
    """Every `udpreceive` outputs in Live's main thread (#264)."""
    return [
        f"{where}: {b.get('id')} ('{b.get('text')}') outputs on the network thread "
        "(the device takes the hub's words in Live's main thread: @defer 1)"
        for b in every
        if lufs.kind(b) == "udpreceive" and lufs.words(b)[-2:] != ["@defer", "1"]
    ]


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
