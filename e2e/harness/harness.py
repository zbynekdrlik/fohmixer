"""The E2E harness: two SimLive hosts, the hub, and a control API for the tests.

    python3 e2e/harness/harness.py --data DIR --layout FILE [--hub BINARY]
        [--http-port 8480] [--control-port 39190] [--band-port 39101]
        [--master-port 39102] [--meters-hz 30] [--log-dir DIR]
        [--public-name NAME --https-port PORT --tls-cert FILE --tls-key FILE]
        [--access-team TEAM --access-aud AUD --access-key FILE]

It starts ``sim/host.py`` for ``band`` and ``master`` (the real FohMixer script on
SimLive), writes the hub's config and layout into the data folder, starts the hub
(when ``--hub`` is given) and serves a small control API on 127.0.0.1 for the
Playwright tests, which need what only the harness can do:

    GET  /health                         {"ok": true}
    POST /host/<name>/line {"line": ...} a control line on the host's stdin
                                         (``stall``, ``rename``, ``listeners``, ``meter``);
                                         {"answer": <its answer line or null>}
    POST /host/<name>/restart            the host stopped (SIGTERM) and started
    POST /hub/restart {"rotate_secret"}  the hub stopped and started; with
                                         ``rotate_secret`` its JWT secret is new,
                                         so every token it issued is refused
    POST /hub/layout {"layout": {...}}   a new layout file (the hub picks it up)
    POST /hub/layout/reset               the original layout file back
    GET  /cdn-cgi/access/certs           the test Access key set (``--access-key``)

Remote access (#17): with ``--public-name`` the hub serves that name over HTTPS
on ``--https-port`` with the test certificate (copied into its store
``tls/``); with ``--access-team`` internet requests need an Access JWT signed by
``--access-key`` (an RSA key made by ``openssl genrsa``), whose public half this
harness serves as the team's key set.

Every process is stopped with SIGTERM and a bounded wait (spec I7). Prints
``HARNESS READY`` once everything answers; SIGTERM or SIGINT stops it all.
"""

import argparse
import base64
import json
import os
import queue
import shutil
import signal
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
# The key id of the test Access key.
ACCESS_KID = "e2e-kid"
# The route of the test Access key set.
CERTS_PATH = "/cdn-cgi/access/certs"
HOST = os.path.join(REPO, "sim", "host.py")
SITE = os.path.join(REPO, "sim", "fixtures", "test-site.json")
READY_S = 20.0
STOP_S = 10.0
ANSWER_S = 5.0
# The control lines a host answers, and the prefix of each answer.
ANSWERS = {"rename": "RENAMED", "listeners": "LISTENERS", "meter": "METER"}


def hub_config(http_port, band_port, master_port, remote=None):
    """The hub's ``fohmixer-hub.toml`` for the two hosts (layout polled fast), with
    the remote-access tables of ``remote`` (``name``, ``https_port``, and
    ``team``, ``aud``, ``jwks_url`` for ``[access]``) when given."""
    text = (
        f"http_port = {http_port}\n"
        'layout = "layout.json"\n'
        "layout_poll_ms = 200\n"
        "[[instances]]\n"
        'name = "band"\n'
        f"port = {band_port}\n"
        "[[instances]]\n"
        'name = "master"\n'
        f"port = {master_port}\n"
    )
    remote = remote or {}
    if remote.get("name"):
        text += f'[tls]\nname = "{remote["name"]}"\nport = {remote["https_port"]}\n'
    if remote.get("team"):
        text += (
            f'[access]\nteam_domain = "{remote["team"]}"\naud = ["{remote["aud"]}"]\n'
            f'jwks_url = "{remote["jwks_url"]}"\n'
        )
    return text


def b64url(data):
    """Unpadded base64url (JWK numbers)."""
    return base64.urlsafe_b64encode(data).rstrip(b"=").decode()


def access_jwks(key_file):
    """The Access key set of the RSA key in ``key_file``: its modulus and
    exponent read by ``openssl rsa`` (the E2E job's keys come from ``openssl
    genrsa``: exponent 65537)."""

    def openssl(*args):
        return subprocess.run(
            ["openssl", "rsa", "-in", key_file, "-noout", *args],
            check=True,
            capture_output=True,
            text=True,
        ).stdout

    modulus = openssl("-modulus").strip().split("=", 1)[1]
    exponent = int(openssl("-text").split("publicExponent:", 1)[1].split()[0])
    return {
        "keys": [
            {
                "kid": ACCESS_KID,
                "kty": "RSA",
                "alg": "RS256",
                "use": "sig",
                "n": b64url(bytes.fromhex(modulus)),
                "e": b64url(exponent.to_bytes((exponent.bit_length() + 7) // 8, "big")),
            }
        ]
    }


def expected_answer(line):
    """The answer prefix a control line waits for, or None."""
    words = line.split()
    return ANSWERS.get(words[0]) if words else None


def stop_process(proc, what):
    """SIGTERM and a bounded wait: the only way a process is stopped (spec I7)."""
    if proc.poll() is not None:
        return proc.returncode
    proc.send_signal(signal.SIGTERM)
    try:
        return proc.wait(STOP_S)
    except subprocess.TimeoutExpired:
        raise RuntimeError(f"{what} did not stop within {STOP_S} s") from None


class Host:
    """One ``sim/host.py`` process with its stdin and stdout lines."""

    def __init__(self, name, port, meters_hz, log_dir):
        self.name = name
        self.port = port
        self.meters_hz = meters_hz
        self.log_dir = os.path.join(log_dir, f"host-{name}")
        os.makedirs(self.log_dir, exist_ok=True)
        self.lock = threading.Lock()
        self.proc = None
        self.lines = None
        self.start()

    def start(self):
        stderr = open(os.path.join(self.log_dir, "stderr.log"), "a", encoding="utf-8")
        self.proc = subprocess.Popen(
            [
                sys.executable,
                HOST,
                "--port",
                str(self.port),
                "--instance",
                self.name,
                "--site",
                SITE,
                "--meters-hz",
                str(self.meters_hz),
                "--log-dir",
                self.log_dir,
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=stderr,
            text=True,
        )
        stderr.close()
        self.lines = queue.Queue()
        threading.Thread(target=self._pump, args=(self.proc, self.lines), daemon=True).start()
        line = self._next(READY_S)
        if line is None or not line.startswith("READY "):
            raise RuntimeError(f"host {self.name} did not start: {line!r}")
        self.port = int(line.split()[1])

    @staticmethod
    def _pump(proc, lines):
        for line in proc.stdout:
            lines.put(line.strip())

    def _next(self, timeout):
        try:
            return self.lines.get(timeout=timeout)
        except queue.Empty:
            return None

    def line(self, text):
        """Writes a control line; the host's answer when the line has one."""
        with self.lock:
            prefix = expected_answer(text)
            self.proc.stdin.write(text.strip() + "\n")
            self.proc.stdin.flush()
            if prefix is None:
                return None
            deadline = time.monotonic() + ANSWER_S
            while time.monotonic() < deadline:
                answer = self._next(deadline - time.monotonic())
                if answer is not None and answer.startswith(prefix + " "):
                    return answer
            raise RuntimeError(f"host {self.name}: no {prefix} answer to {text!r}")

    def restart(self):
        with self.lock:
            self.stop()
            self.start()

    def stop(self):
        stop_process(self.proc, f"host {self.name}")
        self.proc.stdin.close()
        self.proc.stdout.close()


class Hub:
    """The hub binary on the data folder, its output in a log file."""

    def __init__(self, binary, data, http_port, log_dir):
        self.binary = binary
        self.data = data
        self.http_port = http_port
        self.log = os.path.join(log_dir, "hub.log")
        self.proc = None
        self.start()

    def start(self):
        env = dict(os.environ, FOHMIXER_DATA=self.data, PORT=str(self.http_port))
        env.setdefault("RUST_LOG", "info")
        with open(self.log, "a", encoding="utf-8") as log:
            self.proc = subprocess.Popen(
                [self.binary], env=env, stdout=log, stderr=subprocess.STDOUT
            )
        url = f"http://127.0.0.1:{self.http_port}/api/version"
        deadline = time.monotonic() + READY_S
        while time.monotonic() < deadline:
            if self.proc.poll() is not None:
                raise RuntimeError(f"the hub exited with {self.proc.returncode}")
            try:
                with urllib.request.urlopen(url, timeout=1):
                    return
            except (urllib.error.URLError, OSError):
                time.sleep(0.1)
        raise RuntimeError("the hub did not answer")

    def restart(self, rotate_secret):
        stop_process(self.proc, "the hub")
        if rotate_secret:
            secret = os.path.join(self.data, "secrets", "jwt_secret")
            if os.path.exists(secret):
                os.remove(secret)
        self.start()

    def stop(self):
        stop_process(self.proc, "the hub")


class Harness:
    """The hosts, the hub and the layout file the control API acts on."""

    def __init__(self, args):
        self.data = os.path.abspath(args.data)
        self.layout = os.path.abspath(args.layout)
        os.makedirs(self.data, exist_ok=True)
        log_dir = os.path.abspath(args.log_dir or self.data)
        os.makedirs(log_dir, exist_ok=True)
        self.hosts = {}
        self.hosts["band"] = Host("band", args.band_port, args.meters_hz, log_dir)
        self.hosts["master"] = Host("master", args.master_port, args.meters_hz, log_dir)
        self.jwks = access_jwks(args.access_key) if args.access_key else None
        remote = {
            "name": args.public_name,
            "https_port": args.https_port,
            "team": args.access_team,
            "aud": args.access_aud,
            "jwks_url": f"http://127.0.0.1:{args.control_port}{CERTS_PATH}",
        }
        if args.public_name:
            tls = os.path.join(self.data, "tls")
            os.makedirs(tls, exist_ok=True)
            shutil.copyfile(args.tls_cert, os.path.join(tls, "cert.pem"))
            shutil.copyfile(args.tls_key, os.path.join(tls, "key.pem"))
        with open(os.path.join(self.data, "fohmixer-hub.toml"), "w", encoding="utf-8") as f:
            f.write(
                hub_config(
                    args.http_port, self.hosts["band"].port, self.hosts["master"].port, remote
                )
            )
        self.reset_layout()
        self.hub = Hub(args.hub, self.data, args.http_port, log_dir) if args.hub else None

    def write_layout(self, layout):
        """Replaces the layout file whole (the hub never reads half a file)."""
        path = os.path.join(self.data, "layout.json")
        with open(path + ".tmp", "w", encoding="utf-8") as f:
            json.dump(layout, f, indent=1)
        os.replace(path + ".tmp", path)

    def reset_layout(self):
        path = os.path.join(self.data, "layout.json")
        shutil.copyfile(self.layout, path + ".tmp")
        os.replace(path + ".tmp", path)

    def handle(self, method, path, body):
        """One control request: (status, answer)."""
        parts = [p for p in path.split("/") if p]
        if method == "GET" and parts == ["health"]:
            return 200, {"ok": True}
        if method == "GET" and "/" + "/".join(parts) == CERTS_PATH:
            return (200, self.jwks) if self.jwks else (404, {"error": "no --access-key"})
        if method != "POST":
            return 405, {"error": f"{method} {path}"}
        if len(parts) == 3 and parts[0] == "host" and parts[1] in self.hosts:
            host = self.hosts[parts[1]]
            if parts[2] == "line":
                return 200, {"answer": host.line(body.get("line", ""))}
            if parts[2] == "restart":
                host.restart()
                return 200, {"port": host.port}
        if parts == ["hub", "restart"] and self.hub is not None:
            self.hub.restart(bool(body.get("rotate_secret")))
            return 200, {"ok": True}
        if parts == ["hub", "layout"]:
            self.write_layout(body["layout"])
            return 200, {"ok": True}
        if parts == ["hub", "layout", "reset"]:
            self.reset_layout()
            return 200, {"ok": True}
        return 404, {"error": f"no {method} {path}"}

    def stop(self):
        if self.hub is not None:
            self.hub.stop()
        for host in self.hosts.values():
            host.stop()


def handler_for(harness):
    class Handler(BaseHTTPRequestHandler):
        def _serve(self, method):
            length = int(self.headers.get("Content-Length") or 0)
            try:
                body = json.loads(self.rfile.read(length) or b"{}")
                status, answer = harness.handle(method, self.path, body)
            except Exception as e:  # the test sees why, the harness keeps serving
                status, answer = 500, {"error": f"{type(e).__name__}: {e}"}
            data = json.dumps(answer).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            self._serve("GET")

        def do_POST(self):
            self._serve("POST")

        def log_message(self, format, *args):
            print(f"harness: {self.command} {self.path} -> {args[1]}", flush=True)

    return Handler


def parse_args(argv):
    parser = argparse.ArgumentParser(description="fohmixer E2E harness")
    parser.add_argument("--data", required=True)
    parser.add_argument("--layout", required=True)
    parser.add_argument("--hub", default=None)
    parser.add_argument("--http-port", type=int, default=8480)
    parser.add_argument("--control-port", type=int, default=39190)
    parser.add_argument("--band-port", type=int, default=39101)
    parser.add_argument("--master-port", type=int, default=39102)
    parser.add_argument("--meters-hz", type=float, default=30.0)
    parser.add_argument("--log-dir", default=None)
    parser.add_argument("--public-name", default=None)
    parser.add_argument("--https-port", type=int, default=8443)
    parser.add_argument("--tls-cert", default=None)
    parser.add_argument("--tls-key", default=None)
    parser.add_argument("--access-team", default=None)
    parser.add_argument("--access-aud", default=None)
    parser.add_argument("--access-key", default=None)
    return parser.parse_args(argv)


def main(argv=None):
    args = parse_args(argv)
    harness = Harness(args)
    server = ThreadingHTTPServer(("127.0.0.1", args.control_port), handler_for(harness))
    stop = threading.Event()

    def request_stop(signum, frame):
        stop.set()

    signal.signal(signal.SIGTERM, request_stop)
    signal.signal(signal.SIGINT, request_stop)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    print(f"HARNESS READY {server.server_address[1]}", flush=True)
    stop.wait()
    server.shutdown()
    harness.stop()
    return 0


if __name__ == "__main__":
    sys.exit(main())
