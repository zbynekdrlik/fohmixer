"""Tests for scripts/tray_icon.py: the committed tray icon is the drawn one,
and its .ico structure is what Tauri's build step and Windows read (#39)."""

from __future__ import annotations

import struct
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import tray_icon  # noqa: E402


class IconTests(unittest.TestCase):
    def test_the_committed_icon_is_the_drawn_one(self) -> None:
        self.assertEqual(tray_icon.ICON.read_bytes(), tray_icon.ico_bytes())
        self.assertEqual(tray_icon.main(["--check"]), 0)

    def test_the_directory_lists_16_32_and_48_px_32_bit_images(self) -> None:
        data = tray_icon.ico_bytes()
        reserved, kind, count = struct.unpack_from("<HHH", data, 0)
        self.assertEqual((reserved, kind, count), (0, 1, 3))
        end = 6 + 16 * count
        for i, size in enumerate((16, 32, 48)):
            w, h, colours, _, planes, bpp, length, offset = struct.unpack_from(
                "<BBBBHHII", data, 6 + 16 * i
            )
            self.assertEqual((w, h, colours, planes, bpp), (size, size, 0, 1, 32))
            self.assertEqual(offset, end, "images follow each other")
            header = struct.unpack_from("<IiiHHI", data, offset)
            self.assertEqual(header, (40, size, size * 2, 1, 32, 0))
            stride = (size + 31) // 32 * 4
            self.assertEqual(length, 40 + size * size * 4 + stride * size)
            end += length
        self.assertEqual(end, len(data))

    def test_the_mask_marks_the_clear_corners_only(self) -> None:
        rows = tray_icon.pixels(32)
        self.assertEqual(rows[0][0], tray_icon.CLEAR)
        self.assertEqual(rows[16][1], tray_icon.BACKGROUND)
        image = tray_icon.bmp_image(32)
        mask = image[40 + 32 * 32 * 4 :]
        # The mask is bottom-up: its first row is the image's last.
        self.assertEqual(mask[0] & 0x80, 0x80, "the bottom-left corner is transparent")
        self.assertEqual(mask[16 * 4] & 0x40, 0, "the left edge's middle is not")

    def test_every_size_draws_three_caps_on_three_tracks(self) -> None:
        for size in tray_icon.SIZES:
            rows = tray_icon.pixels(size)
            self.assertEqual(len(rows), size)
            self.assertTrue(all(len(row) == size for row in rows))
            for fx, fy in tray_icon.FADERS:
                cx, cy = int(fx * size), int(fy * size)
                self.assertEqual(rows[cy][cx], tray_icon.CAP, f"cap at {size} px, x {cx}")
                self.assertEqual(
                    rows[size * 3 // 16][cx], tray_icon.TRACK, f"track top at {size} px, x {cx}"
                )

    def test_check_reports_a_different_file(self) -> None:
        saved = tray_icon.ICON
        try:
            tray_icon.ICON = saved.with_name("no-such-icon.ico")
            self.assertEqual(tray_icon.main(["--check"]), 1)
        finally:
            tray_icon.ICON = saved


if __name__ == "__main__":
    unittest.main()
