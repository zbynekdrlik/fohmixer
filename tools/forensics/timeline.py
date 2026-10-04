#!/usr/bin/env python3
"""The forensics timeline (#43, design note §5.3): a window of the hub's event
log drawn as one self-contained HTML file, so a report such as "fader 3
jumped at 19:40" is answered from the logs alone.

    python timeline.py --events DIR --from TIME --to TIME [--key TEXT ...]
                       --out FILE

- ``--events``: the hub's ``logs`` folder. It holds the event log
  ``events-YYYY-MM-DD.jsonl`` (one file per UTC date, one JSON object per
  line) and the hub's text logs ``hub.out.log`` (the running hub) and
  ``hub.out.<UTC stamp>.log`` (earlier starts), read for Live's busy changes
  and late heartbeats.
- ``--from`` / ``--to``: local time of the machine that runs the tool:
  ``YYYY-MM-DD HH:MM``, ``YYYY-MM-DD HH:MM:SS`` or ``YYYY-MM-DD HH:MM:SS.fff``
  (``T`` instead of the space too), or ``HH:MM[:SS[.fff]]`` for today. An
  hour that a clock change repeats is read as its first pass.
- ``--key``: only the controls whose key holds TEXT (repeatable: any of them).
  It narrows the control lanes, gaps, jumps and confirmations; the link and
  Live lanes and the record count stay whole.
- ``--out``: the report. Its folder must exist and must not be inside a git
  checkout (a ``.git`` entry in it or above it): the report holds real track
  names and never lands in a repository. Nothing else is written.

It reads only the day files of the window (the UTC dates from 1 min before
``--from`` to 1 min after ``--to``), line by line, and parses only the lines
whose ``"ts"`` is in range. Records from 60 s before ``--from`` (offsets, busy
episodes that started before) to ``--to`` count, a page's ``trace`` up to 60 s
after ``--to`` too (a batch uploaded after the window brings events from
inside it). A line that does not parse (a write in progress) is skipped and
counted.

Page time maps to hub time as ``t + offset``: the ``offset_ms`` of the ping of
the trace's own client nearest in time to the trace record, else the nearest
of any client, else 0 (the report notes that the page clock is not aligned).
A ``set`` carries its own offset.

The report: the window, the files read, the record count and the notes; the
summary table; one time axis with the link lane (the page's and the hub's
round trips, dropouts, the counter's resets, socket transitions, long frames,
visibility), the Live lane (busy episodes, late heartbeats) and one lane per
control with three rows (the page's sends, the hub's arrivals, Live's applied
values), gaps over 100 ms inside one gesture marked; then the volume jumps
over 3 dB with their measured cause (``link``, ``live``, ``page``, ``no
data`` (#43 PR E: the page's recorder dropped the long frames of that time)
or ``move``); then the touches of single volume faders (#43 PR D, from the
page's touch starts and move records and the hub's ``live_before``): where
each started against Live's value before it, first-touch jumps with their
why, the time from the down to the first move and the first send, move gaps
and held values.

stdout: the summary, one ``name=value`` per line, the names of the report's
summary table. It never prints a key: a control is ``key#<the first 10 hex of
its SHA-256>``, so the numbers can go on a public ticket. Exit 0 with a report
also when the window holds nothing; exit 1 with one ``timeline: ...`` line on
stderr on a usage or input error.

This file is the command; beside it ``timeline_read.py`` (values, times, the
logs read), ``timeline_model.py`` (the analysis and the summary),
``timeline_touch.py`` (the touches of single volume faders) and
``timeline_report.py`` (the HTML). Python 3.11 standard library only: the
five files are copied to the Ableton PC into one folder and this one is run
with the PC's Python (its folder is on ``sys.path`` then, so it finds the
other four).
"""

import argparse
import os
import sys

from timeline_model import LEAD_MS, TRACE_TAIL_MS, Timeline, summary
from timeline_read import TimelineError, parse_local, read_events, read_hub_logs
from timeline_report import render


# --- the command ---


class Parser(argparse.ArgumentParser):
    """argparse with the tool's one-line error (exit 1, not usage and exit 2)."""

    def error(self, message):
        raise TimelineError(message)


def parse_args(argv):
    parser = Parser(description="Draw a window of the hub's event log as one HTML report (#43).")
    parser.add_argument("--events", required=True, help="the hub's logs folder")
    parser.add_argument(
        "--from", dest="start", required=True, help="local time, YYYY-MM-DD HH:MM[:SS[.fff]]"
    )
    parser.add_argument("--to", dest="end", required=True, help="local time, as --from")
    parser.add_argument(
        "--key", action="append", default=[], help="only controls whose key holds TEXT"
    )
    parser.add_argument("--out", required=True, help="the HTML file (never in a git checkout)")
    return parser.parse_args(argv)


def check_out(out):
    """Refuses an ``--out`` the tool must not write: no file name, a missing
    folder, or a folder inside a git checkout (a ``.git`` entry in the folder
    or above it): the report holds real track names."""
    if not out or out.endswith(("/", os.sep)) or os.path.isdir(out):
        raise TimelineError(f"--out {out!r} is not a file path")
    folder = os.path.realpath(os.path.dirname(os.path.abspath(out)))
    if not os.path.isdir(folder):
        raise TimelineError(f"--out: the folder {folder} does not exist")
    current = folder
    while True:
        if os.path.lexists(os.path.join(current, ".git")):
            raise TimelineError(
                f"--out: {current} is a git checkout; the report holds real track names "
                "and never goes into a repository"
            )
        parent = os.path.dirname(current)
        if parent == current:
            return
        current = parent


def run(args, today=None):
    """Reads the window of ``args``, writes the report; the summary pairs."""
    start = parse_local(args.start, today)
    end = parse_local(args.end, today)
    if end <= start:
        raise TimelineError(f"--to {args.end!r} must be after --from {args.start!r}")
    check_out(args.out)
    if not os.path.isdir(args.events):
        raise TimelineError(f"--events {args.events}: no such folder")
    lead = start - LEAD_MS
    records, files, missing, skipped = read_events(args.events, lead, end, end + TRACE_TAIL_MS)
    marks, logs, notes = read_hub_logs(args.events, lead, end)
    notes = [f"no event file {name}" for name in missing] + notes
    timeline = Timeline(records, marks, start, end, args.key, skipped, notes)
    pairs = summary(timeline)
    text = render(timeline, pairs, files + logs)
    try:
        with open(args.out, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
    except OSError as e:
        raise TimelineError(f"cannot write {args.out}: {e.strerror or e}") from e
    return pairs


def main(argv=None):
    try:
        pairs = run(parse_args(argv))
    except TimelineError as e:
        print(f"timeline: {e}", file=sys.stderr)
        return 1
    for name, value in pairs:
        print(f"{name}={value}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
