"""The pulled devices' identity re-read on the poll path (#264), checked for
`check_lufs_device.py`.

The design of 26.11.2026 on #264 (the meter's step, then the LUFS
device's): a clock in a Max for Live device runs on Max's scheduler, inside
Live's audio thread, and its output posts from there (a `deferlow`'s
hand-over, a face's redraw). So neither pulled device holds one: its load
comes straight from `live.thisdevice` and its 10 s re-read of the four
parents rides the hub's poll: `live.thisdevice` -> `random N` -> `prepend
set` -> the `counter 0 N-1` that the pull bangs first (its rightmost
outlet, so devices loaded together count from different places and the
re-read's poll already sends the new parents) -> `sel 0` (every Nth poll)
-> the tick's `t b b b b` -> the four parents' paths. N is the device's
`Reread.polls`: 50 for the LUFS device (polled every 200 ms), 300 for the
meter (every 33 ms). Nothing in either device runs a clock or hands a
message over: no `deferlow`, `defer`, `metro`, `qmetro`, `delay`, `del` or
`pipe` (`still_errors`).
"""

from __future__ import annotations

from check_lufs_device import (
    ID_CLOCK,
    ID_CLOCK_DEFER,
    ID_CLOCK_START,
    ID_DEFER,
    ID_LOAD,
    ID_OFFSET,
    ID_OFFSET_DELAY,
    ID_TICK,
    PARENTS,
    Reread,
    kind,
)
from lufs_pull_check import PULL

ID_OFFSET_SET = "obj-id-offset-set"
ID_COUNT = "obj-id-count"
ID_EVERY = "obj-id-every"
# What a pulled device never holds: a clock or a hand-over to another queue.
STILL = ("deferlow", "defer", "metro", "qmetro", "delay", "del", "pipe")
# The pushed identity's deferred load and 10 s clock (no device's since #264).
CLOCK_BOXES = (ID_DEFER, ID_OFFSET_DELAY, ID_CLOCK_DEFER, ID_CLOCK_START, ID_CLOCK)


def identity_boxes(base: dict[str, list[str]], reread: Reread) -> dict[str, list[str]]:
    """A pulled device's identity boxes: the pushed shape's (`base`)
    without the deferred load and the clock, with the re-read on the poll."""
    kept = {box: want for box, want in base.items() if box not in CLOCK_BOXES}
    return kept | reread_boxes(reread)


def identity_wiring(
    base: dict[tuple[str, int], list[tuple[str, int]]], load: int, reread: Reread
) -> dict[tuple[str, int], list[tuple[str, int]]]:
    """A pulled device's guarded identity inlets: the pushed shape's
    (`base`, its load's extra outlets `load`) without the deferred load and
    the clock; each parent's path from the load and the poll's tick."""
    kept = {key: src for key, src in base.items() if key[0] not in CLOCK_BOXES}
    for level in range(1, PARENTS + 1):
        kept[(f"obj-parent{level}-path", 0)] = [
            (ID_LOAD, load + PARENTS - level),
            (ID_TICK, PARENTS - level),
        ]
    return kept | reread_wiring(reread)


def reread_boxes(reread: Reread) -> dict[str, list[str]]:
    """The re-read's boxes on the poll path."""
    return {
        ID_OFFSET: ["random", str(reread.polls)],
        ID_OFFSET_SET: ["prepend", "set"],
        ID_COUNT: ["counter", "0", str(reread.polls - 1)],
        ID_EVERY: ["sel", "0"],
        ID_TICK: ["t", *["b"] * PARENTS],
    }


def reread_wiring(reread: Reread) -> dict[tuple[str, int], list[tuple[str, int]]]:
    """Each inlet of the re-read and the only sources it takes."""
    return {
        (ID_LOAD, 0): [("obj-thisdevice", 0)],
        (ID_OFFSET, 0): [("obj-thisdevice", 0)],
        (ID_OFFSET, 1): [],
        (ID_OFFSET_SET, 0): [(ID_OFFSET, 0)],
        (ID_COUNT, 0): [(ID_OFFSET_SET, 0), (PULL, reread.outlet)],
        (ID_COUNT, 1): [],
        (ID_COUNT, 2): [],
        (ID_COUNT, 3): [],
        (ID_EVERY, 0): [(ID_COUNT, 0)],
        (ID_EVERY, 1): [],
        (ID_TICK, 0): [(ID_EVERY, 0)],
    }


def reread_outputs() -> dict[str, list[list]]:
    """Each box of the re-read and the only lines out of it (the tick's
    into the parents' paths alone: the poll that counted it pulls next)."""
    return {
        ID_OFFSET: [[0, ID_OFFSET_SET, 0]],
        ID_OFFSET_SET: [[0, ID_COUNT, 0]],
        ID_COUNT: [[0, ID_EVERY, 0]],
        ID_EVERY: [[0, ID_TICK, 0]],
        ID_TICK: sorted(
            [[PARENTS - level, f"obj-parent{level}-path", 0] for level in range(1, PARENTS + 1)]
        ),
    }


def still_errors(where: str, every: list[dict]) -> list[str]:
    """No clock and no hand-over anywhere in a pulled device."""
    return [
        f"{where}: {b.get('id')} ('{b.get('text')}') has no place in the device (it answers "
        "the hub's poll in Max's main thread: no deferlow, no metro, no qmetro, no delay)"
        for b in every
        if kind(b) in STILL
    ]
