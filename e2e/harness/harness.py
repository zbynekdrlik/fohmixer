"""The E2E harness: two SimLive hosts, the hub, and a control API for the tests.

    python3 e2e/harness/harness.py --data DIR --layout FILE [--hub BINARY]
        [--http-port 8480] [--control-port 39190] [--band-port 39101]
        [--master-port 39102] [--meters-hz 30] [--log-dir DIR] [--link-port 0]
        [--public-name NAME --https-port PORT --tls-cert FILE --tls-key FILE]
        [--access-team TEAM --access-aud AUD --access-key FILE]
        [--fake-companion-port N | --companion HOST:PORT]

It starts ``sim/host.py`` for ``band`` and ``master`` (the real FohMixer script on
SimLive), writes the hub's config and layout into the data folder, starts the hub
(when ``--hub`` is given), starts the impair proxy (#43, ``impair.py``) on
``--link-port`` in front of the hub's HTTP port (the Playwright projects open
the surface through it, so a test can stall, drop or block the page's link) and
serves a small control API on 127.0.0.1 for the Playwright tests, which need
what only the harness can do:

    GET  /health                         {"ok": true}
    POST /host/<name>/line {"line": ...} a control line on the host's stdin
                                         (``stall``, ``rename``, ``listeners``, ``meter``,
                                         ``tuner``: a Tuner marker, #68);
                                         {"answer": <its answer line or null>}
    POST /host/<name>/restart            the host stopped (SIGTERM) and started
    POST /hub/restart {"rotate_secret"}  the hub stopped and started; with
                                         ``rotate_secret`` its JWT secret is new,
                                         so every token it issued is refused
    POST /hub/layout {"layout": {...}}   a new layout file (the hub picks it up)
    POST /hub/layout/reset               the original layout file back
    GET  /hub/events                     {"events": [...]}: every record of the
                                         hub's event log (#43, ``logs/events-*.jsonl``
                                         in the data folder), oldest day first
    GET  /sim/eq                         {"records": [...]}: what the hub's simulated
                                         window backend did (#71 PR E, ``eq-sim.jsonl``
                                         in the data folder: each take, touch with its
                                         phase and point, release), in order
    POST /sim/eq/clear                   those records forgotten
    POST /forensics/timeline {"from_ms", "to_ms", "key"}
                                         ``tools/forensics/timeline.py`` over the
                                         data folder's ``logs`` from ``from_ms`` to
                                         ``to_ms`` (epoch ms; ``key`` optional, its
                                         ``--key``), the report written to a
                                         temporary folder outside the repo:
                                         {"exit", "stdout", "stderr", "html": <the
                                         report, or null when none was written>}
    GET  /link                           the impair proxy's state: ``port``,
                                         ``connections``, ``held``, ``blocked``,
                                         ``stall_ms``, ``rate``
    POST /link/stall {"ms": <ms>}        both directions of every connection
                                         held for ``ms`` (a running stall is
                                         extended, never shortened)
    POST /link/drop                      every connection reset: {"dropped": n}
    POST /link/block {"on": <bool>}      new connections held (accepted, not
                                         passed on) until ``on`` is false
    POST /link/rate {"bytes_per_s": <n>} the pages' bytes to the hub at most
                                         ``n`` a second, first in first out (a
                                         slow link, #43 PR D; 0 lifts it)
    GET  /companion                      the fake Companion's state (#52,
                                         ``--fake-companion-port``): ``connections``,
                                         ``presses`` [{key, pressed, at}], ``down``,
                                         ``failing``; 404 without the fake
    POST /companion/down                 every connection closed, new ones closed
                                         at once (Companion away)
    POST /companion/up                   new connections served again
    POST /companion/fail {"on": <bool>}  presses answered ERROR, or OK again
    POST /companion/reset                up, not failing, presses and held keys
                                         forgotten, and the hub's ``[companion]``
                                         put back (the hub restarted) if
                                         ``/hub/companion`` removed it
    POST /companion/clear                the recorded presses forgotten
    POST /hub/companion {"on": <bool>}   the hub's config with or without
                                         ``[companion]``, and the hub restarted;
                                         {"companion": what it now holds}: false
                                         when no endpoint is configured
    GET  /cdn-cgi/access/certs           the test Access key set (``--access-key``)

Remote access (#17): with ``--public-name`` the hub serves that name over HTTPS
on ``--https-port`` with the test certificate (copied into its store
``tls/``); with ``--access-team`` internet requests need an Access JWT signed by
``--access-key`` (an RSA key made by ``openssl genrsa``), whose public half this
harness serves as the team's key set.

The Stream Deck (#52): with ``--fake-companion-port`` the harness starts
``fake_companion.py`` there (0: any free port) and the hub's config gets
``[companion]`` on it; with ``--companion HOST:PORT`` it names a real Companion
(the ``companion`` CI job) and starts no fake.

The Pro-Q 4 screen (#71 PR E): the hub's config gets ``[eq] backend = "sim"``,
the simulated window backend, whose records the tests read through ``/sim/eq``.

Every process is stopped with SIGTERM and a bounded wait (spec I7). Prints
``HARNESS READY`` once everything answers; SIGTERM or SIGINT stops it all.
"""

import argparse
import base64
import json
import math
import os
import queue
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import impair
from fake_companion import FakeCompanion

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
# The forensics tool's own local time text (#43): the window the harness asks
# for is the one the tool reads, so the two cannot drift apart.
sys.path.insert(0, os.path.join(REPO, "tools", "forensics"))
from timeline_read import local_text  # noqa: E402

# The key id of the test Access key.
ACCESS_KID = "e2e-kid"
# The route of the test Access key set.
CERTS_PATH = "/cdn-cgi/access/certs"
HOST = os.path.join(REPO, "sim", "host.py")
SITE = os.path.join(REPO, "sim", "fixtures", "test-site.json")
TIMELINE = os.path.join(REPO, "tools", "forensics", "timeline.py")
# The longest a timeline run may take (it reads the window's day files only).
TIMELINE_S = 120
READY_S = 20.0
STOP_S = 10.0
ANSWER_S = 5.0
# The control lines a host answers, and the prefix of each answer.
ANSWERS = {
    "rename": "RENAMED",
    "listeners": "LISTENERS",
    "meter": "METER",
    "tuner": "TUNER",
    "delete-track": "DELETED",
}


def hub_config(http_port, band_port, master_port, remote=None, companion=None):
    """The hub's ``fohmixer-hub.toml`` for the two hosts (layout polled fast) and
    the Pro-Q 4 screen on the simulated window backend (#71 PR E), with the
    Stream Deck's ``[companion]`` on ``companion`` (host, port) when given (#52),
    then the remote-access tables of ``remote`` (``name``, ``https_port``, and
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
        "[eq]\n"
        'backend = "sim"\n'
    )
    if companion:
        host, port = companion
        text += f'[companion]\nhost = "{host}"\nport = {port}\n'
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


def event_records(data):
    """Every record of the hub's event log in the data folder ``data``: the day
    files ``logs/events-YYYY-MM-DD.jsonl`` in date order, each line one JSON
    object (a line cut short by a write in progress is left out)."""
    logs = os.path.join(data, "logs")
    if not os.path.isdir(logs):
        return []
    days = sorted(n for n in os.listdir(logs) if n.startswith("events-") and n.endswith(".jsonl"))
    records = []
    for name in days:
        with open(os.path.join(logs, name), encoding="utf-8") as f:
            for line in f:
                try:
                    records.append(json.loads(line))
                except json.JSONDecodeError:
                    continue
    return records


# The simulated window backend's record file in the data folder (#71 PR E,
# the hub's ``plugwin::sim::RECORD_FILE``).
SIM_EQ = "eq-sim.jsonl"


def sim_eq_records(data):
    """What the hub's simulated window backend recorded in the data folder
    ``data``: one JSON object a line, in order (a line cut short by a write in
    progress is left out); none before its first record."""
    path = os.path.join(data, SIM_EQ)
    if not os.path.exists(path):
        return []
    records = []
    with open(path, encoding="utf-8") as f:
        for line in f:
            try:
                records.append(json.loads(line))
            except json.JSONDecodeError:
                continue
    return records


def clear_sim_eq(data):
    """Forgets the simulated window backend's records (the hub appends to the
    file line by line, opening it each time)."""
    path = os.path.join(data, SIM_EQ)
    if os.path.exists(path):
        with open(path, "w", encoding="utf-8"):
            pass


class BadRequest(Exception):
    """A control request whose body does not say what to do (answered 400)."""


def stall_ms(body):
    """The ``ms`` of a ``/link/stall`` body: a finite number of milliseconds, 0
    or more (JSON's ``Infinity`` would hold the proxy for every later test)."""
    ms = body.get("ms")
    finite = isinstance(ms, (int, float)) and not isinstance(ms, bool) and math.isfinite(ms)
    if not finite or ms < 0:
        raise BadRequest(f'/link/stall wants {{"ms": <0 or more>}}, not {body!r}')
    return ms


def block_on(body):
    """The ``on`` of a ``/link/block`` body: true or false."""
    on = body.get("on")
    if not isinstance(on, bool):
        raise BadRequest(f'/link/block wants {{"on": true|false}}, not {body!r}')
    return on


def rate_of(body):
    """The ``bytes_per_s`` of a ``/link/rate`` body: a finite number, 0 or more
    (0 lifts the limit)."""
    rate = body.get("bytes_per_s")
    finite = isinstance(rate, (int, float)) and not isinstance(rate, bool) and math.isfinite(rate)
    if not finite or rate < 0:
        raise BadRequest(f'/link/rate wants {{"bytes_per_s": <0 or more>}}, not {body!r}')
    return rate


def timeline_window(body):
    """The ``from_ms``, ``to_ms`` and ``key`` of a ``/forensics/timeline``
    body: finite epoch ms, and a text or nothing."""
    want = '/forensics/timeline wants {"from_ms": <ms>, "to_ms": <ms>, "key": <text, optional>}'
    if not isinstance(body, dict):
        raise BadRequest(f"{want}, not {body!r}")
    window = []
    for name in ("from_ms", "to_ms"):
        ms = body.get(name)
        if isinstance(ms, bool) or not isinstance(ms, (int, float)) or not math.isfinite(ms):
            raise BadRequest(f"{want}, not {body!r}")
        window.append(ms)
    key = body.get("key")
    if key is not None and not isinstance(key, str):
        raise BadRequest(f"{want}, not {body!r}")
    return window[0], window[1], key


def forensics_timeline(data, body):
    """Runs ``tools/forensics/timeline.py`` over the event log of the data
    folder ``data`` for the window of ``body`` (``timeline_window``); the
    report goes to a temporary folder outside the repo (the tool refuses a
    git checkout) and is read back. Its exit code, stdout, stderr and the
    report (None when it wrote none)."""
    start, end, key = timeline_window(body)
    with tempfile.TemporaryDirectory(prefix="fohmixer-timeline-") as folder:
        out = os.path.join(folder, "report.html")
        command = [sys.executable, TIMELINE, "--events", os.path.join(data, "logs")]
        command += ["--from", local_text(start), "--to", local_text(end)]
        if key is not None:
            command += ["--key", key]
        command += ["--out", out]
        done = subprocess.run(
            command,
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=TIMELINE_S,
            check=False,
        )
        report = None
        if os.path.exists(out):
            with open(out, encoding="utf-8") as f:
                report = f.read()
    # The tool's stdout and stderr never hold a key; the report does.
    print(f"harness: timeline exit {done.returncode} {done.stderr.strip()}".rstrip(), flush=True)
    return {"exit": done.returncode, "stdout": done.stdout, "stderr": done.stderr, "html": report}


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


def on_of(body, route):
    """The ``on`` of a body: true or false."""
    on = body.get("on")
    if not isinstance(on, bool):
        raise BadRequest(f'{route} wants {{"on": true|false}}, not {body!r}')
    return on


def companion_of(args, fake):
    """The hub's ``[companion]`` endpoint (#52): ``--companion HOST:PORT`` (a
    real Companion), else the fake's port on 127.0.0.1, else none."""
    if args.companion and args.fake_companion_port is not None:
        raise SystemExit("--companion and --fake-companion-port are exclusive")
    if args.companion:
        host, _, port = args.companion.rpartition(":")
        if not host or not port.isdigit():
            raise SystemExit(f"--companion {args.companion!r}: HOST:PORT")
        return host, int(port)
    if fake is not None:
        return "127.0.0.1", fake.port
    return None


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
        self.remote = remote
        self.http_port = args.http_port
        self.fake = None
        if args.fake_companion_port is not None:
            self.fake = FakeCompanion(args.fake_companion_port)
            self.fake.start()
        self.companion = companion_of(args, self.fake)
        self.companion_on = self.companion is not None
        self.write_config(self.companion)
        self.reset_layout()
        # The proxy needs no hub to listen; it is up before the hub starts.
        self.link = impair.Impair(args.link_port, args.http_port)
        self.link.start()
        self.hub = Hub(args.hub, self.data, args.http_port, log_dir) if args.hub else None

    def write_config(self, companion):
        """The hub's config, with ``[companion]`` on ``companion`` or without."""
        with open(os.path.join(self.data, "fohmixer-hub.toml"), "w", encoding="utf-8") as f:
            f.write(
                hub_config(
                    self.http_port,
                    self.hosts["band"].port,
                    self.hosts["master"].port,
                    self.remote,
                    companion,
                )
            )

    def set_companion(self, on):
        """The hub's config with or without ``[companion]``; answers what it now
        holds (never on without an endpoint), the hub restarted when there is one."""
        actual = bool(on) and self.companion is not None
        self.write_config(self.companion if actual else None)
        self.companion_on = actual
        if self.hub is not None:
            self.hub.restart(False)
        return actual

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
        if method == "GET" and parts == ["hub", "events"]:
            return 200, {"events": event_records(self.data)}
        if method == "GET" and parts == ["sim", "eq"]:
            return 200, {"records": sim_eq_records(self.data)}
        if method == "GET" and parts == ["link"]:
            return 200, dict(self.link.state(), port=self.link.port)
        if method == "GET" and parts == ["companion"]:
            return (200, self.fake.state()) if self.fake else (404, {"error": "no fake Companion"})
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
        if parts == ["sim", "eq", "clear"]:
            clear_sim_eq(self.data)
            return 200, {"ok": True}
        if parts == ["hub", "layout", "reset"]:
            self.reset_layout()
            return 200, {"ok": True}
        if parts == ["hub", "companion"] and self.hub is not None:
            return 200, {"companion": self.set_companion(on_of(body, "/hub/companion"))}
        if parts and parts[0] == "companion":
            if self.fake is None:
                return 404, {"error": "no fake Companion"}
            if parts == ["companion", "down"]:
                return 200, self.fake.down()
            if parts == ["companion", "up"]:
                return 200, self.fake.up()
            if parts == ["companion", "reset"]:
                answer = self.fake.reset()
                if not self.companion_on:
                    self.set_companion(True)
                return 200, answer
            if parts == ["companion", "clear"]:
                return 200, self.fake.clear()
            if parts == ["companion", "fail"]:
                return 200, self.fake.fail(on_of(body, "/companion/fail"))
        if parts == ["forensics", "timeline"]:
            return 200, forensics_timeline(self.data, body)
        if parts == ["link", "stall"]:
            return 200, self.link.stall(stall_ms(body))
        if parts == ["link", "drop"]:
            return 200, self.link.drop()
        if parts == ["link", "block"]:
            return 200, self.link.block(block_on(body))
        if parts == ["link", "rate"]:
            return 200, self.link.rate(rate_of(body))
        return 404, {"error": f"no {method} {path}"}

    def stop(self):
        self.link.stop()
        # The hub first: its graceful REMOVE-DEVICE then reaches the fake.
        if self.hub is not None:
            self.hub.stop()
        if self.fake is not None:
            self.fake.stop()
        for host in self.hosts.values():
            host.stop()


def handler_for(harness):
    class Handler(BaseHTTPRequestHandler):
        def _serve(self, method):
            length = int(self.headers.get("Content-Length") or 0)
            try:
                body = json.loads(self.rfile.read(length) or b"{}")
                status, answer = harness.handle(method, self.path, body)
            except BadRequest as e:
                status, answer = 400, {"error": str(e)}
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
    # The impair proxy's port (#43); 0 takes any free one (the harness's own tests).
    parser.add_argument("--link-port", type=int, default=0)
    parser.add_argument("--public-name", default=None)
    parser.add_argument("--https-port", type=int, default=8443)
    parser.add_argument("--tls-cert", default=None)
    parser.add_argument("--tls-key", default=None)
    parser.add_argument("--access-team", default=None)
    parser.add_argument("--access-aud", default=None)
    parser.add_argument("--access-key", default=None)
    # The Stream Deck (#52): a fake Companion on this port (0: any), or a real one.
    parser.add_argument("--fake-companion-port", type=int, default=None)
    parser.add_argument("--companion", default=None)
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
