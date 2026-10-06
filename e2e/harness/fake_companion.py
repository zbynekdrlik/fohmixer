"""A fake Bitfocus Companion for the E2E suite (#52), beside ``impair.py``:
the Satellite API on TCP as Companion 5.0.7 speaks it (checked against the
real one, ``docs/superpowers/plans/2026-10-06-streamdeck-tab.md``), with the
faults the tests turn on:

    fake = FakeCompanion(port)       # 0: any free port, see ``port``
    fake.start()
    fake.down()                      # every connection closed, new ones closed at once
    fake.up()
    fake.fail(True)                  # KEY-PRESS answered ERROR
    fake.clear()                     # forget the presses
    fake.state()                     # connections, presses, down, failing
    fake.stop()

Each key is a small solid PNG (stdlib ``zlib``), coloured by its number and
brighter while pressed; a press is answered ``KEY-PRESS OK`` (or ERROR) and
then the key's new state, as Companion does; a key held when its surface goes away stays held
(shown pressed to the next one); a socket silent for 5 s is closed. Every line ends with a space
before its ``\\n``, as Companion writes them. Stdlib only, one asyncio loop
in a thread of its own; every control call and failure is printed on stderr
(``fake_companion: …``, the harness's log). Nothing is ever force-ended: a
connection is closed, the loop stopped.
"""

import asyncio
import base64
import struct
import sys
import threading
import time
import zlib

# How long a control call waits for the loop.
CALL_S = 5.0
# How long start() waits for the listener.
START_S = 5.0
# Companion closes a socket that sent nothing for this long (the hub pings every 2 s).
IDLE_S = 5.0
# Companion answers a press about this fast.
PRESS_ANSWER_S = 0.003
VERSION = "5.0.7+fake"
API = "1.12.0"


def log(message):
    print(f"fake_companion: {message}", file=sys.stderr, flush=True)


def params(line):
    """A Satellite line's ``NAME=value`` parameters (quotes removed; a value
    may hold spaces inside its quotes); bare words left out."""
    found = {}
    rest = line.split(" ", 1)[1] if " " in line else ""
    while rest:
        rest = rest.lstrip(" ")
        name, eq, after = rest.partition("=")
        if not eq or " " in name:
            # A bare word: skip it.
            rest = rest.partition(" ")[2]
            continue
        if after.startswith('"'):
            value, _, rest = after[1:].partition('"')
        else:
            value, _, rest = after.partition(" ")
        found[name] = value
    return found


def png(rgb, side=8):
    """A solid ``side`` × ``side`` PNG of ``rgb``."""

    def chunk(kind, data):
        crc = zlib.crc32(kind + data) & 0xFFFFFFFF
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", crc)

    raw = (b"\x00" + bytes(rgb) * side) * side
    header = struct.pack(">IIBBBBB", side, side, 8, 2, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(raw))
        + chunk(b"IEND", b"")
    )


def colour(key, pressed):
    """A key's colour: its own, brighter while pressed."""
    base = ((key * 53) % 200, (key * 97) % 200, (key * 31) % 200)
    return tuple(min(255, c + 55) for c in base) if pressed else base


def key_state(device, key, columns, pressed):
    """A ``KEY-STATE`` line as Companion 5.0.7 writes it (a PNG bitmap)."""
    rgb = colour(key, pressed)
    data = base64.b64encode(png(rgb)).decode()
    row, column = divmod(key, columns)
    return (
        f'KEY-STATE DEVICEID="{device}" KEY={key} LOCATION="1/{row}/{column}" '
        f'PRESSED={int(pressed)} TYPE="BUTTON" BITMAP="data:image/png;base64,{data}" '
        f'COLOR="#{rgb[0]:02x}{rgb[1]:02x}{rgb[2]:02x}" TEXTCOLOR="#ffffff" '
    )


class FakeCompanion:
    """The fake on ``port`` of ``host`` (0: any free port, see ``port``)."""

    def __init__(self, port, host="127.0.0.1", idle_s=IDLE_S):
        self.host = host
        self.idle_s = idle_s
        self.listen_port = port
        self.port = None
        self.loop = asyncio.new_event_loop()
        self.thread = threading.Thread(target=self._run, name="fake-companion", daemon=True)
        self.listening = threading.Event()
        self.failure = None
        self.server = None
        # Loop state: only touched on the loop.
        self.writers = set()
        self.presses = []
        # The keys held (device, key): a key stays held when its surface goes away.
        self.held = set()
        self.is_down = False
        self.failing = False

    def start(self):
        """Starts the loop and the listener; raises when it cannot listen."""
        self.thread.start()
        if not self.listening.wait(START_S):
            raise RuntimeError(f"the fake Companion did not listen within {START_S} s")
        if self.failure is not None:
            raise RuntimeError(f"the fake Companion cannot listen: {self.failure!r}")
        log(f"listening on {self.host}:{self.port}")

    def _run(self):
        asyncio.set_event_loop(self.loop)
        try:
            self.server = self.loop.run_until_complete(
                asyncio.start_server(self._accept, self.host, self.listen_port)
            )
        except OSError as e:
            self.failure = e
            self.listening.set()
            self.loop.close()
            return
        self.port = self.server.sockets[0].getsockname()[1]
        self.listening.set()
        self.loop.run_forever()
        self.loop.close()

    def _call(self, fn, *args):
        async def call():
            return fn(*args)

        return asyncio.run_coroutine_threadsafe(call(), self.loop).result(CALL_S)

    # The control calls (any thread).

    def down(self):
        """Companion away: every connection closed, new ones closed at once."""
        answer = self._call(self._down)
        log(f"down: {answer}")
        return answer

    def up(self):
        answer = self._call(self._up)
        log(f"up: {answer}")
        return answer

    def fail(self, on):
        """Presses answered ERROR (``on``) or OK."""
        answer = self._call(self._fail, bool(on))
        log(f"fail {bool(on)}: {answer}")
        return answer

    def clear(self):
        """Forgets the presses and the held keys."""
        return self._call(self._clear)

    def state(self):
        """``connections``, ``presses`` (``key``, ``pressed``, ``at``: epoch ms),
        ``down``, ``failing``."""
        return self._call(self._state)

    def stop(self):
        """Closes every connection and the listener, ends the loop."""
        if not self.thread.is_alive():
            return

        async def close():
            self._down()
            self.server.close()
            await self.server.wait_closed()

        asyncio.run_coroutine_threadsafe(close(), self.loop).result(CALL_S)
        self.loop.call_soon_threadsafe(self.loop.stop)
        self.thread.join(CALL_S)
        log("stopped")

    # On the loop.

    def _down(self):
        self.is_down = True
        for writer in list(self.writers):
            writer.close()
        return self._state()

    def _up(self):
        self.is_down = False
        return self._state()

    def _fail(self, on):
        self.failing = on
        return self._state()

    def _clear(self):
        self.presses = []
        self.held = set()
        return self._state()

    def _state(self):
        return {
            "connections": len(self.writers),
            "presses": [dict(p) for p in self.presses],
            "down": self.is_down,
            "failing": self.failing,
        }

    async def _accept(self, reader, writer):
        if self.is_down:
            writer.close()
            return
        self.writers.add(writer)
        try:
            await self._serve(reader, writer)
        except (ConnectionError, OSError) as e:
            log(f"a connection ended: {e!r}")
        finally:
            self.writers.discard(writer)
            writer.close()

    async def _serve(self, reader, writer):
        def send(line):
            writer.write((line + "\n").encode())

        send(f'BEGIN CompanionVersion="{VERSION}" ApiVersion="{API}" ')
        send('CAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS="rgb,png,webp" ')
        await writer.drain()
        device, columns = "", 8
        while True:
            try:
                raw = await asyncio.wait_for(reader.readline(), self.idle_s)
            except asyncio.TimeoutError:
                log(f"closing a connection silent for {self.idle_s} s")
                return
            if not raw:
                return
            line = raw.decode(errors="replace").rstrip("\r\n")
            cmd = line.split(" ", 1)[0]
            found = params(line)
            if cmd == "PING":
                send("PONG " + line[5:].strip() + " ")
            elif cmd == "ADD-DEVICE":
                device = found.get("DEVICEID", "")
                total = int(found.get("KEYS_TOTAL", "0"))
                columns = int(found.get("KEYS_PER_ROW", "8"))
                send(f'ADD-DEVICE OK DEVICEID="{device}" ')
                send(f'BRIGHTNESS DEVICEID="{device}" VALUE=100 ')
                for key in range(total):
                    send(key_state(device, key, columns, (device, key) in self.held))
            elif cmd == "KEY-PRESS":
                key = int(found.get("KEY", "-1"))
                pressed = found.get("PRESSED") in ("1", "true")
                self.presses.append({"key": key, "pressed": pressed, "at": time.time() * 1000.0})
                await asyncio.sleep(PRESS_ANSWER_S)
                if self.failing:
                    send(f'KEY-PRESS ERROR DEVICEID="{device}" MESSAGE="test refusal" ')
                else:
                    (self.held.add if pressed else self.held.discard)((device, key))
                    send(f'KEY-PRESS OK DEVICEID="{device}" ')
                    send(key_state(device, key, columns, pressed))
            elif cmd == "REMOVE-DEVICE":
                send(f'REMOVE-DEVICE OK DEVICEID="{device}" ')
            else:
                send(f'ERROR MESSAGE="Unknown command: {cmd}" ')
            await writer.drain()
