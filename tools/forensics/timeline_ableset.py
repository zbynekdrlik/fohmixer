"""The forensics timeline's AbleSet tab (#90, design §6): what the hub
recorded of its link to AbleSet (``ableset_link``: a change, up, or down
with why, and the failed reads in a row; at most one per 5 s read, warn
class, read from the start of the window's first day file) and what the
page recorded of the tab (``ableset_view``: the tab opened or closed;
``ableset``: the probe's answer and its ms, the frame's load ms from its
mount, each panel shown, a press of the sign-in). The hub's link is the
truth about AbleSet: the page's down panel also shows while the page's own
socket is down, so each of the page's steps carries the link at its time.
Pure: the model reads them, the report draws a table, stdout counts them."""

import bisect
import collections

from timeline_read import number

# The hub reads AbleSet's index this often (ms, ``ableset/link.rs``
# ``PERIOD``) and takes the link down at the second failed read in a row
# (``DOWN_AFTER``).
PERIOD_MS = 5000.0
DOWN_AFTER = 2

PAGE_EVENTS = ("ableset_view", "ableset")

AbleSetLink = collections.namedtuple("AbleSetLink", ("time", "up", "error", "failures"))
# ``read``: its down record was read; else its start is the up's failed
# reads counted back.
AbleSetOutage = collections.namedtuple("AbleSetOutage", ("start", "end", "ms", "error", "read"))
AbleSetStep = collections.namedtuple("AbleSetStep", ("time", "what", "ok", "ms", "panel", "link"))


def _count(value):
    """An int that is no bool, else None."""
    return value if isinstance(value, int) and not isinstance(value, bool) else None


def _text(value):
    return value if isinstance(value, str) else None


def links(records):
    """The ``AbleSetLink`` of each ``ableset_link`` record (in time order);
    one whose ``up`` is not a bool is left out."""
    return [
        AbleSetLink(r["ts"], r["up"], _text(r.get("error")), _count(r.get("failures")))
        for r in records
        if isinstance(r.get("up"), bool)
    ]


def link_at(links, ms):
    """The hub's link to AbleSet at ``ms``: True (up) or False (down) by the
    latest change at or before it, None when none was read."""
    i = bisect.bisect_right([link.time for link in links], ms)
    return links[i - 1].up if i else None


def state_text(up):
    """``up``, ``down`` or ``n/a`` (no change read)."""
    return "n/a" if up is None else ("up" if up else "down")


def outages(links, start, end):
    """AbleSet's outages, the hub's link down, that overlap ``start``..``end``
    (``links`` in time order): from a down to the next up, else to ``end``.
    An up whose down was not read (before the first day file read) went down
    at the second of its failed reads, ``failures - 1`` periods before it."""
    found = []
    down = None
    for link in links:
        if not link.up and down is None:
            down = link
        elif link.up and down is not None:
            found.append(
                AbleSetOutage(down.time, link.time, link.time - down.time, down.error, True)
            )
            down = None
        elif link.up and (link.failures or 0) >= DOWN_AFTER:
            ms = (link.failures - 1) * PERIOD_MS
            found.append(AbleSetOutage(link.time - ms, link.time, ms, None, False))
    if down is not None:
        found.append(AbleSetOutage(down.time, end, end - down.time, down.error, True))
    return [o for o in found if o.end >= start and o.start <= end]


def step(hub, data, links):
    """The ``AbleSetStep`` of a page ``ableset_view`` or ``ableset`` event at
    hub time ``hub``, with the hub's link then (``link_at``): ``what`` is
    ``open`` or ``close`` for the view (its ``on``), else the event's own
    ``what`` (``probe``, ``load``, ``panel``, ``signin``). A field of the
    wrong type reads as missing (``what`` as ``?``)."""
    if data.get("ev") == "ableset_view":
        on = data.get("on")
        what = "open" if on is True else ("close" if on is False else "?")
    else:
        what = _text(data.get("what")) or "?"
    ok = data.get("ok")
    return AbleSetStep(
        hub,
        what,
        ok if isinstance(ok, bool) else None,
        number(data.get("ms")),
        _text(data.get("panel")),
        link_at(links, hub),
    )
