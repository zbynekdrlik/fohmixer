#!/usr/bin/env python3
"""Write a device's .amxd from its .maxpat (#151, spec §4.2; #210; #220): `FohMixer LUFS`, `FohMixer Meter`, `FohMixer Listen` or `FohMixer Claps`.

An unfrozen Max device is the patcher JSON behind a small header (read
from a real Live 12 .amxd):

    b"ampf" u32le(4) b"aaaa"      the device kind: an audio effect
                                  (b"mmmm": a MIDI effect, FohMixer Claps)
    b"meta" u32le(4) 4 zero bytes
    b"ptch" u32le(len(json))      then the patcher JSON, UTF-8

The JSON is copied byte for byte, except the ports when `--port` (and, for
the Listen device, `--hub-port`; for the Claps device, `--device-port`)
ask for others than the hub's defaults (the device cannot read the hub's
config; README.md): the `udpsend` box's port, the Listen device's
`jit.net.send` box and the hub's HTTP port in its select poll's `sprintf`
box, the Claps device's `udpreceive` box.
The .amxd is never committed (.gitignore): the main session builds it here
and installs it on the PC, where Max can freeze it (README.md).

    python3 build_amxd.py [--patch <maxpat>] [--port <n>] [--hub-port <n>]
                          [--device-port <n>] [--out <amxd>]

`--patch` defaults to the LUFS patch, `--out` to the patch's own name with
`.amxd`, `--port` to 39110 (39111 for the Listen device, 39113 for the
Claps device), `--hub-port` to 8480 (the hub's HTTP port, the Listen device
only),
`--device-port` to 39114 (the Claps device only).
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
CLAPS_MAXPAT = HERE.parent / "FohMixer-Claps" / "FohMixer Claps.maxpat"
KIND_AUDIO_EFFECT = b"aaaa"
KIND_MIDI_EFFECT = b"mmmm"
KINDS = (KIND_AUDIO_EFFECT, KIND_MIDI_EFFECT)
# The hub's default port (fohmixer_proto::lufs::wire::DEFAULT_PORT), as the
# patches carry it; scripts/check_lufs_device.py keeps them equal.
DEFAULT_PORT = 39110
# The Listen device's (#210, fohmixer_proto::listen::DEFAULT_PORT, and the
# hub's HTTP port, fohmixer_hub::config::DEFAULT_HTTP_PORT, where it asks
# whether it is selected; scripts/check_listen_device.py keeps them equal).
LISTEN_PORT = 39111
HUB_PORT = 8480
# The select poll's `sprintf` box: the URL a Listen device asks (`%ld` its
# listen id).
POLL_BOX = "sprintf http://127.0.0.1:{port}/api/listen/select?device=%ld"
# The Claps device's (#220, fohmixer_proto::claps::HUB_PORT and
# DEVICE_PORT; scripts/check_claps_device.py keeps them equal).
CLAPS_PORT = 39113
DEVICE_PORT = 39114


def header(patch_len: int, kind: bytes = KIND_AUDIO_EFFECT) -> bytes:
    return (
        b"ampf"
        + struct.pack("<I", 4)
        + kind
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
    kind = KIND_MIDI_EFFECT if is_claps(patch) else KIND_AUDIO_EFFECT
    return header(len(patch), kind) + patch


def patch_of(device: bytes) -> bytes:
    """The patcher JSON inside a device written by `amxd`."""
    kind = device[8:12]
    head = header(0, kind)
    if kind not in KINDS or device[: len(head) - 4] != head[:-4]:
        raise ValueError("not an unfrozen audio or MIDI effect .amxd")
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


def is_claps(patch: bytes) -> bool:
    """The Claps device's patch (a MIDI effect: it takes Live's MIDI)."""
    return b'"midiin"' in patch


def with_ports(
    patch: bytes,
    port: int | None = None,
    hub_port: int | None = None,
    device_port: int | None = None,
) -> bytes:
    """The patch sending to `port` (its `udpsend`, and the Listen device's
    `jit.net.send`) and, for the Listen device, asking the hub's HTTP port
    `hub_port` whether it is selected, for the Claps device taking the hub's
    words on `device_port`; none keeps the default."""
    listen = is_listen(patch)
    claps = is_claps(patch)
    default = LISTEN_PORT if listen else CLAPS_PORT if claps else DEFAULT_PORT
    port = default if port is None else port
    _check(port)
    patch = _swap(patch, f"udpsend 127.0.0.1 {default}", f"udpsend 127.0.0.1 {port}")
    if device_port is not None and not claps:
        raise ValueError("only the Claps device takes a device port")
    if claps:
        if hub_port is not None:
            raise ValueError("only the Listen device takes a hub port")
        device_port = DEVICE_PORT if device_port is None else device_port
        _check(device_port)
        defer = "@defer 1"
        return _swap(
            patch, f"udpreceive {DEVICE_PORT} {defer}", f"udpreceive {device_port} {defer}"
        )
    if not listen:
        if hub_port is not None:
            raise ValueError("only the Listen device takes a hub port")
        return patch
    hub_port = HUB_PORT if hub_port is None else hub_port
    _check(hub_port)
    patch = _swap(
        patch,
        f"jit.net.send @ip 127.0.0.1 @port {LISTEN_PORT}",
        f"jit.net.send @ip 127.0.0.1 @port {port}",
    )
    return _swap(patch, POLL_BOX.format(port=HUB_PORT), POLL_BOX.format(port=hub_port))


def with_port(patch: bytes, port: int) -> bytes:
    """The patch with its one `udpsend 127.0.0.1 <default>` box sending to `port`."""
    return with_ports(patch, port)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--patch", "--maxpat", dest="patch", type=Path, default=MAXPAT)
    parser.add_argument("--port", type=int, default=None)
    parser.add_argument("--hub-port", type=int, default=None)
    parser.add_argument("--device-port", type=int, default=None)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args(argv)
    out = args.out or args.patch.with_suffix(".amxd")
    patch = args.patch.read_bytes()
    device = amxd(with_ports(patch, args.port, args.hub_port, args.device_port))
    out.write_bytes(device)
    default = LISTEN_PORT if is_listen(patch) else CLAPS_PORT if is_claps(patch) else DEFAULT_PORT
    port = args.port or default
    print(f"wrote {out.name}: {len(device)} bytes (port {port})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
