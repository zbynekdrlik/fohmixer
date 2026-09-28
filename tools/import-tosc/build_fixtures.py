#!/usr/bin/env python3
"""Builds the synthetic fixtures of the TouchOSC import tool from Python literals.

- ``test.tosc``: a TouchOSC Mk2 project (lexml, zlib-compressed), shaped like
  the engineer's surface: a root pager (musician cue page, FOH, Conf) with a
  nested vertical pager, strips of every kind with their decoration, solo
  buttons, the stage mics and STAGE AUT, former MIDI controls, a root overlay
  (TechAlert strip, REFRESH ALL, the battery gauge, the hidden alert box).
- ``test.als``: a band set (gzip XML) with the MIDI mappings those controls hit.

Every name is invented (the tracks of ``sim/fixtures/test-site.json``); the
real project and set never enter this public repository (spec §5.2). The
output is deterministic (fixed compression settings, no timestamps).

    python3 build_fixtures.py OUT_DIR
"""

import gzip
import io
import os
import sys
import zlib
from xml.sax.saxutils import escape

# --- scripts (the hash table is built from these, as from a real project) ---

# Like the real mute script, it mentions a refresh (in a comment).
MUTE_SCRIPT = (
    "-- mute_button\n-- Fixed: keep the state during a refresh\n"
    "function onValueChanged(k)\n  -- send mute\nend\n"
)
METER_SCRIPT = "-- meter_script\nfunction onReceiveNotify(k, v)\nend"  # no trailing newline
FADER_SCRIPT = "-- fader_script\nfunction onValueChanged(k)\nend\n"
PAN_SCRIPT = "-- pan_control\nfunction onValueChanged(k)\nend\n"
GROUP_SCRIPT = "-- group_init\nfunction refresh_tracks()\nend\n"
DB_SCRIPT = "-- db_label\n"
DOUBLE_TAP_SCRIPT = "-- latch on a double tap\nfunction onValueChanged(k)\nend\n"
# The REFRESH ALL control asks the document script to refresh every strip.
REFRESH_SCRIPT = (
    "-- Global Refresh Button Script\nfunction onValueChanged(key)\n"
    '  if key == "x" then\n    root:notify("refresh_all_groups")\n  end\nend\n'
)
BATTERY_SCRIPT = "-- battery\nfunction update()\n  local b = getBatteryLevel()\nend\n"
ALERT_SCRIPT = "-- TechAlert blink\nfunction update()\nend\n"
SOLO_SCRIPT = "-- solo group\n"
STAGE_SCRIPT = "-- stage mics\n"
# Every label of a former MIDI control carries it (restyle by incoming OSC).
RESTYLE_SCRIPT = "-- restyle on receive\nfunction onReceiveOSC(message, connections)\nend\n"
# The dead volume readout next to the Podklady fader.
VOLUME_LABEL_SCRIPT = (
    "-- volume label\nfunction onReceiveOSC(message)\n  self.values.text = '- 0.0'\nend\n"
)

CONFIG_TEXT = (
    "connection_band: 2\n"
    "connection_master: 3\n"
    "unfold_band: 'Vocals Repro grp#'\n"
    "unfold_band: 'Old grp#'\n"
    "double_click_mute: 'master_Hand1 #'\n"
    "double_click_mute: 'master_A-Echo'\n"
    "double_click_mute: 'band_Nothing'\n"
)

WHITE = (1, 1, 1, 1)
BLACK = (0, 0, 0, 1)
GREY = (0.25, 0.25, 0.25, 1)

# --- lexml ---------------------------------------------------------------------------


def _num(value):
    return f"{value:g}" if isinstance(value, float) else str(value)


def _prop(key, value):
    if isinstance(value, bool):
        kind, body = "b", "1" if value else "0"
    elif isinstance(value, int):
        kind, body = "i", str(value)
    elif isinstance(value, float):
        kind, body = "f", _num(value)
    elif isinstance(value, str):
        kind, body = "s", f"<![CDATA[{value}]]>"
    elif len(value) == 4 and key == "frame":
        kind = "r"
        body = "".join(f"<{k}>{_num(v)}</{k}>" for k, v in zip("xywh", value, strict=True))
    else:
        kind = "c"
        body = "".join(f"<{k}>{_num(v)}</{k}>" for k, v in zip("rgba", value, strict=True))
    return f"<property type='{kind}'><key><![CDATA[{key}]]></key><value>{body}</value></property>"


def _value(key, default):
    return (
        f"<value><key><![CDATA[{key}]]></key><locked>0</locked>"
        f"<lockedDefaultCurrent>0</lockedDefaultCurrent>"
        f"<default><![CDATA[{default}]]></default><defaultPull>0</defaultPull></value>"
    )


def _midi(message, channel, data1, scale=(0, 127)):
    lo, hi = scale
    return (
        "<midi><enabled>1</enabled><send>1</send><receive>1</receive><feedback>0</feedback>"
        "<noDuplicates>0</noDuplicates><connections>1111111111</connections>"
        "<triggers><trigger><var><![CDATA[x]]></var><condition>ANY</condition></trigger></triggers>"
        f"<message><type>{message}</type><channel>{channel}</channel>"
        f"<data1>{data1}</data1><data2>0</data2></message>"
        "<values>"
        "<value><type>CONSTANT</type><key><![CDATA[]]></key><scaleMin>0</scaleMin><scaleMax>15</scaleMax></value>"
        "<value><type>CONSTANT</type><key><![CDATA[]]></key><scaleMin>0</scaleMin><scaleMax>127</scaleMax></value>"
        f"<value><type>VALUE</type><key><![CDATA[x]]></key><scaleMin>{lo}</scaleMin><scaleMax>{hi}</scaleMax></value>"
        "</values></midi>"
    )


class N:
    """A node literal: type, name, frame, properties, values, MIDI, children."""

    _ids = iter(range(1, 100000))

    def __init__(self, kind, name, frame, children=(), midi=(), values=None, **props):
        self.kind = kind
        self.props = {"name": name, "frame": tuple(frame), "visible": True, **props}
        self.values = values or {}
        self.midi = list(midi)
        self.children = list(children)
        self.id = f"00000000-0000-4000-8000-{next(N._ids):012d}"

    def xml(self):
        props = "".join(_prop(k, v) for k, v in sorted(self.props.items()))
        values = "".join(_value(k, v) for k, v in self.values.items())
        messages = f"<messages>{''.join(self.midi)}</messages>" if self.midi else ""
        children = (
            f"<children>{''.join(c.xml() for c in self.children)}</children>"
            if self.children
            else ""
        )
        return (
            f"<node ID='{self.id}' type='{self.kind}'><properties>{props}</properties>"
            f"<values>{values}</values>{messages}{children}</node>"
        )


def label(name, frame, text, **props):
    return N("LABEL", name, frame, values={"text": text}, **props)


def strip(name, x, y, w=160, h=710, background=False, color=GREY):
    """A strip group with its parts (parent-relative frames) and its decoration."""
    parts = [
        # Backdrops that carry the mute script but do nothing (decoration).
        N("BUTTON", "button1", (15, 20, w - 30, 58), script=MUTE_SCRIPT, buttonType=1),
        N("BUTTON", "button2", (15, 640, w - 30, 62), script=MUTE_SCRIPT, buttonType=1),
        N("FADER", "pan", (15, 25, w - 30, 48), script=PAN_SCRIPT, response=1),
        N("FADER", "fader", (15, 83, w - 30, 553), script=FADER_SCRIPT, response=1),
        # The script's trailing newlines differ from the second bar's: the
        # hash table normalises them.
        N("FADER", "meter", (1, 83, 10, 553), script=METER_SCRIPT + "\n\n"),
        N("FADER", "fader48", (5, 83, 10, 553), script=METER_SCRIPT),
        N("BUTTON", "mute", (15, 648, w - 30, 52), script=MUTE_SCRIPT, buttonType=1),
        label("track_label", (16, 653, w - 32, 42), "Old name"),
        label("connection_label", (16, 151, w - 32, 20), "band"),
        label("db", (15, 116, w - 30, 30), "-inf", script=DB_SCRIPT),
        label("db_meter_label", (15, 173, w - 30, 20), "-inf dBFS", script=DB_SCRIPT),
        N("BOX", "status_indicator", (24, 88, 88, 20)),
        label("scale6", (w - 40, 332, 30, 20), "-6"),
        N("TEXT", "tick1", (0, 184, w, 10), values={"text": "_____"}),
    ]
    return N(
        "GROUP",
        name,
        (x, y, w, h),
        parts,
        script=GROUP_SCRIPT,
        background=background,
        color=color,
    )


def backdrop(name, frame):
    """A dark, non-interactive button behind a control, carrying the mute script."""
    return N(
        "BUTTON",
        name,
        frame,
        script=MUTE_SCRIPT,
        buttonType=1,
        press=True,
        release=True,
        interactive=False,
        background=True,
        color=(0, 0, 0, 0.74),
    )


def alert_strip(name, x, y):
    """The TechAlert strip: meter, status, mute and label, no fader or pan."""
    parts = [
        # Taller than its group, as in the real project: TouchOSC shows the
        # 12 px inside the group.
        N("FADER", "meter", (10, 85, 10, 552), script=METER_SCRIPT),
        N("BOX", "status_indicator", (20, 5, 80, 15)),
        N("BUTTON", "mute", (15, 50, 94, 40), script=MUTE_SCRIPT, buttonType=1),
        label("track_label", (16, 55, 92, 30), "TechAlert"),
    ]
    return N("GROUP", name, (x, y, 124, 97), parts, script=GROUP_SCRIPT)


def midi_group(name, frame, button, text, state_label=True):
    """A former MIDI control's group: the button, its name label and the
    "ON/OFF" label, both labels carrying the restyle script."""
    kids = [
        button,
        label("name", (0, 60, frame[2], 30), text, textColor=WHITE, script=RESTYLE_SCRIPT),
    ]
    if state_label:
        kids.append(label("state", (0, 80, frame[2], 20), "ON/OFF", script=RESTYLE_SCRIPT))
    return N("GROUP", name, frame, kids)


def project():
    cue = N(
        "GROUP",
        "Cue",
        (0, 59, 2360, 1581),
        [
            midi_group(
                "group29",
                (591, 110, 156, 100),
                N(
                    "BUTTON",
                    "button",
                    (0, 0, 156, 60),
                    [],
                    [_midi("CONTROLCHANGE", 13, 20)],
                    buttonType=1,
                    press=True,
                    release=True,
                    color=(0.46, 0.8, 0.15, 1),
                ),
                "Vox 1 TU",
            ),
            midi_group(
                "group33",
                (747, 233, 156, 100),
                N(
                    "BUTTON",
                    "button",
                    (0, 0, 156, 60),
                    [],
                    [_midi("CONTROLCHANGE", 13, 28)],
                    buttonType=1,
                    press=True,
                    release=True,
                ),
                "Gitara 2",
            ),
            midi_group(
                "group35",
                (776, 359, 254, 92),
                N(
                    "BUTTON",
                    "button",
                    (0, 0, 254, 60),
                    [],
                    [_midi("NOTE_ON", 13, 29, (12, 11))],
                    buttonType=0,
                    press=True,
                    release=True,
                ),
                "ALERT LOOP",
                state_label=False,
            ),
        ],
        tabLabel="Cue",
        tabColorOff=(0.25, 0.25, 0.25, 1),
        tabColorOn=(0.5, 0.5, 0.5, 1),
    )
    stage_page = N(
        "GROUP",
        "STAGE",
        (65, 0, 1681, 773),
        [
            # A grey backdrop far larger than the pager: what shows is the
            # part inside the page.
            label(
                "backdrop",
                (-243, -80, 2512, 920),
                "",
                background=True,
                orientation=3,
                locked=True,
                color=(0.616, 0.616, 0.627, 1),
            ),
            N("BOX", "panel", (0, 0, 1681, 773), color=(0.73, 1, 0.65, 0.34)),
            label(
                "title",
                (0, 0, 60, 700),
                "STAGE",
                background=True,
                orientation=3,
                color=(0.73, 1, 0.65, 1),
            ),
            strip("band_Vocal 1 repro#", 95, 40, w=136, h=706),
            strip("Keys 1", 243, 40, w=136, h=706),
        ],
        tabLabel="STAGE",
        tabColorOff=(0.73, 1, 0.65, 0.34),
    )
    others_page = N(
        "GROUP",
        "OTHERS",
        (65, 0, 1681, 773),
        [strip("master_Hand1 #", 102, 34)],
        tabLabel="OTHERS",
        tabColorOff=(0.98, 0, 0, 0.29),
    )
    sub_pager = N(
        "PAGER",
        "FOH ",
        (229, 2, 1746, 773),
        [stage_page, others_page],
        [_midi("CONTROLCHANGE", 0, 0)],
        values={"page": "0"},
        orientation=3,
        tabbar=True,
        tabbarSize=65,
        textSizeOff=36,
        textSizeOn=51,
    )
    foh = N(
        "GROUP",
        "WORSHIP",
        (0, 59, 2360, 1581),
        [
            N("BOX", "box8", (27, 10, 198, 1240), color=(0, 0, 0, 0.5)),
            # Starts above the page, under the root tab bar.
            N("BOX", "box9", (37, -55, 178, 1240), color=(0, 0, 0, 0.5)),
            # Dark backdrops behind the sidebar controls: they carry the
            # mute script but are never notified (inert).
            *[
                backdrop(name, frame)
                for name, frame in (
                    ("button72", (40, 115, 101, 77)),
                    ("button71", (40, 13, 101, 77)),
                    ("button70", (51, 220, 137, 45)),
                    ("button69", (52, 275, 136, 47)),
                    ("button65", (78, 898, 110, 66)),
                )
            ],
            sub_pager,
            N(
                "GROUP",
                "Mics Stage #",
                (27, 19, 115, 85),
                [
                    N("BUTTON", "btn_mute", (0, 0, 115, 85), script=STAGE_SCRIPT, buttonType=1),
                    label("label", (0, 50, 115, 30), "STAGE"),
                ],
                script=STAGE_SCRIPT,
                background=True,
                color=(0, 0.02, 0.58, 1),
            ),
            N(
                "GROUP",
                "group70",
                (27, 121, 115, 85),
                [
                    N("BUTTON", "btn_stage_aut", (0, 0, 115, 85), buttonType=1),
                    label("label", (0, 50, 115, 30), "STAGE AUT"),
                ],
                background=True,
                color=(0, 0.02, 0.58, 1),
            ),
            N(
                "GROUP",
                "Vocals Repro grp#",
                (27, 219, 161, 65),
                [
                    N("BUTTON", "btn_solo", (0, 0, 161, 65), script=SOLO_SCRIPT, buttonType=2),
                    label("label", (0, 30, 161, 30), "Vocals"),
                ],
                script=SOLO_SCRIPT,
            ),
            N(
                "GROUP",
                "Stems grp#",
                (27, 274, 161, 65),
                [
                    N("BUTTON", "btn_solo", (0, 0, 161, 65), script=SOLO_SCRIPT, buttonType=2),
                    label("label", (0, 30, 161, 30), "Stems"),
                ],
                script=SOLO_SCRIPT,
            ),
            midi_group(
                "group3",
                (27, 396, 149, 120),
                N(
                    "BUTTON",
                    "button37",
                    (0, 0, 149, 60),
                    [],
                    [_midi("CONTROLCHANGE", 13, 55)],
                    buttonType=1,
                    press=True,
                    release=True,
                    color=(0.53, 0.34, 0, 1),
                ),
                "REVERB",
                state_label=False,
            ),
            midi_group(
                "group2",
                (27, 486, 197, 154),
                N(
                    "BUTTON",
                    "button35",
                    (0, 0, 197, 60),
                    [],
                    [_midi("CONTROLCHANGE", 13, 56)],
                    buttonType=1,
                    press=False,
                    release=False,
                    script=DOUBLE_TAP_SCRIPT,
                ),
                "VOC MIC",
                state_label=False,
            ),
            midi_group(
                "group4",
                (27, 613, 204, 122),
                N(
                    "BUTTON",
                    "button36",
                    (0, 0, 204, 60),
                    [],
                    [_midi("CONTROLCHANGE", 13, 36)],
                    buttonType=1,
                    press=True,
                    release=True,
                ),
                "AUTOTUNE",
                state_label=False,
            ),
            midi_group(
                "group68",
                (27, 746, 160, 130),
                N(
                    "BUTTON",
                    "button40",
                    (0, 0, 160, 60),
                    [],
                    [_midi("CONTROLCHANGE", 13, 30)],
                    buttonType=0,
                    press=True,
                    release=True,
                    script=DOUBLE_TAP_SCRIPT,
                ),
                "ZVUKAR",
                state_label=False,
            ),
            # A former MIDI button outside any group (as on the real
            # sidebar): its visible label is a sibling drawn over it.
            N(
                "BUTTON",
                "button42",
                (75, 909, 102, 60),
                [],
                [_midi("CONTROLCHANGE", 13, 31)],
                buttonType=1,
                press=True,
                release=True,
                color=(0.53, 0.34, 0, 1),
            ),
            label(
                "label942",
                (67, 898, 119, 84),
                "REPRO",
                textColor=WHITE,
                textSize=24,
                script=RESTYLE_SCRIPT,
            ),
            midi_group(
                "group81",
                (27, 960, 102, 60),
                N(
                    "BUTTON",
                    "button43",
                    (0, 0, 102, 30),
                    [],
                    [_midi("CONTROLCHANGE", 13, 58)],
                    buttonType=1,
                    press=True,
                    release=True,
                ),
                "HALF",
                state_label=False,
            ),
            midi_group(
                "group82",
                (27, 1030, 102, 60),
                N(
                    "BUTTON",
                    "button44",
                    (0, 0, 102, 30),
                    [],
                    [_midi("CONTROLCHANGE", 13, 67)],
                    buttonType=1,
                    press=True,
                    release=True,
                ),
                "SELECT",
                state_label=False,
            ),
            N(
                "GROUP",
                "group142",
                (755, 1000, 116, 439),
                [
                    N(
                        "FADER",
                        "fader42",
                        (0, 20, 101, 399),
                        [],
                        [_midi("CONTROLCHANGE", 13, 57)],
                        response=1,
                        grid=True,
                        gridSteps=10,
                    ),
                    # The name is two labels ("All" drawn above "Podklady").
                    label("label899", (7, 349, 104, 46), "Podklady"),
                    label("label946", (8, 309, 104, 46), "All"),
                    label("label950", (-43, 28, 205, 84), "- 0.0", script=VOLUME_LABEL_SCRIPT),
                ],
            ),
            # The bottom row's area and its vertical title overhang the
            # canvas's bottom edge, as in the real project.
            label(
                "effects_area",
                (271, 842, 529, 754),
                "",
                background=True,
                orientation=3,
                color=(0.39, 0.39, 0.39, 1),
            ),
            label(
                "effects",
                (247, 843, 53, 748),
                "EFFECTS",
                background=True,
                orientation=3,
                color=(0.39, 0.39, 0.39, 1),
            ),
            N("GROUP", "group100", (100, 100, 60, 40), [label("mark", (10, 10, 20, 20), "[]")]),
            strip(
                "band_B-Main repro #",
                2005,
                46,
                w=161,
                h=700,
                background=True,
                color=(0.38, 0.47, 1, 1),
            ),
            strip("Hand2 #", 2180, 46, w=161, h=700, background=True, color=(0.96, 1, 0.08, 1)),
            # A return strip 28 px past the canvas's bottom edge; every part
            # of it is inside.
            strip(
                "master_A-Echo",
                2180,
                839,
                w=161,
                h=770,
                background=True,
                color=(0.96, 1, 0.08, 1),
            ),
            strip("master_Hand3 #", 1420, 1770),
            label("hidden", (1610, 834, 50, 20), "gone", visible=False),
        ],
        tabLabel="FOH",
        tabColorOff=(0.25, 0.25, 0.25, 1),
    )
    conf = N(
        "GROUP",
        "Conf",
        (0, 59, 2360, 1581),
        [
            N(
                "TEXT",
                "config",
                (40, 40, 800, 400),
                values={"text": CONFIG_TEXT},
                textSize=20,
                textColor=WHITE,
            )
        ],
        tabLabel="Conf",
        tabColorOff=(0.25, 0.25, 0.25, 1),
    )
    pager = N(
        "PAGER",
        "pager1",
        (0, 0, 2360, 1640),
        [cue, foh, conf],
        values={"page": "1"},
        orientation=0,
        tabbar=True,
        tabbarSize=59,
        textSizeOff=33,
        textSizeOn=52,
    )
    tech_alert = alert_strip("band_TechAlert #", 67, 1059)
    return N(
        "GROUP",
        "root",
        (0, 0, 2360, 1640),
        [
            pager,
            tech_alert,
            # The battery gauge: a plain group whose fader carries the
            # battery script, with a box and a "100%" label.
            N(
                "GROUP",
                "battery",
                (66, 1152, 136, 80),
                [
                    N("BOX", "+", (104, 36, 24, 23), color=(0, 1, 0, 0.43)),
                    N(
                        "FADER",
                        "main",
                        (5, 25, 112, 45),
                        script=BATTERY_SCRIPT,
                        orientation=1,
                        background=True,
                        color=(0, 1, 0, 0.27),
                    ),
                    label(
                        "label",
                        (4, 26, 119, 44),
                        "100%",
                        background=True,
                        color=(0, 1, 0, 0.27),
                    ),
                ],
                background=True,
                color=(0, 0, 0, 0),
            ),
            label(
                "refresh",
                (21, 1300, 209, 54),
                "REFRESH ALL",
                script=REFRESH_SCRIPT,
                background=True,
                color=(0.5, 0.5, 0.5, 1),
            ),
            N(
                "BOX",
                "alert",
                (-40, 80, 2400, 1553),
                script=ALERT_SCRIPT,
                visible=False,
                locked=True,
                color=(1, 0, 0, 0.12),
            ),
        ],
        color=BLACK,
        background=True,
    )


def alert_first(root):
    """``root`` with its children in the real project's order: the hidden alert
    box right after the pager, the TechAlert strip last."""
    by_name = {c.props["name"]: c for c in root.children}
    order = ("pager1", "alert", "refresh", "battery", "band_TechAlert #")
    root.children = [by_name[name] for name in order]
    return root


def tosc_bytes(root=None):
    """The zlib-compressed project file of ``root`` (default: ``project()``)."""
    root = project() if root is None else root
    xml = "<?xml version='1.0' encoding='UTF-8'?><lexml version='6'>" + root.xml() + "</lexml>"
    return zlib.compress(xml.encode("utf-8"), 9)


# --- the band set (.als) ---------------------------------------------------------------


def _key_midi(channel, number, is_note=False):
    return (
        '<KeyMidi><PersistentKeyString Value="" />'
        f'<IsNote Value="{"true" if is_note else "false"}" />'
        f'<Channel Value="{channel}" /><NoteOrController Value="{number}" />'
        '<LowerRangeNote Value="-1" /><UpperRangeNote Value="-1" />'
        '<ControllerMapMode Value="0" /></KeyMidi>'
    )


def _param(tag, mapping=None, switch=None, continuous=None):
    """A mappable parameter element; ``mapping`` is (channel, number)."""
    body = '<LomId Value="0" />'
    if mapping is not None:
        body += _key_midi(*mapping)
    body += '<Manual Value="1" />'
    if continuous is not None:
        lo, hi = continuous
        body += (
            f'<MidiControllerRange><Min Value="{lo}" /><Max Value="{hi}" /></MidiControllerRange>'
        )
    body += '<AutomationTarget Id="1"><LockEnvelope Value="0" /></AutomationTarget>'
    if switch is not None:
        lo, hi = switch
        body += f'<MidiCCOnOffThresholds><Min Value="{lo}" /><Max Value="{hi}" /></MidiCCOnOffThresholds>'
    return f"<{tag}>{body}</{tag}>"


VOLUME_RANGE = ("0.0003162277571", "1.99526238")
SEND_RANGE = ("0.0003162277571", "1")


def _mixer(speaker=None, volume=None, sends=(None, None), volume_range=VOLUME_RANGE):
    holders = "".join(
        f'<TrackSendHolder Id="{i}">{_param("Send", m, continuous=SEND_RANGE)}'
        '<EnabledByUser Value="true" /></TrackSendHolder>'
        for i, m in enumerate(sends)
    )
    return (
        "<Mixer>"
        + _param("Speaker", speaker, switch=(64, 127))
        + _param("Volume", volume, continuous=volume_range)
        + _param("Pan", None, continuous=(-1, 1))
        + f"<Sends>{holders}</Sends></Mixer>"
    )


def _name(name):
    name = escape(name, {'"': "&quot;"})
    return (
        f'<Name><EffectiveName Value="{name}" /><UserName Value="{name}" />'
        '<Annotation Value="" /><MemorizedFirstClipName Value="" /></Name>'
    )


def _rack(name, macro=None, branches=""):
    return (
        f'<AudioEffectGroupDevice Id="0"><LomId Value="0" /><UserName Value="{escape(name)}" />'
        + _param("On", None, switch=(64, 127))
        + _param("MacroControls.0", macro, continuous=(0, 127))
        # The rack-macro bus (channel 16): parameter-to-macro links, not MIDI.
        + _param("MacroControls.1", (16, 20), continuous=(0, 127))
        + f"<Branches>{branches}</Branches></AudioEffectGroupDevice>"
    )


def _branch(name, devices):
    return (
        f'<AudioEffectBranch Id="0"><LomId Value="0" />{_name(name)}'
        "<DeviceChain><AudioToAudioDeviceChain><Devices>"
        f"{devices}</Devices></AudioToAudioDeviceChain></DeviceChain></AudioEffectBranch>"
    )


def _track(kind, name, mixer, devices="", extra=""):
    return (
        f'<{kind} Id="0"><LomId Value="0" />{extra}{_name(name)}'
        f"<DeviceChain>{mixer}<DeviceChain><Devices>{devices}</Devices></DeviceChain>"
        f"</DeviceChain></{kind}>"
    )


def als_bytes():
    latencies = _rack("Latencies", macro=(13, 36))
    vox_chain = _rack("Vox Chain", branches=_branch("Main", latencies))
    tracks = "".join(
        [
            _track("AudioTrack", "Hand1 #", _mixer()),
            _track("AudioTrack", "Hand2 #", _mixer()),
            _track("AudioTrack", "Hand3 #", _mixer()),
            _track("AudioTrack", "Hand4 #", _mixer(speaker=(13, 30), sends=(None, (13, 31)))),
            _track("GroupTrack", "Vocals Repro grp#", _mixer()),
            _track("AudioTrack", "Vocal 1 repro#", _mixer(speaker=(13, 56)), vox_chain),
            _track("AudioTrack", "Vocal 2 repro#", _mixer(speaker=(13, 56))),
            _track("AudioTrack", "Vocal 3 repro#", _mixer(speaker=(13, 20))),
            _track("AudioTrack", "Keys 1", _mixer()),
            # A track-level mapping (not a parameter): not reproduced.
            _track("GroupTrack", "Stems grp#", _mixer(), extra=_key_midi(13, 67)),
            _track("AudioTrack", "Drums #", _mixer(volume=(13, 57))),
            _track("AudioTrack", "Bass #", _mixer(volume=(13, 57))),
            _track("AudioTrack", "Keys 1", _mixer()),
            _track(
                "AudioTrack",
                "Mics Stage #",
                _mixer(volume=(13, 58), volume_range=("0.0003162277571", "1")),
            ),
            _track("AudioTrack", "TechAlert #", _mixer()),
            _track("ReturnTrack", "A-Reverb #", _mixer(speaker=(13, 55))),
            _track("ReturnTrack", "B-Main repro #", _mixer()),
        ]
    )
    xml = (
        '<?xml version="1.0" encoding="UTF-8"?>\n'
        '<Ableton MajorVersion="5" MinorVersion="12.0_12200" SchemaChangeCount="1" '
        'Creator="Ableton Live 12.2.5" Revision="synthetic">'
        f"<LiveSet><Tracks>{tracks}</Tracks>"
        f'<MainTrack><LomId Value="0" />{_name("Main")}<DeviceChain>{_mixer()}</DeviceChain>'
        "</MainTrack></LiveSet></Ableton>"
    )
    out = io.BytesIO()
    with gzip.GzipFile(fileobj=out, mode="wb", compresslevel=9, mtime=0, filename="") as gz:
        gz.write(xml.encode("utf-8"))
    return out.getvalue()


def build(out_dir):
    """Writes ``test.tosc`` and ``test.als`` into ``out_dir``; their paths."""
    os.makedirs(out_dir, exist_ok=True)
    tosc = os.path.join(out_dir, "test.tosc")
    als = os.path.join(out_dir, "test.als")
    with open(tosc, "wb") as f:
        f.write(tosc_bytes())
    with open(als, "wb") as f:
        f.write(als_bytes())
    return tosc, als


if __name__ == "__main__":
    if len(sys.argv) != 2:
        print("usage: build_fixtures.py OUT_DIR", file=sys.stderr)
        sys.exit(2)
    for path in build(sys.argv[1]):
        print(path)
