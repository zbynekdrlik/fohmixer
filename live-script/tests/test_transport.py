"""The WebSocket transport: real sockets on an ephemeral port, real WebSocket clients.

The server runs no threads (#5): Live's main thread polls it in its timer tick
(``poll_in``, then the work, then ``poll_out``). ``TransportTest`` gives it a
SimLive ``MainThread`` whose 10 ms timer polls it, and pushes through
``call``, as the surface does; ``OneTickTest`` makes the test thread Live's
main thread and ticks by hand, which pins what one tick does and that nothing
moves between ticks.
"""

import errno
import itertools
import json
import logging
import select
import socket
import time
import unittest

import _paths  # noqa: F401 - puts the script and SimLive on sys.path
import Live
from _rawclient import CLOSE, RawClient, masked_frame, request_frame, upgrade_request
from FohMixer.transport import server as transport
from FohMixer.transport.server import Server
from FohMixer.transport.websocket import OPCODE_TEXT
from main_thread import MainThread
from websockets.exceptions import ConnectionClosed, InvalidStatus
from websockets.sync.client import connect

TIME_WAIT = "06"
TICK_MS = 10


class RecordingHandler(logging.Handler):
    """Keeps the server's log messages (WARNING and up) for the tests to read."""

    def __init__(self):
        super().__init__(logging.WARNING)
        self.messages = []

    def emit(self, record):
        self.messages.append(record.getMessage())


class FakeListener:
    """The server's listening socket, whose accept() raises ``errors`` first
    (one per call) and then accepts: accept failures a test cannot cause."""

    def __init__(self, sock, errors):
        self.sock = sock
        self.errors = list(errors)

    def fileno(self):
        return self.sock.fileno()

    def accept(self):
        if self.errors:
            raise self.errors.pop(0)
        return self.sock.accept()

    def __getattr__(self, name):
        return getattr(self.sock, name)


def recording_logger(name):
    logger = logging.getLogger(f"fohmixer.transport-test.{name}")
    logger.propagate = False
    handler = RecordingHandler()
    logger.handlers = [handler]
    return logger, handler


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


def tcp_wmem_max():
    """The kernel's largest TCP send buffer (Linux: the third field of ``tcp_wmem``)."""
    with open("/proc/sys/net/ipv4/tcp_wmem", encoding="ascii") as f:
        return int(f.read().split()[2])


def send_buffer_frames(frame_bytes):
    """How many frames of ``frame_bytes`` the largest TCP send buffer holds, rounded up."""
    return -(-tcp_wmem_max() // frame_bytes)


def server_side_time_wait(port):
    """Server-side sockets of ``port`` in TIME_WAIT, from /proc/net/tcp (Linux)."""
    with open("/proc/net/tcp", encoding="ascii") as f:
        rows = [line.split() for line in f.readlines()[1:]]
    return [r for r in rows if int(r[1].split(":")[1], 16) == port and r[3] == TIME_WAIT]


class TransportTest(unittest.TestCase):
    """The server polled by a SimLive main thread every 10 ms, as the surface polls it."""

    def setUp(self):
        self.logger, self.log = recording_logger(self.id())
        self.mt = MainThread().start()
        self.addCleanup(self.mt.stop)
        self.server = self.start_server(0)
        self.timer = Live.Base.Timer(callback=self.tick, interval=TICK_MS, repeat=True)
        self.timer.start()
        self.port = self.server.port
        self.url = f"ws://127.0.0.1:{self.port}"
        self.clients = []

    def tearDown(self):
        for ws in self.clients:
            ws.close()
        self.call(self.server.shutdown)
        self.call(self.timer.stop)

    def tick(self):
        self.server.poll_in()
        self.server.poll_out()

    def call(self, fn):
        """Run ``fn`` on the main thread, between two ticks."""
        return self.mt.call(fn)

    def start_server(self, port, result_queue_max=1000):
        server = Server("127.0.0.1", port, {"instance": "test"}, result_queue_max, self.logger)
        server.start()
        self.assertTrue(server.wait_bound(2.0))
        return server

    def connections(self):
        return self.call(self.server.connections)

    def new_connection(self, before):
        return wait_for(lambda: [c for c in self.connections() if c.id not in before])[0]

    def client(self):
        before = {c.id for c in self.connections()}
        ws = connect(self.url, open_timeout=2, close_timeout=1)
        self.clients.append(ws)
        hello = recv_json(ws)
        return ws, self.new_connection(before), hello

    def stalled_client(self):
        """A client that completes the handshake and then never reads (4 KB window)."""
        before = {c.id for c in self.connections()}
        stalled = RawClient(self.port, rcvbuf=4096)
        self.addCleanup(stalled.close)
        return stalled, self.new_connection(before)

    def fill(self, conn):
        """Push 1 MB results until the socket takes no more: the queued results then."""
        for _ in range(64):
            self.call(lambda: conn.push_result("fill", "x" * (1024 * 1024)))
            time.sleep(0.05)
            queued = self.call(lambda: conn.pending()[0])
            if queued:
                return queued
        raise AssertionError("the socket never filled")

    def test_handshake_and_connect_event(self):
        _ws, _conn, hello = self.client()
        self.assertEqual(hello["event"], "connect")
        self.assertEqual(hello["data"], {"instance": "test"})
        self.assertIsInstance(hello["ts"], int)

    def test_values_for_one_key_coalesce(self):
        ws, conn, _ = self.client()

        def push():
            for value in (1, 2, 3):
                conn.push_value("k", {"key": "k", "value": value})

        self.call(push)
        frame = recv_json(ws)
        self.assertEqual(frame["event"], "values")
        self.assertEqual(frame["data"], [{"key": "k", "value": 3}])
        with self.assertRaises(TimeoutError):
            ws.recv(timeout=0.2)

    def test_results_keep_order_and_precede_values(self):
        ws, conn, _ = self.client()

        def push():
            conn.push_value("k", {"key": "k", "value": 1})
            for uuid in ("u1", "u2", "u3"):
                conn.push_result(uuid, [{"ok": True, "data": uuid}])

        self.call(push)
        frames = [recv_json(ws) for _ in range(4)]
        self.assertEqual([f.get("uuid") for f in frames[:3]], ["u1", "u2", "u3"])
        self.assertEqual(frames[0]["data"], [{"ok": True, "data": "u1"}])
        self.assertEqual(frames[3]["event"], "values")

    def test_heartbeat_keeps_only_the_latest(self):
        ws, _conn, _ = self.client()

        def beat():
            for age in (1, 2, 3):
                self.server.broadcast_heartbeat({"main_tick_age_ms": age})

        self.call(beat)
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
        self.assertNotIn(conn, self.connections())
        self.assertTrue(conn.finished)

    def test_browser_origin_is_refused(self):
        with self.assertRaises(InvalidStatus):
            connect(self.url, origin="http://example.com", open_timeout=2)

    def test_several_connections_each_get_their_own_answers(self):
        clients = [self.client() for _ in range(3)]
        for n, (ws, _conn, _hello) in enumerate(clients):
            for k in range(5):
                ws.send(json.dumps({"uuid": f"c{n}-{k}", "commands": []}))
        # The test plays the drain: every request is answered on its own connection.
        for _ in range(15):
            conn, payload = self.server.inbox.get(timeout=2.0)
            self.call(
                lambda c=conn, p=payload: c.push_result(p["uuid"], [{"ok": True, "data": c.id}])
            )
        for n, (ws, conn, _hello) in enumerate(clients):
            results = [recv_json(ws) for _ in range(5)]
            self.assertEqual([r["uuid"] for r in results], [f"c{n}-{k}" for k in range(5)])
            self.assertEqual({r["data"][0]["data"] for r in results}, {conn.id})

    def test_non_reading_client_never_blocks_and_overflow_closes_only_it(self):
        good_ws, good_conn, _ = self.client()
        _stalled, conn = self.stalled_client()
        self.fill(conn)

        def pushes():
            base, _keys, unsent = conn.pending()
            for i in range(5000):
                conn.push_value(f"k{i}", {"key": f"k{i}", "value": i / 5000})
            for i in range(900):
                self.assertTrue(conn.push_result(f"r{i}", [{"ok": True, "data": i}]))
            still_open, pending = conn.is_open, conn.pending()
            accepted = 0
            while conn.push_result(f"extra{accepted}", []):
                accepted += 1
                if accepted > 200:
                    break
            return base, unsent, still_open, pending, accepted, conn.is_open

        # All in one call on the main thread, between two ticks: the pushes
        # only queue. One that wrote to the full socket would wait for ever
        # (the client reads nothing) and the call would time out; the bytes
        # waiting for the socket do not change.
        base, unsent, still_open, pending, accepted, open_after = self.call(pushes)
        self.assertTrue(still_open)
        self.assertEqual(pending, (base + 900, 5000, unsent))
        self.assertEqual(accepted, 1000 - base - 900)
        self.assertFalse(open_after)
        wait_for(lambda: conn.finished)
        self.assertTrue(any("result queue full" in m for m in self.log.messages), self.log.messages)
        self.assertTrue(good_conn.is_open)
        self.call(lambda: good_conn.push_value("still", {"key": "still", "value": "alive"}))
        self.assertEqual(recv_json(good_ws)["data"], [{"key": "still", "value": "alive"}])

    def test_a_heartbeat_goes_out_before_the_results_queued_ahead_of_it(self):
        # The hub reads nothing for a while (4 KB receive window): the socket
        # fills, and many more 64 KB results wait in the queue. A heartbeat set
        # now waits only behind what is already in the write buffer, not
        # behind every queued result: a heartbeat held behind them reaches the
        # hub late and turns the instance "busy" while Live's main thread is
        # fine (#9).
        stalled, conn = self.stalled_client()
        # Twice as many 64 KB results as the kernel's largest send buffer
        # holds, and 100 more: at least half of them are still queued when the
        # heartbeat is set, whatever the runner's buffer sizes.
        frame = 65536
        count = 2 * send_buffer_frames(frame) + 100
        payload = "x" * frame

        def push():
            for n in range(count):
                conn.push_result(f"u{n}", [{"ok": True, "data": payload}])

        self.call(push)
        time.sleep(0.2)
        self.call(lambda: self.server.broadcast_heartbeat({"main_tick_age_ms": 1.0}))
        events = stalled.read_messages(count + 2)
        order = [e.get("uuid") or e["event"] for e in events]
        self.assertEqual(order[0], "connect")
        self.assertEqual(order.count("heartbeat"), 1)
        self.assertLess(order.index("heartbeat"), count // 2, f"late heartbeat: {order[:8]}…")
        self.assertEqual([u for u in order if u.startswith("u")], [f"u{n}" for n in range(count)])

    def test_a_client_that_reads_nothing_is_closed_once_its_socket_took_nothing_for_a_while(self):
        # The result queue alone would keep a stuck reader's output for ever
        # when it is values or a few large results; a socket that took no byte
        # for SEND_STALL_S while output waited closes the connection (the hub
        # resyncs), as the 3 s `sendall` timeout did before #5.
        self.addCleanup(setattr, transport, "SEND_STALL_S", transport.SEND_STALL_S)
        transport.SEND_STALL_S = 0.3
        good_ws, good_conn, _ = self.client()
        _stalled, conn = self.stalled_client()
        queued = self.fill(conn)
        # When exactly is pinned on explicit times (OneTickTest); here, that it
        # happens on the polled server and spares the other client.
        wait_for(lambda: conn.finished, timeout=3.0)
        self.assertLess(queued, 1000, "the queue bound closed it, not the stall")
        self.assertTrue(any("read nothing for" in m for m in self.log.messages), self.log.messages)
        wait_for(lambda: conn not in self.connections())
        self.assertTrue(good_conn.is_open)
        self.call(lambda: good_conn.push_value("still", {"key": "still", "value": "alive"}))
        self.assertEqual(recv_json(good_ws)["data"], [{"key": "still", "value": "alive"}])

    def test_shutdown_then_rebind_at_once_without_time_wait(self):
        ws, _conn, _ = self.client()

        def stop():
            self.server.broadcast("disconnect")
            started = time.monotonic()
            self.server.shutdown()
            return time.monotonic() - started

        self.assertLess(self.call(stop), 1.0)
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
        _stalled, conn = self.stalled_client()

        def stop():
            started = time.monotonic()
            self.server.shutdown()
            return time.monotonic() - started

        self.assertLess(self.call(stop), 1.0)
        self.assertTrue(conn.finished)
        self.assertEqual(server_side_time_wait(self.port), [])


class OneTickTest(unittest.TestCase):
    """The test thread is Live's main thread: the server moves only when it ticks."""

    def setUp(self):
        self.logger, self.log = recording_logger(self.id())
        self.server = Server("127.0.0.1", 0, {"instance": "test"}, 1000, self.logger)
        self.server.start()
        self.assertTrue(self.server.wait_bound(2.0))
        self.addCleanup(self.server.shutdown)
        self.port = self.server.port

    def client(self, rcvbuf=None, extra=b""):
        client = RawClient(self.port, rcvbuf, extra)
        self.addCleanup(client.close)
        return client

    def answer(self):
        """Play the drain: answer every request in the inbox, in order."""
        while not self.server.inbox.empty():
            conn, payload = self.server.inbox.get_nowait()
            if payload is not None:
                conn.push_result(payload["uuid"], [{"ok": True, "data": payload["uuid"]}])

    def tick(self):
        self.server.poll_in()
        self.answer()
        self.server.poll_out()

    def settle(self, client):
        """What a tick sent has reached the client (localhost: at once)."""
        client.readable(0.2)
        client.read_available()

    def test_nothing_is_read_or_written_between_ticks(self):
        client = self.client()
        self.assertFalse(client.readable(0.3), "the server answered without a tick")
        self.assertEqual(self.server.connections(), [])
        self.tick()
        self.settle(client)
        self.assertEqual([m["event"] for m in client.messages()], ["connect"])
        self.assertTrue(client.response.startswith(b"HTTP/1.1 101"), client.response)
        # A heartbeat set now waits for the tick's write.
        self.server.broadcast_heartbeat({"main_tick_age_ms": 1.0})
        self.assertFalse(client.readable(0.2), "a frame left without a tick")
        self.server.poll_out()
        self.settle(client)
        self.assertEqual([m["event"] for m in client.messages()], ["heartbeat"])

    def test_a_burst_of_requests_is_answered_within_two_ticks(self):
        burst = b"".join(request_frame(f"u{n}") for n in range(20))
        client = self.client(extra=burst)
        messages = []
        for _ in range(2):
            self.tick()
            self.settle(client)
            messages += client.messages()
            if len(messages) == 21:
                break
        self.assertEqual(messages[0]["event"], "connect")
        self.assertEqual([m.get("uuid") for m in messages[1:]], [f"u{n}" for n in range(20)])

    def test_connect_is_the_first_frame_even_with_a_heartbeat_in_the_same_tick(self):
        client = self.client()
        self.server.poll_in()
        self.server.broadcast_heartbeat({"main_tick_age_ms": 1.0})
        self.server.poll_out()
        self.assertEqual([m["event"] for m in client.read_messages(2)], ["connect", "heartbeat"])

    def test_a_partial_write_is_carried_to_the_next_tick(self):
        # A frame larger than the kernel's send buffer and the client's 4 KB
        # window: the socket fills and takes no more while the client does not
        # read; what it did not take waits in the write buffer for later ticks.
        client = self.client(rcvbuf=4096)
        self.tick()
        conn = self.server.connections()[0]
        big = "x" * (2 * tcp_wmem_max() + (1 << 20))
        conn.push_result("big", [{"ok": True, "data": big}])
        conn.push_result("after", [])
        # A write that waited for the client would never come back here (it
        # does not read); how long encoding the frame takes is the machine's.
        for _ in range(1000):
            unsent = conn.pending()[2]
            self.server.poll_out()
            if 0 < conn.pending()[2] == unsent:
                break
        self.assertGreater(conn.pending()[2], 0, "the socket took the whole frame")
        self.assertEqual(conn.pending()[2], unsent, "the socket still takes bytes")
        # The client reads (a larger buffer now); each tick writes on from
        # where the last one stopped.
        client.sock.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 1 << 20)
        messages = []
        for _ in range(5000):
            client.read_available()
            messages += client.messages()
            if len(messages) == 3:
                break
            self.server.poll_out()
            time.sleep(0.001)
        self.assertEqual(
            [m.get("uuid") or m["event"] for m in messages], ["connect", "big", "after"]
        )
        self.assertEqual(messages[1]["data"], [{"ok": True, "data": big}])
        self.assertEqual(conn.pending(), (0, 0, 0))

    def test_a_close_from_the_client_sends_what_is_queued_then_the_close_reply(self):
        client = self.client()
        self.tick()
        conn = self.server.connections()[0]
        conn.push_result("last", [])
        client.sock.sendall(masked_frame(CLOSE))
        self.tick()
        self.settle(client)
        messages = client.messages()
        self.assertEqual(
            [m.get("uuid") or m.get("event") or m["opcode"] for m in messages],
            ["connect", "last", CLOSE],
        )
        self.assertTrue(conn.finished)
        self.assertEqual(self.server.connections(), [])
        self.assertEqual(self.server.inbox.get_nowait(), (conn, None))

    def fill_socket(self, conn, now):
        """Push a frame larger than the kernel's buffers and write at ``now``
        until the socket takes no more (the client never reads)."""
        conn.push_result("big", [{"ok": True, "data": "x" * (2 * tcp_wmem_max() + (1 << 20))}])
        for _ in range(1000):
            unsent = conn.pending()[2]
            conn.flush(now)
            if 0 < conn.pending()[2] == unsent:
                return
        raise AssertionError("the socket never filled")

    def test_a_socket_that_takes_nothing_closes_its_connection_after_the_send_stall(self):
        # On explicit times: still open just before SEND_STALL_S without a
        # byte taken, closed at it (the hub resyncs), with the reason logged.
        self.client(rcvbuf=4096)
        self.tick()
        conn = self.server.connections()[0]
        t0 = time.monotonic()
        self.fill_socket(conn, t0)
        conn.flush(t0 + transport.SEND_STALL_S - 0.01)
        self.assertTrue(conn.is_open)
        conn.flush(t0 + transport.SEND_STALL_S)
        self.assertTrue(conn.finished)
        self.assertTrue(any("read nothing for" in m for m in self.log.messages), self.log.messages)
        self.server.poll_out()
        self.assertEqual(self.server.inbox.get_nowait(), (conn, None))

    def test_a_close_the_client_never_answers_ends_after_the_close_handshake_timeout(self):
        client = self.client()
        self.tick()
        conn = self.server.connections()[0]
        conn.close()
        t0 = time.monotonic()
        conn.flush(t0)
        self.settle(client)
        self.assertEqual(
            [m.get("event") or m["opcode"] for m in client.messages()], ["connect", CLOSE]
        )
        conn.flush(t0 + transport.CLOSE_HANDSHAKE_TIMEOUT_S - 0.01)
        self.assertFalse(conn.finished)
        conn.flush(t0 + transport.CLOSE_HANDSHAKE_TIMEOUT_S)
        self.assertTrue(conn.finished)

    def test_one_large_message_goes_out_in_one_tick_when_the_socket_takes_it(self):
        # A byte cap per tick would hold a large result (and every heartbeat
        # set behind it) for several ticks although the socket takes it at once.
        client = self.client()
        self.tick()
        conn = self.server.connections()[0]
        big = "x" * (1 << 20)
        conn.push_result("big", [{"ok": True, "data": big}])
        self.server.poll_out()
        self.assertEqual(conn.pending(), (0, 0, 0))
        uuids = [m.get("uuid") or m["event"] for m in client.read_messages(2)]
        self.assertEqual(uuids, ["connect", "big"])

    def test_writing_stops_at_the_tick_budget_and_goes_on_at_the_next_tick(self):
        self.addCleanup(setattr, transport, "IO_BUDGET_S", transport.IO_BUDGET_S)
        transport.IO_BUDGET_S = 0.0
        client = self.client()
        self.tick()
        conn = self.server.connections()[0]
        for n in range(3):
            conn.push_result(f"r{n}", [{"ok": True, "data": "x" * 100_000}])
        queued = []
        for _ in range(3):
            self.server.poll_out()
            queued.append(conn.pending()[0])
        self.assertEqual(queued, [2, 1, 0])
        uuids = [m.get("uuid") or m["event"] for m in client.read_messages(4)]
        self.assertEqual(uuids, ["connect", "r0", "r1", "r2"])

    def test_reading_stops_at_the_tick_budget_and_goes_on_at_the_next_tick(self):
        self.addCleanup(setattr, transport, "IO_BUDGET_S", transport.IO_BUDGET_S)
        transport.IO_BUDGET_S = 0.0
        pad = "y" * 40_000
        requests = b"".join(
            masked_frame(
                OPCODE_TEXT, json.dumps({"uuid": f"q{n}", "commands": [], "pad": pad}).encode()
            )
            for n in range(3)
        )
        client = self.client()
        self.tick()
        conn = self.server.connections()[0]
        client.sock.sendall(requests)
        # With no budget left each tick reads one chunk (RECV_SIZE, 64 KB),
        # which completes at most two of these 40 KB requests: the inbox never
        # jumps from none to all three in one tick, whatever has arrived.
        counts = [0]
        for _ in range(50):
            select.select([conn.socket], [], [], 1.0)
            self.server.poll_in()
            counts.append(self.server.inbox.qsize())
            if counts[-1] == 3:
                break
        self.assertEqual(counts[-1], 3, counts)
        self.assertLessEqual(max(b - a for a, b in itertools.pairwise(counts)), 2, counts)
        uuids = [self.server.inbox.get_nowait()[1]["uuid"] for _ in range(3)]
        self.assertEqual(uuids, ["q0", "q1", "q2"])

    def test_a_request_too_deep_for_json_gets_an_error_and_the_next_one_still_runs(self):
        deep = masked_frame(OPCODE_TEXT, b"[" * 200_000)
        client = self.client(extra=deep + request_frame("after"))
        messages = []
        for _ in range(10):
            self.tick()
            self.settle(client)
            messages += client.messages()
            if len(messages) >= 3:
                break
        self.assertEqual([m["event"] for m in messages], ["connect", "error", "result"])
        self.assertIn("invalid JSON", messages[1]["data"])
        self.assertEqual(messages[2]["uuid"], "after")
        self.assertTrue(self.server.connections()[0].is_open)

    def test_a_connection_that_fails_while_reading_is_closed_and_the_others_are_served(self):
        a, b = self.client(), self.client()
        self.tick()
        first, second = self.server.connections()

        def broken(_data):
            raise RuntimeError("test: a bug in one connection's read")

        first.take = broken
        a.sock.sendall(request_frame("a1"))
        b.sock.sendall(request_frame("b1"))
        for conn in (first, second):
            self.assertTrue(select.select([conn.socket], [], [], 2.0)[0])
        self.tick()
        self.assertTrue(first.finished)
        self.settle(b)
        self.assertIn("b1", [m.get("uuid") for m in b.messages()])
        self.assertTrue(
            any("reading failed, closing it" in m for m in self.log.messages), self.log.messages
        )
        self.assertEqual(self.server.connections(), [second])

    def test_a_connection_that_fails_while_writing_is_closed_and_the_others_are_served(self):
        _a, b = self.client(), self.client()
        self.tick()
        first, second = self.server.connections()

        def broken(_now):
            raise RuntimeError("test: a bug in one connection's write")

        first.flush = broken
        first.push_result("a1", [])
        second.push_result("b1", [])
        self.server.poll_out()
        self.assertTrue(first.finished)
        self.settle(b)
        self.assertIn("b1", [m.get("uuid") for m in b.messages()])
        self.assertTrue(
            any("writing failed, closing it" in m for m in self.log.messages), self.log.messages
        )
        self.assertEqual(self.server.connections(), [second])

    def test_a_client_reset_before_its_accept_keeps_the_listener(self):
        # Windows reports a client that reset before its accept as
        # WSAECONNRESET from accept(); closing the listener for it would
        # refuse every client until the port can be bound again.
        class ResetOnce:
            def __init__(self, sock):
                self.sock = sock
                self.reset = False

            def fileno(self):
                return self.sock.fileno()

            def accept(self):
                if not self.reset:
                    self.reset = True
                    raise ConnectionResetError("test: reset before accept")
                return self.sock.accept()

            def __getattr__(self, name):
                return getattr(self.sock, name)

        listener = ResetOnce(self.server._listener)
        self.server._listener = listener
        client = self.client()
        for _ in range(3):
            self.tick()
        self.assertIs(self.server._listener, listener)
        self.assertEqual(client.read_messages(1)[0]["event"], "connect")
        self.assertFalse(any("binding again" in m for m in self.log.messages), self.log.messages)

    def test_pending_handshakes_and_connections_are_capped(self):
        self.addCleanup(setattr, transport, "MAX_HANDSHAKES", transport.MAX_HANDSHAKES)
        self.addCleanup(setattr, transport, "MAX_CONNECTIONS", transport.MAX_CONNECTIONS)
        transport.MAX_HANDSHAKES = 2
        transport.MAX_CONNECTIONS = 3
        # Clients that never finish their request: two wait (the handshake
        # cap, under the connection cap), the rest are refused.
        silent = [socket.create_connection(("127.0.0.1", self.port)) for _ in range(4)]
        for sock in silent:
            self.addCleanup(sock.close)
        self.tick()
        closed = [sock for sock in silent if self.closed_by_server(sock)]
        self.assertEqual(len(closed), 2)
        self.assertTrue(any("refused" in m for m in self.log.messages), self.log.messages)
        for sock in silent:
            sock.close()
        self.tick()
        # The waiting ones left; one client connects, the next finds it taken.
        transport.MAX_CONNECTIONS = 1
        member = self.client()
        self.tick()
        self.assertEqual(member.read_messages(1)[0]["event"], "connect")
        refused = self.client()
        self.tick()
        refused.readable(1.0)
        refused.read_available()
        self.assertTrue(refused.closed)
        self.assertEqual(len(self.server.connections()), 1)

    @staticmethod
    def closed_by_server(sock):
        if not select.select([sock], [], [], 0.5)[0]:
            return False
        try:
            return sock.recv(1) == b""
        except ConnectionResetError:
            return True

    def test_clients_the_server_closes_leave_no_time_wait_on_the_port(self):
        # Windows keeps an SO_EXCLUSIVEADDRUSE port busy while connections
        # accepted on it are still closing: every accepted socket the server
        # closes itself is reset, as a finished connection is (#5 review).
        self.addCleanup(setattr, transport, "MAX_CONNECTIONS", transport.MAX_CONNECTIONS)
        self.addCleanup(setattr, transport, "HANDSHAKE_TIMEOUT_S", transport.HANDSHAKE_TIMEOUT_S)
        transport.HANDSHAKE_TIMEOUT_S = 0.2
        # A request that is not an upgrade: 400, then closed.
        bad = socket.create_connection(("127.0.0.1", self.port))
        bad.sendall(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
        self.tick()
        bad.settimeout(2.0)
        self.assertTrue(bad.recv(4096).startswith(b"HTTP/1.1 400"))
        # A request that never completes: dropped after the handshake timeout.
        slow = socket.create_connection(("127.0.0.1", self.port))
        slow.sendall(b"GET / HTTP/1.1\r\n")
        self.tick()
        time.sleep(0.3)
        self.tick()
        self.assertTrue(self.closed_by_server(slow))
        # A client past the cap: refused at accept.
        transport.MAX_CONNECTIONS = 1
        self.client()
        self.tick()
        refused = socket.create_connection(("127.0.0.1", self.port))
        self.tick()
        self.assertTrue(self.closed_by_server(refused))
        for sock in (bad, slow, refused):
            sock.close()
        time.sleep(0.1)
        self.assertEqual(server_side_time_wait(self.port), [])

    def test_pending_handshakes_count_toward_the_connection_cap(self):
        # Handshakes accepted under the cap complete later: counted at accept,
        # the connections never pass MAX_CONNECTIONS.
        self.addCleanup(setattr, transport, "MAX_CONNECTIONS", transport.MAX_CONNECTIONS)
        transport.MAX_CONNECTIONS = 1
        silent = [socket.create_connection(("127.0.0.1", self.port)) for _ in range(2)]
        for sock in silent:
            self.addCleanup(sock.close)
        self.tick()
        self.assertEqual(sum(self.closed_by_server(sock) for sock in silent), 1)

    def test_a_network_error_on_accept_keeps_the_listener(self):
        # accept(2): errors of the network or of the pending client are to be
        # treated like EAGAIN; only errors of the listener itself rebind it.
        listener = FakeListener(self.server._listener, [OSError(errno.EPROTO, "test: EPROTO")])
        self.server._listener = listener
        client = self.client()
        for _ in range(3):
            self.tick()
        self.assertIs(self.server._listener, listener)
        self.assertEqual(client.read_messages(1)[0]["event"], "connect")
        self.assertFalse(any("binding again" in m for m in self.log.messages), self.log.messages)

    def test_accept_failures_and_refusals_that_go_on_are_logged_once_a_while(self):
        # The listener stays readable while accept fails (EMFILE) and a client
        # may retry against the caps every tick: one warning, not ~30 a second.
        self.addCleanup(setattr, transport, "MAX_HANDSHAKES", transport.MAX_HANDSHAKES)
        transport.MAX_HANDSHAKES = 1
        exhausted = [OSError(errno.EMFILE, "test: too many open files")] * 5
        self.server._listener = FakeListener(self.server._listener, exhausted)
        first = socket.create_connection(("127.0.0.1", self.port))
        self.addCleanup(first.close)
        for _ in range(5):
            self.tick()
        failures = [m for m in self.log.messages if "accept failed" in m]
        self.assertEqual(len(failures), 1, self.log.messages)
        self.tick()  # the fake listener accepts again: `first` is the one handshake
        for _ in range(3):
            extra = socket.create_connection(("127.0.0.1", self.port))
            self.addCleanup(extra.close)
            self.tick()
            self.assertTrue(self.closed_by_server(extra))
        refusals = [m for m in self.log.messages if "refused" in m]
        self.assertEqual(len(refusals), 1, self.log.messages)

    def test_a_handshake_that_arrives_in_pieces_completes_on_a_later_tick(self):
        sock = socket.create_connection(("127.0.0.1", self.port))
        self.addCleanup(sock.close)
        request = upgrade_request(self.port)
        sock.sendall(request[:20])
        self.tick()
        self.assertFalse(select.select([sock], [], [], 0.1)[0])
        sock.sendall(request[20:])
        self.tick()
        sock.settimeout(2.0)
        self.assertTrue(sock.recv(4096).startswith(b"HTTP/1.1 101"))

    def test_a_request_that_is_not_an_upgrade_gets_400(self):
        sock = socket.create_connection(("127.0.0.1", self.port))
        self.addCleanup(sock.close)
        sock.sendall(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
        self.tick()
        sock.settimeout(2.0)
        self.assertTrue(sock.recv(4096).startswith(b"HTTP/1.1 400"))
        self.assertEqual(self.server.connections(), [])

    def test_a_handshake_that_never_completes_is_dropped(self):
        self.addCleanup(setattr, transport, "HANDSHAKE_TIMEOUT_S", transport.HANDSHAKE_TIMEOUT_S)
        transport.HANDSHAKE_TIMEOUT_S = 0.2
        sock = socket.create_connection(("127.0.0.1", self.port))
        self.addCleanup(sock.close)
        sock.sendall(b"GET / HTTP/1.1\r\n")
        self.tick()
        time.sleep(0.3)
        self.tick()
        sock.settimeout(2.0)
        try:
            data = sock.recv(1)
        except ConnectionResetError:
            data = b""
        self.assertEqual(data, b"")


if __name__ == "__main__":
    unittest.main()
