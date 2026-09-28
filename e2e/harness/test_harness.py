"""The E2E harness against real ``sim/host.py`` processes (no hub: its binary is
built only in the e2e job, where the Playwright suite drives the harness)."""

import json
import os
import shutil
import sys
import tempfile
import threading
import unittest
import urllib.error
import urllib.request
from http.server import ThreadingHTTPServer

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import harness  # noqa: E402

LAYOUT = os.path.join(harness.REPO, "tools", "import-tosc", "fixtures", "expected-layout.json")


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

    def test_which_lines_wait_for_an_answer(self):
        self.assertEqual(harness.expected_answer('rename "a" "b"'), "RENAMED")
        self.assertEqual(harness.expected_answer("listeners mute live_set"), "LISTENERS")
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
