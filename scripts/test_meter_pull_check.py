"""Tests for scripts/meter_pull_check.py (#264): the FohMixer Meter's pull."""

from __future__ import annotations

import shutil
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import check_lufs_device as cl  # noqa: E402
import lufs_pull_check as lp  # noqa: E402
import meter_pull_check as mp  # noqa: E402
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
        self.assertEqual(ids[lp.POLL_ROUTE], ["route", "/fohmixer/meter/poll"])
        self.assertEqual(cl.feeds(patcher, "obj-udpsend", 0), [["obj-pack", 0]])
        self.assertEqual(
            cl.feeds(patcher, cl.PULL_GATE, 1),
            sorted([[lp.POLL_ROUTE, 0], [cl.ID_LOAD, 0], [lp.ACTIVE_SEL, 0]]),
        )
        self.assertEqual(
            lp.outputs(patcher, lp.PULL),
            [[0, "obj-peak-r", 0], [1, "obj-peak-l", 0], [2, mp.ID_COUNT, 0]],
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
        self.edit(METER.maxpat, '"peakamp~"', '"peakamp~ 33"')
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
        self.assertIn(
            f"{WHERE}: obj-peak-l ('peakamp~ 33') reaches the udpsend obj-udpsend on the "
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

    def test_the_meter_has_no_deferlow_metro_or_qmetro(self) -> None:
        # The design: each is a post from Live's audio thread or a clock.
        for text in ("deferlow", "defer", "metro 33", "qmetro 10000"):
            patch = self.patcher(METER)
            self.node(patch, "obj-x", text, inlets=1)
            self.save(patch, METER)
            self.assertIn(
                f"{WHERE}: obj-x ('{text}') has no place in the meter (it answers the hub's "
                "poll in Max's main thread: no deferlow, no metro, no qmetro)",
                self.errors(),
            )
            self.restore(METER)
        # The LUFS device keeps its identity's qmetro and its deferlows.
        self.assertEqual(self.errors(), [])

    def test_the_load_goes_straight_from_live_thisdevice(self) -> None:
        patcher = self.patcher(METER)["patcher"]
        self.assertEqual(cl.feeds(patcher, cl.ID_LOAD, 0), [["obj-thisdevice", 0]])
        patch = self.patcher(METER)
        self.drop_lines(patch, dst=[cl.ID_LOAD, 0])
        self.node(patch, "obj-x-defer", "deferlow", inlets=1)
        self.line(patch, ["obj-thisdevice", 0], ["obj-x-defer", 0])
        self.line(patch, ["obj-x-defer", 0], [cl.ID_LOAD, 0])
        self.save(patch, METER)
        errors = self.errors()
        self.assertIn(
            f"{WHERE}: {cl.ID_LOAD}:0 takes [['obj-thisdevice', 0]] alone, "
            "has [['obj-x-defer', 0]]",
            errors,
        )
        self.restore(METER)
        self.assertEqual(self.errors(), [])

    def test_the_identity_is_read_again_every_300th_poll(self) -> None:
        # The design: the 10 s re-read rides the poll (300 x 33 ms), from a
        # counter set to a random place at load, so devices loaded together
        # re-read apart; the re-read comes first in the pull, so that
        # poll's datagram already names the new parents.
        self.assertEqual(mp.REREAD_POLLS, 300)
        ids = self.ids()
        self.assertEqual(ids[cl.ID_OFFSET], ["random", "300"])
        self.assertEqual(ids[mp.ID_OFFSET_SET], ["prepend", "set"])
        self.assertEqual(ids[mp.ID_COUNT], ["counter", "0", "299"])
        self.assertEqual(ids[mp.ID_EVERY], ["sel", "0"])
        self.assertEqual(ids[cl.ID_TICK], ["t", "b", "b", "b", "b"])
        patcher = self.patcher(METER)["patcher"]
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
        cases = (
            # The counter banged by something but the pull.
            (
                [["obj-thisdevice", 0], [mp.ID_COUNT, 0]],
                f"{mp.ID_COUNT}:0 takes [['{lp.PULL}', 2], ['{mp.ID_OFFSET_SET}', 0]] alone, "
                f"has [['{lp.PULL}', 2], ['{mp.ID_OFFSET_SET}', 0], ['obj-thisdevice', 0]]",
            ),
            # The tick banged by the load too.
            (
                [[cl.ID_LOAD, 0], [cl.ID_TICK, 0]],
                f"{cl.ID_TICK}:0 takes [['{mp.ID_EVERY}', 0]] alone, "
                f"has [['{cl.ID_LOAD}', 0], ['{mp.ID_EVERY}', 0]]",
            ),
            # The counter's range set from elsewhere.
            (
                [["obj-thisdevice", 1], [mp.ID_COUNT, 3]],
                f"{mp.ID_COUNT}:3 takes [] alone, has [['obj-thisdevice', 1]]",
            ),
            # A LOM read fed from the tick.
            (
                [[cl.ID_TICK, 0], ["obj-parent1-get", 0]],
                f"{cl.ID_TICK} outputs into "
                + str(sorted([[4 - level, f"obj-parent{level}-path", 0] for level in range(1, 5)]))
                + " alone, has "
                + str(
                    sorted(
                        [[4 - level, f"obj-parent{level}-path", 0] for level in range(1, 5)]
                        + [[0, "obj-parent1-get", 0]]
                    )
                ),
            ),
        )
        for (src, dst), want in cases:
            patch = self.patcher(METER)
            self.line(patch, src, dst)
            self.save(patch, METER)
            self.assertIn(f"{WHERE}: {want}", self.errors())
            self.restore(METER)
        for box, text, want in (
            (mp.ID_COUNT, "counter 0 29", "counter 0 299"),
            (cl.ID_OFFSET, "random 10000", "random 300"),
            (mp.ID_EVERY, "sel 1", "sel 0"),
        ):
            patch = self.patcher(METER)
            next(b for b in cl.boxes(patch["patcher"]) if b.get("id") == box)["text"] = text
            self.save(patch, METER)
            self.assertIn(f"{WHERE}: {box} must be '{want}', has {text.split()}", self.errors())
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
                f"{WHERE}: {lp.POLL_ROUTE} must be 'route /fohmixer/meter/ask', has "
                "['route', '/fohmixer/meter/poll']"
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
