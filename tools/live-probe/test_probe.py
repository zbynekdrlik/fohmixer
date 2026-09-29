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


class Summary(unittest.TestCase):
    def test_a_whole_run(self):
        heartbeats = [
            # (arrival s, main_tick_age_ms, max_cmd_ms, gap_ms)
            (10.0, 4.0, 1.5, 100.2),
            (10.1, 8.0, 1.5, 99.8),
            (10.2, 160.0, 1.5, 100.0),
            (10.45, 2.0, 2.25, 250.0),
            (10.55, 6.0, 2.25, 100.1),
        ]
        round_trips = [3.0, 5.0, 210.0, 4.0]
        connect = {"instance": "band", "script_version": "9.9.9", "set_name": "S", "proto": 1}
        summary = probe.summarize(
            heartbeats, round_trips, lost=1, errors=0, connect=connect, seconds=0.6, probe_ms=150
        )
        self.assertEqual(
            summary,
            {
                "instance": "band",
                "script_version": "9.9.9",
                "seconds": 0.6,
                "probe_ms": 150,
                "heartbeats": 5,
                "main_tick_age_ms": {
                    "count": 5,
                    "min": 2.0,
                    "p50": 6.0,
                    "p95": 160.0,
                    "p99": 160.0,
                    "max": 160.0,
                    "mean": 36.0,
                    "distinct": 5,
                },
                "main_tick_age_over_150_ms": 1,
                "main_tick_age_over_200_ms": 0,
                "timer_estimate": {"interval_ms": 72.0, "jitter_ms": 88.0},
                "heartbeat_gap_ms": {"max": 250.0, "over_150_ms": 1},
                "arrival_gap_ms": {"max": 250.0, "over_150_ms": 1},
                "max_cmd_ms": {"start": 1.5, "end": 2.25},
                "round_trip_ms": {
                    "count": 4,
                    "min": 3.0,
                    "p50": 4.0,
                    "p95": 210.0,
                    "p99": 210.0,
                    "max": 210.0,
                    "mean": 55.5,
                    "distinct": 4,
                },
                "round_trip_over_150_ms": 1,
                "round_trip_over_200_ms": 1,
                "round_trips_lost": 1,
                "round_trip_errors": 0,
            },
        )

    def test_no_heartbeats(self):
        summary = probe.summarize([], [], lost=0, errors=0, connect={}, seconds=1, probe_ms=100)
        self.assertEqual(summary["heartbeats"], 0)
        self.assertEqual(summary["max_cmd_ms"], {"start": None, "end": None})
        self.assertEqual(summary["arrival_gap_ms"], {"max": None, "over_150_ms": 0})
        self.assertEqual(summary["heartbeat_gap_ms"], {"max": None, "over_150_ms": 0})
        self.assertIsNone(summary["instance"])


class Framing(unittest.TestCase):
    """The probe's frames against the script's own (vendored) framing."""

    def test_accept_key_of_rfc_6455(self):
        self.assertEqual(
            probe.accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzo+pOm+C0o="
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
        self.assertEqual(sorted(samples), ["heartbeats", "round_trip_ms"])
        self.assertGreaterEqual(len(samples["heartbeats"]), 2)
        self.assertEqual(
            sorted(samples["heartbeats"][0]),
            ["arrival_s", "gap_ms", "main_tick_age_ms", "max_cmd_ms"],
        )
        self.assertGreaterEqual(len(samples["round_trip_ms"]), 5)


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


if __name__ == "__main__":
    unittest.main()
