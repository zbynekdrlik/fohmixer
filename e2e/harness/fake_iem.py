"""A fake iemmixer page for the E2E suite (#166), beside ``fake_ableset.py``:
iemmixer's page as the IEM tab's frame sees it, on an origin of its own over
HTTPS, since the hub takes no other address and the microphone needs it. The
hub proxies nothing of it, so this answers the browser directly:

    fake = FakeIem(port, cert, key, ancestors)   # 0: any free port, see ``port``
    fake.start()
    fake.clear()                                 # the hits forgotten
    fake.state()                                 # port, loads, hits
    fake.stop()

Its routes:

    GET  /                the page: ``#fake-iem`` and a counter its own timer
                          keeps going (``window.ticks``, ``data-ticks`` on the
                          body), which shows a hidden frame still runs; the
                          answer's ``frame-ancestors`` is ``ancestors``, as
                          iemmixer allows its frame itself
    anything else         404

Every request is a hit; ``loads`` counts the page's own, so a frame loaded
again shows there as well as in the document the test marked.

The harness runs it in a process of its own (``Child``: this file's tool, its
control lines ``clear`` and ``state``, as ``child.py`` drives a tool). Stdlib
only; every control call and failure is printed on stderr (``fake_iem: …``,
the harness's log). Nothing is ever force-ended.
"""

import argparse
import http.server
import os
import ssl
import sys
import threading

import child

PAGE = b"""<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><title>Fake iemmixer</title></head>
<body>
<h1 id="fake-iem">ENGINEER</h1>
<script>
window.ticks = 0;
setInterval(() => {
  window.ticks += 1;
  document.body.setAttribute("data-ticks", String(window.ticks));
}, 50);
document.body.setAttribute("data-app", "loaded");
</script>
</body>
</html>
"""


def log(text):
    print(f"fake_iem: {text}", file=sys.stderr, flush=True)


class FakeIem:
    """The page on 127.0.0.1:``port`` over TLS with ``cert`` and ``key``."""

    def __init__(self, port, cert, key, ancestors="*", host="127.0.0.1"):
        self.host = host
        self.port = port
        self.cert = cert
        self.key = key
        self.ancestors = ancestors
        self.lock = threading.Lock()
        self.hits = []
        self.server = None
        self.thread = None

    def start(self):
        """Listens; a ``RuntimeError`` when it cannot."""
        fake = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                fake.hit(self.path)
                if self.path == "/":
                    self.answer(200, "text/html; charset=utf-8", PAGE)
                else:
                    self.answer(404, "text/plain", b"no such page\n")

            def answer(self, status, kind, body):
                self.send_response(status)
                self.send_header("Content-Type", kind)
                self.send_header("Content-Length", str(len(body)))
                self.send_header("Content-Security-Policy", f"frame-ancestors {fake.ancestors}")
                self.send_header("Cache-Control", "no-store")
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, format, *args):
                log(format % args)

        try:
            context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
            context.load_cert_chain(self.cert, self.key)
            self.server = http.server.ThreadingHTTPServer((self.host, self.port), Handler)
        except (OSError, ssl.SSLError) as e:
            raise RuntimeError(f"cannot listen on {self.host}:{self.port}: {e}") from e
        # The handshake runs in the connection's own thread: a client that
        # stalls in it never holds the accept loop.
        self.server.socket = context.wrap_socket(
            self.server.socket, server_side=True, do_handshake_on_connect=False
        )
        self.port = self.server.server_address[1]
        self.thread = threading.Thread(
            target=self.server.serve_forever, name="fake-iem", daemon=True
        )
        self.thread.start()
        log(f"listening on {self.host}:{self.port}")

    def hit(self, path):
        with self.lock:
            self.hits.append(path)

    def clear(self):
        """The hits forgotten."""
        with self.lock:
            self.hits.clear()
        log("hits cleared")
        return self.state()

    def state(self):
        with self.lock:
            hits = list(self.hits)
        return {"port": self.port, "loads": hits.count("/"), "hits": hits}

    def stop(self):
        if self.server is None:
            return
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(5.0)
        self.server = None
        log("stopped")


class Child(child.Child):
    """The fake in a process of its own (``child.py``): this file's tool, with
    the calls of ``FakeIem`` (``start``, ``port``, ``clear``, ``state``,
    ``stop``)."""

    def __init__(self, port, cert, key, ancestors="*", host="127.0.0.1"):
        tool = [
            sys.executable,
            os.path.abspath(__file__),
            *("--port", str(port), "--host", host),
            *("--cert", cert, "--key", key, "--ancestors", ancestors),
        ]
        super().__init__("the fake iemmixer", "FAKE_IEM", tool)

    def clear(self):
        return self.ask("clear")

    def state(self):
        return self.ask("state")


def command(fake, line):
    """One control line of the tool: its answer, a dict (an ``error`` for a
    line it does not know)."""
    calls = {"clear": fake.clear, "state": fake.state}
    call = calls.get(" ".join(line.split()))
    if call is None:
        log(f"unknown line {line.strip()!r}")
        return {"error": f"unknown line {line.strip()!r}: {' | '.join(calls)}"}
    return call()


def main(argv=None, stdin=None, stdout=None):
    parser = argparse.ArgumentParser(description="the fake iemmixer of the E2E suite")
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--cert", required=True)
    parser.add_argument("--key", required=True)
    parser.add_argument("--ancestors", default="*")
    args = parser.parse_args(argv)
    fake = FakeIem(args.port, args.cert, args.key, args.ancestors, host=args.host)
    return child.serve(fake, "FAKE_IEM", command, stdin or sys.stdin, stdout or sys.stdout, log)


if __name__ == "__main__":
    sys.exit(main())
