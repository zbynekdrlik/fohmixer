"""The forensics timeline's values, times and hub log lines (#43,
``timeline_read``): TouchOSC's dB curve, the local time it reads and writes,
and the two hub log messages it reads."""

import calendar
import datetime
import math
import os
import sys
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import timeline_read as read  # noqa: E402

# 2026-10-03 16:20:00 UTC.
BASE = calendar.timegm((2026, 10, 3, 16, 20, 0)) * 1000


def utc_stamp(ms):
    """A hub log line's stamp (``tracing_subscriber::fmt``: RFC 3339, µs, Z)."""
    moment = datetime.datetime.fromtimestamp(ms / 1000.0, datetime.UTC)
    return moment.strftime("%Y-%m-%dT%H:%M:%S.%fZ")


class ValueToDb(unittest.TestCase):
    def test_the_touchosc_curve_in_each_range(self):
        # The table of `value2db_is_the_touchosc_curve_in_each_range` (fader/tests.rs).
        for v, db in (
            (1.0, 6.0),
            (0.85, 0.0),
            (0.4, -18.0),
            (0.39999, -18.001762236950533),
            (0.2, -34.39049787108893),
            (0.15, -40.986236048588935),
            (0.14999, -41.01368891610561),
            (0.1, -48.54258355581338),
            (1e-6, -69.995809918992),
            (1.5, 0.0),
        ):
            self.assertAlmostEqual(read.value2db(v), db, places=9, msg=str(v))
        self.assertEqual(read.value2db(0.0), float("-inf"))
        self.assertEqual(read.value2db(float("nan")), 0.0)
        self.assertTrue(math.isnan(read.value2db(-0.1)), "Rust's powf of a negative: NaN")


class Times(unittest.TestCase):
    def test_every_form_reads_the_same_local_time(self):
        minute = read.parse_local("2026-10-03 18:21")
        expected = time.mktime((2026, 10, 3, 18, 21, 0, 0, 0, -1)) * 1000
        self.assertEqual(minute, round(expected), "this machine's local time")
        for text in ("2026-10-03T18:21", "2026-10-03 18:21:00", "2026-10-03T18:21:00.000"):
            self.assertEqual(read.parse_local(text), minute, text)
        self.assertEqual(read.parse_local("2026-10-03 18:21:05.5") - minute, 5500)
        self.assertEqual(read.parse_local("2026-10-03 18:21:05.125") - minute, 5125)
        self.assertEqual(
            read.parse_local(" 2026-10-03 9:05 "), read.parse_local("2026-10-03 09:05")
        )

    def test_a_time_alone_is_today(self):
        day = datetime.date(2026, 10, 3)
        self.assertEqual(
            read.parse_local("18:21:05.125", today=day),
            read.parse_local("2026-10-03 18:21:05.125"),
        )
        self.assertEqual(read.parse_local("18:21", today=day), read.parse_local("2026-10-03 18:21"))
        now = datetime.datetime.now()
        self.assertEqual(
            read.parse_local("00:00"),
            read.parse_local(now.strftime("%Y-%m-%d") + " 00:00"),
            "today by default",
        )

    def test_local_text_round_trips_and_is_the_local_clock(self):
        for ms in (BASE, BASE + 123, BASE + 86_399_999, BASE + 7):
            self.assertEqual(read.parse_local(read.local_text(ms)), ms)
        seconds = (BASE + 123) // 1000
        expected = time.strftime("%Y-%m-%d %H:%M:%S", time.localtime(seconds)) + ".123"
        self.assertEqual(read.local_text(BASE + 123), expected)
        self.assertEqual(read.utc_text(BASE + 45), "2026-10-03 16:20:00.045Z")

    def test_a_bad_time_is_refused(self):
        for text in (
            "18",
            "25:00",
            "2026-13-01 10:00",
            "2026-10-03 18:21:61",
            "yesterday",
            "18:21:05.1234567",
            "2026-10-03  18:21",
        ):
            with self.assertRaises(read.TimelineError, msg=text):
                read.parse_local(text)


class HubLog(unittest.TestCase):
    def test_hub_log_lines(self):
        ansi = (
            f"\x1b[2m{utc_stamp(BASE + 1000.25)}\x1b[0m \x1b[32m INFO\x1b[0m "
            "\x1b[2mfohmixer_hub::live::client\x1b[0m\x1b[2m:\x1b[0m Live busy changed "
            '\x1b[3minstance\x1b[0m\x1b[2m=\x1b[0m"band" \x1b[3mbusy\x1b[0m\x1b[2m=\x1b[0mtrue'
        )
        self.assertEqual(read.hub_log_mark(ansi), ("busy", BASE + 1000.25, "band", True))
        late_reason = (
            f"{utc_stamp(BASE)}  INFO fohmixer_hub::live::client: Live busy changed "
            'instance="master" busy=false main_tick_age_ms=12 reason="a heartbeat 600 ms after '
            'the previous one (or the connect): the script made it at its tick"'
        )
        self.assertEqual(read.hub_log_mark(late_reason), ("busy", BASE, "master", False))
        late = (
            f"{utc_stamp(BASE + 2)}  WARN fohmixer_hub::live::client: a heartbeat 470 ms after "
            "the previous one (or the connect): the script made it at its tick, 455 ms after "
            'its previous one, it spent 2 ms on the way instance="band" main_tick_age_ms=12'
        )
        self.assertEqual(read.hub_log_mark(late), ("late", BASE + 2, "band", 470.0))
        for other in (
            f"{utc_stamp(BASE)}  INFO fohmixer_hub: Starting fohmixer-hub v0.1.0",
            "Live busy changed busy=true",
            "",
        ):
            self.assertIsNone(read.hub_log_mark(other), other)


if __name__ == "__main__":
    unittest.main()
