"""The forensics timeline's rules (#130 PR 6, D18): why REVERB or STAGE did
something at a time. The page's ``rail`` events (a rail button's press
steps: ``press``, ``tap``, ``full``, ``hold``, ``refused``, ``slid``,
``lost``, ``cancel``, and the AUTOTUNE panel's ``close`` and ``singer``;
``button`` ``stage``, ``reverb`` or ``autotune``; the keys it writes, its
pointer; ``diag/trace.rs`` ``rail``) and the hub's rule records
(``transport``: ``playing`` and ``prev``, or ``error``; ``stage_aut``:
``step`` ``flag``, ``write``, ``failed``; ``reverb``: every step of
``rules/reverb.rs`` ``Step``, ``pending``, ``skipped``, a ``lower`` with no
fader and the instance's ``window`` included; ``fader_link``: ``instance``,
``id``, ``on``; ``autotune``: every step of ``rules/autotune.rs`` ``Step``),
in time order. Pure: the model reads them, the report draws one table,
stdout counts them (never a key, a track label or an error text)."""

import collections

from timeline_read import number

PAGE_EVENTS = ("rail",)
# The hub's rule records (all warn class, ``events.rs`` ``is_warn``: they
# outlive a day file's cap).
HUB_KINDS = ("transport", "stage_aut", "reverb", "fader_link", "autotune")
# The steps that are a failure: a STAGE AUT write Live refused, an AUTOTUNE
# write that failed or whose walks gave up (a transport ``error`` is one too).
FAILED_STEPS = {"stage_aut": ("failed",), "autotune": ("failed", "gave_up")}
# Fields a hub record's detail never shows: the record's frame, the page's
# socket number, and its address (``peer``: never in the report).
FRAME_FIELDS = ("ev", "ts", "step", "client", "peer")

# ``source`` ``page`` or ``hub``; ``kind`` the page event's or the record's
# ``ev``; ``what`` the rail step, the hub's ``step`` (a REVERB window's
# ``open``/``end``), ``play``/``stop``/``error`` for a transport, ``on``/
# ``off`` for a fader link (``?`` when it does not read); ``button`` (rail), ``step`` (a hub record's own ``step``),
# ``keys`` and ``pointer`` (rail), ``detail`` (the table's text), ``failed``.
RuleRow = collections.namedtuple(
    "RuleRow",
    ("time", "source", "kind", "what", "button", "step", "keys", "pointer", "detail", "failed"),
)


def _text(value):
    return value if isinstance(value, str) else None


def _bool(value):
    return value if isinstance(value, bool) else None


def _on_off(value):
    return {True: "on", False: "off"}.get(_bool(value), "n/a")


def rail_row(hub, data):
    """The ``RuleRow`` of a page ``rail`` event at hub time ``hub``. A field
    of the wrong type reads as missing: ``what`` as ``?``, ``button`` and
    ``pointer`` as None, a key that is no text is left out."""
    keys, pointer = data.get("keys"), data.get("pointer")
    keys = tuple(k for k in keys if isinstance(k, str)) if isinstance(keys, list) else ()
    pointer = pointer if isinstance(pointer, int) and not isinstance(pointer, bool) else None
    return RuleRow(
        hub,
        "page",
        "rail",
        _text(data.get("what")) or "?",
        _text(data.get("button")),
        None,
        keys,
        pointer,
        ", ".join(keys),
        False,
    )


def _value_text(value):
    """A field's value in the detail: text, a number, a bool, a list of
    texts; None for anything else (left out)."""
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, str):
        return value
    if number(value) is not None:
        return f"{value:g}"
    if isinstance(value, list) and all(isinstance(v, str) for v in value):
        return ", ".join(value) or "none"
    return None


def generic_detail(record):
    """``name=value`` of a hub record's own fields, sorted (``FRAME_FIELDS``
    and fields of no readable type left out)."""
    parts = []
    for name in sorted(record):
        if name in FRAME_FIELDS:
            continue
        text = _value_text(record[name])
        if text is not None:
            parts.append(f"{name}={text}")
    return " ".join(parts)


def _moves_text(faders):
    """A lowering's or a return's glides: ``label from → to in ms``; a glide
    whose fields do not read is left out."""
    if not isinstance(faders, list):
        return None
    moves = []
    for move in faders:
        if not isinstance(move, dict):
            continue
        label, ms = _text(move.get("label")), number(move.get("ms"))
        start, end = number(move.get("from")), number(move.get("to"))
        if None in (label, ms, start, end):
            continue
        moves.append(f"{label} {start:.3f} → {end:.3f} in {ms:g} ms")
    return "; ".join(moves) if moves else "no fader"


def _transport(record):
    error = _text(record.get("error"))
    instance = _text(record.get("instance")) or "n/a"
    if error is not None:
        return "error", f"{instance}: {error}", True
    playing, prev = _bool(record.get("playing")), _bool(record.get("prev"))
    if playing is None:
        return "?", f"{instance}: n/a", False
    before = {True: "playing", False: "stopped"}.get(prev, "not known")
    now = "playing" if playing else "stopped"
    return ("play" if playing else "stop"), f"{instance}: {now} (before: {before})", False


def _stage(step, record):
    if step == "flag":
        return f"STAGE AUT {_on_off(record.get('on'))}"
    if step == "write":
        mute = _bool(record.get("mute"))
        words = {True: "muted", False: "unmuted"}.get(mute, "n/a")
        return f"{_text(record.get('instance')) or 'n/a'}: the stage mics {words}"
    if step == "failed":
        return f"{_text(record.get('target')) or 'n/a'}: {_text(record.get('problem')) or 'n/a'}"
    return generic_detail(record)


def _reverb(step, record):
    if step == "lower":
        why = _text(record.get("why")) or "n/a"
        return f"by {why}: {_moves_text(record.get('faders')) or 'n/a'}"
    if step == "return":
        return _moves_text(record.get("faders")) or "n/a"
    if step == "window":
        opens = {True: "opens", False: "ends"}.get(_bool(record.get("open")), "n/a")
        instance = _text(record.get("instance")) or "n/a"
        return f"{instance}: the window {opens} ({_text(record.get('why')) or 'n/a'})"
    if step in ("pending", "skipped", "stopped"):
        label = _text(record.get("label")) or "n/a"
        return f"{label}: {_text(record.get('why')) or 'n/a'}"
    return generic_detail(record)


def hub_row(record):
    """The ``RuleRow`` of a hub rule record (``HUB_KINDS``). A field of the
    wrong type reads as missing (``step`` and ``what`` as ``?``)."""
    kind = record.get("ev")
    step = None
    failed = False
    if kind == "transport":
        what, detail, failed = _transport(record)
    elif kind == "fader_link":
        on = _bool(record.get("on"))
        what = {True: "on", False: "off"}.get(on, "?")
        link = f"{_text(record.get('instance')) or 'n/a'} {_text(record.get('id')) or 'n/a'}"
        detail = f"{link}: {_on_off(on)}"
    else:
        step = _text(record.get("step")) or "?"
        what = step
        failed = step in FAILED_STEPS.get(kind, ())
        if kind == "reverb" and step == "window":
            # A window's own word: it opens or ends.
            what = {True: "open", False: "end"}.get(_bool(record.get("open")), "?")
        if kind == "stage_aut":
            detail = _stage(step, record)
        elif kind == "reverb":
            detail = _reverb(step, record)
        else:
            detail = generic_detail(record)
    return RuleRow(record["ts"], "hub", kind, what, None, step, (), None, detail, failed)


def rows(records, rails):
    """The window's rows in time order: ``records`` the hub's rule records
    (in the event log's order) and ``rails`` (hub time, data) of the page's
    ``rail`` events. At one time a page's step comes before the hub's
    records (the press the hub answers); records keep their log order."""
    found = [(r.time, 0, n, r) for n, r in enumerate(rail_row(hub, data) for hub, data in rails)]
    found += [(r["ts"], 1, n, hub_row(r)) for n, r in enumerate(records)]
    return [row for *_, row in sorted(found, key=lambda f: f[:3])]


def summary(found):
    """The rules' stdout pairs (counts only: no key, label or error text)."""

    def count(test):
        return str(sum(1 for r in found if test(r)))

    return [
        ("rule_rail_steps", count(lambda r: r.source == "page")),
        ("rule_records", count(lambda r: r.source == "hub")),
        ("reverb_lowerings", count(lambda r: r.kind == "reverb" and r.step == "lower")),
        (
            "reverb_windows",
            count(lambda r: r.kind == "reverb" and r.step == "window" and r.what == "open"),
        ),
        ("stage_writes", count(lambda r: r.kind == "stage_aut" and r.step == "write")),
        (
            "transport_changes",
            count(lambda r: r.kind == "transport" and r.what in ("play", "stop")),
        ),
        ("rule_failures", count(lambda r: r.failed)),
    ]
