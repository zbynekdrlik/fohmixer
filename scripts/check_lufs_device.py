#!/usr/bin/env python3
"""Keep the FohMixer LUFS and Meter devices' patches in shape (#151).

Spec docs/superpowers/specs/2026-10-26-lufs-design.md §3.2, §4.2, §4.4 and §9,
for each patch:

- no parameter: no `live.*` parameter object, no box with `parameter_enable`
  on, no `parameters` table (probe P1: a parameter the device writes is an
  Undo step);
- the output is silence: `plugout~` takes nothing but a `sig~ 0` (or
  `sig~ 0.`) that nothing feeds (the owner's ruling of 27.10: the devices sit
  in a rack's parallel chain, never in series, so a failing device can never
  cut the sound and the signal is never doubled);
- one `udpsend 127.0.0.1 <port>`, the port equal to the hub's default
  (`DEFAULT_PORT` in `crates/fohmixer-proto/src/lufs/wire.rs`);
- the send rate: the box `obj-send-metro` is `metro <ms>`, 200 for the LUFS
  device (5 datagrams a second: the owner's ruling of 27.10, the readings
  are a 3 s window shown as integers) and 33 for the meter (~30 a second);
- one `pack` whose first argument is the device's OSC address and whose
  version argument is `v<version.txt>`, which the face shows too;
- the identity: the `live.path` messages for the device and its first four
  canonical parents;
- every patch line joins two boxes of the patch within their outlets and
  inlets;
- the LUFS device only: one `gen~` with one codebox whose code equals
  `fohmixer_lufs.genexpr` (no file dependencies);
- no absolute file path.

Exit 0 when all hold, else 1 with one `::error::` line per problem.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WIRE = Path("crates") / "fohmixer-proto" / "src" / "lufs" / "wire.rs"
PORT_LINE = re.compile(r"^pub const DEFAULT_PORT: u16 = (\d+);$", re.MULTILINE)
# A drive letter path (raw or JSON-escaped), a UNC path, a Unix home.
ABSOLUTE_PATH = re.compile(r"\b[A-Za-z]:(?:\\\\|\\|/)|\\\\\\\\|/(?:Users|home)/")
PARENTS = 4
IDENTITY = tuple("path this_device" + " canonical_parent" * level for level in range(PARENTS + 1))
# The pack's arguments: address, wire version, id, version, 4 parents, 2 values.
PACK_ARGS = 10
PACK_VERSION = 3
# The metro that bangs the pack (one datagram per bang).
SEND_METRO = "obj-send-metro"


@dataclass(frozen=True)
class Device:
    """One device's source files and what its patch must say."""

    name: str
    folder: Path
    maxpat: Path
    address: str
    genexpr: Path | None
    send_ms: int

    @property
    def version_txt(self) -> Path:
        return self.folder / "version.txt"


LUFS_FOLDER = Path("live-device") / "FohMixer-LUFS"
METER_FOLDER = Path("live-device") / "FohMixer-Meter"
DEVICES = (
    Device(
        "FohMixer LUFS",
        LUFS_FOLDER,
        LUFS_FOLDER / "FohMixer LUFS.maxpat",
        "/fohmixer/lufs",
        LUFS_FOLDER / "fohmixer_lufs.genexpr",
        200,
    ),
    Device(
        "FohMixer Meter",
        METER_FOLDER,
        METER_FOLDER / "FohMixer Meter.maxpat",
        "/fohmixer/meter",
        None,
        33,
    ),
)


def boxes(patcher: dict) -> list[dict]:
    """Every box of a patcher and of the patchers inside it."""
    found = []
    for item in patcher.get("boxes", []):
        box = item.get("box", {})
        found.append(box)
        if isinstance(box.get("patcher"), dict):
            found += boxes(box["patcher"])
    return found


def words(box: dict) -> list[str]:
    return str(box.get("text", "")).split()


def default_port(wire_text: str) -> int | None:
    match = PORT_LINE.search(wire_text)
    return int(match[1]) if match else None


def is_parameter(box: dict) -> bool:
    """A box with `parameter_enable` on, or a `live.*` UI object (a parameter by kind)."""
    return box.get("parameter_enable") == 1 or str(box.get("maxclass", "")).startswith("live.")


def line_errors(where: str, patcher: dict) -> list[str]:
    """Lines of this patcher (not its subpatchers) within its boxes' ports."""
    own = {item.get("box", {}).get("id"): item.get("box", {}) for item in patcher["boxes"]}
    errors = []
    for item in patcher.get("lines", []):
        line = item.get("patchline", {})
        (src, out), (dst, inlet) = line.get("source"), line.get("destination")
        if src not in own or dst not in own:
            errors.append(f"{where}: a line joins a box that is not there: {src} -> {dst}")
        elif out >= own[src].get("numoutlets", 0) or inlet >= own[dst].get("numinlets", 0):
            errors.append(f"{where}: the line {src}:{out} -> {dst}:{inlet} is past their ports")
    return errors


SILENCE = (["sig~", "0"], ["sig~", "0."])


def silence_errors(where: str, patcher: dict, every: list[dict]) -> list[str]:
    """`plugout~` fed by nothing but a `sig~ 0` that nothing feeds."""
    ids = {b.get("id"): words(b) for b in every}
    plugin = [i for i, w in ids.items() if w[:1] == ["plugin~"]]
    plugout = [i for i, w in ids.items() if w[:1] == ["plugout~"]]
    if len(plugin) != 1 or len(plugout) != 1:
        return [f"{where}: needs exactly one plugin~ and one plugout~"]
    lines = [line.get("patchline", {}) for line in patcher.get("lines", [])]
    sources = {line["source"][0] for line in lines if line["destination"][0] == plugout[0]}
    loud = sorted(str(s) for s in sources if ids.get(s) not in SILENCE)
    fed = sorted(
        str(s)
        for s in sources
        if ids.get(s) in SILENCE and any(ln["destination"][0] == s for ln in lines)
    )
    if loud or fed:
        return [
            f"{where}: plugout~ must take silence only (a sig~ 0 nothing feeds; the device "
            f"sits in a parallel chain), it takes {loud + fed}"
        ]
    return []


def patch_errors(
    where: str,
    maxpat_text: str,
    code: str | None,
    version: str,
    address: str,
    port: int,
    send_ms: int,
) -> list[str]:
    """The problems of one device's patch (`code`: its gen~ code, none for no gen~)."""
    errors: list[str] = []
    if ABSOLUTE_PATH.search(maxpat_text):
        errors.append(f"{where} holds an absolute file path")
    try:
        patcher = json.loads(maxpat_text)["patcher"]
        every = boxes(patcher)
    except (ValueError, KeyError, TypeError, AttributeError) as error:
        return errors + [f"{where} is not a Max patcher: {error}"]
    if "parameters" in patcher:
        errors.append(f"{where}: a parameters table (the device has no parameters)")
    errors += [
        f"{where}: {b.get('id')} is a parameter (the device has none)"
        for b in every
        if is_parameter(b)
    ]
    errors += line_errors(where, patcher)
    errors += silence_errors(where, patcher, every)
    if code is not None:
        gens = [b for b in every if words(b)[:1] == ["gen~"]]
        if len(gens) != 1 or not isinstance(gens[0].get("patcher"), dict):
            errors.append(f"{where}: needs exactly one gen~ with its patcher embedded")
        else:
            codeboxes = [b for b in boxes(gens[0]["patcher"]) if b.get("maxclass") == "codebox"]
            if len(codeboxes) != 1:
                errors.append(f"{where}: the gen~ needs exactly one codebox")
            elif codeboxes[0].get("code") != code:
                errors.append(f"{where}: the gen~ code differs from its .genexpr")
    sends = [words(b) for b in every if words(b)[:1] == ["udpsend"]]
    if sends != [["udpsend", "127.0.0.1", str(port)]]:
        errors.append(f"{where}: needs exactly one 'udpsend 127.0.0.1 {port}', has {sends}")
    metros = [words(b) for b in every if b.get("id") == SEND_METRO]
    if metros != [["metro", str(send_ms)]]:
        errors.append(f"{where}: {SEND_METRO} must be 'metro {send_ms}', has {metros}")
    packs = [words(b) for b in every if words(b)[:1] == ["pack"]]
    if len(packs) != 1 or len(packs[0]) != PACK_ARGS + 1:
        errors.append(f"{where}: needs exactly one pack of {PACK_ARGS} arguments, has {packs}")
    else:
        if packs[0][1] != address:
            errors.append(f"{where}: the pack sends to {packs[0][1]}, expected {address}")
        if packs[0][1 + PACK_VERSION] != f"v{version}":
            errors.append(
                f"{where}: the pack sends {packs[0][1 + PACK_VERSION]}, expected v{version}"
            )
    messages = {str(b.get("text", "")) for b in every if b.get("maxclass") == "message"}
    errors += [f"{where}: no '{path}' message" for path in IDENTITY if path not in messages]
    shown = [b.get("text") for b in every if b.get("varname") == "version"]
    if shown != [f"v{version}"]:
        errors.append(f"{where}: its face shows {shown}, expected ['v{version}'] (version.txt)")
    return errors


def device_errors(root: Path, device: Device, port: int) -> list[str]:
    code = None
    if device.genexpr is not None:
        code = (root / device.genexpr).read_text(encoding="utf-8")
    version = (root / device.version_txt).read_text(encoding="utf-8").strip()
    text = (root / device.maxpat).read_text(encoding="utf-8")
    return patch_errors(
        device.maxpat.as_posix(), text, code, version, device.address, port, device.send_ms
    )


def check(root: Path) -> list[str]:
    port = default_port((root / WIRE).read_text(encoding="utf-8"))
    if port is None:
        return [f"{WIRE.as_posix()}: no 'pub const DEFAULT_PORT: u16 = <port>;' line"]
    return [error for device in DEVICES for error in device_errors(root, device, port)]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="The LUFS devices' patches.")
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args(argv)
    errors = check(args.root)
    for error in errors:
        print(f"::error::{error}")
    if errors:
        return 1
    print("FohMixer LUFS and Meter: the patches, their gen~ code, port and version agree")
    return 0


if __name__ == "__main__":
    sys.exit(main())
