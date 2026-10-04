"""The forensics timeline's pure analysis helpers (#43, ``timeline_model``):
jumps, percentiles, gestures and gaps, busy episodes."""

import math
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


class FirstTouchRule(unittest.TestCase):
    """``timeline_touch.first_touch``: the first applied value against Live's
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


class Thresholds(unittest.TestCase):
    """Each threshold of a touch's rules at its exact boundary and the next
    float."""

    def test_a_first_touch_jump_needs_over_1_db(self):
        self.assertFalse(touches.over_jump(1.0))
        self.assertTrue(touches.over_jump(math.nextafter(1.0, 2.0)))

    def test_a_move_gap_needs_over_50_ms_and_over_3_px(self):
        self.assertEqual(touches.MOVE_GAP_MS, 50.0)
        self.assertEqual(touches.MOVE_GAP_PX, 3.0)
        after, more = math.nextafter(50.0, 60.0), math.nextafter(3.0, 4.0)
        self.assertFalse(touches.is_move_gap(50.0, 30.0))
        self.assertTrue(touches.is_move_gap(after, 30.0))
        self.assertFalse(touches.is_move_gap(180.0, 3.0))
        self.assertTrue(touches.is_move_gap(180.0, more))

    def test_a_finger_move_is_over_1e_4_and_a_held_value_inside_the_travel(self):
        self.assertFalse(touches.finger_moved(1e-4))
        self.assertFalse(touches.finger_moved(-1e-4))
        self.assertTrue(touches.finger_moved(math.nextafter(1e-4, 1.0)))
        self.assertTrue(touches.finger_moved(-math.nextafter(1e-4, 1.0)))
        self.assertFalse(touches.inside_travel(0.0))
        self.assertFalse(touches.inside_travel(1.0))
        self.assertTrue(touches.inside_travel(math.nextafter(0.0, 1.0)))
        self.assertTrue(touches.inside_travel(math.nextafter(1.0, 0.0)))

    def test_a_stale_page_value_is_over_1_db_from_lives(self):
        # Live at -2 dB (0.8); the page's own value of Live 1 dB away (as near
        # as a position gives) is not stale; over_jump pins the boundary.
        start = read.to_pos(0.5)
        raw = start + 0.003
        live_exactly = read.to_pos(0.825)
        rule = touches.first_touch(start, live_exactly, False, 0.8, 0.5, raw)
        self.assertEqual(rule[3:], (True, "other"))
        self.assertAlmostEqual(touches.pos_db(live_exactly), -1.0, places=9)

    def test_a_span_reaches_a_hole_the_recorder_left_from_its_first_to_its_last_ms(self):
        # PR E: page times 1 to 2 reach into a hole from 2 to 3 (both ends
        # count), not into one from the next float after 2, nor one before 1.
        self.assertTrue(touches.across(1.0, 2.0, [(2.0, 3.0)]))
        self.assertFalse(touches.across(1.0, 2.0, [(math.nextafter(2.0, 3.0), 3.0)]))
        self.assertTrue(touches.across(3.0, 4.0, [(2.0, 3.0)]))
        self.assertFalse(touches.across(math.nextafter(3.0, 4.0), 4.0, [(2.0, 3.0)]))
        self.assertTrue(touches.across(1.0, 9.0, [(5.0, 6.0)]), "a hole inside")
        self.assertFalse(touches.across(1.0, 9.0, []))
        # A touch's holes are cut to it, in time order; one outside is none.
        holes = [(50.0, 80.0), (5.0, 15.0), (200.0, 300.0)]
        self.assertEqual(touches.no_data(holes, 10.0, 60.0), [(10.0, 15.0), (50.0, 60.0)])
        self.assertEqual(touches.no_data(holes, 100.0, 150.0), [])

    def test_a_held_run_is_3_holds_in_a_row(self):
        self.assertEqual(touches.HELD_FRAMES, 3)

        def run(n):
            return [{"r": round(0.5 + 0.01 * i, 5), "s": 0.6} for i in range(n)]

        self.assertEqual(touches.held_runs(run(4)), 1, "4 frames, 3 holds")
        self.assertEqual(touches.held_runs(run(3)), 0, "3 frames, 2 holds")

    def test_each_condition_of_the_rules_counts(self):
        # Live's value did not move (no jump) although the finger alone would
        # have moved it 1.25 dB: no first-touch jump.
        start = 0.5
        rule = touches.first_touch(start, start, False, 0.8, 0.8, start + 0.05)
        self.assertEqual(rule[3], False)
        self.assertEqual(rule[0], 0.0)
        self.assertGreater(rule[2], touches.FIRST_JUMP_DB)
        # The same value sent while the finger rests is no hold.
        self.assertFalse(touches.holds({"r": 0.5, "s": 0.6}, {"r": 0.5, "s": 0.6}))
        self.assertTrue(touches.holds({"r": 0.5, "s": 0.6}, {"r": 0.51, "s": 0.6}))


class TouchRecords(unittest.TestCase):
    def test_a_touchs_frames_are_its_pointers_on_its_key_both_ends_included_in_page_order(self):
        frames = [
            {"key": VOX, "p": 4, "t": 1_016.0},
            {"key": VOX, "p": 4, "t": 1_000.0},
            {"key": VOX, "p": 5, "t": 1_010.0},
            {"key": HAND, "p": 4, "t": 1_012.0},
            {"key": VOX, "p": 4, "t": 1_032.0},
            {"key": VOX, "p": 4, "t": 999.0},
            {"key": VOX, "p": 4},
        ]
        mine = touches.own_frames(frames, VOX, 4, 1_000.0, 1_032.0)
        self.assertEqual([f["t"] for f in mine], [1_000.0, 1_016.0, 1_032.0])
        self.assertFalse(touches.within(None, 0.0, 1.0))
        self.assertTrue(touches.within(0.0, 0.0, 1.0))
        self.assertTrue(touches.within(1.0, 0.0, 1.0))
        self.assertFalse(touches.within(math.nextafter(1.0, 2.0), 0.0, 1.0))


class TouchSets(unittest.TestCase):
    def test_a_frame_names_its_set_by_seq_and_time(self):
        frames = [{"t": 1_000.0, "q": 7}, {"t": 1_016.0, "q": 8}]
        self.assertTrue(model.names_set(frames, {"seq": 8, "t": 1_015.8}))
        self.assertTrue(model.names_set(frames, {"seq": 7, "t": 1_050.0}), "50 ms")
        self.assertFalse(model.names_set(frames, {"seq": 7, "t": 1_050.1}))
        self.assertFalse(model.names_set(frames, {"seq": 9, "t": 1_016.0}), "another seq")
        self.assertFalse(model.names_set(frames, {"seq": 8}), "no time")
        self.assertFalse(model.names_set([{"q": 8}], {"seq": 8, "t": 1_016.0}), "a frame without t")


if __name__ == "__main__":
    unittest.main()
