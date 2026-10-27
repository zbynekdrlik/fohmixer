"""Tests for build_amxd.py (#151): the header, the length field, the round trip."""

from __future__ import annotations

import json
import struct
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import build_amxd as ba  # noqa: E402


class AmxdTests(unittest.TestCase):
    def test_the_header_is_an_audio_effects(self) -> None:
        device = ba.amxd(b'{"patcher": {}}')
        self.assertEqual(device[:4], b"ampf")
        self.assertEqual(struct.unpack("<I", device[4:8]), (4,))
        self.assertEqual(device[8:12], b"aaaa")
        self.assertEqual(device[12:16], b"meta")
        self.assertEqual(struct.unpack("<I", device[16:20]), (4,))
        self.assertEqual(device[20:24], bytes(4))
        self.assertEqual(device[24:28], b"ptch")
        self.assertEqual(len(ba.header(0)), 32)

    def test_the_length_field_is_the_jsons_byte_length(self) -> None:
        patch = '{"patcher": {"description": "Lautstärke"}}'.encode()
        device = ba.amxd(patch)
        self.assertEqual(struct.unpack("<I", device[28:32]), (len(patch),))
        self.assertEqual(device[32:], patch)
        self.assertNotEqual(len(patch), len(patch.decode()))

    def test_the_real_patch_round_trips(self) -> None:
        patch = ba.MAXPAT.read_bytes()
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "x.amxd"
            self.assertEqual(ba.main(["--out", str(out)]), 0)
            device = out.read_bytes()
        self.assertEqual(ba.patch_of(device), patch)
        self.assertEqual(json.loads(ba.patch_of(device)), json.loads(patch))

    def test_text_that_is_no_patcher_is_refused(self) -> None:
        with self.assertRaises(ValueError):
            ba.amxd(b'{"box": {}}')
        with self.assertRaises(ValueError):
            ba.amxd(b"not json")

    def test_a_wrong_header_or_length_is_refused(self) -> None:
        device = ba.amxd(b'{"patcher": {}}')
        with self.assertRaises(ValueError):
            ba.patch_of(b"ampf" + device[4:8] + b"mmmm" + device[12:])
        with self.assertRaises(ValueError):
            ba.patch_of(device + b" ")


if __name__ == "__main__":
    unittest.main()
