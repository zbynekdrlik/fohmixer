"""The hub's DRY writes, made when the hub is gone (#140 PR 3a, spec §8):
``SafeWrites`` on SimLive's objects, the ``safe_writes`` command through
``ops``, and the surface making them when a connection ends."""

import logging
import select
import shutil
import struct
import tempfile
import time
import unittest

import _paths
import FohMixer
import Live
import site_builder
from _rawclient import CLOSE, RawClient, masked_frame, request_frame
from c_instance import CInstance
from FohMixer import Config
from FohMixer.lom import ops
from FohMixer.lom import path as lom_path
from FohMixer.lom.errors import OpError
from FohMixer.lom.registry import Registry, is_alive
from FohMixer.safe import SafeWrites

# The fixture's two `Latencies` racks: on top of Vocal 2 repro# (the repro
# form, on 0, off 127) and inside Stream FX on Vocal 3 repro# (the stream
# form, on 127, off 0); each one's first macro.
# RFC 6455's ping opcode, pinned here (as `_rawclient`'s close), not taken
# from the script under test.
PING = 0x9
REPRO_RACK = "live_set tracks 6 devices 0"
STREAM_RACK = "live_set tracks 7 devices 0 chains 0 devices 0"
REPRO = f"{REPRO_RACK} parameters 1"
STREAM = f"{STREAM_RACK} parameters 1"
LOST = "hub lost (disconnect): set 2 autotune selectors to DRY, 0 already there"
SILENCE_S = Config.HUB_SILENCE_MS / 1000.0


class RecordingHandler(logging.Handler):
    def __init__(self):
        super().__init__()
        self.messages = []

    def emit(self, record):
        self.messages.append(record.getMessage())


def recording_logger(name):
    logger = logging.getLogger(f"test-safe.{name}")
    logger.propagate = False
    logger.setLevel(logging.WARNING)
    handler = RecordingHandler()
    logger.handlers = [handler]
    return logger, handler


class Conn:
    """A connection as the drain hands it over: its identity, and when the
    hub was last heard on it (``Connection.last_heard_s``)."""

    def __init__(self, last_heard_s=0.0):
        self.last_heard_s = last_heard_s


class SafeTest(unittest.TestCase):
    def setUp(self):
        self.song = site_builder.build(_paths.FIXTURE)
        self.log, self.lines = recording_logger(self.id())
        self.safe = SafeWrites(self.log, SILENCE_S)
        self.ctx = ops.Context(
            song=self.song,
            app=Live.Application.get_application(),
            registry=Registry(),
            subscriptions=None,
            connection=Conn(),
            safe=self.safe,
        )

    def param(self, path):
        """The parameter object at `path` (SimLive's own, not through a command)."""
        obj, _where = lom_path.resolve_path(path, self.song, Live.Application.get_application())
        return obj

    def run_cmd(self, target, name, args=None):
        return ops.execute({"target": target, "name": name, "args": args or {}}, self.ctx)

    def ref(self, path):
        """The `$ref` of the parameter at `path`, as the hub's find gets it
        (its rack's `parameters` read through the script)."""
        rack, _, index = path.rpartition(" parameters ")
        params = self.run_cmd(rack, "get_prop", {"prop": "parameters"})
        return {"$ref": params[int(index)]["$ref"], "class": "DeviceParameter"}

    def register(self, *writes):
        return self.run_cmd(
            "live_set",
            "safe_writes",
            {"writes": [{"target": self.ref(path), "value": value} for path, value in writes]},
        )

    def refused(self, writes):
        with self.assertRaises(OpError) as caught:
            self.run_cmd("live_set", "safe_writes", {"writes": writes})
        self.assertEqual(caught.exception.kind, "bad args")
        return caught.exception.detail

    # --- SafeWrites ---

    def test_arming_replaces_the_list_and_an_empty_one_disarms(self):
        c = Conn()
        p1, p2 = self.param(REPRO), self.param(STREAM)
        self.assertFalse(self.safe.armed(c))
        self.assertEqual(self.safe.arm(c, [(p1, 0.0), (p2, 127.0)]), 2)
        self.assertTrue(self.safe.armed(c))
        self.assertEqual(self.safe.arm(c, [(p2, 127.0)]), 1)
        self.assertEqual(self.safe.fire(c, "disconnect"), 1)
        self.assertEqual((p1.value, p2.value), (0.0, 127.0), "the second list replaced the first")
        self.assertEqual(self.safe.arm(c, [(p1, 0.0)]), 1)
        self.assertEqual(self.safe.arm(c, []), 0)
        self.assertFalse(self.safe.armed(c))
        self.assertEqual(self.safe.fire(c, "disconnect"), 0)
        self.assertEqual(
            self.lines.messages,
            ["hub lost (disconnect): set 1 autotune selectors to DRY, 0 already there"],
        )

    def test_fire_writes_each_once_and_clears(self):
        c = Conn()
        p1, p2 = self.param(REPRO), self.param(STREAM)
        p1._sim_set("value", 64.0)
        p2._sim_set("value", 64.0)
        self.safe.arm(c, [(p1, 0.0), (p2, 127.0)])
        self.assertEqual(self.safe.fire(c, "disconnect"), 2)
        self.assertEqual((p1.value, p2.value), (0.0, 127.0))
        self.assertEqual((p1._sim_set_count("value"), p2._sim_set_count("value")), (1, 1))
        self.assertFalse(self.safe.armed(c))
        self.assertEqual(self.safe.fire(c, "disconnect"), 0)
        self.assertEqual((p1._sim_set_count("value"), p2._sim_set_count("value")), (1, 1))
        self.assertEqual(self.lines.messages, [LOST])

    def test_a_deleted_object_is_skipped(self):
        c = Conn()
        p1, p2 = self.param(REPRO), self.param(STREAM)
        self.safe.arm(c, [(p1, 127.0), (p2, 0.0)])
        self.song.tracks[6]._devices[0]._sim_delete()
        self.assertFalse(is_alive(p1))
        p2._sim_set("value", 127.0)
        self.assertEqual(self.safe.fire(c, "disconnect"), 1)
        self.assertEqual(p2.value, 0.0)
        self.assertEqual(
            self.lines.messages,
            ["hub lost (disconnect): set 1 autotune selectors to DRY, 0 already there"],
        )

    def test_one_connections_list_is_its_own(self):
        c1, c2 = Conn(), Conn()
        p1, p2 = self.param(REPRO), self.param(STREAM)
        self.safe.arm(c1, [(p1, 127.0)])
        self.safe.arm(c2, [(p2, 0.0)])
        p2._sim_set("value", 127.0)
        self.assertEqual(self.safe.fire(c1, "disconnect"), 1)
        self.assertTrue(self.safe.armed(c2))
        self.assertEqual((p1.value, p2.value), (127.0, 127.0), "c2's selector untouched")
        self.assertEqual(self.safe.fire(c2, "disconnect"), 1)
        self.assertEqual(p2.value, 0.0)

    def test_a_write_live_refuses_is_logged_and_the_others_still_go(self):
        # A Max map holds the parameter: Live refuses the write.
        c = Conn()
        p1, p2 = self.param(REPRO), self.param(STREAM)
        p1._sim_enable(False)
        p2._sim_set("value", 127.0)
        self.safe.arm(c, [(p1, 127.0), (p2, 0.0)])
        self.assertEqual(self.safe.fire(c, "silence"), 1)
        self.assertEqual((p1.value, p2.value), (0.0, 0.0))
        self.assertEqual(len(self.lines.messages), 2, self.lines.messages)
        self.assertTrue(
            self.lines.messages[0].startswith(
                "hub lost (silence): an autotune selector could not be set to DRY: "
            ),
            self.lines.messages,
        )
        self.assertEqual(
            self.lines.messages[1],
            "hub lost (silence): set 1 autotune selectors to DRY, 0 already there",
        )

    def test_a_selector_already_on_dry_is_not_written(self):
        # PR #146 review M6: an equal write at every hub stop could mark the
        # set changed; a selector Live holds at its DRY value is left alone.
        c = Conn()
        p1, p2 = self.param(REPRO), self.param(STREAM)
        p1._sim_set("value", 127.0)
        p2._sim_set("value", 127.0)
        self.safe.arm(c, [(p1, 127.0), (p2, 0.0)])
        self.assertEqual(self.safe.fire(c, "disconnect"), 1)
        self.assertEqual((p1.value, p2.value), (127.0, 0.0))
        self.assertEqual((p1._sim_set_count("value"), p2._sim_set_count("value")), (0, 1))
        self.assertEqual(
            self.lines.messages,
            ["hub lost (disconnect): set 1 autotune selectors to DRY, 1 already there"],
        )

    def test_clear_forgets_every_list_and_writes_nothing(self):
        c = Conn()
        p1 = self.param(REPRO)
        self.safe.arm(c, [(p1, 127.0)])
        self.safe.clear()
        self.assertFalse(self.safe.armed(c))
        self.assertEqual(self.safe.fire(c, "disconnect"), 0)
        self.assertEqual(p1._sim_set_count("value"), 0)
        self.assertEqual(self.lines.messages, [])

    # --- a silent hub ---

    def test_the_silence_is_1_5_s(self):
        self.assertEqual(Config.HUB_SILENCE_MS, 1500)

    def test_silence_fires_an_armed_connection_after_1_5_s_only(self):
        heard, deaf = Conn(last_heard_s=0.0), Conn(last_heard_s=0.0)
        p1, p2 = self.param(REPRO), self.param(STREAM)
        p2._sim_set("value", 127.0)
        self.safe.arm(heard, [(p1, 127.0), (p2, 0.0)])
        self.assertEqual(self.safe.check_silence(1.4, [heard, deaf]), [])
        self.assertEqual(self.safe.check_silence(SILENCE_S, [heard, deaf]), [], "not past it")
        self.assertEqual((p1.value, p2.value), (0.0, 127.0), "nothing written yet")
        self.assertEqual(self.safe.check_silence(1.6, [heard, deaf]), [(heard, 2)])
        self.assertEqual((p1.value, p2.value), (127.0, 0.0))
        self.assertEqual(
            self.lines.messages,
            ["hub lost (silence): set 2 autotune selectors to DRY, 0 already there"],
        )
        # Once fired, it is forgotten; an unarmed connection never fires.
        self.assertEqual(self.safe.check_silence(60.0, [heard, deaf]), [])
        self.assertEqual(len(self.lines.messages), 1)

    def test_silence_counts_from_when_the_hub_was_last_heard(self):
        c = Conn(last_heard_s=100.0)
        p1 = self.param(REPRO)
        self.safe.arm(c, [(p1, 127.0)])
        self.assertEqual(self.safe.check_silence(101.4, [c]), [])
        c.last_heard_s = 101.4  # heard again
        self.assertEqual(self.safe.check_silence(102.8, [c]), [])
        self.assertEqual(self.safe.check_silence(103.0, [c]), [(c, 1)])

    # --- the command ---

    def test_the_command_arms_each_racks_first_macro_by_its_ref(self):
        self.assertEqual(self.register((REPRO, 127.0), (STREAM, 0)), {"armed": 2})
        self.assertTrue(self.safe.armed(self.ctx.connection))
        p1, p2 = self.param(REPRO), self.param(STREAM)
        p2._sim_set("value", 127.0)
        self.assertEqual(self.safe.fire(self.ctx.connection, "disconnect"), 2)
        self.assertEqual((p1.value, p2.value), (127.0, 0.0))
        # An empty list disarms.
        self.register((REPRO, 127.0))
        self.assertEqual(self.run_cmd("live_set", "safe_writes", {"writes": []}), {"armed": 0})
        self.assertFalse(self.safe.armed(self.ctx.connection))

    def test_the_command_refuses_a_target_that_does_not_resolve_and_keeps_the_old_list(self):
        self.register((REPRO, 127.0), (STREAM, 0.0))
        stale = self.ref(STREAM)
        self.song.tracks[7]._devices[0]._sim_delete()
        writes = [
            {"target": self.ref(REPRO), "value": 127.0},
            {"target": stale, "value": 0.0},
            {"target": {"$ref": "live_999999", "class": "DeviceParameter"}, "value": 0.0},
        ]
        self.assertEqual(self.refused(writes), "2 safe write target(s) do not resolve")
        self.assertTrue(self.safe.armed(self.ctx.connection), "the old list stays")
        self.param(REPRO)._sim_set("value", 0.0)
        self.assertEqual(
            self.safe.fire(self.ctx.connection, "disconnect"), 1, "the old two, one gone"
        )
        self.assertEqual(self.param(REPRO).value, 127.0)

    def test_the_command_refuses_anything_but_a_racks_first_macro(self):
        self.register((REPRO, 127.0))
        hand2 = "live_set tracks 1"
        for path in (
            f"{REPRO_RACK} parameters 0",  # the rack's Device On
            f"{REPRO_RACK} parameters 2",  # its second macro
            f"{hand2} devices 0 parameters 0",  # a plug-in's Device On
            f"{hand2} devices 1 chains 0 devices 0 parameters 0",  # one inside a rack
            "live_set tracks 5 devices 0 parameters 1",  # a device's own parameter 1
        ):
            with self.subTest(path=path):
                writes = [
                    {"target": self.ref(REPRO), "value": 127.0},
                    {"target": self.ref(path), "value": 0.0},
                ]
                self.assertEqual(
                    self.refused(writes), "1 safe write target(s) are no rack's first macro"
                )
        volume = self.run_cmd(f"{hand2} mixer_device", "get_prop", {"prop": "volume"})
        writes = [{"target": {"$ref": volume["$ref"], "class": "DeviceParameter"}, "value": 0.0}]
        self.assertEqual(self.refused(writes), "1 safe write target(s) are no rack's first macro")
        # Nothing of it was armed: the old list stays as it was.
        self.param(REPRO)._sim_set("value", 0.0)
        self.assertEqual(self.safe.fire(self.ctx.connection, "disconnect"), 1)
        self.assertEqual(self.param(REPRO).value, 127.0)
        self.assertEqual(self.param(f"{REPRO_RACK} parameters 0").value, 1.0, "Device On kept")

    def test_the_command_refuses_a_value_out_of_the_macros_range(self):
        for value in (128.0, -1.0, float("inf"), True, "0", None):
            with self.subTest(value=value):
                writes = [
                    {"target": self.ref(REPRO), "value": 127.0},
                    {"target": self.ref(STREAM), "value": value},
                ]
                self.assertEqual(
                    self.refused(writes), "1 safe write value(s) out of the macro's range"
                )
        self.assertFalse(self.safe.armed(self.ctx.connection))
        self.assertEqual(self.register((REPRO, 127), (STREAM, 0)), {"armed": 2}, "ints are numbers")

    def test_the_command_refuses_a_target_named_twice(self):
        # PR #146 review M3: one selector with two values, the last winning.
        self.register((REPRO, 127.0))
        writes = [
            {"target": self.ref(REPRO), "value": 127.0},
            {"target": self.ref(STREAM), "value": 0.0},
            {"target": self.ref(REPRO), "value": 0.0},
        ]
        self.assertEqual(self.refused(writes), "1 safe write target(s) named twice")
        self.param(REPRO)._sim_set("value", 0.0)
        self.assertEqual(self.safe.fire(self.ctx.connection, "disconnect"), 1, "the old list")
        self.assertEqual(self.param(REPRO).value, 127.0)

    def test_the_command_takes_at_most_64_writes(self):
        # PR #146 review M3: one command on Live's main thread stays short.
        self.assertEqual(ops.SAFE_MAX, 64)
        ref = self.ref(REPRO)
        writes = [{"target": ref, "value": 127.0}] * 65
        self.assertEqual(self.refused(writes), "more than 64 safe writes")
        # 64 pass the count (here they name one selector, refused as such).
        self.assertEqual(
            self.refused([{"target": ref, "value": 127.0}] * 64),
            "63 safe write target(s) named twice",
        )
        self.assertFalse(self.safe.armed(self.ctx.connection))

    def test_the_command_takes_only_refs_of_parameters(self):
        ref = self.ref(REPRO)
        for writes in (
            [{"target": REPRO, "value": 0.0}],  # a path, never an index
            [{"target": {"path": REPRO}, "value": 0.0}],
            [{"target": {"$ref": ref["$ref"]}, "value": 0.0}],  # no class
            [{"target": {"$ref": ref["$ref"], "class": "Device"}, "value": 0.0}],
            [{"target": ref}],
            [{"target": ref, "value": 0.0, "prop": "value"}],
            ["not a write"],
        ):
            with self.subTest(writes=writes):
                self.assertEqual(
                    self.refused(writes),
                    "a safe write is {target: {$ref, class: DeviceParameter}, value}",
                )
        self.assertEqual(self.refused("all of them"), "writes must be a list")
        with self.assertRaises(OpError) as caught:
            self.run_cmd("live_set", "safe_writes", {})
        self.assertEqual(
            (caught.exception.kind, caught.exception.detail),
            ("bad args", "safe_writes needs writes"),
        )
        self.assertFalse(self.safe.armed(self.ctx.connection))


class SurfaceSafeTest(unittest.TestCase):
    """The surface hand-ticked (no SimLive main thread, as
    ``test_surface.HandTickedSurfaceTest``): a connection's end makes its
    DRY writes, Live unloading the script makes none."""

    def setUp(self):
        self.log_dir = tempfile.mkdtemp(prefix="fohmixer-test-")
        self.addCleanup(shutil.rmtree, self.log_dir, True)
        saved = (Config.PORT, Config.LOG_DIR, Config.INSTANCE)
        self.addCleanup(self.restore, saved)
        Config.PORT = 0
        Config.LOG_DIR = self.log_dir
        Config.INSTANCE = "safe"
        self.assertIsNone(Live._sim_main_thread, "a SimLive main thread would tick the surface")
        self.song = site_builder.build(_paths.FIXTURE)
        self.surface = FohMixer.create_instance(CInstance(self.song))
        self.addCleanup(self.disconnect_once)
        self.assertTrue(self.surface.server.wait_bound(2.0))

    @staticmethod
    def restore(saved):
        Config.PORT, Config.LOG_DIR, Config.INSTANCE = saved

    def disconnect_once(self):
        if self.surface.connected:
            self.surface.disconnect()

    def log_text(self):
        with open(f"{self.log_dir}/fohmixer-safe.log", encoding="utf-8") as f:
            return f.read()

    def request(self, client, uuid, commands):
        """Sends a request and ticks until its result came."""
        client.sock.sendall(request_frame(uuid, commands))
        for _ in range(20):
            self.surface._on_timer()
            client.readable(0.1)
            client.read_available()
            for message in client.messages():
                if message.get("uuid") == uuid:
                    return message["data"]
        raise AssertionError(f"no result for {uuid}")

    def armed_client(self):
        """A client (the hub) that registered both macros' DRY values."""
        client = RawClient(self.surface.server.port)
        self.addCleanup(client.close)
        refs = []
        for rack in (REPRO_RACK, STREAM_RACK):
            slots = self.request(
                client,
                f"read {rack}",
                [{"target": rack, "name": "get_prop", "args": {"prop": "parameters"}}],
            )
            refs.append({"$ref": slots[0]["data"][1]["$ref"], "class": "DeviceParameter"})
        writes = [{"target": refs[0], "value": 127.0}, {"target": refs[1], "value": 0.0}]
        slots = self.request(
            client,
            "safe",
            [{"target": "live_set", "name": "safe_writes", "args": {"writes": writes}}],
        )
        self.assertEqual(slots, [{"ok": True, "data": {"armed": 2}}])
        return client

    def macros(self):
        repro = self.song.tracks[6].devices[0].parameters[1]
        stream = self.song.tracks[7].devices[0].chains[0].devices[0].parameters[1]
        return repro, stream

    def test_a_connections_end_makes_its_dry_writes_once(self):
        client = self.armed_client()
        repro, stream = self.macros()
        repro._sim_set("value", 0.0)  # the repro rack on
        stream._sim_set("value", 127.0)  # the stream rack on
        client.close()
        for _ in range(5):
            self.surface._on_timer()
        self.assertEqual((repro.value, stream.value), (127.0, 0.0))
        self.assertEqual((repro._sim_set_count("value"), stream._sim_set_count("value")), (1, 1))
        self.assertEqual(self.log_text().count("hub lost (disconnect)"), 1, self.log_text())
        self.assertIn(LOST, self.log_text())

    def test_live_unloading_the_script_makes_none(self):
        self.armed_client()
        repro, stream = self.macros()
        stream._sim_set("value", 127.0)
        (conn,) = self.surface.server.connections()
        self.assertTrue(self.surface.safe.armed(conn))
        self.surface.disconnect()
        self.assertFalse(self.surface.safe.armed(conn), "its list is let go")
        self.assertEqual((repro.value, stream.value), (0.0, 127.0))
        self.assertEqual((repro._sim_set_count("value"), stream._sim_set_count("value")), (0, 0))
        self.assertNotIn("hub lost", self.log_text())

    def test_a_silent_hub_gets_its_dry_writes_and_its_connection_closed(self):
        client = self.armed_client()
        repro, stream = self.macros()
        stream._sim_set("value", 127.0)
        (conn,) = self.surface.server.connections()
        # Heard 1.4 s ago: nothing yet.
        conn.last_heard_s = time.monotonic() - 1.4
        self.surface._on_timer()
        self.assertEqual((repro.value, stream.value), (0.0, 127.0))
        self.assertTrue(conn.is_open)
        # 2 s of silence: the DRY writes go, and the connection closes.
        conn.last_heard_s = time.monotonic() - 2.0
        self.surface._on_timer()
        self.assertEqual((repro.value, stream.value), (127.0, 0.0))
        self.assertFalse(conn.is_open)
        # The close frame goes out (the hub would answer it and reconnect).
        events = []
        close = None
        for _ in range(10):
            client.readable(0.1)
            client.read_available()
            for message in client.messages():
                events.append(message.get("event") or message["opcode"])
                if message.get("opcode") == CLOSE:
                    close = message
            if close is not None:
                break
            self.surface._on_timer()
        self.assertEqual(events[-1], CLOSE, events)
        # With its reason (PR #146 review M5): 1001 and `silent`.
        self.assertEqual(close["payload"], struct.pack("!H", 1001) + b"silent")
        for _ in range(3):
            self.surface._on_timer()
        log = self.log_text()
        self.assertEqual(log.count("hub lost"), 1, log)
        self.assertIn("hub lost (silence): set 2 autotune selectors to DRY, 0 already there", log)
        self.assertIn(f"connection {conn.id}: the hub said nothing for 1.5 s, closing it", log)
        self.assertEqual((repro._sim_set_count("value"), stream._sim_set_count("value")), (1, 1))

    def test_a_hub_heard_after_a_live_stall_is_no_silent_hub(self):
        # The hub's pings waited in the socket while Live's main thread was
        # away: the tick reads them before it judges the silence.
        client = self.armed_client()
        stream = self.macros()[1]
        stream._sim_set("value", 127.0)
        (conn,) = self.surface.server.connections()
        conn.last_heard_s = time.monotonic() - 3.0
        client.sock.sendall(masked_frame(PING, b"keep-alive"))
        self.assertTrue(select.select([conn.socket], [], [], 2.0)[0])
        self.surface._on_timer()
        self.assertTrue(conn.is_open)
        self.assertEqual(stream.value, 127.0)
        self.assertTrue(self.surface.safe.armed(conn))


if __name__ == "__main__":
    unittest.main()
