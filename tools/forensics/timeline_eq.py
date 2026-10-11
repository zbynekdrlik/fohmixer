"""The forensics timeline's Pro-Q 4 screen (#71, acceptance point 6): what
the page recorded of each finger on the editor's picture (``eq_touch``:
down, up, cancel, ``stale``, a finger refused on an old picture, PR J, and
``uncovered``, one refused on the strip a pan brought into sight before its
part was drawn, PR L)
and of the picture's latency (``eq_lat``, once a second of drawing: the
frames drawn, every input-to-draw sample in ms, the inputs shown whose
send time the page no longer kept, the longest decode), and
what the hub recorded (``eq`` records: ``stale``, a down it dropped; the
``rate`` and ``frames`` records' frame round trips and the frames that went
because an ack did not come; PR P2a's ``undo``, a page's undo of Pro-Q's
own ↶, and ``undo_read``, the hub's read of the ↶ after an open or a
resize, never tapped). Pure: the model reads them, the report draws a
table, stdout counts them."""

import collections

from timeline_read import number

EqFinger = collections.namedtuple("EqFinger", ("time", "what", "pointer", "x", "y", "zoom"))
EqLatency = collections.namedtuple("EqLatency", ("time", "frames", "samples", "lost", "decode_ms"))
EqLink = collections.namedtuple("EqLink", ("time", "what", "trips", "rtt_max_ms", "ack_waits"))
EqUndo = collections.namedtuple(
    "EqUndo", ("time", "what", "session", "result", "x", "y", "width", "height")
)

UNDO_RECORDS = ("undo", "undo_read")


def _int(value):
    """An int that is no bool, else None."""
    return value if isinstance(value, int) and not isinstance(value, bool) else None


def eq_finger(hub, data):
    """The ``EqFinger`` of a page ``eq_touch`` event at hub time ``hub``. A
    field of the wrong type reads as missing (``what`` as ``?``)."""
    what = data.get("what")
    return EqFinger(
        hub,
        what if isinstance(what, str) else "?",
        _int(data.get("p")),
        number(data.get("x")),
        number(data.get("y")),
        number(data.get("z")),
    )


def eq_latency(hub, data):
    """The ``EqLatency`` of a page ``eq_lat`` event at hub time ``hub``: its
    samples are its numbers in ``ms`` (anything else is left out), ``lost``
    the inputs it showed whose send time was gone (0 when missing)."""
    samples = data.get("ms")
    return EqLatency(
        hub,
        _int(data.get("f")) or 0,
        tuple(s for s in (number(v) for v in samples) if s is not None)
        if isinstance(samples, list)
        else (),
        _int(data.get("lost")) or 0,
        number(data.get("dec")),
    )


def eq_link(record):
    """The ``EqLink`` of a hub ``eq`` ``rate`` or ``frames`` record that
    carries frame round trips; None for one that does not."""
    trips = _int(record.get("frame_trips"))
    if trips is None:
        return None
    return EqLink(
        record["ts"],
        record.get("what"),
        trips,
        number(record.get("frame_rtt_max_ms")),
        _int(record.get("ack_waits")) or 0,
    )


def samples_of(latencies):
    """Every latency sample of the windows, in order."""
    return [s for lat in latencies for s in lat.samples]


def eq_undo(record):
    """The ``EqUndo`` of a hub ``eq`` ``undo`` record (#71 PR P2a: a page's
    undo, its ``result`` ``done``, ``nothing`` or why the hub refused it, the
    tap's ``x``, ``y`` when it tapped, the picture's ``w``, ``h``) or
    ``undo_read`` record (the hub's read of the ↶ after an open or a
    resize, never tapped: its ``state`` ``lit``, ``grey`` or ``missing`` as
    the result, the ``x`` it found). A field of the wrong type reads as
    missing (the result as ``?``)."""
    what = record.get("what")
    result = record.get("state" if what == "undo_read" else "result")
    return EqUndo(
        record["ts"],
        what,
        _int(record.get("session")),
        result if isinstance(result, str) else "?",
        _int(record.get("x")),
        _int(record.get("y")),
        _int(record.get("w")),
        _int(record.get("h")),
    )
