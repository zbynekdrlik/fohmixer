"""A fake Bitfocus Companion for the E2E suite (#52), beside ``impair.py``:
the Satellite API on TCP as Companion 5.0.7 speaks it (checked against the
real one, ``docs/superpowers/plans/2026-10-06-streamdeck-tab.md``), with the
faults the tests turn on:

    fake = FakeCompanion(port)       # 0: any free port, see ``port``
    fake.start()
    fake.down()                      # every connection closed, new ones closed at once
    fake.up()
    fake.fail(True)                  # KEY-PRESS answered ERROR
    fake.clear()                     # forget the presses and the held keys
    fake.reset()                     # up, not failing, nothing recorded or held
    fake.state()                     # connections, devices, presses, down, failing
    fake.stop()

Each key is a small solid PNG (stdlib ``zlib``), coloured by its number and
brighter while pressed (the bitmaps are PNG, not the real Companion's webp: the hub
and the page treat the data URL as opaque, and the real Companion 5.0.7 job
covers real webp images in both browsers). A press is answered ``KEY-PRESS OK``
(or ERROR) and then the key's new state, as Companion does; a press for a device
that is not registered on the connection, or with a key that is not a digit
string below ``KEYS_TOTAL``, is refused as Companion refuses it (``Device not
found``, ``Invalid KEY``), recorded nowhere. A key held when its surface goes
away stays held, per key (a control's state, whatever device id the next
connection brings) and is shown pressed to the next surface until a release.
A socket silent for 5 s is closed. Every line ends with a space before its
``\\n``, as Companion writes them. Stdlib only, one asyncio loop
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


def digits(text):
    """A non-negative integer from ASCII digits only, else None."""
    if text.isascii() and text.isdigit():
        return int(text)
    return None


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
        # The keys held: a key stays held when its surface goes away, whatever
        # device id the next connection brings.
        self.held = set()
        # The ADD-DEVICE parameters of each registered connection, by writer.
        self.devices = {}
        self.tasks = set()
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
            answer = fn(*args)
            if asyncio.iscoroutine(answer):
                answer = await answer
            return answer

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

    def reset(self):
        """Up, not failing, the presses and the held keys forgotten."""
        answer = self._call(self._reset)
        log(f"reset: {answer}")
        return answer

    def clear(self):
        """Forgets the presses and the held keys."""
        return self._call(self._clear)

    def state(self):
        """``connections``, ``devices`` (the ADD-DEVICE parameters of each
        registered connection), ``presses`` (``key``, ``pressed``, ``at``: epoch ms),
        ``down``, ``failing``."""
        return self._call(self._state)

    def stop(self):
        """Closes every connection and the listener, ends the loop."""
        if not self.thread.is_alive():
            return

        async def close():
            await self._down()
            self.server.close()
            await self.server.wait_closed()
            tasks = list(self.tasks)
            for task in tasks:
                task.cancel()
            await asyncio.gather(*tasks, return_exceptions=True)

        asyncio.run_coroutine_threadsafe(close(), self.loop).result(CALL_S)
        self.loop.call_soon_threadsafe(self.loop.stop)
        self.thread.join(CALL_S)
        log("stopped")

    # On the loop.

    async def _down(self):
        self.is_down = True
        writers = list(self.writers)
        for writer in writers:
            writer.close()
        for writer in writers:
            try:
                await writer.wait_closed()
            except (ConnectionError, OSError):
                pass
        self.writers.clear()
        self.devices.clear()
        return self._state()

    def _reset(self):
        self.is_down = False
        self.failing = False
        self.presses = []
        self.held = set()
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
            "devices": [dict(d) for d in self.devices.values()],
            "presses": [dict(p) for p in self.presses],
            "down": self.is_down,
            "failing": self.failing,
        }

    async def _accept(self, reader, writer):
        if self.is_down:
            writer.close()
            return
        self.writers.add(writer)
        self.tasks.add(asyncio.current_task())
        try:
            await self._serve(reader, writer)
        except (ConnectionError, OSError) as e:
            log(f"a connection ended: {e!r}")
        finally:
            self.writers.discard(writer)
            self.devices.pop(writer, None)
            self.tasks.discard(asyncio.current_task())
            writer.close()

    async def _serve(self, reader, writer):
        def send(line):
            writer.write((line + "\n").encode())

        send(f'BEGIN CompanionVersion="{VERSION}" ApiVersion="{API}" ')
        send('CAPS SUBSCRIPTIONS=0 NONSQUARE=1 BITMAP_FORMATS="rgb,png,webp" ')
        await writer.drain()
        device, columns, total = None, 8, 0
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
                new_total = digits(found.get("KEYS_TOTAL", "0"))
                new_columns = digits(found.get("KEYS_PER_ROW", "8"))
                if new_total is None or new_columns is None or new_columns < 1:
                    send('ERROR MESSAGE="Invalid KEYS_TOTAL or KEYS_PER_ROW" ')
                else:
                    device, total, columns = found.get("DEVICEID", ""), new_total, new_columns
                    self.devices[writer] = dict(found)
                    send(f'ADD-DEVICE OK DEVICEID="{device}" ')
                    send(f'BRIGHTNESS DEVICEID="{device}" VALUE=100 ')
                    for key in range(total):
                        send(key_state(device, key, columns, key in self.held))
            elif cmd == "KEY-PRESS":
                asked = found.get("DEVICEID", "")
                key = digits(found.get("KEY", ""))
                if device is None or asked != device:
                    send(f'KEY-PRESS ERROR DEVICEID="{asked}" MESSAGE="Device not found" ')
                elif key is None or key >= total:
                    send(f'KEY-PRESS ERROR DEVICEID="{asked}" MESSAGE="Invalid KEY" ')
                else:
                    pressed = found.get("PRESSED") in ("1", "true")
                    self.presses.append(
                        {"key": key, "pressed": pressed, "at": time.time() * 1000.0}
                    )
                    await asyncio.sleep(PRESS_ANSWER_S)
                    if self.failing:
                        send(f'KEY-PRESS ERROR DEVICEID="{device}" MESSAGE="test refusal" ')
                    else:
                        (self.held.add if pressed else self.held.discard)(key)
                        send(f'KEY-PRESS OK DEVICEID="{device}" ')
                        send(key_state(device, key, columns, pressed))
            elif cmd == "REMOVE-DEVICE":
                send(f'REMOVE-DEVICE OK DEVICEID="{device or ""}" ')
                self.devices.pop(writer, None)
                device = None
            else:
                send(f'ERROR MESSAGE="Unknown command: {cmd}" ')
            await writer.drain()
