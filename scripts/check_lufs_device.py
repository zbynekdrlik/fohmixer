#!/usr/bin/env python3
"""Keep the FohMixer LUFS and Meter devices' patches in shape (#151).

Spec docs/superpowers/specs/2026-10-26-lufs-design.md §3.2, §4.2, §4.4 and §9,
for each patch:

- no parameter: no `live.*` parameter object, no box with `parameter_enable`
  on, no `parameters` table (probe P1: a parameter the device writes is an
  Undo step);
- the output is silence: `plugout~` takes nothing but a `sig~ 0` (or
  `sig~ 0.`) that nothing feeds (the owner's ruling of 27.10: the devices sit
  in a rack's parallel chain, never in series, so a failing device can never
  cut the sound and the signal is never doubled);
- one `udpsend 127.0.0.1 <port>`, the port equal to the hub's default
  (`DEFAULT_PORT` in `crates/fohmixer-proto/src/lufs/wire.rs`), fed by the
  pack alone (both devices are pulled since #264, below);
- the threads (`thread_errors`, #240): no timed box reaches a network
  object's hot inlet without a `deferlow` or `qmetro` between;
- the send follows the reading (the review of PR 2: a `metro` kept sending
  a frozen value with the device's DSP off, so its key never went stale):
  the pack's bang comes only from `obj-send-trigger` (`t b f`), fed only by
  the last value's chain (`send_from`); the readers run no clock of their
  own (`snapshot~ 0`, the meter's `peakamp~` with no interval) and are
  banged by the hub's poll (#264);
- one `pack` whose first argument is the device's OSC address and whose
  version argument is `v<version.txt>`, which the face shows too, each of
  its slots from its one source (`slot_errors`: the device's id, the two
  values);
- the identity: the `live.path` messages for the device and its first four
  canonical parents, read at load, on a change and every 10 s
  (`identity_errors`, #240: `live.path`'s path-following outlet and a
  `live.observer` on the `name` of parents 1 to 3 each trigger a read, each
  parent's `route id` takes both `live.path` outlets, the left at every
  `path` message as before #240, the middle on a follow; the set is read at
  load, on its follow and by the clock; the PC proof's attempt 2 showed
  the follow misses a move into a chain, so the identity's own clock sends
  the four parents' `path` messages again every 10 s: `random 10000` drawn
  at load -> `delay` -> `deferlow` -> `1` -> `qmetro 10000` -> `t b b b b`,
  each device at its own offset, on the low-priority queue, each of its
  boxes once and its lines in and out exactly these (`clock_errors`); no
  other timed box upstream of any LOM object, a faster clock included),
  each name from `live.object` `get name` -> `route name` -> `zl reg` ->
  `tosymbol` into the pack's cold inlets, never from an
  observer's output (`value_errors`: the FINDING of 2026-11-17, an
  observer's output kept a name with a space in quotes and read the set as
  `-`, so no device matched);
- no `metro` or `qmetro` but the LUFS device's identity's `qmetro 10000`;
  the meter holds none, nor any `deferlow` (`meter_pull_check`: its load
  comes straight from `live.thisdevice`, its re-read rides the poll);
- every patch line joins two boxes of the patch within their outlets and
  inlets;
- no `gen~` (nor any other gen object or codebox) in either patch (#264:
  a `gen~` runs a clock of its own, which Max for Live services in Live's
  audio thread, and each tick posts from there);
- the LUFS device only, its DSP in plain MSP objects (`lufs_dsp_check`,
  the DECIDED of 25.11.2026): per channel two `biquad~` (the shelf, then
  the RLB high-pass) with their coefficients from `expr` boxes fed by
  `dspstate~`'s rate, squared and summed; the 3 s and 400 ms means in
  `average~`; the Voice gate `>=~`; every box and line of the chain, and
  the `expr` texts equal to the module's reference formulas;
- both devices, the pull (#264, `pull_errors` with each device's `Shape`,
  the meter's in `meter_pull_check`; the DECIDED of 25.11.2026: no
  message may start in Live's audio thread): the readers (their own
  clocks off: a `bang` reads them in the thread that sends it), banged by
  `obj-pull` (`t b b b`; the meter's counts the re-read first) behind
  `obj-pull-gate` (`gate`, opened by a `live.observer` on `this_device`
  `is_active`: a device switched off answers nothing), which only the hub's
  poll (`udpreceive` -> `route <kind's POLL_ADDRESS>`), the load's last
  outlet, the LUFS device's 10 s re-read's last outlet and the observer
  turning 1 open; the port
  drawn once at load (`random <POLL_PORT_SPAN>` -> `+ <POLL_PORT_MIN>`) into
  the `udpreceive` and the pack's last slot; the pack straight into the
  `udpsend` (no `deferlow` on the send path); no `snapshot~` or `peakamp~`
  with its own clock on, no clock but the identity's driving a reading, and
  no face object fed from an audio clock (probe C of 25.11: number boxes
  fed from the scheduler post from the audio thread too);
- no absolute file path.

Exit 0 when all hold, else 1 with one `::error::` line per problem.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from lufs_pull_check import Poll

ROOT = Path(__file__).resolve().parent.parent
WIRE = Path("crates") / "fohmixer-proto" / "src" / "lufs" / "wire.rs"
PORT_LINE = re.compile(r"^pub const DEFAULT_PORT: u16 = (\d+);$", re.MULTILINE)
# A drive letter path (raw or JSON-escaped), a UNC path, a Unix home.
ABSOLUTE_PATH = re.compile(r"\b[A-Za-z]:(?:\\\\|\\|/)|\\\\\\\\|/(?:Users|home)/")
PARENTS = 4
IDENTITY = tuple("path this_device" + " canonical_parent" * level for level in range(PARENTS + 1))
# The pack's arguments: address, wire version, id, version, 4 parents, 2
# values; the polled form (#264) adds the poll port.
PACK_ARGS = 10
PACK_WIRE = 2
PACK_VERSION = 3
WIRE_POLLED = "2"
# The trigger that bangs the pack (one datagram per reading) and hands the
# last value on first.
SEND_TRIGGER = "obj-send-trigger"
SEND_TRIGGER_WORDS = ["t", "b", "f"]


@dataclass(frozen=True)
class Device:
    """One device's source files and what its patch must say."""

    name: str
    folder: Path
    maxpat: Path
    address: str
    # Its readers' interval argument (None: none at all).
    send_ms: int | None
    clock: str
    send_from: str
    # The hub polls it and it answers in Max's main thread (#264).
    pulled: bool = True
    # Its K-weighted chain in plain MSP objects (#264, `lufs_dsp_check`).
    kweighted: bool = False
    # Its re-read rides the poll, its load is not deferred, and it holds no
    # clock and no deferlow (#264, the meter: `meter_pull_check`).
    on_poll: bool = False

    @property
    def version_txt(self) -> Path:
        return self.folder / "version.txt"


LUFS_FOLDER = Path("live-device") / "FohMixer-LUFS"
METER_FOLDER = Path("live-device") / "FohMixer-Meter"
DEVICES = (
    Device(
        "FohMixer LUFS",
        LUFS_FOLDER,
        LUFS_FOLDER / "FohMixer LUFS.maxpat",
        "/fohmixer/lufs",
        0,
        "snapshot~",
        "obj-clip-v",
        kweighted=True,
    ),
    Device(
        "FohMixer Meter",
        METER_FOLDER,
        METER_FOLDER / "FohMixer Meter.maxpat",
        "/fohmixer/meter",
        None,
        "peakamp~",
        "obj-clip-r",
        on_poll=True,
    ),
)


def boxes(patcher: dict) -> list[dict]:
    """Every box of a patcher and of the patchers inside it."""
    found = []
    for item in patcher.get("boxes", []):
        box = item.get("box", {})
        found.append(box)
        if isinstance(box.get("patcher"), dict):
            found += boxes(box["patcher"])
    return found


def words(box: dict) -> list[str]:
    return str(box.get("text", "")).split()


def default_port(wire_text: str) -> int | None:
    match = PORT_LINE.search(wire_text)
    return int(match[1]) if match else None


def is_parameter(box: dict) -> bool:
    """A box with `parameter_enable` on, or a `live.*` UI object (a parameter by kind)."""
    return box.get("parameter_enable") == 1 or str(box.get("maxclass", "")).startswith("live.")


def line_errors(where: str, patcher: dict) -> list[str]:
    """Lines of this patcher (not its subpatchers) within its boxes' ports."""
    own = {item.get("box", {}).get("id"): item.get("box", {}) for item in patcher["boxes"]}
    errors = []
    for item in patcher.get("lines", []):
        line = item.get("patchline", {})
        (src, out), (dst, inlet) = line.get("source"), line.get("destination")
        if src not in own or dst not in own:
            errors.append(f"{where}: a line joins a box that is not there: {src} -> {dst}")
        elif out >= own[src].get("numoutlets", 0) or inlet >= own[dst].get("numinlets", 0):
            errors.append(f"{where}: the line {src}:{out} -> {dst}:{inlet} is past their ports")
    return errors


SILENCE = (["sig~", "0"], ["sig~", "0."])


def silence_errors(where: str, patcher: dict, every: list[dict]) -> list[str]:
    """`plugout~` fed by nothing but a `sig~ 0` that nothing feeds."""
    ids = {b.get("id"): words(b) for b in every}
    plugin = [i for i, w in ids.items() if w[:1] == ["plugin~"]]
    plugout = [i for i, w in ids.items() if w[:1] == ["plugout~"]]
    if len(plugin) != 1 or len(plugout) != 1:
        return [f"{where}: needs exactly one plugin~ and one plugout~"]
    lines = [line.get("patchline", {}) for line in patcher.get("lines", [])]
    sources = {line["source"][0] for line in lines if line["destination"][0] == plugout[0]}
    loud = sorted(str(s) for s in sources if ids.get(s) not in SILENCE)
    fed = sorted(
        str(s)
        for s in sources
        if ids.get(s) in SILENCE and any(ln["destination"][0] == s for ln in lines)
    )
    if loud or fed:
        return [
            f"{where}: plugout~ must take silence only (a sig~ 0 nothing feeds; the device "
            f"sits in a parallel chain), it takes {loud + fed}"
        ]
    return []


def patch_errors(
    where: str,
    maxpat_text: str,
    version: str,
    address: str,
    port: int,
    send_ms: int | None,
    clock: str,
    send_from: str,
    poll: Poll,
    kweighted: bool = False,
    on_poll: bool = False,
) -> list[str]:
    """The problems of one device's patch (`poll`: the polled form's
    constants; `kweighted`: its K-weighted chain is checked,
    `lufs_dsp_check`; `on_poll`: the meter's shape, `meter_pull_check`)."""
    errors: list[str] = []
    if ABSOLUTE_PATH.search(maxpat_text):
        errors.append(f"{where} holds an absolute file path")
    try:
        patcher = json.loads(maxpat_text)["patcher"]
        every = boxes(patcher)
    except (ValueError, KeyError, TypeError, AttributeError) as error:
        return errors + [f"{where} is not a Max patcher: {error}"]
    if "parameters" in patcher:
        errors.append(f"{where}: a parameters table (the device has no parameters)")
    errors += [
        f"{where}: {b.get('id')} is a parameter (the device has none)"
        for b in every
        if is_parameter(b)
    ]
    errors += line_errors(where, patcher)
    errors += silence_errors(where, patcher, every)
    import lufs_dsp_check  # noqa: PLC0415 (it imports this module)

    errors += lufs_dsp_check.gen_errors(where, every)
    if kweighted:
        errors += lufs_dsp_check.dsp_errors(where, patcher, every)
    sends = [words(b) for b in every if words(b)[:1] == ["udpsend"]]
    if sends != [["udpsend", "127.0.0.1", str(port)]]:
        errors.append(f"{where}: needs exactly one 'udpsend 127.0.0.1 {port}', has {sends}")
    errors += send_errors(where, patcher, every, send_ms, clock, send_from)
    packs = [words(b) for b in every if is_send_pack(b)]
    args = PACK_ARGS + 1
    if len(packs) != 1 or len(packs[0]) != args + 1:
        errors.append(f"{where}: needs exactly one pack of {args} arguments, has {packs}")
    else:
        if packs[0][PACK_WIRE] != WIRE_POLLED:
            errors.append(
                f"{where}: the pack sends wire version {packs[0][PACK_WIRE]}, "
                f"expected {WIRE_POLLED}"
            )
        if packs[0][1] != address:
            errors.append(f"{where}: the pack sends to {packs[0][1]}, expected {address}")
        if packs[0][1 + PACK_VERSION] != f"v{version}":
            errors.append(
                f"{where}: the pack sends {packs[0][1 + PACK_VERSION]}, expected v{version}"
            )
    messages = {str(b.get("text", "")) for b in every if b.get("maxclass") == "message"}
    errors += [f"{where}: no '{path}' message" for path in IDENTITY if path not in messages]
    if len(packs) == 1:
        sink = next(b.get("id") for b in every if is_send_pack(b))
        errors += identity_errors(where, patcher, every, sink, True, on_poll)
        errors += slot_errors(where, patcher, every, sink, send_from)
        import lufs_pull_check  # noqa: PLC0415 (it imports this module)

        shape = (meter().meter_shape if on_poll else lufs_pull_check.lufs_shape)(poll, sink)
        errors += lufs_pull_check.pull_errors(where, patcher, every, shape)
    errors += [
        f"{where}: {b.get('id')} ('{b.get('text')}') runs on a clock (the device has none "
        "but the identity's)"
        for b in every
        if kind(b) in ("metro", "qmetro") and not is_identity_clock(b)
    ]
    errors += meter().still_errors(where, every) if on_poll else []
    shown = [b.get("text") for b in every if b.get("varname") == "version"]
    if shown != [f"v{version}"]:
        errors.append(f"{where}: its face shows {shown}, expected ['v{version}'] (version.txt)")
    return errors


def slot_errors(
    where: str, patcher: dict, every: list[dict], pack: str, send_from: str
) -> list[str]:
    """The pack's other slots, each from its one source: the device's id
    (inlet 2) from `obj-id-route`, the first value (inlet 8) from its own
    `clip` (never the last value's), the last value (inlet 9) from the
    trigger, which bangs the pack after it."""
    errors = []
    for inlet, want in ((2, [["obj-id-route", 0]]), (9, [[SEND_TRIGGER, 1]])):
        if feeds(patcher, pack, inlet) != want:
            errors.append(
                f"{where}: {pack}:{inlet} takes {want} alone, has {feeds(patcher, pack, inlet)}"
            )
    ids = {b.get("id"): b for b in every}
    first = feeds(patcher, pack, 8)
    if len(first) != 1 or first[0][0] == send_from or kind(ids.get(first[0][0], {})) != "clip":
        errors.append(f"{where}: {pack}:8 takes the first value's clip alone, has {first}")
    return errors


def is_send_pack(box: dict) -> bool:
    """The datagram's `pack` (its address and identity among its
    arguments); a `pack` of numbers alone builds a list inside the patch
    (the LUFS device's filter coefficients, #264)."""
    found = words(box)
    if box.get("maxclass") == "message" or found[:1] != ["pack"]:
        return False
    return not found[1:] or not all(NUMBER.fullmatch(word) for word in found[1:])


NUMBER = re.compile(r"-?(\d+\.?\d*|\.\d+)")


def feeds(patcher: dict, box: str, inlet: int) -> list[list]:
    """The sources (box id, outlet) of the lines into `box`'s `inlet`."""
    return sorted(
        list(item.get("patchline", {}).get("source", []))
        for item in patcher.get("lines", [])
        if item.get("patchline", {}).get("destination") == [box, inlet]
    )


def send_errors(
    where: str,
    patcher: dict,
    every: list[dict],
    send_ms: int | None,
    clock: str,
    send_from: str,
) -> list[str]:
    """The send follows the reading: the pack banged only by the trigger,
    the trigger fed only by `send_from`, every `clock` box with the
    interval `send_ms` (None: no argument)."""
    errors = []
    clocks = [words(b) for b in every if words(b)[:1] == [clock]]
    want = [clock] if send_ms is None else [clock, str(send_ms)]
    if not clocks or any(c != want for c in clocks):
        errors.append(f"{where}: every {clock} must be '{' '.join(want)}', has {clocks}")
    triggers = [words(b) for b in every if b.get("id") == SEND_TRIGGER]
    if triggers != [SEND_TRIGGER_WORDS]:
        errors.append(f"{where}: {SEND_TRIGGER} must be 't b f', has {triggers}")
    packs = [b.get("id") for b in every if is_send_pack(b)]
    if len(packs) == 1 and feeds(patcher, packs[0], 0) != [[SEND_TRIGGER, 0]]:
        errors.append(
            f"{where}: the pack must be banged by {SEND_TRIGGER} alone (no metro), "
            f"has {feeds(patcher, packs[0], 0)}"
        )
    if feeds(patcher, SEND_TRIGGER, 0) != [[send_from, 0]]:
        errors.append(
            f"{where}: {SEND_TRIGGER} must be fed by {send_from} alone, "
            f"has {feeds(patcher, SEND_TRIGGER, 0)}"
        )
    errors += direct_errors(where, patcher, every)
    errors += thread_errors(where, patcher, every)
    return errors


def direct_errors(where: str, patcher: dict, every: list[dict]) -> list[str]:
    """A pulled device's send (#264): the pack straight into the udpsend,
    and no `deferlow` (or `defer`) anywhere on the send path, from the
    pull's gate down to the socket: in Live each such hand-over is a
    `PostMessageW` from the thread that runs it, and the pull already runs
    in Max's main thread."""
    errors = []
    packs = [b.get("id") for b in every if is_send_pack(b)]
    sends = [b.get("id") for b in every if words(b)[:1] == ["udpsend"]]
    if len(packs) == 1 and len(sends) == 1 and feeds(patcher, sends[0], 0) != [[packs[0], 0]]:
        errors.append(
            f"{where}: the udpsend must be fed by the pack alone (no deferlow), "
            f"has {feeds(patcher, sends[0], 0)}"
        )
    ids = {b.get("id"): b for b in every}
    for send in sends:
        for src in upstream(
            patcher, ids, [(send, 0)], hot_inlets, ("deferlow", "defer"), ends=("gate",)
        ):
            errors.append(
                f"{where}: {src} ('{ids[src].get('text')}') is on the send path "
                "(the pull runs in the main thread: no deferlow)"
            )
    return errors


# The threads (#240). Max for Live runs Max's scheduler inside Live's audio
# thread, so a timed object's output (and everything it triggers) runs
# there; `deferlow` and `qmetro` hand it to the low-priority queue (Live's
# main thread). A network object is reached from a timed one only through
# one of them.
NETWORK = {"udpsend": (0,), "maxurl": (0,), "jit.net.send": (0, 1)}
LOW_PRIORITY = ("deferlow", "defer", "qmetro")
# Boxes that output from the scheduler: clocks, the audio side's readers,
# the transport, and MIDI in (Max for Live hands it over in the audio
# thread).
TIMED = (
    "metro",
    "delay",
    "del",
    "pipe",
    "snapshot~",
    "peakamp~",
    "edge~",
    "plugsync~",
    "transport",
    "timepoint",
    "clocker",
    "tempo",
    "line",
    "speedlim",
    "makenote",
    "midiin",
    "notein",
    "ctlin",
)
# Hot inlets: the ones whose message makes the box output. The classes
# below take their other inlets cold (they store); `gate` passes its right
# inlet's message (its left is the control); `jit.net.send` sends both. A
# class not named here is taken as hot on every inlet (the safe side).
LEFT_HOT = (
    "pack",
    "zl",
    "prepend",
    "dict.pack",
    "expr",
    "sprintf",
    "delay",
    "del",
    "pipe",
    "sel",
    "select",
    "route",
    "random",
    "clip",
    "*",
    "+",
    "-",
    "/",
    "%",
    "round",
    "metro",
    "qmetro",
    "change",
    "live.object",
    "makenote",
    "maxurl",
    "udpsend",
)
HOT = {"gate": (1,), "jit.net.send": (0, 1)}
# The audio side's readers: with an interval of 0 their own clock is off,
# and they output only when banged, in the banging thread (#264).
AUDIO_READERS = ("snapshot~", "peakamp~")


def clock_off(box: dict) -> bool:
    """An audio reader whose own clock is off (`snapshot~ 0`, and
    `peakamp~` with no interval, #264: it outputs only when banged): no
    timed box."""
    rest = words(box)[1:]
    return kind(box) in AUDIO_READERS and (
        rest == ["0"] or (kind(box) == "peakamp~" and rest == [])
    )


def kind(box: dict) -> str:
    """An object box's class (its first word); a message box has none."""
    if box.get("maxclass") == "message":
        return ""
    found = words(box)
    return found[0] if found else ""


def hot_inlets(box: dict) -> tuple[int, ...] | None:
    """The inlets whose message makes `box` output; None for all of them."""
    if box.get("maxclass") == "message" or kind(box) in LEFT_HOT:
        return (0,)
    return HOT.get(kind(box))


def is_signal(box: dict, outlet: int) -> bool:
    """An outlet that carries audio: its `outlettype` says `signal` (an
    outlet with no declared type is walked, the safe side)."""
    types = box.get("outlettype")
    if isinstance(types, list) and outlet < len(types):
        return types[outlet] in ("signal", "multichannelsignal")
    return False


def upstream(
    patcher: dict,
    ids: dict,
    starts: list[tuple[str, int | None]],
    inlets,
    stop: tuple,
    ends: tuple = LOW_PRIORITY,
) -> list[str]:
    """The boxes of a class in `stop` that reach `starts` ((box, inlet), an
    inlet of None for every inlet), walking upstream through the inlets
    `inlets(box)` names (None: every inlet). A `stop` box is reported and
    not crossed; an `ends` box (by default the low-priority ones) ends its
    path unreported."""
    reached: list[str] = []
    seen: set[str] = set()
    todo = list(starts)
    while todo:
        box, inlet = todo.pop()
        for item in patcher.get("lines", []):
            line = item.get("patchline", {})
            dst = line.get("destination", [None, None])
            if dst[0] != box or (inlet is not None and dst[1] != inlet):
                continue
            src = line["source"][0]
            source = ids.get(src, {})
            if is_signal(source, line["source"][1]) and kind(source) not in stop:
                # An audio line carries no message: nothing upstream of it
                # runs in the thread of what it feeds (#264: the filters'
                # coefficients are set at DSP start and at load). A `stop`
                # box is still reported, whatever its saved outlet type.
                continue
            if src in seen:
                continue
            seen.add(src)
            if kind(source) in stop and not clock_off(source):
                reached.append(src)
                continue
            if kind(source) in ends:
                continue
            hot = inlets(source)
            todo += [(src, None)] if hot is None else [(src, i) for i in hot]
    return sorted(reached)


def thread_errors(where: str, patcher: dict, every: list[dict]) -> list[str]:
    """Every timed box whose output reaches a network object through hot
    inlets with no `deferlow` or `qmetro` between (a cold inlet only
    stores: what a timed box stores there is sent by another thread)."""
    ids = {b.get("id"): b for b in every}
    errors = []
    for net in every:
        if kind(net) not in NETWORK:
            continue
        starts = [(net.get("id"), inlet) for inlet in NETWORK[kind(net)]]
        for src in upstream(patcher, ids, starts, hot_inlets, TIMED):
            errors.append(
                f"{where}: {src} ('{ids[src].get('text')}') reaches the "
                f"{kind(net)} {net.get('id')} on the audio thread "
                "(a deferlow or a qmetro goes between)"
            )
    return errors


# The identity (#240): read at load, on a change and every 10 s. The
# load: `live.thisdevice` -> `deferlow` -> `obj-id-load` (`t b b b b b b`:
# the observers' property first, then the device's path, then the four
# parents' paths). Each parent: its path into a `live.path` whose left
# outlet (the id at every `path` message) and middle outlet (it follows the
# path: a new id when another object comes there) go through `route id`
# into a `t`: the id first into the `live.object`'s
# right inlet (`prepend id`), then (parents 1 to 3) into the
# `live.observer`'s (`id 0` stops observing), then into `sel 0`. An id
# reads the name (`t b b b`: `-` into the `zl reg`, `get name` into the
# `live.object`, whose `route name` overwrites it, then the `zl reg`
# banged); 0 stores and sends `-` (`t b b`). The observer's output (the
# name at its id and at each rename) only bangs the same read (`t b`):
# never a value. The values take the path the PC proved before #240,
# `live.object` -> `route name` -> `zl reg` -> `tosymbol` into the sink's
# cold inlet (4 + the parent's level - 1), which keeps it for every send:
# an observer's output kept a name with a space in literal quotes (the
# FINDING of 2026-11-17). The set (parent 4) read `-` there too, and the
# pack's own default is `-`, so either its observer reported nothing or
# its `live.path`'s middle outlet never gave an id: the set has no
# observer. Every parent takes its id from both of its `live.path`'s
# outlets into its `route id`: the left answers every `path` message (the
# load's and the 10 s re-read's), as the devices before #240 read it every
# second for months on the PC; the middle answers the follow. Nothing else
# takes either outlet.
#
# The follow misses a move into another chain (the PC proof's attempt 2 on
# #240: a moved device kept its old parents for over 30 s), so the identity
# has one clock of its own (the DECIDED of 2026-11-17): `live.thisdevice`
# -> `random 10000` (an offset drawn once at load, so devices loaded
# together read at different instants) -> `delay` (its left inlet takes the
# offset as its time) -> `deferlow` -> `1` -> `qmetro 10000` (the
# low-priority queue) -> `t b b b b`, which sends the four parents' `path`
# messages again in the load's order: each `live.path` resolves its path
# anew, its left outlet gives the id, and the same read follows. It is the
# only clock allowed upstream of a LOM object, at exactly 10 s.
ID_LOAD = "obj-id-load"
ID_DEFER = "obj-id-defer"
ID_PROPERTY = "obj-id-property"
FOLLOWS_PATH = 1
FOLLOWS_OBJECT = 0
LOM = ("live.path", "live.object", "live.observer")
# The parents whose `name` a `live.observer` reports; the set is the last.
OBSERVED = range(1, PARENTS)
# The identity's clock (#240): the re-read's period, the offset's range.
REREAD_MS = 10000
ID_OFFSET = "obj-id-offset"
ID_OFFSET_DELAY = "obj-id-offset-delay"
ID_CLOCK_DEFER = "obj-id-clock-defer"
ID_CLOCK_START = "obj-id-clock-start"
ID_CLOCK = "obj-id-clock"
ID_CLOCK_WORDS = ["qmetro", str(REREAD_MS)]
ID_TICK = "obj-id-tick"


def is_identity_clock(box: dict) -> bool:
    """The identity's own clock: `obj-id-clock`, exactly `qmetro 10000`."""
    return box.get("id") == ID_CLOCK and words(box) == ID_CLOCK_WORDS


# A pulled device's load and 10 s tick have more outlets (#264): the load
# draws the poll port first (its rightmost outlet) and, after the parents,
# sets the `is_active` observer's property and path, then sends one
# datagram (its leftmost); the tick sends one after the parents.
LOAD_EXTRA = 3
TICK_EXTRA = 1


def load_offset(pulled: bool) -> int:
    return LOAD_EXTRA if pulled else 0


def tick_offset(pulled: bool) -> int:
    return TICK_EXTRA if pulled else 0


def meter():
    """`meter_pull_check` (it imports this module): the meter's load and
    re-read on the poll path (#264), in place of the deferred load and the
    10 s clock."""
    import meter_pull_check  # noqa: PLC0415

    return meter_pull_check


def identity_boxes(pulled: bool = False, on_poll: bool = False) -> dict[str, list[str]]:
    """The identity's boxes (id: words)."""
    if on_poll:
        return meter().identity_boxes(identity_boxes(pulled))
    found = {
        ID_DEFER: ["deferlow"],
        ID_LOAD: ["t", *["b"] * (PARENTS + 2 + load_offset(pulled) + int(pulled))],
        ID_PROPERTY: ["property", "name"],
        "obj-id-path": IDENTITY[0].split(),
        "obj-id-live-path": ["live.path"],
        "obj-id-route": ["route", "id"],
        ID_OFFSET: ["random", str(REREAD_MS)],
        ID_OFFSET_DELAY: ["delay"],
        ID_CLOCK_DEFER: ["deferlow"],
        ID_CLOCK_START: ["1"],
        ID_CLOCK: ID_CLOCK_WORDS,
        ID_TICK: ["t", *["b"] * (PARENTS + tick_offset(pulled))],
    }
    for level in range(1, PARENTS + 1):
        name = f"obj-parent{level}"
        observed = level in OBSERVED
        found |= {
            f"{name}-path": IDENTITY[level].split(),
            f"{name}-live-path": ["live.path"],
            f"{name}-route-id": ["route", "id"],
            f"{name}-t": ["t", "i", "i", "i"] if observed else ["t", "i", "i"],
            f"{name}-prepend": ["prepend", "id"],
            f"{name}-sel": ["sel", "0"],
            f"{name}-empty": ["t", "b", "b"],
            f"{name}-read": ["t", "b", "b", "b"],
            f"{name}-none": ["-"],
            f"{name}-get": ["get", "name"],
            f"{name}-object": ["live.object"],
            f"{name}-route-name": ["route", "name"],
            f"{name}-reg": ["zl", "reg"],
            f"{name}-symbol": ["tosymbol"],
        }
        if observed:
            found |= {
                f"{name}-observe": ["prepend", "id"],
                f"{name}-observer": ["live.observer"],
                f"{name}-changed": ["t", "b"],
            }
    return found


def identity_wiring(
    sink: str, pulled: bool = False, on_poll: bool = False
) -> dict[tuple[str, int], list[tuple[str, int]]]:
    """Each guarded inlet of the identity and the only sources it takes."""
    if on_poll:
        return meter().identity_wiring(identity_wiring(sink, pulled), load_offset(pulled))
    load, tick = load_offset(pulled), tick_offset(pulled)
    found: dict[tuple[str, int], list[tuple[str, int]]] = {
        (ID_DEFER, 0): [("obj-thisdevice", 0)],
        (ID_LOAD, 0): [(ID_DEFER, 0)],
        (ID_PROPERTY, 0): [(ID_LOAD, load + PARENTS + 1)],
        ("obj-id-path", 0): [(ID_LOAD, load + PARENTS)],
        ("obj-id-live-path", 0): [("obj-id-path", 0)],
        ("obj-id-route", 0): [("obj-id-live-path", 0)],
        (ID_OFFSET, 0): [("obj-thisdevice", 0)],
        (ID_OFFSET, 1): [],
        (ID_OFFSET_DELAY, 0): [(ID_OFFSET, 0)],
        (ID_OFFSET_DELAY, 1): [],
        (ID_CLOCK_DEFER, 0): [(ID_OFFSET_DELAY, 0)],
        (ID_CLOCK_START, 0): [(ID_CLOCK_DEFER, 0)],
        (ID_CLOCK_START, 1): [],
        (ID_CLOCK, 0): [(ID_CLOCK_START, 0)],
        (ID_CLOCK, 1): [],
        (ID_TICK, 0): [(ID_CLOCK, 0)],
    }
    for level in range(1, PARENTS + 1):
        name = f"obj-parent{level}"
        observed = level in OBSERVED
        reads = [(f"{name}-sel", 1)] + ([(f"{name}-changed", 0)] if observed else [])
        found |= {
            (f"{name}-path", 0): [
                (ID_LOAD, load + PARENTS - level),
                (ID_TICK, tick + PARENTS - level),
            ],
            (f"{name}-live-path", 0): [(f"{name}-path", 0)],
            (f"{name}-route-id", 0): [
                (f"{name}-live-path", FOLLOWS_OBJECT),
                (f"{name}-live-path", FOLLOWS_PATH),
            ],
            (f"{name}-t", 0): [(f"{name}-route-id", 0)],
            (f"{name}-prepend", 0): [(f"{name}-t", 2 if observed else 1)],
            (f"{name}-object", 1): [(f"{name}-prepend", 0)],
            (f"{name}-sel", 0): [(f"{name}-t", 0)],
            (f"{name}-empty", 0): [(f"{name}-sel", 0)],
            (f"{name}-read", 0): reads,
            (f"{name}-none", 0): [(f"{name}-read", 2), (f"{name}-empty", 1)],
            (f"{name}-get", 0): [(f"{name}-read", 1)],
            (f"{name}-object", 0): [(f"{name}-get", 0)],
            (f"{name}-route-name", 0): [(f"{name}-object", 0)],
            (f"{name}-reg", 0): [(f"{name}-read", 0), (f"{name}-empty", 0)],
            (f"{name}-reg", 1): [(f"{name}-route-name", 0), (f"{name}-none", 0)],
            (f"{name}-symbol", 0): [(f"{name}-reg", 0)],
            (sink, 3 + level): [(f"{name}-symbol", 0)],
        }
        if observed:
            found |= {
                (f"{name}-observe", 0): [(f"{name}-t", 1)],
                (f"{name}-observer", 0): [(ID_PROPERTY, 0)],
                (f"{name}-observer", 1): [(f"{name}-observe", 0)],
                (f"{name}-changed", 0): [(f"{name}-observer", 0)],
            }
    return found


def identity_errors(
    where: str,
    patcher: dict,
    every: list[dict],
    sink: str,
    pulled: bool = False,
    on_poll: bool = False,
) -> list[str]:
    """The identity read at load, on a change and by its own 10 s clock,
    never by another clock, its values from `live.object`."""
    ids = {b.get("id"): b for b in every}
    errors = [
        f"{where}: {box} must be '{' '.join(want)}', has {words(ids.get(box, {}))}"
        for box, want in identity_boxes(pulled, on_poll).items()
        if words(ids.get(box, {})) != want
    ]
    for (box, inlet), sources in identity_wiring(sink, pulled, on_poll).items():
        fed = feeds(patcher, box, inlet)
        want = sorted([list(s) for s in sources])
        if fed != want:
            errors.append(f"{where}: {box}:{inlet} takes {want} alone, has {fed}")
    for level in range(1, PARENTS + 1):
        box = f"obj-parent{level}-live-path"
        for outlet in (FOLLOWS_OBJECT, FOLLOWS_PATH):
            out = sorted(
                item["patchline"]["destination"]
                for item in patcher.get("lines", [])
                if item.get("patchline", {}).get("source") == [box, outlet]
            )
            if out != [[f"obj-parent{level}-route-id", 0]]:
                errors.append(
                    f"{where}: {box}'s outlet {outlet} goes into its route id alone, has {out}"
                )
    errors += [
        f"{where}: the set (parent {PARENTS}) has no observer (it reads at load and follow), "
        f"has {b.get('id')}"
        for b in every
        if b.get("id") == f"obj-parent{PARENTS}-observer"
    ]
    errors += value_errors(where, patcher, every, sink, pulled)
    errors += clock_errors(where, patcher, every, pulled, on_poll)
    errors += lom_clock_errors(where, patcher, every)
    return errors


def clock_outputs(pulled: bool = False, on_poll: bool = False) -> dict[str, list[list]]:
    """Each box of the identity's clock and the only lines out of it
    ([its outlet, the destination box, its inlet]); a pulled device's tick
    also opens the pull once (its leftmost outlet, after the parents); the
    meter's re-read on the poll path (`meter_pull_check.reread_outputs`)."""
    if on_poll:
        return meter().reread_outputs()
    chain = [ID_OFFSET, ID_OFFSET_DELAY, ID_CLOCK_DEFER, ID_CLOCK_START, ID_CLOCK, ID_TICK]
    found = {chain[i]: [[0, chain[i + 1], 0]] for i in range(len(chain) - 1)}
    tick = tick_offset(pulled)
    found[ID_TICK] = sorted(
        [[tick + PARENTS - level, f"obj-parent{level}-path", 0] for level in range(1, PARENTS + 1)]
        + ([[0, PULL_GATE, 1]] if pulled else [])
    )
    return found


def clock_errors(
    where: str, patcher: dict, every: list[dict], pulled: bool = False, on_poll: bool = False
) -> list[str]:
    """The identity's clock is exempt from `lom_clock_errors`, so it may
    drive nothing but the parents' paths (the review of the 10 s re-read:
    a LOM read fed from the clock or the tick passed every other check):
    each of its boxes once, the lines out of each exactly
    `clock_outputs`."""
    errors = []
    for box, want in clock_outputs(pulled, on_poll).items():
        count = sum(1 for b in every if b.get("id") == box)
        if count != 1:
            errors.append(f"{where}: {box} must be one box, has {count}")
        out = sorted(
            [line["source"][1], *line["destination"]]
            for line in (item.get("patchline", {}) for item in patcher.get("lines", []))
            if line.get("source", [None])[0] == box
        )
        if out != want:
            errors.append(f"{where}: {box} outputs into {want} alone, has {out}")
    return errors


def value_inlets(box: dict) -> tuple[int, ...] | None:
    """The inlets through which a box's input can reach its output as a
    value: none for a `t` of bangs alone; a message box's right inlet (it
    sets the text the next bang sends) and its left one only when its text
    holds a `$` argument; every inlet of any other box."""
    found = words(box)
    if box.get("maxclass") == "message":
        return (0, 1) if "$" in str(box.get("text", "")) else (1,)
    if found[:1] in (["t"], ["trigger"]) and len(found) > 1 and set(found[1:]) == {"b"}:
        return ()
    return None


def value_errors(
    where: str, patcher: dict, every: list[dict], sink: str, pulled: bool = False
) -> list[str]:
    """Each parent slot's value comes from `live.object` `get name` ->
    `route name` -> `zl reg` -> `tosymbol` (the path the PC proved), never
    from an observer's output: no `live.observer` reaches a slot through
    a value inlet (`value_inlets`: a `t b` or a plain message's bang ends
    the walk), and every observer outputs into its `t b` alone."""
    ids = {b.get("id"): b for b in every}
    errors = []
    for level in range(1, PARENTS + 1):
        name = f"obj-parent{level}"
        chain = [f"{name}-object", f"{name}-route-name", f"{name}-reg", f"{name}-symbol"]
        found = [kind(ids.get(box, {})) for box in chain]
        if found != ["live.object", "route", "zl", "tosymbol"] or feeds(
            patcher, sink, 3 + level
        ) != [[chain[-1], 0]]:
            errors.append(
                f"{where}: {sink}:{3 + level} takes its value from live.object -> route name "
                f"-> zl reg -> tosymbol ({' -> '.join(chain)}), has {found}"
            )
        observers = upstream(
            patcher, ids, [(sink, 3 + level)], value_inlets, ("live.observer",), ends=()
        )
        errors += [
            f"{where}: the observer {src} reaches {sink}:{3 + level} by its "
            "output (an observer only triggers the read)"
            for src in observers
        ]
    for observer in every:
        if kind(observer) != "live.observer":
            continue
        if pulled and observer.get("id") == ACTIVE_OBSERVER:
            # Device On's observer: its one line is pinned by `pull_wiring`.
            continue
        out = sorted(
            item["patchline"]["destination"]
            for item in patcher.get("lines", [])
            if item.get("patchline", {}).get("source", [None])[0] == observer.get("id")
        )
        targets = [ids.get(dst[0], {}) for dst in out]
        if len(out) != 1 or out[0][1] != 0 or words(targets[0]) != ["t", "b"]:
            errors.append(
                f"{where}: the observer {observer.get('id')} outputs into one 't b' alone "
                f"(a trigger of the read), has {out}"
            )
    return errors


def lom_clock_errors(where: str, patcher: dict, every: list[dict]) -> list[str]:
    """A timed box anywhere upstream of a LOM object (any inlet; a
    `deferlow` changes the thread, not the rate): each would call Live
    again and again. The identity's own clock (`is_identity_clock`, its
    boxes and lines pinned by `identity_wiring`) is the one exception; any
    other, a faster one included, is named."""
    ids = {b.get("id"): b for b in every}
    clocks = (*TIMED, "qmetro")
    errors = []
    for lom in every:
        if kind(lom) not in LOM:
            continue
        for src in upstream(patcher, ids, [(lom.get("id"), None)], lambda _: None, clocks, ends=()):
            if is_identity_clock(ids[src]):
                continue
            errors.append(
                f"{where}: {src} ('{ids[src].get('text')}') drives the "
                f"{kind(lom)} {lom.get('id')} (the identity is read at "
                "load, on a change and by its own 10 s clock only)"
            )
    return errors


# The pull's two boxes the identity's checks name (#264): the gate the 10 s
# tick opens, and Device On's observer (its line pinned by the pull's own
# check, `lufs_pull_check.pull_wiring`).
PULL_GATE = "obj-pull-gate"
ACTIVE_OBSERVER = "obj-active-observer"


def device_errors(root: Path, device: Device, port: int, poll: Poll | None = None) -> list[str]:
    version = (root / device.version_txt).read_text(encoding="utf-8").strip()
    text = (root / device.maxpat).read_text(encoding="utf-8")
    return patch_errors(
        device.maxpat.as_posix(),
        text,
        version,
        device.address,
        port,
        device.send_ms,
        device.clock,
        device.send_from,
        poll,
        device.kweighted,
        device.on_poll,
    )


def check(root: Path) -> list[str]:
    wire_text = (root / WIRE).read_text(encoding="utf-8")
    port = default_port(wire_text)
    if port is None:
        return [f"{WIRE.as_posix()}: no 'pub const DEFAULT_PORT: u16 = <port>;' line"]
    import lufs_pull_check  # noqa: PLC0415 (it imports this module)

    poll = lufs_pull_check.poll_constants(wire_text)
    if poll is None:
        names = "POLL_PORT_MIN, POLL_PORT_SPAN, LUFS_POLL_ADDRESS or METER_POLL_ADDRESS"
        return [f"{WIRE.as_posix()}: no {names} line"]
    return [error for device in DEVICES for error in device_errors(root, device, port, poll)]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="The LUFS devices' patches.")
    parser.add_argument("--root", type=Path, default=ROOT)
    args = parser.parse_args(argv)
    errors = check(args.root)
    for error in errors:
        print(f"::error::{error}")
    if errors:
        return 1
    print("FohMixer LUFS and Meter: the patches, the LUFS DSP chain, port and version agree")
    return 0


if __name__ == "__main__":
    sys.exit(main())
