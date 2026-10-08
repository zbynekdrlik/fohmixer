"""End to end: ``sim/host.py`` runs the real script on SimLive; clients speak WebSocket."""

import base64
import itertools
import json
import os
import queue
import shutil
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest

import _paths
from _rawclient import CLOSE, RawClient, request_frame
from FohMixer.version import VERSION
from websockets.sync.client import connect

READY_TIMEOUT_S = 10.0


class Host:
    """A ``sim/host.py`` subprocess: FohMixer on SimLive with a single main thread."""

    def __init__(self, port=0, instance="band", meters_hz=0):
        self.instance = instance
        self.log_dir = tempfile.mkdtemp(prefix="fohmixer-host-test-")
        self.proc = subprocess.Popen(
            [
                sys.executable,
                _paths.HOST,
                "--port",
                str(port),
                "--instance",
                instance,
                "--site",
                _paths.FIXTURE,
                "--meters-hz",
                str(meters_hz),
                "--log-dir",
                self.log_dir,
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        self.stdout = queue.Queue()
        self.stderr = []
        self._pumps = [
            threading.Thread(target=self._pump_stdout, daemon=True),
            threading.Thread(target=self._pump_stderr, daemon=True),
        ]
        for pump in self._pumps:
            pump.start()
        try:
            line = self.stdout.get(timeout=READY_TIMEOUT_S)
        except queue.Empty:
            self.stop()
            raise AssertionError(f"host not ready; stderr: {''.join(self.stderr)}") from None
        assert line.startswith("READY "), line
        self.port = int(line.split()[1])

    def _pump_stdout(self):
        for line in self.proc.stdout:
            self.stdout.put(line)

    def _pump_stderr(self):
        for line in self.proc.stderr:
            self.stderr.append(line)

    def log_text(self):
        path = os.path.join(self.log_dir, f"fohmixer-{self.instance}.log")
        with open(path, encoding="utf-8") as f:
            return f.read()

    def control(self, line):
        self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()

    def request_stop(self):
        """SIGTERM and a bounded wait: the only way a host is stopped (spec I7)."""
        self.proc.send_signal(signal.SIGTERM)
        return self.proc.wait(10)

    def stop(self):
        try:
            if self.proc.poll() is None:
                self.request_stop()  # a host that ignores the request fails the test
        finally:
            for pump in self._pumps:
                pump.join(5)
            for pipe in (self.proc.stdin, self.proc.stdout, self.proc.stderr):
                pipe.close()
            shutil.rmtree(self.log_dir, ignore_errors=True)


class Client:
    """A WebSocket client that keeps every non-result frame it receives."""

    _uuids = itertools.count(1)

    def __init__(self, port):
        self.ws = connect(f"ws://127.0.0.1:{port}", open_timeout=3, close_timeout=1)
        self.frames = []
        self.hello = self.wait_event("connect")

    def close(self):
        self.ws.close()

    def recv(self, timeout):
        frame = json.loads(self.ws.recv(timeout=timeout))
        self.frames.append(frame)
        return frame

    def send(self, commands):
        uuid = f"u{next(Client._uuids)}"
        self.ws.send(json.dumps({"uuid": uuid, "commands": commands}))
        return uuid

    def request(self, commands, timeout=2.0):
        uuid = self.send(commands)
        return self.wait_event("result", timeout, lambda f: f.get("uuid") == uuid)["data"]

    def call(self, target, name, args=None, timeout=2.0):
        slot = self.request([{"target": target, "name": name, "args": args or {}}], timeout)[0]
        assert slot["ok"], slot
        return slot["data"]

    def wait_event(self, event, timeout=2.0, predicate=None):
        deadline = time.monotonic() + timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise AssertionError(f"no {event} event in {timeout} s")
            frame = self.recv(remaining)
            if frame["event"] == event and (predicate is None or predicate(frame)):
                return frame

    def wait_value(self, key, timeout=2.0):
        frame = self.wait_event(
            "values", timeout, lambda f: any(item["key"] == key for item in f["data"])
        )
        return next(item for item in frame["data"] if item["key"] == key)

    def read_until(self, condition, what, timeout):
        """Keep reading frames until ``condition()`` holds; it is checked at
        least every 0.1 s and may look at ``frames`` or at something else."""
        deadline = time.monotonic() + timeout
        while not condition():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise AssertionError(f"not {what} in {timeout} s")
            try:
                self.recv(min(remaining, 0.1))
            except TimeoutError:
                continue  # no frame in 0.1 s: check the condition again

    def saw_value(self, key, value, since=0):
        """Whether a ``values`` frame from ``frames[since:]`` set ``key`` to ``value``."""
        return any(
            item == {"key": key, "value": value}
            for frame in self.frames[since:]
            if frame["event"] == "values"
            for item in frame["data"]
        )


def masked_text_frame(text):
    payload = text.encode("utf-8")
    mask = os.urandom(4)
    masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    if len(payload) < 126:
        header = struct.pack("!BB", 0x81, 0x80 | len(payload))
    else:
        header = struct.pack("!BBH", 0x81, 0x80 | 126, len(payload))
    return header + mask + masked


def stalled_client(port, envelopes):
    """Handshake, send the request envelopes, then never read (a tiny receive buffer)."""
    sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_RCVBUF, 4096)
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
    for n, commands in enumerate(envelopes):
        sock.sendall(masked_text_frame(json.dumps({"uuid": f"stalled{n}", "commands": commands})))
    return sock


def free_port_pair():
    """Two consecutive free localhost ports (band on P, master on P+1)."""
    for _ in range(50):
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        if port >= 65535:
            continue
        try:
            with socket.socket() as a, socket.socket() as b:
                a.bind(("127.0.0.1", port))
                b.bind(("127.0.0.1", port + 1))
        except OSError:
            continue
        return port
    raise AssertionError("no free port pair")


class IntegrationTest(unittest.TestCase):
    def setUp(self):
        self.hosts = []
        self.clients = []

    def tearDown(self):
        for client in self.clients:
            client.close()
        for host in self.hosts:
            host.stop()
            self.assertNotIn("Traceback", "".join(host.stderr), "".join(host.stderr))

    def host(self, **kwargs):
        host = Host(**kwargs)
        self.hosts.append(host)
        return host

    def client(self, host):
        client = Client(host.port)
        self.clients.append(client)
        return client

    def answer(self, host, line):
        host.control(line)
        return host.stdout.get(timeout=5).strip()

    def test_host_control_lines_rename_tracks_and_count_listeners(self):
        host = self.host()
        a = self.client(host)
        volume = "live_set tracks[name=Hand1 #] mixer_device volume"
        self.assertEqual(self.answer(host, f"listeners value {volume}"), "LISTENERS 0")
        a.call(volume, "add_listener", {"prop": "value"})
        self.assertEqual(self.answer(host, f"listeners value {volume}"), "LISTENERS 1")
        name = a.call("live_set tracks 0", "add_listener", {"prop": "name"})
        self.assertEqual(self.answer(host, 'rename "Hand1 #" "Hand9 #"'), "RENAMED 1")
        self.assertEqual(a.wait_value(name["key"])["value"], "Hand9 #")
        self.assertEqual(self.answer(host, f"listeners value {volume}"), "LISTENERS -1")
        self.assertEqual(
            self.answer(host, "listeners value live_set tracks 0 mixer_device volume"),
            "LISTENERS 1",
        )
        self.assertEqual(self.answer(host, 'rename "Nobody" "Else"'), "RENAMED 0")
        for line in ('rename "unbalanced', "rename one", "bogus line", "stall 10"):
            host.control(line)
        self.assertEqual(self.answer(host, 'rename "Hand9 #" "Hand1 #"'), "RENAMED 1")
        deadline = time.monotonic() + 2
        while "".join(host.stderr).count("unknown control line") < 3:
            self.assertLess(time.monotonic(), deadline, "".join(host.stderr))
            time.sleep(0.02)

    def test_a_tuner_marker_is_added_renamed_and_removed_as_in_live(self):
        # #68: the hub follows a track's devices and its Tuner's name.
        host = self.host()
        a = self.client(host)
        devices = a.call("live_set tracks 0", "add_listener", {"prop": "devices"})
        before = len(a.call("live_set tracks 0", "get_prop", {"prop": "devices"}))
        self.assertEqual(self.answer(host, "tuner track 0 set '\"Vox 1\" +G:VOCALS'"), "TUNER 1")
        listed = a.wait_value(devices["key"])["value"]
        self.assertEqual(len(listed), before + 1)
        self.assertEqual(listed[-1]["name"], '"Vox 1" +G:VOCALS')
        tuner = f"live_set tracks 0 devices {before}"
        self.assertEqual(a.call(tuner, "get_prop", {"prop": "class_name"}), "Tuner")
        name = a.call(tuner, "add_listener", {"prop": "name"})
        self.assertEqual(self.answer(host, "tuner track 0 set '\"Vox 2\" +PIN'"), "TUNER 1")
        self.assertEqual(a.wait_value(name["key"])["value"], '"Vox 2" +PIN')
        self.assertEqual(self.answer(host, "tuner track 0 add '\"Vox 3\"'"), "TUNER 2")
        self.assertEqual(self.answer(host, "tuner track 0 remove"), "TUNER 1")
        self.assertEqual(self.answer(host, "tuner track 0 remove"), "TUNER 0")
        self.assertEqual(self.answer(host, "tuner return 0 set '\"Hall\" +G:FX'"), "TUNER 1")
        self.assertEqual(self.answer(host, "tuner track 999 set 'x'"), "TUNER -1")
        for line in (
            "tuner track x set 'y'",
            "tuner bus 0 set 'y'",
            "tuner track 0 set",
            "tuner track 0 remove 'y'",
            "tuner track 0 rename 'y'",
        ):
            host.control(line)
        deadline = time.monotonic() + 2
        while "".join(host.stderr).count("unknown control line") < 5:
            self.assertLess(time.monotonic(), deadline, "".join(host.stderr))
            time.sleep(0.02)

    def test_a_held_meter_stays_until_released(self):
        # The UI tests' clip light (#21): the animation leaves a held track.
        host = self.host(meters_hz=30)
        a = self.client(host)
        track = "live_set tracks[name=Hand1 #]"
        self.assertEqual(self.answer(host, 'meter "Hand1 #" 0.95'), "METER 1")
        for _ in range(3):
            time.sleep(0.1)
            self.assertEqual(a.call(track, "get_prop", {"prop": "output_meter_level"}), 0.95)
            self.assertEqual(a.call(track, "get_prop", {"prop": "output_meter_left"}), 0.95)
        self.assertEqual(self.answer(host, 'meter "Hand1 #" off'), "METER 1")
        deadline = time.monotonic() + 2
        while a.call(track, "get_prop", {"prop": "output_meter_level"}) == 0.95:
            self.assertLess(time.monotonic(), deadline, "the animation moves it again")
            time.sleep(0.05)
        self.assertEqual(self.answer(host, 'meter "Nobody" 0.5'), "METER 0")
        for line in ('meter "Hand1 #" 1.5', 'meter "Hand1 #" loud', 'meter "Hand1 #"', 'meter "x'):
            host.control(line)
        deadline = time.monotonic() + 2
        while "".join(host.stderr).count("unknown control line") < 4:
            self.assertLess(time.monotonic(), deadline, "".join(host.stderr))
            time.sleep(0.02)

    def test_connect_event(self):
        a = self.client(self.host())
        self.assertEqual(
            a.hello["data"],
            {
                "instance": "band",
                "set_name": "Test Site",
                "script_version": VERSION,
                "live_version": "12.2.5",
                "proto": 1,
            },
        )

    def test_batch_returns_one_slot_per_command_in_order(self):
        a = self.client(self.host())
        slots = a.request(
            [
                {"target": "live_set tracks 0", "name": "get_prop", "args": {"prop": "name"}},
                {
                    "target": "live_set tracks[name=Keys 1]",
                    "name": "get_prop",
                    "args": {"prop": "mute"},
                },
                {
                    "target": "live_set tracks[name=Hand2 #]",
                    "name": "set_prop",
                    "args": {"prop": "mute", "value": True},
                },
            ]
        )
        self.assertEqual(slots[0], {"ok": True, "data": "Hand1 #"})
        self.assertFalse(slots[1]["ok"])
        self.assertEqual(slots[1]["errorType"], "PathError")
        self.assertIn("ambiguous", slots[1]["error"])
        self.assertEqual(slots[2], {"ok": True, "data": None})
        self.assertIs(a.call("live_set tracks 1", "get_prop", {"prop": "mute"}), True)

    def test_malformed_requests_never_kill_the_connection(self):
        a = self.client(self.host())
        a.ws.send("not json")
        self.assertEqual(a.wait_event("error")["uuid"], None)
        a.ws.send(json.dumps({"uuid": "x"}))
        self.assertEqual(a.wait_event("error")["uuid"], "x")
        slots = a.request(
            [
                {"target": "live_set", "name": "no_such_function", "args": []},
                {"target": 42, "name": "get_prop", "args": {"prop": "name"}},
                {"target": "live_set", "name": "_sim_children", "args": []},
                "not a command",
            ]
        )
        self.assertEqual([s["ok"] for s in slots], [False, False, False, False])
        self.assertEqual(
            [s["errorType"] for s in slots], ["OpError", "PathError", "OpError", "OpError"]
        )
        self.assertEqual(a.call("live_set", "get_prop", {"prop": "is_playing"}), False)

    def test_add_listener_returns_initial_value_with_display(self):
        a = self.client(self.host())
        out = a.call(
            "live_set tracks[name=Hand2 #] mixer_device volume",
            "add_listener",
            {"prop": "value", "display": True},
        )
        self.assertEqual(out["value"], 0.8)
        self.assertEqual(out["display"], "-2.00 dB")
        self.assertTrue(out["key"].endswith(".value"))

    def test_push_after_a_set_from_another_client(self):
        host = self.host()
        a, b = self.client(host), self.client(host)
        sub = b.call("live_set tracks 0", "add_listener", {"prop": "mute"})
        self.assertIs(sub["value"], False)
        delays = []
        for value in (True, False, True, False, True):
            started = time.monotonic()
            a.send(
                [
                    {
                        "target": "live_set tracks 0",
                        "name": "set_prop",
                        "args": {"prop": "mute", "value": value},
                    }
                ]
            )
            item = b.wait_value(sub["key"])
            delays.append(time.monotonic() - started)
            self.assertEqual(item, {"key": sub["key"], "value": value})
        self.assertLess(sorted(delays)[2], 0.1, delays)
        self.assertLess(max(delays), 1.0, delays)

    def test_two_clients_each_get_the_push(self):
        host = self.host()
        a, b, c = self.client(host), self.client(host), self.client(host)
        target = "live_set tracks[name=Drums #] mixer_device volume"
        key_b = b.call(target, "add_listener", {"prop": "value", "display": True})["key"]
        key_c = c.call(target, "add_listener", {"prop": "value"})["key"]
        self.assertEqual(key_b, key_c)
        a.call(target, "set_prop", {"prop": "value", "value": 0.5})
        self.assertEqual(b.wait_value(key_b)["value"], 0.5)
        self.assertEqual(c.wait_value(key_c)["value"], 0.5)

    def test_deleted_track_pushes_gone(self):
        host = self.host()
        a, b = self.client(host), self.client(host)
        key = b.call("live_set tracks[name=Hand4 #]", "add_listener", {"prop": "mute"})["key"]
        a.call("live_set", "delete_track", [3])
        self.assertEqual(b.wait_value(key, timeout=2.0), {"key": key, "error": "gone"})
        self.assertEqual(len(a.call("live_set", "get_prop", {"prop": "tracks"})), 14)

    def test_a_client_that_stops_reading_blocks_nobody(self):
        """Client C subscribes 20 meters, asks for ~6 MB of results and never reads.

        That is more than loopback buffers take (~4 MB at most), so C's socket
        stops taking data; the main thread keeps C's output (it never waits on
        a socket, #5) until C's socket took nothing for 3 s, then drops C ("read
        nothing for" in the log) and skips what C still asked. A reads
        heartbeats from before C connects until after C is dropped, so they
        cover all of C's stuck time: Live's main thread must keep ticking
        throughout. A's round trips are measured once C's requests are out of
        the way (until then A's requests queue behind C's in the one inbox, by
        design): once C's last request ran, or once the script dropped C, on a
        machine where C's requests take longer than the 3 s stall. The test
        waits for that state and then for C's drop, not for a set time (#5).
        A main thread blocked on C would show seconds of heartbeat age and
        round trip (C is stuck for 3 s), so the worst of each must stay under
        1 s. How fast the typical ones are belongs to the machine (a p90 under
        50 ms failed on a loaded runner, #5 review), so it is not bounded here.
        """
        host = self.host(meters_hz=30)
        a = self.client(host)
        meters = [
            {"target": f"live_set tracks {i}", "name": "add_listener", "args": {"prop": prop}}
            for i in range(10)
            for prop in ("output_meter_left", "output_meter_right")
        ]
        read_eq = {
            "target": "live_set tracks 5 devices 0",
            "name": "get_prop",
            "args": {"prop": "parameters"},
        }
        # C's last request sets a value A watches: its push means all of C's work is done.
        done_key = a.call("live_set tracks 0", "add_listener", {"prop": "solo"})["key"]
        set_solo = {
            "target": "live_set tracks 0",
            "name": "set_prop",
            "args": {"prop": "solo", "value": True},
        }
        first_frame = len(a.frames)
        stalled = stalled_client(host.port, [meters] + [[read_eq] * 100] * 10 + [[set_solo]])
        self.addCleanup(stalled.close)

        def dropped():
            return "read nothing for" in host.log_text()

        a.read_until(
            lambda: a.saw_value(done_key, True, first_frame) or dropped(),
            "C's last request run nor C dropped",
            timeout=15.0,
        )
        # A asks until the script has dropped C, and once more after.
        round_trips = []
        give_up = time.monotonic() + 10.0
        while True:
            gone = dropped()
            started = time.monotonic()
            a.call("live_set", "get_prop", {"prop": "is_playing"})
            round_trips.append(time.monotonic() - started)
            if gone:
                break
            self.assertLess(time.monotonic(), give_up, "C was never dropped: it never blocked")
        a.wait_event("heartbeat")
        ages = [
            f["data"]["main_tick_age_ms"]
            for f in a.frames[first_frame:]
            if f["event"] == "heartbeat"
        ]
        detail = f"round trips ms: {[round(r * 1000, 1) for r in round_trips]}; ages ms: {ages}"
        self.assertGreater(len(ages), 25, detail)
        self.assertLess(max(round_trips), 1.0, detail)
        self.assertLess(max(ages), 1000, detail)
        self.assertEqual(a.call("live_set", "get_prop", {"prop": "is_playing"}), False)

    def test_a_stall_stops_the_heartbeats_and_the_next_one_reports_it(self):
        """A 700 ms stall of the main thread. The heartbeat is made in the
        main thread's tick (#5), so none goes out while the stall lasts (the
        hub marks the instance busy once one is 300 ms overdue), and the first
        one after it reports the stall: its tick age is the time the main
        thread was away, its gap the time since the heartbeat before the stall.
        """
        host = self.host()
        a = self.client(host)
        a.wait_event("heartbeat")
        host.control("stall 700")
        stall_seen = a.wait_event("heartbeat", 2.0, lambda f: f["data"]["main_tick_age_ms"] >= 400)
        self.assertLess(stall_seen["data"]["main_tick_age_ms"], 1500, stall_seen)
        self.assertGreaterEqual(stall_seen["data"]["gap_ms"], 600, stall_seen)
        self.assertLess(stall_seen["data"]["gap_ms"], 1500, stall_seen)
        self.assertIn("main thread stalled", host.log_text())
        self.assertEqual(a.call("live_set", "get_prop", {"prop": "is_playing"}), False)

    def test_sigterm_disconnects_and_a_new_host_rebinds_the_port(self):
        host = self.host()
        port = host.port
        # The raw client, which keeps what the host sent before resetting the
        # socket: `websockets`' client dropped the goodbye when a heartbeat held
        # it past the script's 0.3 s close grace (#5).
        a = RawClient(port)
        self.addCleanup(a.close)
        add = {"target": "live_set tracks 0", "name": "add_listener", "args": {"prop": "mute"}}
        a.sock.sendall(request_frame("sub", [add]))
        deadline = time.monotonic() + 2.0
        messages = []
        while "sub" not in [m.get("uuid") for m in messages]:
            messages += a.read_messages(1, deadline - time.monotonic())
        self.assertTrue(a.response.startswith(b"HTTP/1.1 101"), a.response)
        self.assertEqual(messages[0].get("event"), "connect", messages)
        result = next(m for m in messages if m.get("uuid") == "sub")
        self.assertTrue(result["data"][0]["ok"], result)
        self.assertEqual(host.request_stop(), 0)
        events = [m.get("event") or m["opcode"] for m in a.read_until_closed()]
        self.assertIn("disconnect", events)
        goodbye = events.index("disconnect")
        # No close frame before it (websockets drops what follows one), and
        # nothing but the close frame after it, then the end (websockets'
        # ConnectionClosed on the next receive).
        self.assertNotIn(CLOSE, events[:goodbye], events)
        self.assertIn(events[goodbye + 1 :], ([], [CLOSE]), events)
        started = time.monotonic()
        again = self.host(port=port)
        self.assertLess(time.monotonic() - started, 5.0)
        self.assertEqual(again.port, port)
        b = self.client(again)
        out = b.call("live_set tracks 0", "add_listener", {"prop": "mute"})
        self.assertIs(out["value"], False)

    def test_two_instances_on_two_ports(self):
        port = free_port_pair()
        band = self.host(port=port, instance="band")
        master = self.host(port=port + 1, instance="master")
        a = self.client(band)
        b = self.client(master)
        self.assertEqual(a.hello["data"]["instance"], "band")
        self.assertEqual(b.hello["data"]["instance"], "master")
        a.call("live_set tracks 0", "set_prop", {"prop": "mute", "value": True})
        self.assertIs(a.call("live_set tracks 0", "get_prop", {"prop": "mute"}), True)
        self.assertIs(b.call("live_set tracks 0", "get_prop", {"prop": "mute"}), False)


if __name__ == "__main__":
    unittest.main()
