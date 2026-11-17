"""Tests for check_listen_device.py (#210): the real patch passes, each
broken rule is named."""

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
    "hold": 1000,
    "poll": 250,
    "hub": 8480,
    "path": "/api/listen/select",
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


class ListenDeviceTests(unittest.TestCase):
    def test_the_real_patch_passes(self) -> None:
        self.assertEqual(cl.check(ROOT), [])
        self.assertEqual(cl.numbers_of(ROOT), NUMBERS)

    def test_the_ports_frame_hold_and_poll_must_match_the_hubs(self) -> None:
        for name, word in (
            ("port", "jit.net.send"),
            ("frame", "jit.catch~"),
            ("hold", "delay"),
            ("poll", "metro"),
            ("hub", "sprintf"),
            ("path", "sprintf"),
        ):
            found = errors(TEXT, numbers={**NUMBERS, name: 1})
            self.assertTrue(any(word in e for e in found), (name, found))
        found = errors(TEXT, numbers={**NUMBERS, "port": 1})
        self.assertTrue(any("udpsend 127.0.0.1 1" in e for e in found), found)

    def test_the_version_must_match(self) -> None:
        found = errors(TEXT, version="9.9.9")
        self.assertTrue(any("fohmixer_listen" in e for e in found), found)
        self.assertTrue(any("/fohmixer/listen" in e for e in found), found)
        self.assertTrue(any("its face shows" in e for e in found), found)

    def test_a_qmetro_started_by_the_device_itself_is_refused(self) -> None:
        def loadbang(p: dict) -> None:
            p["boxes"].append(
                {
                    "box": {
                        "id": "obj-lb",
                        "maxclass": "newobj",
                        "text": "loadbang",
                        "numinlets": 1,
                        "numoutlets": 1,
                        "outlettype": ["bang"],
                    }
                }
            )
            p["lines"].append(
                {"patchline": {"source": ["obj-lb", 0], "destination": ["obj-qmetro", 0]}}
            )

        found = errors(edited(loadbang))
        self.assertTrue(any("switched only by" in e and "obj-lb" in e for e in found), found)

    def test_the_audio_goes_to_the_jit_net_send_alone(self) -> None:
        def tap(p: dict) -> None:
            p["lines"].append(
                {"patchline": {"source": ["obj-catch", 0], "destination": ["obj-udpsend", 0]}}
            )

        found = errors(edited(tap))
        self.assertTrue(any("to the jit.net.send alone" in e for e in found), found)
        self.assertTrue(any("takes the announce alone" in e for e in found), found)

        def unfed(p: dict) -> None:
            p["lines"] = [
                ln for ln in p["lines"] if ln["patchline"]["destination"] != ["obj-catch", 1]
            ]

        found = errors(edited(unfed))
        self.assertTrue(any("must take plugin~ and the qmetro" in e for e in found), found)

    def test_the_hello_alone_goes_into_the_right_inlet(self) -> None:
        def stray(p: dict) -> None:
            p["lines"].append(
                {"patchline": {"source": ["obj-announce", 0], "destination": ["obj-net", 1]}}
            )

        found = errors(edited(stray))
        self.assertTrue(any("takes the hello alone" in e for e in found), found)

    def test_a_loud_output_or_a_parameter_is_refused(self) -> None:
        def loud(p: dict) -> None:
            p["lines"].append(
                {"patchline": {"source": ["obj-plugin", 0], "destination": ["obj-plugout", 0]}}
            )

        found = errors(edited(loud))
        self.assertTrue(any("plugout~ must take silence only" in e for e in found), found)

        def param(p: dict) -> None:
            box(p, "obj-sending")["parameter_enable"] = 1

        found = errors(edited(param))
        self.assertTrue(any("obj-sending is a parameter" in e for e in found), found)

    def test_a_second_box_of_a_kind_is_named(self) -> None:
        def twice(p: dict) -> None:
            p["boxes"].append(
                {
                    "box": {
                        "id": "obj-q2",
                        "maxclass": "newobj",
                        "text": "qmetro 20",
                        "numinlets": 2,
                        "numoutlets": 1,
                        "outlettype": ["bang"],
                    }
                }
            )

        found = errors(edited(twice))
        self.assertIn("p: 3 qmetro boxes (the stream's and the poll's)", found)

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

    def test_only_an_on_answer_selects(self) -> None:
        def zero(p: dict) -> None:
            box(p, "obj-poll-sel")["text"] = "sel 0"

        found = errors(edited(zero))
        self.assertIn("p: obj-poll-sel must be 'sel 1', has ['sel', '0']", found)

    def test_the_select_is_never_a_udp_port_again(self) -> None:
        def udp(p: dict) -> None:
            p["boxes"].append(
                {
                    "box": {
                        "id": "obj-u",
                        "maxclass": "newobj",
                        "text": "udpreceive 39112",
                        "numinlets": 1,
                        "numoutlets": 1,
                        "outlettype": [""],
                    }
                }
            )

        found = errors(edited(udp))
        self.assertIn("p: a udpreceive (the select is asked over HTTP)", found)

    def test_the_poll_boxes_are_the_ones_wired(self) -> None:
        def swapped(p: dict) -> None:
            box(p, "obj-poll-qmetro")["text"] = "delay 1000"
            box(p, "obj-select-hold")["text"] = "qmetro 250"

        found = errors(edited(swapped))
        self.assertIn("p: obj-poll-qmetro must be 'qmetro 250'", found)
        self.assertIn("p: obj-select-hold must be 'delay 1000'", found)

    def test_the_poll_and_the_hold_stay_off_the_audio_thread(self) -> None:
        # #240: a `metro` runs on Max for Live's scheduler, inside Live's
        # audio thread; the poll's request and the hold's end leave it.
        def metro(p: dict) -> None:
            box(p, "obj-poll-qmetro")["text"] = "metro 250"

        found = errors(edited(metro))
        self.assertIn(
            "p: obj-poll-qmetro ('metro 250') reaches the maxurl obj-poll-maxurl on the "
            "audio thread (a deferlow or a qmetro goes between)",
            found,
        )

        def undeferred(p: dict) -> None:
            p["lines"].append(
                {
                    "patchline": {
                        "source": ["obj-select-hold", 0],
                        "destination": ["obj-select-off", 0],
                    }
                }
            )

        found = errors(edited(undeferred))
        self.assertIn(
            "p: obj-select-hold ('delay 1000') reaches the jit.net.send obj-net on the "
            "audio thread (a deferlow or a qmetro goes between)",
            found,
        )

    def test_a_timed_value_in_a_cold_inlet_is_no_send(self) -> None:
        # The hold's state into the announce's last (cold) inlet is sent by
        # the deferred announce, never by the delay.
        def cold(p: dict) -> None:
            p["lines"].append(
                {
                    "patchline": {
                        "source": ["obj-select-hold", 0],
                        "destination": ["obj-announce", 9],
                    }
                }
            )

        self.assertFalse(any("audio thread" in e for e in errors(edited(cold))))

        def gate(p: dict) -> None:
            p["lines"].append(
                {
                    "patchline": {
                        "source": ["obj-select-hold", 0],
                        "destination": ["obj-hello-gate", 1],
                    }
                }
            )

        self.assertTrue(any("audio thread" in e for e in errors(edited(gate))))

    def test_the_announce_sends_the_stored_identity_each_second(self) -> None:
        # #240: the tick reads nothing from Live, the names come at load and
        # on a change.
        def lom_tick(p: dict) -> None:
            p["lines"].append(
                {
                    "patchline": {
                        "source": ["obj-announce-t", 0],
                        "destination": ["obj-parent1-path", 0],
                    }
                }
            )

        found = errors(edited(lom_tick))
        self.assertIn(
            "p: obj-announce-metro ('metro 1000') drives the live.path obj-parent1-live-path "
            "(the identity is read at load and on a change only)",
            found,
        )

        def raw_tick(p: dict) -> None:
            p["lines"].append(
                {
                    "patchline": {
                        "source": ["obj-announce-metro", 0],
                        "destination": ["obj-announce", 0],
                    }
                }
            )

        found = errors(edited(raw_tick))
        self.assertIn(
            "p: obj-announce:0 takes [['obj-announce-t', 0]] alone, "
            "has [['obj-announce-metro', 0], ['obj-announce-t', 0]]",
            found,
        )
        self.assertTrue(any("reaches the udpsend" in e for e in found), found)

    def test_the_names_go_into_the_announce(self) -> None:
        def elsewhere(p: dict) -> None:
            p["lines"] = [
                ln for ln in p["lines"] if ln["patchline"]["destination"] != ["obj-announce", 5]
            ]

        found = errors(edited(elsewhere))
        self.assertIn("p: obj-announce:5 takes [['obj-parent2-symbol', 0]] alone, has []", found)

    def test_only_the_listen_id_names_the_device(self) -> None:
        def max_id(p: dict) -> None:
            p["lines"].append(
                {"patchline": {"source": ["obj-id-route", 0], "destination": ["obj-announce", 2]}}
            )

        found = errors(edited(max_id))
        self.assertTrue(any("obj-announce:2 takes the listen id alone" in e for e in found), found)

    def test_constants_read_with_underscores(self) -> None:
        self.assertEqual(cl.constant("pub const X: u32 = 48_000;\n", "X"), 48000)
        self.assertIsNone(cl.constant("pub const Y: u32 = 1;\n", "X"))
        self.assertEqual(cl.text_constant('pub const P: &str = "/a/b";\n', "P"), "/a/b")
        self.assertIsNone(cl.text_constant('pub const Q: &str = "/a";\n', "P"))
        self.assertEqual(
            cl.poll_url(8480, "/api/listen/select"),
            "http://127.0.0.1:8480/api/listen/select?device=%ld",
        )

    def test_main_reports_and_exits(self) -> None:
        self.assertEqual(cl.main(["--root", str(ROOT)]), 0)


if __name__ == "__main__":
    unittest.main()
