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
"""

import itertools
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
NO_PARENT = "-"
PARENTS = 4
# The devices' version on the wire, ``v<version.txt>``.
VERSION = "v1.19.0"
# Nominal rates: the LUFS device's metro 200 (the owner, 27.10), the
# meter's metro 33.
RATES = {LUFS: 5.0, METER: 30.0}
# The rack a device placed with ``rack`` sits in, and its two chains.
RACK_NAME = "LUFS"
DRY_CHAIN = "Dry"
DEVICE_CHAIN = "LUFS"
# The senders' tick (ms) and the most one device sends in one tick.
TICK_MS = 5.0
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


def _string(text):
    """An OSC string: its UTF-8 bytes, a NUL and the padding to four."""
    data = text.encode("utf-8")
    return data + b"\0" * (4 - len(data) % 4)


def encode(kind, device, version, parents, first, second):
    """The datagram of one reading (``parents``: four names, nearest first)."""
    if len(parents) != PARENTS:
        raise ValueError(f"a datagram names {PARENTS} parents, not {len(parents)}")
    out = _string(ADDRESSES[kind]) + _string(TYPE_TAGS)
    out += struct.pack(">ii", WIRE_VERSION, device)
    out += _string(version)
    for parent in parents:
        out += _string(parent)
    return out + struct.pack(">ii", first, second)


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

    def parents(self, song):
        """Its four parents' names as the patch reads them at a send."""
        if self.chain is not None:
            return [self.chain.name, self.rack.name, self.track.name, song.name]
        return [self.track.name, song.name, NO_PARENT, NO_PARENT]

    def datagram(self, song):
        return encode(
            self.kind, self.id, VERSION, self.parents(song), int(self.first), int(self.second)
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

    def __init__(self, song, port=None, clock=time.monotonic):
        self._song = song
        self._port = port
        self._clock = clock
        self._ids = itertools.count(1)
        self.devices = []
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

    def on(self, track, kind):
        """The devices of ``kind`` on ``track``, oldest first."""
        return [d for d in self.devices if d.track is track and d.kind == kind]

    def add(self, track, kind, rack):
        """Adds a device at the end of the track's chain (in a new rack's
        ``LUFS`` chain with ``rack``); fires the track's ``devices``."""
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
        return placed

    def remove(self, track, kind):
        """Deletes the newest device of ``kind`` on the track (its rack with
        it); False when it has none."""
        placed = self.on(track, kind)
        if not placed:
            return False
        last = placed[-1]
        self.devices.remove(last)
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
        """Sends what each device owes now (main thread)."""
        now = self._clock()
        for device in self.devices:
            for _ in range(device.owed(now)):
                self._send(device.datagram(self._song))
                device.sent += 1

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


def device_line(line):
    """A ``maxdev`` line's ``(kind, where, index, add, rack)``, or None."""
    words = line.split()
    if len(words) not in (5, 6) or words[1] not in KINDS:
        return None
    if words[2] not in ("track", "return", "master") or not words[3].isdigit():
        return None
    if words[4] not in ("add", "remove") or (len(words) == 6 and words[5] != "rack"):
        return None
    if len(words) == 6 and words[4] != "add":
        return None
    return words[1], words[2], int(words[3]), words[4] == "add", len(words) == 6


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
    kind, where, index, add, rack = parsed
    track = track_of(song, where, index)
    if track is None:
        return -1
    if add:
        maxdevs.add(track, kind, rack)
    elif not maxdevs.remove(track, kind):
        return -1
    return len(maxdevs.on(track, kind))


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
