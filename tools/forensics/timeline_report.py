"""The forensics timeline's report (#43): one self-contained HTML file,
inline CSS and SVG, no script. One of the five files of ``timeline.py`` (see
its docstring), copied to the Ableton PC together with it.
"""

import datetime
import html
import json
import math

from timeline_model import ROWS, Span, ms_text, no_data_text
from timeline_read import key_hash, key_scale, local_text, number, to_live, utc_text, value2db

# The report's geometry (px).
WIDTH = 1600
PLOT_X0 = 190
# Room on the right for the last tick's label.
PLOT_X1 = WIDTH - 40
AXIS_H = 28
LINK_H = 110
LIVE_H = 36
KEY_TITLE_H = 18
ROW_H = 34
LANE_GAP = 10
# The round-trip axis is at least this tall (ms).
RTT_FLOOR_MS = 100.0
# Tick spacings of the time axis (ms), the first that gives at most MAX_TICKS.
TICK_STEPS_MS = (
    100,
    200,
    500,
    1000,
    2000,
    5000,
    10000,
    15000,
    30000,
    60000,
    120000,
    300000,
    600000,
    900000,
    1800000,
    3600000,
    7200000,
    21600000,
    43200000,
    86400000,
)
MAX_TICKS = 12


# --- the report (HTML, inline CSS and SVG) ---

CSS = """
body { font: 13px/1.4 system-ui, sans-serif; margin: 16px; color: #222; background: #fff; }
h1 { font-size: 18px; margin: 0 0 8px; }
table { border-collapse: collapse; margin: 8px 0 16px; }
th, td { border: 1px solid #ccc; padding: 2px 8px; text-align: left; vertical-align: top; }
td { font-variant-numeric: tabular-nums; }
.notes li { color: #8a4b00; }
svg { display: block; background: #fafafa; border: 1px solid #ddd; }
svg text { font: 11px system-ui, sans-serif; fill: #333; }
svg .tick { fill: #555; }
svg .grid { stroke: #e3e3e3; }
svg .lane { fill: #fff; stroke: #ddd; }
svg .key { font-weight: 600; }
svg .dropout { fill: rgba(220, 30, 30, 0.25); stroke: #c00; }
svg .busy { fill: rgba(255, 150, 0, 0.45); stroke: #e08000; }
svg .gap { fill: rgba(255, 200, 0, 0.45); }
svg .frame { fill: rgba(130, 60, 200, 0.45); }
svg .nodata { fill: rgba(120, 120, 120, 0.22); stroke: #777; stroke-dasharray: 2 2; }
svg .intent.unconfirmed { fill: #e08000; }
svg .intent.not_sent { fill: #c00; }
svg .reset { fill: #06c; }
svg .sock { stroke: #444; stroke-width: 1; }
svg .visibility { stroke: #888; stroke-dasharray: 3 3; }
svg .late-heartbeat { fill: #b35900; }
svg .gesture { fill: #a0a; }
svg .jump { fill: #c00; }
svg .rtt-page { fill: none; stroke: #06c; stroke-width: 1.2; }
svg .rtt-hub { fill: none; stroke: #2a2; stroke-width: 1.2; }
svg .value { fill: none; stroke: #222; stroke-width: 1.2; }
svg .dot { fill: #222; }
svg .unsent { fill: #fff; stroke: #c00; stroke-width: 1.2; }
"""


def esc(value):
    return html.escape(str(value), quote=True)


def tag(name, content=None, **attributes):
    """An element: ``class_`` is ``class``, ``data_x`` is ``data-x``; a None
    attribute is left out; ``content`` is markup."""
    text = "".join(
        f' {"class" if k == "class_" else k.replace("_", "-")}="{esc(v)}"'
        for k, v in attributes.items()
        if v is not None
    )
    if content is None:
        return f"<{name}{text}/>"
    return f"<{name}{text}>{content}</{name}>"


def title(text):
    return f"<title>{esc(text)}</title>"


def num(value):
    return f"{value:.1f}"


class Axis:
    """The window's time axis: epoch ms to x (px)."""

    def __init__(self, start, end):
        self.start, self.end = start, end

    def x(self, ms):
        ms = min(max(ms, self.start), self.end)
        return PLOT_X0 + (ms - self.start) * (PLOT_X1 - PLOT_X0) / (self.end - self.start)

    def ticks(self):
        """(ms, label) of the ticks: round local times."""
        span = self.end - self.start
        step = next((s for s in TICK_STEPS_MS if span / s <= MAX_TICKS), TICK_STEPS_MS[-1])
        local = datetime.datetime.fromtimestamp(self.start / 1000.0).astimezone()
        offset = local.utcoffset().total_seconds() * 1000.0
        ms = math.ceil((self.start + offset) / step) * step - offset
        ticks = []
        while ms <= self.end:
            moment = datetime.datetime.fromtimestamp(ms / 1000.0)
            label = moment.strftime("%H:%M:%S")
            if step < 1000:
                label += f".{moment.microsecond // 1000:03d}"
            ticks.append((ms, label))
            ms += step
        return ticks


def _band(axis, span, y, height, class_, tip, **data):
    x0, x1 = axis.x(span.start), axis.x(span.end)
    return tag(
        "rect",
        title(tip),
        class_=class_,
        x=num(x0),
        y=num(y),
        width=num(max(x1 - x0, 1.0)),
        height=num(height),
        **data,
    )


def _mark(axis, ms, y, height, class_, tip, width=3.0, **data):
    return tag(
        "rect",
        title(tip),
        class_=class_,
        x=num(axis.x(ms) - width / 2),
        y=num(y),
        width=num(width),
        height=num(height),
        **data,
    )


def _polyline(axis, points, class_, y_of):
    coords = " ".join(f"{num(axis.x(ms))},{num(y_of(value))}" for ms, value in points)
    return tag("polyline", "", class_=class_, points=coords) if points else ""


def _link_lane(t, axis, y):
    parts = [
        tag("rect", None, class_="lane", x=PLOT_X0, y=y, width=PLOT_X1 - PLOT_X0, height=LINK_H)
    ]
    top = max([RTT_FLOOR_MS] + [rtt for _, rtt in t.rtt_page + t.rtt_hub])
    parts.append(tag("text", "link", x=8, y=y + 14))
    parts.append(tag("text", esc(f"RTT 0-{top:.0f} ms"), x=8, y=y + 30))
    parts.append(tag("text", esc("page RTT (blue), hub ping RTT (green)"), x=8, y=y + 46))
    for d in t.dropouts:
        tip = f"dropout {d.ms:.1f} ms from {local_text(d.start)}"
        tip += ", socket lost" if d.info else ", socket kept"
        parts.append(
            _band(
                axis,
                d,
                y,
                LINK_H,
                "dropout",
                tip,
                data_start=num(d.start),
                data_end=num(d.end),
                data_ms=num(d.ms),
                data_socket_lost="true" if d.info else "false",
            )
        )
    for f in t.frames:
        tip = f"long frame {f.ms:.1f} ms at {local_text(f.start)}"
        parts.append(_band(axis, f, y + LINK_H - 12, 12, "frame", tip, data_start=num(f.start)))
    for d in t.no_data:
        kind, n = d.info[0], d.info[1]
        tip = f"no data: {no_data_text(d)}"
        parts.append(
            _band(
                axis,
                d,
                y,
                LINK_H,
                "nodata",
                tip,
                data_kind=kind,
                data_n=n,
                data_start=num(d.start),
                data_end=num(d.end),
            )
        )

    def y_of(rtt):
        return y + LINK_H - 4 - min(rtt, top) / top * (LINK_H - 8)

    parts.append(_polyline(axis, t.rtt_page, "rtt-page", y_of))
    parts.append(_polyline(axis, t.rtt_hub, "rtt-hub", y_of))
    for e in t.resets:
        tip = f"counter reset at {local_text(e.hub)} (it showed {e.data.get('count')})"
        parts.append(_mark(axis, e.hub, y, 12, "reset", tip, width=4.0))
    for ms, tip in t.socks:
        x = num(axis.x(ms))
        parts.append(tag("line", title(tip), class_="sock", x1=x, y1=y, x2=x, y2=y + LINK_H))
    for e in t.visibility:
        x = num(axis.x(e.hub))
        tip = "page hidden" if e.data.get("hidden") else "page visible"
        parts.append(tag("line", title(tip), class_="visibility", x1=x, y1=y, x2=x, y2=y + LINK_H))
    for e in t.system:
        tip = f"{system_text(e)} at {local_text(e.time)}"
        parts.append(_mark(axis, e.time, y + 14, 10, "gesture", tip, data_what=e.what))
    return "".join(parts), y + LINK_H + LANE_GAP


def system_text(e):
    """A system gesture (``SystemEvent``) in words, for a mark's tooltip."""
    if e.what == "zoom":
        return f"zoom to {_scale_text(e.scale)}"
    done = "prevented" if e.prevented else "not prevented"
    return f"{e.what} on {e.on or 'n/a'} ({done})"


def _scale_text(scale):
    return "n/a" if scale is None else f"{scale:g}"


def _live_lane(t, axis, y):
    parts = [
        tag("rect", None, class_="lane", x=PLOT_X0, y=y, width=PLOT_X1 - PLOT_X0, height=LIVE_H)
    ]
    parts.append(tag("text", "Live: busy, late heartbeats", x=8, y=y + 14))
    for b in t.busy:
        instance, still = b.info
        tip = f"Live busy ({instance}) {b.ms:.1f} ms from {local_text(b.start)}"
        tip += ", still busy at the window's end" if still else ""
        parts.append(
            _band(
                axis,
                b,
                y,
                LIVE_H - 10,
                "busy",
                tip,
                data_instance=instance,
                data_start=num(b.start),
                data_end=num(b.end),
            )
        )
    for ms, instance, gap in t.late:
        tip = f"late heartbeat ({instance}): {gap:.0f} ms after the previous one"
        parts.append(_mark(axis, ms, y + LIVE_H - 10, 10, "late-heartbeat", tip))
    return "".join(parts), y + LIVE_H + LANE_GAP


def _row(axis, key, row, points, gaps, scale, y):
    """One row of a control lane: its gaps, then its values."""
    digest = key_hash(key)
    parts = [
        tag("rect", None, class_="lane", x=PLOT_X0, y=y, width=PLOT_X1 - PLOT_X0, height=ROW_H)
    ]
    parts.append(tag("text", row, x=PLOT_X0 - 60, y=y + ROW_H // 2 + 4))
    for a, b, ms in gaps:
        parts.append(
            _band(
                axis,
                Span(a, b, ms, None),
                y,
                ROW_H,
                "gap",
                f"{row} gap {ms:.1f} ms from {local_text(a)}",
                data_line=row,
                data_key_hash=digest,
                data_start=num(a),
                data_end=num(b),
                data_ms=num(ms),
            )
        )

    def y_of(value):
        low, high = scale
        share = (min(max(value, low), high) - low) / (high - low)
        return y + ROW_H - 3 - share * (ROW_H - 6)

    scaled = [(p.time, number(p.value)) for p in points if number(p.value) is not None]
    drawn_as_line = scale is not None and len(scaled) > 1
    if drawn_as_line:
        parts.append(_polyline(axis, scaled, f"value {row}", y_of))
    for p in points:
        value = number(p.value)
        if drawn_as_line and not p.hollow:
            continue
        cy = y_of(value) if scale is not None and value is not None else y + ROW_H / 2
        tip = f"{row} {json.dumps(p.value)} at {local_text(p.time)}"
        if p.hollow:
            tip += " (not sent: the socket was down)"
        parts.append(
            tag(
                "circle",
                title(tip),
                class_="unsent" if p.hollow else "dot",
                cx=num(axis.x(p.time)),
                cy=num(cy),
                r="2.5" if p.hollow else "1.8",
            )
        )
    return "".join(parts)


def _key_lane(t, axis, key, y):
    lane = t.lanes[key]
    scale = key_scale(key)
    parts = [tag("text", esc(key), class_="key", x=8, y=y + 13)]
    y += KEY_TITLE_H
    for row in ROWS:
        parts.append(_row(axis, key, row, lane[row], lane["gaps"][row], scale, y))
        if row == "send":
            for e in t.intents.get(key, []):
                state = e.data.get("state")
                tip = f"the write (seq {e.data.get('seq')}) turned {state} at {local_text(e.hub)}"
                parts.append(
                    _mark(
                        axis,
                        e.hub,
                        y,
                        ROW_H,
                        f"intent {state}",
                        tip,
                        width=4.0,
                        data_state=state,
                        data_key_hash=key_hash(key),
                    )
                )
        if row == "applied":
            for jump in (j for j in t.jumps if j.key == key):
                tip = f"jump {db_text(jump.db_from)} -> {db_text(jump.db_to)}: {jump.cause}"
                parts.append(_mark(axis, jump.time, y, ROW_H, "jump", tip, width=2.0))
        y += ROW_H
    return tag("g", "".join(parts), class_="control", data_key_hash=key_hash(key)), y + LANE_GAP


def svg(t):
    """The time axis with every lane, as inline SVG."""
    axis = Axis(t.start, t.end)
    lanes = []
    y = AXIS_H
    for draw in (_link_lane, _live_lane):
        part, y = draw(t, axis, y)
        lanes.append(part)
    for key in t.keys:
        part, y = _key_lane(t, axis, key, y)
        lanes.append(part)
    height = y + 4
    grid = []
    for ms, label in axis.ticks():
        x = num(axis.x(ms))
        grid.append(tag("line", None, class_="grid", x1=x, y1=AXIS_H - 6, x2=x, y2=height))
        grid.append(tag("text", esc(label), class_="tick", x=x, y=16, text_anchor="middle"))
    content = "".join(grid + lanes)
    return tag(
        "svg",
        content,
        xmlns="http://www.w3.org/2000/svg",
        width=WIDTH,
        height=num(height),
        viewBox=f"0 0 {WIDTH} {num(height)}",
    )


def db_text(db):
    return "-inf dB" if db == float("-inf") else f"{db:.1f} dB"


# What the jump table's gap columns measure (S1, S2: the sets behind the two
# applied values).
JUMP_LEGEND = (
    "Cause: link (the hub's arrivals of the control stopped for over 100 ms while "
    "the page kept sending, or a dropout), live (Live's round trip of the batch or "
    "the wait at the hub over 100 ms, or Live busy), page (the page stopped sending "
    "for over 100 ms after S1, or a long frame stalled it between S1's send and "
    "S2's), no data (none of these, but the page's recorder dropped the long frames "
    "of that time), move (none of these). Arrival gap and send gap: the largest "
    "gaps from the arrival (send) before S1 to S2; page gap: the largest send gap "
    "after S1. Overlaps: a dropout or Live busy over the interval of the two "
    "applied values, a long frame (frame) or dropped long frames (nodata) between "
    "S1's send and S2's (over the applied interval when S1 or S2 is not in the "
    "log). S1 and S2 are the sets behind the two applied values."
)


def _jumps_table(t):
    if not t.jumps:
        return "<p>No volume jump over 3 dB between two applied values.</p>"
    head = "".join(
        f"<th>{h}</th>"
        for h in (
            "time (local)",
            "control",
            "from → to",
            "cause",
            "arrival gap",
            "send gap",
            "page gap",
            "Live rtt",
            "wait at the hub",
            "overlaps",
        )
    )
    rows = []
    for j in t.jumps:
        cells = (
            local_text(j.time),
            j.key,
            f"{db_text(j.db_from)} → {db_text(j.db_to)}",
            j.cause,
            f"{ms_text(j.gaps['arrival'])} ms",
            f"{ms_text(j.gaps['send'])} ms",
            f"{ms_text(j.gaps['page'])} ms",
            f"{ms_text(j.rtt_ms)} ms",
            f"{ms_text(j.wait_ms)} ms",
            ", ".join(j.overlaps) or "none",
        )
        content = "".join(f"<td>{esc(c)}</td>" for c in cells)
        rows.append(
            tag("tr", content, class_="jump", data_cause=j.cause, data_key_hash=key_hash(j.key))
        )
    table = f'<table class="jumps"><tr>{head}</tr>{"".join(rows)}</table>'
    return f"<p>{esc(JUMP_LEGEND)}</p>{table}"


# What the touch table's columns measure.
TOUCH_LEGEND = (
    "Touches of a single volume fader that say where they started (PR D). Start: "
    "the fader's position at the down (local: its own, after a release or with a "
    "write open; else the page's value of Live); Live before: Live's value when "
    "the touch's first set reached the hub; first applied: the first value Live "
    "took of the touch's own sets. Jump: first applied against Live before; "
    "finger: the finger's own move up to that set; off: first applied against "
    "where the finger alone would have taken Live from Live before. A "
    "first-touch jump: jump and off both over 1 dB. Why: stale (the page's value "
    "of Live differed from the hub's), local (the fader started from its own "
    "position), other. First move (PR F): anchored when the touch's first "
    "pointer move left the fader where the touch started (the drag counts "
    "from it, and so do finger and off), applied when it moved it, n/a for an "
    "older page or a touch that sent nothing. Down to first move and to "
    "first send on the page's clock. Move gaps: two moves over 50 ms apart while "
    "the finger went on over 3 px; held: 3 or more frames in a row sending the "
    "value of the frame before while the finger moved. No data: spans whose "
    "moves the page's recorder dropped (its backlog was full); no move gap or "
    "held run is counted across them, and the hub's sets in them still arrived."
)


def _db_or_na(db):
    return "n/a" if db is None else f"{db:.1f}"


def _no_data_text(touch):
    """A touch's no-data spans (moves the recorder dropped), or none."""
    if not touch.no_data:
        return "none"
    return (
        f"{touch.no_data} ({ms_text(touch.no_data_ms)} ms; the hub got "
        f"{touch.no_data_sets} sets in it)"
    )


def _touches_table(t):
    if not t.touches:
        return "<p>No touch of a single volume fader with its start in the window.</p>"
    head = "".join(
        f"<th>{h}</th>"
        for h in (
            "time (local)",
            "control",
            "start",
            "Live before",
            "first applied",
            "jump",
            "finger",
            "off",
            "first-touch jump",
            "first move",
            "down to first move",
            "down to first send",
            "move gaps",
            "held",
            "no data",
        )
    )
    rows = []
    for touch in t.touches:

        def level(value):
            return "n/a" if value is None else db_text(value2db(value))

        start = "n/a" if touch.start is None else db_text(value2db(to_live(touch.start)))
        start += " (local)" if touch.local else ""
        worst = max((g.ms for g in touch.move_gaps), default=None)
        cells = (
            local_text(touch.time),
            touch.key,
            start,
            level(touch.live_before),
            level(touch.first_applied),
            f"{_db_or_na(touch.jump_db)} dB",
            f"{_db_or_na(touch.finger_db)} dB",
            f"{_db_or_na(touch.off_db)} dB",
            f"yes ({touch.why})" if touch.first_jump else "no",
            touch.first_move or "n/a",
            f"{ms_text(touch.first_move_ms)} ms",
            f"{ms_text(touch.first_send_ms)} ms",
            f"{len(touch.move_gaps)} (longest {ms_text(worst)} ms)",
            str(touch.held_runs),
            _no_data_text(touch),
        )
        content = "".join(f"<td>{esc(c)}</td>" for c in cells)
        rows.append(
            tag(
                "tr",
                content,
                class_="touch",
                data_key_hash=key_hash(touch.key),
                data_first_jump="true" if touch.first_jump else "false",
                data_why=touch.why,
                data_first_move=touch.first_move,
                data_jump_db=_db_or_na(touch.jump_db),
                data_finger_db=_db_or_na(touch.finger_db),
                data_off_db=_db_or_na(touch.off_db),
                data_first_move_ms=ms_text(touch.first_move_ms),
                data_first_send_ms=ms_text(touch.first_send_ms),
                data_move_gaps=len(touch.move_gaps),
                data_held=touch.held_runs,
                data_no_data=touch.no_data,
                data_no_data_ms=ms_text(touch.no_data_ms),
                data_no_data_sets=touch.no_data_sets,
            )
        )
    table = f'<table class="touches"><tr>{head}</tr>{"".join(rows)}</table>'
    return f"<p>{esc(TOUCH_LEGEND)}</p>{table}"


# What the system gestures' table lists.
SYSTEM_LEGEND = (
    "What the browser or the system did with a touch (PR G), recorded by the "
    "page: a context menu, a selection or a drag that started (the surface "
    "prevents them), a pinch's start, a cancelled pointer, a pointer capture "
    "lost while the finger was still down (the system took the touch), and "
    "each change of the page's zoom. On: the control's kind (its keys in the "
    "next column) or the element's. Prevented: whether the page stopped the "
    "browser's own action (a cancelled pointer or a lost capture cannot be)."
)


def _system_table(t):
    if not t.system:
        return "<p>No system gesture and no zoom in the window.</p>"
    head = "".join(
        f"<th>{h}</th>"
        for h in ("time (local)", "event", "on", "control", "prevented", "pointer", "scale")
    )
    rows = []
    for e in t.system:
        prevented = None if e.prevented is None else ("true" if e.prevented else "false")
        cells = (
            local_text(e.time),
            e.what,
            e.on or "n/a",
            ", ".join(e.keys) or "none",
            {"true": "yes", "false": "no"}.get(prevented, "n/a"),
            "n/a" if e.pointer is None else str(e.pointer),
            _scale_text(e.scale),
        )
        content = "".join(f"<td>{esc(c)}</td>" for c in cells)
        rows.append(
            tag(
                "tr",
                content,
                class_="sys",
                data_time=num(e.time),
                data_what=e.what,
                data_on=e.on,
                data_key_hash=key_hash(e.keys[0]) if e.keys else None,
                data_prevented=prevented,
                data_pointer=None if e.pointer is None else str(e.pointer),
                data_scale=None if e.scale is None else _scale_text(e.scale),
            )
        )
    table = f'<table class="system"><tr>{head}</tr>{"".join(rows)}</table>'
    return f"<p>{esc(SYSTEM_LEGEND)}</p>{table}"


def render(t, pairs, files):
    """The whole report: the window, ``files`` (the names read), the notes,
    the summary ``pairs``, the lanes and the jumps."""
    window = (
        f"{local_text(t.start)} to {local_text(t.end)} local time "
        f"({utc_text(t.start)} to {utc_text(t.end)})"
    )
    notes = "".join(f"<li>{esc(n)}</li>" for n in t.notes) or "<li>none</li>"
    empty = "" if t.records else "<p><strong>The window holds no records.</strong></p>"
    table = "".join(
        f'<tr><th>{esc(n)}</th><td data-k="{esc(n)}">{esc(v)}</td></tr>' for n, v in pairs
    )
    return (
        "<!doctype html>\n"
        '<html lang="en"><head><meta charset="utf-8">'
        # No icon to fetch: a report served over HTTP stays without a 404.
        '<link rel="icon" href="data:,">'
        "<title>fohmixer timeline</title>"
        f"<style>{CSS}</style></head><body>"
        "<h1>fohmixer timeline</h1>"
        f"<p>Window: {esc(window)}</p>"
        f"<p>Files read: {esc(', '.join(files) or 'none')}</p>"
        f"<p>Records in the window: {t.records}</p>"
        f'<p>Notes:</p><ul class="notes">{notes}</ul>'
        f"{empty}"
        f'<h2>Summary</h2><table class="summary">{table}</table>'
        f"<h2>Timeline</h2>{svg(t)}"
        f"<h2>Volume jumps over 3 dB</h2>{_jumps_table(t)}"
        f"<h2>Touches of single volume faders</h2>{_touches_table(t)}"
        f"<h2>System gestures</h2>{_system_table(t)}"
        "</body></html>\n"
    )
