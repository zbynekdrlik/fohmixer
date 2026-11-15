"""SimLive's ``FohMixer Listen`` devices (#210): the Max for Live device that
streams a track's sound to the hub while the hub selects it.

A device is a ``Live.Device.Device`` of ``class_name`` ``MxDeviceAudioEffect``
named ``FohMixer Listen``, at the end of a track's chain or (``rack``, the
owner's placement) in a new rack ``Listen`` whose chains are an empty ``Dry``
and a ``Listen`` chain holding it. Its Max id counts from 1 per host, as the
LUFS devices' do (``maxdev.py``).

What it plays: a sine at its ``tone`` (Hz; 0 is silence), the left channel
at amplitude ``LEFT`` and the right at ``RIGHT``, the phase counted from the
device's stream start, so a test can decode the hub's Opus and find the tone.

The wire is the real device's (``crates/fohmixer-proto/src/listen/wire.rs``,
``listen/jitnet.rs``): an OSC announce to the hub's UDP port once a second;
the selects read from one UDP socket on the select port that every device of
the host shares (as Max's ``udpreceive`` objects of one port are believed to
share one socket: probe P1 on #210); while the newest select names a device
(and for ``SELECT_HOLD_S`` after the last one that did), it connects to the
hub's TCP port, sends its hello as a Jitter message chunk, then every tick
the whole 960-sample frames it owes as one float32 matrix of 2 planes and
960 x k cells, big-endian, as ``jit.catch~ @mode 1`` and ``jit.net.send``
would. The hub's latency answers are read and dropped. Everything runs on
SimLive's main thread (one ``Live.Base.Timer``, ``TICK_MS`` as the patch's
``qmetro 10``).
"""

import itertools
import math
import socket
import struct
import sys
import time

import Live
import maxdev

NAME = "FohMixer Listen"
CLASS_NAME = "MxDeviceAudioEffect"
ANNOUNCE_ADDRESS = "/fohmixer/listen"
ANNOUNCE_TAGS = ",iisssssii"
SELECT_ADDRESS = "/fohmixer/listen/select"
HELLO_WORD = "fohmixer_listen"
WIRE_VERSION = 1
NO_PARENT = "-"
# The devices' version on the wire, one place for every device.
VERSION = maxdev.VERSION
RATE = 48000
FRAME = 960
LEFT = 0.5
RIGHT = 0.25
TICK_MS = 10.0
ANNOUNCE_S = 1.0
SELECT_HOLD_S = 1.5
# At most 25 frames in one tick (a stall never floods): the hub takes a
# matrix of at most 25 frames (jitnet::MAX_DATA), as jit.catch~ caught them.
MAX_FRAMES = 25
RACK_NAME = "Listen"
DRY_CHAIN = "Dry"
DEVICE_CHAIN = "Listen"
MATRIX_HEADER = 288
FLOAT32 = 2
MAX_DIMS = 32
# Device 9, v1.50.0, in the chain Listen of the rack Listen on Vox 1 in the
# set FOH Band, 48 kHz, streaming: the Rust wire's GOLDEN_ANNOUNCE_HEX.
GOLDEN_ANNOUNCE_HEX = (
    "2f666f686d697865722f6c697374656e000000002c69697373737373696900000000000100000009"
    "76312e35302e30004c697374656e00004c697374656e0000566f782031000000464f482042616e64"
    "000000000000bb8000000001"
)


def _string(text):
    data = text.encode("utf-8")
    return data + b"\0" * (4 - len(data) % 4)


def encode_announce(device, version, parents, rate, sending):
    """The announce datagram (``parents``: four names, nearest first)."""
    out = _string(ANNOUNCE_ADDRESS) + _string(ANNOUNCE_TAGS)
    out += struct.pack(">ii", WIRE_VERSION, device)
    out += _string(version)
    for parent in parents:
        out += _string(parent)
    return out + struct.pack(">ii", rate, 1 if sending else 0)


def parse_select(data):
    """A select's device id, or None for any other datagram."""
    head = _string(SELECT_ADDRESS) + _string(",ii")
    if len(data) != len(head) + 8 or not data.startswith(head):
        return None
    device, _turn = struct.unpack(">ii", data[len(head) :])
    return device


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
    """One device: its Max id, the Live device, where it sits, its tone and
    its stream."""

    def __init__(self, device_id, device, track, rack=None, chain=None):
        self.id = device_id
        self.device = device
        self.track = track
        self.rack = rack
        self.chain = chain
        self.tone = 0.0
        self.selected_until = None
        self.started = None
        self.sent = 0
        self.streamed = 0
        self.conn = None

    def parents(self, song):
        if self.chain is not None:
            return [self.chain.name, self.rack.name, self.track.name, song.name]
        return [self.track.name, song.name, NO_PARENT, NO_PARENT]

    def sending(self, now):
        return self.selected_until is not None and now < self.selected_until

    def owed(self, now):
        """The whole frames it owes at ``now`` since its stream started."""
        due = int((now - self.started) * RATE + 1e-6) // FRAME
        return max(0, min(due - self.sent // FRAME, MAX_FRAMES))

    def samples(self, frames):
        """``frames`` frames of the tone from where it stands, interleaved."""
        out = []
        step = 2.0 * math.pi * self.tone / RATE
        for n in range(self.sent, self.sent + frames * FRAME):
            s = math.sin(step * n)
            out += [LEFT * s, RIGHT * s]
        return out

    def close(self):
        if self.conn is not None:
            self.conn.close()
            self.conn = None


def track_of(song, where, index):
    if where == "master":
        return song.master_track
    tracks = song.tracks if where == "track" else song.return_tracks
    return tracks[index] if 0 <= index < len(tracks) else None


class ListenDevices:
    """The host's Listen devices (main thread only)."""

    def __init__(self, song, port=None, select_port=None, clock=time.monotonic):
        self._song = song
        self._port = port
        self._select_port = select_port
        self._clock = clock
        self._ids = itertools.count(1)
        self.devices = []
        self._announce = None
        self._selects = None
        self._timer = None
        self._announced = None

    def start(self):
        """Opens the sockets and starts the timer (a host with both ports)."""
        if self._port is None or self._select_port is None:
            return
        self._announce = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self._selects = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self._selects.bind(("127.0.0.1", self._select_port))
        self._selects.setblocking(False)
        self._timer = Live.Base.Timer(callback=self.tick, interval=TICK_MS, repeat=True)
        self._timer.start()

    def stop(self):
        if self._timer is not None:
            self._timer.stop()
        for device in self.devices:
            device.close()
        for sock in (self._announce, self._selects):
            if sock is not None:
                sock.close()

    def on(self, track):
        return [d for d in self.devices if d.track is track]

    def add(self, track, rack):
        device = Live.Device.Device(NAME, CLASS_NAME)
        if rack:
            chain = Live.Chain.Chain(DEVICE_CHAIN, [device])
            holder = Live.RackDevice.RackDevice(
                RACK_NAME, "AudioEffectGroupDevice", [Live.Chain.Chain(DRY_CHAIN), chain]
            )
            track._devices.append(holder)
            placed = ListenDevice(next(self._ids), device, track, holder, chain)
        else:
            track._devices.append(device)
            placed = ListenDevice(next(self._ids), device, track)
        track._fire("devices")
        self.devices.append(placed)
        return placed

    def remove(self, track):
        placed = self.on(track)
        if not placed:
            return False
        last = placed[-1]
        last.close()
        self.devices.remove(last)
        top = last.rack if last.rack is not None else last.device
        track.delete_device(track._devices.index(top))
        return True

    def tick(self):
        """Reads the selects, announces each second, streams what is owed."""
        now = self._clock()
        self._read_selects(now)
        if self._announced is None or now - self._announced >= ANNOUNCE_S:
            self._announced = now
            for device in self.devices:
                self._send_announce(device, now)
        for device in self.devices:
            if device.sending(now):
                self._stream(device, now)
            else:
                device.started = None

    def _read_selects(self, now):
        for _ in range(64):
            try:
                data = self._selects.recv(512)
            except BlockingIOError:
                return
            except OSError as error:
                _log(f"a select read failed: {error}")
                return
            chosen = parse_select(data)
            if chosen is None:
                _log(f"not a select: {data[:32]!r}")
                continue
            for device in self.devices:
                if device.id == chosen:
                    device.selected_until = now + SELECT_HOLD_S
                else:
                    device.selected_until = None

    def _send_announce(self, device, now):
        data = encode_announce(
            device.id, VERSION, device.parents(self._song), RATE, device.sending(now)
        )
        self._announce.sendto(data, ("127.0.0.1", self._port))

    def _stream(self, device, now):
        if device.conn is None:
            try:
                device.conn = socket.create_connection(("127.0.0.1", self._port), timeout=1.0)
            except OSError as error:
                _log(f"device {device.id} cannot connect: {error}")
                return
            hello = encode_message([HELLO_WORD, WIRE_VERSION, device.id, VERSION, RATE])
            device.conn.sendall(hello)
        if device.started is None:
            device.started = now
            device.sent = 0
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
    """A ``listendev`` line's ``(where, index, add, rack)``, or None."""
    words = line.split()
    if len(words) not in (4, 5) or words[1] not in ("track", "return", "master"):
        return None
    if not words[2].isdigit() or words[3] not in ("add", "remove"):
        return None
    if len(words) == 5 and (words[4] != "rack" or words[3] != "add"):
        return None
    return words[1], int(words[2]), words[3] == "add", len(words) == 5


def tone_line(line):
    """A ``tone`` or ``streamed`` line's ``(verb, where, index, hz)``, or None."""
    words = line.split()
    if len(words) < 3 or words[1] not in ("track", "return", "master"):
        return None
    if not words[2].isdigit():
        return None
    if words[0] == "streamed" and len(words) == 3:
        return "streamed", words[1], int(words[2]), None
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
    where, index, add, rack = parsed
    track = track_of(song, where, index)
    if track is None:
        return -1
    if add:
        listens.add(track, rack)
    elif not listens.remove(track):
        return -1
    return len(listens.on(track))


def tone_control(listens, song, parsed):
    """A ``tone`` line (the newest device's tone; answers it) or a
    ``streamed`` line (the samples per channel it streamed since it was
    added), -1 for no such track or device."""
    verb, where, index, hz = parsed
    track = track_of(song, where, index)
    placed = listens.on(track) if track is not None else []
    if not placed:
        return -1
    if verb == "streamed":
        return placed[-1].streamed
    placed[-1].tone = hz
    return int(hz)
