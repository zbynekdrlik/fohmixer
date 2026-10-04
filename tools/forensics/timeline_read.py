"""The forensics timeline's values, times and reading (#43): TouchOSC's dB
curve, local and UTC time texts, key hashes, and the reading of the hub's
event log (the window's day files) and text logs (busy changes, late
heartbeats). One of the five files of ``timeline.py`` (see its docstring),
copied to the Ableton PC together with it.
"""

import calendar
import datetime
import fnmatch
import hashlib
import json
import math
import os
import re

TS = re.compile(rb'"ts":\s*(-?\d+)')
# The records read past the window's end, up to its trace tail: a page's
# ``trace`` (its events can fall inside the window) and the ``ping``s that
# give that page's clock (a trace on the next socket after an outage has
# only its own socket's pings, all after the window).
TAIL_KINDS = ("trace", "ping")
TAIL_EV = re.compile(rb'"ev":\s*"(?:' + b"|".join(k.encode() for k in TAIL_KINDS) + rb')"')
TIME = re.compile(
    r"(?:(?P<date>\d{4}-\d{2}-\d{2})[ T])?(?P<h>\d{1,2}):(?P<m>\d{2})"
    r"(?::(?P<s>\d{2})(?:\.(?P<f>\d{1,6}))?)?"
)
ANSI = re.compile(r"\x1b\[[0-9;]*m")
STAMP = re.compile(r"(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.(\d+))?Z")
# The two hub log messages it reads; a message follows "<target>: " (a late
# heartbeat's text inside a busy line's reason follows `reason="`).
BUSY_LINE = re.compile(r"(?:^|: )Live busy changed(?= |$)")
LATE_LINE = re.compile(r"(?:^|: )a heartbeat (\d+) ms after the previous one")
FIELD = re.compile(r'(\w+)=("(?:[^"\\]|\\.)*"|\S+)')


class TimelineError(Exception):
    """A usage or input error: one ``timeline: ...`` line on stderr, exit 1."""


# --- values and times (pure) ---


def value2db(v):
    """TouchOSC's ``value2db`` (``behave/fader.rs``): Live's 0..1 volume in dB."""
    if v > 1.0 or v != v:
        return 0.0
    if v >= 0.4:
        return 40.0 * v - 34.0
    if v >= 0.15:
        return -((399.751894 * v - 201.871345) ** 2 + 12630.61132) / 799.503788
    if v < 0.0:
        # Rust's powf of a negative number is NaN (Python's power is complex).
        return float("nan")
    gamma = 7504.0 / 5567.0
    db = 118.426374 * v ** (1.0 / gamma) - 70.0
    return float("-inf") if db <= -70.0 else db


def to_live(p):
    """Live's volume at fader position ``p`` (``behave/fader.rs`` ``to_live``:
    ``p^0.515``, ``p`` clamped to 0..1)."""
    return min(max(p, 0.0), 1.0) ** 0.515


def to_pos(v):
    """The fader position of Live's volume ``v`` (``to_pos``, ``v`` clamped
    to 0..1)."""
    return min(max(v, 0.0), 1.0) ** (1.0 / 0.515)


def number(value):
    """``value`` as a finite float, or None (a bool is no number)."""
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    return float(value) if math.isfinite(value) else None


def parse_local(text, today=None):
    """Epoch ms of a ``--from``/``--to`` text in this machine's local time;
    the time-only form is on ``today`` (default: the local date now)."""
    match = TIME.fullmatch(text.strip())
    if match is None:
        raise TimelineError(
            f"cannot read the time {text!r}: want YYYY-MM-DD HH:MM[:SS[.fff]] "
            "or HH:MM[:SS[.fff]] (local time)"
        )
    if match["date"]:
        year, month, day = (int(part) for part in match["date"].split("-"))
    else:
        date = today or datetime.date.today()
        year, month, day = date.year, date.month, date.day
    fraction = (match["f"] or "0").ljust(6, "0")
    try:
        moment = datetime.datetime(
            year, month, day, int(match["h"]), int(match["m"]), int(match["s"] or 0), int(fraction)
        )
        return round(moment.timestamp() * 1000.0)
    except (ValueError, OverflowError, OSError) as e:
        raise TimelineError(f"cannot read the time {text!r}: {e}") from e


def _clock_text(moment):
    return moment.strftime("%Y-%m-%d %H:%M:%S.") + f"{moment.microsecond // 1000:03d}"


def local_text(ms):
    """``YYYY-MM-DD HH:MM:SS.fff`` of epoch ms in local time (what ``--from`` reads)."""
    return _clock_text(datetime.datetime.fromtimestamp(ms / 1000.0))


def utc_text(ms):
    """``YYYY-MM-DD HH:MM:SS.fffZ`` of epoch ms."""
    return _clock_text(datetime.datetime.fromtimestamp(ms / 1000.0, datetime.UTC)) + "Z"


def utc_date(ms):
    return datetime.datetime.fromtimestamp(ms / 1000.0, datetime.UTC).date()


def key_hash(key):
    """The first 10 hex of the key's SHA-256: names a control on stdout."""
    return hashlib.sha256(key.encode("utf-8")).hexdigest()[:10]


def key_scale(key):
    """The value range a key's rows are drawn in: (0, 1) for a volume, (-1, 1)
    for a pan (Live's panning), None for anything else (drawn as dots)."""
    _, _, rest = key.partition("|")
    target, _, prop = rest.rpartition("|")
    if prop != "value":
        return None
    if target.endswith(" mixer_device volume"):
        return (0.0, 1.0)
    if target.endswith(" mixer_device panning"):
        return (-1.0, 1.0)
    return None


def is_volume(key):
    return key_scale(key) == (0.0, 1.0)


# --- reading ---


def day_files(start_ms, end_ms):
    """The names of the day files that can hold records from ``start_ms`` to
    ``end_ms`` (their UTC dates), in date order."""
    day, last = utc_date(start_ms), utc_date(end_ms)
    names = []
    while day <= last:
        names.append(f"events-{day.isoformat()}.jsonl")
        day += datetime.timedelta(days=1)
    return names


def read_events(folder, start_ms, end_ms, trace_end_ms):
    """The records of the window's day files in ``folder``: ``ts`` from
    ``start_ms`` to ``end_ms`` (``TAIL_KINDS`` to ``trace_end_ms``), in ``ts``
    order. Returns (records, files read, files missing, lines skipped). A line
    whose ``ts`` is out of range, or past ``end_ms`` and of no tail kind, is
    never parsed; a missing file only the tail reaches is not reported."""
    records, read, missing, skipped = [], [], [], 0
    window = day_files(start_ms, end_ms)
    for name in day_files(start_ms, trace_end_ms):
        path = os.path.join(folder, name)
        if not os.path.isfile(path):
            if name in window:
                missing.append(name)
            continue
        try:
            with open(path, "rb") as f:
                for line in f:
                    found = TS.search(line)
                    if found is None:
                        skipped += bool(line.strip())
                        continue
                    ts = int(found[1])
                    if not start_ms <= ts <= trace_end_ms:
                        continue
                    if ts > end_ms and TAIL_EV.search(line) is None:
                        continue
                    try:
                        record = json.loads(line)
                    except ValueError:
                        skipped += 1
                        continue
                    ts = record.get("ts") if isinstance(record, dict) else None
                    if isinstance(ts, bool) or not isinstance(ts, int):
                        skipped += 1
                        continue
                    last = trace_end_ms if record.get("ev") in TAIL_KINDS else end_ms
                    if start_ms <= ts <= last:
                        records.append(record)
        except OSError as e:
            raise TimelineError(f"cannot read {name}: {e.strerror or e}") from e
        read.append(name)
    records.sort(key=lambda record: record["ts"])
    return records, read, missing, skipped


def stamp_ms(text):
    """Epoch ms of a hub log line's RFC 3339 UTC stamp, or None."""
    match = STAMP.fullmatch(text)
    if match is None:
        return None
    fraction = (match[7] or "")[:6].ljust(6, "0")
    try:
        seconds = calendar.timegm(tuple(int(match[i]) for i in range(1, 7)))
    except ValueError:
        return None
    return seconds * 1000.0 + int(fraction) / 1000.0


def _unquote(value):
    if len(value) >= 2 and value[0] == value[-1] == '"':
        return re.sub(r"\\(.)", r"\1", value[1:-1])
    return value


def _fields(text):
    return {name: _unquote(value) for name, value in FIELD.findall(text)}


def hub_log_mark(line):
    """The mark of one hub log line (ANSI colours allowed): ("busy", ms,
    instance, busy) for ``Live busy changed``, ("late", ms, instance, gap ms)
    for a late heartbeat, None for any other line."""
    parts = ANSI.sub("", line).split(None, 1)
    if len(parts) < 2:
        return None
    ms = stamp_ms(parts[0])
    if ms is None:
        return None
    rest = parts[1]
    busy = BUSY_LINE.search(rest)
    if busy is not None:
        fields = _fields(rest[busy.end() :])
        if fields.get("busy") not in ("true", "false"):
            return None
        return ("busy", ms, fields.get("instance"), fields["busy"] == "true")
    late = LATE_LINE.search(rest)
    if late is not None:
        return ("late", ms, _fields(rest[late.end() :]).get("instance"), float(late[1]))
    return None


def read_hub_logs(folder, start_ms, end_ms):
    """The marks of every ``hub.out*.log`` in ``folder`` from ``start_ms`` to
    ``end_ms``. Returns (marks, files read, notes): a file that cannot be read
    is a note."""
    marks, read, notes = [], [], []
    try:
        names = sorted(n for n in os.listdir(folder) if fnmatch.fnmatch(n, "hub.out*.log"))
    except OSError as e:
        raise TimelineError(f"cannot list the --events folder: {e.strerror or e}") from e
    for name in names:
        try:
            with open(os.path.join(folder, name), encoding="utf-8", errors="replace") as f:
                for line in f:
                    if "Live busy changed" not in line and "a heartbeat " not in line:
                        continue
                    mark = hub_log_mark(line)
                    if mark is not None and start_ms <= mark[1] <= end_ms:
                        marks.append(mark)
        except OSError as e:
            notes.append(f"cannot read {name}: {e.strerror or type(e).__name__}")
            continue
        read.append(name)
    if not names:
        notes.append(
            "no hub.out*.log in the folder: busy episodes from the event log only, "
            "no late heartbeats"
        )
    return marks, read, notes
