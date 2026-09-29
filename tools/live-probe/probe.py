#!/usr/bin/env python3
"""A localhost probe of a FohMixer script: Live's main-thread timing (#5, K1/K2).

    python probe.py --port PORT [--seconds 300] [--probe-ms 100] [--label NAME]
                    [--raw FILE]

It connects to the script's WebSocket on 127.0.0.1:PORT (``PORT`` from the
Live user's ``Remote Scripts/FohMixer/Config.py``) as one more client, next
to the hub, for ``--seconds``:

- it records every heartbeat (every 100 ms, from the script's own thread):
  ``main_tick_age_ms`` (how long ago Live's main thread last ran the script's
  timer), ``gap_ms`` (the heartbeat thread's own gap) and ``max_cmd_ms`` (the
  longest single command since the script loaded);
- every ``--probe-ms`` it sends one cheap read, ``get_prop tempo`` of the song
  (the hub's own request shape), and times its round trip: socket, drain
  queue, the next main-thread tick, the LOM read, the result queue, the wire;
- it splits each round trip at the result's ``ts``, the script's wall clock
  (``time.time``) when Live's main thread queued the result: inbound (send to
  queued: socket, reader thread, drain queue, the tick, the LOM read) and
  outbound (queued to received: sender thread, wire), on the same PC's wall
  clock; the heartbeat's ``ts`` gives its outbound delay the same way;
- the gaps between the ticks that queued results are the main thread's tick
  interval when the reads come faster than the ticks (a short run with a
  small ``--probe-ms``); the split's precision is the wall clock's own step,
  measured at the start and reported as ``wall_clock_step_ms``;
- past ``MAX_PENDING`` unanswered reads it skips its read slots and counts
  them (``reads_skipped``): the script's sender sends about one frame per
  main-thread tick on the real Live, and a pile of reads would load it;
- it prints a JSON summary: percentiles (nearest rank), the counts over the
  hub's busy threshold (150 ms) and the script's stall log (200 ms), the
  timer interval estimated from the ages (2 x their mean: a tick every T ms
  sampled at unrelated moments is on average T/2 old) and the jitter (how far
  the 99th percentile runs past that interval). On Windows the ages come in
  15.6 ms steps (``time.monotonic`` is ``GetTickCount64``) and the heartbeat
  does not sample at unrelated moments, so there the result tick gaps are the
  interval (#5). ``--raw`` also writes every sample to a JSON file.

The probe writes nothing to Live but its reads. Exit 1 with one
``live-probe: ...`` line on stderr when it cannot measure (no connection, a
refused handshake, the script gone). Python 3.11 standard library only: it is
copied to the Ableton PC as this single file and run with the PC's Python.
"""

import argparse
import base64
import collections
import contextlib
import datetime
import hashlib
import itertools
import json
import math
import os
import socket
import statistics
import struct
import sys
import time

HOST = "127.0.0.1"
WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
OPCODE_TEXT = 0x1
OPCODE_CLOSE = 0x8
OPCODE_PING = 0x9
OPCODE_PONG = 0xA
# The hub reports Live busy past this tick age (`BUSY_TICK_AGE_MS`).
BUSY_MS = 150.0
# The script logs a main-thread stall past this gap (`STALL_LOG_MS`).
STALL_MS = 200.0
CONNECT_TIMEOUT_S = 3.0
# A read unanswered this long after the run is lost (the hub's `REQUEST_TIMEOUT`).
RESULT_WAIT_S = 3.0
CLOSE_WAIT_S = 1.0
# Unanswered reads at most (far below the script's RESULT_QUEUE_MAX of 1000).
MAX_PENDING = 50
# Results closer than this to the previous one were queued in the same tick:
# the script's `ts` is whole milliseconds and a drain of cheap reads takes far
# less than one.
SAME_TICK_MS = 3.0
HANDSHAKE_MAX_BYTES = 16384
RECV_SIZE = 65536
READ_COMMAND = {"target": "live_set", "name": "get_prop", "args": {"prop": "tempo"}}
DIGITS = 3

Heartbeat = collections.namedtuple(
    "Heartbeat", ("arrival_s", "main_tick_age_ms", "max_cmd_ms", "gap_ms", "outbound_ms")
)
Read = collections.namedtuple(
    "Read", ("sent_s", "round_trip_ms", "inbound_ms", "outbound_ms", "script_ts_ms")
)


class ProbeError(Exception):
    """The probe cannot measure: no connection, a refused handshake, the script gone."""


# --- the summary (pure) ---


def percentile(values, fraction):
    """Nearest rank: the smallest value with at least ``fraction`` of the values
    at or below it; None when there are none."""
    if not values:
        return None
    ordered = sorted(values)
    rank = math.ceil(fraction * len(ordered) - 1e-9)
    return ordered[min(len(ordered), max(rank, 1)) - 1]


def _rounded(value):
    return None if value is None else round(value, DIGITS)


def distribution(values):
    """Count, min, p50/p95/p99, max, mean and the number of distinct values
    (few distinct values means a coarse clock), rounded to the microsecond."""
    return {
        "count": len(values),
        "min": _rounded(min(values)) if values else None,
        "p50": _rounded(percentile(values, 0.50)),
        "p95": _rounded(percentile(values, 0.95)),
        "p99": _rounded(percentile(values, 0.99)),
        "max": _rounded(max(values)) if values else None,
        "mean": _rounded(statistics.fmean(values)) if values else None,
        "distinct": len(set(values)),
    }


def over(values, limit):
    """How many values are strictly over ``limit`` (the hub's and the script's test)."""
    return sum(1 for value in values if value > limit)


def timer_estimate(ages):
    """Live's timer interval (2 x the mean tick age) and its jitter (the 99th
    percentile's excess over that interval, 0 when none)."""
    if not ages:
        return {"interval_ms": None, "jitter_ms": None}
    interval = 2.0 * statistics.fmean(ages)
    jitter = max(0.0, percentile(ages, 0.99) - interval)
    return {"interval_ms": _rounded(interval), "jitter_ms": _rounded(jitter)}


def _gaps(values):
    return {"max": _rounded(max(values)) if values else None, "over_150_ms": over(values, BUSY_MS)}


def tick_gaps(timestamps):
    """The gaps between the ticks that queued results (``timestamps`` in arrival
    order): a result less than ``SAME_TICK_MS`` after the previous one belongs
    to its tick, and a gap runs from one tick's first result to the next's."""
    starts = []
    previous = None
    for ts in timestamps:
        if previous is None or ts - previous >= SAME_TICK_MS:
            starts.append(ts)
        previous = ts
    return [later - earlier for earlier, later in itertools.pairwise(starts)]


def wall_clock_step_ms(clock=time.time, changes=9, limit_s=0.2, timer=time.perf_counter):
    """The wall clock's typical step (the median of its next ``changes``
    steps), in ms; None when it did not move within ``limit_s``. The median,
    not the smallest: on the PC a 0.5 ms clock now and then splits one step in
    two. The script's ``ts`` comes from the same clock on the same PC, so this
    bounds every inbound/outbound split (Windows: 15.625 ms by default, ~0.5 ms
    when an audio app raised the timer resolution)."""
    deadline = timer() + limit_s
    last = clock()
    steps = []
    while len(steps) < changes and timer() < deadline:
        now = clock()
        if now != last:
            steps.append(now - last)
            last = now
    # To the nanosecond: a fine clock (Linux) steps well below a microsecond.
    return round(statistics.median(steps) * 1000.0, 6) if steps else None


def summarize(
    heartbeats, reads, *, lost, errors, connect, seconds, probe_ms, skipped=0, wall_step_ms=None
):
    """The run's summary. ``heartbeats``: ``Heartbeat`` tuples and ``reads``:
    ``Read`` tuples, in arrival order; ``lost``: reads never answered;
    ``errors``: reads answered with an error; ``connect``: the script's
    ``connect`` data; ``skipped``: read slots skipped at ``MAX_PENDING``;
    ``wall_step_ms``: ``wall_clock_step_ms()``."""
    arrivals = [beat[0] for beat in heartbeats]
    ages = [beat[1] for beat in heartbeats]
    max_cmds = [beat[2] for beat in heartbeats]
    gaps = [beat[3] for beat in heartbeats]
    arrival_gaps = [(later - earlier) * 1000.0 for earlier, later in itertools.pairwise(arrivals)]
    round_trips = [read[1] for read in reads]
    return {
        "instance": connect.get("instance"),
        "script_version": connect.get("script_version"),
        "seconds": seconds,
        "probe_ms": probe_ms,
        "wall_clock_step_ms": wall_step_ms,
        "heartbeats": len(heartbeats),
        "main_tick_age_ms": distribution(ages),
        "main_tick_age_over_150_ms": over(ages, BUSY_MS),
        "main_tick_age_over_200_ms": over(ages, STALL_MS),
        "timer_estimate": timer_estimate(ages),
        "heartbeat_gap_ms": _gaps(gaps),
        "arrival_gap_ms": _gaps(arrival_gaps),
        "heartbeat_outbound_ms": distribution([beat[4] for beat in heartbeats]),
        "max_cmd_ms": {
            "start": max_cmds[0] if max_cmds else None,
            "end": max_cmds[-1] if max_cmds else None,
        },
        "round_trip_ms": distribution(round_trips),
        "round_trip_over_150_ms": over(round_trips, BUSY_MS),
        "round_trip_over_200_ms": over(round_trips, STALL_MS),
        "read_inbound_ms": distribution([read[2] for read in reads]),
        "read_outbound_ms": distribution([read[3] for read in reads]),
        "result_tick_gap_ms": distribution(tick_gaps([read[4] for read in reads])),
        "round_trips_lost": lost,
        "round_trip_errors": errors,
        "reads_skipped": skipped,
    }


# --- the WebSocket client (RFC 6455) ---


def accept_key(key):
    """The ``Sec-WebSocket-Accept`` a server answers to ``key``."""
    digest = hashlib.sha1((key + WS_GUID).encode("ascii")).digest()
    return base64.b64encode(digest).decode("ascii")


def _masked(payload, mask):
    if not payload:
        return b""
    key = (mask * (len(payload) // 4 + 1))[: len(payload)]
    value = int.from_bytes(payload, "big") ^ int.from_bytes(key, "big")
    return value.to_bytes(len(payload), "big")


def client_frame(opcode, payload):
    """One final frame, masked as every client frame must be."""
    mask = os.urandom(4)
    size = len(payload)
    if size < 126:
        header = struct.pack("!BB", 0x80 | opcode, 0x80 | size)
    elif size < 65536:
        header = struct.pack("!BBH", 0x80 | opcode, 0x80 | 126, size)
    else:
        header = struct.pack("!BBQ", 0x80 | opcode, 0x80 | 127, size)
    return header + mask + _masked(payload, mask)


def read_frame(buffer):
    """``(opcode, fin, payload)`` of the first whole frame in ``buffer`` (a
    bytearray, consumed), or None until it is whole."""
    if len(buffer) < 2:
        return None
    first, second = buffer[0], buffer[1]
    size = second & 0x7F
    index = 2
    if size == 126:
        if len(buffer) < 4:
            return None
        size = struct.unpack_from("!H", buffer, 2)[0]
        index = 4
    elif size == 127:
        if len(buffer) < 10:
            return None
        size = struct.unpack_from("!Q", buffer, 2)[0]
        index = 10
    mask = None
    if second & 0x80:
        if len(buffer) < index + 4:
            return None
        mask = bytes(buffer[index : index + 4])
        index += 4
    if len(buffer) < index + size:
        return None
    payload = bytes(buffer[index : index + size])
    del buffer[: index + size]
    if mask is not None:
        payload = _masked(payload, mask)
    return first & 0x0F, bool(first & 0x80), payload


class Connection:
    """A client of the script: the handshake, masked frames out, JSON messages in."""

    def __init__(self, port, host=HOST):
        try:
            self._sock = socket.create_connection((host, port), CONNECT_TIMEOUT_S)
        except OSError as e:
            raise ProbeError(f"cannot connect to {host}:{port}: {e}") from e
        self._buffer = bytearray()
        try:
            self._sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            self._handshake(host, port)
        except BaseException:
            self._sock.close()
            raise

    def _handshake(self, host, port):
        key = base64.b64encode(os.urandom(16)).decode("ascii")
        # No Origin header: the script refuses a request with one (a web page
        # open on the PC must never drive Live through localhost).
        request = (
            f"GET / HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\n"
            f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        )
        self._send_bytes(request.encode("ascii"))
        response = bytearray()
        while b"\r\n\r\n" not in response:
            if len(response) > HANDSHAKE_MAX_BYTES:
                raise ProbeError("the handshake response is too large")
            data = self._recv(CONNECT_TIMEOUT_S)
            if data is None:
                raise ProbeError(f"no handshake answer in {CONNECT_TIMEOUT_S} s")
            response.extend(data)
        head, _, rest = bytes(response).partition(b"\r\n\r\n")
        lines = head.decode("latin-1").split("\r\n")
        if not lines[0].startswith("HTTP/1.1 101"):
            raise ProbeError(f"no WebSocket upgrade: {lines[0]!r}")
        headers = {}
        for line in lines[1:]:
            name, _, value = line.partition(":")
            headers[name.strip().lower()] = value.strip()
        if headers.get("sec-websocket-accept") != accept_key(key):
            raise ProbeError("the server's Sec-WebSocket-Accept does not match the key")
        self._buffer.extend(rest)

    def _send_bytes(self, data):
        try:
            self._sock.sendall(data)
        except OSError as e:
            raise ProbeError(f"the script closed the connection ({e})") from e

    def _recv(self, timeout):
        """Bytes within ``timeout`` seconds, or None; the connection's end raises."""
        self._sock.settimeout(timeout)
        try:
            data = self._sock.recv(RECV_SIZE)
        except TimeoutError:
            return None
        except OSError as e:
            raise ProbeError(f"the script closed the connection ({e})") from e
        if not data:
            raise ProbeError("the script closed the connection")
        return data

    def send(self, message):
        self._send_bytes(client_frame(OPCODE_TEXT, json.dumps(message).encode("utf-8")))

    def receive(self, timeout):
        """The next message within ``timeout`` seconds, or None."""
        deadline = time.perf_counter() + timeout
        while True:
            frame = read_frame(self._buffer)
            if frame is not None:
                message = self._message(*frame)
                if message is not None:
                    return message
                continue
            remaining = deadline - time.perf_counter()
            if remaining <= 0:
                return None
            data = self._recv(remaining)
            if data is None:
                return None
            self._buffer.extend(data)

    def _message(self, opcode, fin, payload):
        if opcode == OPCODE_CLOSE:
            raise ProbeError("the script closed the connection (close frame)")
        if opcode == OPCODE_PING:
            self._send_bytes(client_frame(OPCODE_PONG, payload))
            return None
        if opcode == OPCODE_PONG:
            return None
        if opcode != OPCODE_TEXT or not fin:
            # The script sends whole text frames only (`encode_text_frame`).
            raise ProbeError(f"unexpected frame from the script: opcode {opcode}, fin {fin}")
        try:
            message = json.loads(payload.decode("utf-8"))
        except (UnicodeDecodeError, ValueError) as e:
            raise ProbeError(f"unreadable message from the script: {e}") from e
        if not isinstance(message, dict):
            raise ProbeError(f"a message from the script is not a JSON object: {message!r}")
        return message

    def close(self):
        """Our close frame, the script's (up to ``CLOSE_WAIT_S``), then the socket."""
        try:
            self._sock.sendall(client_frame(OPCODE_CLOSE, b""))
            deadline = time.perf_counter() + CLOSE_WAIT_S
            while (remaining := deadline - time.perf_counter()) > 0:
                self._sock.settimeout(remaining)
                if not self._sock.recv(RECV_SIZE):
                    break
        except OSError:
            # Expected, and the run is already recorded: the script resets the
            # connection right after its close frame (so its port can be bound
            # again at once), or it went away first (then the run raised).
            return
        finally:
            self._sock.close()


# --- a run ---


class Recorder:
    """What a run collects from the script's messages."""

    def __init__(self, connect):
        self.connect = connect
        self.heartbeats = []
        self.reads = []
        self.pending = {}
        self.errors = 0
        self.skipped = 0

    def sent(self, uuid, sent_s, sent_wall_ms):
        """A read went out at ``sent_s`` (run seconds) and ``sent_wall_ms`` (wall clock)."""
        self.pending[uuid] = (sent_s, sent_wall_ms)

    def take(self, message, arrival_s, arrival_wall_ms, in_window):
        """One message from the script, received at ``arrival_s`` (run seconds)
        and ``arrival_wall_ms`` (wall clock); heartbeats count only ``in_window``."""
        event = message.get("event")
        if event == "heartbeat":
            if in_window:
                self.heartbeats.append(_heartbeat(message, arrival_s, arrival_wall_ms))
        elif event == "result":
            sent = self.pending.pop(message.get("uuid"), None)
            if sent is None:
                raise ProbeError(f"a result for no read of this probe: {message!r}")
            sent_s, sent_wall_ms = sent
            ts = _ts(message)
            self.reads.append(
                Read(
                    sent_s,
                    (arrival_s - sent_s) * 1000.0,
                    ts - sent_wall_ms,
                    arrival_wall_ms - ts,
                    ts,
                )
            )
            slots = message.get("data")
            if not (isinstance(slots, list) and slots and slots[0].get("ok") is True):
                self.errors += 1
        elif event == "error":
            self.pending.pop(message.get("uuid"), None)
            self.errors += 1
        elif event == "disconnect":
            raise ProbeError("the script closed the connection (Live unloads it)")


def _ts(message):
    """The script's wall clock (ms) when it queued ``message``."""
    try:
        return float(message["ts"])
    except (KeyError, TypeError, ValueError) as e:
        raise ProbeError(f"a message without its ts {message!r}: {e}") from e


def _heartbeat(message, arrival_s, arrival_wall_ms):
    try:
        data = message["data"]
        return Heartbeat(
            arrival_s,
            float(data["main_tick_age_ms"]),
            float(data["max_cmd_ms"]),
            float(data["gap_ms"]),
            arrival_wall_ms - _ts(message),
        )
    except (KeyError, TypeError, ValueError) as e:
        raise ProbeError(f"unreadable heartbeat {message!r}: {e}") from e


def await_connect(conn):
    """The script's ``connect`` data. A heartbeat may come first: the script
    adds a new connection to the heartbeat broadcast before its sender has sent
    the queued connect, and a pending heartbeat goes out first; it is skipped."""
    deadline = time.perf_counter() + CONNECT_TIMEOUT_S
    while (remaining := deadline - time.perf_counter()) > 0:
        message = conn.receive(remaining)
        if message is None:
            break
        if message.get("event") == "connect":
            return message.get("data") or {}
        if message.get("event") != "heartbeat":
            raise ProbeError(f"the script's first message is not connect: {message!r}")
    raise ProbeError(f"no connect from the script in {CONNECT_TIMEOUT_S} s")


def record(conn, seconds, probe_ms, max_pending=MAX_PENDING, result_wait_s=RESULT_WAIT_S):
    """Heartbeats and timed reads for ``seconds`` (a read slot is skipped while
    ``max_pending`` reads are unanswered); then up to ``result_wait_s`` for the
    last reads' results. Returns the ``Recorder``; its times are seconds since
    the start of the run (``time.perf_counter``)."""
    recorder = Recorder(await_connect(conn))
    uuids = itertools.count(1)
    interval = probe_ms / 1000.0
    clock = time.perf_counter
    start = clock()
    end = start + seconds
    next_read = start
    while True:
        now = clock()
        if now >= end:
            if not recorder.pending or now >= end + result_wait_s:
                break
            wake = end + result_wait_s
        elif now >= next_read:
            if len(recorder.pending) < max_pending:
                uuid = f"probe{next(uuids)}"
                recorder.sent(uuid, clock() - start, time.time() * 1000.0)
                conn.send({"uuid": uuid, "commands": [READ_COMMAND]})
            else:
                recorder.skipped += 1
            next_read += interval
            if next_read <= now:
                next_read = now + interval
            continue
        else:
            wake = min(next_read, end)
        message = conn.receive(wake - now)
        if message is not None:
            arrival = clock()
            recorder.take(message, arrival - start, time.time() * 1000.0, arrival < end)
    return recorder


def _open_raw(raw_path):
    """The raw file, opened before the run (a bad path must not cost a run)."""
    if raw_path is None:
        return contextlib.nullcontext()
    try:
        return open(raw_path, "w", encoding="utf-8")
    except OSError as e:
        raise ProbeError(f"cannot write the raw file {raw_path}: {e}") from e


def run(
    port,
    seconds,
    probe_ms,
    raw_path=None,
    host=HOST,
    max_pending=MAX_PENDING,
    result_wait_s=RESULT_WAIT_S,
):
    """Connect, record, close; the summary (and the raw samples to ``raw_path``)."""
    with _open_raw(raw_path) as raw:
        wall_step = wall_clock_step_ms()
        conn = Connection(port, host)
        try:
            recorder = record(conn, seconds, probe_ms, max_pending, result_wait_s)
        finally:
            conn.close()
        if raw is not None:
            samples = {
                "heartbeats": [beat._asdict() for beat in recorder.heartbeats],
                "reads": [read._asdict() for read in recorder.reads],
            }
            try:
                json.dump(samples, raw)
            except OSError as e:
                raise ProbeError(f"cannot write the raw file {raw_path}: {e}") from e
    return summarize(
        recorder.heartbeats,
        recorder.reads,
        lost=len(recorder.pending),
        errors=recorder.errors,
        connect=recorder.connect,
        seconds=seconds,
        probe_ms=probe_ms,
        skipped=recorder.skipped,
        wall_step_ms=wall_step,
    )


def parse_args(argv):
    parser = argparse.ArgumentParser(description="Probe a FohMixer script's timing (#5).")
    parser.add_argument("--port", type=int, required=True, help="the script's PORT (Config.py)")
    parser.add_argument("--seconds", type=float, default=300.0, help="run length (300)")
    parser.add_argument("--probe-ms", type=int, default=100, help="a read every N ms (100)")
    parser.add_argument("--label", default=None, help="a name for the run, in the summary")
    parser.add_argument("--raw", default=None, help="write every sample to this JSON file")
    args = parser.parse_args(argv)
    if args.seconds <= 0 or args.probe_ms <= 0:
        parser.error("--seconds and --probe-ms must be positive")
    return args


def main(argv=None):
    args = parse_args(argv)
    started = datetime.datetime.now(datetime.UTC).strftime("%Y-%m-%dT%H:%M:%SZ")
    try:
        summary = run(args.port, args.seconds, args.probe_ms, raw_path=args.raw)
    except ProbeError as e:
        print(f"live-probe: {e}", file=sys.stderr)
        return 1
    print(json.dumps({"label": args.label, "started_utc": started, **summary}, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
