"""The forensics timeline's channel detail (#71): the steps the page records
(``diag/trace.rs`` ``detail``) of a hold on a strip's burger and of the
detail it opens: ``press``, ``slid`` (the finger slid off: nothing opened),
``tap`` (the hint), ``open`` (``why``: the hold's ``check`` or a ``lift``
after it) and ``close`` (``why``: ``exit``, ``layout``, ``conflict``), each
with the strip's volume and mute keys and the finger's pointer. The report
lists them in a table of their own; stdout counts the presses, opens and
slides. The synthetic logs and the report reader come from
``test_timeline.py``."""

import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

from test_timeline import BASE, OFFSET, VOX, VOX_MUTE, Log, ReportCase, digest  # noqa: E402
from timeline_touch import DetailEvent, detail_event  # noqa: E402

KEYS = [VOX, VOX_MUTE]


def step(t, what, *, why=None, pointer=None):
    """A page ``detail`` event as ``diag/trace.rs`` writes it."""
    event = {"ev": "detail", "t": t, "what": what, "keys": KEYS}
    if why is not None:
        event["why"] = why
    if pointer is not None:
        event["pointer"] = pointer
    return event


def listed(page):
    """The report's detail rows (its table has the class too)."""
    return [
        a for tag, a in page.elements if tag == "tr" and "detail" in (a.get("class") or "").split()
    ]


class DetailSteps(ReportCase):
    def steps(self, log):
        """Pings of socket 7 and one trace of its page's detail steps, from
        page time p0 (hub BASE + 1000) on: a fader grab that slid off the
        burger, a tap, a hold that opened the detail, its exit, and a new
        layout that closed a detail."""
        log.pings(7, BASE, BASE + 4000)
        p0 = BASE + 1000 - OFFSET
        log.trace(
            BASE + 3000,
            7,
            [
                step(p0, "press", pointer=4),
                step(p0 + 40, "slid", pointer=4),
                step(p0 + 300, "press", pointer=5),
                step(p0 + 380, "tap", pointer=5),
                step(p0 + 600, "press", pointer=6),
                step(p0 + 1120, "open", why="check", pointer=6),
                step(p0 + 1700, "close", why="exit", pointer=7),
                step(p0 + 1900, "close", why="layout"),
            ],
        )

    def test_the_details_steps_are_listed_and_counted(self):
        log = Log()
        self.steps(log)
        summary, page, stdout = self.report(log, BASE, BASE + 5000)
        self.assertEqual(
            {name: summary[name] for name in ("detail_presses", "detail_opens", "detail_slid")},
            {"detail_presses": "3", "detail_opens": "1", "detail_slid": "1"},
        )
        rows = listed(page)
        self.assertEqual(
            [(r["data-what"], r.get("data-why")) for r in rows],
            [
                ("press", None),
                ("slid", None),
                ("press", None),
                ("tap", None),
                ("press", None),
                ("open", "check"),
                ("close", "exit"),
                ("close", "layout"),
            ],
            "in time order",
        )
        self.assertEqual(
            [r.get("data-pointer") for r in rows], ["4", "4", "5", "5", "6", "6", "7", None]
        )
        # Each tied to its channel: the strip's first key, hashed.
        self.assertEqual({r["data-key-hash"] for r in rows}, {digest(VOX)})
        # On the hub's clock: the page time plus the socket's offset.
        self.assertEqual(rows[0]["data-time"], f"{BASE + 1000:.1f}")
        self.assertEqual(rows[5]["data-time"], f"{BASE + 2120:.1f}")
        # The table names the channel (the report stays on the PC); stdout
        # never does.
        self.assertIn("Vox 1", page.text)
        self.assertNotIn("Vox 1", stdout)
        self.assertIn("detail_opens=1\n", stdout)

    def test_steps_outside_the_window_are_left_out(self):
        log = Log()
        self.steps(log)
        # The window ends between the tap and the third press.
        summary, page, _ = self.report(log, BASE, BASE + 1500)
        self.assertEqual(
            (summary["detail_presses"], summary["detail_opens"], summary["detail_slid"]),
            ("2", "0", "1"),
        )
        self.assertEqual([r["data-what"] for r in listed(page)], ["press", "slid", "press", "tap"])

    def test_without_steps_the_counts_are_0_and_the_report_says_so(self):
        log = Log()
        log.pings(7, BASE, BASE + 2000)
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(
            [summary[name] for name in ("detail_presses", "detail_opens", "detail_slid")],
            ["0", "0", "0"],
        )
        self.assertEqual(listed(page), [])
        self.assertIn("No channel detail step", page.text)


class DetailEventOf(unittest.TestCase):
    def test_a_detail_record_reads_its_fields(self):
        self.assertEqual(
            detail_event(1000.0, step(5.0, "open", why="lift", pointer=3)),
            DetailEvent(1000.0, "open", "lift", (VOX, VOX_MUTE), 3),
        )
        self.assertEqual(
            detail_event(1000.0, step(5.0, "close", why="conflict")),
            DetailEvent(1000.0, "close", "conflict", (VOX, VOX_MUTE), None),
        )

    def test_malformed_fields_read_as_missing(self):
        data = {"ev": "detail", "t": 5.0, "what": 7, "why": ["x"], "keys": VOX, "pointer": True}
        self.assertEqual(detail_event(1000.0, data), DetailEvent(1000.0, "?", None, (), None))
        mixed = {"ev": "detail", "t": 5.0, "what": "press", "keys": [VOX, 4, None], "pointer": "2"}
        self.assertEqual(
            detail_event(1000.0, mixed), DetailEvent(1000.0, "press", None, (VOX,), None)
        )


if __name__ == "__main__":
    unittest.main()
