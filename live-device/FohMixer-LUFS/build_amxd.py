#!/usr/bin/env python3
"""Write a device's .amxd from its .maxpat (#151, spec §4.2): `FohMixer LUFS` or `FohMixer Meter`.

An unfrozen Max audio effect device is the patcher JSON behind a small
header (read from a real Live 12 .amxd):

    b"ampf" u32le(4) b"aaaa"      the device kind: an audio effect
    b"meta" u32le(4) 4 zero bytes
    b"ptch" u32le(len(json))      then the patcher JSON, UTF-8

The JSON is copied byte for byte, except the `udpsend` box's port when
`--port` asks for another one than the hub's default (the device cannot read
the hub's config; README.md). The .amxd is never committed (.gitignore): the
main session builds it here and installs it on the PC, where Max can freeze
it (README.md).

    python3 build_amxd.py [--patch <maxpat>] [--port <n>] [--out <amxd>]

`--patch` defaults to the LUFS patch, `--out` to the patch's own name with
`.amxd`, `--port` to 39110.
"""

from __future__ import annotations

import argparse
import json
import struct
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
MAXPAT = HERE / "FohMixer LUFS.maxpat"
METER_MAXPAT = HERE.parent / "FohMixer-Meter" / "FohMixer Meter.maxpat"
KIND_AUDIO_EFFECT = b"aaaa"
# The hub's default port (fohmixer_proto::lufs::wire::DEFAULT_PORT), as the
# patches carry it; scripts/check_lufs_device.py keeps them equal.
DEFAULT_PORT = 39110


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


def with_port(patch: bytes, port: int) -> bytes:
    """The patch with its one `udpsend 127.0.0.1 <default>` box sending to `port`."""
    if not 0 < port < 65536:
        raise ValueError(f"no UDP port: {port}")
    box = f'"udpsend 127.0.0.1 {DEFAULT_PORT}"'.encode()
    if patch.count(box) != 1:
        raise ValueError(f"the patch needs exactly one {box.decode()} box")
    return patch.replace(box, f'"udpsend 127.0.0.1 {port}"'.encode())


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--patch", "--maxpat", dest="patch", type=Path, default=MAXPAT)
    parser.add_argument("--port", type=int, default=DEFAULT_PORT)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args(argv)
    out = args.out or args.patch.with_suffix(".amxd")
    device = amxd(with_port(args.patch.read_bytes(), args.port))
    out.write_bytes(device)
    print(f"wrote {out.name}: {len(device)} bytes (port {args.port})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
