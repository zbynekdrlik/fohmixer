#!/usr/bin/env python3
"""The tray's icon, crates/fohmixer-tray/icons/icon.ico (#39), drawn here.

Tauri's build step embeds it as the exe's icon and the tray shows it (the
app's default window icon). Three fader tracks with their caps on a dark
rounded square, at 16, 32 and 48 px, as 32-bit BMP images (no compression, so
the bytes are the same on every Python and zlib). Standard library only.

    python3 scripts/tray_icon.py          # writes the file
    python3 scripts/tray_icon.py --check  # exit 1 when the file differs
"""

from __future__ import annotations

import argparse
import struct
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ICON = ROOT / "crates" / "fohmixer-tray" / "icons" / "icon.ico"
SIZES = (16, 32, 48)

# RGBA colours: the square, the tracks, the caps.
BACKGROUND = (0x1F, 0x25, 0x33, 0xFF)
TRACK = (0x8A, 0x93, 0xA6, 0xFF)
CAP = (0xFF, 0xA0, 0x28, 0xFF)
CLEAR = (0, 0, 0, 0)
# Each fader: its track's centre (a fraction of the width) and its cap's
# centre (a fraction of the height).
FADERS = ((0.25, 0.62), (0.5, 0.38), (0.75, 0.52))


def _inside_rounded_square(x: int, y: int, size: int) -> bool:
    """Whether pixel (x, y) is inside the square with corners of radius size/5."""
    r = size / 5
    cx = min(max(x + 0.5, r), size - r)
    cy = min(max(y + 0.5, r), size - r)
    return (x + 0.5 - cx) ** 2 + (y + 0.5 - cy) ** 2 <= r * r


def pixels(size: int) -> list[list[tuple[int, int, int, int]]]:
    """The icon at `size` px: rows top to bottom, RGBA pixels left to right."""
    rows = [
        [BACKGROUND if _inside_rounded_square(x, y, size) else CLEAR for x in range(size)]
        for y in range(size)
    ]
    unit = max(1, size // 16)
    top, bottom = size * 3 // 16, size - size * 3 // 16
    cap_w, cap_h = unit * 4, max(2, size // 8)
    for fx, fy in FADERS:
        cx = int(fx * size)
        for y in range(top, bottom):
            for x in range(cx - unit // 2, cx - unit // 2 + unit):
                rows[y][x] = TRACK
        cy = int(fy * size)
        for y in range(cy - cap_h // 2, cy - cap_h // 2 + cap_h):
            for x in range(cx - cap_w // 2, cx - cap_w // 2 + cap_w):
                rows[y][x] = CAP
    return rows


def bmp_image(size: int) -> bytes:
    """One ICO image: a BITMAPINFOHEADER (height doubled for the mask), the
    BGRA pixels bottom-up, then the 1-bit AND mask (1 = transparent), each of
    its rows padded to 4 bytes."""
    rows = pixels(size)
    colour = b"".join(bytes((b, g, r, a)) for row in reversed(rows) for (r, g, b, a) in row)
    stride = (size + 31) // 32 * 4
    mask = bytearray()
    for row in reversed(rows):
        line = bytearray(stride)
        for x, (_, _, _, a) in enumerate(row):
            if a == 0:
                line[x // 8] |= 0x80 >> (x % 8)
        mask += line
    header = struct.pack(
        "<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0, len(colour) + len(mask), 0, 0, 0, 0
    )
    return header + colour + bytes(mask)


def ico_bytes() -> bytes:
    """The whole .ico: its directory, then one image per size."""
    images = [bmp_image(size) for size in SIZES]
    out = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    for size, image in zip(SIZES, images, strict=True):
        out += struct.pack("<BBBBHHII", size, size, 0, 0, 1, 32, len(image), offset)
        offset += len(image)
    return out + b"".join(images)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="exit 1 when the icon file differs")
    args = parser.parse_args(argv)
    want = ico_bytes()
    if args.check:
        if not ICON.is_file() or ICON.read_bytes() != want:
            print(
                f"{ICON.relative_to(ROOT)} differs from scripts/tray_icon.py: run it to write the file"
            )
            return 1
        print(f"{ICON.relative_to(ROOT)}: up to date")
        return 0
    ICON.parent.mkdir(parents=True, exist_ok=True)
    ICON.write_bytes(want)
    print(f"wrote {ICON.relative_to(ROOT)} ({len(want)} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
