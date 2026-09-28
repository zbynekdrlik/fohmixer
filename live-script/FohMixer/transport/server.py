# Derived from leolabs/ableton-js v5.0.3 @69de331, midi-script/Socket.py.
# Copyright (c) 2022 Leo Bernard. MIT License, see LICENSE-ableton-js.
# Kept: the accept thread, one reader thread per connection, the frame loop.
# Changed (design §3.7, §3.9): the main thread never sends (a sender thread per
# connection drains a bounded result queue and a latest-value map), no static
# server, no auth, no port files, SO_EXCLUSIVEADDRUSE on Windows, and a close
# that finishes the WebSocket close handshake and then resets the connection so
# the port can be bound again at once (Windows keeps an exclusive port busy
# while accepted connections are still closing).
"""Threaded localhost WebSocket server with a non-blocking send path.

Threads (all daemon): one accept thread; per connection one reader thread
(handshake, frames, JSON, envelope check) and one sender thread. Well-formed
requests go to ``inbox`` as ``(connection, payload)``; ``(connection, None)``
tells the main thread a connection is gone. The main thread only calls the
``push_*`` / ``send_*`` methods, which append under a short lock and notify.
"""

import collections
import contextlib
import itertools
import json
import logging
import os
import queue
import socket
import struct
import threading
import time

from .websocket import (
    OPCODE_CLOSE,
    OPCODE_CONTINUATION,
    OPCODE_PING,
    OPCODE_PONG,
    OPCODE_TEXT,
    complete_websocket_handshake,
    encode_close_frame,
    encode_pong_frame,
    encode_text_frame,
    is_websocket_upgrade,
    read_http_request,
    to_bytes,
    try_read_frame,
)

SOCKET_TIMEOUT_S = 3.0
CLOSE_HANDSHAKE_TIMEOUT_S = 1.0
ACCEPT_POLL_S = 0.5
BIND_RETRY_DELAYS_S = (0.25, 0.5, 1.0, 2.0, 5.0)
MAX_MESSAGE_BYTES = 16 * 1024 * 1024
RECV_SIZE = 65536
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
    """One WebSocket client. ``push_*``/``send_*`` never block; a sender thread writes.

    - ``results``: a bounded queue of result/event messages, in order; going past
      ``result_queue_max`` closes the connection (the hub resyncs);
    - ``values``: the latest item per key, sent as one ``values`` frame;
    - ``heartbeat``: the latest heartbeat only.
    """

    _ids = itertools.count(1)

    def __init__(self, sock, result_queue_max, logger):
        self.id = next(Connection._ids)
        self._sock = sock
        self._max = result_queue_max
        self._log = logger
        self._cond = threading.Condition(threading.RLock())
        self._out = collections.deque()
        self._values = {}
        self._heartbeat = None
        self._state = OPEN
        self._batch_depth = 0
        self._close_wait = threading.Event()
        self.finished = threading.Event()
        self._sender = threading.Thread(
            target=self._send_loop, name=f"fohmixer-send-{self.id}", daemon=True
        )

    def start(self):
        self._sender.start()

    @property
    def is_open(self):
        return self._state == OPEN

    def pending(self):
        """``(queued results/events, pending value keys)``, for diagnostics and tests."""
        with self._cond:
            return len(self._out), len(self._values)

    # --- called from any thread (the main thread included): never blocks on I/O ---

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
        with self._cond:
            if self._state != OPEN:
                return False
            self._values[key] = item
            self._wake()
        return True

    def push_values(self, items):
        with self._cond:
            if self._state != OPEN:
                return False
            self._values.update(items)
            self._wake()
        return True

    def set_heartbeat(self, data):
        with self._cond:
            if self._state != OPEN:
                return False
            self._heartbeat = {"event": "heartbeat", "data": data, "ts": now_ms()}
            self._wake()
        return True

    @contextlib.contextmanager
    def batch(self):
        """Group several pushes so the sender wakes once, after the last one."""
        with self._cond:
            self._batch_depth += 1
            try:
                yield self
            finally:
                self._batch_depth -= 1
                self._wake()

    def close(self):
        """Graceful: send what is queued, a close frame, then close."""
        with self._cond:
            if self._state == OPEN:
                self._state = CLOSING
                self._cond.notify()

    def abort(self):
        """At once: drop what is queued and reset the connection."""
        with self._cond:
            if self._state == CLOSED:
                return
            self._state = CLOSED
            self._cond.notify()
        self._close_wait.set()
        self._shutdown_socket()

    def peer_closed(self):
        """The reader saw the peer's close frame or end of stream."""
        self._close_wait.set()

    # --- internals ---

    def _wake(self):
        if self._batch_depth == 0:
            self._cond.notify()

    def _enqueue(self, message):
        with self._cond:
            if self._state != OPEN:
                return False
            if len(self._out) < self._max:
                self._out.append(message)
                self._wake()
                return True
        self._log.warning(
            "connection %s: result queue full (%s messages), closing it", self.id, self._max
        )
        self.abort()
        return False

    def _send_loop(self):
        try:
            while True:
                with self._cond:
                    while self._state == OPEN and not (
                        self._out or self._values or self._heartbeat
                    ):
                        self._cond.wait()
                    if self._state == CLOSED:
                        return
                    messages = list(self._out)
                    self._out.clear()
                    heartbeat, self._heartbeat = self._heartbeat, None
                    values, self._values = self._values, {}
                    closing = self._state == CLOSING
                for message in messages:
                    frame = message if isinstance(message, bytes) else encode_message(message)
                    self._sock.sendall(frame)
                if heartbeat is not None:
                    self._sock.sendall(encode_message(heartbeat))
                if values:
                    data = list(values.values())
                    self._sock.sendall(
                        encode_message({"event": "values", "data": data, "ts": now_ms()})
                    )
                if closing:
                    self._sock.sendall(encode_close_frame())
                    self._close_wait.wait(CLOSE_HANDSHAKE_TIMEOUT_S)
                    return
        except OSError as e:
            if self._state == OPEN:
                self._log.warning("connection %s: send failed (%s), closing it", self.id, e)
        except Exception:  # noqa: BLE001 - logged with its traceback, then the connection closes
            self._log.exception("connection %s: sender error, closing it", self.id)
        finally:
            self._finish()

    def _finish(self):
        with self._cond:
            self._state = CLOSED
        self._shutdown_socket()
        try:
            self._sock.close()
        except OSError as e:
            self._log.debug("connection %s: close: %s", self.id, e)
        self.finished.set()

    def _shutdown_socket(self):
        """Reset instead of lingering in TIME_WAIT, and wake blocked recv/send calls."""
        try:
            self._sock.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, _LINGER_ABORT)
            self._sock.shutdown(socket.SHUT_RDWR)
        except OSError as e:
            self._log.debug("connection %s: shutdown: %s", self.id, e)


class Server:
    """Accepts WebSocket clients on ``host:port`` and feeds ``inbox``.

    ``connect_data`` is sent as the ``connect`` event, the first frame of every
    connection. A failed bind is retried (0.25 s, 0.5 s, 1 s, 2 s, then every
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
        self._lock = threading.Lock()
        self._connections = []
        self._listener = None
        self._running = False
        self._stop = threading.Event()
        self._bound = threading.Event()
        self._thread = None

    @property
    def port(self):
        return self._port

    def start(self):
        self._running = True
        self._thread = threading.Thread(target=self._serve, name="fohmixer-accept", daemon=True)
        self._thread.start()

    def wait_bound(self, timeout=None):
        return self._bound.wait(timeout)

    def connections(self):
        with self._lock:
            return list(self._connections)

    def broadcast(self, event, data=None):
        for conn in self.connections():
            conn.send_event(event, data)

    def broadcast_heartbeat(self, data):
        for conn in self.connections():
            conn.set_heartbeat(data)

    def shutdown(self, grace_s=0.3):
        """Stop listening, close every connection (graceful up to ``grace_s``, then reset).

        Returns once the listening socket is closed and every connection is
        finished or reset, so a new server can bind the same port at once.
        """
        self._running = False
        self._stop.set()
        self._close_listener()
        if self._thread is not None and self._thread is not threading.current_thread():
            self._thread.join(ACCEPT_POLL_S + 0.5)
        conns = self.connections()
        for conn in conns:
            conn.close()
        deadline = time.monotonic() + grace_s
        for conn in conns:
            conn.finished.wait(max(0.0, deadline - time.monotonic()))
        stragglers = [conn for conn in conns if not conn.finished.is_set()]
        for conn in stragglers:
            conn.abort()
        deadline = time.monotonic() + grace_s
        for conn in stragglers:
            conn.finished.wait(max(0.0, deadline - time.monotonic()))

    # --- accept thread ---

    def _serve(self):
        attempt = 0
        while self._running:
            try:
                listener = self._bind()
            except OSError as e:
                message = f"cannot bind {self.host}:{self._port}: {e}"
                if message != self.bind_error:
                    self._log.warning("%s; retrying", message)
                self.bind_error = message
                delay = BIND_RETRY_DELAYS_S[min(attempt, len(BIND_RETRY_DELAYS_S) - 1)]
                attempt += 1
                if self._stop.wait(delay):
                    return
                continue
            if self.bind_error is not None:
                self._log.warning("bound %s:%s after retrying", self.host, self._port)
            self.bind_error = None
            attempt = 0
            self._accept_loop(listener)

    def _bind(self):
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            exclusive = getattr(socket, "SO_EXCLUSIVEADDRUSE", None)
            if os.name == "nt" and exclusive is not None:
                sock.setsockopt(socket.SOL_SOCKET, exclusive, 1)
            else:
                sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            sock.bind((self.host, self._port))
            sock.listen(16)
            sock.settimeout(ACCEPT_POLL_S)
        except OSError:
            sock.close()
            raise
        self._port = sock.getsockname()[1]
        with self._lock:
            self._listener = sock
        self._bound.set()
        return sock

    def _accept_loop(self, listener):
        while self._running:
            try:
                sock, _addr = listener.accept()
            except TimeoutError:
                continue
            except OSError as e:
                if self._running:
                    self._log.warning("accept failed (%s), binding again", e)
                    self._close_listener()
                return
            try:
                sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                sock.settimeout(SOCKET_TIMEOUT_S)
            except OSError as e:
                self._log.warning("cannot configure a client socket (%s)", e)
                sock.close()
                continue
            threading.Thread(
                target=self._handle, args=(sock,), name="fohmixer-read", daemon=True
            ).start()

    def _close_listener(self):
        with self._lock:
            listener, self._listener = self._listener, None
        if listener is None:
            return
        try:
            listener.shutdown(socket.SHUT_RDWR)
        except OSError as e:
            self._log.debug("listener shutdown: %s", e)
        listener.close()

    # --- reader thread (one per connection) ---

    def _handle(self, sock):
        parsed = read_http_request(sock)
        if parsed is None:
            sock.close()
            return
        _method, _path, headers, leftover = parsed
        # A browser always sends Origin; the hub never does. Refusing it keeps
        # web pages open on this PC from driving Live through localhost.
        if not is_websocket_upgrade(headers) or "origin" in headers:
            try:
                sock.sendall(_BAD_REQUEST)
            except OSError as e:
                self._log.debug("refused request: %s", e)
            sock.close()
            return
        if not complete_websocket_handshake(sock, headers):
            sock.close()
            return
        conn = Connection(sock, self._max, self._log)
        conn.send_event("connect", self.connect_data)
        with self._lock:
            accepted = self._running
            if accepted:
                self._connections.append(conn)
        conn.start()
        if not accepted:
            conn.abort()
            return
        self._log.warning("client connected (connection %s)", conn.id)
        try:
            self._read_loop(conn, sock, leftover)
        except Exception:  # noqa: BLE001 - logged with its traceback, then the connection closes
            self._log.exception("connection %s: reader error", conn.id)
            conn.abort()
        finally:
            with self._lock:
                if conn in self._connections:
                    self._connections.remove(conn)
            self.inbox.put((conn, None))
            self._log.warning("client disconnected (connection %s)", conn.id)

    def _read_loop(self, conn, sock, leftover):
        buffer = bytearray(to_bytes(leftover))
        fragments = bytearray()
        fragment_opcode = None
        while True:
            try:
                data = sock.recv(RECV_SIZE)
            except TimeoutError:
                if conn.finished.is_set():
                    return
                continue
            except OSError:
                conn.peer_closed()
                conn.abort()
                return
            if not data:
                conn.peer_closed()
                conn.abort()
                return
            buffer.extend(data)
            if len(buffer) + len(fragments) > MAX_MESSAGE_BYTES:
                self._log.warning("connection %s: message too large, closing it", conn.id)
                conn.abort()
                return
            while True:
                frame = try_read_frame(buffer)
                if frame is None:
                    break
                opcode, fin, payload = frame
                if opcode == OPCODE_CLOSE:
                    conn.close()
                    conn.peer_closed()
                    return
                if opcode == OPCODE_PING:
                    conn.send_frame(encode_pong_frame(payload))
                    continue
                if opcode == OPCODE_PONG:
                    continue
                if opcode == OPCODE_CONTINUATION:
                    if fragment_opcode is None:
                        continue
                    fragments.extend(payload)
                    if fin:
                        self._handle_payload(fragment_opcode, fragments, conn)
                        fragments = bytearray()
                        fragment_opcode = None
                    continue
                if not fin:
                    fragment_opcode = opcode
                    fragments = bytearray(payload)
                    continue
                self._handle_payload(opcode, payload, conn)

    def _handle_payload(self, opcode, payload, conn):
        if opcode != OPCODE_TEXT:
            conn.send_error(None, "binary frames are not supported")
            return
        try:
            message = json.loads(bytes(payload).decode("utf-8"))
        except (UnicodeDecodeError, ValueError) as e:
            conn.send_error(None, f"invalid JSON: {e}")
            return
        uuid = message.get("uuid") if isinstance(message, dict) else None
        if not isinstance(message, dict) or not isinstance(message.get("commands"), list):
            conn.send_error(uuid, "missing or invalid commands array")
            return
        self.inbox.put((conn, message))
