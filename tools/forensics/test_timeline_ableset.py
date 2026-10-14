"""The forensics timeline's AbleSet tab (#90, design §6): the hub's
``ableset_link`` records, read from the start of the window's first day
file, give AbleSet's state at the window's start, its outages and the link
at each of the page's steps (``ableset_view`` open and close, and the
``ableset`` probe, load, panel and sign-in events of ``diag/trace.rs``),
which the report lists and stdout counts. The synthetic logs and the report
reader come from ``test_timeline.py``."""

import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import timeline_read as read  # noqa: E402
from test_timeline import BASE, OFFSET, Log, ReportCase  # noqa: E402
from timeline_ableset import (  # noqa: E402
    PERIOD_MS,
    AbleSetLink,
    AbleSetOutage,
    AbleSetStep,
    link_at,
    links,
    outages,
    step,
)

MINUTE = 60_000
REFUSED = "AbleSet does not answer: connection refused"


def link(log, ts, up, failures, error=None):
    """A hub ``ableset_link`` record as ``ableset/link.rs`` ``link_fields``
    writes it."""
    if up:
        log.add("ableset_link", ts, up=True, failures=failures)
    else:
        log.add("ableset_link", ts, up=False, error=error, failures=failures)


def view(hub, on):
    return {"ev": "ableset_view", "t": hub - OFFSET, "on": on}


def ableset(hub, what, **fields):
    return dict({"ev": "ableset", "t": hub - OFFSET, "what": what}, **fields)


class AbleSetSection(ReportCase):
    def test_the_hubs_link_is_the_truth_beside_the_pages_steps(self):
        log = Log()
        # AbleSet went down 20 min before the window (the day file's
        # earlier part, past the 60 s lead) and comes back inside it.
        link(log, BASE - 40 * MINUTE, True, 0)
        link(log, BASE - 20 * MINUTE, False, 2, REFUSED)
        link(log, BASE + 3000, True, 241)
        log.pings(7, BASE, BASE + 9000)
        log.trace(
            BASE + 8500,
            7,
            [
                # Opened while AbleSet is down, then it is back.
                view(BASE + 1000, True),
                ableset(BASE + 1001, "panel", panel="down"),
                ableset(BASE + 3100, "panel", panel="checking"),
                ableset(BASE + 3140, "probe", ok=True, ms=35.0),
                ableset(BASE + 3600, "load", ms=420.0),
                view(BASE + 6000, False),
                # Down again with AbleSet up: the page's own socket.
                view(BASE + 6500, True),
                ableset(BASE + 6501, "panel", panel="down"),
                view(BASE + 7000, False),
            ],
        )
        summary, page, stdout = self.report(log, BASE, BASE + 9000)
        self.assertEqual(
            {k: v for k, v in summary.items() if k.startswith("ableset_")},
            {
                "ableset_link_at_start": "down",
                "ableset_link_outages": "1",
                "ableset_opens": "2",
                "ableset_probes_failed": "0",
                "ableset_probe_max_ms": "35.0",
                "ableset_load_max_ms": "420.0",
            },
        )
        self.assertEqual(page.of_class("ableset-start")[0]["data-link"], "down")
        self.assertEqual(
            [(a["data-up"], a["data-failures"]) for a in page.of_class("ableset-link")],
            [("true", "241")],
            "only the window's own change is a row",
        )
        self.assertEqual(
            [
                (a["data-what"], a.get("data-panel"), a.get("data-ms"), a["data-link"])
                for a in page.of_class("ableset-step")
            ],
            [
                ("open", None, None, "down"),
                ("panel", "down", None, "down"),
                ("panel", "checking", None, "up"),
                ("probe", None, "35.0", "up"),
                ("load", None, "420.0", "up"),
                ("close", None, None, "up"),
                ("open", None, None, "up"),
                ("panel", "down", None, "up"),
                ("close", None, None, "up"),
            ],
        )
        self.assertEqual(page.of_class("ableset-step")[3]["data-ok"], "true")
        # The outage from the down read before the lead to the up.
        [outage] = page.of_class("ableset-outage")
        self.assertEqual(
            (float(outage["data-ms"]), outage["data-read"]), (20 * MINUTE + 3000.0, "true")
        )
        self.assertIn(REFUSED, page.text)
        self.assertNotIn(REFUSED, stdout)

    def test_a_failed_probe_counts_and_an_up_alone_gives_its_outage(self):
        log = Log()
        # No change read before the window: AbleSet's state then is not
        # known; the up's 5 failed reads went down 4 periods before it.
        link(log, BASE + 2000, True, 5)
        log.pings(7, BASE, BASE + 12_000)
        log.trace(
            BASE + 11_600,
            7,
            [
                view(BASE + 3000, True),
                ableset(BASE + 3001, "panel", panel="checking"),
                ableset(BASE + 11_001, "probe", ok=False, ms=8000.0),
                ableset(BASE + 11_002, "panel", panel="signin"),
                ableset(BASE + 11_500, "signin"),
            ],
        )
        summary, page, _ = self.report(log, BASE, BASE + 12_000)
        self.assertEqual(
            (
                summary["ableset_link_at_start"],
                summary["ableset_probes_failed"],
                summary["ableset_probe_max_ms"],
                summary["ableset_load_max_ms"],
            ),
            ("n/a", "1", "8000.0", "n/a"),
        )
        [outage] = page.of_class("ableset-outage")
        self.assertEqual((float(outage["data-ms"]), outage["data-read"]), (4 * PERIOD_MS, "false"))
        steps = page.of_class("ableset-step")
        self.assertEqual(
            [(a["data-what"], a.get("data-ok")) for a in steps][2:],
            [("probe", "false"), ("panel", None), ("signin", None)],
        )

    def test_no_link_change_and_no_step_says_so(self):
        log = Log()
        log.pings(7, BASE, BASE + 3000)
        summary, page, _ = self.report(log, BASE, BASE + 3000)
        self.assertEqual((summary["ableset_link_at_start"], summary["ableset_opens"]), ("n/a", "0"))
        self.assertIn("No AbleSet link change and no step of AbleSet's tab", page.text)
        self.assertEqual(page.of_class("ableset-step"), [])


class Reading(ReportCase):
    def test_link_changes_are_read_from_the_previous_day_files_start(self):
        log = Log()
        # The hub writes only changes, so the previous UTC date's file is read
        # too, for the link only: its other early lines are never parsed.
        link(log, BASE - 17 * 60 * MINUTE, False, 2, REFUSED)
        log.add(
            "set", BASE - 17 * 60 * MINUTE, client=1, instance="band", key="k", seq=1, value=0.5
        )
        link(log, BASE - 3 * 60 * MINUTE, True, 3)
        log.add("set", BASE - 3 * 60 * MINUTE, client=1, instance="band", key="k", seq=2, value=0.5)
        log.write(self.logs)
        start = BASE
        records, files, missing, skipped = read.read_events(
            self.logs, start - MINUTE, start + 1000, start + 31 * MINUTE
        )
        self.assertEqual(files, ["events-2026-10-02.jsonl", "events-2026-10-03.jsonl"])
        self.assertEqual(missing, [])
        self.assertEqual(
            [(r["ev"], r["ts"]) for r in records],
            [("ableset_link", BASE - 17 * 60 * MINUTE), ("ableset_link", BASE - 3 * 60 * MINUTE)],
        )
        self.assertEqual(skipped, 0)

    def test_an_outage_that_began_the_day_before_is_the_state_at_the_start(self):
        log = Log()
        # Down before UTC midnight and not back: no change on the window's day.
        link(log, BASE - 17 * 60 * MINUTE, False, 2, REFUSED)
        log.write(self.logs)
        records, _, missing, _ = read.read_events(
            self.logs, BASE - MINUTE, BASE + 1000, BASE + 31 * MINUTE
        )
        # The window's own day file is missing; the previous one only the
        # link reaches is not.
        self.assertEqual(missing, ["events-2026-10-03.jsonl"])
        self.assertIs(link_at(links(records), BASE), False)
        self.assertEqual(links(records)[0].error, REFUSED)

    def test_a_missing_previous_day_file_is_no_note(self):
        log = Log()
        link(log, BASE - 3 * 60 * MINUTE, True, 3)
        log.write(self.logs)
        _, files, missing, _ = read.read_events(
            self.logs, BASE - MINUTE, BASE + 1000, BASE + 31 * MINUTE
        )
        self.assertEqual((files, missing), (["events-2026-10-03.jsonl"], []))


class Pure(unittest.TestCase):
    def test_links_skip_a_record_without_a_bool_up(self):
        self.assertEqual(
            links(
                [
                    {"ts": 1, "up": True, "failures": 0},
                    {"ts": 2, "up": "no", "failures": 2},
                    {"ts": 3, "up": False, "error": 7, "failures": True},
                ]
            ),
            [AbleSetLink(1, True, None, 0), AbleSetLink(3, False, None, None)],
        )

    def test_link_at_takes_the_latest_change_at_or_before(self):
        changes = [AbleSetLink(10, False, "x", 2), AbleSetLink(20, True, None, 3)]
        self.assertEqual(
            [link_at(changes, ms) for ms in (9, 10, 19, 20, 99)],
            [None, False, False, True, True],
        )
        self.assertIsNone(link_at([], 5))

    def test_outages(self):
        up_after = lambda ts, n: AbleSetLink(ts, True, None, n)  # noqa: E731
        # An up from not known after one failed read was never down.
        self.assertEqual(outages([up_after(50_000, 1)], 0, 100_000), [])
        self.assertEqual(
            outages([up_after(50_000, 2)], 0, 100_000),
            [AbleSetOutage(45_000, 50_000, 5000.0, None, False)],
        )
        # A down with no up lasts to the window's end; a second down (a
        # restarted hub) does not start another; one ended before the
        # window is left out.
        changes = [
            AbleSetLink(1000, False, "a", 2),
            up_after(2000, 2),
            AbleSetLink(5000, False, "b", 2),
            AbleSetLink(7000, False, "c", 2),
        ]
        self.assertEqual(outages(changes, 3000, 9000), [AbleSetOutage(5000, 9000, 4000, "b", True)])
        self.assertEqual(
            outages(changes, 2000, 9000),
            [
                AbleSetOutage(1000, 2000, 1000, "a", True),
                AbleSetOutage(5000, 9000, 4000, "b", True),
            ],
        )

    def test_a_step_of_the_wrong_shape_reads_as_missing(self):
        self.assertEqual(
            step(5.0, {"ev": "ableset_view", "t": 1.0, "on": 1}, []),
            AbleSetStep(5.0, "?", None, None, None, None),
        )
        self.assertEqual(
            step(5.0, {"ev": "ableset", "what": 3, "ok": 1, "ms": "8", "panel": 2}, []),
            AbleSetStep(5.0, "?", None, None, None, None),
        )


if __name__ == "__main__":
    unittest.main()
