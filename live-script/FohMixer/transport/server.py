# Derived from leolabs/ableton-js v5.0.3 @69de331, midi-script/Socket.py.
# Copyright (c) 2022 Leo Bernard. MIT License, see LICENSE-ableton-js.
# Kept: the frame loop (control frames, fragments, the envelope check).
# Changed (#5, K1): no threads. Upstream accepts, reads and sends on threads;
# inside Live those got Python only around the main thread's ~30 Hz timer
# tick, so a connection sent about one frame per tick and a result waited
# whole ticks. Here Live's main thread polls non-blocking sockets in its tick
# (`poll_in`, `poll_out`); a connection keeps a bounded result queue and a
# latest-value map, sends a pending heartbeat first, and keeps what its socket
# does not take for the next tick. Also (design §3.7, §3.9): no static server,
# no auth, no port files, SO_EXCLUSIVEADDRUSE on Windows, and a close that
# finishes the WebSocket close handshake and then resets the connection so
# the port can be bound again at once (Windows keeps an exclusive port busy
# while accepted connections are still closing).
"""Single-threaded localhost WebSocket server, polled by Live's main thread.

Each tick calls ``poll_in`` (accept, handshakes, frames: well-formed requests
go to ``inbox`` as ``(connection, payload)``, and ``(connection, None)`` once
a connection is gone), then the script's work (which calls the ``push_*`` /
``send_*`` methods), then ``poll_out`` (every connection writes what is
pending, as far as its socket takes it). Every socket is non-blocking and
``select`` never waits, so a tick never waits for a client; only
``shutdown`` (the script unloading) waits, up to its grace.
"""

import collections
import itertools
import json
import logging
import os
import queue
import select
import socket
import struct
import threading
import time

from .websocket import (
    HANDSHAKE_MAX_BYTES,
    HANDSHAKE_RECV_SIZE,
    OPCODE_CLOSE,
    OPCODE_CONTINUATION,
    OPCODE_PING,
    OPCODE_PONG,
    OPCODE_TEXT,
    encode_close_frame,
    encode_pong_frame,
    encode_text_frame,
    handshake_response,
    is_websocket_upgrade,
    parse_http_request,
    try_read_frame,
)

BIND_RETRY_DELAYS_S = (0.25, 0.5, 1.0, 2.0, 5.0)
LISTEN_BACKLOG = 16
# A client that has not sent its whole upgrade request by then is dropped.
HANDSHAKE_TIMEOUT_S = 3.0
# A connection whose socket took no byte for this long while output waited is
# closed (the hub resyncs): the client stopped reading.
SEND_STALL_S = 3.0
# After our close frame, how long the peer's close frame is awaited.
CLOSE_HANDSHAKE_TIMEOUT_S = 1.0
SHUTDOWN_POLL_S = 0.01
MAX_MESSAGE_BYTES = 16 * 1024 * 1024
RECV_SIZE = 65536
# Per connection and tick: bounds on the main thread's socket work.
READ_BYTES_PER_TICK = 1024 * 1024
WRITE_BYTES_PER_TICK = 256 * 1024
# Messages are encoded at most this far ahead of the socket, so a heartbeat
# set now waits only behind these bytes, never behind the queued results.
WRITE_AHEAD_BYTES = 65536
_LINGER_ABORT = struct.pack("HH" if os.name == "nt" else "ii", 1, 0)
_BAD_REQUEST = b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"
_NO_UUID = object()

OPEN = "open"
CLOSING = "closing"
CLOSED = "closed"


def now_ms():
    return round(time.time() * 1000)


def encode_message(message):
    text = json.dumps(message, ensure_ascii=False, separators=(",", ":"), default=str)
    return encode_text_frame(text.encode("utf-8"))


class Connection:
    """One WebSocket client. ``push_*``/``send_*`` only queue; the tick writes.

    Output, by priority (``_take_next``): the pending heartbeat (the latest
    only), then results and events in order (bounded: going past
    ``result_queue_max`` closes the connection, the hub resyncs), then every
    pending value as one ``values`` message (the latest item per key), and
    last, when closing, the close frame. ``first_bytes`` (the 101 response and
    the ``connect`` frame) go before all of it.
    """

    _ids = itertools.count(1)

    def __init__(self, sock, first_bytes, result_queue_max, logger, inbox, now):
        self.id = next(Connection._ids)
        self._sock = sock
        self._max = result_queue_max
        self._log = logger
        self._inbox = inbox
        self._out = collections.deque()
        self._values = {}
        self._heartbeat = None
        self._wbuf = bytearray(first_bytes)
        self._rbuf = bytearray()
        self._fragments = bytearray()
        self._fragment_opcode = None
        self._state = OPEN
        self._close_queued = False
        self._close_sent_at = None
        self._peer_closed = False
        self._last_progress = now
        self._finished = False

    @property
    def socket(self):
        return self._sock

    @property
    def is_open(self):
        return self._state == OPEN

    @property
    def finished(self):
        """The connection is over and its socket closed."""
        return self._finished

    @property
    def reading(self):
        """Whether the tick still reads this socket (not after the peer's close)."""
        return not (self._finished or self._peer_closed)

    def pending(self):
        """``(queued results/events, pending value keys, unsent bytes)``, for
        diagnostics and tests."""
        return len(self._out), len(self._values), len(self._wbuf)

    # --- queueing: never touches the socket ---

    def push_result(self, uuid, data):
        return self._enqueue({"event": "result", "uuid": uuid, "data": data, "ts": now_ms()})

    def send_event(self, event, data=None, uuid=_NO_UUID):
        message = {"event": event, "data": data, "ts": now_ms()}
        if uuid is not _NO_UUID:
            message["uuid"] = uuid
        return self._enqueue(message)

    def send_error(self, uuid, text):
        return self.send_event("error", text, uuid)

    def send_frame(self, frame):
        return self._enqueue(frame)

    def push_value(self, key, item):
        if self._state != OPEN:
            return False
        self._values[key] = item
        return True

    def push_values(self, items):
        if self._state != OPEN:
            return False
        self._values.update(items)
        return True

    def set_heartbeat(self, data):
        if self._state != OPEN:
            return False
        self._heartbeat = {"event": "heartbeat", "data": data, "ts": now_ms()}
        return True

    def close(self):
        """Graceful: send what is queued, a close frame, then close."""
        if self._state == OPEN:
            self._state = CLOSING

    def abort(self):
        """At once: drop what is queued and reset the connection."""
        self._finish()

    # --- the main thread's tick ---

    def receive(self):
        """Read what has arrived (up to ``READ_BYTES_PER_TICK``): requests go to
        the inbox; the end of the stream or a read error resets the connection."""
        received = 0
        while self.reading and received < READ_BYTES_PER_TICK:
            try:
                data = self._sock.recv(RECV_SIZE)
            except (BlockingIOError, InterruptedError):
                return
            except OSError:
                self.abort()
                return
            if not data:
                self.abort()
                return
            received += len(data)
            self.take(data)

    def take(self, data):
        """Bytes read from the client (also the ones behind its upgrade request)."""
        self._rbuf.extend(data)
        if len(self._rbuf) + len(self._fragments) > MAX_MESSAGE_BYTES:
            self._log.warning("connection %s: message too large, closing it", self.id)
            self.abort()
            return
        while self.reading:
            frame = try_read_frame(self._rbuf)
            if frame is None:
                return
            self._frame(*frame)

    def flush(self, now):
        """Write pending output as far as the socket takes it (up to
        ``WRITE_BYTES_PER_TICK``); the rest waits for the next tick."""
        if self._finished:
            return
        written = 0
        while written < WRITE_BYTES_PER_TICK:
            self._fill()
            if not self._wbuf:
                break
            chunk = self._wbuf[: WRITE_BYTES_PER_TICK - written]
            try:
                sent = self._sock.send(chunk)
            except (BlockingIOError, InterruptedError):
                break
            except OSError as e:
                if self._state == OPEN:
                    self._log.warning("connection %s: send failed (%s), closing it", self.id, e)
                self.abort()
                return
            del self._wbuf[:sent]
            written += sent
            if sent < len(chunk):
                break
        if written or not self._has_output():
            self._last_progress = now
        elif now - self._last_progress >= SEND_STALL_S:
            self._log.warning(
                "connection %s: the client read nothing for %.1f s, closing it",
                self.id,
                SEND_STALL_S,
            )
            self.abort()
            return
        if self._close_queued and not self._wbuf:
            if self._close_sent_at is None:
                self._close_sent_at = now
            if self._peer_closed or now - self._close_sent_at >= CLOSE_HANDSHAKE_TIMEOUT_S:
                self._finish()

    # --- internals ---

    def _enqueue(self, message):
        if self._state != OPEN:
            return False
        if len(self._out) < self._max:
            self._out.append(message)
            return True
        self._log.warning(
            "connection %s: result queue full (%s messages), closing it", self.id, self._max
        )
        self.abort()
        return False

    def _has_output(self):
        return bool(
            self._wbuf
            or self._heartbeat is not None
            or self._out
            or self._values
            or (self._state == CLOSING and not self._close_queued)
        )

    def _fill(self):
        """Encode pending messages into the write buffer, up to ``WRITE_AHEAD_BYTES``."""
        while len(self._wbuf) < WRITE_AHEAD_BYTES:
            message = self._take_next()
            if message is None:
                return
            self._wbuf += message if isinstance(message, bytes) else encode_message(message)

    def _take_next(self):
        """The next message by priority: the pending heartbeat, then the results
        and events in order, then every pending value as one ``values`` message,
        then (closing) the close frame; ``None`` when nothing waits.

        A heartbeat set now goes out behind at most the write buffer, never
        behind the results queued meanwhile: a late heartbeat makes the hub
        report Live busy (#9).
        """
        if self._heartbeat is not None:
            heartbeat, self._heartbeat = self._heartbeat, None
            return heartbeat
        if self._out:
            return self._out.popleft()
        if self._values:
            values, self._values = self._values, {}
            return {"event": "values", "data": list(values.values()), "ts": now_ms()}
        if self._state == CLOSING and not self._close_queued:
            self._close_queued = True
            return encode_close_frame()
        return None

    def _frame(self, opcode, fin, payload):
        if opcode == OPCODE_CLOSE:
            self._peer_closed = True
            self.close()
            return
        if self._state != OPEN:
            return
        if opcode == OPCODE_PING:
            self.send_frame(encode_pong_frame(payload))
            return
        if opcode == OPCODE_PONG:
            return
        if opcode == OPCODE_CONTINUATION:
            if self._fragment_opcode is None:
                return
            self._fragments.extend(payload)
            if fin:
                self._payload(self._fragment_opcode, self._fragments)
                self._fragments = bytearray()
                self._fragment_opcode = None
            return
        if not fin:
            self._fragment_opcode = opcode
            self._fragments = bytearray(payload)
            return
        self._payload(opcode, payload)

    def _payload(self, opcode, payload):
        if opcode != OPCODE_TEXT:
            self.send_error(None, "binary frames are not supported")
            return
        try:
            message = json.loads(bytes(payload).decode("utf-8"))
        except (UnicodeDecodeError, ValueError) as e:
            self.send_error(None, f"invalid JSON: {e}")
            return
        uuid = message.get("uuid") if isinstance(message, dict) else None
        if not isinstance(message, dict) or not isinstance(message.get("commands"), list):
            self.send_error(uuid, "missing or invalid commands array")
            return
        self._inbox.put((self, message))

    def _finish(self):
        """Reset instead of lingering in TIME_WAIT, close, and drop the output."""
        if self._finished:
            return
        self._finished = True
        self._state = CLOSED
        self._out.clear()
        self._values = {}
        self._heartbeat = None
        self._wbuf = bytearray()
        try:
            self._sock.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, _LINGER_ABORT)
            self._sock.shutdown(socket.SHUT_RDWR)
        except OSError as e:
            self._log.debug("connection %s: shutdown: %s", self.id, e)
        try:
            self._sock.close()
        except OSError as e:
            self._log.debug("connection %s: close: %s", self.id, e)


class _Handshake:
    """A client whose upgrade request is still coming in."""

    def __init__(self, sock, now):
        self.sock = sock
        self.started = now
        self.request = bytearray()


class Server:
    """Accepts WebSocket clients on ``host:port`` and feeds ``inbox``, when polled.

    Every connection's first frame is ``connect`` with ``connect_data``: it is
    written with the 101 response, before anything a tick queues. A failed
    bind is retried by ``poll_in`` (after 0.25 s, 0.5 s, 1 s, 2 s, then every
    5 s) and logged once per distinct error; ``bind_error`` holds the last one.
    """

    def __init__(self, host, port, connect_data, result_queue_max=1000, logger=None):
        self.host = host
        self.connect_data = connect_data
        self.inbox = queue.Queue()
        self.bind_error = None
        self._port = int(port)
        self._max = result_queue_max
        self._log = logger or logging.getLogger("fohmixer")
        self._connections = []
        self._handshakes = []
        self._listener = None
        self._running = False
        self._bound = threading.Event()
        self._bind_attempts = 0
        self._next_bind_at = 0.0

    @property
    def port(self):
        return self._port

    def start(self):
        """Bind now; a failed bind is retried by later ticks."""
        self._running = True
        self._try_bind(time.monotonic())

    def wait_bound(self, timeout=None):
        """Wait until the port is bound (another thread may wait: sim/host.py)."""
        return self._bound.wait(timeout)

    def connections(self):
        return list(self._connections)

    def broadcast(self, event, data=None):
        for conn in self.connections():
            conn.send_event(event, data)

    def broadcast_heartbeat(self, data):
        for conn in self.connections():
            conn.set_heartbeat(data)

    # --- the main thread's tick ---

    def poll_in(self):
        """Accept, advance the handshakes, and read every connection that has
        data; never waits."""
        if not self._running:
            return
        now = time.monotonic()
        if self._listener is None and now >= self._next_bind_at:
            self._try_bind(now)
        for handshake in [h for h in self._handshakes if now - h.started >= HANDSHAKE_TIMEOUT_S]:
            self._drop_handshake(handshake)
        owners = {handshake.sock: handshake for handshake in self._handshakes}
        owners.update({conn.socket: conn for conn in self._connections if conn.reading})
        if self._listener is not None:
            owners[self._listener] = self
        if not owners:
            return
        readable, _, _ = select.select(list(owners), [], [], 0)
        for sock in readable:
            owner = owners[sock]
            if owner is self:
                self._accept(now)
            elif isinstance(owner, _Handshake):
                self._advance_handshake(owner, now)
            else:
                owner.receive()
        self._reap()

    def poll_out(self):
        """Every connection writes what is pending, as far as its socket takes it."""
        now = time.monotonic()
        for conn in self.connections():
            conn.flush(now)
        self._reap()

    def shutdown(self, grace_s=0.3):
        """Stop listening, close every connection (graceful up to ``grace_s``, then reset).

        The one call that waits (Live unloads the script): it polls the closing
        connections until each has finished its close handshake or the grace is
        over, then resets the rest, so a new server can bind the same port at once.
        """
        self._running = False
        self._close_listener()
        for handshake in list(self._handshakes):
            self._drop_handshake(handshake)
        conns = self.connections()
        for conn in conns:
            conn.close()
        deadline = time.monotonic() + grace_s
        while True:
            now = time.monotonic()
            for conn in conns:
                conn.flush(now)
            waiting = [conn for conn in conns if not conn.finished]
            if not waiting or now >= deadline:
                break
            reading = {conn.socket: conn for conn in waiting if conn.reading}
            wait_s = min(SHUTDOWN_POLL_S, deadline - now)
            if not reading:
                time.sleep(wait_s)
                continue
            readable, _, _ = select.select(list(reading), [], [], wait_s)
            for sock in readable:
                reading[sock].receive()
        for conn in conns:
            conn.abort()
        self._reap()

    # --- internals ---

    def _try_bind(self, now):
        try:
            self._bind()
        except OSError as e:
            message = f"cannot bind {self.host}:{self._port}: {e}"
            if message != self.bind_error:
                self._log.warning("%s; retrying", message)
            self.bind_error = message
            delay = BIND_RETRY_DELAYS_S[min(self._bind_attempts, len(BIND_RETRY_DELAYS_S) - 1)]
            self._bind_attempts += 1
            self._next_bind_at = now + delay
            return
        if self.bind_error is not None:
            self._log.warning("bound %s:%s after retrying", self.host, self._port)
        self.bind_error = None
        self._bind_attempts = 0

    def _bind(self):
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            exclusive = getattr(socket, "SO_EXCLUSIVEADDRUSE", None)
            if os.name == "nt" and exclusive is not None:
                sock.setsockopt(socket.SOL_SOCKET, exclusive, 1)
            else:
                sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            sock.bind((self.host, self._port))
            sock.listen(LISTEN_BACKLOG)
            sock.setblocking(False)
        except OSError:
            sock.close()
            raise
        self._port = sock.getsockname()[1]
        self._listener = sock
        self._bound.set()

    def _close_listener(self):
        listener, self._listener = self._listener, None
        if listener is None:
            return
        try:
            listener.shutdown(socket.SHUT_RDWR)
        except OSError as e:
            self._log.debug("listener shutdown: %s", e)
        listener.close()

    def _accept(self, now):
        for _ in range(LISTEN_BACKLOG):
            try:
                sock, _addr = self._listener.accept()
            except (BlockingIOError, InterruptedError):
                return
            except OSError as e:
                self._log.warning("accept failed (%s), binding again", e)
                self._close_listener()
                self._next_bind_at = now
                return
            try:
                sock.setblocking(False)
                sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            except OSError as e:
                self._log.warning("cannot configure a client socket (%s)", e)
                sock.close()
                continue
            handshake = _Handshake(sock, now)
            self._handshakes.append(handshake)
            self._advance_handshake(handshake, now)

    def _advance_handshake(self, handshake, now):
        """Read the upgrade request; once whole, answer it: a connection, or a refusal."""
        while b"\r\n\r\n" not in handshake.request:
            try:
                data = handshake.sock.recv(HANDSHAKE_RECV_SIZE)
            except (BlockingIOError, InterruptedError):
                return
            except OSError:
                self._drop_handshake(handshake)
                return
            if not data:
                self._drop_handshake(handshake)
                return
            handshake.request.extend(data)
            if len(handshake.request) > HANDSHAKE_MAX_BYTES:
                self._drop_handshake(handshake)
                return
        self._handshakes.remove(handshake)
        parsed = parse_http_request(handshake.request)
        if parsed is None:
            handshake.sock.close()
            return
        _method, _path, headers, leftover = parsed
        # A browser always sends Origin; the hub never does. Refusing it keeps
        # web pages open on this PC from driving Live through localhost.
        if not is_websocket_upgrade(headers) or "origin" in headers:
            try:
                handshake.sock.send(_BAD_REQUEST)
            except OSError as e:
                self._log.debug("refused request: %s", e)
            handshake.sock.close()
            return
        hello = encode_message({"event": "connect", "data": self.connect_data, "ts": now_ms()})
        conn = Connection(
            handshake.sock,
            handshake_response(headers) + hello,
            self._max,
            self._log,
            self.inbox,
            now,
        )
        self._connections.append(conn)
        self._log.warning("client connected (connection %s)", conn.id)
        if leftover:
            conn.take(leftover)
        conn.receive()

    def _drop_handshake(self, handshake):
        if handshake in self._handshakes:
            self._handshakes.remove(handshake)
        handshake.sock.close()

    def _reap(self):
        """Finished connections leave the list; the drain hears of each once."""
        for conn in [c for c in self._connections if c.finished]:
            self._connections.remove(conn)
            self.inbox.put((conn, None))
            self._log.warning("client disconnected (connection %s)", conn.id)
