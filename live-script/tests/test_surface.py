"""The surface in-process: the budgeted drain, the lifecycle and the log file."""

import json
import os
import queue
import shutil
import tempfile
import unittest

import _paths
import FohMixer
import site_builder
from c_instance import CInstance
from FohMixer import Config
from FohMixer.surface import Drain, heartbeat_data
from main_thread import MainThread
from websockets.sync.client import connect

BUDGET_MS = 5.0
WINDOW_BUDGET_MS = 5.0
WINDOW_MS = 20.0
TIMER_MS = 10.0


class FakeClock:
    """Nanoseconds that only move when a command runs or a timer interval passes."""

    def __init__(self):
        self.ns = 1_000_000_000

    @property
    def ms(self):
        return self.ns / 1_000_000

    def __call__(self):
        return self.ns

    def advance(self, ms):
        self.ns += round(ms * 1_000_000)


class RecordingConnection:
    def __init__(self):
        self.is_open = True
        self.results = []

    def push_result(self, uuid, data):
        self.results.append((uuid, data))


class DrainTest(unittest.TestCase):
    """The work budget, on a clock the commands advance: exact, never timing-dependent."""

    def setUp(self):
        self.clock = FakeClock()
        self.inbox = queue.Queue()
        self.dropped = []
        self.executed = []
        self.command_ms = {}
        self.drain = Drain(
            self.inbox,
            self.command,
            lambda conn, uuid, results: conn.push_result(uuid, results),
            self.dropped.append,
            BUDGET_MS,
            WINDOW_BUDGET_MS,
            WINDOW_MS,
            clock=self.clock,
        )

    def command(self, command, conn):
        """A LOM command that takes ``ms`` (default 1 ms) of main-thread time."""
        self.executed.append((self.clock.ms, command["n"]))
        self.clock.advance(command.get("ms", 1.0))
        return {"ok": True, "data": command["n"]}

    def timer_calls(self, done, limit=500):
        """Call the drain like the 10 ms timer; return (start ms, duration ms, commands run)."""
        calls = []
        while not done():
            self.assertLess(len(calls), limit, "the drain did not finish")
            started, before = self.clock.ms, len(self.executed)
            self.drain.run()
            calls.append((started, self.clock.ms - started, len(self.executed) - before))
            self.clock.advance(TIMER_MS)
        return calls

    def assert_budget(self, calls, longest_command_ms, spread=True):
        for _started, duration, _count in calls:
            self.assertLessEqual(duration, BUDGET_MS + longest_command_ms)
        if spread:
            busy = [started for started, _d, count in calls if count]
            self.assertGreater(len(busy), 1, "the work was not spread over several timer calls")
        for t in (t for t, _n in self.executed):
            in_window = [s for s, _n in self.executed if t - WINDOW_MS < s <= t]
            self.assertLessEqual(len(in_window), WINDOW_BUDGET_MS)

    def test_fifty_slow_envelopes_spread_over_timer_calls(self):
        conn = RecordingConnection()
        for n in range(50):
            self.inbox.put((conn, {"uuid": f"u{n}", "commands": [{"n": n}]}))
        calls = self.timer_calls(lambda: len(conn.results) == 50)
        self.assert_budget(calls, 1.0)
        self.assertEqual([count for _s, _d, count in calls if count], [5] * 10)
        self.assertEqual([uuid for uuid, _ in conn.results], [f"u{n}" for n in range(50)])
        self.assertEqual(self.drain.max_cmd_ms, 1.0)

    def test_one_envelope_of_fifty_commands_is_split_between_commands(self):
        conn = RecordingConnection()
        self.inbox.put((conn, {"uuid": "big", "commands": [{"n": n} for n in range(50)]}))
        calls = self.timer_calls(lambda: len(conn.results) == 1)
        self.assert_budget(calls, 1.0)
        uuid, slots = conn.results[0]
        self.assertEqual(uuid, "big")
        self.assertEqual([s["data"] for s in slots], list(range(50)))

    def test_a_command_longer_than_the_budget_runs_alone(self):
        conn = RecordingConnection()
        self.inbox.put((conn, {"uuid": "slow", "commands": [{"n": 0, "ms": 12.0}]}))
        self.inbox.put((conn, {"uuid": "next", "commands": [{"n": 1}]}))
        self.drain.run()
        self.assertEqual([n for _t, n in self.executed], [0])
        self.assertEqual(self.drain.max_cmd_ms, 12.0)
        calls = self.timer_calls(lambda: len(conn.results) == 2)
        self.assert_budget(calls, 12.0, spread=False)

    def test_window_budget_blocks_a_back_to_back_call(self):
        conn = RecordingConnection()
        for n in range(20):
            self.inbox.put((conn, {"uuid": f"u{n}", "commands": [{"n": n}]}))
        self.drain.run()
        self.assertEqual(len(conn.results), 5)
        self.clock.advance(TIMER_MS)
        self.drain.run()
        self.assertEqual(len(conn.results), 5)
        self.clock.advance(TIMER_MS)
        self.drain.run()
        self.assertEqual(len(conn.results), 10)

    def test_empty_envelope_and_disconnect_sentinel(self):
        conn = RecordingConnection()
        self.inbox.put((conn, {"uuid": "empty", "commands": []}))
        self.inbox.put((conn, None))
        self.drain.run()
        self.assertEqual(conn.results, [("empty", [])])
        self.assertEqual(self.dropped, [conn])

    def test_commands_of_a_closed_connection_are_skipped(self):
        conn = RecordingConnection()
        conn.is_open = False
        self.inbox.put((conn, {"uuid": "u", "commands": [{"n": 1}, {"n": 2}]}))
        self.drain.run()
        self.assertEqual(conn.results, [])
        self.assertEqual(self.executed, [])

    def test_real_clock_by_default(self):
        drain = Drain(self.inbox, lambda cmd, conn: {"ok": True}, lambda *a: None, None, 5, 5, 20)
        conn = RecordingConnection()
        self.inbox.put((conn, {"uuid": "u", "commands": [{"n": 1}]}))
        drain.run()
        self.assertTrue(self.inbox.empty())
        self.assertGreater(drain.max_cmd_ms, 0.0)


class SurfaceLifecycleTest(unittest.TestCase):
    def setUp(self):
        self.log_dir = tempfile.mkdtemp(prefix="fohmixer-test-")
        self.addCleanup(shutil.rmtree, self.log_dir, True)
        saved = (Config.PORT, Config.LOG_DIR, Config.INSTANCE)
        self.addCleanup(self.restore_config, saved)
        Config.PORT = 0
        Config.LOG_DIR = self.log_dir
        Config.INSTANCE = "lifecycle"
        self.song = site_builder.build(_paths.FIXTURE)
        self.mt = MainThread().start()
        self.addCleanup(self.mt.stop)
        self.surface = self.mt.call(lambda: FohMixer.create_instance(CInstance(self.song)))
        self.addCleanup(self.disconnect_once)
        self.assertTrue(self.surface.server.wait_bound(2.0))

    def disconnect_once(self):
        if self.surface.connected:
            self.mt.call(self.surface.disconnect)

    @staticmethod
    def restore_config(saved):
        Config.PORT, Config.LOG_DIR, Config.INSTANCE = saved

    def request(self, ws, commands):
        ws.send(json.dumps({"uuid": "r1", "commands": commands}))
        while True:
            frame = json.loads(ws.recv(timeout=2.0))
            if frame["event"] == "result":
                return frame["data"]

    def test_disconnect_releases_listeners_ids_timer_and_socket(self):
        port = self.surface.server.port
        ws = connect(f"ws://127.0.0.1:{port}", open_timeout=2, close_timeout=1)
        self.addCleanup(ws.close)
        track = self.song.tracks[0]
        slots = self.request(
            ws, [{"target": "live_set tracks 0", "name": "add_listener", "args": {"prop": "mute"}}]
        )
        self.assertTrue(slots[0]["ok"], slots)
        self.assertEqual(track._sim_listener_count("mute"), 1)
        self.mt.call(self.surface.disconnect)
        self.assertEqual(track._sim_listener_count("mute"), 0)
        self.assertEqual(len(self.surface.registry), 0)
        self.assertEqual(self.mt._timers, {})
        events = []
        try:
            while True:
                events.append(json.loads(ws.recv(timeout=2.0))["event"])
        except Exception as e:  # noqa: BLE001 - the loop ends when the socket closes
            closed = type(e).__name__
        self.assertIn("disconnect", events)
        self.assertEqual(closed, "ConnectionClosedOK")

    def test_log_file_is_written_at_warning_level(self):
        self.mt.call(self.surface.disconnect)
        path = os.path.join(self.log_dir, "fohmixer-lifecycle.log")
        with open(path, encoding="utf-8") as f:
            text = f.read()
        self.assertIn("FohMixer", text)
        self.assertIn("WARNING fohmixer.lifecycle", text)


class HeartbeatDataTest(unittest.TestCase):
    def test_heartbeat_data_rounds_ages_and_reports_the_threads_own_gap(self):
        self.assertEqual(
            heartbeat_data(31.25, 0.12345, 0.10749),
            {"main_tick_age_ms": 31.2, "max_cmd_ms": 0.123, "gap_ms": 107.5},
        )
        self.assertEqual(heartbeat_data(0.0, 0.0, 1.5)["gap_ms"], 1500.0)


if __name__ == "__main__":
    unittest.main()
