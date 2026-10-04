"""The forensics timeline's analysis (#43): page time on the hub's clock,
gestures and gaps, busy episodes, the control lanes, confirmations and the
volume jumps with their cause, and the summary. Pure: it reads nothing. One
of the four files of ``timeline.py`` (see its docstring), copied to the
Ableton PC together with it.
"""

import bisect
import collections
import itertools
import json
import math

from timeline_read import is_volume, key_hash, local_text, number, value2db

# A gap between two points of a control's row that is marked, and the bound
# of every measured cause (ms).
GAP_MS = 100.0
# A gesture lasts this long after its finger lifted: late arrivals and Live's
# applied values belong to it (ms).
GESTURE_TAIL_MS = 1000.0
# A key no touch in the window names (a page before the flight recorder): two
# points at most this far apart are one gesture (ms).
UNTOUCHED_GESTURE_MS = 10000.0
# Records from this long before --from count: offsets and busy episodes that
# started before the window (ms).
LEAD_MS = 60000
# Trace records up to this long after --to count: their page events can fall
# inside the window (ms).
TRACE_TAIL_MS = 60000
# A change between two applied volumes past this is a jump (dB).
JUMP_DB = 3.0

ROWS = ("send", "arrival", "applied")

Point = collections.namedtuple("Point", ("time", "value", "hollow", "info"))
PageEvent = collections.namedtuple("PageEvent", ("hub", "ev", "data"))
Span = collections.namedtuple("Span", ("start", "end", "ms", "info"))
Jump = collections.namedtuple(
    "Jump",
    (
        "time",
        "key",
        "db_from",
        "db_to",
        "cause",
        "gaps",
        "rtt_ms",
        "wait_ms",
        "overlaps",
    ),
)


def is_jump(db_from, db_to):
    """Whether two applied volumes (dB) differ by more than ``JUMP_DB``; -inf
    against any finite value is one, -inf against -inf is not."""
    if math.isinf(db_from) or math.isinf(db_to):
        return db_from != db_to
    return abs(db_to - db_from) > JUMP_DB


def percentile(values, fraction):
    """Nearest rank: the smallest value with at least ``fraction`` of the
    values at or below it; None when there are none."""
    if not values:
        return None
    ordered = sorted(values)
    rank = math.ceil(fraction * len(ordered) - 1e-9)
    return ordered[min(len(ordered), max(rank, 1)) - 1]


def max_gap(times, low, high):
    """The largest gap between consecutive ``times`` from ``low`` to ``high``
    (both ends taken as points too); 0.0 when they meet."""
    low, high = min(low, high), max(low, high)
    points = sorted({low, high, *(t for t in times if low <= t <= high)})
    return max((b - a for a, b in itertools.pairwise(points)), default=0.0)


def gap_into(times, low, high):
    """``max_gap`` from the last of ``times`` before ``low``: the gap that
    ends at ``low`` counts too."""
    low, high = min(low, high), max(low, high)
    earlier = [t for t in times if t < low]
    return max_gap(times, max(earlier) if earlier else low, high)


def overlaps(start, end, low, high):
    """Whether [start, end] overlaps the interval (low, high]."""
    return start <= high and end > low


def _names(touch, key):
    keys = touch.data.get("keys")
    return isinstance(keys, list) and key in keys


def _starts(touch, key):
    """Whether ``touch`` begins a touch of ``key`` (a ``down`` or a ``tap``)."""
    return touch.data.get("what") in ("down", "tap") and _names(touch, key)


def gestures(touches, key, end):
    """One span per touch of ``key`` (on the hub's clock), never merged:
    ``touches`` are the window's page touches in time order, up to ``end``.

    - A ``tap`` (mute, solo and stage: no lift follows) spans
      ``GESTURE_TAIL_MS``.
    - A ``down`` spans to its lift (the first ``up`` or ``cancel`` of its
      pointer after it) plus ``GESTURE_TAIL_MS``, cut at the start of the
      next touch of the key that begins after the lift (a fader let go and
      grabbed again within the tail: the time the finger was off is no gap).
    - A ``down`` with no lift spans to the start of the next touch of the
      key, else to ``end`` (a fader held past the window).

    A lift is matched by its pointer alone, so one uploaded on another socket
    after a reconnect still ends its touch (two tablets touching one control
    with the same pointer id at once are taken as one finger). None when no
    touch names the key: then two points ``UNTOUCHED_GESTURE_MS`` apart at
    most are one gesture."""
    named = False
    spans = []
    for index, touch in enumerate(touches):
        if not _names(touch, key):
            continue
        named = True
        what = touch.data.get("what")
        if what == "tap":
            spans.append((touch.hub, touch.hub + GESTURE_TAIL_MS))
            continue
        if what != "down":
            continue
        later = touches[index + 1 :]
        pointer = touch.data.get("pointer")
        lift = next(
            (
                t.hub
                for t in later
                if t.data.get("what") in ("up", "cancel") and t.data.get("pointer") == pointer
            ),
            None,
        )
        if lift is None:
            finish = next((t.hub for t in later if _starts(t, key)), end)
        else:
            cut = next((t.hub for t in later if t.hub >= lift and _starts(t, key)), None)
            finish = lift + GESTURE_TAIL_MS if cut is None else min(lift + GESTURE_TAIL_MS, cut)
        spans.append((touch.hub, finish))
    return spans if named else None


def row_gaps(times, spans):
    """The gaps of one row (its point times, in order) longer than ``GAP_MS``
    whose two points belong to one gesture: (start, end, ms)."""
    gaps = []
    for a, b in itertools.pairwise(times):
        ms = b - a
        if ms <= GAP_MS:
            continue
        if spans is None:
            inside = ms <= UNTOUCHED_GESTURE_MS
        else:
            inside = any(start <= a and b <= end for start, end in spans)
        if inside:
            gaps.append((a, b, ms))
    return gaps


def busy_episodes(changes, end):
    """Busy episodes from ``changes`` ((ms, instance, busy), from the ``link``
    records and the hub log lines, any order): from a busy=true to the next
    busy=false of that instance. A repeat of the state (the other source's
    copy) changes nothing; one still open ends at ``end``. Spans in start
    order, ``info`` = (instance, still open)."""
    since = {}
    episodes = []
    for ms, instance, busy in sorted(changes, key=lambda change: change[0]):
        if busy and instance not in since:
            since[instance] = ms
        elif not busy and instance in since:
            start = since.pop(instance)
            episodes.append(Span(start, ms, ms - start, (instance, False)))
    for instance, start in since.items():
        episodes.append(Span(start, end, end - start, (instance, True)))
    return sorted(episodes, key=lambda span: span.start)


# --- the analysis (pure) ---


class Offsets:
    """The page clocks' offsets (hub − page, ms) of the ``ping`` records."""

    def __init__(self, pings):
        self.by_client = collections.defaultdict(lambda: ([], []))
        self.every = ([], [])
        for ping in sorted(pings, key=lambda p: p["ts"]):
            offset = number(ping.get("offset_ms"))
            if offset is None:
                continue
            for times, values in (self.by_client[ping.get("client")], self.every):
                times.append(ping["ts"])
                values.append(offset)

    def at(self, client, ts):
        """The offset for a trace of ``client`` recorded at ``ts``: its own
        ping nearest in time, else any client's, else None."""
        for times, values in (self.by_client.get(client, ([], [])), self.every):
            if times:
                i = bisect.bisect_left(times, ts)
                near = min(
                    (j for j in (i - 1, i) if 0 <= j < len(times)),
                    key=lambda j: abs(times[j] - ts),
                )
                return values[near]
        return None


def page_events(traces, offsets):
    """The page events of ``traces`` (in ``ts`` order) on the hub's clock, each
    once: an exact duplicate (a batch resent after a lost socket) is dropped.
    Returns (events in time order, traces mapped without an offset, events
    left out for want of ``ev`` and ``t``)."""
    seen = set()
    events = []
    unaligned = bad = 0
    for trace in traces:
        batch = trace.get("events")
        if not isinstance(batch, list) or not batch:
            continue
        offset = offsets.at(trace.get("client"), trace["ts"])
        if offset is None:
            unaligned += 1
            offset = 0.0
        for event in batch:
            t = number(event.get("t")) if isinstance(event, dict) else None
            if t is None or not isinstance(event.get("ev"), str):
                bad += 1
                continue
            identity = json.dumps(event, sort_keys=True)
            if identity in seen:
                continue
            seen.add(identity)
            events.append(PageEvent(t + offset, event["ev"], event))
    events.sort(key=lambda event: event.hub)
    return events, unaligned, bad


def arrival_of(record):
    """A set's arrival at the hub (``hub_ms``, else its ``ts``)."""
    hub_ms = number(record.get("hub_ms"))
    return record["ts"] if hub_ms is None else hub_ms


def sent_of(record):
    """A set's send time on the hub's clock: ``t + offset_ms``, else its arrival."""
    t, offset = number(record.get("t")), number(record.get("offset_ms"))
    return arrival_of(record) if t is None or offset is None else t + offset


def identity_of(instance, item):
    return (instance, item.get("key"), item.get("client"), item.get("seq"))


class Timeline:
    """Everything the report and the summary show of the window ``start`` to
    ``end`` (epoch ms): built from ``records`` (any order; those out of range
    are left out) and the hub log ``marks``."""

    def __init__(self, records, marks, start, end, key_filters=(), skipped=0, notes=()):
        self.start, self.end = start, end
        lead = start - LEAD_MS
        self.filters = list(key_filters)
        self.skipped = skipped
        self.notes = list(notes)
        if skipped:
            self.notes.append(
                f"{skipped} lines of the event log did not parse (a write in progress?) "
                "and were skipped"
            )
        kinds = collections.defaultdict(list)
        for record in sorted(records, key=lambda r: r["ts"]):
            last = end + TRACE_TAIL_MS if record.get("ev") == "trace" else end
            if lead <= record["ts"] <= last:
                kinds[record.get("ev")].append(record)
        in_window = [r for group in kinds.values() for r in group if start <= r["ts"] <= end]
        self.records = len(in_window)
        # Distinct socket numbers: a tablet that reconnects counts twice.
        self.sockets = len(
            {
                r.get("client")
                for r in in_window
                if r.get("ev") in ("set", "trace") and r.get("client") is not None
            }
        )
        self._writer_notes(kinds)
        events, unaligned, bad = page_events(kinds["trace"], Offsets(kinds["ping"]))
        if unaligned:
            self.notes.append(
                f"page clock not aligned: no ping offset for {unaligned} trace records; "
                "their events are drawn at offset 0"
            )
        if bad:
            self.notes.append(f"{bad} page events without ev or t were left out")
        self.page = [e for e in events if e.hub <= end]
        self._link(kinds)
        self._live(kinds, marks)
        self._controls(kinds)
        self.jumps = [jump for key in self.keys for jump in self._jumps(key)]

    # The window's parts.

    def in_window(self, ms):
        return self.start <= ms <= self.end

    def wanted(self, key):
        return isinstance(key, str) and (not self.filters or any(f in key for f in self.filters))

    def _writer_notes(self, kinds):
        for record in kinds["cap"]:
            self.notes.append(
                f"a day file reached its cap at {local_text(record['ts'])}: "
                "after it only warn-class records were written"
            )
        for record in kinds["dropped"]:
            self.notes.append(
                f"the event log dropped {record.get('n')} records before "
                f"{local_text(record['ts'])} (its queue was full)"
            )

    def _page(self, ev):
        return [e for e in self.page if e.ev == ev]

    def _link(self, kinds):
        self.rtt_page = [
            (e.hub, number(e.data.get("rtt")))
            for e in self._page("pong")
            if self.in_window(e.hub) and number(e.data.get("rtt")) is not None
        ]
        self.rtt_hub = [
            (arrival_of(r), number(r.get("rtt")))
            for r in kinds["ping"]
            if self.in_window(arrival_of(r)) and number(r.get("rtt")) is not None
        ]
        self.dropouts = []
        for e in self._page("dropout"):
            ms = number(e.data.get("ms")) or 0.0
            if e.hub <= self.end and e.hub + ms >= self.start:
                lost = e.data.get("socket_lost") is True
                self.dropouts.append(Span(e.hub, e.hub + ms, ms, lost))
        self.resets = [e for e in self._page("reset") if self.in_window(e.hub)]
        self.frames = []
        for e in self._page("frame"):
            ms = number(e.data.get("ms")) or 0.0
            if e.hub <= self.end and e.hub + ms >= self.start:
                self.frames.append(Span(e.hub, e.hub + ms, ms, None))
        self.visibility = [e for e in self._page("visibility") if self.in_window(e.hub)]
        for e in self._page("overflow"):
            if self.in_window(e.hub):
                self.notes.append(
                    f"the page flight recorder dropped {e.data.get('n')} events "
                    f"before {local_text(e.hub)} (its ring was full)"
                )
        socks = []
        for r in kinds["sock"]:
            if self.in_window(r["ts"]):
                title = f"hub socket {r.get('what')} (client {r.get('client')})"
                if r.get("reason"):
                    title += f": {r.get('reason')}"
                socks.append((r["ts"], title))
        for e in self._page("sock"):
            if self.in_window(e.hub):
                title = f"page socket {e.data.get('what')}"
                details = [
                    f"{n} {e.data[n]}" for n in ("socket", "code") if e.data.get(n) is not None
                ]
                if e.data.get("reason"):
                    details.append(str(e.data["reason"]))
                if details:
                    title += ": " + ", ".join(details)
                socks.append((e.hub, title))
        self.socks = sorted(socks, key=lambda sock: sock[0])

    def _live(self, kinds, marks):
        changes = [
            (r["ts"], r.get("instance"), r.get("busy"))
            for r in kinds["link"]
            if isinstance(r.get("busy"), bool)
        ]
        changes += [(m[1], m[2], m[3]) for m in marks if m[0] == "busy"]
        self.busy = [b for b in busy_episodes(changes, self.end) if b.end >= self.start]
        self.late = sorted(
            ((m[1], m[2], m[3]) for m in marks if m[0] == "late" and self.in_window(m[1])),
            key=lambda late: late[0],
        )

    def _controls(self, kinds):
        """The control lanes, the sets behind them and the confirmations."""
        failed = {
            identity_of(r.get("instance"), r) for r in kinds["ack"] if r.get("error") is not None
        }
        self.sets = collections.defaultdict(list)
        self.key_sets = collections.defaultdict(list)
        for r in kinds["set"]:
            if self.wanted(r.get("key")):
                self.sets[identity_of(r.get("instance"), r)].append(r)
                self.key_sets[r["key"]].append(r)
        batches = collections.defaultdict(list)
        for r in kinds["batch"]:
            batches[(r.get("instance"), r.get("batch"))].append(r)
        rows = collections.defaultdict(lambda: {row: [] for row in ROWS})
        applied_at = collections.defaultdict(list)
        for r in kinds["applied"]:
            batch = None
            for candidate in batches.get((r.get("instance"), r.get("batch")), []):
                if candidate["ts"] <= r["ts"]:
                    batch = candidate
            values = {}
            for item in (batch or {}).get("sent") or []:
                if isinstance(item, dict):
                    values[identity_of(r.get("instance"), item)] = item.get("value")
            for item in r.get("sent") or []:
                if not isinstance(item, dict) or not self.wanted(item.get("key")):
                    continue
                identity = identity_of(r.get("instance"), item)
                if identity in failed:
                    continue
                applied_at[identity].append(r["ts"])
                info = {
                    "identity": identity,
                    "rtt_ms": number(r.get("rtt_ms")),
                    "batch_ts": batch["ts"] if batch else None,
                }
                rows[item["key"]]["applied"].append(
                    Point(r["ts"], values.get(identity), False, info)
                )
        for key, sets in self.key_sets.items():
            for r in sets:
                rows[key]["arrival"].append(Point(arrival_of(r), r.get("value"), False, None))
        self.page_sends = collections.defaultdict(list)
        for e in self._page("send"):
            if self.wanted(e.data.get("key")):
                self.page_sends[e.data["key"]].append(e)
                hollow = e.data.get("sent") is False
                rows[e.data["key"]]["send"].append(Point(e.hub, e.data.get("value"), hollow, None))
        touches = self._page("touch")
        continuous = self._continuous()
        self.lanes = {}
        for key in sorted(rows):
            lane = {
                row: sorted(
                    (p for p in rows[key][row] if self.in_window(p.time)), key=lambda p: p.time
                )
                for row in ROWS
            }
            if not any(lane.values()):
                continue
            if key in continuous:
                spans = gestures(touches, key, self.end)
                lane["gaps"] = {row: row_gaps([p.time for p in lane[row]], spans) for row in ROWS}
            else:
                # Only final sends (a toggle's press and release): no row gaps.
                lane["gaps"] = {row: [] for row in ROWS}
            self.lanes[key] = lane
        self.keys = list(self.lanes)
        self.confirmations = []
        for key, sets in self.key_sets.items():
            for r in sets:
                if not self.in_window(r["ts"]):
                    continue
                times = applied_at.get(identity_of(r.get("instance"), r), [])
                i = bisect.bisect_left(times, r["ts"])
                if i < len(times):
                    self.confirmations.append((times[i] - sent_of(r), sent_of(r), key))

    def _continuous(self):
        """The keys with a non-final send in the window (a page ``send`` or a
        hub ``set`` whose ``final`` is exactly false): the continuous controls,
        the only ones whose rows get gaps."""
        keys = set()
        for key, sends in self.page_sends.items():
            if any(e.data.get("final") is False and self.in_window(e.hub) for e in sends):
                keys.add(key)
        for key, sets in self.key_sets.items():
            if any(r.get("final") is False and self.in_window(r["ts"]) for r in sets):
                keys.add(key)
        return keys

    def set_for(self, identity, at):
        """The set record of ``identity`` that an applied value at ``at`` came
        from (the latest one recorded by then)."""
        found = None
        for r in self.sets.get(identity, []):
            if r["ts"] <= at:
                found = r
        return found

    def _send_times(self, key):
        """The page's send times of ``key`` (page clock): its ``send`` events,
        or the sets' own ``t`` when the window has none."""
        sends = self.page_sends.get(key)
        if sends:
            times = [number(e.data.get("t")) for e in sends]
        else:
            times = [number(r.get("t")) for r in self.key_sets[key]]
        return [t for t in times if t is not None]

    def _jumps(self, key):
        """The volume jumps of ``key`` with their measured cause. The link's
        gaps count the gap into S1 too: the hub's setter writes the first set
        after a stall alone, so S1 can be the first of the burst."""
        if not is_volume(key):
            return []
        points = [p for p in self.lanes[key]["applied"] if number(p.value) is not None]
        jumps = []
        for a1, a2 in itertools.pairwise(points):
            db1, db2 = value2db(number(a1.value)), value2db(number(a2.value))
            if not is_jump(db1, db2):
                continue
            s1 = self.set_for(a1.info["identity"], a1.time)
            s2 = self.set_for(a2.info["identity"], a2.time)
            gaps = {"arrival": None, "send": None, "page": None}
            wait = None
            if s1 is not None and s2 is not None:
                arrivals = [arrival_of(r) for r in self.key_sets[key]]
                gaps["arrival"] = gap_into(arrivals, arrival_of(s1), arrival_of(s2))
                t1, t2 = number(s1.get("t")), number(s2.get("t"))
                if t1 is not None and t2 is not None:
                    sends = self._send_times(key)
                    gaps["send"] = gap_into(sends, t1, t2)
                    gaps["page"] = max_gap(sends, t1, t2)
            arrival_gap, send_gap = gaps["arrival"], gaps["send"]
            if s2 is not None and a2.info["batch_ts"] is not None:
                wait = a2.info["batch_ts"] - arrival_of(s2)
            rtt = a2.info["rtt_ms"]
            hits = [
                name
                for name, spans in (
                    ("dropout", self.dropouts),
                    ("busy", self.busy),
                    ("frame", self.frames),
                )
                if any(overlaps(s.start, s.end, a1.time, a2.time) for s in spans)
            ]
            if "dropout" in hits or (
                arrival_gap is not None
                and arrival_gap > GAP_MS
                and send_gap is not None
                and send_gap <= GAP_MS
            ):
                cause = "link"
            elif (rtt or 0.0) > GAP_MS or (wait or 0.0) > GAP_MS or "busy" in hits:
                cause = "live"
            elif (gaps["page"] or 0.0) > GAP_MS or "frame" in hits:
                cause = "page"
            else:
                cause = "move"
            jumps.append(Jump(a2.time, key, db1, db2, cause, gaps, rtt, wait, hits))
        return jumps


# --- the summary ---


def ms_text(value):
    """Milliseconds with one decimal, ``n/a`` for none."""
    return "n/a" if value is None else f"{value:.1f}"


def summary(timeline):
    """The summary as (name, text) pairs: stdout's lines and the report's
    table. No key and no name: a control is ``key#<hash>``."""
    latencies = [c[0] for c in timeline.confirmations]
    worst = max(timeline.confirmations, key=lambda c: c[0], default=None)
    longest = max(timeline.dropouts, key=lambda d: d.ms, default=None)
    page_rtts = [rtt for _, rtt in timeline.rtt_page]
    hub_rtts = [rtt for _, rtt in timeline.rtt_hub]

    def gap_max(row):
        gaps = [g[2] for lane in timeline.lanes.values() for g in lane["gaps"][row]]
        return f"{max(gaps):.1f}" if gaps else "0"

    return [
        ("records", str(timeline.records)),
        ("sockets", str(timeline.sockets)),
        ("confirmation_n", str(len(latencies))),
        ("confirmation_p50_ms", ms_text(percentile(latencies, 0.50))),
        ("confirmation_p90_ms", ms_text(percentile(latencies, 0.90))),
        ("confirmation_p99_ms", ms_text(percentile(latencies, 0.99))),
        ("worst_confirmation_ms", ms_text(worst and worst[0])),
        ("worst_confirmation_at", local_text(worst[1]) if worst else "n/a"),
        ("worst_confirmation_key", f"key#{key_hash(worst[2])}" if worst else "n/a"),
        ("dropouts", str(len(timeline.dropouts))),
        ("longest_dropout_ms", ms_text(longest and longest.ms)),
        ("longest_dropout_at", local_text(longest.start) if longest else "n/a"),
        ("resets", str(len(timeline.resets))),
        ("rtt_page_p50_ms", ms_text(percentile(page_rtts, 0.50))),
        ("rtt_page_max_ms", ms_text(max(page_rtts, default=None))),
        ("rtt_hub_p50_ms", ms_text(percentile(hub_rtts, 0.50))),
        ("rtt_hub_max_ms", ms_text(max(hub_rtts, default=None))),
        ("gap_send_max_ms", gap_max("send")),
        ("gap_arrival_max_ms", gap_max("arrival")),
        ("gap_applied_max_ms", gap_max("applied")),
        ("busy", str(len(timeline.busy))),
        ("busy_longest_ms", ms_text(max((b.ms for b in timeline.busy), default=None))),
        ("late_heartbeats", str(len(timeline.late))),
        ("jumps", str(len(timeline.jumps))),
        ("skipped_lines", str(timeline.skipped)),
        ("notes", str(len(timeline.notes))),
    ]
