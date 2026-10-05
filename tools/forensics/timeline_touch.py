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

- **First-touch jump:** the touch's first applied value (Live's result of
  the first of its own sets that Live applied) is more than ``FIRST_JUMP_DB``
  from ``live_before`` (Live's value before the touch's first set, as the hub
  knew it), and more than ``FIRST_JUMP_DB`` from where the finger alone
  would have taken Live: ``live_before``'s position moved by the finger's own
  travel up to that set's frame (its ``r`` minus ``from``), so a fader that
  went up while the finger went down counts too. Its why: ``stale`` (the
  page's value of Live, ``live``, was more than ``FIRST_JUMP_DB`` from
  ``live_before``), ``local`` (the touch started from the fader's own
  position: a hold, an open write), else ``other``.
- **Down to first move / first send:** on the page's clock, from the down's
  own pointer event to the first move's (to the first set's ``t``).
- **Stutter:** a move gap is two consecutive pointer moves over
  ``MOVE_GAP_MS`` apart whose coordinate changed over ``MOVE_GAP_PX`` across
  it (the finger went on while no move came; a finger at rest sends none and
  moves on by little); a held run is ``HELD_FRAMES`` or more frames in a row
  that each sent the previous frame's value while the finger's position
  changed, not at the travel's ends.
- **First move (PR F):** a touch's first pointer move only anchors the drag
  (the iPad's first event of a touch comes late and several px away); the
  touch's first ``mv`` says where that move left the fader (``a``).
  ``anchored``: ``a`` is the touch's start (``from``, within ``RAW_STEP``);
  ``applied``: it moved the fader; none for an older page or a touch that
  sent nothing. For an anchored touch, where the finger alone would have
  taken Live counts the finger's travel from that first move, not from the
  down (the drag never applied the way before it), and from the coordinates,
  unclamped: the drag is relative, so the fader follows the finger even
  where its 1:1 position from the down (``r``) is past an end.
- **No data (PR E):** past its backlog's bound the page's recorder drops
  moves, oldest first, and says so with the page time of the oldest and the
  newest it dropped (an ``overflow`` marker of kind ``mv``). Such a span is
  a hole in the record, not in the drag: a move gap or two frames of a held
  run across it do not count, and the touch reports the span's length and
  the hub's sets of the touch inside it (they reached the hub; only the
  finger's moves are missing).
"""

import collections
import itertools
import math

from timeline_read import number, to_live, to_pos, value2db

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
        "off_db",
        "first_jump",
        "why",
        "first_move_ms",
        "first_send_ms",
        "move_gaps",
        "held_runs",
        "frames",
        "no_data",
        "no_data_ms",
        "no_data_sets",
        "first_move",
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


def over_jump(db):
    """Whether a difference of ``db`` is more than ``FIRST_JUMP_DB``."""
    return db > FIRST_JUMP_DB


def is_move_gap(ms, px):
    """Whether two consecutive moves ``ms`` apart and ``px`` apart are a move
    gap: over ``MOVE_GAP_MS`` and over ``MOVE_GAP_PX``."""
    return ms > MOVE_GAP_MS and px > MOVE_GAP_PX


def finger_moved(dr):
    """Whether a change ``dr`` of the finger's position is a move (over
    ``RAW_STEP``)."""
    return abs(dr) > RAW_STEP


def inside_travel(s):
    """Whether a sent position ``s`` lies strictly inside 0..1 (at an end the
    value stays put by design)."""
    return 0.0 < s < 1.0


def first_touch(start, live, local, live_before, first_applied, raw, base=None):
    """A touch that started at position ``start`` (the page's Live position
    ``live``, ``local``) when Live held ``live_before``, whose first applied
    value was ``first_applied`` while the finger was at ``raw`` (the 1:1
    position of the frame that sent it): (its jump from Live's value in dB,
    the finger's own move in dB, how far it landed from where the finger
    alone would have taken Live in dB, whether it is a first-touch jump,
    why). The finger's move counts from ``base``, the 1:1 position the drag
    counts from (``start``, or an anchored first move's, PR F). The dB are
    None, and it is no jump, when a value is missing."""
    if None in (start, live_before, first_applied, raw):
        return None, None, None, False, None
    base = start if base is None else base
    applied_db = value2db(first_applied)
    jump = db_apart(applied_db, value2db(live_before))
    finger = db_apart(pos_db(raw), pos_db(base))
    expected = to_live(to_pos(live_before) + raw - base)
    off = db_apart(applied_db, value2db(expected))
    if not (over_jump(jump) and over_jump(off)):
        return jump, finger, off, False, None
    if local:
        why = "local"
    elif live is not None and over_jump(db_apart(pos_db(live), value2db(live_before))):
        why = "stale"
    else:
        why = "other"
    return jump, finger, off, True, why


def first_move_of(frames, start):
    """Whether a touch's first pointer move moved the fader (PR F), from its
    first ``mv`` record's ``a`` (where that move left the fader) against the
    touch's ``start``: ``anchored`` (no move over ``RAW_STEP``), ``applied``,
    or None (an older page's record, no frame or no start)."""
    if not frames or start is None:
        return None
    anchor = number(frames[0].get("a"))
    if anchor is None:
        return None
    return "applied" if finger_moved(anchor - start) else "anchored"


def drag_finger(down, frames, seq, start, how):
    """Where the finger stood at the frame ``frame_at`` picks for set ``seq``
    and the 1:1 position its drag counts from, as (raw, base) for
    ``first_touch``: that frame's ``r`` and ``start``. For an anchored first
    move (``how``, PR F) ``raw`` is ``start`` plus the finger's travel from
    that first move to the frame's last move (up the screen, of the down's
    ``travel``, at least 1 px as the page counts it), unclamped: the drag runs
    relatively from where the touch started, so the fader follows the finger
    even where ``r`` (clamped to 0..1 from the down) is past an end."""
    raw = finger_at(frames, seq)
    moves = moves_of(frames)
    last = moves_of([frame_at(frames, seq)]) if frames else []
    travel = number(down.get("travel"))
    if how != "anchored" or not moves or not last or travel is None:
        return raw, start
    return start + (moves[0][1] - last[-1][1]) / max(travel, 1.0), start


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


def across(t1, t2, holes):
    """Whether page times ``t1`` to ``t2`` reach into one of ``holes`` ((from,
    to) of moves the recorder dropped, page clock): no data between them."""
    return any(t1 <= to and t2 >= start for start, to in holes)


def move_gaps(moves, holes=()):
    """The move gaps of a touch's ``moves`` ((time, coordinate) in order); a
    gap across one of ``holes`` is no data, not a gap."""
    gaps = []
    for (t1, c1), (t2, c2) in itertools.pairwise(moves):
        if is_move_gap(t2 - t1, abs(c2 - c1)) and not across(t1, t2, holes):
            gaps.append(Gap(t1, t2, t2 - t1, abs(c2 - c1)))
    return gaps


def holds(previous, frame):
    """Whether ``frame`` sent ``previous``'s value although the finger moved,
    inside the travel (at an end the value stays put by design)."""
    r1, r2 = number(previous.get("r")), number(frame.get("r"))
    s1, s2 = number(previous.get("s")), number(frame.get("s"))
    if None in (r1, r2, s1, s2):
        return False
    return s1 == s2 and inside_travel(s2) and finger_moved(r2 - r1)


def held_runs(frames, holes=()):
    """How many held runs a touch's ``frames`` hold; two frames across one
    of ``holes`` are not in a row (frames between them went)."""
    runs = 0
    length = 0
    for previous, frame in itertools.pairwise(frames):
        t1, t2 = number(previous.get("t")), number(frame.get("t"))
        broken = None not in (t1, t2) and across(t1, t2, holes)
        if holds(previous, frame) and not broken:
            length += 1
            runs += length == HELD_FRAMES
        else:
            length = 0
    return runs


def no_data(holes, begin, end):
    """The ``holes`` ((from, to), page clock) that reach into a touch from
    ``begin`` to ``end``, cut to it, in time order."""
    return sorted(
        (max(start, begin), min(to, end))
        for start, to in holes
        if across(begin, end, [(start, to)])
    )


def event_time(data):
    """A page event's own time on the page clock: ``t`` plus its ``dt`` (the
    pointer event's own time) when it has one."""
    t = number(data.get("t"))
    if t is None:
        return None
    return t + (number(data.get("dt")) or 0.0)


def own_frames(frames, key, pointer, begin, end):
    """The ``mv`` records (data) of a touch of ``key`` by ``pointer`` from
    page time ``begin`` to ``end`` (both included), in page-time order (the
    order they were taken: records uploaded on two sockets can come back in
    another hub order)."""
    mine = [
        f
        for f in frames
        if f.get("key") == key and f.get("p") == pointer and within(f.get("t"), begin, end)
    ]
    return sorted(mine, key=lambda f: number(f.get("t")))


def within(t, begin, end):
    """Whether page time ``t`` lies from ``begin`` to ``end``, both included."""
    t = number(t)
    return t is not None and begin <= t <= end


def is_start(data):
    """Whether a page ``touch`` is a down that says where it started (PR D)."""
    return data.get("what") == "down" and number(data.get("from")) is not None


def analyse(down, key, frames, first_set, applied, holes=(), hole_sets=0):
    """The ``Touch`` of one down (a PageEvent of a single volume key, with its
    start): ``frames`` its ``mv`` records' data in order, ``first_set`` the
    hub's first set of the touch (None when none), ``applied`` the first value
    Live applied of the touch's own sets as (value, the seq of its set), or
    None; ``holes`` the spans of moves the recorder dropped inside the touch
    (page clock, ``no_data``) and ``hole_sets`` the touch's own sets inside
    them. The finger is read at the frame that sent that set (its ``q``),
    else at the last frame before it, else at the first frame."""
    data = down.data
    start = number(data.get("from"))
    live = number(data.get("live"))
    live_before = number(first_set.get("live_before")) if first_set else None
    value, seq = applied if applied is not None else (None, None)
    moves = moves_of(frames)
    how = first_move_of(frames, start)
    raw, base = drag_finger(data, frames, seq, start, how)
    jump, finger, off, flagged, why = first_touch(
        start,
        live,
        data.get("local") is True,
        live_before,
        number(value),
        raw,
        base,
    )
    pressed = event_time(data)
    first_move = moves[0][0] - pressed if moves and pressed is not None else None
    sent = number(first_set.get("t")) if first_set else None
    first_send = sent - pressed if sent is not None and pressed is not None else None
    return Touch(
        down.hub,
        key,
        data.get("pointer"),
        start,
        live,
        data.get("local") is True,
        live_before,
        number(value),
        jump,
        finger,
        off,
        flagged,
        why,
        first_move,
        first_send,
        move_gaps(moves, holes),
        held_runs(frames, holes),
        len(frames),
        len(holes),
        sum(to - start for start, to in holes),
        hole_sets,
        how,
    )


def frame_at(frames, seq):
    """The frame that sent set ``seq`` (its ``q``), else the last frame
    before it, else the first frame; None without a frame."""
    if not frames:
        return None
    if seq is not None:
        before = [f for f in frames if (number(f.get("q")) or -1) <= seq]
        if before:
            return before[-1]
    return frames[0]


def finger_at(frames, seq):
    """The 1:1 finger position (``r``) of the frame ``frame_at`` picks for set
    ``seq``; None without a frame."""
    frame = frame_at(frames, seq)
    return None if frame is None else number(frame.get("r"))
