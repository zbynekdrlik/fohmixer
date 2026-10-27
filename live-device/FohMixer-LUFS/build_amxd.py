#!/usr/bin/env python3
"""Write `FohMixer LUFS.amxd` from `FohMixer LUFS.maxpat` (#151, spec §4.2).

An unfrozen Max audio effect device is the patcher JSON behind a small
header (read from a real Live 12 .amxd):

    b"ampf" u32le(4) b"aaaa"      the device kind: an audio effect
    b"meta" u32le(4) 4 zero bytes
    b"ptch" u32le(len(json))      then the patcher JSON, UTF-8

The JSON is copied byte for byte. The .amxd is never committed (.gitignore):
the main session builds it here and installs it on the PC, where Max can
freeze it (README.md).

    python3 build_amxd.py [--maxpat <file>] [--out <file>]
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
MAXPAT = HERE / "FohMixer LUFS.maxpat"
AMXD = HERE / "FohMixer LUFS.amxd"
KIND_AUDIO_EFFECT = b"aaaa"


def header(patch_len: int) -> bytes:
    return (
        b"ampf"
        + struct.pack("<I", 4)
        + KIND_AUDIO_EFFECT
        + b"meta"
        + struct.pack("<I", 4)
        + bytes(4)
        + b"ptch"
        + struct.pack("<I", patch_len)
    )


def amxd(patch: bytes) -> bytes:
    """The device's bytes for a patcher's JSON (refused unless it is one)."""
    if "patcher" not in json.loads(patch.decode("utf-8")):
        raise ValueError("not a Max patcher: no top-level 'patcher'")
    return header(len(patch)) + patch


def patch_of(device: bytes) -> bytes:
    """The patcher JSON inside a device written by `amxd`."""
    head = header(0)
    if device[: len(head) - 4] != head[:-4]:
        raise ValueError("not an unfrozen audio effect .amxd")
    (length,) = struct.unpack("<I", device[len(head) - 4 : len(head)])
    patch = device[len(head) :]
    if len(patch) != length:
        raise ValueError(f"the length field says {length} bytes, {len(patch)} follow")
    return patch


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--maxpat", type=Path, default=MAXPAT)
    parser.add_argument("--out", type=Path, default=AMXD)
    args = parser.parse_args(argv)
    device = amxd(args.maxpat.read_bytes())
    args.out.write_bytes(device)
    print(f"wrote {args.out.name}: {len(device)} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main())
