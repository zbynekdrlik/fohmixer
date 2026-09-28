#!/usr/bin/env python3
"""One-shot import of the engineer's TouchOSC project into the hub's layout.

    python3 import_tosc.py FILE.tosc [--set FOH.als] [--set-instance band]
                           --out layout.json [--report report.md]

Spec §2.6 and the S3 design note §7. It reads the TouchOSC Mk2 project
(lexml, zlib), composes parent-relative frames into canvas coordinates, and
classifies nodes by their role, not by their script alone: strips (by the
names of their parts), solo buttons, the stage mics and STAGE AUT, areas,
labels, the root overlay (the TechAlert strip, REFRESH ALL, the alert box).
The former MIDI controls take their targets and ranges from the band set's
MIDI mappings (``KeyMidi`` in the gzip XML of the ``.als``): a clean mapping
becomes a ``param_toggle`` / ``param_fader`` writing its targets directly
(D10); anything else is dropped (D12). Everything not reproduced is listed in
the report (the layout's ``report`` and ``--report``), never fixed silently:
hidden and off-canvas nodes, decoration, stale config, unresolved names.

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
import sys
import zlib
from dataclasses import dataclass, field
from xml.etree import ElementTree

SCHEMA = 1
ALERT_PERIOD_MS = 300
# A strip narrower than this (px) is a narrow strip (the STAGE page's 136).
NARROW_BELOW = 150.0
# TouchOSC's orientation property: north, east, south, west.
ORIENTATION = {0: "top", 1: "right", 2: "bottom", 3: "left"}
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
    """The Conf page's TEXT (the one that names the connections)."""
    if node.type == "TEXT" and "connection_" in node.text:
        return node.text
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


def _midi_of(node):
    return next((m for m in node.midi if m.enabled and m.send), None)


def _is_refresh(node):
    """The REFRESH ALL control: a label or button whose code asks for the refresh."""
    code = LUA_COMMENT.sub("", node.script)
    return node.type in ("LABEL", "BUTTON") and REFRESH_CALL.search(code) is not None


def _static_labels(nodes):
    """The labels among ``nodes`` that can name a former MIDI control."""
    return [
        n
        for n in nodes
        if n.type == "LABEL"
        and not n.script.strip()
        and n.text.strip()
        and n.text.strip().upper() != "ON/OFF"
    ]


def _covers(a, b):
    """Whether two frames of one parent overlap by at least half the smaller one."""
    w = min(a[0] + a[2], b[0] + b[2]) - max(a[0], b[0])
    h = min(a[1] + a[3], b[1] + b[3]) - max(a[1], b[1])
    return w > 0 and h > 0 and w * h >= min(a[2] * a[3], b[2] * b[3]) / 2


def _with_labels(children):
    """Each child with the sibling labels drawn over it when it is a former MIDI
    control outside a group; those labels name the control and are skipped."""
    labels, taken = {}, set()
    static = _static_labels(children)
    for control in children:
        if control.type in ("BUTTON", "FADER") and _midi_of(control):
            own = [n for n in static if id(n) not in taken and _covers(n.frame, control.frame)]
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


class Importer:
    def __init__(self, root, live_set, set_instance, source):
        self.root = root
        self.set = live_set
        self.set_instance = set_instance
        self.canvas = root.frame[2], root.frame[3]
        instances, unfold, guards = _config(_find_config(root) or "")
        self.instances = instances or ["band"]
        self.unfold = unfold
        self.guards = guards
        self.guards_used = set()
        self.z = 0
        self.report = {
            "source": source,
            "instances": self.instances,
            "dropped": [],
            "midi": [],
            "stale_config": [],
            "unresolved": [],
            "decoration": {},
            "macro_bus_links": live_set.macro_bus_links if live_set else 0,
            "notes": [
                "a track name with a leading letter and a hyphen (A-…) is bound as a return track",
                "double_click_mute entries are matched as plain text, not Lua patterns",
            ],
            "scripts": {},
        }
        self.bindings = []  # (instance, anchor) of strips, solos, stages
        self.alerts = []  # (item, its layer, path): bound after the whole overlay
        self.page_ids = set()  # unique across every pager
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

    def count(self, what):
        decoration = self.report["decoration"]
        decoration[what] = decoration.get(what, 0) + 1

    def next_z(self):
        self.z += 1
        return self.z

    def frame(self, x, y, w, h):
        return {"x": float(x), "y": float(y), "w": float(w), "h": float(h)}

    def inside(self, x, y, w, h):
        cw, ch = self.canvas
        return (
            w > 0 and h > 0 and x >= -0.5 and y >= -0.5 and x + w <= cw + 0.5 and y + h <= ch + 0.5
        )

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

    def style(self, node, text=None):
        style = {}
        if node.prop("background", False) and "color" in node.props:
            style["bg"] = color_hex(node.prop("color"))
        if text:
            style["text"] = text
        if "textColor" in node.props:
            style["text_color"] = color_hex(node.prop("textColor"))
        if "textSize" in node.props:
            style["text_size"] = float(node.prop("textSize"))
        if node.prop("orientation", 0) in (1, 3) and node.type in ("LABEL", "TEXT"):
            style["vertical"] = True
        return style

    def item(self, kind, frame, style=None, **fields):
        item = {"kind": kind, "frame": frame, "z": self.next_z(), "style": style or {}}
        item.update(fields)
        return item

    # --- pages ---

    def run(self):
        pager = next((c for c in self.root.children if c.type == "PAGER"), None)
        if pager is None:
            raise ImportError_("the project has no root pager")
        tabbar, pages = self.pager(pager, 0.0, 0.0, "root")
        overlay = []
        for child, labels in _with_labels(self.root.children):
            if child is not pager:
                self.collect(child, 0.0, 0.0, overlay, "root", labels, root_level=True)
        self.resolve_alerts()
        self.check_config()
        self.check_bindings()
        return {
            "schema": SCHEMA,
            "canvas": {"w": float(self.canvas[0]), "h": float(self.canvas[1])},
            "tabbar": tabbar,
            "pages": pages,
            "overlay": overlay,
            "config": {
                "unfold": [
                    {"instance": instance, "name": name}
                    for scope, name in self.unfold
                    for instance in ([scope] if scope else self.instances)
                ]
            },
            "report": self.report,
        }

    def pager(self, node, ox, oy, where):
        x, y, _, _ = node.frame
        ax, ay = ox + x, oy + y
        if _midi_of(node):
            self.drop(f"{where}/{node.name}", "the pager's page-change message (X6)")
        pages = []
        for page in node.children:
            if page.type != "GROUP":
                self.drop(f"{where}/{node.name}/{page.name}", "not a page")
                continue
            pages.append(self.page(page, ax, ay, f"{where}/{node.name}"))
        default = int(float(node.values.get("page", "0") or 0))
        if not 0 <= default < max(1, len(pages)):
            self.drop(f"{where}/{node.name}", f"default page {default} out of range: 0")
            default = 0
        tabbar = {
            "orientation": ORIENTATION.get(node.prop("orientation", 0), "top"),
            "bar_size": float(node.prop("tabbarSize", 0) if node.prop("tabbar", True) else 0),
            "default_page": default,
        }
        self.tab_text = float(node.prop("textSizeOff", 0))
        for page in pages:
            page["tab"]["text_size"] = self.tab_text
        return tabbar, pages

    def page(self, node, ox, oy, where):
        x, y, _, _ = node.frame
        ax, ay = ox + x, oy + y
        title = node.prop("tabLabel", "") or node.name
        base = re.sub(r"[^a-z0-9]+", "-", title.lower()).strip("-") or "page"
        page_id, n = base, 1
        while page_id in self.page_ids:
            n += 1
            page_id = f"{base}-{n}"
        self.page_ids.add(page_id)
        tab = {}
        if "tabColorOff" in node.props:
            tab["color"] = color_hex(node.prop("tabColorOff"))
        page = {"id": page_id, "title": title, "tab": tab, "items": []}
        path = f"{where}/{node.name}"
        for child, labels in _with_labels(node.children):
            if child.type == "PAGER":
                if "pager" in page:
                    self.drop(f"{path}/{child.name}", "a second pager on one page")
                    continue
                cx, cy, cw, chh = child.frame
                if not child.visible or not self.inside(ax + cx, ay + cy, cw, chh):
                    why = "hidden" if not child.visible else "off the canvas"
                    self.drop(f"{path}/{child.name}", why)
                    continue
                tabbar, pages = self.pager(child, ax, ay, path)
                page["pager"] = {
                    "frame": self.frame(ax + cx, ay + cy, cw, chh),
                    "tabbar": tabbar,
                    "pages": pages,
                }
            else:
                self.collect(child, ax, ay, page["items"], path, labels)
        return page

    # --- nodes ---

    def collect(self, node, ox, oy, out, where, labels=(), root_level=False):
        """A node and its children; ``ox``/``oy`` is its parent's canvas origin,
        ``labels`` the sibling labels that name it (a former MIDI control)."""
        x, y, w, h = node.frame
        ax, ay = ox + x, oy + y
        path = f"{where}/{node.name}"
        if "getBatteryLevel" in node.script:
            self.drop(path, "the battery gauge (X5)")
            return
        if not node.visible:
            if root_level and node.type == "BOX" and node.script.strip():
                self.alert(node, ax, ay, out, path)
            else:
                self.drop(path, "hidden")
            return
        if not self.inside(ax, ay, w, h):
            self.drop(path, "off the canvas")
            return
        frame = self.frame(ax, ay, w, h)
        if _is_strip(node):
            out.append(self.strip(node, ax, ay, frame, path))
        elif node.type == "GROUP" and node.child("btn_solo"):
            out.append(self.item("solo", frame, self.style(node), binding=self.binding(node.name)))
        elif node.type == "GROUP" and node.child("btn_mute"):
            out.append(
                self.item(
                    "stage",
                    frame,
                    self.style(node, self.group_text(node)),
                    binding=self.binding(node.name),
                    aut=self.has(self.root, "btn_stage_aut"),
                )
            )
        elif node.name == "btn_stage_aut" or (node.type == "GROUP" and node.child("btn_stage_aut")):
            out.append(
                self.item(
                    "hub_toggle",
                    frame,
                    self.style(node),
                    key="stage_aut",
                    label=self.group_text(node) or "STAGE AUT",
                )
            )
        elif node.type == "GROUP" and any(_midi_of(c) for c in node.children):
            self.midi_group(node, ax, ay, out, path)
        elif node.type in ("BUTTON", "FADER") and _midi_of(node):
            label = labels[0].text.strip() if labels else node.name
            self.midi_control(node, ox, oy, label, out, where)
        elif _is_refresh(node):
            out.append(self.item("refresh", frame, self.style(node), label=node.text or node.name))
        elif node.type == "GROUP":
            if node.prop("background", False):
                out.append(self.item("area", frame, self.style(node)))
            for child, labels in _with_labels(node.children):
                self.collect(child, ax, ay, out, path, labels)
        elif node.type == "BOX":
            out.append(
                self.item("area", frame, {"bg": color_hex(node.prop("color", (0, 0, 0, 1)))})
            )
        elif node.type == "LABEL" and node.prop("background", False):
            out.append(self.item("area", frame, self.style(node, node.text), title=node.text))
        elif node.type in ("LABEL", "TEXT"):
            out.append(self.item("label", frame, self.style(node), text=node.text))
        elif node.type == "BUTTON" and node.prop("background", False):
            # A button with no function here (a backdrop carrying the inert
            # mute script): only its fill is kept, as an inert area.
            out.append(self.item("area", frame, self.style(node)))
        else:
            self.drop(path, f"a {node.type} without a function here")

    def has(self, node, name):
        return node.name == name or any(self.has(c, name) for c in node.children)

    def group_text(self, node):
        return next((c.text for c in node.children if c.type == "LABEL" and c.text), "")

    def alert(self, node, ax, ay, out, path):
        """The hidden full-screen box that blinks while TechAlert is unmuted.

        It keeps its place (z) in node order; its TechAlert strip may come
        later among the root's children, so ``resolve_alerts`` binds it once
        the whole overlay is collected."""
        _, _, w, h = node.frame
        cw, ch = self.canvas
        x0, y0 = max(0.0, ax), max(0.0, ay)
        x1, y1 = min(cw, ax + w), min(ch, ay + h)
        if (x0, y0, x1, y1) != (ax, ay, ax + w, ay + h):
            self.report["notes"].append(f"{path}: clipped to the canvas")
        item = self.item(
            "alert",
            self.frame(x0, y0, x1 - x0, y1 - y0),
            {"bg": color_hex(node.prop("color", (1, 0, 0, 0.12)))},
            binding=None,
            period_ms=ALERT_PERIOD_MS,
        )
        out.append(item)
        self.alerts.append((item, out, path))

    def resolve_alerts(self):
        """Binds each alert box to the TechAlert strip of its layer, or drops it."""
        for item, out, path in self.alerts:
            strips = [
                i for i in out if i["kind"] == "strip" and i["strip_kind"] == "meter_mute_only"
            ]
            if strips:
                item["binding"] = strips[0]["binding"]
            else:
                out.remove(item)
                self.drop(path, "an alert box without a TechAlert strip")

    def strip(self, node, ax, ay, frame, path):
        mute_hashes = self._role_hash("mute")
        meter_hashes = self._role_hash("meter")
        children = {}
        for child in node.children:
            cx, cy, cw, ch = child.frame
            part = STRIP_PARTS.get(child.name)
            if part and part not in children:
                if self.inside(ax + cx, ay + cy, cw, ch):
                    children[part] = self.frame(ax + cx, ay + cy, cw, ch)
                else:
                    self.drop(f"{path}/{child.name}", "a strip part off the canvas")
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
        binding = self.binding(node.name)
        if "fader" not in children:
            kind = "meter_mute_only"
        elif binding["anchor"]["kind"] == "return":
            kind = "return"
        elif node.prop("background", False):
            kind = "solid"
        elif frame["w"] < NARROW_BELOW:
            kind = "narrow"
        else:
            kind = "standard"
        guarded = [g for g in self.guards if g in node.name]
        self.guards_used.update(guarded)
        return self.item(
            "strip",
            frame,
            self.style(node),
            binding=binding,
            strip_kind=kind,
            children=children,
            mute_guard=bool(guarded),
        )

    # --- former MIDI controls ---

    def midi_group(self, node, ox, oy, out, path):
        control = next(c for c in node.children if _midi_of(c))
        static = _static_labels(node.children)
        label = static[0].text.strip() if static else control.name
        for child in node.children:
            if child is control or (static and child is static[0]):
                continue
            if child.type == "LABEL":
                self.count("labels of former MIDI controls")
            else:
                self.drop(f"{path}/{child.name}", "part of a former MIDI control")
        self.midi_control(control, ox, oy, label, out, path)

    def midi_control(self, node, ox, oy, label, out, where):
        """A former MIDI control; ``ox``/``oy`` is its parent's canvas origin."""
        midi = _midi_of(node)
        x, y, w, h = node.frame
        ax, ay = ox + x, oy + y
        entry = {"control": label, "message": _message(midi), "verdict": "dropped", "targets": 0}
        self.report["midi"].append(entry)
        try:
            if not self.inside(ax, ay, w, h):
                raise Drop("off the canvas")
            item = self.midi_item(node, midi, label, self.frame(ax, ay, w, h))
        except Drop as e:
            entry["why"] = str(e)
            self.drop(f"{where}/{node.name}", f"former MIDI control {entry['message']}: {e}")
            return
        entry["verdict"] = "clean"
        entry["targets"] = len(item["targets"])
        out.append(item)

    def midi_item(self, node, midi, label, frame):
        if self.set is None:
            raise Drop("no set given (--set)")
        mappings = self.set.find(midi.kind.startswith("NOTE"), midi.channel, midi.data1)
        if not mappings:
            raise Drop("no mapping in the set")
        style = {"bg": color_hex(node.prop("color", (0.5, 0.5, 0.5, 1)))}
        if node.type == "FADER":
            if not (_close(midi.scale[0], 0) and _close(midi.scale[1], 127)):
                raise Drop("the fader's MIDI scale is not 0-127")
            targets = [self.fader_target(m) for m in mappings]
            return self.item("param_fader", frame, style, label=label, targets=targets)
        press = _press(node)
        off_cc, on_cc = midi.scale
        targets = [self.toggle_target(m, on_cc, off_cc) for m in mappings]
        return self.item("param_toggle", frame, style, label=label, targets=targets, press=press)

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
    """A readable report of what the import dropped and why."""
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
    sections = [
        ("Dropped nodes", [f"{d['node']}: {d['why']}" for d in r["dropped"]]),
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
        f"former MIDI controls clean, {len(layout['report']['dropped'])} nodes dropped"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
