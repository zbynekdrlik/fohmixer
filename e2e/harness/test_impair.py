"""The impair proxy (#43, ``impair.py``) between a client and an echo server:
bytes pass both ways, a stall holds them, a drop resets, a block holds new
connections until it is lifted."""

import contextlib
import io
import json
import os
import socket
import socketserver
import sys
import threading
import time
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import impair  # noqa: E402

# How long a test waits for bytes that must come.
COME_S = 3.0


class Echo(socketserver.BaseRequestHandler):
    """Echoes every byte back; counts the connections that reached it."""

    def handle(self):
        with self.server.lock:
            self.server.reached += 1
        while True:
            try:
                data = self.request.recv(65536)
            except OSError:
                return
            if not data:
                return
            self.request.sendall(data)


class EchoServer(socketserver.ThreadingTCPServer):
    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, host="127.0.0.1"):
        super().__init__((host, 0), Echo)
        self.lock = threading.Lock()
        self.reached = 0


def read_exactly(sock, n, timeout=COME_S):
    """``n`` bytes from ``sock`` within ``timeout``; fewer when it closes."""
    sock.settimeout(timeout)
    data = b""
    while len(data) < n:
        chunk = sock.recv(n - len(data))
        if not chunk:
            break
        data += chunk
    return data


class ImpairTest(unittest.TestCase):
    def setUp(self):
        self.echo = EchoServer()
        threading.Thread(target=self.echo.serve_forever, daemon=True).start()
        self.proxy = impair.Impair(0, self.echo.server_address[1])
        self.proxy.start()
        self.clients = []

    def tearDown(self):
        for client in self.clients:
            client.close()
        self.proxy.stop()
        self.echo.shutdown()
        self.echo.server_close()

    def connect(self):
        client = socket.create_connection(("127.0.0.1", self.proxy.port), timeout=COME_S)
        self.clients.append(client)
        return client

    def wait_for(self, check, what, timeout=COME_S):
        deadline = time.monotonic() + timeout
        while not check():
            if time.monotonic() > deadline:
                self.fail(f"timed out waiting for {what}")
            time.sleep(0.01)

    def test_bytes_pass_both_ways_unchanged(self):
        client = self.connect()
        payload = bytes(range(256)) * 300
        client.sendall(payload)
        self.assertEqual(read_exactly(client, len(payload)), payload)
        self.assertEqual(self.proxy.state()["connections"], 1)
        self.assertEqual(self.proxy.state()["held"], 0)

    def test_a_stall_holds_both_directions_then_delivers_in_order(self):
        client = self.connect()
        client.sendall(b"before")
        self.assertEqual(read_exactly(client, 6), b"before")
        start = time.monotonic()
        answer = self.proxy.stall(400)
        self.assertGreater(answer["stall_ms"], 300)
        client.sendall(b"one")
        client.sendall(b"two")
        self.assertEqual(read_exactly(client, 6), b"onetwo")
        self.assertGreaterEqual(time.monotonic() - start, 0.4, "held for the stall")
        # After the stall, no delay.
        start = time.monotonic()
        client.sendall(b"after")
        self.assertEqual(read_exactly(client, 5), b"after")
        self.assertLess(time.monotonic() - start, 0.3)

    def test_a_running_stall_is_extended_never_shortened(self):
        client = self.connect()
        start = time.monotonic()
        self.proxy.stall(500)
        self.assertLessEqual(self.proxy.stall(100)["stall_ms"], 500)
        self.assertGreater(self.proxy.state()["stall_ms"], 300, "the longer stall stands")
        client.sendall(b"x")
        self.assertEqual(read_exactly(client, 1), b"x")
        self.assertGreaterEqual(time.monotonic() - start, 0.5)
        self.assertEqual(self.proxy.state()["stall_ms"], 0.0)

    def test_a_drop_resets_every_connection(self):
        first, second = self.connect(), self.connect()
        for client in (first, second):
            client.sendall(b"hi")
            self.assertEqual(read_exactly(client, 2), b"hi")
        self.assertEqual(self.proxy.drop(), {"dropped": 2})
        for client in (first, second):
            client.settimeout(COME_S)
            with self.assertRaises(ConnectionResetError, msg="a reset, not an end"):
                client.recv(10)
        self.wait_for(lambda: self.proxy.state()["connections"] == 0, "no connection left")
        # New connections pass again.
        third = self.connect()
        third.sendall(b"again")
        self.assertEqual(read_exactly(third, 5), b"again")

    def test_a_block_holds_new_connections_until_it_is_lifted(self):
        open_before = self.connect()
        open_before.sendall(b"up")
        self.assertEqual(read_exactly(open_before, 2), b"up", "connected through before the block")
        self.assertEqual(self.proxy.block(True)["blocked"], True)
        held = self.connect()
        held.sendall(b"wait")
        self.wait_for(lambda: self.proxy.state()["held"] == 1, "the held connection")
        held.settimeout(0.3)
        with self.assertRaises(socket.timeout, msg="no answer while blocked"):
            held.recv(4)
        self.assertEqual(self.echo.reached, 1, "the held connection never reached the server")
        # A connection made before the block keeps working.
        open_before.sendall(b"ok")
        self.assertEqual(read_exactly(open_before, 2), b"ok")
        state = self.proxy.block(False)
        self.assertEqual(state["blocked"], False)
        self.assertEqual(read_exactly(held, 4), b"wait", "its request goes on once lifted")
        self.assertEqual(self.echo.reached, 2)
        self.assertEqual(self.proxy.state()["held"], 0)

    def test_a_drop_resets_the_held_connections_too(self):
        self.proxy.block(True)
        held = self.connect()
        self.wait_for(lambda: self.proxy.state()["held"] == 1, "the held connection")
        self.assertEqual(self.proxy.drop(), {"dropped": 1})
        held.settimeout(COME_S)
        with self.assertRaises(ConnectionResetError):
            held.recv(4)
        self.proxy.block(False)
        self.assertEqual(self.echo.reached, 0)

    def test_an_end_from_either_side_is_passed_on(self):
        client = self.connect()
        client.sendall(b"bye")
        self.assertEqual(read_exactly(client, 3), b"bye")
        client.shutdown(socket.SHUT_WR)
        # The echo server sees the end and closes: the client reads its end.
        self.assertEqual(read_exactly(client, 1), b"")
        self.wait_for(lambda: self.proxy.state()["connections"] == 0, "the link to end")

    def test_a_listener_that_cannot_bind_fails_loudly(self):
        taken = impair.Impair(self.proxy.port, self.echo.server_address[1])
        with self.assertRaises(RuntimeError) as caught:
            taken.start()
        self.assertIn("cannot listen", str(caught.exception))

    def test_the_upstream_may_be_another_host(self):
        # 127.0.0.2 is loopback too on Linux: an upstream on another address
        # than the listener's (a deployed hub on the LAN, in a live check).
        echo = EchoServer(host="127.0.0.2")
        threading.Thread(target=echo.serve_forever, daemon=True).start()
        proxy = impair.Impair(0, echo.server_address[1], upstream_host="127.0.0.2")
        proxy.start()
        try:
            client = socket.create_connection(("127.0.0.1", proxy.port), timeout=COME_S)
            self.clients.append(client)
            client.sendall(b"far")
            self.assertEqual(read_exactly(client, 3), b"far")
            self.assertEqual(echo.reached, 1)
        finally:
            proxy.stop()
            echo.shutdown()
            echo.server_close()

    def test_a_stall_is_a_finite_number_of_ms(self):
        for bad in (float("inf"), float("nan"), -1, "x"):
            with self.assertRaises(ValueError, msg=repr(bad)):
                self.proxy.stall(bad)
        self.assertEqual(self.proxy.state()["stall_ms"], 0.0, "nothing held")
        self.assertEqual(self.proxy.stall(0)["stall_ms"], 0.0)

    def test_the_tools_control_lines(self):
        client = self.connect()
        client.sendall(b"up")
        self.assertEqual(read_exactly(client, 2), b"up")
        self.assertGreater(impair.command(self.proxy, "stall 300\n")["stall_ms"], 200)
        self.assertEqual(impair.command(self.proxy, "block on")["blocked"], True)
        self.assertEqual(impair.command(self.proxy, "state")["connections"], 1)
        self.assertEqual(impair.command(self.proxy, "block off")["blocked"], False)
        self.assertEqual(impair.command(self.proxy, "drop"), {"dropped": 1})
        for bad in ("stall inf", "stall -5", "stall soon", "block maybe", "drop all", "nap"):
            self.assertIn("error", impair.command(self.proxy, bad), bad)
        self.assertIn("finite", impair.command(self.proxy, "stall inf")["error"])
        self.assertIn("unknown line", impair.command(self.proxy, "nap")["error"])

    def test_the_tool_forwards_and_answers_its_stdin(self):
        r_in, w_in = os.pipe()
        r_out, w_out = os.pipe()
        stdin, feed = os.fdopen(r_in, "r"), os.fdopen(w_in, "w")
        stdout, answers = os.fdopen(w_out, "w"), os.fdopen(r_out, "r")
        upstream = f"127.0.0.1:{self.echo.server_address[1]}"
        ended = {}

        def run():
            ended["code"] = impair.main(["--upstream", upstream], stdin, stdout)

        tool = threading.Thread(target=run, daemon=True)
        tool.start()
        try:
            first = answers.readline()
            self.assertRegex(first, r"^IMPAIR [0-9]+\n$")
            client = socket.create_connection(("127.0.0.1", int(first.split()[1])), timeout=COME_S)
            self.clients.append(client)
            client.sendall(b"through")
            self.assertEqual(read_exactly(client, 7), b"through", "bytes pass the tool's proxy")
            feed.write("state\n\nblock on\n")
            feed.flush()
            self.assertEqual(json.loads(answers.readline())["connections"], 1)
            self.assertEqual(
                json.loads(answers.readline())["blocked"], True, "no answer to a blank line"
            )
        finally:
            feed.close()
            tool.join(COME_S)
            stdout.close()
            answers.close()
            stdin.close()
        self.assertEqual(ended, {"code": 0}, "the end of stdin stops it")

    def test_the_tools_arguments(self):
        args = impair.parse_args(["--upstream", "192.0.2.10:8480", "--listen-port", "9"])
        self.assertEqual(
            (args.upstream_host, args.upstream_port, args.listen_port, args.listen_host),
            ("192.0.2.10", 8480, 9, "127.0.0.1"),
        )
        args = impair.parse_args(["--upstream", "hub.example.org:80", "--listen-host", "0.0.0.0"])
        self.assertEqual((args.upstream_host, args.listen_host), ("hub.example.org", "0.0.0.0"))
        for bad in ("192.0.2.10", ":8480", "host:port", "[::1]:8480", "::1:8480"):
            with self.assertRaises(SystemExit, msg=bad), contextlib.redirect_stderr(io.StringIO()):
                impair.parse_args(["--upstream", bad])

    def test_a_dead_upstream_resets_the_client(self):
        dead = socket.socket()
        dead.bind(("127.0.0.1", 0))
        port = dead.getsockname()[1]
        dead.close()
        proxy = impair.Impair(0, port)
        proxy.start()
        try:
            client = socket.create_connection(("127.0.0.1", proxy.port), timeout=COME_S)
            self.clients.append(client)
            client.settimeout(COME_S)
            with self.assertRaises(ConnectionResetError):
                client.recv(1)
        finally:
            proxy.stop()


if __name__ == "__main__":
    unittest.main()
