"""Tests for scripts/poll_reread_check.py (#264): both pulled devices
re-read their identity on the poll path and hold no clock and no hand-over."""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_lufs_device as cl  # noqa: E402
import lufs_pull_check as lp  # noqa: E402
import poll_reread_check as rr  # noqa: E402
from test_check_lufs_device import TreeCase  # noqa: E402

LUFS, METER = cl.DEVICES
STILL_WORDS = (
    "has no place in the device (it answers the hub's poll in Max's main thread: "
    "no deferlow, no metro, no qmetro, no delay)"
)


class RereadTests(TreeCase):
    def ids(self, device: cl.Device) -> dict[str, list[str]]:
        return {b.get("id"): cl.words(b) for b in cl.boxes(self.patcher(device)["patcher"])}

    def test_each_device_rereads_every_nth_poll(self) -> None:
        # The LUFS device is polled every 200 ms, so its 10 s re-read is every
        # 50th poll; the meter is polled every 33 ms, every 300th (9.9 s).
        self.assertEqual([d.reread for d in cl.DEVICES], [cl.Reread(50, 3), cl.Reread(300, 2)])
        for device in cl.DEVICES:
            polls, outlet = device.reread.polls, device.reread.outlet
            ids = self.ids(device)
            self.assertEqual(ids[cl.ID_OFFSET], ["random", str(polls)], device.name)
            self.assertEqual(ids[rr.ID_OFFSET_SET], ["prepend", "set"])
            self.assertEqual(ids[rr.ID_COUNT], ["counter", "0", str(polls - 1)])
            self.assertEqual(ids[rr.ID_EVERY], ["sel", "0"])
            self.assertEqual(ids[cl.ID_TICK], ["t", "b", "b", "b", "b"])
            self.assertEqual(ids[lp.PULL], ["t", *["b"] * (outlet + 1)])
            patcher = self.patcher(device)["patcher"]
            # The pull counts first (its rightmost outlet), so the re-read's
            # poll already sends the new parents.
            self.assertIn([outlet, rr.ID_COUNT, 0], lp.outputs(patcher, lp.PULL))
            self.assertEqual(
                cl.feeds(patcher, rr.ID_COUNT, 0),
                sorted([[rr.ID_OFFSET_SET, 0], [lp.PULL, outlet]]),
            )
            for level in range(1, cl.PARENTS + 1):
                self.assertEqual(
                    cl.feeds(patcher, f"obj-parent{level}-path", 0),
                    sorted(
                        [
                            [cl.ID_LOAD, cl.LOAD_EXTRA + cl.PARENTS - level],
                            [cl.ID_TICK, cl.PARENTS - level],
                        ]
                    ),
                )
            # The tick no longer opens the pull: it rides one.
            self.assertEqual(
                cl.feeds(patcher, cl.PULL_GATE, 1),
                sorted([[lp.POLL_ROUTE, 0], [cl.ID_LOAD, 0], [lp.ACTIVE_SEL, 0]]),
            )
            for gone in rr.CLOCK_BOXES:
                self.assertNotIn(gone, ids, device.name)
        self.assertEqual(self.errors(), [])

    def test_a_wrong_reread_line_or_box_fails(self) -> None:
        for device in cl.DEVICES:
            where = device.maxpat.as_posix()
            polls, outlet = device.reread.polls, device.reread.outlet
            parents = sorted([[4 - level, f"obj-parent{level}-path", 0] for level in range(1, 5)])
            cases = (
                # The counter banged by something but the pull.
                (
                    [["obj-thisdevice", 0], [rr.ID_COUNT, 0]],
                    f"{rr.ID_COUNT}:0 takes [['{rr.ID_OFFSET_SET}', 0], ['{lp.PULL}', {outlet}]] "
                    f"alone, has [['{rr.ID_OFFSET_SET}', 0], ['{lp.PULL}', {outlet}], "
                    "['obj-thisdevice', 0]]",
                ),
                # The tick banged by the load too.
                (
                    [[cl.ID_LOAD, 0], [cl.ID_TICK, 0]],
                    f"{cl.ID_TICK}:0 takes [['{rr.ID_EVERY}', 0]] alone, "
                    f"has [['{rr.ID_EVERY}', 0], ['{cl.ID_LOAD}', 0]]",
                ),
                # The counter's range set from elsewhere.
                (
                    [["obj-thisdevice", 1], [rr.ID_COUNT, 3]],
                    f"{rr.ID_COUNT}:3 takes [] alone, has [['obj-thisdevice', 1]]",
                ),
                # A LOM read fed from the tick.
                (
                    [[cl.ID_TICK, 0], ["obj-parent1-get", 0]],
                    f"{cl.ID_TICK} outputs into {parents} alone, has "
                    + str(sorted([*parents, [0, "obj-parent1-get", 0]])),
                ),
                # The tick opening the pull again.
                (
                    [[cl.ID_TICK, 0], [cl.PULL_GATE, 1]],
                    f"{cl.ID_TICK} outputs into {parents} alone, has "
                    + str(sorted([*parents, [0, cl.PULL_GATE, 1]])),
                ),
            )
            for (src, dst), want in cases:
                patch = self.patcher(device)
                self.line(patch, src, dst)
                self.save(patch, device)
                self.assertIn(f"{where}: {want}", self.errors())
                self.restore(device)
            for box, text, want in (
                (rr.ID_COUNT, f"counter 0 {polls}", f"counter 0 {polls - 1}"),
                (cl.ID_OFFSET, "random 10000", f"random {polls}"),
                (rr.ID_EVERY, "sel 1", "sel 0"),
            ):
                patch = self.patcher(device)
                next(b for b in cl.boxes(patch["patcher"]) if b.get("id") == box)["text"] = text
                self.save(patch, device)
                self.assertIn(f"{where}: {box} must be '{want}', has {text.split()}", self.errors())
                self.restore(device)
        self.assertEqual(self.errors(), [])

    def test_no_device_holds_a_deferlow_a_clock_or_a_delay(self) -> None:
        # Each is a post from Live's audio thread or a clock there (#264).
        for device in cl.DEVICES:
            where = device.maxpat.as_posix()
            for text in ("deferlow", "defer", "metro 33", "qmetro 10000", "delay 100", "pipe 5"):
                patch = self.patcher(device)
                self.node(patch, "obj-x", text, inlets=1)
                self.save(patch, device)
                self.assertIn(f"{where}: obj-x ('{text}') {STILL_WORDS}", self.errors())
                self.restore(device)
        self.assertEqual(self.errors(), [])

    def test_the_lufs_devices_old_clock_fails(self) -> None:
        # The 10 s clock of #240 put back: its qmetro and deferlows are named.
        where = LUFS.maxpat.as_posix()
        patch = self.patcher(LUFS)
        self.node(patch, "obj-id-clock", "qmetro 10000")
        self.line(patch, [rr.ID_EVERY, 0], ["obj-id-clock", 0])
        self.save(patch, LUFS)
        errors = self.errors()
        self.assertIn(f"{where}: obj-id-clock ('qmetro 10000') {STILL_WORDS}", errors)
        self.restore(LUFS)
        self.assertEqual(self.errors(), [])

    def test_the_load_goes_straight_from_live_thisdevice(self) -> None:
        for device in cl.DEVICES:
            where = device.maxpat.as_posix()
            patcher = self.patcher(device)["patcher"]
            self.assertEqual(cl.feeds(patcher, cl.ID_LOAD, 0), [["obj-thisdevice", 0]])
            patch = self.patcher(device)
            self.drop_lines(patch, dst=[cl.ID_LOAD, 0])
            self.node(patch, "obj-x-defer", "deferlow", inlets=1)
            self.line(patch, ["obj-thisdevice", 0], ["obj-x-defer", 0])
            self.line(patch, ["obj-x-defer", 0], [cl.ID_LOAD, 0])
            self.save(patch, device)
            self.assertIn(
                f"{where}: {cl.ID_LOAD}:0 takes [['obj-thisdevice', 0]] alone, "
                "has [['obj-x-defer', 0]]",
                self.errors(),
            )
            self.restore(device)
        # The LUFS device's sample rate is read at the same load.
        patcher = self.patcher(LUFS)["patcher"]
        self.assertEqual(cl.feeds(patcher, "obj-sr-state", 0), [["obj-thisdevice", 0]])
        self.assertEqual(self.errors(), [])
