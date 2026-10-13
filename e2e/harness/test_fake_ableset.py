"""The fake AbleSet (#90 B3b, ``fake_ableset.py``) as the hub and a browser
see it: the page and its assets over HTTP/1.1 with keep-alive, the press,
the hits (the hub's link reads told apart), a Socket.IO-like WebSocket echo,
and AbleSet stopped and started again."""

import base64
import json
import os
import socket
import sys
import time
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import fake_ableset  # noqa: E402

# How long a test waits for an answer that must come.
COME_S = 3.0
NAME = "ableset.e2e.test:8443"
KEY = "dGhlIHNhbXBsZSBub25jZQ=="


class Conn:
    """A client of the fake: requests out, answers in."""

    def __init__(self, port):
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=COME_S)
        self.file = self.sock.makefile("rb")

    def send(self, data):
        self.sock.sendall(data)

    def ask(self, method, target, headers=(), body=b""):
        """One request: (status, headers by lowercase name, body)."""
        lines = [f"{method} {target} HTTP/1.1", f"Host: {NAME}", *headers]
        if body:
            lines.append(f"Content-Length: {len(body)}")
        self.send(("\r\n".join(lines) + "\r\n\r\n").encode() + body)
        return self.answer()

    def answer(self):
        status = int(self.file.readline().split()[1])
        fields = {}
        while (line := self.file.readline().decode().rstrip("\r\n")) != "":
            name, _, value = line.partition(":")
            fields[name.lower()] = value.strip()
        body = self.file.read(int(fields.get("content-length", "0")))
        return status, fields, body

    def closed(self):
        """Whether the fake closed the connection (an EOF within the wait)."""
        return self.file.read(1) == b""

    def frame(self, opcode, payload):
        """A masked client frame."""
        mask = b"\x01\x02\x03\x04"
        size = len(payload)
        length = bytes([0x80 | size]) if size < 126 else bytes([0x80 | 126]) + size.to_bytes(2)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        self.send(bytes([0x80 | opcode]) + length + mask + masked)

    def read_frame(self):
        """The fake's next frame: (opcode, payload)."""
        first, second = self.file.read(2)
        size = second & 0x7F
        if size == 126:
            size = int.from_bytes(self.file.read(2))
        return first & 0x0F, self.file.read(size)

    def close(self):
        self.file.close()
        self.sock.close()


class FakeAbleSetTest(unittest.TestCase):
    def setUp(self):
        self.fake = fake_ableset.FakeAbleSet(0)
        self.fake.start()
        self.addCleanup(self.fake.stop)

    def connect(self):
        conn = Conn(self.fake.port)
        self.addCleanup(conn.close)
        return conn

    def hits(self):
        return self.fake.state()["hits"]

    def test_the_page_and_its_assets_on_one_kept_connection(self):
        conn = self.connect()
        status, fields, page = conn.ask("GET", "/")
        self.assertEqual((status, fields["content-type"]), (200, "text/html; charset=utf-8"))
        self.assertNotIn("connection", fields, "kept alive")
        self.assertEqual(
            (fields["x-frame-options"], fields["content-security-policy"]),
            ("SAMEORIGIN", "frame-ancestors 'self'"),
        )
        text = page.decode()
        for part in (
            '<script src="/static/app.js">',
            '<link rel="stylesheet" href="/static/app.css">',
            '<link rel="icon" href="data:,">',
            '<button id="fake-button"',
            "<li>Song 200</li>",
        ):
            self.assertIn(part, text)
        status, fields, script = conn.ask("GET", "/static/app.js")
        self.assertEqual((status, fields["content-type"]), (200, "text/javascript; charset=utf-8"))
        for part in ("localStorage", '"/socket.io/?"', "browserId", '"/api/press"', "data"):
            self.assertIn(part, script.decode())
        status, fields, style = conn.ask("GET", "/static/app.css")
        self.assertEqual((status, fields["content-type"]), (200, "text/css; charset=utf-8"))
        self.assertNotIn("x-frame-options", fields, "the page's only")
        self.assertIn(b"rgb(1, 2, 3)", style)
        status, fields, gif = conn.ask("GET", "/static/pixel.gif?n=1")
        self.assertEqual((status, fields["content-type"], gif[:6]), (200, "image/gif", b"GIF89a"))
        self.assertEqual(conn.ask("GET", "/nothing")[0], 404)
        self.assertEqual(conn.ask("POST", "/static/app.js")[0], 404, "only a GET of an asset")
        self.assertEqual(len(self.hits()), 6, "one connection, six requests")

    def test_a_request_asking_to_close_is_answered_and_closed(self):
        conn = self.connect()
        status, fields, _ = conn.ask("GET", "/", ["Connection: close"])
        self.assertEqual((status, fields["connection"]), (200, "close"))
        self.assertTrue(conn.closed())
        old = self.connect()
        old.send(b"GET / HTTP/1.0\r\nHost: x\r\n\r\n")
        self.assertEqual(old.answer()[1]["connection"], "close")
        self.assertTrue(old.closed())

    def test_a_press_is_counted(self):
        conn = self.connect()
        body = json.dumps({"browserId": "b1"}).encode()
        for n in (1, 2):
            status, fields, answer = conn.ask(
                "POST", "/api/press", ["Content-Type: application/json"], body
            )
            self.assertEqual((status, json.loads(answer)), (200, {"pressed": n}))
        self.assertEqual(self.fake.state()["presses"], 2)
        chunked = self.connect()
        self.assertEqual(chunked.ask("POST", "/api/press", ["Transfer-Encoding: chunked"])[0], 411)
        self.assertTrue(chunked.closed())

    def test_each_request_is_a_hit_and_the_hubs_link_reads_are_told_apart(self):
        conn = self.connect()
        conn.ask(
            "GET",
            "/socket.io/?EIO=4&transport=polling&browserId=b7",
            [
                "User-Agent: Mozilla/5.0",
                "Cookie: keep=1; other=2",
                "X-Forwarded-For: 192.0.2.1",
                "cf-ray: r",
            ],
        )
        link = self.connect()
        link.ask("GET", "/", ["User-Agent: fohmixer-hub/1.0.1", "Accept: */*", "Connection: close"])
        hits = self.hits()
        self.assertEqual(len(hits), 2)
        page, read = hits
        self.assertEqual(
            {k: page[k] for k in ("method", "path", "host", "agent", "link", "cookies")},
            {
                "method": "GET",
                "path": "/socket.io/?EIO=4&transport=polling&browserId=b7",
                "host": NAME,
                "agent": "Mozilla/5.0",
                "link": False,
                "cookies": ["keep", "other"],
            },
        )
        self.assertEqual(page["browser_id"], "b7")
        self.assertEqual(
            page["headers"], ["cf-ray", "cookie", "host", "user-agent", "x-forwarded-for"]
        )
        self.assertLessEqual(page["at"], time.time() * 1000.0)
        self.assertEqual((read["link"], read["browser_id"]), (True, None))
        self.assertEqual(fake_ableset.page_hits(hits), [page])
        self.assertEqual(self.fake.clear()["hits"], [])

    def test_a_websocket_answers_engine_ios_packets_and_echoes_the_rest(self):
        conn = self.connect()
        conn.send(
            (
                "GET /socket.io/?EIO=4&transport=websocket&browserId=b9 HTTP/1.1\r\n"
                f"Host: {NAME}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n"
                f"Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {KEY}\r\n\r\n"
            ).encode()
        )
        status, fields, _ = conn.answer()
        self.assertEqual(status, 101)
        # RFC 6455 §1.3's own example.
        self.assertEqual(fields["sec-websocket-accept"], "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=")
        self.assertEqual((fields["upgrade"], fields["connection"]), ("websocket", "Upgrade"))
        self.assertEqual(conn.read_frame(), (0x1, fake_ableset.OPEN_PACKET.encode()))
        self.assertEqual(self.fake.state()["sockets"], 1)
        conn.frame(0x1, b"40")
        self.assertEqual(conn.read_frame(), (0x1, b'40{"sid":"fake-ns"}'))
        conn.frame(0x1, b"2")
        self.assertEqual(conn.read_frame(), (0x1, b"3"))
        echo = '42["echo","b9 é"]'.encode()
        conn.frame(0x1, echo)
        self.assertEqual(conn.read_frame(), (0x1, echo))
        big = bytes(range(256)) * 4
        conn.frame(0x2, big)
        self.assertEqual(conn.read_frame(), (0x2, big))
        conn.frame(0x9, b"p")
        self.assertEqual(conn.read_frame(), (0xA, b"p"))
        conn.frame(0x8, (1000).to_bytes(2) + b"bye")
        self.assertEqual(conn.read_frame(), (0x8, (1000).to_bytes(2)))
        self.assertTrue(conn.closed())
        self.assertEqual(self.fake.state()["sockets"], 0)
        self.assertEqual(self.hits()[0]["browser_id"], "b9")

    def test_a_websocket_without_its_key_is_refused(self):
        conn = self.connect()
        upgrade = ["Connection: keep-alive, Upgrade", "Upgrade: websocket"]
        self.assertEqual(conn.ask("GET", "/socket.io/?transport=websocket", upgrade)[0], 400)
        self.assertTrue(conn.closed())

    def test_down_refuses_connects_and_closes_the_open_ones_up_serves_again(self):
        port = self.fake.port
        conn = self.connect()
        conn.ask("GET", "/")
        state = self.fake.down()
        self.assertEqual((state["down"], state["port"]), (True, port))
        self.assertTrue(conn.closed())
        with self.assertRaises(ConnectionRefusedError):
            socket.create_connection(("127.0.0.1", port), timeout=COME_S)
        self.assertEqual(self.fake.up()["down"], False)
        self.assertEqual(self.connect().ask("GET", "/static/app.css")[0], 200)

    def test_the_pure_parts(self):
        request = fake_ableset.parse_head(
            b"GET /x HTTP/1.1\r\nHost: h\r\nConnection: keep-alive, Upgrade\r\n"
            b"upgrade: WebSocket\r\n\r\n"
        )
        self.assertEqual(request.headers[0], ("host", "h"))
        self.assertTrue(fake_ableset.is_websocket(request))
        self.assertTrue(fake_ableset.keeps_alive(request))
        self.assertFalse(fake_ableset.is_websocket(request._replace(method="POST")))
        self.assertFalse(fake_ableset.keeps_alive(request._replace(version="HTTP/1.0")))
        for bad in (b"GET /x\r\n\r\n", b"GET /x HTTP/2\r\n\r\n", b"GET / HTTP/1.1\r\nbad\r\n\r\n"):
            self.assertIsNone(fake_ableset.parse_head(bad), bad)
        self.assertEqual(fake_ableset.frame(0x1, b"a" * 126)[:4], b"\x81\x7e\x00\x7e")
        self.assertEqual(fake_ableset.frame(0x2, b"a" * 65536)[:2], b"\x82\x7f")
        self.assertEqual(fake_ableset.reply('42["x"]'), '42["x"]')
        self.assertEqual(base64.b64decode(fake_ableset.accept_key(KEY)).__len__(), 20)


if __name__ == "__main__":
    unittest.main()
