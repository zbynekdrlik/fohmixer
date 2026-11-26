"""Tests for check_listen_device.py (#210; #264: the device answers the
hub's polls and holds no clock): the real patch passes, each broken rule is
named."""

from __future__ import annotations

import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_listen_device as cl  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
TEXT = (ROOT / cl.MAXPAT).read_text(encoding="utf-8")
VERSION = (ROOT / cl.VERSION_TXT).read_text(encoding="utf-8").strip()
NUMBERS = {
    "port": 39111,
    "frame": 960,
    "reread": 40,
    "poll": "/fohmixer/listen/poll",
    "frames": "/fohmixer/listen/frame",
    "min": 40000,
    "span": 10000,
}


def errors(text: str, version: str = VERSION, numbers: dict | None = None) -> list[str]:
    return cl.patch_errors("p", text, version, numbers or NUMBERS)


def edited(change) -> str:
    """The real patch with `change(patcher)` applied."""
    patch = json.loads(TEXT)
    change(patch["patcher"])
    return json.dumps(patch)


def box(patcher: dict, box_id: str) -> dict:
    return next(b["box"] for b in patcher["boxes"] if b["box"]["id"] == box_id)


def added(box_id: str, text: str, inlets: int = 2, outlets: int = 1):
    """A change that adds one object box."""

    def change(p: dict) -> None:
        p["boxes"].append(
            {
                "box": {
                    "id": box_id,
                    "maxclass": "newobj",
                    "text": text,
                    "numinlets": inlets,
                    "numoutlets": outlets,
                    "outlettype": ["bang"] * outlets,
                }
            }
        )

    return change


def line(src: str, outlet: int, dst: str, inlet: int):
    """A change that adds one line."""

    def change(p: dict) -> None:
        p["lines"].append({"patchline": {"source": [src, outlet], "destination": [dst, inlet]}})

    return change


def both(*changes):
    def change(p: dict) -> None:
        for one in changes:
            one(p)

    return change


class ListenDeviceTests(unittest.TestCase):
    def test_the_real_patch_passes(self) -> None:
        self.assertEqual(cl.check(ROOT), [])
        self.assertEqual(cl.numbers_of(ROOT), NUMBERS)

    def test_the_numbers_must_match_the_hubs(self) -> None:
        for name, value, word in (
            ("port", 1, "jit.net.send"),
            ("frame", 1, "jit.catch~"),
            ("reread", 7, "counter"),
            ("poll", "/x", "route"),
            ("frames", "/x", "route"),
            ("min", 1, "udpreceive"),
            ("span", 1, "random"),
        ):
            found = errors(TEXT, numbers={**NUMBERS, name: value})
            self.assertTrue(any(word in e for e in found), (name, found))
        found = errors(TEXT, numbers={**NUMBERS, "port": 1})
        self.assertTrue(any("udpsend 127.0.0.1 1" in e for e in found), found)

    def test_the_version_must_match(self) -> None:
        found = errors(TEXT, version="9.9.9")
        self.assertTrue(any("fohmixer_listen" in e for e in found), found)
        self.assertTrue(any("/fohmixer/listen" in e for e in found), found)
        self.assertTrue(any("its face shows" in e for e in found), found)

    def test_every_clock_is_refused_anywhere_in_the_device(self) -> None:
        # #264: a clock runs on Max's scheduler, inside Live's audio thread,
        # and its output posts from there; the device holds none, not even
        # one for the stream (the hub's frame poll drives the catch).
        for text in ("deferlow", "defer", "metro 1000", "qmetro 10", "delay 1000", "del 5", "pipe"):
            found = errors(edited(added("obj-clock", text)))
            self.assertIn(
                f"p: obj-clock ('{text}') has no place in the device (it answers the hub's "
                "poll in Max's main thread: no deferlow, no metro, no qmetro, no delay)",
                found,
            )

    def test_the_catch_is_banged_by_the_frame_gate_alone(self) -> None:
        stream = both(added("obj-clock", "qmetro 10"), line("obj-clock", 0, "obj-catch", 0))
        found = errors(edited(stream))
        self.assertTrue(any("must take plugin~ and the frame gate" in e for e in found), found)

        found = errors(edited(line("obj-poll-route", 1, "obj-catch", 0)))
        self.assertTrue(any("must take plugin~ and the frame gate" in e for e in found), found)

        def unfed(p: dict) -> None:
            p["lines"] = [
                ln for ln in p["lines"] if ln["patchline"]["destination"] != ["obj-catch", 1]
            ]

        found = errors(edited(unfed))
        self.assertTrue(any("must take plugin~ and the frame gate" in e for e in found), found)

    def test_the_frame_gate_is_opened_by_the_select_and_fed_by_the_frame_poll(self) -> None:
        found = errors(edited(line("obj-thisdevice", 0, "obj-frame-gate", 0)))
        self.assertTrue(
            any("frame gate is switched only by a '1' or '0' message" in e for e in found), found
        )
        found = errors(edited(line("obj-poll-route", 0, "obj-frame-gate", 1)))
        self.assertIn(
            "p: obj-frame-gate:1 takes [['obj-poll-route', 1]] alone, "
            "has [['obj-poll-route', 0], ['obj-poll-route', 1]]",
            found,
        )

    def test_the_audio_goes_to_the_jit_net_send_alone(self) -> None:
        found = errors(edited(line("obj-catch", 0, "obj-udpsend", 0)))
        self.assertTrue(any("to the jit.net.send alone" in e for e in found), found)
        self.assertTrue(any("takes the announce alone" in e for e in found), found)

    def test_the_hello_alone_goes_into_the_right_inlet(self) -> None:
        found = errors(edited(line("obj-announce", 0, "obj-net", 1)))
        self.assertTrue(any("takes the hello alone" in e for e in found), found)

    def test_a_loud_output_or_a_parameter_is_refused(self) -> None:
        found = errors(edited(line("obj-plugin", 0, "obj-plugout", 0)))
        self.assertTrue(any("plugout~ must take silence only" in e for e in found), found)

        def param(p: dict) -> None:
            box(p, "obj-sending")["parameter_enable"] = 1

        found = errors(edited(param))
        self.assertTrue(any("obj-sending is a parameter" in e for e in found), found)

    def test_the_select_is_never_asked_over_http_again(self) -> None:
        found = errors(edited(added("obj-url", "maxurl", 1, 2)))
        self.assertIn("p: a maxurl (the hub pushes the select in its poll)", found)

    def test_a_second_box_of_a_kind_is_named(self) -> None:
        found = errors(edited(added("obj-send2", "udpsend 127.0.0.1 39111", 1, 0)))
        self.assertIn("p: 2 udpsend boxes", found)

    def test_a_path_or_junk_is_refused(self) -> None:
        self.assertTrue(any("absolute file path" in e for e in errors(TEXT + " C:\\\\x")))
        self.assertTrue(any("not a Max patcher" in e for e in errors("{}")))

    def test_every_line_of_the_select_logic_is_held(self) -> None:
        for src, dst in cl.WIRING:

            def cut(p: dict, src=src, dst=dst) -> None:
                p["lines"] = [
                    ln
                    for ln in p["lines"]
                    if (ln["patchline"]["source"], ln["patchline"]["destination"])
                    != (list(src), list(dst))
                ]

            found = errors(edited(cut))
            want = f"p: the line {src[0]}:{src[1]} -> {dst[0]}:{dst[1]} is missing"
            self.assertIn(want, found)

    def test_only_an_on_poll_selects(self) -> None:
        def zero(p: dict) -> None:
            box(p, "obj-poll-sel")["text"] = "sel 0"

        found = errors(edited(zero))
        self.assertIn("p: obj-poll-sel must be 'sel 1', has ['sel', '0']", found)

    def test_the_poll_answers_through_its_gate_and_pull(self) -> None:
        # The meter's pull (#264): Device On gates the poll, the pull's int
        # (the poll's `on`; 0 for the load's and Device On's bang) goes to
        # the select first, then the re-read's count, the sample rate and
        # the announce.
        p = json.loads(TEXT)["patcher"]
        self.assertEqual(cl.lufs.words(box(p, "obj-pull")), ["t", "b", "b", "b", "i"])
        found = errors(edited(line("obj-poll-route", 0, "obj-pull", 0)))
        self.assertIn(
            "p: obj-pull:0 takes [['obj-pull-gate', 0]] alone, "
            "has [['obj-poll-route', 0], ['obj-pull-gate', 0]]",
            found,
        )
        found = errors(edited(line("obj-pull", 0, "obj-net", 0)))
        self.assertTrue(any("obj-pull outputs into" in e for e in found), found)

    def test_the_announce_names_the_poll_port(self) -> None:
        def unnamed(p: dict) -> None:
            p["lines"] = [
                ln for ln in p["lines"] if ln["patchline"]["destination"] != ["obj-announce", 10]
            ]

        found = errors(edited(unnamed))
        self.assertIn("p: obj-announce:10 takes [['obj-poll-port', 0]] alone, has []", found)

    def test_the_identity_is_read_again_on_every_40th_poll(self) -> None:
        # The other pulled devices' re-read (`poll_reread_check`): 40 polls
        # of 250 ms are 10 s.
        p = json.loads(TEXT)["patcher"]
        self.assertEqual(cl.lufs.words(box(p, "obj-id-count")), ["counter", "0", "39"])
        self.assertEqual(cl.lufs.words(box(p, "obj-id-offset")), ["random", "40"])

        def faster(p: dict) -> None:
            box(p, "obj-id-count")["text"] = "counter 0 3"

        found = errors(edited(faster))
        self.assertIn("p: obj-id-count must be 'counter 0 39', has ['counter', '0', '3']", found)

    def test_the_names_go_into_the_announce(self) -> None:
        def elsewhere(p: dict) -> None:
            p["lines"] = [
                ln for ln in p["lines"] if ln["patchline"]["destination"] != ["obj-announce", 5]
            ]

        found = errors(edited(elsewhere))
        self.assertIn("p: obj-announce:5 takes [['obj-parent2-symbol', 0]] alone, has []", found)

    def test_the_names_are_read_from_live_object_never_an_observer(self) -> None:
        # The FINDING of 2026-11-17 (#240 on the PC): an observer's output
        # kept a name with a space in quotes and read the set as `-`.
        found = errors(edited(line("obj-parent1-observer", 0, "obj-parent1-reg", 1)))
        self.assertIn(
            "p: the observer obj-parent1-observer reaches obj-announce:4 by its output "
            "(an observer only triggers the read)",
            found,
        )

    def test_the_session_random_is_drawn_before_the_device_id_is_read(self) -> None:
        # The review of #264's Listen step: the load's `t` draws the listen
        # id's random from its rightmost outlet, before its `path
        # this_device` reads the Max id into the `expr`'s hot inlet; a fan
        # out of `live.thisdevice` would leave the order to the boxes'
        # places (the PC's two Lives could then share an id).
        def fanned(p: dict) -> None:
            for ln in p["lines"]:
                if ln["patchline"]["destination"] == ["obj-session-random", 0]:
                    ln["patchline"]["source"] = ["obj-thisdevice", 0]

        found = errors(edited(fanned))
        self.assertIn(
            "p: obj-session-random:0 takes [['obj-id-load', 9]] alone, has [['obj-thisdevice', 0]]",
            found,
        )

    def test_only_the_listen_id_names_the_device(self) -> None:
        found = errors(edited(line("obj-id-route", 0, "obj-announce", 2)))
        self.assertTrue(any("obj-announce:2 takes the listen id alone" in e for e in found), found)

    def test_constants_read_with_underscores(self) -> None:
        self.assertEqual(cl.constant("pub const X: u32 = 48_000;\n", "X"), 48000)
        self.assertIsNone(cl.constant("pub const Y: u32 = 1;\n", "X"))
        self.assertEqual(cl.text_constant('pub const P: &str = "/a/b";\n', "P"), "/a/b")
        self.assertIsNone(cl.text_constant('pub const Q: &str = "/a";\n', "P"))

    def test_main_reports_and_exits(self) -> None:
        self.assertEqual(cl.main(["--root", str(ROOT)]), 0)


if __name__ == "__main__":
    unittest.main()
