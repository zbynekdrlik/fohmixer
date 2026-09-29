"""The surface in-process: the budgeted drain, the lifecycle, the log file, and
the tick that does all of the script's socket work (#5)."""

import json
import os
import queue
import shutil
import tempfile
import threading
import time
import unittest

import _paths
import FohMixer
import Live
import site_builder
from _rawclient import RawClient, request_frame
from c_instance import CInstance
from FohMixer import Config, surface
from FohMixer.lom import ops
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
    def test_heartbeat_data_rounds_the_age_and_reports_the_gap_since_the_previous_one(self):
        self.assertEqual(
            heartbeat_data(31.25, 0.12345, 0.10749),
            {"main_tick_age_ms": 31.2, "max_cmd_ms": 0.123, "gap_ms": 107.5},
        )
        self.assertEqual(heartbeat_data(0.0, 0.0, 1.5)["gap_ms"], 1500.0)


class HeartbeatsTest(unittest.TestCase):
    """Which ticks send a heartbeat and what it says (#5: made in the tick),
    on explicit times."""

    def test_one_heartbeat_per_interval_on_a_fixed_grid(self):
        # Ticks every 33 ms (Live's ~30 Hz timer): a heartbeat every 100 ms on
        # average, not every fourth tick (132 ms).
        heartbeats = surface.Heartbeats(0.1, 10.0)
        sent = []
        for k in range(1, 32):
            now = 10.0 + 0.033 * k
            if heartbeats.on_tick(now, 0.033, 0.5) is not None:
                sent.append(k)
        self.assertEqual(sent, [4, 7, 10, 13, 16, 19, 22, 25, 28, 31])

    def test_it_reports_the_gap_before_its_tick_and_the_time_since_the_previous_one(self):
        heartbeats = surface.Heartbeats(0.1, 10.0)
        self.assertIsNone(heartbeats.on_tick(10.05, 0.05, 0.2))
        self.assertEqual(
            heartbeats.on_tick(10.11, 0.06, 0.2),
            {"main_tick_age_ms": 60.0, "max_cmd_ms": 0.2, "gap_ms": 110.0},
        )
        # A 700 ms stall: nothing goes out while it lasts (no tick runs); the
        # first tick after it reports it.
        self.assertEqual(
            heartbeats.on_tick(10.81, 0.7, 0.2),
            {"main_tick_age_ms": 700.0, "max_cmd_ms": 0.2, "gap_ms": 700.0},
        )
        # No burst of the heartbeats the stall skipped: the next is an interval later.
        self.assertIsNone(heartbeats.on_tick(10.84, 0.03, 0.2))
        self.assertIsNone(heartbeats.on_tick(10.90, 0.06, 0.2))
        self.assertIsNotNone(heartbeats.on_tick(10.92, 0.02, 0.2))


class HandTickedSurfaceTest(unittest.TestCase):
    """The surface with no SimLive main thread: its timer is inert and the test
    thread is Live's main thread, calling ``_on_timer`` by hand (#5)."""

    def setUp(self):
        self.log_dir = tempfile.mkdtemp(prefix="fohmixer-test-")
        self.addCleanup(shutil.rmtree, self.log_dir, True)
        saved = (Config.PORT, Config.LOG_DIR, Config.INSTANCE)
        self.addCleanup(SurfaceLifecycleTest.restore_config, saved)
        Config.PORT = 0
        Config.LOG_DIR = self.log_dir
        Config.INSTANCE = "ticked"
        # These tests are about the tick's socket work, not the work budget
        # (DrainTest pins that on its own clock): a budget no command reaches
        # on a slow or busy machine keeps them free of its timing.
        budgets = (Config.DRAIN_BUDGET_MS, Config.WINDOW_BUDGET_MS)
        self.addCleanup(self.restore_budgets, budgets)
        Config.DRAIN_BUDGET_MS = Config.WINDOW_BUDGET_MS = 10_000
        self.assertIsNone(Live._sim_main_thread, "a SimLive main thread would tick the surface")
        self.threads_before = set(threading.enumerate())
        self.song = site_builder.build(_paths.FIXTURE)
        self.surface = FohMixer.create_instance(CInstance(self.song))
        self.addCleanup(self.disconnect_once)
        self.assertTrue(self.surface.server.wait_bound(2.0))
        self.port = self.surface.server.port

    def disconnect_once(self):
        if self.surface.connected:
            self.surface.disconnect()

    @staticmethod
    def restore_budgets(budgets):
        Config.DRAIN_BUDGET_MS, Config.WINDOW_BUDGET_MS = budgets

    def client(self, extra=b""):
        client = RawClient(self.port, extra=extra)
        self.addCleanup(client.close)
        return client

    def tick_and_read(self, client):
        self.surface._on_timer()
        client.readable(0.2)
        client.read_available()
        return client.messages()

    def test_the_script_runs_no_threads_of_its_own(self):
        read_name = {"target": "live_set tracks 0", "name": "get_prop", "args": {"prop": "name"}}
        client = self.client(extra=request_frame("u1", [read_name]))
        messages = []
        for _ in range(3):
            messages += self.tick_and_read(client)
        started = [t.name for t in threading.enumerate() if t not in self.threads_before]
        self.assertEqual(started, [])
        self.assertIn("u1", [m.get("uuid") for m in messages])

    def test_nothing_moves_between_ticks_and_a_burst_is_answered_within_two(self):
        read_name = {"target": "live_set tracks 0", "name": "get_prop", "args": {"prop": "name"}}
        burst = b"".join(request_frame(f"u{n}", [read_name]) for n in range(20))
        client = self.client(extra=burst)
        self.assertFalse(client.readable(0.3), "the script answered without a tick")
        messages = []
        for _ in range(2):
            messages += self.tick_and_read(client)
            if sum(m["event"] == "result" for m in messages) == 20:
                break
        self.assertEqual(messages[0]["event"], "connect")
        results = [m for m in messages if m["event"] == "result"]
        self.assertEqual([r["uuid"] for r in results], [f"u{n}" for n in range(20)])
        self.assertEqual(
            {json.dumps(r["data"]) for r in results}, {'[{"ok": true, "data": "Hand1 #"}]'}
        )

    def test_the_heartbeat_is_made_in_the_tick_and_reports_the_gap_before_it(self):
        client = self.client()
        messages = []
        for _ in range(3):
            messages += self.tick_and_read(client)
        self.assertEqual(messages[0]["event"], "connect", messages)
        time.sleep(0.3)
        self.assertFalse(client.readable(0.0), "a heartbeat went out without a tick")
        beats = [m for m in self.tick_and_read(client) if m["event"] == "heartbeat"]
        self.assertEqual(len(beats), 1, beats)
        self.assertGreaterEqual(beats[0]["data"]["main_tick_age_ms"], 290.0, beats)
        self.assertLess(beats[0]["data"]["main_tick_age_ms"], 2000.0, beats)
        self.assertGreaterEqual(beats[0]["data"]["gap_ms"], 290.0, beats)

    def test_a_tick_held_by_its_own_work_is_reported_by_the_heartbeat_it_makes(self):
        # A slow command holds a tick 400 ms, and a heartbeat is due in that
        # tick: it goes out after the silence, so it must report the silence
        # (not the short gap before the tick), or the hub shows busy (overdue),
        # free, busy, free within ~170 ms.
        real = ops.execute

        def execute(command, ctx):
            if command.get("name") == "test_slow":
                time.sleep(0.4)
                return None
            return real(command, ctx)

        self.addCleanup(setattr, ops, "execute", real)
        ops.execute = execute
        client = self.client()
        for _ in range(3):
            self.tick_and_read(client)
        time.sleep(0.12)
        slow = {"target": "live_set", "name": "test_slow", "args": {}}
        client.sock.sendall(request_frame("slow", [slow]))
        messages = self.tick_and_read(client)
        self.assertIn("slow", [m.get("uuid") for m in messages])
        beats = [m for m in messages if m["event"] == "heartbeat"]
        self.assertEqual(len(beats), 1, messages)
        self.assertGreaterEqual(beats[0]["data"]["main_tick_age_ms"], 400.0, beats)

    def test_heartbeats_go_on_while_the_subscription_flush_keeps_failing(self):
        # A failing step must not stop the heartbeat: Live would show busy
        # for ever while its main thread ticks.
        def broken(_now_ms):
            raise RuntimeError("test: the flush fails")

        self.surface._subs.flush = broken
        client = self.client()
        messages = []
        for _ in range(8):
            messages += self.tick_and_read(client)
            time.sleep(0.05)
        self.assertEqual(messages[0]["event"], "connect", messages)
        self.assertGreaterEqual(sum(m["event"] == "heartbeat" for m in messages), 2, messages)


if __name__ == "__main__":
    unittest.main()
