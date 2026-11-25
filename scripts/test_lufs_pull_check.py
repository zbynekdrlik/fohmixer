"""Tests for scripts/lufs_pull_check.py (#264): the LUFS device's pull."""

from __future__ import annotations

import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_lufs_device as cl  # noqa: E402
import lufs_pull_check as lp  # noqa: E402
from test_check_lufs_device import REPO, TreeCase  # noqa: E402

LUFS, METER = cl.DEVICES


class PullTests(TreeCase):
    def test_the_lufs_device_is_pulled_by_the_hubs_poll(self) -> None:
        wire = (REPO / cl.WIRE).read_text(encoding="utf-8")
        poll = lp.poll_constants(wire)
        self.assertEqual(poll, lp.Poll(40000, 10000, "/fohmixer/lufs/poll"))
        patch = self.patcher(LUFS)
        ids = {b.get("id"): cl.words(b) for b in cl.boxes(patch["patcher"])}
        self.assertEqual(ids["obj-snap-s"], ["snapshot~", "0"])
        self.assertEqual(ids["obj-snap-v"], ["snapshot~", "0"])
        self.assertEqual(ids["obj-pack"][:3], ["pack", "/fohmixer/lufs", "2"])
        self.assertEqual(len(ids["obj-pack"]), cl.PACK_ARGS + 2)
        self.assertEqual(ids[lp.POLL_ROUTE], ["route", "/fohmixer/lufs/poll"])
        self.assertEqual(ids[lp.POLL_DRAW], ["random", "10000"])
        self.assertEqual(ids[lp.POLL_OFFSET], ["+", "40000"])
        self.assertNotIn("obj-send-defer", ids)
        self.assertNotIn(["deferlow"], [w for i, w in ids.items() if i.startswith("obj-pull")])
        self.assertEqual(cl.feeds(patch["patcher"], "obj-udpsend", 0), [["obj-pack", 0]])
        self.assertEqual(
            cl.feeds(patch["patcher"], cl.PULL_GATE, 1),
            sorted([[lp.POLL_ROUTE, 0], [cl.ID_LOAD, 0], [cl.ID_TICK, 0], [lp.ACTIVE_SEL, 0]]),
        )
        # The meter keeps its audio clock and its deferlow (a later step).
        meter = {b.get("id"): cl.words(b) for b in cl.boxes(self.patcher(METER)["patcher"])}
        self.assertEqual(meter["obj-send-defer"], ["deferlow"])
        self.assertEqual(meter["obj-pack"][2], "1")

    def test_a_deferlow_on_the_lufs_send_path_fails(self) -> None:
        where = LUFS.maxpat.as_posix()
        # Between the pack and the udpsend, as before #264.
        patch = self.patcher(LUFS)
        self.drop_lines(patch, dst=["obj-udpsend", 0])
        self.node(patch, "obj-x-defer", "deferlow", inlets=1)
        self.line(patch, ["obj-pack", 0], ["obj-x-defer", 0])
        self.line(patch, ["obj-x-defer", 0], ["obj-udpsend", 0])
        self.save(patch, LUFS)
        errors = self.errors()
        self.assertIn(
            f"{where}: the udpsend must be fed by the pack alone (no deferlow), "
            "has [['obj-x-defer', 0]]",
            errors,
        )
        self.assertIn(
            f"{where}: obj-x-defer ('deferlow') is on the send path (the pull runs in the "
            "main thread: no deferlow)",
            errors,
        )
        # Higher up: between the pull and a reading.
        patch = self.patcher(LUFS)
        self.drop_lines(patch, src=[lp.PULL, 0])
        self.node(patch, "obj-x-defer", "defer", inlets=1)
        self.line(patch, [lp.PULL, 0], ["obj-x-defer", 0])
        self.line(patch, ["obj-x-defer", 0], ["obj-snap-v", 0])
        self.save(patch, LUFS)
        errors = self.errors()
        self.assertIn(
            f"{where}: obj-x-defer ('defer') is on the send path (the pull runs in the "
            "main thread: no deferlow)",
            errors,
        )
        self.restore(LUFS)
        self.assertEqual(self.errors(), [])

    def test_a_lufs_reader_with_its_own_clock_fails(self) -> None:
        where = LUFS.maxpat.as_posix()
        self.edit(LUFS.maxpat, '"snapshot~ 0"', '"snapshot~ 200"')
        errors = self.errors()
        self.assertIn(
            f"{where}: obj-snap-s ('snapshot~ 200') runs its own clock (the device is pulled: "
            "its readers are 'snapshot~ 0')",
            errors,
        )
        self.assertIn(
            f"{where}: the face's obj-short is fed from obj-snap-s ('snapshot~ 200'), an audio "
            "clock (a redraw from the audio thread posts too)",
            errors,
        )
        self.restore(LUFS)
        patch = self.patcher(LUFS)
        self.node(patch, "obj-x-peak", "peakamp~ 33")
        self.save(patch, LUFS)
        self.assertIn(
            f"{where}: obj-x-peak ('peakamp~ 33') runs its own clock (the device is pulled: "
            "its readers are 'snapshot~ 0')",
            self.errors(),
        )
        self.restore(LUFS)
        self.assertEqual(self.errors(), [])

    def test_a_metro_or_qmetro_driving_a_lufs_reading_fails(self) -> None:
        # Probe A of 25.11: a qmetro pull posts from metro.mxe64 on the
        # audio thread.
        where = LUFS.maxpat.as_posix()
        for text, dst in (
            ("qmetro 200", [cl.PULL_GATE, 1]),
            ("metro 200", ["obj-snap-s", 0]),
            ("delay 200", [lp.PULL, 0]),
        ):
            patch = self.patcher(LUFS)
            self.node(patch, "obj-x-clock", text)
            self.line(patch, ["obj-x-clock", 0], dst)
            self.save(patch, LUFS)
            errors = self.errors()
            self.assertIn(
                f"{where}: obj-x-clock ('{text}') drives the reading obj-snap-s (only the "
                "hub's poll, the load and the re-read do)",
                errors,
            )
            self.assertIn(
                f"{where}: the face's obj-short is fed from obj-x-clock ('{text}'), an audio "
                "clock (a redraw from the audio thread posts too)",
                errors,
            )
        self.restore(LUFS)
        self.assertEqual(self.errors(), [])

    def test_a_lufs_face_object_fed_from_an_audio_clock_fails(self) -> None:
        # Probe C of 25.11: the number boxes fed from snapshot~ 200 posted.
        where = LUFS.maxpat.as_posix()
        patch = self.patcher(LUFS)
        self.drop_lines(patch, dst=["obj-voice", 0])
        self.node(patch, "obj-x-snap", "snapshot~ 200")
        self.line(patch, ["obj-gen", 3], ["obj-x-snap", 0])
        self.line(patch, ["obj-x-snap", 0], ["obj-voice", 0])
        self.save(patch, LUFS)
        errors = self.errors()
        self.assertIn(
            f"{where}: the face's obj-voice is fed from obj-x-snap ('snapshot~ 200'), an audio "
            "clock (a redraw from the audio thread posts too)",
            errors,
        )
        self.restore(LUFS)
        self.assertEqual(self.errors(), [])

    def test_the_pull_is_gated_by_device_on_and_opened_only_by_its_four_sources(self) -> None:
        where = LUFS.maxpat.as_posix()
        # Without the observer's value the gate never opens (and a gate
        # held open would answer for a device switched off).
        patch = self.patcher(LUFS)
        self.drop_lines(patch, dst=[cl.PULL_GATE, 0])
        self.save(patch, LUFS)
        errors = self.errors()
        self.assertIn(
            f"{where}: {cl.PULL_GATE}:0 takes [['{lp.ACTIVE_T}', 1]] alone, has []", errors
        )
        # A poll that goes around the gate.
        self.restore(LUFS)
        patch = self.patcher(LUFS)
        self.line(patch, [lp.POLL_ROUTE, 0], [lp.PULL, 0])
        self.save(patch, LUFS)
        errors = self.errors()
        self.assertIn(
            f"{where}: {lp.PULL}:0 takes [['{cl.PULL_GATE}', 0]] alone, "
            f"has [['{lp.POLL_ROUTE}', 0], ['{cl.PULL_GATE}', 0]]",
            errors,
        )
        # The 10 s tick no longer sends.
        self.restore(LUFS)
        patch = self.patcher(LUFS)
        self.drop_lines(patch, src=[cl.ID_TICK, 0])
        self.save(patch, LUFS)
        self.assertTrue(
            any(f"{cl.ID_TICK} outputs into" in e for e in self.errors()), self.errors()
        )
        # Device On observed through route id (probe B2's id never came).
        self.restore(LUFS)
        patch = self.patcher(LUFS)
        self.drop_lines(patch, dst=[cl.ACTIVE_OBSERVER, 1])
        self.line(patch, ["obj-id-route", 0], [cl.ACTIVE_OBSERVER, 1])
        self.save(patch, LUFS)
        self.assertIn(
            f"{where}: {cl.ACTIVE_OBSERVER}:1 takes [['{lp.ACTIVE_LIVE_PATH}', 0]] alone, "
            "has [['obj-id-route', 0]]",
            self.errors(),
        )
        # Another property observed.
        self.restore(LUFS)
        self.edit(LUFS.maxpat, '"property is_active"', '"property name"')
        self.assertIn(
            f"{where}: {lp.ACTIVE_PROPERTY} must be 'property is_active', has ['property', 'name']",
            self.errors(),
        )
        self.restore(LUFS)
        self.assertEqual(self.errors(), [])

    def test_the_poll_port_is_drawn_from_the_wire_modules_range(self) -> None:
        where = LUFS.maxpat.as_posix()
        self.edit(
            cl.WIRE,
            "pub const POLL_PORT_MIN: u16 = 40_000;",
            "pub const POLL_PORT_MIN: u16 = 41_000;",
        )
        errors = self.errors()
        self.assertIn(f"{where}: {lp.POLL_OFFSET} must be '+ 41000', has ['+', '40000']", errors)
        self.assertIn(
            f"{where}: {lp.POLL_RECEIVE} must be 'udpreceive 40999 @defer 1', has "
            "['udpreceive', '39999', '@defer', '1']",
            errors,
        )
        shutil.copyfile(REPO / cl.WIRE, self.root / cl.WIRE)
        # #264: the poll leaves the udpreceive on the low-priority queue
        # (`@defer 1`), never on the scheduler (Live's audio thread).
        for text in ("udpreceive 39999", "udpreceive 39999 @defer 0"):
            self.restore(LUFS)
            patch = self.patcher(LUFS)
            next(b for b in cl.boxes(patch["patcher"]) if b.get("id") == lp.POLL_RECEIVE)[
                "text"
            ] = text
            self.save(patch, LUFS)
            self.assertEqual(
                self.errors(),
                [
                    f"{where}: {lp.POLL_RECEIVE} must be 'udpreceive 39999 @defer 1', has "
                    f"{text.split()}"
                ],
            )
        self.restore(LUFS)
        self.edit(
            cl.WIRE,
            "pub const POLL_PORT_MIN: u16 = 40_000;",
            "pub const POLL_PORT_MIN: u16 = 41_000;",
        )
        self.edit(cl.WIRE, "pub const POLL_PORT_MIN: u16 = 41_000;", "pub const POLL_MIN: u16 = 1;")
        self.assertEqual(
            self.errors(),
            [f"{cl.WIRE.as_posix()}: no POLL_PORT_MIN, POLL_PORT_SPAN or POLL_ADDRESS line"],
        )
        shutil.copyfile(REPO / cl.WIRE, self.root / cl.WIRE)
        # The port's slot fed by anything but the draw, a second udpreceive.
        patch = self.patcher(LUFS)
        self.drop_lines(patch, dst=["obj-pack", cl.PACK_ARGS])
        self.node(patch, "obj-x-receive", "udpreceive 40001", inlets=1)
        self.save(patch, LUFS)
        errors = self.errors()
        self.assertIn(
            f"{where}: obj-pack:{cl.PACK_ARGS} takes [['{lp.POLL_PORT}', 0]] alone, has []",
            errors,
        )
        self.assertIn(
            f"{where}: needs exactly one udpreceive, {lp.POLL_RECEIVE}, has "
            f"['{lp.POLL_RECEIVE}', 'obj-x-receive']",
            errors,
        )
        self.restore(LUFS)
        self.assertEqual(self.errors(), [])

    def test_each_device_sends_its_own_wire_version(self) -> None:
        self.edit(LUFS.maxpat, '"pack /fohmixer/lufs 2 ', '"pack /fohmixer/lufs 1 ')
        self.assertIn(
            f"{LUFS.maxpat.as_posix()}: the pack sends wire version 1, expected 2", self.errors()
        )
        self.restore(LUFS)
        self.edit(METER.maxpat, '"pack /fohmixer/meter 1 ', '"pack /fohmixer/meter 2 ')
        self.assertIn(
            f"{METER.maxpat.as_posix()}: the pack sends wire version 2, expected 1", self.errors()
        )
        self.restore(METER)
        # The LUFS pack has its port slot, the meter's none.
        self.edit(LUFS.maxpat, ' -700 -700 0"', ' -700 -700"')
        self.assertTrue(
            any("needs exactly one pack of 11 arguments" in e for e in self.errors()),
            self.errors(),
        )
        self.restore(LUFS)
        self.assertEqual(self.errors(), [])

    def test_a_reader_with_its_clock_off_is_no_timed_box(self) -> None:
        self.assertTrue(cl.clock_off({"maxclass": "newobj", "text": "snapshot~ 0"}))
        self.assertTrue(cl.clock_off({"maxclass": "newobj", "text": "peakamp~ 0"}))
        self.assertFalse(cl.clock_off({"maxclass": "newobj", "text": "snapshot~ 200"}))
        self.assertFalse(cl.clock_off({"maxclass": "newobj", "text": "snapshot~"}))
        self.assertFalse(cl.clock_off({"maxclass": "newobj", "text": "metro 0"}))

        def box_(i: str, text: str, ins: int = 1) -> dict:
            return {"id": i, "maxclass": "newobj", "text": text, "numinlets": ins}

        def line(src: str, dst: str) -> dict:
            return {"patchline": {"source": [src, 0], "destination": [dst, 0]}}

        def run(boxes: list[dict], lines: list[dict]) -> list[str]:
            patcher = {"boxes": [{"box": b} for b in boxes], "lines": lines}
            return cl.thread_errors("p", patcher, cl.boxes(patcher))

        net = box_("n", "udpsend 127.0.0.1 1")
        snap = box_("s", "snapshot~ 0", 2)
        self.assertEqual(run([net, snap], [line("s", "n")]), [])
        # A clock that bangs it is still found through it.
        clock = box_("m", "metro 100", 2)
        self.assertEqual(len(run([net, snap, clock], [line("m", "s"), line("s", "n")])), 1)
