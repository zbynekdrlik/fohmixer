"""The FohMixer Meter's pull (#264), checked for `check_lufs_device.py`.

The design of 26.11.2026 on #264 (the LUFS device's pull, proven on the PC by
1.60.0): the meter's `peakamp~ 33` ran a clock on Max's scheduler, inside
Live's audio thread, and every output posted from there (its faces' redraw,
the `deferlow` before the `udpsend`; dasio measured 58 posts per 20 s on
band). So the hub polls each meter every 33 ms on the port it draws at load
(`/fohmixer/meter/poll`, its own address); `udpreceive @defer 1` hands the
poll to Max's main thread, the gate (Device On, as the LUFS device's) passes
it to `t b b b`, which first counts the poll for the re-read
(`poll_reread_check`, every 300th poll), then bangs the L and R `peakamp~`
(no interval: each reports the peak since its last bang), and the R value's
`t b f` bangs the pack straight into the `udpsend`, all in that one
main-thread call; the faces show the same values.
"""

from __future__ import annotations

from lufs_pull_check import (
    POLL_ROUTE,
    PULL,
    Poll,
    Shape,
    pull_boxes,
    pull_outputs,
    pull_wiring,
)
from poll_reread_check import ID_COUNT

# The readers, banged by the pull: L first (into the pack's cold slot),
# then R (its `t b f` bangs the pack).
PEAK_L = "obj-peak-l"
PEAK_R = "obj-peak-r"
READER = ["peakamp~"]


def meter_shape(poll: Poll, sink: str) -> Shape:
    """The meter's pull: the LUFS device's poll, port and Device On boxes
    with the meter's poll address, its two `peakamp~` for the three
    `snapshot~ 0` and a pull of three outlets (the count, then L and R)."""
    boxes = {
        box: words for box, words in pull_boxes(poll).items() if not box.startswith("obj-snap-")
    }
    boxes |= {
        POLL_ROUTE: ["route", poll.meter_address, poll.meter_move_address],
        PULL: ["t", "b", "b", "b"],
        PEAK_L: READER,
        PEAK_R: READER,
    }
    wiring = {
        key: sources
        for key, sources in pull_wiring(sink).items()
        if not key[0].startswith("obj-snap-")
    }
    wiring |= {
        (PEAK_L, 0): [("obj-plugin", 0), (PULL, 1)],
        (PEAK_L, 1): [],
        (PEAK_R, 0): [("obj-plugin", 1), (PULL, 0)],
        (PEAK_R, 1): [],
    }
    outputs = pull_outputs() | {PULL: [[0, PEAK_R, 0], [1, PEAK_L, 0], [2, ID_COUNT, 0]]}
    return Shape(boxes, wiring, outputs, "'peakamp~'")
