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

    def test_builds_the_listen_patch_with_its_defaults(self) -> None:
        patch = ba.LISTEN_MAXPAT.read_bytes()
        self.assertTrue(ba.is_listen(patch))
        self.assertFalse(ba.is_listen(ba.MAXPAT.read_bytes()))
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "l.amxd"
            self.assertEqual(ba.main(["--patch", str(ba.LISTEN_MAXPAT), "--out", str(out)]), 0)
            device = out.read_bytes()
        self.assertEqual(ba.patch_of(device), patch)
        self.assertIn(b"pack /fohmixer/listen ", device)

    def test_the_listen_ports_rewrite_their_three_boxes(self) -> None:
        patch = ba.LISTEN_MAXPAT.read_bytes()
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "l.amxd"
            args = ["--patch", str(ba.LISTEN_MAXPAT), "--port", "40001", "--select-port", "40002"]
            self.assertEqual(ba.main([*args, "--out", str(out)]), 0)
            built = ba.patch_of(out.read_bytes())
        swaps = [
            (b'"udpsend 127.0.0.1 39111"', b'"udpsend 127.0.0.1 40001"'),
            (
                b'"jit.net.send @ip 127.0.0.1 @port 39111"',
                b'"jit.net.send @ip 127.0.0.1 @port 40001"',
            ),
            (b'"udpreceive 39112"', b'"udpreceive 40002"'),
        ]
        back = built
        for old, new in swaps:
            self.assertEqual(built.count(new), 1)
            back = back.replace(new, old)
        self.assertEqual(back, patch)
        self.assertEqual(ba.with_ports(patch), patch)
        only_port = ba.with_ports(patch, 40001)
        self.assertIn(b'"udpreceive 39112"', only_port)
        self.assertIn(b"@port 40001", only_port)
        for port, select in ((0, None), (None, 65536), (70000, 1)):
            with self.assertRaises(ValueError):
                ba.with_ports(patch, port, select)
        with self.assertRaises(ValueError):
            ba.with_ports(ba.MAXPAT.read_bytes(), 39110, 39112)
        with self.assertRaises(ValueError):
            ba.with_ports(patch.replace(b'"udpreceive 39112"', b'"udpreceive 1"'))

    def test_builds_the_claps_patch_as_a_midi_effect(self) -> None:
        patch = ba.CLAPS_MAXPAT.read_bytes()
        self.assertTrue(ba.is_claps(patch))
        self.assertFalse(ba.is_claps(ba.LISTEN_MAXPAT.read_bytes()))
        with tempfile.TemporaryDirectory() as tmp:
            out = Path(tmp) / "c.amxd"
            self.assertEqual(ba.main(["--patch", str(ba.CLAPS_MAXPAT), "--out", str(out)]), 0)
            device = out.read_bytes()
        self.assertEqual(device[8:12], b"mmmm")
        self.assertEqual(ba.patch_of(device), patch)
        self.assertIn(b"pack /fohmixer/claps ", device)

    def test_the_claps_ports_rewrite_their_two_boxes(self) -> None:
        patch = ba.CLAPS_MAXPAT.read_bytes()
        built = ba.with_ports(patch, 40003, device_port=40004)
        swaps = [
            (b'"udpsend 127.0.0.1 39113"', b'"udpsend 127.0.0.1 40003"'),
            (b'"udpreceive 39114"', b'"udpreceive 40004"'),
        ]
        back = built
        for old, new in swaps:
            self.assertEqual(built.count(new), 1)
            back = back.replace(new, old)
        self.assertEqual(back, patch)
        self.assertEqual(ba.with_ports(patch), patch)
        for port, device in ((0, None), (None, 65536)):
            with self.assertRaises(ValueError):
                ba.with_ports(patch, port, device_port=device)
        with self.assertRaises(ValueError):
            ba.with_ports(patch, select_port=39112)
        with self.assertRaises(ValueError):
            ba.with_ports(ba.LISTEN_MAXPAT.read_bytes(), device_port=39114)

    def test_text_that_is_no_patcher_is_refused(self) -> None:
        with self.assertRaises(ValueError):
            ba.amxd(b'{"box": {}}')
        with self.assertRaises(ValueError):
            ba.amxd(b"not json")

    def test_a_wrong_header_or_length_is_refused(self) -> None:
        device = ba.amxd(b'{"patcher": {}}')
        with self.assertRaises(ValueError):
            ba.patch_of(b"ampf" + device[4:8] + b"xxxx" + device[12:])
        with self.assertRaises(ValueError):
            ba.patch_of(b"ampx" + device[4:])
        with self.assertRaises(ValueError):
            ba.patch_of(device + b" ")


if __name__ == "__main__":
    unittest.main()
