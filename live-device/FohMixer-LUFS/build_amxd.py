#!/usr/bin/env python3
"""Write a device's .amxd from its .maxpat (#151, spec §4.2; #210): `FohMixer LUFS`, `FohMixer Meter` or `FohMixer Listen`.

An unfrozen Max audio effect device is the patcher JSON behind a small
header (read from a real Live 12 .amxd):

    b"ampf" u32le(4) b"aaaa"      the device kind: an audio effect
    b"meta" u32le(4) 4 zero bytes
    b"ptch" u32le(len(json))      then the patcher JSON, UTF-8

The JSON is copied byte for byte, except the ports when `--port` (and, for
the Listen device, `--select-port`) ask for others than the hub's defaults
(the device cannot read the hub's config; README.md): the `udpsend` box's
port, and the Listen device's `jit.net.send` and `udpreceive` boxes. The
.amxd is never committed (.gitignore): the main session builds it here and
installs it on the PC, where Max can freeze it (README.md).

    python3 build_amxd.py [--patch <maxpat>] [--port <n>] [--select-port <n>] [--out <amxd>]

`--patch` defaults to the LUFS patch, `--out` to the patch's own name with
`.amxd`, `--port` to 39110 (39111 for the Listen device), `--select-port`
to 39112 (the Listen device only).
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
LISTEN_MAXPAT = HERE.parent / "FohMixer-Listen" / "FohMixer Listen.maxpat"
KIND_AUDIO_EFFECT = b"aaaa"
# The hub's default port (fohmixer_proto::lufs::wire::DEFAULT_PORT), as the
# patches carry it; scripts/check_lufs_device.py keeps them equal.
DEFAULT_PORT = 39110
# The Listen device's (#210, fohmixer_proto::listen::DEFAULT_PORT and
# SELECT_PORT; scripts/check_listen_device.py keeps them equal).
LISTEN_PORT = 39111
SELECT_PORT = 39112


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


def _swap(patch: bytes, old: str, new: str) -> bytes:
    """`patch` with its one box text `old` replaced by `new` (refused unless
    it holds exactly one)."""
    box = f'"{old}"'.encode()
    if patch.count(box) != 1:
        raise ValueError(f"the patch needs exactly one {box.decode()} box")
    return patch.replace(box, f'"{new}"'.encode())


def _check(port: int) -> None:
    if not 0 < port < 65536:
        raise ValueError(f"no port: {port}")


def is_listen(patch: bytes) -> bool:
    """The Listen device's patch (it streams with `jit.net.send`)."""
    return b'"jit.net.send ' in patch


def with_ports(patch: bytes, port: int | None = None, select_port: int | None = None) -> bytes:
    """The patch sending to `port` (its `udpsend`, and the Listen device's
    `jit.net.send`) and, for the Listen device, taking selects on
    `select_port`; none keeps the default."""
    listen = is_listen(patch)
    default = LISTEN_PORT if listen else DEFAULT_PORT
    port = default if port is None else port
    _check(port)
    patch = _swap(patch, f"udpsend 127.0.0.1 {default}", f"udpsend 127.0.0.1 {port}")
    if not listen:
        if select_port is not None:
            raise ValueError("only the Listen device takes a select port")
        return patch
    select_port = SELECT_PORT if select_port is None else select_port
    _check(select_port)
    patch = _swap(
        patch,
        f"jit.net.send @ip 127.0.0.1 @port {LISTEN_PORT}",
        f"jit.net.send @ip 127.0.0.1 @port {port}",
    )
    return _swap(patch, f"udpreceive {SELECT_PORT}", f"udpreceive {select_port}")


def with_port(patch: bytes, port: int) -> bytes:
    """The patch with its one `udpsend 127.0.0.1 <default>` box sending to `port`."""
    return with_ports(patch, port)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--patch", "--maxpat", dest="patch", type=Path, default=MAXPAT)
    parser.add_argument("--port", type=int, default=None)
    parser.add_argument("--select-port", type=int, default=None)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args(argv)
    out = args.out or args.patch.with_suffix(".amxd")
    patch = args.patch.read_bytes()
    device = amxd(with_ports(patch, args.port, args.select_port))
    out.write_bytes(device)
    port = args.port or (LISTEN_PORT if is_listen(patch) else DEFAULT_PORT)
    print(f"wrote {out.name}: {len(device)} bytes (port {port})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
