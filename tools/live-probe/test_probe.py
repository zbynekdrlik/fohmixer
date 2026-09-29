"""The live probe (#5, K1/K2): its summary math on known inputs, its WebSocket
framing against the script's vendored framing, and whole runs against
``sim/host.py`` running the real FohMixer script on SimLive.
"""

import json
import os
import queue
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
HOST = os.path.join(REPO, "sim", "host.py")
SITE = os.path.join(REPO, "sim", "fixtures", "test-site.json")
PROBE = os.path.join(HERE, "probe.py")
for _dir in (HERE, os.path.join(REPO, "live-script")):
    if _dir not in sys.path:
        sys.path.insert(0, _dir)

import probe  # noqa: E402
from FohMixer.transport import websocket as vendored  # noqa: E402
from FohMixer.version import VERSION  # noqa: E402

READY_S = 10.0
STOP_S = 10.0


class Percentiles(unittest.TestCase):
    def test_nearest_rank_on_one_to_a_hundred(self):
        values = list(range(100, 0, -1))
        self.assertEqual(probe.percentile(values, 0.50), 50)
        self.assertEqual(probe.percentile(values, 0.95), 95)
        self.assertEqual(probe.percentile(values, 0.99), 99)
        self.assertEqual(probe.percentile(values, 1.0), 100)

    def test_nearest_rank_on_ten_values(self):
        values = [7, 1, 3, 9, 5, 2, 10, 4, 8, 6]
        self.assertEqual(probe.percentile(values, 0.50), 5)
        self.assertEqual(probe.percentile(values, 0.95), 10)
        self.assertEqual(probe.percentile(values, 0.99), 10)
        self.assertEqual(probe.percentile(values, 0.0), 1)

    def test_no_values_have_no_percentile(self):
        self.assertIsNone(probe.percentile([], 0.5))


class Distribution(unittest.TestCase):
    def test_known_values(self):
        self.assertEqual(
            probe.distribution([4.0, 1.0, 2.0, 2.0, 11.0]),
            {
                "count": 5,
                "min": 1.0,
                "p50": 2.0,
                "p95": 11.0,
                "p99": 11.0,
                "max": 11.0,
                "mean": 4.0,
                "distinct": 4,
            },
        )

    def test_empty(self):
        self.assertEqual(
            probe.distribution([]),
            {
                "count": 0,
                "min": None,
                "p50": None,
                "p95": None,
                "p99": None,
                "max": None,
                "mean": None,
                "distinct": 0,
            },
        )

    def test_rounds_to_microseconds(self):
        self.assertEqual(probe.distribution([1.0 / 3.0])["mean"], 0.333)

    def test_over_counts_strictly_greater(self):
        # The hub's busy check is `age > 150`; the script logs a stall `> 200`.
        values = [150.0, 150.1, 200.0, 200.1, 12.0]
        self.assertEqual(probe.over(values, 150.0), 3)
        self.assertEqual(probe.over(values, 200.0), 1)
        self.assertEqual(probe.over([], 150.0), 0)


class TimerEstimate(unittest.TestCase):
    def test_a_sawtooth_sampled_evenly_gives_its_period(self):
        # Ages of a 10 ms tick sampled at every whole millisecond: mean 4.5.
        ages = [float(a) for a in range(10)] * 30
        self.assertEqual(probe.timer_estimate(ages), {"interval_ms": 9.0, "jitter_ms": 0.0})

    def test_a_late_tail_is_jitter(self):
        # One outlier in 100 moves the mean, not the p99: 99 samples 0..9
        # (sum 441) and one at 49.5: mean 4.905, p99 = 9, no jitter.
        ages = [float(a % 10) for a in range(99)] + [49.5]
        self.assertEqual(probe.timer_estimate(ages), {"interval_ms": 9.81, "jitter_ms": 0.0})
        # 90 of 0..9 and 10 at 30: mean 7.05, p99 = 30.
        ages = [float(a % 10) for a in range(90)] + [30.0] * 10
        self.assertEqual(probe.timer_estimate(ages), {"interval_ms": 14.1, "jitter_ms": 15.9})

    def test_no_samples(self):
        self.assertEqual(probe.timer_estimate([]), {"interval_ms": None, "jitter_ms": None})


def dist(count, low, p50, p95, p99, high, mean, distinct):
    return {
        "count": count,
        "min": low,
        "p50": p50,
        "p95": p95,
        "p99": p99,
        "max": high,
        "mean": mean,
        "distinct": distinct,
    }


class Summary(unittest.TestCase):
    def test_a_whole_run(self):
        heartbeats = [
            # (arrival s, main_tick_age_ms, max_cmd_ms, gap_ms, outbound_ms)
            (10.0, 4.0, 1.5, 100.2, 0.5),
            (10.1, 8.0, 1.5, 99.8, 0.5),
            (10.2, 160.0, 1.5, 100.0, 1.0),
            (10.45, 2.0, 2.25, 250.0, 40.0),
            (10.55, 6.0, 2.25, 100.1, 2.0),
        ]
        reads = [
            # (sent s, round_trip_ms, inbound_ms, outbound_ms, script_ts_ms), in
            # arrival order; the third waited out a stall and came back in the
            # same tick as the fourth.
            (0.0, 3.0, 2.0, 1.0, 1002.0),
            (0.15, 5.0, 3.0, 2.0, 1154.0),
            (0.30, 210.0, 200.0, 10.0, 1500.0),
            (0.45, 4.0, 3.0, 1.0, 1500.0),
        ]
        connect = {"instance": "band", "script_version": "9.9.9", "set_name": "S", "proto": 1}
        summary = probe.summarize(
            heartbeats, reads, lost=1, errors=0, connect=connect, seconds=0.6, probe_ms=150
        )
        self.assertEqual(
            summary,
            {
                "instance": "band",
                "script_version": "9.9.9",
                "seconds": 0.6,
                "probe_ms": 150,
                "heartbeats": 5,
                "main_tick_age_ms": dist(5, 2.0, 6.0, 160.0, 160.0, 160.0, 36.0, 5),
                "main_tick_age_over_150_ms": 1,
                "main_tick_age_over_200_ms": 0,
                "timer_estimate": {"interval_ms": 72.0, "jitter_ms": 88.0},
                "heartbeat_gap_ms": {"max": 250.0, "over_150_ms": 1},
                "arrival_gap_ms": {"max": 250.0, "over_150_ms": 1},
                "heartbeat_outbound_ms": dist(5, 0.5, 1.0, 40.0, 40.0, 40.0, 8.8, 4),
                "max_cmd_ms": {"start": 1.5, "end": 2.25},
                "round_trip_ms": dist(4, 3.0, 4.0, 210.0, 210.0, 210.0, 55.5, 4),
                "round_trip_over_150_ms": 1,
                "round_trip_over_200_ms": 1,
                "read_inbound_ms": dist(4, 2.0, 3.0, 200.0, 200.0, 200.0, 52.0, 3),
                "read_outbound_ms": dist(4, 1.0, 1.0, 10.0, 10.0, 10.0, 3.5, 3),
                # Distinct result timestamps 1002, 1154, 1500: gaps 152 and 346.
                "result_tick_gap_ms": dist(2, 152.0, 152.0, 346.0, 346.0, 346.0, 249.0, 2),
                "round_trips_lost": 1,
                "round_trip_errors": 0,
            },
        )

    def test_no_samples(self):
        summary = probe.summarize([], [], lost=0, errors=0, connect={}, seconds=1, probe_ms=100)
        self.assertEqual(summary["heartbeats"], 0)
        self.assertEqual(summary["max_cmd_ms"], {"start": None, "end": None})
        self.assertEqual(summary["arrival_gap_ms"], {"max": None, "over_150_ms": 0})
        self.assertEqual(summary["heartbeat_gap_ms"], {"max": None, "over_150_ms": 0})
        self.assertEqual(summary["result_tick_gap_ms"]["count"], 0)
        self.assertEqual(summary["read_inbound_ms"]["count"], 0)
        self.assertIsNone(summary["instance"])


class Framing(unittest.TestCase):
    """The probe's frames against the script's own (vendored) framing."""

    def test_accept_key_of_rfc_6455(self):
        self.assertEqual(
            probe.accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        )

    def test_the_script_reads_the_probes_masked_frames(self):
        for size in (0, 5, 125, 126, 65535, 65536):
            payload = bytes(i % 251 for i in range(size))
            frame = probe.client_frame(probe.OPCODE_TEXT, payload)
            self.assertTrue(frame[1] & 0x80, "a client frame is masked")
            buffer = bytearray(frame)
            opcode, fin, read = vendored.try_read_frame(buffer)
            self.assertEqual((opcode, fin, bytes(read)), (probe.OPCODE_TEXT, True, payload))
            self.assertEqual(buffer, bytearray())

    def test_the_probe_reads_the_scripts_frames(self):
        for size in (0, 5, 125, 126, 65535, 65536):
            payload = bytes(i % 251 for i in range(size))
            buffer = bytearray(vendored.encode_text_frame(payload) + b"\x81")
            self.assertEqual(probe.read_frame(buffer), (probe.OPCODE_TEXT, True, payload))
            self.assertEqual(buffer, bytearray(b"\x81"), "the next frame's first byte stays")

    def test_an_incomplete_frame_waits_for_more(self):
        frame = vendored.encode_text_frame(b"x" * 300)
        for cut in (0, 1, 3, 100, len(frame) - 1):
            buffer = bytearray(frame[:cut])
            self.assertIsNone(probe.read_frame(buffer))
            self.assertEqual(len(buffer), cut)


class Host:
    """``sim/host.py`` on a free port: the real FohMixer script on SimLive."""

    def __init__(self):
        self.log_dir = tempfile.mkdtemp(prefix="fohmixer-probe-test-")
        self.proc = subprocess.Popen(
            [
                sys.executable,
                HOST,
                "--port",
                "0",
                "--site",
                SITE,
                "--meters-hz",
                "30",
                "--log-dir",
                self.log_dir,
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        self._lines = queue.Queue()
        self._stderr = []
        self._pumps = [
            threading.Thread(target=self._pump, args=(self.proc.stdout, self._lines.put)),
            threading.Thread(target=self._pump, args=(self.proc.stderr, self._stderr.append)),
        ]
        for pump in self._pumps:
            pump.daemon = True
            pump.start()
        try:
            line = self._lines.get(timeout=READY_S)
        except queue.Empty:
            self.stop()
            raise AssertionError(f"host not ready: {''.join(self._stderr)}") from None
        assert line.startswith("READY "), line
        self.port = int(line.split()[1])

    @staticmethod
    def _pump(stream, sink):
        for line in stream:
            sink(line)

    def control(self, line):
        self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()

    def stop(self):
        """SIGTERM and a bounded wait (spec I7)."""
        try:
            if self.proc.poll() is None:
                self.proc.send_signal(signal.SIGTERM)
                self.proc.wait(STOP_S)
        finally:
            for pump in self._pumps:
                pump.join(5)
            for pipe in (self.proc.stdin, self.proc.stdout, self.proc.stderr):
                pipe.close()
            shutil.rmtree(self.log_dir, ignore_errors=True)


class AgainstSimLive(unittest.TestCase):
    def setUp(self):
        self.host = Host()
        self.addCleanup(self.host.stop)

    def test_a_run_records_heartbeats_and_round_trips(self):
        summary = probe.run(self.host.port, seconds=1.5, probe_ms=50)
        self.assertEqual(summary["instance"], "band")
        self.assertEqual(summary["script_version"], VERSION)
        # A heartbeat every 100 ms, a read every 50 ms, for 1.5 s.
        self.assertGreaterEqual(summary["heartbeats"], 10, summary)
        self.assertEqual(summary["main_tick_age_ms"]["count"], summary["heartbeats"])
        self.assertGreaterEqual(summary["main_tick_age_ms"]["min"], 0.0)
        self.assertLess(summary["main_tick_age_ms"]["max"], 150.0, summary)
        self.assertGreaterEqual(summary["round_trip_ms"]["count"], 20, summary)
        self.assertGreater(summary["round_trip_ms"]["min"], 0.0)
        self.assertEqual(summary["round_trips_lost"], 0)
        self.assertEqual(summary["round_trip_errors"], 0)
        self.assertGreaterEqual(summary["max_cmd_ms"]["end"], summary["max_cmd_ms"]["start"])
        self.assertGreater(summary["heartbeat_gap_ms"]["max"], 50.0)
        self.assertEqual(summary["read_inbound_ms"]["count"], summary["round_trip_ms"]["count"])
        self.assertEqual(summary["heartbeat_outbound_ms"]["count"], summary["heartbeats"])
        # The script's `ts` is whole milliseconds: a split may be half a
        # millisecond under zero.
        self.assertGreater(summary["read_inbound_ms"]["min"], -1.0, summary)
        self.assertGreater(summary["read_outbound_ms"]["min"], -1.0, summary)
        self.assertGreater(summary["heartbeat_outbound_ms"]["min"], -1.0, summary)

    def test_inbound_plus_outbound_is_the_round_trip(self):
        raw = os.path.join(self.host.log_dir, "split.json")
        probe.run(self.host.port, seconds=1.0, probe_ms=50, raw_path=raw)
        with open(raw, encoding="utf-8") as f:
            reads = json.load(f)["reads"]
        self.assertGreaterEqual(len(reads), 15)
        for read in reads:
            # Two clocks (the wall clock for the split, a performance counter
            # for the round trip) and the script's whole-millisecond `ts`.
            self.assertAlmostEqual(
                read["inbound_ms"] + read["outbound_ms"], read["round_trip_ms"], delta=2.0
            )

    def test_reads_faster_than_the_ticks_show_the_tick_interval(self):
        # SimLive runs the script's timer every TIMER_INTERVAL_MS (10 ms); a
        # read every 2 ms lands in nearly every tick.
        summary = probe.run(self.host.port, seconds=1.0, probe_ms=2)
        gaps = summary["result_tick_gap_ms"]
        self.assertGreaterEqual(gaps["count"], 30, summary)
        self.assertGreaterEqual(gaps["p50"], 5.0, summary)
        self.assertLessEqual(gaps["p50"], 30.0, summary)

    def test_a_main_thread_stall_shows_in_the_ages_and_the_round_trips(self):
        stall = threading.Timer(0.5, self.host.control, args=("stall 400",))
        stall.start()
        self.addCleanup(stall.cancel)
        summary = probe.run(self.host.port, seconds=1.5, probe_ms=50)
        self.assertGreaterEqual(summary["main_tick_age_over_150_ms"], 1, summary)
        self.assertGreaterEqual(summary["main_tick_age_ms"]["max"], 250.0, summary)
        self.assertLess(summary["main_tick_age_ms"]["max"], 1000.0, summary)
        # A read sent as the stall starts waits for it to end.
        self.assertGreaterEqual(summary["round_trip_over_150_ms"], 1, summary)
        self.assertGreaterEqual(summary["round_trip_ms"]["max"], 250.0, summary)
        self.assertEqual(summary["round_trips_lost"], 0)
        # The wait is on the way in (the drain waits for the main thread), not
        # on the way out (the sender thread kept running).
        self.assertGreaterEqual(summary["read_inbound_ms"]["max"], 250.0, summary)
        self.assertLess(summary["read_outbound_ms"]["max"], 150.0, summary)

    def test_the_cli_prints_the_summary_as_json(self):
        done = subprocess.run(
            [sys.executable, PROBE, "--port", str(self.host.port), "--seconds", "1"],
            capture_output=True,
            text=True,
            timeout=20,
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        summary = json.loads(done.stdout)
        self.assertEqual(summary["instance"], "band")
        self.assertEqual(summary["probe_ms"], 100)
        self.assertGreaterEqual(summary["heartbeats"], 5, summary)
        self.assertGreaterEqual(summary["round_trip_ms"]["count"], 5, summary)

    def test_the_raw_samples_go_to_a_file(self):
        raw = os.path.join(self.host.log_dir, "raw.json")
        probe.run(self.host.port, seconds=0.5, probe_ms=50, raw_path=raw)
        with open(raw, encoding="utf-8") as f:
            samples = json.load(f)
        self.assertEqual(sorted(samples), ["heartbeats", "reads"])
        self.assertGreaterEqual(len(samples["heartbeats"]), 2)
        self.assertEqual(
            sorted(samples["heartbeats"][0]),
            ["arrival_s", "gap_ms", "main_tick_age_ms", "max_cmd_ms", "outbound_ms"],
        )
        self.assertGreaterEqual(len(samples["reads"]), 5)
        self.assertEqual(
            sorted(samples["reads"][0]),
            ["inbound_ms", "outbound_ms", "round_trip_ms", "script_ts_ms", "sent_s"],
        )


class Failures(unittest.TestCase):
    def test_a_closed_port_fails_loudly(self):
        with socket.socket() as holder:
            holder.bind(("127.0.0.1", 0))
            port = holder.getsockname()[1]
        done = subprocess.run(
            [sys.executable, PROBE, "--port", str(port), "--seconds", "1"],
            capture_output=True,
            text=True,
            timeout=20,
        )
        self.assertEqual(done.returncode, 1)
        self.assertEqual(done.stdout, "")
        self.assertIn("live-probe: cannot connect", done.stderr)

    def test_a_server_that_is_not_a_websocket_is_refused(self):
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen(1)
        self.addCleanup(listener.close)

        def answer():
            conn, _ = listener.accept()
            with conn:
                conn.recv(4096)
                conn.sendall(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")

        server = threading.Thread(target=answer, daemon=True)
        server.start()
        with self.assertRaisesRegex(probe.ProbeError, "no WebSocket upgrade"):
            probe.run(listener.getsockname()[1], seconds=0.2, probe_ms=50)
        server.join(5)

    def test_the_script_going_away_ends_the_run_with_an_error(self):
        host = Host()
        self.addCleanup(host.stop)
        stop = threading.Timer(0.3, host.proc.send_signal, args=(signal.SIGTERM,))
        stop.start()
        self.addCleanup(stop.cancel)
        with self.assertRaisesRegex(probe.ProbeError, "the script closed the connection"):
            probe.run(host.port, seconds=5, probe_ms=50)


def text_frame(message):
    return vendored.encode_text_frame(json.dumps(message).encode("utf-8"))


class FakeScript:
    """A one-connection server speaking the script's protocol from a list of steps."""

    def __init__(self, first_frames):
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.listener.listen(1)
        self.listener.settimeout(5)
        self.port = self.listener.getsockname()[1]
        self._first = first_frames
        self.thread = threading.Thread(target=self._serve, daemon=True)
        self.thread.start()

    def _serve(self):
        conn, _ = self.listener.accept()
        with conn:
            conn.settimeout(5)
            _method, _path, headers, _rest = vendored.read_http_request(conn)
            vendored.complete_websocket_handshake(conn, headers)
            for message in self._first:
                conn.sendall(text_frame(message))
            buffer = bytearray()
            while True:
                frame = vendored.try_read_frame(buffer)
                if frame is None:
                    data = conn.recv(65536)
                    if not data:
                        return
                    buffer.extend(data)
                    continue
                opcode, _fin, payload = frame
                if opcode == vendored.OPCODE_CLOSE:
                    conn.sendall(vendored.encode_close_frame())
                    return
                uuid = json.loads(bytes(payload))["uuid"]
                now = round(time.time() * 1000)
                result = [{"ok": True, "data": 120.0}]
                conn.sendall(
                    text_frame({"event": "result", "uuid": uuid, "data": result, "ts": now})
                )
                conn.sendall(text_frame({"event": "heartbeat", "data": BEAT, "ts": now}))

    def close(self):
        self.thread.join(5)
        self.listener.close()


BEAT = {"main_tick_age_ms": 31.0, "max_cmd_ms": 1.0, "gap_ms": 100.0}
CONNECT = {"instance": "band", "script_version": "x", "set_name": "S", "proto": 1}


class ConnectFirst(unittest.TestCase):
    def test_a_heartbeat_before_connect_is_skipped(self):
        """Seen on the PC: the script lists a new connection for heartbeats
        before its sender has sent the queued connect, and the pending
        heartbeat goes first. The probe waits for connect and counts from it."""
        script = FakeScript(
            [
                {"event": "heartbeat", "data": BEAT, "ts": 1000},
                {"event": "connect", "data": CONNECT, "ts": 1001},
            ]
        )
        self.addCleanup(script.close)
        summary = probe.run(script.port, seconds=0.3, probe_ms=1000)
        self.assertEqual(summary["instance"], "band")
        self.assertEqual(summary["heartbeats"], 1, summary)
        self.assertEqual(summary["round_trip_ms"]["count"], 1, summary)
        self.assertEqual(summary["round_trips_lost"], 0)

    def test_anything_else_before_connect_is_refused(self):
        script = FakeScript([{"event": "values", "data": [], "ts": 1000}])
        self.addCleanup(script.close)
        with self.assertRaisesRegex(probe.ProbeError, "first message is not connect"):
            probe.run(script.port, seconds=0.3, probe_ms=1000)

    def test_a_message_that_is_not_an_object_is_refused(self):
        script = FakeScript([["connect"]])
        self.addCleanup(script.close)
        with self.assertRaisesRegex(probe.ProbeError, "not a JSON object"):
            probe.run(script.port, seconds=0.3, probe_ms=1000)


if __name__ == "__main__":
    unittest.main()
