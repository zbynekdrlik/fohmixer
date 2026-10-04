"""The forensics timeline (#43) through its command (``timeline``): whole
reports of synthetic event logs written into temporary folders, invented
track names only, the synthetic fixture's. The pure helpers' tests are in
``test_timeline_read.py`` and ``test_timeline_model.py``."""

import calendar
import contextlib
import datetime
import hashlib
import html.parser
import io
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import timeline  # noqa: E402
import timeline_read as read  # noqa: E402

# The hub's clock at the start of every scenario: 2026-10-03 16:20:00 UTC.
BASE = calendar.timegm((2026, 10, 3, 16, 20, 0)) * 1000
# The page clock's offset (hub − page, ms) its pings report.
OFFSET = 250.5
VOX = "band|live_set tracks[name=Vox 1] mixer_device volume|value"
HAND = "band|live_set tracks[name=Hand2 #] mixer_device volume|value"
KLAVIR_PAN = "band|live_set tracks[name=Klavir #] mixer_device panning|value"
VOX_MUTE = "band|live_set tracks[name=Vox 1]|mute"
KLAVIR_PARAM = "band|live_set tracks[name=Klavir #] devices 0 parameters 1|value"
PEER = "192.0.2.10:50000"


def digest(key):
    return hashlib.sha256(key.encode("utf-8")).hexdigest()[:10]


def utc_stamp(ms):
    """A hub log line's stamp (``tracing_subscriber::fmt``: RFC 3339, µs, Z)."""
    moment = datetime.datetime.fromtimestamp(ms / 1000.0, datetime.UTC)
    return moment.strftime("%Y-%m-%dT%H:%M:%S.%fZ")


class Log:
    """A synthetic hub event log and hub text log, written as the hub writes
    them (records in ``ts`` order, one day file per UTC date)."""

    def __init__(self):
        self.records = []
        self.lines = {}
        self.batches = {}

    def add(self, ev, ts, **fields):
        self.records.append(dict(fields, ev=ev, ts=int(ts)))

    def pings(self, client, start, end, offset=OFFSET, rtt=12.0):
        """A ping of ``client`` every 100 ms from ``start`` to ``end`` (hub ms)."""
        for n, hub in enumerate(range(int(start), int(end) + 1, 100), start=1):
            self.add(
                "ping",
                hub,
                client=client,
                peer=PEER,
                n=n,
                t=hub - offset,
                hub_ms=float(hub),
                rtt=rtt,
                rtt_n=n - 1,
                offset_ms=offset,
            )

    def trace(self, ts, client, events):
        self.add("trace", ts, client=client, peer=PEER, events=events)

    def drag(self, key, client, sends, *, rtt_ms=10.0, offset=OFFSET, arrive=None, final=False):
        """The hub's records of a page's ``sends`` ((page t, value), in send
        order): each reaches the hub at ``arrive(its send on the hub clock)``
        (3 ms later by default); the setter writes the newest set whenever no
        batch is in flight, and Live answers each batch ``rtt_ms`` later.
        ``final``: every send is final (a toggle's), else none is (a drag's);
        None leaves the field out. Returns the page's ``send`` events for its
        trace."""
        instance = key.split("|")[0]
        arrive = arrive or (lambda sent: sent + 3.0)
        arrivals = sorted(
            (arrive(t + offset), seq, value, t) for seq, (t, value) in enumerate(sends, start=1)
        )
        previous = None
        for hub_ms, seq, value, t in arrivals:
            self.add(
                "set",
                hub_ms,
                client=client,
                peer=PEER,
                instance=instance,
                key=key,
                seq=seq,
                value=value,
                **({} if final is None else {"final": final}),
                t=t,
                hub_ms=hub_ms,
                offset_ms=offset,
                delay_ms=hub_ms - (t + offset),
                gap_ms=None if previous is None else hub_ms - previous,
                dropped_old=False,
                unknown=False,
            )
            previous = hub_ms
        free = float("-inf")
        i = 0
        while i < len(arrivals):
            begin = max(arrivals[i][0], free)
            j = i
            while j + 1 < len(arrivals) and arrivals[j + 1][0] <= begin:
                j += 1
            _, seq, value, _ = arrivals[j]
            self.write_batch(instance, key, client, seq, value, begin, rtt_ms)
            free = begin + rtt_ms
            i = j + 1
        return page_sends(key, sends, final)

    def write_batch(self, instance, key, client, seq, value, begin, rtt_ms, error=None):
        """One batch of one write and Live's answer ``rtt_ms`` later (with
        ``error``: Live refused it)."""
        number = self.batches[instance] = self.batches.get(instance, 0) + 1
        item = {"key": key, "client": client, "seq": seq}
        self.add(
            "batch", begin, instance=instance, batch=number, n=1, sent=[dict(item, value=value)]
        )
        self.add(
            "applied",
            begin + rtt_ms,
            instance=instance,
            batch=number,
            n=1,
            rtt_ms=rtt_ms,
            errors=0 if error is None else 1,
            sent=[item],
        )
        self.add(
            "ack",
            begin + rtt_ms,
            client=client,
            peer=PEER,
            instance=instance,
            key=key,
            seq=seq,
            value=value,
            error=error,
            superseded=False,
            batch=number,
            rtt_ms=rtt_ms,
        )

    def line(self, text, name="hub.out.log"):
        self.lines.setdefault(name, []).append(text)

    def write(self, folder):
        days = {}
        for record in sorted(self.records, key=lambda r: r["ts"]):
            day = datetime.datetime.fromtimestamp(record["ts"] / 1000.0, datetime.UTC).date()
            days.setdefault(day, []).append(json.dumps(record, separators=(",", ":")))
        for day, lines in days.items():
            path = os.path.join(folder, f"events-{day.isoformat()}.jsonl")
            with open(path, "a", encoding="utf-8") as f:
                f.write("".join(line + "\n" for line in lines))
        for name, lines in self.lines.items():
            with open(os.path.join(folder, name), "a", encoding="utf-8") as f:
                f.write("".join(line + "\n" for line in lines))


def page_sends(key, sends, final=False):
    """The page's ``send`` events of ``sends`` ((page t, value), seq from 1);
    ``final`` None leaves the field out."""
    return [
        {"ev": "send", "t": t, "key": key, "seq": seq, "value": v, "sent": True}
        | ({} if final is None else {"final": final})
        for seq, (t, v) in enumerate(sends, start=1)
    ]


class Page(html.parser.HTMLParser):
    """The report read back: its elements, its summary cells, and every end
    tag that did not close the element open last."""

    VOID = {"meta", "br", "hr", "img", "input", "link", "col", "area", "base", "wbr"}

    def __init__(self, text):
        super().__init__()
        self.text = text
        self.elements = []
        self.cells = {}
        self.problems = []
        self.stack = []
        self._cell = None
        self.feed(text)
        self.close()

    def handle_starttag(self, tag, attrs):
        attributes = dict(attrs)
        self.elements.append((tag, attributes))
        if tag not in self.VOID:
            self.stack.append(tag)
        if "data-k" in attributes:
            self._cell = attributes["data-k"]
            self.cells[self._cell] = ""

    def handle_startendtag(self, tag, attrs):
        self.elements.append((tag, dict(attrs)))

    def handle_endtag(self, tag):
        if not self.stack or self.stack[-1] != tag:
            self.problems.append(f"</{tag}> closes {self.stack[-1:]}")
        else:
            self.stack.pop()
        if tag == "td":
            self._cell = None

    def handle_data(self, data):
        if self._cell is not None:
            self.cells[self._cell] += data

    def of_class(self, name):
        return [a for _, a in self.elements if name in (a.get("class") or "").split()]


def run_tool(*argv):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = timeline.main(list(argv))
    return code, out.getvalue(), err.getvalue()


def parse_summary(stdout):
    return dict(line.split("=", 1) for line in stdout.splitlines())


class ReportCase(unittest.TestCase):
    """A temporary ``logs`` folder and a report next to it."""

    def setUp(self):
        folder = tempfile.TemporaryDirectory(prefix="fohmixer-timeline-test-")
        self.addCleanup(folder.cleanup)
        self.logs = os.path.join(folder.name, "logs")
        os.makedirs(self.logs)
        self.out = os.path.join(folder.name, "report.html")

    def report(self, log, start, end, *keys):
        """Writes ``log``, runs the tool from ``start`` to ``end`` (hub ms):
        (summary, the report read back, stdout)."""
        log.write(self.logs)
        argv = ["--events", self.logs, "--out", self.out]
        argv += ["--from", read.local_text(start), "--to", read.local_text(end)]
        for key in keys:
            argv += ["--key", key]
        code, stdout, stderr = run_tool(*argv)
        self.assertEqual((code, stderr), (0, ""))
        with open(self.out, encoding="utf-8") as f:
            page = Page(f.read())
        self.assertEqual((page.problems, page.stack), ([], []), "the report is well formed")
        return parse_summary(stdout), page, stdout

    def gaps(self, page, line, key):
        return [
            a
            for a in page.of_class("gap")
            if a["data-line"] == line and a["data-key-hash"] == digest(key)
        ]

    def jumps(self, page):
        return [
            (a["data-cause"], a["data-key-hash"]) for _, a in page.elements if "data-cause" in a
        ]


# --- the command ---


class Command(unittest.TestCase):
    def test_a_bad_command_line_exits_1_with_one_line(self):
        with tempfile.TemporaryDirectory() as folder:
            out = os.path.join(folder, "r.html")
            base = ["--events", folder, "--out", out]
            for argv, why in (
                (base + ["--from", "noon", "--to", "18:21"], "cannot read the time 'noon'"),
                (base + ["--from", "18:21", "--to", "18:20"], "must be after --from"),
                (base + ["--from", "18:21", "--to", "18:21"], "must be after --from"),
                (["--events", folder], "the following arguments are required"),
                (
                    ["--events", os.path.join(folder, "none"), "--out", out]
                    + ["--from", "18:20", "--to", "18:21"],
                    "no such folder",
                ),
            ):
                code, stdout, stderr = run_tool(*argv)
                self.assertEqual((code, stdout), (1, ""), argv)
                self.assertTrue(stderr.startswith("timeline: "), stderr)
                self.assertIn(why, stderr)
                self.assertEqual(stderr.count("\n"), 1, "one line")
            self.assertFalse(os.path.exists(out), "nothing was written")


# --- whole reports ---


def stall_drag(log):
    """A drag of Vox 1 through a 600 ms stall on the way to the hub: the page
    sends every 16 ms for 2 s (the value up 0.004 a send); what it sends from
    hub 1.5 s on is held 600 ms, then arrives in a burst; the page reports the
    silence as a dropout. Returns the page's dropout event."""
    log.pings(7, BASE, BASE + 4000)
    # Another page with another clock, pinging closer to the trace: a mapping
    # with its offset is wrong.
    log.pings(8, BASE + 50, BASE + 4050, offset=900.0)
    p0 = BASE + 1000 - OFFSET
    sends = [(p0 + 16 * i, round(0.5 + 0.004 * i, 6)) for i in range(125)]
    stall, held = BASE + 1500.0, 600.0

    def arrive(sent):
        if stall <= sent < stall + held:
            return stall + held + (sent - stall) / 1000.0
        return sent + 3.0

    events = log.drag(VOX, 7, sends, rtt_ms=10.0, arrive=arrive)
    dropout = {
        "ev": "dropout",
        "t": stall - OFFSET + 4,
        "ms": 590.0,
        "socket_lost": False,
        "rtts": [12.0, 11.5],
    }
    touch = {"ev": "touch", "keys": [VOX], "pointer": 1}
    events = [
        dict(touch, t=p0 - 30, what="down"),
        *events,
        dropout,
        dict(touch, t=p0 + 16 * 125, what="up"),
    ]
    log.trace(BASE + 3550, 7, events)
    return dropout


class Stall(ReportCase):
    def test_a_drag_through_a_stall_marks_the_arrival_gap_and_blames_the_link(self):
        log = Log()
        dropout = stall_drag(log)
        summary, page, stdout = self.report(log, BASE, BASE + 5000)
        (arrival,) = self.gaps(page, "arrival", VOX)
        self.assertAlmostEqual(float(arrival["data-ms"]), 601.0, delta=0.1)
        self.assertAlmostEqual(float(arrival["data-start"]), BASE + 1499.0, delta=0.05)
        self.assertEqual(self.gaps(page, "send", VOX), [], "the page kept sending")
        self.assertEqual(len(self.gaps(page, "applied", VOX)), 1)
        (rect,) = page.of_class("dropout")
        self.assertAlmostEqual(float(rect["data-start"]), dropout["t"] + OFFSET, delta=0.05)
        self.assertAlmostEqual(float(rect["data-end"]), dropout["t"] + OFFSET + 590, delta=0.05)
        self.assertEqual((rect["data-ms"], rect["data-socket-lost"]), ("590.0", "false"))
        self.assertEqual(self.jumps(page), [("link", digest(VOX))])
        self.assertEqual(summary["jumps"], "1")
        self.assertEqual(summary["gap_send_max_ms"], "0")
        self.assertEqual(summary["gap_arrival_max_ms"], "601.0")
        self.assertEqual(summary["dropouts"], "1")
        self.assertEqual(summary["longest_dropout_ms"], "590.0")
        self.assertEqual(summary["longest_dropout_at"], read.local_text(dropout["t"] + OFFSET))
        self.assertEqual(summary["sockets"], "1", "client 8 only pinged")
        # The worst confirmation is the first set the stall held (sent at
        # 1512 ms, applied at 2110 ms); the sets the setter left out have none.
        self.assertEqual(summary["worst_confirmation_key"], f"key#{digest(VOX)}")
        self.assertEqual(summary["worst_confirmation_ms"], "598.0")
        self.assertEqual(summary["worst_confirmation_at"], read.local_text(BASE + 1512))
        applied = [r for r in log.records if r["ev"] == "applied"]
        self.assertEqual(summary["confirmation_n"], str(len(applied)))
        self.assertLess(len(applied), 125)
        # The page shows the real key; stdout never does.
        self.assertIn("Vox 1", page.text)
        self.assertNotIn("Vox 1", stdout)
        self.assertNotIn("tracks[", stdout)
        self.assertIn(f"worst_confirmation_key=key#{digest(VOX)}\n", stdout)
        self.assertNotIn("192.0.2.10", page.text, "no address in the report")

    def test_the_summary_table_holds_stdouts_values(self):
        log = Log()
        stall_drag(log)
        summary, page, stdout = self.report(log, BASE, BASE + 5000)
        self.assertEqual(page.cells, summary)
        self.assertEqual(list(page.cells), [line.split("=")[0] for line in stdout.splitlines()])
        self.assertEqual(
            list(summary),
            [
                "records",
                "sockets",
                "confirmation_n",
                "confirmation_p50_ms",
                "confirmation_p90_ms",
                "confirmation_p99_ms",
                "worst_confirmation_ms",
                "worst_confirmation_at",
                "worst_confirmation_key",
                "dropouts",
                "longest_dropout_ms",
                "longest_dropout_at",
                "resets",
                "rtt_page_p50_ms",
                "rtt_page_max_ms",
                "rtt_hub_p50_ms",
                "rtt_hub_max_ms",
                "gap_send_max_ms",
                "gap_arrival_max_ms",
                "gap_applied_max_ms",
                "busy",
                "busy_longest_ms",
                "late_heartbeats",
                "jumps",
                "touches",
                "first_touch_jumps",
                "first_move_p50_ms",
                "first_move_max_ms",
                "first_send_p50_ms",
                "first_send_max_ms",
                "move_gaps",
                "move_gap_max_ms",
                "held_runs",
                "no_data_spans",
                "recorder_dropped",
                "unconfirmed",
                "not_sent",
                "skipped_lines",
                "notes",
            ],
        )
        self.assertEqual(summary["rtt_hub_p50_ms"], "12.0")
        self.assertEqual(summary["notes"], "1", "no hub log: one note")


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
        self.assertIn("dropped 1 long frames stamped from", page.text)
        self.assertIn("no data of page stalls from", page.text)
        self.assertAlmostEqual(float(band["data-start"]), p0 + 112 - 50 + OFFSET, delta=0.05)

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


class Confirmations(ReportCase):
    def test_confirmation_latency_percentiles(self):
        log = Log()
        latencies = [5, 10, 15, 20, 25, 30, 35, 40, 45, 300]
        for i, latency in enumerate(latencies):
            sent = BASE + 1000 * (i + 1)
            # The hub's arrival is 5 ms after the send: a latency taken from
            # the arrival would be 5 ms short.
            offset = None if latency == 300 else OFFSET
            log.add(
                "set",
                sent + 5,
                client=3,
                instance="band",
                key=HAND,
                seq=i + 1,
                value=0.5,
                t=sent - OFFSET,
                hub_ms=float(sent + 5),
                offset_ms=offset,
            )
            begin = sent + 5
            applied = (sent + 5 if offset is None else sent) + latency
            log.write_batch("band", HAND, 3, i + 1, 0.5, begin, applied - begin)
        # A superseded write is never applied: no confirmation.
        log.add("set", BASE + 11005, client=3, instance="band", key=HAND, seq=11, value=0.6)
        log.add("ack", BASE + 11006, client=3, instance="band", key=HAND, seq=11, superseded=True)
        # A write Live refused is not applied either.
        log.add("set", BASE + 11105, client=3, instance="band", key=HAND, seq=12, value=0.6)
        log.write_batch("band", HAND, 3, 12, 0.6, BASE + 11105, 7, error="refused")
        summary, _, _ = self.report(log, BASE, BASE + 20000)
        self.assertEqual(summary["confirmation_n"], "10")
        self.assertEqual(summary["confirmation_p50_ms"], "25.0")
        self.assertEqual(summary["confirmation_p90_ms"], "45.0")
        self.assertEqual(summary["confirmation_p99_ms"], "300.0")
        self.assertEqual(summary["worst_confirmation_ms"], "300.0")
        self.assertEqual(summary["worst_confirmation_at"], read.local_text(BASE + 10005))
        self.assertEqual(summary["worst_confirmation_key"], f"key#{digest(HAND)}")
        self.assertEqual(summary["sockets"], "1")

    def test_the_key_filter_keeps_only_matching_controls(self):
        log = Log()
        p0 = BASE + 1000 - OFFSET
        log.drag(VOX, 7, [(p0, 0.5), (p0 + 16, 0.6)])
        log.drag(HAND, 7, [(p0, 0.5), (p0 + 16, 0.9), (p0 + 32, 0.1)])
        summary, page, _ = self.report(log, BASE, BASE + 5000, "Vox")
        self.assertEqual([a["data-key-hash"] for a in page.of_class("control")], [digest(VOX)])
        self.assertEqual(summary["confirmation_n"], "2")
        self.assertNotIn("Hand2", page.text)
        self.assertEqual(summary["records"], str(len(log.records)), "the count stays whole")


class PageEvents(ReportCase):
    def test_a_resent_batch_counts_once(self):
        log = Log()
        log.pings(7, BASE, BASE + 2000)
        log.pings(9, BASE + 2400, BASE + 3000)
        events = [
            {
                "ev": "dropout",
                "t": BASE + 1200 - OFFSET,
                "ms": 800.0,
                "socket_lost": True,
                "rtts": [],
            },
            {"ev": "reset", "t": BASE + 1900 - OFFSET, "count": 3, "active": False},
        ]
        log.trace(BASE + 2000, 7, events)
        # The same batch again after the reconnect, its keys in another order.
        log.trace(BASE + 2500, 9, [dict(reversed(list(e.items()))) for e in events])
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual((summary["dropouts"], summary["resets"]), ("1", "1"))
        self.assertEqual(len(page.of_class("dropout")), 1)
        self.assertEqual(page.of_class("dropout")[0]["data-socket-lost"], "true")
        self.assertEqual(len(page.of_class("reset")), 1)

    def test_the_link_lane_marks(self):
        log = Log()
        log.pings(7, BASE, BASE + 3000, rtt=20.0)
        log.add("sock", BASE + 500, what="open", client=7, peer=PEER, reason=None)
        log.add("sock", BASE + 2800, what="close", client=7, peer=PEER, reason="went away")
        p0 = BASE + 1000 - OFFSET
        sends = log.drag(KLAVIR_PAN, 7, [(p0, 0.1), (p0 + 16, 0.2)])
        unsent = dict(sends[-1], t=p0 + 40, seq=3, value=0.3, sent=False)
        log.trace(
            BASE + 2900,
            7,
            [
                *sends,
                unsent,
                # The page records pongs only (no ping events): its RTT line.
                {"ev": "pong", "t": p0 + 100, "n": 4, "rtt": 30.0},
                {"ev": "pong", "t": p0 + 200, "n": 5, "rtt": 50.0},
                {"ev": "sock", "t": p0 + 1500, "what": "close", "code": 1006, "reason": "lost"},
                {"ev": "frame", "t": p0 + 300, "ms": 120.0},
                {"ev": "visibility", "t": p0 + 900, "hidden": True},
                {"ev": "overflow", "t": p0 + 950, "n": 2},
            ],
        )
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertIn("the page flight recorder dropped 2 events", page.text)
        self.assertEqual(summary["notes"], "2", "the overflow and no hub log")
        self.assertEqual(len(page.of_class("sock")), 3, "two hub records, one page event")
        self.assertEqual(len(page.of_class("frame")), 1)
        self.assertEqual(len(page.of_class("visibility")), 1)
        self.assertEqual(len(page.of_class("unsent")), 1, "the unsent send is drawn hollow")
        (rtt_page,) = page.of_class("rtt-page")
        self.assertEqual(len(rtt_page["points"].split()), 2, "one point per pong")
        self.assertEqual(len(page.of_class("rtt-hub")), 1)
        self.assertEqual((summary["rtt_page_p50_ms"], summary["rtt_page_max_ms"]), ("30.0", "50.0"))
        self.assertEqual((summary["rtt_hub_p50_ms"], summary["rtt_hub_max_ms"]), ("20.0", "20.0"))
        self.assertIn("page socket close: code 1006, lost", page.text)
        self.assertIn("hub socket close (client 7): went away", page.text)
        self.assertEqual(summary["jumps"], "0", "a pan has no dB jumps")

    def test_without_any_ping_the_page_clock_is_not_aligned(self):
        log = Log()
        log.trace(BASE + 2000, 7, [{"ev": "reset", "t": BASE + 1500, "count": 1, "active": False}])
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(summary["resets"], "1")
        self.assertIn("page clock not aligned", page.text)


class LiveLane(ReportCase):
    def test_busy_episodes_from_the_event_log_and_the_hub_logs(self):
        log = Log()
        log.add("link", BASE + 1000, instance="band", busy=True, tick_age_ms=470, reason="slow")
        log.add("link", BASE + 1500, instance="band", busy=False, tick_age_ms=12, reason="ok")
        target = "fohmixer_hub::live::client"
        log.line(
            f"{utc_stamp(BASE + 1000.456)}  INFO {target}: Live busy changed "
            'instance="band" busy=true main_tick_age_ms=470 reason="Live\'s main thread '
            'last ticked 470 ms before the last heartbeat"'
        )
        log.line(
            f"{utc_stamp(BASE + 1500.456)}  INFO {target}: Live busy changed "
            'instance="band" busy=false main_tick_age_ms=12 reason="a heartbeat 600 ms after '
            'the previous one (or the connect): the script made it at its tick"'
        )
        log.line(
            f"{utc_stamp(BASE + 2500)}  WARN {target}: a heartbeat 470 ms after the previous "
            "one (or the connect): the script made it at its tick, 455 ms after its previous "
            'one, it spent 2 ms on the way instance="band" main_tick_age_ms=12'
        )
        log.line(f"{utc_stamp(BASE - 120000)}  INFO {target}: Live busy changed busy=true")
        log.line(f"{utc_stamp(BASE)}  INFO fohmixer_hub: Starting fohmixer-hub v0.1.0")
        earlier = "hub.out.20261003-161502-123.log"
        for ms, busy in ((BASE + 3000, "true"), (BASE + 3200, "false")):
            log.line(
                f"\x1b[2m{utc_stamp(ms)}\x1b[0m \x1b[32m INFO\x1b[0m \x1b[2m{target}\x1b[0m"
                "\x1b[2m:\x1b[0m Live busy changed \x1b[3minstance\x1b[0m\x1b[2m=\x1b[0m"
                f'"master" \x1b[3mbusy\x1b[0m\x1b[2m=\x1b[0m{busy}',
                name=earlier,
            )
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(summary["busy"], "2")
        self.assertEqual(summary["busy_longest_ms"], "500.0")
        self.assertEqual(summary["late_heartbeats"], "1")
        self.assertEqual(
            [(a["data-instance"], a["data-start"], a["data-end"]) for a in page.of_class("busy")],
            [
                ("band", f"{BASE + 1000}.0", f"{BASE + 1500}.0"),
                ("master", f"{BASE + 3000}.0", f"{BASE + 3200}.0"),
            ],
        )
        self.assertEqual(len(page.of_class("late-heartbeat")), 1)
        self.assertIn(earlier, page.text, "every hub.out*.log is read")
        self.assertEqual(summary["notes"], "0")


class Window(ReportCase):
    def test_records_outside_the_window_are_left_out_and_bad_lines_counted(self):
        log = Log()
        start, end = BASE + 10000, BASE + 20000
        log.add("set", BASE - 70000, client=1, instance="band", key=VOX, seq=1, value=0.5)
        log.add("set", BASE + 5000, client=2, instance="band", key=VOX, seq=1, value=0.5)
        log.add("set", BASE + 15000, client=7, instance="band", key=VOX, seq=2, value=0.5)
        log.add("ping", BASE + 15000, client=7, n=1, t=BASE + 15000 - OFFSET, offset_ms=OFFSET)
        log.add("set", BASE + 25000, client=4, instance="band", key=VOX, seq=3, value=0.5)
        reset = {"ev": "reset", "t": BASE + 19000 - OFFSET, "active": False}
        # A batch uploaded after the window still brings its events; one past
        # the 30 minutes after it does not.
        log.trace(BASE + 25000, 7, [dict(reset, count=1)])
        log.trace(end + 31 * 60000, 7, [dict(reset, count=2)])
        log.write(self.logs)
        far_day = os.path.join(self.logs, "events-2026-10-01.jsonl")
        with open(far_day, "w", encoding="utf-8") as f:
            f.write(json.dumps({"ev": "set", "ts": BASE + 15000, "key": VOX}) + "\n")
        day = os.path.join(self.logs, "events-2026-10-03.jsonl")
        with open(day, "a", encoding="utf-8") as f:
            f.write("\n")
            f.write(f'{{"ev":"set","ts":{BASE + 15001},"key":"band|li\n')
            f.write(f'{{"ev":"set","ts":{end + 31 * 60000},"key":"band|li\n')
            f.write('{"ev":"ba')
        summary, page, _ = self.report(Log(), start, end)
        self.assertEqual(summary["records"], "2")
        self.assertEqual(summary["sockets"], "1")
        self.assertEqual(summary["resets"], "1")
        self.assertEqual(summary["skipped_lines"], "2", "only lines that could be in range")
        self.assertIn("events-2026-10-03.jsonl", page.text)
        self.assertNotIn("events-2026-10-01.jsonl", page.text)
        self.assertIn("2 lines of the event log did not parse", page.text)

    def test_an_empty_window_still_gives_a_report(self):
        summary, page, _ = self.report(Log(), BASE, BASE + 60000)
        self.assertEqual((summary["records"], summary["jumps"]), ("0", "0"))
        self.assertEqual(summary["confirmation_p50_ms"], "n/a")
        self.assertIn("The window holds no records.", page.text)
        self.assertIn("no event file events-2026-10-03.jsonl", page.text)
        self.assertEqual(summary["notes"], "2", "no day file, no hub log")

    def test_the_writers_notes_are_counted(self):
        log = Log()
        log.add("dropped", BASE + 1000, n=12)
        log.add("cap", BASE + 2000)
        summary, page, _ = self.report(log, BASE, BASE + 5000)
        self.assertEqual(summary["notes"], "3")
        self.assertIn("the event log dropped 12 records", page.text)
        self.assertIn("reached its cap", page.text)


class Output(unittest.TestCase):
    def test_a_report_never_goes_into_a_git_checkout(self):
        with tempfile.TemporaryDirectory() as folder:
            logs = os.path.join(folder, "logs")
            os.makedirs(logs)
            checkout = os.path.join(folder, "repo")
            os.makedirs(os.path.join(checkout, ".git"))
            worktree = os.path.join(folder, "worktree")
            os.makedirs(os.path.join(worktree, "docs"))
            with open(os.path.join(worktree, ".git"), "w", encoding="utf-8") as f:
                f.write("gitdir: elsewhere\n")
            os.makedirs(os.path.join(checkout, "sub"))
            window = ["--events", logs, "--from", "18:20", "--to", "18:21"]
            for out, why in (
                (os.path.join(checkout, "sub", "r.html"), "is a git checkout"),
                (os.path.join(checkout, "r.html"), "is a git checkout"),
                (os.path.join(worktree, "docs", "r.html"), "is a git checkout"),
                (os.path.join(folder, "missing", "r.html"), "does not exist"),
                (logs, "is not a file path"),
            ):
                code, stdout, stderr = run_tool(*window, "--out", out)
                self.assertEqual((code, stdout), (1, ""), out)
                self.assertTrue(stderr.startswith("timeline: --out"), stderr)
                self.assertIn(why, stderr)
                self.assertFalse(os.path.isfile(out))
            out = os.path.join(folder, "r.html")
            self.assertEqual(run_tool(*window, "--out", out)[0], 0)
            self.assertTrue(os.path.isfile(out))
            self.assertEqual(sorted(os.listdir(folder)), ["logs", "r.html", "repo", "worktree"])

    def test_the_window_is_read_in_the_machines_own_time_zone(self):
        # A PC 5:45 east of UTC (a POSIX TZ: no time zone database needed),
        # so a window read as UTC would miss every record by hours.
        log = Log()
        stall_drag(log)
        with tempfile.TemporaryDirectory() as folder:
            log.write(folder)
            out = os.path.join(folder, "r.html")
            command = [sys.executable, os.path.join(HERE, "timeline.py"), "--events", folder]
            command += ["--from", "2026-10-03 22:05", "--to", "2026-10-03 22:05:05", "--out", out]
            env = dict(os.environ, TZ="XYZ-05:45")
            done = subprocess.run(command, env=env, capture_output=True, text=True, check=False)
            self.assertEqual((done.returncode, done.stderr), (0, ""))
            summary = parse_summary(done.stdout)
            self.assertEqual(summary["records"], str(len(log.records)))
            self.assertEqual(summary["longest_dropout_at"], "2026-10-03 22:05:01.504")


if __name__ == "__main__":
    unittest.main()
