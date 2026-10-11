"""The forensics timeline's Pro-Q 4 screen (#71, acceptance point 6): what
the page recorded of each finger on the editor's picture (``eq_touch``:
down, up, cancel, and ``stale``, a finger refused on an old picture, PR J)
and of the picture's latency (``eq_lat``, once a second of drawing: the
frames drawn, every input-to-draw sample in ms, the longest decode), and
what the hub recorded (``eq`` records: ``stale``, a down it dropped; the
``rate`` and ``frames`` records' frame round trips and the frames that went
because an ack did not come). Pure: the model reads them, the report draws
a table, stdout counts them."""

import collections

from timeline_read import number

EqFinger = collections.namedtuple("EqFinger", ("time", "what", "pointer", "x", "y", "zoom"))
EqLatency = collections.namedtuple("EqLatency", ("time", "frames", "samples", "lost", "decode_ms"))
EqLink = collections.namedtuple("EqLink", ("time", "what", "trips", "rtt_max_ms", "ack_waits"))


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
