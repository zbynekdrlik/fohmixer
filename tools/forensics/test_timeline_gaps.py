"""The forensics timeline's gaps (#43) in whole reports: which touches make
one gesture, and which controls get gaps at all (only continuous ones: a key
with a non-final send in the window). The synthetic logs and the report
reader come from ``test_timeline.py``."""

import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

from test_timeline import (  # noqa: E402
    BASE,
    HAND,
    KLAVIR_PARAM,
    OFFSET,
    VOX,
    VOX_MUTE,
    Log,
    ReportCase,
    digest,
    page_sends,
)


def touch(t, what, key, pointer=1):
    return {"ev": "touch", "t": t, "what": what, "keys": [key], "pointer": pointer}


def strokes(start, count, value, step=0.002):
    """``count`` sends every 16 ms from page time ``start``, the value rising
    ``step`` a send from ``value``; the next value too."""
    sends = [(start + 16 * i, round(value + step * i, 6)) for i in range(count)]
    return sends, round(value + step * count, 6)


class Gestures(ReportCase):
    """Gaps are marked only inside one touch's span, and only for continuous
    controls (a key with a non-final send in the window)."""

    def assertNoGap(self, summary, page):
        self.assertEqual(page.of_class("gap"), [])
        for row in ("send", "arrival", "applied"):
            self.assertEqual(summary[f"gap_{row}_max_ms"], "0", row)

    def test_two_mute_taps_400_ms_apart_mark_no_gap(self):
        log = Log()
        log.pings(7, BASE, BASE + 3000)
        p0 = BASE + 1000 - OFFSET
        sends = log.drag(VOX_MUTE, 7, [(p0, 1), (p0 + 400, 0)], final=True)
        taps = [touch(p0 - 10, "tap", VOX_MUTE), touch(p0 + 390, "tap", VOX_MUTE, pointer=2)]
        log.trace(BASE + 2500, 7, [*taps, *sends])
        summary, page, _ = self.report(log, BASE, BASE + 3000)
        self.assertEqual([a["data-key-hash"] for a in page.of_class("control")], [digest(VOX_MUTE)])
        self.assertNoGap(summary, page)

    def test_a_held_pulse_param_toggle_marks_no_gap(self):
        # A pulse toggle: final On at the press, held 2 s, final Off at the release.
        log = Log()
        log.pings(7, BASE, BASE + 4000)
        p0 = BASE + 1000 - OFFSET
        sends = log.drag(KLAVIR_PARAM, 7, [(p0, 1.0), (p0 + 2005, 0.0)], final=True)
        press = [touch(p0 - 5, "down", KLAVIR_PARAM), touch(p0 + 2000, "up", KLAVIR_PARAM)]
        log.trace(BASE + 3500, 7, [*press, *sends])
        summary, page, _ = self.report(log, BASE, BASE + 4000)
        self.assertEqual(len(page.of_class("control")), 1)
        self.assertNoGap(summary, page)

    def test_a_fader_let_go_and_grabbed_again_within_1_s_marks_no_gap(self):
        # Lifted at 2000, grabbed again at 2500: the time the finger was off
        # is no gap (the first touch's tail is cut at the second's start).
        log = Log()
        log.pings(7, BASE, BASE + 5000)
        p0 = BASE - OFFSET
        first, value = strokes(p0 + 1000, 63, 0.5)
        second, _ = strokes(p0 + 2516, 62, value)
        events = log.drag(HAND, 7, first + second)
        touches = [
            touch(p0 + 1000, "down", HAND),
            touch(p0 + 2000, "up", HAND),
            touch(p0 + 2500, "down", HAND, pointer=2),
            touch(p0 + 3500, "up", HAND, pointer=2),
        ]
        log.trace(BASE + 4500, 7, [*touches, *events])
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(summary["jumps"], "0")
        self.assertNoGap(summary, page)

    def test_another_tablets_touch_does_not_cut_a_gesture(self):
        # Tablet 1 holds the fader from 1000 to 6000 and pauses its moves from
        # 2000 to 5000; tablet 2 (its own clock) touches the same fader from
        # 3000 to 4000 without moving it. The 3 s pause is still a gap.
        log = Log()
        log.pings(7, BASE, BASE + 7000)
        log.pings(9, BASE, BASE + 7000, offset=1250.0)
        p0 = BASE - OFFSET
        before, value = strokes(p0 + 1000, 63, 0.5)
        after, _ = strokes(p0 + 5000, 63, value)
        events = log.drag(HAND, 7, before + after)
        held = [touch(p0 + 1000, "down", HAND), touch(p0 + 6000, "up", HAND)]
        log.trace(BASE + 6500, 7, [*held, *events])
        q0 = BASE - 1250.0
        rest = [touch(q0 + 3000, "down", HAND, pointer=2), touch(q0 + 4000, "up", HAND, pointer=2)]
        log.trace(BASE + 6600, 9, rest)
        summary, page, _ = self.report(log, BASE, BASE + 7000)
        self.assertEqual([g["data-ms"] for g in self.gaps(page, "send", HAND)], ["3008.0"])
        self.assertEqual(summary["gap_send_max_ms"], "3008.0")
        self.assertEqual(summary["gap_arrival_max_ms"], "3008.0")

    def test_two_taps_of_a_toggle_are_two_gestures(self):
        # Mute, solo and stage record a tap (no lift follows): the time between
        # two taps is no gap.
        log = Log()
        log.pings(7, BASE, BASE + 7000)
        p0 = BASE + 1000 - OFFSET
        sends = log.drag(VOX_MUTE, 7, [(p0, 1), (p0 + 5000, 0)], final=True)
        taps = [
            {"ev": "touch", "t": t, "what": "tap", "keys": [VOX_MUTE], "pointer": pointer}
            for t, pointer in ((p0 - 20, 1), (p0 + 4980, 2))
        ]
        log.trace(BASE + 6500, 7, [*taps, *sends])
        summary, page, _ = self.report(log, BASE, BASE + 7000)
        self.assertEqual([a["data-key-hash"] for a in page.of_class("control")], [digest(VOX_MUTE)])
        self.assertEqual(page.of_class("gap"), [])
        for row in ("send", "arrival", "applied"):
            self.assertEqual(summary[f"gap_{row}_max_ms"], "0", row)
        self.assertEqual(summary["jumps"], "0", "a toggle has no dB jumps")

    def test_a_fader_held_past_the_windows_end_keeps_its_gaps(self):
        log = Log()
        log.pings(7, BASE, BASE + 5000)
        p0 = BASE + 1000 - OFFSET
        sends = [(p0 + 16 * i, round(0.5 + 0.004 * i, 6)) for i in range(20)]
        # The finger rests 2 s, moves once more, and lifts after the window.
        sends.append((sends[-1][0] + 2000, 0.58))
        events = log.drag(HAND, 7, sends)
        down = {"ev": "touch", "t": p0 - 20, "what": "down", "keys": [HAND], "pointer": 1}
        log.trace(BASE + 4000, 7, [down, *events])
        end = sends[-1][0] + OFFSET + 500
        summary, _, _ = self.report(log, BASE, end)
        self.assertEqual(summary["gap_send_max_ms"], "2000.0")
        self.assertEqual(summary["gap_arrival_max_ms"], "2000.0")

    def test_another_tablets_lift_on_another_fader_does_not_end_a_hold(self):
        # Tablet 1 holds Hand2 with pointer 2 from 1000 to 11 000 and pauses
        # its moves from 8000 to 10 000; tablet 2 touches Vox 1 with its own
        # pointer 2 from 2500 to 3000. That lift names Vox 1, not Hand2: the
        # late pause is still a gap.
        log = Log()
        log.pings(7, BASE, BASE + 12000)
        log.pings(9, BASE, BASE + 12000, offset=1250.0)
        p0 = BASE - OFFSET
        before, value = strokes(p0 + 1000, 438, 0.3, step=0.001)
        after, _ = strokes(p0 + 10000, 63, value, step=0.001)
        events = log.drag(HAND, 7, before + after)
        hold = [touch(p0 + 1000, "down", HAND, 2), touch(p0 + 11000, "up", HAND, 2)]
        log.trace(BASE + 11500, 7, [*hold, *events])
        q0 = BASE - 1250.0
        other = [touch(q0 + 2500, "down", VOX, 2), touch(q0 + 3000, "up", VOX, 2)]
        log.trace(BASE + 11600, 9, other)
        summary, page, _ = self.report(log, BASE, BASE + 12000)
        self.assertEqual([g["data-ms"] for g in self.gaps(page, "send", HAND)], ["2008.0"])
        self.assertEqual(summary["gap_send_max_ms"], "2008.0")

    def test_a_lost_lift_ends_the_touch_at_the_next_down(self):
        # Pointer 1 goes down on Hand2 at 1000 and again at 6000 with no lift
        # between (the first lift was lost), then lifts at 7000: the first
        # touch ends at the second down, so the 4 s the finger was off is no
        # gap.
        log = Log()
        log.pings(7, BASE, BASE + 9000)
        p0 = BASE - OFFSET
        first, value = strokes(p0 + 1000, 63, 0.5)
        second, _ = strokes(p0 + 6016, 62, value)
        events = log.drag(HAND, 7, first + second)
        touches = [
            touch(p0 + 1000, "down", HAND),
            touch(p0 + 6000, "down", HAND),
            touch(p0 + 7000, "up", HAND),
        ]
        log.trace(BASE + 8000, 7, [*touches, *events])
        summary, page, _ = self.report(log, BASE, BASE + 9000)
        self.assertEqual(summary["jumps"], "0")
        self.assertNoGap(summary, page)


class Continuous(ReportCase):
    """Only a control with a non-final send in the window gets gaps, from the
    page's sends or from the hub's sets, whichever the log holds."""

    def assertNoGap(self, summary, page):
        self.assertEqual(page.of_class("gap"), [])
        for row in ("send", "arrival", "applied"):
            self.assertEqual(summary[f"gap_{row}_max_ms"], "0", row)

    def test_the_hubs_sets_alone_keep_their_arrival_gap(self):
        # A page before the recorder: no trace, only its non-final sets,
        # through a 600 ms stall on the way to the hub.
        log = Log()
        log.pings(7, BASE, BASE + 4000)
        sends, _ = strokes(BASE + 1000 - OFFSET, 125, 0.5)
        stall = BASE + 1500.0

        def arrive(sent):
            if stall <= sent < stall + 600:
                return stall + 600 + (sent - stall) / 1000.0
            return sent + 3.0

        log.drag(VOX, 7, sends, arrive=arrive)
        summary, _, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(summary["gap_arrival_max_ms"], "601.0")
        self.assertEqual(summary["gap_send_max_ms"], "0", "no page sends in the log")

    def test_the_pages_sends_alone_keep_their_send_gap(self):
        # The sends never reached the hub: no set records, only the page's
        # non-final sends with a 316 ms pause.
        log = Log()
        log.pings(7, BASE, BASE + 4000)
        first, value = strokes(BASE + 1000 - OFFSET, 20, 0.5)
        second, _ = strokes(first[-1][0] + 316, 20, value)
        log.trace(BASE + 3000, 7, page_sends(HAND, first + second))
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual([g["data-ms"] for g in self.gaps(page, "send", HAND)], ["316.0"])
        self.assertEqual(summary["gap_arrival_max_ms"], "0", "nothing reached the hub")

    def test_sends_without_a_final_field_get_no_gaps(self):
        # Neither the page's sends nor the hub's sets say whether they are
        # final: not a continuous control, so the 316 ms pause is no gap.
        log = Log()
        log.pings(7, BASE, BASE + 4000)
        first, value = strokes(BASE + 1000 - OFFSET, 20, 0.5)
        second, _ = strokes(first[-1][0] + 316, 20, value)
        log.trace(BASE + 3000, 7, log.drag(HAND, 7, first + second, final=None))
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(len(page.of_class("control")), 1)
        self.assertNoGap(summary, page)

    def test_a_non_final_send_before_the_window_does_not_count(self):
        # A drag 5 s before --from (inside the 60 s lead), then only final
        # sends 400 ms apart inside the window: no gap.
        log = Log()
        log.pings(7, BASE, BASE + 7000)
        log.pings(8, BASE + 9000, BASE + 20000)
        drag, _ = strokes(BASE + 5000 - OFFSET, 10, 0.5)
        log.trace(BASE + 5500, 7, log.drag(HAND, 7, drag))
        release = [(BASE + 12000 - OFFSET, 0.6), (BASE + 12400 - OFFSET, 0.7)]
        log.trace(BASE + 13000, 8, log.drag(HAND, 8, release, final=True))
        summary, page, _ = self.report(log, BASE + 10000, BASE + 20000)
        self.assertEqual(len(page.of_class("control")), 1)
        self.assertNoGap(summary, page)


if __name__ == "__main__":
    unittest.main()
