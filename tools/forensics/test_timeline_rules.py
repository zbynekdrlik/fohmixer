"""The forensics timeline's rules (#130 PR 6): the page's ``rail`` events
(``diag/trace.rs`` ``rail``) and the hub's ``transport``, ``stage_aut``
(``router.rs``), ``reverb`` (``router/reverb.rs``, ``rules/reverb.rs``
``Step``), ``fader_link`` (``router/links.rs``) and ``autotune``
(``rules/autotune.rs`` ``Step``) records in one table, in time order, and
their stdout counts. The synthetic logs and the report reader come from
``test_timeline.py``; invented track names only."""

import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

from test_timeline import BASE, OFFSET, PEER, VOX, VOX_MUTE, Log, ReportCase, digest  # noqa: E402
from timeline_rules import RuleRow, generic_detail, hub_row, rail_row, rows, summary  # noqa: E402

STAGE_KEY = "band|live_set tracks[name=Stage Mics] |mute"
REFUSED = "Live refused: the track is frozen"


def rail(hub, what, button, keys=(), pointer=3):
    """A page ``rail`` event as ``diag::trace::rail`` writes it."""
    return {
        "ev": "rail",
        "t": hub - OFFSET,
        "what": what,
        "button": button,
        "keys": list(keys),
        "pointer": pointer,
    }


def counts(summary_pairs):
    names = (
        "rule_rail_steps",
        "rule_records",
        "reverb_lowerings",
        "reverb_windows",
        "stage_writes",
        "transport_changes",
        "rule_failures",
    )
    return {name: summary_pairs[name] for name in names}


class RulesSection(ReportCase):
    def test_stage_and_its_transport_read_in_one_table(self):
        log = Log()
        log.pings(7, BASE, BASE + 9000)
        # A record before the window (in the lead) is no row.
        log.add("transport", BASE - 30_000, instance="band", playing=True, prev=None)
        # The merged STAGE button: press, the full mark, the hold that acted.
        log.trace(
            BASE + 8000,
            7,
            [
                rail(BASE + 1000, "press", "stage", [STAGE_KEY]),
                rail(BASE + 2000, "full", "stage", [STAGE_KEY]),
                rail(BASE + 2400, "hold", "stage", [STAGE_KEY]),
            ],
        )
        log.add("stage_aut", BASE + 2400, step="flag", on=True, client=7, peer=PEER)
        # Live stops: the transport, then STAGE AUT's write, at one ms; the
        # write fails.
        log.add("transport", BASE + 5000, instance="band", playing=False, prev=True)
        log.add("stage_aut", BASE + 5000, step="write", instance="band", mute=True)
        log.add("stage_aut", BASE + 5040, step="failed", target="Stage Mics", problem=REFUSED)
        # A page turns STAGE AUT off, and the rules go blind in the same ms:
        # the log's order holds (the flag first), whatever the kinds.
        log.add("stage_aut", BASE + 7000, step="flag", on=False, client=7, peer=PEER)
        log.add("transport", BASE + 7000, instance="band", error="the listener closed")
        summary_pairs, page, stdout = self.report(log, BASE, BASE + 9000)
        self.assertEqual(
            counts(summary_pairs),
            {
                "rule_rail_steps": "3",
                "rule_records": "6",
                "reverb_lowerings": "0",
                "reverb_windows": "0",
                "stage_writes": "1",
                "transport_changes": "1",
                "rule_failures": "2",
            },
        )
        found = page.of_class("rule")
        self.assertEqual(
            [(a["data-source"], a["data-kind"], a["data-what"], a.get("data-step")) for a in found],
            [
                ("page", "rail", "press", None),
                ("page", "rail", "full", None),
                ("page", "rail", "hold", None),
                ("hub", "stage_aut", "flag", "flag"),
                ("hub", "transport", "stop", None),
                ("hub", "stage_aut", "write", "write"),
                ("hub", "stage_aut", "failed", "failed"),
                ("hub", "stage_aut", "flag", "flag"),
                ("hub", "transport", "error", None),
            ],
        )
        self.assertEqual(
            [float(a["data-time"]) for a in found],
            [BASE + 1000.0, BASE + 2000.0, BASE + 2400.0, BASE + 2400.0]
            + [BASE + 5000.0, BASE + 5000.0, BASE + 5040.0, BASE + 7000.0, BASE + 7000.0],
        )
        self.assertEqual(
            [(a["data-button"], a["data-key-hash"], a["data-pointer"]) for a in found[:3]],
            [("stage", digest(STAGE_KEY), "3")] * 3,
        )
        self.assertEqual(
            [a["data-failed"] for a in found],
            ["false"] * 6 + ["true", "false", "true"],
        )
        self.assertEqual(
            [("failed" in a["class"].split()) for a in found],
            [False] * 6 + [True, False, True],
        )
        self.assertIn("band: stopped (before: playing)", page.text)
        self.assertIn("band: the stage mics muted", page.text)
        self.assertIn(f"Stage Mics: {REFUSED}", page.text)
        # stdout: counts only, no label, key, error or address.
        for hidden in ("Stage Mics", REFUSED, "listener", "band"):
            self.assertNotIn(hidden, stdout)
        self.assertNotIn(PEER.split(":")[0], page.text, "no address in the report")

    def test_every_reverb_step_is_a_row(self):
        log = Log()
        log.pings(7, BASE, BASE + 9000)
        log.trace(BASE + 8500, 7, [rail(BASE + 500, "tap", "reverb")])
        log.add("reverb", BASE + 500, step="tap", low=True, client=7, peer=PEER)
        glide = {"label": "Vox 1", "from": 0.85, "to": 0.6, "ms": 1500.0}
        log.add("reverb", BASE + 501, step="lower", why="tap", faders=[glide])
        log.add("reverb", BASE + 501, step="skipped", label="Klavir #", why="track list")
        log.add("reverb", BASE + 3000, step="window", instance="band", open=True, why="tracks")
        log.add("reverb", BASE + 3500, step="return", faders=[])
        log.add("reverb", BASE + 3500, step="pending", label="Vox 1", why="track list")
        log.add("reverb", BASE + 4000, step="window", instance="band", open=False, why="settled")
        # A stop with every fader in the hand: a lowering of no fader.
        log.add("reverb", BASE + 6000, step="lower", why="stop", faders=[])
        log.add("fader_link", BASE + 7000, instance="band", id="REV", on=False, client=7)
        log.add(
            "autotune",
            BASE + 7500,
            step="gave_up",
            instance="band",
            target="Vox 1",
            problem="Live did not answer",
            tries=3,
        )
        log.add(
            "autotune",
            BASE + 7600,
            step="write",
            instance="band",
            cause="transport",
            on=["Vox 1"],
            off=[],
        )
        # A control's own set: no rule record, no row.
        log.add("set", BASE + 7700, client=7, instance="band", key=VOX, seq=1, value=0.5)
        summary_pairs, page, stdout = self.report(log, BASE, BASE + 9000)
        self.assertEqual(
            counts(summary_pairs),
            {
                "rule_rail_steps": "1",
                "rule_records": "11",
                "reverb_lowerings": "2",
                "reverb_windows": "1",
                "stage_writes": "0",
                "transport_changes": "0",
                "rule_failures": "1",
            },
        )
        found = page.of_class("rule")
        self.assertEqual(
            [(a["data-kind"], a["data-what"]) for a in found],
            [
                ("rail", "tap"),
                ("reverb", "tap"),
                ("reverb", "lower"),
                ("reverb", "skipped"),
                ("reverb", "open"),
                ("reverb", "return"),
                ("reverb", "pending"),
                ("reverb", "end"),
                ("reverb", "lower"),
                ("fader_link", "off"),
                ("autotune", "gave_up"),
                ("autotune", "write"),
            ],
        )
        self.assertEqual(
            [a.get("data-step") for a in found if a["data-kind"] == "reverb"][3:8],
            ["window", "return", "pending", "window", "lower"],
        )
        self.assertEqual(found[0]["data-button"], "reverb")
        self.assertNotIn("data-key-hash", found[0], "REVERB's press writes no key")
        for text in (
            "by tap: Vox 1 0.850 → 0.600 in 1500 ms",
            "Klavir #: track list",
            "band: the window opens (tracks)",
            "band: the window ends (settled)",
            "by stop: no fader",
            "Vox 1: track list",
            "band REV: off",
            "instance=band problem=Live did not answer target=Vox 1 tries=3",
            "cause=transport instance=band off=none on=Vox 1",
        ):
            self.assertIn(text, page.text)
        for hidden in ("Vox 1", "Klavir", "REV", "Live did not answer"):
            self.assertNotIn(hidden, stdout)

    def test_the_key_filter_keeps_every_row(self):
        log = Log()
        log.pings(7, BASE, BASE + 3000)
        log.trace(BASE + 2500, 7, [rail(BASE + 500, "tap", "stage", [STAGE_KEY])])
        log.add("transport", BASE + 1000, instance="band", playing=True, prev=None)
        summary_pairs, page, _ = self.report(log, BASE, BASE + 3000, VOX_MUTE)
        self.assertEqual(
            (summary_pairs["rule_rail_steps"], summary_pairs["rule_records"]), ("1", "1")
        )
        self.assertEqual(summary_pairs["transport_changes"], "1")
        self.assertIn("band: playing (before: not known)", page.text)

    def test_no_step_and_no_record_says_so(self):
        log = Log()
        log.pings(7, BASE, BASE + 3000)
        summary_pairs, page, _ = self.report(log, BASE, BASE + 3000)
        self.assertEqual(set(counts(summary_pairs).values()), {"0"})
        self.assertIn("No rail step and no rule record in the window.", page.text)
        self.assertEqual(page.of_class("rule"), [])


class Pure(unittest.TestCase):
    def test_a_rail_event_of_the_wrong_shape_reads_as_missing(self):
        self.assertEqual(
            rail_row(5.0, {"ev": "rail", "what": 1, "button": 2, "keys": "k", "pointer": True}),
            RuleRow(5.0, "page", "rail", "?", None, None, (), None, "", False),
        )
        self.assertEqual(
            rail_row(5.0, {"ev": "rail", "what": "tap", "keys": ["a", 3, "b"], "pointer": 4}),
            RuleRow(5.0, "page", "rail", "tap", None, None, ("a", "b"), 4, "a, b", False),
        )

    def test_a_hub_record_of_the_wrong_shape_reads_as_missing(self):
        self.assertEqual(
            hub_row({"ev": "transport", "ts": 1, "playing": "yes", "instance": 2}),
            RuleRow(1, "hub", "transport", "?", None, None, (), None, "n/a: n/a", False),
        )
        # An error that is no text is no error.
        self.assertEqual(hub_row({"ev": "transport", "ts": 1, "error": 5}).what, "?")
        self.assertEqual(hub_row({"ev": "fader_link", "ts": 1, "on": 1}).what, "?")
        row = hub_row({"ev": "stage_aut", "ts": 1, "step": 3})
        self.assertEqual((row.what, row.step, row.failed), ("?", "?", False))
        self.assertEqual(
            hub_row({"ev": "reverb", "ts": 1, "step": "window", "open": "yes"}).what, "?"
        )
        # A glide whose fields do not read is left out.
        self.assertEqual(
            hub_row(
                {"ev": "reverb", "ts": 1, "step": "lower", "why": "stop", "faders": [{"label": 1}]}
            ).detail,
            "by stop: no fader",
        )
        self.assertEqual(
            hub_row({"ev": "reverb", "ts": 1, "step": "lower", "faders": 3}).detail,
            "by n/a: n/a",
        )

    def test_failures(self):
        failed = [
            hub_row(dict(r, ts=1)).failed
            for r in (
                {"ev": "stage_aut", "step": "failed"},
                {"ev": "stage_aut", "step": "write"},
                {"ev": "autotune", "step": "failed"},
                {"ev": "autotune", "step": "gave_up"},
                {"ev": "autotune", "step": "missing"},
                {"ev": "reverb", "step": "failed"},
                {"ev": "transport", "error": "x"},
                {"ev": "transport", "playing": True},
            )
        ]
        self.assertEqual(failed, [True, False, True, True, False, False, True, False])

    def test_the_generic_detail_never_shows_the_frame_or_the_address(self):
        self.assertEqual(
            generic_detail(
                {
                    "ev": "autotune",
                    "ts": 1,
                    "step": "key",
                    "client": 7,
                    "peer": PEER,
                    "song": 3,
                    "key": None,
                    "nested": {"a": 1},
                    "ms": 12.5,
                    "on": True,
                }
            ),
            "ms=12.5 on=true song=3",
        )

    def test_rows_put_a_page_step_before_the_hubs_records_and_keep_the_log_order(self):
        records = [
            {"ev": "stage_aut", "ts": 10, "step": "flag", "on": True},
            {"ev": "transport", "ts": 10, "playing": True},
            {"ev": "reverb", "ts": 5, "step": "aut", "on": True},
        ]
        found = rows(records, [(10.0, {"what": "hold"}), (4.0, {"what": "press"})])
        self.assertEqual(
            [(r.time, r.kind, r.what) for r in found],
            [
                (4.0, "rail", "press"),
                (5, "reverb", "aut"),
                (10.0, "rail", "hold"),
                (10, "stage_aut", "flag"),
                (10, "transport", "play"),
            ],
        )

    def test_summary_counts(self):
        found = rows(
            [
                {"ev": "reverb", "ts": 1, "step": "window", "open": True},
                {"ev": "reverb", "ts": 2, "step": "window", "open": False},
                {"ev": "reverb", "ts": 3, "step": "lower", "faders": []},
                {"ev": "transport", "ts": 4, "playing": False, "prev": None},
                {"ev": "transport", "ts": 5, "error": "x"},
                {"ev": "stage_aut", "ts": 6, "step": "write", "mute": False},
            ],
            [(0.0, {"what": "tap"})],
        )
        self.assertEqual(
            dict(summary(found)),
            {
                "rule_rail_steps": "1",
                "rule_records": "6",
                "reverb_lowerings": "1",
                "reverb_windows": "1",
                "stage_writes": "1",
                "transport_changes": "1",
                "rule_failures": "1",
            },
        )


if __name__ == "__main__":
    unittest.main()
