#!/usr/bin/env python3
"""Keep the FohMixer Listen device's patch in shape (#210).

The device (`live-device/FohMixer-Listen/FohMixer Listen.maxpat`) shares the
LUFS devices' rules (`check_lufs_device.py`'s helpers, never a copy): no
parameter, silence into `plugout~`, every line within its boxes' ports, the
identity's `live.path` messages, the face's version, no absolute path. Its
own:

- the audio: one `jit.catch~ 2 @mode 1 @framesize <FRAME>` fed by
  `plugin~`'s two outlets and by the `qmetro` alone, its matrices into one
  `jit.net.send @ip 127.0.0.1 @port <DEFAULT_PORT>` and nowhere else;
- the stream runs only while selected: one `qmetro 10`, switched by nothing
  but a `1` or `0` message box (no `loadbang`, no `live.thisdevice`);
- the selects: one `udpreceive <SELECT_PORT>`, one
  `route /fohmixer/listen/select`, one `delay <SELECT_HOLD_MS>`;
- the announce: one `udpsend 127.0.0.1 <DEFAULT_PORT>` fed by one
  `pack /fohmixer/listen 1 0 v<version> - - - - 0 0`;
- the hello: one `pack fohmixer_listen 1 0 v<version> 0` into the
  `jit.net.send`'s right inlet;
- the select logic, box by box (`BOXES`, `WIRING`): a select's id compared
  (`== -1` until the device has its own, so a stop select of 0 never turns
  it on) with the device's listen id (`expr`: its Max id mixed with a
  `random` drawn at load, unique across the PC's two Lives), a match
  restarting the hold before it turns the device on, another id or the
  hold's end turning it off, a change to on sending the hello before the
  `qmetro` starts, and the listen id alone into the announce, the hello and
  the comparison.

The numbers are read from `crates/fohmixer-proto/src/listen.rs` and
`listen/wire.rs`. Exit 0 when all hold, else 1 with one `::error::` line per
problem.
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
FOLDER = Path("live-device") / "FohMixer-Listen"
MAXPAT = FOLDER / "FohMixer Listen.maxpat"
VERSION_TXT = FOLDER / "version.txt"
LISTEN_RS = Path("crates") / "fohmixer-proto" / "src" / "listen.rs"
WIRE_RS = Path("crates") / "fohmixer-proto" / "src" / "listen" / "wire.rs"
QMETRO = ["qmetro", "10"]
SWITCHES = ("0", "1")
# The select logic's boxes (id: words) and lines ((box, outlet), (box,
# inlet)); the patch's ids are its own, fixed like the LUFS check's
# `obj-send-trigger`.
BOXES = {
    "obj-select-route": ["route", "/fohmixer/listen/select"],
    "obj-select-unpack": ["unpack", "0", "0"],
    "obj-select-mine": ["==", "-1"],
    "obj-select-sel": ["sel", "1"],
    "obj-select-t": ["t", "b", "b"],
    "obj-select-on": ["1"],
    "obj-select-off": ["0"],
    "obj-state": ["change"],
    "obj-state-sel": ["sel", "1", "0"],
    "obj-start": ["t", "b", "b"],
    "obj-run": ["1"],
    "obj-halt": ["0"],
    "obj-session-random": ["random", "21000"],
    "obj-session-id": ["expr", "$i2", "*", "100000", "+", "$i1", "%", "100000", "+", "1"],
}
WIRING = (
    (("obj-udpreceive", 0), ("obj-select-route", 0)),
    (("obj-select-route", 0), ("obj-select-unpack", 0)),
    (("obj-select-unpack", 0), ("obj-select-mine", 0)),
    (("obj-select-mine", 0), ("obj-select-sel", 0)),
    (("obj-select-sel", 0), ("obj-select-t", 0)),
    (("obj-select-sel", 1), ("obj-select-off", 0)),
    (("obj-select-t", 1), ("obj-select-hold", 0)),
    (("obj-select-t", 0), ("obj-select-on", 0)),
    (("obj-select-hold", 0), ("obj-select-off", 0)),
    (("obj-select-on", 0), ("obj-state", 0)),
    (("obj-select-off", 0), ("obj-state", 0)),
    (("obj-state", 0), ("obj-state-sel", 0)),
    (("obj-state-sel", 0), ("obj-start", 0)),
    (("obj-state-sel", 1), ("obj-halt", 0)),
    (("obj-start", 1), ("obj-hello", 0)),
    (("obj-start", 0), ("obj-run", 0)),
    (("obj-run", 0), ("obj-qmetro", 0)),
    (("obj-halt", 0), ("obj-qmetro", 0)),
    (("obj-thisdevice", 0), ("obj-session-random", 0)),
    (("obj-session-random", 0), ("obj-session-id", 1)),
    (("obj-id-route", 0), ("obj-session-id", 0)),
    (("obj-session-id", 0), ("obj-announce", 2)),
    (("obj-session-id", 0), ("obj-hello", 2)),
    (("obj-session-id", 0), ("obj-select-mine", 1)),
)
# Inlets only the listen id may feed.
SESSION_ONLY = (("obj-announce", 2), ("obj-hello", 2), ("obj-select-mine", 1))


def wiring_errors(where: str, patcher: dict, every: list[dict]) -> list[str]:
    """The select logic's boxes and lines, as `BOXES` and `WIRING` say."""
    ids = {b.get("id"): b for b in every}
    errors = [
        f"{where}: {box} must be '{' '.join(words)}', has {lufs.words(ids.get(box, {}))}"
        for box, words in BOXES.items()
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
    for box, inlet in SESSION_ONLY:
        fed = lufs.feeds(patcher, box, inlet)
        if fed != [["obj-session-id", 0]]:
            errors.append(f"{where}: {box}:{inlet} takes the listen id alone, has {fed}")
    return errors


def constant(text: str, name: str) -> int | None:
    """A `pub const <name>: <type> = <n>;` line's number (`_` allowed)."""
    match = re.search(rf"^pub const {name}: \w+ = ([\d_]+);$", text, re.MULTILINE)
    return int(match[1].replace("_", "")) if match else None


def one(every: list[dict], wanted: list[str], where: str) -> tuple[str | None, list[str]]:
    """The id of the one box whose words are `wanted`, or the problem."""
    found = [b.get("id") for b in every if lufs.words(b) == wanted]
    if len(found) != 1:
        return None, [f"{where}: needs exactly one '{' '.join(wanted)}', has {len(found)}"]
    return found[0], []


def by_kind(every: list[dict], kind: str) -> list[str]:
    return [b.get("id") for b in every if lufs.words(b)[:1] == [kind]]


def outgoing(patcher: dict, box: str) -> list[list]:
    """The destinations of the lines out of `box`."""
    return sorted(
        list(item.get("patchline", {}).get("destination", []))
        for item in patcher.get("lines", [])
        if item.get("patchline", {}).get("source", [None])[0] == box
    )


def patch_errors(where: str, text: str, version: str, numbers: dict[str, int]) -> list[str]:
    """The problems of the Listen patch."""
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
    errors += lufs.silence_errors(where, patcher, every)
    port, select, frame = numbers["port"], numbers["select"], numbers["frame"]
    hold = numbers["hold"]
    catch, e = one(every, ["jit.catch~", "2", "@mode", "1", "@framesize", str(frame)], where)
    errors += e
    net, e = one(every, ["jit.net.send", "@ip", "127.0.0.1", "@port", str(port)], where)
    errors += e
    qmetro, e = one(every, QMETRO, where)
    errors += e
    for wanted in (
        ["udpreceive", str(select)],
        ["route", "/fohmixer/listen/select"],
        ["delay", str(hold)],
    ):
        errors += one(every, wanted, where)[1]
    udpsend, e = one(every, ["udpsend", "127.0.0.1", str(port)], where)
    errors += e
    announce, e = one(
        every,
        ["pack", "/fohmixer/listen", "1", "0", f"v{version}", "-", "-", "-", "-", "0", "0"],
        where,
    )
    errors += e
    hello, e = one(every, ["pack", "fohmixer_listen", "1", "0", f"v{version}", "0"], where)
    errors += e
    for kind in ("jit.catch~", "jit.net.send", "udpsend", "udpreceive", "qmetro"):
        count = len(by_kind(every, kind))
        if count > 1:
            errors.append(f"{where}: {count} {kind} boxes")
    plugin = by_kind(every, "plugin~")
    if catch and plugin:
        fed = lufs.feeds(patcher, catch, 0) + lufs.feeds(patcher, catch, 1)
        want = sorted([[plugin[0], 0], [plugin[0], 1]] + ([[qmetro, 0]] if qmetro else []))
        if sorted(fed) != want:
            errors.append(f"{where}: the jit.catch~ must take plugin~ and the qmetro, has {fed}")
        if net and outgoing(patcher, catch) != [[net, 0]]:
            errors.append(f"{where}: the jit.catch~'s matrices go to the jit.net.send alone")
    if qmetro:
        ids = {b.get("id"): b for b in every}
        for src, _ in lufs.feeds(patcher, qmetro, 0) + lufs.feeds(patcher, qmetro, 1):
            box = ids.get(src, {})
            if box.get("maxclass") != "message" or str(box.get("text")) not in SWITCHES:
                errors.append(
                    f"{where}: the qmetro is switched only by a '1' or '0' message, not {src}"
                )
    if net and hello and lufs.feeds(patcher, net, 1) != [[hello, 0]]:
        errors.append(f"{where}: the jit.net.send's right inlet takes the hello alone")
    if udpsend and announce and lufs.feeds(patcher, udpsend, 0) != [[announce, 0]]:
        errors.append(f"{where}: the udpsend takes the announce alone")
    errors += wiring_errors(where, patcher, every)
    messages = {str(b.get("text", "")) for b in every if b.get("maxclass") == "message"}
    errors += [f"{where}: no '{path}' message" for path in lufs.IDENTITY if path not in messages]
    shown = [b.get("text") for b in every if b.get("varname") == "version"]
    if shown != [f"v{version}"]:
        errors.append(f"{where}: its face shows {shown}, expected ['v{version}'] (version.txt)")
    return errors


def numbers_of(root: Path) -> dict[str, int] | str:
    """The ports, frame and hold from the proto crate, or the problem."""
    listen = (root / LISTEN_RS).read_text(encoding="utf-8")
    wire = (root / WIRE_RS).read_text(encoding="utf-8")
    found = {
        "port": constant(listen, "DEFAULT_PORT"),
        "select": constant(listen, "SELECT_PORT"),
        "frame": constant(listen, "FRAME"),
        "hold": constant(wire, "SELECT_HOLD_MS"),
    }
    missing = [name for name, value in found.items() if value is None]
    if missing:
        return f"{LISTEN_RS.as_posix()} / {WIRE_RS.as_posix()}: no constant for {missing}"
    return {name: int(value) for name, value in found.items() if value is not None}


def check(root: Path) -> list[str]:
    numbers = numbers_of(root)
    if isinstance(numbers, str):
        return [numbers]
    version = (root / VERSION_TXT).read_text(encoding="utf-8").strip()
    text = (root / MAXPAT).read_text(encoding="utf-8")
    return patch_errors(MAXPAT.as_posix(), text, version, numbers)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="The Listen device's patch.")
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args(argv)
    errors = check(args.root)
    for error in errors:
        print(f"::error::{error}")
    if errors:
        return 1
    print("FohMixer Listen: the patch, its ports, frame, hold and version agree")
    return 0


if __name__ == "__main__":
    sys.exit(main())
