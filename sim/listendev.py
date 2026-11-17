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
every ``POLL_S`` each device asks the hub's HTTP port whether it is selected
(``GET /api/listen/select?device=<id>``, as the patch's ``maxurl`` does: one
UDP select port for every device reached only one of them on the PC, #210),
and an answer whose JSON ``on`` is true (the patch's ``dict.unpack on:`` into
``sel 1``) selects it for ``SELECT_HOLD_S``, any other answer or a failed
request stops it; while
selected it connects to the hub's TCP port, sends its hello as a Jitter
message chunk, then every tick the whole 960-sample frames it owes as one
float32 matrix of 2 planes and 960 x k cells, big-endian, as ``jit.catch~
@mode 1`` and ``jit.net.send`` would. The hub's latency answers are read and
dropped. The streams run on SimLive's main thread (one ``Live.Base.Timer``,
``TICK_MS`` as the patch's ``qmetro 10``); the polls on a thread of their
own (a Max ``maxurl`` request does not hold the patch either), whose answers
the next tick takes. The hub's HTTP port is ``--hub-port`` or, once a test's
hub has one, the ``listenhub <port>`` line.
"""

import itertools
import json
import math
import socket
import struct
import sys
import threading
import time
import urllib.request

import Live
import maxdev

NAME = "FohMixer Listen"
CLASS_NAME = "MxDeviceAudioEffect"
ANNOUNCE_ADDRESS = "/fohmixer/listen"
ANNOUNCE_TAGS = ",iisssssii"
SELECT_PATH = "/api/listen/select"
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
POLL_S = 0.25
SELECT_HOLD_S = 1.0
POLL_TIMEOUT_S = 1.0
# At most 50 frames in one tick (a stall never floods): the hub takes a
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


def select_url(hub_port, device):
    """The URL a device asks whether it is selected."""
    return f"http://127.0.0.1:{hub_port}{SELECT_PATH}?device={device}"


def answers_on(body):
    """Whether an answer's body selects the device: JSON whose ``on`` is
    true, as the patch reads it (a body that is no JSON object: off)."""
    try:
        answer = json.loads(body)
    except ValueError:
        return False
    return isinstance(answer, dict) and answer.get("on") is True


# No proxy from the environment: the hub is on loopback.
_OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def ask(hub_port, device):
    """One device's question: True when the hub answers it on, False for an
    off answer, an error answer or a failed request; and the failure's
    words, or None."""
    try:
        with _OPENER.open(select_url(hub_port, device), timeout=POLL_TIMEOUT_S) as response:
            return answers_on(response.read(256)), None
    except (OSError, ValueError) as error:
        return False, str(error)


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

    def __init__(self, device_id, device, track, rack=None, chain=None, rate=RATE):
        self.id = device_id
        self.rate = rate
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


def track_of(song, where, index):
    if where == "master":
        return song.master_track
    tracks = song.tracks if where == "track" else song.return_tracks
    return tracks[index] if 0 <= index < len(tracks) else None


class ListenDevices:
    """The host's Listen devices (main thread only)."""

    def __init__(self, song, port=None, hub_port=None, clock=time.monotonic, polls=True):
        self._song = song
        self._port = port
        self.hub_port = hub_port
        self._clock = clock
        self._polls = polls
        self._ids = itertools.count(1)
        self.devices = []
        self._announce = None
        self._timer = None
        self._announced = None
        # The poll thread's view of the devices (a tuple, replaced whole) and
        # its answers for the next tick (under the lock).
        self._polled = ()
        self._answers = {}
        self._lock = threading.Lock()
        self._halt = threading.Event()
        self._poller = None
        self._failing = False

    def start(self):
        """Opens the socket, starts the timer and the polls (a host with a
        Listen port)."""
        if self._port is None:
            return
        self._announce = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self._timer = Live.Base.Timer(callback=self.tick, interval=TICK_MS, repeat=True)
        self._timer.start()
        if self._polls:
            self._poller = threading.Thread(target=self._poll_loop, daemon=True)
            self._poller.start()

    def stop(self):
        self._halt.set()
        if self._timer is not None:
            self._timer.stop()
        for device in self.devices:
            device.close()
        if self._announce is not None:
            self._announce.close()

    def set_hub_port(self, port):
        """The hub's HTTP port, once it is known (the ``listenhub`` line)."""
        self.hub_port = port
        return port

    def _poll_loop(self):
        while not self._halt.wait(POLL_S):
            self.poll_once()

    def poll_once(self):
        """Asks the hub once for every device (the poll thread, or a test)."""
        port = self.hub_port
        if port is None:
            return
        for device_id in self._polled:
            on, error = ask(port, device_id)
            if error is not None and not self._failing:
                _log(f"the hub's select route does not answer: {error}")
            elif error is None and self._failing:
                _log("the hub's select route answers again")
            self._failing = error is not None
            with self._lock:
                self._answers[device_id] = on

    def on(self, track):
        return [d for d in self.devices if d.track is track]

    def add(self, track, rack, rate=RATE):
        device = Live.Device.Device(NAME, CLASS_NAME)
        if rack:
            chain = Live.Chain.Chain(DEVICE_CHAIN, [device])
            holder = Live.RackDevice.RackDevice(
                RACK_NAME, "AudioEffectGroupDevice", [Live.Chain.Chain(DRY_CHAIN), chain]
            )
            track._devices.append(holder)
            placed = ListenDevice(next(self._ids), device, track, holder, chain, rate)
        else:
            track._devices.append(device)
            placed = ListenDevice(next(self._ids), device, track, rate=rate)
        track._fire("devices")
        self.devices.append(placed)
        self._polled = tuple(d.id for d in self.devices)
        return placed

    def remove(self, track):
        placed = self.on(track)
        if not placed:
            return False
        last = placed[-1]
        last.close()
        self.devices.remove(last)
        self._polled = tuple(d.id for d in self.devices)
        top = last.rack if last.rack is not None else last.device
        track.delete_device(track._devices.index(top))
        return True

    def tick(self):
        """Takes the polls' answers, announces each second, streams what is
        owed."""
        now = self._clock()
        self._take_answers(now)
        if self._announced is None or now - self._announced >= ANNOUNCE_S:
            self._announced = now
            for device in self.devices:
                self._send_announce(device, now)
        for device in self.devices:
            if device.sending(now):
                self._stream(device, now)
            else:
                device.started = None

    def _take_answers(self, now):
        with self._lock:
            answers, self._answers = self._answers, {}
        for device in self.devices:
            if device.id in answers:
                device.selected_until = now + SELECT_HOLD_S if answers[device.id] else None

    def _send_announce(self, device, now):
        data = encode_announce(
            device.id, VERSION, device.parents(self._song), device.rate, device.sending(now)
        )
        self._announce.sendto(data, ("127.0.0.1", self._port))

    def _stream(self, device, now):
        if device.conn is None:
            try:
                device.conn = socket.create_connection(("127.0.0.1", self._port), timeout=1.0)
            except OSError as error:
                _log(f"device {device.id} cannot connect: {error}")
                return
            hello = encode_message([HELLO_WORD, WIRE_VERSION, device.id, VERSION, device.rate])
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
    """A ``listendev`` line's ``(where, index, add, rack, rate)``, or None:
    ``listendev <where> <i> add [rack] [<rate>]`` (48000 by default) or
    ``listendev <where> <i> remove``."""
    words = line.split()
    if len(words) < 4 or len(words) > 6 or words[1] not in ("track", "return", "master"):
        return None
    if not words[2].isdigit() or words[3] not in ("add", "remove"):
        return None
    rest = words[4:]
    if rest and words[3] != "add":
        return None
    rack = bool(rest) and rest[0] == "rack"
    if rack:
        rest = rest[1:]
    rate = RATE
    if rest:
        if len(rest) != 1 or not rest[0].isdigit():
            return None
        rate = int(rest[0])
        if not MIN_RATE <= rate <= MAX_RATE:
            return None
    return words[1], int(words[2]), words[3] == "add", rack, rate


def hub_line(line):
    """A ``listenhub <port>`` line's port (1 to 65535), or None."""
    words = line.split()
    if len(words) != 2 or not words[1].isdigit():
        return None
    port = int(words[1])
    return port if 0 < port < 65536 else None


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
    where, index, add, rack, rate = parsed
    track = track_of(song, where, index)
    if track is None:
        return -1
    if add:
        listens.add(track, rack, rate)
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
