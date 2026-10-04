"""The E2E harness against real ``sim/host.py`` processes (no hub: its binary is
built only in the e2e job, where the Playwright suite drives the harness)."""

import base64
import calendar
import contextlib
import datetime
import io
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request
from http.server import ThreadingHTTPServer

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import harness  # noqa: E402
import timeline_read  # noqa: E402  (on sys.path through the harness)

LAYOUT = os.path.join(harness.REPO, "tools", "import-tosc", "fixtures", "expected-layout.json")
# A synthetic window of the event log: 2026-10-03 16:20:00 UTC on the hub's clock.
BASE = calendar.timegm((2026, 10, 3, 16, 20, 0)) * 1000
VOX = "band|live_set tracks[name=Vox 1] mixer_device volume|value"


def write_window(data):
    """One write of Vox 1 through the hub into ``data``'s event log: its
    ``set``, ``batch`` and ``applied`` records."""
    logs = os.path.join(data, "logs")
    os.makedirs(logs, exist_ok=True)
    item = {"key": VOX, "client": 3, "seq": 1}
    sent = dict(item, value=0.7)
    records = [
        dict(sent, ev="set", ts=BASE + 1003, instance="band", t=BASE + 1000.0, offset_ms=0.0),
        {"ev": "batch", "ts": BASE + 1004, "instance": "band", "batch": 1, "sent": [sent]},
        dict(ev="applied", ts=BASE + 1016, instance="band", batch=1, rtt_ms=12.0, sent=[item]),
    ]
    with open(os.path.join(logs, "events-2026-10-03.jsonl"), "w", encoding="utf-8") as f:
        f.write("".join(json.dumps(r) + "\n" for r in records))
    return logs


def quietly(call, *args):
    """``call(*args)`` with the harness's own log line kept out of the test output."""
    with contextlib.redirect_stdout(io.StringIO()):
        return call(*args)


class ForensicsTimeline(unittest.TestCase):
    """The ``/forensics/timeline`` route's function, no host needed."""

    def setUp(self):
        folder = tempfile.TemporaryDirectory(prefix="fohmixer-harness-forensics-")
        self.addCleanup(folder.cleanup)
        self.data = folder.name

    def test_the_tool_renders_the_window_of_the_data_folders_log(self):
        write_window(self.data)
        body = {"from_ms": BASE, "to_ms": BASE + 5000}
        answer = quietly(harness.forensics_timeline, self.data, body)
        self.assertEqual((answer["exit"], answer["stderr"]), (0, ""))
        summary = dict(line.split("=", 1) for line in answer["stdout"].splitlines())
        self.assertEqual((summary["records"], summary["confirmation_n"]), ("3", "1"))
        self.assertEqual(summary["worst_confirmation_ms"], "16.0", "applied 16 ms after the send")
        self.assertNotIn("Vox 1", answer["stdout"])
        self.assertIn('class="control"', answer["html"])
        self.assertIn("Vox 1", answer["html"])
        filtered = quietly(harness.forensics_timeline, self.data, dict(body, key="Hand2"))
        self.assertEqual(filtered["exit"], 0)
        self.assertNotIn('class="control"', filtered["html"], "--key Hand2 leaves no lane")
        self.assertIn("confirmation_n=0\n", filtered["stdout"])

    def test_a_refusal_of_the_tool_comes_back_without_a_report(self):
        answer = quietly(harness.forensics_timeline, self.data, {"from_ms": BASE, "to_ms": BASE})
        self.assertEqual((answer["exit"], answer["html"], answer["stdout"]), (1, None, ""))
        self.assertTrue(answer["stderr"].startswith("timeline: --to"), answer["stderr"])

    def test_a_bad_body_is_refused(self):
        for bad in (
            {},
            {"from_ms": BASE},
            {"from_ms": str(BASE), "to_ms": BASE + 1},
            {"from_ms": BASE, "to_ms": True},
            {"from_ms": BASE, "to_ms": float("nan")},
            {"from_ms": BASE, "to_ms": float("inf")},
            {"from_ms": BASE, "to_ms": BASE + 1, "key": 3},
            [BASE, BASE + 1],
        ):
            with self.assertRaises(harness.BadRequest, msg=repr(bad)):
                harness.forensics_timeline(self.data, bad)
        self.assertEqual(
            harness.timeline_window({"from_ms": 1.5, "to_ms": 2, "key": "Vox"}), (1.5, 2, "Vox")
        )

    def test_the_local_time_is_what_the_tool_reads(self):
        self.assertIs(harness.local_text, timeline_read.local_text, "the tool's own function")
        ms = BASE + 123
        moment = datetime.datetime.fromtimestamp(ms / 1000)
        self.assertEqual(harness.local_text(ms), moment.strftime("%Y-%m-%d %H:%M:%S") + ".123")
        self.assertEqual(
            time.mktime(time.strptime(harness.local_text(BASE)[:19], "%Y-%m-%d %H:%M:%S")),
            BASE / 1000,
        )
        # The runner is on UTC: a machine 5:45 east of it (a POSIX TZ, no time
        # zone database needed) proves the time is local, not UTC.
        code = f"import harness; print(harness.local_text({ms}))"
        shifted = subprocess.run(
            [sys.executable, "-c", code],
            cwd=os.path.dirname(os.path.abspath(__file__)),
            env=dict(os.environ, TZ="XYZ-05:45"),
            capture_output=True,
            text=True,
            check=True,
        )
        self.assertEqual(shifted.stdout, "2026-10-03 22:05:00.123\n")


class HarnessTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.data = tempfile.mkdtemp(prefix="fohmixer-harness-test-")
        args = harness.parse_args(
            [
                "--data",
                cls.data,
                "--layout",
                LAYOUT,
                "--band-port",
                "0",
                "--master-port",
                "0",
                "--meters-hz",
                "0",
            ]
        )
        cls.harness = harness.Harness(args)

    @classmethod
    def tearDownClass(cls):
        cls.harness.stop()
        shutil.rmtree(cls.data, ignore_errors=True)

    def post(self, path, body=None):
        return self.harness.handle("POST", path, body or {})

    def test_health_and_unknown_requests(self):
        self.assertEqual(self.harness.handle("GET", "/health", {}), (200, {"ok": True}))
        self.assertEqual(self.harness.handle("GET", "/host/band/line", {})[0], 405)
        self.assertEqual(self.post("/host/nobody/line")[0], 404)
        self.assertEqual(self.post("/hub/restart")[0], 404, "no hub in this test")
        self.assertEqual(self.post("/nothing")[0], 404)

    def test_the_event_log_is_read_day_by_day(self):
        logs = os.path.join(self.data, "logs")
        os.makedirs(logs, exist_ok=True)
        with open(os.path.join(logs, "events-2026-10-04.jsonl"), "w", encoding="utf-8") as f:
            f.write('{"ev": "set", "seq": 2}\n{"ev": "ack", "seq": 2}\n{"ev": "ba')
        with open(os.path.join(logs, "events-2026-10-03.jsonl"), "w", encoding="utf-8") as f:
            f.write('{"ev": "set", "seq": 1}\n')
        with open(os.path.join(logs, "hub.out.log"), "w", encoding="utf-8") as f:
            f.write("not an event\n")
        try:
            status, answer = self.harness.handle("GET", "/hub/events", {})
            self.assertEqual(status, 200)
            self.assertEqual(
                [(r["ev"], r["seq"]) for r in answer["events"]],
                [("set", 1), ("set", 2), ("ack", 2)],
                "oldest day first; a half-written line is left out",
            )
        finally:
            shutil.rmtree(logs)
        self.assertEqual(harness.event_records(self.data), [], "no logs folder: none")

    def test_a_control_line_reaches_the_host_and_brings_its_answer(self):
        status, answer = self.post("/host/band/line", {"line": "listeners mute live_set tracks 0"})
        self.assertEqual((status, answer), (200, {"answer": "LISTENERS 0"}))
        status, answer = self.post(
            "/host/master/line", {"line": "listeners mute live_set tracks[name=Nobody]"}
        )
        self.assertEqual(answer, {"answer": "LISTENERS -1"})
        self.assertEqual(
            self.post("/host/band/line", {"line": "stall 20"}), (200, {"answer": None})
        )

    def test_a_rename_and_back(self):
        answer = self.post("/host/band/line", {"line": 'rename "Hand2 #" "Hand2 X"'})[1]
        self.assertEqual(answer, {"answer": "RENAMED 1"})
        answer = self.post("/host/band/line", {"line": 'rename "Hand2 X" "Hand2 #"'})[1]
        self.assertEqual(answer, {"answer": "RENAMED 1"})

    def test_a_restarted_host_answers_again(self):
        old = self.harness.hosts["master"].proc
        status, answer = self.post("/host/master/restart")
        self.assertEqual(status, 200)
        self.assertEqual(old.returncode, 0, "the old host stopped cleanly on SIGTERM")
        new = self.harness.hosts["master"]
        self.assertNotEqual(new.proc.pid, old.pid)
        self.assertEqual(answer, {"port": new.port})
        self.assertEqual(
            self.post("/host/master/line", {"line": "listeners mute live_set tracks 0"})[1],
            {"answer": "LISTENERS 0"},
        )

    def test_the_layout_file_is_replaced_and_reset(self):
        path = os.path.join(self.data, "layout.json")
        with open(LAYOUT, encoding="utf-8") as f:
            original = json.load(f)
        with open(path, encoding="utf-8") as f:
            self.assertEqual(json.load(f), original)
        changed = dict(original, pages=original["pages"][:1])
        self.assertEqual(self.post("/hub/layout", {"layout": changed}), (200, {"ok": True}))
        with open(path, encoding="utf-8") as f:
            self.assertEqual(json.load(f)["pages"], original["pages"][:1])
        self.assertEqual(self.post("/hub/layout/reset"), (200, {"ok": True}))
        with open(path, encoding="utf-8") as f:
            self.assertEqual(json.load(f), original)

    def test_the_hub_config_names_both_hosts(self):
        with open(os.path.join(self.data, "fohmixer-hub.toml"), encoding="utf-8") as f:
            text = f.read()
        self.assertIn(f"port = {self.harness.hosts['band'].port}\n", text)
        self.assertIn(f"port = {self.harness.hosts['master'].port}\n", text)
        self.assertIn("layout_poll_ms = 200\n", text)
        self.assertEqual(
            harness.hub_config(8480, 1, 2),
            'http_port = 8480\nlayout = "layout.json"\nlayout_poll_ms = 200\n'
            '[[instances]]\nname = "band"\nport = 1\n'
            '[[instances]]\nname = "master"\nport = 2\n',
        )

    def test_the_remote_tables_of_the_hub_config(self):
        remote = {
            "name": "foh.e2e.test",
            "https_port": 8443,
            "team": "team.example.com",
            "aud": "aud-1",
            "jwks_url": "http://127.0.0.1:39190/cdn-cgi/access/certs",
        }
        text = harness.hub_config(8480, 1, 2, remote)
        self.assertTrue(text.startswith(harness.hub_config(8480, 1, 2)))
        self.assertTrue(
            text.endswith(
                '[tls]\nname = "foh.e2e.test"\nport = 8443\n'
                '[access]\nteam_domain = "team.example.com"\naud = ["aud-1"]\n'
                'jwks_url = "http://127.0.0.1:39190/cdn-cgi/access/certs"\n'
            ),
            text,
        )
        only_tls = harness.hub_config(8480, 1, 2, dict(remote, team=None))
        self.assertIn("[tls]", only_tls)
        self.assertNotIn("[access]", only_tls)
        self.assertEqual(
            harness.hub_config(8480, 1, 2, {"name": None}), harness.hub_config(8480, 1, 2)
        )

    def test_the_access_key_set_is_the_keys_public_half(self):
        key = os.path.join(self.data, "access.pem")
        subprocess.run(["openssl", "genrsa", "-out", key, "2048"], check=True, capture_output=True)
        jwks = harness.access_jwks(key)
        (jwk,) = jwks["keys"]
        self.assertEqual(
            (jwk["kid"], jwk["kty"], jwk["alg"], jwk["e"]), ("e2e-kid", "RSA", "RS256", "AQAB")
        )
        modulus = base64.urlsafe_b64decode(jwk["n"] + "=" * (-len(jwk["n"]) % 4))
        self.assertEqual(len(modulus), 256)
        text = subprocess.run(
            ["openssl", "rsa", "-in", key, "-noout", "-modulus"],
            check=True,
            capture_output=True,
            text=True,
        ).stdout
        self.assertEqual(modulus.hex().upper(), text.strip().split("=")[1])
        self.assertEqual(harness.b64url(b"\x01\x00\x01"), "AQAB")
        # The harness serves it at the team's key-set path, else 404.
        self.assertEqual(self.harness.handle("GET", harness.CERTS_PATH, {})[0], 404)
        self.harness.jwks = jwks
        try:
            self.assertEqual(self.harness.handle("GET", harness.CERTS_PATH, {}), (200, jwks))
        finally:
            self.harness.jwks = None

    def test_a_public_name_puts_the_certificate_into_the_hubs_store(self):
        data = tempfile.mkdtemp(prefix="fohmixer-harness-remote-")
        cert = os.path.join(data, "leaf.pem")
        key = os.path.join(data, "leaf.key")
        with open(cert, "w", encoding="utf-8") as f:
            f.write("CERT")
        with open(key, "w", encoding="utf-8") as f:
            f.write("KEY")
        args = harness.parse_args(
            [
                "--data",
                os.path.join(data, "hub"),
                "--layout",
                LAYOUT,
                "--band-port",
                "0",
                "--master-port",
                "0",
                "--meters-hz",
                "0",
                "--control-port",
                "39999",
                "--public-name",
                "foh.e2e.test",
                "--https-port",
                "9443",
                "--tls-cert",
                cert,
                "--tls-key",
                key,
                "--access-team",
                "team.example.com",
                "--access-aud",
                "aud-1",
            ]
        )
        remote = harness.Harness(args)
        try:
            tls = os.path.join(data, "hub", "tls")
            with open(os.path.join(tls, "cert.pem"), encoding="utf-8") as f:
                self.assertEqual(f.read(), "CERT")
            with open(os.path.join(tls, "key.pem"), encoding="utf-8") as f:
                self.assertEqual(f.read(), "KEY")
            with open(os.path.join(data, "hub", "fohmixer-hub.toml"), encoding="utf-8") as f:
                text = f.read()
            self.assertIn('[tls]\nname = "foh.e2e.test"\nport = 9443\n', text)
            self.assertIn('jwks_url = "http://127.0.0.1:39999/cdn-cgi/access/certs"\n', text)
        finally:
            remote.stop()
            shutil.rmtree(data, ignore_errors=True)

    def test_the_link_routes_stall_block_and_drop_the_proxy(self):
        status, state = self.harness.handle("GET", "/link", {})
        self.assertEqual(status, 200)
        self.assertEqual(state["port"], self.harness.link.port)
        self.assertGreater(state["port"], 0, "a free port was taken")
        self.assertEqual(
            (state["connections"], state["held"], state["blocked"], state["stall_ms"]),
            (0, 0, False, 0.0),
        )
        status, answer = self.post("/link/stall", {"ms": 400})
        self.assertEqual(status, 200)
        self.assertGreater(answer["stall_ms"], 300)
        self.assertEqual(self.post("/link/block", {"on": True})[1]["blocked"], True)
        held = socket.create_connection(("127.0.0.1", state["port"]), timeout=5)
        try:
            deadline = time.monotonic() + 5
            while self.harness.handle("GET", "/link", {})[1]["held"] != 1:
                self.assertLess(time.monotonic(), deadline, "the connection is held")
                time.sleep(0.01)
            self.assertEqual(self.post("/link/drop"), (200, {"dropped": 1}))
            with self.assertRaises(ConnectionResetError):
                held.recv(1)
        finally:
            held.close()
        self.assertEqual(self.post("/link/block", {"on": False})[1]["blocked"], False)

    def test_the_link_rate_route_sets_and_lifts_the_slow_link(self):
        self.assertEqual(self.harness.handle("GET", "/link", {})[1]["rate"], 0.0)
        status, state = self.post("/link/rate", {"bytes_per_s": 24576})
        self.assertEqual((status, state["rate"]), (200, 24576.0))
        self.assertEqual(self.harness.handle("GET", "/link", {})[1]["rate"], 24576.0)
        self.assertEqual(self.post("/link/rate", {"bytes_per_s": 0})[1]["rate"], 0.0)

    def test_a_link_request_without_its_value_is_refused(self):
        for bad in (
            {},
            {"ms": -1},
            {"ms": "300"},
            {"ms": True},
            {"ms": float("nan")},
            {"ms": float("inf")},
        ):
            with self.assertRaises(harness.BadRequest, msg=repr(bad)):
                self.post("/link/stall", bad)
        for bad in ({}, {"on": 1}, {"on": "true"}):
            with self.assertRaises(harness.BadRequest, msg=repr(bad)):
                self.post("/link/block", bad)
        for bad in (
            {},
            {"bytes_per_s": -1},
            {"bytes_per_s": "1000"},
            {"bytes_per_s": True},
            {"bytes_per_s": float("inf")},
        ):
            with self.assertRaises(harness.BadRequest, msg=repr(bad)):
                self.post("/link/rate", bad)
        self.assertEqual(harness.rate_of({"bytes_per_s": 0}), 0)
        self.assertEqual(harness.rate_of({"bytes_per_s": 24576}), 24576)
        self.assertEqual(harness.stall_ms({"ms": 0}), 0)
        self.assertEqual(harness.stall_ms({"ms": 1500.5}), 1500.5)
        self.assertIs(harness.block_on({"on": False}), False)
        # Over HTTP a bad body is a 400 with the reason.
        server = ThreadingHTTPServer(("127.0.0.1", 0), harness.handler_for(self.harness))
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            request = urllib.request.Request(
                f"http://127.0.0.1:{server.server_address[1]}/link/block",
                data=b'{"on": "yes"}',
                method="POST",
            )
            with self.assertRaises(urllib.error.HTTPError) as raised:
                urllib.request.urlopen(request, timeout=5)
            self.assertEqual(raised.exception.code, 400)
            self.assertIn("/link/block wants", json.load(raised.exception)["error"])
            raised.exception.close()
        finally:
            server.shutdown()
            server.server_close()

    def test_the_forensics_route_runs_the_timeline_over_the_data_folder(self):
        logs = write_window(self.data)
        try:
            body = {"from_ms": BASE, "to_ms": BASE + 5000}
            status, answer = quietly(self.post, "/forensics/timeline", body)
            self.assertEqual((status, answer["exit"]), (200, 0), answer["stderr"])
            self.assertIn("records=3\n", answer["stdout"])
            self.assertIn("<svg", answer["html"])
        finally:
            shutil.rmtree(logs)
        with self.assertRaises(harness.BadRequest):
            self.post("/forensics/timeline", {"from_ms": BASE})
        # Over HTTP a bad body is a 400 with the reason.
        server = ThreadingHTTPServer(("127.0.0.1", 0), harness.handler_for(self.harness))
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            request = urllib.request.Request(
                f"http://127.0.0.1:{server.server_address[1]}/forensics/timeline",
                data=b'{"from_ms": "today"}',
                method="POST",
            )
            with self.assertRaises(urllib.error.HTTPError) as raised:
                urllib.request.urlopen(request, timeout=5)
            self.assertEqual(raised.exception.code, 400)
            self.assertIn("/forensics/timeline wants", json.load(raised.exception)["error"])
            raised.exception.close()
        finally:
            server.shutdown()
            server.server_close()

    def test_which_lines_wait_for_an_answer(self):
        self.assertEqual(harness.expected_answer('rename "a" "b"'), "RENAMED")
        self.assertEqual(harness.expected_answer("listeners mute live_set"), "LISTENERS")
        self.assertEqual(harness.expected_answer('meter "Hand1 #" 1.0'), "METER")
        self.assertIsNone(harness.expected_answer("stall 400"))
        self.assertIsNone(harness.expected_answer(""))

    def test_the_http_api_answers_json_and_reports_errors(self):
        server = ThreadingHTTPServer(("127.0.0.1", 0), harness.handler_for(self.harness))
        threading.Thread(target=server.serve_forever, daemon=True).start()
        base = f"http://127.0.0.1:{server.server_address[1]}"
        try:
            with urllib.request.urlopen(f"{base}/health", timeout=5) as r:
                self.assertEqual(json.load(r), {"ok": True})
            request = urllib.request.Request(
                f"{base}/host/band/line",
                data=json.dumps({"line": "listeners mute live_set tracks 0"}).encode(),
                method="POST",
            )
            with urllib.request.urlopen(request, timeout=5) as r:
                self.assertEqual(json.load(r), {"answer": "LISTENERS 0"})
            bad = urllib.request.Request(f"{base}/hub/layout", data=b"{}", method="POST")
            with self.assertRaises(urllib.error.HTTPError) as raised:
                urllib.request.urlopen(bad, timeout=5)
            self.assertEqual(raised.exception.code, 500)
            self.assertIn("KeyError", json.load(raised.exception)["error"])
            raised.exception.close()
        finally:
            server.shutdown()
            server.server_close()


if __name__ == "__main__":
    unittest.main()
