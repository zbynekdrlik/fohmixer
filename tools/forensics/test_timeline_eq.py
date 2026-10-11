"""The forensics timeline's Pro-Q 4 screen (#71, acceptance point 6): the
page's ``eq_touch`` fingers (down, up, cancel, stale) and ``eq_lat``
latency windows, the hub's ``eq`` records of a dropped stale down and of
the frames' round trips. The report lists the fingers and the windows;
stdout counts them and gives the latency's median, p95 and largest. The
synthetic logs and the report reader come from ``test_timeline.py``."""

import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

from test_timeline import BASE, OFFSET, Log, ReportCase  # noqa: E402
from timeline_eq import (  # noqa: E402
    EqFinger,
    EqLatency,
    EqLink,
    eq_finger,
    eq_latency,
    eq_link,
    samples_of,
)

PATH = "live_set tracks 1 devices 0"


def finger(t, what, p, x=431.0, y=321.0, z=3.0):
    """A page ``eq_touch`` event as ``diag/trace/fingers.rs`` writes it."""
    return {"ev": "eq_touch", "t": t, "what": what, "p": p, "x": x, "y": y, "z": z}


def lat(t, frames, ms, dec):
    """A page ``eq_lat`` event as ``diag/trace/eq_lat.rs`` writes it."""
    return {"ev": "eq_lat", "t": t, "span": 1000.0, "f": frames, "ms": ms, "dec": dec}


def finger_rows(page):
    return [
        a
        for tag, a in page.elements
        if tag == "tr" and "eq-finger" in (a.get("class") or "").split()
    ]


def lat_rows(page):
    return [
        a for tag, a in page.elements if tag == "tr" and "eq-lat" in (a.get("class") or "").split()
    ]


class EqScreen(ReportCase):
    def session(self, log):
        """Socket 7's pings and one trace of its EQ screen from page time p0
        (hub BASE + 1000): a drag, a finger refused on an old picture, a tap
        and two seconds of drawing; the hub's stale down, a rate and a close's
        frames record."""
        log.pings(7, BASE, BASE + 5000)
        p0 = BASE + 1000 - OFFSET
        log.trace(
            BASE + 4500,
            7,
            [
                finger(p0, "down", 3),
                {
                    "ev": "eq_mv",
                    "t": p0 + 20,
                    "p": 3,
                    "n": 9,
                    "a": [431, 321],
                    "b": [470, 321],
                    "ms": 180,
                },
                finger(p0 + 220, "up", 3, x=470.0),
                finger(p0 + 230, "stale", 4),
                finger(p0 + 1200, "down", 5),
                finger(p0 + 1240, "up", 5),
                lat(p0 + 100, 22, [80.0, 120.5, 95.0], 9.5),
                lat(p0 + 1100, 18, [70.0, 210.0], 14.0),
            ],
        )
        log.add(
            "eq",
            BASE + 1300,
            what="stale",
            instance="band",
            path=PATH,
            client=7,
            session=1,
            why=None,
        )
        log.add(
            "eq",
            BASE + 2000,
            what="rate",
            session=1,
            frame_trips=30,
            frame_rtt_ms=40.0,
            frame_rtt_max_ms=150.0,
            ack_waits=1,
        )
        log.add(
            "eq",
            BASE + 3000,
            what="frames",
            instance="band",
            path=PATH,
            client=7,
            session=1,
            why=None,
            frame_trips=4,
            frame_rtt_ms=50.0,
            frame_rtt_max_ms=90.0,
            ack_waits=0,
        )
        # A rate whose holder was gone (no trips) counts nothing.
        log.add("eq", BASE + 3100, what="rate", session=1)

    def test_the_fingers_and_the_latency_are_listed_and_counted(self):
        log = Log()
        self.session(log)
        summary, page, stdout = self.report(log, BASE, BASE + 5000)
        self.assertEqual(
            {
                name: summary[name]
                for name in (
                    "eq_fingers",
                    "eq_stale",
                    "eq_hub_stale",
                    "eq_lat_n",
                    "eq_lat_p50_ms",
                    "eq_lat_p95_ms",
                    "eq_lat_max_ms",
                    "eq_frames_drawn",
                    "eq_decode_max_ms",
                    "eq_ack_waits",
                    "eq_frame_rtt_max_ms",
                )
            },
            {
                "eq_fingers": "2",
                "eq_stale": "1",
                "eq_hub_stale": "1",
                "eq_lat_n": "5",
                "eq_lat_p50_ms": "95.0",
                "eq_lat_p95_ms": "210.0",
                "eq_lat_max_ms": "210.0",
                "eq_frames_drawn": "40",
                "eq_decode_max_ms": "14.0",
                "eq_ack_waits": "1",
                "eq_frame_rtt_max_ms": "150.0",
            },
        )
        rows = finger_rows(page)
        self.assertEqual(
            [(r["data-what"], r.get("data-pointer")) for r in rows],
            [("down", "3"), ("up", "3"), ("stale", "4"), ("down", "5"), ("up", "5")],
            "in time order",
        )
        # On the hub's clock: the page time plus the socket's offset.
        self.assertEqual(rows[0]["data-time"], f"{BASE + 1000:.1f}")
        self.assertEqual([r["data-samples"] for r in lat_rows(page)], ["3", "2"])
        self.assertIn("eq_lat_p95_ms=210.0\n", stdout)

    def test_outside_the_window_is_left_out(self):
        log = Log()
        self.session(log)
        # The window ends after the first latency window and the stale finger.
        summary, page, _ = self.report(log, BASE, BASE + 1500)
        self.assertEqual(
            (
                summary["eq_fingers"],
                summary["eq_stale"],
                summary["eq_lat_n"],
                summary["eq_hub_stale"],
            ),
            ("1", "1", "3", "1"),
        )
        self.assertEqual([r["data-what"] for r in finger_rows(page)], ["down", "up", "stale"])
        self.assertEqual(summary["eq_ack_waits"], "0")

    def test_without_the_screen_the_counts_are_0_and_the_report_says_so(self):
        log = Log()
        log.pings(7, BASE, BASE + 2000)
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(
            [summary[name] for name in ("eq_fingers", "eq_stale", "eq_lat_n", "eq_frames_drawn")],
            ["0", "0", "0", "0"],
        )
        self.assertEqual(summary["eq_lat_p50_ms"], "n/a")
        self.assertEqual(finger_rows(page), [])
        self.assertIn("No Pro-Q 4 finger or picture", page.text)


class EqRecordsOf(unittest.TestCase):
    def test_the_records_read_their_fields(self):
        self.assertEqual(
            eq_finger(1000.0, finger(5.0, "stale", 4, x=1.5, y=2.5, z=2.0)),
            EqFinger(1000.0, "stale", 4, 1.5, 2.5, 2.0),
        )
        self.assertEqual(
            eq_latency(1000.0, lat(5.0, 12, [80.0, 90], 7.5)),
            EqLatency(1000.0, 12, (80.0, 90.0), 7.5),
        )
        self.assertEqual(
            eq_link(
                {
                    "ts": 2000.0,
                    "what": "rate",
                    "frame_trips": 3,
                    "frame_rtt_max_ms": 40.0,
                    "ack_waits": 2,
                }
            ),
            EqLink(2000.0, "rate", 3, 40.0, 2),
        )
        self.assertIsNone(eq_link({"ts": 2000.0, "what": "rate"}), "no trips: the holder was gone")
        self.assertEqual(
            samples_of([EqLatency(0.0, 1, (1.0, 2.0), None), EqLatency(1.0, 1, (3.0,), None)]),
            [1.0, 2.0, 3.0],
        )

    def test_malformed_fields_read_as_missing(self):
        bad = {"ev": "eq_touch", "t": 5.0, "what": 7, "p": True, "x": "1", "y": None, "z": [2]}
        self.assertEqual(eq_finger(1000.0, bad), EqFinger(1000.0, "?", None, None, None, None))
        self.assertEqual(
            eq_latency(
                1000.0, {"ev": "eq_lat", "t": 5.0, "f": "x", "ms": [1.0, "2", None, 3], "dec": "9"}
            ),
            EqLatency(1000.0, 0, (1.0, 3.0), None),
        )
        self.assertEqual(eq_latency(1000.0, {"ev": "eq_lat", "t": 5.0, "ms": 7}).samples, ())
        self.assertEqual(
            eq_link(
                {
                    "ts": 1.0,
                    "what": "frames",
                    "frame_trips": 2,
                    "frame_rtt_max_ms": "x",
                    "ack_waits": False,
                }
            ),
            EqLink(1.0, "frames", 2, None, 0),
        )


if __name__ == "__main__":
    unittest.main()
