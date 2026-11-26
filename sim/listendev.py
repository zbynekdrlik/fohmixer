"""SimLive's ``FohMixer Listen`` devices (#210): the Max for Live device that
streams a track's sound to the hub while the hub selects it.

A device is a ``Live.Device.Device`` of ``class_name`` ``MxDeviceAudioEffect``
named ``FohMixer Listen``, at the end of a track's chain or (``rack``, the
owner's placement) in a new rack ``Listen`` whose chains are an empty ``Dry``
and a ``Listen`` chain holding it. Its Max id counts from 1 per host, as the
LUFS devices' do (``maxdev.py``).

A rack's ``Device On`` (its ``parameters 0``, on unless added ``off``, as the
PC's sets are saved since #238) is honoured: a device in a rack that is off
answers nothing and streams nothing (the patch's poll gate is its
``is_active``), and the hub switches the rack on only while a page listens
to its track; when it turns on the device announces itself at once, off.

What it plays: a sine at its ``tone`` (Hz; 0 is silence), the left channel
at amplitude ``LEFT`` and the right at ``RIGHT``, the phase counted from the
device's stream start, so a test can decode the hub's Opus and find the tone.

The wire is the real device's (``crates/fohmixer-proto/src/listen/wire.rs``,
``listen/jitnet.rs``), version 2 (#264: the device holds no clock). At its
add it binds its poll socket on a port of the LUFS devices' range
(``POLL_PORT_MIN``, ``POLL_PORT_SPAN``; SimLive draws again when a port is
taken, the real patch cannot tell) and announces itself to the hub's UDP
port once, naming that port. Each of the hub's polls
(``/fohmixer/listen/poll <on>``) sets it on or off and is answered with its
announce; on, it connects to the hub's TCP port and sends its hello as a
Jitter message chunk; each of the hub's frame polls
(``/fohmixer/listen/frame``) while on sends the whole 960-sample frames it
owes as one float32 matrix of 2 planes and 960 x k cells, big-endian, as
``jit.catch~ @mode 1`` and ``jit.net.send`` would. Another kind's poll is
dropped (the patch's ``route``). The hub's latency answers are read and
dropped. Everything runs on SimLive's main thread (one ``Live.Base.Timer``
reading each device's socket, as the patch's ``udpreceive @defer 1`` hands
a datagram to Max's main thread).
"""

import itertools
import math
import random
import socket
import struct
import sys
import time

import Live
import maxdev

NAME = "FohMixer Listen"
CLASS_NAME = "MxDeviceAudioEffect"
ANNOUNCE_ADDRESS = "/fohmixer/listen"
ANNOUNCE_TAGS = ",iisssssiii"
POLL_ADDRESS = "/fohmixer/listen/poll"
FRAME_ADDRESS = "/fohmixer/listen/frame"
HELLO_WORD = "fohmixer_listen"
WIRE_VERSION = 2
NO_PARENT = "-"
# The poll ports a device draws from (the LUFS devices' range,
# crates/fohmixer-proto/src/lufs/wire.rs POLL_PORT_MIN / POLL_PORT_SPAN).
POLL_PORT_MIN = 40000
POLL_PORT_SPAN = 10000
# How often SimLive draws again for a port taken.
BIND_TRIES = 64
# The devices' version on the wire, one place for every device.
VERSION = maxdev.VERSION
RATE = 48000
FRAME = 960
LEFT = 0.5
RIGHT = 0.25
# SimLive's main-thread tick (ms): how often the devices' sockets are read.
TICK_MS = 5.0
# At most 50 frames in one matrix (a stall never floods): the hub takes a
# matrix of at most 50 frames (jitnet::MAX_DATA: 0.5 s at 96 kHz), as
# jit.catch~ caught them.
MAX_FRAMES = 50
# The rates a device may be added at (#210: the master Live runs at 96 kHz;
# any other is for the hub's refusal).
MIN_RATE = 8000
MAX_RATE = 192000
RACK_NAME = "Listen"
DRY_CHAIN = "Dry"
DEVICE_CHAIN = "Listen"
MATRIX_HEADER = 288
FLOAT32 = 2
MAX_DIMS = 32
# Device 9, v1.50.0, in the chain Listen of the rack Listen on Vox 1 in the
# set FOH Band, 48 kHz, streaming, polled on 40007: the Rust wire's
# GOLDEN_ANNOUNCE_HEX, GOLDEN_POLL_ON_HEX and GOLDEN_FRAME_HEX.
GOLDEN_ANNOUNCE_HEX = (
    "2f666f686d697865722f6c697374656e000000002c69697373737373696969000000000200000009"
    "76312e35302e30004c697374656e00004c697374656e0000566f782031000000464f482042616e64"
    "000000000000bb800000000100009c47"
)
GOLDEN_POLL_ON_HEX = "2f666f686d697865722f6c697374656e2f706f6c6c0000002c69000000000001"
GOLDEN_FRAME_HEX = "2f666f686d697865722f6c697374656e2f6672616d6500002c000000"


def _string(text):
    data = text.encode("utf-8")
    return data + b"\0" * (4 - len(data) % 4)


def encode_announce(device, version, parents, rate, sending, port):
    """The announce datagram (``parents``: four names, nearest first)."""
    out = _string(ANNOUNCE_ADDRESS) + _string(ANNOUNCE_TAGS)
    out += struct.pack(">ii", WIRE_VERSION, device)
    out += _string(version)
    for parent in parents:
        out += _string(parent)
    return out + struct.pack(">iii", rate, 1 if sending else 0, port)


def encode_poll(on):
    """The hub's poll (the tests' hub)."""
    return _string(POLL_ADDRESS) + _string(",i") + struct.pack(">i", 1 if on else 0)


def encode_frame():
    """The hub's frame poll (the tests' hub)."""
    return _string(FRAME_ADDRESS) + _string(",")


def parse_poll(data):
    """What a datagram is to the device, as its patch's ``route`` and ``sel
    1`` read it: ``("poll", on)`` (on only for 1), ``("frame", None)``, or
    None for anything else (another kind's poll, a malformed one)."""
    if data == encode_frame():
        return "frame", None
    head = _string(POLL_ADDRESS) + _string(",i")
    if len(data) != len(head) + 4 or not data.startswith(head):
        return None
    return "poll", struct.unpack(">i", data[len(head) :])[0] == 1


def bind_poll_socket(draw=None):
    """A non-blocking UDP socket on 127.0.0.1 at a port ``draw()`` gives
    (default: random in the range), drawn again while one is taken; the
    last error after ``BIND_TRIES``."""
    draw = draw or (lambda: POLL_PORT_MIN + random.randrange(POLL_PORT_SPAN))
    error = None
    for _ in range(BIND_TRIES):
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        try:
            sock.bind(("127.0.0.1", draw()))
        except OSError as taken:
            sock.close()
            error = taken
            continue
        sock.setblocking(False)
        return sock
    raise error


def encode_message(atoms):
    """A Jitter message chunk (``JMMP``) of str and int atoms, big-endian."""
    body = b""
    for atom in atoms:
        if isinstance(atom, str):
            body += b"s" + atom.encode("utf-8") + b"\0"
        else:
            body += b"l" + struct.pack(">i", atom)
    size = len(body) + 8
    return b"JMMP" + struct.pack(">iii", size, size, len(atoms)) + body


def encode_matrix(samples, cols, rows, sent_ms):
    """A Jitter matrix chunk (``JMTX``) of stereo float32 ``samples``
    (interleaved, ``cols`` x ``rows`` cells), big-endian."""
    dims = [cols, rows] + [0] * (MAX_DIMS - 2)
    strides = [8, 8 * cols] + [0] * (MAX_DIMS - 2)
    data = struct.pack(f">{len(samples)}f", *samples)
    header = b"JMTX" + struct.pack(">iiii", MATRIX_HEADER, 2, FLOAT32, 2)
    header += struct.pack(f">{MAX_DIMS}i", *dims) + struct.pack(f">{MAX_DIMS}i", *strides)
    header += struct.pack(">id", len(data), sent_ms)
    return b"JMTX" + struct.pack(">i", MATRIX_HEADER) + header + data


def _log(text):
    print(f"listendev: {text}", file=sys.stderr, flush=True)


class ListenDevice:
    """One device: its Max id, the Live device, where it sits, its tone, its
    poll socket and its stream."""

    def __init__(self, device_id, device, track, rack=None, chain=None, rate=RATE):
        self.id = device_id
        self.rate = rate
        self.device = device
        self.track = track
        self.rack = rack
        self.chain = chain
        self.tone = 0.0
        self.on = False
        self.was_active = False
        self.started = None
        self.sent = 0
        self.streamed = 0
        self.conn = None
        self.socket = None
        self.port = 0

    def parents(self, song):
        if self.chain is not None:
            return [self.chain.name, self.rack.name, self.track.name, song.name]
        return [self.track.name, song.name, NO_PARENT, NO_PARENT]

    def active(self):
        """Whether Live runs it: no rack, or its rack's ``Device On`` on."""
        return self.rack is None or self.rack.parameters[0].value >= 0.5

    def owed(self, now):
        """The whole frames it owes at ``now`` since its stream started."""
        due = int((now - self.started) * self.rate + 1e-6) // FRAME
        return max(0, min(due - self.sent // FRAME, MAX_FRAMES))

    def samples(self, frames):
        """``frames`` frames of the tone from where it stands, interleaved."""
        out = []
        step = 2.0 * math.pi * self.tone / self.rate
        for n in range(self.sent, self.sent + frames * FRAME):
            s = math.sin(step * n)
            out += [LEFT * s, RIGHT * s]
        return out

    def close(self):
        if self.conn is not None:
            self.conn.close()
            self.conn = None

    def close_all(self):
        self.close()
        if self.socket is not None:
            self.socket.close()
            self.socket = None


def track_of(song, where, index):
    if where == "master":
        return song.master_track
    tracks = song.tracks if where == "track" else song.return_tracks
    return tracks[index] if 0 <= index < len(tracks) else None


class ListenDevices:
    """The host's Listen devices (main thread only)."""

    def __init__(self, song, port=None, clock=time.monotonic):
        self._song = song
        self._port = port
        self._clock = clock
        self._ids = itertools.count(1)
        self.devices = []
        self._announce = None
        self._timer = None

    def start(self):
        """Opens the announce socket and starts the timer (a host with a
        Listen port)."""
        if self._port is None:
            return
        self._announce = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self._timer = Live.Base.Timer(callback=self.tick, interval=TICK_MS, repeat=True)
        self._timer.start()

    def stop(self):
        if self._timer is not None:
            self._timer.stop()
        for device in self.devices:
            device.close_all()
        if self._announce is not None:
            self._announce.close()

    def on(self, track):
        return [d for d in self.devices if d.track is track]

    def add(self, track, rack, rate=RATE, on=True):
        device = Live.Device.Device(NAME, CLASS_NAME)
        if rack:
            chain = Live.Chain.Chain(DEVICE_CHAIN, [device])
            holder = Live.RackDevice.RackDevice(
                RACK_NAME, "AudioEffectGroupDevice", [Live.Chain.Chain(DRY_CHAIN), chain]
            )
            if not on:
                holder.parameters[0]._sim_set("value", 0.0)
            track._devices.append(holder)
            placed = ListenDevice(next(self._ids), device, track, holder, chain, rate)
        else:
            track._devices.append(device)
            placed = ListenDevice(next(self._ids), device, track, rate=rate)
        track._fire("devices")
        placed.socket = bind_poll_socket()
        placed.port = placed.socket.getsockname()[1]
        self.devices.append(placed)
        # Its load: one announce while Live runs it.
        placed.was_active = placed.active()
        if placed.was_active:
            self._send_announce(placed)
        return placed

    def remove(self, track):
        placed = self.on(track)
        if not placed:
            return False
        last = placed[-1]
        last.close_all()
        self.devices.remove(last)
        top = last.rack if last.rack is not None else last.device
        track.delete_device(track._devices.index(top))
        return True

    def reload(self):
        """Every device Live runs announces itself once, as at its load
        (the ``listenload`` line: a test's hub starts after its host, and
        its sweep is off); how many announced."""
        running = [d for d in self.devices if d.active()]
        for device in running:
            self._send_announce(device)
        return len(running)

    def tick(self):
        """Reads each device's poll socket and answers what came."""
        now = self._clock()
        for device in self.devices:
            active = device.active()
            if active and not device.was_active:
                # Device On turned 1: the patch's `is_active` observer bangs
                # the pull, whose int is 0 for a bang.
                self._set_on(device, False, now)
                self._send_announce(device)
            device.was_active = active
            for data in self._datagrams(device):
                self._take(device, parse_poll(data), active, now)

    def _datagrams(self, device):
        if device.socket is None:
            return []
        out = []
        while True:
            try:
                out.append(device.socket.recv(512))
            except BlockingIOError:
                return out
            except OSError as error:
                # An ICMP reset of an announce the hub's port refused.
                _log(f"device {device.id}'s poll socket: {error}")
                return out

    def _take(self, device, parsed, active, now):
        if parsed is None or not active:
            return
        kind, on = parsed
        if kind == "frame":
            if device.on:
                self._stream(device, now)
            return
        self._set_on(device, on, now)
        self._send_announce(device)

    def _set_on(self, device, on, now):
        """On: it connects and sends its hello, and its frames count from
        now (``jit.catch~`` catches all the while; the first frame poll
        takes what it caught since)."""
        if on and not device.on:
            device.on = True
            device.started = now
            device.sent = 0
            self._connect(device)
        elif not on and device.on:
            device.on = False
            device.started = None

    def _send_announce(self, device):
        if self._announce is None:
            return
        data = encode_announce(
            device.id, VERSION, device.parents(self._song), device.rate, device.on, device.port
        )
        try:
            self._announce.sendto(data, ("127.0.0.1", self._port))
        except OSError as error:
            _log(f"device {device.id} cannot announce: {error}")

    def _connect(self, device):
        if device.conn is not None or self._port is None:
            return
        try:
            device.conn = socket.create_connection(("127.0.0.1", self._port), timeout=1.0)
        except OSError as error:
            _log(f"device {device.id} cannot connect: {error}")
            return
        hello = encode_message([HELLO_WORD, WIRE_VERSION, device.id, VERSION, device.rate])
        device.conn.sendall(hello)

    def _stream(self, device, now):
        self._connect(device)
        if device.conn is None:
            return
        frames = device.owed(now)
        if frames:
            data = encode_matrix(device.samples(frames), FRAME, frames, now * 1000.0)
            try:
                device.conn.sendall(data)
            except OSError as error:
                _log(f"device {device.id} lost its connection: {error}")
                device.close()
                return
            device.sent += frames * FRAME
            device.streamed += frames * FRAME
        self._drop_answers(device)

    def _drop_answers(self, device):
        """Reads the hub's latency answers waiting (never blocks)."""
        device.conn.setblocking(False)
        try:
            device.conn.recv(65536)
        except BlockingIOError:
            return
        except OSError as error:
            _log(f"device {device.id}'s answers: {error}")
            device.close()
            return
        finally:
            if device.conn is not None:
                device.conn.settimeout(1.0)


def device_line(line):
    """A ``listendev`` line's ``(where, index, add, rack, rate, on)``, or
    None: ``listendev <where> <i> add [rack [off]] [<rate>]`` (48000 by
    default; ``off``: the rack's ``Device On`` off, #238) or ``listendev
    <where> <i> remove``."""
    words = line.split()
    if len(words) < 4 or len(words) > 7 or words[1] not in ("track", "return", "master"):
        return None
    if not words[2].isdigit() or words[3] not in ("add", "remove"):
        return None
    rest = words[4:]
    if rest and words[3] != "add":
        return None
    rack = bool(rest) and rest[0] == "rack"
    if rack:
        rest = rest[1:]
    on = not (rack and bool(rest) and rest[0] == "off")
    if not on:
        rest = rest[1:]
    rate = RATE
    if rest:
        if len(rest) != 1 or not rest[0].isdigit():
            return None
        rate = int(rest[0])
        if not MIN_RATE <= rate <= MAX_RATE:
            return None
    return words[1], int(words[2]), words[3] == "add", rack, rate, on


def tone_line(line):
    """A ``tone``, ``streamed`` or ``listenrack`` line's ``(verb, where,
    index, hz)``, or None."""
    words = line.split()
    if len(words) < 3 or words[1] not in ("track", "return", "master"):
        return None
    if not words[2].isdigit():
        return None
    if words[0] in ("streamed", "listenrack") and len(words) == 3:
        return words[0], words[1], int(words[2]), None
    if words[0] == "tone" and len(words) == 4:
        try:
            hz = float(words[3])
        except ValueError:
            return None
        if 0 <= hz < RATE / 2:
            return "tone", words[1], int(words[2]), hz
    return None


def device_control(listens, song, parsed):
    """A ``listendev`` line on the main thread: the devices on the track
    then, -1 for no such track or nothing to remove."""
    where, index, add, rack, rate, on = parsed
    track = track_of(song, where, index)
    if track is None:
        return -1
    if add:
        listens.add(track, rack, rate, on)
    elif not listens.remove(track):
        return -1
    return len(listens.on(track))


def tone_control(listens, song, parsed):
    """A ``tone`` line (the newest device's tone; answers it), a
    ``streamed`` line (the samples per channel it streamed since it was
    added) or a ``listenrack`` line (its rack's ``Device On``, 0 or 1, #238),
    -1 for no such track, device or rack."""
    verb, where, index, hz = parsed
    track = track_of(song, where, index)
    placed = listens.on(track) if track is not None else []
    if not placed:
        return -1
    if verb == "streamed":
        return placed[-1].streamed
    if verb == "listenrack":
        rack = placed[-1].rack
        return -1 if rack is None else int(rack.parameters[0].value >= 0.5)
    placed[-1].tone = hz
    return int(hz)
