"""A fake AbleSet for the E2E suite (#90 B3b), beside ``fake_companion.py``:
AbleSet's web app on this PC as the hub's proxy and link see it, plain
HTTP/1.1 with keep-alive, and a Socket.IO-like WebSocket echo:

    fake = FakeAbleSet(port)          # 0: any free port, see ``port``
    fake.start()
    fake.down()                       # AbleSet stopped: the listener and every
                                      # connection closed (a connect is refused)
    fake.up()                         # listening again on the same port
    fake.clear()                      # the hits forgotten
    fake.state()                      # port, down, sockets, presses, hits
    fake.stop()

Its routes:

    GET  /                    the page: an absolute-path script and stylesheet,
                              a button ``#fake-button`` and a long list
    GET  /static/app.js       the page's script: the ``AbleSetLocked`` it read
                              from ``localStorage`` at load as ``data-locked``
                              (``none``: nothing stored; #90), a ``browserId``
                              kept in ``localStorage``, the socket, the button
    GET  /static/app.css      the page's stylesheet
    GET  /static/pixel.gif    a 1 x 1 GIF
    POST /api/press           counted: ``{"pressed": <presses so far>}``
    GET  /socket.io/...       a WebSocket upgrade: Engine.IO's open packet
                              (``0{...}``), ``40`` answered ``40{...}``, ``2``
                              answered ``3``, anything else echoed as it came
    anything else             404

The harness runs it in a process of its own (``Child``: this file's tool, its
control lines ``down``, ``up``, ``clear``, ``state``, as ``child.py`` drives a
tool, #43): as a thread of the harness it shared the interpreter's lock, and
the harness's ``GET /hub/events`` held every connection of it.

Every request is a hit (``hit``): its method, target, ``Host``, user agent,
header names, cookie names, the target's ``browserId`` and whether it is the
hub's own link read (``link``: the hub reads ``/`` every 5 s as
``fohmixer-hub/<version>``); ``page_hits`` leaves those out. Stdlib only, one
asyncio loop in a thread of its own; every control call and failure is
printed on stderr (``fake_ableset: …``, the harness's log). Nothing is ever
force-ended: a connection is closed, the loop stopped.
"""

import argparse
import asyncio
import base64
import collections
import hashlib
import json
import os
import sys
import threading
import time
import urllib.parse

import child

# How long a control call waits for the loop.
CALL_S = 5.0
# How long start() waits for the listener.
START_S = 5.0
# The largest request head and body the fake reads.
MAX_HEAD = 65536
MAX_BODY = 1_048_576
# The largest WebSocket message the fake takes.
MAX_FRAME = 1_048_576
# The hits kept, newest last (the hub's link alone adds one every 5 s).
MAX_HITS = 2000
# The hub's own requests (its link) name it so.
LINK_AGENT = "fohmixer-hub/"
# RFC 6455's key suffix.
WS_GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
# Engine.IO v4's open packet, as AbleSet's Socket.IO server sends it first.
OPEN_PACKET = (
    '0{"sid":"fake","upgrades":[],"pingInterval":25000,"pingTimeout":20000,"maxPayload":1000000}'
)
CONNECTED = '40{"sid":"fake-ns"}'

PIXEL = (
    b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff!\xf9\x04\x01\x00\x00\x00\x00"
    b",\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;"
)
ITEMS = "".join(f"<li>Song {n}</li>" for n in range(1, 201))
PAGE = (
    "<!doctype html>\n"
    '<html lang="en"><head><meta charset="utf-8">'
    '<meta name="viewport" content="width=device-width, initial-scale=1">'
    # No icon to fetch: a 404 would be a console error.
    '<link rel="icon" href="data:,">'
    '<link rel="stylesheet" href="/static/app.css">'
    "<title>fake AbleSet</title></head>"
    '<body data-app="waiting">'
    '<button id="fake-button" type="button">Press</button>'
    f'<ol id="fake-list">{ITEMS}</ol>'
    '<script src="/static/app.js"></script>'
    "</body></html>\n"
)
STYLE = "#fake-button { outline: 3px solid rgb(1, 2, 3); }\n"
SCRIPT = """\
"use strict";
// The fake AbleSet's page (#90 B3b): loaded by an absolute path, so the
// hub must serve AbleSet's assets under AbleSet's name.
(() => {
  const body = document.body;
  // AbleSet 3.4.5 reads its lock once at load ("true": locked, #90): the
  // fake says what it read.
  const locked = localStorage.getItem("AbleSetLocked");
  body.dataset.locked = locked === null ? "none" : locked;
  let id = localStorage.getItem("browserId");
  if (!id) {
    id = "b" + Math.random().toString(36).slice(2, 12);
    localStorage.setItem("browserId", id);
  }
  body.dataset.browserId = id;
  const echo = '42["echo","' + id + '"]';
  const scheme = location.protocol === "https:" ? "wss:" : "ws:";
  const query = "EIO=4&transport=websocket&browserId=" + encodeURIComponent(id);
  const socket = new WebSocket(scheme + "//" + location.host + "/socket.io/?" + query);
  socket.onmessage = (event) => {
    const text = String(event.data);
    if (text.startsWith("0")) socket.send("40");
    else if (text.startsWith("40")) socket.send(echo);
    else if (text === echo) body.dataset.ws = "echo";
  };
  document.getElementById("fake-button").addEventListener("click", async () => {
    const answer = await fetch("/api/press", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ browserId: id }),
    });
    body.dataset.pressed = answer.ok ? "ok" : String(answer.status);
  });
  body.dataset.app = "loaded";
})();
"""
# The page says it may be framed by its own origin only, as a web app may:
# the hub replaces both with fohmixer's own framing.
OWN_FRAMING = [
    ("X-Frame-Options", "SAMEORIGIN"),
    ("Content-Security-Policy", "frame-ancestors 'self'"),
]
STATIC = {
    "/": ("text/html; charset=utf-8", PAGE.encode(), OWN_FRAMING),
    "/static/app.js": ("text/javascript; charset=utf-8", SCRIPT.encode(), []),
    "/static/app.css": ("text/css; charset=utf-8", STYLE.encode(), []),
    "/static/pixel.gif": ("image/gif", PIXEL, []),
}
REASONS = {
    101: "Switching Protocols",
    200: "OK",
    400: "Bad Request",
    404: "Not Found",
    411: "Length Required",
    413: "Content Too Large",
}

Request = collections.namedtuple("Request", ("method", "target", "version", "headers"))


def log(message):
    print(f"fake_ableset: {message}", file=sys.stderr, flush=True)


def parse_head(raw):
    """A request head (its bytes up to and with the blank line) as a
    ``Request`` (``headers``: (lowercase name, value) pairs in order), or
    None when it is no HTTP/1 request."""
    lines = raw.decode("latin-1").split("\r\n")
    parts = lines[0].split(" ")
    if len(parts) != 3 or not parts[2].startswith("HTTP/1."):
        return None
    headers = []
    for line in lines[1:]:
        if not line:
            continue
        name, colon, value = line.partition(":")
        if not colon or not name or name != name.strip():
            return None
        headers.append((name.lower(), value.strip()))
    return Request(parts[0], parts[1], parts[2], headers)


def header(request, name):
    """The first value of header ``name`` (lowercase), else None."""
    return next((v for n, v in request.headers if n == name), None)


def tokens(request, name):
    """The comma-separated tokens of every ``name`` header, lowercase."""
    return {t.strip().lower() for n, v in request.headers if n == name for t in v.split(",")}


def cookie_names(request):
    """The names of every cookie the request carries, in order."""
    pairs = (p.strip() for n, v in request.headers if n == "cookie" for p in v.split(";"))
    return [p.partition("=")[0] for p in pairs if p]


def hit(request, at_ms):
    """What the fake records of a request."""
    query = urllib.parse.parse_qs(urllib.parse.urlsplit(request.target).query)
    agent = header(request, "user-agent") or ""
    return {
        "method": request.method,
        "path": request.target,
        "host": header(request, "host"),
        "agent": agent,
        "link": agent.startswith(LINK_AGENT),
        "headers": sorted({n for n, _ in request.headers}),
        "cookies": cookie_names(request),
        "browser_id": query.get("browserId", [None])[0],
        "at": at_ms,
    }


def page_hits(hits):
    """``hits`` without the hub's own link reads."""
    return [h for h in hits if not h["link"]]


def keeps_alive(request):
    """Whether the connection stays open after the answer."""
    if "close" in tokens(request, "connection"):
        return False
    return request.version == "HTTP/1.1"


def is_websocket(request):
    """A WebSocket upgrade: a GET naming ``upgrade`` and ``websocket``."""
    return (
        request.method == "GET"
        and "upgrade" in tokens(request, "connection")
        and "websocket" in tokens(request, "upgrade")
    )


def accept_key(key):
    """``Sec-WebSocket-Accept`` of a ``Sec-WebSocket-Key``."""
    return base64.b64encode(hashlib.sha1((key + WS_GUID).encode()).digest()).decode()


def answer_head(status, fields):
    """A response head: its status line and ``fields``, then the blank line."""
    lines = [f"HTTP/1.1 {status} {REASONS[status]}"] + [f"{n}: {v}" for n, v in fields]
    return ("\r\n".join(lines) + "\r\n\r\n").encode()


def frame(opcode, payload):
    """An unmasked WebSocket frame, as a server sends it."""
    size = len(payload)
    if size < 126:
        length = bytes([size])
    elif size < 65536:
        length = bytes([126]) + size.to_bytes(2, "big")
    else:
        length = bytes([127]) + size.to_bytes(8, "big")
    return bytes([0x80 | opcode]) + length + payload


def reply(text):
    """The fake's answer to a client's text message (Engine.IO's packets)."""
    if text == "40":
        return CONNECTED
    if text == "2":
        return "3"
    return text


class FakeAbleSet:
    """The fake on ``port`` of ``host`` (0: any free port, see ``port``)."""

    def __init__(self, port, host="127.0.0.1"):
        self.host = host
        self.listen_port = port
        self.port = None
        self.loop = asyncio.new_event_loop()
        self.thread = threading.Thread(target=self._run, name="fake-ableset", daemon=True)
        self.listening = threading.Event()
        self.failure = None
        self.server = None
        # Loop state: only touched on the loop.
        self.writers = set()
        self.tasks = set()
        self.sockets = 0
        self.presses = 0
        self.hits = collections.deque(maxlen=MAX_HITS)
        self.is_down = False

    def start(self):
        """Starts the loop and the listener; raises when it cannot listen."""
        self.thread.start()
        if not self.listening.wait(START_S):
            raise RuntimeError(f"the fake AbleSet did not listen within {START_S} s")
        if self.failure is not None:
            raise RuntimeError(f"the fake AbleSet cannot listen: {self.failure!r}")
        log(f"listening on {self.host}:{self.port}")

    def _run(self):
        asyncio.set_event_loop(self.loop)
        try:
            self.server = self.loop.run_until_complete(self._listen(self.listen_port))
        except OSError as e:
            self.failure = e
            self.listening.set()
            self.loop.close()
            return
        self.port = self.server.sockets[0].getsockname()[1]
        self.listening.set()
        self.loop.run_forever()
        self.loop.close()

    async def _listen(self, port):
        return await asyncio.start_server(self._accept, self.host, port, limit=MAX_HEAD)

    def _call(self, fn, *args):
        async def call():
            answer = fn(*args)
            if asyncio.iscoroutine(answer):
                answer = await answer
            return answer

        return asyncio.run_coroutine_threadsafe(call(), self.loop).result(CALL_S)

    # The control calls (any thread).

    def down(self):
        """AbleSet stopped: the listener and every connection closed."""
        answer = self._call(self._down)
        log(f"down: {self._summary(answer)}")
        return answer

    def up(self):
        """Listening again on the same port."""
        answer = self._call(self._up)
        log(f"up: {self._summary(answer)}")
        return answer

    def clear(self):
        """Forgets the hits."""
        return self._call(self._clear)

    def state(self):
        """``port``, ``down``, ``sockets`` (open WebSockets), ``presses`` and
        ``hits`` (oldest first, ``hit``)."""
        return self._call(self._state)

    def stop(self):
        """Closes every connection and the listener, ends the loop."""
        if not self.thread.is_alive():
            return

        async def close():
            await self._down()
            tasks = list(self.tasks)
            for task in tasks:
                task.cancel()
            await asyncio.gather(*tasks, return_exceptions=True)

        asyncio.run_coroutine_threadsafe(close(), self.loop).result(CALL_S)
        self.loop.call_soon_threadsafe(self.loop.stop)
        self.thread.join(CALL_S)
        log("stopped")

    @staticmethod
    def _summary(state):
        return {k: state[k] for k in ("port", "down", "sockets")}

    # On the loop.

    async def _down(self):
        self.is_down = True
        if self.server is not None:
            self.server.close()
        writers = list(self.writers)
        for writer in writers:
            writer.close()
        for writer in writers:
            try:
                await writer.wait_closed()
            except (ConnectionError, OSError) as e:
                log(f"a connection closed with an error: {e!r}")
        if self.server is not None:
            await self.server.wait_closed()
            self.server = None
        return self._state()

    async def _up(self):
        if self.server is None:
            self.server = await self._listen(self.port)
        self.is_down = False
        return self._state()

    def _clear(self):
        self.hits.clear()
        return self._state()

    def _state(self):
        return {
            "port": self.port,
            "down": self.is_down,
            "sockets": self.sockets,
            "presses": self.presses,
            "hits": [dict(h) for h in self.hits],
        }

    async def _accept(self, reader, writer):
        self.writers.add(writer)
        self.tasks.add(asyncio.current_task())
        try:
            await self._serve(reader, writer)
        except (ConnectionError, OSError, asyncio.IncompleteReadError) as e:
            log(f"a connection ended: {e!r}")
        except asyncio.LimitOverrunError:
            log("a request head over its limit: connection closed")
        finally:
            self.writers.discard(writer)
            self.tasks.discard(asyncio.current_task())
            writer.close()

    async def _serve(self, reader, writer):
        while True:
            try:
                raw = await reader.readuntil(b"\r\n\r\n")
            except asyncio.IncompleteReadError as e:
                if e.partial:
                    log(f"a connection ended inside a request head: {len(e.partial)} bytes")
                return
            request = parse_head(raw)
            if request is None:
                writer.write(answer_head(400, [("Content-Length", "0"), ("Connection", "close")]))
                await writer.drain()
                return
            self.hits.append(hit(request, time.time() * 1000.0))
            if is_websocket(request):
                await self._websocket(request, reader, writer)
                return
            if not await self._answer(request, reader, writer):
                return

    async def _answer(self, request, reader, writer):
        """Answers one plain request; whether the connection stays open."""
        if header(request, "transfer-encoding") is not None:
            await self._send(writer, 411, "text/plain", b"a body needs a Content-Length\n", False)
            return False
        length = header(request, "content-length") or "0"
        if not length.isdigit() or int(length) > MAX_BODY:
            await self._send(writer, 413, "text/plain", b"no such body\n", False)
            return False
        body = await reader.readexactly(int(length))
        alive = keeps_alive(request)
        path = urllib.parse.urlsplit(request.target).path
        if request.method == "POST" and path == "/api/press":
            self.presses += 1
            log(f"press {self.presses}: {body[:200]!r}")
            answer = json.dumps({"pressed": self.presses}).encode()
            await self._send(writer, 200, "application/json", answer, alive)
        elif request.method == "GET" and path in STATIC:
            content_type, data, fields = STATIC[path]
            await self._send(writer, 200, content_type, data, alive, fields)
        else:
            await self._send(writer, 404, "text/plain", b"not found\n", alive)
        return alive

    async def _send(self, writer, status, content_type, body, alive, extra=()):
        fields = [
            ("Content-Type", content_type),
            ("Content-Length", str(len(body))),
            ("Cache-Control", "no-store"),
            *extra,
        ]
        if not alive:
            fields.append(("Connection", "close"))
        writer.write(answer_head(status, fields) + body)
        await writer.drain()

    async def _websocket(self, request, reader, writer):
        """The handshake, the open packet, then each message answered until a
        close (either side's) or the connection's end."""
        key = header(request, "sec-websocket-key")
        if key is None:
            await self._send(writer, 400, "text/plain", b"no Sec-WebSocket-Key\n", False)
            return
        fields = [
            ("Upgrade", "websocket"),
            ("Connection", "Upgrade"),
            ("Sec-WebSocket-Accept", accept_key(key)),
        ]
        writer.write(answer_head(101, fields) + frame(0x1, OPEN_PACKET.encode()))
        await writer.drain()
        self.sockets += 1
        try:
            while True:
                opcode, payload = await self._read_frame(reader)
                if opcode == 0x8:
                    writer.write(frame(0x8, payload[:2]))
                    await writer.drain()
                    return
                if opcode == 0x9:
                    writer.write(frame(0xA, payload))
                elif opcode == 0x1:
                    writer.write(frame(0x1, reply(payload.decode()).encode()))
                elif opcode == 0x2:
                    writer.write(frame(0x2, payload))
                await writer.drain()
        finally:
            self.sockets -= 1

    async def _read_frame(self, reader):
        """One whole client frame: its opcode and unmasked payload. A
        fragment or a frame over ``MAX_FRAME`` ends the connection."""
        first, second = await reader.readexactly(2)
        size = second & 0x7F
        if size == 126:
            size = int.from_bytes(await reader.readexactly(2), "big")
        elif size == 127:
            size = int.from_bytes(await reader.readexactly(8), "big")
        if not first & 0x80 or size > MAX_FRAME:
            raise ConnectionError(f"a frame the fake does not take: {first:#x}, {size} bytes")
        mask = await reader.readexactly(4) if second & 0x80 else bytes(4)
        data = await reader.readexactly(size)
        return first & 0x0F, bytes(b ^ mask[i % 4] for i, b in enumerate(data))


class Child(child.Child):
    """The fake in a process of its own (``child.py``, #43): this file's tool,
    with the calls of ``FakeAbleSet`` (``start``, ``port``, ``down``, ``up``,
    ``clear``, ``state``, ``stop``)."""

    def __init__(self, port, host="127.0.0.1"):
        tool = [sys.executable, os.path.abspath(__file__), "--port", str(port), "--host", host]
        super().__init__("the fake AbleSet", "FAKE_ABLESET", tool)

    def down(self):
        return self.ask("down")

    def up(self):
        return self.ask("up")

    def clear(self):
        return self.ask("clear")

    def state(self):
        return self.ask("state")


def command(fake, line):
    """One control line of the tool: its answer, a dict (an ``error`` for a
    line it does not know)."""
    calls = {"down": fake.down, "up": fake.up, "clear": fake.clear, "state": fake.state}
    call = calls.get(" ".join(line.split()))
    if call is None:
        log(f"unknown line {line.strip()!r}")
        return {"error": f"unknown line {line.strip()!r}: {' | '.join(calls)}"}
    return call()


def main(argv=None, stdin=None, stdout=None):
    parser = argparse.ArgumentParser(description="the fake AbleSet of the E2E suite")
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--host", default="127.0.0.1")
    args = parser.parse_args(argv)
    fake = FakeAbleSet(args.port, host=args.host)
    return child.serve(fake, "FAKE_ABLESET", command, stdin or sys.stdin, stdout or sys.stdout, log)


if __name__ == "__main__":
    sys.exit(main())
