#!/usr/bin/env python3
"""One-shot import of the engineer's TouchOSC project into the hub's layout.

    python3 import_tosc.py FILE.tosc [--set FOH.als] [--set-instance band]
                           --out layout.json [--report report.md]

Spec §2.6, the S3 design note §7 and the UI redesign note §3 (#21). It reads
the TouchOSC Mk2 project (lexml, zlib), composes parent-relative frames into
canvas coordinates, clips each node to the part TouchOSC shows (inside its
group, page, pager and the canvas; a clipped node is reported, one with
nothing shown is dropped), and classifies nodes by their role, not by their
script alone: strips (by the names of their parts), solo buttons, the stage
mics and STAGE AUT, areas and their titles, labels, the root overlay (the
TechAlert strip, REFRESH ALL, the alert box). The former MIDI controls take
their targets and ranges from the band set's MIDI mappings (``KeyMidi`` in the
gzip XML of the ``.als``): a clean mapping becomes a ``param_toggle`` /
``param_fader`` writing its targets directly (D10); anything else is dropped
(D12).

The layout it writes is schema 2: what is on each page, no geometry. The
frames only group: on a page with strips the non-strip controls form the
page's rail; the other controls are grouped by the area around them (its
title, its fill), the controls in no area by contiguous runs; the groups and
the nested pager are clustered into rows; the overlay becomes the global
controls. The grouping is listed in the report for an eye check. Everything
not reproduced is listed in the report (the layout's ``report`` and
``--report``), never fixed silently: hidden and off-canvas nodes, decoration,
free labels, stale config, unresolved names.

Python 3.11 standard library only. The tool is deleted at S6 with its
fixtures (spec §2.6). The layout it writes holds real track names, so it
stays on the Ableton PC (spec §5.2).
"""

import argparse
import gzip
import hashlib
import json
import os
import re
import statistics
import sys
import zlib
from dataclasses import dataclass, field
from xml.etree import ElementTree

SCHEMA = 2
ALERT_PERIOD_MS = 300
# The grouping (the UI redesign note §3). A title names the box it overlaps or
# touches within this gap (px).
TITLE_TOUCH = 8.0
# Controls in no area break into another run past this gap, in median control
# widths of their page.
RUN_GAP = 1.5
# A strip this many times its page's median strip width is drawn wide.
WIDE_FROM = 1.2
# The non-strip controls that go to the rail of a page with strips.
RAIL_KINDS = ("stage", "hub_toggle", "solo", "param_toggle", "refresh")
# The strip parts, by their node names (abl-touchosc's group_init.lua).
STRIP_PARTS = {
    "fader": "fader",
    "pan": "pan",
    "mute": "mute",
    "meter": "meter",
    "status_indicator": "status",
    "db": "db",
    "track_label": "label",
    "connection_label": "instance_label",
}
# A return track is named with a leading letter and a hyphen (`A-…`).
RETURN_NAME = re.compile(r"^[A-Z]-")
SCALE_LABEL = re.compile(r"^-?\d+$")
# A label's stored text that is a value readout, not a name.
VALUE_TEXT = re.compile(r"^[-+]?\s*(\d+([.,]\d+)?|inf|∞)\s*(%|db|dbfs)?$", re.IGNORECASE)
LETTER = re.compile(r"[^\W\d_]")
CONF_LINE = re.compile(r"^\s*([A-Za-z_]\w*)\s*:\s*(.*?)\s*$")
MACRO = re.compile(r"^MacroControls\.(\d+)$")
# The full MIDI-map ranges (the set's units) and the LOM range they map to.
FULL_RANGES = {
    "volume": ((0.0003162277571, 1.99526238), (0.0, 1.0)),
    "send": ((0.0003162277571, 1.0), (0.0, 1.0)),
    "panning": ((-1.0, 1.0), (-1.0, 1.0)),
    "macro": ((0.0, 127.0), (0.0, 127.0)),
}
MACRO_BUS_CHANNEL = 16
# REFRESH ALL asks the document script to refresh every strip (abl-touchosc's
# global_refresh_button.lua); other scripts only mention a refresh.
REFRESH_CALL = re.compile(r"""\bnotify\s*\(\s*["']refresh_all_groups["']""")
LUA_COMMENT = re.compile(r"--\[(=*)\[.*?\]\1\]|--[^\n]*", re.DOTALL)


class ImportError_(Exception):
    """A file the tool cannot read."""


# --- the TouchOSC project ---------------------------------------------------------------


@dataclass
class Midi:
    enabled: bool
    send: bool
    kind: str
    channel: int
    data1: int
    scale: tuple  # the VALUE entry's (scaleMin, scaleMax)


@dataclass
class Node:
    type: str
    props: dict
    values: dict
    midi: list
    children: list = field(default_factory=list)

    def prop(self, key, default=None):
        return self.props.get(key, default)

    @property
    def name(self):
        return self.prop("name", "")

    @property
    def frame(self):
        return self.prop("frame", (0.0, 0.0, 0.0, 0.0))

    @property
    def visible(self):
        return self.prop("visible", True)

    @property
    def script(self):
        return self.prop("script", "")

    @property
    def text(self):
        return self.values.get("text", "")

    def child(self, name):
        return next((c for c in self.children if c.name == name), None)


def _parse_value(kind, value):
    if kind == "b":
        return (value.text or "").strip() == "1"
    if kind == "i":
        return int(float(value.text))
    if kind == "f":
        return float(value.text)
    if kind == "r":
        return tuple(float(value.find(k).text) for k in "xywh")
    if kind == "c":
        return tuple(float(value.find(k).text) for k in "rgba")
    return value.text or ""


def _parse_midi(element):
    def flag(tag):
        return (element.findtext(tag) or "0").strip() == "1"

    scale = (0.0, 127.0)
    for value in element.findall("values/value"):
        if (value.findtext("type") or "") == "VALUE":
            scale = (float(value.findtext("scaleMin")), float(value.findtext("scaleMax")))
    return Midi(
        enabled=flag("enabled"),
        send=flag("send"),
        kind=element.findtext("message/type") or "",
        channel=int(element.findtext("message/channel") or 0),
        data1=int(element.findtext("message/data1") or 0),
        scale=scale,
    )


def _parse_node(element):
    props = {}
    for prop in element.findall("properties/property"):
        props[prop.findtext("key") or ""] = _parse_value(prop.get("type"), prop.find("value"))
    values = {}
    for value in element.findall("values/value"):
        values[value.findtext("key") or ""] = value.findtext("default") or ""
    return Node(
        type=element.get("type") or "",
        props=props,
        values=values,
        midi=[_parse_midi(m) for m in element.findall("messages/midi")],
        children=[_parse_node(c) for c in element.findall("children/node")],
    )


def read_tosc(path):
    """The root node of a TouchOSC project file."""
    with open(path, "rb") as f:
        data = f.read()
    try:
        root = ElementTree.fromstring(zlib.decompress(data))
    except (zlib.error, ElementTree.ParseError) as e:
        raise ImportError_(f"{path}: not a TouchOSC project ({e})") from e
    node = root.find("node")
    if root.tag != "lexml" or node is None:
        raise ImportError_(f"{path}: not a TouchOSC project (no lexml root node)")
    return _parse_node(node)


def script_hash(script):
    """The hash of a script with its trailing newlines normalised."""
    return hashlib.sha256((script.rstrip("\n") + "\n").encode("utf-8")).hexdigest()[:10]


def color_hex(color):
    """``#RRGGBBAA`` of a TouchOSC colour (floats 0…1)."""
    return "#" + "".join(f"{max(0, min(255, round(v * 255))):02X}" for v in color)


def rgb(color):
    """``#RRGGBB`` of a ``#RRGGBBAA`` colour (its alpha dropped); None for a
    fully transparent one or none."""
    if not color or color[7:9] == "00":
        return None
    return color[:7]


# --- the Live set -------------------------------------------------------------------------


@dataclass
class Mapping:
    """One MIDI mapping of the set: the message and the parameter it drives."""

    is_note: bool
    channel: int
    number: int
    track_kind: str  # track | return | master
    track: str
    steps: list  # LOM steps from the track to the parameter's device
    param: str  # the parameter element (Speaker, Volume, Send, MacroControls.0, On, …)
    parent: str  # its parent element
    send_index: int
    in_device: bool
    range_kind: str  # switch | range | none
    lo: float
    hi: float


def _escape(name):
    return name.replace("\\", "\\\\").replace("]", "\\]")


class LiveSet:
    """The tracks and MIDI mappings of a set (streamed: a set can be large)."""

    def __init__(self, path):
        self.tracks = []  # (kind, name)
        self.group_tracks = set()
        self.mappings = []
        self.macro_bus_links = 0
        try:
            with gzip.open(path, "rb") as f:
                self._read(f)
        except (OSError, EOFError, ElementTree.ParseError) as e:
            raise ImportError_(f"{path}: not a Live set ({e})") from e

    def names(self, kind):
        return [name for k, name in self.tracks if k == kind]

    def _read(self, f):
        stack = []  # [tag, frame] with frame: {kind, name, …}
        for event, element in ElementTree.iterparse(f, events=("start", "end")):
            tag = element.tag
            if event == "start":
                parent = stack[-1] if stack else (None, {})
                frame = {"kind": None}
                if parent[0] == "Tracks":
                    frame = {
                        "kind": "track",
                        "track": "return" if tag == "ReturnTrack" else "track",
                        "name": "",
                    }
                elif tag == "MainTrack":
                    frame = {"kind": "track", "track": "master", "name": ""}
                elif parent[0] == "Devices":
                    frame = {"kind": "device", "name": ""}
                elif parent[0] == "Branches":
                    frame = {"kind": "branch", "name": ""}
                elif tag == "Sends":
                    frame = {"kind": "sends", "count": 0}
                elif tag == "TrackSendHolder" and parent[1].get("kind") == "sends":
                    frame = {"kind": "holder", "index": parent[1]["count"]}
                    parent[1]["count"] += 1
                elif tag == "EffectiveName" and len(stack) >= 2 and stack[-1][0] == "Name":
                    owner = stack[-2][1]
                    if owner.get("kind") in ("track", "branch"):
                        owner["name"] = element.get("Value") or ""
                elif tag == "UserName" and parent[1].get("kind") == "device":
                    parent[1]["name"] = element.get("Value") or ""
                stack.append((tag, frame))
                continue
            _, frame = stack.pop()
            key_midi = element.find("KeyMidi")
            if key_midi is not None and tag != "KeyMidi":
                self._mapping(element, key_midi, stack, frame)
            if frame.get("kind") == "track":
                self.tracks.append((frame["track"], frame["name"]))
                if tag == "GroupTrack":
                    self.group_tracks.add(frame["name"])
            if frame.get("kind") in ("track", "device", "branch"):
                element.clear()

    def _mapping(self, element, key_midi, stack, own):
        def value(tag, default="0"):
            node = key_midi.find(tag)
            return node.get("Value", default) if node is not None else default

        channel = int(value("Channel"))
        if channel == MACRO_BUS_CHANNEL:
            self.macro_bus_links += 1
            return
        track = own if own.get("kind") == "track" else None
        track = track or next((f for _, f in reversed(stack) if f.get("kind") == "track"), None)
        if track is None:
            return
        steps = []
        send_index = -1
        for _tag, frame in stack:
            if frame.get("kind") == "device":
                steps.append(f"devices[name={_escape(frame['name'])}]")
            elif frame.get("kind") == "branch":
                steps.append(f"chains[name={_escape(frame['name'])}]")
            elif frame.get("kind") == "holder":
                send_index = frame["index"]
        range_kind, lo, hi = "none", 0.0, 0.0
        for tag, kind in (("MidiCCOnOffThresholds", "switch"), ("MidiControllerRange", "range")):
            node = element.find(tag)
            if node is not None:
                range_kind = kind
                lo = float(node.find("Min").get("Value"))
                hi = float(node.find("Max").get("Value"))
        self.mappings.append(
            Mapping(
                is_note=value("IsNote", "false") == "true",
                channel=channel,
                number=int(value("NoteOrController")),
                track_kind=track["track"],
                track=track["name"],
                steps=steps,
                param=element.tag,
                parent=stack[-1][0] if stack else "",
                send_index=send_index,
                in_device=bool(stack) and stack[-1][1].get("kind") == "device",
                range_kind=range_kind,
                lo=lo,
                hi=hi,
            )
        )

    def find(self, is_note, channel, number):
        return [
            m
            for m in self.mappings
            if m.is_note == is_note and m.channel == channel and m.number == number
        ]


# --- geometry: the grouping (the UI redesign note §3) --------------------------------------
#
# Frames are ``(x, y, w, h)`` in canvas coordinates; they group, they are never
# written. Items are the importer's placed nodes: dicts with a ``frame`` and a
# ``z`` (node order).


def _right(f):
    return f[0] + f[2]


def _bottom(f):
    return f[1] + f[3]


def _centre(f):
    return (f[0] + f[2] / 2, f[1] + f[3] / 2)


def _holds(f, point):
    """Whether frame ``f`` holds ``point`` (its edges included)."""
    x, y = point
    return f[0] <= x <= _right(f) and f[1] <= y <= _bottom(f)


def _encloses(outer, inner):
    return (
        outer[0] <= inner[0]
        and outer[1] <= inner[1]
        and _right(inner) <= _right(outer)
        and _bottom(inner) <= _bottom(outer)
    )


def _union(frames):
    x0 = min(f[0] for f in frames)
    y0 = min(f[1] for f in frames)
    return (x0, y0, max(_right(f) for f in frames) - x0, max(_bottom(f) for f in frames) - y0)


def _same_band(a, b):
    """Whether two vertical ranges ``(top, bottom)`` overlap by more than half
    of the shorter one (a sliver of overlap is two rows, not one)."""
    overlap = min(a[1], b[1]) - max(a[0], b[0])
    return overlap > 0 and overlap > 0.5 * min(a[1] - a[0], b[1] - b[0])


def _span(f):
    return (f[1], _bottom(f))


def innermost(frame, areas):
    """The innermost of ``areas`` holding the centre of ``frame``: the
    smallest; of equal ones, the one drawn last. None when none holds it."""
    centre = _centre(frame)
    holding = [a for a in areas if _holds(a["frame"], centre)]
    return min(holding, key=lambda a: (a["frame"][2] * a["frame"][3], -a["z"]), default=None)


def title_key(title, box):
    """How well ``title`` names ``box`` (lower is better), None when it does
    not touch it. A title sits on its box's edge: overlapping it, or beside,
    above or below it within ``TITLE_TOUCH`` px. A box that only encloses the
    title (a page backdrop behind the whole section) is the weaker claim."""
    t, b = title["frame"], box["frame"]
    across = min(_right(t), _right(b)) - max(t[0], b[0])
    down = min(_bottom(t), _bottom(b)) - max(t[1], b[1])
    if not ((across > 0 and down >= -TITLE_TOUCH) or (down > 0 and across >= -TITLE_TOUCH)):
        return None
    if _encloses(b, t):
        return (1, b[2] * b[3], 0.0, -box["z"])
    gap = max(0.0, -across) + max(0.0, -down)
    return (0, -max(0.0, across) * max(0.0, down), gap, -box["z"])


def titles_of(boxes, titles):
    """``{id(box): title}``: each title names the box it sits on best, each
    box keeps the best of the titles naming it."""
    claims = {}
    for title in titles:
        scored = [(k, box) for box in boxes if (k := title_key(title, box)) is not None]
        if scored:
            key, box = min(scored, key=lambda s: s[0])
            claims.setdefault(id(box), []).append((key, title))
    return {b: min(found, key=lambda s: s[0])[1] for b, found in claims.items()}


def runs(controls):
    """The controls in no area as horizontally contiguous runs (the untitled
    groups): left to right, a control joins a run whose vertical range it
    shares (``_same_band``) when the gap to the run's right edge is at most
    ``RUN_GAP`` median control widths; else it starts a run."""
    if not controls:
        return []
    limit = RUN_GAP * statistics.median(c["frame"][2] for c in controls)
    found = []
    for control in sorted(controls, key=lambda c: (c["frame"][0], c["frame"][1])):
        f = control["frame"]
        fits = [
            run
            for run in found
            if _same_band(run["span"], _span(f)) and f[0] - run["right"] <= limit
        ]
        if not fits:
            found.append({"span": _span(f), "right": _right(f), "controls": [control]})
            continue
        run = min(fits, key=lambda r: f[0] - r["right"])
        run["controls"].append(control)
        run["right"] = max(run["right"], _right(f))
        run["span"] = (min(run["span"][0], f[1]), max(run["span"][1], _bottom(f)))
    return [run["controls"] for run in found]


def rows(sections):
    """Sections clustered into rows by vertical overlap (``_same_band``), top
    to bottom; each row ordered by x."""
    found = []
    for section in sorted(sections, key=lambda s: (s["frame"][1], s["frame"][0])):
        span = _span(section["frame"])
        if found and _same_band(found[-1]["span"], span):
            row = found[-1]
            row["sections"].append(section)
            row["span"] = (min(row["span"][0], span[0]), max(row["span"][1], span[1]))
        else:
            found.append({"span": span, "sections": [section]})
    return [sorted(r["sections"], key=lambda s: (s["frame"][0], s["frame"][1])) for r in found]


def wide_flags(widths):
    """For each strip width, whether it is at least ``WIDE_FROM`` times the
    median of ``widths`` (its page's strips)."""
    if not widths:
        return []
    median = statistics.median(widths)
    return [w >= WIDE_FROM * median - 1e-9 for w in widths]


def display(control):
    """A control's name in the report (schema 2 form)."""
    kind = control["kind"]
    if kind == "text":
        return control["text"]
    if control.get("label"):
        return control["label"]
    anchor = control.get("binding", {}).get("anchor", {})
    return anchor.get("name", anchor.get("kind", kind))


# --- the import ---------------------------------------------------------------------------


class Drop(Exception):
    """A former MIDI control that is not reproduced (the reason)."""


def _close(a, b):
    return abs(a - b) <= 1e-6 * max(1.0, abs(a), abs(b))


def _config(text):
    """``connection_*``, ``unfold[_*]`` and ``double_click_mute`` of the Conf text."""
    instances, unfold, guards = [], [], []
    for line in text.splitlines():
        match = CONF_LINE.match(line)
        if not match:
            continue
        key, value = match.group(1), match.group(2).strip().strip("'\"")
        if key.startswith("connection_"):
            instances.append(key[len("connection_") :])
        elif key == "unfold" or key.startswith("unfold_"):
            unfold.append((key[len("unfold_") :] if key != "unfold" else None, value))
        elif key == "double_click_mute":
            guards.append(value)
    return instances, unfold, guards


def _find_config(node):
    """The Conf page's TEXT node (the one that names the connections)."""
    if node.type == "TEXT" and "connection_" in node.text:
        return node
    for child in node.children:
        found = _find_config(child)
        if found is not None:
            return found
    return None


def _is_strip(node):
    return node.type == "GROUP" and any(
        c.name in ("mute", "fader", "meter") and c.type in ("BUTTON", "FADER")
        for c in node.children
    )


def _full_strip(item):
    """A strip with a fader (not the TechAlert's meter and mute)."""
    return item["kind"] == "strip" and item["fader"]


def _midi_of(node):
    return next((m for m in node.midi if m.enabled and m.send), None)


def _intersect(a, b):
    """The overlap of two ``(x, y, w, h)`` rectangles; None when they do not overlap."""
    if a is None or b is None:
        return None
    x0, y0 = max(a[0], b[0]), max(a[1], b[1])
    x1, y1 = min(a[0] + a[2], b[0] + b[2]), min(a[1] + a[3], b[1] + b[3])
    if x1 <= x0 or y1 <= y0:
        return None
    return (x0, y0, x1 - x0, y1 - y0)


def _is_battery(node):
    """The battery gauge (X5): the node reading the battery, or the group
    around it (its box and "NN%" label go with it)."""

    def reads(n):
        return "getBatteryLevel" in n.script

    return reads(node) or (node.type == "GROUP" and any(reads(c) for c in node.children))


def _is_refresh(node):
    """The REFRESH ALL control: a label or button whose code asks for the refresh."""
    code = LUA_COMMENT.sub("", node.script)
    return node.type in ("LABEL", "BUTTON") and REFRESH_CALL.search(code) is not None


def _name_labels(nodes):
    """The labels among ``nodes`` that name a former MIDI control: visible
    words, whatever script they carry (the real ones restyle on incoming OSC);
    not the "ON/OFF" state nor a value readout ("- 0.0", "-inf", "100%")."""
    return [
        n
        for n in nodes
        if n.type == "LABEL"
        and n.visible
        and LETTER.search(n.text)
        and n.text.strip().upper() != "ON/OFF"
        and not VALUE_TEXT.match(n.text.strip())
    ]


def _label_text(labels):
    """The name the labels spell together, in node order ("Podklady" "All")."""
    return " ".join(" ".join(n.text.split()) for n in labels)


def _covers(a, b):
    """Whether two frames of one parent overlap by at least half the smaller one."""
    w = min(a[0] + a[2], b[0] + b[2]) - max(a[0], b[0])
    h = min(a[1] + a[3], b[1] + b[3]) - max(a[1], b[1])
    return w > 0 and h > 0 and w * h >= min(a[2] * a[3], b[2] * b[3]) / 2


def _with_labels(children):
    """Each child with the sibling labels drawn over it when it is a former MIDI
    control outside a group; those labels name the control and are skipped."""
    labels, taken = {}, set()
    names = _name_labels(children)
    for control in children:
        if control.type in ("BUTTON", "FADER") and _midi_of(control):
            own = [n for n in names if id(n) not in taken and _covers(n.frame, control.frame)]
            labels[id(control)] = own
            taken.update(id(n) for n in own)
    return [(c, labels.get(id(c), [])) for c in children if id(c) not in taken]


def _message(midi):
    kind = "NOTE" if midi.kind.startswith("NOTE") else "CC"
    return f"{kind}{midi.data1} ch{midi.channel + 1}"


def _press(node):
    """The press mode of a former MIDI button (its type and its script)."""
    button_type = node.prop("buttonType", 1)
    scripted = bool(node.script.strip())
    if button_type in (1, 2) and (node.prop("press", True) or node.prop("release", True)):
        return "toggle"
    if button_type in (1, 2) and scripted:
        return "double_tap_latch"
    if button_type == 0 and scripted:
        return "pulse_and_double_tap_latch"
    raise Drop("button behaviour not reproduced (momentary without a latch)")


def _words(text):
    return " ".join(text.split())


class Importer:
    def __init__(self, root, live_set, set_instance, source):
        self.root = root
        self.set = live_set
        self.set_instance = set_instance
        self.canvas_rect = (0.0, 0.0, float(root.frame[2]), float(root.frame[3]))
        self.config_node = _find_config(root)
        instances, unfold, guards = _config(self.config_node.text if self.config_node else "")
        self.instances = instances or ["band"]
        self.unfold = unfold
        self.guards = guards
        self.guards_used = set()
        self.z = 0
        self.report = {
            "source": source,
            "instances": self.instances,
            "dropped": [],
            "clipped": [],
            "midi": [],
            "stale_config": [],
            "unresolved": [],
            "groups": {},
            "guessed": [],
            "decoration": {},
            "macro_bus_links": live_set.macro_bus_links if live_set else 0,
            "notes": [
                "a track name with a leading letter and a hyphen (A-…) is bound as a return track",
                "double_click_mute entries are matched as plain text, not Lua patterns",
                "groups: an area holding a control's centre, titled by the title on its edge; "
                "the controls in no area are guessed (listed under guessed)",
            ],
            "scripts": {},
        }
        self.bindings = []  # (instance, anchor) of strips, solos, stages
        self.alerts = []  # (item, its layer, path): bound after the whole overlay
        self.ids = set()  # page, pager and group ids: unique across the layout
        self._scripts(root, None)
        # A strip decoration carrying a part's script is named for it.
        for roles in self.report["scripts"].values():
            if "strip decoration" in roles:
                for part, name in (
                    ("meter", "second meter bar"),
                    ("mute", "backdrop (mute script)"),
                ):
                    if part in roles:
                        roles[roles.index("strip decoration")] = name
                        break

    # --- bookkeeping ---

    def _scripts(self, node, parent):
        """The script table: every script's hash and the roles that carry it."""
        role = None
        if parent is not None and _is_strip(parent):
            role = STRIP_PARTS.get(node.name, "strip decoration")
        elif _is_strip(node):
            role = "strip group"
        if node.script.strip():
            roles = self.report["scripts"].setdefault(script_hash(node.script), [])
            label = role or node.type.lower()
            if label not in roles:
                roles.append(label)
        for child in node.children:
            self._scripts(child, node)

    def _role_hash(self, part):
        return {h for h, roles in self.report["scripts"].items() if part in roles}

    def drop(self, where, why):
        self.report["dropped"].append({"node": where, "why": why})

    def count(self, what, n=1):
        decoration = self.report["decoration"]
        decoration[what] = decoration.get(what, 0) + n

    def unique(self, base):
        """``base``, or ``base-2``, ``base-3``, … when taken: an id unique
        across the layout's pages, pagers and groups."""
        candidate, n = base, 1
        while candidate in self.ids:
            n += 1
            candidate = f"{base}-{n}"
        self.ids.add(candidate)
        return candidate

    def clip(self, rect, clip):
        """(the part of ``rect`` TouchOSC shows, why none shows).

        TouchOSC clips a node to its parent group, page and pager (``clip``,
        their visible part) and to the canvas."""
        if _intersect(rect, self.canvas_rect) is None:
            return None, "off the canvas"
        part = _intersect(rect, clip)
        return part, None if part else "outside its parent"

    def visible(self, rect, clip, path):
        """The shown part of a node; a node with none is dropped, a clipped one
        is reported."""
        part, why = self.clip(rect, clip)
        if part is None:
            self.drop(path, why)
        elif part != rect:
            self.report["clipped"].append(path)
        return part

    def split(self, name):
        """(instance, track name) of a group name: a known instance prefix, else band."""
        for instance in self.instances:
            if name.startswith(instance + "_"):
                return instance, name[len(instance) + 1 :]
        return "band" if "band" in self.instances else self.instances[0], name

    def binding(self, name):
        instance, track = self.split(name)
        kind = "return" if RETURN_NAME.match(track) else "track"
        binding = {"instance": instance, "anchor": {"kind": kind, "name": track}}
        self.bindings.append(binding)
        return binding

    def item(self, kind, frame, path, **fields):
        """A placed node: its kind, its shown frame (canvas coordinates, used
        to group, never written), its node order and its fields."""
        self.z += 1
        return {"kind": kind, "frame": frame, "z": self.z, "path": path, **fields}

    @staticmethod
    def fill(node):
        """A node's own fill (``background`` with its ``color``), or None."""
        if node.prop("background", False) and "color" in node.props:
            return color_hex(node.prop("color"))
        return None

    # --- pages ---

    def run(self):
        pager = next((c for c in self.root.children if c.type == "PAGER"), None)
        if pager is None:
            raise ImportError_("the project has no root pager")
        # The root group is the canvas: it clips the pager and the overlay.
        shown = _intersect(pager.frame, self.canvas_rect)
        pages, default = self.pager(pager, 0.0, 0.0, shown, "root", nested=False)
        if not pages:
            raise ImportError_("the root pager has no pages")
        overlay = []
        for child, labels in _with_labels(self.root.children):
            if child is not pager:
                self.collect(
                    child, 0.0, 0.0, self.canvas_rect, overlay, "root", labels, root_level=True
                )
        self.resolve_alerts(overlay)
        self.check_config()
        self.check_bindings()
        out_pages = [self.emit_page(page) for page in pages]
        return {
            "schema": SCHEMA,
            "default_page": pages[default]["id"],
            "pages": out_pages,
            "global": self.emit_global(overlay),
            "config": {
                "unfold": [
                    {"instance": instance, "name": name}
                    for scope, name in self.unfold
                    for instance in ([scope] if scope else self.instances)
                ]
            },
            "report": self.report,
        }

    def pager(self, node, ox, oy, shown, where, nested):
        """A pager's pages and its default page's index; ``shown`` is its
        visible part, which clips its pages."""
        x, y, _, _ = node.frame
        ax, ay = ox + x, oy + y
        if _midi_of(node):
            self.drop(f"{where}/{node.name}", "the pager's page-change message (X6)")
        pages = []
        for page in node.children:
            if page.type != "GROUP":
                self.drop(f"{where}/{node.name}/{page.name}", "not a page")
                continue
            pages.append(self.page(page, ax, ay, shown, f"{where}/{node.name}", nested))
        default = int(float(node.values.get("page", "0") or 0))
        if not 0 <= default < max(1, len(pages)):
            self.drop(f"{where}/{node.name}", f"default page {default} out of range: 0")
            default = 0
        return pages, default

    def page(self, node, ox, oy, clip, where, nested):
        """A page: its id, its title, its placed items and its nested pager."""
        x, y, w, h = node.frame
        ax, ay = ox + x, oy + y
        shown = _intersect((ax, ay, w, h), clip)
        title = node.prop("tabLabel", "") or node.name
        base = re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-") or "page"
        page = {"id": self.unique(base), "title": title, "items": []}
        path = f"{where}/{node.name}"
        for child, labels in _with_labels(node.children):
            if child.type == "PAGER":
                where_pager = f"{path}/{child.name}"
                if nested:
                    self.drop(where_pager, "a pager in a nested pager's page (one level nests)")
                    continue
                if "pager" in page:
                    self.drop(where_pager, "a second pager on one page")
                    continue
                cx, cy, cw, chh = child.frame
                if not child.visible:
                    self.drop(where_pager, "hidden")
                    continue
                sub = self.visible((ax + cx, ay + cy, cw, chh), shown, where_pager)
                if sub is None:
                    continue
                pages, default = self.pager(child, ax, ay, sub, path, nested=True)
                if not pages:
                    self.drop(where_pager, "a pager without pages")
                    continue
                page["pager"] = {"frame": sub, "pages": pages, "default": default}
            else:
                self.collect(child, ax, ay, shown, page["items"], path, labels)
        return page

    # --- nodes ---

    def collect(self, node, ox, oy, clip, out, where, labels=(), root_level=False):
        """A node and its children. ``ox``/``oy`` is its parent's canvas
        origin, ``clip`` the parent's visible part, ``labels`` the sibling
        labels that name it (a former MIDI control)."""
        x, y, w, h = node.frame
        ax, ay = ox + x, oy + y
        rect = (ax, ay, w, h)
        path = f"{where}/{node.name}"
        if _is_battery(node):
            self.drop(path, "the battery gauge (X5)")
            return
        if not node.visible:
            if root_level and node.type == "BOX" and node.script.strip():
                self.alert(node, rect, clip, out, path)
            else:
                self.drop(path, "hidden")
            return
        if node.type in ("BUTTON", "FADER") and _midi_of(node):
            label = _label_text(labels) or node.name
            self.midi_control(node, ox, oy, clip, label, out, where)
            return
        shown = self.visible(rect, clip, path)
        if shown is None:
            return
        if _is_strip(node):
            out.append(self.strip(node, ax, ay, shown, path))
        elif node.type == "GROUP" and node.child("btn_solo"):
            label = self.group_text(node)
            out.append(self.item("solo", shown, path, binding=self.binding(node.name), label=label))
        elif node.type == "GROUP" and node.child("btn_mute"):
            out.append(
                self.item(
                    "stage",
                    shown,
                    path,
                    binding=self.binding(node.name),
                    aut=self.has(self.root, "btn_stage_aut"),
                    label=self.group_text(node),
                )
            )
        elif node.name == "btn_stage_aut" or (node.type == "GROUP" and node.child("btn_stage_aut")):
            label = self.group_text(node) or "STAGE AUT"
            out.append(self.item("hub_toggle", shown, path, key="stage_aut", label=label))
        elif node.type == "GROUP" and any(_midi_of(c) for c in node.children):
            self.midi_group(node, ax, ay, shown, out, path)
        elif _is_refresh(node):
            out.append(self.item("refresh", shown, path, label=_words(node.text) or node.name))
        elif node.type == "GROUP":
            if node.prop("background", False):
                out.append(self.area("box", shown, path, self.fill(node)))
            for child, labels in _with_labels(node.children):
                self.collect(child, ax, ay, shown, out, path, labels)
        elif node.type == "BOX":
            fill = color_hex(node.prop("color", (0, 0, 0, 1)))
            out.append(self.area("box", shown, path, fill))
        elif node.type == "LABEL" and node.prop("background", False):
            # A label with a fill: a section title when it has words, a box
            # when it has none.
            text = _words(node.text)
            out.append(self.area("title" if text else "box", shown, path, self.fill(node), text))
        elif node.type in ("LABEL", "TEXT"):
            config = node is self.config_node
            out.append(self.item("label", shown, path, text=node.text, config=config))
        elif node.type == "BUTTON" and node.prop("background", False):
            # A button with no function here (a backdrop carrying the inert
            # mute script): never a section's box.
            out.append(self.area("backdrop", shown, path, self.fill(node)))
        else:
            self.drop(path, f"a {node.type} without a function here")

    def area(self, role, frame, path, fill, text=""):
        """A filled area: a section's ``box``, a section ``title``, or an inert
        ``backdrop``."""
        return self.item("area", frame, path, role=role, fill=fill, text=text)

    def has(self, node, name):
        return node.name == name or any(self.has(c, name) for c in node.children)

    def group_text(self, node):
        return next((_words(c.text) for c in node.children if c.type == "LABEL" and c.text), "")

    def alert(self, node, rect, clip, out, path):
        """The hidden full-screen box that blinks while TechAlert is unmuted.

        Its TechAlert strip may come later among the root's children, so
        ``resolve_alerts`` binds it once the whole overlay is collected."""
        shown = self.visible(rect, clip, path)
        if shown is None:
            return
        item = self.item("alert", shown, path, strip=None, period_ms=ALERT_PERIOD_MS)
        out.append(item)
        self.alerts.append((item, out, path))

    def resolve_alerts(self, overlay):
        """Binds each alert box to the TechAlert strip (the strip without a
        fader) of its layer, or drops it."""
        for item, out, path in self.alerts:
            strips = [i for i in out if i["kind"] == "strip" and not i["fader"]]
            if strips:
                item["strip"] = strips[0]
            else:
                out.remove(item)
                self.drop(path, "an alert box without a TechAlert strip")

    def strip(self, node, ax, ay, shown, path):
        """A strip; ``shown`` is its visible part, which clips its parts."""
        mute_hashes = self._role_hash("mute")
        meter_hashes = self._role_hash("meter")
        parts = set()
        for child in node.children:
            cx, cy, cw, ch = child.frame
            part = STRIP_PARTS.get(child.name)
            if part and part not in parts:
                seen = self.visible((ax + cx, ay + cy, cw, ch), shown, f"{path}/{child.name}")
                if seen is not None:
                    parts.add(part)
            elif child.name == "db_meter_label":
                self.count("meter dBFS labels (D11)")
            elif child.script.strip() and script_hash(child.script) in mute_hashes:
                self.count("mute script carriers")
            elif child.script.strip() and script_hash(child.script) in meter_hashes:
                self.count("second meter bars")
            elif child.type == "TEXT":
                self.count("tick lines")
            elif child.type == "LABEL" and SCALE_LABEL.match(child.text.strip()):
                self.count("scale labels")
            else:
                self.count("other strip decoration")
        guarded = [g for g in self.guards if g in node.name]
        self.guards_used.update(guarded)
        label = node.child("track_label")
        return self.item(
            "strip",
            shown,
            path,
            binding=self.binding(node.name),
            fader="fader" in parts,
            # The TouchOSC width, not the shown one: ``wide`` compares designs.
            width=float(node.frame[2]),
            mute_guard=bool(guarded),
            label=_words(label.text) if label is not None else "",
        )

    # --- former MIDI controls ---

    def midi_group(self, node, ox, oy, shown, out, path):
        control = next(c for c in node.children if _midi_of(c))
        names = _name_labels(node.children)
        label = _label_text(names) or control.name
        used = {id(control)} | {id(n) for n in names}
        for child in node.children:
            if id(child) in used:
                continue
            if child.type == "LABEL":
                self.count("labels of former MIDI controls")
            else:
                self.drop(f"{path}/{child.name}", "part of a former MIDI control")
        self.midi_control(control, ox, oy, shown, label, out, path)

    def midi_control(self, node, ox, oy, clip, label, out, where):
        """A former MIDI control; ``ox``/``oy`` is its parent's canvas origin,
        ``clip`` the parent's visible part."""
        midi = _midi_of(node)
        x, y, w, h = node.frame
        rect = (ox + x, oy + y, w, h)
        path = f"{where}/{node.name}"
        entry = {"control": label, "message": _message(midi), "verdict": "dropped", "targets": 0}
        self.report["midi"].append(entry)
        shown, why = self.clip(rect, clip)
        try:
            if shown is None:
                raise Drop(why)
            item = self.midi_item(node, midi, label, shown, path)
        except Drop as e:
            entry["why"] = str(e)
            self.drop(path, f"former MIDI control {entry['message']}: {e}")
            return
        if shown != rect:
            self.report["clipped"].append(path)
        entry["verdict"] = "clean"
        entry["targets"] = len(item["targets"])
        out.append(item)

    def midi_item(self, node, midi, label, frame, path):
        if self.set is None:
            raise Drop("no set given (--set)")
        mappings = self.set.find(midi.kind.startswith("NOTE"), midi.channel, midi.data1)
        if not mappings:
            raise Drop("no mapping in the set")
        if node.type == "FADER":
            if not (_close(midi.scale[0], 0) and _close(midi.scale[1], 127)):
                raise Drop("the fader's MIDI scale is not 0-127")
            targets = [self.fader_target(m) for m in mappings]
            return self.item("param_fader", frame, path, label=label, targets=targets)
        press = _press(node)
        off_cc, on_cc = midi.scale
        targets = [self.toggle_target(m, on_cc, off_cc) for m in mappings]
        # The button's own colour is the toggle's lit colour (an identity hint).
        color = rgb(color_hex(node.prop("color"))) if "color" in node.props else None
        return self.item(
            "param_toggle", frame, path, label=label, targets=targets, press=press, color=color
        )

    def target(self, m):
        """(binding, prop, kind, LOM range key) of a mapping's parameter."""
        anchor = {"kind": m.track_kind, "name": m.track}
        if m.track_kind == "master":
            anchor = {"kind": "master"}
        binding = {"instance": self.set_instance, "anchor": anchor}
        macro = MACRO.match(m.param)
        if m.param == "Speaker" and m.parent == "Mixer":
            return binding, "mute", "activator", None
        if m.param == "Volume" and m.parent == "Mixer":
            path, key = "mixer_device volume", "volume"
        elif m.param == "Pan" and m.parent == "Mixer":
            path, key = "mixer_device panning", "panning"
        elif m.param == "Send" and m.send_index >= 0:
            path, key = f"mixer_device sends {m.send_index}", "send"
        elif macro and m.in_device:
            path = " ".join([*m.steps, f"parameters {int(macro.group(1)) + 1}"])
            key = "macro"
        elif m.param == "On" and m.in_device:
            path = " ".join([*m.steps, "parameters 0"])
            binding["path"] = path
            return binding, "value", "device_on", None
        else:
            raise Drop(f"a mapping on {m.param} is not reproduced")
        binding["path"] = path
        return binding, "value", "continuous", key

    def check_range(self, m, kind, key):
        if kind in ("activator", "device_on"):
            if m.range_kind != "switch":
                raise Drop(f"{m.param} without on/off thresholds")
            return
        if m.range_kind != "range":
            raise Drop(f"{m.param} without a MIDI range")
        (full_lo, full_hi), _ = FULL_RANGES[key]
        if not (_close(m.lo, full_lo) and _close(m.hi, full_hi)):
            raise Drop(f"a partial range on {m.param} ({m.lo:g}..{m.hi:g})")

    def toggle_target(self, m, on_cc, off_cc):
        binding, prop, kind, key = self.target(m)
        self.check_range(m, kind, key)
        if kind in ("activator", "device_on"):
            on_active = m.lo <= on_cc <= m.hi
            off_active = m.lo <= off_cc <= m.hi
            if on_active == off_active:
                raise Drop(f"the toggle does not switch {m.param}")
            if kind == "activator":
                on, off = not on_active, not off_active
            else:
                on, off = (1.0 if on_active else 0.0), (1.0 if off_active else 0.0)
        else:
            _, (lom_lo, lom_hi) = FULL_RANGES[key]
            on = round(lom_lo + (lom_hi - lom_lo) * on_cc / 127.0, 6)
            off = round(lom_lo + (lom_hi - lom_lo) * off_cc / 127.0, 6)
        return {"binding": binding, "prop": prop, "on": on, "off": off}

    def fader_target(self, m):
        binding, prop, kind, key = self.target(m)
        if kind != "continuous":
            raise Drop(f"a fader on the switch {m.param}")
        self.check_range(m, kind, key)
        return {"binding": binding, "prop": prop, "scale": "cc_linear"}

    # --- schema 2: grouping and output ---

    @staticmethod
    def control(item):
        """The schema 2 control of a placed item (defaults omitted)."""
        kind = item["kind"]
        out = {"kind": kind}
        if kind == "strip":
            anchor_kind = item["binding"]["anchor"]["kind"]
            out["binding"] = item["binding"]
            out["strip_kind"] = "return" if anchor_kind == "return" else "standard"
            if item.get("wide"):
                out["wide"] = True
            if item["mute_guard"]:
                out["mute_guard"] = True
        elif kind in ("solo", "stage"):
            out["binding"] = item["binding"]
            if kind == "stage" and item["aut"]:
                out["aut"] = True
            if item["label"]:
                out["label"] = item["label"]
        elif kind == "hub_toggle":
            out |= {"key": item["key"], "label": item["label"]}
        elif kind == "param_toggle":
            out |= {"label": item["label"], "targets": item["targets"], "press": item["press"]}
            if item["color"]:
                out["color"] = item["color"]
        elif kind == "param_fader":
            out |= {"label": item["label"], "targets": item["targets"]}
        elif kind == "refresh":
            if item["label"]:
                out["label"] = item["label"]
        elif kind == "text":
            out["text"] = item["text"]
        else:
            raise ValueError(f"no schema 2 control for {kind}")
        return out

    def sections(self, page, rail):
        """The sections (groups, with the frames that place them) of one page's
        items. ``rail`` (a page with strips): its non-strip controls go there;
        None: every control is grouped."""
        items = page["items"]
        strips = [i for i in items if _full_strip(i)]
        for strip, wide in zip(strips, wide_flags([s["width"] for s in strips]), strict=True):
            strip["wide"] = wide
        controls, areas, texts = [], [], []
        for item in items:
            kind = item["kind"]
            if kind == "strip" and not item["fader"]:
                self.drop(item["path"], "a strip without a fader outside the overlay")
            elif kind in RAIL_KINDS and rail is not None:
                rail.append(item)
            elif kind == "area":
                areas.append(item)
            elif kind == "label" and item["config"]:
                texts.append(item)
            elif kind == "label":
                self.drop(item["path"], "a free label (schema 2 keeps no free text)")
            else:
                controls.append(item)
        # A control belongs to the innermost box or titled area holding its
        # centre; a backdrop behind one control is no section.
        holders = [a for a in areas if a["role"] in ("box", "title")]
        held, loose = {}, []
        for control in controls:
            box = innermost(control["frame"], holders)
            if box is None:
                loose.append(control)
            else:
                held.setdefault(id(box), (box, []))[1].append(control)
        titles = [a for a in areas if a["role"] == "title" and id(a) not in held]
        boxes = [a for a in areas if a["role"] == "box" or id(a) in held]
        named = titles_of(boxes, titles)
        out, titled = [], set()
        for box, members in held.values():
            if box["role"] == "title":
                title, frames = box["text"], [box["frame"]]
            elif id(box) in named:
                title = named[id(box)]
                titled.add(id(title))
                title, frames = title["text"], [box["frame"], title["frame"]]
            else:
                title, frames = None, [box["frame"]]
            out.append(
                {
                    "frame": _union(frames),
                    "title": title,
                    "color": rgb(box["fill"]),
                    "controls": members,
                    "guessed": False,
                }
            )
        for title in titles:
            if id(title) not in titled:
                self.drop(title["path"], "a section title without controls in its box")
        unused = sum(1 for a in areas if a["role"] != "title" and id(a) not in held)
        if unused:
            self.count("boxes and backdrops without controls", unused)
        for run in runs(loose):
            frame = _union([c["frame"] for c in run])
            out.append(
                {"frame": frame, "title": None, "color": None, "controls": run, "guessed": True}
            )
        for text in texts:
            lines = [line.rstrip() for line in text["text"].splitlines() if line.strip()]
            controls = [{"kind": "text", "text": line, "frame": text["frame"]} for line in lines]
            out.append(
                {
                    "frame": text["frame"],
                    "title": None,
                    "color": None,
                    "controls": controls,
                    "guessed": False,
                }
            )
        return out

    def emit_group(self, page_id, n, section):
        """A section as a schema 2 group, ``<page>-<n>``; its controls by x, then y."""
        group = {"kind": "group", "id": self.unique(f"{page_id}-{n}")}
        if section["title"]:
            group["title"] = section["title"]
        if section["color"]:
            group["color"] = section["color"]
        members = sorted(section["controls"], key=lambda c: (c["frame"][0], c["frame"][1]))
        group["controls"] = [self.control(c) for c in members]
        if section["guessed"]:
            self.report["guessed"] += [
                f"{page_id}/{group['id']}: {display(c)}" for c in group["controls"]
            ]
        return group

    @staticmethod
    def group_line(group):
        """A group in the report: ``<id> <title>: <control names>``."""
        head = group["id"] + (f" {group['title']}" if "title" in group else "")
        return f"{head}: {', '.join(display(c) for c in group['controls'])}"

    def emit_page(self, page):
        """A root page: its rail and its rows (the nested pager among them)."""
        pager = page.get("pager")
        subs = pager["pages"] if pager else []
        rail = []
        with_strips = any(_full_strip(i) for p in (page, *subs) for i in p["items"])
        sections = self.sections(page, rail if with_strips else None)
        if pager:
            sections.append({"frame": pager["frame"], "pager": pager})
        report_rows = []
        self.report["groups"][page["id"]] = report_rows
        out_rows = []
        n = 0
        for row in rows(sections):
            out_sections, report_row = [], []
            for section in row:
                # The pager takes its place in the numbering: the groups keep
                # their row-major positions.
                n += 1
                if "pager" in section:
                    emitted = self.emit_pager(page["id"], section["pager"], rail)
                    sub_titles = ", ".join(p["title"] for p in emitted["pages"])
                    report_row.append(f"{emitted['id']} (pager): {sub_titles}")
                else:
                    emitted = self.emit_group(page["id"], n, section)
                    report_row.append(self.group_line(emitted))
                out_sections.append(emitted)
            out_rows.append({"sections": out_sections})
            report_rows.append(report_row)
        out = {"id": page["id"], "title": page["title"]}
        if rail:
            ordered = sorted(rail, key=lambda i: (i["frame"][1], i["frame"][0]))
            out["rail"] = [self.control(i) for i in ordered]
        if out_rows:
            out["rows"] = out_rows
        return out

    def emit_pager(self, page_id, pager, rail):
        """A nested pager as a section; a sub-page with strips sends its
        non-strip controls to the page's ``rail``."""
        pages = pager["pages"]
        out = {
            "kind": "pager",
            "id": self.unique(f"{page_id}-pager"),
            "default_page": pages[pager["default"]]["id"],
            "pages": [],
        }
        for sub in pages:
            with_strips = any(_full_strip(i) for i in sub["items"])
            sections = self.sections(sub, rail if with_strips else None)
            # A sub-page holds groups only, in row-major order.
            ordered = [section for row in rows(sections) for section in row]
            groups = [self.emit_group(sub["id"], n, s) for n, s in enumerate(ordered, 1)]
            self.report["groups"][sub["id"]] = (
                [[self.group_line(g) for g in groups]] if groups else []
            )
            out["pages"].append({"id": sub["id"], "title": sub["title"], "sections": groups})
        return out

    def emit_global(self, overlay):
        """The controls on every page (the rail's footer): the TechAlert strip
        and its alert box as one ``alert``, REFRESH ALL, any other overlay
        control; top to bottom. The overlay's labels are dropped."""
        placed = []  # (frame, control)
        bound = set()
        for item in overlay:
            if item["kind"] != "alert":
                continue
            strip = item["strip"]
            bound.add(id(strip))
            alert = {"kind": "alert", "binding": strip["binding"], "period_ms": item["period_ms"]}
            if strip["label"]:
                alert["label"] = strip["label"]
            if strip["mute_guard"]:
                # A TechAlert strip in double_click_mute keeps its guard on
                # the one alert control (#21).
                alert["mute_guard"] = True
            placed.append((strip["frame"], alert))
        for item in overlay:
            kind = item["kind"]
            if kind == "alert" or id(item) in bound:
                continue
            if kind == "strip":
                why = (
                    "a strip in the overlay"
                    if item["fader"]
                    else "a TechAlert strip without an alert box"
                )
                self.drop(item["path"], why)
            elif kind == "label" or (kind == "area" and item["role"] == "title"):
                self.drop(item["path"], "an overlay label (schema 2 keeps no free text)")
            elif kind == "area":
                self.count("overlay boxes")
            else:
                placed.append((item["frame"], self.control(item)))
        placed.sort(key=lambda p: (p[0][1], p[0][0]))
        return [control for _, control in placed]

    # --- checks (reported, never fixed: X7) ---

    def check_config(self):
        for scope, name in self.unfold:
            key = f"unfold_{scope}" if scope else "unfold"
            checkable = self.set is not None and scope in (None, self.set_instance)
            if checkable and name not in self.set.group_tracks:
                self.report["stale_config"].append(
                    f"{key} '{name}': no such group track in the set"
                )
        for guard in self.guards:
            if guard not in self.guards_used:
                self.report["stale_config"].append(f"double_click_mute '{guard}': matches no strip")

    def check_bindings(self):
        if self.set is None:
            return
        seen = set()
        for binding in self.bindings:
            if binding["instance"] != self.set_instance:
                continue
            anchor = binding["anchor"]
            key = (anchor["kind"], anchor["name"])
            if key in seen:
                continue
            seen.add(key)
            count = self.set.names(anchor["kind"]).count(anchor["name"])
            label = f"{self.set_instance} {anchor['kind']} '{anchor['name']}'"
            if count == 0:
                self.report["unresolved"].append(f"{label}: not in the set")
            elif count > 1:
                self.report["unresolved"].append(
                    f"{label}: {count} tracks have this name (ambiguous)"
                )


def import_files(tosc_path, set_path, set_instance="band"):
    """The layout document (a dict) of a project and, optionally, its band set."""
    root = read_tosc(tosc_path)
    live_set = LiveSet(set_path) if set_path else None
    importer = Importer(root, live_set, set_instance, os.path.basename(tosc_path))
    return importer.run()


def dumps(layout):
    """The layout file's text."""
    return json.dumps(layout, indent=1, ensure_ascii=False) + "\n"


def report_markdown(layout):
    """A readable report of what the import dropped and why, and how it grouped."""
    r = layout["report"]
    lines = [
        "# TouchOSC import report",
        "",
        f"- source: {r['source']}",
        f"- instances: {', '.join(r['instances'])}",
        f"- pages: {', '.join(p['title'] for p in layout['pages'])}",
        f"- rack-macro bus links ignored (channel 16): {r['macro_bus_links']}",
        "",
        "## Former MIDI controls",
        "",
        "| Control | Message | Verdict | Targets | Why |",
        "|---|---|---|---|---|",
    ]
    for m in r["midi"]:
        lines.append(
            f"| {m['control']} | {m['message']} | {m['verdict']} | {m['targets']} | "
            f"{m.get('why', '')} |"
        )
    lines += ["", "## Groups (check them: rows top to bottom, sections left to right)", ""]
    rails = {p["id"]: p.get("rail", []) for p in layout["pages"]}
    for page_id, page_rows in r["groups"].items():
        if rails.get(page_id):
            lines.append(f"- {page_id} rail: {', '.join(display(c) for c in rails[page_id])}")
        for n, row in enumerate(page_rows, 1):
            lines.append(f"- {page_id} row {n}: {' | '.join(row)}")
    global_names = ", ".join(display(c) for c in layout["global"]) or "none"
    lines.append(f"- global (every page): {global_names}")
    sections = [
        ("Guessed groups (their controls are in no area)", r["guessed"]),
        ("Dropped nodes", [f"{d['node']}: {d['why']}" for d in r["dropped"]]),
        ("Clipped to their visible part", r["clipped"]),
        ("Stale config (not fixed)", r["stale_config"]),
        ("Unresolved bindings", r["unresolved"]),
        (
            "Decoration (redrawn natively or dropped)",
            [f"{k}: {v}" for k, v in sorted(r["decoration"].items())],
        ),
        ("Notes", r["notes"]),
    ]
    for title, entries in sections:
        lines += ["", f"## {title}", ""]
        lines += [f"- {e}" for e in entries] or ["- none"]
    return "\n".join(lines) + "\n"


def main(argv=None):
    parser = argparse.ArgumentParser(description="Import a TouchOSC project into layout.json")
    parser.add_argument("tosc")
    parser.add_argument("--set", dest="set_path", default=None, help="the band set (.als)")
    parser.add_argument("--set-instance", default="band")
    parser.add_argument("--out", required=True)
    parser.add_argument("--report", default=None)
    args = parser.parse_args(argv)
    try:
        layout = import_files(args.tosc, args.set_path, args.set_instance)
    except (ImportError_, OSError) as e:
        print(f"import-tosc: {e}", file=sys.stderr)
        return 1
    with open(args.out, "w", encoding="utf-8") as f:
        f.write(dumps(layout))
    if args.report:
        with open(args.report, "w", encoding="utf-8") as f:
            f.write(report_markdown(layout))
    clean = sum(1 for m in layout["report"]["midi"] if m["verdict"] == "clean")
    print(
        f"import-tosc: {len(layout['pages'])} pages, {clean}/{len(layout['report']['midi'])} "
        f"former MIDI controls clean, {len(layout['report']['dropped'])} nodes dropped, "
        f"{len(layout['report']['guessed'])} controls in guessed groups"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
