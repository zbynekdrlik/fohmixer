"""The forensics timeline's pure analysis helpers (#43, ``timeline_model``):
jumps, percentiles, gestures and gaps, busy episodes."""

import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import timeline_model as model  # noqa: E402
import timeline_read as read  # noqa: E402
import timeline_touch as touches  # noqa: E402

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
        # A touch of the control that begins exactly at the lift cuts the tail
        # there (at or after the lift, not only after it).
        regrab = [touch(1000, "down"), touch(2000, "up"), touch(2000, "down", pointer=2)]
        self.assertEqual(model.gestures(regrab, VOX, 9000), [(1000, 2000), (2000, 9000)])
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


class FirstTouchRule(unittest.TestCase):
    """``timeline_touches.first_touch``: the first applied value against Live's
    before and against where the finger alone would have taken Live."""

    def test_a_stale_start_is_a_jump_whichever_way_the_finger_went(self):
        # The page showed 0 dB while Live sat at -4 dB; the finger pulled the
        # fader 2 dB down and Live went UP to -2 dB.
        start = read.to_pos(0.85)
        raw = read.to_pos(0.8)
        jump, finger, off, flagged, why = touches.first_touch(start, start, False, 0.75, 0.8, raw)
        self.assertAlmostEqual(jump, 2.0, places=6)
        self.assertAlmostEqual(finger, 2.0, places=6)
        self.assertGreater(off, 4.0, "the finger alone would have taken Live to about -6 dB")
        self.assertEqual((flagged, why), (True, "stale"))

    def test_a_fast_first_move_is_the_fingers(self):
        start = read.to_pos(0.7)
        raw = start + 0.1
        jump, _, off, flagged, why = touches.first_touch(
            start, start, False, 0.7, read.to_live(raw), raw
        )
        self.assertGreater(jump, 1.0)
        self.assertLess(off, 1e-9)
        self.assertEqual((flagged, why), (False, None))

    def test_the_why_local_stale_or_other(self):
        start = read.to_pos(0.5)
        raw = start + 0.003
        # From the fader's own position (a hold), Live elsewhere.
        self.assertEqual(touches.first_touch(start, 0.6, True, 0.85, 0.5, raw)[3:], (True, "local"))
        # The page's value of Live was Live's; the first applied value came
        # from elsewhere.
        self.assertEqual(
            touches.first_touch(start, start, False, 0.5, 0.6, raw)[3:], (True, "other")
        )
        # Without the page's Live value, not stale.
        self.assertEqual(
            touches.first_touch(start, None, False, 0.85, 0.5, raw)[3:], (True, "other")
        )

    def test_both_differences_must_be_over_1_db(self):
        self.assertEqual(touches.FIRST_JUMP_DB, 1.0)
        start = read.to_pos(0.8)
        # -2 dB to exactly -1 dB with the finger still: jump and off both 1.0.
        jump, _, off, flagged, _ = touches.first_touch(start, start, False, 0.8, 0.825, start)
        self.assertEqual((jump, off, flagged), (1.0, 1.0, False))
        self.assertTrue(touches.first_touch(start, start, False, 0.8, 0.8250001, start)[3])
        # A missing value is no jump.
        self.assertEqual(
            touches.first_touch(None, None, False, 0.8, 0.9, 0.5), (None, None, None, False, None)
        )
        self.assertEqual(touches.first_touch(0.5, 0.5, False, None, 0.9, 0.5)[3], False)

    def test_db_apart_of_silence(self):
        inf = float("-inf")
        self.assertEqual(touches.db_apart(inf, inf), 0.0)
        self.assertEqual(touches.db_apart(inf, -60.0), float("inf"))
        self.assertEqual(touches.db_apart(-3.0, -1.5), 1.5)

    def test_the_finger_at_the_frame_that_sent_the_applied_set(self):
        frames = [{"q": 4, "r": 0.5}, {"q": 5, "r": 0.52}, {"q": 7, "r": 0.6}]
        self.assertEqual(touches.finger_at(frames, 5), 0.52)
        self.assertEqual(touches.finger_at(frames, 6), 0.52, "the last frame before it")
        self.assertEqual(touches.finger_at(frames, 3), 0.5, "none before: the first")
        self.assertEqual(touches.finger_at(frames, None), 0.5)
        self.assertEqual(touches.finger_at([], 5), None)


class TouchSets(unittest.TestCase):
    def test_a_frame_names_its_set_by_seq_and_time(self):
        frames = [{"t": 1_000.0, "q": 7}, {"t": 1_016.0, "q": 8}]
        self.assertTrue(model.names_set(frames, {"seq": 8, "t": 1_015.8}))
        self.assertTrue(model.names_set(frames, {"seq": 7, "t": 1_050.0}), "50 ms")
        self.assertFalse(model.names_set(frames, {"seq": 7, "t": 1_050.1}))
        self.assertFalse(model.names_set(frames, {"seq": 9, "t": 1_016.0}), "another seq")
        self.assertFalse(model.names_set(frames, {"seq": 8}), "no time")
        self.assertFalse(model.names_set([{"q": 8}], {"seq": 8, "t": 1_016.0}), "a frame without t")
