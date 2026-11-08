"""A fake Presenter for the E2E suite (#90, the stage tab), beside
``fake_ableset.py``: Presenter's server as the hub's link and proxy see it
(its source: ``presenter-server/src/router/stage.rs``,
``presenter-core/src/live.rs``), plain HTTP/1.1 with keep-alive:

    fake = FakePresenter(port)        # 0: any free port, see ``port``
    fake.start()
    fake.down()                       # Presenter stopped: the listener and
                                      # every connection closed
    fake.up()                         # listening again on the same port
    fake.switch(code)                 # the operator switches in Presenter
    fake.clear()                      # the hits forgotten
    fake.state()                      # port, down, current, sockets, hits
    fake.stop()

Its routes:

    GET  /stage                 the stage page: ``/ui-pkg/stage.js`` by an
                                absolute path, the layout on stage in
                                ``#fake-stage`` and ``data-layout``
    GET  /ui-pkg/stage.js       the page's script: ``GET /stage/layout``, then
                                ``/live/ws?surface=stage&preview=1``, each
                                ``stage_layout`` event into the page
    GET  /stage-displays        ``[{code, name, description}]`` (``LAYOUTS``)
    GET  /stage/layout          ``{code, layout}``
    POST /stage/layout          ``{"code"}``: 400 empty, 404 unknown, else the
                                switch (an event to every socket) and
                                ``{code, layout}``
    POST anything else          200, counted (a write the hub must never pass)
    GET  /live/ws               a WebSocket: ``stage_layout`` events; pings
                                answered
    anything else               404

The layout names are invented (no site data). The harness runs it in a
process of its own (``Child``: this file's tool, its control lines ``down``,
``up``, ``switch <code>``, ``clear``, ``state``, as ``child.py`` drives a
tool). Stdlib only, one asyncio loop in a thread of its own; every control
call and failure is printed on stderr (``fake_presenter: …``). Nothing is
ever force-ended.
"""

import argparse
import asyncio
import collections
import json
import os
import sys
import threading
import time
import urllib.parse

import child
from fake_ableset import (
    MAX_BODY,
    MAX_HEAD,
    accept_key,
    answer_head,
    frame,
    header,
    is_websocket,
    keeps_alive,
    parse_head,
)

CALL_S = 5.0
START_S = 5.0
MAX_FRAME = 65536
MAX_HITS = 2000
LINK_AGENT = "fohmixer-hub/"

# Presenter's nine layouts by kind, the names invented.
LAYOUTS = [
    ("worship", "WORSHIP", "Lyrics, the current and the next line"),
    ("overview", "OVERVIEW", "Lyrics and the slides beside them"),
    ("timer", "TIMER", "Countdown to the start"),
    ("preach", "PREACH", "Stopwatch for the speaker"),
    ("video", "VIDEO", "Full-screen video"),
    ("bible", "BIBLE", "Bible passage"),
    ("fulltext", "FULL TEXT", "The whole slide text"),
    ("api", "API", "External lyrics"),
    ("api-video", "API + VIDEO", "Video with lyrics over it"),
]
FIRST = "worship"

PAGE = (
    "<!doctype html>\n"
    '<html lang="en"><head><meta charset="utf-8">'
    '<link rel="icon" href="data:,">'
    "<title>fake stage</title>"
    "<style>body{margin:0;background:#000;color:#fff;font:96px sans-serif}</style>"
    '</head><body data-app="waiting">'
    '<div id="fake-stage">-</div>'
    '<script src="/ui-pkg/stage.js"></script>'
    "</body></html>\n"
)
SCRIPT = """\
"use strict";
// The fake stage page (#90): its calls and its socket at root paths of its
// own origin, as Presenter's stage page makes them.
(() => {
  const body = document.body;
  const show = (code) => {
    body.dataset.layout = code;
    document.getElementById("fake-stage").textContent = code;
  };
  fetch("/stage/layout")
    .then((answer) => answer.json())
    .then((answer) => {
      show(answer.code);
      body.dataset.app = "loaded";
    });
  const scheme = location.protocol === "https:" ? "wss:" : "ws:";
  const socket = new WebSocket(scheme + "//" + location.host + "/live/ws?surface=stage&preview=1");
  socket.onopen = () => {
    body.dataset.ws = "open";
  };
  socket.onmessage = (event) => {
    const data = JSON.parse(String(event.data));
    if (data.type === "stage_layout") show(data.code);
  };
})();
"""


def log(message):
    print(f"fake_presenter: {message}", file=sys.stderr, flush=True)


def layout_json(code):
    """``{code, name, description}`` of a listed code, else None."""
    for c, name, description in LAYOUTS:
        if c == code:
            return {"code": c, "name": name, "description": description}
    return None


def switch_answer(body):
    """``POST /stage/layout``'s status and answer for a request ``body``
    (bytes), and the code on stage after it (None: unchanged)."""
    try:
        code = str(json.loads(body or b"{}").get("code", "")).strip()
    except (ValueError, AttributeError):
        return 400, {"error": "no JSON object"}, None
    if not code:
        return 400, {"error": "code cannot be empty"}, None
    layout = layout_json(code)
    if layout is None:
        return 404, {"error": f"unknown stage layout {code}"}, None
    return 200, {"code": code, "layout": layout}, code


class FakePresenter:
    """The fake on ``port`` of ``host`` (0: any free port, see ``port``)."""

    def __init__(self, port, host="127.0.0.1"):
        self.host = host
        self.listen_port = port
        self.port = None
        self.loop = asyncio.new_event_loop()
        self.thread = threading.Thread(target=self._run, name="fake-presenter", daemon=True)
        self.listening = threading.Event()
        self.failure = None
        self.server = None
        # Loop state: only touched on the loop.
        self.writers = set()
        self.tasks = set()
        self.sockets = set()
        self.current = FIRST
        self.hits = collections.deque(maxlen=MAX_HITS)
        self.is_down = False

    def start(self):
        self.thread.start()
        if not self.listening.wait(START_S):
            raise RuntimeError(f"the fake Presenter did not listen within {START_S} s")
        if self.failure is not None:
            raise RuntimeError(f"the fake Presenter cannot listen: {self.failure!r}")
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
        answer = self._call(self._down)
        log(f"down: {self._summary(answer)}")
        return answer

    def up(self):
        answer = self._call(self._up)
        log(f"up: {self._summary(answer)}")
        return answer

    def switch(self, code):
        """The operator's switch in Presenter: ``code`` on stage, an event to
        every socket (an unknown code refused, as Presenter does)."""
        answer = self._call(self._switch, code)
        log(f"switch {code}: {self._summary(answer)}")
        return answer

    def clear(self):
        return self._call(self._clear)

    def state(self):
        """``port``, ``down``, ``current``, ``sockets`` (open), ``hits``."""
        return self._call(self._state)

    def stop(self):
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

    def _summary(self, state):
        return {k: state[k] for k in ("port", "down", "current", "sockets") if k in state}

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

    async def _switch(self, code):
        if layout_json(code) is None:
            return {"error": f"unknown stage layout {code}"}
        await self._on_stage(code)
        return self._state()

    async def _on_stage(self, code):
        self.current = code
        event = frame(0x1, json.dumps({"type": "stage_layout", "code": code}).encode())
        for writer in list(self.sockets):
            try:
                writer.write(event)
                await writer.drain()
            except (ConnectionError, OSError) as e:
                log(f"an event did not go: {e!r}")

    def _clear(self):
        self.hits.clear()
        return self._state()

    def _state(self):
        return {
            "port": self.port,
            "down": self.is_down,
            "current": self.current,
            "sockets": len(self.sockets),
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
            self.sockets.discard(writer)
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
            agent = header(request, "user-agent") or ""
            path = urllib.parse.urlsplit(request.target).path
            self.hits.append(
                {
                    "method": request.method,
                    "path": path,
                    "link": agent.startswith(LINK_AGENT),
                    "at": time.time() * 1000.0,
                }
            )
            if is_websocket(request) and path == "/live/ws":
                await self._websocket(request, reader, writer)
                return
            if not await self._answer(request, reader, writer, path):
                return

    async def _answer(self, request, reader, writer, path):
        length = header(request, "content-length") or "0"
        if header(request, "transfer-encoding") is not None or not length.isdigit():
            await self._send(writer, 400, "text/plain", b"no such body\n", False)
            return False
        if int(length) > MAX_BODY:
            await self._send(writer, 400, "text/plain", b"too large\n", False)
            return False
        body = await reader.readexactly(int(length))
        alive = keeps_alive(request)
        if request.method == "POST" and path == "/stage/layout":
            status, answer, code = switch_answer(body)
            if code is not None:
                await self._on_stage(code)
            await self._json(writer, status, answer, alive)
        elif request.method == "POST":
            log(f"a write {path} came: {body[:200]!r}")
            await self._json(writer, 200, {"written": path}, alive)
        elif request.method == "GET" and path == "/stage":
            await self._send(writer, 200, "text/html; charset=utf-8", PAGE.encode(), alive)
        elif request.method == "GET" and path == "/ui-pkg/stage.js":
            await self._send(writer, 200, "text/javascript", SCRIPT.encode(), alive)
        elif request.method == "GET" and path == "/stage-displays":
            await self._json(writer, 200, [layout_json(c) for c, _, _ in LAYOUTS], alive)
        elif request.method == "GET" and path == "/stage/layout":
            answer = {"code": self.current, "layout": layout_json(self.current)}
            await self._json(writer, 200, answer, alive)
        else:
            await self._send(writer, 404, "text/plain", b"not found\n", alive)
        return alive

    async def _json(self, writer, status, answer, alive):
        await self._send(writer, status, "application/json", json.dumps(answer).encode(), alive)

    async def _send(self, writer, status, content_type, body, alive):
        fields = [
            ("Content-Type", content_type),
            ("Content-Length", str(len(body))),
            ("Cache-Control", "no-store"),
        ]
        if not alive:
            fields.append(("Connection", "close"))
        writer.write(answer_head(status, fields) + body)
        await writer.drain()

    async def _websocket(self, request, reader, writer):
        key = header(request, "sec-websocket-key")
        if key is None:
            await self._send(writer, 400, "text/plain", b"no Sec-WebSocket-Key\n", False)
            return
        fields = [
            ("Upgrade", "websocket"),
            ("Connection", "Upgrade"),
            ("Sec-WebSocket-Accept", accept_key(key)),
        ]
        writer.write(answer_head(101, fields))
        await writer.drain()
        self.sockets.add(writer)
        try:
            while True:
                opcode, payload = await self._read_frame(reader)
                if opcode == 0x8:
                    writer.write(frame(0x8, payload[:2]))
                    await writer.drain()
                    return
                if opcode == 0x9:
                    writer.write(frame(0xA, payload))
                    await writer.drain()
        finally:
            self.sockets.discard(writer)

    async def _read_frame(self, reader):
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
    """The fake in a process of its own (``child.py``): this file's tool, with
    the calls of ``FakePresenter``."""

    def __init__(self, port, host="127.0.0.1"):
        tool = [sys.executable, os.path.abspath(__file__), "--port", str(port), "--host", host]
        super().__init__("the fake Presenter", "FAKE_PRESENTER", tool)

    def down(self):
        return self.ask("down")

    def up(self):
        return self.ask("up")

    def switch(self, code):
        return self.ask(f"switch {code}")

    def clear(self):
        return self.ask("clear")

    def state(self):
        return self.ask("state")


def command(fake, line):
    """One control line of the tool: its answer, a dict (an ``error`` for a
    line it does not know)."""
    words = line.split()
    if len(words) == 2 and words[0] == "switch":
        return fake.switch(words[1])
    calls = {"down": fake.down, "up": fake.up, "clear": fake.clear, "state": fake.state}
    call = calls.get(" ".join(words))
    if call is None:
        log(f"unknown line {line.strip()!r}")
        return {"error": f"unknown line {line.strip()!r}: {' | '.join(calls)} | switch <code>"}
    return call()


def main(argv=None, stdin=None, stdout=None):
    parser = argparse.ArgumentParser(description="the fake Presenter of the E2E suite")
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--host", default="127.0.0.1")
    args = parser.parse_args(argv)
    fake = FakePresenter(args.port, host=args.host)
    return child.serve(
        fake, "FAKE_PRESENTER", command, stdin or sys.stdin, stdout or sys.stdout, log
    )


if __name__ == "__main__":
    sys.exit(main())
