"""Run the real FohMixer script on SimLive, as a process.

    python3 sim/host.py --port P [--instance band] [--site sim/fixtures/test-site.json]
                        [--meters-hz 30] [--log-dir DIR] [--lufs-port P]

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
    sets <prop> <path>          print ``SETS <n>``: the sets of <prop> through
                                Live's setter on the object at the LOM <path>
                                since the host started, refused ones too (the
                                hub tests prove "nothing written" by count,
                                #140), or ``SETS -1`` when the path does not
                                resolve
    meter "<name>" <level|off>  pin the meters of every track and return named
                                <name> at <level> (the animation leaves them)
                                until ``off``; prints ``METER <count>`` (the UI
                                tests' clip light, #21)
    tuner <track|return> <index> <set|add|remove> ['<name>']
                                a Tuner marker (#68) on the track (or return) at
                                <index>, as a user in Live would: ``set`` renames
                                its first Tuner (one is added if it has none),
                                ``add`` adds another, ``remove`` deletes the first;
                                prints ``TUNER <tuners on the track>``, or
                                ``TUNER -1`` when there is no such track
    delete-track <index>        delete the track at <index> as a user in Live
                                would (its group's children too; the lists'
                                listeners fire); prints ``DELETED <tracks
                                left>``, or ``DELETED -1`` with no such track
    cue add <time> <name...>    a locator (#140), as a user in Live would: add
    cue rename <index> <name...>  one (kept in time order) or remove one (the
    cue time <index> <time>     ``cue_points`` listener fires), rename one or
    cue remove <index>          move it (only its own ``name`` / ``time``
                                listener fires; the list stays in time order);
                                prints ``CUE <cue points>``, or ``CUE -1`` for
                                a bad line or no such locator
    song time <beats>           jump the playhead as a user would (#140; the
                                ``current_song_time`` listener fires); prints
                                ``SONG <time>``, or ``SONG -1`` for a bad line
    param <path> value <v>      write the parameter at the LOM <path> as another
                                writer would (a Max map, a click in Live: its
                                ``value`` listener fires); prints ``PARAM
                                <value>``, or ``PARAM -1`` for a bad path or a
                                refused value (a held parameter refuses)
    param <path> enabled <0|1>  hold the parameter as a Max map does (0) or free
                                it (1); prints ``PARAM <0|1>``, or ``PARAM -1``
    clip add <track> <start> <end> <name...>
    clip rename <track> <clip> <name...>
    clip remove <track> <clip>  an arrangement clip of the track at <track>
                                (#140): added (kept in start order) or removed
                                (the track's ``arrangement_clips`` listener
                                fires), or renamed (its ``name`` listener
                                fires); prints ``CLIP <clips on the track>``,
                                or ``CLIP -1`` for a bad line or no such
                                track or clip

    maxdev <lufs|meter> <track|return|master> <index> <add|remove> [rack]
                                a Max for Live device (#151, ``sim/maxdev.py``):
                                ``FohMixer LUFS`` or ``FohMixer Meter`` added at
                                the end of the track's chain (with ``rack``,
                                inside a new rack ``LUFS``: an empty ``Dry``
                                chain and a ``LUFS`` chain holding it, the
                                owner's placement), or the newest one removed
                                (its rack with it; the ``devices`` listener
                                fires); its Max id counts from 1 per host, so
                                two hosts collide on purpose; prints ``MAXDEV
                                <devices of that kind on the track>``, or
                                ``MAXDEV -1`` (no such track, nothing to remove,
                                a bad line)
    lufs <track|return|master> <index> <s_tenths> <v_tenths>
    lufs <track|return|master> <index> off
                                the newest LUFS device there sends ``S`` and
                                ``V`` (tenths, -700..0) 10 times a second to
                                ``--lufs-port`` on 127.0.0.1, or stops; its
                                names are read at each send (a rename or a
                                move changes its key); prints ``LUFS <devices
                                sending>``, or ``LUFS -1`` (no such track or
                                device, no port, a bad line)
    level <track|return|master> <index> <l_tenths> <r_tenths>
    level <track|return|master> <index> off
                                the same for the newest meter device (L and R
                                peaks, -700..120, 30 times a second); prints
                                ``LEVEL <devices sending>`` or ``LEVEL -1``
    flood <track|return|master> <index> <per_second>
    flood <track|return|master> <index> off
                                the newest LUFS device sends at that rate (the
                                hub's bound's test); prints ``FLOOD <devices
                                sending>`` or ``FLOOD -1``

While Live plays, the song clock moves ``current_song_time`` at the tempo
(``SongClock``, 30 Hz, as Live's playhead does between the script's ticks).

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
import time

import Live
import maxdev
import site_builder
from c_instance import CInstance
from main_thread import MainThread

SIM_DIR = os.path.dirname(os.path.abspath(__file__))
SCRIPT_DIR = os.path.join(os.path.dirname(SIM_DIR), "live-script")
DEFAULT_SITE = os.path.join(SIM_DIR, "fixtures", "test-site.json")
READY_WAIT_S = 3600.0
# The song clock's rate: Live's playhead as the script sees it, ~30 times a
# second (#140).
SONG_CLOCK_HZ = 30.0


def parse_args(argv):
    parser = argparse.ArgumentParser(description="FohMixer on SimLive")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--instance", default="band")
    parser.add_argument("--site", default=DEFAULT_SITE)
    parser.add_argument("--meters-hz", type=float, default=0.0)
    parser.add_argument("--log-dir", default=None)
    parser.add_argument("--lufs-port", type=int, default=None)
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


class SongClock:
    """Moves the playhead while Live plays (#140): every tick on a Live timer
    (``hz`` a second, main thread) it advances ``current_song_time`` by the
    time since its last tick (``clock``, seconds) at the tempo, and its
    listener fires; stopped, it stands (and a start does not jump by the
    pause)."""

    def __init__(self, song, hz, clock=time.monotonic):
        self._song = song
        self._clock = clock
        self._last = clock()
        self._timer = Live.Base.Timer(callback=self._tick, interval=1000.0 / hz, repeat=True)

    def start(self):
        self._last = self._clock()
        self._timer.start()

    def stop(self):
        self._timer.stop()

    def _tick(self):
        now = self._clock()
        elapsed, self._last = now - self._last, now
        song = self._song
        if song.is_playing:
            song._sim_set("current_song_time", song.current_song_time + elapsed * song.tempo / 60.0)


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


def tuner(song, kind, index, action, name):
    """Set, add or remove a Tuner on the track (or return) at ``index`` (main
    thread); the Tuners it then holds, -1 when there is no such track."""
    tracks = song.return_tracks if kind == "return" else song.tracks
    if not 0 <= index < len(tracks):
        return -1
    track = tracks[index]
    tuners = [d for d in track._devices if d.class_name == "Tuner"]
    if action == "remove":
        if tuners:
            track.delete_device(track._devices.index(tuners[0]))
    elif action == "set" and tuners:
        tuners[0]._sim_set("name", name)
    else:
        track._devices.append(Live.Device.Device(name, "Tuner", display_name="Tuner"))
        track._fire("devices")
    return sum(1 for d in track._devices if d.class_name == "Tuner")


def delete_track(song, index):
    """Delete the track at ``index`` (main thread); the tracks left, -1 if none."""
    if not 0 <= index < len(song.tracks):
        return -1
    song.delete_track(index)
    return len(song.tracks)


def _beats(text):
    """A time in beats, or None."""
    try:
        value = float(text)
    except ValueError:
        return None
    return value if math.isfinite(value) else None


def cue_line(line):
    """A ``cue`` line's ``(action, index or time, name or time)``, or None."""
    words = line.rstrip("\r\n").split(None, 3)
    if len(words) < 3:
        return None
    action, first = words[1], words[2]
    rest = words[3] if len(words) == 4 else ""
    if action == "add":
        time = _beats(first)
        return None if time is None or not rest else ("add", time, rest)
    if not first.isdigit():
        return None
    index = int(first)
    if action == "rename" and rest:
        return ("rename", index, rest)
    if action == "time" and _beats(rest) is not None:
        return ("time", index, _beats(rest))
    if action == "remove" and not rest:
        return ("remove", index, None)
    return None


def cue(song, action, first, second):
    """Add, rename, move or remove a locator (main thread); the locators
    then, -1 when there is no such locator."""
    if action == "add":
        song._cue_points.append(Live.Song.CuePoint(song, second, first))
        song._sim_sort_cue_points()
        song._fire("cue_points")
        return len(song._cue_points)
    if not 0 <= first < len(song._cue_points):
        return -1
    point = song._cue_points[first]
    if action == "rename":
        point._sim_set("name", second)
    elif action == "time":
        point._sim_set("time", second)
        song._sim_sort_cue_points()
    else:
        song._cue_points.pop(first)._sim_delete()
        song._fire("cue_points")
    return len(song._cue_points)


def clip_line(line):
    """A ``clip`` line's ``(action, track, rest)``, or None: ``add``'s rest is
    ``(start, end, name)``, ``rename``'s ``(clip, name)``, ``remove``'s
    ``(clip,)``."""
    words = line.rstrip("\r\n").split(None, 3)
    if len(words) < 4 or not words[2].isdigit():
        return None
    action, track, args = words[1], int(words[2]), words[3]
    if action == "add":
        parts = args.split(None, 2)
        if len(parts) == 3:
            start, end = _beats(parts[0]), _beats(parts[1])
            if start is not None and end is not None and start < end:
                return ("add", track, (start, end, parts[2]))
        return None
    parts = args.split(None, 1)
    if not parts[0].isdigit():
        return None
    if action == "rename" and len(parts) == 2:
        return ("rename", track, (int(parts[0]), parts[1]))
    if action == "remove" and len(parts) == 1:
        return ("remove", track, (int(parts[0]),))
    return None


def clip(song, action, track_index, rest):
    """Add, rename or remove an arrangement clip of the track at
    ``track_index`` (main thread); the track's clips then, -1 when there is
    no such track or clip."""
    if not 0 <= track_index < len(song.tracks):
        return -1
    track = song.tracks[track_index]
    clips = track._arrangement_clips
    if action == "add":
        start, end, name = rest
        clips.append(Live.Clip.Clip(name, start, end))
        clips.sort(key=lambda c: c.start_time)
        track._fire("arrangement_clips")
        return len(clips)
    if not 0 <= rest[0] < len(clips):
        return -1
    if action == "rename":
        clips[rest[0]]._sim_set("name", rest[1])
    else:
        clips.pop(rest[0])._sim_delete()
        track._fire("arrangement_clips")
    return len(clips)


def tuner_line(line):
    """A ``tuner`` line's ``(kind, index, action, name)``, or None."""
    try:
        parts = shlex.split(line)[1:]
    except ValueError:
        return None
    if len(parts) not in (3, 4) or parts[0] not in ("track", "return") or not parts[1].isdigit():
        return None
    if parts[2] not in ("set", "add", "remove") or (parts[2] != "remove") != (len(parts) == 4):
        return None
    return parts[0], int(parts[1]), parts[2], parts[3] if len(parts) == 4 else ""


def meter_level(text):
    """A held meter level: ``off`` is None; a number 0..1; anything else is not one."""
    if text == "off":
        return None
    try:
        level = float(text)
    except ValueError:
        return False
    return level if 0.0 <= level <= 1.0 else False


def resolve(song, text):
    """The object at the LOM path ``text`` (the script's own resolver), or None."""
    from FohMixer.lom import path as lom_path
    from FohMixer.lom.errors import FohError

    try:
        obj, _ = lom_path.resolve_path(text, song, Live.Application.get_application())
    except FohError:
        return None
    return obj


def count_listeners(song, prop, text):
    """The listeners on ``prop`` of the object at ``text`` (main thread); -1 if unresolved."""
    obj = resolve(song, text)
    return -1 if obj is None else obj._sim_listener_count(prop)


def count_sets(song, prop, text):
    """The sets of ``prop`` through Live's setter on the object at ``text``
    (main thread); -1 if unresolved."""
    obj = resolve(song, text)
    return -1 if obj is None else obj._sim_set_count(prop)


def song_line(line):
    """A ``song`` line's time to jump to (beats), or None."""
    words = line.split()
    if len(words) != 3 or words[1] != "time":
        return None
    return _beats(words[2])


def song_time(song, time_beats):
    """Jump the playhead to ``time_beats`` as a user would (main thread); the
    time then."""
    song.current_song_time = time_beats
    return song.current_song_time


def param_line(line):
    """A ``param`` line's ``(path, verb, value)``, or None: ``value`` takes a
    number, ``enabled`` 0 or 1."""
    words = line.split()
    if len(words) < 4 or words[-2] not in ("value", "enabled"):
        return None
    path, verb, text = " ".join(words[1:-2]), words[-2], words[-1]
    if verb == "enabled":
        return (path, verb, text == "1") if text in ("0", "1") else None
    value = _beats(text)
    return None if value is None else (path, verb, value)


def param(song, path, verb, value):
    """Write the parameter at ``path`` as another writer would (``value``),
    or hold or free it (``enabled``; main thread); the answer's number, -1
    for no parameter there or a refused value."""
    obj = resolve(song, path)
    if obj is None or not hasattr(obj, "_sim_enable"):
        return -1
    if verb == "enabled":
        obj._sim_enable(value)
        return int(value)
    try:
        obj.value = value
    except (RuntimeError, TypeError):
        return -1
    return obj.value


def maxdev_answer(main_thread, song, line, maxdevs):
    """A Max device line's answer (#151), or None for another line."""
    words = line.split()
    if not words or words[0] not in ("maxdev", "lufs", "level", "flood"):
        return None
    word = words[0].upper()
    if words[0] == "maxdev":
        parsed = maxdev.device_line(line)
        if parsed is None or maxdevs is None:
            return f"{word} -1"
        return f"{word} {main_thread.call(lambda: maxdev.device_control(maxdevs, song, parsed))}"
    parsed = maxdev.reading_line(line)
    if parsed is None or maxdevs is None:
        return f"{word} -1"
    return f"{word} {main_thread.call(lambda: maxdev.reading_control(maxdevs, song, parsed))}"


def control(main_thread, song, line, held, maxdevs=None):
    """One control line: the answer to print, or None."""
    answer = maxdev_answer(main_thread, song, line, maxdevs)
    if answer is not None:
        return answer
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
    if len(words) == 2 and words[0] == "delete-track" and words[1].isdigit():
        index = int(words[1])
        return f"DELETED {main_thread.call(lambda: delete_track(song, index))}"
    if words and words[0] == "tuner":
        parsed = tuner_line(line)
        if parsed is not None:
            return f"TUNER {main_thread.call(lambda: tuner(song, *parsed))}"
    if words and words[0] == "cue":
        parsed = cue_line(line)
        if parsed is None:
            return "CUE -1"
        return f"CUE {main_thread.call(lambda: cue(song, *parsed))}"
    if words and words[0] == "song":
        parsed = song_line(line)
        if parsed is None:
            return "SONG -1"
        return f"SONG {main_thread.call(lambda: song_time(song, parsed))}"
    if words and words[0] == "param":
        parsed = param_line(line)
        if parsed is None:
            return "PARAM -1"
        return f"PARAM {main_thread.call(lambda: param(song, *parsed))}"
    if words and words[0] == "clip":
        parsed = clip_line(line)
        if parsed is None:
            return "CLIP -1"
        return f"CLIP {main_thread.call(lambda: clip(song, *parsed))}"
    if len(words) == 3 and words[0] == "listeners":
        prop, text = words[1], words[2].strip()
        return f"LISTENERS {main_thread.call(lambda: count_listeners(song, prop, text))}"
    if len(words) == 3 and words[0] == "sets":
        prop, text = words[1], words[2].strip()
        return f"SETS {main_thread.call(lambda: count_sets(song, prop, text))}"
    if words:
        print(f"host: unknown control line: {line.strip()!r}", file=sys.stderr, flush=True)
    return None


def read_controls(main_thread, song, held, maxdevs):
    for line in sys.stdin:
        answer = control(main_thread, song, line, held, maxdevs)
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
    song_clock = SongClock(song, SONG_CLOCK_HZ)
    song_clock.start()
    maxdevs = maxdev.MaxDevices(song, args.lufs_port)
    maxdevs.start()

    def on_stop():
        if animation is not None:
            animation.stop()
        song_clock.stop()
        maxdevs.stop()
        surface.disconnect()

    signal.signal(signal.SIGTERM, lambda signum, frame: main_thread.request_stop())
    signal.signal(signal.SIGINT, lambda signum, frame: main_thread.request_stop())
    threading.Thread(target=announce_ready, args=(surface,), daemon=True).start()
    threading.Thread(
        target=read_controls, args=(main_thread, song, held, maxdevs), daemon=True
    ).start()
    main_thread.run_forever(on_stop=on_stop)
    main_thread.uninstall()
    if main_thread.errors:
        print(f"host: {len(main_thread.errors)} main-thread errors", file=sys.stderr, flush=True)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
