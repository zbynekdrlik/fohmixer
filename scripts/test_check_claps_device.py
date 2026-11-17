"""Tests for check_claps_device.py (#220): the real patch passes, each
broken rule is named."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_claps_device as cc  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
TEXT = (ROOT / cc.MAXPAT).read_text(encoding="utf-8")
VERSION = (ROOT / cc.VERSION_TXT).read_text(encoding="utf-8").strip()
NUMBERS = {"hub": 39113, "device": 39114, "forget": 3000}


def errors(text: str, version: str = VERSION, numbers: dict | None = None) -> list[str]:
    return cc.patch_errors("p", text, version, numbers or NUMBERS)


def edited(change) -> str:
    """The real patch with `change(patcher)` applied."""
    patch = json.loads(TEXT)
    change(patch["patcher"])
    return json.dumps(patch)


def box(patcher: dict, box_id: str) -> dict:
    return next(b["box"] for b in patcher["boxes"] if b["box"]["id"] == box_id)


def add_line(patcher: dict, src: list, dst: list) -> None:
    patcher["lines"].append({"patchline": {"source": src, "destination": dst}})


class ClapsDeviceTests(unittest.TestCase):
    def test_the_real_patch_passes(self) -> None:
        self.assertEqual(cc.check(ROOT), [])
        self.assertEqual(cc.numbers_of(ROOT), NUMBERS)
        self.assertEqual(cc.main(["--root", str(ROOT)]), 0)

    def test_the_ports_must_match_the_hubs(self) -> None:
        found = errors(TEXT, numbers={**NUMBERS, "hub": 1})
        self.assertTrue(any("udpsend 127.0.0.1 1" in e for e in found), found)
        found = errors(TEXT, numbers={**NUMBERS, "device": 2})
        self.assertTrue(any("udpreceive 2" in e for e in found), found)
        found = errors(TEXT, numbers={**NUMBERS, "forget": 5})
        self.assertTrue(any("'delay 5'" in e for e in found), found)

    def test_the_version_must_match(self) -> None:
        found = errors(TEXT, version="9.9.9")
        self.assertTrue(any("/fohmixer/claps 1 v9.9.9" in e for e in found), found)
        self.assertTrue(any("its face shows" in e for e in found), found)

    def test_a_box_of_other_words_is_named(self) -> None:
        def quantize(p: dict) -> None:
            box(p, "obj-beats")["text"] = "metro 4n"

        found = errors(edited(quantize))
        self.assertTrue(any("obj-beats must be" in e for e in found), found)

        def velocity(p: dict) -> None:
            box(p, "obj-makenote")["text"] = "makenote 100 8n"

        self.assertTrue(any("obj-makenote" in e for e in errors(edited(velocity))))

    def test_the_watchdog_must_stop_the_device_and_forget_the_switch(self) -> None:
        def deaf(p: dict) -> None:
            p["lines"] = [
                ln for ln in p["lines"] if ln["patchline"]["source"] != ["obj-watchdog", 0]
            ]

        found = errors(edited(deaf))
        self.assertTrue(any("obj-watchdog:0 -> obj-stop:0 is missing" in e for e in found), found)
        self.assertTrue(any("obj-watchdog:0 -> obj-forget:0 is missing" in e for e in found), found)

    def test_the_hello_leaves_the_audio_thread_through_a_deferlow(self) -> None:
        # #240: the hello's metro runs in Live's audio thread.
        def direct(p: dict) -> None:
            p["lines"] = [
                ln for ln in p["lines"] if ln["patchline"]["destination"] != ["obj-udpsend", 0]
            ]
            add_line(p, ["obj-hello", 0], ["obj-udpsend", 0])

        found = errors(edited(direct))
        self.assertIn(
            "p: obj-hello-metro ('metro 1000') reaches the udpsend obj-udpsend on the audio "
            "thread (a deferlow or a qmetro goes between)",
            found,
        )
        self.assertIn(
            "p: obj-udpsend:0 takes [['obj-send-defer', 0]] alone, has [['obj-hello', 0]]",
            found,
        )

    def test_a_missing_line_is_named(self) -> None:
        def unwire(p: dict) -> None:
            p["lines"] = [ln for ln in p["lines"] if ln["patchline"]["source"] != ["obj-bar", 0]]

        found = errors(edited(unwire))
        self.assertTrue(any("obj-bar:0 -> obj-armed:0 is missing" in e for e in found), found)

    def test_midi_passes_only_from_midiin_to_midiout(self) -> None:
        def tap(p: dict) -> None:
            add_line(p, ["obj-midiin", 0], ["obj-noteout", 0])

        found = errors(edited(tap))
        self.assertTrue(any("midiin goes to midiout alone" in e for e in found), found)
        self.assertTrue(any("obj-noteout:0 takes" in e for e in found), found)

    def test_a_stray_line_into_a_guarded_inlet_is_named(self) -> None:
        def arm(p: dict) -> None:
            add_line(p, ["obj-start", 0], ["obj-armed", 0])

        found = errors(edited(arm))
        self.assertTrue(any("obj-armed:0 takes" in e for e in found), found)

    def test_a_second_udpsend_or_a_parameter_is_refused(self) -> None:
        def second(p: dict) -> None:
            copy = dict(box(p, "obj-udpsend"), id="obj-udpsend-2")
            p["boxes"].append({"box": copy})

        found = errors(edited(second))
        self.assertTrue(any("exactly one udpsend, has 2" in e for e in found), found)

        def parameter(p: dict) -> None:
            box(p, "obj-clapping")["parameter_enable"] = 1

        found = errors(edited(parameter))
        self.assertTrue(any("obj-clapping is a parameter" in e for e in found), found)

        def table(p: dict) -> None:
            p["parameters"] = {}

        self.assertTrue(any("parameters table" in e for e in errors(edited(table))))

    def test_a_line_past_its_ports_and_an_absolute_path_are_named(self) -> None:
        def past(p: dict) -> None:
            add_line(p, ["obj-iter", 3], ["obj-noteout", 2])

        self.assertTrue(any("past their ports" in e for e in errors(edited(past))))
        found = errors(TEXT.replace('"FohMixer Claps"', '"C:\\\\x\\\\FohMixer Claps"'))
        self.assertTrue(any("absolute file path" in e for e in found), found)

    def test_text_that_is_no_patcher_is_named(self) -> None:
        self.assertTrue(errors("not json")[0].startswith("p is not a Max patcher"))

    def test_a_proto_crate_without_the_ports_is_named(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            rs = Path(tmp) / cc.CLAPS_RS
            rs.parent.mkdir(parents=True)
            rs.write_text("pub const HUB_PORT: u16 = 39113;\n", encoding="utf-8")
            self.assertIn("no constant for ['device', 'forget']", cc.check(Path(tmp))[0])
            self.assertEqual(cc.main(["--root", tmp]), 1)


if __name__ == "__main__":
    unittest.main()
