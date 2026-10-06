"""The forensics timeline's Stream Deck section (#52): the hub's
``deck_press`` records with Companion's round trip from their ``deck_ok``
(matched by client and seq), its ``deck_release`` and ``deck_link`` records,
and the page's ``deck`` events (``diag/trace.rs``), listed in the report and
counted on stdout, numbers only. The synthetic logs and the report reader
come from ``test_timeline.py``."""

import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

from test_timeline import BASE, OFFSET, Log, ReportCase  # noqa: E402
from timeline_touch import DeckOutage, deck_outages, deck_presses  # noqa: E402

PEER = "192.0.2.10"


def press(log, ts, client, key, down, seq, *, forwarded=True, reason=None, why=None, hold=None):
    """A hub ``deck_press`` record as ``deck.rs`` ``press_fields`` writes it."""
    log.add(
        "deck_press",
        ts,
        client=client,
        peer=PEER,
        key=key,
        down=down,
        seq=seq,
        t=ts - OFFSET - 4.0,
        hub_ms=float(ts),
        offset_ms=OFFSET,
        delay_ms=4.0,
        gap_ms=None,
        hold_ms=hold,
        why=why,
        hub_hold_ms=None if hold is None else hold - 4.0,
        forwarded=forwarded,
        reason=reason,
        holders=1 if down else 0,
    )


def ok(log, ts, client, key, down, seq, rtt_ms):
    """Companion's answer, a hub ``deck_ok`` record."""
    log.add(
        "deck_ok",
        ts,
        client=client,
        seq=seq,
        key=key,
        down=down,
        ok=True,
        error=None,
        rtt_ms=rtt_ms,
    )


def link(log, ts, state, **fields):
    log.add("deck_link", ts, state=state, **fields)


class DeckSection(ReportCase):
    def deck(self, log):
        """Socket 7's page: key 3 held 1.5 s; Companion away 2 s, a press
        refused then; key 5 released by the hub (silent), key 8 lost; a down the page
        could not send (key 6)."""
        log.pings(7, BASE, BASE + 9000)
        link(
            log,
            BASE + 100,
            "up",
            companion="5.0.7",
            api="1.12.0",
            error=None,
            down_ms=None,
            attempts=1,
        )
        press(log, BASE + 1000, 7, 3, True, 1)
        ok(log, BASE + 1005, 7, 3, True, 1, 5.0)
        press(log, BASE + 2500, 7, 3, False, 2, why="up", hold=1500.0)
        ok(log, BASE + 2512, 7, 3, False, 2, 12.0)
        link(
            log,
            BASE + 3000,
            "down",
            companion=None,
            api=None,
            error="Companion closed the connection",
        )
        press(log, BASE + 3500, 7, 4, True, 3, forwarded=False, reason="offline")
        link(
            log,
            BASE + 5000,
            "up",
            companion="5.0.7",
            api="1.12.0",
            error=None,
            down_ms=2000.0,
            attempts=1,
        )
        press(log, BASE + 6000, 7, 5, True, 4)
        ok(log, BASE + 6004, 7, 5, True, 4, 4.0)
        log.add("deck_release", BASE + 8000, client=7, key=5, reason="silent", hub_hold_ms=2000.0)
        # A key the hub could not release: it stopped while Companion was away.
        log.add("deck_release", BASE + 8200, client=None, key=8, reason="lost", hub_hold_ms=None)
        page = -OFFSET
        log.trace(
            BASE + 8500,
            7,
            [
                {"ev": "deck", "t": BASE + 996 + page, "k": 3, "d": 1, "sent": True, "q": 1},
                {
                    "ev": "deck",
                    "t": BASE + 2496 + page,
                    "k": 3,
                    "d": 0,
                    "h": 1500,
                    "why": "up",
                    "sent": True,
                    "q": 2,
                },
                {"ev": "deck", "t": BASE + 7000 + page, "k": 6, "d": 1, "sent": False},
            ],
        )

    def test_presses_releases_flashes_and_outages_are_listed_and_counted(self):
        log = Log()
        self.deck(log)
        summary, page, stdout = self.report(log, BASE, BASE + 9000)
        self.assertEqual(
            {
                k: summary[k]
                for k in (
                    "deck_presses",
                    "deck_unsent",
                    "deck_forced_releases",
                    "deck_link_outages",
                    "deck_rtt_p50_ms",
                    "deck_rtt_p99_ms",
                )
            },
            {
                "deck_presses": "3",
                "deck_unsent": "2",
                "deck_forced_releases": "2",
                "deck_link_outages": "1",
                "deck_rtt_p50_ms": "5.0",
                "deck_rtt_p99_ms": "12.0",
            },
        )
        presses = page.of_class("deck-press")
        self.assertEqual(
            [(a["data-key"], a["data-down"], a["data-forwarded"]) for a in presses],
            [
                ("3", "true", "true"),
                ("3", "false", "true"),
                ("4", "true", "false"),
                ("5", "true", "true"),
            ],
        )
        self.assertEqual(float(presses[1]["data-rtt"]), 12.0)
        self.assertEqual(presses[1]["data-why"], "up")
        self.assertEqual(presses[2]["data-reason"], "offline")
        self.assertNotIn("data-rtt", presses[2], "a refused press has no round trip")
        self.assertEqual(
            [(a["data-key"], a["data-where"]) for a in page.of_class("deck-unsent")],
            [("4", "hub"), ("6", "page")],
        )
        self.assertEqual(
            [(a["data-key"], a["data-reason"]) for a in page.of_class("deck-release")],
            [("5", "silent"), ("8", "lost")],
        )
        # Only the lost one never reached Companion.
        self.assertEqual(
            [a.get("data-released") for a in page.of_class("deck-release")], ["true", "false"]
        )
        outages = page.of_class("deck-outage")
        self.assertEqual(len(outages), 1)
        self.assertEqual(float(outages[0]["data-ms"]), 2000.0)
        # stdout names no key and no peer.
        self.assertNotIn(PEER, stdout)
        self.assertNotIn(PEER, page.text)

    def test_a_window_without_the_deck_says_so(self):
        log = Log()
        log.pings(7, BASE, BASE + 1000)
        summary, page, _ = self.report(log, BASE, BASE + 2000)
        self.assertEqual(
            (summary["deck_presses"], summary["deck_unsent"], summary["deck_rtt_p50_ms"]),
            ("0", "0", "n/a"),
        )
        self.assertIn("No Stream Deck activity in the window.", page.text)


class DeckHelpers(unittest.TestCase):
    def test_an_outage_runs_from_the_first_down_to_the_next_up(self):
        def at(ts, state):
            return {"ts": ts, "state": state, "error": f"e{ts}"}

        self.assertEqual(
            deck_outages([at(10, "down"), at(30, "up")], 0, 100),
            [DeckOutage(10, 30, 20, "e10")],
        )
        # Refused attempts during one outage: one outage, from the first.
        self.assertEqual(
            deck_outages([at(10, "refused"), at(15, "refused"), at(40, "up")], 0, 100),
            [DeckOutage(10, 40, 30, "e10")],
        )
        # Still down at the window's end.
        self.assertEqual(deck_outages([at(10, "down")], 0, 50), [DeckOutage(10, 50, 40, "e10")])
        self.assertEqual(deck_outages([at(5, "up")], 0, 50), [])
        # Before the window: left out; across its start: kept.
        self.assertEqual(deck_outages([at(10, "down"), at(20, "up")], 30, 100), [])
        self.assertEqual(
            deck_outages([at(-100, "down"), at(20, "up")], 0, 100),
            [DeckOutage(-100, 20, 120, "e-100")],
        )

    def test_a_press_gets_its_own_answer_by_client_and_seq(self):
        presses = [
            {"ts": 100, "client": 7, "seq": 1, "key": 3, "down": True, "forwarded": True},
            {
                "ts": 110,
                "client": 9,
                "seq": 1,
                "key": 3,
                "down": True,
                "forwarded": False,
                "reason": "held",
            },
        ]
        answers = [
            # An older answer of the same seq (a page counts from 1 again).
            {"ts": 50, "client": 7, "seq": 1, "rtt_ms": 99.0},
            {"ts": 105, "client": 9, "seq": 1, "rtt_ms": 77.0},
            {"ts": 106, "client": 7, "seq": 1, "rtt_ms": 6.0},
        ]
        got = deck_presses(presses, answers)
        self.assertEqual([p.rtt_ms for p in got], [6.0, None])
        self.assertEqual([p.reason for p in got], [None, "held"])


if __name__ == "__main__":
    unittest.main()
