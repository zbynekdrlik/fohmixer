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

    def test_builds_the_meter_patch(self) -> None:
        patch = ba.METER_MAXPAT.read_bytes()
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "m.amxd"
            self.assertEqual(ba.main(["--patch", str(ba.METER_MAXPAT), "--out", str(out)]), 0)
            device = out.read_bytes()
        self.assertEqual(ba.patch_of(device), patch)
        self.assertEqual(device[8:12], b"aaaa")
        self.assertIn(b"pack /fohmixer/meter ", device)

    def test_port_rewrites_only_the_udpsend_box(self) -> None:
        patch = ba.MAXPAT.read_bytes()
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "x.amxd"
            self.assertEqual(ba.main(["--port", "39999", "--out", str(out)]), 0)
            built = ba.patch_of(out.read_bytes())
        old, new = b'"udpsend 127.0.0.1 39110"', b'"udpsend 127.0.0.1 39999"'
        self.assertEqual(built.count(new), 1)
        self.assertEqual(built.replace(new, old), patch)
        self.assertEqual(len(built), len(patch))
        # The default writes the port as it is.
        self.assertEqual(ba.with_port(patch, ba.DEFAULT_PORT), patch)

    def test_a_bad_port_or_a_patch_without_one_udpsend_is_refused(self) -> None:
        patch = ba.MAXPAT.read_bytes()
        for port in (0, 65536, -1):
            with self.assertRaises(ValueError):
                ba.with_port(patch, port)
        self.assertEqual(ba.with_port(patch, 65535).count(b"127.0.0.1 65535"), 1)
        self.assertEqual(ba.with_port(patch, 1).count(b'127.0.0.1 1"'), 1)
        with self.assertRaises(ValueError):
            ba.with_port(b'{"patcher": {}}', 39111)
        with self.assertRaises(ValueError):
            ba.with_port(patch + patch, 39111)

    def test_the_default_out_is_the_patchs_name(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            patch = Path(tmp) / "Some Device.maxpat"
            patch.write_bytes(ba.METER_MAXPAT.read_bytes())
            self.assertEqual(ba.main(["--patch", str(patch)]), 0)
            self.assertTrue((Path(tmp) / "Some Device.amxd").is_file())

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
