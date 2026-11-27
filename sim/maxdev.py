"""SimLive's two Max for Live devices (#151, spec §4.2, §4.4): ``FohMixer
LUFS`` and ``FohMixer Meter``, and the datagrams they send to the hub.

A device is a ``Live.Device.Device`` of ``class_name`` ``MxDeviceAudioEffect``
(the class P1 read on the PC), named as the real device. It sits at the end of
a track's chain, or (``rack``, the owner's placement: always in a parallel
chain) inside a new rack ``LUFS`` whose chains are an empty ``Dry`` and a
``LUFS`` chain holding it. Its Max id comes from a counter per host from 1,
so two hosts' devices collide on purpose, as two Live processes' do.

While a device sends, each datagram carries its names AS THEY ARE at the
send (the real patch re-reads them every second): a renamed track or a moved
device changes its key, as in Live. The datagram is the OSC message of
``crates/fohmixer-proto/src/lufs/wire.rs`` (``encode`` here pins the same
golden bytes). The senders run on SimLive's main thread (one
``Live.Base.Timer``); each device sends its datagrams by count since it
started (``owed``), so a tick that comes late sends what it owes, and a
stall never sends more than a second's worth at once.

A ``polled`` device (#264) is the real device's polled form instead: it
binds a poll port of the devices' range (drawn, or one a test names),
announces itself once (its load), answers each of the hub's polls of its
kind with one reading naming its port (wire version 2) and, on the hub's
move of its kind (``/fohmixer/<kind>/move``), draws another port and
announces itself there; another kind's datagram is dropped (the patch's
``route``). Its port sits in ``PollPorts``, which plays what the FOH PC
showed (the design comment 7228766032 on #264): Max's ``udpreceive`` binds
with ``SO_REUSEADDR``, and Windows gives a shared port's datagrams to its
FIRST binder only; when that one leaves the port, the next one hears.
"""

import itertools
import random
import socket
import struct
import time

import Live

LUFS = "lufs"
METER = "meter"
KINDS = (LUFS, METER)
# The devices' names (the hub's confirming walk looks for exactly these).
NAMES = {LUFS: "FohMixer LUFS", METER: "FohMixer Meter"}
ADDRESSES = {LUFS: "/fohmixer/lufs", METER: "/fohmixer/meter"}
# The LOM's class_name of a Max audio effect (read on the PC, #151 P1).
CLASS_NAME = "MxDeviceAudioEffect"
TYPE_TAGS = ",iisssssii"
WIRE_VERSION = 1
# The polled form (#264): the pushed form and the device's poll port.
POLLED_TAGS = ",iisssssiii"
POLLED_WIRE_VERSION = 2
# The hub's poll and move of each kind (lufs::wire's LUFS_POLL_ADDRESS,
# METER_POLL_ADDRESS, LUFS_MOVE_ADDRESS, METER_MOVE_ADDRESS).
POLL_ADDRESSES = {LUFS: "/fohmixer/lufs/poll", METER: "/fohmixer/meter/poll"}
MOVE_ADDRESSES = {LUFS: "/fohmixer/lufs/move", METER: "/fohmixer/meter/move"}
# The poll ports a device draws from (lufs::wire POLL_PORT_MIN, POLL_PORT_SPAN).
POLL_PORT_MIN = 40000
POLL_PORT_SPAN = 10000
# How often a draw is tried again while its port is held by another program.
BIND_TRIES = 64
NO_PARENT = "-"
PARENTS = 4
# The devices' version on the wire, ``v<version.txt>``.
VERSION = "v1.60.10"
# Nominal rates: the LUFS device's 5 a second (the owner, 27.10), the
# meter's `peakamp~ 33`; a device switched off goes silent, its key stale,
# and SimLive's `lufs`/`level ... off` lines play that. SimLive pushes the
# wire version 1 form for both unless a device is added `polled`; the real
# devices answer the hub's poll (#264, the polled form: LUFS since 1.58.0,
# the meter since its step; the hub takes both), whose path
# `crates/fohmixer-hub/tests/it/lufs_listener.rs` covers with fake devices
# and `lufs_ports.rs` with polled ones on a shared port.
RATES = {LUFS: 5.0, METER: 30.0}
# The rack a device placed with ``rack`` sits in, and its two chains.
RACK_NAME = "LUFS"
DRY_CHAIN = "Dry"
DEVICE_CHAIN = "LUFS"
# The senders' tick (ms) and the most one device sends in one tick.
TICK_MS = 5.0
# The most datagrams a polled port is read for in one tick.
READS = 256
# The golden datagram of crates/fohmixer-proto/src/lufs/wire.rs
# (GOLDEN_HEX, its tests parses_the_golden_datagram and
# encode_gives_the_golden_bytes): device 7, v1.19.0, in the chain LUFS IN
# of the rack CORE Effects on Vox 1 in the set FOH Band, -253 and -248.
GOLDEN_HEX = (
    "2f666f686d697865722f6c75667300002c6969737373737369690000"
    "00000001000000077631"
    "2e31392e30004c55465320494e00434f5245204566666563747300000000566f782031000000464f482042"
    "616e6400000000ffffff03ffffff08"
)
# The hub's moves (lufs/wire/tests.rs GOLDEN_LUFS_MOVE_HEX,
# GOLDEN_METER_MOVE_HEX).
GOLDEN_MOVE_HEX = {
    LUFS: "2f666f686d697865722f6c7566732f6d6f7665002c000000",
    METER: "2f666f686d697865722f6d657465722f6d6f7665000000002c000000",
}


def _string(text):
    """An OSC string: its UTF-8 bytes, a NUL and the padding to four."""
    data = text.encode("utf-8")
    return data + b"\0" * (4 - len(data) % 4)


def encode(kind, device, version, parents, first, second, port=None):
    """The datagram of one reading (``parents``: four names, nearest first):
    the polled form when it names a ``port``, else the pushed form."""
    if len(parents) != PARENTS:
        raise ValueError(f"a datagram names {PARENTS} parents, not {len(parents)}")
    tags, wire = (TYPE_TAGS, WIRE_VERSION) if port is None else (POLLED_TAGS, POLLED_WIRE_VERSION)
    out = _string(ADDRESSES[kind]) + _string(tags)
    out += struct.pack(">ii", wire, device)
    out += _string(version)
    for parent in parents:
        out += _string(parent)
    out += struct.pack(">ii", first, second)
    return out if port is None else out + struct.pack(">i", port)


def poll_datagram(kind):
    """The hub's poll of a device of ``kind`` (the tests' hub)."""
    return _string(POLL_ADDRESSES[kind]) + _string(",")


def move_datagram(kind):
    """The hub's move of a device of ``kind`` (the tests' hub)."""
    return _string(MOVE_ADDRESSES[kind]) + _string(",")


def routed(kind, data):
    """What a datagram is to a polled device of ``kind``, as its patch's
    ``route`` reads it: ``"poll"``, ``"move"``, or None (another kind's,
    anything else)."""
    if data == poll_datagram(kind):
        return "poll"
    if data == move_datagram(kind):
        return "move"
    return None


def draw_port():
    """A port of the devices' range, as the patch's ``random`` draws it."""
    return POLL_PORT_MIN + random.randrange(POLL_PORT_SPAN)


class PollPorts:
    """The polled devices' ports, with Windows' delivery on a shared one
    (#264, measured on the FOH PC): one real socket per port and the
    devices on it in bind order; a datagram reaches the first only, and
    when it leaves the next one hears. Two devices on one port here stand
    for devices of TWO Lives (two processes, two sockets); inside one real
    Live both share Max's socket and both hear, which SimLive does not
    play. A drawn port another program holds is drawn again (SimLive's only
    liberty); a port the test names that another program holds raises."""

    def __init__(self, draw=draw_port):
        self._draw = draw
        self._ports = {}

    def join(self, device, port=None):
        """Puts ``device`` on ``port`` (a drawn one when None), after the
        devices already there; returns the port."""
        tries = 1 if port is not None else BIND_TRIES
        error = None
        for _ in range(tries):
            want = port if port is not None else self._draw()
            if want in self._ports:
                self._ports[want][1].append(device)
                return want
            sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            try:
                sock.bind(("127.0.0.1", want))
            except OSError as taken:
                sock.close()
                error = taken
                continue
            sock.setblocking(False)
            self._ports[want] = (sock, [device])
            return want
        raise error

    def leave(self, device, port):
        """Takes ``device`` off ``port``; the port's socket closes with its
        last device."""
        held = self._ports.get(port)
        if held is None or device not in held[1]:
            return
        held[1].remove(device)
        if not held[1]:
            held[0].close()
            del self._ports[port]

    def on(self, port):
        """The devices on ``port``, the one that hears first."""
        held = self._ports.get(port)
        return list(held[1]) if held is not None else []

    def take(self, deliver):
        """Reads every port's datagrams one by one, each to the port's first
        device then (``deliver(device, data)``; a move it handles can hand
        the port to the next one). At most ``READS`` a port and tick."""
        for port in list(self._ports):
            for _ in range(READS):
                held = self._ports.get(port)
                if held is None:
                    break
                try:
                    data = held[0].recv(512)
                except BlockingIOError:
                    break
                except OSError:
                    # An ICMP reset of a datagram the hub's port refused.
                    break
                deliver(held[1][0], data)

    def close(self):
        for sock, _ in self._ports.values():
            sock.close()
        self._ports.clear()


class MaxDevice:
    """One device in the set: its kind, Max id, the Live device and where it
    sits (the track, and the rack and chain when in one)."""

    def __init__(self, kind, device_id, device, track, rack=None, chain=None):
        self.kind = kind
        self.id = device_id
        self.device = device
        self.track = track
        self.rack = rack
        self.chain = chain
        self.first = -700
        self.second = -700
        self.rate = RATES[kind]
        self.started = None
        self.sent = 0
        # The polled form's port (None: the pushed form) and the polls it
        # answered.
        self.port = None
        self.answered = 0

    def parents(self, song):
        """Its four parents' names as the patch reads them at a send."""
        if self.chain is not None:
            return [self.chain.name, self.rack.name, self.track.name, song.name]
        return [self.track.name, song.name, NO_PARENT, NO_PARENT]

    def datagram(self, song):
        return encode(
            self.kind,
            self.id,
            VERSION,
            self.parents(song),
            int(self.first),
            int(self.second),
            self.port,
        )

    def owed(self, now):
        """How many datagrams it owes at ``now`` (one at its start, then at its
        rate), at most a second's worth."""
        if self.started is None:
            return 0
        # 1e-6 of a datagram: a clock's float steps never lose one.
        due = int((now - self.started) * self.rate + 1e-6) + 1
        return max(0, min(due - self.sent, max(1, int(self.rate))))


def track_of(song, where, index):
    """The track (``track``), return (``return``) or master (``master``, any
    index) named, or None."""
    if where == "master":
        return song.master_track
    tracks = song.tracks if where == "track" else song.return_tracks
    return tracks[index] if 0 <= index < len(tracks) else None


class MaxDevices:
    """The host's Max devices and their senders (main thread only)."""

    def __init__(self, song, port=None, clock=time.monotonic, draw=draw_port):
        self._song = song
        self._port = port
        self._clock = clock
        self._ids = itertools.count(1)
        self.devices = []
        self.ports = PollPorts(draw)
        self._socket = None
        self._timer = None

    def start(self):
        """Starts the senders' timer (a host with a port)."""
        if self._port is None:
            return
        self._socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self._timer = Live.Base.Timer(callback=self.tick, interval=TICK_MS, repeat=True)
        self._timer.start()

    def stop(self):
        if self._timer is not None:
            self._timer.stop()
        if self._socket is not None:
            self._socket.close()
        self.ports.close()

    def on(self, track, kind):
        """The devices of ``kind`` on ``track``, oldest first."""
        return [d for d in self.devices if d.track is track and d.kind == kind]

    def add(self, track, kind, rack, polled=False, port=None):
        """Adds a device at the end of the track's chain (in a new rack's
        ``LUFS`` chain with ``rack``); fires the track's ``devices``. A
        ``polled`` one binds its poll port (``port``, else drawn; OSError
        when another program holds the one named) and announces itself
        once, as at its load."""
        device = Live.Device.Device(NAMES[kind], CLASS_NAME)
        if rack:
            chain = Live.Chain.Chain(DEVICE_CHAIN, [device])
            holder = Live.RackDevice.RackDevice(
                RACK_NAME, "AudioEffectGroupDevice", [Live.Chain.Chain(DRY_CHAIN), chain]
            )
            track._devices.append(holder)
            placed = MaxDevice(kind, next(self._ids), device, track, holder, chain)
        else:
            track._devices.append(device)
            placed = MaxDevice(kind, next(self._ids), device, track)
        track._fire("devices")
        self.devices.append(placed)
        if polled:
            try:
                placed.port = self.ports.join(placed, port)
            except OSError:
                self.remove(track, kind)
                raise
            self._send(placed.datagram(self._song))
        return placed

    def remove(self, track, kind):
        """Deletes the newest device of ``kind`` on the track (its rack with
        it); False when it has none."""
        placed = self.on(track, kind)
        if not placed:
            return False
        last = placed[-1]
        self.devices.remove(last)
        if last.port is not None:
            self.ports.leave(last, last.port)
        top = last.rack if last.rack is not None else last.device
        track.delete_device(track._devices.index(top))
        return True

    def send(self, track, kind, first, second, rate=None):
        """The newest device of ``kind`` on the track sends ``first`` and
        ``second`` from now on (at ``rate`` a second, its own when none);
        ``first`` None stops it. False when the track has none."""
        placed = self.on(track, kind)
        if not placed:
            return False
        device = placed[-1]
        if device.port is not None:
            # A polled device only answers the hub: its values from now on.
            if first is not None:
                device.first, device.second = first, second
            return True
        if first is None:
            device.started = None
            device.rate = RATES[kind]
            return True
        device.first, device.second = first, second
        new_rate = RATES[kind] if rate is None else rate
        if device.started is None or new_rate != device.rate:
            device.rate = new_rate
            device.started = self._clock()
            device.sent = 0
        return True

    def sending(self):
        """The devices sending now."""
        return sum(1 for d in self.devices if d.started is not None)

    def tick(self):
        """Sends what each device owes now and answers what the polled ones'
        ports got (main thread)."""
        now = self._clock()
        for device in self.devices:
            for _ in range(device.owed(now)):
                self._send(device.datagram(self._song))
                device.sent += 1
        self.ports.take(self._take)

    def _take(self, device, data):
        """One datagram to a polled device: a poll of its kind is answered,
        a move of its kind draws another port and announces it there."""
        what = routed(device.kind, data)
        if what == "poll":
            device.answered += 1
            self._send(device.datagram(self._song))
        elif what == "move":
            self.ports.leave(device, device.port)
            device.port = self.ports.join(device)
            self._send(device.datagram(self._song))

    def _send(self, data):
        if self._socket is not None:
            self._socket.sendto(data, ("127.0.0.1", self._port))


def _tenths(text, top):
    """A reading in tenths within -700..``top``, or None."""
    try:
        value = int(text)
    except ValueError:
        return None
    return value if -700 <= value <= top else None


def _poll_port(text):
    """A port of the devices' range named in a line, or None."""
    if not text.isdigit():
        return None
    port = int(text)
    return port if POLL_PORT_MIN <= port < POLL_PORT_MIN + POLL_PORT_SPAN else None


def device_line(line):
    """A ``maxdev`` line's ``(kind, where, index, add, rack, polled, port)``,
    or None: ``maxdev <kind> <where> <i> add [rack] [polled [<port>]]`` or
    ``maxdev <kind> <where> <i> remove``."""
    words = line.split()
    if len(words) < 5 or words[1] not in KINDS:
        return None
    if words[2] not in ("track", "return", "master") or not words[3].isdigit():
        return None
    if words[4] not in ("add", "remove"):
        return None
    rest = words[5:]
    if rest and words[4] != "add":
        return None
    rack = bool(rest) and rest[0] == "rack"
    rest = rest[1:] if rack else rest
    polled = bool(rest) and rest[0] == "polled"
    rest = rest[1:] if polled else rest
    port = None
    if polled and rest:
        port = _poll_port(rest[0])
        rest = rest[1:] if port is not None else rest
    if rest:
        return None
    return words[1], words[2], int(words[3]), words[4] == "add", rack, polled, port


def polled_line(line):
    """A ``polled`` or ``pollport`` line's ``(verb, kind, where, index)``, or
    None."""
    words = line.split()
    if len(words) != 4 or words[0] not in ("polled", "pollport") or words[1] not in KINDS:
        return None
    if words[2] not in ("track", "return", "master") or not words[3].isdigit():
        return None
    return words[0], words[1], words[2], int(words[3])


def reading_line(line):
    """A ``lufs``, ``level`` or ``flood`` line's ``(kind, where, index,
    first, second, rate)`` (``first`` None: ``off``), or None."""
    words = line.split()
    if len(words) < 4 or words[1] not in ("track", "return", "master"):
        return None
    if not words[2].isdigit():
        return None
    verb, where, index, rest = words[0], words[1], int(words[2]), words[3:]
    kind = METER if verb == "level" else LUFS
    if rest == ["off"]:
        return kind, where, index, None, None, None
    if verb == "flood":
        if len(rest) != 1 or not rest[0].isdigit() or int(rest[0]) <= 0:
            return None
        return kind, where, index, -300, -300, float(rest[0])
    top = 120 if kind == METER else 0
    if len(rest) != 2:
        return None
    first, second = _tenths(rest[0], top), _tenths(rest[1], top)
    if first is None or second is None:
        return None
    return kind, where, index, first, second, None


def device_control(maxdevs, song, parsed):
    """A ``maxdev`` line on the main thread: the devices of that kind on the
    track then, -1 for no such track or nothing to remove."""
    kind, where, index, add, rack, polled, port = parsed
    track = track_of(song, where, index)
    if track is None:
        return -1
    if add:
        try:
            maxdevs.add(track, kind, rack, polled, port)
        except OSError:
            # The named port is another program's.
            return -1
    elif not maxdevs.remove(track, kind):
        return -1
    return len(maxdevs.on(track, kind))


def polled_control(maxdevs, song, parsed):
    """A ``polled`` line (the polls the newest polled device of that kind on
    the track answered) or a ``pollport`` line (its poll port) on the main
    thread; -1 for no such track or polled device."""
    verb, kind, where, index = parsed
    track = track_of(song, where, index)
    on = maxdevs.on(track, kind) if track is not None else []
    placed = [d for d in on if d.port is not None]
    if not placed:
        return -1
    return placed[-1].answered if verb == "polled" else placed[-1].port


def reading_control(maxdevs, song, parsed):
    """A ``lufs``/``level``/``flood`` line on the main thread: the devices
    sending then, -1 for no such track, no such device or no port."""
    kind, where, index, first, second, rate = parsed
    track = track_of(song, where, index)
    if track is None or maxdevs._port is None:
        return -1
    if not maxdevs.send(track, kind, first, second, rate):
        return -1
    return maxdevs.sending()
