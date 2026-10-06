"""The forensics timeline's Stream Deck section (#52): the hub's
``deck_press`` records with Companion's answer from their ``deck_ok``
(matched by client and seq: round trip, ok, error), its ``deck_release``
records with theirs (a ``deck_ok`` without a client, by key and order), its
``deck_link`` and ``deck_key`` records, and the page's ``deck`` events
(``diag/trace.rs``), listed in the report and counted on stdout, numbers
only. The synthetic logs and the report reader come from
``test_timeline.py``."""

import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

from test_timeline import BASE, OFFSET, Log, ReportCase  # noqa: E402
from timeline_touch import (  # noqa: E402
    DeckOutage,
    deck_key_changes,
    deck_outages,
    deck_presses,
    deck_releases,
    in_press_window,
)

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


def answer(log, ts, client, key, down, seq, *, ok=True, error=None, rtt_ms=5.0):
    """Companion's answer as a hub ``deck_ok`` record (``client`` and ``seq``
    None for the hub's own release)."""
    log.add(
        "deck_ok",
        ts,
        client=client,
        seq=seq,
        key=key,
        down=down,
        ok=ok,
        error=error,
        rtt_ms=rtt_ms,
    )


def key_state(log, ts, key, *, pressed, img, color="#00aa00"):
    """A hub ``deck_key`` record (the image as its hash only)."""
    log.add(
        "deck_key",
        ts,
        key=key,
        pressed=pressed,
        color=color,
        img_hash=img,
        img_bytes=700,
        changes=1,
    )


def release(log, ts, key, reason, client=None):
    """A hub ``deck_release`` record."""
    log.add("deck_release", ts, client=client, key=key, reason=reason, hub_hold_ms=None)


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

    def test_a_down_the_hub_refused_as_late_is_a_red_flash(self):
        log = Log()
        log.pings(7, BASE, BASE + 3000)
        # A down that waited in a stalled link (the hub refused it, late),
        # then its up (not held).
        press(log, BASE + 1000, 7, 9, True, 1, forwarded=False, reason="late")
        press(log, BASE + 1200, 7, 9, False, 2, forwarded=False, reason="not held", why="up")
        summary, page, _ = self.report(log, BASE, BASE + 3000)
        self.assertEqual((summary["deck_presses"], summary["deck_unsent"]), ("1", "1"))
        self.assertEqual(
            [(a["data-key"], a["data-where"]) for a in page.of_class("deck-unsent")],
            [("9", "hub")],
        )
        self.assertEqual(page.of_class("deck-press")[0]["data-reason"], "late")

    def test_companions_answer_shows_on_each_press_and_a_refused_down_flashes(self):
        log = Log()
        log.pings(7, BASE, BASE + 6000)
        # Companion refuses a down; the link goes with a down and its up on
        # the way (both answered offline); a down answered ok; a down whose
        # answer was not read.
        press(log, BASE + 1000, 7, 3, True, 1)
        answer(log, BASE + 1004, 7, 3, True, 1, ok=False, error="Invalid KEY", rtt_ms=4.0)
        press(log, BASE + 2000, 7, 4, True, 2)
        press(log, BASE + 2100, 7, 4, False, 3, why="up", hold=100.0)
        answer(log, BASE + 2500, 7, 4, True, 2, ok=False, error="offline", rtt_ms=None)
        answer(log, BASE + 2501, 7, 4, False, 3, ok=False, error="offline", rtt_ms=None)
        press(log, BASE + 3000, 7, 5, True, 4)
        answer(log, BASE + 3003, 7, 5, True, 4)
        press(log, BASE + 4000, 7, 6, True, 5)
        summary, page, stdout = self.report(log, BASE, BASE + 6000)
        self.assertEqual((summary["deck_presses"], summary["deck_unsent"]), ("4", "2"))
        presses = page.of_class("deck-press")
        self.assertEqual(
            [
                (a["data-key"], a["data-down"], a.get("data-ok"), a.get("data-error"))
                for a in presses
            ],
            [
                ("3", "true", "false", "Invalid KEY"),
                ("4", "true", "false", "offline"),
                ("4", "false", "false", "offline"),
                ("5", "true", "true", None),
                ("6", "true", None, None),
            ],
        )
        for text in ("refused: Invalid KEY", "no answer read"):
            self.assertIn(text, page.text)
        # Only the downs answered not ok flash; the up's refusal never does.
        self.assertEqual(
            [
                (a["data-key"], a["data-where"], a.get("data-why"))
                for a in page.of_class("deck-unsent")
            ],
            [("3", "companion", "Invalid KEY"), ("4", "companion", "offline")],
        )
        self.assertNotIn("Invalid KEY", stdout)

    def test_each_hub_release_shows_whether_companion_got_it(self):
        log = Log()
        log.pings(7, BASE - 1000, BASE + 9000)
        # A release just before the window owns the answer after it.
        release(log, BASE - 100, 5, "silent", client=7)
        answer(log, BASE + 500, None, 5, False, None)
        release(log, BASE + 1000, 5, "detach", client=7)
        # A page's own up of the key is answered too: not the hub's.
        answer(log, BASE + 1001, 7, 5, False, 9, ok=False, error="Invalid KEY")
        answer(log, BASE + 1003, None, 5, False, None)
        release(log, BASE + 5000, 5, "reconnect")
        answer(log, BASE + 5100, None, 5, False, None, ok=False, error="offline", rtt_ms=None)
        release(log, BASE + 6000, 8, "stop")
        release(log, BASE + 6000, 9, "lost")
        # A later answer of key 8 is not the stop's (it gets none).
        release(log, BASE + 7000, 8, "reconnect")
        answer(log, BASE + 7002, None, 8, False, None)
        summary, page, _ = self.report(log, BASE, BASE + 9000)
        self.assertEqual(summary["deck_forced_releases"], "5")
        self.assertEqual(
            [
                (a["data-key"], a["data-reason"], a.get("data-ok"), a.get("data-error"))
                for a in page.of_class("deck-release")
            ],
            [
                ("5", "detach", "true", None),
                ("5", "reconnect", "false", "offline"),
                ("8", "stop", None, None),
                ("9", "lost", None, None),
                ("8", "reconnect", "true", None),
            ],
        )
        for text in ("detach: ok", "reconnect: offline", "lost: never released"):
            self.assertIn(text, page.text)

    def test_companions_key_changes_within_ten_seconds_of_a_press_are_listed(self):
        log = Log()
        log.pings(7, BASE, BASE + 12000)
        key_state(log, BASE + 500, 3, pressed=False, img="aaaa")
        press(log, BASE + 1000, 7, 3, True, 1)
        key_state(log, BASE + 1010, 3, pressed=True, img="bbbb")
        key_state(log, BASE + 1500, 3, pressed=True, img="bbbb", color="#ff0000")
        # A key nobody pressed, and one whose press was not forwarded.
        key_state(log, BASE + 1600, 4, pressed=False, img="cccc")
        press(log, BASE + 2000, 7, 7, True, 2, forwarded=False, reason="held")
        key_state(log, BASE + 2010, 7, pressed=True, img="dddd")
        # The hub's own release reached Companion too.
        release(log, BASE + 3000, 9, "reconnect")
        key_state(log, BASE + 3020, 9, pressed=False, img="eeee")
        # 10 s after the press: still listed; a millisecond later: not.
        key_state(log, BASE + 11000, 3, pressed=False, img="aaaa")
        key_state(log, BASE + 11001, 3, pressed=False, img="aaaa")
        _, page, stdout = self.report(log, BASE, BASE + 12000)
        keys = page.of_class("deck-key")
        self.assertEqual(
            [
                (
                    a["data-key"],
                    a.get("data-pressed"),
                    a.get("data-img-changed"),
                    float(a["data-since-press"]),
                )
                for a in keys
            ],
            [
                ("3", "true", "true", 10.0),
                ("3", "true", "false", 500.0),
                ("9", "false", None, 20.0),
                ("3", "false", "true", 10000.0),
            ],
        )
        self.assertEqual(keys[1]["data-color"], "#ff0000")
        self.assertIn("pressed, #00aa00, image changed", page.text)
        self.assertNotIn("#00aa00", stdout)

    def test_a_window_without_the_deck_says_so(self):
        log = Log()
        log.pings(7, BASE, BASE + 1000)
        summary, page, _ = self.report(log, BASE, BASE + 2000)
        self.assertEqual(
            (summary["deck_presses"], summary["deck_unsent"], summary["deck_rtt_p50_ms"]),
            ("0", "0", "n/a"),
        )
        self.assertIn("No Stream Deck activity in the window.", page.text)

    def test_an_answer_just_after_the_window_still_gives_the_round_trip(self):
        log = Log()
        log.pings(7, BASE, BASE + 3000)
        press(log, BASE + 990, 7, 3, True, 1)
        ok(log, BASE + 1010, 7, 3, True, 1, 20.0)
        summary, page, _ = self.report(log, BASE, BASE + 1000)
        self.assertEqual(summary["deck_rtt_p50_ms"], "20.0")
        self.assertEqual(float(page.of_class("deck-press")[0]["data-rtt"]), 20.0)

    def test_an_outage_from_before_the_lead_is_rebuilt_from_the_up_records_down_ms(self):
        log = Log()
        log.pings(7, BASE, BASE + 9000)
        # The down record is far before the window; only the up is read.
        link(
            log,
            BASE + 4000,
            "up",
            companion="5.0.7",
            api="1.12.0",
            error=None,
            down_ms=500000.0,
            attempts=3,
        )
        summary, page, _ = self.report(log, BASE, BASE + 9000)
        self.assertEqual(summary["deck_link_outages"], "1")
        self.assertEqual(float(page.of_class("deck-outage")[0]["data-ms"]), 500000.0)


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
        # An up whose down was not read: its down_ms gives the start.
        self.assertEqual(
            deck_outages([{"ts": 40, "state": "up", "down_ms": 25.0}], 0, 100),
            [DeckOutage(15, 40, 25.0, None)],
        )
        self.assertEqual(deck_outages([{"ts": 40, "state": "up", "down_ms": None}], 0, 100), [])
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
        self.assertEqual([(p.ok, p.error) for p in got], [(None, None), (None, None)])

    def test_a_hub_release_takes_its_keys_next_answer_in_order(self):
        def rel(ts, key, reason="detach"):
            return {"ts": ts, "key": key, "reason": reason, "client": 7}

        def ans(ts, key, ok=True, client=None, down=False):
            return {"ts": ts, "key": key, "ok": ok, "client": client, "down": down}

        # Both answers after both releases: the first answer is the first's.
        got = deck_releases([rel(10, 2), rel(20, 2)], [ans(30, 2, ok=False), ans(31, 2, ok=True)])
        self.assertEqual([r.ok for r in got], [False, True])
        # A page's answer, a down's answer and another key's are not taken.
        got = deck_releases(
            [rel(10, 2)],
            [ans(11, 2, client=7), ans(12, 2, down=True), ans(13, 3), ans(14, 2, ok=False)],
        )
        self.assertEqual([r.ok for r in got], [False])
        # An answer exactly at the release counts; one before it does not.
        self.assertEqual([r.ok for r in deck_releases([rel(10, 2)], [ans(10, 2)])], [True])
        self.assertEqual([r.ok for r in deck_releases([rel(10, 2)], [ans(9, 2)])], [None])
        # Lost and stop releases take none.
        got = deck_releases([rel(10, 2, "lost"), rel(11, 2, "stop")], [ans(12, 2)])
        self.assertEqual([r.ok for r in got], [None, None])

    def test_a_key_change_takes_its_keys_latest_press_at_or_before_it(self):
        def rec(ts, key):
            return {"ts": ts, "key": key, "pressed": True, "color": None, "img_hash": "h"}

        # Presses in any order, of several keys; one at the record's own ms
        # counts (0 ms), one after it does not.
        presses = [(30_000, 1), (100, 1), (5_000, 2), (20_000, 1), (25_000, 1)]
        got = deck_key_changes(
            [rec(50, 1), rec(5_000, 2), rec(19_000, 1), rec(25_000, 1), rec(26_000, 3)],
            presses,
            0,
            40_000,
        )
        self.assertEqual(
            [(c.time, c.key, c.since_press_ms) for c in got],
            [(5_000, 2, 0), (25_000, 1, 0)],
        )
        # Outside the window: left out, but still the image's previous record.
        got = deck_key_changes([rec(150, 1), rec(200, 1)], [(100, 1)], 160, 400)
        self.assertEqual([(c.time, c.img_changed) for c in got], [(200, False)])

    def test_a_key_change_counts_up_to_ten_seconds_after_a_press(self):
        self.assertTrue(in_press_window(0.0))
        self.assertTrue(in_press_window(10_000.0))
        self.assertFalse(in_press_window(10_000.001))
        self.assertFalse(in_press_window(-0.001))


if __name__ == "__main__":
    unittest.main()
