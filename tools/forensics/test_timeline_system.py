"""The forensics timeline's system gestures (#43 PR G): what the browser or
the system did with a touch, recorded by the page (``diag/trace/sys.rs``) as
``sys`` events (a context menu, a selection, a drag, a pinch's start, a
cancelled pointer, a capture lost while the finger was still down; each with
the control's kind or the element's, its keys, the pointer and whether the
page prevented it) and ``zoom`` events (the visual viewport's scale when it
changes). The report lists them and marks them in the link lane; stdout
counts them. The synthetic logs and the report reader come from
``test_timeline.py``."""

import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

from test_timeline import BASE, OFFSET, VOX, VOX_MUTE, Log, ReportCase, digest  # noqa: E402
from timeline_touch import SystemEvent, system_event  # noqa: E402


def sys_event(t, what, on, *, keys=None, pointer=None, prevented=False):
    """A page ``sys`` event as ``diag/trace/sys.rs`` writes it."""
    event = {"ev": "sys", "t": t, "what": what, "on": on, "prevented": prevented}
    if keys is not None:
        event["keys"] = keys
    if pointer is not None:
        event["pointer"] = pointer
    return event


def listed(page):
    return [a for _, a in page.elements if "sys" in (a.get("class") or "").split()]


class SystemGestures(ReportCase):
    def gestures(self, log):
        """Pings of socket 7 and one trace of its page's system gestures,
        from page time p0 (hub BASE + 1000) on."""
        log.pings(7, BASE, BASE + 4000)
        p0 = BASE + 1000 - OFFSET
        log.trace(
            BASE + 3000,
            7,
            [
                sys_event(p0, "contextmenu", "fader", keys=[VOX], prevented=True),
                sys_event(p0 + 100, "selectstart", "row", prevented=True),
                sys_event(p0 + 200, "gesturestart", "strip-instance"),
                sys_event(p0 + 300, "pointercancel", "mute", keys=[VOX_MUTE], pointer=5),
                sys_event(p0 + 400, "lostpointercapture", "fader", keys=[VOX], pointer=6),
                {"ev": "zoom", "t": p0 + 500, "scale": 1.5},
                {"ev": "zoom", "t": p0 + 600, "scale": 1.0},
            ],
        )

    def test_the_pages_system_gestures_are_listed_counted_and_marked(self):
        log = Log()
        self.gestures(log)
        summary, page, stdout = self.report(log, BASE, BASE + 5000)
        self.assertEqual(summary["system_events"], "5")
        self.assertEqual(summary["system_not_prevented"], "3")
        self.assertEqual(summary["zooms"], "2")
        rows = listed(page)
        self.assertEqual(
            [(r["data-what"], r.get("data-on")) for r in rows],
            [
                ("contextmenu", "fader"),
                ("selectstart", "row"),
                ("gesturestart", "strip-instance"),
                ("pointercancel", "mute"),
                ("lostpointercapture", "fader"),
                ("zoom", None),
                ("zoom", None),
            ],
            "in time order",
        )
        self.assertEqual(
            [r.get("data-prevented") for r in rows],
            ["true", "true", "false", "false", "false", None, None],
        )
        self.assertEqual([r.get("data-pointer") for r in rows][3:5], ["5", "6"])
        self.assertEqual(rows[0]["data-key-hash"], digest(VOX))
        self.assertEqual(rows[3]["data-key-hash"], digest(VOX_MUTE))
        self.assertNotIn("data-key-hash", rows[1], "a row has no keys")
        self.assertEqual([r.get("data-scale") for r in rows][5:], ["1.5", "1"])
        # Each on the hub's clock: the page time plus the socket's offset.
        self.assertEqual(rows[0]["data-time"], f"{BASE + 1000:.1f}")
        # The table names the control (the report stays on the PC); stdout
        # never does.
        self.assertIn("Vox 1", page.text)
        self.assertNotIn("Vox 1", stdout)
        marks = page.of_class("gesture")
        self.assertEqual(len(marks), 7, "one mark each in the link lane")
        self.assertEqual(marks[0]["data-what"], "contextmenu")

    def test_gestures_outside_the_window_are_left_out(self):
        log = Log()
        self.gestures(log)
        # The window ends between the second and the third gesture.
        summary, page, _ = self.report(log, BASE, BASE + 1150)
        self.assertEqual(
            (summary["system_events"], summary["system_not_prevented"], summary["zooms"]),
            ("2", "0", "0"),
        )
        self.assertEqual([r["data-what"] for r in listed(page)], ["contextmenu", "selectstart"])

    def test_without_gestures_the_counts_are_0_and_the_report_says_so(self):
        log = Log()
        log.pings(7, BASE, BASE + 2000)
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(
            (summary["system_events"], summary["system_not_prevented"], summary["zooms"]),
            ("0", "0", "0"),
        )
        self.assertEqual(listed(page), [])
        self.assertIn("No system gesture", page.text)


class SystemEventOf(unittest.TestCase):
    def test_a_sys_record_reads_its_fields(self):
        data = sys_event(5.0, "dragstart", "pan", keys=[VOX], pointer=3, prevented=True)
        self.assertEqual(
            system_event(1000.0, data),
            SystemEvent(1000.0, "dragstart", "pan", (VOX,), 3, True, None),
        )

    def test_a_zoom_record_has_a_scale_and_nothing_else(self):
        self.assertEqual(
            system_event(1000.0, {"ev": "zoom", "t": 5.0, "scale": 2.25}),
            SystemEvent(1000.0, "zoom", None, (), None, None, 2.25),
        )

    def test_malformed_fields_read_as_missing(self):
        data = {
            "ev": "sys",
            "t": 5.0,
            "what": 7,
            "on": ["x"],
            "keys": "band|x|value",
            "pointer": "3",
            "prevented": "true",
        }
        self.assertEqual(
            system_event(1000.0, data),
            SystemEvent(1000.0, "?", None, (), None, False, None),
        )
        mixed = sys_event(5.0, "contextmenu", "fader", keys=[VOX, 4, None])
        self.assertEqual(system_event(1000.0, mixed).keys, (VOX,), "only the text keys")
        self.assertIsNone(system_event(1000.0, {"ev": "zoom", "t": 5.0}).scale)


if __name__ == "__main__":
    unittest.main()
