"""The WebSocket transport: real sockets on an ephemeral port, a real WebSocket client."""

import base64
import json
import logging
import os
import socket
import statistics
import time
import unittest

import _paths  # noqa: F401 - puts the script on sys.path
from FohMixer.transport.server import Server
from websockets.exceptions import ConnectionClosed, InvalidStatus
from websockets.sync.client import connect

TIME_WAIT = "06"


def quiet_logger():
    logger = logging.getLogger("fohmixer.transport-test")
    logger.propagate = False
    if not logger.handlers:
        logger.addHandler(logging.NullHandler())
    return logger


def recv_json(ws, timeout=2.0):
    return json.loads(ws.recv(timeout=timeout))


def wait_for(predicate, timeout=2.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.005)
    raise AssertionError("condition not met in time")


def raw_client(port, rcvbuf=4096):
    """A WebSocket client that completes the handshake and then never reads."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, rcvbuf)
    sock.connect(("127.0.0.1", port))
    key = base64.b64encode(os.urandom(16)).decode("ascii")
    sock.sendall(
        (
            f"GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\n"
            f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        ).encode("ascii")
    )
    response = b""
    while b"\r\n\r\n" not in response:
        response += sock.recv(1)
    assert response.startswith(b"HTTP/1.1 101"), response
    return sock


def server_side_time_wait(port):
    """Server-side sockets of ``port`` in TIME_WAIT, from /proc/net/tcp (Linux)."""
    with open("/proc/net/tcp", encoding="ascii") as f:
        rows = [line.split() for line in f.readlines()[1:]]
    return [r for r in rows if int(r[1].split(":")[1], 16) == port and r[3] == TIME_WAIT]


class TransportTest(unittest.TestCase):
    def setUp(self):
        self.server = self.start_server(0)
        self.port = self.server.port
        self.url = f"ws://127.0.0.1:{self.port}"
        self.clients = []

    def tearDown(self):
        for ws in self.clients:
            ws.close()
        self.server.shutdown()

    def start_server(self, port, result_queue_max=1000):
        server = Server("127.0.0.1", port, {"instance": "test"}, result_queue_max, quiet_logger())
        server.start()
        self.assertTrue(server.wait_bound(2.0))
        return server

    def client(self):
        before = {c.id for c in self.server.connections()}
        ws = connect(self.url, open_timeout=2, close_timeout=1)
        self.clients.append(ws)
        hello = recv_json(ws)
        conn = wait_for(lambda: [c for c in self.server.connections() if c.id not in before])[0]
        return ws, conn, hello

    def test_handshake_and_connect_event(self):
        _ws, _conn, hello = self.client()
        self.assertEqual(hello["event"], "connect")
        self.assertEqual(hello["data"], {"instance": "test"})
        self.assertIsInstance(hello["ts"], int)

    def test_values_for_one_key_coalesce(self):
        ws, conn, _ = self.client()
        with conn.batch():
            conn.push_value("k", {"key": "k", "value": 1})
            conn.push_value("k", {"key": "k", "value": 2})
            conn.push_value("k", {"key": "k", "value": 3})
        frame = recv_json(ws)
        self.assertEqual(frame["event"], "values")
        self.assertEqual(frame["data"], [{"key": "k", "value": 3}])
        with self.assertRaises(TimeoutError):
            ws.recv(timeout=0.2)

    def test_results_keep_order_and_precede_values(self):
        ws, conn, _ = self.client()
        with conn.batch():
            conn.push_value("k", {"key": "k", "value": 1})
            for uuid in ("u1", "u2", "u3"):
                conn.push_result(uuid, [{"ok": True, "data": uuid}])
        frames = [recv_json(ws) for _ in range(4)]
        self.assertEqual([f.get("uuid") for f in frames[:3]], ["u1", "u2", "u3"])
        self.assertEqual(frames[0]["data"], [{"ok": True, "data": "u1"}])
        self.assertEqual(frames[3]["event"], "values")

    def test_heartbeat_keeps_only_the_latest(self):
        ws, conn, _ = self.client()
        with conn.batch():
            for age in (1, 2, 3):
                self.server.broadcast_heartbeat({"main_tick_age_ms": age})
        frame = recv_json(ws)
        self.assertEqual(frame["event"], "heartbeat")
        self.assertEqual(frame["data"], {"main_tick_age_ms": 3})
        with self.assertRaises(TimeoutError):
            ws.recv(timeout=0.2)

    def test_malformed_requests_get_error_events_and_the_connection_stays_open(self):
        ws, conn, _ = self.client()
        ws.send("not json")
        error = recv_json(ws)
        self.assertEqual(error["event"], "error")
        self.assertIsNone(error["uuid"])
        self.assertIn("invalid JSON", error["data"])
        ws.send(json.dumps({"uuid": "u7"}))
        self.assertEqual(recv_json(ws)["uuid"], "u7")
        ws.send(json.dumps([1, 2]))
        self.assertEqual(recv_json(ws)["event"], "error")
        ws.send(json.dumps({"uuid": "u9", "commands": "x"}))
        self.assertEqual(recv_json(ws)["uuid"], "u9")
        ws.send(b"\x00\x01")
        self.assertIn("binary", recv_json(ws)["data"])
        ws.send(json.dumps({"uuid": "u8", "commands": []}))
        got_conn, payload = self.server.inbox.get(timeout=2.0)
        self.assertIs(got_conn, conn)
        self.assertEqual(payload, {"uuid": "u8", "commands": []})
        self.assertTrue(conn.is_open)

    def test_ping_gets_pong(self):
        ws, _conn, _ = self.client()
        self.assertTrue(ws.ping().wait(2.0))

    def test_client_close_drops_the_connection(self):
        ws, conn, _ = self.client()
        ws.close()
        got_conn, payload = self.server.inbox.get(timeout=2.0)
        self.assertIs(got_conn, conn)
        self.assertIsNone(payload)
        wait_for(lambda: conn not in self.server.connections())
        self.assertTrue(conn.finished.wait(2.0))

    def test_browser_origin_is_refused(self):
        with self.assertRaises(InvalidStatus):
            connect(self.url, origin="http://example.com", open_timeout=2)

    def test_non_reading_client_never_blocks_and_overflow_closes_only_it(self):
        good_ws, good_conn, _ = self.client()
        before = {c.id for c in self.server.connections()}
        stalled = raw_client(self.port)
        self.addCleanup(stalled.close)
        conn = wait_for(lambda: [c for c in self.server.connections() if c.id not in before])[0]
        # Fill the socket buffers until the sender blocks in sendall: then a pushed
        # result stays queued.
        for _ in range(64):
            conn.push_result("fill", "x" * (1024 * 1024))
            time.sleep(0.1)
            if conn.pending()[0]:
                break
        base = conn.pending()[0]
        self.assertGreater(base, 0, "the sender never blocked")
        durations = []
        for i in range(5000):
            started = time.perf_counter()
            conn.push_value(f"k{i}", {"key": f"k{i}", "value": i / 5000})
            durations.append(time.perf_counter() - started)
        for i in range(900):
            started = time.perf_counter()
            self.assertTrue(conn.push_result(f"r{i}", [{"ok": True, "data": i}]))
            durations.append(time.perf_counter() - started)
        durations.sort()
        self.assertLess(statistics.median(durations), 0.0001)
        self.assertLess(durations[int(len(durations) * 0.99)], 0.001)
        self.assertLess(durations[-1], 0.5)  # a blocked send would take the 3 s timeout
        self.assertTrue(conn.is_open)
        self.assertEqual(conn.pending(), (base + 900, 5000))
        accepted = 0
        while conn.push_result(f"extra{accepted}", []):
            accepted += 1
            self.assertLess(accepted, 200)
        self.assertEqual(accepted, 1000 - base - 900)
        self.assertFalse(conn.is_open)
        self.assertTrue(conn.finished.wait(2.0))
        self.assertTrue(good_conn.is_open)
        good_conn.push_value("still", {"key": "still", "value": "alive"})
        self.assertEqual(recv_json(good_ws)["data"], [{"key": "still", "value": "alive"}])

    def test_shutdown_then_rebind_at_once_without_time_wait(self):
        ws, _conn, _ = self.client()
        self.server.broadcast("disconnect")
        started = time.monotonic()
        self.server.shutdown()
        self.assertLess(time.monotonic() - started, 1.0)
        self.assertEqual(recv_json(ws)["event"], "disconnect")
        with self.assertRaises(ConnectionClosed):
            ws.recv(timeout=2.0)
        self.assertEqual(server_side_time_wait(self.port), [])
        started = time.monotonic()
        self.server = self.start_server(self.port)
        self.assertLess(time.monotonic() - started, 0.5)
        _ws2, _conn2, hello = self.client()
        self.assertEqual(hello["event"], "connect")

    def test_shutdown_aborts_a_client_that_does_not_answer_close(self):
        before = {c.id for c in self.server.connections()}
        stalled = raw_client(self.port)
        self.addCleanup(stalled.close)
        conn = wait_for(lambda: [c for c in self.server.connections() if c.id not in before])[0]
        started = time.monotonic()
        self.server.shutdown()
        self.assertLess(time.monotonic() - started, 1.0)
        self.assertTrue(conn.finished.wait(1.0))
        self.assertEqual(server_side_time_wait(self.port), [])


if __name__ == "__main__":
    unittest.main()
