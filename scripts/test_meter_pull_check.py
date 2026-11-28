"""Tests for scripts/meter_pull_check.py (#264): the FohMixer Meter's pull."""

from __future__ import annotations

import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_lufs_device as cl  # noqa: E402
import lufs_pull_check as lp  # noqa: E402
import poll_reread_check as rr  # noqa: E402
from test_check_lufs_device import REPO, TreeCase  # noqa: E402

LUFS, METER = cl.DEVICES
WHERE = METER.maxpat.as_posix()


class MeterPullTests(TreeCase):
    def ids(self) -> dict[str, list[str]]:
        return {b.get("id"): cl.words(b) for b in cl.boxes(self.patcher(METER)["patcher"])}

    def test_the_meter_is_pulled_by_the_hubs_poll(self) -> None:
        # The design of 26.11 on #264: the hub polls the meter every 33 ms;
        # it answers on `udpreceive @defer 1` (Max's main thread), bangs
        # its interval-less `peakamp~` pair and sends in that same call.
        ids = self.ids()
        patcher = self.patcher(METER)["patcher"]
        self.assertEqual(ids["obj-peak-l"], ["peakamp~"])
        self.assertEqual(ids["obj-peak-r"], ["peakamp~"])
        self.assertEqual(ids["obj-pack"][:3], ["pack", "/fohmixer/meter", "2"])
        self.assertEqual(len(ids["obj-pack"]), cl.PACK_ARGS + 2)
        self.assertEqual(ids[lp.POLL_RECEIVE], ["udpreceive", "39999", "@defer", "1"])
        self.assertEqual(
            ids[lp.POLL_ROUTE], ["route", "/fohmixer/meter/poll", "/fohmixer/meter/move"]
        )
        self.assertEqual(cl.feeds(patcher, "obj-udpsend", 0), [["obj-pack", 0]])
        self.assertEqual(
            cl.feeds(patcher, cl.PULL_GATE, 1),
            sorted([[lp.POLL_ROUTE, 0], [cl.ID_LOAD, 0], [lp.ACTIVE_SEL, 0], [lp.POLL_MOVE, 0]]),
        )
        self.assertEqual(
            lp.outputs(patcher, lp.PULL),
            [[0, "obj-peak-r", 0], [1, "obj-peak-l", 0], [2, rr.ID_COUNT, 0]],
        )
        # The faces show the clipped dBFS of the poll's own reading.
        self.assertEqual(cl.feeds(patcher, "obj-left", 0), [["obj-show-l", 0]])
        self.assertEqual(cl.feeds(patcher, "obj-right", 0), [["obj-show-r", 0]])
        for gone in ("obj-send-defer", "obj-id-defer", "obj-id-clock", "obj-id-clock-defer"):
            self.assertNotIn(gone, ids)
        self.assertEqual(self.errors(), [])

    def test_a_peakamp_with_an_interval_fails(self) -> None:
        # The old `peakamp~ 33` ran a clock on Max's scheduler, inside
        # Live's audio thread: each output and the faces' redraw posted.
        patch = self.patcher(METER)
        for b in cl.boxes(patch["patcher"]):
            if b.get("text") == "peakamp~":
                b["text"] = "peakamp~ 33"
        self.save(patch, METER)
        errors = self.errors()
        self.assertIn(
            f"{WHERE}: obj-peak-l ('peakamp~ 33') runs its own clock (the device is pulled: "
            "its readers are 'peakamp~')",
            errors,
        )
        self.assertIn(
            f"{WHERE}: the face's obj-left is fed from obj-peak-l ('peakamp~ 33'), an audio "
            "clock (a redraw from the audio thread posts too)",
            errors,
        )
        # R's value bangs the pack (L only fills its cold slot).
        self.assertIn(
            f"{WHERE}: obj-peak-r ('peakamp~ 33') reaches the udpsend obj-udpsend on the "
            "audio thread (a deferlow or a qmetro goes between)",
            errors,
        )
        self.restore(METER)
        # A reader's interval set from elsewhere.
        patch = self.patcher(METER)
        self.line(patch, ["obj-thisdevice", 1], ["obj-peak-r", 1])
        self.save(patch, METER)
        self.assertIn(
            f"{WHERE}: obj-peak-r:1 takes [] alone, has [['obj-thisdevice', 1]]", self.errors()
        )
        self.restore(METER)
        self.assertEqual(self.errors(), [])

    def test_the_meters_poll_follows_the_wire_module(self) -> None:
        self.edit(
            cl.WIRE,
            'pub const METER_POLL_ADDRESS: &str = "/fohmixer/meter/poll";',
            'pub const METER_POLL_ADDRESS: &str = "/fohmixer/meter/ask";',
        )
        self.assertEqual(
            self.errors(),
            [
                f"{WHERE}: {lp.POLL_ROUTE} must be 'route /fohmixer/meter/ask "
                "/fohmixer/meter/move', has ['route', '/fohmixer/meter/poll', "
                "'/fohmixer/meter/move']"
            ],
        )
        shutil.copyfile(REPO / cl.WIRE, self.root / cl.WIRE)
        for text in ("udpreceive 39999", "udpreceive 39999 @defer 0"):
            patch = self.patcher(METER)
            next(b for b in cl.boxes(patch["patcher"]) if b.get("id") == lp.POLL_RECEIVE)[
                "text"
            ] = text
            self.save(patch, METER)
            self.assertEqual(
                self.errors(),
                [
                    f"{WHERE}: {lp.POLL_RECEIVE} must be 'udpreceive 39999 @defer 1', has "
                    f"{text.split()}"
                ],
            )
            self.restore(METER)
        self.assertEqual(self.errors(), [])
