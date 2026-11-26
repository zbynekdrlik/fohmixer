"""The FohMixer Meter's pull (#264), checked for `check_lufs_device.py`.

The design of 26.11.2026 on #264 (the LUFS device's pull, proven on the PC by
1.60.0): the meter's `peakamp~ 33` ran a clock on Max's scheduler, inside
Live's audio thread, and every output posted from there (its faces' redraw,
the `deferlow` before the `udpsend`; dasio measured 58 posts per 20 s on
band). So the hub polls each meter every 33 ms on the port it draws at load
(`/fohmixer/meter/poll`, its own address); `udpreceive @defer 1` hands the
poll to Max's main thread, the gate (Device On, as the LUFS device's) passes
it to `t b b b`, which first counts the poll for the re-read, then bangs the
L and R `peakamp~` (no interval: each reports the peak since its last bang),
and the R value's `t b f` bangs the pack straight into the `udpsend`, all in
that one main-thread call; the faces show the same values. Nothing in the
meter runs a clock or hands a message over: no `deferlow`, `defer`,
`metro` or `qmetro` (`still_errors`).

The identity is read as the LUFS device's (`check_lufs_device.identity_*`),
but the load comes straight from `live.thisdevice` and its 10 s re-read
rides the poll: `live.thisdevice` -> `random 300` -> `prepend set` -> the
`counter 0 299` the pull bangs first (so devices loaded together count from
different places) -> `sel 0` (every 300th poll, 9.9 s at 33 ms) -> the
tick's `t b b b b` -> the four parents' paths, so that poll's datagram
already names the new parents.
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
    PULL_GATE,
    kind,
)
from lufs_pull_check import (
    ACTIVE_SEL,
    POLL_ROUTE,
    PULL,
    Poll,
    Shape,
    pull_boxes,
    pull_outputs,
    pull_wiring,
)

# The readers, banged by the pull: L first (into the pack's cold slot),
# then R (its `t b f` bangs the pack).
PEAK_L = "obj-peak-l"
PEAK_R = "obj-peak-r"
READER = ["peakamp~"]
# The re-read on the poll path: every REREAD_POLLS-th poll (300 x 33 ms).
REREAD_POLLS = 300
ID_OFFSET_SET = "obj-id-offset-set"
ID_COUNT = "obj-id-count"
ID_EVERY = "obj-id-every"
# What the meter never holds: a clock or a hand-over to another queue.
STILL = ("deferlow", "defer", "metro", "qmetro")
# The LUFS device's deferred load and 10 s clock, which the meter has not.
CLOCK_BOXES = (ID_DEFER, ID_OFFSET_DELAY, ID_CLOCK_DEFER, ID_CLOCK_START, ID_CLOCK)


def identity_boxes(base: dict[str, list[str]]) -> dict[str, list[str]]:
    """The meter's identity boxes: a pulled device's (`base`) without the
    deferred load and the clock, with the re-read on the poll path."""
    kept = {box: want for box, want in base.items() if box not in CLOCK_BOXES}
    return kept | reread_boxes()


def identity_wiring(
    base: dict[tuple[str, int], list[tuple[str, int]]], load: int
) -> dict[tuple[str, int], list[tuple[str, int]]]:
    """The meter's guarded identity inlets: a pulled device's (`base`,
    its load's extra outlets `load`) without the deferred load and the
    clock; each parent's path from the load and the poll's tick."""
    kept = {key: src for key, src in base.items() if key[0] not in CLOCK_BOXES}
    for level in range(1, PARENTS + 1):
        kept[(f"obj-parent{level}-path", 0)] = [
            (ID_LOAD, load + PARENTS - level),
            (ID_TICK, PARENTS - level),
        ]
    return kept | reread_wiring()


def meter_shape(poll: Poll, sink: str) -> Shape:
    """The meter's pull: the LUFS device's poll, port and Device On boxes
    with the meter's poll address, its two `peakamp~` for the three
    `snapshot~ 0`, the re-read's count for the tick's gate line."""
    boxes = {
        box: words for box, words in pull_boxes(poll).items() if not box.startswith("obj-snap-")
    }
    boxes |= {POLL_ROUTE: ["route", poll.meter_address], PEAK_L: READER, PEAK_R: READER}
    wiring = {
        key: sources
        for key, sources in pull_wiring(sink).items()
        if not key[0].startswith("obj-snap-")
    }
    wiring |= {
        (PULL_GATE, 1): [(POLL_ROUTE, 0), (ID_LOAD, 0), (ACTIVE_SEL, 0)],
        (PEAK_L, 0): [("obj-plugin", 0), (PULL, 1)],
        (PEAK_L, 1): [],
        (PEAK_R, 0): [("obj-plugin", 1), (PULL, 0)],
        (PEAK_R, 1): [],
    }
    outputs = pull_outputs() | {PULL: [[0, PEAK_R, 0], [1, PEAK_L, 0], [2, ID_COUNT, 0]]}
    return Shape(boxes, wiring, outputs, "'peakamp~'")


def reread_boxes() -> dict[str, list[str]]:
    """The re-read's boxes on the poll path (they replace the LUFS
    device's `delay` -> `deferlow` -> `1` -> `qmetro 10000`)."""
    return {
        ID_OFFSET: ["random", str(REREAD_POLLS)],
        ID_OFFSET_SET: ["prepend", "set"],
        ID_COUNT: ["counter", "0", str(REREAD_POLLS - 1)],
        ID_EVERY: ["sel", "0"],
        ID_TICK: ["t", *["b"] * PARENTS],
    }


def reread_wiring() -> dict[tuple[str, int], list[tuple[str, int]]]:
    """Each inlet of the re-read and the only sources it takes."""
    return {
        (ID_LOAD, 0): [("obj-thisdevice", 0)],
        (ID_OFFSET, 0): [("obj-thisdevice", 0)],
        (ID_OFFSET, 1): [],
        (ID_OFFSET_SET, 0): [(ID_OFFSET, 0)],
        (ID_COUNT, 0): [(ID_OFFSET_SET, 0), (PULL, 2)],
        (ID_COUNT, 1): [],
        (ID_COUNT, 2): [],
        (ID_COUNT, 3): [],
        (ID_EVERY, 0): [(ID_COUNT, 0)],
        (ID_EVERY, 1): [],
        (ID_TICK, 0): [(ID_EVERY, 0)],
    }


def reread_outputs() -> dict[str, list[list]]:
    """Each box of the re-read and the only lines out of it (the tick's
    into the parents' paths alone: a pull follows it in the same poll)."""
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
    """No clock and no hand-over anywhere in the meter."""
    return [
        f"{where}: {b.get('id')} ('{b.get('text')}') has no place in the meter (it answers "
        "the hub's poll in Max's main thread: no deferlow, no metro, no qmetro)"
        for b in every
        if kind(b) in STILL
    ]
