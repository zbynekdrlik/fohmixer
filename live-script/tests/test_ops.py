"""Generic operations: get_prop, set_prop, describe and raw LOM calls."""

import unittest

import _paths
import Live
import site_builder
from FohMixer.lom import ops
from FohMixer.lom.errors import OpError, PathError
from FohMixer.lom.registry import Registry

VOLUME = "live_set tracks 0 mixer_device volume"


class OpsTest(unittest.TestCase):
    def setUp(self):
        self.song = site_builder.build(_paths.FIXTURE)
        self.ctx = ops.Context(
            song=self.song,
            app=Live.Application.get_application(),
            registry=Registry(),
            subscriptions=None,
            connection=None,
        )

    def run_cmd(self, target, name, args=None):
        command = {"target": target, "name": name}
        if args is not None:
            command["args"] = args
        return ops.execute(command, self.ctx)

    def assert_op_error(self, kind, target, name, args=None):
        with self.assertRaises(OpError) as ctx:
            self.run_cmd(target, name, args)
        self.assertEqual(ctx.exception.kind, kind)
        return ctx.exception

    # --- get_prop ---

    def test_get_prop_name(self):
        self.assertEqual(self.run_cmd("live_set tracks 0", "get_prop", {"prop": "name"}), "Hand1 #")

    def test_get_prop_value_with_display(self):
        self.assertEqual(
            self.run_cmd(VOLUME, "get_prop", {"prop": "value", "display": True}),
            {"value": 0.85, "display": "0.00 dB"},
        )

    def test_get_prop_object_and_list_and_enum(self):
        mixer = self.run_cmd("live_set tracks[name=Hand2 #]", "get_prop", {"prop": "mixer_device"})
        self.assertEqual(mixer["class"], "MixerDevice")
        self.assertEqual(mixer["path"], "live_set tracks 1 mixer_device")
        tracks = self.run_cmd("live_set", "get_prop", {"prop": "tracks"})
        self.assertEqual([t["name"] for t in tracks][:2], ["Hand1 #", "Hand2 #"])
        state = self.run_cmd("live_set tracks 0", "get_prop", {"prop": "current_monitoring_state"})
        self.assertEqual(state, {"$enum": "AUTO", "value": 1})

    def test_get_prop_by_ref(self):
        tracks = self.run_cmd("live_set", "get_prop", {"prop": "tracks"})
        ref = {"$ref": tracks[2]["$ref"], "class": "Track"}
        self.assertEqual(self.run_cmd(ref, "get_prop", {"prop": "name"}), "Hand3 #")

    def test_get_prop_errors(self):
        self.assert_op_error("no such property", "live_set tracks 0", "get_prop", {"prop": "nope"})
        self.assert_op_error("forbidden", "live_set tracks 0", "get_prop", {"prop": "_live_ptr"})
        self.assert_op_error(
            "not a property", "live_set tracks 0", "get_prop", {"prop": "delete_device"}
        )
        self.assert_op_error("bad args", "live_set tracks 0", "get_prop", {})
        self.assert_op_error("bad args", "live_set tracks 0", "get_prop", ["name"])
        self.assert_op_error("bad args", "live_set tracks 0", "get_prop", {"prop": "name", "x": 1})

    # --- set_prop ---

    def test_set_prop_mute(self):
        self.assertIsNone(
            self.run_cmd("live_set tracks 0", "set_prop", {"prop": "mute", "value": True})
        )
        self.assertTrue(self.song.tracks[0].mute)

    def test_set_prop_out_of_range_fails_and_keeps_value(self):
        error = self.assert_op_error(
            "live error", VOLUME, "set_prop", {"prop": "value", "value": 1.5}
        )
        self.assertEqual(error.error_type, "RuntimeError")
        self.assertEqual(self.song.tracks[0].mixer_device.volume.value, 0.85)

    def test_set_prop_enum_by_member_name(self):
        self.run_cmd(
            "live_set tracks 0",
            "set_prop",
            {"prop": "current_monitoring_state", "value": {"$enum": "IN"}},
        )
        self.assertIs(
            self.song.tracks[0].current_monitoring_state, Live.Track.Track.monitoring_states.IN
        )

    def test_set_prop_errors(self):
        self.assert_op_error(
            "live error", "live_set tracks 0", "set_prop", {"prop": "is_foldable", "value": 1}
        )
        self.assert_op_error(
            "no such property", "live_set tracks 0", "set_prop", {"prop": "nope", "value": 1}
        )
        self.assert_op_error("bad args", "live_set tracks 0", "set_prop", {"prop": "mute"})
        self.assert_op_error(
            "forbidden", "live_set tracks 0", "set_prop", {"prop": "_v", "value": {}}
        )

    # --- calls ---

    def test_raw_call_positional_and_keyword(self):
        self.assertEqual(self.run_cmd(VOLUME, "str_for_value", [0.5]), "-14.0 dB")
        self.assertEqual(self.run_cmd(VOLUME, "str_for_value", {"value": 0.5}), "-14.0 dB")

    def test_raw_call_mutates_and_returns_null(self):
        self.assertIsNone(self.run_cmd("live_set", "delete_track", [0]))
        self.assertEqual(self.song.tracks[0].name, "Hand2 #")

    def test_raw_call_returning_object(self):
        doc = self.run_cmd("live_app", "get_document", [])
        self.assertEqual(doc["class"], "Song")
        self.assertEqual(doc["$ref"], f"live_{self.song._live_ptr}")

    def test_raw_call_errors(self):
        self.assert_op_error("no such function", "live_set tracks 0", "nope", [])
        self.assert_op_error("no such function", "live_set tracks 0", "mute", [])
        self.assert_op_error("no such function", "live_set tracks 0", "monitoring_states", [])
        self.assert_op_error("forbidden", "live_set tracks 0", "add_mute_listener", [])
        self.assert_op_error("live error", VOLUME, "str_for_value", [])
        self.assert_op_error("bad args", VOLUME, "str_for_value", "0.5")

    def test_underscore_names_are_forbidden(self):
        for name in ("_sim_set", "__str__", "__class__", "_"):
            with self.subTest(name=name):
                self.assert_op_error("forbidden", VOLUME, name, [])

    # --- describe ---

    def test_describe_track(self):
        out = self.run_cmd("live_set tracks 0", "describe", {})
        self.assertEqual(out["class"], "Track")
        self.assertIn("mute", out["observable"])
        self.assertIn("output_meter_left", out["observable"])
        self.assertIn("delete_device", out["functions"])
        self.assertIn("name", out["properties"])
        self.assertIn("mixer_device", out["properties"])
        everything = out["observable"] + out["functions"] + out["properties"]
        self.assertFalse([n for n in everything if n.startswith("_")])
        self.assertNotIn("add_mute_listener", out["functions"])
        self.assertNotIn("monitoring_states", out["functions"])

    def test_describe_parameter(self):
        out = self.run_cmd(VOLUME, "describe")
        self.assertEqual(out["class"], "DeviceParameter")
        self.assertIn("str_for_value", out["functions"])
        self.assertIn("value", out["observable"])

    # --- malformed commands ---

    def test_malformed_commands(self):
        for command in (
            None,
            [],
            {"target": "live_set"},
            {"target": "live_set", "name": 3},
            {"target": "live_set", "name": ""},
            {"name": "get_prop", "args": {"prop": "name"}},
        ):
            with self.subTest(command=command):
                with self.assertRaises(OpError) as ctx:
                    ops.execute(command, self.ctx)
                self.assertEqual(ctx.exception.kind, "malformed command")

    def test_bad_target_is_a_path_error(self):
        with self.assertRaises(PathError):
            self.run_cmd(42, "get_prop", {"prop": "name"})


if __name__ == "__main__":
    unittest.main()
