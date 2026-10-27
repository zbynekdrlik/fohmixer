"""Tests for scripts/check_lufs_device.py (#151)."""

from __future__ import annotations

import json
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_lufs_device as cl  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
LUFS, METER = cl.DEVICES


class TreeTests(unittest.TestCase):
    """The real files, copied into a scratch root and changed one at a time."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        files = [cl.WIRE]
        for device in cl.DEVICES:
            files += [device.maxpat, device.version_txt]
            if device.genexpr is not None:
                files.append(device.genexpr)
        for rel in files:
            (self.root / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(REPO / rel, self.root / rel)

    def tearDown(self) -> None:
        shutil.rmtree(self.root)

    def edit(self, rel: Path, old: str, new: str) -> None:
        path = self.root / rel
        text = path.read_text(encoding="utf-8")
        self.assertIn(old, text)
        path.write_text(text.replace(old, new, 1), encoding="utf-8")

    def patcher(self, device: cl.Device = LUFS) -> dict:
        return json.loads((self.root / device.maxpat).read_text(encoding="utf-8"))

    def save(self, patch: dict, device: cl.Device = LUFS) -> None:
        (self.root / device.maxpat).write_text(json.dumps(patch, indent="\t"), encoding="utf-8")

    def box(self, patch: dict, first_word: str) -> dict:
        return next(b for b in cl.boxes(patch["patcher"]) if cl.words(b)[:1] == [first_word])

    def restore(self, device: cl.Device) -> None:
        shutil.copyfile(REPO / device.maxpat, self.root / device.maxpat)

    def errors(self) -> list[str]:
        return cl.check(self.root)

    def test_the_repository_passes(self) -> None:
        self.assertEqual(cl.check(REPO), [])
        self.assertEqual(cl.main(["--root", str(REPO)]), 0)

    def test_both_patches_pass(self) -> None:
        self.assertEqual(self.errors(), [])
        self.assertEqual([d.address for d in cl.DEVICES], ["/fohmixer/lufs", "/fohmixer/meter"])

    def test_the_hubs_port_is_read_from_the_wire_module(self) -> None:
        self.assertEqual(cl.default_port("x\npub const DEFAULT_PORT: u16 = 39110;\ny"), 39110)
        self.assertIsNone(cl.default_port("pub const DEFAULT_PORT: u32 = 39110;"))
        self.edit(cl.WIRE, "pub const DEFAULT_PORT: u16 = 39110;", "pub const PORT: u16 = 1;")
        self.assertEqual(len(self.errors()), 1)
        self.assertIn("DEFAULT_PORT", self.errors()[0])

    def test_a_parameter_object_fails(self) -> None:
        for device in cl.DEVICES:
            patch = self.patcher(device)
            patch["patcher"]["boxes"].append(
                {"box": {"id": "obj-x", "maxclass": "live.numbox", "numinlets": 1}}
            )
            self.save(patch, device)
            self.assertIn(
                f"{device.maxpat.as_posix()}: obj-x is a parameter (the device has none)",
                self.errors(),
            )
        patch = self.patcher(METER)
        self.box(patch, "plugin~")["parameter_enable"] = 1
        patch["patcher"]["parameters"] = {}
        self.save(patch, METER)
        errors = self.errors()
        self.assertIn(
            f"{METER.maxpat.as_posix()}: obj-plugin is a parameter (the device has none)", errors
        )
        self.assertTrue(any("parameters table" in e for e in errors), errors)
        self.assertFalse(cl.is_parameter({"maxclass": "flonum", "parameter_enable": 0}))
        self.assertFalse(cl.is_parameter({"maxclass": "newobj", "text": "live.path"}))

    def test_udpsend_must_be_localhost_and_the_hubs_port(self) -> None:
        for text in ("udpsend 0.0.0.0 39110", "udpsend 127.0.0.1 39111", "udpsend 127.0.0.1"):
            patch = self.patcher()
            self.box(patch, "udpsend")["text"] = text
            self.save(patch)
            self.assertTrue(any("udpsend 127.0.0.1 39110" in e for e in self.errors()), text)
        patch = self.patcher(METER)
        patch["patcher"]["boxes"].append(
            {"box": {"id": "obj-x", "maxclass": "newobj", "text": "udpsend 127.0.0.1 39110"}}
        )
        self.save(patch, METER)
        self.assertTrue(any("exactly one 'udpsend" in e for e in self.errors()))

    def test_the_send_metro_sets_each_devices_rate(self) -> None:
        # The owner's ruling of 27.10: the LUFS device sends every 200 ms,
        # the meter every 33 ms.
        self.assertEqual([d.send_ms for d in cl.DEVICES], [200, 33])
        for device, wrong in ((LUFS, "metro 100"), (METER, "metro 200")):
            patch = self.patcher(device)
            box = next(b for b in cl.boxes(patch["patcher"]) if b.get("id") == cl.SEND_METRO)
            box["text"] = wrong
            self.save(patch, device)
            self.assertIn(
                f"{device.maxpat.as_posix()}: obj-send-metro must be 'metro {device.send_ms}',"
                f" has [{wrong.split()!r}]",
                self.errors(),
            )
            self.restore(device)
        patch = self.patcher(LUFS)
        patch["patcher"]["boxes"] = [
            item for item in patch["patcher"]["boxes"] if item["box"].get("id") != cl.SEND_METRO
        ]
        self.save(patch, LUFS)
        self.assertTrue(
            any("obj-send-metro must be 'metro 200', has []" in e for e in self.errors())
        )

    def test_the_port_follows_the_wire_module(self) -> None:
        self.edit(cl.WIRE, "DEFAULT_PORT: u16 = 39110;", "DEFAULT_PORT: u16 = 39111;")
        errors = self.errors()
        self.assertEqual(len(errors), 2, errors)
        self.assertTrue(all("'udpsend 127.0.0.1 39111'" in e for e in errors), errors)

    def test_the_address_box_names_the_kind(self) -> None:
        patch = self.patcher()
        pack = self.box(patch, "pack")
        pack["text"] = pack["text"].replace("/fohmixer/lufs", "/fohmixer/meter")
        self.save(patch)
        self.assertIn(
            f"{LUFS.maxpat.as_posix()}: the pack sends to /fohmixer/meter, expected /fohmixer/lufs",
            self.errors(),
        )
        patch = self.patcher(METER)
        pack = self.box(patch, "pack")
        pack["text"] = pack["text"] + " 0"
        self.save(patch, METER)
        self.assertTrue(any("one pack of 10 arguments" in e for e in self.errors()))

    def test_the_pack_sends_the_version(self) -> None:
        patch = self.patcher(METER)
        pack = self.box(patch, "pack")
        version = (REPO / METER.version_txt).read_text(encoding="utf-8").strip()
        pack["text"] = pack["text"].replace(f"v{version}", "v0.0.1")
        self.save(patch, METER)
        self.assertIn(
            f"{METER.maxpat.as_posix()}: the pack sends v0.0.1, expected v{version}",
            self.errors(),
        )

    def test_the_output_is_silence_and_nothing_else_feeds_plugout(self) -> None:
        # The input straight through (the probe's pass-through), or the gen~.
        for source in (["obj-plugin", 0], ["obj-gen", 2]):
            self.restore(LUFS)
            patch = self.patcher()
            patch["patcher"]["lines"].append(
                {"patchline": {"source": source, "destination": ["obj-plugout", 1]}}
            )
            self.save(patch)
            errors = self.errors()
            self.assertEqual(
                errors,
                [
                    f"{LUFS.maxpat.as_posix()}: plugout~ must take silence only (a sig~ 0 "
                    f"nothing feeds; the device sits in a parallel chain), it takes "
                    f"['{source[0]}']"
                ],
            )
        # A sig~ with another value, or one the input feeds.
        self.restore(LUFS)
        patch = self.patcher(METER)
        self.box(patch, "sig~")["text"] = "sig~ 1."
        self.save(patch, METER)
        self.assertTrue(any("silence only" in e for e in self.errors()))
        self.restore(METER)
        patch = self.patcher(METER)
        patch["patcher"]["lines"].append(
            {"patchline": {"source": ["obj-plugin", 0], "destination": ["obj-silence", 0]}}
        )
        self.save(patch, METER)
        self.assertTrue(any("it takes ['obj-silence']" in e for e in self.errors()))
        # `sig~ 0` and no audio into plugout~ at all are silence too.
        self.restore(METER)
        patch = self.patcher(METER)
        self.box(patch, "sig~")["text"] = "sig~ 0"
        self.save(patch, METER)
        self.assertEqual(self.errors(), [])
        patch["patcher"]["lines"] = [
            ln
            for ln in patch["patcher"]["lines"]
            if ln["patchline"]["destination"][0] != "obj-plugout"
        ]
        self.save(patch, METER)
        self.assertEqual(self.errors(), [])

    def test_a_patch_without_plugout_fails(self) -> None:
        patch = self.patcher()
        self.box(patch, "plugout~")["text"] = "dac~"
        self.save(patch)
        self.assertIn(
            f"{LUFS.maxpat.as_posix()}: needs exactly one plugin~ and one plugout~", self.errors()
        )

    def test_a_line_past_a_boxs_ports_fails(self) -> None:
        patch = self.patcher(METER)
        patch["patcher"]["lines"].append(
            {"patchline": {"source": ["obj-plugin", 5], "destination": ["obj-pack", 0]}}
        )
        patch["patcher"]["lines"].append(
            {"patchline": {"source": ["obj-gone", 0], "destination": ["obj-pack", 0]}}
        )
        self.save(patch, METER)
        errors = self.errors()
        self.assertTrue(any("obj-plugin:5 -> obj-pack:0 is past" in e for e in errors), errors)
        self.assertTrue(any("not there: obj-gone" in e for e in errors), errors)

    def test_the_identity_messages_are_there(self) -> None:
        patch = self.patcher()
        box = next(
            b
            for b in cl.boxes(patch["patcher"])
            if b.get("text") == "path this_device canonical_parent canonical_parent"
        )
        box["text"] = "getpath"
        self.save(patch)
        self.assertIn(
            f"{LUFS.maxpat.as_posix()}: no 'path this_device canonical_parent canonical_parent' "
            "message",
            self.errors(),
        )
        self.assertEqual(len(cl.IDENTITY), 5)

    def test_the_meter_patch_needs_no_gen(self) -> None:
        patch = self.patcher(METER)
        self.assertEqual([b for b in cl.boxes(patch["patcher"]) if cl.words(b)[:1] == ["gen~"]], [])
        self.assertEqual(self.errors(), [])
        # The LUFS patch does.
        patch = self.patcher()
        self.box(patch, "gen~")["text"] = "mc.gen~"
        self.save(patch)
        self.assertTrue(any("exactly one gen~" in e for e in self.errors()))

    def test_the_embedded_code_must_equal_the_genexpr(self) -> None:
        self.edit(LUFS.genexpr, "out1 = 0;", "out1 = in1;")
        self.assertEqual(
            self.errors(), [f"{LUFS.maxpat.as_posix()}: the gen~ code differs from its .genexpr"]
        )

    def test_an_absolute_path_fails(self) -> None:
        for path in ("C:\\\\Users\\\\x\\\\a.amxd", "/Users/x/a.amxd", "/home/x/a", "D:/x"):
            patch = self.patcher(METER)
            patch["patcher"]["filename"] = path
            self.save(patch, METER)
            self.assertTrue(any("absolute file path" in e for e in self.errors()), path)

    def test_the_lufs_face_shows_its_version_txt(self) -> None:
        (self.root / LUFS.version_txt).write_text("9.9.9\n", encoding="utf-8")
        errors = self.errors()
        self.assertTrue(any("expected ['v9.9.9']" in e for e in errors), errors)
        self.assertTrue(all(LUFS.maxpat.as_posix() in e for e in errors), errors)

    def test_the_meter_face_shows_its_version_txt(self) -> None:
        (self.root / METER.version_txt).write_text("9.9.9\n", encoding="utf-8")
        errors = self.errors()
        self.assertTrue(any("expected ['v9.9.9']" in e for e in errors), errors)
        self.assertTrue(all(METER.maxpat.as_posix() in e for e in errors), errors)

    def test_text_that_is_no_patcher_fails(self) -> None:
        (self.root / METER.maxpat).write_text("not json", encoding="utf-8")
        self.assertTrue(any("is not a Max patcher" in e for e in self.errors()))
        self.assertEqual(cl.main(["--root", str(self.root)]), 1)


if __name__ == "__main__":
    unittest.main()
