"""The impair proxy (#43, design note §6.1): a TCP proxy between the browser and
the hub that the E2E tests can stall, drop and block on demand, so a degraded
link is deterministic (random ``tc netem`` stays out of CI).

    proxy = Impair(listen_port, upstream_port)
    proxy.start()            # serves on 127.0.0.1:<proxy.port>
    proxy.stall(1500)        # hold both directions for 1.5 s
    proxy.drop()             # close every proxied socket with a reset
    proxy.block(True)        # hold new connections until block(False)
    proxy.stop()

It is a plain byte pipe (stdlib asyncio, one event loop in a thread of its own),
so the page and the hub talk HTTP and WebSocket through it unchanged: the page's
``Host`` and ``Origin`` both name the proxy, which the hub's Origin guard accepts.

- **Stall:** every byte read from either side waits until the stall ends, then
  goes on in order; a pipe holds one chunk while it waits and reads nothing more,
  so the kernel buffers push back. A stall that is already running is extended,
  never shortened.
- **Drop:** every connection, the ones held by a block too, is closed on both
  sides with ``SO_LINGER`` 0: the peers see a reset, as after a dead Wi-Fi link
  whose TCP state is gone.
- **Block:** a new connection is accepted (the browser sees it connect and sends
  its request) but not connected to the hub: it waits, unread, until the block
  is lifted, then goes on as any other. Nothing is refused: a refused request or
  socket is a console error in the browser, a held one is not.

Every control call and every failure is printed (``impair: …``) on stderr,
the harness's log.

On its own (a degraded-link check of a deployed hub, from another machine):

    python3 impair.py --upstream <hub host>:<port> [--listen-port 0]
        [--listen-host 127.0.0.1]

prints ``IMPAIR <port>`` once it listens (on 127.0.0.1 unless
``--listen-host`` names another address, e.g. a LAN one for a real tablet)
and then takes one control line per stdin line — ``stall <ms>``, ``drop``, ``block on|off``,
``state`` — answering each with one JSON line; the end of stdin stops it.
"""

import argparse
import asyncio
import json
import math
import socket
import struct
import sys
import threading

# How long a control call waits for the proxy's loop.
CALL_S = 5.0
# How long start() waits for the listener.
START_S = 5.0
# One read from a socket.
CHUNK = 65536
# SO_LINGER on, 0 s: close() sends a reset.
RESET = struct.pack("ii", 1, 0)


def log(message):
    """A line on stderr (the harness's log; the tool's stdout stays its answers)."""
    print(f"impair: {message}", file=sys.stderr, flush=True)


def reset(writer):
    """Closes a stream's socket with a reset (``SO_LINGER`` 0), never a FIN."""
    sock = writer.get_extra_info("socket")
    if sock is not None:
        try:
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_LINGER, RESET)
        except OSError as e:
            log(f"no reset, the socket is already closed: {e!r}")
    writer.transport.abort()


class Link:
    """One proxied connection: the browser's side and, once connected, the hub's."""

    def __init__(self, reader, writer):
        self.reader = reader
        self.writer = writer
        self.upstream = None
        self.task = None

    def reset(self):
        reset(self.writer)
        if self.upstream is not None:
            reset(self.upstream[1])
        if self.task is not None:
            self.task.cancel()


class Impair:
    """The proxy: listens on ``listen_port`` of ``host`` (0: any free port, see
    ``port``) and forwards to ``upstream_port`` of ``upstream_host`` (default:
    ``host``; another machine's hub for a check of a deployed one)."""

    def __init__(self, listen_port, upstream_port, host="127.0.0.1", upstream_host=None):
        self.host = host
        self.upstream_host = upstream_host or host
        self.listen_port = listen_port
        self.upstream_port = upstream_port
        self.port = None
        self.loop = asyncio.new_event_loop()
        self.thread = threading.Thread(target=self._run, name="impair", daemon=True)
        self.listening = threading.Event()
        self.failure = None
        self.server = None
        self.links = set()
        # Loop state: only touched on the loop.
        self.stall_until = 0.0
        self.blocked = False
        self.unblocked = None

    def start(self):
        """Starts the loop and the listener; raises when it cannot listen."""
        self.thread.start()
        if not self.listening.wait(START_S):
            raise RuntimeError(f"the impair proxy did not listen within {START_S} s")
        if self.failure is not None:
            raise RuntimeError(f"the impair proxy cannot listen: {self.failure!r}")
        log(f"listening on {self.host}:{self.port}, forwarding to port {self.upstream_port}")

    def _run(self):
        asyncio.set_event_loop(self.loop)
        self.unblocked = asyncio.Event()
        self.unblocked.set()
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
        """Runs ``fn(*args)`` on the loop and returns its result."""

        async def call():
            return fn(*args)

        return asyncio.run_coroutine_threadsafe(call(), self.loop).result(CALL_S)

    # The control calls (any thread).

    def stall(self, ms):
        """Holds both directions of every connection for ``ms`` from now (a
        finite number, 0 or more: an endless stall would never end)."""
        ms = float(ms)
        if not math.isfinite(ms) or ms < 0:
            raise ValueError(f"a stall is a finite number of ms, 0 or more, not {ms!r}")
        answer = self._call(self._stall, ms)
        log(f"stall {ms} ms: {answer}")
        return answer

    def drop(self):
        """Resets every proxied connection: how many there were."""
        answer = self._call(self._drop)
        log(f"drop: {answer}")
        return answer

    def block(self, on):
        """Holds new connections (``on``) or lets them, and the held ones, through."""
        answer = self._call(self._block, bool(on))
        log(f"block {bool(on)}: {answer}")
        return answer

    def state(self):
        """What the proxy does now: ``connections``, ``held``, ``blocked``, ``stall_ms``."""
        return self._call(self._state)

    def stop(self):
        """Resets every connection, closes the listener and ends the loop."""
        if not self.thread.is_alive():
            return

        async def close():
            self._drop()
            self.server.close()
            await self.server.wait_closed()

        asyncio.run_coroutine_threadsafe(close(), self.loop).result(CALL_S)
        self.loop.call_soon_threadsafe(self.loop.stop)
        self.thread.join(CALL_S)
        log("stopped")

    # On the loop.

    def _stall(self, ms):
        self.stall_until = max(self.stall_until, self.loop.time() + ms / 1000.0)
        return {"stall_ms": self._stall_left_ms()}

    def _stall_left_ms(self):
        return max(0.0, (self.stall_until - self.loop.time()) * 1000.0)

    def _drop(self):
        links = list(self.links)
        for link in links:
            link.reset()
        self.links.clear()
        return {"dropped": len(links)}

    def _block(self, on):
        self.blocked = on
        if on:
            self.unblocked.clear()
        else:
            self.unblocked.set()
        return self._state()

    def _state(self):
        held = sum(1 for link in self.links if link.upstream is None)
        return {
            "connections": len(self.links),
            "held": held,
            "blocked": self.blocked,
            "stall_ms": self._stall_left_ms(),
        }

    async def _accept(self, reader, writer):
        link = Link(reader, writer)
        link.task = asyncio.current_task()
        self.links.add(link)
        try:
            await self.unblocked.wait()
            link.upstream = await asyncio.open_connection(self.upstream_host, self.upstream_port)
            await self._pipe_both(link)
        except asyncio.CancelledError:
            # A drop: both sides were reset already.
            log("a connection was dropped")
        except OSError as e:
            # A peer gone (a reset, the hub down): both sides go with it.
            log(f"a connection ended: {e!r}")
            link.reset()
        finally:
            self.links.discard(link)

    async def _pipe_both(self, link):
        up_reader, up_writer = link.upstream
        down = asyncio.ensure_future(self._pipe(link.reader, up_writer))
        up = asyncio.ensure_future(self._pipe(up_reader, link.writer))
        try:
            await asyncio.gather(down, up)
        finally:
            down.cancel()
            up.cancel()
        for writer in (link.writer, up_writer):
            writer.close()

    async def _pipe(self, reader, writer):
        """Copies ``reader`` to ``writer`` until its end, through the stall; an
        end is passed on as a half-close."""
        while True:
            data = await reader.read(CHUNK)
            if not data:
                break
            await self._through_stall()
            writer.write(data)
            await writer.drain()
        if writer.can_write_eof():
            writer.write_eof()

    async def _through_stall(self):
        """Waits while a stall lasts (it may be extended meanwhile)."""
        left = self.stall_until - self.loop.time()
        while left > 0:
            await asyncio.sleep(left)
            left = self.stall_until - self.loop.time()


def command(proxy, line):
    """One control line of the command-line tool: its answer, a dict (an
    ``error`` for a line it does not know or a value it refuses)."""
    words = line.split()
    try:
        if len(words) == 2 and words[0] == "stall":
            return proxy.stall(float(words[1]))
        if words == ["drop"]:
            return proxy.drop()
        if len(words) == 2 and words[0] == "block" and words[1] in ("on", "off"):
            return proxy.block(words[1] == "on")
        if words == ["state"]:
            return proxy.state()
    except ValueError as e:
        log(f"refused {line.strip()!r}: {e}")
        return {"error": str(e)}
    log(f"unknown line {line.strip()!r}")
    return {"error": f"unknown line {line.strip()!r}: stall <ms> | drop | block on|off | state"}


def parse_args(argv):
    parser = argparse.ArgumentParser(description="the impair proxy in front of a hub")
    parser.add_argument("--upstream", required=True, help="<IPv4 or name>:<port> of the hub")
    parser.add_argument("--listen-port", type=int, default=0)
    parser.add_argument("--listen-host", default="127.0.0.1")
    args = parser.parse_args(argv)
    host, _, port = args.upstream.rpartition(":")
    if not host or not port.isdigit() or any(c in host for c in "[]:"):
        parser.error(f"--upstream wants <IPv4 address or name>:<port>, not {args.upstream!r}")
    args.upstream_host, args.upstream_port = host, int(port)
    return args


def main(argv=None, stdin=None, stdout=None):
    args = parse_args(argv)
    stdin = stdin or sys.stdin
    stdout = stdout or sys.stdout
    proxy = Impair(
        args.listen_port,
        args.upstream_port,
        host=args.listen_host,
        upstream_host=args.upstream_host,
    )
    proxy.start()
    print(f"IMPAIR {proxy.port}", file=stdout, flush=True)
    try:
        for line in stdin:
            if line.strip():
                print(json.dumps(command(proxy, line)), file=stdout, flush=True)
    finally:
        proxy.stop()
    return 0


if __name__ == "__main__":
    sys.exit(main())
