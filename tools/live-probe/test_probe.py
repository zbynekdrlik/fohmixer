"""The live probe (#5, K1/K2): its summary math on known inputs, its WebSocket
framing against the script's vendored framing, and whole runs against
``sim/host.py`` running the real FohMixer script on SimLive.
"""

import itertools
import json
import os
import platform
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
            heartbeats,
            reads,
            lost=1,
            errors=0,
            connect=connect,
            seconds=0.6,
            probe_ms=150,
            skipped=2,
            wall_step_ms=0.5,
        )
        self.assertEqual(
            summary,
            {
                "instance": "band",
                "script_version": "9.9.9",
                "seconds": 0.6,
                "probe_ms": 150,
                "wall_clock_step_ms": 0.5,
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
                "reads_skipped": 2,
            },
        )

    def test_no_samples(self):
        summary = probe.summarize([], [], lost=0, errors=0, connect={}, seconds=1, probe_ms=100)
        self.assertEqual(summary["heartbeats"], 0)
        self.assertEqual(summary["max_cmd_ms"], {"start": None, "end": None})
        self.assertEqual(summary["arrival_gap_ms"], {"max": None, "over_150_ms": 0})
        self.assertEqual(summary["heartbeat_gap_ms"], {"max": None, "over_150_ms": 0})
        self.assertEqual(summary["reads_skipped"], 0)
        self.assertIsNone(summary["wall_clock_step_ms"])
        self.assertEqual(summary["result_tick_gap_ms"]["count"], 0)
        self.assertEqual(summary["read_inbound_ms"]["count"], 0)
        self.assertIsNone(summary["instance"])


class TickGaps(unittest.TestCase):
    def test_results_of_one_tick_split_by_a_millisecond_are_one_tick(self):
        # The script's `ts` is whole milliseconds: one drain's results can
        # land on both sides of a millisecond boundary (#5 review).
        self.assertEqual(probe.tick_gaps([1000, 1001, 1010, 1020, 1021]), [10, 10])
        self.assertEqual(probe.tick_gaps([1000, 1000, 1034, 1035, 1068]), [34, 34])

    def test_the_gap_is_measured_between_the_ticks_first_results(self):
        self.assertEqual(probe.tick_gaps([1000, 1002, 1034, 1036, 1037, 1068]), [34, 34])

    def test_few_timestamps(self):
        self.assertEqual(probe.tick_gaps([]), [])
        self.assertEqual(probe.tick_gaps([1000, 1001]), [])

    def test_the_same_tick_limit_is_three_milliseconds(self):
        self.assertEqual(probe.SAME_TICK_MS, 3.0)
        self.assertEqual(probe.tick_gaps([1000, 1003]), [3])
        self.assertEqual(probe.tick_gaps([1000, 1002.9]), [])


def stepping(steps_s, start=1.0):
    """A clock read twice per value, advancing by ``steps_s`` in a cycle."""
    values = itertools.accumulate(itertools.cycle(steps_s), initial=start)
    return itertools.chain.from_iterable((value, value) for value in values).__next__


class WallClockStep(unittest.TestCase):
    def test_a_half_millisecond_clock(self):
        self.assertEqual(probe.wall_clock_step_ms(clock=stepping([0.0005])), 0.5)

    def test_a_coarse_clock_flags_itself(self):
        # Windows' GetSystemTimeAsFileTime at the default 15.625 ms.
        self.assertEqual(probe.wall_clock_step_ms(clock=stepping([0.015625], 0.0)), 15.625)

    def test_an_occasional_split_step_is_not_the_clocks_step(self):
        # Seen on the PC: a 0.5 ms clock now and then splits one step in two
        # (0.502 = 0.037 + 0.465); the smallest step read 0.025 ms on a run.
        clock = stepping([0.0005, 0.0005, 0.00005, 0.00045])
        self.assertEqual(probe.wall_clock_step_ms(clock=clock), 0.5)

    def test_a_clock_that_does_not_move(self):
        still = itertools.repeat(5.0)
        steady = itertools.count(0.0, 0.001)
        self.assertIsNone(
            probe.wall_clock_step_ms(clock=still.__next__, limit_s=0.01, timer=steady.__next__)
        )


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
        # A heartbeat every 100 ms, a read every 50 ms, for 1.5 s (15 and 30
        # nominally; a loaded machine delays both, so only "several" is pinned,
        # and the ages are the machine's, not the probe's: never bounded here).
        self.assertGreaterEqual(summary["heartbeats"], 3, summary)
        self.assertEqual(summary["main_tick_age_ms"]["count"], summary["heartbeats"])
        self.assertGreaterEqual(summary["main_tick_age_ms"]["min"], 0.0)
        self.assertGreaterEqual(summary["round_trip_ms"]["count"], 5, summary)
        self.assertGreater(summary["round_trip_ms"]["min"], 0.0)
        self.assertEqual(summary["round_trips_lost"], 0)
        self.assertEqual(summary["round_trip_errors"], 0)
        self.assertEqual(summary["reads_skipped"], 0)
        self.assertGreater(summary["wall_clock_step_ms"], 0.0)
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
        self.assertGreaterEqual(len(reads), 5)
        # Two clocks (the wall clock for the split, a performance counter for
        # the round trip) read one after the other (the script's `ts` cancels
        # out of the sum): nine reads in ten agree within 2 ms (a probe
        # preempted between its two clock reads may miss once).
        misses = sorted(
            abs(read["inbound_ms"] + read["outbound_ms"] - read["round_trip_ms"]) for read in reads
        )
        self.assertLessEqual(misses[int(len(misses) * 0.9) - 1], 2.0, misses)

    def test_reads_faster_than_the_ticks_show_the_tick_interval(self):
        # SimLive runs the script's timer every TIMER_INTERVAL_MS (10 ms); a
        # read every 2 ms lands in nearly every tick.
        summary = probe.run(self.host.port, seconds=1.0, probe_ms=2)
        gaps = summary["result_tick_gap_ms"]
        self.assertGreaterEqual(gaps["count"], 10, summary)
        self.assertGreaterEqual(gaps["p50"], 5.0, summary)
        self.assertLessEqual(gaps["p50"], 30.0, summary)

    def test_a_main_thread_stall_shows_in_the_ages_and_the_round_trips(self):
        """A 700 ms stall of SimLive's main thread (as the script's own
        integration test uses): heartbeats report it, and the read that waited
        longest waited mostly on the way in (the drain), not on the way out.

        Judged per read, not by comparing maxima: on a loaded machine any read
        can also be held on its way out by the scheduler (#5 review: 3 of 24
        parallel runs; with 5 suites at once even the stall's read was held
        377 ms), but less than the 700 ms it waited on the way in."""
        raw = os.path.join(self.host.log_dir, "stall.json")
        stall = threading.Timer(0.5, self.host.control, args=("stall 700",))
        stall.start()
        self.addCleanup(stall.cancel)
        summary = probe.run(self.host.port, seconds=2.0, probe_ms=50, raw_path=raw)
        self.assertGreaterEqual(summary["main_tick_age_over_150_ms"], 1, summary)
        self.assertGreaterEqual(summary["main_tick_age_ms"]["max"], 400.0, summary)
        self.assertLess(summary["main_tick_age_ms"]["max"], 1500.0, summary)
        self.assertGreaterEqual(summary["round_trip_over_150_ms"], 1, summary)
        self.assertEqual(summary["round_trips_lost"], 0)
        with open(raw, encoding="utf-8") as f:
            reads = json.load(f)["reads"]
        longest = max(reads, key=lambda read: read["inbound_ms"])
        self.assertGreaterEqual(longest["inbound_ms"], 300.0, longest)
        self.assertLess(longest["outbound_ms"], longest["inbound_ms"], longest)

    def test_the_cli_prints_the_summary_as_json(self):
        done = subprocess.run(
            [sys.executable, PROBE, "--port", str(self.host.port), "--seconds", "1.5"],
            capture_output=True,
            text=True,
            timeout=20,
        )
        self.assertEqual(done.returncode, 0, done.stderr)
        summary = json.loads(done.stdout)
        self.assertEqual(summary["instance"], "band")
        self.assertEqual(summary["probe_ms"], 100)
        self.assertGreaterEqual(summary["heartbeats"], 3, summary)
        self.assertGreaterEqual(summary["round_trip_ms"]["count"], 3, summary)
        # Which clock the split's wall_clock_step_ms belongs to: it is the
        # script's `ts` clock only while both read the same one (Windows:
        # GetSystemTimeAsFileTime before Python 3.13, as Live's 3.11).
        self.assertEqual(summary["python"], platform.python_version())
        self.assertEqual(summary["wall_clock"], time.get_clock_info("time").implementation)

    def test_the_raw_samples_go_to_a_file(self):
        raw = os.path.join(self.host.log_dir, "raw.json")
        probe.run(self.host.port, seconds=1.0, probe_ms=50, raw_path=raw)
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

    def __init__(self, first_frames, answer=True, batch=1, delay_s=0.0):
        """``answer=False``: never answers a read; ``batch=N``: holds the reads
        until N have come, then answers them all ``delay_s`` later (a script
        behind its reads)."""
        self.answer = answer
        self.batch = batch
        self.delay_s = delay_s
        self.held = []
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
                if not self.answer:
                    continue
                self.held.append(json.loads(bytes(payload))["uuid"])
                if len(self.held) < self.batch:
                    continue
                time.sleep(self.delay_s)
                now = round(time.time() * 1000)
                result = [{"ok": True, "data": 120.0}]
                for uuid in self.held:
                    conn.sendall(
                        text_frame({"event": "result", "uuid": uuid, "data": result, "ts": now})
                    )
                self.held = []
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
        # The heartbeat before connect carries an age no later one has, so
        # the summary shows whether it was counted, whatever the timing.
        early = {"main_tick_age_ms": 999.0, "max_cmd_ms": 1.0, "gap_ms": 100.0}
        script = FakeScript(
            [
                {"event": "heartbeat", "data": early, "ts": 1000},
                {"event": "connect", "data": CONNECT, "ts": 1001},
                {"event": "heartbeat", "data": BEAT, "ts": 1002},
            ]
        )
        self.addCleanup(script.close)
        summary = probe.run(script.port, seconds=1.0, probe_ms=1000)
        self.assertEqual(summary["instance"], "band")
        self.assertGreaterEqual(summary["heartbeats"], 1, summary)
        self.assertEqual(summary["main_tick_age_ms"]["max"], 31.0, summary)
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


class Limits(unittest.TestCase):
    def test_unanswered_reads_are_capped_and_the_skipped_slots_counted(self):
        """A script that stops answering (or answers slower than the reads
        come) must not pile reads onto a production Live: past ``max_pending``
        unanswered reads the probe skips its read slots and counts them."""
        script = FakeScript([{"event": "connect", "data": CONNECT, "ts": 1000}], answer=False)
        self.addCleanup(script.close)
        summary = probe.run(script.port, seconds=0.3, probe_ms=1, max_pending=5, result_wait_s=0.2)
        # 5 reads out, none answered, so every later slot is skipped (300
        # slots nominally; a loaded machine runs the loop fewer times).
        self.assertEqual(summary["round_trips_lost"], 5, summary)
        self.assertEqual(summary["round_trip_ms"]["count"], 0)
        self.assertGreaterEqual(summary["reads_skipped"], 10, summary)

    def test_reads_resume_once_the_script_catches_up(self):
        # The fake answers only in batches of 5, 20 ms after the fifth read:
        # at the cap of 5 the probe skips its 2 ms slots meanwhile, and it
        # reads again once the batch is answered.
        connect = [{"event": "connect", "data": CONNECT, "ts": 1000}]
        script = FakeScript(connect, batch=5, delay_s=0.02)
        self.addCleanup(script.close)
        summary = probe.run(script.port, seconds=0.5, probe_ms=2, max_pending=5, result_wait_s=0.2)
        self.assertGreater(summary["reads_skipped"], 0, summary)
        self.assertGreaterEqual(summary["round_trip_ms"]["count"], 10, summary)
        self.assertLessEqual(summary["round_trips_lost"], 5, summary)

    def test_the_default_cap_keeps_far_below_the_scripts_result_queue(self):
        # RESULT_QUEUE_MAX = 1000 in the script's Config.py.
        self.assertEqual(probe.MAX_PENDING, 50)

    def test_an_unwritable_raw_file_fails_before_the_run(self):
        missing = os.path.join(tempfile.gettempdir(), "no-such-dir-live-probe", "raw.json")
        started = time.monotonic()
        with self.assertRaisesRegex(probe.ProbeError, "cannot write the raw file"):
            probe.run(1, seconds=300, probe_ms=100, raw_path=missing)
        self.assertLess(time.monotonic() - started, 5.0)


class RawFile(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.mkdtemp(prefix="live-probe-raw-")
        self.addCleanup(shutil.rmtree, self.folder, True)

    def test_a_failed_run_leaves_an_earlier_raw_file_as_it_was(self):
        raw = os.path.join(self.folder, "run.json")
        with open(raw, "w", encoding="utf-8") as f:
            f.write("earlier run")
        with socket.socket() as holder:
            holder.bind(("127.0.0.1", 0))
            port = holder.getsockname()[1]
        with self.assertRaisesRegex(probe.ProbeError, "cannot connect"):
            probe.run(port, seconds=1, probe_ms=100, raw_path=raw)
        with open(raw, encoding="utf-8") as f:
            self.assertEqual(f.read(), "earlier run")

    def test_a_raw_write_that_fails_after_the_run_keeps_the_summary(self):
        script = FakeScript([{"event": "connect", "data": CONNECT, "ts": 1000}])
        self.addCleanup(script.close)
        # A folder where the file should go: writable parent, the write fails.
        with self.assertRaisesRegex(probe.RawWriteError, "cannot write the raw file") as caught:
            probe.run(script.port, seconds=0.3, probe_ms=100, raw_path=self.folder)
        self.assertEqual(caught.exception.summary["instance"], "band")
        self.assertGreaterEqual(caught.exception.summary["round_trip_ms"]["count"], 1)
        # The half-written temporary file is not left behind.
        self.assertFalse(os.path.exists(self.folder + ".tmp"))

    def test_the_cli_prints_the_summary_when_only_the_raw_file_failed(self):
        script = FakeScript([{"event": "connect", "data": CONNECT, "ts": 1000}])
        self.addCleanup(script.close)
        done = subprocess.run(
            [
                sys.executable,
                PROBE,
                "--port",
                str(script.port),
                "--seconds",
                "0.3",
                "--raw",
                self.folder,
            ],
            capture_output=True,
            text=True,
            timeout=20,
        )
        self.assertEqual(done.returncode, 1, done.stderr)
        self.assertEqual(json.loads(done.stdout)["instance"], "band")
        self.assertIn("live-probe: cannot write the raw file", done.stderr)


if __name__ == "__main__":
    unittest.main()
