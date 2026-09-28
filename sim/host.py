"""Run the real FohMixer script on SimLive, as a process.

    python3 sim/host.py --port P [--instance band] [--site sim/fixtures/test-site.json]
                        [--meters-hz 30] [--log-dir DIR]

The process's main thread is SimLive's single Live main thread: it runs the
script's timer, its scheduled messages and the optional meter animation.
``--port 0`` binds a free port. Prints ``READY <port>`` on stdout once the
WebSocket server is bound. Control lines on stdin (never over the WebSocket):

    stall <ms>    block the main thread for <ms> milliseconds

SIGTERM or SIGINT calls ``FohMixer.disconnect()`` on the main thread and exits 0.
Used by the S2 integration tests and by the hub (S3) and UI (S4) tests.
"""

import argparse
import math
import os
import signal
import sys
import tempfile
import threading

import Live
import site_builder
from c_instance import CInstance
from main_thread import MainThread

SIM_DIR = os.path.dirname(os.path.abspath(__file__))
SCRIPT_DIR = os.path.join(os.path.dirname(SIM_DIR), "live-script")
DEFAULT_SITE = os.path.join(SIM_DIR, "fixtures", "test-site.json")
READY_WAIT_S = 3600.0


def parse_args(argv):
    parser = argparse.ArgumentParser(description="FohMixer on SimLive")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--instance", default="band")
    parser.add_argument("--site", default=DEFAULT_SITE)
    parser.add_argument("--meters-hz", type=float, default=0.0)
    parser.add_argument("--log-dir", default=None)
    return parser.parse_args(argv)


class MeterAnimation:
    """Moves every track's meters like music would, ``hz`` times a second (main thread)."""

    def __init__(self, song, hz):
        self._song = song
        self._step = 0
        self._timer = Live.Base.Timer(callback=self._tick, interval=1000.0 / hz, repeat=True)

    def start(self):
        self._timer.start()

    def stop(self):
        self._timer.stop()

    def _tick(self):
        self._step += 1
        tracks = [*self._song.tracks, *self._song.return_tracks, self._song.master_track]
        for i, track in enumerate(tracks):
            phase = self._step / 7.0 + i
            left = 0.5 + 0.4 * math.sin(phase)
            track._sim_set_meter(left, 0.5 + 0.4 * math.sin(phase + 0.3))


def announce_ready(surface):
    if surface.server.wait_bound(READY_WAIT_S):
        print(f"READY {surface.server.port}", flush=True)


def read_controls(main_thread):
    for line in sys.stdin:
        parts = line.split()
        if len(parts) == 2 and parts[0] == "stall" and parts[1].isdigit():
            main_thread.stall(int(parts[1]))
        elif parts:
            print(f"host: unknown control line: {line.strip()!r}", file=sys.stderr, flush=True)


def main(argv=None):
    args = parse_args(argv)
    if SCRIPT_DIR not in sys.path:
        sys.path.insert(0, SCRIPT_DIR)
    from FohMixer import Config

    Config.INSTANCE = args.instance
    Config.PORT = args.port
    Config.LOG_DIR = args.log_dir or tempfile.mkdtemp(prefix=f"fohmixer-{args.instance}-")
    import FohMixer

    song = site_builder.build(args.site)
    main_thread = MainThread().install()
    surface = FohMixer.create_instance(CInstance(song))
    animation = None
    if args.meters_hz > 0:
        animation = MeterAnimation(song, args.meters_hz)
        animation.start()

    def on_stop():
        if animation is not None:
            animation.stop()
        surface.disconnect()

    signal.signal(signal.SIGTERM, lambda signum, frame: main_thread.request_stop())
    signal.signal(signal.SIGINT, lambda signum, frame: main_thread.request_stop())
    threading.Thread(target=announce_ready, args=(surface,), daemon=True).start()
    threading.Thread(target=read_controls, args=(main_thread,), daemon=True).start()
    main_thread.run_forever(on_stop=on_stop)
    main_thread.uninstall()
    if main_thread.errors:
        print(f"host: {len(main_thread.errors)} main-thread errors", file=sys.stderr, flush=True)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
