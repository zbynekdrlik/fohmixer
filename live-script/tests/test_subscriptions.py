"""Listener subscriptions: coalescing, meter cap, shared Live listeners, deleted objects."""

import unittest

import _paths
import Live
import site_builder
from FohMixer.lom import ops
from FohMixer.lom.errors import OpError
from FohMixer.lom.registry import Registry
from FohMixer.subscriptions import Subscriptions

SWEEP_MS = Subscriptions.LIVENESS_INTERVAL_MS


class RecordingConnection:
    """Stands in for a transport connection: records what flush pushes to it."""

    def __init__(self):
        self.pushes = []

    def push_values(self, items):
        self.pushes.append(dict(items))

    def items(self):
        return [item for push in self.pushes for item in push.values()]


class SubscriptionsTest(unittest.TestCase):
    def setUp(self):
        self.song = site_builder.build(_paths.FIXTURE)
        self.registry = Registry()
        self.subs = Subscriptions(self.registry, meter_min_interval_ms=33)
        self.a = RecordingConnection()
        self.b = RecordingConnection()
        self.track = self.song.tracks[0]
        self.volume = self.track.mixer_device.volume

    def test_add_returns_initial_value_and_display(self):
        key, item = self.subs.add(self.volume, "value", True, self.a, "live_set tracks 0 x volume")
        self.assertEqual(key, f"live_{self.volume._live_ptr}.value")
        self.assertEqual(item, {"key": key, "value": 0.85, "display": "0.0 dB"})
        key2, item2 = self.subs.add(self.track, "mute", False, self.a)
        self.assertEqual(item2, {"key": key2, "value": False})

    def test_hundred_changes_coalesce_into_one_push_with_last_value(self):
        key, _ = self.subs.add(self.volume, "value", True, self.a)
        for i in range(100):
            self.volume.value = i / 100.0
        self.assertEqual(self.a.pushes, [])
        self.subs.flush(1000.0)
        self.assertEqual(
            self.a.pushes, [{key: {"key": key, "value": 0.99, "display": str(self.volume)}}]
        )
        self.subs.flush(1010.0)
        self.assertEqual(len(self.a.pushes), 1)

    def test_listener_callback_does_no_lom_change(self):
        self.subs.add(self.track, "mute", False, self.a)
        # SimLive raises if anything is set while a notification runs.
        self.track.mute = True
        self.track.solo = True
        self.assertEqual(self.a.pushes, [])

    def test_two_connections_share_one_live_listener(self):
        key, _ = self.subs.add(self.volume, "value", False, self.a)
        key_b, _ = self.subs.add(self.volume, "value", False, self.b)
        self.assertEqual(key, key_b)
        self.assertEqual(self.volume._sim_listener_count("value"), 1)
        self.volume.value = 0.5
        self.subs.flush(1000.0)
        self.assertEqual(self.a.items(), [{"key": key, "value": 0.5}])
        self.assertEqual(self.b.items(), [{"key": key, "value": 0.5}])
        self.subs.remove(key, self.a)
        self.assertEqual(self.volume._sim_listener_count("value"), 1)
        self.subs.remove(key, self.b)
        self.assertEqual(self.volume._sim_listener_count("value"), 0)
        self.assertEqual(len(self.subs), 0)
        with self.assertRaises(OpError) as ctx:
            self.subs.remove(key, self.a)
        self.assertEqual(ctx.exception.kind, "not subscribed")

    def test_meter_cap(self):
        key, _ = self.subs.add(self.track, "output_meter_left", False, self.a)
        for i in range(10):
            self.track._sim_set_meter(0.1 + i / 100.0)
        self.subs.flush(20.0)
        self.assertEqual(self.a.items(), [{"key": key, "value": 0.1 + 9 / 100.0}])
        self.track._sim_set_meter(0.5)
        self.track._sim_set_meter(0.6)
        self.subs.flush(40.0)
        self.assertEqual(len(self.a.pushes), 1)
        self.subs.flush(53.0)
        self.assertEqual(self.a.pushes[-1], {key: {"key": key, "value": 0.6}})

    def test_non_meter_is_not_capped(self):
        key, _ = self.subs.add(self.track, "mute", False, self.a)
        self.track.mute = True
        self.subs.flush(20.0)
        self.track.mute = False
        self.subs.flush(21.0)
        self.assertEqual([p[key]["value"] for p in self.a.pushes], [True, False])

    def test_deleted_track_gives_gone_and_drops_subscription(self):
        key, _ = self.subs.add(self.track, "mute", False, self.a)
        self.song.delete_track(0)
        self.subs.flush(SWEEP_MS + 1.0)
        self.assertEqual(self.a.items(), [{"key": key, "error": "gone"}])
        self.assertEqual(len(self.subs), 0)
        self.subs.flush(3 * SWEEP_MS)
        self.assertEqual(len(self.a.pushes), 1)

    def test_deleted_while_dirty_gives_gone(self):
        key, _ = self.subs.add(self.volume, "value", False, self.a)
        self.volume.value = 0.3
        self.song.delete_track(0)
        self.subs.flush(1.0)
        self.assertEqual(self.a.items(), [{"key": key, "error": "gone"}])

    def test_vector_property_pushes_refs(self):
        chain_track = self.song.tracks[5]
        key, item = self.subs.add(chain_track, "devices", False, self.a, "live_set tracks 5")
        self.assertEqual(item["value"][0]["name"], "EQ Eight")
        self.assertEqual(item["value"][0]["path"], "live_set tracks 5 devices 0")
        chain_track.delete_device(0)
        self.subs.flush(1.0)
        self.assertEqual(self.a.items(), [{"key": key, "value": []}])

    def test_drop_connection_and_clear_remove_live_listeners(self):
        self.subs.add(self.track, "mute", False, self.a)
        self.subs.add(self.track, "solo", False, self.b)
        self.subs.drop_connection(self.a)
        self.assertEqual(self.track._sim_listener_count("mute"), 0)
        self.assertEqual(self.track._sim_listener_count("solo"), 1)
        self.subs.add(self.volume, "value", False, self.a)
        self.subs.clear()
        self.assertEqual(self.track._sim_listener_count("solo"), 0)
        self.assertEqual(self.volume._sim_listener_count("value"), 0)
        self.assertEqual(len(self.subs), 0)
        self.track.mute = True
        self.subs.flush(10.0)
        self.assertEqual(self.a.pushes, [])


class ListenerOpsTest(unittest.TestCase):
    def setUp(self):
        self.song = site_builder.build(_paths.FIXTURE)
        self.registry = Registry()
        self.conn = RecordingConnection()
        self.subs = Subscriptions(self.registry)
        self.ctx = ops.Context(
            self.song, Live.Application.get_application(), self.registry, self.subs, self.conn
        )

    def run_cmd(self, target, name, args):
        return ops.execute({"target": target, "name": name, "args": args}, self.ctx)

    def test_add_and_remove_listener(self):
        target = "live_set tracks[name=Hand3 #] mixer_device volume"
        out = self.run_cmd(target, "add_listener", {"prop": "value", "display": True})
        volume = self.song.tracks[2].mixer_device.volume
        self.assertEqual(
            out, {"key": f"live_{volume._live_ptr}.value", "value": 0.75, "display": "-4.0 dB"}
        )
        self.assertEqual(volume._sim_listener_count("value"), 1)
        self.assertIsNone(self.run_cmd(target, "remove_listener", {"prop": "value"}))
        self.assertEqual(volume._sim_listener_count("value"), 0)

    def test_listener_errors(self):
        with self.assertRaises(OpError) as ctx:
            self.run_cmd("live_set tracks 0", "add_listener", {"prop": "is_foldable"})
        self.assertEqual(ctx.exception.kind, "not observable")
        with self.assertRaises(OpError) as ctx:
            self.run_cmd("live_set tracks 0", "remove_listener", {"prop": "mute"})
        self.assertEqual(ctx.exception.kind, "not subscribed")


if __name__ == "__main__":
    unittest.main()
