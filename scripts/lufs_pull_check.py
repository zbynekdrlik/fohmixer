"""The FohMixer LUFS device's pull (#264), checked for `check_lufs_device.py`.

The DECIDED of 25.11.2026 (built on probe B3): nothing in the device starts a
message in Live's audio thread. The hub polls the device on a port it draws
at load; each poll, the load's last outlet and Device On turning 1 open the
gate's right inlet; the gate (its left inlet `is_active`, observed) passes
the bang to `t b b b b`, which first counts the poll for the identity's
re-read (`poll_reread_check`: every 50th poll), then bangs the three
`snapshot~ 0` (short-term first, the voiced mean last), so the reading,
the face and the send run in Max's main thread (`udpreceive @defer 1`
hands the poll to the low-priority queue). This module pins those boxes
and lines and fails on an audio clock anywhere on a reading or the face;
the wire module's constants
(the port range, each kind's poll address) come from
`crates/fohmixer-proto/src/lufs/wire.rs`. FohMixer Meter is pulled the same
way since its step of #264; its own boxes and lines are a `Shape` of
`meter_pull_check.py`, checked by the same `pull_errors`.
"""

from __future__ import annotations

import re
from dataclasses import dataclass

from check_lufs_device import (
    ACTIVE_OBSERVER,
    AUDIO_READERS,
    ID_LOAD,
    IDENTITY,
    LOAD_EXTRA,
    PACK_ARGS,
    PARENTS,
    PULL_GATE,
    TIMED,
    clock_off,
    feeds,
    kind,
    upstream,
    words,
)

# The polled form's constants (#264), read from the same module.
POLL_LINES = {
    "min": re.compile(r"^pub const POLL_PORT_MIN: u16 = ([\d_]+);$", re.MULTILINE),
    "span": re.compile(r"^pub const POLL_PORT_SPAN: u16 = ([\d_]+);$", re.MULTILINE),
    "address": re.compile(r'^pub const LUFS_POLL_ADDRESS: &str = "([^"]+)";$', re.MULTILINE),
    "meter": re.compile(r'^pub const METER_POLL_ADDRESS: &str = "([^"]+)";$', re.MULTILINE),
}


@dataclass(frozen=True)
class Poll:
    """The polled form's constants (#264): the poll ports a device draws
    from (`min`, `span` of them) and each kind's poll address."""

    min: int
    span: int
    address: str
    meter_address: str


def poll_constants(wire_text: str) -> Poll | None:
    found = {name: line.search(wire_text) for name, line in POLL_LINES.items()}
    if any(match is None for match in found.values()):
        return None
    return Poll(
        int(found["min"][1].replace("_", "")),
        int(found["span"][1].replace("_", "")),
        found["address"][1],
        found["meter"][1],
    )


@dataclass(frozen=True)
class Shape:
    """One pulled device's pull: its boxes (id: words), each guarded inlet
    and the only sources it takes, each box whose every line out is
    pinned, and its readers' words for the message."""

    boxes: dict[str, list[str]]
    wiring: dict[tuple[str, int], list[tuple[str, int]]]
    outputs: dict[str, list[list]]
    readers: str


def lufs_shape(poll: Poll, sink: str) -> Shape:
    """The LUFS device's pull."""
    return Shape(pull_boxes(poll), pull_wiring(sink), pull_outputs(), "'snapshot~ 0'")


# The pull (#264, the DECIDED of 25.11.2026, built on probe B3): the hub
# polls the device on a port it draws at load; each poll, the load's last
# outlet and Device On turning 1 open the gate's right inlet; the gate (its
# left inlet `is_active`, observed) passes the bang to `t b b b b`, which
# counts the poll for the re-read, then bangs the three `snapshot~ 0`
# (short-term first), so the reading, the face and the send run in Max's
# main thread.
PULL = "obj-pull"
POLL_RECEIVE = "obj-poll-receive"
POLL_ROUTE = "obj-poll-route"
POLL_DRAW = "obj-poll-draw"
POLL_OFFSET = "obj-poll-offset"
POLL_PORT = "obj-poll-port"
POLL_SET = "obj-poll-set"
ACTIVE_PROPERTY = "obj-active-property"
ACTIVE_PATH = "obj-active-path"
ACTIVE_LIVE_PATH = "obj-active-live-path"
ACTIVE_T = "obj-active-t"
ACTIVE_SEL = "obj-active-sel"
# The face's boxes that take no input (its captions).
FACE_KINDS = ("comment",)


def pull_boxes(poll: Poll) -> dict[str, list[str]]:
    """The pull's boxes (id: words)."""
    return {
        PULL_GATE: ["gate"],
        PULL: ["t", "b", "b", "b", "b"],
        # Bound below the range until the load draws its port: the hub
        # never polls it, so no poll reaches a device still loading.
        POLL_RECEIVE: ["udpreceive", str(poll.min - 1), "@defer", "1"],
        POLL_ROUTE: ["route", poll.address],
        POLL_DRAW: ["random", str(poll.span)],
        POLL_OFFSET: ["+", str(poll.min)],
        POLL_PORT: ["t", "i", "i"],
        POLL_SET: ["prepend", "port"],
        ACTIVE_PROPERTY: ["property", "is_active"],
        ACTIVE_PATH: IDENTITY[0].split(),
        ACTIVE_LIVE_PATH: ["live.path"],
        ACTIVE_OBSERVER: ["live.observer"],
        ACTIVE_T: ["t", "i", "i"],
        ACTIVE_SEL: ["sel", "1"],
        "obj-snap-s": ["snapshot~", "0"],
        "obj-snap-v": ["snapshot~", "0"],
        "obj-snap-g": ["snapshot~", "0"],
    }


def pull_wiring(sink: str) -> dict[tuple[str, int], list[tuple[str, int]]]:
    """Each inlet of the pull and the only sources it takes."""
    top = PARENTS + 2 + LOAD_EXTRA
    return {
        (PULL_GATE, 0): [(ACTIVE_T, 1)],
        (PULL_GATE, 1): [(POLL_ROUTE, 0), (ID_LOAD, 0), (ACTIVE_SEL, 0)],
        (PULL, 0): [(PULL_GATE, 0)],
        ("obj-snap-s", 1): [],
        ("obj-snap-v", 1): [],
        ("obj-snap-g", 1): [],
        (POLL_RECEIVE, 0): [(POLL_SET, 0)],
        (POLL_ROUTE, 0): [(POLL_RECEIVE, 0)],
        (POLL_ROUTE, 1): [],
        (POLL_DRAW, 0): [(ID_LOAD, top)],
        (POLL_DRAW, 1): [],
        (POLL_OFFSET, 0): [(POLL_DRAW, 0)],
        (POLL_OFFSET, 1): [],
        (POLL_PORT, 0): [(POLL_OFFSET, 0)],
        (POLL_SET, 0): [(POLL_PORT, 1)],
        (sink, PACK_ARGS): [(POLL_PORT, 0)],
        (ACTIVE_PROPERTY, 0): [(ID_LOAD, 2)],
        (ACTIVE_PATH, 0): [(ID_LOAD, 1)],
        (ACTIVE_LIVE_PATH, 0): [(ACTIVE_PATH, 0)],
        (ACTIVE_OBSERVER, 0): [(ACTIVE_PROPERTY, 0)],
        (ACTIVE_OBSERVER, 1): [(ACTIVE_LIVE_PATH, 0)],
        (ACTIVE_T, 0): [(ACTIVE_OBSERVER, 0)],
        (ACTIVE_SEL, 0): [(ACTIVE_T, 0)],
        (ACTIVE_SEL, 1): [],
    }


def outputs(patcher: dict, box: str) -> list[list]:
    """The lines out of `box`: [its outlet, the destination box, its inlet]."""
    return sorted(
        [line["source"][1], *line["destination"]]
        for line in (item.get("patchline", {}) for item in patcher.get("lines", []))
        if line.get("source", [None])[0] == box
    )


def pull_outputs() -> dict[str, list[list]]:
    """The pull's boxes whose every line out is pinned (the rest feed the
    readings and the pack, which their own checks pin)."""
    return {
        PULL_GATE: [[0, PULL, 0]],
        PULL: [
            [0, "obj-snap-v", 0],
            [1, "obj-snap-g", 0],
            [2, "obj-snap-s", 0],
            [3, "obj-id-count", 0],
        ],
        POLL_RECEIVE: [[0, POLL_ROUTE, 0]],
        POLL_ROUTE: [[0, PULL_GATE, 1]],
        POLL_DRAW: [[0, POLL_OFFSET, 0]],
        POLL_OFFSET: [[0, POLL_PORT, 0]],
        POLL_SET: [[0, POLL_RECEIVE, 0]],
        ACTIVE_PROPERTY: [[0, ACTIVE_OBSERVER, 0]],
        ACTIVE_PATH: [[0, ACTIVE_LIVE_PATH, 0]],
        ACTIVE_LIVE_PATH: [[0, ACTIVE_OBSERVER, 1]],
        ACTIVE_OBSERVER: [[0, ACTIVE_T, 0]],
        ACTIVE_T: [[0, ACTIVE_SEL, 0], [1, PULL_GATE, 0]],
        ACTIVE_SEL: [[0, PULL_GATE, 1]],
    }


def clocks_reaching(patcher: dict, ids: dict, starts: list[tuple[str, int | None]]) -> list[str]:
    """The clocks upstream of `starts` through any inlet: every timed box
    (an audio reader with its own clock on, a `metro`, a `delay`, ...) and
    every `qmetro`, but a reader whose clock is off (a pulled device holds
    no other clock, `poll_reread_check`)."""
    return upstream(patcher, ids, starts, lambda _: None, (*TIMED, "qmetro"), ends=())


def pull_errors(where: str, patcher: dict, every: list[dict], shape: Shape) -> list[str]:
    """The pull (#264): its boxes and lines, no audio clock anywhere on
    a reading or the face, one `udpreceive`."""
    ids = {b.get("id"): b for b in every}
    errors = [
        f"{where}: {box} must be '{' '.join(want)}', has {words(ids.get(box, {}))}"
        for box, want in shape.boxes.items()
        if words(ids.get(box, {})) != want
    ]
    for (box, inlet), sources in shape.wiring.items():
        fed = feeds(patcher, box, inlet)
        want = sorted([list(s) for s in sources])
        if fed != want:
            errors.append(f"{where}: {box}:{inlet} takes {want} alone, has {fed}")
    for box, want in shape.outputs.items():
        out = outputs(patcher, box)
        if out != want:
            errors.append(f"{where}: {box} outputs into {want} alone, has {out}")
    receives = [b.get("id") for b in every if kind(b) == "udpreceive"]
    if receives != [POLL_RECEIVE]:
        errors.append(f"{where}: needs exactly one udpreceive, {POLL_RECEIVE}, has {receives}")
    for reader in every:
        if kind(reader) in AUDIO_READERS and not clock_off(reader):
            errors.append(
                f"{where}: {reader.get('id')} ('{reader.get('text')}') runs its own clock "
                f"(the device is pulled: its readers are {shape.readers})"
            )
    for reader in every:
        if kind(reader) not in AUDIO_READERS:
            continue
        for src in clocks_reaching(patcher, ids, [(reader.get("id"), None)]):
            errors.append(
                f"{where}: {src} ('{ids[src].get('text')}') drives the reading "
                f"{reader.get('id')} (only the hub's poll, the load and the re-read do)"
            )
    for face in every:
        if face.get("presentation") != 1 or face.get("maxclass") in FACE_KINDS:
            continue
        for src in clocks_reaching(patcher, ids, [(face.get("id"), None)]):
            errors.append(
                f"{where}: the face's {face.get('id')} is fed from {src} "
                f"('{ids[src].get('text')}'), an audio clock (a redraw from the "
                "audio thread posts too)"
            )
    return errors
