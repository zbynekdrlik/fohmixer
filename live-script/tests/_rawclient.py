"""A WebSocket client on a plain socket, for tests that must see exactly what
the script writes and when: it reads only when the test says so (#5)."""

import base64
import json
import os
import select
import socket
import struct
import time

import _paths  # noqa: F401 - puts the script on sys.path
from FohMixer.transport.websocket import OPCODE_TEXT, try_read_frame

# RFC 6455's close opcode, and its close frame with no body (FIN, opcode 0x8,
# unmasked, length 0), as a client must see them: pinned here, not taken
# from the script under test.
CLOSE = 0x8
EMPTY_CLOSE_FRAME = b"\x88\x00"


def upgrade_request(port):
    key = base64.b64encode(os.urandom(16)).decode("ascii")
    return (
        f"GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\n"
        f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
    ).encode("ascii")


def masked_frame(opcode, payload=b""):
    """A client frame (clients always mask)."""
    mask = os.urandom(4)
    masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    if len(payload) < 126:
        header = struct.pack("!BB", 0x80 | opcode, 0x80 | len(payload))
    elif len(payload) < 65536:
        header = struct.pack("!BBH", 0x80 | opcode, 0x80 | 126, len(payload))
    else:
        header = struct.pack("!BBQ", 0x80 | opcode, 0x80 | 127, len(payload))
    return header + mask + masked


def request_frame(uuid, commands=()):
    """A request envelope as a client text frame."""
    text = json.dumps({"uuid": uuid, "commands": list(commands)})
    return masked_frame(OPCODE_TEXT, text.encode("utf-8"))


class RawClient:
    """Sends the upgrade (and ``extra`` bytes right behind it), then reads only
    when the test says so. ``rcvbuf`` shrinks its receive window (a client that
    stops reading)."""

    def __init__(self, port, rcvbuf=None, extra=b""):
        self.sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        if rcvbuf is not None:
            self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, rcvbuf)
        self.sock.connect(("127.0.0.1", port))
        self.sock.sendall(upgrade_request(port) + extra)
        self.buffer = bytearray()
        self.response = None
        self.closed = False

    def close(self):
        self.sock.close()

    def readable(self, timeout=0.0):
        return bool(select.select([self.sock], [], [], timeout)[0])

    def read_available(self):
        """Everything the socket holds right now; ``closed`` once it ended."""
        self.sock.setblocking(False)
        try:
            while True:
                try:
                    data = self.sock.recv(1 << 20)
                except BlockingIOError:
                    return
                except ConnectionResetError:
                    data = b""
                if not data:
                    self.closed = True
                    return
                self.buffer.extend(data)
        finally:
            self.sock.setblocking(True)

    def messages(self):
        """The messages complete in the buffer: text frames as JSON, others as
        ``{"opcode": n, "head": bytes, "payload": bytes}``, ``head`` being the
        frame's first two bytes as sent (FIN, RSV, opcode, mask bit, length).
        The HTTP response head is kept in ``response``."""
        if self.response is None:
            end = self.buffer.find(b"\r\n\r\n")
            if end < 0:
                return []
            self.response = bytes(self.buffer[:end])
            del self.buffer[: end + 4]
        out = []
        while True:
            head = bytes(self.buffer[:2])
            frame = try_read_frame(self.buffer)
            if frame is None:
                return out
            opcode, _fin, payload = frame
            if opcode == OPCODE_TEXT:
                out.append(json.loads(bytes(payload).decode("utf-8")))
            else:
                out.append({"opcode": opcode, "head": head, "payload": bytes(payload)})

    def read_messages(self, count, timeout=10.0):
        """The next ``count`` messages, waiting for them."""
        deadline = time.monotonic() + timeout
        out = []
        while len(out) < count:
            out.extend(self.messages())
            if len(out) >= count:
                break
            remaining = deadline - time.monotonic()
            if remaining <= 0 or self.closed:
                raise AssertionError(f"{len(out)} of {count} messages; closed={self.closed}")
            self.readable(min(remaining, 0.05))
            self.read_available()
        return out

    def read_until_closed(self, timeout=2.0):
        """The messages still to come until the server ends the connection. The
        ones it sent before resetting the socket are all kept: `websockets`'
        sync client drops those when its close reply fails on the reset (#5)."""
        deadline = time.monotonic() + timeout
        while not self.closed:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise AssertionError(f"not closed in {timeout} s: {self.messages()}")
            self.readable(remaining)
            self.read_available()
        return self.messages()
