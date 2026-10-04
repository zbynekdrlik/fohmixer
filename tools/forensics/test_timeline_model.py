"""The forensics timeline's pure analysis helpers (#43, ``timeline_model``):
jumps, percentiles, gestures and gaps, busy episodes."""

import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import timeline_model as model  # noqa: E402

VOX = "band|live_set tracks[name=Vox 1] mixer_device volume|value"
HAND = "band|live_set tracks[name=Hand2 #] mixer_device volume|value"


def touch(hub, what, pointer=1, keys=(VOX,)):
    data = {"ev": "touch", "t": hub, "what": what, "keys": list(keys), "pointer": pointer}
    return model.PageEvent(hub, "touch", data)


class Jumps(unittest.TestCase):
    def test_a_jump_is_over_3_db_or_silence_against_sound(self):
        self.assertTrue(model.is_jump(-18.0, -14.9))
        self.assertFalse(model.is_jump(-18.0, -15.0))
        self.assertTrue(model.is_jump(float("-inf"), -60.0))
        self.assertTrue(model.is_jump(-60.0, float("-inf")))
        self.assertFalse(model.is_jump(float("-inf"), float("-inf")))
        self.assertFalse(model.is_jump(float("nan"), 0.0))


class Helpers(unittest.TestCase):
    def test_nearest_rank_percentiles(self):
        values = [45, 5, 300, 10, 15, 20, 25, 30, 35, 40]
        self.assertEqual(model.percentile(values, 0.50), 25)
        self.assertEqual(model.percentile(values, 0.90), 45)
        self.assertEqual(model.percentile(values, 0.99), 300)
        self.assertEqual(model.percentile([5, 1, 4, 2, 3], 0.50), 3, "rank 2.5 is 3")
        self.assertEqual(model.percentile([7, 6, 5, 4, 3, 2, 1], 0.90), 7, "rank 6.3 is 7")
        self.assertIsNone(model.percentile([], 0.5))

    def test_gaps_count_only_inside_one_gesture(self):
        touches = [
            touch(1000, "down"),
            touch(2000, "up"),
            touch(5000, "down"),
            touch(5100, "cancel"),
        ]
        spans = model.gestures(touches, VOX, 99999)
        self.assertEqual(spans, [(1000, 3000.0), (5000, 6100.0)])
        times = [1100, 1150, 1400, 2900, 4000, 5050, 5600]
        self.assertEqual(
            model.row_gaps(times, spans),
            [(1150, 1400, 250), (1400, 2900, 1500), (5050, 5600, 550)],
            "the gap from one gesture into the next is no gap",
        )
        self.assertIsNone(model.gestures(touches, HAND, 99999), "no touch names it")
        untouched = model.row_gaps([0, 50, 2050, 14050], None)
        self.assertEqual(untouched, [(50, 2050, 2000)], "12 s apart: two gestures")
        held = model.gestures([touch(1000, "down", pointer=4)], VOX, 9000)
        self.assertEqual(held, [(1000, 9000)], "no lift in the window: to its end")
        # Two fingers: each touch its own span, never merged, and a touch that
        # begins before the first one's lift never cuts it.
        downs = [touch(1000, "down"), touch(3000, "down", pointer=2), touch(3500, "up")]
        self.assertEqual(model.gestures(downs, VOX, 9000), [(1000, 4500.0), (3000, 9000)])
        tablets = [
            touch(1000, "down"),
            touch(3000, "down", pointer=2),
            touch(4000, "up", pointer=2),
            touch(6000, "up"),
        ]
        spans = model.gestures(tablets, VOX, 9000)
        self.assertEqual(spans, [(1000, 7000.0), (3000, 5000.0)])
        self.assertEqual(model.row_gaps([2000, 5000], spans), [(2000, 5000, 3000)])
        # Let go and grabbed again within the tail: the first span ends where
        # the next touch begins, so the time the finger was off is no gap.
        regrab = [
            touch(1000, "down"),
            touch(2000, "up"),
            touch(2500, "down", pointer=2),
            touch(3500, "up", pointer=2),
        ]
        spans = model.gestures(regrab, VOX, 9000)
        self.assertEqual(spans, [(1000, 2500), (2500, 4500.0)])
        self.assertEqual(model.row_gaps([1992, 2516], spans), [])
        # No lift: to the start of the next touch naming the key.
        unlifted = [touch(1000, "down"), touch(4000, "tap", pointer=2)]
        self.assertEqual(model.gestures(unlifted, VOX, 9000), [(1000, 4000), (4000, 5000.0)])

    def test_a_lift_ends_only_its_own_touch_of_that_control(self):
        # A lift names its control: another tablet's pointer 2 lifting on
        # Hand2 does not end a hold of Vox 1 by pointer 2.
        tablets = [
            touch(1000, "down", pointer=2),
            touch(1500, "down", pointer=2, keys=(HAND,)),
            touch(2000, "up", pointer=2, keys=(HAND,)),
            touch(10000, "up", pointer=2),
        ]
        self.assertEqual(model.gestures(tablets, VOX, 12000), [(1000, 11000.0)])
        self.assertEqual(model.gestures(tablets, HAND, 12000), [(1500, 3000.0)])
        # A lost lift: the lift is searched only up to the pointer's next down
        # on the control, so the first touch ends at that down.
        lost = [touch(1000, "down"), touch(6000, "down"), touch(7000, "up")]
        self.assertEqual(model.gestures(lost, VOX, 9000), [(1000, 6000), (6000, 8000.0)])
        # Another control's down after the lift does not cut the tail.
        other = [touch(1000, "down"), touch(2000, "up"), touch(2500, "down", 3, keys=(HAND,))]
        self.assertEqual(model.gestures(other, VOX, 9000), [(1000, 3000.0)])
        # A toggle's tap (no lift follows) lasts the tail only.
        taps = [touch(1000, "tap"), touch(6000, "tap", pointer=2)]
        self.assertEqual(model.gestures(taps, VOX, 9000), [(1000, 2000.0), (6000, 7000.0)])

    def test_busy_episodes_merge_both_sources(self):
        changes = [
            (1500.4, "band", False),
            (1000.0, "band", True),
            (1000.4, "band", True),
            (1500.0, "band", False),
            (3000.0, "master", True),
            (4000.0, "master", False),
            (5000.0, "band", True),
        ]
        episodes = model.busy_episodes(changes, 6000)
        self.assertEqual(
            [(e.info, e.start, e.end, e.ms) for e in episodes],
            [
                (("band", False), 1000.0, 1500.0, 500.0),
                (("master", False), 3000.0, 4000.0, 1000.0),
                (("band", True), 5000.0, 6000, 1000.0),
            ],
        )


if __name__ == "__main__":
    unittest.main()
