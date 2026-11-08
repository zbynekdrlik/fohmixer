"""The fake Presenter (#90, ``fake_presenter.py``) as the hub and the stage
page see it: its page and script, its list of layouts, the switch and its
refusals, the socket's events, down and up, and the tool in a process of its
own."""

import base64
import http.client
import json
import os
import socket
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import fake_presenter  # noqa: E402


def call(port, method, path, body=None):
    """One request: (status, body bytes)."""
    conn = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
    try:
        data = None if body is None else json.dumps(body).encode()
        headers = {} if data is None else {"Content-Type": "application/json"}
        conn.request(method, path, body=data, headers=headers)
        answer = conn.getresponse()
        return answer.status, answer.read()
    finally:
        conn.close()


def open_socket(port):
    """A WebSocket to ``/live/ws`` past its 101: the raw socket."""
    raw = socket.create_connection(("127.0.0.1", port), timeout=5)
    key = base64.b64encode(os.urandom(16)).decode()
    raw.sendall(
        (
            "GET /live/ws?surface=stage&preview=1 HTTP/1.1\r\nHost: x\r\n"
            "Connection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\n"
            f"Sec-WebSocket-Key: {key}\r\n\r\n"
        ).encode()
    )
    head = b""
    while b"\r\n\r\n" not in head:
        head += raw.recv(1)
    assert head.startswith(b"HTTP/1.1 101"), head
    return raw


def read_text(raw):
    """One unmasked text frame from the fake: its text."""
    first, second = raw.recv(1)[0], raw.recv(1)[0]
    assert first == 0x81, first
    size = second & 0x7F
    data = b""
    while len(data) < size:
        data += raw.recv(size - len(data))
    return data.decode()


class Fake(unittest.TestCase):
    def setUp(self):
        self.fake = fake_presenter.FakePresenter(0)
        self.fake.start()
        self.addCleanup(self.fake.stop)

    def test_the_stage_page_its_script_and_the_layouts_are_served(self):
        status, body = call(self.fake.port, "GET", "/stage")
        self.assertEqual(status, 200)
        self.assertIn(b'src="/ui-pkg/stage.js"', body)
        status, body = call(self.fake.port, "GET", "/ui-pkg/stage.js")
        self.assertEqual(status, 200)
        self.assertIn(b"/live/ws?surface=stage&preview=1", body)
        status, body = call(self.fake.port, "GET", "/stage-displays")
        layouts = json.loads(body)
        self.assertEqual(status, 200)
        self.assertEqual(len(layouts), 9)
        self.assertEqual(layouts[0]["code"], "worship")
        self.assertEqual(set(layouts[0]), {"code", "name", "description"})
        status, body = call(self.fake.port, "GET", "/stage/layout")
        self.assertEqual((status, json.loads(body)["code"]), (200, "worship"))
        self.assertEqual(call(self.fake.port, "GET", "/nothing")[0], 404)

    def test_a_switch_is_answered_as_presenter_answers_it_and_told_to_every_socket(self):
        raw = open_socket(self.fake.port)
        self.addCleanup(raw.close)
        status, body = call(self.fake.port, "POST", "/stage/layout", {"code": "timer"})
        self.assertEqual(status, 200)
        self.assertEqual(json.loads(body)["layout"]["name"], "TIMER")
        self.assertEqual(json.loads(read_text(raw)), {"type": "stage_layout", "code": "timer"})
        self.assertEqual(call(self.fake.port, "POST", "/stage/layout", {"code": ""})[0], 400)
        self.assertEqual(call(self.fake.port, "POST", "/stage/layout", {"code": "x"})[0], 404)
        self.assertEqual(self.fake.state()["current"], "timer")
        # The operator's own switch reaches the socket too.
        self.assertEqual(self.fake.switch("bible")["current"], "bible")
        self.assertEqual(json.loads(read_text(raw))["code"], "bible")
        self.assertIn("error", self.fake.switch("nope"))
        self.assertEqual(self.fake.state()["sockets"], 1)

    def test_any_other_write_is_counted_and_nothing_changes(self):
        self.assertEqual(call(self.fake.port, "POST", "/stage/state", {"a": 1})[0], 200)
        hits = self.fake.state()["hits"]
        self.assertEqual(hits[-1]["method"], "POST")
        self.assertEqual(hits[-1]["path"], "/stage/state")
        self.assertEqual(self.fake.state()["current"], "worship")
        self.assertEqual(self.fake.clear()["hits"], [])

    def test_down_refuses_a_connect_and_up_listens_again(self):
        port = self.fake.port
        self.assertTrue(self.fake.down()["down"])
        with self.assertRaises(OSError):
            call(port, "GET", "/stage")
        self.assertFalse(self.fake.up()["down"])
        self.assertEqual(call(port, "GET", "/stage")[0], 200)

    def test_a_line_it_does_not_know_is_an_error(self):
        self.assertIn("unknown line", fake_presenter.command(self.fake, "nothing")["error"])
        answer = fake_presenter.command(self.fake, " switch  preach ")
        self.assertEqual(answer["current"], "preach")
        self.assertEqual(fake_presenter.command(self.fake, "state")["port"], self.fake.port)


class ChildTest(unittest.TestCase):
    def test_the_tool_in_a_process_of_its_own(self):
        work = fake_presenter.Child(0)
        work.start()
        self.addCleanup(work.stop)
        self.assertEqual(call(work.port, "GET", "/stage")[0], 200)
        self.assertEqual(work.switch("timer")["current"], "timer")
        self.assertTrue(work.down()["down"])
        self.assertFalse(work.up()["down"])
        self.assertEqual(work.clear()["hits"], [])
        self.assertEqual(work.state()["current"], "timer")
        work.stop()
        self.assertEqual(work.proc.poll(), 0)


if __name__ == "__main__":
    unittest.main()
