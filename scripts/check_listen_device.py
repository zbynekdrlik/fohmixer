#!/usr/bin/env python3
"""Keep the FohMixer Listen device's patch in shape (#210; #264: polled by
the hub, no clock).

The device (`live-device/FohMixer-Listen/FohMixer Listen.maxpat`) shares the
LUFS devices' rules (`check_lufs_device.py`'s helpers, never a copy): no
parameter, silence into `plugout~`, every line within its boxes' ports, the
identity's `live.path` messages, the face's version, no absolute path. Since
#264 (the lane's design in comment 7184226996 on #264) it holds no clock
at all: a Max for Live clock runs in Live's audio thread and its hand-over
posts a window message from there. The hub drives it:

- the pull (`listen_pull_check`, the meter's and LUFS device's boxes and
  lines through `lufs_pull_check.pull_errors`): the poll port drawn at
  load into `udpreceive @defer 1` and the announce's last slot, the gate on
  Device On, one `route` of the hub's poll (its `on`) and frame poll;
- no `deferlow`, `defer`, `metro`, `qmetro`, `delay`, `del` or `pipe`
  anywhere (`poll_reread_check.still_errors`), and no `maxurl`: the select
  comes in the poll;
- the audio: one `jit.catch~ 2 @mode 1 @framesize <FRAME>` fed by
  `plugin~`'s two outlets and by the frame gate alone, its matrices into
  one `jit.net.send @ip 127.0.0.1 @port <DEFAULT_PORT>` and nowhere else;
  the frame gate switched by nothing but a `1` or `0` message box, fed by
  the route's frame outlet alone;
- the select logic, box by box (`BOXES`, `WIRING`): a poll's 1 turns the
  device on (the hello before the frame gate opens), anything else off at
  once; the listen id (`expr`: its Max id mixed with a `random` drawn at
  load, unique across the PC's two Lives) alone into the announce and the
  hello;
- the identity is the other pulled devices' (`check_lufs_device.
  identity_errors` with the re-read on every `REREAD_POLLS`th poll), each
  name from `live.object`;
- the announce: one `udpsend 127.0.0.1 <DEFAULT_PORT>` fed by one
  `pack /fohmixer/listen 2 0 v<version> - - - - 0 0 0`; the hello: one
  `pack fohmixer_listen 2 0 v<version> 0` into the `jit.net.send`'s right
  inlet; the threads (`check_lufs_device.thread_errors`).

The numbers are read from `crates/fohmixer-proto/src/listen.rs`,
`listen/wire.rs` and `lufs/wire.rs` (the poll ports' range). Exit 0 when all
hold, else 1 with one `::error::` line per problem.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_lufs_device as lufs  # noqa: E402
import listen_pull_check  # noqa: E402
import lufs_pull_check  # noqa: E402
import poll_reread_check  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
FOLDER = Path("live-device") / "FohMixer-Listen"
MAXPAT = FOLDER / "FohMixer Listen.maxpat"
VERSION_TXT = FOLDER / "version.txt"
LISTEN_RS = Path("crates") / "fohmixer-proto" / "src" / "listen.rs"
WIRE_RS = Path("crates") / "fohmixer-proto" / "src" / "listen" / "wire.rs"
LUFS_WIRE_RS = Path("crates") / "fohmixer-proto" / "src" / "lufs" / "wire.rs"
SWITCHES = ("0", "1")
FRAME_GATE = listen_pull_check.FRAME_GATE
# The pull's outlet that counts the re-read (`t b b b i`'s third).
REREAD_OUTLET = 2
# The select logic's boxes (id: words) and lines ((box, outlet), (box,
# inlet)); the patch's ids are its own, fixed like the LUFS check's.
BOXES = {
    "obj-poll-sel": ["sel", "1"],
    "obj-select-on": ["1"],
    "obj-select-off": ["0"],
    "obj-state": ["change"],
    "obj-state-sel": ["sel", "1", "0"],
    "obj-start": ["t", "b", "b"],
    "obj-run": ["1"],
    "obj-halt": ["0"],
    "obj-hello-gate": ["gate"],
    FRAME_GATE: ["gate"],
    "obj-session-random": ["random", "21000"],
    "obj-session-id": ["expr", "$i2", "*", "100000", "+", "$i1", "%", "100000", "+", "1"],
}
WIRING = (
    (("obj-poll-sel", 0), ("obj-select-on", 0)),
    (("obj-poll-sel", 1), ("obj-select-off", 0)),
    (("obj-select-on", 0), ("obj-state", 0)),
    (("obj-select-off", 0), ("obj-state", 0)),
    (("obj-state", 0), ("obj-state-sel", 0)),
    (("obj-state", 0), ("obj-announce", 9)),
    (("obj-state", 0), ("obj-hello-gate", 0)),
    (("obj-state-sel", 0), ("obj-start", 0)),
    (("obj-state-sel", 1), ("obj-halt", 0)),
    (("obj-start", 1), ("obj-hello", 0)),
    (("obj-start", 0), ("obj-run", 0)),
    (("obj-run", 0), (FRAME_GATE, 0)),
    (("obj-halt", 0), (FRAME_GATE, 0)),
    ((FRAME_GATE, 0), ("obj-catch", 0)),
    (("obj-hello-gate", 0), ("obj-hello", 0)),
    (("obj-id-load", 9), ("obj-session-random", 0)),
    (("obj-session-random", 0), ("obj-session-id", 1)),
    (("obj-id-route", 0), ("obj-session-id", 0)),
    (("obj-session-id", 0), ("obj-announce", 2)),
    (("obj-session-id", 0), ("obj-hello", 2)),
)
# Inlets only the listen id may feed.
SESSION_ONLY = (("obj-announce", 2), ("obj-hello", 2))
# The listen id's random, drawn from the load's rightmost outlet: before the
# load's `path this_device` reads the Max id into the `expr`'s hot inlet
# (the review of #264's step: a fan out of `live.thisdevice` would leave
# that order to the boxes' places, and two devices could share an id).
RANDOM_FIRST = (("obj-session-random", 0), [["obj-id-load", 9]])


def wiring_errors(where: str, patcher: dict, every: list[dict], reread: int) -> list[str]:
    """The select logic's boxes and lines, as `BOXES` and `WIRING` say,
    and the identity re-read on the poll."""
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
    (box, inlet), want = RANDOM_FIRST
    fed = lufs.feeds(patcher, box, inlet)
    if fed != want:
        errors.append(f"{where}: {box}:{inlet} takes {want} alone, has {fed}")
    errors += lufs.identity_errors(
        where, patcher, every, "obj-announce", True, lufs.Reread(reread, REREAD_OUTLET)
    )
    return errors


def constant(text: str, name: str) -> int | None:
    """A `pub const <name>: <type> = <n>;` line's number (`_` allowed)."""
    match = re.search(rf"^pub const {name}: \w+ = ([\d_]+);$", text, re.MULTILINE)
    return int(match[1].replace("_", "")) if match else None


def text_constant(text: str, name: str) -> str | None:
    """A `pub const <name>: &str = "<text>";` line's text."""
    match = re.search(rf'^pub const {name}: &str = "([^"\\]*)";$', text, re.MULTILINE)
    return match[1] if match else None


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


def audio_errors(where: str, patcher: dict, every: list[dict], numbers: dict) -> list[str]:
    """The catch, the frame gate and the audio connection."""
    catch, errors = one(
        every, ["jit.catch~", "2", "@mode", "1", "@framesize", str(numbers["frame"])], where
    )
    net, e = one(every, ["jit.net.send", "@ip", "127.0.0.1", "@port", str(numbers["port"])], where)
    errors += e
    plugin = by_kind(every, "plugin~")
    if catch and plugin:
        fed = lufs.feeds(patcher, catch, 0) + lufs.feeds(patcher, catch, 1)
        want = sorted([[plugin[0], 0], [plugin[0], 1], [FRAME_GATE, 0]])
        if sorted(fed) != want:
            errors.append(
                f"{where}: the jit.catch~ must take plugin~ and the frame gate, has {fed}"
            )
        if net and outgoing(patcher, catch) != [[net, 0]]:
            errors.append(f"{where}: the jit.catch~'s matrices go to the jit.net.send alone")
    ids = {b.get("id"): b for b in every}
    for src, _ in lufs.feeds(patcher, FRAME_GATE, 0):
        box = ids.get(src, {})
        if box.get("maxclass") != "message" or str(box.get("text")) not in SWITCHES:
            errors.append(
                f"{where}: the frame gate is switched only by a '1' or '0' message, not {src}"
            )
    return errors


def patch_errors(where: str, text: str, version: str, numbers: dict) -> list[str]:
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
    errors += audio_errors(where, patcher, every, numbers)
    if by_kind(every, "maxurl"):
        errors.append(f"{where}: a maxurl (the hub pushes the select in its poll)")
    port = numbers["port"]
    udpsend, e = one(every, ["udpsend", "127.0.0.1", str(port)], where)
    errors += e
    announce, e = one(
        every,
        ["pack", "/fohmixer/listen", "2", "0", f"v{version}", "-", "-", "-", "-", "0", "0", "0"],
        where,
    )
    errors += e
    hello, e = one(every, ["pack", "fohmixer_listen", "2", "0", f"v{version}", "0"], where)
    errors += e
    for kind in ("jit.catch~", "jit.net.send", "udpsend"):
        count = len(by_kind(every, kind))
        if count > 1:
            errors.append(f"{where}: {count} {kind} boxes")
    net = (by_kind(every, "jit.net.send") or [None])[0]
    if net and hello and lufs.feeds(patcher, net, 1) != [[hello, 0]]:
        errors.append(f"{where}: the jit.net.send's right inlet takes the hello alone")
    if udpsend and announce and lufs.feeds(patcher, udpsend, 0) != [[announce, 0]]:
        errors.append(f"{where}: the udpsend takes the announce alone")
    poll = lufs_pull_check.Poll(numbers["min"], numbers["span"], "", "")
    shape = listen_pull_check.listen_shape(poll, numbers["poll"], numbers["frames"])
    errors += lufs_pull_check.pull_errors(where, patcher, every, shape)
    errors += wiring_errors(where, patcher, every, numbers["reread"])
    errors += poll_reread_check.still_errors(where, every)
    errors += lufs.thread_errors(where, patcher, every)
    messages = {str(b.get("text", "")) for b in every if b.get("maxclass") == "message"}
    errors += [f"{where}: no '{path}' message" for path in lufs.IDENTITY if path not in messages]
    shown = [b.get("text") for b in every if b.get("varname") == "version"]
    if shown != [f"v{version}"]:
        errors.append(f"{where}: its face shows {shown}, expected ['v{version}'] (version.txt)")
    return errors


def numbers_of(root: Path) -> dict | str:
    """The ports, frame, polls and re-read from the proto crate, or the
    problem."""
    listen = (root / LISTEN_RS).read_text(encoding="utf-8")
    wire = (root / WIRE_RS).read_text(encoding="utf-8")
    lufs_wire = (root / LUFS_WIRE_RS).read_text(encoding="utf-8")
    found = {
        "port": constant(listen, "DEFAULT_PORT"),
        "frame": constant(listen, "FRAME"),
        "reread": constant(wire, "REREAD_POLLS"),
        "poll": text_constant(wire, "POLL_ADDRESS"),
        "frames": text_constant(wire, "FRAME_ADDRESS"),
        "min": constant(lufs_wire, "POLL_PORT_MIN"),
        "span": constant(lufs_wire, "POLL_PORT_SPAN"),
    }
    missing = [name for name, value in found.items() if value is None]
    if missing:
        where = " / ".join(p.as_posix() for p in (LISTEN_RS, WIRE_RS, LUFS_WIRE_RS))
        return f"{where}: no constant for {missing}"
    return found


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
    print("FohMixer Listen: the patch, its ports, frame, polls and version agree; no clock")
    return 0


if __name__ == "__main__":
    sys.exit(main())
