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


class TreeTests(unittest.TestCase):
    """The real files, copied into a scratch root and changed one at a time."""

    def setUp(self) -> None:
        self.root = Path(tempfile.mkdtemp())
        for rel in (cl.GENEXPR, cl.MAXPAT, cl.VERSION_TXT):
            (self.root / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(REPO / rel, self.root / rel)

    def tearDown(self) -> None:
        shutil.rmtree(self.root)

    def edit(self, rel: Path, old: str, new: str) -> None:
        path = self.root / rel
        text = path.read_text(encoding="utf-8")
        self.assertIn(old, text)
        path.write_text(text.replace(old, new, 1), encoding="utf-8")

    def patcher(self) -> dict:
        return json.loads((self.root / cl.MAXPAT).read_text(encoding="utf-8"))

    def save(self, patch: dict) -> None:
        (self.root / cl.MAXPAT).write_text(json.dumps(patch, indent="\t"), encoding="utf-8")

    def numbox(self, patch: dict, name: str) -> dict:
        return next(b for b in cl.boxes(patch["patcher"]) if b.get("varname") == name)

    def errors(self) -> list[str]:
        return cl.check(self.root)

    def test_the_repository_passes(self) -> None:
        self.assertEqual(cl.check(REPO), [])
        self.assertEqual(cl.main(["--root", str(REPO)]), 0)

    def test_the_copy_passes(self) -> None:
        self.assertEqual(self.errors(), [])

    def test_the_embedded_code_must_equal_the_genexpr(self) -> None:
        self.edit(cl.GENEXPR, "out1 = 0;", "out1 = in1;")
        errors = self.errors()
        self.assertEqual(
            errors, [f"{cl.MAXPAT.as_posix()}: the gen~ code differs from {cl.GENEXPR.as_posix()}"]
        )

    def test_an_absolute_path_fails(self) -> None:
        for path in ("C:\\\\Users\\\\x\\\\a.amxd", "/Users/x/a.amxd", "/home/x/a", "D:/x"):
            patch = self.patcher()
            patch["patcher"]["filename"] = path
            self.save(patch)
            self.assertTrue(any("absolute file path" in e for e in self.errors()), path)

    def test_a_parameter_with_another_range_or_unit_fails(self) -> None:
        patch = self.patcher()
        self.numbox(patch, "Voice")["saved_attribute_attributes"]["valueof"][
            "parameter_mmin"
        ] = -60.0
        self.numbox(patch, "Short-term")["saved_attribute_attributes"]["valueof"][
            "parameter_units"
        ] = "dB"
        self.save(patch)
        errors = self.errors()
        self.assertTrue(any("Voice has parameter_mmin" in e for e in errors), errors)
        self.assertTrue(any("Short-term's unit" in e for e in errors), errors)

    def test_a_missing_parameter_fails(self) -> None:
        patch = self.patcher()
        self.numbox(patch, "Voice")["varname"] = "Voix"
        self.save(patch)
        self.assertTrue(any("live.numbox parameters" in e for e in self.errors()))

    def test_the_visibility_is_one_setting_on_both(self) -> None:
        for short, voice, ok in ((1, 1, True), (0, 1, False), (2, 2, False)):
            patch = self.patcher()
            self.numbox(patch, "Short-term")["saved_attribute_attributes"]["valueof"][
                "parameter_invisible"
            ] = short
            self.numbox(patch, "Voice")["saved_attribute_attributes"]["valueof"][
                "parameter_invisible"
            ] = voice
            self.save(patch)
            visibility = [e for e in self.errors() if "visibility" in e]
            self.assertEqual(visibility == [], ok, (short, voice, visibility))

    def test_the_face_shows_version_txt(self) -> None:
        (self.root / cl.VERSION_TXT).write_text("9.9.9\n", encoding="utf-8")
        self.assertTrue(any("expected ['v9.9.9']" in e for e in self.errors()))

    def test_a_patch_without_plugout_fails(self) -> None:
        patch = self.patcher()
        box = next(b for b in cl.boxes(patch["patcher"]) if b.get("text") == "plugout~")
        box["text"] = "dac~"
        self.save(patch)
        self.assertIn(f"{cl.MAXPAT.as_posix()}: no plugout~", self.errors())

    def test_text_that_is_no_patcher_fails(self) -> None:
        (self.root / cl.MAXPAT).write_text("not json", encoding="utf-8")
        self.assertTrue(any("is not a Max patcher" in e for e in self.errors()))


if __name__ == "__main__":
    unittest.main()
