"""The forensics timeline's jump causes (#43) in whole reports: a jump's
link, Live, page and no-data causes, and the page's long frames and the
recorder's dropped spans they rest on. The synthetic logs and the report
reader come from ``test_timeline.py``."""

import calendar
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

from test_timeline import (  # noqa: E402
    BASE,
    KLAVIR_PAN,
    OFFSET,
    VOX,
    Log,
    ReportCase,
    digest,
)
import timeline_read as read  # noqa: E402


class Causes(ReportCase):
    def test_a_slow_live_round_trip_is_live(self):
        log = Log()
        log.pings(7, BASE, BASE + 3000)
        p0 = BASE + 1000 - OFFSET
        sends = [(p0 + 16 * i, round(0.5 + 0.004 * i, 6)) for i in range(100)]
        log.trace(BASE + 3000, 7, log.drag(VOX, 7, sends, rtt_ms=400.0))
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        causes = self.jumps(page)
        self.assertGreaterEqual(len(causes), 2)
        self.assertEqual({cause for cause, _ in causes}, {"live"})
        self.assertEqual(summary["jumps"], str(len(causes)))
        self.assertEqual(self.gaps(page, "arrival", VOX), [], "the link was even")

    def test_a_page_that_stopped_sending_is_page(self):
        log = Log()
        log.pings(7, BASE, BASE + 3000)
        p0 = BASE + 1000 - OFFSET
        sends = [(p0 + 16 * i, round(0.5 + 0.004 * i, 6)) for i in range(20)]
        # 300 ms without a send (a frame of the page's main thread), then the
        # finger's value, 0.1 (4 dB) further.
        resume = sends[-1][0] + 316
        sends += [(resume + 16 * i, round(0.676 + 0.004 * i, 6)) for i in range(20)]
        events = log.drag(VOX, 7, sends)
        # The page stamps a long frame when the frame that ends it comes
        # (`diag::frame`: its own time, the gap since the frame before).
        events.append({"ev": "frame", "t": resume, "ms": 300.0})
        log.trace(BASE + 3000, 7, events)
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(self.jumps(page), [("page", digest(VOX))])
        (frame,) = page.of_class("frame")
        self.assertAlmostEqual(float(frame["data-start"]), resume - 300 + OFFSET, delta=0.05)
        self.assertEqual(summary["gap_send_max_ms"], "316.0", "no touch: 10 s gestures")

    def stalled_drag(self, log, latency, set_offset=OFFSET, stamp=0.0, kept=True, lost_set=None):
        """Sends 16 ms apart but one 80 ms gap (not over 100 ms: no page
        gap), each value 0.1 up (every step a jump), each set ``latency`` ms
        on the way with its own ``set_offset``; the page's main thread
        stalled those 80 ms, and its long frame says so at the frame that
        ended the stall, ``stamp`` ms after the send of that frame: ``kept``,
        or dropped by the recorder (its note goes up later). ``lost_set``:
        the seq of a set the event log lost. Returns p0."""
        log.pings(7, BASE, BASE + 3000)
        p0 = BASE + 1000 - OFFSET
        times = [p0, p0 + 16, p0 + 32, p0 + 112, p0 + 128, p0 + 144]
        sends = [(t, round(0.45 + 0.1 * i, 6)) for i, t in enumerate(times)]
        events = log.drag(VOX, 7, sends, offset=set_offset, arrive=lambda sent: sent + latency)
        if lost_set is not None:
            log.records = [
                r for r in log.records if not (r["ev"] == "set" and r["seq"] == lost_set)
            ]
        stamp_t = p0 + 112 + stamp
        if kept:
            events.append({"ev": "frame", "t": stamp_t, "ms": 80.0})
        else:
            note = {"ev": "overflow", "t": p0 + 900, "n": 1, "kinds": {"frame": 1}}
            events.append(dict(note, **{"from": stamp_t, "to": stamp_t}))
        log.trace(BASE + 3000, 7, events)
        return p0

    def test_a_long_frame_is_the_stall_before_its_own_time(self):
        # Only the jump between the last send before the stall and the first
        # after it is the page's; the band is the 80 ms before the stamp.
        log = Log()
        p0 = self.stalled_drag(log, 3.0)
        _, page, _ = self.report(log, BASE, BASE + 5000)
        causes = [cause for cause, _ in self.jumps(page)]
        self.assertEqual(causes, ["move", "move", "page", "move", "move"])
        (frame,) = page.of_class("frame")
        self.assertAlmostEqual(float(frame["data-start"]), p0 + 32 + OFFSET, delta=0.05)

    def test_a_stall_is_matched_to_the_pages_sends_whatever_the_links_latency(self):
        # The same stall with each set 30 ms on the way: Live applies every
        # value later, but the stall still lies between the same two sends.
        log = Log()
        self.stalled_drag(log, 30.0)
        _, page, _ = self.report(log, BASE, BASE + 5000)
        causes = [cause for cause, _ in self.jumps(page)]
        self.assertEqual(causes, ["move", "move", "page", "move", "move"])

    def test_a_stall_is_matched_whatever_the_two_offsets_by_a_few_ms(self):
        # The frame is stamped just before the first send after the stall
        # (the same tick), and the set's own offset reads 1 ms below the
        # trace's: on the hub clock the stamp lands just after that send.
        # The stall still explains the jump into that send.
        log = Log()
        self.stalled_drag(log, 3.0, set_offset=OFFSET - 1.0, stamp=-0.3)
        _, page, _ = self.report(log, BASE, BASE + 5000)
        causes = [cause for cause, _ in self.jumps(page)]
        self.assertEqual(causes, ["move", "move", "page", "move", "move"])

    def test_without_its_sets_a_jump_is_matched_on_lives_applied_times(self):
        # The event log lost the set right after the stall (its line never
        # came): the jumps into and out of it have no send to match, so a
        # stall over Live's applied interval is the page's.
        log = Log()
        self.stalled_drag(log, 3.0, lost_set=4)
        _, page, _ = self.report(log, BASE, BASE + 5000)
        causes = [cause for cause, _ in self.jumps(page)]
        self.assertEqual(causes, ["move", "move", "page", "move", "move"])

    def test_without_its_sets_dropped_long_frames_are_matched_on_lives_applied_times(self):
        # The same, with the long frame dropped by the recorder: no data.
        log = Log()
        self.stalled_drag(log, 3.0, kept=False, lost_set=4)
        _, page, _ = self.report(log, BASE, BASE + 5000)
        causes = [cause for cause, _ in self.jumps(page)]
        self.assertEqual(causes, ["move", "move", "no data", "move", "move"])

    def test_a_stall_running_past_the_windows_end_is_drawn(self):
        # The window ends inside the stall: its frame, stamped after the
        # end, is still drawn up to the window's edge; one wholly after the
        # end is not.
        log = Log()
        p0 = self.stalled_drag(log, 3.0)
        log.trace(BASE + 3100, 7, [{"ev": "frame", "t": p0 + 400, "ms": 60.0}])
        _, page, _ = self.report(log, BASE, BASE + 1080)
        self.assertEqual(len(page.of_class("frame")), 1)

    def test_a_jump_where_the_recorder_dropped_the_long_frames_is_no_data(self):
        # The stall of the test above, but the page's recorder dropped its
        # long frame (PR E), and the set's offset reads 1 ms below the
        # trace's: a page stall can no longer be ruled out, so that jump is
        # no data, not the finger's move.
        log = Log()
        p0 = self.stalled_drag(log, 3.0, set_offset=OFFSET - 1.0, kept=False)
        _, page, _ = self.report(log, BASE, BASE + 5000)
        causes = [cause for cause, _ in self.jumps(page)]
        self.assertEqual(causes, ["move", "move", "no data", "move", "move"])
        # Drawn as the stall a dropped long frame can be: the page's 50 ms
        # before its stamp.
        (band,) = page.of_class("nodata")
        self.assertEqual(band["data-kind"], "frame")
        self.assertAlmostEqual(float(band["data-start"]), p0 + 112 - 50 + OFFSET, delta=0.05)
        # The note and the band's tooltip say the same: the dropped stamp
        # and the stall it can stand for (p0 + OFFSET is BASE + 1000).
        at = read.local_text(BASE + 1112)
        note = (
            f"the page flight recorder dropped 1 long frame stamped at {at}: "
            f"no data of page stalls from {read.local_text(BASE + 1062)} to {at}"
        )
        self.assertIn(f"<li>{note}</li>", page.text)
        self.assertIn(f"<title>no data: {note}</title>", page.text)

    def test_a_window_ending_before_the_drop_note_still_reads_its_span(self):
        # The note goes up with its batch, after the drag: a window ending
        # before it still reads the span.
        log = Log()
        self.stalled_drag(log, 3.0, kept=False)
        summary, page, _ = self.report(log, BASE, BASE + 1200)
        causes = [cause for cause, _ in self.jumps(page)]
        self.assertEqual(causes, ["move", "move", "no data", "move", "move"])
        self.assertEqual(summary["no_data_spans"], "1")

    def test_a_trace_uploaded_up_to_30_minutes_after_the_window_counts(self):
        # A full recorder backlog drains in about 95 s once the fingers rest
        # (design note §5.2), but while the link is down the page's events
        # wait for the next socket, and a hidden page uploads once a second:
        # a long frame inside the window can reach the event log minutes
        # after its end.
        log = Log()
        log.pings(7, BASE, BASE + 3000)
        p0 = BASE + 1000 - OFFSET
        log.trace(BASE + 5000 + 300000, 7, [{"ev": "frame", "t": p0 + 500, "ms": 120.0}])
        _, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(len(page.of_class("frame")), 1)

    def test_a_trace_from_the_next_socket_after_the_window_takes_its_own_clock(self):
        # Tablet 7's link went down inside the window; its events go up on
        # the next socket (client 8) 5 min after the window, whose own pings
        # are then the only ones of that page's clock. Tablet 9, its clock
        # 400 ms apart, pinged last before the window's end.
        log = Log()
        end = BASE + 5000
        log.pings(7, BASE, BASE + 3000)
        log.pings(9, BASE, BASE + 4000, offset=OFFSET + 400)
        back = end + 5 * 60000
        # Its round trips were slow: the tail's pings give the clock only.
        log.pings(8, back - 1000, back, rtt=900.0)
        p0 = BASE + 1000 - OFFSET
        log.trace(back, 8, [{"ev": "frame", "t": p0 + 500, "ms": 120.0}])
        summary, page, _ = self.report(log, BASE, end)
        (frame,) = page.of_class("frame")
        self.assertAlmostEqual(float(frame["data-start"]), p0 + 380 + OFFSET, delta=0.05)
        self.assertEqual(summary["rtt_hub_max_ms"], "12.0", "the window's pings only")

    def test_a_trace_29_minutes_after_a_window_ending_before_midnight_counts(self):
        # The tail reaches into the next UTC date's day file.
        end = calendar.timegm((2026, 10, 3, 23, 50, 0)) * 1000
        start = end - 5000
        log = Log()
        log.pings(7, start, start + 3000)
        p0 = start + 1000 - OFFSET
        log.trace(end + 29 * 60000, 7, [{"ev": "frame", "t": p0 + 500, "ms": 120.0}])
        _, page, _ = self.report(log, start, end)
        (frame,) = page.of_class("frame")
        self.assertAlmostEqual(float(frame["data-start"]), p0 + 380 + OFFSET, delta=0.05)

    def alternating_drag(self, log, times, latency=3.0, lost_set=None):
        """The page's sends at ``times`` (page clock), the value jumping
        between 0.3 and 0.9 every send (every step a jump), each set
        ``latency`` ms on the way; ``lost_set``: a seq the log lost."""
        log.pings(7, BASE, BASE + 3000)
        sends = [(t, 0.3 if i % 2 == 0 else 0.9) for i, t in enumerate(times)]
        events = log.drag(VOX, 7, sends, arrive=lambda sent: sent + latency)
        if lost_set is not None:
            log.records = [
                r for r in log.records if not (r["ev"] == "set" and r["seq"] == lost_set)
            ]
        return events

    def dropped_frames(self, p0, first, last, n):
        return dict(
            {"ev": "overflow", "t": p0 + 900, "n": n, "kinds": {"frame": n}},
            **{"from": p0 + first, "to": p0 + last},
        )

    def test_dropped_long_frames_cover_every_jump_from_their_first_stall_to_their_last(self):
        # Two dropped long frames, stamped at +112 and +224, each the end of
        # a stall with no send in it: every jump from the one into the first
        # stamp to the one into the last is no data.
        log = Log()
        p0 = BASE + 1000 - OFFSET
        times = [p0 + d for d in (0, 16, 32, 112, 128, 144, 224, 240)]
        events = self.alternating_drag(log, times)
        events.append(self.dropped_frames(p0, 112, 224, 2))
        log.trace(BASE + 3000, 7, events)
        _, page, _ = self.report(log, BASE, BASE + 5000)
        causes = [cause for cause, _ in self.jumps(page)]
        expected = ["move", "move"] + ["no data"] * 4 + ["move"]
        self.assertEqual(causes, expected)
        last = read.local_text(BASE + 1224)
        note = (
            "the page flight recorder dropped 2 long frames stamped from "
            f"{read.local_text(BASE + 1112)} to {last}: "
            f"no data of page stalls from {read.local_text(BASE + 1062)} to {last}"
        )
        self.assertIn(f"<li>{note}</li>", page.text)
        self.assertIn(f"<title>no data: {note}</title>", page.text)

    def test_a_dropped_long_frame_is_matched_inside_its_shortest_stall(self):
        # A 60 ms stall (the gap from the send at +52 to +112): the dropped
        # frame stamped at +112 is the jump into that send, not the one
        # before the stall.
        log = Log()
        p0 = BASE + 1000 - OFFSET
        events = self.alternating_drag(log, [p0 + d for d in (0, 16, 32, 52, 112, 128)])
        events.append(self.dropped_frames(p0, 112, 112, 1))
        log.trace(BASE + 3000, 7, events)
        _, page, _ = self.report(log, BASE, BASE + 5000)
        causes = [cause for cause, _ in self.jumps(page)]
        self.assertEqual(causes, ["move", "move", "move", "no data", "move"])

    def test_without_its_sets_a_dropped_long_frame_covers_its_whole_possible_stall(self):
        # The set at +32 was lost and each set took 30 ms: Live applied the
        # value of +16 at +56, the lost one at +72. A dropped long frame
        # stamped at +112 was a stall of at least 50 ms, from +62 on: the
        # jumps into and out of the lost set meet it on Live's applied times.
        log = Log()
        p0 = BASE + 1000 - OFFSET
        times = [p0 + d for d in (0, 16, 32, 112, 128, 144)]
        events = self.alternating_drag(log, times, latency=30.0, lost_set=3)
        events.append(self.dropped_frames(p0, 112, 112, 1))
        log.trace(BASE + 3000, 7, events)
        _, page, _ = self.report(log, BASE, BASE + 5000)
        causes = [cause for cause, _ in self.jumps(page)]
        self.assertEqual(causes, ["move", "no data", "no data", "move", "move"])

    def test_dropped_moves_or_round_trips_say_nothing_of_a_jumps_cause(self):
        # The same fast move with spans of dropped moves and round-trip
        # summaries over it: only dropped long frames hide a page stall.
        log = Log()
        log.pings(7, BASE, BASE + 3000)
        p0 = BASE + 1000 - OFFSET
        sends = [(p0 + 16 * i, round(0.45 + 0.1 * i, 6)) for i in range(6)]
        events = log.drag(VOX, 7, sends)
        for kind in ("mv", "rtt"):
            note = {"ev": "overflow", "t": p0 + 900, "n": 2, "kinds": {kind: 2}}
            events.append(dict(note, **{"from": p0, "to": p0 + 90}))
        log.trace(BASE + 3000, 7, events)
        _, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(self.jumps(page), [("move", digest(VOX))] * 5)

    def test_a_fast_move_with_every_gap_small_is_move(self):
        log = Log()
        log.pings(7, BASE, BASE + 3000)
        p0 = BASE + 1000 - OFFSET
        sends = [(p0 + 16 * i, round(0.45 + 0.1 * i, 6)) for i in range(6)]
        log.trace(BASE + 3000, 7, log.drag(VOX, 7, sends))
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(self.jumps(page), [("move", digest(VOX))] * 5)
        self.assertEqual((summary["gap_send_max_ms"], summary["gap_arrival_max_ms"]), ("0", "0"))

    def test_a_busy_live_is_live_and_a_pan_has_no_jumps(self):
        log = Log()
        log.pings(7, BASE, BASE + 3000)
        p0 = BASE + 1000 - OFFSET
        sends = [(p0 + 16 * i, round(0.45 + 0.1 * i, 6)) for i in range(3)]
        log.drag(VOX, 7, sends)
        log.add("link", BASE + 1010, instance="band", busy=True, tick_age_ms=470, reason="slow")
        log.add("link", BASE + 1600, instance="band", busy=False, tick_age_ms=10, reason="ok")
        pans = [(p0 + 16 * i, round(-0.9 + 0.6 * i, 6)) for i in range(4)]
        log.drag(KLAVIR_PAN, 7, pans)
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(self.jumps(page), [("live", digest(VOX))] * 2)
        lanes = [a["data-key-hash"] for a in page.of_class("control")]
        self.assertEqual(sorted(lanes), sorted([digest(VOX), digest(KLAVIR_PAN)]))
        pan_line = [a for a in page.of_class("value") if "arrival" in a["class"].split()]
        self.assertEqual(len(pan_line), 2, "a polyline of each key's arrivals")
        self.assertEqual(summary["busy"], "1")


if __name__ == "__main__":
    unittest.main()
