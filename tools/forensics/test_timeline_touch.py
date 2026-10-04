"""The forensics timeline's touches (#43, PR D) in whole reports: where a
touch of a single volume fader started against Live's value before it (the
hub's ``live_before``), the first-touch jump, the time from the finger's down
to its first move and first send, and the stutter inside a drag (gaps
between the finger's moves, a value held while the finger moved). The page
records of PR D: a ``touch`` down with its start, ``mv`` records per frame,
``rtt`` summaries, a ``send`` only when the socket did not take it. The
synthetic logs and the report reader come from ``test_timeline.py``."""

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
    Log,
    ReportCase,
    digest,
)

# A volume's Live value at fader position p (``behave/fader.rs`` ``to_live``).
EXPONENT = 0.515


def value_at(p):
    return round(p**EXPONENT, 9)


def down(t, key, pointer, *, start, live, local=False, pos=None, dt=-4.0, c=500.0):
    """A finger's down on a fader (the page's ``touch`` with its start)."""
    return {
        "ev": "touch",
        "t": t,
        "what": "down",
        "keys": [key],
        "pointer": pointer,
        "dt": dt,
        "c": c,
        "travel": 300.0,
        "pos": start if pos is None else pos,
        "live": live,
        "local": local,
        "from": start,
    }


def lift(t, key, pointer):
    return {"ev": "touch", "t": t, "what": "up", "keys": [key], "pointer": pointer}


def frames(key, pointer, t0, start, steps, *, every=16.0, c0=500.0, seq=1):
    """The ``mv`` records of a drag from page time ``t0``: one frame every
    ``every`` ms; each step (finger px since the down, the position sent)
    one frame with one move 3 ms before it. Returns (records, the page's
    sends as (t, Live value))."""
    records, sends = [], []
    for i, (px, sent) in enumerate(steps):
        t = t0 + every * i
        records.append(
            {
                "ev": "mv",
                "t": t,
                "key": key,
                "p": pointer,
                "e": [[-3.0, c0 - px]],
                "r": round(start + px / 300.0, 5),
                "s": sent,
                "q": seq + i,
            }
        )
        sends.append((t + 0.5, value_at(sent)))
    return records, sends


def first_set(log, key):
    return min(
        (r for r in log.records if r["ev"] == "set" and r["key"] == key), key=lambda r: r["t"]
    )


def touch_rows(page):
    return [a for _, a in page.elements if "touch" in (a.get("class") or "").split()]


class FirstTouch(ReportCase):
    def drag_from(self, log, *, start, live, live_before, local=False, first_px=1.0):
        """A touch of Vox 1 at page time p0 + 0 that starts at ``start`` (the
        page's Live position ``live``), its first frame ``first_px`` px up,
        then 20 frames of 1 px; the hub held ``live_before``."""
        log.pings(7, BASE, BASE + 4000)
        p0 = BASE + 1000 - OFFSET
        steps = [(first_px + i, round(start + (first_px + i) / 300.0, 5)) for i in range(21)]
        records, sends = frames(VOX, 4, p0 + 200, start, steps)
        log.drag(VOX, 7, sends)
        first_set(log, VOX)["live_before"] = live_before
        events = [
            down(p0, VOX, 4, start=start, live=live, local=local),
            *records,
            lift(records[-1]["t"] + 10, VOX, 4),
        ]
        log.trace(BASE + 3000, 7, events)

    def test_a_touch_from_a_stale_value_is_a_first_touch_jump(self):
        # Live sat at 0 dB (0.85); the page still showed -14 dB (position
        # 0.26), and the finger's first move of 1 px set Live to -14 dB.
        log = Log()
        self.drag_from(log, start=0.26, live=0.26, live_before=0.85)
        summary, page, stdout = self.report(log, BASE, BASE + 5000)
        self.assertEqual(summary["touches"], "1")
        self.assertEqual(summary["first_touch_jumps"], "1")
        (row,) = touch_rows(page)
        self.assertEqual(row["data-key-hash"], digest(VOX))
        self.assertEqual(row["data-first-jump"], "true")
        self.assertEqual(row["data-why"], "stale")
        self.assertAlmostEqual(float(row["data-jump-db"]), 13.9, delta=0.2)
        self.assertLess(float(row["data-finger-db"]), 0.2)
        self.assertNotIn("Vox 1", stdout)

    def test_a_fast_first_move_is_no_first_touch_jump(self):
        # The finger's first frame went 30 px (about 3 dB): Live followed it.
        log = Log()
        self.drag_from(log, start=0.7, live=0.7, live_before=value_at(0.7), first_px=30.0)
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual((summary["touches"], summary["first_touch_jumps"]), ("1", "0"))
        (row,) = touch_rows(page)
        self.assertEqual(row["data-first-jump"], "false")
        self.assertGreater(float(row["data-jump-db"]), 1.0)
        self.assertGreater(float(row["data-finger-db"]), 1.0)

    def test_a_touch_from_the_faders_own_hold_is_local(self):
        # The fader still showed its own last write (the hold) while Live
        # had gone to 0 dB meanwhile.
        log = Log()
        self.drag_from(log, start=0.3, live=0.729, live_before=0.85, local=True)
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(summary["first_touch_jumps"], "1")
        (row,) = touch_rows(page)
        self.assertEqual(row["data-why"], "local")

    def test_a_fader_that_went_up_while_the_finger_went_down_is_a_jump(self):
        # Live at -4 dB (0.75), the page still at 0 dB (0.85); the finger
        # pulls 24 px down (to about -2 dB), so Live goes UP by 2 dB.
        log = Log()
        start = round(0.85 ** (1 / EXPONENT), 5)
        self.drag_from(log, start=start, live=start, live_before=0.75, first_px=-24.3)
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(summary["first_touch_jumps"], "1")
        (row,) = touch_rows(page)
        self.assertEqual((row["data-first-jump"], row["data-why"]), ("true", "stale"))
        self.assertGreater(float(row["data-off-db"]), 3.0)

    def test_the_first_applied_value_is_the_touchs_own(self):
        # Another tablet (its own seq 1, its own page clock) writes the key
        # inside this touch; its write is applied just after this touch's
        # first set reached the hub: it is not this touch's.
        log = Log()
        self.drag_from(log, start=0.5, live=0.5, live_before=value_at(0.5))
        first = first_set(log, VOX)
        other = dict(
            client=9,
            instance="band",
            key=VOX,
            seq=1,
            value=0.95,
            final=True,
            t=first["t"] - 100,
            hub_ms=first["hub_ms"] - 1,
            offset_ms=OFFSET,
        )
        log.add("set", first["hub_ms"] - 1, **other)
        log.write_batch("band", VOX, 9, 1, 0.95, first["hub_ms"] - 1, 2.0)
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(summary["first_touch_jumps"], "0")
        (row,) = touch_rows(page)
        self.assertLess(float(row["data-jump-db"]), 0.2)

    def test_down_to_first_move_and_first_send_on_the_pages_clock(self):
        log = Log()
        self.drag_from(log, start=0.5, live=0.5, live_before=value_at(0.5))
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        (row,) = touch_rows(page)
        # The down's event at p0 - 4; the first move at p0 + 200 - 3; the
        # first send at p0 + 200.5.
        self.assertEqual(row["data-first-move-ms"], "201.0")
        self.assertEqual(row["data-first-send-ms"], "204.5")
        self.assertEqual(row["data-first-jump"], "false")
        self.assertEqual(summary["first_move_p50_ms"], "201.0")
        self.assertEqual(summary["first_move_max_ms"], "201.0")
        self.assertEqual(summary["first_send_p50_ms"], "204.5")
        self.assertEqual(summary["first_send_max_ms"], "204.5")

    def test_a_touch_of_several_keys_and_an_old_touch_are_not_analysed(self):
        log = Log()
        log.pings(7, BASE, BASE + 4000)
        p0 = BASE + 1000 - OFFSET
        several = down(p0, VOX, 4, start=0.5, live=0.5)
        several["keys"] = [VOX, HAND]
        old = {"ev": "touch", "t": p0 + 500, "what": "down", "keys": [HAND], "pointer": 5}
        param = down(p0 + 900, KLAVIR_PARAM, 6, start=0.5, live=0.5)
        log.trace(BASE + 3000, 7, [several, old, param])
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(summary["touches"], "0")
        self.assertEqual(touch_rows(page), [])


class Stutter(ReportCase):
    def drag(self, log, steps, gaps=None):
        """A drag of Vox 1 from 0.5: ``steps`` as in ``frames``, a frame every
        16 ms except before the frames ``gaps`` names (frame -> ms)."""
        log.pings(7, BASE, BASE + 4000)
        p0 = BASE + 1000 - OFFSET
        records, sends = frames(VOX, 4, p0 + 50, 0.5, steps)
        t = p0 + 50
        for i, record in enumerate(records):
            t += (gaps or {}).get(i, 16.0) if i else 0.0
            record["t"] = t
        sends = [(r["t"] + 0.5, value_at(r["s"])) for r in records]
        log.drag(VOX, 7, sends)
        first_set(log, VOX)["live_before"] = value_at(0.5)
        events = [down(p0, VOX, 4, start=0.5, live=0.5), *records, lift(t + 20, VOX, 4)]
        log.trace(BASE + 3500, 7, events)

    def test_a_gap_between_moves_while_the_finger_travelled_is_a_move_gap(self):
        # The moves stop for 180 ms while the finger goes on 30 px (a
        # stutter), and later for 300 ms while it goes 1 px (a rest).
        log = Log()
        px = [i + 1 for i in range(10)] + [40 + i for i in range(10)] + [50 + i for i in range(10)]
        steps = [(p, round(0.5 + p / 300.0, 5)) for p in px]
        self.drag(log, steps, gaps={10: 180.0, 20: 300.0})
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(summary["move_gaps"], "1")
        self.assertEqual(summary["move_gap_max_ms"], "180.0")
        (row,) = touch_rows(page)
        self.assertEqual((row["data-move-gaps"], row["data-held"]), ("1", "0"))

    def test_a_touch_without_its_lift_ends_at_the_keys_next_down(self):
        # The first touch's lift was lost; the second touch's frames (with a
        # stalled move) are not the first one's.
        log = Log()
        log.pings(7, BASE, BASE + 6000)
        p0 = BASE + 1000 - OFFSET
        steps = [(i + 1, round(0.5 + (i + 1) / 300.0, 5)) for i in range(10)]
        first, sends = frames(VOX, 4, p0 + 50, 0.5, steps)
        p1 = p0 + 1500
        second, more = frames(VOX, 4, p1 + 50, 0.5, steps)
        second[5]["e"] = [[-3.0 + 180.0, 470.0]]
        log.drag(VOX, 7, sends + more)
        first_set(log, VOX)["live_before"] = value_at(0.5)
        events = [
            down(p0, VOX, 4, start=0.5, live=0.5),
            *first,
            down(p1, VOX, 4, start=0.5, live=0.5),
            *second,
            lift(p1 + 400, VOX, 4),
        ]
        log.trace(BASE + 4000, 7, events)
        summary, page, _ = self.report(log, BASE, BASE + 6000)
        self.assertEqual(summary["touches"], "2")
        rows = touch_rows(page)
        self.assertEqual([r["data-move-gaps"] for r in rows], ["0", "1"])

    def test_a_value_held_while_the_finger_moved_is_a_held_run(self):
        log = Log()
        steps = [(i, round(0.5 + i / 300.0, 5)) for i in range(1, 11)]
        # Frames 3 to 6 sent the same value although the finger moved on.
        for i in range(3, 7):
            steps[i] = (steps[i][0], steps[2][1])
        # At the top the value stays at 1.0: no stutter.
        steps += [(400 + i, 1.0) for i in range(5)]
        self.drag(log, steps)
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(summary["held_runs"], "1")
        (row,) = touch_rows(page)
        self.assertEqual(row["data-held"], "1")
        self.assertEqual(summary["move_gaps"], "0")


class NewRecords(ReportCase):
    def test_rtt_summaries_and_sends_from_the_hubs_sets(self):
        # PR D's page records no pong and no send the socket took: the page's
        # round trips come from its `rtt` summaries, its send row from the
        # hub's sets (their page time), and an unsent write is a hollow point.
        log = Log()
        log.pings(7, BASE, BASE + 4000)
        p0 = BASE + 1000 - OFFSET
        sends = [(p0 + 16 * i, round(0.5 + 0.002 * i, 6)) for i in range(10)]
        sends += [(p0 + 16 * 9 + 400 + 16 * i, round(0.52 + 0.002 * i, 6)) for i in range(10)]
        log.drag(VOX, 7, sends)
        kept = {
            "ev": "send",
            "t": p0 + 760,
            "seq": 99,
            "key": VOX,
            "value": 0.51,
            "final": True,
            "sent": False,
        }
        rtts = [
            {"ev": "rtt", "t": p0 - 1000, "n": 10, "min": 3.0, "med": 5.0, "max": 40.0},
            {"ev": "rtt", "t": p0, "n": 9, "min": 4.0, "med": 7.0, "max": 12.0},
        ]
        log.trace(
            BASE + 3000,
            7,
            [down(p0 - 5, VOX, 4, start=0.5, live=0.5), *rtts, kept, lift(p0 + 700, VOX, 4)],
        )
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(summary["rtt_page_p50_ms"], "5.0")
        self.assertEqual(summary["rtt_page_max_ms"], "40.0")
        (gap,) = self.gaps(page, "send", VOX)
        self.assertEqual(gap["data-ms"], "400.0", "the hub's sets are the page's sends")
        self.assertEqual(len(page.of_class("unsent")), 1, "the write the socket did not take")


class SendTimes(ReportCase):
    def test_a_write_the_socket_did_not_take_counts_among_the_pages_sends(self):
        # The page sent every 16 ms, then nothing reached the hub for 180 ms
        # but one write the socket did not take, half way; the next set is
        # 4 dB further. The page did send (its unsent write), the hub heard
        # nothing: the link's.
        log = Log()
        log.pings(7, BASE, BASE + 4000)
        p0 = BASE + 1000 - OFFSET
        sends = [(p0 + 16 * i, round(0.5 + 0.002 * i, 6)) for i in range(10)]
        resume = sends[-1][0] + 180
        sends += [(resume + 16 * i, round(0.618 + 0.002 * i, 6)) for i in range(10)]
        log.drag(VOX, 7, sends)
        kept = {
            "ev": "send",
            "t": sends[9][0] + 90,
            "seq": 99,
            "key": VOX,
            "value": 0.56,
            "final": False,
            "sent": False,
        }
        log.trace(BASE + 3000, 7, [kept])
        _, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(self.jumps(page), [("link", digest(VOX))])


if __name__ == "__main__":
    unittest.main()
