"""The forensics timeline's touches of a single volume fader (#43, PR D): where
a touch started against Live's value before it (the hub's ``live_before``),
the first-touch jump, and the stutter of a drag. Pure: the analysis
(``timeline_model.py``) hands it each touch's records. One of the five files
of ``timeline.py`` (see its docstring), copied to the Ableton PC with it.

The page's records (``diag/trace/moves.rs``): a ``touch`` down with its start
(``dt``: the pointer event's time minus ``t``; ``c``: its coordinate; ``pos``,
``live``, ``from``: positions 0..1 of the travel; ``local``) and per frame
that sent from the finger an ``mv`` (``e``: ``[dt, c]`` of each pointer move,
``r``: the 1:1 finger position, ``s``: the position sent, ``q``: its set's
seq). A volume's Live value at position p is p^0.515.

- **First-touch jump:** the first applied value of the touch is more than
  ``FIRST_JUMP_DB`` from ``live_before`` (Live's value before the touch's
  first set, as the hub knew it), and more than ``FIRST_JUMP_DB`` of that is
  not the finger's: the dB the finger alone moved, from ``from`` to the first
  frame's ``r``. Its why: ``stale`` (the touch started from the page's value
  of Live, which differed from the hub's by over ``FIRST_JUMP_DB``),
  ``local`` (it started from the fader's own position: a hold, an open
  write), else ``other``.
- **Down to first move / first send:** on the page's clock, from the down's
  own pointer event to the first move's (to the first set's ``t``).
- **Stutter:** a move gap is two consecutive pointer moves over
  ``MOVE_GAP_MS`` apart whose coordinate changed over ``MOVE_GAP_PX`` across
  it (the finger went on while no move came; a finger at rest sends none and
  moves on by little); a held run is ``HELD_FRAMES`` or more frames in a row
  that each sent the previous frame's value while the finger's position
  changed, not at the travel's ends.
"""

import collections
import itertools
import math

from timeline_read import number, to_live, value2db

# A first applied value further than this from Live's before (dB), with more
# than this of it not the finger's, is a first-touch jump.
FIRST_JUMP_DB = 1.0
# Two consecutive moves further apart than this (ms) ...
MOVE_GAP_MS = 50.0
# ... with the finger more than this further (px) are a move gap.
MOVE_GAP_PX = 3.0
# This many frames in a row holding a value while the finger moved are a
# held run.
HELD_FRAMES = 3
# A finger position change smaller than this is no move.
RAW_STEP = 1e-4

Touch = collections.namedtuple(
    "Touch",
    (
        "time",
        "key",
        "pointer",
        "start",
        "live",
        "local",
        "live_before",
        "first_applied",
        "jump_db",
        "finger_db",
        "first_jump",
        "why",
        "first_move_ms",
        "first_send_ms",
        "move_gaps",
        "held_runs",
        "frames",
    ),
)
Gap = collections.namedtuple("Gap", ("start", "end", "ms", "px"))


def db_apart(a, b):
    """How far apart two dB levels are; -inf against -inf is 0, against a
    finite level infinite."""
    if math.isinf(a) and math.isinf(b):
        return 0.0
    return abs(a - b)


def pos_db(p):
    """The dB of a volume fader at position ``p``."""
    return value2db(to_live(p))


def first_touch(start, live, local, live_before, first_applied, first_raw):
    """(jump dB, finger dB, whether it is a first-touch jump, why) of a touch
    that started at position ``start`` (the page's Live position ``live``,
    ``local``), when Live held ``live_before``, the first applied value was
    ``first_applied`` and the finger's first frame was at ``first_raw``. The
    dB are None, and it is no jump, when a value is missing."""
    if None in (start, live_before, first_applied, first_raw):
        return None, None, False, None
    jump = db_apart(value2db(first_applied), value2db(live_before))
    finger = db_apart(pos_db(first_raw), pos_db(start))
    if not (jump > FIRST_JUMP_DB and jump - finger > FIRST_JUMP_DB):
        return jump, finger, False, None
    if db_apart(pos_db(start), value2db(live_before)) <= FIRST_JUMP_DB:
        return jump, finger, True, "other"
    return jump, finger, True, "local" if local else "stale"


def moves_of(frames):
    """The pointer moves of ``frames`` (``mv`` records' data, in order) as
    (page time, coordinate), in time order; a malformed entry is left out."""
    moves = []
    for frame in frames:
        t = number(frame.get("t"))
        entries = frame.get("e")
        if t is None or not isinstance(entries, list):
            continue
        for entry in entries:
            if isinstance(entry, list) and len(entry) == 2:
                dt, c = number(entry[0]), number(entry[1])
                if dt is not None and c is not None:
                    moves.append((t + dt, c))
    return sorted(moves)


def move_gaps(moves):
    """The move gaps of a touch's ``moves`` ((time, coordinate) in order)."""
    gaps = []
    for (t1, c1), (t2, c2) in itertools.pairwise(moves):
        if t2 - t1 > MOVE_GAP_MS and abs(c2 - c1) > MOVE_GAP_PX:
            gaps.append(Gap(t1, t2, t2 - t1, abs(c2 - c1)))
    return gaps


def holds(previous, frame):
    """Whether ``frame`` sent ``previous``'s value although the finger moved,
    inside the travel (at an end the value stays put by design)."""
    r1, r2 = number(previous.get("r")), number(frame.get("r"))
    s1, s2 = number(previous.get("s")), number(frame.get("s"))
    if None in (r1, r2, s1, s2):
        return False
    return s1 == s2 and 0.0 < s2 < 1.0 and abs(r2 - r1) > RAW_STEP


def held_runs(frames):
    """How many held runs a touch's ``frames`` hold."""
    runs = 0
    length = 0
    for previous, frame in itertools.pairwise(frames):
        if holds(previous, frame):
            length += 1
            runs += length == HELD_FRAMES
        else:
            length = 0
    return runs


def event_time(data):
    """A page event's own time on the page clock: ``t`` plus its ``dt`` (the
    pointer event's own time) when it has one."""
    t = number(data.get("t"))
    if t is None:
        return None
    return t + (number(data.get("dt")) or 0.0)


def is_start(data):
    """Whether a page ``touch`` is a down that says where it started (PR D)."""
    return data.get("what") == "down" and number(data.get("from")) is not None


def analyse(down, key, frames, first_set, first_applied):
    """The ``Touch`` of one down (a PageEvent of a single volume key, with its
    start): ``frames`` its ``mv`` records' data in order, ``first_set`` the
    hub's first set of the touch (None when none), ``first_applied`` the
    first value Live applied for it (None when none)."""
    data = down.data
    start = number(data.get("from"))
    live_before = number(first_set.get("live_before")) if first_set else None
    first_raw = number(frames[0].get("r")) if frames else None
    jump, finger, flagged, why = first_touch(
        start,
        number(data.get("live")),
        data.get("local") is True,
        live_before,
        number(first_applied),
        first_raw,
    )
    pressed = event_time(data)
    moves = moves_of(frames)
    first_move = moves[0][0] - pressed if moves and pressed is not None else None
    sent = number(first_set.get("t")) if first_set else None
    first_send = sent - pressed if sent is not None and pressed is not None else None
    return Touch(
        down.hub,
        key,
        data.get("pointer"),
        start,
        number(data.get("live")),
        data.get("local") is True,
        live_before,
        number(first_applied),
        jump,
        finger,
        flagged,
        why,
        first_move,
        first_send,
        move_gaps(moves),
        held_runs(frames),
        len(frames),
    )
