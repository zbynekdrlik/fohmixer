"""The FohMixer Listen device's pull (#264), checked for
`check_listen_device.py`.

The lane's design (comment 7184226996 on #264): the device holds no clock
(a Max for Live clock runs in Live's audio thread, and its hand-over posts
a window message from there). It is pulled as the meter and the LUFS
device are, with their boxes and checks (`lufs_pull_check.pull_errors`
over a `Shape`): the port drawn at load (`random 10000` `+ 40000` into
`udpreceive @defer 1` and the announce's last slot), the gate whose left
inlet is Device On (`is_active`, observed), opened by each poll, the load
and Device On turning 1. Its own:

- one `route` for both of the hub's datagrams: the poll (its `on` int)
  into the gate, the frame poll into the frame gate (`FRAME_GATE`, which
  the select opens), whose bang alone drives `jit.catch~`, and the hub's
  move (#264) into `t b b`, the port draw then the gate, as the LUFS
  device's;
- the pull is `t b b b i`: the int first into the select's `sel 1` (a bang,
  the load's or Device On's, is 0: off), then the re-read's count
  (`poll_reread_check`, every `REREAD_POLLS`th poll), the sample rate
  (`dspstate~`), then the announce and the hello again while on.
"""

from __future__ import annotations

from lufs_pull_check import (
    POLL_MOVE,
    POLL_ROUTE,
    PULL,
    PULL_GATE,
    Poll,
    Shape,
    pull_boxes,
    pull_outputs,
    pull_wiring,
)
from poll_reread_check import ID_COUNT

FRAME_GATE = "obj-frame-gate"
SELECT = "obj-poll-sel"
SINK = "obj-announce"


def listen_shape(poll: Poll, poll_address: str, frame_address: str, move_address: str) -> Shape:
    """The Listen device's pull: the LUFS device's poll, port and Device On
    boxes with its own route and a pull of four outlets."""
    boxes = {
        box: words for box, words in pull_boxes(poll).items() if not box.startswith("obj-snap-")
    }
    boxes |= {
        POLL_ROUTE: ["route", poll_address, frame_address, move_address],
        PULL: ["t", "b", "b", "b", "i"],
        FRAME_GATE: ["gate"],
    }
    wiring = {
        key: sources
        for key, sources in pull_wiring(SINK, move_outlet=2).items()
        if not key[0].startswith("obj-snap-")
    }
    wiring |= {
        (POLL_ROUTE, 1): [],
        (FRAME_GATE, 1): [(POLL_ROUTE, 1)],
        (SELECT, 0): [(PULL, 3)],
        (SELECT, 1): [],
    }
    outputs = pull_outputs() | {
        POLL_ROUTE: [[0, PULL_GATE, 1], [1, FRAME_GATE, 1], [2, POLL_MOVE, 0]],
        PULL: [
            [0, SINK, 0],
            [0, "obj-hello-gate", 1],
            [1, "obj-dsp", 0],
            [2, ID_COUNT, 0],
            [3, SELECT, 0],
        ],
        FRAME_GATE: [[0, "obj-catch", 0]],
    }
    return Shape(boxes, wiring, outputs, "none (it reads no level)")
