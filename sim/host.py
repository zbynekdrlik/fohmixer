"""Run the real FohMixer script on SimLive, as a process.

    python3 sim/host.py --port P [--instance band] [--site sim/fixtures/test-site.json]
                        [--meters-hz 30] [--log-dir DIR]

The process's main thread is SimLive's single Live main thread: it runs the
script's timer, its scheduled messages and the optional meter animation.
``--port 0`` binds a free port. Prints ``READY <port>`` on stdout once the
WebSocket server is bound. Control lines on stdin (never over the WebSocket):

    stall <ms>                  block the main thread for <ms> milliseconds
    rename "<old>" "<new>"      rename every track and return named <old>, as a
                                user in Live would (listeners fire); prints
                                ``RENAMED <count>``
    listeners <prop> <path>     print ``LISTENERS <n>``: the Live listeners on
                                <prop> of the object at the LOM <path> (the
                                hub tests prove one listener per key), or
                                ``LISTENERS -1`` when the path does not resolve
    meter "<name>" <level|off>  pin the meters of every track and return named
                                <name> at <level> (the animation leaves them)
                                until ``off``; prints ``METER <count>`` (the UI
                                tests' clip light, #21)

SIGTERM or SIGINT calls ``FohMixer.disconnect()`` on the main thread and exits 0.
Used by the S2 integration tests and by the hub (S3) and UI (S4) tests.
"""

import argparse
import math
import os
import shlex
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

    def __init__(self, song, hz, held):
        self._song = song
        self._held = held
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
            held = self._held.get(track.name)
            if held is not None:
                track._sim_set_meter(held)
                continue
            phase = self._step / 7.0 + i
            left = 0.5 + 0.4 * math.sin(phase)
            track._sim_set_meter(left, 0.5 + 0.4 * math.sin(phase + 0.3))


def announce_ready(surface):
    if surface.server.wait_bound(READY_WAIT_S):
        print(f"READY {surface.server.port}", flush=True)


def rename_tracks(song, old, new):
    """Rename every track and return named ``old`` (main thread); the count."""
    renamed = 0
    for track in [*song.tracks, *song.return_tracks]:
        if track.name == old:
            track._sim_set("name", new)
            renamed += 1
    return renamed


def hold_meter(song, held, name, level):
    """Pin the meters of every track and return named ``name`` at ``level``
    (``None`` releases them; main thread); the count."""
    count = 0
    for track in [*song.tracks, *song.return_tracks]:
        if track.name == name:
            count += 1
            if level is not None:
                track._sim_set_meter(level)
    if level is None:
        held.pop(name, None)
    else:
        held[name] = level
    return count


def meter_level(text):
    """A held meter level: ``off`` is None; a number 0..1; anything else is not one."""
    if text == "off":
        return None
    try:
        level = float(text)
    except ValueError:
        return False
    return level if 0.0 <= level <= 1.0 else False


def count_listeners(song, prop, text):
    """The listeners on ``prop`` of the object at ``text`` (main thread); -1 if unresolved."""
    from FohMixer.lom import path as lom_path
    from FohMixer.lom.errors import FohError

    try:
        obj, _ = lom_path.resolve_path(text, song, Live.Application.get_application())
    except FohError:
        return -1
    return obj._sim_listener_count(prop)


def control(main_thread, song, line, held):
    """One control line: the answer to print, or None."""
    words = line.split(None, 2)
    if len(words) == 2 and words[0] == "stall" and words[1].isdigit():
        main_thread.stall(int(words[1]))
        return None
    if words and words[0] == "rename":
        try:
            names = shlex.split(line)[1:]
        except ValueError:
            names = []
        if len(names) == 2:
            count = main_thread.call(lambda: rename_tracks(song, names[0], names[1]))
            return f"RENAMED {count}"
    if words and words[0] == "meter":
        try:
            parts = shlex.split(line)[1:]
        except ValueError:
            parts = []
        level = meter_level(parts[1]) if len(parts) == 2 else False
        if level is not False:
            count = main_thread.call(lambda: hold_meter(song, held, parts[0], level))
            return f"METER {count}"
    if len(words) == 3 and words[0] == "listeners":
        prop, text = words[1], words[2].strip()
        return f"LISTENERS {main_thread.call(lambda: count_listeners(song, prop, text))}"
    if words:
        print(f"host: unknown control line: {line.strip()!r}", file=sys.stderr, flush=True)
    return None


def read_controls(main_thread, song, held):
    for line in sys.stdin:
        answer = control(main_thread, song, line, held)
        if answer is not None:
            print(answer, flush=True)


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
    held = {}
    animation = None
    if args.meters_hz > 0:
        animation = MeterAnimation(song, args.meters_hz, held)
        animation.start()

    def on_stop():
        if animation is not None:
            animation.stop()
        surface.disconnect()

    signal.signal(signal.SIGTERM, lambda signum, frame: main_thread.request_stop())
    signal.signal(signal.SIGINT, lambda signum, frame: main_thread.request_stop())
    threading.Thread(target=announce_ready, args=(surface,), daemon=True).start()
    threading.Thread(target=read_controls, args=(main_thread, song, held), daemon=True).start()
    main_thread.run_forever(on_stop=on_stop)
    main_thread.uninstall()
    if main_thread.errors:
        print(f"host: {len(main_thread.errors)} main-thread errors", file=sys.stderr, flush=True)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
