"""The forensics timeline's volume laws (#63): a page's fader touch names
Live's own law (the position is Live's volume) in its down's ``law``; a down
without it was on TouchOSC's (``p^0.515``), as every page before #63."""

import collections
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import timeline_read as read  # noqa: E402
import timeline_touch as touch  # noqa: E402

Down = collections.namedtuple("Down", ("hub", "data"))


class Laws(unittest.TestCase):
    def test_a_down_names_lives_law_or_was_on_touchoscs(self):
        self.assertEqual(read.law_of({"law": "live"}), "live")
        self.assertEqual(read.law_of({}), "touchosc")
        self.assertEqual(read.law_of({"law": "console"}), "touchosc")

    def test_lives_law_is_the_position_itself(self):
        self.assertEqual(read.to_live(0.6, "live"), 0.6)
        self.assertEqual(read.to_pos(0.85, "live"), 0.85)
        self.assertEqual(read.to_live(1.5, "live"), 1.0)
        self.assertEqual(read.to_pos(-0.5, "live"), 0.0)
        self.assertAlmostEqual(read.to_live(0.5), 0.5**0.515, places=12)
        self.assertAlmostEqual(read.to_pos(0.85), 0.85 ** (1 / 0.515), places=12)
        self.assertAlmostEqual(touch.pos_db(0.85, "live"), 0.0, places=9)
        self.assertAlmostEqual(touch.pos_db(0.85 ** (1 / 0.515)), 0.0, places=9)

    def test_a_first_touch_is_measured_on_its_own_law(self):
        # From 0.3 (Live's −24.6 dB) the finger took the fader to 0.5 and
        # Live applied 0.5: on Live's law exactly where the finger went, on
        # TouchOSC's law 1.4 dB away from it, a jump.
        jump, finger, off, flagged, why = touch.first_touch(0.3, 0.3, False, 0.3, 0.5, 0.5, "live")
        self.assertGreater(jump, 1.0)
        self.assertAlmostEqual(off, 0.0, places=9)
        self.assertAlmostEqual(finger, jump, places=9)
        self.assertFalse(flagged)
        self.assertIsNone(why)
        flagged_tosc = touch.first_touch(0.3, 0.3, False, 0.3, 0.5, 0.5)[3]
        self.assertTrue(flagged_tosc)

    def test_a_touch_keeps_the_law_of_its_down(self):
        start = {"what": "down", "from": 0.6, "live": 0.6, "local": False, "pointer": 1}
        live = touch.analyse(Down(0.0, dict(start, law="live")), "k", [], None, None)
        self.assertEqual(live.law, "live")
        older = touch.analyse(Down(0.0, start), "k", [], None, None)
        self.assertEqual(older.law, "touchosc")


if __name__ == "__main__":
    unittest.main()
